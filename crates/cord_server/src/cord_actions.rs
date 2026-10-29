//! Cord's RPC actions: creating, listing, reading, and renaming cords.
//!
//! This module also holds the helpers the invite and member actions share
//! ([`crate::invite::actions`], [`crate::cord_member_actions`]), and
//! [`cord_action_specs`] returns all of them.
//!
//! - `cord_create`: instance admins only. Creates the cord cell, grants the
//!   caller `cord_owner`, and stamps the cord's role-shaped `cell_grant`
//!   rows, in the action's one transaction, then audits `cord_create`.
//! - `cord_list`: the caller's cords with their role in each, derived from
//!   the caller's cord-scoped role grants.
//! - `cord_get`: one cord, for its members only.
//! - `cord_update`: rename a cord; its owner or admins only, by their role
//!   read afresh in the action's transaction, not the request's snapshot.
//! - `cord_admin_list`: every cord with its member count, for instance
//!   admins' operations. It's the one verb that shows an instance admin a
//!   cord they aren't in; the admin audit log also names such cords,
//!   through their `cord_create` and `cord_update` events.
//!
//! Every cord-role check goes through [`crate::cord_authz`], which matches
//! grant scope exactly. A caller who isn't in a cord gets the same
//! `cord_not_found` as for a cord that doesn't exist, so cord ids can't be
//! probed. The instance-admin actions require a global (unscoped) `admin`
//! grant: the dispatcher's role gate admits only a global grant, and the
//! handlers check it again with `has_scoped_role(ROLE_ADMIN, None)`.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use deadpool_postgres::GenericClient;
use fuz_actions::{ActionContext, ActionHandler, ActionSpec};
use fuz_auth::{
    AuditEventType, AuditLogInput, AuditRouting, AuthSpec, CredentialGate, ROLE_ADMIN,
    RequestActorContext, ResolvedAuthContext,
};
use fuz_http::{
    JsonrpcError, forbidden, internal_error, internal_error_with_source, invalid_input_params,
    not_found, parse_strict_params, parse_uuid_param, reject_unknown_keys,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::cord_authz::{CordRole, cord_memberships, cord_role_in};
use crate::cord_member_queries::{CordMemberError, query_cord_manager_role};
use crate::cord_queries::{
    CordAdminRow, CordCreated, CordRow, query_cord_admin_list, query_cord_create, query_cord_get,
    query_cord_rename, query_cords_by_ids, validate_cord_name,
};
use crate::roles::CORD_SCOPE_KIND;

/// Error reason: no such cord, or the caller isn't in it.
pub const ERROR_CORD_NOT_FOUND: &str = "cord_not_found";
/// Error reason: only instance admins create cords.
pub const ERROR_CORD_CREATE_FORBIDDEN: &str = "cord_create_forbidden";
/// Error reason: only the cord's owner or admins may change it.
pub const ERROR_CORD_UPDATE_FORBIDDEN: &str = "cord_update_forbidden";
/// Error reason: the cord name is missing, not a string, empty, too long, or
/// has a forbidden or misplaced character — every refusal of `name` on
/// `cord_create` and `cord_update`.
pub const ERROR_CORD_NAME_INVALID: &str = "cord_name_invalid";

/// The instance-admin role gate the admin-only specs declare. The dispatcher
/// admits only a global grant of it, and the handlers check that again.
const INSTANCE_ADMIN_ROLES: &[&str] = &[ROLE_ADMIN];

// -- Wire inputs --------------------------------------------------------------

/// The input of `cord_create`. Decoded by hand
/// ([`parse_cord_create_params`]), so every refusal of `name` can carry its
/// reason.
#[derive(Debug)]
struct CordCreateInput {
    /// Validated and trimmed ([`validate_cord_name`]).
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordListInput {
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_http::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordGetInput {
    #[serde(deserialize_with = "fuz_http::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_http::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

/// The input of `cord_update`. Decoded by hand
/// ([`parse_cord_update_params`]), so every refusal of `name` can carry its
/// reason.
#[derive(Debug)]
struct CordUpdateInput {
    cord_id: Uuid,
    /// Validated and trimmed ([`validate_cord_name`]).
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordAdminListInput {
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_http::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

// -- Wire outputs -------------------------------------------------------------

/// A cord as its member sees it: `CordJson` in `cord_action_specs.ts`.
pub(crate) fn to_cord_json(cord: &CordRow, role: CordRole) -> Value {
    json!({
        "id": cord.id,
        "name": cord.name,
        "role": role,
        "created_at": cord.created_at,
    })
}

/// A row of the admin ops list: `CordAdminJson` in `cord_action_specs.ts`.
fn to_cord_admin_json(row: &CordAdminRow) -> Value {
    json!({
        "id": row.cord.id,
        "name": row.cord.name,
        "member_count": row.member_count,
        "created_at": row.cord.created_at,
    })
}

// -- Dispatch -----------------------------------------------------------------

/// Run a handler `(client, …)` over whichever `ActionDb` variant is live: the
/// pooled client for reads, the transaction for side-effecting actions.
macro_rules! dispatch_db {
    ($ctx:expr, $handler:ident ( $($arg:expr),* $(,)? )) => {
        fuz_db::action_db_dispatch!($ctx.db, client => $handler(client, $($arg),*).await)
    };
}

/// Like `dispatch_db!`, also passing `&ctx` so a mutation can audit on the
/// same transaction.
macro_rules! dispatch_db_ctx {
    ($ctx:expr, $handler:ident ( $($arg:expr),* $(,)? )) => {
        fuz_db::action_db_dispatch!($ctx.db, client => $handler(client, &$ctx, $($arg),*).await)
    };
}

pub(crate) use {dispatch_db, dispatch_db_ctx};

pub(crate) type HandlerFuture<'a> =
    Pin<Box<dyn Future<Output = Result<Value, JsonrpcError>> + Send + 'a>>;

/// Erase a handler closure into an [`ActionHandler`], giving the closure its
/// higher-ranked signature so `Box::pin(async move { … })` coerces.
pub(crate) fn handler<F>(f: F) -> ActionHandler
where
    F: for<'a> Fn(Value, ActionContext<'a>) -> HandlerFuture<'a> + Send + Sync + 'static,
{
    Arc::new(f)
}

/// The acting actor. Every cord spec requires one, so the dispatcher has
/// already resolved it; `None` here is a wiring bug.
pub(crate) fn require_actor(
    auth: ResolvedAuthContext<'_>,
) -> Result<&RequestActorContext, JsonrpcError> {
    auth.actor()
        .ok_or_else(|| internal_error("unreachable: a cord action ran without an acting actor"))
}

/// Whether the caller is an instance admin: a **global** `admin` grant. A
/// scoped `admin` grant doesn't count.
fn is_instance_admin(auth: &RequestActorContext) -> bool {
    auth.has_scoped_role(ROLE_ADMIN, None)
}

pub(crate) fn cord_not_found() -> JsonrpcError {
    not_found("cord", Some(ERROR_CORD_NOT_FOUND))
}

fn name_invalid(detail: impl std::fmt::Display) -> JsonrpcError {
    invalid_input_params(&detail.to_string(), Some(ERROR_CORD_NAME_INVALID))
}

fn update_forbidden() -> JsonrpcError {
    forbidden(
        "only the cord's owner or admins may change it",
        Some(ERROR_CORD_UPDATE_FORBIDDEN),
    )
}

/// The caller's role in the cord, read afresh in the action's transaction
/// ([`query_cord_manager_role`]) rather than from the request's snapshot, for
/// the managing verbs that take no cord lock (rename, invite create and
/// revoke): `cord_not_found` outside the cord (the IDOR mask), and the
/// verb's own refusal, `member_forbidden()`, for a plain member, who already
/// knows the cord exists.
pub(crate) async fn require_cord_manager_fresh<C: GenericClient + ?Sized>(
    client: &C,
    method: &str,
    auth: &RequestActorContext,
    cord_id: Uuid,
    member_forbidden: fn() -> JsonrpcError,
) -> Result<CordRole, JsonrpcError> {
    query_cord_manager_role(client, cord_id, auth.actor.id)
        .await
        .map_err(|e| match e {
            CordMemberError::CordNotFound => cord_not_found(),
            CordMemberError::Forbidden => member_forbidden(),
            e => internal_error_with_source(method, &e),
        })
}

// -- Audit --------------------------------------------------------------------

/// Audit one of Cord's own events (`cord_create`, `cord_update`,
/// `cord_invite_*`) in the action's transaction: in the caller's name, or,
/// for an anonymous caller (`auth` of `None`), with the request's IP alone.
pub(crate) async fn audit_cord_event<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: Option<&RequestActorContext>,
    event: &'static str,
    metadata: Value,
) -> Result<(), JsonrpcError> {
    let mut input = AuditLogInput::for_custom_event(event, AuditRouting::InTx, AuditRouting::Pool);
    if let Some(auth) = auth {
        input = input.actor(auth.actor.id).account(auth.account.id);
    }
    ctx.audit_emit(client, input.ip(ctx.client_ip.clone()).metadata(metadata))
        .await
}

/// A cord-role grant, as the spine's grant events record it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CordGrantAudit {
    /// The grant holder's account.
    pub target_account_id: Uuid,
    /// The grant holder.
    pub target_actor_id: Uuid,
    pub cord_id: Uuid,
    pub role: CordRole,
    pub role_grant_id: Uuid,
}

/// Audit a cord-role grant change as the spine's own event
/// (`role_grant_create` or `role_grant_revoke`), in the caller's name, with
/// the cord as the grant's scope. `extra` appends one more metadata field
/// (a revoke's `reason`, an accept's `source_invite_id`).
pub(crate) async fn audit_cord_grant_event<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    event: AuditEventType,
    grant: &CordGrantAudit,
    extra: Option<(&str, Value)>,
) -> Result<(), JsonrpcError> {
    let mut metadata = json!({
        "role": grant.role,
        "role_grant_id": grant.role_grant_id,
        "scope_kind": CORD_SCOPE_KIND,
        "scope_id": grant.cord_id,
    });
    if let Some((key, value)) = extra {
        metadata[key] = value;
    }
    ctx.audit_emit(
        client,
        AuditLogInput::for_event(event)
            .actor(auth.actor.id)
            .account(auth.account.id)
            .target_account(grant.target_account_id)
            .target_actor(Some(grant.target_actor_id))
            .ip(ctx.client_ip.clone())
            .metadata(metadata),
    )
    .await
}

// -- Params validation --------------------------------------------------------
//
// `cord_create` and `cord_update` decode by hand, checking what the declared
// TS spec's schema refuses before the handler runs, in its issue order: an
// unknown key, then each field as the twin lists it. `acting` is declared on
// both, but the dispatcher's authorization phase reads and checks it, not
// these parsers.

/// The keys of `CordCreateInput`'s TS twin.
const CORD_CREATE_KEYS: &[&str] = &["name", "acting"];

/// The keys of `CordUpdateInput`'s TS twin.
const CORD_UPDATE_KEYS: &[&str] = &["cord_id", "name", "acting"];

fn parse_cord_create_params(params: &Value) -> Result<CordCreateInput, JsonrpcError> {
    reject_unknown_keys(params, CORD_CREATE_KEYS)?;
    let name = parse_cord_name_param(params)?;
    Ok(CordCreateInput { name })
}

/// An unknown key or a bad `cord_id` is refused first, with no reason, then
/// the name.
fn parse_cord_update_params(params: &Value) -> Result<CordUpdateInput, JsonrpcError> {
    reject_unknown_keys(params, CORD_UPDATE_KEYS)?;
    let cord_id = parse_uuid_param(params, "cord_id")?;
    let name = parse_cord_name_param(params)?;
    Ok(CordUpdateInput { cord_id, name })
}

/// Read `name` as the twin's `CordName`, returning it trimmed. Every refusal
/// carries [`ERROR_CORD_NAME_INVALID`], the reason the twin's field declares:
/// missing, `null`, not a string, or breaking a rule of
/// [`validate_cord_name`].
fn parse_cord_name_param(params: &Value) -> Result<String, JsonrpcError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .ok_or_else(|| name_invalid("name must be a string"))?;
    validate_cord_name(name).map_err(name_invalid)
}

// -- Handlers -----------------------------------------------------------------

async fn handle_cord_create<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordCreateInput,
) -> Result<Value, JsonrpcError> {
    if !is_instance_admin(auth) {
        return Err(forbidden(
            "only instance admins create cords",
            Some(ERROR_CORD_CREATE_FORBIDDEN),
        ));
    }
    let CordCreated {
        cord,
        owner_role_grant_id,
    } = query_cord_create(client, &input.name, auth.actor.id)
        .await
        .map_err(|e| internal_error_with_source("cord_create", &e))?;
    // the cell has no `created_by`, so the audit row is the creator's record
    audit_cord_event(
        client,
        ctx,
        Some(auth),
        "cord_create",
        json!({
            "cord_id": cord.id,
            "name": cord.name,
            "owner_role_grant_id": owner_role_grant_id,
        }),
    )
    .await?;
    Ok(json!({ "cord": to_cord_json(&cord, CordRole::Owner) }))
}

async fn handle_cord_list<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
) -> Result<Value, JsonrpcError> {
    let memberships = cord_memberships(&auth.role_grants);
    let ids: Vec<Uuid> = memberships.iter().map(|(id, _)| *id).collect();
    let rows = query_cords_by_ids(client, &ids)
        .await
        .map_err(|e| internal_error_with_source("cord_list", &e))?;
    // keep the membership order; a grant whose cord is gone drops out
    let cords: Vec<Value> = memberships
        .iter()
        .filter_map(|(id, role)| {
            rows.iter()
                .find(|row| row.id == *id)
                .map(|row| to_cord_json(row, *role))
        })
        .collect();
    Ok(json!({ "cords": cords }))
}

async fn handle_cord_get<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
    input: &CordGetInput,
) -> Result<Value, JsonrpcError> {
    let Some(role) = cord_role_in(&auth.role_grants, input.cord_id) else {
        return Err(cord_not_found());
    };
    let cord = query_cord_get(client, input.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_get", &e))?
        .ok_or_else(cord_not_found)?;
    Ok(json!({ "cord": to_cord_json(&cord, role) }))
}

async fn handle_cord_update<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordUpdateInput,
) -> Result<Value, JsonrpcError> {
    // the role as the database has it now, not the dispatcher's snapshot,
    // so an admin demoted or removed a moment ago can't rename
    let role =
        require_cord_manager_fresh(client, "cord_update", auth, input.cord_id, update_forbidden)
            .await?;
    let cord = query_cord_rename(client, input.cord_id, &input.name, auth.actor.id)
        .await
        .map_err(|e| internal_error_with_source("cord_update", &e))?
        .ok_or_else(cord_not_found)?;
    audit_cord_event(
        client,
        ctx,
        Some(auth),
        "cord_update",
        json!({ "cord_id": cord.id, "name": cord.name }),
    )
    .await?;
    Ok(json!({ "cord": to_cord_json(&cord, role) }))
}

async fn handle_cord_admin_list<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
) -> Result<Value, JsonrpcError> {
    if !is_instance_admin(auth) {
        return Err(forbidden(
            "only instance admins list every cord",
            Some(fuz_http::INSUFFICIENT_PERMISSIONS_REASON),
        ));
    }
    let rows = query_cord_admin_list(client)
        .await
        .map_err(|e| internal_error_with_source("cord_admin_list", &e))?;
    let cords: Vec<Value> = rows.iter().map(to_cord_admin_json).collect();
    Ok(json!({ "cords": cords }))
}

// -- Specs --------------------------------------------------------------------

/// Cord's action specs, folded into the registry beside the spine's: the
/// cord verbs, then the invite verbs, then the member verbs.
///
/// `invite_redeem_rate_limiter` is `cord_invite_redeem`'s own per-IP limiter
/// ([`crate::invite::actions::CORD_INVITE_REDEEM_RATE_LIMIT`]); `None`, as
/// under `testing_cordd`, leaves redeem to the spine's shared IP class alone.
#[must_use]
pub fn cord_action_specs(
    invite_redeem_rate_limiter: Option<Arc<fuz_auth::RateLimiter>>,
) -> Vec<ActionSpec> {
    let mut specs = cord_domain_action_specs();
    specs.extend(crate::invite::actions::cord_invite_action_specs(
        invite_redeem_rate_limiter,
    ));
    specs.extend(crate::cord_member_actions::cord_member_action_specs());
    specs
}

/// The cord verbs themselves: create, list, get, update, and the admin list.
fn cord_domain_action_specs() -> Vec<ActionSpec> {
    vec![
        ActionSpec::with_side_effects(
            "cord_create",
            AuthSpec::authenticated_gated(CredentialGate::Any, INSTANCE_ADMIN_ROLES),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input = parse_cord_create_params(&params)?;
                    dispatch_db_ctx!(ctx, handle_cord_create(auth, &input))
                })
            }),
        ),
        ActionSpec::read_only(
            "cord_list",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let _input: CordListInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_list(auth))
                })
            }),
        ),
        ActionSpec::read_only(
            "cord_get",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordGetInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_get(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_update",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input = parse_cord_update_params(&params)?;
                    dispatch_db_ctx!(ctx, handle_cord_update(auth, &input))
                })
            }),
        ),
        ActionSpec::read_only(
            "cord_admin_list",
            AuthSpec::authenticated_gated(CredentialGate::Any, INSTANCE_ADMIN_ROLES),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let _input: CordAdminListInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_admin_list(auth))
                })
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_declare_their_auth_and_effects() {
        let specs = cord_domain_action_specs();
        let find = |method: &str| {
            specs
                .iter()
                .find(|s| s.method == method)
                .unwrap_or_else(|| panic!("missing {method}"))
        };
        for method in ["cord_create", "cord_admin_list"] {
            assert_eq!(
                find(method).auth.roles,
                Some(INSTANCE_ADMIN_ROLES),
                "{method}"
            );
        }
        for method in ["cord_list", "cord_get", "cord_update"] {
            assert_eq!(find(method).auth.roles, None, "{method}");
            assert!(find(method).auth.needs_actor(), "{method}");
        }
        for method in ["cord_create", "cord_update"] {
            assert!(find(method).side_effects, "{method}");
        }
        for method in ["cord_list", "cord_get", "cord_admin_list"] {
            assert!(!find(method).side_effects, "{method}");
        }
    }

    #[test]
    fn inputs_are_strict() {
        let parsed: Result<CordGetInput, _> = parse_strict_params(json!({ "cord_id": null }));
        assert!(parsed.is_err(), "null is refused");
        let parsed: CordGetInput = parse_strict_params(json!({
            "cord_id": "11111111-1111-4111-8111-111111111111",
            "acting": "22222222-2222-4222-9222-222222222222",
        }))
        .unwrap();
        assert_eq!(
            parsed.cord_id,
            Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111)
        );
        let _: CordListInput = parse_strict_params(Value::Null).unwrap();
    }

    fn assert_name_invalid(result: Result<impl std::fmt::Debug, JsonrpcError>, case: &str) {
        use fuz_http::{INVALID_PARAMS_MESSAGE, JsonrpcErrorCode};
        let err = result.expect_err(case);
        assert_eq!(err.code, JsonrpcErrorCode::InvalidParams, "{case}");
        assert_eq!(err.message, INVALID_PARAMS_MESSAGE, "{case}");
        assert_eq!(
            err.data,
            Some(json!({ "reason": ERROR_CORD_NAME_INVALID })),
            "{case}"
        );
    }

    fn assert_reasonless_refusal(result: Result<impl std::fmt::Debug, JsonrpcError>, case: &str) {
        use fuz_http::{INVALID_PARAMS_MESSAGE, JsonrpcErrorCode};
        let err = result.expect_err(case);
        assert_eq!(err.code, JsonrpcErrorCode::InvalidParams, "{case}");
        assert_eq!(err.message, INVALID_PARAMS_MESSAGE, "{case}");
        assert_eq!(err.data, None, "{case}: no reason");
    }

    /// Every refusal of `name` on both verbs that take one: missing, `null`,
    /// a non-string, and each rule of `validate_cord_name`.
    #[test]
    fn every_name_refusal_carries_its_reason() {
        let too_long = "x".repeat(crate::cord_queries::CORD_NAME_LENGTH_MAX + 1);
        let cases = [
            ("missing", None),
            ("null", Some(Value::Null)),
            ("a number", Some(json!(42))),
            ("an object", Some(json!({}))),
            ("empty", Some(json!(""))),
            ("blank", Some(json!("   "))),
            ("a line break", Some(json!("two\nlines"))),
            ("an invisible character", Some(json!("\u{200B}"))),
            ("a doubled joiner", Some(json!("a\u{200D}\u{200D}b"))),
            ("too long", Some(json!(too_long))),
        ];
        let cord_id = "11111111-1111-4111-8111-111111111111";
        for (case, name) in cases {
            let mut create = json!({});
            let mut update = json!({ "cord_id": cord_id });
            if let Some(name) = name {
                create["name"] = name.clone();
                update["name"] = name;
            }
            assert_name_invalid(parse_cord_create_params(&create), case);
            assert_name_invalid(parse_cord_update_params(&update), case);
        }
        // no params at all is the empty object: the name is missing
        assert_name_invalid(parse_cord_create_params(&Value::Null), "null params");
    }

    #[test]
    fn a_valid_name_parses_trimmed() {
        let parsed = parse_cord_create_params(&json!({
            "name": "  the porch  ",
            "acting": "22222222-2222-4222-9222-222222222222",
        }))
        .expect("parses");
        assert_eq!(parsed.name, "the porch");
        let parsed = parse_cord_update_params(&json!({
            "cord_id": "11111111-1111-4111-8111-111111111111",
            "name": "the stoop ",
        }))
        .expect("parses");
        assert_eq!(
            (parsed.cord_id, parsed.name.as_str()),
            (
                Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111),
                "the stoop"
            )
        );
    }

    /// An unknown key on the input, or a bad `cord_id` (the twin's first
    /// field), is refused with no reason, whatever the name.
    #[test]
    fn unknown_keys_and_bad_cord_ids_carry_no_reason() {
        for params in [
            json!({ "name": "x", "extra": 1 }),
            json!({ "name": 42, "extra": 1 }),
        ] {
            assert_reasonless_refusal(parse_cord_create_params(&params), &params.to_string());
        }
        for params in [
            json!({ "cord_id": "11111111-1111-4111-8111-111111111111", "name": "x", "extra": 1 }),
            json!({ "name": "x" }),
            json!({ "cord_id": null, "name": "x" }),
            json!({ "cord_id": "not-a-uuid", "name": "x" }),
            json!({ "cord_id": "not-a-uuid", "name": "" }),
        ] {
            assert_reasonless_refusal(parse_cord_update_params(&params), &params.to_string());
        }
    }

    #[test]
    fn cord_json_carries_the_role_string() {
        let cord = CordRow {
            id: Uuid::nil(),
            name: "the porch".to_owned(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        assert_eq!(
            to_cord_json(&cord, CordRole::Admin),
            json!({
                "id": Uuid::nil(),
                "name": "the porch",
                "role": "cord_admin",
                "created_at": "2026-01-01T00:00:00Z",
            })
        );
    }
}

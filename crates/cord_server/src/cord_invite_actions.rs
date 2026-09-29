//! Cord's invite-link actions, for people who already have an account on
//! the instance.
//!
//! - `cord_invite_create`: the cord's owner or admins. Mints a link with an
//!   optional expiry and use limit and returns its raw token, the only time
//!   the token is ever shown. Audited as `cord_invite_create`.
//! - `cord_invite_list`: the owner or admins. The cord's links with their
//!   status, never their tokens.
//! - `cord_invite_revoke`: the owner or admins. Idempotent: revoking a
//!   revoked link succeeds and changes nothing. Audited as
//!   `cord_invite_revoke` when it revokes.
//! - `cord_invite_preview`: any signed-in account holding a token. The link's
//!   status, and the cord's name when the link is active or the caller is
//!   already in the cord, so a dead link shows nothing about its cord.
//! - `cord_invite_accept`: a cookie session only, like the spine's other
//!   self-service grant paths. Grants `cord_member` on the cord and spends a
//!   use; audited as the spine's `role_grant_create` plus
//!   `cord_invite_accept`. A caller already in the cord gets their role back
//!   and spends nothing.
//!
//! The token is the secret, so a real token's refusals name their reason
//! (revoked, expired, exhausted), and any other string gets
//! `cord_invite_not_found`. The managing verbs answer `cord_not_found` to
//! anyone outside the cord, as `cord_get` does.
//!
//! Create and revoke read the caller's role afresh inside the action's
//! transaction ([`query_cord_manager_role`]), not from the request's
//! role-grant snapshot, which the dispatcher takes before the transaction
//! opens: an admin removed in that gap can't mint a link to rejoin through.
//! Neither takes the cord's row lock. Accept locks the link and then the
//! cord, so a revoke that locked the cord and then the link could deadlock
//! against it. List is a read, so it trusts the snapshot.
//!
//! Create is rate-limited per account (the spine's
//! [`RateLimitClass::Account`], as on `role_grant_assign`), since each call
//! mints a live credential.

use deadpool_postgres::GenericClient;
use fuz_actions::{ActionContext, ActionSpec};
use fuz_auth::{
    AuditEventType, AuditLogInput, AuditRouting, AuthSpec, CredentialGate, RateLimitClass,
    RequestActorContext,
};
use fuz_http::{
    JsonrpcError, conflict, forbidden, internal_error_with_source, invalid_params, not_found,
    parse_strict_params,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::cord_actions::{
    cord_not_found, deserialize_present, dispatch_db, dispatch_db_ctx, handler, require_actor,
    to_cord_json,
};
use crate::cord_authz::cord_role_in;
use crate::cord_invite_queries::{
    CORD_INVITE_EXPIRES_IN_SECONDS_MAX, CORD_INVITE_EXPIRES_IN_SECONDS_MIN,
    CORD_INVITE_MAX_USES_MAX, CordInviteAcceptError, CordInviteRow, CordInviteStatus,
    generate_cord_invite_token, hash_cord_invite_token, is_plausible_cord_invite_token,
    query_cord_invite_accept, query_cord_invite_by_token_hash, query_cord_invite_create,
    query_cord_invite_list, query_cord_invite_revoke,
};
use crate::cord_member_queries::{CordMemberError, query_cord_manager_role};
use crate::cord_queries::query_cord_get;
use crate::roles::CORD_SCOPE_KIND;

/// Error reason: no link has that token (or id, on this cord).
pub const ERROR_CORD_INVITE_NOT_FOUND: &str = "cord_invite_not_found";
/// Error reason: the link was revoked.
pub const ERROR_CORD_INVITE_REVOKED: &str = "cord_invite_revoked";
/// Error reason: the link has expired.
pub const ERROR_CORD_INVITE_EXPIRED: &str = "cord_invite_expired";
/// Error reason: the link has no uses left.
pub const ERROR_CORD_INVITE_EXHAUSTED: &str = "cord_invite_exhausted";
/// Error reason: only the cord's owner or admins manage its links.
pub const ERROR_CORD_INVITE_FORBIDDEN: &str = "cord_invite_forbidden";
/// Error reason: the expiry or use limit is out of bounds.
pub const ERROR_CORD_INVITE_INVALID: &str = "cord_invite_invalid";

// -- Wire inputs --------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteCreateInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(default, deserialize_with = "deserialize_present")]
    expires_in_seconds: Option<u32>,
    #[serde(default, deserialize_with = "deserialize_present")]
    max_uses: Option<u32>,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteListInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteRevokeInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    invite_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

/// The input of both `cord_invite_preview` and `cord_invite_accept`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordInviteTokenInput {
    token: String,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

// -- Wire outputs -------------------------------------------------------------

/// A link as its cord's owner and admins see it: `CordInviteJson` in
/// `cord_action_specs.ts`. Never carries the token or its digest.
fn to_cord_invite_json(row: &CordInviteRow) -> Value {
    json!({
        "id": row.id,
        "cord_id": row.cord_id,
        "role": row.role,
        "created_by": row.created_by,
        "created_at": row.created_at,
        "expires_at": row.expires_at,
        "max_uses": row.max_uses,
        "use_count": row.use_count,
        "revoked_at": row.revoked_at,
        "status": row.status(),
    })
}

// -- Helpers ------------------------------------------------------------------

fn invite_not_found() -> JsonrpcError {
    not_found("cord invite", Some(ERROR_CORD_INVITE_NOT_FOUND))
}

fn invite_forbidden() -> JsonrpcError {
    forbidden(
        "only the cord's owner or admins manage its invite links",
        Some(ERROR_CORD_INVITE_FORBIDDEN),
    )
}

/// Check, from the request's role-grant snapshot, that the caller may manage
/// the cord's links: `cord_not_found` outside the cord (the IDOR mask),
/// `cord_invite_forbidden` for a plain member. For the read-only list; the
/// writes use [`require_invite_manager_fresh`].
fn require_invite_manager(auth: &RequestActorContext, cord_id: Uuid) -> Result<(), JsonrpcError> {
    let Some(role) = cord_role_in(&auth.role_grants, cord_id) else {
        return Err(cord_not_found());
    };
    if !role.can_manage() {
        return Err(invite_forbidden());
    }
    Ok(())
}

/// The same check against the caller's role read afresh in the action's
/// transaction ([`query_cord_manager_role`]), for create and revoke.
async fn require_invite_manager_fresh<C: GenericClient + ?Sized>(
    client: &C,
    method: &str,
    auth: &RequestActorContext,
    cord_id: Uuid,
) -> Result<(), JsonrpcError> {
    match query_cord_manager_role(client, cord_id, auth.actor.id).await {
        Ok(_) => Ok(()),
        Err(CordMemberError::CordNotFound) => Err(cord_not_found()),
        Err(CordMemberError::Forbidden) => Err(invite_forbidden()),
        Err(e) => Err(internal_error_with_source(method, &e)),
    }
}

fn validate_invite_limits(input: &CordInviteCreateInput) -> Result<(), JsonrpcError> {
    if let Some(seconds) = input.expires_in_seconds
        && !(CORD_INVITE_EXPIRES_IN_SECONDS_MIN..=CORD_INVITE_EXPIRES_IN_SECONDS_MAX)
            .contains(&seconds)
    {
        return Err(invalid_params(
            &format!(
                "expires_in_seconds must be from {CORD_INVITE_EXPIRES_IN_SECONDS_MIN} to \
                 {CORD_INVITE_EXPIRES_IN_SECONDS_MAX}"
            ),
            Some(ERROR_CORD_INVITE_INVALID),
        ));
    }
    if let Some(max_uses) = input.max_uses
        && !(1..=CORD_INVITE_MAX_USES_MAX).contains(&max_uses)
    {
        return Err(invalid_params(
            &format!("max_uses must be from 1 to {CORD_INVITE_MAX_USES_MAX}"),
            Some(ERROR_CORD_INVITE_INVALID),
        ));
    }
    Ok(())
}

// -- Handlers -----------------------------------------------------------------

async fn handle_cord_invite_create<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordInviteCreateInput,
) -> Result<Value, JsonrpcError> {
    require_invite_manager_fresh(client, "cord_invite_create", auth, input.cord_id).await?;
    validate_invite_limits(input)?;
    // a grant can outlive its cell only in a race with deletion; answer as
    // for any missing cord
    if query_cord_get(client, input.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_create", &e))?
        .is_none()
    {
        return Err(cord_not_found());
    }
    let minted = generate_cord_invite_token();
    let invite = query_cord_invite_create(
        client,
        input.cord_id,
        &minted.token_hash,
        auth.actor.id,
        input.expires_in_seconds,
        input.max_uses,
    )
    .await
    .map_err(|e| internal_error_with_source("cord_invite_create", &e))?;
    ctx.audit_emit(
        client,
        AuditLogInput::for_custom_event(
            "cord_invite_create",
            AuditRouting::InTx,
            AuditRouting::Pool,
        )
        .actor(auth.actor.id)
        .account(auth.account.id)
        .ip(ctx.client_ip.clone())
        .metadata(json!({
            "cord_id": invite.cord_id,
            "invite_id": invite.id,
            "expires_at": invite.expires_at,
            "max_uses": invite.max_uses,
        })),
    )
    .await?;
    Ok(json!({
        "invite": to_cord_invite_json(&invite),
        "token": minted.token,
    }))
}

async fn handle_cord_invite_list<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
    input: &CordInviteListInput,
) -> Result<Value, JsonrpcError> {
    require_invite_manager(auth, input.cord_id)?;
    let rows = query_cord_invite_list(client, input.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_list", &e))?;
    let invites: Vec<Value> = rows.iter().map(to_cord_invite_json).collect();
    Ok(json!({ "invites": invites }))
}

async fn handle_cord_invite_revoke<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordInviteRevokeInput,
) -> Result<Value, JsonrpcError> {
    // no cord lock: accept locks the link then the cord, and this UPDATE
    // locks the link, so taking the cord first would invert that order
    require_invite_manager_fresh(client, "cord_invite_revoke", auth, input.cord_id).await?;
    let Some((invite, revoked)) = query_cord_invite_revoke(client, input.cord_id, input.invite_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_revoke", &e))?
    else {
        return Err(invite_not_found());
    };
    if revoked {
        ctx.audit_emit(
            client,
            AuditLogInput::for_custom_event(
                "cord_invite_revoke",
                AuditRouting::InTx,
                AuditRouting::Pool,
            )
            .actor(auth.actor.id)
            .account(auth.account.id)
            .ip(ctx.client_ip.clone())
            .metadata(json!({ "cord_id": invite.cord_id, "invite_id": invite.id })),
        )
        .await?;
    }
    Ok(json!({ "invite": to_cord_invite_json(&invite) }))
}

async fn handle_cord_invite_preview<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
    input: &CordInviteTokenInput,
) -> Result<Value, JsonrpcError> {
    if !is_plausible_cord_invite_token(&input.token) {
        return Err(invite_not_found());
    }
    let Some(invite) =
        query_cord_invite_by_token_hash(client, &hash_cord_invite_token(&input.token))
            .await
            .map_err(|e| internal_error_with_source("cord_invite_preview", &e))?
    else {
        return Err(invite_not_found());
    };
    let Some(cord) = query_cord_get(client, invite.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_invite_preview", &e))?
    else {
        return Err(invite_not_found());
    };
    let status = invite.status();
    let role = cord_role_in(&auth.role_grants, cord.id);
    // a dead link names no cord to someone outside it
    let cord_json = (status == CordInviteStatus::Active || role.is_some())
        .then(|| json!({ "id": cord.id, "name": cord.name }));
    Ok(json!({ "status": status, "cord": cord_json, "role": role }))
}

async fn handle_cord_invite_accept<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordInviteTokenInput,
) -> Result<Value, JsonrpcError> {
    if !is_plausible_cord_invite_token(&input.token) {
        return Err(invite_not_found());
    }
    let token_hash = hash_cord_invite_token(&input.token);
    let accepted = match query_cord_invite_accept(client, &token_hash, auth.actor.id).await {
        Ok(accepted) => accepted,
        Err(CordInviteAcceptError::NotFound) => return Err(invite_not_found()),
        Err(CordInviteAcceptError::Revoked) => {
            return Err(conflict(
                "this invite link was revoked",
                Some(ERROR_CORD_INVITE_REVOKED),
            ));
        }
        Err(CordInviteAcceptError::Expired) => {
            return Err(conflict(
                "this invite link has expired",
                Some(ERROR_CORD_INVITE_EXPIRED),
            ));
        }
        Err(CordInviteAcceptError::Exhausted) => {
            return Err(conflict(
                "this invite link has no uses left",
                Some(ERROR_CORD_INVITE_EXHAUSTED),
            ));
        }
        Err(CordInviteAcceptError::Db(e)) => {
            return Err(internal_error_with_source("cord_invite_accept", &e));
        }
    };
    if let Some(role_grant_id) = accepted.role_grant_id {
        // the spine's grant event, as its offer accept emits: the caller is
        // both the actor and the target
        ctx.audit_emit(
            client,
            AuditLogInput::for_event(AuditEventType::RoleGrantCreate)
                .actor(auth.actor.id)
                .account(auth.account.id)
                .target_account(auth.account.id)
                .target_actor(Some(auth.actor.id))
                .ip(ctx.client_ip.clone())
                .metadata(json!({
                    "role": accepted.role,
                    "role_grant_id": role_grant_id,
                    "scope_kind": CORD_SCOPE_KIND,
                    "scope_id": accepted.cord.id,
                    "source_invite_id": accepted.invite_id,
                })),
        )
        .await?;
        ctx.audit_emit(
            client,
            AuditLogInput::for_custom_event(
                "cord_invite_accept",
                AuditRouting::InTx,
                AuditRouting::Pool,
            )
            .actor(auth.actor.id)
            .account(auth.account.id)
            .ip(ctx.client_ip.clone())
            .metadata(json!({
                "cord_id": accepted.cord.id,
                "invite_id": accepted.invite_id,
                "role_grant_id": role_grant_id,
            })),
        )
        .await?;
    }
    Ok(json!({
        "cord": to_cord_json(&accepted.cord, accepted.role),
        "joined": accepted.role_grant_id.is_some(),
    }))
}

// -- Specs --------------------------------------------------------------------

/// The invite-link action specs.
pub(crate) fn cord_invite_action_specs() -> Vec<ActionSpec> {
    vec![
        ActionSpec::with_side_effects(
            "cord_invite_create",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordInviteCreateInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_invite_create(auth, &input))
                })
            }),
        )
        .with_rate_limit(RateLimitClass::Account),
        ActionSpec::read_only(
            "cord_invite_list",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordInviteListInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_invite_list(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_invite_revoke",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordInviteRevokeInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_invite_revoke(auth, &input))
                })
            }),
        ),
        ActionSpec::read_only(
            "cord_invite_preview",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordInviteTokenInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_invite_preview(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_invite_accept",
            AuthSpec::authenticated(CredentialGate::SESSION_ONLY),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordInviteTokenInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_invite_accept(auth, &input))
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
        let specs = cord_invite_action_specs();
        let find = |method: &str| {
            specs
                .iter()
                .find(|s| s.method == method)
                .unwrap_or_else(|| panic!("missing {method}"))
        };
        for spec in &specs {
            assert_eq!(spec.auth.roles, None, "{}", spec.method);
            assert!(spec.auth.needs_actor(), "{}", spec.method);
        }
        for method in [
            "cord_invite_create",
            "cord_invite_revoke",
            "cord_invite_accept",
        ] {
            assert!(find(method).side_effects, "{method}");
        }
        for method in ["cord_invite_list", "cord_invite_preview"] {
            assert!(!find(method).side_effects, "{method}");
        }
        assert_eq!(
            find("cord_invite_accept").auth,
            AuthSpec::authenticated(CredentialGate::SESSION_ONLY),
            "accept is cookie-session only"
        );
        assert_eq!(
            find("cord_invite_preview").auth,
            AuthSpec::authenticated(CredentialGate::Any)
        );
    }

    #[test]
    fn only_minting_is_rate_limited_per_account() {
        for spec in cord_invite_action_specs() {
            let expected = (spec.method == "cord_invite_create").then_some(RateLimitClass::Account);
            assert_eq!(spec.rate_limit, expected, "{}", spec.method);
        }
    }

    #[test]
    fn optional_limits_may_be_absent_but_not_null() {
        let cord_id = "11111111-1111-4111-8111-111111111111";
        let parsed: CordInviteCreateInput =
            parse_strict_params(json!({ "cord_id": cord_id })).expect("limits are optional");
        assert_eq!((parsed.expires_in_seconds, parsed.max_uses), (None, None));
        for field in ["expires_in_seconds", "max_uses"] {
            let parsed: Result<CordInviteCreateInput, _> =
                parse_strict_params(json!({ "cord_id": cord_id, field: null }));
            assert!(parsed.is_err(), "{field}: null is refused");
            let parsed: Result<CordInviteCreateInput, _> =
                parse_strict_params(json!({ "cord_id": cord_id, field: -1 }));
            assert!(parsed.is_err(), "{field}: negative is refused");
        }
    }

    #[test]
    fn limits_are_bounded() {
        let input = |expires_in_seconds, max_uses| CordInviteCreateInput {
            cord_id: Uuid::nil(),
            expires_in_seconds,
            max_uses,
            _acting: None,
        };
        assert!(validate_invite_limits(&input(None, None)).is_ok());
        assert!(
            validate_invite_limits(&input(
                Some(CORD_INVITE_EXPIRES_IN_SECONDS_MIN),
                Some(CORD_INVITE_MAX_USES_MAX)
            ))
            .is_ok()
        );
        assert!(
            validate_invite_limits(&input(Some(CORD_INVITE_EXPIRES_IN_SECONDS_MAX), Some(1)))
                .is_ok()
        );
        for (expires, uses) in [
            (Some(CORD_INVITE_EXPIRES_IN_SECONDS_MIN - 1), None),
            (Some(CORD_INVITE_EXPIRES_IN_SECONDS_MAX + 1), None),
            (None, Some(0)),
            (None, Some(CORD_INVITE_MAX_USES_MAX + 1)),
        ] {
            assert!(
                validate_invite_limits(&input(expires, uses)).is_err(),
                "{expires:?} {uses:?}"
            );
        }
    }

    #[test]
    fn invite_json_never_carries_a_token() {
        let row = CordInviteRow {
            id: Uuid::from_u128(1),
            cord_id: Uuid::from_u128(2),
            role: "cord_member".to_owned(),
            admits_newcomers: false,
            created_by: None,
            created_at: "2026-01-01T00:00:00Z".to_owned(),
            expires_at: None,
            max_uses: Some(3),
            use_count: 3,
            revoked_at: None,
            expired: false,
        };
        let json = to_cord_invite_json(&row);
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert!(!keys.iter().any(|k| k.contains("token")), "{keys:?}");
        assert_eq!(json["status"], json!("exhausted"));
    }
}

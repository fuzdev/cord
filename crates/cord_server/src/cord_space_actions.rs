//! Cord's space actions.
//!
//! - `cord_space_list`: any member. The cord's live spaces, oldest first.
//! - `cord_space_create`: the cord's owner or admins. Creates a space of a
//!   kind, under the cord's row lock and its cap on live spaces
//!   ([`crate::cord_space_queries::CORD_SPACE_COUNT_MAX`]), then audits
//!   `cord_space_create`.
//! - `cord_space_update`: the cord's owner or admins. Renames a space, then
//!   audits `cord_space_update`.
//! - `cord_space_delete`: the cord's owner or admins. Deletes a space, then
//!   audits `cord_space_delete`.
//!
//! Every verb names the cord, and a space by its cord and its id. Anyone
//! outside the cord gets `cord_not_found`, as for a cord that doesn't exist,
//! before any space is looked up; a member naming a space that isn't a live
//! space of that cord gets `cord_space_not_found`. The request's role-grant
//! snapshot answers `cord_not_found` early; the managing verbs then read the
//! caller's role afresh in their transaction ([`crate::cord_space_queries`]).
//! Instance admins get no implicit access.

use deadpool_postgres::GenericClient;
use fuz_actions::{ActionContext, ActionSpec};
use fuz_auth::{AuthSpec, CredentialGate, RequestActorContext};
use fuz_http::{
    InputShape, JsonrpcError, conflict, forbidden, internal_error_with_source,
    invalid_input_params, not_found, parse_strict_params, parse_uuid_param, reject_unknown_keys,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::cord_actions::{
    audit_cord_event, cord_not_found, dispatch_db, dispatch_db_ctx, handler, parse_name_param,
    require_actor,
};
use crate::cord_member_actions::require_member;
use crate::cord_space_queries::{
    CordSpaceError, CordSpaceKind, CordSpaceRow, query_cord_space_create, query_cord_space_delete,
    query_cord_space_rename, query_cord_spaces, validate_cord_space_name,
};

/// Error reason: no live space under the cord has that id.
pub const ERROR_CORD_SPACE_NOT_FOUND: &str = "cord_space_not_found";
/// Error reason: only the cord's owner or admins manage its spaces.
pub const ERROR_CORD_SPACE_FORBIDDEN: &str = "cord_space_forbidden";
/// Error reason: the space name is missing, not a string, empty, too long, or
/// has a forbidden or misplaced character — every refusal of `name` on
/// `cord_space_create` and `cord_space_update`.
pub const ERROR_CORD_SPACE_NAME_INVALID: &str = "cord_space_name_invalid";
/// Error reason: the cord already holds as many live spaces as it may.
pub const ERROR_CORD_SPACE_LIMIT_REACHED: &str = "cord_space_limit_reached";

// -- Wire inputs --------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordSpaceListInput {
    #[serde(deserialize_with = "fuz_http::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_http::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

/// The input of `cord_space_create`. Decoded by hand
/// ([`parse_cord_space_create_params`]), so every refusal of `name` can carry
/// its reason.
#[derive(Debug)]
struct CordSpaceCreateInput {
    cord_id: Uuid,
    kind: CordSpaceKind,
    /// Validated and trimmed
    /// ([`validate_cord_space_name`]).
    name: String,
}

/// The input of `cord_space_update`. Decoded by hand
/// ([`parse_cord_space_update_params`]), so every refusal of `name` can carry
/// its reason.
#[derive(Debug)]
struct CordSpaceUpdateInput {
    cord_id: Uuid,
    space_id: Uuid,
    /// Validated and trimmed
    /// ([`validate_cord_space_name`]).
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordSpaceDeleteInput {
    #[serde(deserialize_with = "fuz_http::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(deserialize_with = "fuz_http::deserialize_wire_uuid")]
    space_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_http::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

// -- Wire outputs -------------------------------------------------------------

/// A space as the cord's members see it: `CordSpaceJson` in
/// `cord_action_specs.ts`.
fn to_cord_space_json(space: &CordSpaceRow) -> Value {
    json!({
        "id": space.id,
        "cord_id": space.cord_id,
        "kind": space.kind,
        "name": space.name,
        "created_at": space.created_at,
    })
}

// -- Helpers ------------------------------------------------------------------

fn space_error(method: &str, e: CordSpaceError) -> JsonrpcError {
    match e {
        CordSpaceError::CordNotFound => cord_not_found(),
        CordSpaceError::Forbidden => forbidden(
            "only the cord's owner or admins manage its spaces",
            Some(ERROR_CORD_SPACE_FORBIDDEN),
        ),
        CordSpaceError::SpaceNotFound => not_found("cord space", Some(ERROR_CORD_SPACE_NOT_FOUND)),
        CordSpaceError::LimitReached => conflict(
            "the cord holds as many spaces as it may; delete one first",
            Some(ERROR_CORD_SPACE_LIMIT_REACHED),
        ),
        CordSpaceError::Db(e) => internal_error_with_source(method, &e),
    }
}

// -- Params validation --------------------------------------------------------
//
// `cord_space_create` and `cord_space_update` decode by hand, checking what
// the declared TS spec's schema refuses before the handler runs, in its issue
// order: an unknown key, then each field as the twin lists it. Every field
// but `name` is refused with no reason, as the twin declares none for them.
// `acting` is declared on both, but the dispatcher's authorization phase
// reads and checks it, not these parsers.

/// The keys of `CordSpaceCreateInput`'s TS twin.
const CORD_SPACE_CREATE_KEYS: &[&str] = &["cord_id", "kind", "name", "acting"];

/// The keys of `CordSpaceUpdateInput`'s TS twin.
const CORD_SPACE_UPDATE_KEYS: &[&str] = &["cord_id", "space_id", "name", "acting"];

fn parse_cord_space_create_params(params: &Value) -> Result<CordSpaceCreateInput, JsonrpcError> {
    reject_unknown_keys(params, CORD_SPACE_CREATE_KEYS)?;
    let cord_id = parse_uuid_param(params, "cord_id")?;
    let kind = params
        .get("kind")
        .cloned()
        .and_then(|kind| serde_json::from_value::<CordSpaceKind>(kind).ok())
        .ok_or_else(|| invalid_input_params("'kind' is not a space kind", None))?;
    let name = parse_cord_space_name_param(params)?;
    Ok(CordSpaceCreateInput {
        cord_id,
        kind,
        name,
    })
}

fn parse_cord_space_update_params(params: &Value) -> Result<CordSpaceUpdateInput, JsonrpcError> {
    reject_unknown_keys(params, CORD_SPACE_UPDATE_KEYS)?;
    let cord_id = parse_uuid_param(params, "cord_id")?;
    let space_id = parse_uuid_param(params, "space_id")?;
    let name = parse_cord_space_name_param(params)?;
    Ok(CordSpaceUpdateInput {
        cord_id,
        space_id,
        name,
    })
}

/// Read `name` as the twin's `CordSpaceName`, returning it trimmed. Every
/// refusal carries [`ERROR_CORD_SPACE_NAME_INVALID`].
fn parse_cord_space_name_param(params: &Value) -> Result<String, JsonrpcError> {
    parse_name_param(
        params,
        validate_cord_space_name,
        ERROR_CORD_SPACE_NAME_INVALID,
    )
}

// -- Handlers -----------------------------------------------------------------

async fn handle_cord_space_list<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
    input: &CordSpaceListInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let spaces = query_cord_spaces(client, input.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_space_list", &e))?;
    let spaces: Vec<Value> = spaces.iter().map(to_cord_space_json).collect();
    Ok(json!({ "spaces": spaces }))
}

async fn handle_cord_space_create<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordSpaceCreateInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let space = query_cord_space_create(
        client,
        input.cord_id,
        auth.actor.id,
        input.kind,
        &input.name,
    )
    .await
    .map_err(|e| space_error("cord_space_create", e))?;
    // the cell has no `created_by`, so the audit row is the creator's record
    audit_cord_event(
        client,
        ctx,
        Some(auth),
        "cord_space_create",
        json!({
            "cord_id": space.cord_id,
            "space_id": space.id,
            "kind": space.kind,
            "name": space.name,
        }),
    )
    .await?;
    Ok(json!({ "space": to_cord_space_json(&space) }))
}

async fn handle_cord_space_update<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordSpaceUpdateInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let space = query_cord_space_rename(
        client,
        input.cord_id,
        input.space_id,
        auth.actor.id,
        &input.name,
    )
    .await
    .map_err(|e| space_error("cord_space_update", e))?;
    audit_cord_event(
        client,
        ctx,
        Some(auth),
        "cord_space_update",
        json!({ "cord_id": space.cord_id, "space_id": space.id, "name": space.name }),
    )
    .await?;
    Ok(json!({ "space": to_cord_space_json(&space) }))
}

async fn handle_cord_space_delete<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordSpaceDeleteInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let space = query_cord_space_delete(client, input.cord_id, input.space_id, auth.actor.id)
        .await
        .map_err(|e| space_error("cord_space_delete", e))?;
    audit_cord_event(
        client,
        ctx,
        Some(auth),
        "cord_space_delete",
        json!({ "cord_id": space.cord_id, "space_id": space.id, "name": space.name }),
    )
    .await?;
    Ok(json!({ "ok": true }))
}

// -- Specs --------------------------------------------------------------------

/// The space action specs.
pub(crate) fn cord_space_action_specs() -> Vec<ActionSpec> {
    vec![
        ActionSpec::read_only(
            "cord_space_list",
            AuthSpec::authenticated(CredentialGate::Any),
            InputShape::Object,
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordSpaceListInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_space_list(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_space_create",
            AuthSpec::authenticated(CredentialGate::Any),
            InputShape::Object,
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input = parse_cord_space_create_params(&params)?;
                    dispatch_db_ctx!(ctx, handle_cord_space_create(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_space_update",
            AuthSpec::authenticated(CredentialGate::Any),
            InputShape::Object,
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input = parse_cord_space_update_params(&params)?;
                    dispatch_db_ctx!(ctx, handle_cord_space_update(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_space_delete",
            AuthSpec::authenticated(CredentialGate::Any),
            InputShape::Object,
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordSpaceDeleteInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_space_delete(auth, &input))
                })
            }),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cord_space_queries::CORD_SPACE_NAME_LENGTH_MAX;
    use fuz_http::{INVALID_PARAMS_MESSAGE, JsonrpcErrorCode};

    const CORD_ID: &str = "11111111-1111-4111-8111-111111111111";
    const SPACE_ID: &str = "33333333-3333-4333-8333-333333333333";

    #[test]
    fn specs_declare_their_auth_and_effects() {
        let specs = cord_space_action_specs();
        for spec in &specs {
            assert_eq!(spec.auth.roles, None, "{}", spec.method);
            assert!(spec.auth.needs_actor(), "{}", spec.method);
            assert_eq!(
                spec.side_effects,
                spec.method != "cord_space_list",
                "{}",
                spec.method
            );
        }
        let methods: Vec<&str> = specs.iter().map(|s| s.method).collect();
        assert_eq!(
            methods,
            [
                "cord_space_list",
                "cord_space_create",
                "cord_space_update",
                "cord_space_delete"
            ]
        );
    }

    fn assert_refused(
        result: Result<impl std::fmt::Debug, JsonrpcError>,
        reason: Option<&str>,
        case: &str,
    ) {
        let err = result.expect_err(case);
        assert_eq!(err.code, JsonrpcErrorCode::InvalidParams, "{case}");
        assert_eq!(err.message, INVALID_PARAMS_MESSAGE, "{case}");
        assert_eq!(
            err.data,
            reason.map(|reason| json!({ "reason": reason })),
            "{case}"
        );
    }

    /// Every refusal of `name` on both verbs that take one: missing, `null`,
    /// a non-string, and each rule of `validate_cord_space_name`.
    #[test]
    fn every_name_refusal_carries_its_reason() {
        let too_long = "x".repeat(CORD_SPACE_NAME_LENGTH_MAX + 1);
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
        for (case, name) in cases {
            let mut create = json!({ "cord_id": CORD_ID, "kind": "line" });
            let mut update = json!({ "cord_id": CORD_ID, "space_id": SPACE_ID });
            if let Some(name) = name {
                create["name"] = name.clone();
                update["name"] = name;
            }
            let reason = Some(ERROR_CORD_SPACE_NAME_INVALID);
            assert_refused(parse_cord_space_create_params(&create), reason, case);
            assert_refused(parse_cord_space_update_params(&update), reason, case);
        }
    }

    #[test]
    fn a_valid_input_parses_trimmed() {
        let parsed = parse_cord_space_create_params(&json!({
            "cord_id": CORD_ID,
            "kind": "line",
            "name": "  the den  ",
            "acting": "22222222-2222-4222-9222-222222222222",
        }))
        .expect("parses");
        assert_eq!(parsed.kind, CordSpaceKind::Line);
        assert_eq!(parsed.name, "the den");
        let parsed = parse_cord_space_update_params(&json!({
            "cord_id": CORD_ID,
            "space_id": SPACE_ID,
            "name": "the nook ",
        }))
        .expect("parses");
        assert_eq!(
            (parsed.cord_id, parsed.space_id, parsed.name.as_str()),
            (
                Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111),
                Uuid::from_u128(0x3333_3333_3333_4333_8333_3333_3333_3333),
                "the nook"
            )
        );
    }

    /// An unknown key, or a bad field ahead of `name` in the twin's order, is
    /// refused with no reason, whatever the name.
    #[test]
    fn fields_before_the_name_refuse_with_no_reason() {
        for params in [
            json!({ "cord_id": CORD_ID, "kind": "line", "name": "x", "extra": 1 }),
            json!({ "cord_id": CORD_ID, "kind": "line", "name": "", "extra": 1 }),
            json!({ "kind": "line", "name": "x" }),
            json!({ "cord_id": "not-a-uuid", "kind": "line", "name": "" }),
            json!({ "cord_id": CORD_ID, "name": "x" }),
            json!({ "cord_id": CORD_ID, "kind": null, "name": "x" }),
            json!({ "cord_id": CORD_ID, "kind": "chat", "name": "x" }),
            json!({ "cord_id": CORD_ID, "kind": "Line", "name": "" }),
            json!({ "cord_id": CORD_ID, "kind": 1, "name": "x" }),
        ] {
            assert_refused(
                parse_cord_space_create_params(&params),
                None,
                &params.to_string(),
            );
        }
        for params in [
            json!({ "cord_id": CORD_ID, "space_id": SPACE_ID, "name": "x", "extra": 1 }),
            json!({ "cord_id": CORD_ID, "name": "x" }),
            json!({ "cord_id": CORD_ID, "space_id": null, "name": "" }),
            json!({ "cord_id": "not-a-uuid", "space_id": SPACE_ID, "name": "" }),
        ] {
            assert_refused(
                parse_cord_space_update_params(&params),
                None,
                &params.to_string(),
            );
        }
    }

    #[test]
    fn list_and_delete_inputs_are_strict() {
        let parsed: CordSpaceDeleteInput =
            parse_strict_params(json!({ "cord_id": CORD_ID, "space_id": SPACE_ID })).unwrap();
        assert_eq!(
            parsed.space_id,
            Uuid::from_u128(0x3333_3333_3333_4333_8333_3333_3333_3333)
        );
        for params in [
            json!({ "cord_id": CORD_ID }),
            json!({ "cord_id": CORD_ID, "space_id": SPACE_ID, "extra": 1 }),
        ] {
            assert!(parse_strict_params::<CordSpaceDeleteInput>(params).is_err());
        }
        let _: CordSpaceListInput = parse_strict_params(json!({ "cord_id": CORD_ID })).unwrap();
        assert!(parse_strict_params::<CordSpaceListInput>(json!({})).is_err());
    }

    #[test]
    fn errors_map_to_their_reasons() {
        let reason = |e| {
            space_error("m", e)
                .data
                .and_then(|d| d.get("reason").cloned())
        };
        assert_eq!(
            reason(CordSpaceError::CordNotFound),
            Some(json!("cord_not_found"))
        );
        assert_eq!(
            reason(CordSpaceError::Forbidden),
            Some(json!(ERROR_CORD_SPACE_FORBIDDEN))
        );
        assert_eq!(
            reason(CordSpaceError::SpaceNotFound),
            Some(json!(ERROR_CORD_SPACE_NOT_FOUND))
        );
        assert_eq!(
            reason(CordSpaceError::LimitReached),
            Some(json!(ERROR_CORD_SPACE_LIMIT_REACHED))
        );
    }

    #[test]
    fn space_json_carries_its_cord_and_kind() {
        let space = CordSpaceRow {
            id: Uuid::from_u128(1),
            cord_id: Uuid::from_u128(2),
            kind: CordSpaceKind::Line,
            name: "the den".to_owned(),
            created_at: "2026-01-01T00:00:00Z".to_owned(),
        };
        assert_eq!(
            to_cord_space_json(&space),
            json!({
                "id": Uuid::from_u128(1),
                "cord_id": Uuid::from_u128(2),
                "kind": "line",
                "name": "the den",
                "created_at": "2026-01-01T00:00:00Z",
            })
        );
    }
}

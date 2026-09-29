//! Cord's member actions.
//!
//! - `cord_member_list`: any member. Every member's actor id, username,
//!   display name, and role, owners first.
//! - `cord_member_role_set`: owners only. Sets a member (the caller
//!   included) to owner, admin, or member, revoking the old grant and
//!   creating the new one in one transaction. A cord may have several
//!   owners, but never none.
//! - `cord_member_remove`: an owner removes anyone else, an admin removes
//!   plain members; the last owner can't be removed. Nobody removes
//!   themself (`cord_member_remove_self`); they leave instead, so the audit
//!   records a leave as a leave.
//! - `cord_leave`: any member, except the last owner.
//!
//! Each change is audited as the spine's `role_grant_revoke` (and, for a
//! role change, `role_grant_create`) with the cord as the scope. The rules
//! apply to membership read inside the transaction under the cord's row
//! lock ([`crate::cord_member_queries`]); the request's role-grant snapshot
//! only answers `cord_not_found` early to anyone outside the cord.

use deadpool_postgres::GenericClient;
use fuz_actions::{ActionContext, ActionSpec};
use fuz_auth::{AuditEventType, AuthSpec, CredentialGate, RequestActorContext};
use fuz_http::{
    JsonrpcError, conflict, forbidden, internal_error_with_source, invalid_params, not_found,
    parse_strict_params,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

use crate::cord_actions::{
    CordGrantAudit, audit_cord_grant_event, cord_not_found, dispatch_db, dispatch_db_ctx, handler,
    require_actor,
};
use crate::cord_authz::{CordRole, is_cord_member};
use crate::cord_member_queries::{
    CORD_REVOKE_REASON_LEAVE, CORD_REVOKE_REASON_REMOVE, CORD_REVOKE_REASON_ROLE_SET, CordMember,
    CordMemberChange, CordMemberError, query_cord_leave, query_cord_member_remove,
    query_cord_member_role_set, query_cord_members,
};

/// Error reason: the target isn't a member of the cord.
pub const ERROR_CORD_MEMBER_NOT_FOUND: &str = "cord_member_not_found";
/// Error reason: the caller's cord role can't make this change.
pub const ERROR_CORD_MEMBER_FORBIDDEN: &str = "cord_member_forbidden";
/// Error reason: the change would leave the cord with no owner.
pub const ERROR_CORD_LAST_OWNER: &str = "cord_last_owner";
/// Error reason: `cord_member_remove` named the caller; `cord_leave` is how
/// a member takes themself out.
pub const ERROR_CORD_MEMBER_REMOVE_SELF: &str = "cord_member_remove_self";

// -- Wire inputs --------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordMemberListInput {
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
struct CordMemberRoleSetInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    actor_id: Uuid,
    role: CordRole,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordMemberRemoveInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    actor_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct CordLeaveInput {
    #[serde(deserialize_with = "fuz_auth::deserialize_wire_uuid")]
    cord_id: Uuid,
    #[serde(
        default,
        rename = "acting",
        deserialize_with = "fuz_auth::deserialize_optional_wire_uuid"
    )]
    _acting: Option<Uuid>,
}

// -- Wire outputs -------------------------------------------------------------

/// A member as the cord's members see them: `CordMemberJson` in
/// `cord_action_specs.ts`. No account id, email, or timestamps.
fn to_cord_member_json(member: &CordMember, role: CordRole) -> Value {
    json!({
        "actor_id": member.actor_id,
        "username": member.username,
        "display_name": member.display_name,
        "role": role,
    })
}

// -- Helpers ------------------------------------------------------------------

/// The snapshot check every member verb starts with: anyone outside the cord
/// gets `cord_not_found`, as for a cord that doesn't exist.
fn require_member(auth: &RequestActorContext, cord_id: Uuid) -> Result<(), JsonrpcError> {
    if is_cord_member(&auth.role_grants, cord_id) {
        Ok(())
    } else {
        Err(cord_not_found())
    }
}

fn member_error(method: &str, e: CordMemberError) -> JsonrpcError {
    match e {
        CordMemberError::CordNotFound => cord_not_found(),
        CordMemberError::Forbidden => forbidden(
            "your role in this cord can't make that change",
            Some(ERROR_CORD_MEMBER_FORBIDDEN),
        ),
        CordMemberError::MemberNotFound => {
            not_found("cord member", Some(ERROR_CORD_MEMBER_NOT_FOUND))
        }
        CordMemberError::LastOwner => conflict(
            "a cord needs an owner; make someone else an owner first",
            Some(ERROR_CORD_LAST_OWNER),
        ),
        CordMemberError::SelfRemove => invalid_params(
            "you can't remove yourself from a cord; use cord_leave",
            Some(ERROR_CORD_MEMBER_REMOVE_SELF),
        ),
        CordMemberError::Db(e) => internal_error_with_source(method, &e),
    }
}

/// Audit a change as the spine's grant events: one `role_grant_revoke` per
/// revoked grant, then `role_grant_create` for a new one.
async fn audit_member_change<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    cord_id: Uuid,
    change: &CordMemberChange,
    reason: &str,
) -> Result<(), JsonrpcError> {
    let grant = |role_grant_id, role| CordGrantAudit {
        target_account_id: change.member.account_id,
        target_actor_id: change.member.actor_id,
        cord_id,
        role,
        role_grant_id,
    };
    for revoked in &change.revoked {
        audit_cord_grant_event(
            client,
            ctx,
            auth,
            AuditEventType::RoleGrantRevoke,
            &grant(revoked.role_grant_id, revoked.role),
            Some(("reason", json!(reason))),
        )
        .await?;
    }
    if let Some((role_grant_id, role)) = change.granted {
        audit_cord_grant_event(
            client,
            ctx,
            auth,
            AuditEventType::RoleGrantCreate,
            &grant(role_grant_id, role),
            None,
        )
        .await?;
    }
    Ok(())
}

// -- Handlers -----------------------------------------------------------------

async fn handle_cord_member_list<C: GenericClient + ?Sized>(
    client: &C,
    auth: &RequestActorContext,
    input: &CordMemberListInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let members = query_cord_members(client, input.cord_id)
        .await
        .map_err(|e| internal_error_with_source("cord_member_list", &e))?;
    let members: Vec<Value> = members
        .iter()
        .map(|m| to_cord_member_json(m, m.role))
        .collect();
    Ok(json!({ "members": members }))
}

async fn handle_cord_member_role_set<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordMemberRoleSetInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let change = query_cord_member_role_set(
        client,
        input.cord_id,
        auth.actor.id,
        input.actor_id,
        input.role,
    )
    .await
    .map_err(|e| member_error("cord_member_role_set", e))?;
    audit_member_change(
        client,
        ctx,
        auth,
        input.cord_id,
        &change,
        CORD_REVOKE_REASON_ROLE_SET,
    )
    .await?;
    Ok(json!({ "member": to_cord_member_json(&change.member, input.role) }))
}

async fn handle_cord_member_remove<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordMemberRemoveInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let change = query_cord_member_remove(client, input.cord_id, auth.actor.id, input.actor_id)
        .await
        .map_err(|e| member_error("cord_member_remove", e))?;
    audit_member_change(
        client,
        ctx,
        auth,
        input.cord_id,
        &change,
        CORD_REVOKE_REASON_REMOVE,
    )
    .await?;
    Ok(json!({ "ok": true }))
}

async fn handle_cord_leave<C: GenericClient + ?Sized>(
    client: &C,
    ctx: &ActionContext<'_>,
    auth: &RequestActorContext,
    input: &CordLeaveInput,
) -> Result<Value, JsonrpcError> {
    require_member(auth, input.cord_id)?;
    let change = query_cord_leave(client, input.cord_id, auth.actor.id)
        .await
        .map_err(|e| member_error("cord_leave", e))?;
    audit_member_change(
        client,
        ctx,
        auth,
        input.cord_id,
        &change,
        CORD_REVOKE_REASON_LEAVE,
    )
    .await?;
    Ok(json!({ "ok": true }))
}

// -- Specs --------------------------------------------------------------------

/// The member action specs.
pub(crate) fn cord_member_action_specs() -> Vec<ActionSpec> {
    vec![
        ActionSpec::read_only(
            "cord_member_list",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordMemberListInput = parse_strict_params(params)?;
                    dispatch_db!(ctx, handle_cord_member_list(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_member_role_set",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordMemberRoleSetInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_member_role_set(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_member_remove",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordMemberRemoveInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_member_remove(auth, &input))
                })
            }),
        ),
        ActionSpec::with_side_effects(
            "cord_leave",
            AuthSpec::authenticated(CredentialGate::Any),
            handler(|params, ctx| {
                Box::pin(async move {
                    let auth = require_actor(ctx.auth)?;
                    let input: CordLeaveInput = parse_strict_params(params)?;
                    dispatch_db_ctx!(ctx, handle_cord_leave(auth, &input))
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
        let specs = cord_member_action_specs();
        for spec in &specs {
            assert_eq!(spec.auth.roles, None, "{}", spec.method);
            assert!(spec.auth.needs_actor(), "{}", spec.method);
            assert_eq!(
                spec.side_effects,
                spec.method != "cord_member_list",
                "{}",
                spec.method
            );
        }
        let methods: Vec<&str> = specs.iter().map(|s| s.method).collect();
        assert_eq!(
            methods,
            [
                "cord_member_list",
                "cord_member_role_set",
                "cord_member_remove",
                "cord_leave"
            ]
        );
    }

    #[test]
    fn role_set_takes_only_cord_roles() {
        let base = json!({
            "cord_id": "11111111-1111-4111-8111-111111111111",
            "actor_id": "22222222-2222-4222-9222-222222222222",
        });
        let with_role = |role: &str| {
            let mut v = base.clone();
            v["role"] = json!(role);
            parse_strict_params::<CordMemberRoleSetInput>(v)
        };
        assert_eq!(with_role("cord_owner").unwrap().role, CordRole::Owner);
        assert_eq!(with_role("cord_member").unwrap().role, CordRole::Member);
        for role in ["admin", "keeper", "owner", ""] {
            assert!(with_role(role).is_err(), "{role}");
        }
    }

    #[test]
    fn member_json_carries_no_account_id() {
        let member = CordMember {
            actor_id: Uuid::from_u128(1),
            account_id: Uuid::from_u128(2),
            username: "amy".to_owned(),
            display_name: None,
            role: CordRole::Member,
            grants: Vec::new(),
        };
        assert_eq!(
            to_cord_member_json(&member, CordRole::Admin),
            json!({
                "actor_id": Uuid::from_u128(1),
                "username": "amy",
                "display_name": null,
                "role": "cord_admin",
            })
        );
    }

    #[test]
    fn errors_map_to_their_reasons() {
        let reason = |e| {
            member_error("m", e)
                .data
                .and_then(|d| d.get("reason").cloned())
        };
        assert_eq!(
            reason(CordMemberError::CordNotFound),
            Some(json!("cord_not_found"))
        );
        assert_eq!(
            reason(CordMemberError::Forbidden),
            Some(json!(ERROR_CORD_MEMBER_FORBIDDEN))
        );
        assert_eq!(
            reason(CordMemberError::MemberNotFound),
            Some(json!(ERROR_CORD_MEMBER_NOT_FOUND))
        );
        assert_eq!(
            reason(CordMemberError::LastOwner),
            Some(json!(ERROR_CORD_LAST_OWNER))
        );
        assert_eq!(
            reason(CordMemberError::SelfRemove),
            Some(json!(ERROR_CORD_MEMBER_REMOVE_SELF))
        );
    }
}

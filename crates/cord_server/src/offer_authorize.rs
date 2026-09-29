//! [`CordOfferAuthorize`], cord's gate on `role_grant_offer_create`.
//!
//! The spine's default gate ([`fuz_auth::AdminOfferAuthorize`]) admits any
//! offer from a global admin, whatever its scope. An offer of the builtin
//! `admin` role with a scope would mint a *scoped* admin, and the spine
//! treats that as more than it is: its dispatcher role gate and several
//! handlers check `admin` without regard to scope, so a scoped admin passes
//! every spine admin action, and can `role_grant_assign` itself a global
//! `admin`. Cord has no use for a scoped builtin role, so it refuses them at
//! the only path that can create one.

use std::future::Future;
use std::pin::Pin;

use fuz_auth::{
    OfferCreateAuthorize, OfferCreateAuthorizeInput, ROLE_ADMIN, RequestActorContext,
    is_builtin_role,
};
use fuz_http::JsonrpcError;

/// Cord's offer gate: the caller holds a global `admin` grant (the spine
/// default), and a builtin role (`admin`, `keeper`) is offered only
/// globally, never with a scope.
#[derive(Debug, Clone, Copy, Default)]
pub struct CordOfferAuthorize;

impl CordOfferAuthorize {
    /// The decision, as a pure function of the caller's grants and the offer.
    #[must_use]
    pub fn admits(auth: &RequestActorContext, input: &OfferCreateAuthorizeInput) -> bool {
        let caller_is_instance_admin = auth.has_scoped_role(ROLE_ADMIN, None);
        let scoped = input.scope_id.is_some() || input.scope_kind.is_some();
        caller_is_instance_admin && !(scoped && is_builtin_role(&input.role))
    }
}

impl OfferCreateAuthorize for CordOfferAuthorize {
    fn authorize<'a>(
        &'a self,
        auth: &'a RequestActorContext,
        input: &'a OfferCreateAuthorizeInput,
    ) -> Pin<Box<dyn Future<Output = Result<bool, JsonrpcError>> + Send + 'a>> {
        let admitted = Self::admits(auth, input);
        Box::pin(async move { Ok(admitted) })
    }
}

#[cfg(test)]
mod tests {
    use fuz_auth::{AccountRow, ActorRow, ROLE_KEEPER, RoleGrantRow};
    use uuid::Uuid;

    use super::*;

    fn caller(grants: &[(&str, Option<Uuid>)]) -> RequestActorContext {
        RequestActorContext {
            account: AccountRow {
                id: Uuid::from_u128(1),
                username: "caller".to_owned(),
            },
            actor: ActorRow {
                id: Uuid::from_u128(2),
                account_id: Uuid::from_u128(1),
                name: "caller".to_owned(),
            },
            role_grants: grants
                .iter()
                .map(|(role, scope_id)| RoleGrantRow {
                    id: Uuid::nil(),
                    actor_id: Uuid::from_u128(2),
                    role: (*role).to_owned(),
                    scope_id: *scope_id,
                })
                .collect(),
        }
    }

    fn offer(role: &str, scope_id: Option<Uuid>) -> OfferCreateAuthorizeInput {
        OfferCreateAuthorizeInput {
            role: role.to_owned(),
            scope_kind: scope_id.map(|_| "cord".to_owned()),
            scope_id,
            to_account_id: Uuid::from_u128(3),
        }
    }

    const SCOPE: Uuid = Uuid::from_u128(0xc);

    #[test]
    fn a_global_admin_offers_admin_globally() {
        let admin = caller(&[(ROLE_ADMIN, None)]);
        assert!(CordOfferAuthorize::admits(&admin, &offer(ROLE_ADMIN, None)));
    }

    #[test]
    fn builtin_roles_are_never_offered_with_a_scope() {
        let admin = caller(&[(ROLE_ADMIN, None)]);
        for role in [ROLE_ADMIN, ROLE_KEEPER] {
            assert!(!CordOfferAuthorize::admits(
                &admin,
                &offer(role, Some(SCOPE))
            ));
        }
        // a scope kind alone still counts as scoped
        let kind_only = OfferCreateAuthorizeInput {
            scope_kind: Some("cord".to_owned()),
            ..offer(ROLE_ADMIN, None)
        };
        assert!(!CordOfferAuthorize::admits(&admin, &kind_only));
    }

    #[test]
    fn only_a_global_admin_offers_at_all() {
        for grants in [
            vec![],
            vec![(ROLE_ADMIN, Some(SCOPE))],
            vec![("cord_owner", Some(SCOPE))],
        ] {
            let not_admin = caller(&grants);
            assert!(!CordOfferAuthorize::admits(
                &not_admin,
                &offer(ROLE_ADMIN, None)
            ));
        }
    }

    #[test]
    fn a_non_builtin_role_may_carry_a_scope() {
        // no cord role is offerable (none has a grant path), but the gate
        // itself only restricts the builtins
        let admin = caller(&[(ROLE_ADMIN, None)]);
        assert!(CordOfferAuthorize::admits(
            &admin,
            &offer("some_app_role", Some(SCOPE))
        ));
    }
}

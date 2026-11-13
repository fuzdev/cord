//! Cord's roles: the three cord-scoped membership roles and the
//! [`RoleRegistry`] that declares them to the spine.
//!
//! Membership in a cord is a spine `role_grant` whose `scope_kind` is
//! [`CORD_SCOPE_KIND`] and whose `scope_id` is the cord's cell id. An actor
//! holds at most one of these roles per cord; Cord's verbs enforce that.
//!
//! The roles are registered with no grant paths, so the spine's generic
//! conferral surfaces (the admin "+role" UI, `role_grant_assign`, and the
//! role-grant offer flow) refuse them. Only Cord's own verbs confer them,
//! through the spine's role-grant creation query.

use fuz_auth::{RoleRegistry, RoleSpec};

/// The cord's owner: every power over the cord, including handing it on.
pub const CORD_OWNER: &str = "cord_owner";

/// A cord admin: manages the cord and its members, under the owner.
pub const CORD_ADMIN: &str = "cord_admin";

/// A cord member: sees and takes part in the cord.
pub const CORD_MEMBER: &str = "cord_member";

/// Every cord-scoped role, highest authority first.
pub const CORD_ROLES: [&str; 3] = [CORD_OWNER, CORD_ADMIN, CORD_MEMBER];

/// The `role_grant.scope_kind` of every cord-scoped grant. The spine's
/// `role_grant_scope_kind_paired` check requires a kind beside every
/// `scope_id`.
pub const CORD_SCOPE_KIND: &str = "cord";

/// Cord's role registry: the spine builtins plus the three cord roles, none
/// of them conferrable through a spine grant path.
///
/// # Panics
///
/// Never in practice: the cord role names are statically valid and distinct
/// from the builtins, so [`RoleRegistry::new`] can't fail here.
#[must_use]
#[expect(
    clippy::expect_used,
    reason = "the cord role names are statically valid and builtin-distinct"
)]
pub fn cord_role_registry() -> RoleRegistry {
    RoleRegistry::new(
        CORD_ROLES
            .iter()
            .map(|role| RoleSpec::new(*role, Vec::new()))
            .collect(),
    )
    .expect("the cord role names are statically valid and builtin-distinct")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_knows_the_cord_roles_but_grants_none_of_them() {
        let registry = cord_role_registry();
        let names = registry.role_names();
        for role in CORD_ROLES {
            assert!(names.iter().any(|n| n == role), "{role} is registered");
            assert!(
                !registry.is_admin_grantable(role),
                "{role} must not be admin-grantable"
            );
        }
        // the builtins keep their paths
        assert_eq!(registry.list_admin_grantable(), vec!["admin".to_owned()]);
    }
}

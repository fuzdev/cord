//! Cord authorization: pure predicates over an actor's active role grants.
//!
//! Every check here matches the grant's scope **exactly**: a cord role counts
//! only when its `scope_id` is the cord's id. The spine's
//! `RequestActorContext::has_role` ignores scope, so it would let a
//! `cord_owner` grant on one cord stand in for any other; never use it for
//! cord roles. The grants passed in are the request's role-grant snapshot
//! (already filtered to active grants), so these predicates need no database.
//!
//! An actor holds at most one cord role per cord. The spine can't enforce
//! that: its unique index is per (actor, role, scope), so `cord_member` and
//! `cord_admin` on one cord can coexist. Cord's verbs keep the rule
//! themselves: `cord_create` confers the owner role on a cord that didn't
//! exist, a role change ([`crate::cord_member_queries`]) revokes the old
//! grant and creates the new one within one transaction, and accepting an
//! invite ([`crate::invite::queries`]) never adds a grant beside an
//! existing cord role on the same cord. [`cord_role_in`] still resolves a
//! pair to the higher role, as a backstop.
//!
//! The member rules are pure predicates here too: who may change roles
//! ([`CordRole::can_set_member_roles`]), who may remove whom
//! ([`CordRole::can_remove`]), and the last-owner rule
//! ([`leaves_no_owner`]). The verbs apply them to membership read under the
//! cord's row lock, not to the request's snapshot, so a concurrent change
//! can't slip past them.
//!
//! Instance admins get no implicit access to cords here. They create cords
//! and see the admin ops list, but a cord they aren't in stays hidden from
//! them in the normal surface.

use fuz_auth::RoleGrantRow;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::roles::{CORD_ADMIN, CORD_MEMBER, CORD_OWNER};

/// An actor's role in one cord, highest authority first.
///
/// Serializes to (and deserializes from) its `role_grant.role` string. The
/// variant order is the authority order, so `min` picks the highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum CordRole {
    #[serde(rename = "cord_owner")]
    Owner,
    #[serde(rename = "cord_admin")]
    Admin,
    #[serde(rename = "cord_member")]
    Member,
}

impl CordRole {
    /// The role's `role_grant.role` string.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Owner => CORD_OWNER,
            Self::Admin => CORD_ADMIN,
            Self::Member => CORD_MEMBER,
        }
    }

    /// The cord role a `role_grant.role` string names, or `None` for any
    /// other role.
    #[must_use]
    pub fn from_role(role: &str) -> Option<Self> {
        match role {
            CORD_OWNER => Some(Self::Owner),
            CORD_ADMIN => Some(Self::Admin),
            CORD_MEMBER => Some(Self::Member),
            _ => None,
        }
    }

    /// Whether the role may manage the cord (rename it, manage its invite
    /// links, remove plain members, and later manage its spaces): the owner
    /// and admins.
    #[must_use]
    pub const fn can_manage(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }

    /// Whether the role may set members' roles (to owner, admin, or member):
    /// owners only.
    #[must_use]
    pub const fn can_set_member_roles(self) -> bool {
        matches!(self, Self::Owner)
    }

    /// Whether this role may remove a member holding `target`: an owner
    /// removes anyone, an admin removes plain members only, and a member
    /// removes no one. The last-owner rule ([`leaves_no_owner`]) applies on
    /// top.
    #[must_use]
    pub const fn can_remove(self, target: Self) -> bool {
        match self {
            Self::Owner => true,
            Self::Admin => matches!(target, Self::Member),
            Self::Member => false,
        }
    }
}

/// Whether moving one member from `current` to `next` (`None` when they
/// leave or are removed) would leave the cord with no owner, given
/// `owner_count` owners before the change.
///
/// A cord always keeps an owner: the last one can't leave, be removed, or
/// step down. Several owners are fine, so an owner may step down or leave
/// while another remains.
#[must_use]
pub fn leaves_no_owner(owner_count: usize, current: CordRole, next: Option<CordRole>) -> bool {
    current == CordRole::Owner && next != Some(CordRole::Owner) && owner_count <= 1
}

/// The actor's role in the cord `cord_id`, from grants scoped exactly to it.
///
/// Cord's verbs keep one role per (actor, cord); should two ever coexist, the
/// higher authority wins, so a stray lower grant never demotes an owner.
#[must_use]
pub fn cord_role_in(grants: &[RoleGrantRow], cord_id: Uuid) -> Option<CordRole> {
    grants
        .iter()
        .filter(|g| g.scope_id == Some(cord_id))
        .filter_map(|g| CordRole::from_role(&g.role))
        .min()
}

/// Whether the actor is in the cord `cord_id` at all (owner, admin, or
/// member).
#[must_use]
pub fn is_cord_member(grants: &[RoleGrantRow], cord_id: Uuid) -> bool {
    cord_role_in(grants, cord_id).is_some()
}

/// Every cord the actor is in, with its role there, in the order the grants
/// arrive (the spine loads them oldest first). A cord role with no scope is
/// malformed for Cord and is skipped.
#[must_use]
pub fn cord_memberships(grants: &[RoleGrantRow]) -> Vec<(Uuid, CordRole)> {
    let mut out: Vec<(Uuid, CordRole)> = Vec::new();
    for grant in grants {
        let (Some(cord_id), Some(role)) = (grant.scope_id, CordRole::from_role(&grant.role)) else {
            continue;
        };
        match out.iter_mut().find(|(id, _)| *id == cord_id) {
            Some(entry) => entry.1 = entry.1.min(role),
            None => out.push((cord_id, role)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const CORD_A: Uuid = Uuid::from_u128(0xa);
    const CORD_B: Uuid = Uuid::from_u128(0xb);

    // the predicates read only `role` and `scope_id`
    fn grant(role: &str, scope_id: Option<Uuid>) -> RoleGrantRow {
        RoleGrantRow {
            id: Uuid::nil(),
            actor_id: Uuid::from_u128(1),
            role: role.to_owned(),
            scope_id,
        }
    }

    #[test]
    fn role_strings_round_trip() {
        for role in [CordRole::Owner, CordRole::Admin, CordRole::Member] {
            assert_eq!(CordRole::from_role(role.as_str()), Some(role));
            assert_eq!(
                serde_json::to_value(role).unwrap(),
                serde_json::json!(role.as_str())
            );
        }
        assert_eq!(CordRole::from_role("admin"), None);
        assert_eq!(CordRole::from_role("keeper"), None);
    }

    #[test]
    fn scope_must_match_exactly() {
        let grants = [grant(CORD_OWNER, Some(CORD_A))];
        assert_eq!(cord_role_in(&grants, CORD_A), Some(CordRole::Owner));
        // owning one cord confers nothing in another
        assert_eq!(cord_role_in(&grants, CORD_B), None);
        assert!(!is_cord_member(&grants, CORD_B));
    }

    #[test]
    fn an_unscoped_cord_role_confers_nothing() {
        // `has_role` would count this for every cord; exact scope counts it
        // for none
        let grants = [grant(CORD_OWNER, None)];
        assert_eq!(cord_role_in(&grants, CORD_A), None);
        assert!(cord_memberships(&grants).is_empty());
    }

    #[test]
    fn instance_admin_is_not_a_cord_member() {
        let grants = [grant("admin", None), grant("keeper", None)];
        assert!(!is_cord_member(&grants, CORD_A));
        assert!(cord_memberships(&grants).is_empty());
    }

    #[test]
    fn other_roles_scoped_to_the_cord_confer_nothing() {
        let grants = [grant("admin", Some(CORD_A))];
        assert!(!is_cord_member(&grants, CORD_A));
    }

    #[test]
    fn management_is_owner_or_admin() {
        assert!(CordRole::Owner.can_manage());
        assert!(CordRole::Admin.can_manage());
        assert!(!CordRole::Member.can_manage());
        assert!(is_cord_member(&[grant(CORD_MEMBER, Some(CORD_A))], CORD_A));
    }

    #[test]
    fn the_highest_role_wins_when_two_coexist() {
        let grants = [
            grant(CORD_MEMBER, Some(CORD_A)),
            grant(CORD_OWNER, Some(CORD_A)),
        ];
        assert_eq!(cord_role_in(&grants, CORD_A), Some(CordRole::Owner));
        assert_eq!(cord_memberships(&grants), vec![(CORD_A, CordRole::Owner)]);
    }

    #[test]
    fn roles_deserialize_from_their_strings() {
        let role: CordRole = serde_json::from_value(serde_json::json!("cord_admin")).unwrap();
        assert_eq!(role, CordRole::Admin);
        assert!(serde_json::from_value::<CordRole>(serde_json::json!("admin")).is_err());
        assert!(serde_json::from_value::<CordRole>(serde_json::json!("Owner")).is_err());
    }

    #[test]
    fn only_owners_set_member_roles() {
        assert!(CordRole::Owner.can_set_member_roles());
        assert!(!CordRole::Admin.can_set_member_roles());
        assert!(!CordRole::Member.can_set_member_roles());
    }

    #[test]
    fn removal_follows_the_authority_order() {
        use CordRole::{Admin, Member, Owner};
        for target in [Owner, Admin, Member] {
            assert!(Owner.can_remove(target), "owner removes {target:?}");
            assert!(!Member.can_remove(target), "member removes no one");
        }
        assert!(Admin.can_remove(Member));
        assert!(!Admin.can_remove(Admin));
        assert!(!Admin.can_remove(Owner));
    }

    #[test]
    fn the_last_owner_stays() {
        use CordRole::{Admin, Member, Owner};
        // the only owner can't leave, be removed, or step down
        assert!(leaves_no_owner(1, Owner, None));
        assert!(leaves_no_owner(1, Owner, Some(Admin)));
        assert!(leaves_no_owner(1, Owner, Some(Member)));
        // but keeping the role is fine
        assert!(!leaves_no_owner(1, Owner, Some(Owner)));
        // with another owner, any of those is fine
        assert!(!leaves_no_owner(2, Owner, None));
        assert!(!leaves_no_owner(2, Owner, Some(Member)));
        // non-owners never touch the owner count
        for current in [Admin, Member] {
            assert!(!leaves_no_owner(1, current, None));
            assert!(!leaves_no_owner(0, current, Some(Member)));
        }
        // a count of zero (every owner gone from the instance) still refuses
        // to let an owner row go
        assert!(leaves_no_owner(0, Owner, None));
    }

    #[test]
    fn memberships_list_each_cord_once_in_grant_order() {
        let grants = [
            grant("admin", None),
            grant(CORD_MEMBER, Some(CORD_B)),
            grant(CORD_OWNER, Some(CORD_A)),
            grant(CORD_ADMIN, Some(CORD_B)),
        ];
        assert_eq!(
            cord_memberships(&grants),
            vec![(CORD_B, CordRole::Admin), (CORD_A, CordRole::Owner)]
        );
    }
}

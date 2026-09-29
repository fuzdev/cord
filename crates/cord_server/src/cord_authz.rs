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
//! `cord_admin` on one cord can coexist. Today `cord_create` is the only verb
//! that confers a cord role, on a cord that didn't exist, so the rule holds.
//! [`cord_role_in`] still resolves a pair to the higher role, as a backstop.
//!
//! TODO: the member and invite verbs (invite accept, role set, remove, leave)
//! must keep the rule themselves: changing a role revokes the old grant and
//! creates the new one within one transaction, and a grant is never added
//! beside an existing cord role on the same cord.
//!
//! Instance admins get no implicit access to cords here. They create cords
//! and see the admin ops list, but a cord they aren't in stays hidden from
//! them in the normal surface.

use fuz_auth::RoleGrantRow;
use serde::Serialize;
use uuid::Uuid;

use crate::roles::{CORD_ADMIN, CORD_MEMBER, CORD_OWNER};

/// An actor's role in one cord, highest authority first.
///
/// Serializes to its `role_grant.role` string. The variant order is the
/// authority order, so `min` picks the highest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
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

    /// Whether the role may manage the cord (rename it, and later manage its
    /// members and spaces): the owner and admins.
    #[must_use]
    pub const fn can_manage(self) -> bool {
        matches!(self, Self::Owner | Self::Admin)
    }
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

/// Whether the actor may manage the cord `cord_id`: its owner or an admin.
#[must_use]
pub fn can_manage_cord(grants: &[RoleGrantRow], cord_id: Uuid) -> bool {
    cord_role_in(grants, cord_id).is_some_and(CordRole::can_manage)
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
        assert!(!can_manage_cord(&grants, CORD_B));
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
        assert!(!can_manage_cord(&grants, CORD_A));
        assert!(cord_memberships(&grants).is_empty());
    }

    #[test]
    fn other_roles_scoped_to_the_cord_confer_nothing() {
        let grants = [grant("admin", Some(CORD_A))];
        assert!(!is_cord_member(&grants, CORD_A));
    }

    #[test]
    fn management_is_owner_or_admin() {
        assert!(can_manage_cord(&[grant(CORD_OWNER, Some(CORD_A))], CORD_A));
        assert!(can_manage_cord(&[grant(CORD_ADMIN, Some(CORD_A))], CORD_A));
        let member = [grant(CORD_MEMBER, Some(CORD_A))];
        assert!(is_cord_member(&member, CORD_A));
        assert!(!can_manage_cord(&member, CORD_A));
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

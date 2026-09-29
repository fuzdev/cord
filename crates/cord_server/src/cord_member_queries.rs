//! Cord membership over the spine's `role_grant` table: reading a cord's
//! members, and changing them (role set, remove, leave) in one transaction
//! each.
//!
//! A member is an actor holding a live cord-scoped role grant on the cord
//! ([`crate::roles`]): not revoked, not expired, on a live actor and a live
//! account. Soft-deleted ones keep their grants but aren't members anyone
//! sees or can target. `live_role_grant_sql` is that rule in SQL, and every
//! read of who holds a role goes through it.
//!
//! Every change starts by locking the cord's cell row ([`query_cord_lock`]),
//! then reads the membership afresh and applies the rules in
//! [`crate::cord_authz`] to that read, not to the request's role-grant
//! snapshot. Membership changes on one cord therefore run one at a time, so
//! two owners stepping down together can't both pass the last-owner check,
//! and a caller whose demotion committed first can't act on the power they
//! lost. Invite accepts take the same lock ([`crate::invite::queries`]).
//!
//! The other managing verbs (renaming the cord, minting and revoking its
//! links) take no lock: they read the caller's role afresh with a plain
//! SELECT ([`query_cord_manager_role`]). A demotion that committed before
//! that read is honored; one that commits while the verb runs is ordered
//! after it, as if the verb had finished first.
//!
//! Changes go through the spine's role-grant queries
//! (`query_create_role_grant`, through `query_cord_role_grant_create`, and
//! `query_revoke_role_grant`), keeping one role per (actor, cord): a role
//! change revokes every cord-role grant the member holds on the cord, then
//! creates the new one. The spine's own grant paths can't touch cord roles
//! ([`crate::roles`]), so this lock covers every writer.
//!
//! Call the change functions inside a transaction; the lock is held until it
//! ends.

use deadpool_postgres::GenericClient;
use fuz_auth::query_create_role_grant;
use fuz_auth::role_grant_offer_queries::query_revoke_role_grant;
use fuz_cell::CellError;
use fuz_db::iso8601_timestamp_column;
use uuid::Uuid;

use crate::cord_authz::{CordRole, leaves_no_owner};
use crate::cord_queries::{CORD_CELL_KIND, CordData, CordRow};
use crate::roles::{CORD_ROLES, CORD_SCOPE_KIND};

/// The revoke reason `query_cord_member_role_set` records on the old grant.
pub const CORD_REVOKE_REASON_ROLE_SET: &str = "cord_member_role_set";
/// The revoke reason `query_cord_member_remove` records.
pub const CORD_REVOKE_REASON_REMOVE: &str = "cord_member_remove";
/// The revoke reason `query_cord_leave` records.
pub const CORD_REVOKE_REASON_LEAVE: &str = "cord_leave";

/// A `SELECT {select}` over the **live** role grants matching `conditions`:
/// not revoked, not expired, and held by a live (not soft-deleted) actor of a
/// live account.
///
/// This is the one definition of a grant that counts: the membership reads
/// here, the admin list's member counts ([`crate::cord_queries`]), and the
/// newcomer links' global-`admin` check
/// ([`crate::invite::redemption_queries`]) all read through it, so a
/// soft-deleted account's grants never make a member, an owner, or a minter.
/// The grant is aliased `rg`, its actor `ac`, and its account `a`.
/// `conditions` continues the `WHERE` (it's joined with `AND`) and may end
/// with `GROUP BY`, `ORDER BY`, `LIMIT`, or a locking clause.
pub(crate) fn live_role_grant_sql(select: &str, conditions: &str) -> String {
    format!(
        "SELECT {select}
         FROM role_grant rg
         JOIN actor ac ON ac.id = rg.actor_id AND ac.deleted_at IS NULL
         JOIN account a ON a.id = ac.account_id AND a.deleted_at IS NULL
         WHERE rg.revoked_at IS NULL
           AND (rg.expires_at IS NULL OR rg.expires_at > NOW())
           AND {conditions}"
    )
}

/// [`live_role_grant_sql`] narrowed to the cord roles: `rg.scope_kind` is
/// `$1` and `rg.role` is one of `$2`, which the caller binds to
/// [`CORD_SCOPE_KIND`] and [`CORD_ROLES`], numbering its own parameters from
/// `$3`.
pub(crate) fn live_cord_grant_sql(select: &str, conditions: &str) -> String {
    live_role_grant_sql(
        select,
        &format!("rg.scope_kind = $1 AND rg.role = ANY($2::text[]) AND {conditions}"),
    )
}

/// Grant `role` on the cord `cord_id` to the actor `actor_id`, recorded as
/// granted by `granted_by`, returning the grant's id.
///
/// The one way Cord's verbs confer a cord role: a cord-scoped grant through
/// the spine's `query_create_role_grant`, with no expiry. The caller keeps
/// the one-role-per-(actor, cord) rule.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure, including the spine's unique index
/// refusing a second grant of the same role.
pub(crate) async fn query_cord_role_grant_create<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    actor_id: Uuid,
    role: CordRole,
    granted_by: Option<Uuid>,
) -> Result<Uuid, CellError> {
    Ok(query_create_role_grant(
        client,
        &actor_id,
        role.as_str(),
        Some(CORD_SCOPE_KIND),
        Some(&cord_id),
        granted_by.as_ref(),
        None,
    )
    .await?
    .id)
}

/// One live cord-role grant, as read from the database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CordMemberGrant {
    pub role_grant_id: Uuid,
    pub actor_id: Uuid,
    pub role: CordRole,
    pub account_id: Uuid,
    pub username: String,
    /// The actor's `name`, verbatim.
    pub actor_name: String,
}

/// A member of a cord: one actor, their role, and the grants behind it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordMember {
    pub actor_id: Uuid,
    pub account_id: Uuid,
    pub username: String,
    /// The actor's name, or `None` when it's blank (the spine's
    /// `actor_lookup` omits a blank name the same way).
    pub display_name: Option<String>,
    /// The highest role among `grants`.
    pub role: CordRole,
    /// Every active cord-role grant the actor holds on the cord: one, by the
    /// one-role rule, but a change revokes them all.
    pub grants: Vec<(Uuid, CordRole)>,
}

/// Fold grant rows into members, one per actor with their highest role,
/// ordered by role (owners first), then username, then actor id.
#[must_use]
pub(crate) fn fold_cord_members(grants: Vec<CordMemberGrant>) -> Vec<CordMember> {
    let mut members: Vec<CordMember> = Vec::new();
    for grant in grants {
        if let Some(member) = members.iter_mut().find(|m| m.actor_id == grant.actor_id) {
            member.role = member.role.min(grant.role);
            member.grants.push((grant.role_grant_id, grant.role));
            continue;
        }
        let name = grant.actor_name.trim();
        members.push(CordMember {
            actor_id: grant.actor_id,
            account_id: grant.account_id,
            username: grant.username,
            display_name: (!name.is_empty()).then(|| name.to_owned()),
            role: grant.role,
            grants: vec![(grant.role_grant_id, grant.role)],
        });
    }
    members.sort_by(|a, b| {
        a.role
            .cmp(&b.role)
            .then_with(|| a.username.cmp(&b.username))
            .then_with(|| a.actor_id.cmp(&b.actor_id))
    });
    members
}

/// How many members of `members` are owners.
#[must_use]
pub(crate) fn cord_owner_count(members: &[CordMember]) -> usize {
    members.iter().filter(|m| m.role == CordRole::Owner).count()
}

/// Lock the cord's cell row until the transaction ends, returning the cord.
///
/// `None` when no active cord has that id, or when its cell doesn't read as
/// a cord, as [`CordRow::from_cell`] decides everywhere else.
///
/// `FOR NO KEY UPDATE` serializes the cord's membership changes against
/// each other while still letting rows that reference the cell (a new
/// invite, a cell grant) be inserted alongside.
///
/// The serialization relies on READ COMMITTED, Postgres's default and the
/// level the action dispatcher's transactions run at: each statement after
/// the lock takes a fresh snapshot, so the membership read that follows a
/// wait sees what the previous holder committed. Under REPEATABLE READ the
/// transaction's snapshot would predate that commit, the post-lock read
/// would be stale, and two owners could both pass the last-owner check.
/// `cordd` enforces it at boot: it refuses to serve unless the database's
/// `default_transaction_isolation` is `read committed`
/// ([`crate::transaction_isolation`]).
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub async fn query_cord_lock<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<Option<CordRow>, CellError> {
    let sql = format!(
        "SELECT id, data::text, {} FROM cell
         WHERE id = $1 AND kind = $2 AND deleted_at IS NULL
         FOR NO KEY UPDATE",
        iso8601_timestamp_column("cell", "created_at"),
    );
    let Some(row) = client.query_opt(&sql, &[&cord_id, &CORD_CELL_KIND]).await? else {
        return Ok(None);
    };
    let data: String = row.get(1);
    Ok(serde_json::from_str::<CordData>(&data)
        .ok()
        .map(|data| CordRow {
            id: row.get(0),
            name: data.name,
            created_at: row.get(2),
        }))
}

/// Every live cord-role grant on the cord `cord_id` ([`live_cord_grant_sql`]),
/// oldest first.
///
/// # Errors
///
/// [`CellError::Db`] on a driver failure.
pub(crate) async fn query_cord_member_grants<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<Vec<CordMemberGrant>, CellError> {
    let sql = live_cord_grant_sql(
        "rg.id, rg.actor_id, rg.role, ac.account_id, a.username, ac.name",
        "rg.scope_id = $3 ORDER BY rg.created_at, rg.id",
    );
    let rows = client
        .query(&sql, &[&CORD_SCOPE_KIND, &CORD_ROLES.as_slice(), &cord_id])
        .await?;
    rows.iter()
        .map(|row| {
            let role: String = row.get(2);
            let role = CordRole::from_role(&role)
                .ok_or_else(|| CellError::Decode(format!("not a cord role: {role}")))?;
            Ok(CordMemberGrant {
                role_grant_id: row.get(0),
                actor_id: row.get(1),
                role,
                account_id: row.get(3),
                username: row.get(4),
                actor_name: row.get(5),
            })
        })
        .collect()
}

/// The actor's role in the cord `cord_id` as the database has it now (the
/// highest of their live cord grants, `live_cord_grant_sql`), or `None`
/// when they aren't in it. A plain read with no lock.
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cord_member_role<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    actor_id: Uuid,
) -> Result<Option<CordRole>, CellError> {
    let sql = live_cord_grant_sql("rg.role", "rg.scope_id = $3 AND rg.actor_id = $4");
    let rows = client
        .query(
            &sql,
            &[
                &CORD_SCOPE_KIND,
                &CORD_ROLES.as_slice(),
                &cord_id,
                &actor_id,
            ],
        )
        .await?;
    let mut highest: Option<CordRole> = None;
    for row in &rows {
        let role: String = row.get(0);
        let role = CordRole::from_role(&role)
            .ok_or_else(|| CellError::Decode(format!("not a cord role: {role}")))?;
        highest = Some(highest.map_or(role, |h| h.min(role)));
    }
    Ok(highest)
}

/// Read the caller's role in the cord afresh ([`query_cord_member_role`])
/// and require that it may manage the cord ([`CordRole::can_manage`]).
///
/// The gate for the managing verbs that take no cord lock (rename, invite
/// create and revoke). It reads the database inside the verb's transaction,
/// never the request's role-grant snapshot, which the dispatcher takes
/// before the transaction opens.
///
/// # Errors
///
/// [`CordMemberError::CordNotFound`] when the caller isn't in the cord,
/// [`CordMemberError::Forbidden`] for a plain member, or a driver failure.
pub async fn query_cord_manager_role<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    caller_actor_id: Uuid,
) -> Result<CordRole, CordMemberError> {
    let Some(role) = query_cord_member_role(client, cord_id, caller_actor_id).await? else {
        return Err(CordMemberError::CordNotFound);
    };
    if !role.can_manage() {
        return Err(CordMemberError::Forbidden);
    }
    Ok(role)
}

/// The members of the cord `cord_id`, owners first (`fold_cord_members`).
///
/// # Errors
///
/// [`CellError`] on a driver or decode failure.
pub async fn query_cord_members<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
) -> Result<Vec<CordMember>, CellError> {
    Ok(fold_cord_members(
        query_cord_member_grants(client, cord_id).await?,
    ))
}

/// Why a membership change was refused.
#[derive(Debug, thiserror::Error)]
pub enum CordMemberError {
    /// No active cord has that id, or the caller isn't in it.
    #[error("no such cord, or the caller isn't in it")]
    CordNotFound,
    /// The caller's role can't make this change.
    #[error("the caller's role can't make this change")]
    Forbidden,
    /// The target isn't a member of the cord.
    #[error("the target isn't a member of the cord")]
    MemberNotFound,
    /// The caller targeted themself for removal; leaving is its own verb.
    #[error("the caller can't remove themself; they leave instead")]
    SelfRemove,
    /// The change would leave the cord with no owner.
    #[error("the cord would have no owner")]
    LastOwner,
    /// A driver or decode failure.
    #[error(transparent)]
    Db(#[from] CellError),
}

/// A cord grant a change revoked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RevokedCordGrant {
    pub role_grant_id: Uuid,
    pub role: CordRole,
}

/// What a membership change did to one member.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CordMemberChange {
    /// The member as they were before the change.
    pub member: CordMember,
    /// The grants revoked, empty when nothing changed.
    pub revoked: Vec<RevokedCordGrant>,
    /// The grant created (its id and role), for a role change that changed
    /// something.
    pub granted: Option<(Uuid, CordRole)>,
}

impl CordMemberChange {
    /// Whether the change wrote anything.
    #[must_use]
    pub const fn changed(&self) -> bool {
        !self.revoked.is_empty() || self.granted.is_some()
    }
}

/// Lock the cord and read its members, then find the caller among them.
async fn lock_and_read<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    caller_actor_id: Uuid,
) -> Result<(Vec<CordMember>, CordMember), CordMemberError> {
    if query_cord_lock(client, cord_id).await?.is_none() {
        return Err(CordMemberError::CordNotFound);
    }
    let members = query_cord_members(client, cord_id).await?;
    let caller = members
        .iter()
        .find(|m| m.actor_id == caller_actor_id)
        .cloned()
        .ok_or(CordMemberError::CordNotFound)?;
    Ok((members, caller))
}

/// Revoke every cord grant `member` holds on the cord.
async fn revoke_member_grants<C: GenericClient + ?Sized>(
    client: &C,
    member: &CordMember,
    revoked_by: Uuid,
    reason: &str,
) -> Result<Vec<RevokedCordGrant>, CordMemberError> {
    let mut revoked = Vec::with_capacity(member.grants.len());
    for &(role_grant_id, role) in &member.grants {
        let result = query_revoke_role_grant(
            client,
            role_grant_id,
            member.actor_id,
            Some(revoked_by),
            Some(reason),
        )
        .await
        .map_err(CellError::Db)?;
        // `None` means it was already revoked; under the cord lock only a
        // cascade outside Cord's verbs could do that, and it's gone either way
        if result.is_some() {
            revoked.push(RevokedCordGrant {
                role_grant_id,
                role,
            });
        }
    }
    Ok(revoked)
}

/// Set a member's role in the cord, as the caller.
///
/// Owners only. Refuses a change that would leave the cord with no owner, so
/// an owner may step down only while another owner remains. Setting the role
/// a member already has changes nothing. Otherwise revokes the member's cord
/// grants and creates the new one, all in the caller's transaction.
///
/// # Errors
///
/// [`CordMemberError`] naming the rule the change breaks, or a driver
/// failure.
pub async fn query_cord_member_role_set<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    caller_actor_id: Uuid,
    target_actor_id: Uuid,
    role: CordRole,
) -> Result<CordMemberChange, CordMemberError> {
    let (members, caller) = lock_and_read(client, cord_id, caller_actor_id).await?;
    if !caller.role.can_set_member_roles() {
        return Err(CordMemberError::Forbidden);
    }
    let target = members
        .iter()
        .find(|m| m.actor_id == target_actor_id)
        .cloned()
        .ok_or(CordMemberError::MemberNotFound)?;
    if target.role == role && target.grants.len() == 1 {
        return Ok(CordMemberChange {
            member: target,
            revoked: Vec::new(),
            granted: None,
        });
    }
    if leaves_no_owner(cord_owner_count(&members), target.role, Some(role)) {
        return Err(CordMemberError::LastOwner);
    }
    let revoked = revoke_member_grants(
        client,
        &target,
        caller_actor_id,
        CORD_REVOKE_REASON_ROLE_SET,
    )
    .await?;
    let role_grant_id = query_cord_role_grant_create(
        client,
        cord_id,
        target.actor_id,
        role,
        Some(caller_actor_id),
    )
    .await?;
    Ok(CordMemberChange {
        member: target,
        revoked,
        granted: Some((role_grant_id, role)),
    })
}

/// Remove a member from the cord, as the caller.
///
/// An owner removes anyone else, an admin removes plain members, and a
/// member removes no one ([`CordRole::can_remove`]); the last owner can't be
/// removed. Nobody removes themself, whatever their role: that's
/// [`query_cord_leave`], so the revoke reason says what happened.
///
/// # Errors
///
/// [`CordMemberError`] naming the rule the removal breaks, or a driver
/// failure.
pub async fn query_cord_member_remove<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    caller_actor_id: Uuid,
    target_actor_id: Uuid,
) -> Result<CordMemberChange, CordMemberError> {
    let (members, caller) = lock_and_read(client, cord_id, caller_actor_id).await?;
    if target_actor_id == caller_actor_id {
        return Err(CordMemberError::SelfRemove);
    }
    if !caller.role.can_manage() {
        return Err(CordMemberError::Forbidden);
    }
    let target = members
        .iter()
        .find(|m| m.actor_id == target_actor_id)
        .cloned()
        .ok_or(CordMemberError::MemberNotFound)?;
    if !caller.role.can_remove(target.role) {
        return Err(CordMemberError::Forbidden);
    }
    if leaves_no_owner(cord_owner_count(&members), target.role, None) {
        return Err(CordMemberError::LastOwner);
    }
    let revoked =
        revoke_member_grants(client, &target, caller_actor_id, CORD_REVOKE_REASON_REMOVE).await?;
    Ok(CordMemberChange {
        member: target,
        revoked,
        granted: None,
    })
}

/// Leave the cord. Any member may, except its last owner.
///
/// # Errors
///
/// [`CordMemberError::LastOwner`] for the last owner,
/// [`CordMemberError::CordNotFound`] when the caller isn't in the cord, or a
/// driver failure.
pub async fn query_cord_leave<C: GenericClient + ?Sized>(
    client: &C,
    cord_id: Uuid,
    caller_actor_id: Uuid,
) -> Result<CordMemberChange, CordMemberError> {
    let (members, caller) = lock_and_read(client, cord_id, caller_actor_id).await?;
    if leaves_no_owner(cord_owner_count(&members), caller.role, None) {
        return Err(CordMemberError::LastOwner);
    }
    let revoked =
        revoke_member_grants(client, &caller, caller_actor_id, CORD_REVOKE_REASON_LEAVE).await?;
    Ok(CordMemberChange {
        member: caller,
        revoked,
        granted: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(n: u128, actor: u128, role: CordRole, username: &str, name: &str) -> CordMemberGrant {
        CordMemberGrant {
            role_grant_id: Uuid::from_u128(n),
            actor_id: Uuid::from_u128(actor),
            role,
            account_id: Uuid::from_u128(actor + 1000),
            username: username.to_owned(),
            actor_name: name.to_owned(),
        }
    }

    #[test]
    fn members_fold_to_one_per_actor_with_the_highest_role() {
        let members = fold_cord_members(vec![
            grant(1, 10, CordRole::Member, "zed", "Zed"),
            grant(2, 20, CordRole::Member, "amy", "  "),
            grant(3, 10, CordRole::Owner, "zed", "Zed"),
            grant(4, 30, CordRole::Admin, "bea", "Bea"),
        ]);
        let summary: Vec<(&str, CordRole, Option<&str>, usize)> = members
            .iter()
            .map(|m| {
                (
                    m.username.as_str(),
                    m.role,
                    m.display_name.as_deref(),
                    m.grants.len(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("zed", CordRole::Owner, Some("Zed"), 2),
                ("bea", CordRole::Admin, Some("Bea"), 1),
                ("amy", CordRole::Member, None, 1),
            ]
        );
        assert_eq!(cord_owner_count(&members), 1);
    }

    #[test]
    fn members_of_one_role_sort_by_username() {
        let members = fold_cord_members(vec![
            grant(1, 1, CordRole::Member, "cat", "c"),
            grant(2, 2, CordRole::Member, "ann", "a"),
            grant(3, 3, CordRole::Owner, "bob", "b"),
            grant(4, 4, CordRole::Owner, "abe", "a"),
        ]);
        let names: Vec<&str> = members.iter().map(|m| m.username.as_str()).collect();
        assert_eq!(names, ["abe", "bob", "ann", "cat"]);
        assert_eq!(cord_owner_count(&members), 2);
    }

    #[test]
    fn a_change_that_wrote_nothing_is_unchanged() {
        let member = fold_cord_members(vec![grant(1, 1, CordRole::Member, "a", "a")])
            .pop()
            .unwrap();
        let unchanged = CordMemberChange {
            member: member.clone(),
            revoked: Vec::new(),
            granted: None,
        };
        assert!(!unchanged.changed());
        let removed = CordMemberChange {
            member,
            revoked: vec![RevokedCordGrant {
                role_grant_id: Uuid::from_u128(1),
                role: CordRole::Member,
            }],
            granted: None,
        };
        assert!(removed.changed());
    }
}

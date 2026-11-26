//! Database tests for cord membership changes
//! (`cord_server::cord_member_queries`).
//!
//! What only the database can show: a role change leaves exactly one active
//! cord grant per (actor, cord), with the old one revoked; the permission
//! tiers and the last-owner rule hold against membership read under the
//! cord's lock; nobody removes themself; the unlocked managing verbs' role
//! read sees a demotion or removal the request's snapshot doesn't;
//! soft-deleted actors aren't members; two owners leaving at once can't
//! both get out; an invite accept racing a removal and a role change on
//! the same actor ends with at most one role and no deadlock; and a role set
//! and an accept take the cord's scope lock before any row, so with the lock
//! held exclusive (as a teardown would) they wait holding nothing.
//!
//! The scenarios run in order from one test function, over a database of
//! their own (`common::DB`), but for the ones that observe a lock
//! wait, which only PostgreSQL shows: those are a test of their own, on
//! PostgreSQL alone.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside `#[test]` fns panic on a failed setup or check"
)]

mod common;

use common::{DB, add_member, assert_refused, create_cord, mint, purge_account_steps, seed_actor};
use cord_server::cord_authz::{CordRole, cord_role_in};
use cord_server::cord_member_queries::{
    CORD_REVOKE_REASON_LEAVE, CORD_REVOKE_REASON_REMOVE, CORD_REVOKE_REASON_ROLE_SET,
    CordMemberError, query_cord_leave, query_cord_manager_role, query_cord_member_remove,
    query_cord_member_role_set, query_cord_members,
};
use cord_server::cord_queries::{CordRow, query_cord_rename};
use cord_server::invite::queries::{
    CordInviteError, hash_cord_invite_token, query_cord_invite_accept,
};
use cord_server::roles::{CORD_MEMBER, CORD_SCOPE_KIND};
use fuz_auth::account_queries::{query_account_lock_for_purge, query_account_purge_advisory_lock};
use fuz_auth::query_scope_lock_exclusive;
use fuz_db::Pool;
use fuz_testing::{backend_pid, in_tx, until_blocked_by, until_lock_waiters};
use uuid::Uuid;

/// The members as (actor, role), owners first.
async fn roster(pool: &Pool, cord_id: Uuid) -> Vec<(Uuid, CordRole)> {
    let client = pool.get().await.expect("pool.get");
    query_cord_members(&client, cord_id)
        .await
        .expect("members")
        .iter()
        .map(|m| (m.actor_id, m.role))
        .collect()
}

/// Every active cord-scoped grant row on the cord, as (actor, role).
async fn active_grants(pool: &Pool, cord_id: Uuid) -> Vec<(Uuid, String)> {
    let client = pool.get().await.expect("pool.get");
    let mut rows: Vec<(Uuid, String)> = client
        .query(
            "SELECT actor_id, role FROM role_grant
             WHERE scope_kind = $1 AND scope_id = $2 AND revoked_at IS NULL",
            &[&CORD_SCOPE_KIND, &cord_id],
        )
        .await
        .expect("grants")
        .iter()
        .map(|r| (r.get(0), r.get(1)))
        .collect();
    rows.sort();
    rows
}

/// The revoke reasons recorded for an actor's cord grants on the cord.
async fn revoke_reasons(pool: &Pool, cord_id: Uuid, actor_id: Uuid) -> Vec<String> {
    let client = pool.get().await.expect("pool.get");
    client
        .query(
            "SELECT revoked_reason FROM role_grant
             WHERE scope_id = $1 AND actor_id = $2 AND revoked_at IS NOT NULL
             ORDER BY revoked_at, id",
            &[&cord_id, &actor_id],
        )
        .await
        .expect("revoked")
        .iter()
        .map(|r| r.get(0))
        .collect()
}

/// The first cord and the people in and around it.
#[derive(Debug, Clone)]
struct Porch {
    cord: CordRow,
    owner: Uuid,
    admin: Uuid,
    member: Uuid,
    other: Uuid,
    stranger: Uuid,
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn membership_changes_keep_one_role_and_an_owner() {
    let pool = DB.open().await;
    let owner = seed_actor(&pool, "owner").await;
    let admin = seed_actor(&pool, "admin").await;
    let member = seed_actor(&pool, "member").await;
    let other = seed_actor(&pool, "other").await;
    let stranger = seed_actor(&pool, "stranger").await;
    let cord = create_cord(&pool, "the porch", owner).await;
    add_member(&pool, cord.id, admin, CordRole::Admin).await;
    add_member(&pool, cord.id, member, CordRole::Member).await;
    add_member(&pool, cord.id, other, CordRole::Member).await;
    let porch = Porch {
        cord,
        owner,
        admin,
        member,
        other,
        stranger,
    };

    scenario_the_roster(&pool, &porch).await;
    scenario_only_owners_set_roles(&pool, &porch).await;
    scenario_the_last_owner_stays(&pool, &porch).await;
    scenario_remove(&pool, &porch).await;
    scenario_leave(&pool, &porch).await;
    scenario_soft_deleted_actors_arent_members(&pool, &porch).await;
    scenario_two_owners_leaving_at_once(&pool).await;
    let (shed, host) = scenario_the_unlocked_verbs_read_the_role_afresh(&pool, stranger).await;
    scenario_accept_remove_and_role_set_race(&pool, &shed, host).await;
}

/// The lock order on real row locks, which only PostgreSQL shows waiting.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "postgres-only: lock waits observed through `pg_stat_activity`"]
async fn membership_changes_take_their_locks_in_order() {
    let pool = DB.postgres_only().open().await;
    scenario_a_purge_of_a_renamer_mid_remove(&pool).await;
    scenario_the_scope_lock_comes_first(&pool).await;
}

/// Owners first, soft-deleted actors left out.
async fn scenario_the_roster(pool: &Pool, porch: &Porch) {
    let Porch {
        cord,
        owner,
        admin,
        member,
        other,
        ..
    } = porch.clone();
    assert_eq!(
        roster(pool, cord.id).await,
        vec![
            (owner, CordRole::Owner),
            (admin, CordRole::Admin),
            (member, CordRole::Member),
            (other, CordRole::Member),
        ]
    );
}

/// Owners set roles, keeping one role per (actor, cord).
async fn scenario_only_owners_set_roles(pool: &Pool, porch: &Porch) {
    let Porch {
        cord,
        owner,
        admin,
        member,
        other,
        stranger,
    } = porch.clone();
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            cord.id,
            admin,
            member,
            CordRole::Admin
        )),
        CordMemberError::Forbidden,
    );
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            cord.id,
            member,
            member,
            CordRole::Owner
        )),
        CordMemberError::Forbidden,
    );
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            cord.id,
            stranger,
            member,
            CordRole::Admin
        )),
        CordMemberError::CordNotFound,
    );
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            cord.id,
            owner,
            stranger,
            CordRole::Admin
        )),
        CordMemberError::MemberNotFound,
    );
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            Uuid::nil(),
            owner,
            member,
            CordRole::Admin
        )),
        CordMemberError::CordNotFound,
    );

    // promote a member: the old grant is revoked, the new one is the only
    // active cord grant they hold
    let change = in_tx!(pool, |tx| query_cord_member_role_set(
        &tx,
        cord.id,
        owner,
        member,
        CordRole::Admin
    ))
    .expect("promote");
    assert_eq!(change.member.role, CordRole::Member);
    assert_eq!(change.revoked.len(), 1);
    assert_eq!(change.revoked[0].role, CordRole::Member);
    assert_eq!(change.granted.map(|(_, role)| role), Some(CordRole::Admin));
    assert!(
        active_grants(pool, cord.id)
            .await
            .iter()
            .filter(|(actor, _)| *actor == member)
            .map(|(_, role)| role.as_str())
            .eq(["cord_admin"]),
        "one role per (actor, cord)"
    );
    assert_eq!(
        revoke_reasons(pool, cord.id, member).await,
        [CORD_REVOKE_REASON_ROLE_SET]
    );

    // setting the same role again changes nothing
    let change = in_tx!(pool, |tx| query_cord_member_role_set(
        &tx,
        cord.id,
        owner,
        member,
        CordRole::Admin
    ))
    .expect("no-op");
    assert!(!change.changed());

    // a stray second grant (which no verb writes) is folded away by the next
    // role change
    add_member(pool, cord.id, other, CordRole::Admin).await;
    in_tx!(pool, |tx| query_cord_member_role_set(
        &tx,
        cord.id,
        owner,
        other,
        CordRole::Member
    ))
    .expect("normalize");
    assert!(
        active_grants(pool, cord.id)
            .await
            .iter()
            .filter(|(actor, _)| *actor == other)
            .map(|(_, role)| role.as_str())
            .eq([CORD_MEMBER]),
    );
}

/// The last owner can't step down or leave until another owner is made;
/// after this, `admin` is the only owner and `member` an admin.
async fn scenario_the_last_owner_stays(pool: &Pool, porch: &Porch) {
    let Porch {
        cord,
        owner,
        admin,
        member,
        other,
        ..
    } = porch.clone();
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            cord.id,
            owner,
            owner,
            CordRole::Admin
        )),
        CordMemberError::LastOwner,
    );
    assert_refused!(
        in_tx!(pool, |tx| query_cord_leave(&tx, cord.id, owner)),
        CordMemberError::LastOwner,
    );
    // removing yourself is leaving, so the remove verb refuses it outright
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_remove(
            &tx, cord.id, owner, owner
        )),
        CordMemberError::SelfRemove,
    );

    // a second owner lets the first step down
    in_tx!(pool, |tx| query_cord_member_role_set(
        &tx,
        cord.id,
        owner,
        admin,
        CordRole::Owner
    ))
    .expect("second owner");
    in_tx!(pool, |tx| query_cord_member_role_set(
        &tx,
        cord.id,
        owner,
        owner,
        CordRole::Member
    ))
    .expect("step down");
    assert_eq!(
        roster(pool, cord.id).await,
        vec![
            (admin, CordRole::Owner),
            (member, CordRole::Admin),
            // members of one role sort by username
            (other, CordRole::Member),
            (owner, CordRole::Member),
        ]
    );
    // the one who stepped down has no owner power left, under the lock
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            cord.id,
            owner,
            owner,
            CordRole::Owner
        )),
        CordMemberError::Forbidden,
    );
    // `admin` is now the only owner
}

async fn scenario_remove(pool: &Pool, porch: &Porch) {
    let Porch {
        cord,
        owner,
        admin,
        member,
        other,
        stranger,
    } = porch.clone();
    // the last-owner scenario made `admin` the only owner, `member` an admin
    let (new_owner, new_admin) = (admin, member);
    // an admin removes plain members only
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_remove(
            &tx, cord.id, new_admin, new_owner
        )),
        CordMemberError::Forbidden,
    );
    // nobody removes themself, whatever their role
    for caller in [new_owner, new_admin, other] {
        assert_refused!(
            in_tx!(pool, |tx| query_cord_member_remove(
                &tx, cord.id, caller, caller
            )),
            CordMemberError::SelfRemove,
        );
    }
    // a member removes no one
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_remove(
            &tx, cord.id, other, owner
        )),
        CordMemberError::Forbidden,
    );
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_remove(
            &tx, cord.id, new_admin, stranger
        )),
        CordMemberError::MemberNotFound,
    );
    let change = in_tx!(pool, |tx| query_cord_member_remove(
        &tx, cord.id, new_admin, other
    ))
    .expect("admin removes a member");
    assert_eq!(change.revoked.len(), 1);
    assert_eq!(
        revoke_reasons(pool, cord.id, other).await.last().unwrap(),
        CORD_REVOKE_REASON_REMOVE
    );
    // the owner removes an admin
    in_tx!(pool, |tx| query_cord_member_remove(
        &tx, cord.id, new_owner, new_admin
    ))
    .expect("owner removes an admin");
    assert_eq!(
        roster(pool, cord.id).await,
        vec![(new_owner, CordRole::Owner), (owner, CordRole::Member)]
    );
    // a removed member is out: they can't act on the cord
    assert_refused!(
        in_tx!(pool, |tx| query_cord_leave(&tx, cord.id, other)),
        CordMemberError::CordNotFound,
    );
}

async fn scenario_leave(pool: &Pool, porch: &Porch) {
    let Porch {
        cord, owner, admin, ..
    } = porch.clone();
    let new_owner = admin;
    in_tx!(pool, |tx| query_cord_leave(&tx, cord.id, owner)).expect("a member leaves");
    assert_eq!(
        revoke_reasons(pool, cord.id, owner).await.last().unwrap(),
        CORD_REVOKE_REASON_LEAVE
    );
    assert_eq!(
        roster(pool, cord.id).await,
        vec![(new_owner, CordRole::Owner)]
    );
}

async fn scenario_soft_deleted_actors_arent_members(pool: &Pool, porch: &Porch) {
    let Porch {
        cord,
        admin,
        stranger,
        ..
    } = porch.clone();
    let new_owner = admin;
    add_member(pool, cord.id, stranger, CordRole::Owner).await;
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "UPDATE actor SET deleted_at = NOW() WHERE id = $1",
                &[&stranger],
            )
            .await
            .expect("soft-delete");
    }
    assert_eq!(
        roster(pool, cord.id).await,
        vec![(new_owner, CordRole::Owner)]
    );
    // so a soft-deleted owner doesn't count toward keeping the cord owned
    assert_refused!(
        in_tx!(pool, |tx| query_cord_leave(&tx, cord.id, new_owner)),
        CordMemberError::LastOwner,
    );
    assert_refused!(
        in_tx!(pool, |tx| query_cord_member_role_set(
            &tx,
            cord.id,
            new_owner,
            stranger,
            CordRole::Member
        )),
        CordMemberError::MemberNotFound,
    );
}

/// Two owners leave at once: one stays.
async fn scenario_two_owners_leaving_at_once(pool: &Pool) {
    let a = seed_actor(pool, "race_a").await;
    let b = seed_actor(pool, "race_b").await;
    let raced = create_cord(pool, "the stoop", a).await;
    add_member(pool, raced.id, b, CordRole::Owner).await;
    for _ in 0..5 {
        let leave = |actor: Uuid| {
            let pool = pool.clone();
            let cord_id = raced.id;
            tokio::spawn(async move {
                let mut client = pool.get().await.expect("pool.get");
                let tx = client.transaction().await.expect("begin");
                let result = query_cord_leave(&tx, cord_id, actor).await;
                // hold the lock a moment so the other leave has to wait on it
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
                if result.is_ok() {
                    tx.commit().await.expect("commit");
                } else {
                    tx.rollback().await.expect("rollback");
                }
                result.map(|_| ())
            })
        };
        let (ra, rb) = tokio::join!(leave(a), leave(b));
        let (ra, rb) = (ra.expect("join"), rb.expect("join"));
        let left: Vec<bool> = vec![ra.is_ok(), rb.is_ok()];
        assert_eq!(
            left.iter().filter(|ok| **ok).count(),
            1,
            "exactly one owner leaves: {ra:?} {rb:?}"
        );
        let refused = if ra.is_ok() { rb } else { ra };
        assert!(
            matches!(refused, Err(CordMemberError::LastOwner)),
            "{refused:?}"
        );
        let remaining = roster(pool, raced.id).await;
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].1, CordRole::Owner);
        // put the one who left back for the next round
        let gone = if remaining[0].0 == a { b } else { a };
        add_member(pool, raced.id, gone, CordRole::Owner).await;
    }
}

/// Rename and invite create and revoke gate on `query_cord_manager_role`,
/// which reads the database in the action's transaction; the request's
/// role-grant snapshot, taken before it, can still show a lost role.
/// Returns the cord it made and its owner.
async fn scenario_the_unlocked_verbs_read_the_role_afresh(
    pool: &Pool,
    stranger: Uuid,
) -> (CordRow, Uuid) {
    let host = seed_actor(pool, "host").await;
    let deputy = seed_actor(pool, "deputy").await;
    let shed = create_cord(pool, "the shed", host).await;
    add_member(pool, shed.id, deputy, CordRole::Admin).await;
    let snapshot = || async {
        let client = pool.get().await.expect("pool.get");
        fuz_auth::actor_queries::query_role_grant_find_active_for_actor(&client, &deputy)
            .await
            .expect("grants")
    };
    let manager_role = || async {
        let client = pool.get().await.expect("pool.get");
        query_cord_manager_role(&client, shed.id, deputy).await
    };
    assert!(matches!(manager_role().await, Ok(CordRole::Admin)));
    assert!(matches!(
        {
            let client = pool.get().await.expect("pool.get");
            query_cord_manager_role(&client, shed.id, host).await
        },
        Ok(CordRole::Owner)
    ));
    assert!(matches!(
        {
            let client = pool.get().await.expect("pool.get");
            query_cord_manager_role(&client, shed.id, stranger).await
        },
        Err(CordMemberError::CordNotFound)
    ));
    // demoted: the stale snapshot still says admin, the fresh read refuses
    let stale = snapshot().await;
    in_tx!(pool, |tx| query_cord_member_role_set(
        &tx,
        shed.id,
        host,
        deputy,
        CordRole::Member
    ))
    .expect("demote");
    assert_eq!(cord_role_in(&stale, shed.id), Some(CordRole::Admin));
    assert!(matches!(
        manager_role().await,
        Err(CordMemberError::Forbidden)
    ));
    // removed: the stale snapshot still says member, the fresh read masks
    let stale = snapshot().await;
    in_tx!(pool, |tx| query_cord_member_remove(
        &tx, shed.id, host, deputy
    ))
    .expect("remove");
    assert_eq!(cord_role_in(&stale, shed.id), Some(CordRole::Member));
    assert!(matches!(
        manager_role().await,
        Err(CordMemberError::CordNotFound)
    ));
    (shed, host)
}

/// The three writers take the cord's lock (accept after the link's), so
/// whatever order they land in, the actor ends with at most one cord role
/// and nobody deadlocks.
async fn scenario_accept_remove_and_role_set_race(pool: &Pool, shed: &CordRow, host: Uuid) {
    let (token, _) = mint(pool, shed.id, host, None, None, false).await;
    let token_hash = hash_cord_invite_token(&token);
    for round in 0..8_u64 {
        let target = seed_actor(pool, &format!("contested_{round}")).await;
        add_member(pool, shed.id, target, CordRole::Member).await;
        // stagger the starts differently each round to vary the interleaving
        let delay = |slot: u64| std::time::Duration::from_millis(((round + slot) % 3) * 5);
        let run = |slot: u64, op: u8| {
            let pool = pool.clone();
            let token_hash = token_hash.clone();
            let cord_id = shed.id;
            let start = delay(slot);
            tokio::spawn(async move {
                tokio::time::sleep(start).await;
                let mut client = pool.get().await.expect("pool.get");
                let tx = client.transaction().await.expect("begin");
                let result: Result<&'static str, String> = match op {
                    0 => match query_cord_invite_accept(&tx, &token_hash, target).await {
                        Ok(a) if a.role_grant_id.is_some() => Ok("joined"),
                        Ok(_) => Ok("already in"),
                        Err(e @ CordInviteError::Db(_)) => Err(format!("{e:?}")),
                        Err(e) => Err(format!("unexpected: {e:?}")),
                    },
                    1 => match query_cord_member_remove(&tx, cord_id, host, target).await {
                        Ok(_) => Ok("removed"),
                        Err(CordMemberError::MemberNotFound) => Ok("not in"),
                        Err(e) => Err(format!("{e:?}")),
                    },
                    _ => match query_cord_member_role_set(
                        &tx,
                        cord_id,
                        host,
                        target,
                        CordRole::Admin,
                    )
                    .await
                    {
                        Ok(_) => Ok("promoted"),
                        Err(CordMemberError::MemberNotFound) => Ok("not in"),
                        Err(e) => Err(format!("{e:?}")),
                    },
                };
                // hold the locks a moment so the others queue on them
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                if result.is_ok() {
                    tx.commit().await.expect("commit");
                } else {
                    tx.rollback().await.expect("rollback");
                }
                result
            })
        };
        let all = async { tokio::join!(run(0, 0), run(1, 1), run(2, 2)) };
        let (accepted, removed, promoted) =
            tokio::time::timeout(std::time::Duration::from_secs(10), all)
                .await
                .expect("no hang: every writer finishes");
        let outcome = [
            accepted.expect("join").expect("accept"),
            removed.expect("join").expect("remove"),
            promoted.expect("join").expect("role set"),
        ];
        // one role per (actor, cord), and the roster agrees with the grants
        let grants: Vec<String> = active_grants(pool, shed.id)
            .await
            .into_iter()
            .filter(|(actor, _)| *actor == target)
            .map(|(_, role)| role)
            .collect();
        assert!(
            grants.len() <= 1,
            "round {round}: {outcome:?} left {grants:?}"
        );
        let listed: Vec<CordRole> = roster(pool, shed.id)
            .await
            .into_iter()
            .filter(|(actor, _)| *actor == target)
            .map(|(_, role)| role)
            .collect();
        assert_eq!(
            listed.iter().map(|r| r.as_str()).collect::<Vec<_>>(),
            grants.iter().map(String::as_str).collect::<Vec<_>>(),
            "round {round}: {outcome:?}"
        );
        // the end state follows from the order the writers landed in: the
        // removal, if it found the actor, took them out; only an accept
        // landing after it can put them back (a role change after the
        // removal finds no one), as a member, or as an admin when the role
        // change landed after that accept
        match outcome {
            // out for good: a join lands after the removal, so it would leave
            // them in
            [a, "removed", _] if grants.is_empty() => assert_eq!(
                a, "already in",
                "round {round}: joined yet left out: {outcome:?}"
            ),
            [_, "removed", _] => {
                assert_eq!(
                    outcome[0], "joined",
                    "round {round}: back in with nothing to bring them back: {outcome:?}"
                );
                let admin_ok = outcome[2] == "promoted" && grants == ["cord_admin"];
                assert!(
                    grants == ["cord_member"] || admin_ok,
                    "round {round}: {outcome:?} left {grants:?}"
                );
            }
            [_, "not in", _] => panic!("round {round}: the removal missed a member: {outcome:?}"),
            _ => unreachable!(),
        }
    }
}

/// A remove racing a purge of the member it removes, who renamed the cord, on
/// real row locks: the remove holds the cord's row and waits on the member's
/// account row, which the purge holds from its first statements. The cord's
/// row names no actor, so the purge's cascade never writes it: the purge
/// commits, then the remove, the member already gone.
async fn scenario_a_purge_of_a_renamer_mid_remove(pool: &Pool) {
    let host = seed_actor(pool, "renamed_host").await;
    let cord = create_cord(pool, "the renamed shed", host).await;
    let renamer = seed_actor(pool, "renamer").await;
    add_member(pool, cord.id, renamer, CordRole::Admin).await;
    in_tx!(pool, |tx| query_cord_rename(
        &tx,
        cord.id,
        "the shed, renamed",
    ))
    .expect("rename")
    .expect("the cord exists");
    let renamer_account: Uuid = {
        let client = pool.get().await.expect("pool.get");
        client
            .query_one("SELECT account_id FROM actor WHERE id = $1", &[&renamer])
            .await
            .expect("the actor")
            .get(0)
    };

    // the purge's lock and the target's account row, held open
    let mut purge_client = pool.get().await.expect("pool.get");
    let purge = purge_client.transaction().await.expect("begin");
    let purge_pid = backend_pid(&purge).await;
    query_account_purge_advisory_lock(&purge)
        .await
        .expect("the purge lock");
    assert!(
        query_account_lock_for_purge(&purge, &renamer_account)
            .await
            .expect("lock the target for purge")
    );

    let removing = tokio::spawn({
        let pool = pool.clone();
        let cord_id = cord.id;
        async move {
            in_tx!(pool, |tx| query_cord_member_remove(
                &tx, cord_id, host, renamer
            ))
        }
    });
    until_blocked_by(
        pool,
        purge_pid,
        "the remove holds the cord and waits on the renamer's account",
    )
    .await;

    let purged = match purge_account_steps(&purge, renamer, renamer_account).await {
        Ok(()) => purge.commit().await,
        Err(e) => Err(e),
    };
    let removed = removing.await.expect("the remove task");
    assert!(purged.is_ok(), "the purge commits: {purged:?}");
    assert!(removed.is_ok(), "the remove commits: {removed:?}");
    assert_eq!(roster(pool, cord.id).await, vec![(host, CordRole::Owner)]);
}

/// Take `sql`'s row lock with `NOWAIT` in a transaction of its own, rolled
/// back: `Err` when another transaction holds a conflicting lock.
async fn try_row_lock(pool: &Pool, sql: &str, id: Uuid) -> Result<(), String> {
    let mut client = pool.get().await.expect("pool.get");
    let tx = client.transaction().await.expect("begin");
    let result = tx
        .query(sql, &[&id])
        .await
        .map(|_| ())
        .map_err(|e| format!("{e:?}"));
    tx.rollback().await.expect("rollback");
    result
}

const CORD_ROW_PROBE: &str = "SELECT 1 FROM cell WHERE id = $1 FOR NO KEY UPDATE NOWAIT";
const LINK_ROW_PROBE: &str = "SELECT 1 FROM cord_invite WHERE id = $1 FOR NO KEY UPDATE NOWAIT";
// the purge's lock, which conflicts with accept's `FOR KEY SHARE`
const ACCOUNT_ROW_PROBE: &str = "SELECT 1 FROM account WHERE id = $1 FOR UPDATE NOWAIT";

/// With the cord's scope lock held exclusive, as a teardown of the cord
/// would hold it while it revokes the cord's grants and then reaches the
/// cord's row, a role set and an accept wait on that lock before taking any
/// row: the cord's row (and accept's link and minter account) stay free to
/// another transaction, so the teardown could take them. Released, both
/// complete.
async fn scenario_the_scope_lock_comes_first(pool: &Pool) {
    let host = seed_actor(pool, "scope_host").await;
    let target = seed_actor(pool, "scope_target").await;
    let joiner = seed_actor(pool, "scope_joiner").await;
    let cord = create_cord(pool, "the scope lock", host).await;
    add_member(pool, cord.id, target, CordRole::Member).await;
    let (token, invite_id) = mint(pool, cord.id, host, None, None, false).await;
    let host_account: Uuid = {
        let client = pool.get().await.expect("pool.get");
        client
            .query_one("SELECT account_id FROM actor WHERE id = $1", &[&host])
            .await
            .expect("the actor")
            .get(0)
    };
    // the teardown's first statement, held open
    let mut teardown_client = pool.get().await.expect("pool.get");
    let teardown = teardown_client.transaction().await.expect("begin");
    query_scope_lock_exclusive(&teardown, cord.id)
        .await
        .expect("the scope lock, exclusive");

    let setting = tokio::spawn({
        let pool = pool.clone();
        let cord_id = cord.id;
        async move {
            in_tx!(pool, |tx| query_cord_member_role_set(
                &tx,
                cord_id,
                host,
                target,
                CordRole::Admin
            ))
        }
    });
    let accepting = tokio::spawn({
        let pool = pool.clone();
        let token_hash = hash_cord_invite_token(&token);
        async move {
            in_tx!(pool, |tx| query_cord_invite_accept(
                &tx,
                &token_hash,
                joiner
            ))
        }
    });
    // both wait on the teardown, so two waiters in the test's database
    until_lock_waiters(
        pool,
        2,
        "the role set and the accept wait on the scope lock",
    )
    .await;
    assert!(!setting.is_finished() && !accepting.is_finished());

    // neither holds a row while it waits
    let cord_row = try_row_lock(pool, CORD_ROW_PROBE, cord.id).await;
    let link_row = try_row_lock(pool, LINK_ROW_PROBE, invite_id).await;
    let account_row = try_row_lock(pool, ACCOUNT_ROW_PROBE, host_account).await;

    teardown.commit().await.expect("release the scope lock");
    let set = setting.await.expect("the role set task");
    let accepted = accepting.await.expect("the accept task");
    assert!(
        cord_row.is_ok(),
        "the cord's row is free while they wait: {cord_row:?}"
    );
    assert!(
        link_row.is_ok(),
        "the link's row is free while accept waits: {link_row:?}"
    );
    assert!(
        account_row.is_ok(),
        "the minter's account is free while accept waits: {account_row:?}"
    );
    assert!(set.is_ok(), "the role set completes: {set:?}");
    assert!(
        accepted.as_ref().is_ok_and(|a| a.role_grant_id.is_some()),
        "the accept joins: {accepted:?}"
    );
    let mut members = roster(pool, cord.id).await;
    members.sort();
    let mut expected = vec![
        (host, CordRole::Owner),
        (target, CordRole::Admin),
        (joiner, CordRole::Member),
    ];
    expected.sort();
    assert_eq!(members, expected);
}

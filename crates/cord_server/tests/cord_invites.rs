//! Postgres-backed tests for cord invite links
//! (`cord_server::invite::queries`).
//!
//! What only the database can show: the table keeps a digest, never the
//! token, and refuses a bad role, a non-positive limit, or a use past the
//! limit; the status a read reports follows revocation, the database
//! clock, and the use count; accepting grants exactly one `cord_member` role
//! and spends one use, an existing member spends none, and concurrent
//! accepts never spend more uses than the link has.
//!
//! `#[ignore]`d (needs Postgres; shares the `schema_ready` suite's database
//! and its `CORD_SCHEMA_READY_TEST_URL` override). It resets the `public`
//! schema, so the scenarios run in order from one test function:
//!
//! ```bash
//! cargo test -p cord_server --test cord_invites -- --ignored
//! ```

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside `#[test]` fns panic on a failed setup or check"
)]

mod common;

use common::{accept, create_cord, fresh_db_pool, mint, seed_actor, status_of};
use cord_server::cord_authz::CordRole;
use cord_server::cord_member_queries::query_cord_members;
use cord_server::cord_queries::CordRow;
use cord_server::invite::queries::{
    CordInviteError, CordInviteStatus, hash_cord_invite_token, query_cord_invite_list,
    query_cord_invite_revoke,
};
use cord_server::roles::CORD_SCOPE_KIND;
use fuz_db::Pool;
use uuid::Uuid;

async fn cord_grant_count(pool: &Pool, cord_id: Uuid, actor_id: Uuid) -> i64 {
    let client = pool.get().await.expect("pool.get");
    client
        .query_one(
            "SELECT COUNT(*) FROM role_grant
             WHERE scope_kind = $1 AND scope_id = $2 AND actor_id = $3 AND revoked_at IS NULL",
            &[&CORD_SCOPE_KIND, &cord_id, &actor_id],
        )
        .await
        .expect("count")
        .get(0)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn invite_links_admit_existing_accounts() {
    let pool = fresh_db_pool().await;
    let owner = seed_actor(&pool, "owner").await;
    let guest = seed_actor(&pool, "guest").await;
    let cord = create_cord(&pool, "the porch", owner).await;

    let (token, invite_id) = scenario_the_stored_row_and_its_guards(&pool, &cord, owner).await;
    scenario_accept_grants_one_role_and_one_use(&pool, &cord, owner, guest, &token).await;
    let third = scenario_an_exhausted_link(&pool, &token, guest).await;
    let revokable_id = scenario_a_revoked_link(&pool, &cord, owner, third).await;
    let expiring_id = scenario_an_expired_link(&pool, &cord, owner, third).await;
    scenario_an_unknown_token(&pool, &cord, third).await;
    scenario_the_list(&pool, &cord, &[expiring_id, revokable_id, invite_id]).await;
    scenario_a_deleted_cord_takes_its_links(&pool, &cord, owner, third).await;
    scenario_a_purged_minter_leaves_the_link_standing(&pool).await;
    scenario_accepts_never_overshoot_max_uses(&pool).await;
}

/// The stored row: a digest, the member role, no newcomer flag; then the
/// table's own guards. Returns the link, with two uses and an hour to live.
async fn scenario_the_stored_row_and_its_guards(
    pool: &Pool,
    cord: &CordRow,
    owner: Uuid,
) -> (String, Uuid) {
    let (token, invite_id) = mint(pool, cord.id, owner, Some(3600), Some(2), false).await;
    let client = pool.get().await.expect("pool.get");
    let row = client
        .query_one(
            "SELECT token_hash, role, admits_newcomers, created_by, use_count,
                    expires_at > NOW() + INTERVAL '59 minutes'
             FROM cord_invite WHERE id = $1",
            &[&invite_id],
        )
        .await
        .expect("row");
    let stored_hash: String = row.get(0);
    assert_eq!(stored_hash, hash_cord_invite_token(&token));
    assert_ne!(stored_hash, token, "the raw token is never stored");
    assert_eq!(row.get::<_, String>(1), "cord_member");
    assert!(!row.get::<_, bool>(2));
    assert_eq!(row.get::<_, Option<Uuid>>(3), Some(owner));
    assert_eq!(row.get::<_, i32>(4), 0);
    assert!(
        row.get::<_, bool>(5),
        "expiry counts from the database's now"
    );
    assert_eq!(status_of(pool, &token).await, CordInviteStatus::Active);

    for (sql, what) in [
        (
            "INSERT INTO cord_invite (cord_id, token_hash, role) VALUES ($1, 'h1', 'cord_admin')",
            "a role other than cord_member",
        ),
        (
            "INSERT INTO cord_invite (cord_id, token_hash, max_uses) VALUES ($1, 'h2', 0)",
            "a zero use limit",
        ),
        (
            "INSERT INTO cord_invite (cord_id, token_hash, max_uses, use_count) VALUES ($1, 'h3', 1, 2)",
            "a use count past the limit",
        ),
    ] {
        assert!(client.execute(sql, &[&cord.id]).await.is_err(), "{what}");
    }
    let dup = client
        .execute(
            "INSERT INTO cord_invite (cord_id, token_hash) VALUES ($1, $2)",
            &[&cord.id, &hash_cord_invite_token(&token)],
        )
        .await;
    assert!(dup.is_err(), "token digests are unique");
    (token, invite_id)
}

/// Accept grants one role and spends one use; accepting again, or the owner
/// accepting, spends nothing and keeps the role they have.
async fn scenario_accept_grants_one_role_and_one_use(
    pool: &Pool,
    cord: &CordRow,
    owner: Uuid,
    guest: Uuid,
    token: &str,
) {
    let accepted = accept(pool, token, guest).await.expect("accept");
    assert_eq!(accepted.cord.id, cord.id);
    assert_eq!(accepted.role, CordRole::Member);
    assert!(accepted.role_grant_id.is_some());
    assert_eq!(cord_grant_count(pool, cord.id, guest).await, 1);
    {
        let client = pool.get().await.expect("pool.get");
        let granted_by: Option<Uuid> = client
            .query_one(
                "SELECT granted_by FROM role_grant WHERE id = $1",
                &[&accepted.role_grant_id.unwrap()],
            )
            .await
            .expect("grant")
            .get(0);
        assert_eq!(granted_by, Some(owner), "the link's minter granted it");
    }

    let again = accept(pool, token, guest).await.expect("accept again");
    assert_eq!((again.role, again.role_grant_id), (CordRole::Member, None));
    let by_owner = accept(pool, token, owner).await.expect("owner accepts");
    assert_eq!(
        (by_owner.role, by_owner.role_grant_id),
        (CordRole::Owner, None)
    );
    assert_eq!(cord_grant_count(pool, cord.id, guest).await, 1);
    assert_eq!(cord_grant_count(pool, cord.id, owner).await, 1);
}

/// The second use exhausts the link; a member still gets their role back.
/// Returns an actor the link refused, for the scenarios after.
async fn scenario_an_exhausted_link(pool: &Pool, token: &str, guest: Uuid) -> Uuid {
    let second = seed_actor(pool, "second").await;
    let third = seed_actor(pool, "third").await;
    accept(pool, token, second).await.expect("second use");
    assert_eq!(status_of(pool, token).await, CordInviteStatus::Exhausted);
    assert!(matches!(
        accept(pool, token, third).await,
        Err(CordInviteError::Exhausted)
    ));
    // a member still gets their role back from a dead link
    assert!(accept(pool, token, guest).await.is_ok());
    third
}

/// Revoking is idempotent and refuses the accept. Returns the link's id.
async fn scenario_a_revoked_link(pool: &Pool, cord: &CordRow, owner: Uuid, third: Uuid) -> Uuid {
    let (revokable, revokable_id) = mint(pool, cord.id, owner, None, None, false).await;
    {
        let client = pool.get().await.expect("pool.get");
        let first = query_cord_invite_revoke(&client, cord.id, revokable_id)
            .await
            .expect("revoke")
            .expect("exists");
        assert!(first.revoked);
        assert!(first.released_signup_invite_ids.is_empty());
        let first_revoked_at = first.invite.invite.revoked_at.clone().expect("revoked_at");
        let again = query_cord_invite_revoke(&client, cord.id, revokable_id)
            .await
            .expect("revoke again")
            .expect("exists");
        assert!(!again.revoked, "a second revoke changes nothing");
        assert_eq!(again.invite.invite.revoked_at, Some(first_revoked_at));
        // another cord's id doesn't reach it
        assert!(
            query_cord_invite_revoke(&client, Uuid::nil(), revokable_id)
                .await
                .expect("revoke")
                .is_none()
        );
    }
    assert_eq!(status_of(pool, &revokable).await, CordInviteStatus::Revoked);
    assert!(matches!(
        accept(pool, &revokable, third).await,
        Err(CordInviteError::Revoked)
    ));
    revokable_id
}

/// Expiry follows the database clock. Returns the link's id.
async fn scenario_an_expired_link(pool: &Pool, cord: &CordRow, owner: Uuid, third: Uuid) -> Uuid {
    let (expiring, expiring_id) = mint(pool, cord.id, owner, Some(60), None, false).await;
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "UPDATE cord_invite SET expires_at = NOW() - INTERVAL '1 second' WHERE id = $1",
                &[&expiring_id],
            )
            .await
            .expect("backdate");
    }
    assert_eq!(status_of(pool, &expiring).await, CordInviteStatus::Expired);
    assert!(matches!(
        accept(pool, &expiring, third).await,
        Err(CordInviteError::Expired)
    ));
    expiring_id
}

async fn scenario_an_unknown_token(pool: &Pool, cord: &CordRow, third: Uuid) {
    assert!(matches!(
        accept(pool, "not a real token", third).await,
        Err(CordInviteError::NotFound)
    ));
    assert_eq!(cord_grant_count(pool, cord.id, third).await, 0);
}

/// The list is newest first, with statuses as read: `expected` is the
/// expired, revoked, and exhausted links' ids, newest first.
async fn scenario_the_list(pool: &Pool, cord: &CordRow, expected: &[Uuid; 3]) {
    let client = pool.get().await.expect("pool.get");
    let listed: Vec<(Uuid, CordInviteStatus)> = query_cord_invite_list(&client, cord.id)
        .await
        .expect("list")
        .iter()
        .map(|r| (r.invite.id, r.invite.status()))
        .collect();
    assert_eq!(
        listed,
        vec![
            (expected[0], CordInviteStatus::Expired),
            (expected[1], CordInviteStatus::Revoked),
            (expected[2], CordInviteStatus::Exhausted),
        ]
    );
}

async fn scenario_a_deleted_cord_takes_its_links(
    pool: &Pool,
    cord: &CordRow,
    owner: Uuid,
    third: Uuid,
) {
    let (orphan, _) = mint(pool, cord.id, owner, None, None, false).await;
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute(
                "UPDATE cell SET deleted_at = NOW() WHERE id = $1",
                &[&cord.id],
            )
            .await
            .expect("soft-delete the cord");
    }
    assert!(
        matches!(
            accept(pool, &orphan, third).await,
            Err(CordInviteError::NotFound)
        ),
        "a link to a deleted cord is unknown"
    );
    let client = pool.get().await.expect("pool.get");
    client
        .execute("DELETE FROM cell WHERE id = $1", &[&cord.id])
        .await
        .expect("hard-delete the cord");
    let left: i64 = client
        .query_one("SELECT COUNT(*) FROM cord_invite", &[])
        .await
        .expect("count")
        .get(0);
    assert_eq!(left, 0, "links cascade with their cord");
}

async fn scenario_a_purged_minter_leaves_the_link_standing(pool: &Pool) {
    let minter = seed_actor(pool, "minter").await;
    let joiner = seed_actor(pool, "joiner").await;
    let kept = create_cord(pool, "the yard", minter).await;
    let (kept_token, kept_id) = mint(pool, kept.id, minter, None, None, false).await;
    {
        let client = pool.get().await.expect("pool.get");
        client
            .execute("DELETE FROM role_grant WHERE actor_id = $1", &[&minter])
            .await
            .expect("drop the minter's grants");
        client
            .execute("DELETE FROM actor WHERE id = $1", &[&minter])
            .await
            .expect("purge the minter");
        let created_by: Option<Uuid> = client
            .query_one(
                "SELECT created_by FROM cord_invite WHERE id = $1",
                &[&kept_id],
            )
            .await
            .expect("row")
            .get(0);
        assert_eq!(created_by, None);
    }
    let joined = accept(pool, &kept_token, joiner).await.expect("accept");
    assert_eq!(joined.role, CordRole::Member);
}

async fn scenario_accepts_never_overshoot_max_uses(pool: &Pool) {
    let host = seed_actor(pool, "host").await;
    let busy = create_cord(pool, "the kitchen", host).await;
    let max_uses = 3;
    let (busy_token, busy_id) = mint(pool, busy.id, host, None, Some(max_uses), false).await;
    let mut guests = Vec::new();
    for i in 0..10 {
        guests.push(seed_actor(pool, &format!("rush_{i}")).await);
    }
    let handles: Vec<_> = guests
        .iter()
        .map(|&actor| {
            let pool = pool.clone();
            let token = busy_token.clone();
            tokio::spawn(async move { accept(&pool, &token, actor).await.map(|_| ()) })
        })
        .collect();
    let mut joined = 0;
    let mut exhausted = 0;
    for handle in handles {
        match handle.await.expect("join") {
            Ok(()) => joined += 1,
            Err(CordInviteError::Exhausted) => exhausted += 1,
            Err(e) => panic!("unexpected: {e:?}"),
        }
    }
    assert_eq!((joined, exhausted), (3, 7));
    let client = pool.get().await.expect("pool.get");
    let use_count: i32 = client
        .query_one(
            "SELECT use_count FROM cord_invite WHERE id = $1",
            &[&busy_id],
        )
        .await
        .expect("row")
        .get(0);
    assert_eq!(use_count, 3);
    let members = query_cord_members(&client, busy.id).await.expect("members");
    assert_eq!(members.len(), 4, "the host and three guests");
}

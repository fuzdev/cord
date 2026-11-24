//! Shared setup for the Postgres-backed integration tests: a freshly
//! migrated database, seeded actors, and the cord and invite-link steps the
//! suites take, each run in its own transaction as the actions do.
//!
//! Each test opens a database of its own ([`fresh_db`], or
//! [`fresh_postgres_db`] for a test of what only PostgreSQL has), through the
//! spine's `fuz_testing::fresh_test_db`: an in-process pglet server by
//! default, a created and dropped PostgreSQL database under
//! `FUZ_TEST_POSTGRES_URL`. That and the identity seeding are the spine's
//! shared ones in `fuz_testing` (its `fresh_db` and `seed` modules); the
//! cord steps are this module's.

// `allow`, not `expect`: this module is compiled into every test binary, and
// which helpers go unused differs from one binary to the next.
#![allow(dead_code, reason = "each test binary uses only part of this module")]
#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "test helpers outside `#[test]` fns panic on a failed setup or check"
)]

use std::fmt::Debug;
use std::time::{Duration, Instant};

use cord_server::cord_authz::CordRole;
use cord_server::cord_queries::{CordRow, query_cord_create};
use cord_server::invite::queries::{
    CordInviteAccepted, CordInviteError, CordInviteStatus, generate_cord_invite_token,
    hash_cord_invite_token, query_cord_invite_accept, query_cord_invite_by_token_hash,
    query_cord_invite_create,
};
use cord_server::invite::redemption_queries::{
    CORD_INVITE_REDEMPTION_COLUMNS, CordInviteRedeemed, CordInviteRedemptionRow,
    query_cord_invite_redeem,
};
use cord_server::roles::CORD_SCOPE_KIND;
use cord_server::router::cord_migration_namespaces;
use fuz_auth::account_queries::{query_account_lock_for_purge, query_purge_account};
use fuz_auth::invite_queries::query_invite_delete_unclaimed_by_creators;
use fuz_auth::query_create_role_grant;
use fuz_db::{Pool, col, iso8601_timestamp_expr, qualify_columns};
use fuz_testing::{TestDb, TestEngine, TestingDbOptions, fresh_test_db, seed_account_actor};
use uuid::Uuid;

/// The suites' database: the chain `build_router` runs, no scratch DDL.
pub const DB: TestingDbOptions<'static> = TestingDbOptions {
    suite: "cord_server",
    migrations: &cord_migration_namespaces(),
    setup_sql: None,
    engine: TestEngine::Any,
};

/// Run `$body` in its own transaction bound to `$tx`, committing on `Ok`
/// and rolling back on `Err`, as the action dispatcher does.
macro_rules! in_tx {
    ($pool:expr, |$tx:ident| $body:expr) => {{
        let mut client = $pool.get().await.expect("pool.get");
        let $tx = client.transaction().await.expect("begin");
        let result = $body;
        if result.is_ok() {
            $tx.commit().await.expect("commit");
        } else {
            $tx.rollback().await.expect("rollback");
        }
        result
    }};
}

// `allow`, not `expect`: as for `dead_code` above; only some binaries call `in_tx!`.
#[allow(
    unused_imports,
    reason = "each test binary uses only part of this module"
)]
pub(crate) use in_tx;

/// A freshly migrated database of the test's own.
pub async fn fresh_db() -> TestDb {
    fresh_test_db(DB).await
}

/// [`fresh_db`] on PostgreSQL alone, for a test of what only PostgreSQL has.
pub async fn fresh_postgres_db() -> TestDb {
    fresh_test_db(DB.postgres_only()).await
}

/// Create a cord owned by `owner`.
pub async fn create_cord(pool: &Pool, name: &str, owner: Uuid) -> CordRow {
    let client = pool.get().await.expect("pool.get");
    query_cord_create(&client, name, owner)
        .await
        .expect("create cord")
        .cord
}

/// Give `actor_id` the cord role `role` on the cord `cord_id`, as a direct
/// grant rather than through Cord's verbs.
pub async fn add_member(pool: &Pool, cord_id: Uuid, actor_id: Uuid, role: CordRole) {
    let client = pool.get().await.expect("pool.get");
    query_create_role_grant(
        &client,
        &actor_id,
        role.as_str(),
        Some(CORD_SCOPE_KIND),
        Some(&cord_id),
        None,
        None,
    )
    .await
    .expect("grant");
}

/// Assert `result` was refused with the error whose `Debug` is `expected`.
pub fn assert_refused<T: Debug, E: Debug>(result: Result<T, E>, expected: &str) {
    match result {
        Err(e) => assert_eq!(format!("{e:?}"), expected),
        Ok(ok) => panic!("expected {expected}, got {ok:?}"),
    }
}

/// Mint a link on the cord `cord_id` as `by`, returning its raw token and id.
pub async fn mint(
    pool: &Pool,
    cord_id: Uuid,
    by: Uuid,
    expires_in_seconds: Option<u32>,
    max_uses: Option<u32>,
    admits_newcomers: bool,
) -> (String, Uuid) {
    let client = pool.get().await.expect("pool.get");
    let minted = generate_cord_invite_token();
    let created = query_cord_invite_create(
        &client,
        cord_id,
        &minted.token_hash,
        by,
        expires_in_seconds,
        max_uses,
        admits_newcomers,
    )
    .await
    .expect("create invite");
    (minted.token, created.invite.id)
}

/// Accept `token` as `actor_id` in its own transaction, as the action does.
pub async fn accept(
    pool: &Pool,
    token: &str,
    actor_id: Uuid,
) -> Result<CordInviteAccepted, CordInviteError> {
    in_tx!(pool, |tx| query_cord_invite_accept(
        &tx,
        &hash_cord_invite_token(token),
        actor_id
    )
    .await)
}

/// Redeem `token` for `username` in its own transaction, as the action does.
pub async fn redeem(
    pool: &Pool,
    token: &str,
    username: &str,
) -> Result<CordInviteRedeemed, CordInviteError> {
    in_tx!(pool, |tx| query_cord_invite_redeem(
        &tx,
        &hash_cord_invite_token(token),
        username
    )
    .await)
}

/// The status of the link `token` as a read reports it now.
pub async fn status_of(pool: &Pool, token: &str) -> CordInviteStatus {
    let client = pool.get().await.expect("pool.get");
    query_cord_invite_by_token_hash(&client, &hash_cord_invite_token(token))
        .await
        .expect("read")
        .expect("exists")
        .status()
}

/// Seed an account and its actor, returning the actor id.
pub async fn seed_actor(pool: &Pool, username: &str) -> Uuid {
    seed_account_actor(pool, username).await.1
}

/// Every redemption of the link `cord_invite_id`, oldest first. The server
/// never lists them (the link list carries counts), so the read lives here.
pub async fn redemption_list(pool: &Pool, cord_invite_id: Uuid) -> Vec<CordInviteRedemptionRow> {
    let columns = &CORD_INVITE_REDEMPTION_COLUMNS;
    let projection = qualify_columns(
        columns,
        "r",
        iso8601_timestamp_expr(columns, &["created_at", "accepted_at"], "r"),
    );
    let client = pool.get().await.expect("pool.get");
    client
        .query(
            &format!(
                "SELECT {projection} FROM cord_invite_redemption r
                 WHERE r.cord_invite_id = $1
                 ORDER BY r.created_at, r.id"
            ),
            &[&cord_invite_id],
        )
        .await
        .expect("list redemptions")
        .iter()
        .map(|row| CordInviteRedemptionRow {
            id: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "id")),
            cord_invite_id: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "cord_invite_id")),
            signup_invite_id: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "signup_invite_id")),
            username: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "username")),
            created_at: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "created_at")),
            accepted_by: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "accepted_by")),
            accepted_at: row.get(col!(CORD_INVITE_REDEMPTION_COLUMNS, "accepted_at")),
        })
        .collect()
}

/// Backends of this database waiting on a lock.
pub async fn lock_waiters(pool: &Pool) -> i64 {
    let client = pool.get().await.expect("pool.get");
    client
        .query_one(
            "SELECT COUNT(*) FROM pg_stat_activity
             WHERE wait_event_type = 'Lock' AND datname = current_database()",
            &[],
        )
        .await
        .expect("pg_stat_activity")
        .get(0)
}

/// Wait until `n` backends of this database wait on a lock.
pub async fn await_lock_waiters(pool: &Pool, n: i64, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while lock_waiters(pool).await < n {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Wait until `n` backends of this database wait on a lock, or `task` has
/// finished without waiting.
pub async fn await_lock_waiters_or_finished<T>(
    pool: &Pool,
    n: i64,
    task: &tokio::task::JoinHandle<T>,
    what: &str,
) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !task.is_finished() && lock_waiters(pool).await < n {
        assert!(Instant::now() < deadline, "{what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The steps of the spine's `account_purge` that reach the purged actor's
/// rows, after its advisory lock and last-admin guard (the caller's): the
/// account's `FOR UPDATE`, the invite void, then the delete, whose cascade
/// deletes the actor and nulls every `ON DELETE SET NULL` column naming it
/// (a link's `created_by`).
pub async fn purge_account_steps(
    tx: &deadpool_postgres::Transaction<'_>,
    actor: Uuid,
    account: Uuid,
) -> Result<(), fuz_db::tokio_postgres::Error> {
    query_account_lock_for_purge(tx, &account).await?;
    query_invite_delete_unclaimed_by_creators(tx, &[actor]).await?;
    query_purge_account(tx, &account).await?;
    Ok(())
}

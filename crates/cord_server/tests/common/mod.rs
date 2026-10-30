//! Shared setup for the Postgres-backed integration tests: a freshly
//! migrated database, seeded actors, and the cord and invite-link steps the
//! suites take, each run in its own transaction as the actions do.
//!
//! Every suite here but `transaction_isolation` (which boots on its own
//! scratch database) drops and recreates the
//! `public` schema of the one test database (`CORD_SCHEMA_READY_TEST_URL`,
//! default `postgres://localhost/cord_schema_ready_test`), so each suite keeps
//! its scenarios in a single test function; cargo runs the test binaries one
//! at a time. That cycle and the identity seeding are the spine's shared ones
//! in `fuz_testing` (`fresh_db`, `seed`); the cord steps are this module's.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

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
use cord_server::router::cord_migration_namespaces;
use fuz_db::{Pool, col, iso8601_timestamp_expr, qualify_columns};
use fuz_testing::{TestingDbOptions, fresh_test_db_pool, seed_account_actor};
use uuid::Uuid;

/// The suites' database: `CORD_SCHEMA_READY_TEST_URL` (default
/// `postgres://localhost/cord_schema_ready_test`) over the chain
/// `build_router` runs, no scratch DDL.
pub const DB: TestingDbOptions<'static> = TestingDbOptions {
    env_var: "CORD_SCHEMA_READY_TEST_URL",
    default_db: "cord_schema_ready_test",
    migrations: &cord_migration_namespaces(),
    setup_sql: None,
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

#[allow(unused_imports)]
pub(crate) use in_tx;

/// A pool over a freshly reset and migrated database.
pub async fn fresh_db_pool() -> Pool {
    fresh_test_db_pool(DB).await
}

/// Create a cord owned by `owner`.
pub async fn create_cord(pool: &Pool, name: &str, owner: Uuid) -> CordRow {
    let client = pool.get().await.expect("pool.get");
    query_cord_create(&client, name, owner)
        .await
        .expect("create cord")
        .cord
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

//! Shared setup for the Postgres-backed integration tests: a freshly
//! migrated database and seeded actors.
//!
//! Every suite here drops and recreates the `public` schema of the one test
//! database (`CORD_SCHEMA_READY_TEST_URL`, default
//! `postgres://localhost/cord_schema_ready_test`), so each suite keeps its
//! scenarios in a single test function; cargo runs the test binaries one at
//! a time.

#![allow(clippy::unwrap_used, clippy::expect_used, dead_code)]

use cord_server::router::cord_migration_namespaces;
use fuz_db::{Pool, create_pool, run_migrations};
use uuid::Uuid;

pub fn test_url() -> String {
    std::env::var("CORD_SCHEMA_READY_TEST_URL")
        .unwrap_or_else(|_| "postgres://localhost/cord_schema_ready_test".to_owned())
}

/// A pool over a freshly reset and migrated database.
pub async fn fresh_db_pool() -> Pool {
    let pool = create_pool(&test_url()).expect("create_pool");
    {
        let client = pool.get().await.expect("pool.get");
        client
            .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public")
            .await
            .expect("reset schema");
    }
    run_migrations(&pool, &cord_migration_namespaces())
        .await
        .expect("migrations");
    pool
}

/// Seed an account and its actor, returning the actor id.
pub async fn seed_actor(pool: &Pool, username: &str) -> Uuid {
    let client = pool.get().await.expect("pool.get");
    let account_id: Uuid = client
        .query_one(
            "INSERT INTO account (username, password_hash) VALUES ($1, 'x') RETURNING id",
            &[&username],
        )
        .await
        .expect("insert account")
        .get(0);
    client
        .query_one(
            "INSERT INTO actor (account_id, name) VALUES ($1, $2) RETURNING id",
            &[&account_id, &username],
        )
        .await
        .expect("insert actor")
        .get(0)
}

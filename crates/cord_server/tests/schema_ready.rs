//! Schema-readiness fixture and drift check for `cord_server`.
//!
//! Generates and guards the committed `expected_schema.json` that the `/ready`
//! probe embeds (`router.rs` `include_str!`s it). Bootstraps cord's migration
//! chain via `cord_migration_namespaces()` (the exact chain `build_router`
//! runs) on a real Postgres, introspects the column map via
//! `fuz_db::query_ready_columns` (which keeps the `schema_version` tracker),
//! then either rewrites the fixture (`UPDATE_SCHEMA_READY=1`) or asserts the
//! committed copy still matches a fresh bootstrap.
//!
//! `#[ignore]`d so `cargo test --workspace` passes without Postgres. Run it
//! explicitly (it drops and recreates the `public` schema of its database):
//!
//! ```bash
//! createdb cord_schema_ready_test
//! # regenerate after an intentional schema change:
//! UPDATE_SCHEMA_READY=1 cargo test -p cord_server --test schema_ready -- --ignored
//! # verify (the drift guard):
//! cargo test -p cord_server --test schema_ready -- --ignored
//! ```
//!
//! Override the connection URL with `CORD_SCHEMA_READY_TEST_URL`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;
use std::env;
use std::path::PathBuf;

use cord_server::router::cord_migration_namespaces;
use fuz_db::{check_schema_drift, create_pool, query_ready_columns, run_migrations};

fn test_database_url() -> String {
    env::var("CORD_SCHEMA_READY_TEST_URL")
        .unwrap_or_else(|_| "postgres://localhost/cord_schema_ready_test".to_owned())
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected_schema.json")
}

#[tokio::test]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn committed_expected_schema_matches_fresh_bootstrap() {
    let pool = create_pool(&test_database_url()).expect("test pool build");

    {
        let client = pool.get().await.expect("pool get");
        client
            .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
            .await
            .expect("reset public schema");
    }
    run_migrations(&pool, &cord_migration_namespaces())
        .await
        .expect("run migrations");

    let client = pool.get().await.expect("pool get");
    let live: BTreeMap<String, Vec<String>> = query_ready_columns(&client, "public")
        .await
        .expect("query ready columns");

    // `schema_version` must be present so a never-migrated DB fails readiness
    assert!(
        live.contains_key("schema_version"),
        "query_ready_columns keeps the schema_version tracker"
    );

    let path = fixture_path();
    if env::var("UPDATE_SCHEMA_READY").as_deref() == Ok("1") {
        let json = serde_json::to_string_pretty(&live).expect("serialize fixture") + "\n";
        std::fs::write(&path, json).expect("write fixture");
    }

    let raw = std::fs::read_to_string(&path).expect("read committed expected_schema.json");
    let committed: BTreeMap<String, Vec<String>> =
        serde_json::from_str(&raw).expect("parse committed expected_schema.json");
    assert_eq!(
        live, committed,
        "expected_schema.json drifted from a fresh bootstrap; regenerate with UPDATE_SCHEMA_READY=1"
    );

    // the drift checker accepts the live schema and flags a missing column
    assert!(check_schema_drift(&live, &committed).ok);
    let mut absent = committed;
    absent
        .entry("account".to_owned())
        .or_default()
        .push("nonexistent_col".to_owned());
    assert!(!check_schema_drift(&live, &absent).ok);
}

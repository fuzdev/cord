//! Schema-readiness fixture and drift check for `cord_server`.
//!
//! Generates and guards the committed `expected_schema.json` that the `/ready`
//! probe embeds (`router.rs` `include_str!`s it). Bootstraps cord's migration
//! chain via `cord_migration_namespaces()` (the exact chain `build_router`
//! runs) on a real Postgres, then the spine's shared
//! `fuz_testing::assert_expected_schema_fixture` either rewrites the fixture
//! (`UPDATE_SCHEMA_READY=1`) or asserts the committed copy still matches a
//! fresh bootstrap.
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

mod common;

use std::path::PathBuf;

use common::fresh_db_pool;
use fuz_testing::assert_expected_schema_fixture;

#[tokio::test]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn committed_expected_schema_matches_fresh_bootstrap() {
    // the exact chain `build_router` runs, on a freshly reset schema
    let pool = fresh_db_pool().await;
    assert_expected_schema_fixture(
        &pool,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected_schema.json"),
    )
    .await;
}

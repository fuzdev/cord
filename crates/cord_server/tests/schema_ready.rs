//! Schema-readiness fixture and drift check for `cord_server`.
//!
//! Generates and guards the committed `expected_schema.json` that the `/ready`
//! probe embeds (`router.rs` `include_str!`s it). Bootstraps cord's migration
//! chain via `cord_migration_namespaces()` (the exact chain `build_router`
//! runs) on a fresh database, then the spine's shared
//! `fuz_testing::assert_expected_schema_fixture` either rewrites the fixture
//! (`UPDATE_SCHEMA_READY=1`) or asserts the committed copy still matches a
//! fresh bootstrap.
//!
//! It runs in plain `cargo test` over a database of its own
//! (`common::fresh_db`: pglet by default, PostgreSQL under
//! `FUZ_TEST_POSTGRES_URL`); the column map is the same on either engine.
//!
//! ```bash
//! cargo test -p cord_server --test schema_ready
//! # regenerate after an intentional schema change:
//! UPDATE_SCHEMA_READY=1 cargo test -p cord_server --test schema_ready
//! ```

mod common;

use std::path::PathBuf;

use common::fresh_db;
use fuz_testing::assert_expected_schema_fixture;

#[tokio::test]
async fn committed_expected_schema_matches_fresh_bootstrap() {
    // the exact chain `build_router` runs, on a freshly reset schema
    let pool = fresh_db().await;
    assert_expected_schema_fixture(
        &pool,
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected_schema.json"),
    )
    .await;
}

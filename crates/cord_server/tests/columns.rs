//! Registry guard for the per-table projection constants over the chain
//! `cord_server` bootstraps: every live table is either a `*_COLUMNS` const
//! naming exactly its live columns or a reasoned exemption.
//!
//! Composes the spine chains' exported registries (the `fuz_db` tracker,
//! `fuz_auth`, and `fuz_cell`) with cord's own
//! (`cord_server::migrations::CORD_COLUMN_PROJECTIONS`), so a new cord table
//! fails here until it's registered there.
//!
//! `#[ignore]`d (needs Postgres; shares the `schema_ready` suite's database
//! and `CORD_SCHEMA_READY_TEST_URL` override):
//!
//! ```bash
//! cargo test -p cord_server --test columns -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

use cord_server::migrations::CORD_COLUMN_PROJECTIONS;
use cord_server::router::cord_migration_namespaces;
use fuz_auth::AUTH_COLUMN_PROJECTIONS;
use fuz_cell::CELL_COLUMN_PROJECTIONS;
use fuz_db::{
    DB_COLUMN_PROJECTIONS, column_projection_mismatches_merged, create_pool, query_ready_columns,
    run_migrations,
};

fn test_url() -> String {
    std::env::var("CORD_SCHEMA_READY_TEST_URL")
        .unwrap_or_else(|_| "postgres://localhost/cord_schema_ready_test".to_owned())
}

#[tokio::test]
#[ignore = "requires PostgreSQL with the cord_schema_ready_test database"]
async fn columns_match_live_schema() {
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
    let client = pool.get().await.expect("pool.get");
    let live = query_ready_columns(&client, "public")
        .await
        .expect("query_ready_columns");
    let mismatches = column_projection_mismatches_merged(
        &live,
        &[
            DB_COLUMN_PROJECTIONS,
            AUTH_COLUMN_PROJECTIONS,
            CELL_COLUMN_PROJECTIONS,
            CORD_COLUMN_PROJECTIONS,
        ],
    );
    assert!(mismatches.is_empty(), "{}", mismatches.join("\n"));
}

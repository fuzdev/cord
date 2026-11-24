//! Registry guard for the per-table projection constants over the chain
//! `cord_server` bootstraps: every live table is either a `*_COLUMNS` const
//! naming exactly its live columns or a reasoned exemption.
//!
//! Composes the spine chains' exported registries (the `fuz_db` tracker,
//! `fuz_auth`, and `fuz_cell`) with cord's own
//! (`cord_server::migrations::CORD_COLUMN_PROJECTIONS`), so a new cord table
//! fails here until it's registered there. The assertion over them is the
//! spine's shared `fuz_testing::assert_column_projections`.
//!
//! It runs over a database of its own (`common::fresh_db`).

mod common;

use common::fresh_db;
use cord_server::migrations::CORD_COLUMN_PROJECTIONS;
use fuz_auth::AUTH_COLUMN_PROJECTIONS;
use fuz_cell::CELL_COLUMN_PROJECTIONS;
use fuz_db::DB_COLUMN_PROJECTIONS;
use fuz_testing::assert_column_projections;

#[tokio::test]
async fn columns_match_live_schema() {
    let pool = fresh_db().await;
    assert_column_projections(
        &pool,
        &[
            DB_COLUMN_PROJECTIONS,
            AUTH_COLUMN_PROJECTIONS,
            CELL_COLUMN_PROJECTIONS,
            CORD_COLUMN_PROJECTIONS,
        ],
    )
    .await;
}

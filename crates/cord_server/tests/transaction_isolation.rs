//! The boot check that Postgres runs transactions at READ COMMITTED, against
//! a live database: the default passes, and a database whose
//! `default_transaction_isolation` an operator set to `repeatable read` is
//! refused.
//!
//! The refusal sets the database's own default (`ALTER DATABASE ... SET`)
//! and resets it before asserting, so a failure leaves the test database as
//! it found it. It needs no schema, so it doesn't reset `public`.
//!
//! ```bash
//! cargo test -p cord_server --test transaction_isolation -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use cord_server::error::StartupError;
use cord_server::transaction_isolation::{
    REQUIRED_TRANSACTION_ISOLATION, ensure_read_committed, query_default_transaction_isolation,
};
use fuz_db::create_pool;

use common::test_url;

/// Set (or, with `None`, reset) the test database's own
/// `default_transaction_isolation`. It reaches only connections opened
/// afterwards.
async fn set_database_isolation(value: Option<&str>) {
    let pool = create_pool(&test_url()).expect("create_pool");
    let client = pool.get().await.expect("pool.get");
    let database: String = client
        .query_one("SELECT current_database()", &[])
        .await
        .expect("current_database")
        .get(0);
    let database = format!("\"{}\"", database.replace('"', "\"\""));
    let sql = value.map_or_else(
        || format!("ALTER DATABASE {database} RESET default_transaction_isolation"),
        |value| format!("ALTER DATABASE {database} SET default_transaction_isolation = '{value}'"),
    );
    client.batch_execute(&sql).await.expect("alter database");
}

#[tokio::test]
#[ignore = "requires Postgres at CORD_SCHEMA_READY_TEST_URL"]
async fn the_boot_check_refuses_anything_but_read_committed() {
    // a clean slate, whatever an interrupted run left
    set_database_isolation(None).await;

    let pool = create_pool(&test_url()).expect("create_pool");
    let client = pool.get().await.expect("pool.get");
    assert_eq!(
        query_default_transaction_isolation(&client)
            .await
            .expect("reads the setting"),
        REQUIRED_TRANSACTION_ISOLATION,
        "the Postgres default"
    );
    drop(client);
    ensure_read_committed(&pool)
        .await
        .expect("the default passes");

    set_database_isolation(Some("repeatable read")).await;
    // a fresh pool: the database default reaches only new connections
    let refused = ensure_read_committed(&create_pool(&test_url()).expect("create_pool")).await;
    set_database_isolation(None).await;

    let err = refused.expect_err("repeatable read is refused");
    assert!(
        matches!(&err, StartupError::TransactionIsolation { found } if found == "repeatable read"),
        "{err}"
    );
    assert_eq!(err.exit_code(), 2);

    let pool = create_pool(&test_url()).expect("create_pool");
    ensure_read_committed(&pool)
        .await
        .expect("the reset restores the default");
}

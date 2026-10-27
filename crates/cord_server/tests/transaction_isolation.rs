//! `cordd` refuses a database whose `default_transaction_isolation` isn't
//! `read committed` — the membership serialization's precondition — and
//! refuses it **before migrating**, so a misconfigured database is never
//! written to.
//!
//! The boot runs on its own scratch database
//! (`fuz_testing::ScratchDb`, `cord_isolation_scratch`), since the setting is
//! database-wide and would leak into every suite sharing the test database.
//! The check itself is the spine's (`fuz_actions::server`), tested there.
//!
//! ```bash
//! cargo test -p cord_server --test transaction_isolation -- --ignored
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used)]

mod common;

use std::sync::Arc;

use fuz_actions::server::{SpineBackendOptions, StartupError};
use fuz_db::create_pool;
use fuz_testing::ScratchDb;

#[tokio::test]
#[ignore = "requires Postgres at CORD_SCHEMA_READY_TEST_URL"]
async fn the_boot_refuses_anything_but_read_committed_before_migrating() {
    let scratch = ScratchDb::create(common::DB, "cord_isolation_scratch").await;
    create_pool(scratch.url())
        .expect("create_pool")
        .get()
        .await
        .expect("pool.get")
        .batch_execute(
            "ALTER DATABASE cord_isolation_scratch \
             SET default_transaction_isolation = 'repeatable read'",
        )
        .await
        .expect("alter database");

    let url = scratch.url().to_owned();
    let env = fuz_http::BaseServerEnv::from_vars(
        &|k| match k {
            "DATABASE_URL" => Some(url.clone()),
            "SECRET_FUZ_COOKIE_KEYS" => Some("k".repeat(fuz_auth::keyring::MIN_KEY_LENGTH)),
            "FUZ_ALLOWED_ORIGINS" => Some("http://localhost".to_owned()),
            _ => None,
        },
        cord_server::runtime::ENV_OPTIONS,
    )
    .expect("the env reads");
    let refused = cord_server::router::build_router(
        &env,
        SpineBackendOptions::production(Arc::new(fuz_auth::Argon2idHasher::new())),
        None,
    )
    .await;

    let migrated: bool = create_pool(scratch.url())
        .expect("create_pool")
        .get()
        .await
        .expect("pool.get")
        .query_one(
            "SELECT to_regclass('public.schema_version') IS NOT NULL",
            &[],
        )
        .await
        .expect("query the tracker")
        .get(0);
    scratch.drop_database().await;

    let err = refused.expect_err("repeatable read is refused");
    assert!(
        matches!(&err, StartupError::TransactionIsolation { found } if found == "repeatable read"),
        "{err}"
    );
    assert_eq!(err.exit_code(), 2);
    assert!(!migrated, "the refusal comes before any migration");
}

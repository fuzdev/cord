//! `cordd` refuses a database whose `default_transaction_isolation` isn't
//! `read committed` — the membership serialization's precondition — and
//! refuses it **before migrating**, so a misconfigured database is never
//! written to.
//!
//! The setting is database-level (`ALTER DATABASE … SET`), which only
//! PostgreSQL has, so the test runs on PostgreSQL alone, over a database of
//! its own. The check itself is the spine's (`fuz_actions::server`), tested
//! there.

mod common;

use std::sync::Arc;

use fuz_actions::server::{SpineBackendOptions, StartupError};
use fuz_db::create_pool;
use fuz_testing::{TestingDbOptions, fresh_test_db};

#[tokio::test]
#[ignore = "postgres-only: a database-level `ALTER DATABASE … SET`"]
async fn the_boot_refuses_anything_but_read_committed_before_migrating() {
    // bare: the boot under test runs the chain, so nothing is migrated first
    let db = fresh_test_db(TestingDbOptions {
        migrations: &[],
        ..common::DB.postgres_only()
    })
    .await;
    let client = db.get().await.expect("pool.get");
    let name: String = client
        .query_one("SELECT current_database()", &[])
        .await
        .expect("read the database name")
        .get(0);
    client
        .batch_execute(&format!(
            "ALTER DATABASE {name} SET default_transaction_isolation = 'repeatable read'"
        ))
        .await
        .expect("alter database");
    drop(client);

    let url = db.url().to_owned();
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
        SpineBackendOptions::production(
            Arc::new(fuz_auth::Argon2idHasher::new()),
            fuz_auth::AuthCleanupSchedule::Off,
        ),
        None,
        None,
    )
    .await;

    let migrated: bool = create_pool(db.url(), &fuz_db::PoolOptions::default())
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

    let err = refused.expect_err("repeatable read is refused");
    assert!(
        matches!(&err, StartupError::TransactionIsolation { found } if found == "repeatable read"),
        "{err}"
    );
    assert_eq!(err.exit_code(), 2);
    assert!(!migrated, "the refusal comes before any migration");
}

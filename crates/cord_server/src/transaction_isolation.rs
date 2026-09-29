//! The boot check that Postgres runs cord's transactions at READ COMMITTED.
//!
//! Cord's membership serialization ([`crate::cord_member_queries::query_cord_lock`])
//! relies on READ COMMITTED: each statement after the lock wait takes a fresh
//! snapshot, so the membership read that follows sees what the previous
//! holder committed. The action dispatcher opens its transactions with a
//! plain `BEGIN`, so they run at the database's
//! `default_transaction_isolation`. Postgres defaults it to `read committed`,
//! but an operator can change it in `postgresql.conf`, per database
//! (`ALTER DATABASE ... SET`), or per role (`ALTER ROLE ... SET`); under
//! `repeatable read` or `serializable`, two owners stepping down together
//! could both pass the last-owner rule. So [`crate::runtime::run_app`]
//! refuses to serve unless the setting reads exactly `read committed`
//! ([`ensure_read_committed`]).
//!
//! `read uncommitted` is refused too, though Postgres runs it as READ
//! COMMITTED: it names a level cord doesn't run at, a Postgres-compatible
//! engine might honor it, and the fix, dropping the setting, costs nothing.
//!
//! The check runs once, at boot. A setting changed while `cordd` runs
//! reaches the pool's new connections unchecked until the next restart.

use deadpool_postgres::GenericClient;
use fuz_db::{DbError, Pool};

use crate::error::StartupError;

/// The one `default_transaction_isolation` value `cordd` serves under.
pub const REQUIRED_TRANSACTION_ISOLATION: &str = "read committed";

/// The session's `default_transaction_isolation`, as `SHOW` reports it
/// (lowercase, e.g. `read committed`).
///
/// # Errors
///
/// [`DbError::Postgres`] on a driver failure.
pub async fn query_default_transaction_isolation<C: GenericClient + ?Sized>(
    client: &C,
) -> Result<String, DbError> {
    let row = client
        .query_one(
            "SELECT current_setting('default_transaction_isolation')",
            &[],
        )
        .await?;
    Ok(row.get(0))
}

/// Whether `value`, a `default_transaction_isolation` setting, is the one
/// `cordd` serves under: exactly [`REQUIRED_TRANSACTION_ISOLATION`].
///
/// # Errors
///
/// [`StartupError::TransactionIsolation`] for any other value.
pub fn check_transaction_isolation(value: &str) -> Result<(), StartupError> {
    if value == REQUIRED_TRANSACTION_ISOLATION {
        Ok(())
    } else {
        Err(StartupError::TransactionIsolation {
            found: value.to_owned(),
        })
    }
}

/// Read `default_transaction_isolation` on one pooled connection and refuse
/// anything but `read committed`.
///
/// Every pooled connection shares the setting: it comes from the server,
/// database, and role, which are the same for all of them. That holds
/// because `fuz_db::create_pool` passes only host, port, user, database, and
/// password through, dropping any `options=` in the URL, and nothing reads
/// `PGOPTIONS`; a pool that forwarded per-connection options would need
/// this check to move per connection.
///
/// # Errors
///
/// [`StartupError::Database`] when the connection or query fails;
/// [`StartupError::TransactionIsolation`] when the setting is anything but
/// `read committed`.
pub async fn ensure_read_committed(pool: &Pool) -> Result<(), StartupError> {
    let client = pool.get().await.map_err(DbError::PoolGet)?;
    let value = query_default_transaction_isolation(&client).await?;
    check_transaction_isolation(&value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_read_committed_passes() {
        check_transaction_isolation("read committed").expect("the Postgres default passes");
        for value in [
            "repeatable read",
            "serializable",
            // Postgres runs it as read committed, but it names another level
            "read uncommitted",
            "READ COMMITTED",
            "",
        ] {
            let err = check_transaction_isolation(value).expect_err(value);
            assert!(
                matches!(&err, StartupError::TransactionIsolation { found } if found == value),
                "{err}"
            );
            assert_eq!(err.exit_code(), 2, "the operator fixes the config");
        }
    }
}

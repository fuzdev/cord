//! Typed errors for `cord_server`.
//!
//! Exit-code dialect for `cordd`: `0` success; `2` = the caller must change
//! something local (arguments, env, config) before retrying; `1` = everything
//! else (database, bind, serve, internal).

use std::net::SocketAddr;

/// Errors raised while starting or running the server.
#[derive(Debug, thiserror::Error)]
pub enum StartupError {
    #[error("failed to bind to {addr}")]
    Bind {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },

    #[error("server error")]
    Serve(#[source] std::io::Error),

    #[error("invalid configuration: {0}")]
    Config(String),

    #[error("database error")]
    Database(#[from] fuz_db::DbError),

    /// Postgres runs new transactions at an isolation level other than READ
    /// COMMITTED, which cord's membership serialization relies on (see
    /// [`crate::transaction_isolation`]).
    #[error(
        "Postgres's default_transaction_isolation is {found:?}; cordd requires \"read committed\""
    )]
    TransactionIsolation { found: String },

    #[error("internal error: {0}")]
    Internal(String),
}

impl StartupError {
    /// Maps the error to its stable process exit code.
    pub const fn exit_code(&self) -> u8 {
        match self {
            Self::Config(_) | Self::TransactionIsolation { .. } => 2,
            Self::Bind { .. } | Self::Serve(_) | Self::Database(_) | Self::Internal(_) => 1,
        }
    }

    /// An optional remediation hint printed under the error.
    pub const fn hint(&self) -> Option<&'static str> {
        match self {
            Self::Database(_) => {
                Some("check that Postgres is running and DATABASE_URL points at a database")
            }
            Self::Bind { .. } => Some("another process may hold the port; set PORT to move cordd"),
            Self::Config(_) => Some("see .env.development.example for the server's env vars"),
            Self::TransactionIsolation { .. } => Some(
                "set default_transaction_isolation back to 'read committed', the Postgres default, \
                 wherever it was changed: postgresql.conf, ALTER DATABASE ... SET, or \
                 ALTER ROLE ... SET (psql's \\drds lists the database and role settings)",
            ),
            Self::Serve(_) | Self::Internal(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_codes_are_stable() {
        let bind = || StartupError::Bind {
            addr: crate::runtime::DEFAULT_ADDR,
            source: std::io::Error::other("in use"),
        };
        let serve = || StartupError::Serve(std::io::Error::other("boom"));
        let config = || StartupError::Config(String::new());
        let database = || StartupError::Database(fuz_db::DbError::InvalidUrl(String::new()));
        let internal = || StartupError::Internal(String::new());
        let isolation = || StartupError::TransactionIsolation {
            found: "repeatable read".to_owned(),
        };

        assert_eq!(config().exit_code(), 2);
        assert_eq!(bind().exit_code(), 1);
        assert_eq!(serve().exit_code(), 1);
        assert_eq!(database().exit_code(), 1);
        assert_eq!(internal().exit_code(), 1);
        assert_eq!(isolation().exit_code(), 2);

        assert!(
            config()
                .hint()
                .is_some_and(|h| h.contains(".env.development.example"))
        );
        assert!(bind().hint().is_some_and(|h| h.contains("PORT")));
        assert!(
            database()
                .hint()
                .is_some_and(|h| h.contains("DATABASE_URL"))
        );
        assert!(
            isolation()
                .hint()
                .is_some_and(|h| h.contains("default_transaction_isolation"))
        );
        assert!(isolation().to_string().contains("repeatable read"));
        assert_eq!(serve().hint(), None);
        assert_eq!(internal().hint(), None);
    }
}

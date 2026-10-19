//! [`CordApp`], the composition root for the `cordd` process.
//!
//! `CordApp` is the wiring root, not a handler context. Handlers receive
//! narrow `*RouteState` slices via `axum::State` (the spine's
//! [`fuz_auth::AccountRouteState`], [`fuz_actions::RpcRouteState`],
//! [`fuz_actions::WsRouteState`], and friends), built once from `CordApp` in
//! [`crate::router`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use deadpool_postgres::Pool;
use fuz_auth::{
    AuditEmitter, Keyring, PasswordHasher, RateLimiter, SharedDaemonTokenState, SocketRevoker,
};
use fuz_realtime::{ConnectionRegistry, RealtimeRevoker, SseRegistry};

use crate::env::CordEnv;
use crate::error::StartupError;

/// Long-lived state composed once at startup.
///
/// Every field is `Arc`-wrapped (or an `Arc`-backed handle like [`Pool`]), so
/// `Clone` is cheap.
#[derive(Clone)]
pub struct CordApp {
    // spine: db + auth
    pub pool: Pool,
    pub keyring: Arc<Keyring>,
    pub password_hasher: Arc<dyn PasswordHasher>,
    pub audit_emitter: Arc<AuditEmitter>,
    pub bootstrap_available: Arc<AtomicBool>,
    pub login_ip_rate_limiter: Option<Arc<RateLimiter>>,
    pub login_account_rate_limiter: Option<Arc<RateLimiter>>,
    /// `cord_invite_redeem`'s own per-IP limiter
    /// ([`crate::invite::actions::CORD_INVITE_REDEEM_RATE_LIMIT`]), charged
    /// in the handler on top of the spine's shared IP class.
    pub cord_invite_redeem_rate_limiter: Option<Arc<RateLimiter>>,
    /// The daemon-token credential. Always `None` in production: only the
    /// `testing_cordd` binary mints one and injects it through
    /// `RunAppOptions::daemon_token_state`. The keeper it resolves to is
    /// looked up after migrations (in [`crate::router::build_router`]).
    pub daemon_token_state: Option<SharedDaemonTokenState>,

    // spine: realtime
    /// The WebSocket connection registry, also the notification sender and,
    /// with [`Self::audit_sse`], the socket revoker.
    pub realtime: Arc<ConnectionRegistry>,
    /// The open admin audit-log SSE streams behind
    /// `GET /api/admin/audit/stream`; audit rows fan out to them and
    /// revocations close them.
    pub audit_sse: Arc<SseRegistry>,

    // shared config
    pub allowed_origins: fuz_http::AllowedOrigins,
    pub bootstrap_token_path: Option<PathBuf>,
}

impl std::fmt::Debug for CordApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CordApp")
            .field(
                "allowed_origins_count",
                &self.allowed_origins.patterns().len(),
            )
            .field(
                "has_bootstrap_token_path",
                &self.bootstrap_token_path.is_some(),
            )
            .field("has_daemon_token_state", &self.daemon_token_state.is_some())
            .finish_non_exhaustive()
    }
}

impl CordApp {
    /// [`Self::realtime`] and [`Self::audit_sse`] as one `SocketRevoker`, so
    /// logout, password change, session or token revocation, and the auth
    /// cleanup's sweep of an expired session drop the matching live sockets
    /// and audit streams. The streams can't wait for the SSE listener: it
    /// fires only on a written audit row, and the REST `/logout` /
    /// `/password` audit write is fail-open.
    #[must_use]
    pub fn socket_revoker(&self) -> Arc<dyn SocketRevoker> {
        RealtimeRevoker::socket_revoker(&self.realtime, &self.audit_sse)
    }

    /// [`Self::realtime`] as a `NotificationSender`, the same instance the
    /// WebSocket endpoint populates, so a notification raised over HTTP
    /// reaches the recipient's sockets.
    #[must_use]
    pub fn notification_sender(&self) -> Arc<dyn fuz_auth::NotificationSender> {
        let registry: Arc<ConnectionRegistry> = Arc::clone(&self.realtime);
        registry
    }

    /// Build the app from a validated env, an injected password hasher, and
    /// the daemon-token credential (production passes `None`).
    ///
    /// # Errors
    ///
    /// [`StartupError::Config`] when `DATABASE_URL` is absent or the cookie
    /// keys are missing or weak; [`StartupError::Database`] when the pool
    /// can't be created.
    pub fn build(
        env: &CordEnv,
        password_hasher: Arc<dyn PasswordHasher>,
        rate_limiters: fuz_auth::RateLimiterMode,
        daemon_token_state: Option<SharedDaemonTokenState>,
    ) -> Result<Self, StartupError> {
        let database_url = env
            .database_url
            .as_deref()
            .ok_or_else(|| StartupError::Config("DATABASE_URL is required".to_owned()))?;

        let pool = fuz_db::create_pool(database_url)?;

        // Refuses to boot without real cookie keys: falling back to an
        // ephemeral key would reset every session on restart.
        let keyring = Arc::new(build_keyring(env.cookie_keys.as_deref())?);

        // TODO: build with `AuditEmitter::new_with_inflight_tracking` when a
        // cross-backend test asserts on a fire-and-forget audit row, so
        // `testing_cordd` can register `_testing_drain_effects`
        let audit_emitter = Arc::new(AuditEmitter::new(pool.clone()));
        // Recomputed against the live DB after migrations (`build_router`).
        let bootstrap_available = Arc::new(AtomicBool::new(false));

        // Built through the process-wide `RateLimiterMode`, so a disabled
        // limiter always announces itself.
        let login_ip_rate_limiter = rate_limiters.limiter(fuz_auth::DEFAULT_LOGIN_IP_RATE_LIMIT);
        let login_account_rate_limiter =
            rate_limiters.limiter(fuz_auth::DEFAULT_LOGIN_ACCOUNT_RATE_LIMIT);
        let cord_invite_redeem_rate_limiter =
            rate_limiters.limiter(crate::invite::actions::CORD_INVITE_REDEEM_RATE_LIMIT);

        Ok(Self {
            pool,
            keyring,
            password_hasher,
            audit_emitter,
            bootstrap_available,
            login_ip_rate_limiter,
            login_account_rate_limiter,
            cord_invite_redeem_rate_limiter,
            daemon_token_state,
            realtime: Arc::new(ConnectionRegistry::new()),
            audit_sse: Arc::new(SseRegistry::new()),
            allowed_origins: env.allowed_origins.clone(),
            bootstrap_token_path: env.bootstrap_token_path.clone(),
        })
    }
}

/// Build a [`Keyring`] from the `SECRET_FUZ_COOKIE_KEYS` value.
///
/// [`Keyring::new`] validates, so the per-key length policy stays
/// single-sourced in the spine.
///
/// # Errors
///
/// [`StartupError::Config`] when the value is absent, empty, or any key is
/// shorter than the spine minimum.
fn build_keyring(cookie_keys: Option<&str>) -> Result<Keyring, StartupError> {
    let value = cookie_keys.ok_or_else(|| {
        StartupError::Config(
            "SECRET_FUZ_COOKIE_KEYS is required (cookie-signing keys, rotated with `__`)"
                .to_owned(),
        )
    })?;
    Keyring::new(value)
        .map_err(|e| StartupError::Config(format!("SECRET_FUZ_COOKIE_KEYS invalid: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_requires_cookie_keys() {
        let err = build_keyring(None).unwrap_err();
        assert!(err.to_string().contains("SECRET_FUZ_COOKIE_KEYS"), "{err}");
        assert_eq!(err.exit_code(), 2);
    }

    #[test]
    fn keyring_rejects_weak_keys() {
        let err = build_keyring(Some("short")).unwrap_err();
        assert!(err.to_string().contains("SECRET_FUZ_COOKIE_KEYS"), "{err}");
    }

    #[test]
    fn keyring_accepts_a_strong_key() {
        build_keyring(Some("a-cookie-signing-key-that-is-long-enough-to-pass")).unwrap();
    }
}

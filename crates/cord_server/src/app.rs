//! [`CordApp`], the composition root for the `cordd` process.
//!
//! `CordApp` is the wiring root, not a handler context: the spine's
//! `AppBackend` beside cord's own invite limiter. Handlers receive narrow
//! `*RouteState` slices via `axum::State` (the spine's
//! [`fuz_auth::AccountRouteState`], [`fuz_actions::ActionDispatch`],
//! [`fuz_actions::WsRouteState`], and friends), built once from `CordApp` in
//! [`crate::router`].

use std::sync::Arc;

use fuz_actions::server::AppBackend;
use fuz_auth::RateLimiter;

/// Long-lived state composed once at startup.
///
/// Every field is `Arc`-wrapped (or an `Arc`-backed handle), so `Clone` is
/// cheap.
#[derive(Clone)]
pub struct CordApp {
    /// The spine core: pool, keyring, hasher, audit emitter, login limiters,
    /// the daemon-token credential (always `None` in production: only the
    /// `testing_cordd` binary mints one and injects it), the WebSocket
    /// registry, and the admin audit-log SSE registry behind
    /// `GET /api/admin/audit/stream`. Its [`AppBackend::socket_revoker`]
    /// closes both.
    pub backend: AppBackend,
    /// `cord_invite_redeem`'s own per-IP limiter
    /// ([`crate::invite::actions::CORD_INVITE_REDEEM_RATE_LIMIT`]), charged
    /// in the handler on top of the spine's shared IP class. Built through
    /// the core's limiter mode, so a disabled one announces itself.
    pub cord_invite_redeem_rate_limiter: Option<Arc<RateLimiter>>,
}

impl std::fmt::Debug for CordApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CordApp")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

impl CordApp {
    /// Compose cord's state over a spine core.
    #[must_use]
    pub fn new(backend: AppBackend) -> Self {
        Self {
            cord_invite_redeem_rate_limiter: backend
                .rate_limiters
                .limiter(crate::invite::actions::CORD_INVITE_REDEEM_RATE_LIMIT),
            backend,
        }
    }
}

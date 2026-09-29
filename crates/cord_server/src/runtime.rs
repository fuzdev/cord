//! The server lifecycle, owned by [`run_app`].
//!
//! [`run_app`] loads env, installs the signal handler, builds the router,
//! starts the invite expiry sweep, binds the listener, and drains on
//! shutdown, so the binary's `main` is tracing init, hasher selection, and
//! one call.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::env::CordEnv;
use crate::error::StartupError;
use crate::invite::sweep::{CORD_INVITE_SWEEP_INTERVAL, run_cord_invite_sweep};
use crate::router::{BuiltRouter, build_router};

/// cord's instantiation of the spine's extra-action-specs factory, over
/// [`crate::app::CordApp`].
///
/// Production passes `None`; the `testing_cordd` binary passes `Some(_)` to
/// fold in the `_testing_*` action specs without `fuz_testing` entering the
/// production `cordd` dep graph.
pub type ExtraActionSpecsFactory = fuz_actions::ExtraActionSpecsFactory<crate::app::CordApp>;

/// cord's instantiation of the spine's pre-migration hook, over
/// [`StartupError`].
///
/// Production passes `None`; the `testing_cordd` binary passes `Some(_)` to
/// run the env-gated startup schema reset.
pub type PreMigrationHook = fuz_actions::PreMigrationHook<StartupError>;

/// Default loopback bind for `cordd` (`127.0.0.1:4061`). A reverse proxy
/// terminates TLS and forwards to this address; the server serves no static
/// files.
pub const DEFAULT_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 4061);

/// Bound on the graceful-shutdown connection drain, the spine's shared value.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = fuz_http::DEFAULT_DRAIN_TIMEOUT;

/// Options for [`run_app`].
///
/// Named fields so swap points land additively. Production passes `None`
/// for each test-only seam (extra action specs, the pre-migration hook, the
/// daemon-token credential); only the `testing_cordd` binary fills them.
#[allow(
    missing_debug_implementations,
    reason = "holds an `Arc<dyn PasswordHasher>`"
)]
pub struct RunAppOptions {
    /// The password hasher; production wires [`fuz_auth::Argon2idHasher`].
    pub password_hasher: Arc<dyn fuz_auth::PasswordHasher>,
    /// Bind address used when `HOST` and `PORT` are unset; production passes
    /// [`DEFAULT_ADDR`].
    pub default_addr: SocketAddr,
    /// Bounds the graceful-shutdown connection drain.
    pub drain_timeout: Duration,
    /// Production passes [`fuz_auth::RateLimiterMode::Enforced`]; every spine
    /// limiter in the process is built through this one mode.
    pub rate_limiters: fuz_auth::RateLimiterMode,
    /// Extra action specs folded into the registry after cord's own, the
    /// `testing_cordd` seam for the `_testing_*` actions. Production passes
    /// `None`.
    pub extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
    /// Fired after the pool exists and before migrations run, the
    /// `testing_cordd` seam for its env-gated schema reset. Production passes
    /// `None`.
    pub pre_migration_hook: Option<PreMigrationHook>,
    /// The daemon-token credential. Production passes `None`; only
    /// `testing_cordd` mints one (through `fuz_testing`, which
    /// `cargo xtask check-release` keeps out of production binaries), so
    /// `_testing_reset` can authenticate as the keeper.
    pub daemon_token_state: Option<fuz_auth::SharedDaemonTokenState>,
}

/// Run the server to completion.
///
/// With a database, the invite expiry sweep
/// ([`crate::invite::sweep`]) runs beside the server: once migrations
/// have run, then every [`CORD_INVITE_SWEEP_INTERVAL`]. It stops when
/// shutdown begins.
///
/// Returns once SIGINT or SIGTERM arrives and connections have drained.
///
/// # Errors
///
/// [`StartupError`] for env or config validation failures, database
/// connectivity or migration failures, listener bind failures, and serve
/// errors.
pub async fn run_app(options: RunAppOptions) -> Result<(), StartupError> {
    let RunAppOptions {
        password_hasher,
        default_addr,
        drain_timeout,
        rate_limiters,
        extra_action_specs_factory,
        pre_migration_hook,
        daemon_token_state,
    } = options;

    let env = CordEnv::from_process()?;
    let addr = env.bind_addr.unwrap_or(default_addr);

    // SIGINT/SIGTERM → `CancellationToken`, shared with the other spine
    // servers.
    let shutdown = fuz_http::shutdown_token();

    let BuiltRouter { router, app } = build_router(
        &env,
        password_hasher,
        rate_limiters,
        daemon_token_state,
        extra_action_specs_factory,
        pre_migration_hook,
    )
    .await?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| StartupError::Bind { addr, source })?;

    // Migrations have run, so the sweep's first pass can start now.
    let sweep = app.map(|app| {
        let shutdown = shutdown.clone();
        tokio::spawn(run_cord_invite_sweep(
            app.pool,
            app.audit_emitter,
            CORD_INVITE_SWEEP_INTERVAL,
            async move { shutdown.cancelled().await },
        ))
    });

    // Unconditional human-visible sentinel, whatever the log filter.
    eprintln!("cordd listening on http://{addr}");

    // `serve_with_shutdown` supplies `ConnectInfo<SocketAddr>` per connection
    // and bounds the drain.
    let served =
        fuz_http::serve_with_shutdown(listener, router, shutdown.clone(), drain_timeout).await;

    // A serve error returns without the signal, so cancel for the sweep's
    // sake; it stops at once either way.
    shutdown.cancel();
    if let Some(sweep) = sweep
        && let Err(e) = sweep.await
    {
        tracing::warn!(error = %e, "the invite expiry sweep task failed");
    }
    served.map_err(StartupError::Serve)?;

    eprintln!("cordd shutdown complete");
    Ok(())
}

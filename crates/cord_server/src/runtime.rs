//! The server lifecycle, owned by [`run_app`].
//!
//! [`run_app`] loads env, installs the signal handler, builds the router
//! (running migrations), checks the database's transaction isolation, binds
//! the listener, starts the upkeep tasks ([`spawn_upkeep`]), and drains on
//! shutdown, so the binary's `main` is tracing init, hasher selection, and
//! one call.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use fuz_auth::AuditEmitter;
use fuz_db::Pool;
use fuz_http::CancellationToken;
use tokio::task::JoinHandle;

use crate::env::CordEnv;
use crate::error::StartupError;
use crate::invite::sweep::{CORD_INVITE_SWEEP_INTERVAL, spawn_cord_invite_sweep};
use crate::router::{BuiltRouter, build_router};
use crate::transaction_isolation::ensure_read_committed;

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

/// The upkeep tasks [`run_app`] runs beside the server, each a pass now and
/// then one per interval until `shutdown` is cancelled:
///
/// - the spine's auth cleanup ([`fuz_auth::spawn_auth_cleanup`]): expired
///   sessions deleted, expired role-grant offers audited once, every
///   [`fuz_auth::DEFAULT_AUTH_CLEANUP_INTERVAL`]
/// - the invite expiry sweep ([`crate::invite::sweep`]), every
///   [`CORD_INVITE_SWEEP_INTERVAL`]
///
/// `audit` is the app's emitter, so what they audit reaches the admin audit
/// streams. Call it once migrations have run: the startup passes read the
/// schema. A failed pass is logged and retried on the next interval, and
/// cancelling `shutdown` drops a pass in progress, so the handles resolve at
/// once.
///
/// # Panics
///
/// When called outside a tokio runtime.
#[must_use]
pub fn spawn_upkeep(
    pool: &Pool,
    audit: &Arc<AuditEmitter>,
    shutdown: &CancellationToken,
) -> [JoinHandle<()>; 2] {
    [
        fuz_auth::spawn_auth_cleanup(
            fuz_auth::AuthCleanupOptions {
                pool: pool.clone(),
                audit: Arc::clone(audit),
            },
            fuz_auth::DEFAULT_AUTH_CLEANUP_INTERVAL,
            shutdown.clone(),
        ),
        spawn_cord_invite_sweep(
            pool.clone(),
            Arc::clone(audit),
            CORD_INVITE_SWEEP_INTERVAL,
            shutdown.clone(),
        ),
    ]
}

/// Run the server to completion.
///
/// With a database, the upkeep tasks ([`spawn_upkeep`]: the spine's auth
/// cleanup and the invite expiry sweep) run beside the server, from the
/// bind until shutdown begins.
///
/// With a database, it refuses to serve unless Postgres runs transactions at
/// READ COMMITTED ([`crate::transaction_isolation`]).
///
/// Returns once SIGINT or SIGTERM arrives and connections have drained.
///
/// # Errors
///
/// [`StartupError`] for env or config validation failures, database
/// connectivity or migration failures, a `default_transaction_isolation`
/// other than `read committed`, listener bind failures, and serve errors.
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

    // The membership serialization relies on READ COMMITTED; refuse before
    // the listener binds, so a misconfigured database never serves.
    if let Some(app) = &app {
        ensure_read_committed(&app.pool).await?;
    }

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| StartupError::Bind { addr, source })?;

    // Migrations have run, so each task's startup pass can start now.
    let upkeep = app.map(|app| spawn_upkeep(&app.pool, &app.audit_emitter, &shutdown));

    // Unconditional human-visible sentinel, whatever the log filter.
    eprintln!("cordd listening on http://{addr}");

    // `serve_with_shutdown` supplies `ConnectInfo<SocketAddr>` per connection
    // and bounds the drain.
    let served =
        fuz_http::serve_with_shutdown(listener, router, shutdown.clone(), drain_timeout).await;

    // A serve error returns without the signal, so cancel for the upkeep
    // tasks' sake; they stop at once either way.
    shutdown.cancel();
    for task in upkeep.into_iter().flatten() {
        if let Err(e) = task.await {
            tracing::warn!(error = %e, "an upkeep task failed");
        }
    }
    served.map_err(StartupError::Serve)?;

    eprintln!("cordd shutdown complete");
    Ok(())
}

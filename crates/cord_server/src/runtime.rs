//! The server lifecycle, owned by [`run_app`].
//!
//! [`run_app`] loads env, installs the signal handler, builds the router,
//! binds the listener, and drains on shutdown, so the binary's `main` is
//! tracing init, hasher selection, and one call.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use crate::env::CordEnv;
use crate::error::StartupError;
use crate::router::build_router;

/// Default loopback bind for `cordd` (`127.0.0.1:4061`). A reverse proxy
/// terminates TLS and forwards to this address; the server serves no static
/// files.
pub const DEFAULT_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 4061);

/// Bound on the graceful-shutdown connection drain, the spine's shared value.
pub const DEFAULT_DRAIN_TIMEOUT: Duration = fuz_http::DEFAULT_DRAIN_TIMEOUT;

/// Options for [`run_app`].
///
/// Named fields so swap points land additively. The spine's test-only seams
/// (extra action specs, a pre-migration hook, the daemon-token credential)
/// are absent until cord has a testing sibling binary that supplies them.
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
}

/// Run the server to completion.
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
    } = options;

    let env = CordEnv::from_process()?;
    let addr = env.bind_addr.unwrap_or(default_addr);

    // SIGINT/SIGTERM → `CancellationToken`, shared with the other spine
    // servers.
    let shutdown = fuz_http::shutdown_token();

    let router = build_router(&env, password_hasher, rate_limiters).await?;

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| StartupError::Bind { addr, source })?;

    // Unconditional human-visible sentinel, whatever the log filter.
    eprintln!("cordd listening on http://{addr}");

    // `serve_with_shutdown` supplies `ConnectInfo<SocketAddr>` per connection
    // and bounds the drain.
    fuz_http::serve_with_shutdown(listener, router, shutdown, drain_timeout)
        .await
        .map_err(StartupError::Serve)?;

    eprintln!("cordd shutdown complete");
    Ok(())
}

//! The server lifecycle, owned by [`run_app`].
//!
//! Over the spine's server toolkit (`fuz_actions::server`): [`run_app`] reads
//! the env, installs the signal handler, builds the router (the READ
//! COMMITTED check, then migrations), binds the listener, starts the upkeep
//! tasks ([`upkeep_options`]), and drains on shutdown, so the binary's `main`
//! is tracing init, swap-point selection, and one call.

use std::net::SocketAddr;

use fuz_actions::server::{
    ServeOptions, SpineRunOptions, StartupError, UpkeepOptions, serve_until_shutdown,
};
use fuz_http::{BaseServerEnv, ServerEnvOptions};

use crate::invite::sweep::{CORD_INVITE_SWEEP_INTERVAL, spawn_cord_invite_sweep};
use crate::router::{BuiltRouter, build_router};

/// cord's instantiation of the spine's extra-action-specs factory, over
/// [`crate::app::CordApp`].
///
/// Production passes `None`; the `testing_cordd` binary passes `Some(_)` to
/// fold in the `_testing_*` action specs without `fuz_testing` entering the
/// production `cordd` dep graph.
pub type ExtraActionSpecsFactory = fuz_actions::ExtraActionSpecsFactory<crate::app::CordApp>;

/// Builds action specs over the **assembled** spec list, run after
/// [`ExtraActionSpecsFactory`], and mounts what it returns last.
///
/// The seam `_testing_action_manifest` needs: that spec reports the mounted
/// method set, so it must be built from every spec mounted before it, which
/// [`ExtraActionSpecsFactory`] can't see (it gets the app and the spine's
/// runtime, not the list). Production passes `None`; `testing_cordd` passes
/// `Some(_)`, closing over `fuz_testing` in its own process, so the `cordd`
/// graph stays clean (`cargo xtask check-release`).
pub type FinalActionSpecsFactory =
    Box<dyn FnOnce(&[fuz_actions::ActionSpec]) -> Vec<fuz_actions::ActionSpec> + Send>;

/// The name cord's sentinels (`cordd listening on …`) and `GET /` carry.
pub const SERVER_NAME: &str = "cordd";

/// Default loopback bind for `cordd` (`127.0.0.1:4061`). A reverse proxy
/// terminates TLS and forwards to this address; the server serves no static
/// files.
pub const DEFAULT_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 4061);

/// The env cord reads: the spine's base vars, its trusted proxies under
/// `CORD_TRUSTED_PROXIES` — the loopback reverse-proxy hop (`127.0.0.1`,
/// `::1`) when unset, `,` to trust none.
pub const ENV_OPTIONS: ServerEnvOptions = ServerEnvOptions {
    trusted_proxies_var: "CORD_TRUSTED_PROXIES",
};

/// The hint printed under a start failure: for a configuration error, where
/// cord's env vars are listed; otherwise the spine's.
#[must_use]
pub fn startup_hint(error: &StartupError) -> Option<&'static str> {
    match error {
        StartupError::Config(_) => Some("see .env.development.example for the server's env vars"),
        _ => error.hint(),
    }
}

/// Options for [`run_app`].
#[expect(
    missing_debug_implementations,
    reason = "the spine run options and the boxed action-spec factories have no `Debug`"
)]
pub struct RunAppOptions {
    /// The spine's swap points. Production: [`fuz_auth::Argon2idHasher`],
    /// [`DEFAULT_ADDR`], [`fuz_http::DEFAULT_DRAIN_TIMEOUT`], every limiter
    /// enforced (cord's invite limiter too, built through the same mode), no
    /// daemon token, no pre-migration hook. Only `testing_cordd` fills the
    /// seams.
    pub spine: SpineRunOptions,
    /// Extra action specs folded into the registry after cord's own, the
    /// `testing_cordd` seam for the `_testing_*` actions. Production passes
    /// `None`.
    pub extra_action_specs_factory: Option<ExtraActionSpecsFactory>,
    /// Action specs built from the assembled list and mounted last, the
    /// `testing_cordd` seam for `_testing_action_manifest`. Production passes
    /// `None`.
    pub final_action_specs_factory: Option<FinalActionSpecsFactory>,
}

/// The upkeep [`run_app`] runs beside the server, each task a pass now and
/// then one per interval until shutdown:
///
/// - the spine's auth cleanup ([`fuz_auth::spawn_auth_cleanup`]): expired
///   sessions deleted and the connections they opened closed, expired
///   role-grant offers audited once, every
///   [`fuz_auth::DEFAULT_AUTH_CLEANUP_INTERVAL`];
/// - the invite expiry sweep ([`crate::invite::sweep`]), every
///   [`CORD_INVITE_SWEEP_INTERVAL`], over the cleanup's pool and emitter.
///
/// `auth_cleanup` is the app's
/// ([`fuz_actions::server::AppBackend::auth_cleanup_options`]): its emitter,
/// so what both audit reaches the admin audit streams, and its closer, over
/// the sockets and the streams.
#[must_use]
pub fn upkeep_options(auth_cleanup: fuz_auth::AuthCleanupOptions) -> UpkeepOptions {
    let pool = auth_cleanup.pool.clone();
    let audit = std::sync::Arc::clone(&auth_cleanup.audit);
    UpkeepOptions {
        auth_cleanup,
        extra: vec![Box::new(move |shutdown| {
            spawn_cord_invite_sweep(pool, audit, CORD_INVITE_SWEEP_INTERVAL, shutdown)
        })],
    }
}

/// Run the server to completion.
///
/// Reads the env, installs the SIGINT/SIGTERM handler, builds the router via
/// [`build_router`] (which refuses a database not at READ COMMITTED before
/// migrating), and serves until the signal
/// ([`fuz_actions::server::serve_until_shutdown`]): every live WebSocket is
/// closed with 1001 and every audit stream ended, then in-flight requests
/// drain, then the audit writes in flight drain. With a database, the upkeep
/// ([`upkeep_options`]) runs beside the server, from the bind until shutdown
/// begins.
///
/// # Errors
///
/// [`StartupError`] for env or config failures, database connectivity or
/// migration failures, a `default_transaction_isolation` other than
/// `read committed`, and bind or serve failures.
pub async fn run_app(options: RunAppOptions) -> Result<(), StartupError> {
    let RunAppOptions {
        spine:
            SpineRunOptions {
                backend,
                default_addr,
                drain_timeout,
            },
        extra_action_specs_factory,
        final_action_specs_factory,
    } = options;

    let env = BaseServerEnv::from_process(ENV_OPTIONS)?;
    let addr = env.bind_addr(default_addr)?;
    let shutdown = fuz_http::shutdown_token();

    let BuiltRouter { router, app } = build_router(
        &env,
        backend,
        extra_action_specs_factory,
        final_action_specs_factory,
    )
    .await?;

    serve_until_shutdown(
        router,
        ServeOptions {
            server_name: SERVER_NAME,
            addr,
            drain_timeout,
            shutdown,
            backend: app.as_ref().map(|app| {
                app.backend
                    .serve_backend(Some(upkeep_options(app.backend.auth_cleanup_options())))
            }),
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn read(pairs: &[(&str, &str)]) -> Result<BaseServerEnv, fuz_http::ServerEnvError> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        BaseServerEnv::from_vars(&|k| map.get(k).cloned(), ENV_OPTIONS)
    }

    const ORIGINS: (&str, &str) = ("FUZ_ALLOWED_ORIGINS", "http://localhost");

    /// Unset, cord trusts the loopback reverse-proxy hop (`127.0.0.1`, `::1`)
    /// — the hop its deploy sets explicitly — so `X-Forwarded-For` from the
    /// proxy names the client; `,` opts out.
    #[test]
    fn trusted_proxies_default_to_loopback_under_cords_var() {
        assert_eq!(read(&[ORIGINS]).expect("reads").trusted_proxies.len(), 2);
        assert!(
            read(&[ORIGINS, ("CORD_TRUSTED_PROXIES", ",")])
                .expect("reads")
                .trusted_proxies
                .is_empty()
        );
        let error = read(&[ORIGINS, ("CORD_TRUSTED_PROXIES", "not-an-ip")]).expect_err("malformed");
        assert!(error.0.starts_with("CORD_TRUSTED_PROXIES: "), "{error}");
    }

    /// A blank `PORT` is refused rather than defaulted: a deploy template that
    /// rendered it empty is a bug.
    #[test]
    fn a_blank_port_is_refused() {
        let error = StartupError::from(read(&[ORIGINS, ("PORT", " ")]).expect_err("blank"));
        assert_eq!(error.exit_code(), 2);
        assert!(error.to_string().contains("PORT: "), "{error}");
    }

    /// A config failure is exit `2`, with cord's env-example hint.
    #[test]
    fn a_config_error_exits_two_with_the_env_hint() {
        let error = StartupError::from(read(&[]).expect_err("no allowlist"));
        assert_eq!(error.exit_code(), 2);
        assert!(
            startup_hint(&error).is_some_and(|h| h.contains(".env.development.example")),
            "{error}"
        );
        let bind = StartupError::Bind {
            addr: DEFAULT_ADDR,
            source: std::io::Error::other("in use"),
        };
        assert_eq!(startup_hint(&bind), bind.hint());
    }

    #[test]
    fn the_bind_defaults_to_loopback_4061() {
        assert_eq!(
            read(&[ORIGINS])
                .expect("reads")
                .bind_addr(DEFAULT_ADDR)
                .expect("binds"),
            DEFAULT_ADDR
        );
    }
}

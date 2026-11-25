//! `testing_cordd`: the test-mode cord server, **never shipped**.
//!
//! It runs the production composition root through
//! [`cord_server::runtime::run_app`], swapping in
//! [`fuz_testing::TestingArgon2idHasher`] (fast, weak argon2 parameters) and
//! folding in the `_testing_reset` action through the
//! `extra_action_specs_factory` seam, so the production `cordd` graph never
//! links `fuz_testing`. Three guardrails keep it out of production:
//!
//! 1. The `testing_` binary-name prefix marks it unshippable, and is what
//!    exempts it, by name, from the release audit below.
//! 2. `cargo xtask check-release` asserts no production binary reaches
//!    `fuz_testing` or `fuz_audit` through a runtime dependency; this is a
//!    separate crate, so `cord_server` takes `fuz_testing` only as a
//!    dev-dependency for its Postgres suites, an edge that never reaches
//!    `cordd`.
//! 3. `TestingArgon2idHasher::new` logs `WARN: test-mode argon2 hasher active`,
//!    and the disabled rate limiters print their own banner.
//!
//! It binds `127.0.0.1:4161` by default (`cordd` is `4061`) so both can run
//! side by side; the cross-backend harness overrides with `HOST` and `PORT`.
//! Its command line is `cordd`'s: no arguments (or `start`) serves,
//! `db-status` reports, `--help` prints usage, and anything else exits `2`.
//! It requires `CORD_DAEMON_TOKEN_DIR`: the daemon token written under it is
//! the keeper credential `_testing_reset` authenticates with.
//!
//! Beside the spine's `_testing_reset` it registers `_testing_drain_effects`,
//! the barrier that awaits in-flight fire-and-forget audit writes (a
//! signup's, or a failure audit from an RPC verb), `_testing_db_invariants`,
//! the read the cross-process setup checks the auth schema's invariants
//! through after each test, `_testing_shorten_credential_expiry`, which pulls
//! an account's session and token expiries close so a test can watch its open socket and audit stream
//! close when they lapse, and cord's own test seams
//! ([`cord_testing_actions`]), all under the same daemon-token gate. Last,
//! through the `final_action_specs_factory` seam, it mounts
//! `_testing_action_manifest`
//! ([`fuz_testing::create_testing_action_manifest_action_spec`]) over the
//! assembled spec list, so the manifest reports every method `cordd` mounts
//! but the protocol actions, plus the test-only ones. The drain is a real
//! barrier because every spine audit emitter tracks its in-flight writes.

mod cord_testing_actions;

use std::net::SocketAddr;
use std::process::ExitCode;

use cord_server::router::cord_migration_namespaces;
use cord_server::runtime::{
    ExtraActionSpecsFactory, FinalActionSpecsFactory, RunAppOptions, run_app, startup_hint,
};
use fuz_actions::server::{ServerInvocation, SpineRunOptions, StartCommand, server_invocation};
use fuz_testing::{
    require_daemon_token_from_env, testing_backend_options, testing_reset_specs,
    truncate_reset_state,
};

/// The env var naming the directory the daemon token is written under
/// (`{dir}/run/daemon_token`).
const DAEMON_TOKEN_DIR_ENV: &str = "CORD_DAEMON_TOKEN_DIR";

/// Default loopback bind, distinct from `cordd`'s `127.0.0.1:4061`.
const TESTING_DEFAULT_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 4161);

/// cord's own tables — its invite links and their redemptions — which
/// `_testing_reset`'s `reset_state` hook empties with the cell tables (cords
/// live there). The auth tables are then wiped and a fresh keeper seeded, all
/// in one transaction, so no cord or link outlives the accounts that were in
/// it.
const CORD_TABLES: &[&str] = &["cord_invite_redemption", "cord_invite"];

#[tokio::main]
async fn main() -> ExitCode {
    let invocation = match server_invocation("testing_cordd", StartCommand::Implicit) {
        Ok(invocation) => invocation,
        Err(exit) => return exit,
    };

    let _log_guard = fuz_sys::logging::init_non_blocking_stdout(
        "info,cord_server=info,testing_cord_server=info",
    );

    if invocation == ServerInvocation::DbStatus {
        return fuz_db::status::run_db_status_cli(&cord_migration_namespaces()).await;
    }

    // Always-visible sentinel, whatever the log filter.
    eprintln!("testing_cordd starting (test-mode argon2 active)");

    // Mint the daemon token, the keeper channel `_testing_reset` rides.
    // `fuz_testing` is its sole producer; it's static for the process
    // lifetime (the harness reads it once at spawn).
    let daemon_token_state = match require_daemon_token_from_env(DAEMON_TOKEN_DIR_ENV).await {
        Ok(state) => state,
        Err(e) => return fuz_sys::cli::report_failure(&e, e.hint(), e.exit_code()),
    };

    let extra_action_specs_factory: ExtraActionSpecsFactory = Box::new(move |_app, runtime| {
        let mut specs = testing_reset_specs(
            &runtime,
            Some(truncate_reset_state(
                &[fuz_cell::CELL_DROP_TABLES, CORD_TABLES].concat(),
            )),
        )?;
        specs.push(fuz_testing::create_testing_shorten_credential_expiry_action_spec());
        specs.push(cord_testing_actions::create_testing_cord_invite_expire_action_spec());
        Ok(specs)
    });

    // `_testing_action_manifest`, built over the assembled list so it reports
    // every method mounted above it, plus its own entry; the cross suite holds
    // it to the frontend's specs
    let final_action_specs_factory: FinalActionSpecsFactory = Box::new(|mounted| {
        vec![fuz_testing::create_testing_action_manifest_action_spec(
            mounted,
        )]
    });

    match run_app(RunAppOptions {
        spine: SpineRunOptions {
            backend: testing_backend_options(
                daemon_token_state,
                fuz_auth::AuthCleanupSchedule::Every(fuz_auth::DEFAULT_AUTH_CLEANUP_INTERVAL),
            ),
            default_addr: TESTING_DEFAULT_ADDR,
            drain_timeout: fuz_http::DEFAULT_DRAIN_TIMEOUT,
        },
        extra_action_specs_factory: Some(extra_action_specs_factory),
        final_action_specs_factory: Some(final_action_specs_factory),
    })
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fuz_sys::cli::report_failure(&e, startup_hint(&e), e.exit_code()),
    }
}

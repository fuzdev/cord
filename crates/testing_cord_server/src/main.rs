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
//! It requires `CORD_DAEMON_TOKEN_DIR`: the daemon token written under it is
//! the keeper credential `_testing_reset` authenticates with. With
//! `FUZ_TESTING_RESET_DB_ON_STARTUP=true` it drops and recreates the
//! database's `public` schema before migrating.
//!
//! Beside the spine's `_testing_reset` it registers `_testing_drain_effects`,
//! the barrier that awaits in-flight fire-and-forget audit writes (a
//! signup's, or a failure audit from an RPC verb), and cord's own test seams
//! ([`cord_testing_actions`]), all under the same daemon-token gate. The
//! drain is a real barrier because this binary builds the app's audit
//! emitter with in-flight tracking (`RunAppOptions::audit_inflight_tracking`,
//! `AuditEmitter::new_with_inflight_tracking`); over an untracked emitter it
//! would return at once.

mod cord_testing_actions;

use std::net::SocketAddr;
use std::process::ExitCode;
use std::sync::Arc;

use cord_server::error::StartupError;
use cord_server::runtime::{
    DEFAULT_DRAIN_TIMEOUT, ExtraActionSpecsFactory, PreMigrationHook, RunAppOptions, run_app,
};
use fuz_auth::PasswordHasher;
use fuz_testing::{
    TestingArgon2idHasher, TestingResetOptions, create_testing_drain_effects_action_spec,
    create_testing_reset_action_spec,
};

/// The env var naming the directory the daemon token is written under
/// (`{dir}/run/daemon_token`).
const DAEMON_TOKEN_DIR_ENV: &str = "CORD_DAEMON_TOKEN_DIR";

/// Default loopback bind, distinct from `cordd`'s `127.0.0.1:4061`.
const TESTING_DEFAULT_ADDR: SocketAddr =
    SocketAddr::new(std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST), 4161);

#[tokio::main]
async fn main() -> ExitCode {
    let _log_guard = fuz_sys::logging::init_non_blocking_stdout(
        "info,cord_server=info,testing_cord_server=info",
    );

    let password_hasher: Arc<dyn PasswordHasher> = Arc::new(TestingArgon2idHasher::new());

    // Always-visible sentinel, whatever the log filter.
    eprintln!("testing_cordd starting (test-mode argon2 active)");

    // `_testing_reset`'s `reset_state` hook first empties cord's invite links
    // and their redemptions and the cell tables (cords live there), then the
    // auth tables are wiped and a fresh keeper seeded, all in one transaction,
    // so no cord or link outlives the accounts that were in it.
    let extra_action_specs_factory: ExtraActionSpecsFactory = Box::new(|app, runtime| {
        #[allow(
            clippy::expect_used,
            reason = "fail loud at startup if the test binary is misconfigured"
        )]
        let daemon_token_state = runtime
            .daemon_token_state
            .expect("testing_cordd requires CORD_DAEMON_TOKEN_DIR (main checks it before run_app)");
        vec![
            create_testing_reset_action_spec(TestingResetOptions {
                password_hasher: runtime.password_hasher,
                keyring: runtime.keyring,
                daemon_token_state,
                session_cookie_name: runtime.session_cookie_name,
                reset_state: Some(Arc::new(|db: fuz_actions::ActionDb<'_>| {
                    Box::pin(async move {
                        db.execute(
                            "TRUNCATE cord_invite_redemption, cord_invite, cell_field, cell_item, \
                             cell_grant, cell RESTART IDENTITY CASCADE",
                            &[],
                        )
                        .await
                        .map_err(|e| {
                            fuz_http::internal_error_with_source(
                                "_testing_reset cord and cell truncate",
                                &e,
                            )
                        })?;
                        Ok(())
                    })
                })),
            }),
            // awaits the emitter's detached writes; tracked because
            // `audit_inflight_tracking` is on below
            create_testing_drain_effects_action_spec(Arc::clone(&app.audit_emitter)),
            cord_testing_actions::create_testing_cord_invite_expire_action_spec(),
        ]
    });

    // Env-gated (`FUZ_TESTING_RESET_DB_ON_STARTUP=true`) schema wipe, so
    // repeated cross-backend runs against one database start clean. A no-op
    // when unset.
    let pre_migration_hook: PreMigrationHook = Box::new(|pool: &fuz_db::Pool| {
        Box::pin(async move {
            let client = pool.get().await.map_err(|e| {
                StartupError::Config(format!("failed to get a client for the startup reset: {e}"))
            })?;
            fuz_testing::reset_db_on_startup_if_env_set(&client)
                .await
                .map_err(|e| StartupError::Config(format!("startup DB reset failed: {e}")))
        })
    });

    // Mint the daemon token, the keeper channel `_testing_reset` rides.
    // `fuz_testing` is its sole producer; it's static for the process
    // lifetime (the harness reads it once at spawn).
    let daemon_token_state = match fuz_testing::init_daemon_token_from_env(DAEMON_TOKEN_DIR_ENV)
        .await
    {
        Ok(Some(state)) => state,
        Ok(None) => {
            eprintln!(
                "error: testing_cordd requires {DAEMON_TOKEN_DIR_ENV} (the daemon token written \
                 under it is the keeper credential _testing_reset authenticates with)"
            );
            return ExitCode::from(2);
        }
        Err(e) => {
            eprintln!("error: daemon token init failed: {e}");
            return ExitCode::from(2);
        }
    };

    let Err(e) = run_app(RunAppOptions {
        password_hasher,
        default_addr: TESTING_DEFAULT_ADDR,
        drain_timeout: DEFAULT_DRAIN_TIMEOUT,
        // The cross-backend suites log in and sign up many times per
        // process; the limiters' startup banner announces the nulls.
        rate_limiters: fuz_auth::RateLimiterMode::DisabledForTesting,
        extra_action_specs_factory: Some(extra_action_specs_factory),
        pre_migration_hook: Some(pre_migration_hook),
        daemon_token_state: Some(daemon_token_state),
        // makes `_testing_drain_effects` a real barrier
        audit_inflight_tracking: true,
    })
    .await
    else {
        return ExitCode::SUCCESS;
    };
    eprintln!("error: {e}");
    if let Some(hint) = e.hint() {
        eprintln!("hint: {hint}");
    }
    ExitCode::from(e.exit_code())
}

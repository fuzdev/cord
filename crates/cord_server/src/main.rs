//! `cordd`, the cord server binary.
//!
//! The command line is strict: no arguments (or `start`) runs the server (see
//! [`cord_server::runtime::run_app`]), configured by the process environment;
//! `db-status` prints a read-only migration report (connectivity, table
//! counts, applied and pending migrations) via the spine's
//! `fuz_db::status::run_db_status_cli`, never mutating; `--help` / `-h`
//! prints usage. Anything else exits `2` with usage on stderr.
//!
//! A failed start prints its error chain and a hint, and exits `2` when the
//! caller must fix something local (env, the database's isolation setting),
//! `1` otherwise.

use std::process::ExitCode;
use std::sync::Arc;

use fuz_actions::server::{ServerInvocation, SpineRunOptions, StartCommand, server_invocation};
use fuz_auth::{
    Argon2idHasher, AuthCleanupSchedule, DEFAULT_AUTH_CLEANUP_INTERVAL, PasswordHasher,
};

use cord_server::router::cord_migration_namespaces;
use cord_server::runtime::{DEFAULT_ADDR, RunAppOptions, run_app, startup_hint};

#[tokio::main]
async fn main() -> ExitCode {
    let invocation = match server_invocation("cordd", StartCommand::Implicit) {
        Ok(invocation) => invocation,
        Err(exit) => return exit,
    };

    // Non-blocking stdout logging, so a stalled stdout consumer can't starve
    // the runtime; the guard must live for the whole process.
    let _log_guard = fuz_sys::logging::init_non_blocking_stdout("info,cord_server=info");

    if invocation == ServerInvocation::DbStatus {
        return fuz_db::status::run_db_status_cli(&cord_migration_namespaces()).await;
    }

    let password_hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2idHasher::new());

    match run_app(RunAppOptions {
        spine: SpineRunOptions::production(
            password_hasher,
            AuthCleanupSchedule::Every(DEFAULT_AUTH_CLEANUP_INTERVAL),
            DEFAULT_ADDR,
        ),
        extra_action_specs_factory: None,
        final_action_specs_factory: None,
    })
    .await
    {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => fuz_sys::cli::report_failure(&e, startup_hint(&e), e.exit_code()),
    }
}

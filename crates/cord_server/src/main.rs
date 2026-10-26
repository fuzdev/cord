//! `cordd`, the cord server binary.
//!
//! - `cordd` runs the server (see [`cord_server::runtime::run_app`]).
//! - `cordd db-status` prints a read-only migration report (connectivity,
//!   table counts, applied and pending migrations) via the spine's
//!   `fuz_db::status::run_db_status_cli`. It never mutates.
//!
//! Exit codes: `0` success, `2` the caller must fix something (arguments,
//! env), `1` everything else.

use std::process::ExitCode;
use std::sync::Arc;

use fuz_auth::{Argon2idHasher, PasswordHasher};

use cord_server::router::cord_migration_namespaces;
use cord_server::runtime::{DEFAULT_ADDR, DEFAULT_DRAIN_TIMEOUT, RunAppOptions, run_app};

const USAGE: &str = "usage: cordd [db-status]";

#[tokio::main]
async fn main() -> ExitCode {
    // Non-blocking stdout logging, so a stalled stdout consumer can't starve
    // the runtime; the guard must live for the whole process.
    let _log_guard = fuz_sys::logging::init_non_blocking_stdout("info,cord_server=info");

    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args.as_slice() {
        [] => {}
        ["db-status"] => {
            return fuz_db::status::run_db_status_cli(&cord_migration_namespaces()).await;
        }
        ["-h" | "--help"] => {
            println!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    }

    let password_hasher: Arc<dyn PasswordHasher> = Arc::new(Argon2idHasher::new());

    let Err(e) = run_app(RunAppOptions {
        password_hasher,
        default_addr: DEFAULT_ADDR,
        drain_timeout: DEFAULT_DRAIN_TIMEOUT,
        rate_limiters: fuz_auth::RateLimiterMode::Enforced,
        extra_action_specs_factory: None,
        pre_migration_hook: None,
        daemon_token_state: None,
        audit_inflight_tracking: false,
    })
    .await
    else {
        return ExitCode::SUCCESS;
    };
    eprintln!("error: {e}");
    let mut source = std::error::Error::source(&e);
    while let Some(cause) = source {
        eprintln!("  caused by: {cause}");
        source = cause.source();
    }
    if let Some(hint) = e.hint() {
        eprintln!("hint: {hint}");
    }
    ExitCode::from(e.exit_code())
}

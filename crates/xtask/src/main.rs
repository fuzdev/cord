//! `cargo xtask`: build automation for the cord workspace.
//!
//! One subcommand, `check-release`, the production-binary dep-graph audit:
//! no production binary (`cordd`) may link `fuz_testing` or `fuz_audit`.
//! The `testing_`-prefixed `testing_cordd` is exempt by name, and this crate
//! by its `dev_only` metadata. Dispatch, usage, and exit codes come from
//! [`fuz_audit::xtask_main`]: clean `0`, a policy violation `65`, a tooling
//! failure `69` or `70`.

use std::process::ExitCode;

fn main() -> ExitCode {
    fuz_audit::xtask_main()
}

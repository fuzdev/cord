//! `testing_cordd`'s command line, against the real binary: `--help` exits `0`
//! with usage on stdout, and an unknown command or an extra argument exits `2`
//! with usage on stderr before the env is read — the strict server command
//! line, with no arguments serving as the cross-process harness spawns it.

#![expect(
    clippy::expect_used,
    reason = "test helpers outside `#[test]` fns panic on a failed setup or check"
)]

use std::process::{Command, Output};

fn testing_cordd(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_testing_cordd"))
        .args(args)
        .env_clear()
        .output()
        .expect("spawn testing_cordd")
}

#[test]
fn help_prints_usage_and_exits_zero() {
    for flag in ["--help", "-h"] {
        let out = testing_cordd(&[flag]);
        assert_eq!(out.status.code(), Some(0), "{flag}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.starts_with("Usage: testing_cordd [command]"),
            "{flag}: {stdout}"
        );
    }
}

#[test]
fn an_unknown_argument_refuses_to_start() {
    for args in [&["strat"][..], &["--port", "1"], &["start", "extra"]] {
        let out = testing_cordd(args);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("Usage: testing_cordd [command]"),
            "{args:?}: {stderr}"
        );
        assert!(
            !stderr.contains("starting"),
            "{args:?}: refused before the startup sentinel: {stderr}"
        );
    }
}

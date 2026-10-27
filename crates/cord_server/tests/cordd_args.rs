//! `cordd`'s command line and start-failure exit codes, against the real
//! binary: `--help` exits `0` with usage on stdout; an unknown command or an
//! extra argument exits `2` with usage on stderr before the env is read; and a
//! configuration failure exits `2` with the error and cord's env hint on
//! stderr — a blank `PORT` among them. None of these needs a database or
//! binds a port.

#![allow(clippy::expect_used)]

use std::process::{Command, Output};

fn cordd(args: &[&str], env: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cordd"))
        .args(args)
        .env_clear()
        .envs(env.iter().copied())
        .output()
        .expect("spawn cordd")
}

#[test]
fn help_prints_usage_and_exits_zero() {
    for flag in ["--help", "-h"] {
        let out = cordd(&[flag], &[]);
        assert_eq!(out.status.code(), Some(0), "{flag}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.starts_with("Usage: cordd [command]"),
            "{flag}: {stdout}"
        );
    }
}

#[test]
fn an_unknown_argument_refuses_to_start() {
    for args in [&["strat"][..], &["--port", "1"], &["start", "extra"]] {
        let out = cordd(args, &[]);
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains("Usage: cordd [command]"),
            "{args:?}: {stderr}"
        );
        assert!(out.stdout.is_empty(), "{args:?}: nothing on stdout");
    }
}

#[test]
fn a_config_error_exits_two_with_the_env_hint() {
    for (args, env, error) in [
        (
            &[][..],
            &[][..],
            "error: invalid configuration: FUZ_ALLOWED_ORIGINS: ",
        ),
        (
            &["start"],
            &[("FUZ_ALLOWED_ORIGINS", "http://localhost"), ("PORT", "")],
            "error: invalid configuration: PORT: ",
        ),
    ] {
        let out = cordd(args, env);
        assert_eq!(out.status.code(), Some(2), "{args:?} {env:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains(error), "{args:?} {env:?}: {stderr}");
        assert!(
            stderr.contains("hint: see .env.development.example"),
            "{args:?} {env:?}: {stderr}"
        );
    }
}

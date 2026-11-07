//! Smoke check for `scripts/agent-bootstrap.sh`.
//!
//! The script parses (`bash -n`), `--help` exits 0 with the usage text, and
//! an unknown flag exits 2. None of these runs rustup or cargo.
#![cfg(unix)]

use std::path::PathBuf;
use std::process::Command;

fn script() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/agent-bootstrap.sh")
}

#[test]
fn agent_bootstrap_parses() {
    let out = Command::new("bash")
        .arg("-n")
        .arg(script())
        .output()
        .expect("bash runs");
    assert!(
        out.status.success(),
        "bash -n failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn agent_bootstrap_help_exits_zero() {
    let out = Command::new("bash")
        .arg(script())
        .arg("--help")
        .output()
        .expect("bash runs");
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.starts_with("usage: scripts/agent-bootstrap.sh [--check] [--help]"),
        "{stdout}"
    );
}

#[test]
fn agent_bootstrap_rejects_unknown_flag() {
    let out = Command::new("bash")
        .arg(script())
        .arg("--no-such-flag")
        .output()
        .expect("bash runs");
    assert_eq!(out.status.code(), Some(2));
}

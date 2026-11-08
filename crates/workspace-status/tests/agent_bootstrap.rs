//! Smoke check for `scripts/agent-bootstrap.sh`.
//!
//! The script parses (`bash -n`), `--help` exits 0 with the usage text, and
//! an unknown flag exits 2. `--check` runs against a stub `rustup` on `PATH`
//! and must never ask it to install anything. No test runs the real rustup
//! or cargo.
#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

// Only `unique_root` is used here; `git_env` serves the other test binaries.
#[allow(dead_code)]
#[path = "common/fixture.rs"]
mod fixture;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn script() -> PathBuf {
    repo_root().join("scripts/agent-bootstrap.sh")
}

/// Pinned channel of `rust-toolchain.toml` (`channel = "1.85.0"`).
fn pinned_channel() -> String {
    let toml = fs::read_to_string(repo_root().join("rust-toolchain.toml")).unwrap();
    toml.lines()
        .find_map(|line| {
            let value = line.strip_prefix("channel")?.trim_start();
            let value = value.strip_prefix('=')?.trim();
            Some(value.trim_matches('"').to_string())
        })
        .expect("channel in rust-toolchain.toml")
}

/// Stub `rustup`: logs `RUSTUP_AUTO_INSTALL|PWD|args` per call and fails
/// loudly when a call could install (auto-install not off, a
/// `RUSTUP_TOOLCHAIN` override, `toolchain install`, `component add`).
/// `STUB_TOOLCHAINS` is its `toolchain list`.
const STUB_RUSTUP: &str = r#"#!/usr/bin/env bash
printf '%s|%s|%s\n' "${RUSTUP_AUTO_INSTALL-unset}" "$PWD" "$*" >>"$STUB_LOG"
if [[ "${RUSTUP_AUTO_INSTALL-}" != 0 ]]; then
  echo "stub rustup: called without RUSTUP_AUTO_INSTALL=0: $*" >&2
  exit 97
fi
if [[ -n "${RUSTUP_TOOLCHAIN-}" ]]; then
  echo "stub rustup: RUSTUP_TOOLCHAIN override leaked: $*" >&2
  exit 96
fi
case "$*" in
  "toolchain install"* | "component add"* | "self update"* | update*)
    echo "stub rustup: asked to install: $*" >&2
    exit 98
    ;;
  --version) echo "rustup 1.27.1 (stub)" ;;
  "toolchain list") printf '%s\n' "$STUB_TOOLCHAINS" ;;
  "component list --installed --toolchain "*) printf 'clippy-stub-host\nrustfmt-stub-host\n' ;;
  *)
    echo "stub rustup: unexpected call: $*" >&2
    exit 99
    ;;
esac
"#;

/// Run `agent-bootstrap.sh --check` with the stub `rustup` first on `PATH`.
/// Returns the output and the stub's call log.
fn run_check_with_stub(toolchains: &str) -> (Output, String) {
    let root = fixture::unique_root("agent-bootstrap-stub");
    let bin = root.join("bin");
    fs::create_dir(&bin).unwrap();
    let rustup = bin.join("rustup");
    fs::write(&rustup, STUB_RUSTUP).unwrap();
    fs::set_permissions(&rustup, fs::Permissions::from_mode(0o755)).unwrap();
    let log = root.join("rustup.log");
    fs::write(&log, "").unwrap();

    let path = format!("{}:{}", bin.display(), std::env::var("PATH").unwrap());
    let out = Command::new("bash")
        .arg(script())
        .arg("--check")
        .current_dir(repo_root())
        .env("PATH", path)
        .env("STUB_LOG", &log)
        .env("STUB_TOOLCHAINS", toolchains)
        .env_remove("RUSTUP_AUTO_INSTALL")
        .env("RUSTUP_TOOLCHAIN", "stable")
        .output()
        .expect("bash runs");
    let calls = fs::read_to_string(&log).unwrap();
    let _ = fs::remove_dir_all(&root);
    (out, calls)
}

/// Every stub call had auto-install off, ran outside the repository, and
/// asked for nothing that installs.
fn assert_read_only_calls(calls: &str, repo: &Path) {
    assert!(!calls.is_empty(), "--check never ran rustup");
    for call in calls.lines() {
        let mut parts = call.splitn(3, '|');
        let (auto, pwd, args) = (
            parts.next().unwrap(),
            parts.next().unwrap(),
            parts.next().unwrap(),
        );
        assert_eq!(auto, "0", "RUSTUP_AUTO_INSTALL not 0: {call}");
        assert!(
            !Path::new(pwd).starts_with(repo),
            "rustup ran inside the repository (rust-toolchain.toml applies): {call}"
        );
        for verb in [
            "toolchain install",
            "component add",
            "install",
            "update",
            "self update",
        ] {
            assert!(
                !args.starts_with(verb),
                "--check asked rustup to {verb}: {call}"
            );
        }
    }
}

#[test]
fn agent_bootstrap_check_reports_ready_without_installing() {
    let channel = pinned_channel();
    let (out, calls) = run_check_with_stub(&format!(
        "stable-stub-host\n{channel}-stub-host (active, default)"
    ));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_read_only_calls(&calls, &repo_root());
    // A missing git or C linker on the test host is not what this test checks.
    if !stderr.contains("missing git") && !stderr.contains("missing C linker") {
        assert_eq!(out.status.code(), Some(0), "{stdout}\n{stderr}");
        assert!(stdout.contains("agent-bootstrap: ready"), "{stdout}");
    }
    assert!(
        stdout.contains(&format!("ok      toolchain {channel}")),
        "{stdout}"
    );
    assert!(stdout.contains("ok      component clippy"), "{stdout}");
    assert!(stdout.contains("ok      component rustfmt"), "{stdout}");
}

#[test]
fn agent_bootstrap_check_reports_missing_toolchain_without_installing() {
    let channel = pinned_channel();
    // Near misses only: a longer version and a prefixed name.
    let (out, calls) = run_check_with_stub(&format!(
        "stable-stub-host (default)\n{channel}9-stub-host\nx{channel}-stub-host"
    ));
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_read_only_calls(&calls, &repo_root());
    assert_eq!(out.status.code(), Some(1), "{stdout}\n{stderr}");
    assert!(
        stderr.contains(&format!("missing toolchain {channel}")),
        "{stderr}"
    );
    assert!(stderr.contains("missing component clippy"), "{stderr}");
    assert!(
        !calls.contains("component list"),
        "components probed on a missing toolchain: {calls}"
    );
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

//! Smoke checks for `wsctl`, the controller of the verify skill
//! (`.agents/skills/verify-workspace-status/scripts/wsctl`).
//!
//! These need bash only, no tmux and no build of a run, so they pass in CI:
//! the script parses, `--help` lists every command, each command has its own
//! help, and a bad command fails with a JSON failure object. Live recipes that
//! drive the TUI are in the skill's `features/` files. That wsctl spawns only
//! through `scripts/verify-tui.sh` is guarded in `release_watch.rs`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};

/// Every command of `wsctl --help`, in its order.
const COMMANDS: [&str; 11] = [
    "start",
    "doctor",
    "key",
    "type",
    "snap",
    "wait",
    "wait-ready",
    "resize",
    "run",
    "stop",
    "help",
];

fn wsctl() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../.agents/skills/verify-workspace-status/scripts/wsctl")
        .canonicalize()
        .expect("wsctl exists")
}

/// Run `bash wsctl <args>` with a verify root that holds no run, so a run of
/// the operator can never be picked up.
fn run_wsctl(args: &[&str]) -> Output {
    let root = std::env::temp_dir().join(format!("wsctl-test-root-{}", std::process::id()));
    Command::new("bash")
        .arg(wsctl())
        .args(args)
        .env("WS_VERIFY_ROOT", &root)
        .env_remove("WSCTL_RUN")
        .output()
        .expect("run bash wsctl")
}

#[test]
fn wsctl_parses_under_bash() {
    let out = Command::new("bash")
        .arg("-n")
        .arg(wsctl())
        .output()
        .expect("run bash -n");
    assert!(
        out.status.success(),
        "bash -n wsctl failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn wsctl_is_executable() {
    let mode = std::fs::metadata(wsctl())
        .expect("wsctl metadata")
        .permissions()
        .mode();
    assert_ne!(mode & 0o111, 0, "wsctl must keep its executable bit");
}

#[test]
fn wsctl_help_lists_every_command() {
    let out = run_wsctl(&["--help"]);
    assert!(out.status.success(), "wsctl --help exited {:?}", out.status);
    let help = String::from_utf8(out.stdout).expect("utf-8 help");
    for cmd in COMMANDS {
        assert!(
            help.lines().any(|line| {
                let line = line.trim_start();
                line == cmd || line.starts_with(&format!("{cmd} "))
            }),
            "wsctl --help must list `{cmd}`:\n{help}"
        );
    }
}

#[test]
fn wsctl_command_help_exits_zero() {
    for cmd in COMMANDS {
        let out = run_wsctl(&[cmd, "--help"]);
        assert!(
            out.status.success(),
            "wsctl {cmd} --help exited {:?}",
            out.status
        );
        let text = String::from_utf8(out.stdout).expect("utf-8 help");
        let first = if cmd == "help" {
            "wsctl: "
        } else {
            "usage: wsctl "
        };
        assert!(
            text.starts_with(first) && text.contains(cmd),
            "wsctl {cmd} --help must print its usage:\n{text}"
        );
    }
}

#[test]
fn wsctl_unknown_command_fails_with_json() {
    let out = run_wsctl(&["no-such-command"]);
    assert_eq!(out.status.code(), Some(1), "an unknown command must exit 1");
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    assert_eq!(
        stdout.trim(),
        r#"{"ok":false,"cmd":"no-such-command","error":"unknown command 'no-such-command' (wsctl --help)"}"#
    );
}

#[test]
fn wsctl_stop_without_a_run_is_ok() {
    let out = run_wsctl(&["stop"]);
    assert!(out.status.success(), "wsctl stop with no run must exit 0");
    let stdout = String::from_utf8(out.stdout).expect("utf-8 stdout");
    assert!(
        stdout.starts_with(r#"{"ok":true,"cmd":"stop","stopped":false"#),
        "wsctl stop with no run: {stdout}"
    );
}

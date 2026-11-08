//! Smoke checks for `wsctl`, the controller of the verify skill
//! (`.agents/skills/verify-workspace-status/scripts/wsctl`).
//!
//! These need bash only, no tmux and no build of a run, so they pass in CI:
//! the script parses, `--help` lists every command, each command has its own
//! help, a bad command fails with a JSON failure object, and a run that is not
//! this checkout's (outside the root, a shared socket, another checkout's
//! pointer, path or binary, an incomplete run.env) is refused before any tmux
//! call. Live recipes that drive the TUI are in the skill's `features/` files.
//! That wsctl spawns only through `scripts/verify-tui.sh` is guarded in
//! `release_watch.rs`.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

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

// ---------- run checks ----------

/// The real path of this checkout, as wsctl computes it (`pwd -P`).
fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root exists")
}

/// The binary path the driver names for this checkout (CARGO_TARGET_DIR unset).
fn this_binary() -> PathBuf {
    repo_root().join("target/debug/workspace-status")
}

/// The pointer key wsctl derives from a checkout path (POSIX `cksum`).
fn checkout_key(path: &Path) -> String {
    let out = Command::new("sh")
        .args(["-c", "printf '%s' \"$1\" | cksum", "sh"])
        .arg(path)
        .output()
        .expect("run cksum");
    String::from_utf8(out.stdout)
        .expect("utf-8 cksum")
        .split_whitespace()
        .next()
        .expect("cksum prints a checksum")
        .to_string()
}

/// A throwaway verify root plus a `tmux` stub on PATH that records every call
/// and fails, so a test sees whether wsctl reached tmux at all.
struct Sandbox {
    base: PathBuf,
}

impl Sandbox {
    fn new(name: &str) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let base = std::env::temp_dir().join(format!(
            "wsctl-test-{name}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&base).expect("create sandbox");
        // The real path: wsctl compares real paths (macOS temp dirs are links).
        let base = base.canonicalize().expect("sandbox real path");
        let stub = base.join("stub");
        std::fs::create_dir_all(&stub).expect("create stub dir");
        std::fs::create_dir_all(base.join("root")).expect("create root");
        let tmux = stub.join("tmux");
        std::fs::write(
            &tmux,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 1\n",
                base.join("tmux.log").display()
            ),
        )
        .expect("write tmux stub");
        std::fs::set_permissions(&tmux, std::fs::Permissions::from_mode(0o755))
            .expect("chmod tmux stub");
        Sandbox { base }
    }

    fn root(&self) -> PathBuf {
        self.base.join("root")
    }

    /// Write a run dir at `dir` with a run.env of these members.
    fn write_run(&self, dir: &Path, members: &[(&str, String)]) {
        std::fs::create_dir_all(dir).expect("create run dir");
        let mut body = String::new();
        for (k, v) in members {
            body.push_str(&format!("{k}={v}\n"));
        }
        std::fs::write(dir.join("run.env"), body).expect("write run.env");
    }

    /// A complete run.env of this checkout for run `id`, with `sock`.
    fn members(&self, id: &str, sock: &str) -> Vec<(&'static str, String)> {
        vec![
            ("RUN_ID", id.to_string()),
            ("RUN_SOCK", sock.to_string()),
            ("RUN_BIN", this_binary().display().to_string()),
            (
                "RUN_FIXTURE",
                self.base.join("fixture").display().to_string(),
            ),
            ("RUN_FORGE", String::new()),
            ("RUN_SIZE", "160x45".to_string()),
            ("RUN_REPO", repo_root().display().to_string()),
            (
                "RUN_SOCKET_PATH",
                self.base.join("sock").display().to_string(),
            ),
        ]
    }

    fn wsctl(&self, args: &[&str]) -> Output {
        Command::new("bash")
            .arg(wsctl())
            .args(args)
            .env(
                "PATH",
                format!("{}:/usr/bin:/bin", self.base.join("stub").display()),
            )
            .env("WS_VERIFY_ROOT", self.root())
            .env_remove("WSCTL_RUN")
            .env_remove("CARGO_TARGET_DIR")
            .output()
            .expect("run bash wsctl")
    }

    fn tmux_calls(&self) -> String {
        std::fs::read_to_string(self.base.join("tmux.log")).unwrap_or_default()
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.base);
    }
}

/// Assert `out` is a refusal: exit 1, one JSON failure line holding `needle`.
fn assert_refused(out: &Output, cmd: &str, needle: &str) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        out.status.code(),
        Some(1),
        "wsctl {cmd} must exit 1: {stdout}"
    );
    assert!(
        stdout.starts_with(&format!(r#"{{"ok":false,"cmd":"{cmd}","error":"#))
            && stdout.trim_end().ends_with('}')
            && stdout.trim_end().lines().count() == 1,
        "wsctl {cmd} must print one JSON failure object: {stdout}"
    );
    assert!(
        stdout.contains(needle),
        "wsctl {cmd}: want `{needle}` in {stdout}"
    );
}

#[test]
fn wsctl_refuses_a_run_dir_outside_the_root() {
    let sb = Sandbox::new("outside");
    let forged = sb.base.join("elsewhere/r1");
    sb.write_run(&forged, &sb.members("r1", "ws-verify-r1"));
    let dir = forged.display().to_string();
    for cmd in ["key", "snap", "stop"] {
        let mut args = vec!["--run", dir.as_str(), cmd];
        if cmd == "key" {
            args.push("j");
        }
        assert_refused(
            &sb.wsctl(&args),
            cmd,
            "not a run dir directly under the root",
        );
    }
    assert_eq!(sb.tmux_calls(), "", "a refused run must never reach tmux");
}

#[test]
fn wsctl_refuses_a_run_on_the_default_socket() {
    let sb = Sandbox::new("default-sock");
    sb.write_run(&sb.root().join("r1"), &sb.members("r1", "default"));
    assert_refused(
        &sb.wsctl(&["--run", "r1", "key", "j"]),
        "key",
        "socket name 'default' is not this run's private socket ws-verify-r1",
    );
    assert_refused(
        &sb.wsctl(&["--run", "r1", "snap"]),
        "snap",
        "socket name 'default'",
    );
    assert_eq!(sb.tmux_calls(), "", "a refused run must never reach tmux");
}

#[test]
fn wsctl_refuses_an_incomplete_or_misnamed_run_env() {
    let sb = Sandbox::new("fields");
    let mut members = sb.members("r1", "ws-verify-r1");
    members.retain(|(k, _)| *k != "RUN_BIN");
    sb.write_run(&sb.root().join("r1"), &members);
    assert_refused(
        &sb.wsctl(&["--run", "r1", "snap"]),
        "snap",
        "run.env has no RUN_BIN",
    );
    sb.write_run(&sb.root().join("r2"), &sb.members("r1", "ws-verify-r1"));
    assert_refused(
        &sb.wsctl(&["--run", "r2", "snap"]),
        "snap",
        "RUN_ID 'r1' is not the name of its dir",
    );
    assert_eq!(sb.tmux_calls(), "", "a refused run must never reach tmux");
}

#[test]
fn wsctl_refuses_a_run_of_another_checkout() {
    let sb = Sandbox::new("other-checkout");
    let other = sb.base.join("other-checkout");
    let mut members = sb.members("r1", "ws-verify-r1");
    for (k, v) in members.iter_mut() {
        match *k {
            "RUN_REPO" => *v = other.display().to_string(),
            "RUN_BIN" => {
                *v = other
                    .join("target/debug/workspace-status")
                    .display()
                    .to_string()
            }
            _ => {}
        }
    }
    sb.write_run(&sb.root().join("r1"), &members);
    // The pointer of this checkout names the other checkout's run.
    std::fs::write(
        sb.root()
            .join(format!("wsctl-current-{}", checkout_key(&repo_root()))),
        format!("{}\n", sb.root().join("r1").display()),
    )
    .expect("write pointer");
    assert_refused(&sb.wsctl(&["snap"]), "snap", "it is a run of the checkout");
    // Same path, another binary (a shared or changed CARGO_TARGET_DIR).
    let mut members = sb.members("r2", "ws-verify-r2");
    for (k, v) in members.iter_mut() {
        if *k == "RUN_BIN" {
            *v = other.join("bin/workspace-status").display().to_string();
        }
    }
    sb.write_run(&sb.root().join("r2"), &members);
    assert_refused(
        &sb.wsctl(&["--run", "r2", "key", "j"]),
        "key",
        "not this checkout's binary",
    );
    assert_eq!(sb.tmux_calls(), "", "a refused run must never reach tmux");
}

#[test]
fn wsctl_does_not_see_the_current_run_of_another_checkout() {
    let sb = Sandbox::new("other-pointer");
    // A valid run of this checkout, but only another checkout's pointer names it.
    let run = sb.root().join("r1");
    sb.write_run(&run, &sb.members("r1", "ws-verify-r1"));
    let other_key = checkout_key(&sb.base.join("other-checkout"));
    assert_ne!(other_key, checkout_key(&repo_root()));
    std::fs::write(
        sb.root().join(format!("wsctl-current-{other_key}")),
        format!("{}\n", run.display()),
    )
    .expect("write pointer");
    assert_refused(
        &sb.wsctl(&["snap"]),
        "snap",
        "no current run of this checkout",
    );
    let out = sb.wsctl(&["stop"]);
    assert!(
        out.status.success(),
        "stop with no run of this checkout exits 0"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).contains(r#""stopped":false"#),
        "stop must not stop another checkout's run"
    );
    assert_eq!(
        sb.tmux_calls(),
        "",
        "another checkout's run must never reach tmux"
    );
}

#[test]
fn wsctl_drives_a_valid_run_only_on_its_private_socket() {
    let sb = Sandbox::new("valid");
    sb.write_run(&sb.root().join("r1"), &sb.members("r1", "ws-verify-r1"));
    // The stub tmux has no session, so key fails, but only after the checks.
    let out = sb.wsctl(&["--run", "r1", "key", "j"]);
    assert_refused(&out, "key", "no tmux session for run");
    let calls = sb.tmux_calls();
    assert!(!calls.is_empty(), "a valid run must reach tmux");
    for line in calls.lines() {
        assert!(
            line.starts_with("-L ws-verify-r1 "),
            "every tmux call must use the run's private socket: {line}"
        );
    }
}

#[test]
fn wsctl_type_treats_words_after_dashdash_as_text() {
    let sb = Sandbox::new("type");
    // No run: `-- --help` is the text, so the parse reaches the run lookup
    // instead of printing the help; `--run` after the text is a second word.
    assert_refused(
        &sb.wsctl(&["type", "--", "--help"]),
        "type",
        "no current run of this checkout",
    );
    assert_refused(
        &sb.wsctl(&["type", "x", "--run"]),
        "type",
        "type: give one text argument",
    );
}

//! `workspace-status doctor` with stdout not a terminal: the report text,
//! the rows that need a terminal or the TUI, and the exit code.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[path = "common/fixture.rs"]
mod fixture;

use fixture::{git_env, unique_root};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_workspace-status"))
}

/// A workspace with one committed repo per name that pushes to a GitHub
/// remote, plus the temp homes the spawn uses. Returns `(root, workspace)`.
fn workspace_with(repos: &[&str]) -> (PathBuf, PathBuf) {
    let root = unique_root("ws-doctor-cli");
    let workspace = root.join("workspace");
    for name in repos {
        let repo = workspace.join(name);
        fs::create_dir_all(&repo).unwrap();
        for args in [
            &["init", "-q"][..],
            &["commit", "-q", "--allow-empty", "-m", "seed"][..],
            &["remote", "add", "origin", "https://github.com/octo/app.git"][..],
        ] {
            let status = Command::new("git")
                .args(args)
                .current_dir(&repo)
                .envs(git_env())
                .status()
                .expect("git runs");
            assert!(status.success(), "git {args:?} in {name}");
        }
    }
    for dir in ["xdg-config", "xdg-state"] {
        fs::create_dir_all(root.join(dir)).unwrap();
    }
    (root, workspace)
}

/// Run the binary from `cwd` with `args`, stdout and stderr piped (not a
/// terminal), and stdin empty. The user config, every store, and the
/// workspace pin come from the temp `root`, never the operator's.
fn run(root: &Path, cwd: &Path, args: &[&str]) -> Output {
    let state = root.join("xdg-state");
    Command::new(bin())
        .args(args)
        .current_dir(cwd)
        .env_remove("WS_STATUS_WORKSPACE")
        .env_remove("WORKSPACE_STATUS_GIT")
        .env_remove("TMUX")
        .env("XDG_CONFIG_HOME", root.join("xdg-config"))
        .env("XDG_STATE_HOME", &state)
        .env("WS_STATUS_UI_STORE", state.join("ui-state.json"))
        .env(
            "WS_STATUS_UPDATE_CHECK_STORE",
            state.join("update-check.json"),
        )
        .envs(git_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .expect("workspace-status runs")
}

fn stdout(out: &Output) -> String {
    String::from_utf8(out.stdout.clone()).expect("UTF-8 stdout")
}

/// The row line `  <status padded to 7> <name>` and the line under it.
fn row_and_why<'a>(lines: &'a [&'a str], status: &str, name: &str) -> Option<&'a str> {
    let head = format!("  {status:<7} {name}");
    let at = lines.iter().position(|line| *line == head)?;
    lines.get(at + 1).map(|line| line.trim_start())
}

/// A clean workspace: the report prints as plain text, the rows that need
/// a terminal or the TUI are unknown with why, no row fails, and the exit
/// code is 0. Fail on an escape byte, a missing row, or a non-zero exit.
#[test]
fn doctor_without_a_terminal_prints_unknown_rows_and_exits_zero() {
    let (root, workspace) = workspace_with(&["app"]);
    let out = run(&root, &workspace, &["doctor"]);
    let text = stdout(&out);
    assert_eq!(
        out.status.code(),
        Some(0),
        "stdout:\n{text}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!text.contains('\x1b'), "no escape codes:\n{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("Doctor · setup checkup · workspace-status v"),
        "{text}"
    );
    let counts: Vec<&str> = lines[1].split(" · ").collect();
    assert_eq!(counts.len(), 4, "{text}");
    for (count, label) in counts.iter().zip(["ok", "warn", "fail", "unknown"]) {
        let (n, word) = count.split_once(' ').expect("count and word");
        assert!(n.parse::<usize>().is_ok() && word == label, "{text}");
    }
    assert_eq!(counts[2], "0 fail", "{text}");
    assert_eq!(lines[2..4], ["", "TERMINAL"], "{text}");
    assert_eq!(
        row_and_why(&lines, "unknown", "Terminal size"),
        Some("stdout is not a terminal."),
        "{text}"
    );
    assert_eq!(
        row_and_why(&lines, "unknown", "Mouse wheel (live test)"),
        Some("TUI only: open Doctor: setup checkup in the TUI and scroll once."),
        "{text}"
    );
    for name in [
        "Terminal (XTVERSION)",
        "SGR mouse (1006)",
        "Bracketed paste (2004)",
        "Synchronized output (2026)",
        "Kitty keyboard (CSI u)",
        "Background (OSC 11)",
    ] {
        assert_eq!(
            row_and_why(&lines, "unknown", name),
            Some("Not asked: stdin or stdout is not a terminal."),
            "{name}:\n{text}"
        );
    }
    assert_eq!(row_and_why(&lines, "ok", "Mouse"), Some("On."), "{text}");
    for group in ["TOOLS", "FILES"] {
        assert!(lines.contains(&group), "{group}:\n{text}");
    }
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("  ok      State files")),
        "{text}"
    );
    assert!(
        !root.join("xdg-state").join("ui-state.json").exists(),
        "the Doctor writes no store"
    );
    let _ = fs::remove_dir_all(root);
}

/// A store file that is not its JSON makes the State files row fail with
/// the line to move it aside, and the exit code 1. `-C` after `doctor`
/// pins the workspace from another directory.
#[test]
fn doctor_with_a_corrupt_store_fails_its_row_and_exits_one() {
    let (root, workspace) = workspace_with(&["app"]);
    let ui_store = root.join("xdg-state").join("ui-state.json");
    fs::write(&ui_store, b"{not json").unwrap();
    let ws = workspace.display().to_string();
    let out = run(&root, &root, &["doctor", "-C", &ws]);
    let text = stdout(&out);
    assert_eq!(out.status.code(), Some(1), "{text}");
    let lines: Vec<&str> = text.lines().collect();
    assert!(lines[1].contains("1 fail"), "{text}");
    assert!(lines.contains(&"  fail    State files"), "{text}");
    assert!(
        lines
            .iter()
            .any(|line| line.starts_with("          fix: mv ") && line.contains("ui-state.json")),
        "{text}"
    );
    assert_eq!(fs::read(&ui_store).unwrap(), b"{not json", "left unchanged");
    let _ = fs::remove_dir_all(root);
}

/// `./doctor` filters the repo named `doctor` instead of running the
/// Doctor; `doctor` with a mode flag is a usage error (exit 2).
#[test]
fn dot_slash_doctor_filters_the_repo_and_doctor_rejects_mode_flags() {
    let (root, workspace) = workspace_with(&["app", "doctor"]);
    let out = run(&root, &workspace, &["--json", "./doctor"]);
    assert_eq!(out.status.code(), Some(0), "{}", stdout(&out));
    let snapshot: serde_json::Value = serde_json::from_slice(&out.stdout).expect("JSON snapshot");
    let repos: Vec<&str> = snapshot["repos"]
        .as_array()
        .expect("repos array")
        .iter()
        .map(|repo| repo["repo"].as_str().expect("repo name"))
        .collect();
    assert_eq!(repos, ["doctor"]);

    let out = run(&root, &workspace, &["--json", "doctor"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stdout.is_empty());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--json"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(root);
}

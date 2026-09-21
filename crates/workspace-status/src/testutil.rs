//! Shared git fixtures for this crate's inline unit tests.
//!
//! Integration tests under `tests/` use `tests/common/seed.rs`. Inline
//! `#[cfg(test)]` modules cannot reach that file, so each one used to carry
//! its own `git_env` / `git` / `init_repo` copy — five `git_env` variants
//! and fifteen copies of the same init block. This module is the one place
//! those live for `src/`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::git::git_binary;

/// Deterministic author / committer plus config isolation.
///
/// `GIT_CONFIG_GLOBAL` / `GIT_CONFIG_NOSYSTEM` keep a developer's own
/// `init.defaultBranch`, hooks, and aliases out of fixture repos.
pub fn git_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_AUTHOR_NAME", "workspace-status test"),
        ("GIT_AUTHOR_EMAIL", "workspace-status-test@example.invalid"),
        ("GIT_COMMITTER_NAME", "workspace-status test"),
        (
            "GIT_COMMITTER_EMAIL",
            "workspace-status-test@example.invalid",
        ),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ]
}

/// Run `git` in `cwd` with [`git_env`]. Panics on a non-zero exit.
pub fn git(cwd: &Path, args: &[&str]) {
    let mut cmd = Command::new(git_binary());
    cmd.args(args).current_dir(cwd);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let status = cmd.status().expect("git runs");
    assert!(status.success(), "git {args:?} in {}", cwd.display());
}

/// Run `git` in `cwd` and return trimmed stdout. Panics on a non-zero exit.
pub fn git_stdout(cwd: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new(git_binary());
    cmd.args(args).current_dir(cwd);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("git runs");
    assert!(out.status.success(), "git {args:?} in {}", cwd.display());
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// `git init -b main` in `dir`, creating it when missing.
///
/// `git init -b` needs git 2.28 (2020). The checkout fallback keeps older
/// hosts working; it is a no-op on any supported CI image.
pub fn init_repo_empty(dir: &Path) {
    fs::create_dir_all(dir).expect("create fixture dir");
    let mut cmd = Command::new(git_binary());
    cmd.args(["init", "-q", "-b", "main"]).current_dir(dir);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    let born_on_main = cmd.status().map(|s| s.success()).unwrap_or(false);
    if !born_on_main {
        git(dir, &["init", "-q"]);
        git(dir, &["checkout", "-q", "-b", "main"]);
    }
}

/// [`init_repo_empty`] plus a committed `README.md` on `main`.
pub fn init_repo(dir: &Path) {
    init_repo_empty(dir);
    fs::write(dir.join("README.md"), "# seed\n").expect("write seed README");
    git(dir, &["add", "README.md"]);
    git(dir, &["commit", "-q", "-m", "seed"]);
}

/// A unique temp path for `prefix`. Not created.
///
/// Process id plus a counter keeps parallel test threads apart; the
/// timestamp keeps reruns apart.
pub fn unique_dir(prefix: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock after epoch")
        .as_nanos();
    let seq = NEXT.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "{prefix}-{pid}-{nanos}-{seq}",
        pid = std::process::id()
    ))
}

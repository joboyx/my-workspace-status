//! TTY-only workspace layouts on top of the shared TUI e2e seed.
//!
//! Shared git helpers, daily / focus fixtures, long-path / long-diff files,
//! and the primary+linked family live in `tests/common/seed.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub use super::common::seed::{
    daily_workspace, focus_workspace, git, git_env, seed_long_diff_file, seed_long_path_file,
    seed_long_subject_repo, seed_many_commit_files, seed_merge_mark_family,
    seed_primary_and_linked_family, seed_repo, seed_tall_graph, unique_root,
};

use super::common::seed::new_workspace;

fn git_path_arg(path: &Path) -> String {
    path.to_str().expect("utf-8 path").to_string()
}

fn configure_identity(repo: &Path) {
    git(repo, &["config", "user.name", "workspace-status e2e"]);
    git(
        repo,
        &[
            "config",
            "user.email",
            "workspace-status-e2e@example.invalid",
        ],
    );
}

/// Two visible checkouts for streamed-collect e2e (`fast` clean, `slow` dirty).
pub fn stream_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace) = new_workspace("ws-tui-tty-stream");
    seed_repo(&workspace, "fast", "feature/fast", false);
    seed_repo(&workspace, "slow", "feature/slow", true);
    (root, workspace)
}

/// Bare origin under `root` so fetch / pull / push stay off the network.
pub fn seed_bare_remote(root: &Path) -> PathBuf {
    let remote = root.join("origin.git");
    let init = Command::new("git")
        .args(["init", "-q", "--bare", "-b", "main"])
        .arg(&remote)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        fs::create_dir_all(&remote).unwrap();
        git(&remote, &["init", "-q", "--bare"]);
    }
    remote
}

fn seed_tracking_repo(workspace: &Path, name: &str, remote: &Path) {
    seed_repo(workspace, name, "main", false);
    let repo = workspace.join(name);
    git(&repo, &["remote", "add", "origin", &git_path_arg(remote)]);
    git(&repo, &["push", "-u", "origin", "main", "--quiet"]);
}

fn write_commit(repo: &Path, file: &str, body: &str, message: &str) {
    fs::write(repo.join(file), body).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-q", "-m", message]);
}

/// Clone `remote` into `helper`, commit, and push. Local checkouts are unchanged.
pub fn push_commit_to_remote(remote: &Path, helper: &Path, file: &str, body: &str, message: &str) {
    if !helper.join(".git").exists() {
        let mut cmd = Command::new("git");
        cmd.args(["clone", "-q", &git_path_arg(remote), &git_path_arg(helper)]);
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        cmd.stdout(Stdio::null()).stderr(Stdio::piped());
        let out = cmd.output().expect("git clone runs");
        assert!(
            out.status.success(),
            "git clone failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        configure_identity(helper);
    }
    write_commit(helper, file, body, message);
    git(helper, &["push", "-q", "origin", "HEAD"]);
}

fn tracking_workspace(prefix: &str, name: &str) -> (PathBuf, PathBuf, PathBuf) {
    let root = unique_root(prefix);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    let remote = seed_bare_remote(&root);
    seed_tracking_repo(&workspace, name, &remote);
    (root, workspace, remote)
}

/// Local `syncbox` tracking a bare origin. Origin is one commit ahead; local
/// has not fetched yet (status still looks in-sync).
pub fn unfetched_behind_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace, remote) = tracking_workspace("ws-tui-tty-fetch", "syncbox");
    push_commit_to_remote(
        &remote,
        &root.join("helper"),
        "origin-tip.txt",
        "from origin\n",
        "origin-tip-commit",
    );
    (root, workspace)
}

/// Local `syncbox` is behind `origin/main` (fetch already ran).
pub fn behind_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace) = unfetched_behind_workspace();
    git(&workspace.join("syncbox"), &["fetch", "-q"]);
    (root, workspace)
}

/// Local `syncbox` is one commit ahead of origin.
pub fn ahead_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace, _remote) = tracking_workspace("ws-tui-tty-push", "syncbox");
    write_commit(
        &workspace.join("syncbox"),
        "ahead-tip.txt",
        "local ahead\n",
        "ahead-tip-commit",
    );
    (root, workspace)
}

/// Primary checkout plus a linked worktree for `W` remove.
pub fn worktree_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace) = new_workspace("ws-tui-tty-wt");
    seed_primary_and_linked_family(&workspace);
    (root, workspace)
}

/// Family with a default-tip linked branch and a truly merged linked branch.
pub fn merge_mark_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace) = new_workspace("ws-tui-tty-merge-mark");
    seed_merge_mark_family(&workspace);
    (root, workspace)
}

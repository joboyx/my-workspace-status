//! Temp git workspaces for the real-TTY e2e. Same seed style as the
//! headless daily suite — real repos, no mocked git.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn git_env() -> Vec<(&'static str, &'static str)> {
    vec![
        ("GIT_AUTHOR_NAME", "workspace-status e2e"),
        ("GIT_AUTHOR_EMAIL", "workspace-status-e2e@example.invalid"),
        ("GIT_COMMITTER_NAME", "workspace-status e2e"),
        (
            "GIT_COMMITTER_EMAIL",
            "workspace-status-e2e@example.invalid",
        ),
        ("GIT_CONFIG_GLOBAL", "/dev/null"),
        ("GIT_CONFIG_NOSYSTEM", "1"),
    ]
}

pub fn git(cwd: &Path, args: &[&str]) {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(cwd);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::null()).stderr(Stdio::piped());
    let out = cmd.output().expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

pub fn unique_root(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

pub fn seed_repo(workspace: &Path, name: &str, branch: &str, dirty: bool) {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", branch])
        .current_dir(&repo)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        git(&repo, &["init", "-q"]);
        git(&repo, &["checkout", "-q", "-b", branch]);
    }
    git(&repo, &["config", "user.name", "workspace-status e2e"]);
    git(
        &repo,
        &[
            "config",
            "user.email",
            "workspace-status-e2e@example.invalid",
        ],
    );
    fs::write(repo.join("README.md"), format!("# {name}\n")).unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", &format!("seed {name}")]);
    if dirty {
        fs::write(repo.join("README.md"), format!("# {name}\ndirty\n")).unwrap();
    }
}

/// Merge diamond + stash spur on a non-default branch so the repo stays visible.
pub fn seed_merge_graph(workspace: &Path, name: &str) {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        git(&repo, &["init", "-q"]);
        git(&repo, &["checkout", "-q", "-b", "main"]);
    }
    git(&repo, &["config", "user.name", "workspace-status e2e"]);
    git(
        &repo,
        &[
            "config",
            "user.email",
            "workspace-status-e2e@example.invalid",
        ],
    );
    fs::write(repo.join("README.md"), "# root\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "root"]);
    git(&repo, &["checkout", "-q", "-b", "left"]);
    fs::write(repo.join("left.txt"), "left\n").unwrap();
    git(&repo, &["add", "left.txt"]);
    git(&repo, &["commit", "-q", "-m", "left"]);
    git(&repo, &["checkout", "-q", "main"]);
    fs::write(repo.join("right.txt"), "right\n").unwrap();
    git(&repo, &["add", "right.txt"]);
    git(&repo, &["commit", "-q", "-m", "right"]);
    git(&repo, &["merge", "--no-ff", "-m", "merge", "left"]);
    git(&repo, &["checkout", "-q", "-b", "feature/graph"]);
    fs::write(repo.join("wip.txt"), "stash me\n").unwrap();
    git(&repo, &["add", "wip.txt"]);
    git(&repo, &["stash", "push", "-q", "-m", "WIP on graph"]);
}

/// Diverging local branches for graph focus (`o`).
pub fn seed_branch_focus_graph(workspace: &Path, name: &str) {
    let repo = workspace.join(name);
    fs::create_dir_all(&repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", "main"])
        .current_dir(&repo)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        git(&repo, &["init", "-q"]);
        git(&repo, &["checkout", "-q", "-b", "main"]);
    }
    git(&repo, &["config", "user.name", "workspace-status e2e"]);
    git(
        &repo,
        &[
            "config",
            "user.email",
            "workspace-status-e2e@example.invalid",
        ],
    );
    fs::write(repo.join("README.md"), "# focus-root-commit\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "focus-root-commit"]);
    git(&repo, &["checkout", "-q", "-b", "feature/keep"]);
    fs::write(repo.join("keep.txt"), "keep\n").unwrap();
    git(&repo, &["add", "keep.txt"]);
    git(&repo, &["commit", "-q", "-m", "keep-leaf-commit"]);
    git(&repo, &["checkout", "-q", "main"]);
    fs::write(repo.join("main.txt"), "main\n").unwrap();
    git(&repo, &["add", "main.txt"]);
    git(&repo, &["commit", "-q", "-m", "main-leaf-commit"]);
    git(&repo, &["checkout", "-q", "-b", "topic/noise", "HEAD~1"]);
    fs::write(repo.join("noise.txt"), "noise\n").unwrap();
    git(&repo, &["add", "noise.txt"]);
    git(&repo, &["commit", "-q", "-m", "noise-leaf-commit"]);
    git(&repo, &["checkout", "-q", "feature/keep"]);
}

pub fn seed_long_path_file(workspace: &Path) {
    let long_dir = workspace.join("app/src/app/workspace-tree");
    fs::create_dir_all(&long_dir).unwrap();
    fs::write(
        long_dir.join("very-long-workspace-tree-component-name-TAIL99.ts"),
        "export const pan = 1;\n",
    )
    .unwrap();
}

/// Daily-path workspace: dirty `app`, clean `lib`, ignored `notes`, merge graph.
pub fn daily_workspace() -> (PathBuf, PathBuf) {
    let root = unique_root("ws-tui-tty");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    seed_repo(&workspace, "lib", "main", false);
    seed_repo(&workspace, "notes", "main", true);
    seed_merge_graph(&workspace, "merger");
    fs::write(
        workspace.join(".workspace-status-config.json"),
        "{\n  \"ignoredRepos\": [\"notes\"]\n}\n",
    )
    .unwrap();
    (root, workspace)
}

/// Two visible checkouts for streamed-collect e2e (`fast` clean, `slow` dirty).
pub fn stream_workspace() -> (PathBuf, PathBuf) {
    let root = unique_root("ws-tui-tty-stream");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "fast", "feature/fast", false);
    seed_repo(&workspace, "slow", "feature/slow", true);
    (root, workspace)
}

pub fn focus_workspace() -> (PathBuf, PathBuf) {
    let root = unique_root("ws-tui-tty-focus");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_branch_focus_graph(&workspace, "focusbox");
    (root, workspace)
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

fn git_path_arg(path: &Path) -> String {
    path.to_str().expect("utf-8 path").to_string()
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

//! Temp git workspaces shared by TestBackend and PTY TUI e2e.
//!
//! Real repos, no mocked git. Same seed style as `snapshot_contract.rs`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use super::hscroll::{GRAPH_HSCROLL_TAIL, TREE_HSCROLL_DIR, TREE_HSCROLL_FILE};

/// Author / committer plus isolated git config for e2e repos.
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

/// Run `git` in `cwd` with [`git_env`]. Panics on non-zero exit.
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

/// Unique temp directory: `{prefix}-{nanos}`.
pub fn unique_root(prefix: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "{prefix}-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ))
}

/// Unique root plus a `workspace/` directory inside it.
pub fn new_workspace(prefix: &str) -> (PathBuf, PathBuf) {
    let root = unique_root(prefix);
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
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

fn init_repo(repo: &Path, branch: &str) {
    fs::create_dir_all(repo).unwrap();
    let init = Command::new("git")
        .args(["init", "-q", "-b", branch])
        .current_dir(repo)
        .status();
    if init.map(|s| s.success()).unwrap_or(false) == false {
        git(repo, &["init", "-q"]);
        git(repo, &["checkout", "-q", "-b", branch]);
    }
    configure_identity(repo);
}

/// One checkout: README commit, optional dirty README.
pub fn seed_repo(workspace: &Path, name: &str, branch: &str, dirty: bool) {
    let repo = workspace.join(name);
    init_repo(&repo, branch);
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
    init_repo(&repo, "main");
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
    init_repo(&repo, "main");
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

/// Long tree path for the shared hscroll oracle (`hscroll` prefix vs tail).
pub fn seed_long_path_file(workspace: &Path) {
    let long_dir = workspace.join(TREE_HSCROLL_DIR);
    fs::create_dir_all(&long_dir).unwrap();
    fs::write(long_dir.join(TREE_HSCROLL_FILE), "export const pan = 1;\n").unwrap();
}

/// Linear history of 30 commits so the graph list overflows a default pane.
pub fn seed_tall_graph(workspace: &Path, name: &str) {
    seed_repo(workspace, name, "main", false);
    let repo = workspace.join(name);
    for i in 0..30 {
        fs::write(repo.join("count.txt"), format!("{i}\n")).unwrap();
        git(&repo, &["add", "count.txt"]);
        git(&repo, &["commit", "-q", "-m", &format!("count {i}")]);
    }
    git(&repo, &["checkout", "-q", "-b", "feature/tall"]);
}

/// One commit with many files so the commit-file list overflows a default pane.
pub fn seed_many_commit_files(workspace: &Path, name: &str, count: usize) {
    seed_repo(workspace, name, "main", false);
    let repo = workspace.join(name);
    for i in 0..count {
        fs::write(
            repo.join(format!("keepmid-{i:02}.txt")),
            format!("keepmid-{i:02}-body\n"),
        )
        .unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "keepmid-files-commit"]);
    git(&repo, &["checkout", "-q", "-b", "feature/files"]);
}

/// Long graph subject so horizontal pan must reveal `UNIQUE_GRAP`.
///
/// `n` prefix plus [`GRAPH_HSCROLL_TAIL`]. Do not `/` search the tail first.
pub fn seed_long_subject_repo(workspace: &Path, name: &str) {
    let repo = workspace.join(name);
    init_repo(&repo, "main");
    fs::write(repo.join("README.md"), "# long\n").unwrap();
    git(&repo, &["add", "README.md"]);
    git(&repo, &["commit", "-q", "-m", "root"]);
    git(&repo, &["checkout", "-q", "-b", "feature/long-subject"]);
    fs::write(repo.join("wip.txt"), "x\n").unwrap();
    git(&repo, &["add", "wip.txt"]);
    let subject = format!("{}{GRAPH_HSCROLL_TAIL}", "n".repeat(80));
    git(&repo, &["commit", "-q", "-m", &subject]);
}

/// Long line plus many rows so a focused file-diff can pan and scroll.
///
/// `tail` is usually `hscroll::DIFF_HSCROLL_TAIL`.
pub fn seed_long_diff_file(workspace: &Path, name: &str, tail: &str) {
    let mut body = format!("{}{tail}\n", "n".repeat(80));
    for i in 0..40 {
        body.push_str(&format!("line {i}\n"));
    }
    fs::write(workspace.join("app").join(name), body).unwrap();
}

/// Dirty `app`, clean `lib`, ignored `notes`, merge graph.
pub fn daily_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace) = new_workspace("ws-tui-e2e");
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

/// One checkout with diverging local branches (`focusbox`).
pub fn focus_workspace() -> (PathBuf, PathBuf) {
    let (root, workspace) = new_workspace("ws-tui-e2e-focus");
    seed_branch_focus_graph(&workspace, "focusbox");
    (root, workspace)
}

/// Primary checkout plus a linked worktree (no extra dirty README).
pub fn seed_primary_and_linked_family(workspace: &Path) {
    seed_repo(workspace, "app", "main", false);
    let repo = workspace.join("app");
    fs::write(repo.join(".gitignore"), ".worktrees/\n").unwrap();
    git(&repo, &["add", ".gitignore"]);
    git(&repo, &["commit", "-q", "-m", "ignore linked worktree dir"]);
    git(&repo, &["checkout", "-q", "-b", "feature/primary-open"]);
    fs::write(repo.join("primary.txt"), "primary off default\n").unwrap();
    git(&repo, &["add", "primary.txt"]);
    git(&repo, &["commit", "-q", "-m", "primary off default"]);
    fs::create_dir_all(repo.join(".worktrees")).unwrap();
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "feature/linked-open",
            ".worktrees/feat",
            "main",
        ],
    );
    let linked = repo.join(".worktrees/feat");
    fs::write(linked.join("open.txt"), "linked open vs default\n").unwrap();
    git(&linked, &["add", "open.txt"]);
    git(&linked, &["commit", "-q", "-m", "linked open"]);
}

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

/// `git init -b main` in `dir`, creating it when missing.
///
/// `git init -b` needs git 2.28 (2020). The checkout fallback keeps older
/// hosts working; it is a no-op on any supported CI image.
///
/// Identity is written into the repo's own config, not left to [`git_env`].
/// The env vars only reach commands this module spawns, and these fixtures
/// exist to drive production code — `merge_into_head`, `stash_push`, and the
/// rest spawn their own git with no such env. Without a repo-local identity
/// they fail with `empty ident name` on any host that has no global
/// `user.email`, which is every clean CI runner.
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
    git(dir, &["config", "user.name", "workspace-status test"]);
    git(
        dir,
        &[
            "config",
            "user.email",
            "workspace-status-test@example.invalid",
        ],
    );
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

/// What the checkout under test holds when the maintainer force-pushes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RewriteKind {
    /// Nothing of yours: the checkout only holds the dropped bot commits.
    Dropped,
    /// One commit of yours sits on top of the dropped bot commits.
    YourCommit,
    /// Normal divergence: your commit, and `main` moved on without a rewrite.
    NormalDivergence,
}

/// A bare remote, a maintainer clone, and a checkout after the maintainer
/// force-pushed `main`. Built by [`upstream_rewrite_fixture`].
pub struct RewriteFixture {
    /// Directory that holds every repo of the fixture. Removed on drop.
    pub root: PathBuf,
    /// The checkout under test, on `main` tracking `origin/main`, fetched.
    pub checkout: PathBuf,
    /// Tip of the checkout's `main` before the fetch, with your commit when any.
    pub local_tip: String,
    /// The base commit both histories share.
    pub base: String,
    /// The first bot commit on top of `base` (the old remote history).
    pub bot1: String,
    /// The remote tip after the push (`origin/main`).
    pub upstream_tip: String,
}

impl Drop for RewriteFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

/// Commit one file in `dir` and return the new `HEAD`.
fn commit_file_at(dir: &Path, name: &str, msg: &str) -> String {
    fs::write(dir.join(name), format!("{name}\n")).expect("write fixture file");
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", msg]);
    head_sha(dir)
}

/// Full `HEAD` sha of the repo at `dir`.
fn head_sha(dir: &Path) -> String {
    let out = Command::new(git_binary())
        .args(["rev-parse", "HEAD"])
        .current_dir(dir)
        .output()
        .expect("git rev-parse runs");
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// Clone a non-empty `remote` into `root/<name>` with a repo-local identity.
fn clone_with_identity(root: &Path, remote: &Path, name: &str) -> PathBuf {
    git(
        root,
        &["clone", "-q", remote.to_str().expect("utf8 path"), name],
    );
    let dir = root.join(name);
    git(&dir, &["config", "user.name", "workspace-status test"]);
    git(
        &dir,
        &[
            "config",
            "user.email",
            "workspace-status-test@example.invalid",
        ],
    );
    dir
}

/// Build the "a maintainer force-pushed `main` and dropped bot commits" story.
///
/// A bare remote gets a base commit and two bot commits from a maintainer
/// clone. The checkout under test clones them. Then the maintainer drops the
/// bot commits (`reset --hard HEAD~2`), commits a release commit, and runs
/// `push --force`. The checkout fetches, so `origin/main` and its reflog show
/// the rewrite. `kind` picks what the checkout holds, see [`RewriteKind`].
pub fn upstream_rewrite_fixture(kind: RewriteKind) -> RewriteFixture {
    let root = unique_dir("ws-rewrite");
    fs::create_dir_all(&root).expect("create fixture root");
    let remote = root.join("remote.git");
    git(
        &root,
        &[
            "init",
            "-q",
            "--bare",
            "-b",
            "main",
            remote.to_str().unwrap(),
        ],
    );
    // `git clone` of an empty remote warns; build the maintainer repo by init.
    let maint = root.join("maint");
    init_repo_empty(&maint);
    git(
        &maint,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    let base = commit_file_at(&maint, "base.txt", "base");
    let bot1 = commit_file_at(&maint, "bot1.txt", "chore(deps): bot bump 1");
    commit_file_at(&maint, "bot2.txt", "chore(deps): bot bump 2");
    git(&maint, &["push", "-q", "-u", "origin", "main"]);
    let checkout = clone_with_identity(&root, &remote, "checkout");

    if kind != RewriteKind::Dropped {
        commit_file_at(&checkout, "mine.txt", "feat: mine");
    }
    let local_tip = head_sha(&checkout);

    if kind == RewriteKind::NormalDivergence {
        commit_file_at(&maint, "up.txt", "feat: upstream");
        git(&maint, &["push", "-q", "origin", "main"]);
    } else {
        git(&maint, &["reset", "-q", "--hard", "HEAD~2"]);
        commit_file_at(&maint, "release.txt", "chore(release): 1.0.0");
        git(&maint, &["push", "-q", "--force", "origin", "main"]);
    }
    let upstream_tip = head_sha(&maint);
    git(&checkout, &["fetch", "-q"]);

    RewriteFixture {
        root,
        checkout,
        local_tip,
        base,
        bot1,
        upstream_tip,
    }
}

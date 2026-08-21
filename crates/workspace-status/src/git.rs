//! Git subprocess helpers. Prefer `/usr/bin/git` so WSL does not pick git.exe.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

static GIT_BINARY: OnceLock<PathBuf> = OnceLock::new();

/// Resolve the git binary: `WORKSPACE_STATUS_GIT`, else `/usr/bin/git` if present, else `git`.
pub fn git_binary() -> &'static Path {
    GIT_BINARY.get_or_init(|| {
        if let Ok(override_bin) = std::env::var("WORKSPACE_STATUS_GIT") {
            if !override_bin.is_empty() {
                return PathBuf::from(override_bin);
            }
        }
        let usr = PathBuf::from("/usr/bin/git");
        if usr.is_file() {
            usr
        } else {
            PathBuf::from("git")
        }
    })
}

fn run(args: &[&str], cwd: &Path) -> std::io::Result<std::process::Output> {
    Command::new(git_binary())
        .args(args)
        .current_dir(cwd)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
}

/// Run git and return trimmed stdout. Empty string on failure.
pub fn exec_git(args: &[&str], cwd: &Path) -> String {
    match run(args, cwd) {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout).trim().to_string(),
        _ => String::new(),
    }
}

/// Run git and return the exit code (`-1` when the process did not start).
pub fn exec_git_status(args: &[&str], cwd: &Path) -> i32 {
    match run(args, cwd) {
        Ok(out) => out.status.code().unwrap_or(-1),
        Err(_) => -1,
    }
}

/// Run git. `Err` when the process exits non-zero or fails to start.
pub fn exec_git_checked(args: &[&str], cwd: &Path) -> Result<(), String> {
    match run(args, cwd) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "git {} exited with code {}",
            args.first().copied().unwrap_or("git"),
            out.status.code().unwrap_or(-1)
        )),
        Err(err) => Err(err.to_string()),
    }
}

/// True when the worktree or index has tracked changes.
pub fn repo_has_local_changes(cwd: &Path) -> bool {
    exec_git_status(&["diff", "--quiet"], cwd) != 0
        || exec_git_status(&["diff", "--cached", "--quiet"], cwd) != 0
}

/// Resolve `ref` to a commit SHA. Missing refs return `None`.
pub fn rev_parse_quiet(git_ref: &str, cwd: &Path) -> Option<String> {
    let sha = exec_git(&["rev-parse", "--verify", "--quiet", git_ref], cwd);
    if sha.is_empty() {
        None
    } else {
        Some(sha)
    }
}

/// Checkout an existing branch, or create it tracking `origin/<branch>`.
pub fn checkout_branch(branch: &str, cwd: &Path) -> bool {
    if exec_git_status(&["checkout", branch, "--quiet"], cwd) == 0 {
        return true;
    }
    let origin = format!("origin/{branch}");
    exec_git_status(&["checkout", "-b", branch, &origin, "--quiet"], cwd) == 0
}

const AUTO_STASH_MESSAGE: &str = "ws-status: auto-stash before pull";

#[derive(Debug, Clone, Copy)]
pub struct PullQuietResult {
    pub ok: bool,
    pub stashed: bool,
    pub stash_pop_failed: bool,
}

/// `git pull --quiet`, stashing tracked local changes first when needed.
pub fn pull_quiet_detailed(cwd: &Path) -> PullQuietResult {
    let dirty = repo_has_local_changes(cwd);
    let mut stashed = false;
    if dirty {
        if exec_git_status(
            &["stash", "push", "-m", AUTO_STASH_MESSAGE, "--quiet"],
            cwd,
        ) != 0
        {
            return PullQuietResult {
                ok: false,
                stashed: false,
                stash_pop_failed: false,
            };
        }
        stashed = true;
    }

    let pull_ok = exec_git_status(&["pull", "--quiet"], cwd) == 0;
    let mut stash_pop_failed = false;
    if stashed && exec_git_status(&["stash", "pop", "--quiet"], cwd) != 0 {
        stash_pop_failed = true;
    }
    PullQuietResult {
        ok: pull_ok && !stash_pop_failed,
        stashed,
        stash_pop_failed,
    }
}

pub fn pull_quiet(cwd: &Path) -> bool {
    pull_quiet_detailed(cwd).ok
}

/// Whether `maybe_ancestor` is an ancestor of `tip`. `None` when git cannot decide.
pub fn is_ancestor(cwd: &Path, maybe_ancestor: &str, tip: &str) -> Option<bool> {
    match exec_git_status(&["merge-base", "--is-ancestor", maybe_ancestor, tip], cwd) {
        0 => Some(true),
        1 => Some(false),
        _ => None,
    }
}

/// First existing tip among `origin/<default>` then `<default>`.
pub fn resolve_default_branch_tip_ref(cwd: &Path, default_branch: &str) -> Option<String> {
    let origin = format!("origin/{default_branch}");
    for git_ref in [origin.as_str(), default_branch] {
        let verify = format!("{git_ref}^{{commit}}");
        if exec_git_status(&["rev-parse", "--verify", "--quiet", &verify], cwd) == 0 {
            return Some(git_ref.to_string());
        }
    }
    None
}

/// Default branch name for merge-into-default classification.
pub fn resolve_default_branch_name(cwd: &Path, override_name: Option<&str>) -> String {
    if let Some(name) = override_name {
        if !name.is_empty() {
            return name.to_string();
        }
    }
    let remote_head = exec_git(
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        cwd,
    );
    if !remote_head.is_empty() {
        if let Some(rest) = remote_head.strip_prefix("origin/") {
            return rest.to_string();
        }
        return remote_head;
    }
    "main".to_string()
}

/// Default branch used by `--default-branch` (origin/HEAD, then develop/main/master).
pub fn get_default_branch(cwd: &Path, override_name: Option<&str>) -> Option<String> {
    if let Some(name) = override_name {
        if !name.is_empty() {
            return Some(name.to_string());
        }
    }
    let remote_head = exec_git(
        &[
            "symbolic-ref",
            "--quiet",
            "--short",
            "refs/remotes/origin/HEAD",
        ],
        cwd,
    );
    if !remote_head.is_empty() {
        if let Some(rest) = remote_head.strip_prefix("origin/") {
            return Some(rest.to_string());
        }
        return Some(remote_head);
    }
    for name in ["develop", "main", "master"] {
        let remote = format!("refs/remotes/origin/{name}");
        if exec_git_status(&["show-ref", "--verify", &remote], cwd) == 0 {
            return Some(name.to_string());
        }
    }
    for name in ["develop", "main", "master"] {
        let local = format!("refs/heads/{name}");
        if exec_git_status(&["show-ref", "--verify", &local], cwd) == 0 {
            return Some(name.to_string());
        }
    }
    None
}

/// `git worktree list --porcelain` stdout (empty on failure).
pub fn list_worktrees_porcelain(cwd: &Path) -> String {
    exec_git(&["worktree", "list", "--porcelain"], cwd)
}

/// Stage one path (`git add --`).
pub fn stage_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["add", "--", file_path], cwd)
}

/// Unstage one path (`git restore --staged --`).
pub fn unstage_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["restore", "--staged", "--", file_path], cwd)
}

/// Discard worktree changes to a tracked path (`git restore --`).
pub fn revert_tracked_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["restore", "--", file_path], cwd)
}

/// Delete an untracked path (`git clean -f --`). Destructive.
pub fn remove_untracked_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["clean", "-f", "--", file_path], cwd)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn git_binary_is_nonempty() {
        assert!(!git_binary().as_os_str().is_empty());
    }

    fn git_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("GIT_AUTHOR_NAME", "workspace-status test"),
            ("GIT_AUTHOR_EMAIL", "workspace-status-test@example.invalid"),
            ("GIT_COMMITTER_NAME", "workspace-status test"),
            ("GIT_COMMITTER_EMAIL", "workspace-status-test@example.invalid"),
        ]
    }

    fn git(cwd: &Path, args: &[&str]) {
        let mut cmd = Command::new(git_binary());
        cmd.args(args).current_dir(cwd);
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        let status = cmd.status().expect("git");
        assert!(status.success(), "git {args:?}");
    }

    fn porcelain(cwd: &Path) -> String {
        exec_git(&["status", "--porcelain=v1"], cwd)
    }

    #[test]
    fn stage_unstage_revert_on_fixture() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-ops-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let init = Command::new(git_binary())
            .args(["init", "-q", "-b", "main"])
            .current_dir(&dir)
            .status();
        if init.map(|s| s.success()).unwrap_or(false) == false {
            git(&dir, &["init", "-q"]);
            git(&dir, &["checkout", "-q", "-b", "main"]);
        }
        fs::write(dir.join("README.md"), "# seed\n").unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "seed"]);
        fs::write(dir.join("README.md"), "# dirty\n").unwrap();
        assert_ne!(exec_git_status(&["diff", "--quiet"], &dir), 0);
        stage_file(&dir, "README.md").unwrap();
        assert_ne!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
        unstage_file(&dir, "README.md").unwrap();
        assert_eq!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
        assert_ne!(exec_git_status(&["diff", "--quiet"], &dir), 0);
        revert_tracked_file(&dir, "README.md").unwrap();
        assert_eq!(exec_git_status(&["diff", "--quiet"], &dir), 0);
        let _ = porcelain(&dir);
        fs::write(dir.join("tmp-untracked.txt"), "x\n").unwrap();
        remove_untracked_file(&dir, "tmp-untracked.txt").unwrap();
        assert!(!dir.join("tmp-untracked.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }
}

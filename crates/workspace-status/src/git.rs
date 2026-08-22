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


/// `git push --quiet`. First publish uses `git push -u <remote> HEAD`.
pub fn push_quiet(cwd: &Path) -> Result<(), String> {
    let branch = exec_git(&["branch", "--show-current"], cwd);
    if branch.is_empty() {
        return Err("detached HEAD cannot push".into());
    }
    if needs_upstream_publish(cwd, &branch) {
        let remote = push_remote_name(cwd, &branch);
        exec_git_checked(&["push", "-u", &remote, "HEAD", "--quiet"], cwd)
    } else {
        exec_git_checked(&["push", "--quiet"], cwd)
    }
}

fn needs_upstream_publish(cwd: &Path, branch: &str) -> bool {
    let upstream = exec_git(&["rev-parse", "--abbrev-ref", "@{upstream}"], cwd);
    if upstream.is_empty() {
        return true;
    }
    let key = format!("branch.{branch}.remote");
    let remote = exec_git(&["config", "--get", &key], cwd);
    let remote = if remote.is_empty() {
        "origin".to_string()
    } else {
        remote
    };
    let prefix = format!("{remote}/");
    if !upstream.starts_with(&prefix) {
        return true;
    }
    &upstream[prefix.len()..] != branch
}

fn push_remote_name(cwd: &Path, branch: &str) -> String {
    let key = format!("branch.{branch}.remote");
    let configured = exec_git(&["config", "--get", &key], cwd);
    if !configured.is_empty() {
        return configured;
    }
    let remotes = exec_git(&["remote"], cwd);
    remotes
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("origin")
        .to_string()
}

/// Stash worktree changes. Includes untracked files (`-u`).
pub fn stash_push(cwd: &Path, paths: &[String]) -> Result<(), String> {
    let before = exec_git(&["stash", "list"], cwd);
    let mut args: Vec<&str> = vec!["stash", "push", "-u"];
    if !paths.is_empty() {
        args.push("--");
        for path in paths {
            args.push(path);
        }
    }
    exec_git_checked(&args, cwd)?;
    let after = exec_git(&["stash", "list"], cwd);
    if after == before {
        return Err("no local changes to save".into());
    }
    Ok(())
}

/// Apply a stash and keep the entry.
pub fn stash_apply(cwd: &Path, stash_ref: &str) -> Result<(), String> {
    exec_git_checked(&["stash", "apply", stash_ref], cwd)
}

/// Pop a stash entry (apply then drop).
pub fn stash_pop(cwd: &Path, stash_ref: &str) -> Result<(), String> {
    exec_git_checked(&["stash", "pop", stash_ref], cwd)
}

/// Drop a stash entry.
pub fn stash_drop(cwd: &Path, stash_ref: &str) -> Result<(), String> {
    exec_git_checked(&["stash", "drop", stash_ref], cwd)
}

/// Stash refs newest first (`stash@{0}`, …).
pub fn list_stash_refs(cwd: &Path) -> Vec<String> {
    exec_git(&["stash", "list", "--format=%gd"], cwd)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect()
}

/// Newest stash ref, if any.
pub fn latest_stash_ref(cwd: &Path) -> Option<String> {
    list_stash_refs(cwd).into_iter().next()
}

/// One local branch for the picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalBranch {
    pub name: String,
    pub current: bool,
    pub authordate: i64,
}

/// Local branches only (no remotes).
pub fn list_local_branches(cwd: &Path) -> Vec<LocalBranch> {
    let raw = exec_git(
        &[
            "for-each-ref",
            "--format=%(refname:short)\t%(authordate:unix)\t%(HEAD)",
            "refs/heads/",
        ],
        cwd,
    );
    raw.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.to_string();
            if name.is_empty() {
                return None;
            }
            let authordate = parts
                .next()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);
            let current = parts.next() == Some("*");
            Some(LocalBranch {
                name,
                current,
                authordate,
            })
        })
        .collect()
}

/// `origin/<branch>` when that ref exists and differs from the local tip.
pub fn origin_out_of_sync(cwd: &Path, branch: &str) -> Option<String> {
    let origin = format!("origin/{branch}");
    let local = rev_parse_quiet(branch, cwd)?;
    let remote = rev_parse_quiet(&origin, cwd)?;
    if local == remote {
        None
    } else {
        Some(origin)
    }
}


/// Drop a linked worktree. Runs `git worktree remove [--force] <path>` from the primary.
pub fn remove_worktree(primary_abs: &Path, worktree_abs: &Path, force: bool) -> Result<(), String> {
    let porcelain = list_worktrees_porcelain(primary_abs);
    let entries = crate::worktrees::parse_worktree_list_porcelain(&porcelain);
    let target = crate::worktrees::resolve_worktree_remove_target(&entries, primary_abs, worktree_abs);
    let path = target.git_path.to_string_lossy().into_owned();
    if force {
        exec_git_checked(&["worktree", "remove", "--force", &path], &target.git_cwd)
    } else {
        exec_git_checked(&["worktree", "remove", &path], &target.git_cwd)
    }
}

/// Create and check out a new branch at HEAD.
pub fn create_branch_checkout(cwd: &Path, name: &str) -> Result<(), String> {
    exec_git_checked(&["checkout", "-b", name, "--quiet"], cwd)
}

/// One path from `git diff --name-status` / `diff-tree` / `stash show`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameStatus {
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
}

/// Parse newline `name-status` (`M\\tpath` or `R100\\told\\tnew`).
pub fn parse_name_status_lines(stdout: &str) -> Vec<NameStatus> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut parts = line.split('\t');
        let status = parts.next().unwrap_or("").to_string();
        if status.is_empty() {
            continue;
        }
        if status.starts_with('R') || status.starts_with('C') {
            let old_path = parts.next().map(str::to_string).filter(|s| !s.is_empty());
            let Some(path) = parts.next().map(str::to_string).filter(|s| !s.is_empty()) else {
                continue;
            };
            out.push(NameStatus {
                status: status.chars().next().unwrap_or('M').to_string(),
                path,
                old_path,
            });
            continue;
        }
        let Some(path) = parts.next().map(str::to_string).filter(|s| !s.is_empty()) else {
            continue;
        };
        out.push(NameStatus {
            status: status.chars().next().unwrap_or('M').to_string(),
            path,
            old_path: None,
        });
    }
    out
}

/// First-parent files in `commit_id`. Root commits fall back to `--root`.
pub fn list_commit_name_status(cwd: &Path, commit_id: &str) -> Vec<NameStatus> {
    let parent = format!("{commit_id}^");
    let out = exec_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-r",
            &parent,
            commit_id,
        ],
        cwd,
    );
    if !out.is_empty() {
        return parse_name_status_lines(&out);
    }
    let root = exec_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-status",
            "-r",
            "--root",
            commit_id,
        ],
        cwd,
    );
    parse_name_status_lines(&root)
}

/// Files recorded in a stash entry.
pub fn list_stash_name_status(cwd: &Path, stash_ref: &str) -> Vec<NameStatus> {
    parse_name_status_lines(&exec_git(
        &["stash", "show", "--name-status", stash_ref],
        cwd,
    ))
}

/// Worktree + index changes versus HEAD, plus untracked files.
pub fn list_worktree_name_status(cwd: &Path) -> Vec<NameStatus> {
    let mut files = parse_name_status_lines(&exec_git(
        &["diff", "HEAD", "--name-status"],
        cwd,
    ));
    let untracked = exec_git(&["ls-files", "--others", "--exclude-standard"], cwd);
    for path in untracked.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if files.iter().any(|f| f.path == path) {
            continue;
        }
        files.push(NameStatus {
            status: "?".into(),
            path: path.to_string(),
            old_path: None,
        });
    }
    files
}

fn lines_or_empty_diff(text: &str) -> Vec<String> {
    if text.is_empty() {
        vec!["(no diff)".into()]
    } else {
        text.lines().map(str::to_string).collect()
    }
}

/// First-parent unified diff for one path in a commit.
pub fn diff_commit_file(cwd: &Path, commit_id: &str, path: &str) -> Vec<String> {
    let parent = format!("{commit_id}^");
    let primary = exec_git(&["diff", &parent, commit_id, "--", path], cwd);
    if !primary.is_empty() {
        return primary.lines().map(str::to_string).collect();
    }
    lines_or_empty_diff(&exec_git(
        &["show", "--first-parent", commit_id, "--", path],
        cwd,
    ))
}

/// First-parent unified diff for one path inside a stash.
pub fn diff_stash_file(cwd: &Path, stash_ref: &str, path: &str) -> Vec<String> {
    let parent = format!("{stash_ref}^1");
    lines_or_empty_diff(&exec_git(&["diff", &parent, stash_ref, "--", path], cwd))
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
        fs::write(dir.join("tmp-untracked.txt"), "x\n").unwrap();
        remove_untracked_file(&dir, "tmp-untracked.txt").unwrap();
        assert!(!dir.join("tmp-untracked.txt").exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stash_and_branch_on_fixture() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-stash-{}",
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
        stash_push(&dir, &[]).unwrap();
        assert_eq!(fs::read_to_string(dir.join("README.md")).unwrap(), "# seed\n");
        let latest = latest_stash_ref(&dir).expect("stash");
        stash_apply(&dir, &latest).unwrap();
        assert_eq!(fs::read_to_string(dir.join("README.md")).unwrap(), "# dirty\n");
        stash_drop(&dir, &latest).unwrap();
        assert!(latest_stash_ref(&dir).is_none());
        fs::write(dir.join("README.md"), "# dirty2\n").unwrap();
        stash_push(&dir, &["README.md".into()]).unwrap();
        let latest = latest_stash_ref(&dir).expect("stash2");
        stash_pop(&dir, &latest).unwrap();
        assert_eq!(fs::read_to_string(dir.join("README.md")).unwrap(), "# dirty2\n");
        assert!(latest_stash_ref(&dir).is_none());

        create_branch_checkout(&dir, "feature/x").unwrap();
        let branches = list_local_branches(&dir);
        assert!(branches.iter().any(|b| b.name == "feature/x" && b.current));
        assert!(checkout_branch("main", &dir));
        assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");

        let remote = dir.join("remote.git");
        Command::new(git_binary())
            .args(["init", "-q", "--bare", remote.to_str().unwrap()])
            .status()
            .unwrap();
        git(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&dir, &["push", "-u", "origin", "main", "--quiet"]);
        git(&dir, &["checkout", "-q", "-b", "feature/behind"]);
        fs::write(dir.join("README.md"), "# behind-local\n").unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "local"]);
        git(&dir, &["push", "-u", "origin", "feature/behind", "--quiet"]);
        // advance origin
        let other = dir.join("other");
        Command::new(git_binary())
            .args(["clone", "-q", remote.to_str().unwrap(), other.to_str().unwrap()])
            .status()
            .unwrap();
        git(&other, &["checkout", "-q", "feature/behind"]);
        fs::write(other.join("README.md"), "# origin-ahead\n").unwrap();
        git(&other, &["add", "README.md"]);
        git(&other, &["commit", "-q", "-m", "remote"]);
        git(&other, &["push", "--quiet"]);
        git(&dir, &["fetch", "--quiet"]);
        assert_eq!(
            origin_out_of_sync(&dir, "feature/behind").as_deref(),
            Some("origin/feature/behind")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remove_worktree_linked_fixture() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-wt-{}",
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
        let wt = dir.join(".worktrees").join("feat");
        fs::create_dir_all(dir.join(".worktrees")).unwrap();
        git(&dir, &["worktree", "add", "-b", "feature/x", wt.to_str().unwrap()]);
        assert!(wt.join(".git").exists() || wt.exists());
        remove_worktree(&dir, &wt, false).unwrap();
        assert!(!wt.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn parse_name_status_and_commit_files_fixture() {
        assert_eq!(
            parse_name_status_lines("M\tsrc/a.rs\nR100\told.rs\tnew.rs\n"),
            vec![
                NameStatus {
                    status: "M".into(),
                    path: "src/a.rs".into(),
                    old_path: None,
                },
                NameStatus {
                    status: "R".into(),
                    path: "new.rs".into(),
                    old_path: Some("old.rs".into()),
                },
            ]
        );
        let dir = std::env::temp_dir().join(format!(
            "ws-git-commit-files-{}",
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
        fs::write(dir.join("one.txt"), "one\n").unwrap();
        git(&dir, &["add", "one.txt"]);
        git(&dir, &["commit", "-q", "-m", "one"]);
        fs::write(dir.join("two.txt"), "two\n").unwrap();
        git(&dir, &["add", "two.txt"]);
        git(&dir, &["commit", "-q", "-m", "two"]);
        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        let files = list_commit_name_status(&dir, &head);
        assert!(files.iter().any(|f| f.path == "two.txt"), "{files:?}");
        let diff = diff_commit_file(&dir, &head, "two.txt");
        assert!(diff.iter().any(|l| l.contains("two")), "{diff:?}");
        fs::write(dir.join("two.txt"), "dirty\n").unwrap();
        stash_push(&dir, &[]).unwrap();
        fs::write(dir.join("two.txt"), "older\n").unwrap();
        stash_push(&dir, &[]).unwrap();
        let refs = list_stash_refs(&dir);
        assert!(refs.len() >= 2, "{refs:?}");
        let older = refs.iter().find(|r| r.ends_with("{1}")).cloned().unwrap_or_else(|| refs[1].clone());
        let stash_files = list_stash_name_status(&dir, &older);
        assert!(stash_files.iter().any(|f| f.path == "two.txt"), "{stash_files:?}");
        let stash_diff = diff_stash_file(&dir, &older, "two.txt");
        assert!(!stash_diff.is_empty(), "{stash_diff:?}");
        fs::write(dir.join("untracked.txt"), "u\n").unwrap();
        let worktree = list_worktree_name_status(&dir);
        assert!(worktree.iter().any(|f| f.path == "untracked.txt"), "{worktree:?}");
        let _ = fs::remove_dir_all(&dir);
    }

}

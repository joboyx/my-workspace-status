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

/// Build a git subprocess that cannot steal the TUI's TTY.
///
/// Stdin is `/dev/null` so a credential prompt cannot deadlock against the
/// event loop. `GIT_TERMINAL_PROMPT=0` fails fast instead of waiting on a
/// hidden prompt.
fn git_command(bin: &Path, args: &[&str], cwd: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0");
    cmd
}

fn run(args: &[&str], cwd: &Path) -> std::io::Result<std::process::Output> {
    git_command(git_binary(), args, cwd).output()
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

/// Fast-forward HEAD to an already-fetched remote-tracking ref (no fetch, no reset).
///
/// Accepts `origin/foo` or `refs/remotes/origin/foo`. Uses `git merge --ff-only`
/// so an ahead or diverged local tip is left unchanged (no merge commit).
///
/// Returns true when HEAD now matches the remote-tracking tip.
pub fn fast_forward_to_remote_ref(remote_ref: &str, cwd: &Path) -> bool {
    let git_ref = if remote_ref.starts_with("refs/") {
        remote_ref.to_string()
    } else {
        format!("refs/remotes/{remote_ref}")
    };
    let Some(target_sha) = rev_parse_quiet(&git_ref, cwd) else {
        return false;
    };
    if exec_git_status(&["merge", "--ff-only", "--quiet", &git_ref], cwd) != 0 {
        return false;
    }
    rev_parse_quiet("HEAD", cwd).as_deref() == Some(target_sha.as_str())
}

/// Outcome of merging a rev into the current HEAD.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeIntoHeadResult {
    /// HEAD already contained `rev`.
    AlreadyUpToDate,
    /// HEAD fast-forwarded to `rev`.
    FastForward,
    /// Created a merge commit (`--no-ff` after a failed fast-forward).
    MergeCommit,
    /// Conflicts left in the worktree. The merge is not aborted or continued.
    Conflict,
    /// Merge did not start or failed without leaving a merge in progress.
    Failed(String),
}

fn run_merge(args: &[&str], cwd: &Path) -> std::io::Result<std::process::Output> {
    let mut cmd = git_command(git_binary(), args, cwd);
    cmd.env("GIT_EDITOR", "true")
        .env("GIT_MERGE_AUTOEDIT", "no");
    cmd.output()
}

fn merge_failed_message(out: &std::process::Output) -> String {
    let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if err.is_empty() {
        format!("git merge exited {}", out.status.code().unwrap_or(-1))
    } else {
        err
    }
}

/// Merge `rev` into HEAD. Fast-forward when that is possible, otherwise a
/// merge commit. Does not rebase, does not abort on conflict, and does not
/// open an editor (`--no-edit`).
///
/// Tries `git merge --ff-only`, then `git merge --no-ff --no-edit`. Conflicts
/// stay uncommitted (`MERGE_HEAD` remains). Callers refuse a dirty worktree
/// before invoking this.
pub fn merge_into_head(rev: &str, cwd: &Path) -> MergeIntoHeadResult {
    let before = rev_parse_quiet("HEAD", cwd);
    match run_merge(&["merge", "--ff-only", "--quiet", "--", rev], cwd) {
        Ok(out) if out.status.success() => {
            let after = rev_parse_quiet("HEAD", cwd);
            if before == after {
                return MergeIntoHeadResult::AlreadyUpToDate;
            }
            return MergeIntoHeadResult::FastForward;
        }
        Ok(_) => {}
        Err(err) => return MergeIntoHeadResult::Failed(err.to_string()),
    }
    if rev_parse_quiet("MERGE_HEAD", cwd).is_some() {
        return MergeIntoHeadResult::Conflict;
    }
    match run_merge(
        &["merge", "--no-ff", "--no-edit", "--quiet", "--", rev],
        cwd,
    ) {
        Ok(out) if out.status.success() => MergeIntoHeadResult::MergeCommit,
        Ok(out) => {
            if rev_parse_quiet("MERGE_HEAD", cwd).is_some() {
                MergeIntoHeadResult::Conflict
            } else {
                MergeIntoHeadResult::Failed(merge_failed_message(&out))
            }
        }
        Err(err) => MergeIntoHeadResult::Failed(err.to_string()),
    }
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
        if exec_git_status(&["stash", "push", "-m", AUTO_STASH_MESSAGE, "--quiet"], cwd) != 0 {
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
            let authordate = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
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
    let target =
        crate::worktrees::resolve_worktree_remove_target(&entries, primary_abs, worktree_abs);
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

/// Argv for `git branch -- <name> <commitId>` (ref only, no checkout).
pub fn create_branch_at_args<'a>(name: &'a str, commit_id: &'a str) -> [&'a str; 4] {
    ["branch", "--", name, commit_id]
}

/// Create a local branch at `commit_id` without checking it out.
pub fn create_branch_at(cwd: &Path, name: &str, commit_id: &str) -> Result<(), String> {
    exec_git_checked(&create_branch_at_args(name, commit_id), cwd)
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
    let mut files = parse_name_status_lines(&exec_git(&["diff", "HEAD", "--name-status"], cwd));
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

/// Context large enough to show a typical source file in one hunk.
pub const FULL_DIFF_CONTEXT_LINES: u32 = 999_999;

/// Build `git diff` argv, inserting `-U{n}` when `context` is set.
pub fn git_diff_args(base: &[&str], path: &str, context: Option<u32>) -> Vec<String> {
    let mut args: Vec<String> = base.iter().map(|s| (*s).to_string()).collect();
    if let Some(n) = context {
        args.push(format!("-U{n}"));
    }
    args.push("--".into());
    args.push(path.into());
    args
}

fn exec_git_owned(args: &[String], cwd: &Path) -> String {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    exec_git(&refs, cwd)
}

/// First-parent unified diff for one path in a commit.
pub fn diff_commit_file(cwd: &Path, commit_id: &str, path: &str) -> Vec<String> {
    diff_commit_file_ctx(cwd, commit_id, path, None)
}

/// First-parent unified diff with optional `-U` context.
pub fn diff_commit_file_ctx(
    cwd: &Path,
    commit_id: &str,
    path: &str,
    context: Option<u32>,
) -> Vec<String> {
    let parent = format!("{commit_id}^");
    let args = git_diff_args(&["diff", &parent, commit_id], path, context);
    let primary = exec_git_owned(&args, cwd);
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
    diff_stash_file_ctx(cwd, stash_ref, path, None)
}

/// Stash-file unified diff with optional `-U` context.
pub fn diff_stash_file_ctx(
    cwd: &Path,
    stash_ref: &str,
    path: &str,
    context: Option<u32>,
) -> Vec<String> {
    let parent = format!("{stash_ref}^1");
    let args = git_diff_args(&["diff", &parent, stash_ref], path, context);
    lines_or_empty_diff(&exec_git_owned(&args, cwd))
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

    #[test]
    fn git_command_nulls_stdin_and_disables_prompts() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-stdin-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let script = dir.join("probe");
        fs::write(
            &script,
            "#!/bin/sh\nif [ -t 0 ]; then echo TTY; exit 7; fi\nprintf 'prompt=%s\\n' \"${GIT_TERMINAL_PROMPT-}\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let out = git_command(&script, &["status"], &dir)
            .output()
            .expect("probe runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            out.status.success(),
            "probe failed: {stdout} {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains("prompt=0"),
            "expected GIT_TERMINAL_PROMPT=0, got {stdout:?}"
        );
        assert!(
            !stdout.contains("TTY"),
            "stdin must not be a TTY: {stdout:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn git_diff_args_inserts_full_context() {
        let normal = git_diff_args(&["diff"], "README.md", None);
        assert_eq!(normal, vec!["diff", "--", "README.md"]);
        let full = git_diff_args(&["diff"], "README.md", Some(FULL_DIFF_CONTEXT_LINES));
        assert_eq!(
            full,
            vec![
                "diff".to_string(),
                format!("-U{FULL_DIFF_CONTEXT_LINES}"),
                "--".into(),
                "README.md".into(),
            ]
        );
    }

    fn git_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("GIT_AUTHOR_NAME", "workspace-status test"),
            ("GIT_AUTHOR_EMAIL", "workspace-status-test@example.invalid"),
            ("GIT_COMMITTER_NAME", "workspace-status test"),
            (
                "GIT_COMMITTER_EMAIL",
                "workspace-status-test@example.invalid",
            ),
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

    fn init_repo(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let init = Command::new(git_binary())
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .status();
        if init.map(|s| s.success()).unwrap_or(false) == false {
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
        fs::write(dir.join("README.md"), "# seed\n").unwrap();
        git(dir, &["add", "README.md"]);
        git(dir, &["commit", "-q", "-m", "seed"]);
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
        assert!(repo_has_local_changes(&dir));
        assert_ne!(exec_git_status(&["diff", "--quiet"], &dir), 0);
        stage_file(&dir, "README.md").unwrap();
        assert_ne!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
        unstage_file(&dir, "README.md").unwrap();
        assert_eq!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
        assert_ne!(exec_git_status(&["diff", "--quiet"], &dir), 0);
        revert_tracked_file(&dir, "README.md").unwrap();
        assert_eq!(exec_git_status(&["diff", "--quiet"], &dir), 0);
        assert!(!repo_has_local_changes(&dir));
        fs::write(dir.join("tmp-untracked.txt"), "x\n").unwrap();
        assert!(!repo_has_local_changes(&dir));
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
        assert_eq!(
            fs::read_to_string(dir.join("README.md")).unwrap(),
            "# seed\n"
        );
        let latest = latest_stash_ref(&dir).expect("stash");
        stash_apply(&dir, &latest).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("README.md")).unwrap(),
            "# dirty\n"
        );
        stash_drop(&dir, &latest).unwrap();
        assert!(latest_stash_ref(&dir).is_none());
        fs::write(dir.join("README.md"), "# dirty2\n").unwrap();
        stash_push(&dir, &["README.md".into()]).unwrap();
        let latest = latest_stash_ref(&dir).expect("stash2");
        stash_pop(&dir, &latest).unwrap();
        assert_eq!(
            fs::read_to_string(dir.join("README.md")).unwrap(),
            "# dirty2\n"
        );
        assert!(latest_stash_ref(&dir).is_none());

        create_branch_checkout(&dir, "feature/x").unwrap();
        let branches = list_local_branches(&dir);
        assert!(branches.iter().any(|b| b.name == "feature/x" && b.current));
        assert!(checkout_branch("main", &dir));
        assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");
        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        assert_eq!(
            create_branch_at_args("feature/at", &head),
            ["branch", "--", "feature/at", head.as_str()]
        );
        create_branch_at(&dir, "feature/at", &head).unwrap();
        assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");
        assert_eq!(exec_git(&["rev-parse", "feature/at"], &dir), head);

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
            .args([
                "clone",
                "-q",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ])
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
        let remote_sha = exec_git(&["rev-parse", "origin/feature/behind"], &dir);
        assert_ne!(exec_git(&["rev-parse", "HEAD"], &dir), remote_sha);
        assert!(fast_forward_to_remote_ref("origin/feature/behind", &dir));
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), remote_sha);
        assert_eq!(
            exec_git(&["branch", "--show-current"], &dir),
            "feature/behind"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn fast_forward_to_remote_ref_ahead_and_missing_leave_head() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-ff-{}",
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
        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        assert!(!fast_forward_to_remote_ref("origin/foo", &dir));
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), head);
        assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");

        let remote = dir.join("remote.git");
        Command::new(git_binary())
            .args(["init", "-q", "--bare", remote.to_str().unwrap()])
            .status()
            .unwrap();
        git(&dir, &["remote", "add", "origin", remote.to_str().unwrap()]);
        git(&dir, &["push", "-u", "origin", "main", "--quiet"]);
        git(&dir, &["checkout", "-q", "-b", "foo"]);
        git(&dir, &["push", "-u", "origin", "foo", "--quiet"]);
        fs::write(dir.join("ahead.txt"), "ahead\n").unwrap();
        git(&dir, &["add", "ahead.txt"]);
        git(&dir, &["commit", "-q", "-m", "ahead"]);
        let ahead = exec_git(&["rev-parse", "HEAD"], &dir);
        assert!(!fast_forward_to_remote_ref("origin/foo", &dir));
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), ahead);
        assert_eq!(exec_git(&["branch", "--show-current"], &dir), "foo");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn merge_into_head_ff_merge_commit_conflict_and_worktree() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-merge-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        init_repo(&dir);
        let seed = exec_git(&["rev-parse", "HEAD"], &dir);
        assert_eq!(
            merge_into_head(&seed, &dir),
            MergeIntoHeadResult::AlreadyUpToDate
        );
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), seed);

        git(&dir, &["checkout", "-q", "-b", "topic"]);
        fs::write(dir.join("topic.txt"), "topic\n").unwrap();
        git(&dir, &["add", "topic.txt"]);
        git(&dir, &["commit", "-q", "-m", "topic"]);
        let topic = exec_git(&["rev-parse", "HEAD"], &dir);
        git(&dir, &["tag", "v1.0"]);
        git(&dir, &["checkout", "-q", "main"]);
        assert_eq!(
            merge_into_head(&topic, &dir),
            MergeIntoHeadResult::FastForward
        );
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), topic);
        assert_eq!(exec_git(&["branch", "--show-current"], &dir), "main");

        git(&dir, &["reset", "--hard", "--quiet", &seed]);
        fs::write(dir.join("main.txt"), "main\n").unwrap();
        git(&dir, &["add", "main.txt"]);
        git(&dir, &["commit", "-q", "-m", "main"]);
        let main_tip = exec_git(&["rev-parse", "HEAD"], &dir);
        assert_eq!(
            merge_into_head(&topic, &dir),
            MergeIntoHeadResult::MergeCommit
        );
        assert_ne!(exec_git(&["rev-parse", "HEAD"], &dir), main_tip);
        assert_eq!(exec_git(&["rev-parse", "HEAD^1"], &dir), main_tip);
        assert_eq!(exec_git(&["rev-parse", "HEAD^2"], &dir), topic);
        assert!(rev_parse_quiet("MERGE_HEAD", &dir).is_none());

        git(&dir, &["reset", "--hard", "--quiet", &seed]);
        fs::write(dir.join("README.md"), "# main-side\n").unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "main-side"]);
        git(&dir, &["checkout", "-q", "-B", "conflict-topic", &seed]);
        fs::write(dir.join("README.md"), "# topic-side\n").unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "topic-side"]);
        let conflict_topic = exec_git(&["rev-parse", "HEAD"], &dir);
        git(&dir, &["checkout", "-q", "main"]);
        let main_before = exec_git(&["rev-parse", "HEAD"], &dir);
        assert_eq!(
            merge_into_head(&conflict_topic, &dir),
            MergeIntoHeadResult::Conflict
        );
        assert!(rev_parse_quiet("MERGE_HEAD", &dir).is_some());
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), main_before);
        git(&dir, &["merge", "--abort"]);

        git(&dir, &["reset", "--hard", "--quiet", &seed]);
        let wt = dir.join(".worktrees").join("feat");
        fs::create_dir_all(dir.join(".worktrees")).unwrap();
        git(
            &dir,
            &[
                "worktree",
                "add",
                "-b",
                "wt-main",
                wt.to_str().unwrap(),
                &seed,
            ],
        );
        let primary_head = exec_git(&["rev-parse", "HEAD"], &dir);
        assert_eq!(
            merge_into_head(&topic, &wt),
            MergeIntoHeadResult::FastForward
        );
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &wt), topic);
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), primary_head);

        match merge_into_head("this-ref-does-not-exist", &dir) {
            MergeIntoHeadResult::Failed(_) => {}
            other => panic!("{other:?}"),
        }
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
        git(
            &dir,
            &["worktree", "add", "-b", "feature/x", wt.to_str().unwrap()],
        );
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
        let older = refs
            .iter()
            .find(|r| r.ends_with("{1}"))
            .cloned()
            .unwrap_or_else(|| refs[1].clone());
        let stash_files = list_stash_name_status(&dir, &older);
        assert!(
            stash_files.iter().any(|f| f.path == "two.txt"),
            "{stash_files:?}"
        );
        let stash_diff = diff_stash_file(&dir, &older, "two.txt");
        assert!(!stash_diff.is_empty(), "{stash_diff:?}");
        fs::write(dir.join("untracked.txt"), "u\n").unwrap();
        let worktree = list_worktree_name_status(&dir);
        assert!(
            worktree.iter().any(|f| f.path == "untracked.txt"),
            "{worktree:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

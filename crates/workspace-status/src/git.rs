//! Git subprocess helpers. Prefer `/usr/bin/git` so WSL does not pick git.exe.

use std::io::Write;
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

/// Build a git subprocess for `bin`, with the env every `ws` git spawn needs.
///
/// `GIT_OPTIONAL_LOCKS=0` stops git from taking `.git/index.lock` for
/// background refreshes (git-status(1), "BACKGROUND REFRESH"). `ws` polls
/// every repo every few seconds; without this, a poll can collide with a
/// concurrent `ws` instance or a manual `git add`/`commit`/`checkout` and
/// fail with `Unable to create '.git/index.lock': File exists`. The flag
/// only skips *optional* locks — write commands still take the locks they
/// need, so it is safe on every spawn, read or write.
pub(crate) fn git_process(bin: &Path) -> Command {
    let mut cmd = Command::new(bin);
    cmd.env("GIT_OPTIONAL_LOCKS", "0");
    cmd
}

/// Build a git subprocess that cannot steal the TUI's TTY.
///
/// Stdin is `/dev/null` so a credential prompt cannot deadlock against the
/// event loop. `GIT_TERMINAL_PROMPT=0` fails fast instead of waiting on a
/// hidden prompt.
fn git_command(bin: &Path, args: &[&str], cwd: &Path) -> Command {
    let mut cmd = git_process(bin);
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

/// Run git with `stdin` piped: the `git apply` wrappers and index blame.
fn run_with_stdin(
    args: &[&str],
    cwd: &Path,
    stdin: &[u8],
) -> std::io::Result<std::process::Output> {
    let mut cmd = git_process(git_binary());
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0");
    let mut child = cmd.spawn()?;
    let write_err = match child.stdin.take() {
        Some(mut pipe) => pipe.write_all(stdin).err(),
        None => None,
    };
    let out = child.wait_with_output()?;
    if let Some(err) = write_err {
        return Err(err);
    }
    Ok(out)
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
///
/// The error is git's own reason line ([`git_reason_line`]), or
/// `git <sub> exited with code N` when git printed none.
pub fn exec_git_checked(args: &[&str], cwd: &Path) -> Result<(), String> {
    match run(args, cwd) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

fn exit_code_message(args: &[&str], out: &std::process::Output) -> String {
    format!(
        "git {} exited with code {}",
        args.first().copied().unwrap_or("git"),
        out.status.code().unwrap_or(-1)
    )
}

/// One line that says why a git run failed, without the `fatal:` / `error:` tag.
///
/// Order: a push `! [rejected]` line, then the first `fatal:` / `error:`
/// line of stderr, then the first other stderr line that is not a `hint:`,
/// then a stdout `CONFLICT` line (`stash apply` prints its conflict there).
/// `None` when git printed nothing useful.
pub(crate) fn git_reason_line(out: &std::process::Output) -> Option<String> {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines = || stderr.lines().map(str::trim).filter(|l| !l.is_empty());
    let tagged = |line: &str| {
        ["fatal:", "error:"]
            .iter()
            .find_map(|tag| line.strip_prefix(tag))
            .map(|rest| rest.trim().to_string())
    };
    lines()
        .find_map(|line| line.strip_prefix("! ").filter(|rest| rest.starts_with('[')))
        .map(|rest| rest.split_whitespace().collect::<Vec<_>>().join(" "))
        .or_else(|| lines().find_map(tagged))
        .or_else(|| {
            lines()
                .find(|line| !line.starts_with("hint:"))
                .map(str::to_string)
        })
        .or_else(|| {
            stdout
                .lines()
                .map(str::trim)
                .find(|line| line.starts_with("CONFLICT"))
                .map(str::to_string)
        })
}

/// Binary-safe blob bytes for `<rev>:<path>` (`git cat-file blob`). Missing path → `None`.
///
/// Unlike [`exec_git`], this does not UTF-8-decode or trim stdout.
/// TTY callers must run this on `spawn_blocking`, not the event loop.
pub fn blob_bytes(cwd: &Path, rev: &str, path: &str) -> Option<Vec<u8>> {
    let spec = format!("{rev}:{path}");
    match run(&["cat-file", "blob", &spec], cwd) {
        Ok(out) if out.status.success() => Some(out.stdout),
        _ => None,
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

/// Commits only on `left` and only on `right` (`rev-list --left-right --count left...right`).
///
/// `None` when either rev does not resolve or git fails.
pub fn ahead_behind(left: &str, right: &str, cwd: &Path) -> Option<(usize, usize)> {
    let range = format!("{left}...{right}");
    let out = exec_git(&["rev-list", "--left-right", "--count", &range, "--"], cwd);
    let mut counts = out.split_whitespace().map(str::parse::<usize>);
    match (counts.next(), counts.next(), counts.next()) {
        (Some(Ok(ahead)), Some(Ok(behind)), None) => Some((ahead, behind)),
        _ => None,
    }
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

/// What [`pull_quiet_detailed`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullQuietResult {
    /// The pull ran and any auto-stash came back cleanly.
    pub ok: bool,
    /// Tracked local changes were stashed before the pull.
    pub stashed: bool,
    /// The pull ran, but popping the auto-stash conflicted. The stash is kept.
    pub stash_pop_failed: bool,
    /// Git's reason line when the auto-stash push or the pull failed.
    pub error: Option<String>,
}

/// `git pull --quiet`, stashing tracked local changes first when needed.
pub fn pull_quiet_detailed(cwd: &Path) -> PullQuietResult {
    let dirty = repo_has_local_changes(cwd);
    let mut stashed = false;
    if dirty {
        let args = ["stash", "push", "-m", AUTO_STASH_MESSAGE, "--quiet"];
        if let Err(err) = exec_git_checked(&args, cwd) {
            return PullQuietResult {
                ok: false,
                stashed: false,
                stash_pop_failed: false,
                error: Some(format!("auto-stash failed: {err}")),
            };
        }
        stashed = true;
    }

    let pulled = exec_git_checked(&["pull", "--quiet"], cwd);
    let mut stash_pop_failed = false;
    if stashed && exec_git_status(&["stash", "pop", "--quiet"], cwd) != 0 {
        stash_pop_failed = true;
    }
    PullQuietResult {
        ok: pulled.is_ok() && !stash_pop_failed,
        stashed,
        stash_pop_failed,
        error: pulled.err(),
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

/// True when `HEAD` and `git_ref` resolve to the same commit SHA.
///
/// A just-created branch at the default tip is this case. Same-commit is
/// not merged work (`classify_merged_into_default`).
pub fn head_equals_ref(cwd: &Path, git_ref: &str) -> bool {
    match (rev_parse_quiet("HEAD", cwd), rev_parse_quiet(git_ref, cwd)) {
        (Some(head), Some(tip)) if !head.is_empty() && !tip.is_empty() => head == tip,
        _ => false,
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

/// Apply a unified patch to the index (`git apply --cached`).
///
/// `reverse` is `git apply --reverse --cached` (unstage selected lines).
/// Stdin carries the patch. Other git wrappers attach stdin to `/dev/null`.
pub fn apply_cached_patch(cwd: &Path, patch: &str, reverse: bool) -> Result<(), String> {
    let mut args: Vec<&str> = vec!["apply", "--cached", "--unidiff-zero", "--whitespace=nowarn"];
    if reverse {
        args.push("--reverse");
    }
    args.push("-");
    apply_patch_stdin(cwd, patch, &args)
}

/// Discard a unified patch from the worktree (`git apply --reverse`).
///
/// No `--cached`: the index stays untouched. Stdin carries the patch.
pub fn apply_worktree_patch_reverse(cwd: &Path, patch: &str) -> Result<(), String> {
    apply_patch_stdin(
        cwd,
        patch,
        &[
            "apply",
            "--reverse",
            "--unidiff-zero",
            "--whitespace=nowarn",
            "-",
        ],
    )
}

fn apply_patch_stdin(cwd: &Path, patch: &str, args: &[&str]) -> Result<(), String> {
    if patch.trim().is_empty() {
        return Err("empty patch".into());
    }
    match run_with_stdin(args, cwd, patch.as_bytes()) {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Discard worktree changes to a tracked path (`git restore --`).
pub fn revert_tracked_file(cwd: &Path, file_path: &str) -> Result<(), String> {
    exec_git_checked(&["restore", "--", file_path], cwd)
}

/// Start of every [`ensure_compare_head_clean`] refusal.
///
/// The TUI shows a refusal as-is (warn), not as a failed git write.
pub const COMPARE_REVERT_ABORTED: &str = "revert aborted: ";

/// Refuse a compare-tab write unless the checkout still is the compare head.
///
/// `Err` when `HEAD` is not `head`, or when `git status` lists any of
/// `paths` (staged, unstaged, untracked, or ignored). A compare tab writes
/// the worktree only while it equals `head` for those paths, so this runs
/// on the write worker right before the write: the TUI snapshot can be a
/// watch tick old.
pub fn ensure_compare_head_clean(cwd: &Path, head: &str, paths: &[&str]) -> Result<(), String> {
    match rev_parse_commit(cwd, "HEAD")? {
        Some(now) if now == head => {}
        _ => return Err(format!("{COMPARE_REVERT_ABORTED}HEAD moved")),
    }
    let mut args = vec![
        "status",
        "--porcelain=v1",
        "--untracked-files=all",
        "--ignored",
        "--",
    ];
    args.extend_from_slice(paths);
    let out = exec_git_stdout(&args, cwd)?;
    if out.trim().is_empty() {
        Ok(())
    } else {
        let path = paths.first().copied().unwrap_or_default();
        Err(format!(
            "{COMPARE_REVERT_ABORTED}{path} has uncommitted changes"
        ))
    }
}

/// Restore `paths` in the worktree from `rev`
/// (`git restore --source=<rev> --worktree --`).
///
/// The index stays untouched. A tracked path that `rev` does not have is
/// removed from the worktree; a path only `rev` has is written.
pub fn restore_worktree_from(cwd: &Path, rev: &str, paths: &[&str]) -> Result<(), String> {
    let source = format!("--source={rev}");
    let mut args = vec!["restore", source.as_str(), "--worktree", "--"];
    args.extend_from_slice(paths);
    exec_git_stdout(&args, cwd).map(|_| ())
}

/// Paths a compare whole-file revert touches: `old_path` (renames) first.
pub fn compare_revert_paths<'a>(path: &'a str, old_path: Option<&'a str>) -> Vec<&'a str> {
    match old_path {
        Some(old) if old != path => vec![old, path],
        _ => vec![path],
    }
}

/// Compare `x` on a file: restore it in the worktree from `merge_base`.
///
/// Checks [`ensure_compare_head_clean`] first. Status `M` restores the
/// content, `A` deletes the file, `D` writes it back, and `R` writes
/// `old_path` back and deletes `path`. The index stays untouched.
pub fn revert_compare_file(
    cwd: &Path,
    merge_base: &str,
    head: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<(), String> {
    let paths = compare_revert_paths(path, old_path);
    ensure_compare_head_clean(cwd, head, &paths)?;
    restore_worktree_from(cwd, merge_base, &paths)
}

/// Compare `x` on highlighted lines: `git apply --reverse` of `patch`.
///
/// Checks [`ensure_compare_head_clean`] for `path` first. The patch is a
/// slice of the committed `merge_base...head` diff, so its post-image is
/// the head blob, which the check proves is the worktree file.
pub fn revert_compare_patch(cwd: &Path, head: &str, path: &str, patch: &str) -> Result<(), String> {
    ensure_compare_head_clean(cwd, head, &[path])?;
    apply_worktree_patch_reverse(cwd, patch)
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

/// Added and deleted line counts for one file, from `git --numstat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineStat {
    pub added: u32,
    pub deleted: u32,
}

/// One path from `git diff --name-status` / `diff-tree` / `stash show`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NameStatus {
    pub status: String,
    pub path: String,
    pub old_path: Option<String>,
    /// Line counts. `None` for binary and untracked files, and when numstat failed.
    pub stat: Option<LineStat>,
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
                stat: None,
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
            stat: None,
        });
    }
    out
}

/// Parse `git --numstat -z` output into `(path, counts)` rows.
///
/// A plain row is `added\tdeleted\tpath\0`. A rename or copy row is
/// `added\tdeleted\t\0old\0new\0`; the path is the new one. Binary files
/// print `-\t-` and give `None`.
pub fn parse_numstat_z(stdout: &str) -> Vec<(String, Option<LineStat>)> {
    let mut out = Vec::new();
    let mut tokens = stdout.split('\0').filter(|t| !t.is_empty());
    while let Some(token) = tokens.next() {
        let mut parts = token.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        let stat = match (added.parse::<u32>(), deleted.parse::<u32>()) {
            (Ok(added), Ok(deleted)) => Some(LineStat { added, deleted }),
            _ => None,
        };
        let path = if path.is_empty() {
            let _old = tokens.next();
            match tokens.next() {
                Some(new) => new,
                None => continue,
            }
        } else {
            path
        };
        out.push((path.to_string(), stat));
    }
    out
}

/// Fill `stat` on each file from `--numstat -z` output, matched by new path.
fn attach_numstat(files: &mut [NameStatus], numstat_z: &str) {
    for (path, stat) in parse_numstat_z(numstat_z) {
        if let Some(file) = files.iter_mut().find(|f| f.path == path) {
            file.stat = stat;
        }
    }
}

/// First-parent files in `commit_id`. Root commits fall back to `--root`.
///
/// Line counts come from a second `--numstat -z` call with the same revisions.
/// A numstat failure leaves `stat` as `None`; the list still loads.
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
        let mut files = parse_name_status_lines(&out);
        let numstat = exec_git(
            &[
                "diff-tree",
                "--no-commit-id",
                "--numstat",
                "-z",
                "-r",
                &parent,
                commit_id,
            ],
            cwd,
        );
        attach_numstat(&mut files, &numstat);
        return files;
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
    let mut files = parse_name_status_lines(&root);
    let numstat = exec_git(
        &[
            "diff-tree",
            "--no-commit-id",
            "--numstat",
            "-z",
            "-r",
            "--root",
            commit_id,
        ],
        cwd,
    );
    attach_numstat(&mut files, &numstat);
    files
}

/// Files recorded in a stash entry, with line counts from `--numstat -z`.
pub fn list_stash_name_status(cwd: &Path, stash_ref: &str) -> Vec<NameStatus> {
    let mut files = parse_name_status_lines(&exec_git(
        &["stash", "show", "--name-status", stash_ref],
        cwd,
    ));
    let numstat = exec_git(&["stash", "show", "--numstat", "-z", stash_ref], cwd);
    attach_numstat(&mut files, &numstat);
    files
}

/// Worktree + index changes versus HEAD, plus untracked files.
///
/// Tracked files carry line counts from `diff HEAD --numstat -z`. Untracked
/// files have `stat: None`.
pub fn list_worktree_name_status(cwd: &Path) -> Vec<NameStatus> {
    let mut files = parse_name_status_lines(&exec_git(&["diff", "HEAD", "--name-status"], cwd));
    let numstat = exec_git(&["diff", "HEAD", "--numstat", "-z"], cwd);
    attach_numstat(&mut files, &numstat);
    let untracked = exec_git(&["ls-files", "--others", "--exclude-standard"], cwd);
    for path in untracked.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if files.iter().any(|f| f.path == path) {
            continue;
        }
        files.push(NameStatus {
            status: "?".into(),
            path: path.to_string(),
            old_path: None,
            stat: None,
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

/// Git's reason line ([`git_reason_line`]), or `git <sub> exited with code N`.
fn git_failure_message(args: &[&str], out: &std::process::Output) -> String {
    git_reason_line(out).unwrap_or_else(|| exit_code_message(args, out))
}

/// Run git and return stdout. Failure is `Err`, never an empty success.
pub fn exec_git_stdout(args: &[&str], cwd: &Path) -> Result<String, String> {
    match run(args, cwd) {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string())
        }
        Ok(out) => Err(git_failure_message(args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Files git would show in a checkout: tracked plus untracked, minus ignored.
///
/// Runs `ls-files -z --cached --others --exclude-standard`, so `.gitignore`,
/// `.git/info/exclude`, and `core.excludesFile` apply. Exclude pathspecs keep
/// `target/` and `node_modules/` trees (at any depth) out of the listing
/// even when they are tracked or not ignored. Paths are relative to `cwd`
/// and decoded lossily. Failure is git's reason line, or
/// `git ls-files exited with code N`.
pub fn list_checkout_files(cwd: &Path) -> Result<Vec<String>, String> {
    let args = [
        "ls-files",
        "-z",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
        ".",
        ":(exclude,glob)**/target/**",
        ":(exclude,glob)**/node_modules/**",
    ];
    match run(&args, cwd) {
        Ok(out) if out.status.success() => Ok(out
            .stdout
            .split(|b| *b == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect()),
        Ok(out) => Err(git_failure_message(&args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Resolve `<ref>^{commit}`. Missing ref is `Ok(None)`. Other failures are `Err`.
pub fn rev_parse_commit(cwd: &Path, git_ref: &str) -> Result<Option<String>, String> {
    let verify = format!("{git_ref}^{{commit}}");
    let args = ["rev-parse", "--verify", "--quiet", verify.as_str()];
    match run(&args, cwd) {
        Ok(out) if out.status.success() => {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok((!sha.is_empty()).then_some(sha))
        }
        Ok(out) if out.status.code() == Some(1) => Ok(None),
        Ok(out) => Err(git_failure_message(&args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Three-dot merge base. Unrelated histories are `Ok(None)`.
pub fn merge_base(cwd: &Path, a: &str, b: &str) -> Result<Option<String>, String> {
    let args = ["merge-base", a, b];
    match run(&args, cwd) {
        Ok(out) if out.status.success() => {
            let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
            Ok((!sha.is_empty()).then_some(sha))
        }
        Ok(out) if out.status.code() == Some(1) => Ok(None),
        Ok(out) => Err(git_failure_message(&args, &out)),
        Err(err) => Err(err.to_string()),
    }
}

/// Committed `base...HEAD` paths. Empty stdout is an empty list, not a failure.
///
/// Line counts come from a second `--numstat -z` call. Its failure leaves
/// `stat` as `None`; it does not fail the list.
pub fn list_compare_name_status(
    cwd: &Path,
    base_sha: &str,
    head_sha: &str,
) -> Result<Vec<NameStatus>, String> {
    let range = format!("{base_sha}...{head_sha}");
    let stdout = exec_git_stdout(
        &["diff", "--name-status", "--find-renames", &range, "--"],
        cwd,
    )?;
    let mut files = parse_name_status_lines(&stdout);
    let numstat = exec_git_stdout(
        &["diff", "--numstat", "-z", "--find-renames", &range, "--"],
        cwd,
    )
    .unwrap_or_default();
    attach_numstat(&mut files, &numstat);
    Ok(files)
}

/// `git diff` argv for one compare path, with optional rename old path.
pub fn git_compare_diff_args(
    base_sha: &str,
    head_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Vec<String> {
    let range = format!("{base_sha}...{head_sha}");
    let mut args = vec!["diff".to_string()];
    if let Some(n) = context {
        args.push(format!("-U{n}"));
    }
    args.push(range);
    args.push("--".into());
    if let Some(old) = old_path {
        if old != path {
            args.push(old.to_string());
        }
    }
    args.push(path.into());
    args
}

/// Unified compare diff for one path. Success with empty stdout is `(no diff)`.
pub fn diff_compare_file_ctx(
    cwd: &Path,
    base_sha: &str,
    head_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Result<Vec<String>, String> {
    let args = git_compare_diff_args(base_sha, head_sha, path, old_path, context);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    let stdout = exec_git_stdout(&refs, cwd)?;
    if stdout.trim().is_empty() {
        Ok(vec!["(no diff)".into()])
    } else {
        Ok(stdout.lines().map(str::to_string).collect())
    }
}

/// `git diff` argv for one path, commit `base_sha` against the working tree.
///
/// `diff [-U<n>] -M <base> -- [<old>] <path>`: git's own commit-vs-worktree
/// form, so no revision stands for the working tree. `old_path` joins the
/// pathspec when it differs from `path`, so `-M` can pair a rename.
pub fn git_worktree_compare_diff_args(
    base_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Vec<String> {
    let mut args = vec!["diff".to_string()];
    if let Some(n) = context {
        args.push(format!("-U{n}"));
    }
    args.push("-M".into());
    args.push(base_sha.into());
    args.push("--".into());
    args.extend(
        compare_revert_paths(path, old_path)
            .into_iter()
            .map(String::from),
    );
    args
}

/// The one path of a commit-vs-working-tree compare, as a name-status row.
///
/// Runs `diff --name-status -M <base> -- [<old>] <path>`, then the same with
/// `--numstat -z` for line counts (its failure leaves `stat` as `None`).
/// Empty stdout (the worktree file equals the commit) is an empty list.
pub fn list_worktree_vs_commit_name_status(
    cwd: &Path,
    base_sha: &str,
    path: &str,
    old_path: Option<&str>,
) -> Result<Vec<NameStatus>, String> {
    let paths = compare_revert_paths(path, old_path);
    let mut args = vec!["diff", "--name-status", "-M", base_sha, "--"];
    args.extend_from_slice(&paths);
    let mut files = parse_name_status_lines(&exec_git_stdout(&args, cwd)?);
    let mut numstat_args = vec!["diff", "--numstat", "-z", "-M", base_sha, "--"];
    numstat_args.extend_from_slice(&paths);
    let numstat = exec_git_stdout(&numstat_args, cwd).unwrap_or_default();
    attach_numstat(&mut files, &numstat);
    Ok(files)
}

/// Unified diff of one path, commit `base_sha` against the working tree.
///
/// Argv from [`git_worktree_compare_diff_args`]. Success with empty stdout
/// is `(no diff)`.
pub fn diff_worktree_vs_commit_file_ctx(
    cwd: &Path,
    base_sha: &str,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Result<Vec<String>, String> {
    let args = git_worktree_compare_diff_args(base_sha, path, old_path, context);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    Ok(lines_or_empty_diff(&exec_git_stdout(&refs, cwd)?))
}

/// Local branches plus `origin/*`, excluding `origin/HEAD` and the current local.
pub fn list_compare_picker_branches(cwd: &Path) -> Result<Vec<LocalBranch>, String> {
    let raw = exec_git_stdout(
        &[
            "for-each-ref",
            "--format=%(refname:short)\t%(authordate:unix)\t%(HEAD)",
            "refs/heads/",
            "refs/remotes/origin/",
        ],
        cwd,
    )?;
    Ok(parse_compare_picker_branches(&raw))
}

/// Pure filter for [`list_compare_picker_branches`].
pub fn parse_compare_picker_branches(raw: &str) -> Vec<LocalBranch> {
    raw.lines()
        .filter(|line| !line.is_empty())
        .filter_map(|line| {
            let mut parts = line.split('\t');
            let name = parts.next()?.to_string();
            if name.is_empty() || name == "origin/HEAD" {
                return None;
            }
            let authordate = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
            let current = parts.next() == Some("*");
            if current {
                return None;
            }
            Some(LocalBranch {
                name,
                current: false,
                authordate,
            })
        })
        .collect()
}

/// Newest HEAD ancestors the compare commit picker lists (HEAD itself is
/// not counted). Older history is cut off so a huge repo still opens the
/// picker quickly; type a sha or subject to find a listed commit.
pub const COMPARE_PICKER_COMMIT_LIMIT: usize = 10_000;

/// One ancestor row in the compare commit picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AncestorCommit {
    /// Full object id.
    pub id: String,
    /// First line of the message.
    pub subject: String,
}

/// HEAD's ancestors, newest first, without HEAD: at most
/// [`COMPARE_PICKER_COMMIT_LIMIT`] rows. A root HEAD lists none.
///
/// Runs `git log HEAD` with `--max-count=<limit + 1>` and drops the first
/// row (the walk starts at HEAD), which leaves at most `limit` rows.
/// `HEAD^@` is not used: on a root commit it expands to nothing and
/// `git log` would fall back to HEAD.
pub fn list_compare_picker_commits(cwd: &Path) -> Result<Vec<AncestorCommit>, String> {
    list_compare_picker_commits_capped(cwd, COMPARE_PICKER_COMMIT_LIMIT)
}

fn list_compare_picker_commits_capped(
    cwd: &Path,
    limit: usize,
) -> Result<Vec<AncestorCommit>, String> {
    let max_count = format!("--max-count={}", limit.saturating_add(1));
    let raw = exec_git_stdout(&["log", "--format=%H%x09%s", &max_count, "HEAD", "--"], cwd)?;
    Ok(raw
        .lines()
        .skip(1)
        .filter_map(|line| {
            let (id, subject) = line.split_once('\t').unwrap_or((line, ""));
            (!id.is_empty()).then(|| AncestorCommit {
                id: id.to_string(),
                subject: subject.to_string(),
            })
        })
        .collect())
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

/// Which version of a file [`blame_line`] reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum BlameRev {
    /// The working-tree file (no revision).
    Worktree,
    /// The index blob `:<path>`, fed through `--contents -`.
    Index,
    /// A revision git can resolve (`HEAD`, a full sha, `<sha>^`, a stash ref).
    Commit(String),
}

/// Who last changed one line, from `git blame --porcelain`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineBlame {
    /// Full commit id. All zeros when the line is not committed.
    pub sha: String,
    /// `author` header (`Not Committed Yet` for an uncommitted line).
    pub author: String,
    /// `author-time`, unix seconds.
    pub author_time: i64,
    /// `summary` header: the commit subject.
    pub summary: String,
    /// 1-based line number in that commit's version of the file.
    pub orig_line: u32,
    /// Path of the file in that commit (differs after a rename).
    pub filename: String,
    /// `previous <sha> <path>`: the commit blame looked at before `sha`
    /// and the file's path there. `None` when the file was added in `sha`.
    pub previous: Option<(String, String)>,
    /// `boundary`: git did not look past `sha` (a root commit here).
    pub boundary: bool,
    /// The line is not committed yet (all-zero sha).
    pub uncommitted: bool,
}

/// Parse `git blame --porcelain` output for one line. `None` when malformed.
///
/// Reads the first entry only: `<sha> <orig> <final> [<count>]`, then
/// headers up to the tab-prefixed content line.
pub fn parse_blame_porcelain(stdout: &str) -> Option<LineBlame> {
    let mut lines = stdout.lines();
    let mut head = lines.next()?.split_whitespace();
    let sha = head.next()?.to_string();
    if sha.len() < 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let orig_line = head.next()?.parse().ok()?;
    let mut blame = LineBlame {
        uncommitted: sha.bytes().all(|b| b == b'0'),
        sha,
        author: String::new(),
        author_time: 0,
        summary: String::new(),
        orig_line,
        filename: String::new(),
        previous: None,
        boundary: false,
    };
    for line in lines {
        if line.starts_with('\t') {
            break;
        }
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        match key {
            "author" => blame.author = value.to_string(),
            "author-time" => blame.author_time = value.parse().unwrap_or(0),
            "summary" => blame.summary = value.to_string(),
            "filename" => blame.filename = value.to_string(),
            "boundary" => blame.boundary = true,
            "previous" => {
                blame.previous = value
                    .split_once(' ')
                    .map(|(sha, path)| (sha.to_string(), path.to_string()));
            }
            _ => {}
        }
    }
    (!blame.filename.is_empty()).then_some(blame)
}

/// Blame line `line` (1-based) of `path` in `rev`.
///
/// Runs `git -c blame.showRoot=false blame --porcelain -L n,n [<rev>] -- <path>`.
/// [`BlameRev::Index`] feeds the index blob through `--contents -`, so lines
/// that differ from HEAD come back uncommitted. `Ok(None)` when git exits
/// non-zero (untracked or missing path, unborn HEAD, line out of range) or
/// the path has no index entry. `Err` only when git cannot start.
/// TTY callers must run this on `spawn_blocking`, not the event loop.
pub fn blame_line(
    cwd: &Path,
    rev: &BlameRev,
    path: &str,
    line: u32,
) -> Result<Option<LineBlame>, String> {
    if line == 0 {
        return Ok(None);
    }
    let range = format!("{line},{line}");
    let mut args = vec![
        "-c",
        "blame.showRoot=false",
        "blame",
        "--porcelain",
        "-L",
        &range,
    ];
    match rev {
        BlameRev::Worktree => {}
        BlameRev::Index => args.extend_from_slice(&["--contents", "-"]),
        BlameRev::Commit(rev) => args.push(rev),
    }
    args.extend_from_slice(&["--", path]);
    let out = match rev {
        BlameRev::Index => {
            let Some(blob) = blob_bytes(cwd, "", path) else {
                return Ok(None);
            };
            run_with_stdin(&args, cwd, &blob)
        }
        _ => run(&args, cwd),
    }
    .map_err(|err| err.to_string())?;
    if !out.status.success() {
        return Ok(None);
    }
    Ok(parse_blame_porcelain(&String::from_utf8_lossy(&out.stdout)))
}

/// Where the change before a blamed line's commit came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviousLineChange {
    /// Blame of the matching line in the earlier version. A `boundary`
    /// result is a root commit: it has no parent of its own.
    Found(LineBlame),
    /// The blamed commit added the line (or the whole file).
    AddedIn,
}

/// The change to `blamed`'s line before `blamed.sha`.
///
/// Uses porcelain `previous <prev-sha> <prev-path>` (no `previous` means
/// the file was added). Then `diff -U0 -M <prev-sha> <sha> -- <prev-path>
/// <path>` maps the line into the earlier version with
/// [`map_line_to_parent`], and [`blame_line`] blames it there. For a
/// non-merge commit `<prev-sha>` is `<sha>^`. `Err` for an uncommitted
/// line or a failed git call.
pub fn previous_line_change(cwd: &Path, blamed: &LineBlame) -> Result<PreviousLineChange, String> {
    if blamed.uncommitted {
        return Err("line is not committed yet".into());
    }
    let Some((prev_sha, prev_path)) = &blamed.previous else {
        return Ok(PreviousLineChange::AddedIn);
    };
    let paths = compare_revert_paths(&blamed.filename, Some(prev_path.as_str()));
    let mut args = vec!["diff", "-U0", "-M", prev_sha, &blamed.sha, "--"];
    args.extend_from_slice(&paths);
    let diff = exec_git_stdout(&args, cwd)?;
    let Some(mapped) = map_line_to_parent(&diff, blamed.orig_line) else {
        return Ok(PreviousLineChange::AddedIn);
    };
    match blame_line(cwd, &BlameRev::Commit(prev_sha.clone()), prev_path, mapped)? {
        Some(earlier) => Ok(PreviousLineChange::Found(earlier)),
        None => Err(format!("no blame for {prev_path}:{mapped}")),
    }
}

/// Map new-side line `line` of a `diff -U0` onto the old side.
///
/// A line outside every hunk shifts by the line-count change of the
/// hunks above it. A line inside a hunk maps to the old line at the same
/// offset when the hunk's old side is that long. `None` when the line has
/// no old counterpart (pure insertion, or past the end of a grown hunk).
pub fn map_line_to_parent(diff_u0: &str, line: u32) -> Option<u32> {
    let mut delta: i64 = 0;
    for header in diff_u0.lines().filter(|l| l.starts_with("@@")) {
        let mut parts = header.split_whitespace().skip(1);
        let (old_start, old_count) = hunk_range(parts.next()?.strip_prefix('-')?)?;
        let (new_start, new_count) = hunk_range(parts.next()?.strip_prefix('+')?)?;
        // A pure deletion (`+n,0`) sits after new line `n`.
        let below = if new_count == 0 {
            line > new_start
        } else {
            line >= new_start + new_count
        };
        if below {
            delta += i64::from(old_count) - i64::from(new_count);
            continue;
        }
        if line < new_start || new_count == 0 {
            break;
        }
        let offset = line - new_start;
        return (offset < old_count).then_some(old_start + offset);
    }
    u32::try_from(i64::from(line) + delta).ok()
}

/// `n` or `n,count` from a hunk header side. A bare `n` counts one line.
fn hunk_range(token: &str) -> Option<(u32, u32)> {
    match token.split_once(',') {
        Some((start, count)) => Some((start.parse().ok()?, count.parse().ok()?)),
        None => Some((token.parse().ok()?, 1)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{git, init_repo, init_repo_empty, unique_dir};
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
        // Write + chmod + exec can return ETXTBSY on a busy runner.
        let out = {
            let mut last_err = None;
            let mut result = None;
            for _ in 0..8 {
                match git_command(&script, &["status"], &dir).output() {
                    Ok(out) => {
                        result = Some(out);
                        break;
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        last_err = Some(err);
                        std::thread::sleep(std::time::Duration::from_millis(25));
                    }
                    Err(err) => panic!("probe runs: {err}"),
                }
            }
            result.unwrap_or_else(|| panic!("probe runs: {last_err:?}"))
        };
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
    fn git_process_disables_optional_locks() {
        let cmd = git_process(git_binary());
        assert_eq!(
            cmd.get_envs()
                .find(|(k, _)| *k == "GIT_OPTIONAL_LOCKS")
                .and_then(|(_, v)| v),
            Some(std::ffi::OsStr::new("0")),
            "every ws git spawn must disable optional locks so a background poll cannot \
             collide with a concurrent write on .git/index.lock"
        );
    }

    #[test]
    fn git_command_and_run_with_stdin_disable_optional_locks() {
        let dir = std::env::temp_dir();
        let has_no_locks_env = |cmd: &Command| {
            cmd.get_envs()
                .find(|(k, _)| *k == "GIT_OPTIONAL_LOCKS")
                .and_then(|(_, v)| v)
                == Some(std::ffi::OsStr::new("0"))
        };
        assert!(has_no_locks_env(&git_command(
            git_binary(),
            &["status"],
            &dir
        )));

        let out = run_with_stdin(&["hash-object", "--stdin"], &dir, b"probe\n")
            .expect("run_with_stdin spawns");
        assert!(out.status.success(), "hash-object failed: {out:?}");
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

    #[test]
    fn stage_unstage_revert_on_fixture() {
        let dir = unique_dir("ws-git-ops");
        init_repo(&dir);
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
    fn apply_cached_patch_stages_and_unstages_one_hunk() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-apply-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        init_repo(&dir);
        let body = "\
keep-a
keep-b
keep-c
ALPHA-OLD
keep-d
keep-e
keep-f
pad-1
pad-2
pad-3
pad-4
pad-5
pad-6
OMEGA-OLD
keep-x
keep-y
keep-z
";
        fs::write(dir.join("regions.txt"), body).unwrap();
        git(&dir, &["add", "regions.txt"]);
        git(&dir, &["commit", "-q", "-m", "regions"]);
        fs::write(
            dir.join("regions.txt"),
            body.replace("ALPHA-OLD", "ALPHA-NEW")
                .replace("OMEGA-OLD", "OMEGA-NEW"),
        )
        .unwrap();
        let diff = exec_git(&["diff", "--", "regions.txt"], &dir);
        assert!(
            diff.contains("ALPHA-NEW") && diff.contains("OMEGA-NEW"),
            "{diff}"
        );
        let first_hunk = first_unified_hunk(&diff);
        apply_cached_patch(&dir, &first_hunk, false).unwrap();
        let cached = exec_git(&["diff", "--cached", "--", "regions.txt"], &dir);
        let unstaged = exec_git(&["diff", "--", "regions.txt"], &dir);
        assert!(
            cached.contains("ALPHA-NEW") && !cached.contains("OMEGA-NEW"),
            "{cached}"
        );
        assert!(
            unstaged.contains("OMEGA-NEW") && !unstaged.contains("ALPHA-NEW"),
            "{unstaged}"
        );
        apply_cached_patch(&dir, &first_hunk, true).unwrap();
        assert_eq!(exec_git_status(&["diff", "--cached", "--quiet"], &dir), 0);
        let unstaged = exec_git(&["diff", "--", "regions.txt"], &dir);
        assert!(
            unstaged.contains("ALPHA-NEW") && unstaged.contains("OMEGA-NEW"),
            "{unstaged}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn first_unified_hunk(diff: &str) -> String {
        let mut header = String::new();
        let mut hunk = String::new();
        let mut in_hunk = false;
        for line in diff.lines() {
            if line.starts_with("@@") {
                if in_hunk {
                    break;
                }
                in_hunk = true;
                hunk.push_str(line);
                hunk.push('\n');
                continue;
            }
            if in_hunk {
                hunk.push_str(line);
                hunk.push('\n');
            } else {
                header.push_str(line);
                header.push('\n');
            }
        }
        format!("{header}{hunk}")
    }

    #[test]
    fn stash_and_branch_on_fixture() {
        let dir = unique_dir("ws-git-stash");
        init_repo(&dir);
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
        let dir = unique_dir("ws-git-ff");
        init_repo(&dir);
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
        assert_eq!(ahead_behind("foo", "origin/foo", &dir), Some((1, 0)));
        assert_eq!(ahead_behind("foo", "origin/missing", &dir), None);
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
        let dir = unique_dir("ws-git-wt");
        init_repo(&dir);
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
                    stat: None,
                },
                NameStatus {
                    status: "R".into(),
                    path: "new.rs".into(),
                    old_path: Some("old.rs".into()),
                    stat: None,
                },
            ]
        );
        let dir = unique_dir("ws-git-commit-files");
        init_repo_empty(&dir);
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

    #[test]
    fn parse_numstat_z_plain_rename_binary_and_empty() {
        assert!(parse_numstat_z("").is_empty());
        assert_eq!(
            parse_numstat_z(concat!(
                "3\t1\tsrc/a.rs\0",
                "-\t-\timg.png\0",
                "0\t0\tdir/with\ttab.txt\0"
            )),
            vec![
                (
                    "src/a.rs".to_string(),
                    Some(LineStat {
                        added: 3,
                        deleted: 1
                    })
                ),
                ("img.png".to_string(), None),
                (
                    "dir/with\ttab.txt".to_string(),
                    Some(LineStat {
                        added: 0,
                        deleted: 0
                    })
                ),
            ]
        );
        assert_eq!(
            parse_numstat_z(concat!("2\t5\t\0old.rs\0new.rs\0", "1\t1\tz.rs\0")),
            vec![
                (
                    "new.rs".to_string(),
                    Some(LineStat {
                        added: 2,
                        deleted: 5
                    })
                ),
                (
                    "z.rs".to_string(),
                    Some(LineStat {
                        added: 1,
                        deleted: 1
                    })
                ),
            ]
        );
        // A rename row cut short drops the row instead of panicking.
        assert!(parse_numstat_z("2\t5\t\0old.rs\0").is_empty());
    }

    fn stat_of(files: &[NameStatus], path: &str) -> Option<LineStat> {
        files
            .iter()
            .find(|f| f.path == path)
            .unwrap_or_else(|| panic!("{path} not listed in {files:?}"))
            .stat
    }

    fn counts(added: u32, deleted: u32) -> Option<LineStat> {
        Some(LineStat { added, deleted })
    }

    #[test]
    fn list_commit_name_status_carries_line_counts() {
        let dir = unique_dir("ws-git-numstat-commit");
        init_repo_empty(&dir);
        // Root commit: exercises the `--root` fallback.
        fs::write(dir.join("a.txt"), "1\n2\n3\n").unwrap();
        git(&dir, &["add", "a.txt"]);
        git(&dir, &["commit", "-q", "-m", "root"]);
        let root = exec_git(&["rev-parse", "HEAD"], &dir);
        assert_eq!(
            stat_of(&list_commit_name_status(&dir, &root), "a.txt"),
            counts(3, 0)
        );

        fs::write(dir.join("a.txt"), "1\nX\n3\n4\n").unwrap();
        fs::write(dir.join("bin.dat"), b"\x00\x01\x02").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "second"]);
        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        let files = list_commit_name_status(&dir, &head);
        assert_eq!(stat_of(&files, "a.txt"), counts(2, 1));
        assert_eq!(stat_of(&files, "bin.dat"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_stash_name_status_carries_line_counts() {
        let dir = unique_dir("ws-git-numstat-stash");
        init_repo(&dir);
        fs::write(dir.join("README.md"), "# seed\nmore\nlines\n").unwrap();
        stash_push(&dir, &[]).unwrap();
        let refs = list_stash_refs(&dir);
        let files = list_stash_name_status(&dir, &refs[0]);
        assert_eq!(stat_of(&files, "README.md"), counts(2, 0));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_worktree_name_status_carries_line_counts_but_not_for_untracked() {
        let dir = unique_dir("ws-git-numstat-worktree");
        init_repo(&dir);
        fs::write(dir.join("README.md"), "changed\n").unwrap();
        fs::write(dir.join("untracked.txt"), "u\nv\n").unwrap();
        let files = list_worktree_name_status(&dir);
        assert_eq!(stat_of(&files, "README.md"), counts(1, 1));
        assert_eq!(stat_of(&files, "untracked.txt"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn list_compare_name_status_carries_line_counts_incl_rename() {
        let dir = unique_dir("ws-git-numstat-compare");
        init_repo(&dir);
        let base = exec_git(&["rev-parse", "HEAD"], &dir);
        fs::write(dir.join("new.txt"), "a\nb\n").unwrap();
        git(&dir, &["mv", "README.md", "RENAMED.md"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "change"]);
        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        let files = list_compare_name_status(&dir, &base, &head).unwrap();
        assert_eq!(stat_of(&files, "new.txt"), counts(2, 0));
        assert_eq!(stat_of(&files, "RENAMED.md"), counts(0, 0));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blob_bytes_returns_head_contents_without_utf8_trim() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-blob-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        init_repo(&dir);
        let payload = b"hello \n\x00\xff";
        fs::write(dir.join("bin.dat"), payload).unwrap();
        git(&dir, &["add", "bin.dat"]);
        git(&dir, &["commit", "-q", "-m", "bin"]);
        assert_eq!(
            blob_bytes(&dir, "HEAD", "bin.dat").as_deref(),
            Some(payload.as_slice())
        );
        assert_eq!(
            blob_bytes(&dir, "HEAD", "README.md").as_deref(),
            Some(b"# seed\n".as_slice())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blob_bytes_missing_path_is_none() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-blob-miss-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        init_repo(&dir);
        assert_eq!(blob_bytes(&dir, "HEAD", "nope.txt"), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compare_wrappers_ahead_behind_diverged_rename_and_failure() {
        let dir = std::env::temp_dir().join(format!(
            "ws-git-compare-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        init_repo(&dir);
        let base = exec_git(&["rev-parse", "HEAD"], &dir);
        fs::write(dir.join("one.txt"), "one\n").unwrap();
        git(&dir, &["add", "one.txt"]);
        git(&dir, &["commit", "-q", "-m", "ahead"]);
        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        let ahead = list_compare_name_status(&dir, &base, &head).unwrap();
        assert!(ahead.iter().any(|row| row.path == "one.txt"), "{ahead:?}");
        let behind = list_compare_name_status(&dir, &head, &base).unwrap();
        assert!(behind.is_empty(), "{behind:?}");
        git(&dir, &["checkout", "-q", "-b", "topic", &base]);
        fs::write(dir.join("topic.txt"), "topic\n").unwrap();
        git(&dir, &["add", "topic.txt"]);
        git(&dir, &["commit", "-q", "-m", "topic"]);
        git(&dir, &["checkout", "-q", "main"]);
        fs::write(dir.join("main-only.txt"), "main\n").unwrap();
        git(&dir, &["add", "main-only.txt"]);
        git(&dir, &["commit", "-q", "-m", "main-only"]);
        let main = exec_git(&["rev-parse", "HEAD"], &dir);
        let topic = exec_git(&["rev-parse", "topic"], &dir);
        let mb = merge_base(&dir, &main, &topic).unwrap();
        assert_eq!(mb.as_deref(), Some(base.as_str()));
        let diverged = list_compare_name_status(&dir, &topic, &main).unwrap();
        assert!(
            diverged.iter().any(|row| row.path == "main-only.txt"),
            "{diverged:?}"
        );
        git(&dir, &["mv", "one.txt", "renamed.txt"]);
        git(&dir, &["commit", "-q", "-m", "rename"]);
        let after_rename = exec_git(&["rev-parse", "HEAD"], &dir);
        // `main` still has `one.txt`. Three-dot from the initial seed commit
        // sees only an add (`renamed.txt` never existed on that base).
        let renamed = list_compare_name_status(&dir, &main, &after_rename).unwrap();
        assert!(
            renamed.iter().any(|row| {
                row.path == "renamed.txt" && row.old_path.as_deref() == Some("one.txt")
            }),
            "{renamed:?}"
        );
        assert!(rev_parse_commit(&dir, "no-such-ref").unwrap().is_none());
        assert!(exec_git_stdout(&["this-is-not-a-git-command"], &dir).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    /// `main` has `m.txt`, `d.txt`, `old.txt`; `feature` (checked out)
    /// modifies `m.txt`, deletes `d.txt`, renames `old.txt` → `new.txt`,
    /// and adds `added.txt`. Returns (dir, merge base, head).
    fn compare_revert_fixture() -> (std::path::PathBuf, String, String) {
        let dir = unique_dir("ws-git-compare-revert");
        init_repo(&dir);
        fs::write(dir.join("m.txt"), "m-base\n").unwrap();
        fs::write(dir.join("d.txt"), "d-base\n").unwrap();
        fs::write(dir.join("old.txt"), "r1\nr2\nr3\nr4\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "base"]);
        let merge_base = exec_git(&["rev-parse", "HEAD"], &dir);
        git(&dir, &["checkout", "-q", "-b", "feature"]);
        fs::write(dir.join("m.txt"), "m-head\n").unwrap();
        fs::write(dir.join("added.txt"), "added\n").unwrap();
        git(&dir, &["rm", "-q", "d.txt"]);
        git(&dir, &["mv", "old.txt", "new.txt"]);
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "feature"]);
        let head = exec_git(&["rev-parse", "HEAD"], &dir);
        (dir, merge_base, head)
    }

    fn index_is_head(dir: &Path) -> bool {
        exec_git_status(&["diff", "--cached", "--quiet"], dir) == 0
    }

    #[test]
    fn revert_compare_file_restores_each_status_from_merge_base() {
        let (dir, mb, head) = compare_revert_fixture();
        let listed = list_compare_name_status(&dir, &mb, &head).unwrap();
        let status = |path: &str| {
            listed
                .iter()
                .find(|row| row.path == path)
                .map(|row| row.status.clone())
        };
        assert_eq!(status("m.txt").as_deref(), Some("M"));
        assert_eq!(status("added.txt").as_deref(), Some("A"));
        assert_eq!(status("d.txt").as_deref(), Some("D"));
        assert_eq!(status("new.txt").as_deref(), Some("R"));

        // M: content back to the merge base.
        revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap();
        assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "m-base\n");
        // A: the file is not in the merge base, so it is deleted.
        revert_compare_file(&dir, &mb, &head, "added.txt", None).unwrap();
        assert!(!dir.join("added.txt").exists());
        // D: the file is written back.
        revert_compare_file(&dir, &mb, &head, "d.txt", None).unwrap();
        assert_eq!(fs::read_to_string(dir.join("d.txt")).unwrap(), "d-base\n");
        // R: old path written back, new path deleted.
        revert_compare_file(&dir, &mb, &head, "new.txt", Some("old.txt")).unwrap();
        assert!(!dir.join("new.txt").exists());
        assert_eq!(
            fs::read_to_string(dir.join("old.txt")).unwrap(),
            "r1\nr2\nr3\nr4\n"
        );

        assert!(
            index_is_head(&dir),
            "compare revert must not touch the index"
        );
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &dir), head);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_compare_file_aborts_when_head_moved_or_path_dirty() {
        let (dir, mb, head) = compare_revert_fixture();

        // Unstaged edit.
        fs::write(dir.join("m.txt"), "local edit\n").unwrap();
        let err = revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap_err();
        assert!(err.contains("m.txt has uncommitted changes"), "{err}");
        assert_eq!(
            fs::read_to_string(dir.join("m.txt")).unwrap(),
            "local edit\n"
        );
        revert_tracked_file(&dir, "m.txt").unwrap();

        // Staged-only edit.
        fs::write(dir.join("m.txt"), "staged edit\n").unwrap();
        stage_file(&dir, "m.txt").unwrap();
        fs::write(dir.join("m.txt"), "m-head\n").unwrap();
        let err = revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap_err();
        assert!(err.contains("uncommitted"), "{err}");
        unstage_file(&dir, "m.txt").unwrap();

        // Untracked file at a rename's old path would be overwritten.
        fs::write(dir.join("old.txt"), "mine\n").unwrap();
        let err = revert_compare_file(&dir, &mb, &head, "new.txt", Some("old.txt")).unwrap_err();
        assert!(err.contains("uncommitted"), "{err}");
        assert_eq!(fs::read_to_string(dir.join("old.txt")).unwrap(), "mine\n");
        assert!(dir.join("new.txt").exists());
        fs::remove_file(dir.join("old.txt")).unwrap();

        // HEAD moved since the compare loaded.
        fs::write(dir.join("other.txt"), "x\n").unwrap();
        git(&dir, &["add", "other.txt"]);
        git(&dir, &["commit", "-q", "-m", "moved"]);
        let err = revert_compare_file(&dir, &mb, &head, "m.txt", None).unwrap_err();
        assert!(err.contains("HEAD moved"), "{err}");
        assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "m-head\n");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn revert_compare_patch_checks_before_apply() {
        let (dir, mb, head) = compare_revert_fixture();
        let patch = diff_compare_file_ctx(&dir, &mb, &head, "m.txt", None, None)
            .unwrap()
            .join("\n")
            + "\n";
        fs::write(dir.join("m.txt"), "dirty\n").unwrap();
        let err = revert_compare_patch(&dir, &head, "m.txt", &patch).unwrap_err();
        assert!(err.contains("uncommitted"), "{err}");
        assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "dirty\n");
        revert_tracked_file(&dir, "m.txt").unwrap();
        revert_compare_patch(&dir, &head, "m.txt", &patch).unwrap();
        assert_eq!(fs::read_to_string(dir.join("m.txt")).unwrap(), "m-base\n");
        assert!(index_is_head(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn compare_picker_drops_origin_head_and_current_local() {
        let raw = "main\t1\t*\norigin/HEAD\t2\t\norigin/main\t3\t\nfeature\t4\t\n";
        let names: Vec<_> = parse_compare_picker_branches(raw)
            .into_iter()
            .map(|b| b.name)
            .collect();
        assert_eq!(names, vec!["origin/main", "feature"]);
    }

    #[test]
    fn compare_picker_commits_list_ancestors_without_head() {
        let dir = unique_dir("ws-git-picker-commits");
        init_repo(&dir);
        let rev = |r: &str| rev_parse_commit(&dir, r).unwrap().unwrap();
        // Root HEAD has no ancestors.
        assert_eq!(list_compare_picker_commits(&dir).unwrap(), Vec::new());
        let root = rev("HEAD");

        git(&dir, &["checkout", "-q", "-b", "side"]);
        fs::write(dir.join("side.txt"), "side\n").unwrap();
        git(&dir, &["add", "side.txt"]);
        git(&dir, &["commit", "-q", "-m", "Side\twork"]);
        let side = rev("HEAD");
        git(&dir, &["checkout", "-q", "main"]);
        fs::write(dir.join("main.txt"), "main\n").unwrap();
        git(&dir, &["add", "main.txt"]);
        git(&dir, &["commit", "-q", "-m", "Main work"]);
        let main = rev("HEAD");
        git(
            &dir,
            &["merge", "-q", "--no-ff", "-m", "Merge side", "side"],
        );
        let head = rev("HEAD");

        let rows = list_compare_picker_commits(&dir).unwrap();
        let mut ids: Vec<_> = rows.iter().map(|c| c.id.as_str()).collect();
        assert!(!ids.contains(&head.as_str()), "HEAD is excluded: {ids:?}");
        ids.sort_unstable();
        let mut want = vec![root.as_str(), side.as_str(), main.as_str()];
        want.sort_unstable();
        assert_eq!(ids, want, "first- and merge-side ancestors");
        let side_row = rows.iter().find(|c| c.id == side).unwrap();
        assert_eq!(side_row.subject, "Side\twork", "a tab stays in the subject");

        // The cap keeps the newest `limit` ancestors and still drops HEAD.
        let capped = list_compare_picker_commits_capped(&dir, 1).unwrap();
        assert_eq!(capped.len(), 1);
        assert_eq!(capped[0].id, rows[0].id);
        assert_ne!(capped[0].id, head);
        assert_eq!(
            list_compare_picker_commits_capped(&dir, 0).unwrap(),
            Vec::new()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn checked_git_errors_carry_the_git_reason_line() {
        let dir = unique_dir("ws-git-reason");
        init_repo(&dir);
        let err = stage_file(&dir, "nope.txt").unwrap_err();
        assert_eq!(err, "pathspec 'nope.txt' did not match any files");
        let err = stash_pop(&dir, "stash@{0}").unwrap_err();
        assert!(err.contains("stash@{0} is not a valid reference"), "{err}");
        assert!(!err.starts_with("error:"), "tag is stripped: {err}");

        // `stash apply` prints its conflict on stdout, not stderr.
        fs::write(dir.join("README.md"), "# stashed\n").unwrap();
        git(&dir, &["stash", "-q"]);
        fs::write(dir.join("README.md"), "# committed\n").unwrap();
        git(&dir, &["commit", "-qam", "conflicting"]);
        let err = stash_apply(&dir, "stash@{0}").unwrap_err();
        assert!(err.starts_with("CONFLICT"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn failed_patch_apply_reports_one_reason_line() {
        let dir = unique_dir("ws-git-patch-reason");
        init_repo(&dir);
        // git prints two `error:` lines for a patch that does not apply.
        let patch = "diff --git a/README.md b/README.md\n--- a/README.md\n+++ b/README.md\n\
                     @@ -1 +1 @@\n-# other\n+# new\n";
        let err = apply_cached_patch(&dir, patch, false).unwrap_err();
        assert_eq!(err, "patch failed: README.md:1");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn pull_reports_an_auto_stash_conflict_and_keeps_the_stash() {
        let root = unique_dir("ws-git-pull-conflict");
        let up = root.join("up");
        let down = root.join("down");
        init_repo(&up);
        git(
            &root,
            &["clone", "-q", up.to_str().unwrap(), down.to_str().unwrap()],
        );
        git(&down, &["config", "user.name", "workspace-status test"]);
        git(
            &down,
            &[
                "config",
                "user.email",
                "workspace-status-test@example.invalid",
            ],
        );
        fs::write(up.join("README.md"), "# upstream\n").unwrap();
        git(&up, &["commit", "-qam", "upstream"]);
        fs::write(down.join("README.md"), "# local edit\n").unwrap();

        let result = pull_quiet_detailed(&down);
        assert!(result.stashed && result.stash_pop_failed && !result.ok);
        assert_eq!(result.error, None, "the pull itself worked");
        assert_eq!(list_stash_refs(&down).len(), 1, "stash entry is kept");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn pull_failure_carries_the_git_reason() {
        let dir = unique_dir("ws-git-pull-fail");
        init_repo(&dir);
        let result = pull_quiet_detailed(&dir);
        assert!(!result.ok && !result.stash_pop_failed);
        let reason = result.error.expect("reason");
        assert!(
            !reason.is_empty() && !reason.contains("exited with code"),
            "{reason}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn push_on_detached_head_says_why() {
        let dir = unique_dir("ws-git-push-detached");
        init_repo(&dir);
        git(&dir, &["checkout", "-q", "--detach"]);
        assert_eq!(push_quiet(&dir).unwrap_err(), "detached HEAD cannot push");
        let _ = fs::remove_dir_all(&dir);
    }

    fn commit_file(dir: &std::path::Path, path: &str, text: &str, msg: &str) -> String {
        fs::write(dir.join(path), text).unwrap();
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", msg]);
        exec_git(&["rev-parse", "HEAD"], dir)
    }

    fn blame(dir: &std::path::Path, rev: BlameRev, path: &str, line: u32) -> LineBlame {
        blame_line(dir, &rev, path, line)
            .expect("git starts")
            .unwrap_or_else(|| panic!("blame {path}:{line}"))
    }

    #[test]
    fn parse_blame_porcelain_reads_headers_previous_and_boundary() {
        let renamed = "\
63b08ad4823cc8096b108736af6268306d1c5328 2 2 1
author Ada Lovelace
author-mail <ada@example.invalid>
author-time 1791204822
author-tz +0000
summary rename and fix
previous 3ef72304f100a91bb7f85a6f16253820cd51d07d old name.txt
filename new.txt
\tB
";
        let blame = parse_blame_porcelain(renamed).expect("parses");
        assert_eq!(blame.sha, "63b08ad4823cc8096b108736af6268306d1c5328");
        assert_eq!(blame.author, "Ada Lovelace");
        assert_eq!(blame.author_time, 1_791_204_822);
        assert_eq!(blame.summary, "rename and fix");
        assert_eq!(blame.orig_line, 2);
        assert_eq!(blame.filename, "new.txt");
        assert_eq!(
            blame.previous,
            Some((
                "3ef72304f100a91bb7f85a6f16253820cd51d07d".into(),
                "old name.txt".into()
            ))
        );
        assert!(!blame.boundary && !blame.uncommitted);

        let root = "3ef72304f100a91bb7f85a6f16253820cd51d07d 1 1 1\nauthor T\nsummary first\nboundary\nfilename f\n\ta\n";
        let root = parse_blame_porcelain(root).expect("parses");
        assert!(root.boundary && root.previous.is_none());

        let zero = format!(
            "{} 4 4 1\nauthor Not Committed Yet\nfilename f\n\tz\n",
            "0".repeat(40)
        );
        assert!(parse_blame_porcelain(&zero).expect("parses").uncommitted);

        assert_eq!(parse_blame_porcelain(""), None);
        assert_eq!(parse_blame_porcelain("not-a-sha 1 1 1\nfilename f\n"), None);
        assert_eq!(
            parse_blame_porcelain("3ef72304f100a91bb7f85a6f16253820cd51d07d 1 1 1\n\ta\n"),
            None,
            "no filename"
        );
    }

    #[test]
    fn blame_line_reads_commit_worktree_and_index_versions() {
        let dir = unique_dir("ws-git-blame");
        init_repo(&dir);
        let first = commit_file(&dir, "f.txt", "a\nb\nc\n", "first");

        let committed = blame(&dir, BlameRev::Worktree, "f.txt", 2);
        assert_eq!(committed.sha, first);
        assert_eq!(committed.summary, "first");
        assert_eq!(committed.author, "workspace-status test");
        assert_eq!(
            (committed.orig_line, committed.filename.as_str()),
            (2, "f.txt")
        );
        assert!(!committed.boundary && !committed.uncommitted);
        assert!(committed.author_time > 0);

        // A worktree edit is uncommitted; the index and HEAD still blame `first`.
        fs::write(dir.join("f.txt"), "a\nB\nc\n").unwrap();
        assert!(blame(&dir, BlameRev::Worktree, "f.txt", 2).uncommitted);
        assert_eq!(blame(&dir, BlameRev::Index, "f.txt", 2).sha, first);
        assert_eq!(
            blame(&dir, BlameRev::Commit("HEAD".into()), "f.txt", 2).sha,
            first
        );

        // Staged, the index copy differs from HEAD; line 1 still matches.
        git(&dir, &["add", "f.txt"]);
        fs::write(dir.join("f.txt"), "a\nb\nc\n").unwrap();
        assert!(blame(&dir, BlameRev::Index, "f.txt", 2).uncommitted);
        assert_eq!(blame(&dir, BlameRev::Index, "f.txt", 1).sha, first);
        assert!(!blame(&dir, BlameRev::Worktree, "f.txt", 2).uncommitted);

        // The seed commit is the root: porcelain marks it `boundary`.
        let root = blame(&dir, BlameRev::Commit(first.clone()), "README.md", 1);
        assert!(root.boundary && root.previous.is_none(), "{root:?}");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn blame_line_is_none_when_git_has_no_answer() {
        let dir = unique_dir("ws-git-blame-none");
        init_repo(&dir);
        fs::write(dir.join("untracked.txt"), "u\n").unwrap();
        for (rev, path, line) in [
            (BlameRev::Worktree, "untracked.txt", 1),
            (BlameRev::Index, "untracked.txt", 1),
            (BlameRev::Worktree, "missing.txt", 1),
            (BlameRev::Worktree, "README.md", 9),
            (BlameRev::Worktree, "README.md", 0),
            (BlameRev::Commit("no-such-ref".into()), "README.md", 1),
        ] {
            assert_eq!(
                blame_line(&dir, &rev, path, line),
                Ok(None),
                "{rev:?} {path}:{line}"
            );
        }
        let unborn = unique_dir("ws-git-blame-unborn");
        init_repo_empty(&unborn);
        fs::write(unborn.join("f.txt"), "a\n").unwrap();
        git(&unborn, &["add", "f.txt"]);
        assert_eq!(
            blame_line(&unborn, &BlameRev::Worktree, "f.txt", 1),
            Ok(None)
        );
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&unborn);
    }

    #[test]
    fn blame_line_follows_a_rename_through_previous() {
        let dir = unique_dir("ws-git-blame-rename");
        init_repo(&dir);
        let first = commit_file(&dir, "old.txt", "a\nb\n", "first");
        git(&dir, &["mv", "old.txt", "new.txt"]);
        let second = commit_file(&dir, "new.txt", "a\nB\n", "rename and fix");
        let changed = blame(&dir, BlameRev::Worktree, "new.txt", 2);
        assert_eq!(changed.sha, second);
        assert_eq!(changed.filename, "new.txt");
        assert_eq!(changed.previous, Some((first.clone(), "old.txt".into())));
        let kept = blame(&dir, BlameRev::Worktree, "new.txt", 1);
        assert_eq!(
            (kept.sha.as_str(), kept.filename.as_str()),
            (first.as_str(), "old.txt")
        );
        // The previous change of the renamed line is `first`, under the old name.
        match previous_line_change(&dir, &changed).unwrap() {
            PreviousLineChange::Found(earlier) => {
                assert_eq!(earlier.sha, first);
                assert_eq!(
                    (earlier.filename.as_str(), earlier.orig_line),
                    ("old.txt", 2)
                );
            }
            other => panic!("{other:?}"),
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn previous_line_change_finds_the_older_edit_or_the_adding_commit() {
        let dir = unique_dir("ws-git-blame-previous");
        init_repo(&dir);
        commit_file(&dir, "f.txt", "a\nb\nc\n", "first");
        let second = commit_file(&dir, "f.txt", "a\nB\nc\n", "second");
        // `third` inserts a line above, edits B again, and appends `d`.
        let third = commit_file(&dir, "f.txt", "top\na\nBB\nc\nd\n", "third");

        let line = blame(&dir, BlameRev::Worktree, "f.txt", 3);
        assert_eq!((line.sha.as_str(), line.orig_line), (third.as_str(), 3));
        match previous_line_change(&dir, &line).unwrap() {
            PreviousLineChange::Found(earlier) => {
                assert_eq!(
                    (earlier.sha.as_str(), earlier.orig_line),
                    (second.as_str(), 2)
                );
                assert!(!earlier.boundary);
            }
            other => panic!("{other:?}"),
        }

        let appended = blame(&dir, BlameRev::Worktree, "f.txt", 5);
        assert_eq!(appended.sha, third);
        assert_eq!(
            previous_line_change(&dir, &appended),
            Ok(PreviousLineChange::AddedIn)
        );

        let new_file_sha = commit_file(&dir, "g.txt", "g\n", "add g");
        let new_file = blame(&dir, BlameRev::Worktree, "g.txt", 1);
        assert_eq!(
            (new_file.sha.as_str(), new_file.previous.as_ref()),
            (new_file_sha.as_str(), None)
        );
        assert_eq!(
            previous_line_change(&dir, &new_file),
            Ok(PreviousLineChange::AddedIn)
        );

        // Editing the seed line: the earlier change is the root commit.
        commit_file(&dir, "README.md", "# edited\n", "edit readme");
        let readme = blame(&dir, BlameRev::Worktree, "README.md", 1);
        match previous_line_change(&dir, &readme).unwrap() {
            PreviousLineChange::Found(earlier) => assert!(earlier.boundary, "{earlier:?}"),
            other => panic!("{other:?}"),
        }

        fs::write(dir.join("f.txt"), "top\na\nzz\nc\nd\n").unwrap();
        let dirty = blame(&dir, BlameRev::Worktree, "f.txt", 3);
        assert_eq!(
            previous_line_change(&dir, &dirty),
            Err("line is not committed yet".into())
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn map_line_to_parent_follows_u0_hunks() {
        // Line 2 modified in place.
        let modified = "@@ -2 +2 @@\n-b\n+B\n";
        assert_eq!(map_line_to_parent(modified, 2), Some(2));
        assert_eq!(map_line_to_parent(modified, 1), Some(1));
        assert_eq!(map_line_to_parent(modified, 3), Some(3));
        // Lines 2..3 became 2..5: only the first two map back.
        let grown = "@@ -2,2 +2,4 @@\n-b\n-c\n+B\n+C\n+D\n+E\n";
        assert_eq!(map_line_to_parent(grown, 3), Some(3));
        assert_eq!(map_line_to_parent(grown, 4), None);
        assert_eq!(map_line_to_parent(grown, 6), Some(4), "shifted by -2");
        // Pure insertion after line 1, then a pure deletion after new line 5.
        let mixed = "@@ -1,0 +2,2 @@\n+x\n+y\n@@ -4,3 +5,0 @@\n-p\n-q\n-r\n";
        assert_eq!(map_line_to_parent(mixed, 1), Some(1));
        assert_eq!(map_line_to_parent(mixed, 2), None);
        assert_eq!(map_line_to_parent(mixed, 3), None);
        assert_eq!(map_line_to_parent(mixed, 4), Some(2));
        assert_eq!(map_line_to_parent(mixed, 5), Some(3));
        assert_eq!(map_line_to_parent(mixed, 6), Some(7));
        assert_eq!(map_line_to_parent("", 4), Some(4));
    }

    #[test]
    fn git_worktree_compare_diff_args_adds_context_and_rename_path() {
        assert_eq!(
            git_worktree_compare_diff_args("abc", "new.txt", Some("old.txt"), Some(3)),
            ["diff", "-U3", "-M", "abc", "--", "old.txt", "new.txt"]
        );
        assert_eq!(
            git_worktree_compare_diff_args("abc", "f.txt", Some("f.txt"), None),
            ["diff", "-M", "abc", "--", "f.txt"]
        );
    }

    #[test]
    fn worktree_vs_commit_lists_and_diffs_one_path() {
        let dir = unique_dir("ws-git-worktree-compare");
        init_repo(&dir);
        let base = commit_file(&dir, "old.txt", "one\ntwo\nthree\n", "base");
        commit_file(&dir, "f.txt", "x\n", "later");

        let clean = list_worktree_vs_commit_name_status(&dir, &base, "old.txt", None).unwrap();
        assert!(clean.is_empty(), "{clean:?}");
        assert_eq!(
            diff_worktree_vs_commit_file_ctx(&dir, &base, "old.txt", None, None).unwrap(),
            ["(no diff)"]
        );

        // Rename in the index, edit in the worktree only: both count.
        git(&dir, &["mv", "old.txt", "new.txt"]);
        fs::write(dir.join("new.txt"), "one\ntwo\nthree\nfour\n").unwrap();
        let rows =
            list_worktree_vs_commit_name_status(&dir, &base, "new.txt", Some("old.txt")).unwrap();
        assert_eq!(rows.len(), 1, "{rows:?}");
        assert_eq!(rows[0].status, "R");
        assert_eq!(rows[0].path, "new.txt");
        assert_eq!(rows[0].old_path.as_deref(), Some("old.txt"));
        assert_eq!(rows[0].stat, counts(1, 0));
        let diff =
            diff_worktree_vs_commit_file_ctx(&dir, &base, "new.txt", Some("old.txt"), Some(0))
                .unwrap();
        assert!(diff.iter().any(|l| l == "rename from old.txt"), "{diff:?}");
        assert!(diff.iter().any(|l| l == "+four"), "{diff:?}");

        let modified = list_worktree_vs_commit_name_status(&dir, &base, "f.txt", None).unwrap();
        assert_eq!(
            modified[0].status, "A",
            "f.txt is newer than base: {modified:?}"
        );
        assert!(
            diff_worktree_vs_commit_file_ctx(&dir, "no-such-ref", "f.txt", None, None).is_err()
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

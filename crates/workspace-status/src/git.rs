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

/// Run git with `stdin` piped. Used only by the `git apply` wrappers.
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
        Ok(out) => Err(git_reason_line(&out).unwrap_or_else(|| exit_code_message(args, &out))),
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
        _ => return Err("revert aborted: HEAD moved".into()),
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
        Err(format!("revert aborted: {path} has uncommitted changes"))
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

fn git_failure_message(args: &[&str], out: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !stderr.is_empty() {
        return stderr;
    }
    format!(
        "git {} exited with code {}",
        args.first().copied().unwrap_or("git"),
        out.status.code().unwrap_or(-1)
    )
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
    Ok(parse_name_status_lines(&stdout))
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
                },
                NameStatus {
                    status: "R".into(),
                    path: "new.rs".into(),
                    old_path: Some("old.rs".into()),
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
}

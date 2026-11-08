//! External diff tool config, argv, and LEFT/RIGHT temp files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::{SystemTime, UNIX_EPOCH};

use super::editor::parse_editor_argv;
use crate::git::blob_bytes;

/// Config `diffTool`, or `vimdiff` when omitted / blank.
pub fn resolve_diff_tool(config: Option<&str>) -> String {
    if let Some(raw) = config {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    "vimdiff".into()
}

/// Build `(command, args)` for an external diff of `left` vs `right`.
///
/// git-difftool-style `$LOCAL` (LEFT) and `$REMOTE` (RIGHT) are substituted in
/// any argv token, including inside a token such as `--left=$LOCAL`. When
/// neither placeholder appears anywhere in the parsed argv, LEFT then RIGHT
/// are appended after the existing flags.
pub fn diff_tool_command(tool: &str, left: &str, right: &str) -> (String, Vec<String>) {
    let argv = parse_editor_argv(tool);
    let command = argv.first().cloned().unwrap_or_else(|| tool.to_string());
    let rest: Vec<String> = argv.into_iter().skip(1).collect();
    let has_placeholder = [&command]
        .into_iter()
        .chain(rest.iter())
        .any(|t| t.contains("$LOCAL") || t.contains("$REMOTE"));
    if has_placeholder {
        let sub = |s: &str| s.replace("$LOCAL", left).replace("$REMOTE", right);
        (sub(&command), rest.into_iter().map(|t| sub(&t)).collect())
    } else {
        let mut args = rest;
        args.push(left.into());
        args.push(right.into());
        (command, args)
    }
}

/// Name prefix of this process's external-diff directory under `$TMPDIR`.
const EXT_DIFF_DIR_PREFIX: &str = "workspace-status-ext-diff";

/// This process's external-diff directory.
enum ExtDiffDir {
    /// Not made yet.
    Unmade,
    /// Made at this path.
    Made(PathBuf),
    /// Removed at quit; no new one is made.
    Closed,
}

/// This process's external-diff directory ([`ext_diff_temp_dir`]).
static EXT_DIFF_DIR: Mutex<ExtDiffDir> = Mutex::new(ExtDiffDir::Unmade);

/// Session directories a detached tool still has open ([`wait_and_cleanup`]).
static LIVE_SESSIONS: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

/// Lock `mutex`, also after a panic in another holder.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// This process's private directory for external-diff temp files:
/// `$TMPDIR/workspace-status-ext-diff-<pid>-<nanos>-<n>`, made on first use
/// by [`create_private_dir`].
///
/// Each process and user gets its own directory, so a shared `/tmp` never
/// mixes their temps and another user cannot plant files or links in it.
/// [`remove_ext_diff_temp_dir`] deletes it when the TUI quits; after that
/// this is `Err`, so a diff worker that outlives quit leaves nothing.
pub fn ext_diff_temp_dir() -> Result<PathBuf, String> {
    let mut slot = lock(&EXT_DIFF_DIR);
    match &*slot {
        ExtDiffDir::Made(dir) if dir.is_dir() => return Ok(dir.clone()),
        ExtDiffDir::Closed => return Err("ws is exiting".into()),
        ExtDiffDir::Made(_) | ExtDiffDir::Unmade => {}
    }
    let dir = create_private_dir(&std::env::temp_dir(), EXT_DIFF_DIR_PREFIX)
        .map_err(|e| e.to_string())?;
    *slot = ExtDiffDir::Made(dir.clone());
    Ok(dir)
}

/// Create a new directory `<parent>/<prefix>-<pid>-<nanos>-<n>` that only
/// its owner can open (mode 0700 on Unix).
///
/// The create fails on a name that exists, so a directory someone else
/// made first is never used: the next `n` is tried instead.
fn create_private_dir(parent: &Path, prefix: &str) -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    for _ in 0..100 {
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = parent.join(format!("{prefix}-{}-{nanos}-{n}", std::process::id()));
        match builder.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "no free external-diff temp directory name",
    ))
}

/// Delete this process's external-diff directory when the TUI quits.
///
/// A session a detached tool still has open stays on disk (no one waits
/// for that tool after the TUI exits), and so does the directory that
/// holds it. Everything else goes.
pub fn remove_ext_diff_temp_dir() {
    let mut slot = lock(&EXT_DIFF_DIR);
    if let ExtDiffDir::Made(root) = std::mem::replace(&mut *slot, ExtDiffDir::Closed) {
        remove_ext_diff_dir_at(&root, &lock(&LIVE_SESSIONS));
    }
}

/// Delete every session under `root` except `live`, then `root` when it
/// is empty.
fn remove_ext_diff_dir_at(root: &Path, live: &[PathBuf]) {
    if let Ok(entries) = fs::read_dir(root) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !live.contains(&path) {
                let _ = fs::remove_dir_all(&path);
            }
        }
    }
    let _ = fs::remove_dir(root);
}

/// LEFT/RIGHT paths for one external-diff session.
#[derive(Debug, Clone)]
pub struct PreparedDiff {
    /// Base / LEFT path passed to the tool (`$LOCAL`).
    pub left: PathBuf,
    /// Worktree or RIGHT blob path (`$REMOTE`).
    pub right: PathBuf,
    left_is_temp: bool,
    right_is_temp: bool,
    /// Per-session directory under [`ext_diff_temp_dir`]. Never the worktree.
    session_dir: PathBuf,
}

/// One new session directory `session-<n>` under [`ext_diff_temp_dir`].
///
/// Later prepares do not delete sibling sessions. A detached GUI keeps this
/// directory until [`wait_and_cleanup`] runs after the child exits.
fn new_session_dir() -> Result<PathBuf, String> {
    create_private_dir(&ext_diff_temp_dir()?, "session").map_err(|e| e.to_string())
}

/// `{stem}.{side}.{nanos}.{ext}` so tools that key off the last suffix (vimdiff) see `.rs`, not `{nanos}`.
fn unique_temp_path(session_dir: &Path, rel_path: &str, side: &str) -> PathBuf {
    let file_name = Path::new(rel_path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let path = Path::new(file_name);
    let name = match (
        path.file_stem().and_then(|s| s.to_str()),
        path.extension().and_then(|s| s.to_str()),
    ) {
        (Some(stem), Some(ext)) if !stem.is_empty() => {
            format!("{stem}.{side}.{nanos}.{ext}")
        }
        _ => format!("{file_name}.{side}.{nanos}"),
    };
    session_dir.join(name)
}

fn write_temp_bytes(
    session_dir: &Path,
    rel_path: &str,
    side: &str,
    bytes: &[u8],
) -> Result<PathBuf, String> {
    let path = unique_temp_path(session_dir, rel_path, side);
    fs::write(&path, bytes).map_err(|e| e.to_string())?;
    Ok(path)
}

/// HEAD vs worktree for a dirty-tree file.
///
/// LEFT is always a temp: `HEAD:path` bytes when the path exists in HEAD, otherwise
/// an empty file. RIGHT is the absolute worktree path, or an empty temp when the
/// worktree file is missing (deleted).
pub fn prepare_worktree_diff(repo_abs: &Path, rel_path: &str) -> Result<PreparedDiff, String> {
    let session_dir = new_session_dir()?;
    let left_bytes = blob_bytes(repo_abs, "HEAD", rel_path).unwrap_or_default();
    let left = write_temp_bytes(&session_dir, rel_path, "left", &left_bytes)?;
    let worktree = repo_abs.join(rel_path);
    if worktree.exists() {
        Ok(PreparedDiff {
            left,
            right: worktree,
            left_is_temp: true,
            right_is_temp: false,
            session_dir,
        })
    } else {
        let right = write_temp_bytes(&session_dir, rel_path, "right", &[])?;
        Ok(PreparedDiff {
            left,
            right,
            left_is_temp: true,
            right_is_temp: true,
            session_dir,
        })
    }
}

#[cfg(test)]
/// Commit/stash file: both sides are blob temps (`left_rev:path` / `right_rev:path`).
///
/// Missing path at a rev becomes an empty temp (added on the right, or deleted on the right).
pub fn prepare_rev_diff(
    repo_abs: &Path,
    left_rev: &str,
    right_rev: &str,
    rel_path: &str,
) -> Result<PreparedDiff, String> {
    prepare_rev_diff_paths(repo_abs, left_rev, right_rev, rel_path, None)
}

/// Like [`prepare_rev_diff`], with an optional LEFT path for renames.
pub fn prepare_rev_diff_paths(
    repo_abs: &Path,
    left_rev: &str,
    right_rev: &str,
    rel_path: &str,
    left_path: Option<&str>,
) -> Result<PreparedDiff, String> {
    let session_dir = new_session_dir()?;
    let left_file = left_path.unwrap_or(rel_path);
    let left_bytes = blob_bytes(repo_abs, left_rev, left_file).unwrap_or_default();
    let right_bytes = blob_bytes(repo_abs, right_rev, rel_path).unwrap_or_default();
    let left = write_temp_bytes(&session_dir, left_file, "left", &left_bytes)?;
    let right = write_temp_bytes(&session_dir, rel_path, "right", &right_bytes)?;
    Ok(PreparedDiff {
        left,
        right,
        left_is_temp: true,
        right_is_temp: true,
        session_dir,
    })
}

/// Delete temp files created for this session. A worktree RIGHT path is left in place.
pub fn cleanup_prepared(prepared: &PreparedDiff) {
    if prepared.left_is_temp {
        let _ = fs::remove_file(&prepared.left);
    }
    if prepared.right_is_temp {
        let _ = fs::remove_file(&prepared.right);
    }
    let _ = fs::remove_dir(&prepared.session_dir);
}

/// Wait for the spawned diff-tool process to exit, then delete this session's
/// temps only.
///
/// GUI CLIs such as Cursor and VS Code return in a few seconds unless the
/// command includes `--wait`. Cleanup runs when that process exits. Without
/// `--wait`, the LEFT HEAD temp is gone before the IDE opens the files.
/// Config `diffTool` is used as-is. Samples use `cursor --diff --wait`.
///
/// While it waits, the session counts as live, so
/// [`remove_ext_diff_temp_dir`] at quit leaves it in place. The live loop
/// uses [`wait_and_cleanup_in_background`].
#[cfg(test)]
pub fn wait_and_cleanup(child: Child, prepared: PreparedDiff) {
    lock(&LIVE_SESSIONS).push(prepared.session_dir.clone());
    wait_then_cleanup(child, prepared);
}

/// [`wait_and_cleanup`] on a new thread. The session counts as live
/// before this returns, so a quit right after the spawn keeps it.
pub fn wait_and_cleanup_in_background(child: Child, prepared: PreparedDiff) {
    lock(&LIVE_SESSIONS).push(prepared.session_dir.clone());
    std::thread::spawn(move || wait_then_cleanup(child, prepared));
}

/// Wait for `child`, delete the session, and drop it from the live list.
fn wait_then_cleanup(mut child: Child, prepared: PreparedDiff) {
    let _ = child.wait();
    cleanup_prepared(&prepared);
    lock(&LIVE_SESSIONS).retain(|dir| dir != &prepared.session_dir);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{git, init_repo};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn resolve_none_blank_whitespace_is_vimdiff() {
        assert_eq!(resolve_diff_tool(None), "vimdiff");
        assert_eq!(resolve_diff_tool(Some("")), "vimdiff");
        assert_eq!(resolve_diff_tool(Some("   ")), "vimdiff");
    }

    #[test]
    fn resolve_non_blank_config_wins() {
        assert_eq!(resolve_diff_tool(Some("cursor --diff")), "cursor --diff");
        assert_eq!(resolve_diff_tool(Some("  nvim -d  ")), "nvim -d");
    }

    #[test]
    fn vimdiff_appends_left_then_right() {
        let (cmd, args) = diff_tool_command("vimdiff", "/tmp/left", "/repo/file.rs");
        assert_eq!(cmd, "vimdiff");
        assert_eq!(args, vec!["/tmp/left", "/repo/file.rs"]);
    }

    #[test]
    fn cursor_diff_appends_paths_when_no_placeholders() {
        let (cmd, args) = diff_tool_command("cursor --diff", "/tmp/left", "/repo/file.rs");
        assert_eq!(cmd, "cursor");
        assert_eq!(args, vec!["--diff", "/tmp/left", "/repo/file.rs"]);
    }

    #[test]
    fn cursor_diff_wait_appends_paths() {
        let (cmd, args) = diff_tool_command("cursor --diff --wait", "/tmp/left", "/repo/file.rs");
        assert_eq!(cmd, "cursor");
        assert_eq!(args, vec!["--diff", "--wait", "/tmp/left", "/repo/file.rs"]);
    }

    #[test]
    fn code_diff_wait_appends_paths() {
        let (cmd, args) = diff_tool_command("code --diff --wait", "/l", "/r");
        assert_eq!(cmd, "code");
        assert_eq!(args, vec!["--diff", "--wait", "/l", "/r"]);
    }

    #[test]
    fn local_remote_placeholders_substitute_paths() {
        let (cmd, args) = diff_tool_command("vimdiff $LOCAL $REMOTE", "/tmp/left", "/repo/file.rs");
        assert_eq!(cmd, "vimdiff");
        assert_eq!(args, vec!["/tmp/left", "/repo/file.rs"]);
    }

    #[test]
    fn remote_local_placeholders_swap_order() {
        let (cmd, args) = diff_tool_command("tool $REMOTE $LOCAL", "/left", "/right");
        assert_eq!(cmd, "tool");
        assert_eq!(args, vec!["/right", "/left"]);
    }

    #[test]
    fn placeholders_inside_a_token_are_replaced() {
        let (cmd, args) = diff_tool_command(
            "tool --left=$LOCAL --right=$REMOTE",
            "/tmp/left",
            "/repo/file.rs",
        );
        assert_eq!(cmd, "tool");
        assert_eq!(args, vec!["--left=/tmp/left", "--right=/repo/file.rs"]);
    }

    static PREPARE_LOCK: Mutex<()> = Mutex::new(());

    fn lock_temps() -> std::sync::MutexGuard<'static, ()> {
        PREPARE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn unique_repo(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        init_repo(&dir);
        dir
    }

    #[test]
    fn prepared_temp_paths_keep_source_extension_last() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-suffix");
        fs::write(dir.join("src.rs"), "fn a() {}\n").unwrap();
        git(&dir, &["add", "src.rs"]);
        git(&dir, &["commit", "-q", "-m", "rs"]);
        fs::write(dir.join("src.rs"), "fn b() {}\n").unwrap();
        let prepared = prepare_worktree_diff(&dir, "src.rs").unwrap();
        assert_eq!(
            prepared.left.extension().and_then(|s| s.to_str()),
            Some("rs"),
            "LEFT temp last suffix should be .rs, got {:?}",
            prepared.left
        );
        cleanup_prepared(&prepared);

        fs::write(dir.join("notes.md"), "# n\n").unwrap();
        git(&dir, &["add", "notes.md"]);
        git(&dir, &["commit", "-q", "-m", "md"]);
        fs::remove_file(dir.join("notes.md")).unwrap();
        let prepared = prepare_worktree_diff(&dir, "notes.md").unwrap();
        assert_eq!(
            prepared.left.extension().and_then(|s| s.to_str()),
            Some("md"),
            "LEFT temp last suffix should be .md, got {:?}",
            prepared.left
        );
        assert_eq!(
            prepared.right.extension().and_then(|s| s.to_str()),
            Some("md"),
            "RIGHT temp last suffix should be .md, got {:?}",
            prepared.right
        );
        cleanup_prepared(&prepared);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn modified_tracked_left_is_head_temp_right_is_worktree() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-mod");
        fs::write(dir.join("README.md"), "# dirty\n").unwrap();
        let prepared = prepare_worktree_diff(&dir, "README.md").unwrap();
        assert_eq!(fs::read(&prepared.left).unwrap(), b"# seed\n");
        assert_eq!(prepared.right, dir.join("README.md"));
        assert_ne!(prepared.left, dir.join("README.md"));
        assert!(prepared.left.starts_with(ext_diff_temp_dir().unwrap()));
        assert!(prepared
            .left
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.starts_with("session-")));
        cleanup_prepared(&prepared);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn untracked_left_is_empty_temp_right_is_worktree() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-new");
        fs::write(dir.join("new.txt"), "new\n").unwrap();
        let prepared = prepare_worktree_diff(&dir, "new.txt").unwrap();
        assert_eq!(fs::read(&prepared.left).unwrap(), b"");
        assert_eq!(prepared.right, dir.join("new.txt"));
        assert!(prepared.left.starts_with(ext_diff_temp_dir().unwrap()));
        cleanup_prepared(&prepared);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleted_right_is_empty_temp_left_has_head_bytes() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-del");
        fs::remove_file(dir.join("README.md")).unwrap();
        let prepared = prepare_worktree_diff(&dir, "README.md").unwrap();
        assert_eq!(fs::read(&prepared.left).unwrap(), b"# seed\n");
        assert_eq!(fs::read(&prepared.right).unwrap(), b"");
        assert_ne!(prepared.right, dir.join("README.md"));
        assert!(prepared.right.starts_with(ext_diff_temp_dir().unwrap()));
        cleanup_prepared(&prepared);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn cleanup_removes_left_and_right_temps_not_worktree() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-clean");
        fs::write(dir.join("README.md"), "# dirty\n").unwrap();
        let prepared = prepare_worktree_diff(&dir, "README.md").unwrap();
        let left = prepared.left.clone();
        let right = prepared.right.clone();
        assert!(left.exists());
        cleanup_prepared(&prepared);
        assert!(!left.exists());
        assert!(right.exists());
        assert_eq!(fs::read(&right).unwrap(), b"# dirty\n");

        fs::remove_file(dir.join("README.md")).unwrap();
        let prepared = prepare_worktree_diff(&dir, "README.md").unwrap();
        let left = prepared.left.clone();
        let right = prepared.right.clone();
        cleanup_prepared(&prepared);
        assert!(!left.exists());
        assert!(!right.exists());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_does_not_delete_other_session_temps() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-nosweep");
        let other = ext_diff_temp_dir().unwrap().join("999999-1");
        fs::create_dir_all(&other).unwrap();
        let leftover = other.join("still-open-in-gui.rs");
        fs::write(&leftover, b"stale").unwrap();
        fs::write(dir.join("README.md"), "# dirty\n").unwrap();
        let prepared = prepare_worktree_diff(&dir, "README.md").unwrap();
        assert!(
            leftover.exists(),
            "other session temps must survive prepare"
        );
        assert_ne!(
            prepared.left.parent().map(Path::as_os_str),
            Some(other.as_os_str())
        );
        cleanup_prepared(&prepared);
        assert!(leftover.exists(), "cleanup must not touch other sessions");
        let _ = fs::remove_dir_all(&other);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_and_cleanup_removes_only_this_session() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-wait");
        fs::write(dir.join("README.md"), "# dirty\n").unwrap();
        let prepared = prepare_worktree_diff(&dir, "README.md").unwrap();
        let left = prepared.left.clone();
        let other = ext_diff_temp_dir().unwrap().join("888888-2");
        fs::create_dir_all(&other).unwrap();
        let leftover = other.join("gui-left.rs");
        fs::write(&leftover, b"open").unwrap();
        let child = Command::new(crate::git::git_binary())
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn git --version");
        wait_and_cleanup(child, prepared);
        assert!(!left.exists());
        assert!(leftover.exists());
        let _ = fs::remove_dir_all(&other);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn wait_and_cleanup_keeps_left_temp_until_child_exits() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-wait-hold");
        fs::write(dir.join("README.md"), "# dirty\n").unwrap();
        let prepared = prepare_worktree_diff(&dir, "README.md").unwrap();
        let left = prepared.left.clone();
        let mut child = Command::new(crate::git::git_binary())
            .args(["hash-object", "--stdin"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn git hash-object --stdin");
        let stdin = child.stdin.take().expect("piped stdin");
        let worker = std::thread::spawn(move || {
            wait_and_cleanup(child, prepared);
        });
        assert!(
            left.exists(),
            "LEFT HEAD temp must survive while the diff-tool process is still running"
        );
        drop(stdin);
        worker.join().expect("wait_and_cleanup thread");
        assert!(
            !left.exists(),
            "LEFT HEAD temp must be deleted after the diff-tool process exits"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// The temp root belongs to this process: not the old shared
    /// `$TMPDIR/workspace-status-ext-diff`, named by this process id, and
    /// owner-only on Unix.
    #[test]
    fn ext_diff_dir_is_private_to_this_process() {
        let _lock = lock_temps();
        let root = ext_diff_temp_dir().unwrap();
        assert_ne!(root, std::env::temp_dir().join(EXT_DIFF_DIR_PREFIX));
        let name = root.file_name().and_then(|s| s.to_str()).unwrap();
        let own = format!("{EXT_DIFF_DIR_PREFIX}-{}-", std::process::id());
        assert!(name.starts_with(&own), "{name}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&root).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "root mode {mode:o}");
        }
    }

    /// Each private directory is new: two calls never share one.
    #[test]
    fn private_dirs_are_new_each_time() {
        let parent = crate::testutil::unique_dir("ws-ext-diff-private");
        fs::create_dir_all(&parent).unwrap();
        let a = create_private_dir(&parent, "x").unwrap();
        let b = create_private_dir(&parent, "x").unwrap();
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        let _ = fs::remove_dir_all(&parent);
    }

    /// Quit deletes finished sessions and keeps one a detached tool still
    /// has open; with none open, the root goes too.
    #[test]
    fn quit_removes_every_session_but_the_live_ones() {
        let parent = crate::testutil::unique_dir("ws-ext-diff-quit");
        fs::create_dir_all(&parent).unwrap();
        let root = create_private_dir(&parent, "root").unwrap();
        let done = create_private_dir(&root, "session").unwrap();
        fs::write(done.join("left.rs"), "x").unwrap();
        let live = create_private_dir(&root, "session").unwrap();
        fs::write(live.join("left.rs"), "y").unwrap();
        remove_ext_diff_dir_at(&root, std::slice::from_ref(&live));
        assert!(!done.exists(), "finished session removed");
        assert!(live.join("left.rs").exists(), "live session kept");
        remove_ext_diff_dir_at(&root, &[]);
        assert!(!root.exists(), "empty root removed");
        let _ = fs::remove_dir_all(&parent);
    }

    #[test]
    fn rev_diff_both_sides_are_blob_temps() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-rev");
        fs::write(dir.join("README.md"), "# v2\n").unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "v2"]);
        let prepared = prepare_rev_diff(&dir, "HEAD^", "HEAD", "README.md").unwrap();
        assert_eq!(fs::read(&prepared.left).unwrap(), b"# seed\n");
        assert_eq!(fs::read(&prepared.right).unwrap(), b"# v2\n");
        assert!(prepared.left.starts_with(ext_diff_temp_dir().unwrap()));
        assert!(prepared.right.starts_with(ext_diff_temp_dir().unwrap()));
        assert_ne!(prepared.right, dir.join("README.md"));
        cleanup_prepared(&prepared);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rev_diff_added_path_has_empty_left() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-rev-add");
        fs::write(dir.join("added.txt"), "only-at-head\n").unwrap();
        git(&dir, &["add", "added.txt"]);
        git(&dir, &["commit", "-q", "-m", "add"]);
        let prepared = prepare_rev_diff(&dir, "HEAD^", "HEAD", "added.txt").unwrap();
        assert_eq!(fs::read(&prepared.left).unwrap(), b"");
        assert_eq!(fs::read(&prepared.right).unwrap(), b"only-at-head\n");
        cleanup_prepared(&prepared);
        let _ = fs::remove_dir_all(&dir);
    }
}

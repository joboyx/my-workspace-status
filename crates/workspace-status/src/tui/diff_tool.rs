//! External diff tool config, argv, and LEFT/RIGHT temp files.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Child;
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

/// Shared directory for external-diff temp files (`$TMPDIR/workspace-status-ext-diff`).
pub fn ext_diff_temp_dir() -> PathBuf {
    std::env::temp_dir().join("workspace-status-ext-diff")
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

/// One session directory: `$TMPDIR/workspace-status-ext-diff/<pid>-<nanos>/`.
///
/// Later prepares do not delete sibling sessions. A detached GUI keeps this
/// directory until [`wait_and_cleanup`] runs after the child exits.
fn new_session_dir() -> Result<PathBuf, String> {
    let root = ext_diff_temp_dir();
    fs::create_dir_all(&root).map_err(|e| e.to_string())?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let dir = root.join(format!("{}-{nanos}", std::process::id()));
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
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

/// Commit/stash file: both sides are blob temps (`left_rev:path` / `right_rev:path`).
///
/// Missing path at a rev becomes an empty temp (added on the right, or deleted on the right).
pub fn prepare_rev_diff(
    repo_abs: &Path,
    left_rev: &str,
    right_rev: &str,
    rel_path: &str,
) -> Result<PreparedDiff, String> {
    let session_dir = new_session_dir()?;
    let left_bytes = blob_bytes(repo_abs, left_rev, rel_path).unwrap_or_default();
    let right_bytes = blob_bytes(repo_abs, right_rev, rel_path).unwrap_or_default();
    let left = write_temp_bytes(&session_dir, rel_path, "left", &left_bytes)?;
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

/// Wait for a detached diff-tool child, then delete this session's temps only.
pub fn wait_and_cleanup(mut child: Child, prepared: PreparedDiff) {
    let _ = child.wait();
    cleanup_prepared(&prepared);
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn git_env() -> [(&'static str, &'static str); 4] {
        [
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
        let mut cmd = Command::new(crate::git::git_binary());
        cmd.args(args).current_dir(cwd);
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        let status = cmd.status().expect("git");
        assert!(status.success(), "git {args:?}");
    }

    fn init_repo(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let init = Command::new(crate::git::git_binary())
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
        assert!(prepared.left.starts_with(ext_diff_temp_dir()));
        assert!(prepared
            .left
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .is_some_and(|name| name.starts_with(&format!("{}-", std::process::id()))));
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
        assert!(prepared.left.starts_with(ext_diff_temp_dir()));
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
        assert!(prepared.right.starts_with(ext_diff_temp_dir()));
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
        fs::create_dir_all(ext_diff_temp_dir()).unwrap();
        let other = ext_diff_temp_dir().join("999999-1");
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
        let other = ext_diff_temp_dir().join("888888-2");
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
    fn rev_diff_both_sides_are_blob_temps() {
        let _lock = lock_temps();
        let dir = unique_repo("ws-ext-diff-rev");
        fs::write(dir.join("README.md"), "# v2\n").unwrap();
        git(&dir, &["add", "README.md"]);
        git(&dir, &["commit", "-q", "-m", "v2"]);
        let prepared = prepare_rev_diff(&dir, "HEAD^", "HEAD", "README.md").unwrap();
        assert_eq!(fs::read(&prepared.left).unwrap(), b"# seed\n");
        assert_eq!(fs::read(&prepared.right).unwrap(), b"# v2\n");
        assert!(prepared.left.starts_with(ext_diff_temp_dir()));
        assert!(prepared.right.starts_with(ext_diff_temp_dir()));
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

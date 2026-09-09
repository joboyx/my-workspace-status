use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use crate::harness::{left_tree, tree_row_containing, PtySession};
use crate::seed::stream_workspace;
use crate::support::{right_pane, tree_cursor_on, tree_has, GIT_WAIT, TREE_LABEL_COL, WAIT};

/// Unique graph subject for `fast` (`seed_repo` commit message).
const FAST_GRAPH_SUBJECT: &str = "seed fast";

/// Unique graph subject for `slow`. A focused-fast pane must not show this.
const SLOW_GRAPH_SUBJECT: &str = "seed slow";

/// Unique worktree body written into `fast` during streamed collect.
const STREAMED_MARKER_BODY: &str = "streamed-collect-body";

/// `WORKSPACE_STATUS_GIT` wrapper that blocks `git status` in `slow`.
struct SlowGitStatusBlock {
    shim: PathBuf,
    arm: PathBuf,
    wait: PathBuf,
    release: PathBuf,
}

impl SlowGitStatusBlock {
    fn install(workspace: &Path) -> Self {
        let shim_dir = workspace.join(".e2e-git-shim");
        fs::create_dir_all(&shim_dir).unwrap();
        let shim = shim_dir.join("git");
        let arm = shim_dir.join("arm");
        let wait = shim_dir.join("wait");
        let release = shim_dir.join("release");
        let real_git = std::env::var("WS_E2E_REAL_GIT").unwrap_or_else(|_| {
            if Path::new("/usr/bin/git").is_file() {
                "/usr/bin/git".into()
            } else {
                "git".into()
            }
        });
        let slow = workspace.join("slow");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\n\
                 real=\"{real_git}\"\n\
                 arm=\"{arm}\"\n\
                 waitf=\"{wait}\"\n\
                 rel=\"{release}\"\n\
                 slow=\"{slow}\"\n\
                 is_status=0\n\
                 for a in \"$@\"; do\n\
                   case \"$a\" in\n\
                     status) is_status=1; break ;;\n\
                   esac\n\
                 done\n\
                 if [ \"$is_status\" = 1 ] && [ -f \"$arm\" ]; then\n\
                   case \"$PWD\" in\n\
                     \"$slow\"|\"$slow\"/*)\n\
                       : > \"$waitf\"\n\
                       while [ ! -f \"$rel\" ]; do\n\
                         sleep 0.05\n\
                       done\n\
                       ;;\n\
                   esac\n\
                 fi\n\
                 exec \"$real\" \"$@\"\n",
                real_git = real_git,
                arm = arm.display(),
                wait = wait.display(),
                release = release.display(),
                slow = slow.display(),
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&shim).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim, perms).unwrap();
        Self {
            shim,
            arm,
            wait,
            release,
        }
    }

    fn arm(&self) {
        fs::write(&self.arm, "1\n").unwrap();
    }

    fn release(&self) {
        fs::write(&self.release, "1\n").unwrap();
    }

    fn still_blocked(&self) -> bool {
        self.wait.exists() && !self.release.exists()
    }
}

/// Focused `fast` after first paint / click: graph pane, no marker yet.
///
/// Chrome/status rows are excluded. `slow`'s graph subject cannot pass.
fn focused_fast_clean_pane(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    tree_has(screen, "fast")
        && tree_has(screen, "slow")
        && tree_cursor_on(screen, "fast")
        && right.contains("Working tree clean")
        && right.contains(FAST_GRAPH_SUBJECT)
        && !right.contains("Uncommitted changes")
        && !right.contains(SLOW_GRAPH_SUBJECT)
        && !right.contains("# slow")
        && !left.contains("streamed-e2e-")
        && !right.trim().is_empty()
}

/// Focused `fast` tree row + right pane after the marker lands.
///
/// Fail if only chrome ticks, if the pane stays clean/blank, or if `slow`
/// is the painted body. Marker must be on the tree; pane must show the
/// dirty graph or the new file.
fn focused_fast_updated_before_slow(screen: &str, marker: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    let pane_body = right.contains("Uncommitted changes")
        && (right.contains(FAST_GRAPH_SUBJECT)
            || right.contains(marker)
            || right.contains(STREAMED_MARKER_BODY));
    let pane_diff = right.contains(marker) || right.contains(STREAMED_MARKER_BODY);
    tree_has(screen, "fast")
        && tree_has(screen, "slow")
        && left.contains(marker)
        && !right.trim().is_empty()
        && (pane_body || pane_diff)
        && !right.contains("Working tree clean")
        && !right.contains(SLOW_GRAPH_SUBJECT)
        && !right.contains("# slow")
}

/// Streamed watch collect paints the focused checkout before a blocked peer.
///
/// Docs (`tui-rust.md`, `app.rs`): each
/// checkout result applies as it finishes. Unfinished paths stay on the
/// previous generation. The focused checkout is queued first; its pane
/// reloads as soon as that identity changes (`focused_repo_needs_pane`).
/// A slow `git status` must not hold the focused tree or pane. `r` and
/// watch-while-keys are out of scope.
///
/// Setup: `stream_workspace` (`fast` clean, `slow` dirty). Watch on via
/// [`PtySession::open_with_env`]. A `WORKSPACE_STATUS_GIT` wrapper blocks
/// `git status` in `slow` after ARM. First paint, click `fast`, write a
/// unique untracked file, arm, then wait until slow is blocked. The
/// focused tree row and right pane must update while the wrapper still
/// holds. Fail if nothing happens, if only chrome/status ticks, if the
/// focused body stays blank, if `slow` is the pane, or if the update
/// waits for slow to unblock.
#[test]
fn pty_streamed_collect_updates_focused_repo_before_slow() {
    let (_root, workspace) = stream_workspace();
    let marker = format!(
        "streamed-e2e-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let block = SlowGitStatusBlock::install(&workspace);
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_WATCH_MS", "3000"),
            (
                "WORKSPACE_STATUS_GIT",
                block.shim.to_str().expect("utf-8 shim path"),
            ),
        ],
    );
    tui.wait_pred(
        |screen| tree_has(screen, "fast") && tree_has(screen, "slow"),
        "first paint: fast and slow tree rows",
        WAIT,
    );
    let fast_row = tree_row_containing(&tui.screen(), "fast")
        .unwrap_or_else(|| panic!("fast tree row after first paint; screen:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, fast_row);
    tui.wait_pred(
        focused_fast_clean_pane,
        "click fast: tree cursor + Working tree clean pane (seed fast), no marker",
        GIT_WAIT,
    );

    fs::write(workspace.join("fast").join(&marker), STREAMED_MARKER_BODY).unwrap();
    block.arm();

    let start = Instant::now();
    while !block.wait.exists() {
        if start.elapsed() >= WAIT {
            panic!(
                "timeout waiting for slow git status to block; screen:\n{}",
                tui.screen()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        block.still_blocked(),
        "slow repo must still be blocked when waiting for fast; screen:\n{}",
        tui.screen()
    );

    let marker_ref = marker.as_str();
    tui.wait_pred(
        |screen| block.still_blocked() && focused_fast_updated_before_slow(screen, marker_ref),
        "focused fast tree + pane update while slow git status is still blocked",
        Duration::from_secs(8),
    );
    assert!(
        block.still_blocked(),
        "fast tree/pane must update before slow git status is released; screen:\n{}",
        tui.screen()
    );
    block.release();
}

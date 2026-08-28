//! Real-TTY e2e for the ratatui TUI.
//!
//! Spawns the `workspace-status` binary on a PTY so the live loop's
//! `event::read` sees keys and xterm SGR mouse bytes. This is not the
//! TestBackend suite (`tui_headless_e2e.rs`) and not screenshot capture
//! (`scripts/capture-demo-stills.sh`).
//!
//! Unix only (PTY). Windows `cargo test --workspace` compiles this crate
//! with no tests.

#[cfg(unix)]
#[path = "../common/mod.rs"]
mod common;
#[cfg(unix)]
mod desktop;
#[cfg(unix)]
mod harness;
#[cfg(unix)]
mod operator;
#[cfg(unix)]
mod seed;

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use harness::{
    assert_contains, assert_tree_clipped_long_path, left_tree, tree_is_panned_to_tail,
    tree_row_containing, PtySession, SGR_WHEEL_RIGHT, SGR_WHEEL_RIGHT_MOTION,
};
#[cfg(unix)]
use seed::{
    ahead_workspace, behind_workspace, daily_workspace, focus_workspace, seed_long_path_file,
    stream_workspace, unfetched_behind_workspace,
};

#[cfg(unix)]
const WAIT: Duration = Duration::from_secs(12);
#[cfg(unix)]
const GIT_WAIT: Duration = Duration::from_secs(20);
#[cfg(unix)]
const SETTLE_MS: u64 = 200;

#[cfg(unix)]
fn tree_line_containing(screen: &str, needle: &str) -> Option<String> {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(needle))
        .map(str::to_string)
}

#[cfg(unix)]
fn readme_row_reviewed(screen: &str) -> bool {
    tree_line_containing(screen, "README.md").is_some_and(|line| line.contains('*'))
}

#[cfg(unix)]
fn op_finished(screen: &str, verb: &str) -> bool {
    screen.contains(&format!("{verb} 1 repo")) && !screen.contains("failed")
}

#[cfg(unix)]
fn tree_cleared_ahead_behind(screen: &str) -> bool {
    let left = left_tree(screen);
    !left.contains("v1") && !left.contains("^1")
}

#[cfg(unix)]
fn tree_has(screen: &str, needle: &str) -> bool {
    left_tree(screen).contains(needle)
}

/// Left-tree cursor bar (`▌`) on the row that contains `needle`.
fn tree_cursor_on(screen: &str, needle: &str) -> bool {
    tree_line_containing(screen, needle).is_some_and(|line| line.contains('\u{258C}'))
}

/// Breadcrumb is the workspace basename only (file-focused; no repo crumb).
fn launch_breadcrumb_workspace_only(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(crumb) = lines.get(lines.len().saturating_sub(2)) else {
        return false;
    };
    crumb.trim() == "workspace"
}

/// Idle status: directory-tree + preferred split pills, help, file hints.
fn launch_status_chrome(screen: &str) -> bool {
    let Some(status) = screen.lines().last() else {
        return false;
    };
    status.contains(" tree")
        && status.contains(" split")
        && status.contains("? help")
        && status.contains("focus right")
        && status.contains("stage")
        && status.contains("revert")
        && status.contains("fetch")
        && status.contains("edit")
        && status.contains("reviewed")
        && !status.contains("drill")
        && !status.contains("SEARCH")
        && !status.contains("Flat paths")
}

/// Left tree focused, right diff unfocused (title padding).
fn launch_panes_left_tree_right_diff(screen: &str) -> bool {
    let Some(top) = screen.lines().next() else {
        return false;
    };
    top.contains(" tree ") && top.contains(" diff") && !top.contains(" diff ")
}

/// Documented first paint on the daily seed. A blank, graph-first, ignored-
/// shown, unfolded No-updates, or paint-changed-only frame cannot pass.
fn documented_launch_first_paint(screen: &str) -> bool {
    let left = left_tree(screen);
    let readme = tree_line_containing(screen, "README.md");
    let no_updates = tree_line_containing(screen, "No updates");
    launch_panes_left_tree_right_diff(screen)
        && left.contains("# workspace")
        && left.contains("1 changed · all current")
        && tree_has(screen, "app")
        && tree_has(screen, "& main")
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && tree_has(screen, "feature/graph")
        && tree_has(screen, "No updates")
        && readme.is_some_and(|line| line.contains('M'))
        && no_updates.is_some_and(|line| line.contains('>') && line.contains('1'))
        && !tree_has(screen, "lib")
        && !screen.contains("notes")
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && screen.contains("app/README.md  inline (too narrow)")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("@@ -1 +1,2 @@")
        && launch_breadcrumb_workspace_only(screen)
        && launch_status_chrome(screen)
        && !screen.contains("[workspace]")
        && !screen.contains("workspace ›")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("Working tree")
        && !screen.contains("focus a repo for the graph")
        && !screen.contains("No matching rows")
        && !screen.contains("loading")
}

/// Spawn paints the documented first chrome. No keys.
///
/// Docs: left tree focused, first file selected, file diff on the right,
/// ignored repos hidden, No updates folded, breadcrumb is the workspace
/// basename while the right pane is a diff. Right-pane git is a worker, so
/// a tree-only frame or a `+dirty` substring is not enough. A no-op, a
/// blank screen, a graph-first launch, or a paint-changed-only assert
/// cannot pass.
#[cfg(unix)]
#[test]
fn pty_launch_paints_tree_diff_and_chrome() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "documented first paint: focused tree, README cursor, file diff, breadcrumb, status, seed rows",
        WAIT,
    );
}

#[cfg(unix)]
#[test]
fn pty_help_overlay() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);

    tui.key('?');
    tui.wait_contains("MOVE", WAIT);
    tui.wait_contains("GIT", WAIT);
    tui.wait_contains("VIEW", WAIT);
    tui.wait_contains(workspace_status::APP_VERSION, WAIT);
    tui.esc();
    tui.wait_absent("MOVE", WAIT);
}

#[cfg(unix)]
#[test]
fn pty_graph_drill_enter_esc() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("merger");
    tui.wait_contains("merger", WAIT);
    tui.wait_contains("WIP on graph", WAIT);

    tui.tab();
    tui.wait_contains("Working tree", WAIT);
    tui.key('j');
    tui.wait_ms(150);
    tui.enter();
    tui.wait_contains("wip.txt", WAIT);
    tui.enter();
    tui.wait_contains_any(&["@@", "NEW", "UNSTAGED", "wip"], WAIT);
    tui.esc();
    tui.esc();
    tui.wait_contains("WIP on graph", WAIT);
}

#[cfg(unix)]
#[test]
fn pty_graph_branch_focus_overlay() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("focusbox", WAIT);
    tui.tab();
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_contains("noise-leaf-commit", WAIT);

    tui.key('o');
    tui.wait_contains("Focus branches", WAIT);
    tui.wait_contains("feature/keep", WAIT);
    tui.keys("keep");
    tui.enter();
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_absent("noise-leaf-commit", WAIT);

    tui.key('O');
    tui.wait_contains("noise-leaf-commit", WAIT);
}

/// Shift+O via CSI-u (unshifted codepoint + SHIFT), not a raw `'O'` byte.
///
/// The live loop requests press/release on letters. Matching only `Char('O')`
/// misses `Char('o')` + SHIFT, so this must fail if clear-focus regresses.
#[cfg(unix)]
#[test]
fn pty_shift_o_csi_u_clears_graph_branch_focus() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("focusbox", WAIT);
    tui.tab();
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_contains("noise-leaf-commit", WAIT);

    tui.key('o');
    tui.wait_contains("Focus branches", WAIT);
    tui.keys("keep");
    tui.enter();
    tui.wait_absent("Focus branches", WAIT);
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_absent("noise-leaf-commit", WAIT);

    tui.shift_letter('O');
    tui.wait_absent("Focus branches", WAIT);
    tui.wait_contains("noise-leaf-commit", WAIT);
    tui.wait_contains("main-leaf-commit", WAIT);
}

/// `/` then Shift+letters as CSI-u must type into the SEARCH prompt.
#[cfg(unix)]
#[test]
fn pty_shift_letters_csi_u_type_into_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.key('/');
    tui.wait_contains("SEARCH", WAIT);
    tui.shift_keys("MERGER");
    tui.wait_contains("MERGER▏", WAIT);
    tui.enter();
    tui.wait_contains("/MERGER", WAIT);
    tui.wait_contains("WIP on graph", WAIT);
}

/// Shift+S via CSI-u opens the stash overlay (not `s` stage).
#[cfg(unix)]
#[test]
fn pty_shift_s_csi_u_opens_stash_menu() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("README.md", WAIT);
    tui.shift_letter('S');
    tui.wait_contains("Stash ", WAIT);
    tui.wait_contains("stash", WAIT);
}

/// Unmark every `[x]` then Enter restores `--all` (does not keep the cursor row).
#[cfg(unix)]
#[test]
fn pty_graph_focus_unmark_enter_clears() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("focusbox", WAIT);
    tui.tab();
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_contains("noise-leaf-commit", WAIT);

    tui.key('o');
    tui.wait_contains("Focus branches", WAIT);
    tui.keys("keep");
    tui.enter();
    tui.wait_absent("Focus branches", WAIT);
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_absent("noise-leaf-commit", WAIT);

    tui.key('o');
    tui.wait_contains("Focus branches", WAIT);
    tui.wait_contains("[x]", WAIT);
    tui.key(' ');
    tui.wait_absent("[x]", WAIT);
    tui.enter();
    tui.wait_absent("Focus branches", WAIT);
    tui.wait_contains("noise-leaf-commit", WAIT);
    tui.wait_contains("main-leaf-commit", WAIT);
    tui.wait_contains("keep-leaf-commit", WAIT);
}

#[cfg(unix)]
#[test]
fn pty_ctrl_c_prompts_before_quit() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.ctrl('c');
    tui.wait_contains("Press Ctrl+C again to exit", WAIT);
    tui.key('q');
}

/// Tree hscroll via live `event::read`. Must fail if the tree does not pan.
///
/// Same clipped-prefix vs tail oracle as headless
/// `tree_trackpad_sgr_hscroll_pans_without_stealing_focus` (`common::hscroll`).
/// No `/` search: that would put the tail on the status chip before any wheel.
#[cfg(unix)]
#[test]
fn pty_tree_sgr_hscroll_pans_clipped_path() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    // Start at the clipped size. A later resize can paint a frame where
    // the prefix was seen, then the next snapshot has no wheel target.
    let mut tui = PtySession::open_size(&workspace, 64, 24);
    let _ = tui.wait_clipped_long_path_row(WAIT);

    // Same setup as the headless TestBackend case: a short README diff so
    // hscroll over the tree pans the tree, not a long file-diff.
    if let Some(readme_row) = tree_row_containing(&tui.screen(), "README.md") {
        tui.sgr_click(6, readme_row);
    }
    let row = tui.wait_clipped_long_path_row(WAIT);
    let col = 6u16;

    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT_MOTION, col, row);
    }
    tui.wait_ms(80);
    assert_tree_clipped_long_path(&tui.screen());

    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, col, row);
    }
    tui.wait_pred(
        tree_is_panned_to_tail,
        "tree row shows the hscroll tail and drops the clipped prefix",
        WAIT,
    );
    crate::common::hscroll::assert_panned_to_tail(&left_tree(&tui.screen()));
}

/// Continuous harmless input must not starve `WatchTick`.
///
/// The old loop drained keys and `continue`d, so watch timers only ran on
/// poll timeout. This writes a new file and spam-sends `1` (unbound) until
/// the name appears — no `r`.
#[cfg(unix)]
#[test]
fn pty_watch_applies_while_keys_arrive() {
    let (_root, workspace) = daily_workspace();
    let marker = format!(
        "watch-live-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_contains("app", WAIT);
    fs::write(workspace.join("app").join(&marker), "live-watch\n").unwrap();
    tui.wait_contains_while(&marker, WAIT, |session| session.key('1'));
    let screen = tui.screen();
    assert!(
        !screen.contains("refreshed app") && !screen.contains("refreshed workspace"),
        "must not send r; screen:\n{screen}"
    );
}

/// Per-repo apply must not wait for a slow checkout, including the pane.
///
/// A `WORKSPACE_STATUS_GIT` shim blocks `git status` in `slow` after ARM.
/// `fast` is focused and modified; its tree + pane must update while `slow`
/// is still blocked.
#[cfg(unix)]
#[test]
fn pty_streamed_collect_updates_focused_repo_before_slow() {
    let (_root, workspace) = stream_workspace();
    let marker = format!(
        "fast-live-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let shim_dir = workspace.join(".e2e-git-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let shim = shim_dir.join("git");
    let arm = shim_dir.join("arm");
    let wait = shim_dir.join("wait");
    let release = shim_dir.join("release");
    let real_git = std::env::var("WS_E2E_REAL_GIT").unwrap_or_else(|_| {
        if std::path::Path::new("/usr/bin/git").is_file() {
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

    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_WATCH_MS", "500"),
            ("WORKSPACE_STATUS_GIT", shim.to_str().unwrap()),
        ],
    );
    tui.search("fast");
    tui.wait_contains("fast", WAIT);
    tui.wait_contains("Working tree clean", WAIT);
    fs::write(workspace.join("fast").join(&marker), "stream-me\n").unwrap();
    fs::write(&arm, "1\n").unwrap();

    let start = Instant::now();
    while !wait.exists() {
        if start.elapsed() >= WAIT {
            panic!(
                "timeout waiting for slow git status to block; screen:\n{}",
                tui.screen()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        !release.exists(),
        "slow repo must still be blocked when asserting fast"
    );
    tui.wait_contains(&marker, Duration::from_secs(8));
    tui.wait_contains("Uncommitted changes", Duration::from_secs(8));
    assert!(
        wait.exists() && !release.exists(),
        "fast tree/pane must update before slow git status is released"
    );
    fs::write(&release, "1\n").unwrap();
}

/// Space on a dirty file paints the ASCII reviewed mark (`*`) before the badge.
#[cfg(unix)]
#[test]
fn pty_space_marks_dirty_file_reviewed() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| !readme_row_reviewed(screen),
        "README row has no reviewed mark yet",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.key(' ');
    tui.wait_pred(
        readme_row_reviewed,
        "README row shows ASCII reviewed mark `*`",
        WAIT,
    );
    tui.key(' ');
    tui.wait_pred(
        |screen| !readme_row_reviewed(screen),
        "second space clears the reviewed mark",
        WAIT,
    );
}

/// `s` stages the focused dirty file; `u` unstages. Diff labels must flip.
#[cfg(unix)]
#[test]
fn pty_stage_and_unstage_dirty_file() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_contains("stage", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('s');
    tui.wait_contains("STAGED", GIT_WAIT);
    tui.wait_pred(
        |screen| tree_line_containing(screen, "README.md").is_some_and(|line| line.contains("S ")),
        "README badge is staged `S `",
        WAIT,
    );
    tui.wait_absent("UNSTAGED", WAIT);

    tui.key('u');
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        |screen| tree_line_containing(screen, "README.md").is_some_and(|line| line.contains("M ")),
        "README badge is modified `M ` after unstage",
        WAIT,
    );
}

/// `f` against a local bare origin. Must fail if fetch is a no-op.
#[cfg(unix)]
#[test]
fn pty_fetch_local_remote_marks_behind() {
    let (_root, workspace) = unfetched_behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("syncbox");
    tui.wait_contains("/syncbox", WAIT);
    tui.wait_contains("Working tree", WAIT);
    tui.wait_contains("fetch", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('f');
    tui.wait_pred(
        |screen| op_finished(screen, "Fetched") || left_tree(screen).contains("v1"),
        "Fetched 1 repo or tree shows behind-by-1",
        GIT_WAIT,
    );
    tui.wait_pred(
        |screen| left_tree(screen).contains("v1"),
        "tree shows behind-by-1 after fetch",
        WAIT,
    );
    tui.wait_contains("origin-tip-commit", WAIT);
}

/// `p` on a behind checkout. Must fail if pull is a no-op.
#[cfg(unix)]
#[test]
fn pty_pull_behind_local_remote() {
    let (_root, workspace) = behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("syncbox");
    tui.wait_contains("/syncbox", WAIT);
    tui.wait_contains("origin-tip-commit", WAIT);
    tui.wait_pred(
        |screen| left_tree(screen).contains("v1"),
        "tree shows behind-by-1 before pull",
        WAIT,
    );
    tui.wait_contains("pull", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('p');
    tui.wait_pred(
        |screen| op_finished(screen, "Pulled"),
        "Pulled 1 repo without failure",
        GIT_WAIT,
    );
    tui.wait_pred(
        tree_cleared_ahead_behind,
        "behind mark cleared after pull",
        WAIT,
    );
    tui.tab();
    tui.wait_contains("origin-tip-commit", WAIT);
}

/// Shift+P via CSI-u pushes an ahead checkout to the local origin.
#[cfg(unix)]
#[test]
fn pty_shift_p_csi_u_pushes_ahead() {
    let (_root, workspace) = ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("syncbox");
    tui.wait_contains("/syncbox", WAIT);
    tui.wait_contains("ahead-tip-commit", WAIT);
    tui.wait_pred(
        |screen| left_tree(screen).contains("^1"),
        "tree shows ahead-by-1 before push",
        WAIT,
    );
    tui.wait_contains("push", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.shift_letter('P');
    tui.wait_pred(
        |screen| op_finished(screen, "Pushed"),
        "Pushed 1 repo without failure",
        GIT_WAIT,
    );
    tui.wait_pred(
        tree_cleared_ahead_behind,
        "ahead mark cleared after push",
        WAIT,
    );
}

/// Graph `m` is the TUI write that creates a commit (merge into HEAD).
#[cfg(unix)]
#[test]
fn pty_graph_merge_creates_commit() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("/focusbox", WAIT);
    tui.tab();
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_contains("main-leaf-commit", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('/');
    tui.keys("main-leaf-commit");
    tui.enter();
    tui.wait_contains("/main-leaf-commit", WAIT);
    tui.wait_contains("merge", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('m');
    tui.wait_contains("Merge", WAIT);
    tui.wait_contains("into", WAIT);
    tui.key('y');
    tui.wait_contains("Merge branch 'main'", GIT_WAIT);
}

/// Create (`S` then `s`), apply (`a`), drop (`D` then `y`) — not menu-open only.
#[cfg(unix)]
#[test]
fn pty_stash_create_apply_and_drop() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.shift_letter('S');
    tui.wait_contains("s create", WAIT);
    tui.key('s');
    tui.wait_contains("Stashed", GIT_WAIT);
    tui.wait_pred(
        |screen| !left_tree(screen).contains("README.md"),
        "stashed README leaves the dirty tree",
        WAIT,
    );

    tui.esc();
    tui.search("app");
    tui.wait_contains("/app", WAIT);
    tui.tab();
    tui.wait_contains("Working tree", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('/');
    tui.keys("stash@{");
    tui.enter();
    tui.wait_contains("/stash@{", WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('a');
    tui.wait_contains("README.md", GIT_WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.shift_letter('D');
    tui.wait_contains("Drop", WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.key('y');
    tui.wait_contains("dropped stash@{0}", GIT_WAIT);
    tui.wait_contains("README.md", WAIT);
}

/// Graph `p` pops the focused stash (apply + drop).
#[cfg(unix)]
#[test]
fn pty_stash_graph_pop() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("merger");
    tui.wait_contains("/merger", WAIT);
    tui.tab();
    tui.wait_contains("WIP on graph", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('/');
    tui.keys("stash@{");
    tui.enter();
    tui.wait_contains("/stash@{", WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('p');
    tui.wait_contains("wip.txt", GIT_WAIT);
    tui.wait_contains("popped stash@{0}", GIT_WAIT);
}

#[cfg(target_os = "linux")]
mod xfce {
    use super::*;
    use crate::desktop::DesktopSession;
    use crate::harness::{assert_tree_clipped_long_path, left_tree, tree_row_containing};
    use crate::seed::seed_long_path_file;

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_keys_help_and_search() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("shift+slash");
        tui.wait_contains("MOVE", WAIT);
        tui.wait_contains("GIT", WAIT);
        tui.wait_contains("VIEW", WAIT);
        tui.key("Escape");
        tui.key("slash");
        tui.type_text("merger");
        tui.key("Return");
        tui.wait_contains("merger", WAIT);
        tui.wait_contains("WIP on graph", WAIT);
    }

    /// xfce + XTEST Shift keys: search capitals, unmark-then-Enter, Shift+O.
    ///
    /// Overlay toggle is space (`[x]` / `[ ]`), not X. Reopen after `O`
    /// has no pre-mark; unmark-then-Enter runs while a focus is still on.
    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_shift_keys_search_and_clear_focus() {
        let (_root, workspace) = focus_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.key("shift+r");
        tui.key("shift+e");
        tui.key("shift+a");
        tui.key("shift+d");
        tui.key("shift+m");
        tui.key("shift+e");
        tui.wait_contains("README▏", WAIT);
        tui.key("Escape");

        tui.key("slash");
        tui.type_text("focusbox");
        tui.key("Return");
        tui.wait_contains("focusbox", WAIT);
        tui.key("Tab");
        tui.wait_contains("keep-leaf-commit", WAIT);
        tui.wait_contains("noise-leaf-commit", WAIT);
        tui.key("o");
        tui.wait_contains("Focus branches", WAIT);
        tui.type_text("keep");
        tui.key("Return");
        tui.wait_pred(
            |screen| !screen.contains("Focus branches"),
            "focus overlay closed after apply",
            WAIT,
        );
        tui.wait_contains("keep-leaf-commit", WAIT);
        tui.wait_pred(
            |screen| !screen.contains("noise-leaf-commit"),
            "noise-leaf-commit hidden after focus",
            WAIT,
        );

        tui.key("o");
        tui.wait_contains("Focus branches", WAIT);
        tui.wait_contains("[x]", WAIT);
        tui.key("space");
        tui.wait_pred(
            |screen| !screen.contains("[x]"),
            "[x] mark cleared after space",
            WAIT,
        );
        tui.key("Return");
        tui.wait_pred(
            |screen| !screen.contains("Focus branches"),
            "focus overlay closed after empty apply",
            WAIT,
        );
        tui.wait_contains("noise-leaf-commit", WAIT);
        tui.wait_contains("main-leaf-commit", WAIT);

        tui.key("o");
        tui.wait_contains("Focus branches", WAIT);
        tui.type_text("keep");
        tui.key("Return");
        tui.wait_pred(
            |screen| !screen.contains("noise-leaf-commit"),
            "noise-leaf-commit hidden before Shift+O",
            WAIT,
        );
        tui.key("shift+o");
        tui.wait_contains("noise-leaf-commit", WAIT);
    }

    /// XTEST wheel right. Must fail if the tree does not pan.
    ///
    /// XTEST `click 7` (no `--window`) after a root-coordinate warp, in
    /// xterm (VTE 0.76 does not report buttons 6/7). Same clipped-prefix vs
    /// tail tree-row oracle as the PTY case (`common::hscroll`). No `/` search.
    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; xterm encodes XTEST button 7"]
    fn desktop_xterm_xtest_trackpad_hscroll() {
        let (_root, workspace) = daily_workspace();
        seed_long_path_file(&workspace);
        let tui = DesktopSession::open_xterm_size(&workspace, 64, 24);
        tui.wait_pred(
            |screen| crate::common::hscroll::is_clipped(&left_tree(screen)),
            "clipped long path prefix on the tree row (no tail)",
            WAIT,
        );
        assert_tree_clipped_long_path(&tui.screen());

        if let Some(readme_row) = tree_row_containing(&tui.screen(), "README.md") {
            tui.click_cell(6, readme_row);
        }
        tui.wait_pred(
            |screen| crate::harness::clipped_long_path_row(screen).is_some(),
            "clipped long path on a tree row after focusing README",
            WAIT,
        );
        let row = crate::harness::clipped_long_path_row(&tui.screen())
            .unwrap_or_else(|| panic!("tree row with clipped long path:\n{}", tui.screen()));
        tui.wheel_right_at_cell(6, row, 40);
        tui.wait_pred(
            tree_is_panned_to_tail,
            "tree row shows the hscroll tail and drops the clipped prefix",
            WAIT,
        );
        crate::common::hscroll::assert_panned_to_tail(&left_tree(&tui.screen()));
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_review_and_stage() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("README");
        tui.key("Return");
        tui.wait_contains("/README", WAIT);
        tui.wait_contains("UNSTAGED", WAIT);
        tui.wait_pred(
            |screen| !readme_row_reviewed(screen),
            "README row has no reviewed mark yet",
            WAIT,
        );
        tui.wait_ms(SETTLE_MS);
        tui.key("space");
        tui.wait_pred(
            readme_row_reviewed,
            "README row shows ASCII reviewed mark `*`",
            WAIT,
        );
        tui.key("s");
        tui.wait_contains("STAGED", GIT_WAIT);
        tui.wait_absent("UNSTAGED", WAIT);
        tui.wait_pred(
            |screen| {
                tree_line_containing(screen, "README.md").is_some_and(|line| line.contains("S "))
            },
            "staged README badge `S `",
            WAIT,
        );
        tui.key("u");
        tui.wait_contains("UNSTAGED", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_fetch_then_pull_local_remote() {
        let (_root, workspace) = unfetched_behind_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("syncbox");
        tui.key("Return");
        tui.wait_contains("/syncbox", WAIT);
        tui.wait_contains("Working tree", WAIT);
        tui.wait_contains("fetch", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("f");
        tui.wait_pred(
            |screen| op_finished(screen, "Fetched") || left_tree(screen).contains("v1"),
            "Fetched 1 repo or tree shows behind-by-1",
            GIT_WAIT,
        );
        tui.wait_pred(
            |screen| left_tree(screen).contains("v1"),
            "tree shows behind-by-1 after fetch",
            WAIT,
        );
        tui.wait_contains("origin-tip-commit", WAIT);
        tui.wait_contains("pull", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("p");
        tui.wait_pred(
            |screen| op_finished(screen, "Pulled"),
            "Pulled 1 repo without failure",
            GIT_WAIT,
        );
        tui.wait_pred(
            tree_cleared_ahead_behind,
            "behind mark cleared after pull",
            WAIT,
        );
        tui.key("Tab");
        tui.wait_contains("origin-tip-commit", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_shift_p_pushes_ahead() {
        let (_root, workspace) = ahead_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("syncbox");
        tui.key("Return");
        tui.wait_contains("/syncbox", WAIT);
        tui.wait_contains("ahead-tip-commit", WAIT);
        tui.wait_pred(
            |screen| left_tree(screen).contains("^1"),
            "tree shows ahead-by-1 before push",
            WAIT,
        );
        tui.wait_contains("push", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("shift+p");
        tui.wait_pred(
            |screen| op_finished(screen, "Pushed"),
            "Pushed 1 repo without failure",
            GIT_WAIT,
        );
        tui.wait_pred(
            tree_cleared_ahead_behind,
            "ahead mark cleared after push",
            WAIT,
        );
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_graph_merge_creates_commit() {
        let (_root, workspace) = focus_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("focusbox");
        tui.key("Return");
        tui.wait_contains("/focusbox", WAIT);
        tui.key("Tab");
        tui.wait_contains("keep-leaf-commit", WAIT);
        tui.wait_contains("main-leaf-commit", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("slash");
        tui.type_text("main-leaf-commit");
        tui.key("Return");
        tui.wait_contains("/main-leaf-commit", WAIT);
        tui.wait_contains("merge", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("m");
        tui.wait_contains("Merge", WAIT);
        tui.wait_contains("into", WAIT);
        tui.key("y");
        tui.wait_contains("Merge branch 'main'", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_stash_create_apply_and_drop() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("README");
        tui.key("Return");
        tui.wait_contains("/README", WAIT);
        tui.wait_contains("UNSTAGED", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("shift+s");
        tui.wait_contains("s create", WAIT);
        tui.key("s");
        tui.wait_contains("Stashed", GIT_WAIT);
        tui.wait_pred(
            |screen| !left_tree(screen).contains("README.md"),
            "stashed README leaves the dirty tree",
            WAIT,
        );
        tui.key("Escape");
        tui.key("slash");
        tui.type_text("app");
        tui.key("Return");
        tui.wait_contains("/app", WAIT);
        tui.key("Tab");
        tui.wait_contains("Working tree", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("slash");
        tui.type_text("stash@{");
        tui.key("Return");
        tui.wait_contains("/stash@{", WAIT);
        tui.wait_contains("stash@{0}", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("a");
        tui.wait_contains("README.md", GIT_WAIT);
        tui.wait_contains("stash@{0}", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("shift+d");
        tui.wait_contains("Drop", WAIT);
        tui.key("y");
        tui.wait_contains("dropped stash@{0}", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_stash_graph_pop() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("merger");
        tui.key("Return");
        tui.wait_contains("/merger", WAIT);
        tui.key("Tab");
        tui.wait_contains("WIP on graph", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("slash");
        tui.type_text("stash@{");
        tui.key("Return");
        tui.wait_contains("/stash@{", WAIT);
        tui.wait_contains("stash@{0}", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("p");
        tui.wait_contains("wip.txt", GIT_WAIT);
        tui.wait_contains("popped stash@{0}", GIT_WAIT);
    }
}

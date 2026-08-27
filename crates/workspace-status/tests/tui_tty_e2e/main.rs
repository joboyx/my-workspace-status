//! Real-TTY e2e for the ratatui TUI.
//!
//! Spawns the `workspace-status` binary on a PTY so the live loop's
//! `event::read` sees keys and xterm SGR mouse bytes. This is not the
//! TestBackend suite (`tui_daily_e2e.rs`) and not screenshot capture
//! (`scripts/capture-demo-stills.sh`).
//!
//! Unix only (PTY). Windows `cargo test --workspace` compiles this crate
//! with no tests.

#[cfg(unix)]
mod desktop;
#[cfg(unix)]
mod harness;
#[cfg(unix)]
mod seed;

#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use harness::{
    assert_absent, assert_contains, PtySession, SGR_SHIFT_WHEEL_DOWN, SGR_WHEEL_DOWN,
    SGR_WHEEL_RIGHT, SGR_WHEEL_RIGHT_MOTION,
};
#[cfg(unix)]
use seed::{
    daily_workspace, focus_workspace, seed_long_diff_file, seed_long_path_file,
    seed_tall_dirty_file,
};

#[cfg(unix)]
const WAIT: Duration = Duration::from_secs(12);

#[cfg(unix)]
#[test]
fn pty_launch_paints_tree_diff_and_chrome() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    let screen = tui.screen();
    assert_contains(&screen, "app");
    assert_contains(&screen, "README.md");
    assert_contains(&screen, " tree");
    assert_contains(&screen, "+dirty");
    assert_absent(&screen, "notes");
}

#[cfg(unix)]
#[test]
fn pty_nav_tab_help_search_gg_g() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);

    tui.key('?');
    tui.wait_contains("MOVE", WAIT);
    tui.wait_contains("GIT", WAIT);
    tui.wait_contains("VIEW", WAIT);
    tui.wait_contains(workspace_status::APP_VERSION, WAIT);
    tui.esc();
    tui.wait_absent("MOVE", WAIT);

    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_contains("README.md", WAIT);

    tui.gg();
    tui.wait_ms(80);
    tui.key('G');
    tui.wait_contains("No updates", WAIT);

    tui.tab();
    tui.wait_ms(120);
    tui.tab();
    let screen = tui.screen();
    assert_contains(&screen, "app");
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

#[cfg(unix)]
#[test]
fn pty_ignored_stash_confirm_reviewed_fold() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);

    tui.key('.');
    tui.wait_contains("notes", WAIT);

    tui.search("app");
    tui.wait_contains("app", WAIT);
    tui.key('S');
    tui.wait_contains("stash", WAIT);
    tui.esc();
    tui.wait_ms(80);

    tui.search("README");
    tui.wait_contains("README.md", WAIT);
    tui.key('x');
    tui.wait_contains_any(&["Discard", "revert", "y"], WAIT);
    tui.key('n');
    tui.wait_ms(80);

    tui.key(' ');
    tui.wait_ms(120);
    let reviewed = tui.screen();
    assert!(
        reviewed.contains('*') || reviewed.contains("reviewed"),
        "space should mark reviewed:\n{reviewed}"
    );

    tui.search("No updates");
    tui.wait_contains("No updates", WAIT);
    tui.key('z');
    tui.wait_contains("lib", WAIT);
}

#[cfg(unix)]
#[test]
fn pty_sgr_mouse_click_wheel_and_trackpad_hscroll() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open(&workspace);

    tui.resize(64, 24);
    tui.wait_ms(200);
    tui.key('t');
    tui.search("TAIL99");
    tui.wait_contains("TAIL99", Duration::from_secs(8));
    tui.wait_ms(80);
    let clipped = tui.screen();
    assert_absent(
        &clipped,
        "very-long-workspace-tree-component-name-TAIL99.ts",
    );

    let row = tui
        .row_containing("TAIL99")
        .or_else(|| tui.row_containing("workspace-tree"))
        .unwrap_or(4);
    let col = 6u16;
    let before = tui.screen();

    tui.sgr_mouse(SGR_WHEEL_RIGHT_MOTION, col, row);
    tui.wait_ms(80);
    assert_eq!(
        tui.screen(),
        before,
        "crossterm 0.28 drops SGR 99; the tree must not pan"
    );

    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, col, row);
    }
    tui.wait_contains("TAIL99", WAIT);
    let panned = tui.screen();
    assert_contains(&panned, "TAIL99");

    tui.sgr_mouse(SGR_SHIFT_WHEEL_DOWN, col, row);
    tui.wait_ms(80);

    tui.sgr_mouse(SGR_WHEEL_DOWN, col, row);
    tui.wait_ms(80);

    if let Some(app_row) = tui.row_containing("app") {
        tui.click(8, app_row);
        tui.wait_ms(120);
    }
    let screen = tui.screen();
    assert_contains(&screen, "app");
}

#[cfg(unix)]
#[test]
fn pty_diff_pan_resize_ctrl_ud() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace);
    seed_tall_dirty_file(&workspace, "tall.ts");
    let mut tui = PtySession::open(&workspace);

    tui.search("wide.ts");
    tui.wait_contains("wide.ts", WAIT);
    tui.wait_contains("TAIL42", Duration::from_secs(8));
    tui.tab();
    tui.wait_ms(100);
    for _ in 0..24 {
        tui.key('l');
    }
    tui.wait_ms(80);
    let panned = tui.screen();
    assert_contains(&panned, "wide.ts");

    tui.tab();
    tui.search("tall.ts");
    tui.wait_contains("tall.ts", WAIT);
    tui.tab();
    tui.gg();
    tui.wait_contains("tall line 0", WAIT);
    tui.key('G');
    tui.wait_contains("tall line 49", WAIT);
    tui.ctrl('u');
    tui.wait_ms(80);
    tui.ctrl('d');
    tui.wait_ms(80);

    tui.resize(80, 24);
    tui.wait_ms(200);
    let resized = tui.screen();
    assert_contains(&resized, "tall.ts");
}

#[cfg(unix)]
#[test]
fn pty_stage_unstage_and_quit_chords() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("README.md", WAIT);
    tui.key('s');
    tui.wait_contains_any(&["S", "staged", "stage"], WAIT);
    tui.key('u');
    tui.wait_ms(200);

    tui.ctrl('c');
    tui.wait_contains("Press Ctrl+C again to exit", WAIT);
    tui.key('q');
}

#[cfg(target_os = "linux")]
mod xfce {
    use super::*;
    use crate::desktop::DesktopSession;
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

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; VTE encodes trackpad hscroll"]
    fn desktop_xfce_vte_trackpad_hscroll() {
        let (_root, workspace) = daily_workspace();
        seed_long_path_file(&workspace);
        let tui = DesktopSession::open(&workspace);
        tui.key("t");
        tui.key("slash");
        tui.type_text("TAIL99");
        tui.key("Return");
        tui.wait_contains("TAIL99", WAIT);
        tui.wheel_right_tree(40);
        let screen = tui.screen();
        assert_contains(&screen, "TAIL99");
    }
}

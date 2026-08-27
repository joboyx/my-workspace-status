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
    assert_contains, assert_tree_clipped_long_path, left_tree, tree_is_panned_to_tail,
    tree_row_containing, PtySession, SGR_WHEEL_RIGHT, SGR_WHEEL_RIGHT_MOTION,
};
#[cfg(unix)]
use seed::{daily_workspace, focus_workspace, seed_long_path_file};

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
    harness::assert_absent(&screen, "notes");
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
/// Same clipped-prefix vs `TAIL99` oracle as headless
/// `tree_trackpad_sgr_hscroll_pans_without_stealing_focus`. No `/` search:
/// that would put `TAIL99` on the status chip before any wheel.
#[cfg(unix)]
#[test]
fn pty_tree_sgr_hscroll_pans_clipped_path() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.resize(64, 24);
    tui.wait_pred(
        |screen| left_tree(screen).contains("very-long") && !left_tree(screen).contains("TAIL99"),
        "clipped long path prefix on the tree row (no TAIL99)",
        WAIT,
    );
    assert_tree_clipped_long_path(&tui.screen());

    // Same setup as the daily TestBackend case: a short README diff so
    // hscroll over the tree pans the tree, not a long file-diff.
    if let Some(readme_row) = tree_row_containing(&tui.screen(), "README.md") {
        tui.sgr_click(6, readme_row);
        tui.wait_ms(120);
        assert_tree_clipped_long_path(&tui.screen());
    }

    let row =
        tree_row_containing(&tui.screen(), "very-long").expect("tree row with clipped long path");
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
        "tree row shows TAIL99 and drops the clipped prefix",
        WAIT,
    );
    let left = left_tree(&tui.screen());
    harness::assert_absent(&left, "very-long");
    assert_contains(&left, "TAIL99");
}

#[cfg(target_os = "linux")]
mod desktop {
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

    /// XTEST wheel right. Must fail if the tree does not pan.
    ///
    /// XTEST `click 7` (no `--window`) after a root-coordinate warp, in
    /// xterm (VTE 0.76 does not report buttons 6/7). Same clipped-prefix vs
    /// `TAIL99` tree-row oracle as the PTY case. No `/` search.
    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; xterm encodes XTEST button 7"]
    fn desktop_xterm_xtest_trackpad_hscroll() {
        let (_root, workspace) = daily_workspace();
        seed_long_path_file(&workspace);
        let tui = DesktopSession::open_xterm_size(&workspace, 64, 24);
        tui.wait_pred(
            |screen| {
                left_tree(screen).contains("very-long") && !left_tree(screen).contains("TAIL99")
            },
            "clipped long path prefix on the tree row (no TAIL99)",
            WAIT,
        );
        assert_tree_clipped_long_path(&tui.screen());

        if let Some(readme_row) = tree_row_containing(&tui.screen(), "README.md") {
            tui.click_cell(6, readme_row);
            tui.wait_pred(
                |screen| {
                    left_tree(screen).contains("very-long") && !left_tree(screen).contains("TAIL99")
                },
                "tree still clipped after focusing README",
                WAIT,
            );
        }

        let row = tree_row_containing(&tui.screen(), "very-long")
            .expect("tree row with clipped long path");
        tui.wheel_right_at_cell(6, row, 40);
        tui.wait_pred(
            tree_is_panned_to_tail,
            "tree row shows TAIL99 and drops the clipped prefix",
            WAIT,
        );
        let left = left_tree(&tui.screen());
        crate::harness::assert_absent(&left, "very-long");
        assert_contains(&left, "TAIL99");
    }
}

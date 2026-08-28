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
    tui.wait_contains("SEARCH MERGER", WAIT);
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
/// Same clipped-prefix vs `TAIL99` oracle as headless
/// `tree_trackpad_sgr_hscroll_pans_without_stealing_focus`. No `/` search:
/// that would put `TAIL99` on the status chip before any wheel.
#[cfg(unix)]
#[test]
fn pty_tree_sgr_hscroll_pans_clipped_path() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    // Start at the clipped size. A later resize can paint a frame where
    // the prefix was seen, then the next snapshot has no wheel target.
    let mut tui = PtySession::open_size(&workspace, 64, 24);
    let _ = tui.wait_clipped_long_path_row(WAIT);

    // Same setup as the daily TestBackend case: a short README diff so
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
        "tree row shows TAIL99 and drops the clipped prefix",
        WAIT,
    );
    let left = left_tree(&tui.screen());
    harness::assert_absent(&left, "very-long");
    assert_contains(&left, "TAIL99");
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

    /// xfce + XTEST Shift keys: search capitals, Shift+O clears graph focus.
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
        tui.wait_contains("SEARCH README", WAIT);
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
        tui.wait_contains("keep-leaf-commit", WAIT);
        tui.wait_pred(
            |screen| !screen.contains("noise-leaf-commit"),
            "noise-leaf-commit hidden after focus",
            WAIT,
        );
        tui.key("shift+o");
        tui.wait_contains("noise-leaf-commit", WAIT);

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
        tui.wait_contains("noise-leaf-commit", WAIT);
        tui.wait_contains("main-leaf-commit", WAIT);
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
            "tree row shows TAIL99 and drops the clipped prefix",
            WAIT,
        );
        let left = left_tree(&tui.screen());
        crate::harness::assert_absent(&left, "very-long");
        assert_contains(&left, "TAIL99");
    }
}

use crate::common::hscroll::TREE_HSCROLL_TAIL;
use crate::harness::{
    self, assert_tree_clipped_long_path, left_tree, status_has_tree_hscroll_tail,
    tree_cursor_bar_on_row, tree_is_panned_to_tail, tree_row_containing, PtySession,
    SGR_WHEEL_RIGHT, SGR_WHEEL_RIGHT_MOTION,
};
use crate::seed::{daily_workspace, seed_long_path_file};
use crate::support::{tree_cursor_on, GIT_WAIT, SETTLE_MS, TREE_LABEL_COL, WAIT};

/// Default mouse-on: clipped tree row, README short diff, cursor stays.
fn tree_sgr_hscroll_clipped_readme_focus(screen: &str, readme_row: u16) -> bool {
    harness::clipped_long_path_row(screen).is_some()
        && tree_cursor_bar_on_row(screen, readme_row)
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !screen.contains("Mouse on")
        && !status_has_tree_hscroll_tail(screen)
}

/// Documented tree pan: `TAIL99` on the tree row, prefix gone, README cursor.
fn documented_tree_sgr_hscroll_panned(screen: &str, readme_row: u16) -> bool {
    tree_is_panned_to_tail(screen)
        && tree_row_containing(screen, TREE_HSCROLL_TAIL).is_some()
        && tree_cursor_bar_on_row(screen, readme_row)
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !status_has_tree_hscroll_tail(screen)
}

/// Default mouse-on trackpad hscroll pans a clipped tree row.
///
/// Docs / keymap: write xterm SGR wheel right (`CSI < 67`) into the live
/// `event::read` loop. Motion-bit `CSI < 99` is dropped by crossterm 0.28
/// and must not pan. Shared oracle (`common::hscroll`): clipped `very-long`
/// prefix on the **tree row**, then `TAIL99` after pan, prefix gone. A
/// search chip that already contains `TAIL99` does not count. Do not `/`
/// search the tail first. Wait for a clipped tree row on the same frame.
/// Default mouse-on: this is not `pty_m_toggles_mouse_capture` and not
/// file-diff SGR pan. A no-op, a motion-bit-only pan, or a pan of only the
/// right pane / file-diff is red.
#[test]
fn pty_tree_sgr_hscroll_pans_clipped_path() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("README.md")
                && !screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && !screen.contains("SEARCH")
        },
        "launch paints the tree; mouse toast and SEARCH are absent",
        GIT_WAIT,
    );
    let _ = tui.wait_clipped_long_path_row(WAIT);

    // Short README diff so hscroll over the tree pans the tree, not a
    // long file-diff. Click is setup, not the click-to-select claim.
    let readme_hit = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_hit);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && screen.contains("+dirty")
                && screen.contains("app/README.md")
                && !screen.contains("SEARCH")
                && !screen.contains("Mouse off")
                && !screen.contains("Mouse on")
        },
        "default mouse-on click loads a short README diff (not the long path)",
        GIT_WAIT,
    );
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row before hscroll:\n{}", tui.screen()));
    let row = tui.wait_clipped_long_path_row(WAIT);
    assert_tree_clipped_long_path(&tui.screen());

    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT_MOTION, 6, row);
    }
    tui.wait_ms(SETTLE_MS);
    assert!(
        tree_sgr_hscroll_clipped_readme_focus(&tui.screen(), readme_row),
        "motion-bit CSI < 99 must not pan (crossterm 0.28 drops it):\n{}",
        tui.screen()
    );

    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, row);
    }
    tui.wait_pred(
        |screen| documented_tree_sgr_hscroll_panned(screen, readme_row),
        "tree row shows TAIL99, drops very-long, keeps README cursor and short diff",
        WAIT,
    );
    crate::common::hscroll::assert_panned_to_tail(&left_tree(&tui.screen()));
}

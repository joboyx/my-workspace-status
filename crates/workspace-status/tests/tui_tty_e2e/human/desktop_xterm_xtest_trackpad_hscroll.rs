#![cfg(target_os = "linux")]

use crate::common::hscroll::TREE_HSCROLL_TAIL;
use crate::desktop::DesktopSession;
use crate::harness::{
    assert_tree_clipped_long_path, left_tree, status_has_tree_hscroll_tail, tree_cursor_bar_on_row,
    tree_is_panned_to_tail, tree_row_containing,
};
use crate::seed::{daily_workspace, seed_long_path_file};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// XTEST wheel right. Must fail if the tree does not pan.
///
/// XTEST `click 7` (no `--window`) after a root-coordinate warp, in
/// xterm (VTE 0.76 does not report buttons 6/7). Same clipped-prefix vs
/// tail tree-row oracle as the PTY case (`common::hscroll`). No `/`
/// search. Wait for a clipped tree row on the same frame. Click README
/// so the tree pans, not a long file-diff. Cursor stays on that row.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; xterm encodes XTEST button 7"]
fn desktop_xterm_xtest_trackpad_hscroll() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let tui = DesktopSession::open_xterm_size(&workspace, 64, 24);
    let _ = tui.wait_clipped_long_path_row(WAIT);

    let readme_hit = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));
    tui.click_cell(6, readme_hit);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && !screen.contains("SEARCH")
        },
        "XTEST click loads a short README diff (not the long path)",
        GIT_WAIT,
    );
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row before hscroll:\n{}", tui.screen()));
    let row = tui.wait_clipped_long_path_row(WAIT);
    assert_tree_clipped_long_path(&tui.screen());
    tui.wheel_right_at_cell(6, row, 40);
    tui.wait_pred(
        |screen| {
            tree_is_panned_to_tail(screen)
                && tree_row_containing(screen, TREE_HSCROLL_TAIL).is_some()
                && tree_cursor_bar_on_row(screen, readme_row)
                && !status_has_tree_hscroll_tail(screen)
        },
        "tree row shows TAIL99, drops very-long, keeps README cursor",
        WAIT,
    );
    crate::common::hscroll::assert_panned_to_tail(&left_tree(&tui.screen()));
}

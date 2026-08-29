use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{tree_row_containing, PtySession, SGR_WHEEL_RIGHT};
use crate::seed::{daily_workspace, seed_long_diff_file};
use crate::support::WAIT;

/// Trackpad hscroll over the left pane pans a long file-diff.
#[test]
fn pty_left_pane_sgr_hscroll_pans_long_diff() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace, "unique-diffline.rs", DIFF_HSCROLL_TAIL);
    let mut tui = PtySession::open_size(&workspace, 80, 24);
    tui.search("unique-diffline");
    tui.wait_contains("/unique-diffline", WAIT);
    tui.wait_contains("unique-diffline.rs", WAIT);
    tui.wait_pred(
        |screen| screen.contains("nnnn") && !screen.contains(DIFF_HSCROLL_TAIL),
        "long diff tail is clipped before pan",
        WAIT,
    );
    let row = tree_row_containing(&tui.screen(), "unique-diffline")
        .unwrap_or_else(|| panic!("long diff file row:\n{}", tui.screen()));
    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, row);
    }
    tui.wait_contains(DIFF_HSCROLL_TAIL, WAIT);
}

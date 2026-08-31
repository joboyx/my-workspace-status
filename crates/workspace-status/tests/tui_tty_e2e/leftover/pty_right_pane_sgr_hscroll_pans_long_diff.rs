use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{
    left_tree, tree_cursor_bar_on_row, tree_row_containing, PtySession, SGR_WHEEL_RIGHT,
    SGR_WHEEL_RIGHT_MOTION,
};
use crate::seed::{daily_workspace, seed_long_diff_file};
use crate::support::{
    no_wrong_overlays, pane_top, right_pane, status_row, tree_cursor_on, tree_has, SETTLE_MS, WAIT,
};

const FILE: &str = "unique-diffline.rs";
/// Right pane on an 80-col layout (tree fraction 0.4 → `right_x` ≈ 32).
const NARROW_RIGHT_COL: u16 = 50;

/// Left tree focused, right file-diff unfocused. Not graph / not files.
fn panes_tree_focused_diff_unfocused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" tree ")
        && top.contains(" diff")
        && !top.contains(" diff ")
        && !top.contains(" graph")
        && !top.contains(" files")
}

fn status_has_diff_tail(screen: &str) -> bool {
    status_row(screen).contains(DIFF_HSCROLL_TAIL)
}

/// Long NEW file-diff is loaded, tail still clipped, tree focused on the file.
fn long_diff_clipped_tree_focus(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, FILE)
        && left.contains(FILE)
        && !left.contains(DIFF_HSCROLL_TAIL)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains("nnnn")
        && right.contains("inline (too narrow)")
        && !right.contains("inline (too narrow) ·")
        && !right.contains(DIFF_HSCROLL_TAIL)
        && !right.contains('█')
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !screen.contains("WIP on graph")
        && !screen.contains("┌ files")
        && !status_has_diff_tail(screen)
        && no_wrong_overlays(screen)
}

/// Documented right-pane trackpad hscroll: the long file-diff panned.
///
/// Tail is on the right pane, not the tree or the search chip. Header
/// starts the ` · pan N` suffix (`N` clips off at 80 columns). The
/// horizontal bar is shown. Tree filename and left focus stay.
fn documented_right_pane_sgr_hscroll_panned(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, FILE)
        && left.contains(FILE)
        && !left.contains(DIFF_HSCROLL_TAIL)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains(DIFF_HSCROLL_TAIL)
        && right.contains("inline (too narrow) ·")
        && right.contains('█')
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !screen.contains("WIP on graph")
        && !screen.contains("┌ files")
        && !status_has_diff_tail(screen)
        && no_wrong_overlays(screen)
}

/// Trackpad hscroll over the right pane pans a long file-diff.
///
/// Docs / keymap: write xterm SGR wheel right (`CSI < 67`) into the live
/// `event::read` loop. Motion-bit `CSI < 99` is dropped by crossterm 0.28
/// and must not pan. Horizontal wheel pans the pane under the pointer
/// without moving the focused row. Keys `h` / `l` already pan a focused
/// file-diff (`pty_h_l_pan_graph_or_file_diff`). This leftover is the
/// mouse path over the **right** pane. Left-pane SGR over a long diff is
/// `pty_left_pane_sgr_hscroll_pans_long_diff`.
///
/// Live PTY (80×24 so the NEW line clips): `/unique-diffline` loads the
/// file. Wheel over the right pane must put `UNIQUE_DIFF_TAIL` on the
/// right pane, start the pan suffix, paint the h-bar, keep tree focus
/// and the filename, and leave the status chip without the tail. A
/// no-op, a motion-bit-only pan, a tree pan, a left-pane-only pan,
/// vertical-only scroll, focus steal, or paint-only flicker is red.
#[test]
fn pty_right_pane_sgr_hscroll_pans_long_diff() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace, FILE, DIFF_HSCROLL_TAIL);
    let mut tui = PtySession::open_size(&workspace, 80, 24);
    tui.search("unique-diffline");
    tui.wait_pred(
        long_diff_clipped_tree_focus,
        "search loads the clipped NEW file-diff; tree stays focused on the file",
        WAIT,
    );
    let row = tree_row_containing(&tui.screen(), "unique-diffline")
        .unwrap_or_else(|| panic!("long diff file row:\n{}", tui.screen()));
    assert!(
        tree_cursor_bar_on_row(&tui.screen(), row),
        "wheel aims at the focused file row, right-pane column:\n{}",
        tui.screen()
    );

    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT_MOTION, NARROW_RIGHT_COL, row);
    }
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| long_diff_clipped_tree_focus(screen) && tree_cursor_bar_on_row(screen, row),
        "motion-bit CSI < 99 must not pan the long file-diff or the tree",
        WAIT,
    );

    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, NARROW_RIGHT_COL, row);
    }
    tui.wait_pred(
        |screen| {
            documented_right_pane_sgr_hscroll_panned(screen) && tree_cursor_bar_on_row(screen, row)
        },
        "SGR 67 over the right pane pans the long file-diff (tail + pan chrome; tree stays)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            documented_right_pane_sgr_hscroll_panned(screen) && tree_cursor_bar_on_row(screen, row)
        },
        "file-diff pan holds (not a flicker, tree pan, or focus steal)",
        WAIT,
    );
}

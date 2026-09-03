use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{
    left_tree, tree_cursor_bar_on_row, tree_row_containing, PtySession, SGR_WHEEL_RIGHT,
    SGR_WHEEL_RIGHT_MOTION,
};
use crate::seed::{daily_workspace, seed_long_diff_file};
use crate::support::{
    no_wrong_overlays, panes_tree_focused_diff_unfocused, right_pane, status_row, title_has_files,
    tree_cursor_on, tree_has, SETTLE_MS, WAIT,
};

const FILE: &str = "unique-diffline.rs";

fn status_has_diff_tail(screen: &str) -> bool {
    status_row(screen).contains(DIFF_HSCROLL_TAIL)
}

/// Long NEW file-diff is loaded, tail still clipped, tree focused on the file.
///
/// 80×24 clips `UNIQUE_DIFF_TAIL`. The header has no pan suffix. The
/// origin-hidden horizontal bar is absent.
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
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
        && no_wrong_overlays(screen)
}

/// Documented left-pane trackpad hscroll: the long file-diff panned.
///
/// Tail is on the right pane, not the tree or the search chip. Header
/// starts the ` · pan N` suffix (`N` clips off at 80 columns). The
/// horizontal bar is shown. Tree filename and left focus stay.
fn documented_left_pane_sgr_hscroll_panned(screen: &str) -> bool {
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
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
        && no_wrong_overlays(screen)
}

/// Trackpad hscroll over the left pane pans a long file-diff.
///
/// Docs / keymap: write xterm SGR wheel right (`CSI < 67`) into the live
/// `event::read` loop. Motion-bit `CSI < 99` is dropped by crossterm 0.28
/// and must not pan. Help VIEW `m` is mouse; MOVE `h l` pans lists/diff.
/// Configuration / diff-rendering: when a file diff has long lines, that
/// report over the left pane pans the diff rather than a short tree
/// label. Header shows `· pan N` (the `N` clips off at 80 columns). A
/// 1-row horizontal bar paints after the viewport leaves the left edge.
/// This is not tree SGR hscroll.
///
/// Live PTY (80×24 so the NEW line clips): `/unique-diffline` loads the
/// file. Wheel over the tree row must put `UNIQUE_DIFF_TAIL` on the
/// right pane, start the pan suffix, paint the h-bar, keep tree focus
/// and the filename, and leave the status chip without the tail. A
/// no-op, a motion-bit-only pan, a tree pan, a right-pane-only pan,
/// vertical-only scroll, or paint-only flicker is red.
#[test]
fn pty_left_pane_sgr_hscroll_pans_long_diff() {
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
        "wheel aims at the focused file row:\n{}",
        tui.screen()
    );

    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT_MOTION, 6, row);
    }
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| long_diff_clipped_tree_focus(screen) && tree_cursor_bar_on_row(screen, row),
        "motion-bit CSI < 99 must not pan the long file-diff or the tree",
        WAIT,
    );

    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, row);
    }
    tui.wait_pred(
        |screen| {
            documented_left_pane_sgr_hscroll_panned(screen) && tree_cursor_bar_on_row(screen, row)
        },
        "SGR 67 over the left pane pans the long file-diff (tail + pan chrome; tree stays)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            documented_left_pane_sgr_hscroll_panned(screen) && tree_cursor_bar_on_row(screen, row)
        },
        "file-diff pan holds (not a flicker, tree pan, or focus steal)",
        WAIT,
    );
}

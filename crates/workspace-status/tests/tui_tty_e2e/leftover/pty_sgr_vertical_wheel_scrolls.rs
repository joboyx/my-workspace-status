use crate::harness::{tree_row_containing, PtySession, SGR_WHEEL_DOWN};
use crate::seed::daily_workspace;
use crate::support::{
    seed_tree_page_files, tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, TREE_LABEL_COL, WAIT,
};

/// Wheel down + 1003 motion bit (`65 | 32`). crossterm 0.28 drops this.
const SGR_WHEEL_DOWN_MOTION: u8 = 65 | 32;

/// Launch: README focused, page-29 clipped off the tree viewport.
fn tree_overflow_before_scroll(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && tree_has(screen, "README.md")
        && tree_has(screen, "page-00.txt")
        && !tree_has(screen, "page-29.txt")
        && !tree_cursor_on(screen, "page-00.txt")
        && !tree_cursor_on(screen, "page-29.txt")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && screen.contains("focus right")
        && !screen.contains("drill")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !screen.contains("Mouse on")
}

/// Documented vertical wheel: page-29 is focused and in view, README left.
fn documented_tree_sgr_vertical_wheel_scrolled(screen: &str) -> bool {
    tree_cursor_on(screen, "page-29.txt")
        && tree_has(screen, "page-29.txt")
        && screen.contains("page-29-body")
        && screen.contains("NEW")
        && !tree_has(screen, "README.md")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "page-00.txt")
        && !tree_cursor_on(screen, "page-26.txt")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "workspace")
        && !screen.contains("UNSTAGED")
        && screen.contains("focus right")
        && !screen.contains("drill")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !screen.contains("Mouse on")
}

/// Default mouse-on SGR vertical wheel scrolls an overflowing tree.
///
/// Docs / keymap: write xterm SGR wheel down (`CSI < 65`) into the live
/// `event::read` loop. Wheel over a list moves that list's cursor (±1)
/// and the viewport follows. Motion-bit `CSI < 97` (`65 | 32`) is dropped
/// by crossterm 0.28 and must not scroll. This is not hscroll (`66`/`67`)
/// and not `pty_m_toggles_mouse_capture` (one-notch cursor while mouse
/// toggles).
///
/// Daily seed plus 30 `page-NN.txt` files so the focused tree cannot fit.
/// Launch keeps README in view and clips `page-29`. Thirty wheel-down
/// reports land on `page-29.txt`: README leaves, page-29 appears, the
/// right pane loads `page-29-body`. A no-op stays on README. `j` would
/// hit page-00. PageDown would hit page-26. `G` would hit No updates.
/// Horizontal pan, focus steal to the right pane, or chrome-only flicker
/// is red.
#[test]
fn pty_sgr_vertical_wheel_scrolls() {
    let (_root, workspace) = daily_workspace();
    seed_tree_page_files(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        tree_overflow_before_scroll,
        "launch focuses README; page-29 is clipped off the tree viewport",
        GIT_WAIT,
    );

    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));

    for _ in 0..30 {
        tui.sgr_mouse(SGR_WHEEL_DOWN_MOTION, TREE_LABEL_COL, readme_row);
    }
    tui.wait_ms(SETTLE_MS);
    assert!(
        tree_overflow_before_scroll(&tui.screen()),
        "motion-bit CSI < 97 must not scroll (crossterm 0.28 drops it):\n{}",
        tui.screen()
    );

    for _ in 0..30 {
        tui.sgr_mouse(SGR_WHEEL_DOWN, TREE_LABEL_COL, readme_row);
    }
    tui.wait_pred(
        documented_tree_sgr_vertical_wheel_scrolled,
        "tree shows page-29, drops README, loads page-29-body (a no-op stays on README; j would hit page-00; PageDown would hit page-26; G would hit No updates)",
        GIT_WAIT,
    );
}

use crate::harness::{left_tree, PtySession};
use crate::seed::tree_focus_workspace;
use crate::support::{
    focus_feature_from_app, tree_cursor_on, tree_focused_on_abc, workspace_row_has, FOCUS_CHIP_ABC,
    OUTSIDE_ROW_ABC, WAIT,
};

/// Row index of the first left-tree line that contains `needle`.
fn tree_row(screen: &str, needle: &str) -> Option<usize> {
    left_tree(screen)
        .lines()
        .position(|line| line.contains(needle))
}

/// The outside row is open under the focus: `notes` (dirty, attention
/// first) then `lib` (clean), both below the outside row; the chip stays.
fn outside_open(screen: &str) -> bool {
    let (Some(outside), Some(notes), Some(lib)) = (
        tree_row(screen, OUTSIDE_ROW_ABC),
        tree_row(screen, "notes"),
        tree_row(screen, "lib"),
    ) else {
        return false;
    };
    workspace_row_has(screen, FOCUS_CHIP_ABC) && outside < notes && notes < lib
}

/// The folded outside-focus row opens with `l` and lists the repos outside
/// the focus, while the focus stays on.
///
/// After `*` on `app` (`ABC-123`), End moves the cursor to the last tree row,
/// `2 outside focus · 1 dirty`. `l` opens it: `notes` (dirty) then `lib`
/// (clean) show under it, and the chip `(o) ABC-123 · 2/4 repos` stays.
#[test]
fn pty_focus_outside_row_opens() {
    let (_root, workspace) = tree_focus_workspace();
    let mut tui = PtySession::open(&workspace);
    focus_feature_from_app(&mut tui);

    tui.end();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, OUTSIDE_ROW_ABC) && tree_focused_on_abc(screen),
        "End moves the cursor to the folded outside row (the last tree row)",
        WAIT,
    );
    tui.key('l');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, OUTSIDE_ROW_ABC) && outside_open(screen),
        "`l` opens the outside row: notes then lib under it, the focus chip stays",
        WAIT,
    );
}

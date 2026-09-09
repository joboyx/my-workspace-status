use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, panes_tree_focused_diff_unfocused,
    panes_tree_unfocused_diff_focused, right_diff_has_focused_cursor,
    right_diff_has_inactive_selection, tree_cursor_on, tree_inactive_selection_on, SETTLE_MS, WAIT,
};

fn tree_focused_readme_keeps_right_inactive(screen: &str) -> bool {
    documented_launch_first_paint(screen)
        && panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, "README.md")
        && !tree_inactive_selection_on(screen, "README.md")
        && right_diff_has_inactive_selection(screen)
        && !right_diff_has_focused_cursor(screen)
}

fn right_focused_readme_keeps_tree_inactive(screen: &str) -> bool {
    panes_tree_unfocused_diff_focused(screen)
        && tree_has_inactive_readme(screen)
        && right_diff_has_focused_cursor(screen)
        && !right_diff_has_inactive_selection(screen)
}

fn tree_has_inactive_readme(screen: &str) -> bool {
    !tree_cursor_on(screen, "README.md")
        && tree_inactive_selection_on(screen, "README.md")
        && !tree_inactive_selection_on(screen, "app")
        && !tree_inactive_selection_on(screen, "workspace")
        && !tree_inactive_selection_on(screen, "merger")
}

/// Tab swaps the focused list cursor and the unfocused selection marker.
///
/// First paint: tree focused (`▌` on README), unfocused file-diff keeps
/// `▏`. After Tab: tree keeps `▏` on README, focused cursor is on the
/// file-diff (`▌` on UNSTAGED / `+dirty` / `@@`). Titles stay plain
/// `tree` / `diff` with no focus glyph.
///
/// Fail if Tab is a no-op (still left-focused / still `▌` on the tree
/// README) or if the markers stay swapped the wrong way.
#[test]
fn pty_tab_swaps_focused_and_inactive_list_markers() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        tree_focused_readme_keeps_right_inactive,
        "first paint: focused tree cursor on README, inactive marker on the unfocused diff (Tab no-op stays left-focused)",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        right_focused_readme_keeps_tree_inactive,
        "Tab: focused diff cursor, inactive marker on the unfocused README tree row (a no-op still paints the focused cursor on README)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        right_focused_readme_keeps_tree_inactive,
        "right-pane focus holds with the inactive tree marker (not a flicker, swapped markers, or a Tab no-op)",
        WAIT,
    );
}

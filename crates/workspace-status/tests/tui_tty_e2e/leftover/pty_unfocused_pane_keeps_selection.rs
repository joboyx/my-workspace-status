use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, panes_tree_focused_diff_unfocused,
    panes_tree_unfocused_diff_focused, right_diff_has_focused_cursor,
    right_diff_has_inactive_selection, tree_cursor_on, tree_inactive_selection_on, SETTLE_MS, WAIT,
};

/// CSI-u Enter (`CSI 13 ; 1 : 1 u` press, `: 3` release).
fn csi_u_enter(tui: &mut PtySession) {
    tui.csi_u(13, 1, 1);
    tui.csi_u(13, 1, 3);
}

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

/// Unfocused tree and file-diff still mark the selected row.
///
/// Docs: focused lists paint the full cursor bar and `cursorBg`. Unfocused
/// lists keep a thinner marker and `cursorBgInactive`. A blank unfocused
/// row is red. First paint is the daily README file-diff with the tree
/// focused.
///
/// Hunt leftover: Enter (or a missing inactive paint) that drops the
/// tree marker, or Esc that drops the diff marker, cannot pass. `j` on
/// the focused file-diff must not move or erase the tree marker.
#[test]
fn pty_unfocused_pane_keeps_selection() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        tree_focused_readme_keeps_right_inactive,
        "first paint: focused tree cursor on README, inactive marker on the unfocused diff",
        WAIT,
    );

    csi_u_enter(&mut tui);
    tui.wait_pred(
        right_focused_readme_keeps_tree_inactive,
        "Enter: focused diff cursor, inactive marker stays on the unfocused README tree row",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        right_focused_readme_keeps_tree_inactive,
        "right-pane focus holds with the inactive tree marker (not a flicker or a blank tree row)",
        WAIT,
    );

    tui.key('j');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        right_focused_readme_keeps_tree_inactive,
        "j on the focused file-diff keeps the inactive README tree marker",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        tree_focused_readme_keeps_right_inactive,
        "Esc: focused tree cursor returns; unfocused diff keeps the inactive marker",
        WAIT,
    );
}

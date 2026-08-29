use crate::harness::{tree_row_containing, PtySession};
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, merger_graph_left_unfocused, no_mouse_toggle_toast,
    no_updates_group_folded, no_wrong_overlays, status_row, tree_cursor_on, tree_dir_expanded,
    tree_has, tree_pane_focused, GIT_WAIT, SETTLE_MS, TREE_DEPTH1_CHEVRON_COL, TREE_LABEL_COL,
    WAIT,
};

/// Clicked merger label: cursor + graph pane, stay left, no fold, no Enter.
fn click_selects_merger_row(screen: &str) -> bool {
    merger_graph_left_unfocused(screen)
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && tree_pane_focused(screen)
        && !screen.contains("[workspace]")
        && !screen.contains("[merger]")
        && !screen.contains("┌ files")
        && !screen.contains("wip.txt")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && !screen.contains("app/README.md")
        && (screen.contains("Working tree") || screen.contains("working tree clean"))
        && screen.contains("WIP on graph")
        && crumb_row(screen).contains("workspace › merger")
        && status_row(screen).contains("focus right")
        && status_row(screen).contains(" tree")
        && status_row(screen).contains(" split")
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

/// Left-click a tree row selects it and loads that row's right pane.
///
/// Docs: SGR press+release. Must change the right pane. Setup clicks in
/// the hscroll test and `m` mouse-toggle clicks are not this claim.
/// Chevron click and right-pane click are separate leftovers.
///
/// Live PTY after first paint (cursor already on dirty README, file-diff
/// on the right): SGR press+release on the merger *label* (not the
/// chevron) moved the cursor to merger and replaced UNSTAGED / `+dirty`
/// with that repo's graph (`WIP on graph`, working tree clean). Stay
/// left. Not Enter. Not fold.
///
/// A no-op, cursor-only, chevron fold, right-pane click (`[workspace]`),
/// or Enter drill (`[merger]`) cannot pass.
#[test]
fn pty_click_selects_tree_row() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file-diff (click has not run)",
        WAIT,
    );

    let row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    assert_ne!(
        TREE_LABEL_COL, TREE_DEPTH1_CHEVRON_COL,
        "label click must not hit the depth-1 chevron"
    );
    tui.sgr_click(TREE_LABEL_COL, row);
    tui.wait_pred(
        click_selects_merger_row,
        "SGR press+release on merger label selects that row and loads its graph",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        click_selects_merger_row,
        "selected merger graph holds (not a flicker, cursor-only, or toast-only)",
        WAIT,
    );
}

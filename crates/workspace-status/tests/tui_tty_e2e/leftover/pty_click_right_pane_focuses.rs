use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, no_mouse_toggle_toast, no_updates_group_folded,
    no_wrong_overlays, panes_tree_unfocused_diff_focused, status_row, title_has_files,
    tree_cursor_on, tree_dir_expanded, tree_has, RIGHT_PANE_COL, SETTLE_MS,
    TREE_DEPTH1_CHEVRON_COL, TREE_LABEL_COL, WAIT,
};

/// 0-based screen row whose full line contains `needle`.
fn screen_row_containing(screen: &str, needle: &str) -> Option<u16> {
    screen
        .lines()
        .enumerate()
        .find_map(|(i, line)| line.contains(needle).then_some(i as u16))
}

/// Clicked file-diff: focus moved right. Same README. No fold. No drill.
fn click_focuses_readme_diff(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    panes_tree_unfocused_diff_focused(screen)
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("@@ -1 +1,2 @@")
        && screen.contains("app/README.md  inline (too narrow)")
        && crumb.trim() == "[workspace]"
        && !crumb.contains('›')
        && !crumb.contains("[merger]")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && status.contains(" tree")
        && status.contains(" split")
        && !status.contains("focus right")
        && !title_has_files(screen)
        && !screen.contains("wip.txt")
        && !screen.contains("WIP on graph")
        && !screen.contains("Working tree")
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

/// Click the file-diff to focus the right pane.
///
/// Docs + keymap: click selects a tree, graph, or commit-file row, or
/// focuses the right pane. Configuration: click the diff to focus the
/// right pane. Help VIEW: Tab is other pane; Enter on the left is the
/// same focus move. Chevron click and tree-row click are separate.
///
/// Live PTY after first paint: SGR press+release on the UNSTAGED body
/// pads ` diff `, brackets `[workspace]`, and swaps `focus right` for
/// `drill` / Esc back. `j` does not move the tree cursor. Esc unfocuses.
/// A no-op, tree-row select, chevron fold, files drill, or paint-only
/// flicker cannot pass.
#[test]
fn pty_click_right_pane_focuses() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file-diff (click has not run)",
        WAIT,
    );

    let row = screen_row_containing(&tui.screen(), "UNSTAGED")
        .unwrap_or_else(|| panic!("UNSTAGED row:\n{}", tui.screen()));
    assert_ne!(
        RIGHT_PANE_COL, TREE_LABEL_COL,
        "diff click must not hit a tree label"
    );
    assert_ne!(
        RIGHT_PANE_COL, TREE_DEPTH1_CHEVRON_COL,
        "diff click must not hit a fold chevron"
    );
    tui.sgr_click(RIGHT_PANE_COL, row);
    tui.wait_pred(
        click_focuses_readme_diff,
        "SGR press+release on the UNSTAGED diff focuses the right pane",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        click_focuses_readme_diff,
        "right-pane focus holds (not a flicker, toast-only, or tree-row select)",
        WAIT,
    );

    tui.key('j');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        click_focuses_readme_diff,
        "j on the focused file-diff does not move the tree cursor (a no-op click would land on merger)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        documented_launch_first_paint,
        "CSI-u Esc unfocuses the diff (files drill / graph cannot pass)",
        WAIT,
    );
}

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, no_mouse_toggle_toast, no_updates_group_folded,
    no_wrong_overlays, panes_tree_unfocused_diff_focused, status_row, title_has_files,
    tree_cursor_on, tree_dir_expanded, tree_has, SETTLE_MS, WAIT,
};

/// CSI-u Enter (`CSI 13 ; 1 : 1 u` press, `: 3` release).
///
/// The live loop requested `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus event
/// types. `PtySession::enter` sends CR (`\r`), which is a different path.
fn csi_u_enter(tui: &mut PtySession) {
    tui.csi_u(13, 1, 1);
    tui.csi_u(13, 1, 3);
}

/// Enter from the tree file row focused the file-diff. Same stack. No drill.
fn enter_focuses_readme_diff(screen: &str) -> bool {
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

/// Enter on a tree file row focuses the right pane (file-diff).
///
/// Docs + VIEW: left Enter is `focus right` on the same stack. Help lists
/// `Enter dblclick` as `focus right / drill`. Right Enter drills graph to
/// commit files. Launch is the README file-diff with left tree focused.
///
/// Live PTY after first paint: CSI-u Enter pads ` diff `, brackets
/// `[workspace]`, and swaps `focus right` for `drill` / Esc back. `j` does
/// not move the tree cursor. Esc unfocuses. A no-op, a graph drill, a
/// files drill, a crumb-only paint, Tab, `/`, or a flicker cannot pass.
#[test]
fn pty_enter_from_tree_focuses_right() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file-diff (Enter has not run)",
        WAIT,
    );

    csi_u_enter(&mut tui);
    tui.wait_pred(
        enter_focuses_readme_diff,
        "CSI-u Enter on the README tree row focuses the file-diff",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        enter_focuses_readme_diff,
        "right-pane focus holds (not a flicker, toast-only, or crumb-only paint)",
        WAIT,
    );

    tui.key('j');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        enter_focuses_readme_diff,
        "j on the focused file-diff does not move the tree cursor (a no-op Enter would land on merger)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        documented_launch_first_paint,
        "CSI-u Esc unfocuses the diff (files drill / graph cannot pass)",
        WAIT,
    );
}

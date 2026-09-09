use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{
    apply_current_keep_graph_focus, focusbox_keep_only_graph_body, graph_focus_cleared_full,
    graph_focus_overlay_open, GIT_WAIT, WAIT,
};

/// Cursor row in the focus overlay (`❯` plus the mark box).
fn graph_focus_overlay_cursor_row(screen: &str) -> Option<&str> {
    screen
        .lines()
        .find(|line| line.contains('❯') && (line.contains("[x]") || line.contains("[ ]")))
}

/// Reopen after a keep focus: current focus is pre-marked on the cursor row.
/// Overlay stays open. Keep-only graph behind must not reload yet.
fn graph_focus_keep_row_premarked(screen: &str) -> bool {
    let Some(row) = graph_focus_overlay_cursor_row(screen) else {
        return false;
    };
    graph_focus_overlay_open(screen)
        && screen.contains("space toggle")
        && row.contains("[x]")
        && row.contains("feature/keep")
        && focusbox_keep_only_graph_body(screen)
}

/// Docs: Space clears `[x]` on the focused overlay row. Overlay stays open.
/// The keep-only graph must not reload until Enter.
fn graph_focus_keep_row_unmarked(screen: &str) -> bool {
    let Some(row) = graph_focus_overlay_cursor_row(screen) else {
        return false;
    };
    graph_focus_overlay_open(screen)
        && screen.contains("space toggle")
        && row.contains("[ ]")
        && row.contains("feature/keep")
        && !row.contains("[x]")
        && !screen.contains("[x]")
        && focusbox_keep_only_graph_body(screen)
}

/// Unmark every `[x]` then Enter restores `--all`. Must not re-apply the
/// cursor row. Overlay Space/Enter, not `O`. A no-op or `[x]`-gone-only
/// screen delta cannot pass.
#[test]
fn pty_graph_focus_unmark_enter_clears() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    apply_current_keep_graph_focus(&mut tui);

    tui.key('o');
    tui.wait_pred(
        graph_focus_keep_row_premarked,
        "reopen pre-marks [x] on the focused keep row; overlay stays; keep-only graph",
        WAIT,
    );
    tui.key(' ');
    tui.wait_pred(
        graph_focus_keep_row_unmarked,
        "Space clears [x] on the focused keep row; overlay stays; graph does not reload",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        graph_focus_cleared_full,
        "Enter after unmark restores --all / full graph (does not re-drill keep)",
        GIT_WAIT,
    );
}

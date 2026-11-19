use std::cell::Cell;

use crate::harness::PtySession;
use crate::seed::tree_focus_workspace;
use crate::support::{
    ack_rows, focus_feature_from_app, toast_rows, tree_cursor_on, tree_focused_on_abc, GIT_WAIT,
    WAIT,
};

/// Completion text of the focused workspace fetch: the focused count, then
/// the count of repos outside the focus that were skipped.
const FETCH_DONE: &str = "Fetched 2 focused repos · 2 skipped";

/// Workspace-row `f` under a focus fetches the focused repos only and names
/// the skipped count.
///
/// Each repo of the fixture has a bare origin under the fixture root, so the
/// fetch runs offline. After `*` on `app` (`ABC-123`), `gg` moves to the
/// workspace row and `f` fetches `app` and `services/api`: one toast row
/// reads exactly [`FETCH_DONE`] (whole, not cut with `…`), with no failure.
#[test]
fn pty_focus_workspace_fetch_skips_outside() {
    let (_root, workspace) = tree_focus_workspace();
    let mut tui = PtySession::open(&workspace);
    focus_feature_from_app(&mut tui);

    tui.gg();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "# workspace") && tree_focused_on_abc(screen),
        "gg moves the cursor to the workspace row; the focus stays",
        WAIT,
    );
    tui.key('f');
    // A toast lives 2.5 s: note the ack on any frame, then wait for the end state.
    let seen = Cell::new(false);
    tui.wait_pred(
        |screen| {
            // A toast row ends one cell in from the pane's right border `│`.
            let whole = toast_rows(screen)
                .lines()
                .any(|row| row.trim_end_matches(['│', ' ']) == FETCH_DONE);
            if whole {
                seen.set(true);
            }
            seen.get() && tree_focused_on_abc(screen) && !ack_rows(screen).contains("failed")
        },
        "workspace `f` under ABC-123 toasts `Fetched 2 focused repos · 2 skipped` whole",
        GIT_WAIT,
    );
}

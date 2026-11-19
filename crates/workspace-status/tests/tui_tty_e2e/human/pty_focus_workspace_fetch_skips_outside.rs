use crate::harness::PtySession;
use crate::seed::tree_focus_workspace;
use crate::support::{
    ack_rows, after_ack, focus_feature_from_app, tree_cursor_on, tree_focused_on_abc, GIT_WAIT,
    WAIT,
};

/// The part of the completion toast that fits its 48-column row: the
/// focused count and the outside count.
const ACK_SHOWN: &str = "Fetched 2 focused repos · 2 outside focus";

/// Workspace-row `f` under a focus fetches the focused repos only and names
/// the skipped count.
///
/// Each repo of the fixture has a bare origin under the fixture root, so the
/// fetch runs offline. After `*` on `app` (`ABC-123`), `gg` moves to the
/// workspace row and `f` fetches `app` and `services/api`, with no failure.
///
/// The completion text is `Fetched 2 focused repos · 2 outside focus
/// skipped` (49 columns). It shows only as a toast, and a toast row is at
/// most 48 columns with its ` ok ` mark, so the word `skipped` is cut with
/// `…`. The test reads the part that fits, [`ACK_SHOWN`], which holds both
/// counts.
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
    tui.wait_pred(
        after_ack(ACK_SHOWN, |screen| {
            tree_focused_on_abc(screen) && !ack_rows(screen).contains("failed")
        }),
        "workspace `f` under ABC-123 fetches 2 focused repos and skips 2 outside",
        GIT_WAIT,
    );
}

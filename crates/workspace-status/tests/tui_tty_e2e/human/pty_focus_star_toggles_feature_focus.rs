use crate::harness::PtySession;
use crate::seed::tree_focus_workspace;
use crate::support::{
    after_ack, focus_feature_from_app, tree_cursor_on, tree_has, workspace_row_has, WAIT,
};

/// `*` on a checkout whose branch holds a ticket token focuses the tree on
/// that token; `*` again clears it.
///
/// `app` is on `feature/ABC-123-invoice-export`. `*` shows the chip
/// `(o) ABC-123 · 2/4 repos` on the workspace row and folds `lib` and the
/// dirty `notes` into one row `2 outside focus · 1 dirty`. A second `*` on
/// the same row shows `focus cleared`, drops the chip and the outside row,
/// and `notes` is back on the tree.
#[test]
fn pty_focus_star_toggles_feature_focus() {
    let (_root, workspace) = tree_focus_workspace();
    let mut tui = PtySession::open(&workspace);
    focus_feature_from_app(&mut tui);

    tui.key('*');
    tui.wait_pred(
        after_ack("focus cleared", |screen| {
            tree_cursor_on(screen, "app")
                && !workspace_row_has(screen, "(o)")
                && !tree_has(screen, "outside focus")
                && tree_has(screen, "notes")
                && tree_has(screen, "services/api")
        }),
        "a second `*` clears the focus: no chip, no outside row, notes shows again",
        WAIT,
    );
}

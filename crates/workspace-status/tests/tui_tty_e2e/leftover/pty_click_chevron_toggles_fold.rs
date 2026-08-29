use crate::harness::{tree_row_containing, PtySession};
use crate::seed::daily_workspace;
use crate::support::{tree_has, TREE_DEPTH1_CHEVRON_COL, WAIT};

/// Click the fold chevron, not the label.
#[test]
fn pty_click_chevron_toggles_fold() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "No updates") && !tree_has(screen, "lib"),
        "lib hidden before chevron click",
        WAIT,
    );
    let row = tree_row_containing(&tui.screen(), "No updates")
        .unwrap_or_else(|| panic!("No updates row:\n{}", tui.screen()));
    tui.sgr_click(TREE_DEPTH1_CHEVRON_COL, row);
    tui.wait_pred(
        |screen| tree_has(screen, "lib"),
        "chevron click expands No updates",
        WAIT,
    );
}

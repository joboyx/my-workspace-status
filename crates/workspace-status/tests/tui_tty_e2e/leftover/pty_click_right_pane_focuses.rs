use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{RIGHT_PANE_COL, WAIT};

/// Click the right pane to focus it. Breadcrumb brackets the last segment.
#[test]
fn pty_click_right_pane_focuses() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| !screen.contains("[workspace]"),
        "left focus does not bracket the workspace crumb",
        WAIT,
    );
    tui.sgr_click(RIGHT_PANE_COL, 6);
    tui.wait_contains("[workspace]", WAIT);
}

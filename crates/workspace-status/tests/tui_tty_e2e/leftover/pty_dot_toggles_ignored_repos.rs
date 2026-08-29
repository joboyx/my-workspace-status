use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_has, WAIT};

/// `.` shows ignored `notes`, then hides it again.
#[test]
fn pty_dot_toggles_ignored_repos() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.wait_pred(
        |screen| !tree_has(screen, "notes"),
        "notes hidden until shown",
        WAIT,
    );
    tui.key('.');
    tui.wait_pred(
        |screen| tree_has(screen, "notes"),
        "dot shows ignored notes",
        WAIT,
    );
    tui.key('.');
    tui.wait_pred(
        |screen| !tree_has(screen, "notes"),
        "second dot hides notes",
        WAIT,
    );
}

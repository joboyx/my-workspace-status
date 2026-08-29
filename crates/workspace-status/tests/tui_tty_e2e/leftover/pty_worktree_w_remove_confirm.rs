use crate::harness::PtySession;
use crate::seed::worktree_workspace;
use crate::support::{tree_has, GIT_WAIT, SETTLE_MS, WAIT};

/// `W` on a linked worktree asks, then removes.
#[test]
fn pty_worktree_w_remove_confirm() {
    let (_root, workspace) = worktree_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("linked-open");
    tui.wait_contains("/linked-open", WAIT);
    tui.wait_contains("feature/linked-open", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.shift_letter('W');
    tui.wait_contains("Remove worktree", WAIT);
    tui.key('y');
    tui.wait_contains("removed worktree", GIT_WAIT);
    tui.wait_pred(
        |screen| !tree_has(screen, "feature/linked-open"),
        "linked worktree row gone after remove",
        WAIT,
    );
}

use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{GIT_WAIT, SETTLE_MS, WAIT};

/// Graph `c` creates a ref at the focused commit (not a commit overlay).
#[test]
fn pty_graph_c_creates_branch_at_commit() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("/focusbox", WAIT);
    tui.tab();
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('/');
    tui.keys("keep-leaf-commit");
    tui.enter();
    tui.wait_contains("/keep-leaf-commit", WAIT);
    tui.wait_contains("create branch", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('c');
    tui.wait_contains("Create branch", WAIT);
    tui.wait_contains("at ", WAIT);
    tui.keys("e2e-at-commit");
    tui.enter();
    tui.wait_contains("created e2e-at-commit at", GIT_WAIT);
    tui.wait_absent("Create branch", WAIT);
}

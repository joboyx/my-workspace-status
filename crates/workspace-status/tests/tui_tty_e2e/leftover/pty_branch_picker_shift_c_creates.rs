use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{GIT_WAIT, SETTLE_MS, WAIT};

/// Picker `C` creates (and checks out) a branch at HEAD.
#[test]
fn pty_branch_picker_shift_c_creates() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("/focusbox", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('b');
    tui.wait_contains("Branch ", WAIT);
    tui.shift_letter('C');
    tui.wait_contains("Create branch", WAIT);
    tui.keys("e2e-from-picker");
    tui.enter();
    tui.wait_contains("created e2e-from-picker", GIT_WAIT);
}

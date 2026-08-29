use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{SETTLE_MS, WAIT};

/// `x` opens the revert confirm; `n` cancels.
#[test]
fn pty_revert_confirm_n_cancels() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('x');
    tui.wait_contains("Revert ", WAIT);
    tui.wait_contains("tracked", WAIT);
    tui.key('n');
    tui.wait_absent("Revert ", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
}

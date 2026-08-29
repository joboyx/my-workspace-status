use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{op_finished, GIT_WAIT, SETTLE_MS, WAIT};

/// `d` switches a clean non-default checkout.
#[test]
fn pty_d_switches_to_default_branch() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("/focusbox", WAIT);
    tui.wait_contains("feature/keep", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('d');
    tui.wait_pred(
        |screen| op_finished(screen, "Switched") || screen.contains("Switched 1 repo"),
        "Switched 1 repo without failure",
        GIT_WAIT,
    );
}

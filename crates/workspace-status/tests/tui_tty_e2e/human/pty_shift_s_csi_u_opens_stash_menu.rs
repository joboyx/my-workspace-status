use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// Shift+S via CSI-u opens the stash overlay (not `s` stage).
#[test]
fn pty_shift_s_csi_u_opens_stash_menu() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("README.md", WAIT);
    tui.shift_letter('S');
    tui.wait_contains("Stash ", WAIT);
    tui.wait_contains("stash", WAIT);
}

use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{GIT_WAIT, WAIT};

/// Esc on compare right focuses left. Esc on compare left closes that tab.
#[test]
fn pty_compare_esc_closes_tab() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app · vs origin/main", GIT_WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| screen.contains("COMMITTED") && screen.contains("alpha.txt"),
        "Enter moves compare focus to the diff",
        WAIT,
    );
    tui.esc();
    tui.wait_contains("app · vs origin/main", WAIT);
    tui.esc();
    tui.wait_pred(
        |screen| screen.contains("# workspace") && !screen.contains("app · vs origin/main"),
        "Esc on compare left closes that tab",
        WAIT,
    );
}

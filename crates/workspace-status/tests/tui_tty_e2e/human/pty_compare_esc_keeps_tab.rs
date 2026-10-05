use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{GIT_WAIT, WAIT};

/// Esc on compare right focuses left. Esc on compare left keeps the tab.
/// The palette `Close tab` row closes it.
#[test]
fn pty_compare_esc_keeps_tab() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app ↔ origin/main", GIT_WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| screen.contains("COMMITTED") && screen.contains("alpha.txt"),
        "Enter moves compare focus to the diff",
        WAIT,
    );
    tui.esc();
    tui.wait_contains("app ↔ origin/main", WAIT);
    tui.esc();
    tui.wait_ms(300);
    let screen = tui.screen();
    assert!(
        screen.contains("app ↔ origin/main"),
        "Esc on compare left keeps the tab:\n{screen}"
    );

    tui.ctrl_letter('k');
    tui.wait_contains(">▏", WAIT);
    for c in "close tab".chars() {
        tui.key(c);
        if c == 'l' {
            tui.wait_ms(150);
        }
    }
    tui.wait_pred(
        |screen| screen.contains("Close tab") && screen.contains("close tab"),
        "palette filter `close tab` lists Close tab",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| screen.contains("# workspace") && !screen.contains("app ↔ origin/main"),
        "palette Close tab closes the compare tab",
        WAIT,
    );
}

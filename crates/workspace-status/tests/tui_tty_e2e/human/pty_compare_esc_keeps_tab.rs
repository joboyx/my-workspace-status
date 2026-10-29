use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{prove_keys_handled, type_palette_filter, GIT_WAIT, WAIT};

/// Esc on compare right focuses left. Esc on compare left keeps the tab.
/// The palette `Close tab` row closes it.
#[test]
fn pty_compare_esc_keeps_tab() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.key(':');
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
    // The probe proves both Esc presses were handled before the check.
    prove_keys_handled(&mut tui);
    let screen = tui.screen();
    assert!(
        screen.contains("app ↔ origin/main"),
        "Esc on compare left keeps the tab:\n{screen}"
    );

    tui.key(':');
    tui.wait_contains(">▏", WAIT);
    type_palette_filter(&mut tui, "close tab");
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

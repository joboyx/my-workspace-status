use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_has, SETTLE_MS, WAIT};

/// CSI-u Repeat of `j` keeps moving. A single press must not reach the end.
#[test]
fn pty_key_repeat_j_reaches_no_updates() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.gg();
    tui.wait_ms(SETTLE_MS);
    tui.letter_press('j');
    tui.wait_ms(80);
    for _ in 0..10 {
        tui.letter_repeat('j');
        tui.wait_ms(50);
    }
    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib"),
        "Repeat j must land on No updates so l reveals lib",
        WAIT,
    );
}

use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_has, GIT_WAIT, SETTLE_MS, WAIT};

/// `r` reloads the focused repo while watch is off.
///
/// `PtySession` defaults to `WS_STATUS_WATCH_MS=0`. Live watch without `r`
/// is `pty_watch_applies_while_keys_arrive`.
#[test]
fn pty_r_refreshes_new_dirty_file() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_pred(
        |screen| !tree_has(screen, "r-live.txt"),
        "new file is absent before r",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    fs::write(workspace.join("app").join("r-live.txt"), "refresh me\n").unwrap();
    tui.key('r');
    tui.wait_contains("r-live.txt", GIT_WAIT);
}

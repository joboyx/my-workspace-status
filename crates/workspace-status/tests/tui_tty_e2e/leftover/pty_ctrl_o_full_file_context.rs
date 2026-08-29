use crate::harness::{self, PtySession};
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// `Ctrl-o` paints the full-file marker on the focused diff.
#[test]
fn pty_ctrl_o_full_file_context() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    harness::assert_absent(&tui.screen(), " · full");
    tui.ctrl('o');
    tui.wait_contains(" · full", WAIT);
}

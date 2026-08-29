use crate::harness::{self, assert_contains, PtySession};
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// `c` on a dirty file is a no-op. It must not open a commit overlay.
#[test]
fn pty_c_on_tree_file_is_not_commit() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    tui.key('c');
    tui.wait_ms(400);
    let screen = tui.screen();
    harness::assert_absent(&screen, "Create branch");
    harness::assert_absent(&screen, "commit message");
    assert_contains(&screen, "UNSTAGED");
}

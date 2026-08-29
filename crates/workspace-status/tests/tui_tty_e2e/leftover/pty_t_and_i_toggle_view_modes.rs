use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// `t` flips the tree/flat pill. `i` flips inline/split on a file diff.
#[test]
fn pty_t_and_i_toggle_view_modes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains(" tree", WAIT);
    tui.key('t');
    tui.wait_contains("Flat paths", WAIT);
    tui.key('t');
    tui.wait_contains("Directory tree", WAIT);

    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    let was_split = tui.screen().contains("split");
    tui.key('i');
    if was_split {
        tui.wait_contains("inline", WAIT);
    } else {
        tui.wait_contains("split", WAIT);
    }
}

use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{status_row, GIT_WAIT, WAIT};

fn seed_nested(workspace: &std::path::Path) {
    fs::create_dir_all(workspace.join("app").join("src")).unwrap();
    fs::write(workspace.join("app").join("src").join("view.rs"), "fn view() {}\n").unwrap();
}

/// Bare `t` still toggles tree/flat. It does not switch compare tabs.
#[test]
fn pty_compare_bare_t_still_tree_flat() {
    let (_root, workspace) = daily_workspace();
    seed_nested(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app · vs main", GIT_WAIT);

    tui.key('t');
    tui.wait_pred(
        |screen| {
            screen.contains("app · vs main")
                && (status_row(screen).contains("Flat paths")
                    || screen.contains("Flat paths"))
        },
        "bare t stays on the compare tab and toggles flat/tree",
        WAIT,
    );
    tui.key('t');
    tui.wait_contains("app · vs main", WAIT);
}

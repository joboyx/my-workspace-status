use std::time::Duration;

use crate::common::seed::{new_workspace, seed_repo};
use crate::harness::PtySession;
use crate::support::{tree_has, GIT_WAIT};

/// Watch period of this test. Short, so several collects run in the wait.
const WATCH_MS: &str = "300";

/// Long enough for a few watch collects to walk the empty workspace.
const EMPTY_TICKS: Duration = Duration::from_millis(1200);

/// A watch collect whose walk finds no checkout must end, or the watch
/// stops for the rest of the run: a repo that appears later must show up.
#[test]
fn pty_watch_finds_repo_added_to_empty_workspace() {
    let (_root, workspace) = new_workspace("ws-tui-tty-watch-empty");
    let tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", WATCH_MS)]);
    std::thread::sleep(EMPTY_TICKS);
    assert!(
        !tree_has(&tui.screen(), "late"),
        "no checkout before the repo is made"
    );
    seed_repo(&workspace, "late", "main", true);
    tui.wait_pred(
        |screen| tree_has(screen, "late"),
        "watch picks up the repo made after launch",
        GIT_WAIT,
    );
}

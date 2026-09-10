use crate::seed::unfetched_behind_workspace;
use crate::slow_git::{open_with_slow_git, wait_slow_git_started, SlowFetchPullGit};
use crate::support::{crumb_row, tree_has, WAIT};

/// Idle first paint: workspace chrome, no Ctrl+C quit prompt.
fn idle_tui_ready_for_fetch(screen: &str) -> bool {
    tree_has(screen, "# workspace")
        && tree_has(screen, "No updates")
        && screen.contains(" tree")
        && screen.contains("? help")
        && !screen.contains("Press Ctrl+C again to exit")
        && !screen.contains("MOVE")
}

/// `q` quits while a slow fetch is still running.
///
/// Help `q` / quit must not wait for the git child and must not panic.
/// Live PTY: workspace `f` starts a slow fetch, then `q`. Process exits.
/// Hang or non-zero exit is red. The Ctrl+C twice-to-quit prompt must
/// never paint.
#[test]
fn pty_q_quits_while_fetch_is_running() {
    let (_root, workspace) = unfetched_behind_workspace();
    let slow = SlowFetchPullGit::install(&workspace);
    let mut tui = open_with_slow_git(&workspace, &slow);
    tui.wait_pred(
        idle_tui_ready_for_fetch,
        "first paint: workspace chrome, no Ctrl+C quit prompt",
        WAIT,
    );

    tui.key('f');
    wait_slow_git_started(&slow, || tui.screen(), WAIT);
    assert!(
        crumb_row(&tui.screen()).contains("Fetching") || tui.screen().contains("Fetching"),
        "q must be pressed during occupy; screen:\n{}",
        tui.screen()
    );

    tui.key('q');
    tui.wait_exit_without("Press Ctrl+C again to exit", WAIT);
}

use std::time::{Duration, Instant};

use crate::harness::PtySession;
use crate::seed::behind_workspace;
use crate::support::{
    crumb_row, has_pull_hint, syncbox_row_behind, syncbox_row_current, tree_cursor_on, tree_has,
    GIT_WAIT, WAIT,
};

/// Long enough for a pull against the local bare origin to start and paint.
const NO_PULL_WINDOW: Duration = Duration::from_millis(1500);

/// Cursor on the behind checkout; nothing pulled yet.
fn idle_behind_syncbox(screen: &str) -> bool {
    tree_cursor_on(screen, "syncbox")
        && tree_has(screen, "1 behind")
        && syncbox_row_behind(screen)
        && has_pull_hint(screen)
        && screen.contains("main v1")
        && !screen.contains("Pulling")
        && !screen.contains("Pulled")
}

/// Still behind with no pull started. The Quick Open overlay may cover the
/// footer pull hint, so this does not ask for it.
fn no_pull_started(screen: &str) -> bool {
    tree_has(screen, "1 behind")
        && syncbox_row_behind(screen)
        && !screen.contains("Pulling")
        && !screen.contains("Pulled")
}

fn crumb_pulled_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Pulled 1 repo") && !crumb.contains("failed")
}

/// Ctrl-p is not `p`: it opens Quick Open on files and never pulls.
///
/// Same behind checkout as the plain `p` pull test. CSI-u Ctrl+p must not
/// start a pull: for a settle window every frame keeps `1 behind` and no
/// `Pulling` / `Pulled`, and the file picker (`Go to file`) paints. Esc
/// closes it, then plain `p` pulls, so the fixture really was pullable.
#[test]
fn pty_ctrl_p_does_not_pull() {
    let (_root, workspace) = behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("syncbox", WAIT);
    tui.wait_contains("origin-tip-commit", GIT_WAIT);
    tui.wait_pred(
        idle_behind_syncbox,
        "first paint: cursor on behind syncbox, pull hint, nothing pulled",
        WAIT,
    );

    tui.ctrl_letter('p');
    let start = Instant::now();
    while start.elapsed() < NO_PULL_WINDOW {
        let screen = tui.screen();
        assert!(
            no_pull_started(&screen),
            "Ctrl-p must not pull; screen:\n{screen}"
        );
        tui.wait_ms(50);
    }
    tui.assert_running("after Ctrl-p");
    tui.wait_pred(
        |screen| screen.contains("Go to file") && no_pull_started(screen),
        "Ctrl-p opens Quick Open on files instead of pulling",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| idle_behind_syncbox(screen) && !screen.contains("Go to file"),
        "Esc closes the file picker; cursor back on the behind syncbox, nothing pulled",
        WAIT,
    );

    tui.key('p');
    tui.wait_pred(
        |screen| crumb_pulled_one(screen) && syncbox_row_current(screen),
        "plain p still pulls: Pulled 1 repo, syncbox current",
        GIT_WAIT,
    );
}

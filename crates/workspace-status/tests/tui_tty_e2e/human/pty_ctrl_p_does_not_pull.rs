use crate::harness::PtySession;
use crate::seed::behind_workspace;
use crate::support::{
    ack_rows, after_ack, has_pull_hint, syncbox_row_behind, syncbox_row_current, tree_cursor_on,
    tree_has, GIT_WAIT, WAIT,
};

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

fn no_failed_ack(screen: &str) -> bool {
    !ack_rows(screen).contains("failed")
}

/// Ctrl-p is not `p`: it opens Quick Open on files and never pulls.
///
/// Same behind checkout as the plain `p` pull test. CSI-u Ctrl+p must not
/// start a pull: the file picker (`Go to file`) paints with `1 behind` and
/// no `Pulling` / `Pulled`, and after Esc the row is still behind with no
/// pull shown. Plain `p` then pulls, so the fixture really was pullable.
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

    // A pull from Ctrl-p would show in every later frame: `Pulling`, then
    // `Pulled`, then a current row with no `1 behind`. The Quick Open paint
    // proves Ctrl-p was handled, and the idle check after Esc reads a frame
    // where any pull it started would show.
    tui.ctrl_letter('p');
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
        after_ack("Pulled 1 repo", |screen| {
            no_failed_ack(screen) && syncbox_row_current(screen)
        }),
        "plain p still pulls: Pulled 1 repo, syncbox current",
        GIT_WAIT,
    );
}

use crate::harness::PtySession;
use crate::seed::{compare_history_workspace, git_stdout, set_view_default, HISTORY_SUBJECTS};
use crate::support::{graph_cursor_on, graph_pane_focused, GIT_WAIT, WAIT};

/// The focused diff row ends with who last changed it: ` · <sha7> ·
/// <subject>` of the commit that added the line. `B` hides it and shows
/// it again.
///
/// Live PTY on the middle commit's own diff (palette Diff commit vs
/// parent), inline so the subject is not cut. A blame of the wrong
/// commit, a row that is not focused, or a toggle that only toasts
/// cannot pass.
#[test]
fn pty_line_blame_shows_commit_on_focused_row() {
    let (_root, workspace) = compare_history_workspace();
    set_view_default(&workspace, "diff", "inline");
    let [_, middle, newest] = HISTORY_SUBJECTS;
    let sha = git_stdout(&workspace.join("app"), &["rev-parse", "HEAD~1"]);
    let note = format!(" · {} · {middle}", &sha[..7]);
    let annotated = |screen: &str| {
        screen
            .lines()
            .any(|line| line.contains(&format!("+{middle}")) && line.contains(&note))
    };

    let mut tui = PtySession::open_size(&workspace, 160, 30);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.wait_contains("feature/history", GIT_WAIT);
    tui.tab();
    tui.wait_pred(
        |screen| graph_pane_focused(screen) && screen.contains(newest) && screen.contains(middle),
        "Tab focuses the app graph with the history commits",
        GIT_WAIT,
    );
    tui.search(middle);
    tui.wait_pred(
        |screen| graph_cursor_on(screen, middle) && !graph_cursor_on(screen, newest),
        "graph cursor on the middle commit",
        WAIT,
    );
    tui.ctrl_letter('k');
    tui.keys("vs parent");
    tui.wait_contains("Diff commit vs parent in new tab", WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| screen.contains("COMMITTED") && screen.contains(&format!("+{middle}")),
        "the middle commit's diff opens in a compare tab",
        GIT_WAIT,
    );

    tui.tab();
    tui.key('G');
    tui.wait_pred(annotated, "focused added line shows its commit", GIT_WAIT);

    tui.key('B');
    tui.wait_pred(
        |screen| !screen.contains(&note) && screen.contains("line blame off"),
        "B hides the annotation",
        WAIT,
    );
    tui.key('B');
    tui.wait_pred(
        |screen| annotated(screen) && screen.contains("line blame on"),
        "B shows it again",
        GIT_WAIT,
    );
}

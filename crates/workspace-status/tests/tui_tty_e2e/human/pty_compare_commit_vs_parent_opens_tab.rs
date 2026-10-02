use crate::harness::{left_tree, PtySession};
use crate::seed::{compare_history_workspace, git_stdout, HISTORY_SUBJECTS};
use crate::support::{graph_cursor_on, graph_pane_focused, GIT_WAIT, WAIT};

fn rev(workspace: &std::path::Path, rev: &str) -> String {
    git_stdout(&workspace.join("app"), &["rev-parse", rev])
}

/// Tab-strip row: the line that holds the Workspace tab and a compare tab.
fn tab_strip_has(screen: &str, label: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains("Workspace") && line.contains(label))
}

/// Palette Diff commit vs parent opens the focused graph commit's own diff
/// in a new tab `app ↔ <short>^`. A later commit's file is not listed, and
/// HEAD does not move.
#[test]
fn pty_compare_commit_vs_parent_opens_tab() {
    let (_root, workspace) = compare_history_workspace();
    let [_, middle, newest] = HISTORY_SUBJECTS;
    let before = rev(&workspace, "HEAD");
    let middle_sha = rev(&workspace, "HEAD~1");
    let label = format!("app ↔ {}^", &middle_sha[..7]);

    let mut tui = PtySession::open(&workspace);
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
        "graph cursor on the middle commit, not HEAD",
        WAIT,
    );

    tui.ctrl_letter('k');
    tui.keys("vs parent");
    tui.wait_contains("Diff commit vs parent in new tab", WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| {
            let left = left_tree(screen);
            tab_strip_has(screen, &label)
                && screen.contains("COMMITTED")
                && left.contains("second.txt")
                && !left.contains("third.txt")
                && !left.contains("first.txt")
                && !left.contains("README.md")
        },
        "new tab lists only the middle commit's own file next to Workspace",
        GIT_WAIT,
    );
    assert_eq!(rev(&workspace, "HEAD"), before);
}

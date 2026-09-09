use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, status_row, tree_cursor_on, tree_inactive_selection_on, GIT_WAIT, WAIT,
};

/// Enter-armed `/merger` on the left tree. Tab has not run.
fn armed_merger_chip_left(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    status.contains("/merger")
        && !status.contains("n next")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && !screen.contains("MOVE")
        && !screen.contains("HELP  /")
        && crumb.contains("workspace › merger")
        && !crumb.contains("[merger]")
        && tree_cursor_on(screen, "merger")
        && !tree_inactive_selection_on(screen, "merger")
}

/// Tab moved focus right. The armed chip and breadcrumb stay.
fn armed_merger_chip_after_tab(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    status.contains("/merger")
        && !status.contains("n next")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && !screen.contains("MOVE")
        && crumb.contains('›')
        && crumb.contains("[merger]")
        && tree_inactive_selection_on(screen, "merger")
        && !tree_cursor_on(screen, "merger")
}

/// `/merger` Enter-arm on the tree, then Tab to the right pane.
///
/// Idle pills and first-paint chrome stay on
/// `pty_launch_paints_tree_diff_and_chrome`. Left-only arm stays on
/// `pty_slash_pane_search`. This claim is that Tab keeps the `/merger`
/// chip, does not paint `n next`, and the breadcrumb still names the
/// focused repo with `›` (`[merger]`).
///
/// Fail if Tab is a no-op (left crumb without brackets), clears the
/// chip, or arms `n next`.
#[test]
fn pty_armed_search_chip_survives_tab() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("merger")
                && screen.contains("? help")
                && !screen.contains("SEARCH")
                && !screen.contains("/merger")
        },
        "launch focuses README with search unarmed",
        WAIT,
    );

    tui.search("merger");
    tui.wait_pred(
        armed_merger_chip_left,
        "Enter arms /merger on the tree; stay left (Tab has not run)",
        GIT_WAIT,
    );
    assert!(
        !armed_merger_chip_after_tab(&tui.screen()),
        "Tab claim must be false before Tab (left crumb without [merger]):\n{}",
        tui.screen()
    );

    tui.tab();
    tui.wait_pred(
        armed_merger_chip_after_tab,
        "Tab keeps /merger, omits n next, and marks [merger] on the breadcrumb",
        GIT_WAIT,
    );
}

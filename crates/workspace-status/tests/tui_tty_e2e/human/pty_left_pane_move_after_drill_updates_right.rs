use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, graph_cursor_on, merger_graph_drilled_right,
    merger_graph_left_unfocused, panes_files_focused, panes_graph_focused_files_unfocused,
    right_pane, title_has_diff, title_has_files, title_has_graph, GIT_WAIT, WAIT,
};

/// CSI-u Enter (`CSI 13 ; 1 : 1 u` press, `: 3` release).
///
/// `PtySession::enter` sends CR (`\r`), which is a different path.
fn csi_u_enter(tui: &mut PtySession) {
    tui.csi_u(13, 1, 1);
    tui.csi_u(13, 1, 3);
}

fn names_a_commit_path(screen: &str) -> bool {
    screen.contains("left.txt") || screen.contains("right.txt") || screen.contains("README.md")
}

fn merge_commit_selected(screen: &str) -> bool {
    graph_cursor_on(screen, "merge")
        && !graph_cursor_on(screen, "WIP on graph")
        && !graph_cursor_on(screen, "working tree")
}

fn merge_commit_files_right(screen: &str) -> bool {
    panes_files_focused(screen)
        && title_has_files(screen)
        && title_has_graph(screen)
        && !title_has_diff(screen)
        && names_a_commit_path(screen)
        && !screen.contains("wip.txt")
}

/// First Esc may clear an armed pane search. A second Esc runs only while
/// the right pane still holds keyboard focus, so files drill does not pop.
fn unfocus_right(
    tui: &mut PtySession,
    still_right: impl Fn(&str) -> bool,
    now_left: impl Fn(&str) -> bool,
    why: &str,
) {
    tui.esc();
    tui.wait_ms(120);
    if still_right(&tui.screen()) {
        tui.esc();
    }
    tui.wait_pred(now_left, why, WAIT);
}

/// `j` on the focused graph after a files drill reloads the right pane.
///
/// Docs: left-pane move with the graph on the left and commit files on
/// the right updates the file list. A no-op `j` cannot pass.
#[test]
fn pty_left_pane_move_after_drill_updates_right() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff (graph drill has not run)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph",
        GIT_WAIT,
    );
    csi_u_enter(&mut tui);
    tui.wait_pred(
        merger_graph_drilled_right,
        "CSI-u Enter on merger focuses that graph",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "WIP on graph") && !graph_cursor_on(screen, "working tree")
        },
        "first j selects stash@{0} (skip this row)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        merge_commit_selected,
        "second j selects the merge commit (not stash / uncommitted)",
        WAIT,
    );
    csi_u_enter(&mut tui);
    tui.wait_pred(
        merge_commit_files_right,
        "CSI-u Enter on the merge commit opens its file list",
        GIT_WAIT,
    );

    unfocus_right(
        &mut tui,
        panes_files_focused,
        panes_graph_focused_files_unfocused,
        "Esc leaves the graph focused on the left with commit files on the right",
    );

    let before = tui.screen();
    let before_right = right_pane(&before);
    tui.key('j');
    tui.wait_pred(
        |screen| {
            panes_graph_focused_files_unfocused(screen)
                && screen != before
                && right_pane(screen) != before_right
        },
        "j on the left graph reloads the right pane for the next row (a no-op cannot pass)",
        GIT_WAIT,
    );
}

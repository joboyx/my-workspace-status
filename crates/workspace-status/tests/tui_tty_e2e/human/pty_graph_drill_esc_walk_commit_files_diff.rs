use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, graph_cursor_on, merger_graph_drilled_right,
    merger_graph_left_unfocused, panes_files_focused, panes_files_focused_diff_unfocused,
    panes_files_unfocused_diff_focused, panes_graph_focused_files_unfocused, still_file_diff,
    title_has_diff, title_has_files, title_has_graph, GIT_WAIT, WAIT,
};

/// CSI-u Enter (`CSI 13 ; 1 : 1 u` press, `: 3` release).
///
/// `PtySession::enter` sends CR (`\r`), which is a different path.
fn csi_u_enter(tui: &mut PtySession) {
    tui.csi_u(13, 1, 1);
    tui.csi_u(13, 1, 3);
}

fn last_crumb_unbracketed(screen: &str) -> bool {
    let crumb = crumb_row(screen).trim();
    let last = crumb.rsplit(" › ").next().unwrap_or(crumb);
    !last.trim_start().starts_with('[')
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
        && crumb_row(screen).contains('›')
        && !last_crumb_unbracketed(screen)
}

fn commit_file_diff_right(screen: &str) -> bool {
    panes_files_unfocused_diff_focused(screen)
        && title_has_diff(screen)
        && title_has_files(screen)
        && !title_has_graph(screen)
        && names_a_commit_path(screen)
        && (screen.contains("inline") || screen.contains("split"))
        && !screen.contains("wip.txt")
}

fn commit_diff_unfocused_not_popped(screen: &str) -> bool {
    panes_files_focused_diff_unfocused(screen)
        && title_has_diff(screen)
        && title_has_files(screen)
        && !title_has_graph(screen)
        && names_a_commit_path(screen)
        && last_crumb_unbracketed(screen)
        && crumb_row(screen).contains('›')
}

fn back_on_merger_graph(screen: &str) -> bool {
    (merger_graph_drilled_right(screen) || merger_graph_left_unfocused(screen))
        && !title_has_files(screen)
        && !still_file_diff(screen)
}

/// First Esc may clear an armed pane search. A second Esc runs only while
/// the right pane still holds keyboard focus, so the diff does not pop.
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

/// Enter drills graph → commit files → commit diff. Esc unfocuses, then pops.
///
/// Docs + VIEW: Enter is `focus right / drill`. Esc is `back / unfocus`
/// and never quits. Daily `merger`: `j`, Enter (graph), skip stash, Enter
/// (files), Enter (diff). First Esc unfocuses the diff without popping.
/// Next Esc pops to commit files. Next Esc pops to the graph. A no-op,
/// a stash drill, or one Esc burst back to the graph cannot pass.
#[test]
fn pty_graph_drill_esc_walk_commit_files_diff() {
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
        "j lands on merger and loads its graph (left focus, not yet Enter)",
        GIT_WAIT,
    );

    csi_u_enter(&mut tui);
    tui.wait_pred(
        merger_graph_drilled_right,
        "CSI-u Enter on merger focuses that graph (file-diff / files drill / no-op cannot pass)",
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
        "CSI-u Enter on the merge commit opens its file list (graph left, files right, right focused)",
        GIT_WAIT,
    );

    csi_u_enter(&mut tui);
    tui.wait_pred(
        commit_file_diff_right,
        "CSI-u Enter on a commit file opens the commit diff (files left, diff right, right focused)",
        WAIT,
    );

    unfocus_right(
        &mut tui,
        panes_files_unfocused_diff_focused,
        commit_diff_unfocused_not_popped,
        "Esc unfocuses the commit diff without popping (files left, diff right, left focused)",
    );

    tui.esc();
    tui.wait_pred(
        panes_graph_focused_files_unfocused,
        "Esc on the left files list pops to commit files (graph left, files right, left focused)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        back_on_merger_graph,
        "Esc on the left graph pops to the merger graph (not files, not the README file-diff)",
        WAIT,
    );
}

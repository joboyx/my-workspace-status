use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, graph_cursor_on, graph_pane_focused,
    merger_graph_drilled_right, merger_graph_left_unfocused, no_mouse_toggle_toast, right_pane,
    status_row, tree_cursor_on, tree_has, tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};

fn no_wrong_stash_pop_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("Drop stash@{")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("no visible repos for that op")
        && no_mouse_toggle_toast(screen)
}

fn after_wip_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "wip.txt")?;
    let at = line.find("wip.txt")?;
    Some(line[at + "wip.txt".len()..].to_string())
}

/// Restored `wip.txt` on the merger tree. Badge `A` is the staged add.
fn merger_wip_added(screen: &str) -> bool {
    tree_has(screen, "wip.txt")
        && tree_cursor_on(screen, "merger")
        && after_wip_name(screen).is_some_and(|after| after.contains('A'))
}

fn graph_stash_still_listed(screen: &str) -> bool {
    let right = right_pane(screen);
    right.contains("WIP on graph") || right.contains("stash@{0}")
}

fn no_pull_or_other_stash_write(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    !crumb.contains("Pulled")
        && !crumb.contains("applied")
        && !crumb.contains("dropped")
        && !crumb.contains("Stashed")
        && !crumb.contains("failed")
}

/// Tab focused the merger graph. HEAD is clean. Stash is listed. Pop idle.
fn graph_focused_merger_stash_listed(screen: &str) -> bool {
    merger_graph_drilled_right(screen)
        && graph_pane_focused(screen)
        && graph_stash_still_listed(screen)
        && !tree_has(screen, "wip.txt")
        && !crumb_row(screen).contains("popped")
        && no_pull_or_other_stash_write(screen)
        && no_wrong_stash_pop_overlays(screen)
}

/// Tab lands on the uncommitted row. Stash is the next `j`.
fn graph_focused_merger_before_stash_pop(screen: &str) -> bool {
    graph_focused_merger_stash_listed(screen)
        && graph_cursor_on(screen, "working tree")
        && !graph_cursor_on(screen, "WIP on graph")
}

/// Graph cursor on `stash@{0}` (`WIP on graph`). Hint `p` is pop stash.
fn stash_row_ready_to_pop(screen: &str) -> bool {
    let status = status_row(screen);
    graph_focused_merger_stash_listed(screen)
        && graph_cursor_on(screen, "WIP on graph")
        && !graph_cursor_on(screen, "working tree")
        && status.contains("apply stash")
        && status.contains("pop stash")
        && status.contains("drop stash")
        && !status.contains("pull")
}

/// Graph `p` popped `stash@{0}`: apply + drop. Apply-only / drop-only fail.
fn documented_graph_stash_pop(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "merger")
        && merger_wip_added(screen)
        && !graph_stash_still_listed(screen)
        && screen.contains("uncommitted changes")
        && !screen.contains("working tree clean")
        && crumb.contains("popped stash@{0}")
        && no_pull_or_other_stash_write(screen)
        && !status.contains("pop stash")
        && !status.contains("apply stash")
        && !status.contains("drop stash")
        && status.contains("drill")
        && status.contains(" tree")
        && status.contains(" split")
        && no_wrong_stash_pop_overlays(screen)
}

/// Graph `p` pops the focused stash (apply + drop).
///
/// Docs: Help GIT `a p D` = focused stash apply/pop/drop. Keymap: graph
/// stash row `p` is `Action::GraphStashPop` (`git stash pop` of that
/// `stash@{n}`). Workspace / tree `p` is `Action::Pull`
/// (`pty_pull_behind_local_remote`). Overlay `S` then `a` / `D` is
/// `pty_stash_create_apply_and_drop`. Pop runs immediately. Drop still
/// confirms.
///
/// After first paint, `j` lands on merger. Tab focuses the graph. `j`
/// selects `stash@{0}` (`WIP on graph`). `p` must restore `wip.txt` on
/// the merger tree, drop that stash from the graph, and toast `popped
/// stash@{0}`. A no-op, workspace pull, apply-only (stash stays),
/// drop-only (no `wip.txt`), overlay, or toast-only is red.
#[test]
fn pty_stash_graph_pop() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file diff (graph stash pop has not run)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph (left focus, stash still listed)",
        GIT_WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_focused_merger_before_stash_pop,
        "Tab focuses the merger graph: working tree clean, stash@{0} listed, pop idle",
        GIT_WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        stash_row_ready_to_pop,
        "j selects stash@{0}: graph cursor on WIP on graph; p pop stash hint",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        stash_row_ready_to_pop,
        "stash row holds (not a flicker); overlay closed; not pull",
        WAIT,
    );

    tui.key('p');
    tui.wait_pred(
        documented_graph_stash_pop,
        "graph p pops stash@{0}: wip.txt A on merger, stash gone, popped toast",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_graph_stash_pop,
        "popped paint holds (not a flicker, toast-only tick, or apply-only)",
        WAIT,
    );
}

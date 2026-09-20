use std::fs;

use crate::harness::{assert_contains, PtySession};
use crate::seed::{git, watch_pair_workspace};
use crate::support::{
    graph_cursor_on, graph_pane_focused, graph_subject_line, panes_tree_focused_graph_unfocused,
    tree_cursor_on, tree_has, tree_inactive_selection_on, GIT_WAIT, WAIT,
};

/// Subject of the seed commit on `alpha`. Must stay focused after a new HEAD.
const KEEP: &str = "seed alpha";
/// Subject of the silent disk commit that watch must paint without stealing focus.
const NEW_HEAD: &str = "watch-keep-graph-focus";

/// `r` reload toast. Watch apply must not paint this.
fn refresh_now_toast(screen: &str) -> bool {
    screen.contains("refreshed app") || screen.contains("refreshed workspace")
}

/// `/alpha` loaded the graph. Tree stays focused. New HEAD subject is absent.
fn alpha_graph_loaded_tree_focus(screen: &str) -> bool {
    tree_cursor_on(screen, "alpha")
        && !tree_cursor_on(screen, "beta")
        && tree_has(screen, "alpha")
        && tree_has(screen, "beta")
        && panes_tree_focused_graph_unfocused(screen)
        && screen.contains(KEEP)
        && screen.contains("Working tree clean")
        && !screen.contains(NEW_HEAD)
        && !refresh_now_toast(screen)
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && screen.contains("? help")
}

/// Graph is focused on the seed commit, not working tree and not the new HEAD.
fn graph_focused_on_keep_commit(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_inactive_selection_on(screen, "alpha")
        && graph_cursor_on(screen, KEEP)
        && !graph_cursor_on(screen, "working tree")
        && !graph_cursor_on(screen, NEW_HEAD)
        && screen.contains(KEEP)
        && !screen.contains(NEW_HEAD)
        && !refresh_now_toast(screen)
}

/// Watch painted the new HEAD. Focus stayed on the seed commit.
fn keeps_keep_commit_after_new_head(screen: &str) -> bool {
    graph_pane_focused(screen)
        && graph_cursor_on(screen, KEEP)
        && !graph_cursor_on(screen, NEW_HEAD)
        && !graph_cursor_on(screen, "working tree")
        && graph_subject_line(screen, NEW_HEAD).is_some()
        && screen.contains(KEEP)
        && !refresh_now_toast(screen)
}

/// Live watch reloads a silent HEAD move without jumping graph focus to the top.
///
/// Depth-0 graph (tree left, graph right): Tab focuses the graph, `/`
/// lands on a non-top commit, a disk commit prepends a new HEAD. Watch
/// must paint that subject and keep the cursor on the earlier commit.
/// A reset to working tree or to the new HEAD, a no-op until `r`, or a
/// refresh-now toast cannot pass.
#[test]
fn pty_watch_keeps_graph_focus() {
    let (_root, workspace) = watch_pair_workspace();
    let alpha = workspace.join("alpha");
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_pred(
        |screen| tree_has(screen, "alpha") && tree_has(screen, "beta") && screen.contains("? help"),
        "first paint: alpha and beta are on the tree, watch on",
        WAIT,
    );

    tui.search("alpha");
    tui.wait_pred(
        alpha_graph_loaded_tree_focus,
        "search focuses alpha and loads the graph; watch-keep-graph-focus is absent",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_pane_focused,
        "Tab focuses the graph pane",
        WAIT,
    );
    tui.search(KEEP);
    tui.wait_pred(
        graph_focused_on_keep_commit,
        "graph search lands on seed alpha (not working tree)",
        WAIT,
    );

    fs::write(alpha.join("tick.txt"), "keep-focus\n").unwrap();
    git(&alpha, &["add", "tick.txt"]);
    git(&alpha, &["commit", "-q", "-m", NEW_HEAD]);

    tui.wait_pred(
        keeps_keep_commit_after_new_head,
        "watch paints the new HEAD and keeps graph focus on seed alpha",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert_contains(&screen, NEW_HEAD);
    assert_contains(&screen, KEEP);
    assert!(
        graph_cursor_on(&screen, KEEP),
        "focused graph must stay on seed alpha after the new HEAD; screen:\n{screen}"
    );
    assert!(
        !graph_cursor_on(&screen, NEW_HEAD),
        "new HEAD must not steal graph focus; screen:\n{screen}"
    );
    assert!(
        !graph_cursor_on(&screen, "working tree"),
        "graph focus must not jump to the top working-tree row; screen:\n{screen}"
    );
    crate::harness::assert_absent(&screen, "refreshed app");
    crate::harness::assert_absent(&screen, "refreshed workspace");
}

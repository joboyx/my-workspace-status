use std::fs;

use crate::harness::{assert_contains, PtySession};
use crate::seed::{git, watch_pair_workspace};
use crate::support::{
    graph_pane_focused, graph_subject_line, panes_tree_focused_graph_unfocused, tree_cursor_on,
    tree_has, tree_inactive_selection_on, GIT_WAIT, WAIT,
};

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
        && screen.contains("seed alpha")
        && screen.contains("Working tree clean")
        && !screen.contains("watch-head-move")
        && !refresh_now_toast(screen)
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && screen.contains("? help")
}

/// Watch painted the first silent HEAD on the graph (tree still focused).
fn paints_watch_head_move(screen: &str) -> bool {
    graph_subject_line(screen, "watch-head-move").is_some_and(|line| {
        line.contains("@  watch-head-move") && !line.contains("watch-head-move-2")
    }) && screen.contains("seed alpha")
        && !screen.contains("watch-head-move-2")
        && !refresh_now_toast(screen)
}

/// Tab focused the graph after the first HEAD move.
fn graph_focused_after_first_head(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_inactive_selection_on(screen, "alpha")
        && paints_watch_head_move(screen)
}

/// Watch painted the second silent HEAD while the graph pane is focused.
fn paints_watch_head_move_2(screen: &str) -> bool {
    graph_subject_line(screen, "watch-head-move-2")
        .is_some_and(|line| line.contains("@  watch-head-move-2"))
        && screen.contains("watch-head-move")
        && !refresh_now_toast(screen)
}

/// Live watch reloads a silent HEAD move without `r` (tree, then graph).
///
/// `/alpha` loads the graph. A disk commit on `alpha` is a real poll
/// move (`HEAD` sits in `checkout_watch_identity`). Tab focuses the
/// graph; a second disk commit must paint too. A no-op, a refresh-now
/// toast, or a graph that stays on the old HEAD until `r` cannot pass.
#[test]
fn pty_watch_reloads_silent_head_move() {
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
        "search focuses alpha and loads the graph; watch-head-move is absent",
        WAIT,
    );

    fs::write(alpha.join("tick.txt"), "head-move\n").unwrap();
    git(&alpha, &["add", "tick.txt"]);
    git(&alpha, &["commit", "-q", "-m", "watch-head-move"]);

    tui.wait_pred(
        paints_watch_head_move,
        "watch paints watch-head-move on the graph without r or refresh toast",
        GIT_WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_focused_after_first_head,
        "Tab focuses the graph; first HEAD subject still painted",
        WAIT,
    );
    tui.wait_pred(
        |screen| !screen.contains("watch-head-move-2"),
        "second HEAD subject is absent before the disk commit",
        WAIT,
    );

    fs::write(alpha.join("tick.txt"), "head-move-2\n").unwrap();
    git(&alpha, &["add", "tick.txt"]);
    git(&alpha, &["commit", "-q", "-m", "watch-head-move-2"]);

    tui.wait_pred(
        paints_watch_head_move_2,
        "watch paints watch-head-move-2 on the focused graph without r or refresh toast",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert_contains(&screen, "watch-head-move-2");
    assert!(
        paints_watch_head_move_2(&screen),
        "focused graph must show the new HEAD subject (frozen graph until r fails); screen:\n{screen}"
    );
    crate::harness::assert_absent(&screen, "refreshed app");
    crate::harness::assert_absent(&screen, "refreshed workspace");
}

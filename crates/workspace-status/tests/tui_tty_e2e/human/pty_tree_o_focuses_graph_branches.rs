use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{
    crumb_line, focusbox_graph_left_full, focusbox_keep_only_graph_body, not_files_search_or_stash,
    panes_tree_focused_graph_unfocused, status_line, tree_cursor_on, GIT_WAIT, WAIT,
};

/// Overlay chrome from the tree. Graph-pane `o` stays on
/// `pty_graph_branch_focus_overlay`.
fn tree_graph_focus_overlay_open(screen: &str) -> bool {
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, "focusbox")
        && screen.contains("Focus branches")
        && screen.contains("filter:")
        && screen.contains("* feature/keep")
        && screen.contains("topic/noise")
        && screen.contains("Enter apply")
        && screen.contains("O clear")
        && screen.contains("Esc cancel")
        && screen.contains("workspace › focusbox")
        && !screen.contains("workspace › [focusbox]")
        && !screen.contains("graph focus:")
        && !screen.contains("Enter checkout")
        && !screen.contains("C create")
        && not_files_search_or_stash(screen)
}

fn tree_graph_focus_overlay_filtered_keep(screen: &str) -> bool {
    tree_graph_focus_overlay_open(screen)
        && screen.contains("filter: feature")
        && screen
            .lines()
            .any(|line| line.contains('❯') && line.contains("feature/keep"))
        && !screen.contains("[ ]   main")
}

fn tree_graph_focus_applied_keep(screen: &str) -> bool {
    let crumb = crumb_line(screen);
    let status = status_line(screen);
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, "focusbox")
        && crumb.contains("workspace › focusbox")
        && !crumb.contains("[focusbox]")
        && crumb.contains("graph focus: feature/keep")
        && status.contains("focus right")
        && !status.contains("drill")
        && !screen.contains("Focus branches")
        && !screen.contains("Enter apply")
        && focusbox_keep_only_graph_body(screen)
        && not_files_search_or_stash(screen)
}

/// Tree `o` on the highlighted repo: overlay, filter-apply `feature/keep`.
///
/// Docs: `o` opens from a highlighted repo / worktree, not only the graph
/// list. Tab, `b` checkout, files drill, `/` SEARCH, or a no-op cannot pass.
#[test]
fn pty_tree_o_focuses_graph_branches() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        focusbox_graph_left_full,
        "focusbox graph loaded on the left (full --all; o / Tab have not run)",
        GIT_WAIT,
    );

    tui.key('o');
    tui.wait_pred(
        tree_graph_focus_overlay_open,
        "tree o opens Focus branches without Tab (b picker / files / SEARCH cannot pass)",
        WAIT,
    );
    tui.keys("feature");
    tui.wait_pred(
        tree_graph_focus_overlay_filtered_keep,
        "typing feature filters the overlay onto feature/keep",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        tree_graph_focus_applied_keep,
        "Enter applies feature/keep from the tree: toast, keep-only graph, left still focused",
        GIT_WAIT,
    );
}

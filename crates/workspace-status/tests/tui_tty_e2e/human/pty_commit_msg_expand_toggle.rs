use crate::harness::{left_tree, PtySession};
use crate::seed::{daily_workspace, seed_multiline_message_repo, COMMIT_MSG_BODY};
use crate::support::{
    crumb_row, graph_cursor_on, no_wrong_overlays, panes_files_focused,
    panes_tree_focused_graph_unfocused, panes_tree_unfocused_graph_focused, right_pane, status_row,
    title_has_files, title_has_graph, tree_cursor_on, tree_has, tree_inactive_selection_on,
    SETTLE_MS, WAIT,
};

const REPO: &str = "longmsg";

fn help_lists_expand(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    screen.contains("VIEW")
        && compact.contains("i \\ M")
        && compact.contains("wrap")
        && compact.contains("msg")
        && compact.contains("inline")
}

fn long_msg_graph_tree_focus(screen: &str) -> bool {
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, REPO)
        && tree_has(screen, REPO)
        && right_pane(screen).contains("nnnn")
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && !left_tree(screen).contains(COMMIT_MSG_BODY)
        && !title_has_files(screen)
        && no_wrong_overlays(screen)
}

fn long_msg_graph_focused(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && tree_inactive_selection_on(screen, REPO)
        && right_pane(screen).contains("nnnn")
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && crumb_row(screen).contains("workspace › [longmsg]")
        && status_row(screen).contains("drill")
        && no_wrong_overlays(screen)
}

fn long_msg_commit_selected(screen: &str) -> bool {
    long_msg_graph_focused(screen)
        && graph_cursor_on(screen, "nnnn")
        && !graph_cursor_on(screen, "working tree")
        && !graph_cursor_on(screen, "root")
}

fn graph_msg_expanded(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && tree_inactive_selection_on(screen, REPO)
        && right_pane(screen).contains(COMMIT_MSG_BODY)
        && !left_tree(screen).contains(COMMIT_MSG_BODY)
        && crumb_row(screen).contains("msg on")
        && !crumb_row(screen).contains("msg off")
        && !title_has_files(screen)
        && no_wrong_overlays(screen)
}

fn files_msg_expanded(screen: &str) -> bool {
    panes_files_focused(screen)
        && title_has_files(screen)
        && title_has_graph(screen)
        && right_pane(screen).contains(COMMIT_MSG_BODY)
        && right_pane(screen).contains("wip.txt")
        && crumb_row(screen).contains("longmsg")
        && crumb_row(screen).contains('[')
        && !crumb_row(screen).contains("msg off")
        && no_wrong_overlays(screen)
}

fn files_msg_collapsed(screen: &str) -> bool {
    panes_files_focused(screen)
        && title_has_files(screen)
        && title_has_graph(screen)
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && right_pane(screen).contains("wip.txt")
        && crumb_row(screen).contains("msg off")
        && !crumb_row(screen).contains("msg on")
        && no_wrong_overlays(screen)
}

/// `M` expands the selected commit message on the graph footer and the
/// commit-files header. Collapse restores the dense clip.
///
/// Docs + help VIEW: `M` is expand commit message. Graph list rows stay
/// one line. Default clip hides `UNIQUE_MSG_BODY_LINE`. Expand paints that
/// body and toasts `msg on`. Enter keeps expand on the files header. A
/// second `M` hides the body again.
///
/// Live PTY (80×28 so the subject clips). A no-op, a graph-row wrap, or a
/// header-only toast cannot pass.
#[test]
fn pty_commit_msg_expand_toggle() {
    let (_root, workspace) = daily_workspace();
    seed_multiline_message_repo(&workspace, REPO);
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(
        help_lists_expand,
        "help VIEW lists i \\ M wrap · msg",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("inline / split · wrap · msg")
                && screen.contains("README.md")
                && screen.contains("? help")
        },
        "Esc closes help so M is expand, not a help key",
        WAIT,
    );

    tui.search(REPO);
    tui.wait_pred(
        long_msg_graph_tree_focus,
        "search loads the long-message graph; body is hidden",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        long_msg_graph_focused,
        "Tab focuses the graph; body stays hidden",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        long_msg_commit_selected,
        "j selects the long-subject tip (not working tree)",
        WAIT,
    );
    assert!(
        !graph_msg_expanded(&tui.screen()),
        "clipped graph must not satisfy the expanded claim:\n{}",
        tui.screen()
    );

    tui.key('M');
    tui.wait_pred(
        graph_msg_expanded,
        "M wraps the body onto the graph selection footer",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        graph_msg_expanded,
        "graph expand holds (not a flicker or toast-only)",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        files_msg_expanded,
        "Enter keeps the expanded message on the commit-files header",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        files_msg_collapsed,
        "second M hides the body on the commit-files header",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        files_msg_collapsed,
        "collapse holds (not a no-op second M)",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        files_msg_expanded,
        "third M shows the body on the commit-files header again",
        WAIT,
    );
}

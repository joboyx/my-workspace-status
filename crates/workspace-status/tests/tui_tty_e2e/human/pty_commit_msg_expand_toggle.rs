use crate::harness::{left_tree, PtySession, SGR_WHEEL_DOWN};
use crate::seed::{
    daily_workspace, seed_multiline_message_repo, COMMIT_MSG_BODY, COMMIT_MSG_BODY_TAIL,
};
use crate::support::{
    crumb_row, graph_cursor_on, no_wrong_overlays, panes_files_focused,
    panes_tree_focused_graph_unfocused, panes_tree_unfocused_graph_focused, right_pane, status_row,
    title_has_files, title_has_graph, tree_cursor_on, tree_has, tree_inactive_selection_on,
    SETTLE_MS, WAIT,
};

const REPO: &str = "longmsg";
/// xterm SGR button for wheel up (`ScrollUp`, `Cb` 64).
const SGR_WHEEL_UP: u8 = 64;

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
        && crumb_row(screen).contains("workspace › [longmsg]")
        && status_row(screen).contains("drill")
        && no_wrong_overlays(screen)
}

fn graph_msg_collapsed(screen: &str) -> bool {
    long_msg_commit_selected_expanded(screen)
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && crumb_row(screen).contains("msg off")
}

/// Expanded with no key press: body on the footer, no `msg on/off` toast.
fn graph_msg_default_expanded(screen: &str) -> bool {
    long_msg_commit_selected_expanded(screen)
        && right_pane(screen).contains(COMMIT_MSG_BODY)
        && !right_pane(screen).contains(COMMIT_MSG_BODY_TAIL)
        && !crumb_row(screen).contains("msg on")
        && !crumb_row(screen).contains("msg off")
}

/// Graph focused on the long-message commit (footer may show the body).
fn long_msg_commit_selected_expanded(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && tree_inactive_selection_on(screen, REPO)
        && graph_cursor_on(screen, "nnnn")
        && !left_tree(screen).contains(COMMIT_MSG_BODY)
        && !title_has_files(screen)
        && no_wrong_overlays(screen)
}

/// Wheel scrolled the footer to the end: tail in, first body line out, and
/// the list cursor did not move.
fn graph_msg_scrolled_to_tail(screen: &str) -> bool {
    long_msg_commit_selected_expanded(screen)
        && right_pane(screen).contains(COMMIT_MSG_BODY_TAIL)
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
}

/// Screen cell `(col, row)` of the first `needle` on the graph footer.
fn footer_cell(screen: &str, needle: &str) -> (u16, u16) {
    screen
        .lines()
        .enumerate()
        .find_map(|(row, line)| {
            let at = line.find(needle)?;
            Some((line[..at].chars().count() as u16, row as u16))
        })
        .unwrap_or_else(|| panic!("{needle} on screen:\n{screen}"))
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

/// The selected commit message is multiline by default on the graph footer
/// and the commit-files footer. The wheel scrolls a message taller than the
/// footer. `M` collapses to the dense clip and expands again.
///
/// Docs + help VIEW: `M` is the commit-message toggle. Graph list rows stay
/// one line. With no key, `j` onto the commit paints `UNIQUE_MSG_BODY_LINE`
/// but not `UNIQUE_MSG_BODY_TAIL`. Wheel down over the footer brings the
/// tail in without moving the list cursor; wheel up goes back. `M` hides the
/// body (`msg off`), `M` again shows it (`msg on`). Enter keeps expand on
/// the files footer, where `M` toggles it the same way.
///
/// Live PTY (80×28 so the subject clips). A collapsed default, a wheel that
/// moves the list, a graph-row wrap, or a toast-only toggle cannot pass.
#[test]
fn pty_commit_msg_expand_toggle() {
    let (_root, workspace) = daily_workspace();
    seed_multiline_message_repo(&workspace, REPO);
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(help_lists_expand, "help VIEW lists i \\ M wrap · msg", WAIT);
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
        graph_msg_default_expanded,
        "j onto the long-message tip shows the body with no key press",
        WAIT,
    );

    let (col, row) = footer_cell(&tui.screen(), COMMIT_MSG_BODY);
    for _ in 0..30 {
        tui.sgr_mouse(SGR_WHEEL_DOWN, col, row);
    }
    tui.wait_pred(
        graph_msg_scrolled_to_tail,
        "wheel over the footer scrolls the message to its tail",
        WAIT,
    );
    let (col, row) = footer_cell(&tui.screen(), COMMIT_MSG_BODY_TAIL);
    for _ in 0..30 {
        tui.sgr_mouse(SGR_WHEEL_UP, col, row);
    }
    tui.wait_pred(
        graph_msg_default_expanded,
        "wheel up scrolls the footer back to the first body line",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(graph_msg_collapsed, "M collapses the graph footer", WAIT);

    tui.key('M');
    tui.wait_pred(
        graph_msg_expanded,
        "second M wraps the body onto the graph selection footer again",
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
        "Enter keeps the expanded message on the commit-files footer",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        files_msg_collapsed,
        "second M hides the body on the commit-files footer",
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
        "third M shows the body on the commit-files footer again",
        WAIT,
    );
}

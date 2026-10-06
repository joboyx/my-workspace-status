use crate::harness::{left_tree, PtySession, SGR_WHEEL_DOWN};
use crate::seed::{
    daily_workspace, seed_multiline_message_repo, COMMIT_MSG_BODY, COMMIT_MSG_BODY_TAIL,
};
use crate::support::{
    crumb_row, graph_cursor_on, no_wrong_overlays, panes_files_focused,
    panes_files_unfocused_diff_focused, panes_tree_focused_graph_unfocused,
    panes_tree_unfocused_graph_focused, right_pane, status_row, title_has_diff, title_has_files,
    title_has_graph, tree_cursor_on, tree_has, tree_inactive_selection_on, SETTLE_MS, WAIT,
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

/// Right-pane row of the graph footer's first line: the first row with
/// `subject` that is not the cursor list row.
fn footer_top(screen: &str, subject: &str) -> Option<usize> {
    right_pane(screen)
        .lines()
        .position(|line| line.contains(subject) && !line.contains('\u{258C}'))
}

/// The one-line `root` commit is selected with its footer painted.
fn root_selected(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && graph_cursor_on(screen, "root")
        && footer_top(screen, "root").is_some()
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && no_wrong_overlays(screen)
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

/// Depth 1: the left graph footer carries the message; the right files pane
/// is a plain file list with no commit footer.
fn files_msg_expanded(screen: &str) -> bool {
    panes_files_focused(screen)
        && title_has_files(screen)
        && title_has_graph(screen)
        && left_tree(screen).contains(COMMIT_MSG_BODY)
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
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
        && !left_tree(screen).contains(COMMIT_MSG_BODY)
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && right_pane(screen).contains("wip.txt")
        && crumb_row(screen).contains("msg off")
        && !crumb_row(screen).contains("msg on")
        && no_wrong_overlays(screen)
}

/// Depth 2: the left files pane pins the message under `wip.txt`; the right
/// diff pane carries no commit message.
fn diff_msg_expanded(screen: &str) -> bool {
    let left = left_tree(screen);
    panes_files_unfocused_diff_focused(screen)
        && title_has_diff(screen)
        && left
            .find("wip.txt")
            .is_some_and(|file| left.find(COMMIT_MSG_BODY).is_some_and(|body| body > file))
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && !crumb_row(screen).contains("msg off")
        && no_wrong_overlays(screen)
}

fn diff_msg_collapsed(screen: &str) -> bool {
    panes_files_unfocused_diff_focused(screen)
        && title_has_diff(screen)
        && left_tree(screen).contains("wip.txt")
        && !left_tree(screen).contains(COMMIT_MSG_BODY)
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
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
/// tail in without moving the list cursor; wheel up goes back. `j` onto the
/// one-line `root` commit keeps the footer top row (fixed height); `+` / `-`
/// move it by one row with `msg lines <N>`. `M` hides the
/// body (`msg off`), `M` again shows it (`msg on`). Enter (depth 1) keeps
/// expand on the left graph footer; the right files pane shows no message,
/// and `M` toggles the graph footer the same way. Enter on `wip.txt` (depth
/// 2) pins the message under the left file list; the right diff shows none,
/// and `M` toggles that footer.
///
/// Live PTY (80×28 so the subject clips). A collapsed default, a wheel that
/// moves the list, a graph-row wrap, a toast-only toggle, or a message on
/// the depth 1 files pane or the depth 2 diff pane cannot pass.
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

    // Fixed height: the footer starts on the same row for the long message
    // and for the one-line `root` commit below it.
    let long_top = footer_top(&tui.screen(), "nnnn").expect("long footer top");
    tui.key('j');
    tui.wait_pred(root_selected, "j onto the one-line root commit", WAIT);
    assert_eq!(
        footer_top(&tui.screen(), "root"),
        Some(long_top),
        "a one-line message keeps the footer height:\n{}",
        tui.screen()
    );
    tui.key('k');
    tui.wait_pred(
        graph_msg_default_expanded,
        "k back onto the long message",
        WAIT,
    );
    assert_eq!(footer_top(&tui.screen(), "nnnn"), Some(long_top));

    // `+` grows the footer by one row, `-` shrinks it back.
    tui.key('+');
    tui.wait_pred(
        |screen| {
            crumb_row(screen).contains("msg lines 9")
                && footer_top(screen, "nnnn") == Some(long_top - 1)
        },
        "+ grows the footer by one message row",
        WAIT,
    );
    tui.key('-');
    tui.wait_pred(
        |screen| {
            crumb_row(screen).contains("msg lines 8")
                && footer_top(screen, "nnnn") == Some(long_top)
        },
        "- shrinks the footer back",
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
        "depth 1 keeps the expanded message on the left graph footer only",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        files_msg_collapsed,
        "depth 1 M hides the body on the graph footer",
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
        "depth 1 M shows the body on the graph footer again",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        diff_msg_expanded,
        "depth 2 pins the expanded message under the left file list",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        diff_msg_collapsed,
        "depth 2 M hides the body on the left files footer",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(diff_msg_collapsed, "depth 2 collapse holds", WAIT);

    tui.key('M');
    tui.wait_pred(
        diff_msg_expanded,
        "depth 2 M shows the body on the left files footer again",
        WAIT,
    );
}

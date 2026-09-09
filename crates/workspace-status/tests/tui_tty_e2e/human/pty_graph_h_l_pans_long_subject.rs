use crate::common::hscroll::GRAPH_HSCROLL_VISIBLE;
use crate::harness::{left_tree, PtySession};
use crate::seed::{daily_workspace, seed_long_subject_repo};
use crate::support::{
    crumb_row, graph_cursor_on, no_wrong_overlays, panes_tree_focused_graph_unfocused,
    panes_tree_unfocused_graph_focused, right_of_split, right_pane, status_row, title_has_files,
    tree_cursor_on, tree_has, tree_inactive_selection_on, SETTLE_MS, WAIT,
};

const REPO: &str = "longsubj";
/// Gap so the input thread does not drain the held-nav backlog as one move.
const REPEAT_GAP_MS: u64 = 50;

fn status_has_graph_tail(screen: &str) -> bool {
    status_row(screen).contains(GRAPH_HSCROLL_VISIBLE)
}

fn long_subject_clip_holds(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    tree_has(screen, REPO)
        && !tree_cursor_on(screen, "README.md")
        && left.contains(REPO)
        && !left.contains(GRAPH_HSCROLL_VISIBLE)
        && right.contains("nnnn")
        && !right.contains(GRAPH_HSCROLL_VISIBLE)
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !title_has_files(screen)
        && !status_has_graph_tail(screen)
        && no_wrong_overlays(screen)
}

/// Long graph subject is loaded and still clipped. Tree stays on the repo.
fn long_graph_clipped_tree_focus(screen: &str) -> bool {
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, REPO)
        && long_subject_clip_holds(screen)
}

/// Tab focused the overflowing graph. The long subject is still clipped.
fn long_graph_clipped_graph_focus(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    panes_tree_unfocused_graph_focused(screen)
        && tree_inactive_selection_on(screen, REPO)
        && !tree_cursor_on(screen, REPO)
        && long_subject_clip_holds(screen)
        && crumb.contains("workspace › [longsubj]")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !status.contains("focus right")
}

/// Held `l` revealed UNIQUE_GRAP on the graph, not the tree or status chip.
fn documented_hl_panned_graph(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    panes_tree_unfocused_graph_focused(screen)
        && tree_inactive_selection_on(screen, REPO)
        && !tree_cursor_on(screen, REPO)
        && tree_has(screen, REPO)
        && left.contains(REPO)
        && !left.contains(GRAPH_HSCROLL_VISIBLE)
        && right.contains(GRAPH_HSCROLL_VISIBLE)
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !title_has_files(screen)
        && !status_has_graph_tail(screen)
        && crumb.contains("workspace › [longsubj]")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !status.contains("focus right")
        && no_wrong_overlays(screen)
}

fn graph_still_focused_after_jk(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && tree_inactive_selection_on(screen, REPO)
        && !title_has_files(screen)
        && no_wrong_overlays(screen)
}

fn cursor_on_unique_grap_tip(screen: &str) -> bool {
    documented_hl_panned_graph(screen) && graph_cursor_on(screen, GRAPH_HSCROLL_VISIBLE)
}

/// After pan, `root` is scrolled off the label; the ASCII commit node `*` stays.
fn graph_cursor_on_panned_root(screen: &str) -> bool {
    screen.lines().any(|line| {
        let right = right_of_split(line);
        right.contains('\u{258C}')
            && right.contains('*')
            && !right.contains(GRAPH_HSCROLL_VISIBLE)
            && !right.contains('@')
    })
}

/// CSI-u press, spaced Repeat, release. The input thread drains a held-nav
/// burst after each move, so Repeats must not be written as one flush.
fn csi_u_hold_letter(tui: &mut PtySession, letter: char) {
    tui.letter_press(letter);
    tui.wait_ms(REPEAT_GAP_MS);
    tui.wait_pred_while(
        documented_hl_panned_graph,
        "CSI-u held l pans UNIQUE_GRAP onto the focused graph (right pane, not tree/chip)",
        WAIT,
        |tui| {
            tui.letter_repeat(letter);
            tui.wait_ms(REPEAT_GAP_MS);
        },
    );
    tui.csi_u(u32::from(letter.to_ascii_lowercase()), 1, 3);
}

/// CSI-u unmodified letter (`CSI code ; 1 : 1 u` press, `: 3` release).
fn csi_u_letter(tui: &mut PtySession, letter: char) {
    tui.letter_press(letter);
    tui.csi_u(u32::from(letter.to_ascii_lowercase()), 1, 3);
}

/// CSI-u `h` / `l` pan a focused overflowing graph subject.
///
/// Live PTY (80×28 so `UNIQUE_GRAP` clips). `/longsubj` loads the graph.
/// Do not `/` search the tail. Tab focuses the graph; the long subject
/// stays clipped. CSI-u held `l` (press, spaced Repeat, release) puts
/// `UNIQUE_GRAP` on the right pane, not the tree or the status chip.
/// CSI-u `j` then `k` keep the graph focused and move the graph cursor
/// (tip `UNIQUE_GRAP` → panned `root` node → tip). After pan the `root`
/// subject may be scrolled off; the ASCII `*` node stays. A raw `'l'`
/// byte is a different path. The file-diff PTY test does not cover this.
/// A no-op, a tree fold, a file-diff, or a chrome-only suffix cannot pass.
///
/// MYWS-005.
#[test]
fn pty_graph_h_l_pans_long_subject() {
    let (_root, workspace) = daily_workspace();
    seed_long_subject_repo(&workspace, REPO);
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.search(REPO);
    tui.wait_pred(
        long_graph_clipped_tree_focus,
        "search loads the clipped long-subject graph; tree stays focused on the repo",
        WAIT,
    );
    assert!(
        !documented_hl_panned_graph(&tui.screen()),
        "clipped tree-focus frame must not satisfy the panned graph claim:\n{}",
        tui.screen()
    );

    tui.tab();
    tui.wait_pred(
        long_graph_clipped_graph_focus,
        "Tab focuses the overflowing graph; the long subject is still clipped",
        WAIT,
    );
    assert!(
        !documented_hl_panned_graph(&tui.screen()),
        "clipped graph-focus frame must not satisfy the panned graph claim:\n{}",
        tui.screen()
    );

    csi_u_hold_letter(&mut tui, 'l');
    tui.wait_pred(
        documented_hl_panned_graph,
        "CSI-u held l pans UNIQUE_GRAP onto the right pane (tree and status chip stay clean)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_hl_panned_graph,
        "graph pan holds (not a flicker, tree fold, file-diff, or chrome-only suffix)",
        WAIT,
    );

    if !graph_cursor_on(&tui.screen(), GRAPH_HSCROLL_VISIBLE) {
        csi_u_letter(&mut tui, 'j');
        tui.wait_pred(
            cursor_on_unique_grap_tip,
            "j from working tree lands on the UNIQUE_GRAP tip before the leave-row claim",
            WAIT,
        );
    }

    csi_u_letter(&mut tui, 'j');
    tui.wait_pred(
        |screen| {
            graph_still_focused_after_jk(screen)
                && documented_hl_panned_graph(screen)
                && graph_cursor_on_panned_root(screen)
                && !graph_cursor_on(screen, GRAPH_HSCROLL_VISIBLE)
        },
        "CSI-u j leaves the UNIQUE_GRAP tip (typically onto root); graph stays focused",
        WAIT,
    );

    csi_u_letter(&mut tui, 'k');
    tui.wait_pred(
        |screen| {
            graph_still_focused_after_jk(screen)
                && documented_hl_panned_graph(screen)
                && graph_cursor_on(screen, GRAPH_HSCROLL_VISIBLE)
                && !graph_cursor_on_panned_root(screen)
        },
        "CSI-u k returns the graph cursor to the UNIQUE_GRAP tip; graph stays focused",
        WAIT,
    );
}

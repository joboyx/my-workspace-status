use crate::common::hscroll::GRAPH_HSCROLL_VISIBLE;
use crate::harness::{left_tree, PtySession, SGR_WHEEL_RIGHT, SGR_WHEEL_RIGHT_MOTION};
use crate::seed::{daily_workspace, seed_long_subject_repo};
use crate::support::{
    no_wrong_overlays, panes_tree_focused_graph_unfocused, right_pane, status_row, title_has_files,
    tree_cursor_on, tree_has, SETTLE_MS, WAIT,
};

const REPO: &str = "longsubj";
/// Right pane on an 80-col layout (tree fraction 0.4 → `right_x` ≈ 32).
const NARROW_RIGHT_COL: u16 = 50;
/// Graph body row (below the pane title). Same cell as keep-middle wheel.
const GRAPH_BODY_ROW: u16 = 8;

fn status_has_graph_tail(screen: &str) -> bool {
    status_row(screen).contains(GRAPH_HSCROLL_VISIBLE)
}

/// Long graph subject is loaded and still clipped. Tree stays on the repo.
fn long_graph_clipped_tree_focus(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, REPO)
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, REPO)
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

/// Documented right-pane trackpad hscroll: the long graph subject panned.
///
/// `UNIQUE_GRAP` is on the right pane, not the tree or the search chip.
/// Tree repo row and left focus stay. Vertical keep-middle must not run
/// (the focused graph row does not jump).
fn documented_right_pane_graph_sgr_hscroll_panned(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, REPO)
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, REPO)
        && left.contains(REPO)
        && !left.contains(GRAPH_HSCROLL_VISIBLE)
        && right.contains(GRAPH_HSCROLL_VISIBLE)
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !title_has_files(screen)
        && !status_has_graph_tail(screen)
        && no_wrong_overlays(screen)
}

/// Trackpad hscroll over the right pane pans a long graph subject.
///
/// Docs / keymap: write xterm SGR wheel right (`CSI < 67`) into the live
/// `event::read` loop. Motion-bit `CSI < 99` is dropped by crossterm 0.28
/// and must not pan. Horizontal wheel pans the pane under the pointer
/// without moving the focused row or stealing keyboard focus. Keys
/// `h` / `l` already pan a focused graph. Headless
/// `mouse_hscroll_pans_graph_and_shows_horizontal_bar` is not this proof.
///
/// Live PTY (80×28 so `UNIQUE_GRAP` clips): `/longsubj` loads the graph.
/// Do not `/` search the tail. Wheel over the right pane must put
/// `UNIQUE_GRAP` on the right pane, keep tree focus, and leave the
/// status chip without the tail. A no-op, a motion-bit-only pan, a tree
/// pan, vertical keep-middle, focus steal, or paint-only flicker is red.
#[test]
fn pty_right_pane_sgr_hscroll_pans_graph() {
    let (_root, workspace) = daily_workspace();
    seed_long_subject_repo(&workspace, REPO);
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.search(REPO);
    tui.wait_pred(
        long_graph_clipped_tree_focus,
        "search loads the clipped long-subject graph; tree stays focused on the repo",
        WAIT,
    );

    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT_MOTION, NARROW_RIGHT_COL, GRAPH_BODY_ROW);
    }
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        long_graph_clipped_tree_focus,
        "motion-bit CSI < 99 must not pan the graph or the tree",
        WAIT,
    );

    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, NARROW_RIGHT_COL, GRAPH_BODY_ROW);
    }
    tui.wait_pred(
        documented_right_pane_graph_sgr_hscroll_panned,
        "SGR 67 over the right pane pans the long graph subject (UNIQUE_GRAP; tree stays)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_right_pane_graph_sgr_hscroll_panned,
        "graph pan holds (not a flicker, tree pan, keep-middle jump, or focus steal)",
        WAIT,
    );
}

use crate::common::hscroll::GRAPH_HSCROLL_VISIBLE;
use crate::harness::{left_tree, PtySession, SGR_WHEEL_RIGHT};
use crate::seed::{daily_workspace, seed_long_subject_repo};
use crate::support::{
    no_mouse_toggle_toast, no_wrong_overlays, panes_tree_focused_graph_unfocused,
    panes_tree_unfocused_graph_focused, right_pane, status_row, title_has_files, tree_cursor_on,
    tree_has, tree_inactive_selection_on, SETTLE_MS, WAIT,
};

const REPO: &str = "longsubj";
/// Right pane on an 80-col layout (tree fraction 0.4 → `right_x` ≈ 32).
const NARROW_RIGHT_COL: u16 = 50;
/// Graph body row (below the pane title). Same cell as keep-middle wheel.
const GRAPH_BODY_ROW: u16 = 8;
/// xterm SGR left-button drag (`Cb` 0 + motion bit 32).
const SGR_LEFT_DRAG: u8 = 32;
const THUMB: char = '█';

fn status_has_graph_tail(screen: &str) -> bool {
    status_row(screen).contains(GRAPH_HSCROLL_VISIBLE)
}

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

/// Painted graph horizontal bar after a small pan. UNIQUE_GRAP still clipped.
fn hbar_painted_tail_clipped(screen: &str) -> bool {
    let right = right_pane(screen);
    tree_has(screen, REPO)
        && right.contains("nnnn")
        && right.contains(THUMB)
        && hbar_span(screen).is_some()
        && !right.contains(GRAPH_HSCROLL_VISIBLE)
        && !left_tree(screen).contains(GRAPH_HSCROLL_VISIBLE)
        && !status_has_graph_tail(screen)
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

fn hbar_drag_panned(screen: &str) -> bool {
    let right = right_pane(screen);
    panes_tree_unfocused_graph_focused(screen)
        && tree_has(screen, REPO)
        && (tree_inactive_selection_on(screen, REPO) || tree_cursor_on(screen, REPO))
        && right.contains(GRAPH_HSCROLL_VISIBLE)
        && right.contains(THUMB)
        && !left_tree(screen).contains(GRAPH_HSCROLL_VISIBLE)
        && !status_has_graph_tail(screen)
        && !title_has_files(screen)
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

/// First thumb cell and last track cell (`█` / `═` / `─`) on the h-bar row.
fn hbar_span(screen: &str) -> Option<(u16, u16, u16)> {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    for (y, line) in lines.iter().take(end).enumerate().rev() {
        let chars: Vec<char> = line.chars().collect();
        let thumbs: Vec<u16> = chars
            .iter()
            .enumerate()
            .filter(|(_, ch)| **ch == THUMB)
            .map(|(x, _)| x as u16)
            .collect();
        if thumbs.is_empty() {
            continue;
        }
        let has_track = chars.iter().any(|ch| matches!(*ch, '═' | '─')) || thumbs.len() >= 2;
        if !has_track {
            continue;
        }
        let last = chars
            .iter()
            .enumerate()
            .rfind(|(_, ch)| matches!(**ch, '█' | '═' | '─'))
            .map(|(x, _)| x as u16)?;
        return Some((y as u16, thumbs[0], last));
    }
    None
}

fn sgr_release(tui: &mut PtySession, col: u16, row: u16) {
    let seq = format!(
        "\x1b[<0;{};{}m",
        col.saturating_add(1),
        row.saturating_add(1)
    );
    tui.send_bytes(seq.as_bytes());
}

/// Drag the painted graph horizontal scrollbar. Not wheel, not keys.
///
/// Graph h-bar thumb drag already works via `SplitHit::GraphHThumb` /
/// `SplitDrag::GraphHScrollbar`. This is the live PTY counterpart of
/// `graph_horizontal_scrollbar_thumb_drag_updates_offset`.
///
/// Live PTY (80×28 so `UNIQUE_GRAP` clips): `/longsubj` loads the graph.
/// Wheel until `█` paints with the tail still clipped. SGR press on the
/// thumb must not jump. Motion-bit 32 drag to the track end must put
/// `UNIQUE_GRAP` on the right pane and focus the graph. A no-op,
/// paint-only flicker, keep-middle jump, or tree pan cannot pass.
#[test]
fn pty_graph_hscrollbar_thumb_drag() {
    let (_root, workspace) = daily_workspace();
    seed_long_subject_repo(&workspace, REPO);
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.search(REPO);
    tui.wait_pred(
        long_graph_clipped_tree_focus,
        "search loads the clipped long-subject graph; tree stays focused on the repo",
        WAIT,
    );

    tui.wait_pred_while(
        hbar_painted_tail_clipped,
        "small SGR 67 pan paints the graph h-bar without UNIQUE_GRAP",
        WAIT,
        |tui| tui.sgr_mouse(SGR_WHEEL_RIGHT, NARROW_RIGHT_COL, GRAPH_BODY_ROW),
    );
    let before = tui.screen();
    let (bar_row, thumb_col, track_end) =
        hbar_span(&before).unwrap_or_else(|| panic!("graph h-bar █/═ after small pan:\n{before}"));
    assert!(
        track_end > thumb_col,
        "h-bar track {thumb_col}..{track_end} is too short to drag:\n{before}"
    );

    tui.sgr_mouse(0, thumb_col, bar_row);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        hbar_painted_tail_clipped,
        "thumb grab must not jump (a track click would reveal UNIQUE_GRAP)",
        WAIT,
    );

    tui.sgr_mouse(SGR_LEFT_DRAG, track_end, bar_row);
    sgr_release(&mut tui, track_end, bar_row);
    tui.wait_pred(
        hbar_drag_panned,
        "h-thumb drag pans UNIQUE_GRAP onto the graph (a no-op stays clipped)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        hbar_drag_panned,
        "graph pan after h-thumb drag holds (not a flicker, tree pan, or Mouse toast)",
        WAIT,
    );
}

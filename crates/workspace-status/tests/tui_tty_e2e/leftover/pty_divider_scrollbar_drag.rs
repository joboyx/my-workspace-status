use std::fs;
use std::path::PathBuf;

use crate::harness::{tree_row_containing, PtySession, COLS};
use crate::seed::{daily_workspace, seed_tall_graph, unique_root};
use crate::support::{
    crumb_row, documented_launch_first_paint, no_mouse_toggle_toast, no_wrong_overlays,
    panes_tree_focused_diff_unfocused, panes_tree_focused_graph_unfocused,
    panes_tree_unfocused_graph_focused, status_row, title_has_graph, tree_cursor_on, tree_has,
    GIT_WAIT, SETTLE_MS, TREE_LABEL_COL, WAIT,
};

/// xterm SGR left-button drag (`Cb` 0 + motion bit 32).
const SGR_LEFT_DRAG: u8 = 32;

/// Thumb glyph ratatui paints on a vertical graph scrollbar.
const GRAPH_THUMB: char = '█';

/// How far right a pane-divider drag must move the join (cells).
const DIVIDER_DRAG_DELTA: u16 = 24;

fn tall_graph_workspace() -> (PathBuf, PathBuf) {
    let root = unique_root("ws-tui-tty-tall-graph");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_tall_graph(&workspace, "history");
    (root, workspace)
}

/// 0-based column of the tree/right box join (`┐┌` / `││`).
fn pane_join_col(screen: &str) -> Option<u16> {
    let line = screen.lines().next()?;
    for sep in ["┐┌", "││", "┘└"] {
        if let Some(byte_at) = line.find(sep) {
            return Some(line[..byte_at].chars().count() as u16);
        }
    }
    screen.lines().skip(1).find_map(|line| {
        line.find("││")
            .map(|byte_at| line[..byte_at].chars().count() as u16)
    })
}

fn sgr_release(tui: &mut PtySession, col: u16, row: u16) {
    let seq = format!(
        "\x1b[<0;{};{}m",
        col.saturating_add(1),
        row.saturating_add(1)
    );
    tui.send_bytes(seq.as_bytes());
}

/// Left press, motion-bit drag, release. Same bytes a 1002 SGR terminal sends.
fn sgr_drag(tui: &mut PtySession, from_col: u16, from_row: u16, to_col: u16, to_row: u16) {
    tui.sgr_mouse(0, from_col, from_row);
    tui.sgr_mouse(SGR_LEFT_DRAG, to_col, to_row);
    sgr_release(tui, to_col, to_row);
}

fn launch_tree_diff_chrome(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && status_row(screen).contains("focus right")
        && !status_row(screen).contains("drill")
        && crumb_row(screen).trim() == "workspace"
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

fn divider_dragged_wider(screen: &str, start_join: u16) -> bool {
    let Some(join) = pane_join_col(screen) else {
        return false;
    };
    launch_tree_diff_chrome(screen)
        && join >= start_join.saturating_add(DIVIDER_DRAG_DELTA)
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && tree_has(screen, "app")
}

fn graph_thumb_cells(screen: &str) -> Vec<(u16, u16)> {
    screen
        .lines()
        .enumerate()
        .flat_map(|(y, line)| {
            line.chars()
                .enumerate()
                .filter_map(move |(x, ch)| (ch == GRAPH_THUMB).then_some((x as u16, y as u16)))
        })
        .collect()
}

fn bottom_graph_thumb(screen: &str) -> Option<(u16, u16)> {
    graph_thumb_cells(screen)
        .into_iter()
        .max_by_key(|(_, y)| *y)
}

/// First and last scrollbar cells (`║` track or `█` thumb) on `thumb_col`.
///
/// After `G` the thumb sits mid-track with dead `║` below it. Grab-delta maps
/// from the press row, so a drag must start on the last track cell (same as
/// headless `track_y + track_h - 1`), not on that mid-track `█`.
fn scrollbar_track_span(screen: &str, thumb_col: u16) -> Option<(u16, u16)> {
    let mut top = None;
    let mut bottom = None;
    for (y, line) in screen.lines().enumerate() {
        match line.chars().nth(thumb_col as usize) {
            Some('║' | '█') => {
                let y = y as u16;
                if top.is_none() {
                    top = Some(y);
                }
                bottom = Some(y);
            }
            _ => {}
        }
    }
    Some((top?, bottom?))
}

fn history_graph_at_top(screen: &str) -> bool {
    screen.contains("count 29")
        && (screen.contains("Working tree") || screen.contains("working tree clean"))
        && tree_has(screen, "history")
        && title_has_graph(screen)
        && graph_thumb_cells(screen).is_empty()
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

fn history_graph_at_bottom(screen: &str) -> bool {
    screen.contains("count 0")
        && !screen.contains("count 29")
        && tree_has(screen, "history")
        && title_has_graph(screen)
        && bottom_graph_thumb(screen).is_some()
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

/// Drag the pane divider or a graph scrollbar. Not click, not wheel.
///
/// Help VIEW: `m` = mouse · drag pane, split, or graph scrollbars. Docs:
/// drag the tree / right splitter to resize (3-column grab). Drag a graph
/// scrollbar thumb to scroll; click the track to jump. `m` itself is the
/// capture toggle leftover. In-diff RULE needs a wide pane (`NARROW_SXS`).
///
/// Live PTY, xterm SGR press + `Cb` 32 drag + release:
/// 1. Divider: `┐┌` moves at least 24 cells right. README stays. No
///    focus steal, no Mouse toast.
/// 2. Graph: `G` on overflowing `history` paints `█`. Drag from the last
///    `║`/`█` track cell to the first restores `count 29` and hides the
///    bar. Track click jumps. A mid-track `█` grab cannot reach the top.
/// A no-op, row-select, pane-steal, or chrome flicker cannot pass.
#[test]
fn pty_divider_scrollbar_drag() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file-diff (drag has not run)",
        WAIT,
    );
    let start_join = pane_join_col(&tui.screen())
        .unwrap_or_else(|| panic!("tree/right join on first paint:\n{}", tui.screen()));
    assert!(
        start_join > TREE_LABEL_COL && start_join < COLS.saturating_sub(20),
        "join {start_join} is not a pane divider:\n{}",
        tui.screen()
    );
    let drag_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row:\n{}", tui.screen()));
    let dest_col = start_join.saturating_add(DIVIDER_DRAG_DELTA);
    sgr_drag(&mut tui, start_join, drag_row, dest_col, drag_row);
    tui.wait_pred(
        |screen| divider_dragged_wider(screen, start_join),
        "SGR divider drag widens the tree pane (a no-op keeps the launch join)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| divider_dragged_wider(screen, start_join),
        "wider split holds (not a flicker, row-select, or Mouse toast)",
        WAIT,
    );

    let (_tall_root, tall) = tall_graph_workspace();
    let mut graph = PtySession::open(&tall);
    graph.wait_contains("history", WAIT);
    graph.wait_pred(
        |screen| {
            tree_cursor_on(screen, "history")
                && screen.contains("count 29")
                && (screen.contains("Working tree") || screen.contains("working tree clean"))
                && panes_tree_focused_graph_unfocused(screen)
        },
        "tall history paints a graph that still fits at the top (no scrollbar yet)",
        GIT_WAIT,
    );
    graph.tab();
    graph.wait_pred(
        |screen| panes_tree_unfocused_graph_focused(screen) && screen.contains("count 29"),
        "Tab focuses the graph (G on the tree would jump the workspace list)",
        WAIT,
    );
    graph.key('G');
    graph.wait_pred(
        history_graph_at_bottom,
        "G scrolls the overflowing graph and paints a █ thumb (a no-op stays on count 29)",
        GIT_WAIT,
    );
    let after_g = graph.screen();
    let (thumb_col, thumb_row) =
        bottom_graph_thumb(&after_g).unwrap_or_else(|| panic!("█ thumb after G:\n{after_g}"));
    let (track_top, track_bottom) = scrollbar_track_span(&after_g, thumb_col)
        .unwrap_or_else(|| panic!("║/█ track at col={thumb_col}:\n{after_g}"));
    assert!(
        track_bottom.saturating_sub(track_top) > 8,
        "track {track_top}..{track_bottom} is too short to drag:\n{after_g}"
    );
    assert!(
        track_bottom > thumb_row,
        "track end {track_bottom} must sit below mid-track █ {thumb_row} (grab-delta from the thumb cannot reach count 29):\n{after_g}"
    );
    graph.sgr_mouse(0, thumb_col, track_bottom);
    graph.wait_ms(SETTLE_MS);
    graph.wait_pred(
        history_graph_at_bottom,
        "grab at the last track cell must not jump (a click on the top would leave count 0)",
        WAIT,
    );
    graph.sgr_mouse(SGR_LEFT_DRAG, thumb_col, track_top);
    sgr_release(&mut graph, thumb_col, track_top);
    graph.wait_pred(
        history_graph_at_top,
        "track drag toward the top restores count 29 and hides █ (a no-op stays on count 0)",
        GIT_WAIT,
    );
    graph.wait_ms(SETTLE_MS);
    graph.wait_pred(
        history_graph_at_top,
        "graph scroll after track drag holds (not a chrome flicker)",
        WAIT,
    );

    graph.key('G');
    graph.wait_pred(
        history_graph_at_bottom,
        "G returns to count 0 before the track-click jump",
        GIT_WAIT,
    );
    graph.sgr_click(thumb_col, track_top);
    graph.wait_pred(
        |screen| {
            screen.contains("count 29")
                && (screen.contains("Working tree") || screen.contains("working tree clean"))
                && tree_has(screen, "history")
                && no_wrong_overlays(screen)
        },
        "track click toward the top jumps the graph (a dead click stays on count 0)",
        GIT_WAIT,
    );
}

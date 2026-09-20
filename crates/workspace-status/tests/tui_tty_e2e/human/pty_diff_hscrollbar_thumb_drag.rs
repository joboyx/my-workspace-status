use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{
    left_tree, tree_cursor_bar_on_row, tree_row_containing, PtySession, SGR_WHEEL_RIGHT,
};
use crate::seed::{daily_workspace, seed_long_diff_file};
use crate::support::{
    no_mouse_toggle_toast, no_wrong_overlays, panes_tree_focused_diff_unfocused,
    panes_tree_unfocused_diff_focused, right_pane, status_row, title_has_files, tree_cursor_on,
    tree_has, tree_inactive_selection_on, SETTLE_MS, WAIT,
};

const FILE: &str = "unique-diffline.rs";
/// Right pane on an 80-col layout (tree fraction 0.4 → `right_x` ≈ 32).
const NARROW_RIGHT_COL: u16 = 50;
/// xterm SGR left-button drag (`Cb` 0 + motion bit 32).
const SGR_LEFT_DRAG: u8 = 32;
const THUMB: char = '█';

fn status_has_diff_tail(screen: &str) -> bool {
    status_row(screen).contains(DIFF_HSCROLL_TAIL)
}

fn long_diff_clipped_tree_focus(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, FILE)
        && left.contains(FILE)
        && !left.contains(DIFF_HSCROLL_TAIL)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains("nnnn")
        && right.contains("inline (too narrow)")
        && !right.contains("inline (too narrow) ·")
        && !right.contains(DIFF_HSCROLL_TAIL)
        && !right.contains(THUMB)
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
        && no_wrong_overlays(screen)
}

/// Painted horizontal bar after a small pan. Tail still clipped.
fn hbar_painted_tail_clipped(screen: &str) -> bool {
    let right = right_pane(screen);
    tree_has(screen, FILE)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains("inline (too narrow) ·")
        && right.contains(THUMB)
        && hbar_span(screen).is_some()
        && !right.contains(DIFF_HSCROLL_TAIL)
        && !left_tree(screen).contains(DIFF_HSCROLL_TAIL)
        && !status_has_diff_tail(screen)
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

fn hbar_drag_panned(screen: &str) -> bool {
    let right = right_pane(screen);
    panes_tree_unfocused_diff_focused(screen)
        && tree_has(screen, FILE)
        && (tree_inactive_selection_on(screen, FILE) || tree_cursor_on(screen, FILE))
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains(DIFF_HSCROLL_TAIL)
        && right.contains("inline (too narrow) ·")
        && right.contains(THUMB)
        && !left_tree(screen).contains(DIFF_HSCROLL_TAIL)
        && !status_has_diff_tail(screen)
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

/// Drag the painted file-diff horizontal scrollbar. Not wheel, not keys.
///
/// Help VIEW: `m` = mouse · pane/split/bars. File-diff paints
/// `ScrollbarOrientation::HorizontalBottom` after pan leaves column 0.
/// `hit_split` / `SplitDrag::DiffHScrollbar` must change `diff_col_offset`.
///
/// Live PTY (80×24 so the NEW line clips): `/unique-diffline` loads the
/// file. Wheel until `█` paints with the tail still clipped. SGR press on
/// the thumb must not jump. Motion-bit 32 drag to the track end must put
/// `UNIQUE_DIFF_TAIL` on the right pane and focus the diff. A no-op,
/// paint-only flicker, cursor jump, or tree pan cannot pass.
#[test]
fn pty_diff_hscrollbar_thumb_drag() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace, FILE, DIFF_HSCROLL_TAIL);
    let mut tui = PtySession::open_size(&workspace, 80, 24);
    tui.search("unique-diffline");
    tui.wait_pred(
        long_diff_clipped_tree_focus,
        "search loads the clipped NEW file-diff; tree stays focused on the file",
        WAIT,
    );
    let row = tree_row_containing(&tui.screen(), "unique-diffline")
        .unwrap_or_else(|| panic!("long diff file row:\n{}", tui.screen()));
    assert!(
        tree_cursor_bar_on_row(&tui.screen(), row),
        "wheel aims at the focused file row, right-pane column:\n{}",
        tui.screen()
    );

    tui.wait_pred_while(
        hbar_painted_tail_clipped,
        "small SGR 67 pan paints the file-diff h-bar without UNIQUE_DIFF_TAIL",
        WAIT,
        |tui| tui.sgr_mouse(SGR_WHEEL_RIGHT, NARROW_RIGHT_COL, row),
    );
    let before = tui.screen();
    let (bar_row, thumb_col, track_end) = hbar_span(&before)
        .unwrap_or_else(|| panic!("file-diff h-bar █/═ after small pan:\n{before}"));
    assert!(
        track_end > thumb_col,
        "h-bar track {thumb_col}..{track_end} is too short to drag:\n{before}"
    );

    tui.sgr_mouse(0, thumb_col, bar_row);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        hbar_painted_tail_clipped,
        "thumb grab must not jump (a track click would reveal UNIQUE_DIFF_TAIL)",
        WAIT,
    );

    tui.sgr_mouse(SGR_LEFT_DRAG, track_end, bar_row);
    sgr_release(&mut tui, track_end, bar_row);
    tui.wait_pred(
        hbar_drag_panned,
        "h-thumb drag pans UNIQUE_DIFF_TAIL onto the file-diff (a no-op stays clipped)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        hbar_drag_panned,
        "file-diff pan after h-thumb drag holds (not a flicker, tree pan, or Mouse toast)",
        WAIT,
    );
}

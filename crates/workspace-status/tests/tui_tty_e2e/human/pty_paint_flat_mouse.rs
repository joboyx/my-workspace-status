use std::time::{Duration, Instant};

use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{PtySession, UserConfig, COLS, ROWS, SGR_WHEEL_RIGHT};
use crate::seed::{daily_workspace, seed_long_diff_file, unwrap_diffs_at_launch};
use crate::support::{
    crumb_row, flat_join_col, flat_left_body, flat_left_cursor_on, flat_left_row_containing,
    flat_pane_last_row, flat_right_body, status_row, FLAT_PAD_COLS, SETTLE_MS, SLATE_SIDEBAR, WAIT,
};

/// xterm SGR left-button drag (`Cb` 0 + motion bit 32).
const SGR_LEFT_DRAG: u8 = 32;

/// How far a pane-divider drag moves the join (cells).
const DIVIDER_DRAG_DELTA: u16 = 24;

/// Cells of slack on a divider bound: the split is a fraction of the width.
const DIVIDER_SLACK: u16 = 1;

/// Long-line file for the diff h-bar ([`seed_long_diff_file`]).
const LONG_DIFF_FILE: &str = "unique-diffline.rs";

fn sgr_release(tui: &mut PtySession, col: u16, row: u16) {
    let seq = format!(
        "\x1b[<0;{};{}m",
        col.saturating_add(1),
        row.saturating_add(1)
    );
    tui.send_bytes(seq.as_bytes());
}

/// Left press, motion-bit drag, release. Same bytes a 1002 SGR terminal sends.
fn sgr_drag(tui: &mut PtySession, from: (u16, u16), to: (u16, u16)) {
    tui.sgr_mouse(0, from.0, from.1);
    tui.sgr_mouse(SGR_LEFT_DRAG, to.0, to.1);
    sgr_release(tui, to.0, to.1);
}

/// Shipped look (no user config): Slate, flat panes.
fn open_flat(workspace: &std::path::Path, cols: u16, rows: u16) -> PtySession {
    PtySession::open_size_with_config(workspace, cols, rows, &[], UserConfig::Own)
}

/// Launch frame on the daily seed: tree focused on README, its diff right.
fn flat_launch(tui: &PtySession) -> bool {
    let Some(join) = flat_join_col(tui, SLATE_SIDEBAR) else {
        return false;
    };
    let screen = tui.screen();
    let left = flat_left_body(tui, join);
    let right = flat_right_body(tui, join);
    left.contains("README.md")
        && left.contains("merger")
        && flat_left_cursor_on(tui, join, "README.md")
        && right.contains("+dirty")
        && right.contains("UNSTAGED")
        && crumb_row(&screen).trim() == "workspace › app"
        && status_row(&screen).contains("focus right")
}

fn wait_flat_launch(tui: &PtySession) -> u16 {
    tui.wait_pred(
        |_| flat_launch(tui),
        "first paint: flat panes, README diff on the right",
        WAIT,
    );
    flat_join_col(tui, SLATE_SIDEBAR).unwrap()
}

/// A click on the first column of a tree row selects that row.
///
/// Flat panes have no border: column 0 is the row's left pad cell (where
/// the selected row's `▌` sits), and a pad cell maps to its row. Live PTY:
/// SGR press + release at column 0 of the merger row moves the cursor
/// there and loads that repo's graph; focus stays left. A click dropped as
/// a border hit leaves README selected.
#[test]
fn pty_paint_flat_click_first_column_selects_row() {
    let (_root, workspace) = daily_workspace();
    let mut tui = open_flat(&workspace, COLS, ROWS);
    let join = wait_flat_launch(&tui);
    let row = flat_left_row_containing(&tui, join, "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));

    tui.sgr_click(0, row);
    let merger_selected = |tui: &PtySession| {
        let screen = tui.screen();
        flat_left_cursor_on(tui, join, "merger")
            && !flat_left_cursor_on(tui, join, "README.md")
            && flat_right_body(tui, join).contains("WIP on graph")
            && crumb_row(&screen).contains("workspace › merger")
            && status_row(&screen).contains("focus right")
    };
    tui.wait_pred(
        |_| merger_selected(&tui),
        "column-0 click selects merger and loads its graph",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert!(
        merger_selected(&tui) && tui.clipboard_payloads().is_empty(),
        "selection holds and a click copies nothing:\n{}",
        tui.screen()
    );
}

/// Dragging either column of the flat divider band resizes the panes.
///
/// Flat mode grabs the two columns at the pane boundary: the left pane's
/// last column and the right pane's first column. Live PTY: a drag from the
/// left pane's last column moves the join right by the drag; a drag from
/// the right pane's first column moves it back left. The split follows the
/// pointer (the left pane ends at the pointer column), so each bound allows
/// one cell of rounding. The tree cursor stays on README and nothing is
/// copied (the band drag is not a text selection).
#[test]
fn pty_paint_flat_divider_drag_resizes_panes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = open_flat(&workspace, COLS, ROWS);
    let start = wait_flat_launch(&tui);
    let row = flat_left_row_containing(&tui, start, "README.md")
        .unwrap_or_else(|| panic!("README row:\n{}", tui.screen()));

    sgr_drag(
        &mut tui,
        (start - 1, row),
        (start - 1 + DIVIDER_DRAG_DELTA, row),
    );
    tui.wait_pred(
        |_| {
            flat_join_col(&tui, SLATE_SIDEBAR)
                .is_some_and(|join| join + DIVIDER_SLACK >= start + DIVIDER_DRAG_DELTA)
                && flat_launch(&tui)
        },
        "drag from the left pane's last column widens the tree pane",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    let wide = flat_join_col(&tui, SLATE_SIDEBAR).unwrap();
    assert!(
        wide + DIVIDER_SLACK >= start + DIVIDER_DRAG_DELTA && flat_launch(&tui),
        "wider split holds (not a flicker or a row select):\n{}",
        tui.screen()
    );

    sgr_drag(&mut tui, (wide, row), (wide - DIVIDER_DRAG_DELTA, row));
    tui.wait_pred(
        |_| {
            flat_join_col(&tui, SLATE_SIDEBAR)
                .is_some_and(|join| join + DIVIDER_DRAG_DELTA <= wide + 1 + DIVIDER_SLACK)
                && flat_launch(&tui)
        },
        "drag from the right pane's first column narrows the tree pane",
        WAIT,
    );
    assert!(
        tui.clipboard_payloads().is_empty(),
        "a divider drag must not copy text:\n{}",
        tui.screen()
    );
}

/// Long-line file diff loaded, tail clipped, tree still focused on the file.
fn long_diff_clipped(tui: &PtySession) -> bool {
    let Some(join) = flat_join_col(tui, SLATE_SIDEBAR) else {
        return false;
    };
    let right = flat_right_body(tui, join);
    flat_left_cursor_on(tui, join, LONG_DIFF_FILE)
        && right.contains(LONG_DIFF_FILE)
        && right.contains("nnnn")
        && !right.contains(DIFF_HSCROLL_TAIL)
}

/// File-diff h-bar on the pane's last row, tail still clipped, thumb right
/// of a track cell (a left-edge thumb shares its cell with a jump to the
/// origin). Returns `(row, first thumb col, last track col)`.
fn diff_hbar_off_left_edge(tui: &PtySession) -> Option<(u16, u16, u16)> {
    let join = flat_join_col(tui, SLATE_SIDEBAR)?;
    let (row, thumb, end) = tui.grid_hbar_span()?;
    let right = flat_right_body(tui, join);
    (row == flat_pane_last_row(tui)
        && thumb > join
        && end > thumb
        && matches!(tui.grid_cell_char(row, thumb - 1), Some('═' | '─'))
        && right.contains("nnnn")
        && !right.contains(DIFF_HSCROLL_TAIL))
    .then_some((row, thumb, end))
}

/// Wheel right one notch at a time until the file-diff h-bar paints.
///
/// Each notch waits for its own frame and the frame must hold for
/// `SETTLE_MS`, so no wheel is in flight when the caller reads the thumb.
fn wheel_until_hbar(tui: &mut PtySession, col: u16, row: u16) -> (u16, u16, u16) {
    const NOTCH_WAIT: Duration = Duration::from_secs(3);
    let start = Instant::now();
    loop {
        let sent_on = tui.screen();
        tui.sgr_mouse(SGR_WHEEL_RIGHT, col, row);
        let notch = Instant::now();
        while tui.screen() == sent_on && notch.elapsed() < NOTCH_WAIT {
            tui.wait_ms(25);
        }
        let mut screen = tui.screen();
        loop {
            tui.wait_ms(SETTLE_MS);
            let next = tui.screen();
            if next == screen || start.elapsed() >= WAIT {
                break;
            }
            screen = next;
        }
        if let Some(span) = diff_hbar_off_left_edge(tui) {
            return span;
        }
        if start.elapsed() >= WAIT {
            panic!("timeout waiting for the file-diff h-bar on the pane's last row:\n{screen}");
        }
    }
}

/// The file-diff h-bar on the flat pane's last row drags.
///
/// Flat panes have no bottom border; the h-bar sits on the pane's last row
/// and that row is hit-tested. Live PTY (80×24, unwrapped, so the long line
/// clips): `/unique-diffline` loads the file, a wheel pan paints the bar on
/// the last pane row, a press on the thumb does not jump, and a drag to the
/// track end pans `UNIQUE_DIFF_TAIL` into view. A dropped last-row hit
/// leaves the tail clipped.
#[test]
fn pty_paint_flat_diff_hbar_drags_on_last_row() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace, LONG_DIFF_FILE, DIFF_HSCROLL_TAIL);
    unwrap_diffs_at_launch(&workspace);
    let mut tui = open_flat(&workspace, 80, 24);
    tui.search("unique-diffline");
    tui.wait_pred(
        |_| long_diff_clipped(&tui),
        "search loads the clipped file diff; tree stays on the file",
        WAIT,
    );
    let join = flat_join_col(&tui, SLATE_SIDEBAR).unwrap();
    let row = flat_left_row_containing(&tui, join, LONG_DIFF_FILE).unwrap();

    let (bar_row, thumb, end) = wheel_until_hbar(&mut tui, join + 18, row);
    tui.sgr_mouse(0, thumb, bar_row);
    tui.wait_ms(SETTLE_MS);
    assert!(
        diff_hbar_off_left_edge(&tui).is_some(),
        "thumb grab must not jump (a track click moves the pan):\n{}",
        tui.screen()
    );
    tui.sgr_mouse(SGR_LEFT_DRAG, end, bar_row);
    sgr_release(&mut tui, end, bar_row);
    let panned = |tui: &PtySession| {
        flat_right_body(tui, join).contains(DIFF_HSCROLL_TAIL)
            && !flat_left_body(tui, join).contains(DIFF_HSCROLL_TAIL)
    };
    tui.wait_pred(
        |_| panned(&tui),
        "h-bar drag on the last pane row pans UNIQUE_DIFF_TAIL onto the diff",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert!(
        panned(&tui),
        "diff pan holds after the drag:\n{}",
        tui.screen()
    );
}

/// A drag in the flat tree pane copies that pane's text only.
///
/// Live PTY: SGR press on the first content column of the README row (one
/// pad cell in; flat panes have no border column), motion-bit drag into
/// the right pane one row down, release. The OSC 52 payload holds the README label and no diff text;
/// the breadcrumb flashes `copied`. A plain click copies nothing.
#[test]
fn pty_paint_flat_drag_select_copies_pane_text() {
    let (_root, workspace) = daily_workspace();
    let mut tui = open_flat(&workspace, COLS, ROWS);
    let join = wait_flat_launch(&tui);
    let row = flat_left_row_containing(&tui, join, "README.md")
        .unwrap_or_else(|| panic!("README row:\n{}", tui.screen()));

    tui.sgr_click(0, row);
    tui.wait_ms(SETTLE_MS);
    assert!(
        tui.clipboard_payloads().is_empty(),
        "plain click must not copy:\n{}",
        tui.screen()
    );

    sgr_drag(&mut tui, (FLAT_PAD_COLS, row), (join + 30, row + 1));
    tui.wait_clipboard_pred(
        |payloads| {
            payloads.iter().any(|text| {
                text.contains("README.md")
                    && !text.contains("UNSTAGED")
                    && !text.contains("+dirty")
                    && !text.contains("app/README.md")
            })
        },
        "OSC 52 payload is tree text: README.md, no diff pane text",
        WAIT,
    );
    assert_eq!(
        tui.clipboard_payloads().len(),
        1,
        "one drag copies once:\n{}",
        tui.screen()
    );
    tui.wait_pred(
        |screen| crumb_row(screen).contains("copied"),
        "release flashes the copied toast",
        WAIT,
    );
}

use std::ops::Range;

use crate::harness::{PtySession, UserConfig, COLS, ROWS};
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, flat_join_col, flat_pane_last_row, status_row, FLAT_TITLE_ROW, SLATE_CHROME,
    SLATE_CURSOR, SLATE_HEADING, SLATE_MUTED, SLATE_SIDEBAR, SLATE_SURFACE, WAIT,
};

/// Tokyo Night surface: the harness baseline theme. A launch that still
/// reads a baseline config paints it.
const TOKYO_NIGHT_SURFACE: (u8, u8, u8) = (0x1a, 0x1b, 0x26);

/// Box-drawing glyphs of a boxed pane or a popup. Flat panes paint none.
const BOX_CORNERS: [char; 8] = ['┌', '┐', '└', '┘', '╭', '╮', '╰', '╯'];

/// Flat title row over `cols` with focus: every blank cell is `accent` in
/// `cursor`, the title (one cell in) is `heading`, bold, underlined. All
/// cells keep the pane fill `bg`.
fn title_focused(
    tui: &PtySession,
    cols: Range<u16>,
    title: &str,
    accent: char,
    bg: (u8, u8, u8),
) -> bool {
    let title_at = cols.start + 1;
    let title_end = title_at + title.chars().count() as u16;
    let mut title_chars = title.chars();
    cols.into_iter().all(|col| {
        let Some(cell) = tui.cell_paint(FLAT_TITLE_ROW, col) else {
            return false;
        };
        if cell.bg != Some(bg) {
            return false;
        }
        if (title_at..title_end).contains(&col) {
            cell.glyph == title_chars.next()
                && cell.fg == Some(SLATE_HEADING)
                && cell.bold
                && cell.underline
        } else {
            cell.glyph == Some(accent) && cell.fg == Some(SLATE_CURSOR) && !cell.underline
        }
    })
}

/// Flat title row over `cols` without focus: the title (one cell in) is
/// `muted`, normal weight, no underline; other cells are blank. No accent.
fn title_unfocused(tui: &PtySession, cols: Range<u16>, title: &str, bg: (u8, u8, u8)) -> bool {
    let title_at = cols.start + 1;
    let title_end = title_at + title.chars().count() as u16;
    let mut title_chars = title.chars();
    cols.into_iter().all(|col| {
        let Some(cell) = tui.cell_paint(FLAT_TITLE_ROW, col) else {
            return false;
        };
        if cell.bg != Some(bg) || cell.underline || cell.bold {
            return false;
        }
        if (title_at..title_end).contains(&col) {
            cell.glyph == title_chars.next() && cell.fg == Some(SLATE_MUTED)
        } else {
            matches!(cell.glyph, None | Some(' '))
        }
    })
}

/// Launch frame: README diff on the right, both panes flat.
fn flat_first_paint(tui: &PtySession) -> bool {
    let Some(join) = flat_join_col(tui, SLATE_SIDEBAR) else {
        return false;
    };
    let body = FLAT_TITLE_ROW + 1;
    let left = tui.grid_row_text(body, 0..join);
    let right = tui.grid_row_text(body, join..COLS);
    let screen = tui.screen();
    left.contains("# workspace")
        && right.contains("app/README.md")
        && screen.contains("+dirty")
        && screen.contains("UNSTAGED")
        && crumb_row(&screen).trim() == "workspace › app"
        && status_row(&screen).contains("focus right")
}

fn has_bg(tui: &PtySession, row: u16, col: u16, bg: (u8, u8, u8)) -> bool {
    tui.cell_paint(row, col)
        .is_some_and(|cell| cell.bg == Some(bg))
}

fn assert_no_pane_borders(tui: &PtySession, join: u16) {
    let screen = tui.screen();
    assert!(
        !screen.chars().any(|ch| BOX_CORNERS.contains(&ch)) && !screen.contains("││"),
        "flat panes paint no box corners or `││` join:\n{screen}"
    );
    let title = tui.grid_row_text(FLAT_TITLE_ROW, 0..COLS);
    assert!(
        !title.contains('─'),
        "flat title row has no border rule:\n{screen}"
    );
    for row in FLAT_TITLE_ROW..=flat_pane_last_row(tui) {
        for col in [0, join - 1, join, COLS - 1] {
            assert_ne!(
                tui.cell_paint(row, col).and_then(|cell| cell.glyph),
                Some('│'),
                "no pane border `│` at row {row} col {col}:\n{screen}"
            );
        }
    }
}

/// With no user config the shipped look paints: Slate, flat panes.
///
/// Docs: paint is the default background and Slate the default theme. The
/// left pane fills with `sidebar`, the right pane with `surface`, and the
/// tab strip, breadcrumb and status rows with `chrome`. Panes have no
/// borders: row 0 of a pane is its title row and the body starts on the
/// next row. The active pane's title row carries a `cursor` accent line
/// (`▁`, `_` with ASCII glyphs) under a `heading`, bold, underlined title;
/// the other title is `muted` with no line. Tab moves the line.
///
/// Live PTY, `UserConfig::Own` with no file: Slate fills on the expected
/// cells, no Tokyo Night surface, no box glyphs, accent on the tree title,
/// then on the diff title after Tab. A Nerd-glyph launch paints `▁`.
#[test]
fn pty_paint_default_launch_is_flat_slate() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size_with_config(&workspace, COLS, ROWS, &[], UserConfig::Own);
    tui.wait_pred(
        |_| flat_first_paint(&tui),
        "first paint: flat Slate panes with the README diff on the right",
        WAIT,
    );
    let join = flat_join_col(&tui, SLATE_SIDEBAR).unwrap();
    let last = flat_pane_last_row(&tui);
    let screen = tui.screen();
    assert!(
        join > 20 && join < COLS - 20,
        "join {join} is not a pane split:\n{screen}"
    );

    for (row, col, bg, what) in [
        (0, COLS - 1, SLATE_CHROME, "tab strip"),
        (FLAT_TITLE_ROW, 0, SLATE_SIDEBAR, "left title row"),
        (last, 0, SLATE_SIDEBAR, "left pane body"),
        (last, join - 1, SLATE_SIDEBAR, "left pane last column"),
        (FLAT_TITLE_ROW, join, SLATE_SURFACE, "right title row"),
        (last, join, SLATE_SURFACE, "right pane body"),
        (last, COLS - 1, SLATE_SURFACE, "right pane last column"),
        (ROWS - 2, COLS - 1, SLATE_CHROME, "breadcrumb row"),
        (ROWS - 1, COLS - 1, SLATE_CHROME, "status row"),
    ] {
        assert!(
            has_bg(&tui, row, col, bg),
            "{what} cell ({row},{col}) bg {bg:?}, got {:?}:\n{screen}",
            tui.cell_paint(row, col)
        );
    }
    let (r, g, b) = TOKYO_NIGHT_SURFACE;
    assert!(
        !tui.has_rgb(r, g, b),
        "no user config: Slate, not the Tokyo Night baseline:\n{screen}"
    );
    assert_no_pane_borders(&tui, join);

    let title = tui.grid_row_text(FLAT_TITLE_ROW, 0..COLS);
    assert!(
        !title.contains("# workspace") && !title.contains("app/README.md"),
        "pane body starts below the title row:\n{screen}"
    );
    assert!(
        title_focused(&tui, 0..join, "tree", '_', SLATE_SIDEBAR),
        "tree title row: `_` accent in cursor, heading bold underlined title:\n{screen}"
    );
    assert!(
        title_unfocused(&tui, join..COLS, "diff", SLATE_SURFACE),
        "diff title row: muted title, no accent:\n{screen}"
    );

    tui.tab();
    tui.wait_pred(
        |_| {
            title_focused(&tui, join..COLS, "diff", '_', SLATE_SURFACE)
                && title_unfocused(&tui, 0..join, "tree", SLATE_SIDEBAR)
        },
        "Tab moves the accent line and heading title to the diff pane",
        WAIT,
    );
    assert!(
        has_bg(&tui, last, 0, SLATE_SIDEBAR) && has_bg(&tui, last, join, SLATE_SURFACE),
        "pane fills do not change with focus:\n{}",
        tui.screen()
    );
    drop(tui);

    let (_root, nerd) = daily_workspace();
    let tui = PtySession::open_size_with_config(
        &nerd,
        COLS,
        ROWS,
        &[("WS_STATUS_GLYPHS", "nerd")],
        UserConfig::Own,
    );
    tui.wait_pred(
        |_| {
            flat_join_col(&tui, SLATE_SIDEBAR).is_some_and(|join| {
                tui.screen().contains("+dirty")
                    && title_focused(&tui, 0..join, "tree", '\u{2581}', SLATE_SIDEBAR)
            })
        },
        "Nerd glyphs: the accent line is `▁` in cursor under the tree title",
        WAIT,
    );
}

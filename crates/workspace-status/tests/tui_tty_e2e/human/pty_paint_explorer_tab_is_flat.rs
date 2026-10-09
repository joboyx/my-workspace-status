use std::fs;
use std::path::Path;

use crate::harness::{PtySession, UserConfig, COLS, ROWS};
use crate::seed::{daily_workspace, git};
use crate::support::{
    crumb_row, flat_join_col, flat_left_body, flat_left_row_containing, flat_pane_last_row,
    flat_right_body, FLAT_ACCENT_ROW, FLAT_BODY_ROW, FLAT_TITLE_ROW, GIT_WAIT, SLATE_CHROME,
    SLATE_CURSOR, SLATE_HEADING, SLATE_MUTED, SLATE_SIDEBAR, SLATE_SURFACE, WAIT,
};

/// Strip label of the `app` checkout's Explorer tab.
const EXPLORER_APP: &str = "Explorer · app";
/// Body of the committed, clean `kept.rs`.
const KEPT_BODY: &str = "pub fn kept_body() {}";
/// Box-drawing glyphs of a boxed pane. Flat panes paint none.
const BOX_CORNERS: [char; 4] = ['┌', '┐', '└', '┘'];

/// Add a committed, clean `app/kept.rs` to the daily seed. `app/README.md`
/// stays modified, so the Explorer root lists `kept.rs`, then `README.md`.
fn seed_kept_file(workspace: &Path) {
    let app = workspace.join("app");
    fs::write(app.join("kept.rs"), format!("{KEPT_BODY}\n")).unwrap();
    git(&app, &["add", "kept.rs"]);
    git(&app, &["commit", "-q", "-m", "add kept.rs"]);
}

/// Accent row cell `col` is the accent line: `_` in `cursor` on `bg`.
fn accent_at(tui: &PtySession, col: u16, bg: (u8, u8, u8)) -> bool {
    tui.cell_paint(FLAT_ACCENT_ROW, col).is_some_and(|cell| {
        cell.glyph == Some('_') && cell.fg == Some(SLATE_CURSOR) && cell.bg == Some(bg)
    })
}

/// Title row and accent row cells at `col` are plain blanks on `bg` (no
/// accent line).
fn blank_at(tui: &PtySession, col: u16, bg: (u8, u8, u8)) -> bool {
    [FLAT_TITLE_ROW, FLAT_ACCENT_ROW].into_iter().all(|row| {
        tui.cell_paint(row, col).is_some_and(|cell| {
            matches!(cell.glyph, None | Some(' ')) && !cell.underline && cell.bg == Some(bg)
        })
    })
}

/// First title character one cell in from `col`: `heading` and bold when
/// focused; `muted`, plain when not. Never underlined.
fn title_char_at(tui: &PtySession, col: u16, focused: bool) -> bool {
    tui.cell_paint(FLAT_TITLE_ROW, col + 1).is_some_and(|cell| {
        if focused {
            cell.fg == Some(SLATE_HEADING) && cell.bold && !cell.underline
        } else {
            cell.fg == Some(SLATE_MUTED) && !cell.bold && !cell.underline
        }
    })
}

/// Explorer · app is active and flat: tree on the left lists both files,
/// the preview on the right holds `preview`, and the right title names
/// `title`.
fn flat_explorer(tui: &PtySession, title: &str, preview: &str) -> bool {
    let Some(join) = flat_join_col(tui, SLATE_SIDEBAR) else {
        return false;
    };
    let screen = tui.screen();
    let left = flat_left_body(tui, join);
    let right = flat_right_body(tui, join);
    screen
        .lines()
        .next()
        .unwrap_or_default()
        .contains(EXPLORER_APP)
        && tui
            .grid_row_text(FLAT_TITLE_ROW, join..COLS)
            .starts_with(&format!(" {title}"))
        && left.contains("kept.rs")
        && left.contains("README.md")
        && !left.contains("# workspace")
        && right.contains(preview)
}

/// The Explorer tab paints flat in paint mode, like the Workspace panes.
///
/// Docs: an Explorer tab paints its tree and preview panes through the
/// shared pane chrome. In paint mode the tree fills with `sidebar`, the
/// preview with `surface`, no border glyphs, and the focused pane's accent
/// row (under its title) carries the `cursor` line. Mouse clicks hit the
/// flat content rows.
///
/// Live PTY on the shipped look (no user config, Slate): `-` on
/// `README.md` opens `Explorer · app` with its diff. Fills, borders, and
/// the accent on the tree title are checked per cell. A click on the
/// `kept.rs` row (first column) previews its body; a click on the
/// preview's first column moves the accent to the preview. A boxed
/// Explorer, an unfilled pane, or an accent that does not follow focus
/// cannot pass.
#[test]
fn pty_paint_explorer_tab_is_flat() {
    let (_root, workspace) = daily_workspace();
    seed_kept_file(&workspace);
    let mut tui = PtySession::open_size_with_config(&workspace, COLS, ROWS, &[], UserConfig::Own);
    tui.wait_pred(
        |screen| {
            screen.contains("+dirty")
                && crumb_row(screen).contains("workspace › app")
                && flat_join_col(&tui, SLATE_SIDEBAR).is_some()
        },
        "first paint: flat Slate panes with the README diff",
        GIT_WAIT,
    );

    tui.key('-');
    tui.wait_pred(
        |_| flat_explorer(&tui, "diff", "+dirty"),
        "`-` opens a flat Explorer · app on README.md with its diff",
        GIT_WAIT,
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
        (FLAT_TITLE_ROW, 0, SLATE_SIDEBAR, "tree title row"),
        (last, 0, SLATE_SIDEBAR, "tree body"),
        (last, join - 1, SLATE_SIDEBAR, "tree last column"),
        (FLAT_TITLE_ROW, join, SLATE_SURFACE, "preview title row"),
        (FLAT_ACCENT_ROW, join, SLATE_SURFACE, "preview accent row"),
        (last, join, SLATE_SURFACE, "preview body"),
        (last, COLS - 1, SLATE_SURFACE, "preview last column"),
        (ROWS - 1, COLS - 1, SLATE_CHROME, "status row"),
    ] {
        assert!(
            tui.cell_paint(row, col)
                .is_some_and(|cell| cell.bg == Some(bg)),
            "{what} cell ({row},{col}) bg {bg:?}, got {:?}:\n{screen}",
            tui.cell_paint(row, col)
        );
    }
    assert!(
        !screen.chars().any(|ch| BOX_CORNERS.contains(&ch)),
        "flat Explorer panes paint no box corners:\n{screen}"
    );
    for row in FLAT_TITLE_ROW..=last {
        for col in [0, join - 1, join, COLS - 1] {
            assert_ne!(
                tui.cell_paint(row, col).and_then(|cell| cell.glyph),
                Some('│'),
                "no pane border `│` at row {row} col {col}:\n{screen}"
            );
        }
    }
    assert!(
        accent_at(&tui, 0, SLATE_SIDEBAR)
            && accent_at(&tui, join - 1, SLATE_SIDEBAR)
            && title_char_at(&tui, 0, true)
            && blank_at(&tui, join, SLATE_SURFACE)
            && blank_at(&tui, COLS - 1, SLATE_SURFACE)
            && title_char_at(&tui, join, false),
        "accent line under the tree title only:\n{screen}"
    );

    let kept = flat_left_row_containing(&tui, join, "kept.rs").expect("kept.rs row");
    tui.sgr_click(0, kept);
    tui.wait_pred(
        |_| flat_explorer(&tui, "app/kept.rs", KEPT_BODY),
        "click on the kept.rs row (first column) previews its body",
        GIT_WAIT,
    );

    tui.sgr_click(join, FLAT_BODY_ROW);
    tui.wait_pred(
        |_| {
            accent_at(&tui, join, SLATE_SURFACE)
                && accent_at(&tui, COLS - 1, SLATE_SURFACE)
                && title_char_at(&tui, join, true)
                && blank_at(&tui, 0, SLATE_SIDEBAR)
                && title_char_at(&tui, 0, false)
        },
        "click on the preview's first column moves the accent to the preview",
        WAIT,
    );
}

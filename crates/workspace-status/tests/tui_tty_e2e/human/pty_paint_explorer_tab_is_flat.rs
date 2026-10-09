use std::fs;
use std::path::Path;

use crate::harness::{PtySession, UserConfig, COLS, ROWS};
use crate::seed::{daily_workspace, git};
use crate::support::{
    crumb_row, flat_join_col, flat_left_body, flat_left_row_containing, flat_pane_last_row,
    flat_right_body, FLAT_BODY_ROW, FLAT_PAD_COLS, FLAT_TITLE_ROW, GIT_WAIT, SLATE_BORDER_DIM,
    SLATE_CHROME, SLATE_CURSOR, SLATE_HEADING, SLATE_MUTED, SLATE_SIDEBAR, SLATE_SURFACE, WAIT,
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

/// Fewest rule cells a title row must end with to count as the rule.
const MIN_RULE_CELLS: u16 = 3;

/// Title row of the pane over `start..end` ends in the rule: from `end - 1`
/// leftwards at least [`MIN_RULE_CELLS`] `_` cells in `cursor` (focused) or
/// `border_dim` (not), then one blank cell, then a title glyph. Every cell
/// keeps the pane fill `bg`; none is underlined.
fn title_rule(tui: &PtySession, start: u16, end: u16, focused: bool, bg: (u8, u8, u8)) -> bool {
    let rule_fg = if focused {
        SLATE_CURSOR
    } else {
        SLATE_BORDER_DIM
    };
    let cell = |col: u16| {
        tui.cell_paint(FLAT_TITLE_ROW, col)
            .filter(|cell| cell.bg == Some(bg) && !cell.underline)
    };
    let mut col = end;
    while col > start
        && cell(col - 1).is_some_and(|c| c.glyph == Some('_') && c.fg == Some(rule_fg))
    {
        col -= 1;
    }
    end - col >= MIN_RULE_CELLS
        && col >= start + 2
        && cell(col - 1).is_some_and(|c| matches!(c.glyph, None | Some(' ')))
        && cell(col - 2).is_some_and(|c| !matches!(c.glyph, None | Some(' ')))
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
/// preview with `surface`, no border glyphs. Each pane's title row ends in
/// a rule to its right edge: `cursor` on the focused pane, `border_dim` on
/// the other. The body starts on the next row, one pad column in; the
/// selected row's bar sits in the pad column. Mouse clicks hit the flat
/// content rows, pad cells included.
///
/// Live PTY on the shipped look (no user config, Slate): `-` on
/// `README.md` opens `Explorer · app` with its diff. Fills, borders, and
/// both title rules are checked per cell. A click on the `kept.rs` row
/// (pad column) previews its body and puts the bar in that column; a click
/// on the preview's first column moves the `cursor` rule to the preview. A
/// boxed Explorer, an unfilled pane, row text off the pad grid, or a rule
/// that does not follow focus cannot pass.
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
        (FLAT_BODY_ROW, join, SLATE_SURFACE, "preview first body row"),
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
        title_rule(&tui, 0, join, true, SLATE_SIDEBAR)
            && title_char_at(&tui, 0, true)
            && title_rule(&tui, join, COLS, false, SLATE_SURFACE)
            && title_char_at(&tui, join, false),
        "cursor rule after the tree title, border_dim rule after the preview title:\n{screen}"
    );

    let kept = flat_left_row_containing(&tui, join, "kept.rs").expect("kept.rs row");
    tui.sgr_click(0, kept);
    tui.wait_pred(
        |_| flat_explorer(&tui, "app/kept.rs", KEPT_BODY),
        "click on the kept.rs row (first column) previews its body",
        GIT_WAIT,
    );

    let readme = flat_left_row_containing(&tui, join, "README.md").expect("README.md row");
    let kept_text = tui.grid_row_text(kept, 0..join);
    let readme_text = tui.grid_row_text(readme, 0..join);
    let text_col = |line: &str| line.chars().skip(1).position(|ch| ch != ' ');
    assert!(
        kept_text.starts_with('\u{258C}')
            && readme_text.starts_with("  ")
            && text_col(&kept_text) == text_col(&readme_text)
            && text_col(&readme_text).is_some_and(|col| col >= usize::from(FLAT_PAD_COLS)),
        "selected kept.rs row: bar in pane column 0; unselected README.md row: \
         pad and gutter blank; both texts start at the same column:\n{}",
        tui.screen()
    );

    tui.sgr_click(join, FLAT_BODY_ROW);
    tui.wait_pred(
        |_| {
            title_rule(&tui, join, COLS, true, SLATE_SURFACE)
                && title_char_at(&tui, join, true)
                && title_rule(&tui, 0, join, false, SLATE_SIDEBAR)
                && title_char_at(&tui, 0, false)
        },
        "click on the preview's first column moves the cursor rule to the preview",
        WAIT,
    );
}

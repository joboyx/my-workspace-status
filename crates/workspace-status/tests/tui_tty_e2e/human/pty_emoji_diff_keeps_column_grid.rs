use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git};
use crate::support::{
    no_wrong_overlays, panes_tree_focused_diff_unfocused, right_pane, status_row, title_has_diff,
    tree_cursor_on, tree_has, SETTLE_MS, WAIT,
};

/// Wide enough that split paints (`NARROW_SXS` is 100). Default 140 falls
/// back to `inline (too narrow)` and has no in-diff RULE to check.
const WIDE_COLS: u16 = 200;
const WIDE_ROWS: u16 = 32;

const FILE: &str = "emoji-grid.rs";
const BEFORE: &str = "ASCII_BEFORE";
const AFTER: &str = "ASCII_AFTER";
const LEFT_MARK: &str = "EMOJI_LEFT";
const RIGHT_MARK: &str = "hello world";
const EMOJI: char = '😀';
const EMOJI_COUNT: usize = 30;

/// Committed emoji on the old side, dirty ASCII on the new side.
///
/// Split paints the emoji in the left cell. Char-count clip/pad shifts
/// the in-diff RULE. Display-column clip/pad keeps that RULE on the
/// same column as the ASCII context rows.
fn seed_emoji_left_cell(workspace: &Path) {
    let app = workspace.join("app");
    let path = app.join(FILE);
    let emoji: String = std::iter::repeat(EMOJI).take(EMOJI_COUNT).collect();
    fs::write(&path, format!("{BEFORE}\n{LEFT_MARK} {emoji}\n{AFTER}\n")).unwrap();
    git(&app, &["add", FILE]);
    git(&app, &["commit", "-q", "-m", "emoji-grid base"]);
    fs::write(&path, format!("{BEFORE}\n{RIGHT_MARK}\n{AFTER}\n")).unwrap();
}

/// MesloLGS NF / `visible_width`: pictographs ≥ U+1F300 are two columns.
fn display_width(value: &str) -> usize {
    value
        .chars()
        .map(|ch| if (ch as u32) >= 0x1f300 { 2 } else { 1 })
        .sum()
}

fn pane_line_containing<'a>(pane: &'a str, needle: &str) -> Option<&'a str> {
    pane.lines().find(|line| line.contains(needle))
}

/// Display column of the `n`th `│` (0-based) on a painted row.
fn nth_rule_display_col(line: &str, n: usize) -> Option<usize> {
    let mut seen = 0usize;
    let mut col = 0usize;
    for ch in line.chars() {
        if ch == '│' {
            if seen == n {
                return Some(col);
            }
            seen += 1;
        }
        col += display_width(&ch.to_string());
    }
    None
}

/// In-diff RULE is the second `│` (left gutter is first, right gutter third).
fn split_rule_col(line: &str) -> Option<usize> {
    nth_rule_display_col(line, 1)
}

fn file_diff_header_split(right: &str) -> bool {
    right.lines().next().is_some_and(|line| {
        line.contains(&format!("app/{FILE}"))
            && line.contains("split")
            && !line.contains("too narrow")
            && !line.contains("inline")
    })
}

fn documented_emoji_split_grid(screen: &str) -> bool {
    let right = right_pane(screen);
    let before = pane_line_containing(&right, BEFORE);
    let emoji_row = pane_line_containing(&right, LEFT_MARK);
    let after = pane_line_containing(&right, AFTER);
    let Some(before) = before else {
        return false;
    };
    let Some(emoji_row) = emoji_row else {
        return false;
    };
    let Some(after) = after else {
        return false;
    };
    let Some(before_rule) = split_rule_col(before) else {
        return false;
    };
    let Some(emoji_rule) = split_rule_col(emoji_row) else {
        return false;
    };
    let Some(after_rule) = split_rule_col(after) else {
        return false;
    };
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && tree_has(screen, FILE)
        && tree_has(screen, "README.md")
        && title_has_diff(screen)
        && file_diff_header_split(&right)
        && right.contains("UNSTAGED")
        && !right.contains("NEW")
        && emoji_row.contains(EMOJI)
        && emoji_row.contains(LEFT_MARK)
        && emoji_row.contains(RIGHT_MARK)
        && !emoji_row.contains(BEFORE)
        && !emoji_row.contains(AFTER)
        && before.contains(BEFORE)
        && !before.contains(EMOJI)
        && !before.contains(LEFT_MARK)
        && after.contains(AFTER)
        && !after.contains(EMOJI)
        && !after.contains(LEFT_MARK)
        && before_rule == emoji_rule
        && after_rule == emoji_rule
        && display_width(before) <= usize::from(WIDE_COLS)
        && display_width(emoji_row) <= usize::from(WIDE_COLS)
        && display_width(after) <= usize::from(WIDE_COLS)
        && status_row(screen).contains(" split")
        && !status_row(screen).contains(" inline")
        && !screen.contains("WIP on graph")
        && no_wrong_overlays(screen)
}

/// Split file-diff clip/pad uses display columns, not Unicode scalars.
///
/// Docs (`docs/diff-rendering.md`): cells clip with `slice_cols` and pad
/// with `visible_width`. Emoji stay emoji (MesloLGS NF: two columns).
/// Char-count clip/pad under-counts those glyphs, so the left cell grows
/// and the in-diff RULE walks off the context-row column. That wrap also
/// splits `EMOJI_LEFT` from the paired `hello world` cell.
///
/// Use a 200-col PTY so split paints. Search `emoji-grid`. The
/// deleted left cell is `EMOJI_LEFT` plus thirty `😀`. The RULE on that
/// row must match `ASCII_BEFORE` / `ASCII_AFTER`. A no-op, an ASCII
/// placeholder, a RULE shift, or a wrap fragment is red.
#[test]
fn pty_emoji_diff_keeps_column_grid() {
    let (_root, workspace) = daily_workspace();
    seed_emoji_left_cell(&workspace);
    let mut tui = PtySession::open_size(&workspace, WIDE_COLS, WIDE_ROWS);
    tui.wait_pred(
        |screen| {
            panes_tree_focused_diff_unfocused(screen)
                && tree_has(screen, FILE)
                && screen.contains("app/README.md")
                && screen.contains("UNSTAGED")
        },
        "launch paints README; emoji-grid.rs is in the tree",
        WAIT,
    );

    tui.search("emoji-grid");
    tui.wait_pred(
        documented_emoji_split_grid,
        "search loads the UNSTAGED split: emoji stay emoji, RULE matches ASCII rows",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_emoji_split_grid,
        "emoji split grid holds (not a flicker, RULE walk, wrap, or placeholder)",
        WAIT,
    );

    let screen = tui.screen();
    let right = right_pane(&screen);
    let emoji_row =
        pane_line_containing(&right, LEFT_MARK).expect("EMOJI_LEFT row after hold");
    assert!(
        emoji_row.contains(EMOJI),
        "left cell must keep the emoji glyph, not an ASCII placeholder:\n{right}"
    );
    assert_eq!(
        split_rule_col(pane_line_containing(&right, BEFORE).expect("ASCII_BEFORE row")),
        split_rule_col(emoji_row),
        "in-diff RULE must stay on the context-row column:\n{right}"
    );
    for line in screen.lines() {
        assert!(
            display_width(line) <= usize::from(WIDE_COLS),
            "under-pad wrap must not grow a screen row past {WIDE_COLS} columns:\n{line}"
        );
    }
}

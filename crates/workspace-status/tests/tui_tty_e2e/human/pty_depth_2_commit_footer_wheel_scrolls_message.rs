use std::fs;

use crate::harness::{left_tree, pane_body_start, user_config_file, PtySession, SGR_WHEEL_DOWN};
use crate::seed::{
    daily_workspace, seed_two_file_long_message_repo, COMMIT_MSG_BODY, COMMIT_MSG_BODY_TAIL,
    PAIR_BASE_FILE_LINE, PAIR_FIRST_FILE_LINE, PAIR_SECOND_FILE_LINE,
};
use crate::support::{
    graph_cursor_on, no_wrong_overlays, panes_files_focused, panes_files_focused_diff_unfocused,
    panes_files_unfocused_diff_focused, panes_graph_focused_files_unfocused,
    panes_tree_focused_graph_unfocused, panes_tree_unfocused_graph_focused, right_cursor_on,
    right_pane, title_has_diff, title_has_files, WAIT,
};

const REPO: &str = "msgpair";
/// Subject of the two-file commit with the tall message.
const SUBJECT: &str = "pair tip";
/// First commit-file row; the depth-2 diff opens on it.
const FIRST: &str = "first.txt";
/// Second commit-file row; a wheel that moves the file cursor lands here.
const SECOND: &str = "second.txt";
/// Subject of the other long-message commit, below the tip.
const BASE_SUBJECT: &str = "pair base";
/// Only commit-file row of [`BASE_SUBJECT`].
const BASE: &str = "base.txt";
/// xterm SGR button for wheel up (`ScrollUp`, `Cb` 64).
const SGR_WHEEL_UP: u8 = 64;
/// Wheel notches per burst: more than the message has hidden lines.
const NOTCHES: usize = 30;

/// Left-pane body rows, top to bottom (the same rows as [`left_tree`]).
fn left_rows(screen: &str) -> Vec<String> {
    left_tree(screen).lines().map(str::to_string).collect()
}

/// Footer rule: the row is all `▁` (`_` in ASCII mode) inside its borders.
fn is_rule(row: &str) -> bool {
    let inner = row.trim_matches(|c: char| c == '\u{2502}' || c.is_whitespace());
    inner.chars().count() >= 10
        && (inner.chars().all(|c| c == '\u{2581}') || inner.chars().all(|c| c == '_'))
}

/// Left-pane rows under the last footer rule: the commit-message footer.
fn footer_rows(screen: &str) -> Vec<String> {
    let rows = left_rows(screen);
    let Some(rule) = rows.iter().rposition(|row| is_rule(row)) else {
        return Vec::new();
    };
    rows[rule + 1..].to_vec()
}

fn footer_has(screen: &str, needle: &str) -> bool {
    footer_rows(screen).iter().any(|row| row.contains(needle))
}

/// Left-pane list row with `name` carries the selection bar (focused `▌`
/// or unfocused `▏`).
fn file_selected(screen: &str, name: &str) -> bool {
    left_rows(screen)
        .iter()
        .any(|row| row.contains(name) && (row.contains('\u{258C}') || row.contains('\u{258F}')))
}

/// Depth 2 on `first.txt`: the selection stays on the first file row and
/// the right pane paints the first file's diff, not the second's.
fn first_file_diff(screen: &str) -> bool {
    let right = right_pane(screen);
    panes_files_unfocused_diff_focused(screen)
        && title_has_diff(screen)
        && title_has_files(screen)
        && left_tree(screen).contains(SECOND)
        && file_selected(screen, FIRST)
        && !file_selected(screen, SECOND)
        && right.contains(PAIR_FIRST_FILE_LINE)
        && !right.contains(PAIR_SECOND_FILE_LINE)
        && no_wrong_overlays(screen)
}

/// Depth 2, message at its top: first body line in the left footer, tail
/// hidden below.
fn depth_2_msg_at_top(screen: &str) -> bool {
    first_file_diff(screen)
        && footer_has(screen, COMMIT_MSG_BODY)
        && !footer_has(screen, COMMIT_MSG_BODY_TAIL)
}

/// Depth 2, message scrolled to its end: tail in the left footer, first
/// body line out, file cursor and diff unchanged.
fn depth_2_msg_at_tail(screen: &str) -> bool {
    first_file_diff(screen)
        && footer_has(screen, COMMIT_MSG_BODY_TAIL)
        && !footer_has(screen, COMMIT_MSG_BODY)
}

/// Depth 2 on `pair base`: `base.txt` selected with its diff on the right,
/// and the footer at the top of the message (first body line in, tail
/// out, more-below hint shown).
fn base_depth_2_msg_at_top(screen: &str) -> bool {
    let right = right_pane(screen);
    panes_files_unfocused_diff_focused(screen)
        && title_has_diff(screen)
        && title_has_files(screen)
        && file_selected(screen, BASE)
        && right.contains(PAIR_BASE_FILE_LINE)
        && !right.contains(PAIR_FIRST_FILE_LINE)
        && footer_has(screen, COMMIT_MSG_BODY)
        && !footer_has(screen, COMMIT_MSG_BODY_TAIL)
        && more_below_hint(screen)
        && no_wrong_overlays(screen)
}

/// Left-pane row up to its last inner column. [`left_tree`] already cuts
/// the pane's right border at the `││` split; a row that still ends with
/// `│` loses it here. Flat panes have no border: the cut there keeps the
/// pane's blank right pad cell, which the trim drops.
fn inner_row(row: &str) -> &str {
    let row = row.trim_end();
    row.strip_suffix('\u{2502}').unwrap_or(row)
}

/// ratatui's default vertical scrollbar cells: `║` track, `█` thumb.
fn is_bar_cell(c: char) -> bool {
    matches!(c, '\u{2551}' | '\u{2588}')
}

/// Footer row text left of the 1-column scrollbar.
fn row_before_bar(row: &str) -> &str {
    let row = inner_row(row);
    row.strip_suffix(is_bar_cell).unwrap_or(row).trim_end()
}

/// A footer row ends with the muted more-below hint: the arrow glyph
/// (`\u{f103}`, `v` in ASCII mode) and the hidden-line count.
fn more_below_hint(screen: &str) -> bool {
    footer_rows(screen).iter().any(|row| {
        row_before_bar(row)
            .split_whitespace()
            .last()
            .and_then(|token| {
                token
                    .strip_prefix('\u{f103}')
                    .or_else(|| token.strip_prefix('v'))
            })
            .is_some_and(|count| !count.is_empty() && count.chars().all(|c| c.is_ascii_digit()))
    })
}

/// The footer paints a 1-column vertical scrollbar in the pane's last inner
/// column: ratatui's `║` track and `█` thumb, on at least two message
/// rows, with a thumb.
fn footer_scrollbar(screen: &str) -> bool {
    let bar: Vec<char> = footer_rows(screen)
        .iter()
        .filter_map(|row| inner_row(row).chars().last())
        .filter(|&c| is_bar_cell(c))
        .collect();
    bar.len() >= 2 && bar.contains(&'\u{2588}')
}

/// Screen cell `(col, row)` of the first `needle` in the left-pane footer.
fn left_footer_cell(screen: &str, needle: &str) -> (u16, u16) {
    let start = pane_body_start(screen.lines().count());
    let rows = left_rows(screen);
    let rule = rows
        .iter()
        .rposition(|row| is_rule(row))
        .unwrap_or_else(|| panic!("footer rule on the left pane:\n{screen}"));
    rows.iter()
        .enumerate()
        .skip(rule + 1)
        .find_map(|(i, row)| {
            let at = row.find(needle)?;
            Some((row[..at].chars().count() as u16, (start + i) as u16))
        })
        .unwrap_or_else(|| panic!("{needle} on the left footer:\n{screen}"))
}

/// Pane look of a run.
#[derive(Clone, Copy)]
enum Look {
    /// The harness baseline config: `viewDefaults.background` `"terminal"`.
    Boxed,
    /// A test-owned config with `viewDefaults.background` `"paint"`.
    Flat,
}

/// Test-owned user config of [`Look::Flat`]: the baseline theme and mode,
/// flat panes.
const FLAT_USER_CONFIG: &str =
    "{\"theme\":\"tokyo-night\",\"themeMode\":\"dark\",\"viewDefaults\":{\"background\":\"paint\"}}\n";

impl Look {
    /// `screen` as the boxed detectors read it.
    ///
    /// Boxed: unchanged. Flat panes have no `││` border pair to split on,
    /// so this inserts one at the pane join on every pane body row. The
    /// join is the title row's first blank after the left pane's title
    /// rule (` files ____ diff ____`): the right pane's first cell. Cells
    /// left of the join keep their columns, so [`left_footer_cell`] still
    /// gives screen cells.
    fn view(self, screen: &str) -> String {
        let Look::Flat = self else {
            return screen.to_string();
        };
        let lines: Vec<&str> = screen.lines().collect();
        let Some(join) = lines.get(1).and_then(|title| {
            let chars: Vec<char> = title.chars().collect();
            let rule = chars.iter().position(|&c| c == '_')?;
            chars[rule..]
                .iter()
                .position(|&c| c == ' ')
                .map(|i| rule + i)
        }) else {
            return screen.to_string();
        };
        let start = pane_body_start(lines.len());
        let end = lines.len().saturating_sub(2);
        lines
            .iter()
            .enumerate()
            .map(|(i, line)| {
                if i < start || i >= end {
                    return line.to_string();
                }
                let chars: Vec<char> = line.chars().collect();
                let cut = join.min(chars.len());
                let left: String = chars[..cut].iter().collect();
                let right: String = chars[cut..].iter().collect();
                format!("{left:<join$}\u{2502}\u{2502}{right}")
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Wait until `pred` holds on the screen as [`Look::view`] shows it.
fn wait(tui: &PtySession, look: Look, pred: impl Fn(&str) -> bool, what: &str) {
    tui.wait_pred(|screen| pred(&look.view(screen)), what, WAIT);
}

fn wheel(tui: &mut PtySession, look: Look, button: u8, needle: &str) {
    let (col, row) = left_footer_cell(&look.view(&tui.screen()), needle);
    for _ in 0..NOTCHES {
        tui.sgr_mouse(button, col, row);
    }
}

/// Esc out of the depth-2 diff to the depth-2 file list. The first Esc
/// may only clear the armed search chip.
fn esc_unfocus_diff(tui: &mut PtySession, look: Look) {
    tui.esc();
    tui.wait_ms(120);
    if panes_files_unfocused_diff_focused(&look.view(&tui.screen())) {
        tui.esc();
    }
    wait(
        tui,
        look,
        panes_files_focused_diff_unfocused,
        "Esc unfocuses the depth 2 diff",
    );
}

/// The commit-files footer at depth 2 scrolls a long message with the
/// wheel, like the graph selection footer, and keeps the file cursor.
///
/// Seed: `root`, `pair base` (adds `base.txt`), then the tip `pair tip`
/// adding `first.txt` and `second.txt`; both long commits carry a body
/// taller than the 4 message rows. On `pair tip`: depth 0 → Enter (depth 1) →
/// Enter on `first.txt` (depth 2: files left, `first.txt` diff right). At
/// depth 1 the left graph footer is the reference: it paints the
/// more-below hint (`v<K>` in ASCII mode) and a 1-column `║`/`█`
/// scrollbar, which validates the detectors. At depth 2, wheel down over
/// the footer's first body line brings `UNIQUE_MSG_BODY_TAIL` in and
/// `UNIQUE_MSG_BODY_LINE` out, while `first.txt` stays selected and its
/// diff stays on the right. At the tail the hint is gone and the scrollbar
/// stays. Wheel up returns to the top, with the hint and the scrollbar.
/// The offset is per commit: scroll `pair tip` to its tail again, Esc to
/// depth 0, `j` onto `pair base`, and drill to its depth 2: its footer
/// starts at the top with the hint. Going back to `pair tip` is not
/// asserted (it may keep its offset, like the graph footer).
///
/// Live PTY 80×28, in both pane looks ([`Look`]). A wheel that moves the
/// file cursor (loading the `second.txt` diff), a footer that clips the
/// message, a missing hint or scrollbar, or one commit's offset leaking
/// onto another cannot pass.
#[test]
fn pty_depth_2_commit_footer_wheel_scrolls_message_boxed() {
    depth_2_commit_footer_wheel_scrolls_message(Look::Boxed);
}

/// [`pty_depth_2_commit_footer_wheel_scrolls_message_boxed`] with flat
/// panes (`viewDefaults.background` `"paint"`, the shipped look): no pane
/// borders, a pad cell on each side of the content.
#[test]
fn pty_depth_2_commit_footer_wheel_scrolls_message_flat() {
    depth_2_commit_footer_wheel_scrolls_message(Look::Flat);
}

fn depth_2_commit_footer_wheel_scrolls_message(look: Look) {
    let (_root, workspace) = daily_workspace();
    seed_two_file_long_message_repo(&workspace, REPO);
    if let Look::Flat = look {
        // The harness keeps a config file the test wrote.
        let file = user_config_file(&workspace.join(".e2e-config"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, FLAT_USER_CONFIG).unwrap();
    }
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.wait_contains("README.md", WAIT);
    if let Look::Flat = look {
        let screen = tui.screen();
        assert!(
            !screen.contains('\u{250C}') && !screen.contains('\u{2510}'),
            "flat panes paint no box corners:\n{screen}"
        );
    }

    tui.search(REPO);
    wait(
        &tui,
        look,
        |screen| right_pane(screen).contains(SUBJECT),
        "search loads the two-file long-message graph",
    );
    tui.tab();
    wait(
        &tui,
        look,
        panes_tree_unfocused_graph_focused,
        "Tab focuses the graph",
    );
    tui.key('j');
    wait(
        &tui,
        look,
        |screen| panes_tree_unfocused_graph_focused(screen) && graph_cursor_on(screen, SUBJECT),
        "j onto the two-file long-message commit",
    );

    tui.enter();
    wait(
        &tui,
        look,
        |screen| {
            panes_files_focused(screen)
                && right_cursor_on(screen, FIRST)
                && footer_has(screen, COMMIT_MSG_BODY)
                && more_below_hint(screen)
                && footer_scrollbar(screen)
        },
        "depth 1 reference: the left graph footer paints the more-below hint and scrollbar",
    );

    tui.enter();
    wait(
        &tui,
        look,
        depth_2_msg_at_top,
        "depth 2: first.txt diff right, first body line on the left footer",
    );

    wheel(&mut tui, look, SGR_WHEEL_DOWN, COMMIT_MSG_BODY);
    wait(
        &tui,
        look,
        depth_2_msg_at_tail,
        "wheel over the depth 2 commit-files footer scrolls the message to its tail \
         (first.txt stays selected, its diff stays on the right)",
    );
    wait(
        &tui,
        look,
        |screen| depth_2_msg_at_tail(screen) && !more_below_hint(screen),
        "at the message tail the more-below hint is gone",
    );
    wait(
        &tui,
        look,
        |screen| depth_2_msg_at_tail(screen) && footer_scrollbar(screen),
        "at the message tail the footer scrollbar stays",
    );

    wheel(&mut tui, look, SGR_WHEEL_UP, COMMIT_MSG_BODY_TAIL);
    wait(
        &tui,
        look,
        depth_2_msg_at_top,
        "wheel up over the depth 2 footer returns to the first body line",
    );
    wait(
        &tui,
        look,
        |screen| depth_2_msg_at_top(screen) && more_below_hint(screen),
        "depth 2 footer at the top ends its last message row with the more-below hint",
    );
    wait(
        &tui,
        look,
        |screen| depth_2_msg_at_top(screen) && footer_scrollbar(screen),
        "depth 2 footer paints a 1-column scrollbar while the message overflows",
    );

    // Per-commit offset: leave `pair tip` scrolled to its tail; another
    // long-message commit starts at the top. Returning to `pair tip` is
    // not asserted (it may keep its offset, like the graph footer).
    wheel(&mut tui, look, SGR_WHEEL_DOWN, COMMIT_MSG_BODY);
    wait(
        &tui,
        look,
        depth_2_msg_at_tail,
        "second wheel down scrolls the depth 2 footer to the tail again",
    );
    esc_unfocus_diff(&mut tui, look);
    tui.esc();
    wait(
        &tui,
        look,
        panes_graph_focused_files_unfocused,
        "Esc pops to depth 1",
    );
    tui.esc();
    wait(
        &tui,
        look,
        panes_tree_focused_graph_unfocused,
        "Esc pops to depth 0 (tree focused, graph right)",
    );
    tui.tab();
    wait(
        &tui,
        look,
        |screen| panes_tree_unfocused_graph_focused(screen) && graph_cursor_on(screen, SUBJECT),
        "Tab focuses the graph on the long-message commit",
    );
    tui.key('j');
    wait(
        &tui,
        look,
        |screen| graph_cursor_on(screen, BASE_SUBJECT),
        "j onto the pair base commit",
    );
    tui.enter();
    wait(
        &tui,
        look,
        |screen| panes_files_focused(screen) && right_cursor_on(screen, BASE),
        "Enter opens the pair base commit files with base.txt selected",
    );
    tui.enter();
    wait(
        &tui,
        look,
        base_depth_2_msg_at_top,
        "another commit's depth 2 footer starts at the first body line with the more-below hint",
    );
}

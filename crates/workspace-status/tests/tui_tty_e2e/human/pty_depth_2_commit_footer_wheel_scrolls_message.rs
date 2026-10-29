use crate::harness::{left_tree, pane_body_start, PtySession, SGR_WHEEL_DOWN};
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
/// `│` loses it here.
fn inner_row(row: &str) -> &str {
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

fn wheel(tui: &mut PtySession, button: u8, needle: &str) {
    let (col, row) = left_footer_cell(&tui.screen(), needle);
    for _ in 0..NOTCHES {
        tui.sgr_mouse(button, col, row);
    }
}

/// Esc out of the depth-2 diff to the depth-2 file list. The first Esc
/// may only clear the armed search chip.
fn esc_unfocus_diff(tui: &mut PtySession) {
    tui.esc();
    tui.wait_ms(120);
    if panes_files_unfocused_diff_focused(&tui.screen()) {
        tui.esc();
    }
    tui.wait_pred(
        panes_files_focused_diff_unfocused,
        "Esc unfocuses the depth 2 diff",
        WAIT,
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
/// Live PTY 80×28. A wheel that moves the file cursor (loading the
/// `second.txt` diff), a footer that clips the message, a missing hint or
/// scrollbar, or one commit's offset leaking onto another cannot pass.
#[test]
fn pty_depth_2_commit_footer_wheel_scrolls_message() {
    let (_root, workspace) = daily_workspace();
    seed_two_file_long_message_repo(&workspace, REPO);
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.wait_contains("README.md", WAIT);

    tui.search(REPO);
    tui.wait_pred(
        |screen| right_pane(screen).contains(SUBJECT),
        "search loads the two-file long-message graph",
        WAIT,
    );
    tui.tab();
    tui.wait_pred(
        panes_tree_unfocused_graph_focused,
        "Tab focuses the graph",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| panes_tree_unfocused_graph_focused(screen) && graph_cursor_on(screen, SUBJECT),
        "j onto the two-file long-message commit",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| {
            panes_files_focused(screen)
                && right_cursor_on(screen, FIRST)
                && footer_has(screen, COMMIT_MSG_BODY)
                && more_below_hint(screen)
                && footer_scrollbar(screen)
        },
        "depth 1 reference: the left graph footer paints the more-below hint and scrollbar",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        depth_2_msg_at_top,
        "depth 2: first.txt diff right, first body line on the left footer",
        WAIT,
    );

    wheel(&mut tui, SGR_WHEEL_DOWN, COMMIT_MSG_BODY);
    tui.wait_pred(
        depth_2_msg_at_tail,
        "wheel over the depth 2 commit-files footer scrolls the message to its tail \
         (first.txt stays selected, its diff stays on the right)",
        WAIT,
    );
    tui.wait_pred(
        |screen| depth_2_msg_at_tail(screen) && !more_below_hint(screen),
        "at the message tail the more-below hint is gone",
        WAIT,
    );
    tui.wait_pred(
        |screen| depth_2_msg_at_tail(screen) && footer_scrollbar(screen),
        "at the message tail the footer scrollbar stays",
        WAIT,
    );

    wheel(&mut tui, SGR_WHEEL_UP, COMMIT_MSG_BODY_TAIL);
    tui.wait_pred(
        depth_2_msg_at_top,
        "wheel up over the depth 2 footer returns to the first body line",
        WAIT,
    );
    tui.wait_pred(
        |screen| depth_2_msg_at_top(screen) && more_below_hint(screen),
        "depth 2 footer at the top ends its last message row with the more-below hint",
        WAIT,
    );
    tui.wait_pred(
        |screen| depth_2_msg_at_top(screen) && footer_scrollbar(screen),
        "depth 2 footer paints a 1-column scrollbar while the message overflows",
        WAIT,
    );

    // Per-commit offset: leave `pair tip` scrolled to its tail; another
    // long-message commit starts at the top. Returning to `pair tip` is
    // not asserted (it may keep its offset, like the graph footer).
    wheel(&mut tui, SGR_WHEEL_DOWN, COMMIT_MSG_BODY);
    tui.wait_pred(
        depth_2_msg_at_tail,
        "second wheel down scrolls the depth 2 footer to the tail again",
        WAIT,
    );
    esc_unfocus_diff(&mut tui);
    tui.esc();
    tui.wait_pred(
        panes_graph_focused_files_unfocused,
        "Esc pops to depth 1",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        panes_tree_focused_graph_unfocused,
        "Esc pops to depth 0 (tree focused, graph right)",
        WAIT,
    );
    tui.tab();
    tui.wait_pred(
        |screen| panes_tree_unfocused_graph_focused(screen) && graph_cursor_on(screen, SUBJECT),
        "Tab focuses the graph on the long-message commit",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| graph_cursor_on(screen, BASE_SUBJECT),
        "j onto the pair base commit",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| panes_files_focused(screen) && right_cursor_on(screen, BASE),
        "Enter opens the pair base commit files with base.txt selected",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        base_depth_2_msg_at_top,
        "another commit's depth 2 footer starts at the first body line with the more-below hint",
        WAIT,
    );
}

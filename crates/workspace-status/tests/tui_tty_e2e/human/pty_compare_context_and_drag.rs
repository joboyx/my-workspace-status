use crate::harness::{tree_row_containing, PtySession};
use crate::seed::{compare_regions_workspace, COMPARE_ADDED_FILE, REGIONS_ALPHA, REGIONS_OMEGA};
use crate::support::{
    compare_regions_diff_focused, crumb_row, open_compare_regions_diff, right_of_split, right_pane,
    GIT_WAIT, RIGHT_PANE_COL, VISUAL_KEY_GAP_MS, WAIT,
};

/// Unchanged `regions.txt` line between the two hunks. Default `-U3`
/// context hides it; full-file context shows it.
const BETWEEN_HUNKS: &str = "pad-3";

/// xterm SGR left-button drag (`Cb` 0 + motion bit 32).
const SGR_LEFT_DRAG: u8 = 32;

/// First column inside the left pane box (column 0 is its border).
const LEFT_INNER_COL: u16 = 1;

/// The diff cursor bar (`▌`) sits on the row that shows `needle`.
fn diff_cursor_on(screen: &str, needle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains(needle) && line.contains('▌'))
}

fn diff_header_full(screen: &str) -> bool {
    right_pane(screen)
        .lines()
        .next()
        .is_some_and(|line| line.contains(" · full"))
}

/// Compare diff at default context: both hunks, the gap between them hidden.
fn compare_hunk_only(screen: &str) -> bool {
    compare_regions_diff_focused(screen)
        && !right_pane(screen).contains(BETWEEN_HUNKS)
        && !diff_header_full(screen)
}

/// Compare diff at full-file context: the gap between the hunks is visible.
fn compare_full_file(screen: &str) -> bool {
    compare_regions_diff_focused(screen)
        && right_pane(screen).contains(BETWEEN_HUNKS)
        && diff_header_full(screen)
}

/// A pane box edge leaked into the copy: a row that starts or ends with `│`.
///
/// The diff gutter `│` (between the line number and the text) is pane text,
/// so a bare `contains('│')` cannot tell the two apart.
fn has_box_border(text: &str) -> bool {
    text.lines()
        .any(|line| line.starts_with('│') || line.trim_end().ends_with('│'))
}

/// 0-based screen row of a right-pane cell that contains `needle`.
fn right_row_containing(screen: &str, needle: &str) -> Option<u16> {
    screen
        .lines()
        .position(|line| right_of_split(line).contains(needle))
        .map(|i| i as u16)
}

fn sgr_release(tui: &mut PtySession, col: u16, row: u16) {
    let seq = format!(
        "\x1b[<0;{};{}m",
        col.saturating_add(1),
        row.saturating_add(1)
    );
    tui.send_bytes(seq.as_bytes());
}

/// Press at `from`, drag to `to`, release there.
fn drag(tui: &mut PtySession, from: (u16, u16), to: (u16, u16)) {
    tui.sgr_mouse(0, from.0, from.1);
    tui.sgr_mouse(SGR_LEFT_DRAG, to.0, to.1);
    sgr_release(tui, to.0, to.1);
}

/// Ctrl-o on a compare diff toggles full-file context and keeps the cursor
/// on its hunk.
///
/// Docs: on a focused diff, Ctrl-o reloads with unlimited context and the
/// current hunk stays on screen; a second Ctrl-o restores hunk-only. A
/// compare tab uses the same key on its own diff.
///
/// Live PTY on `compare_regions_workspace`: open `app ↔ origin/main` by real
/// input, move the diff cursor with plain `j` to the second hunk (OMEGA),
/// Ctrl-o. The unchanged lines between the hunks (`pad-3`) appear, the
/// header shows ` · full`, and the cursor bar is still on OMEGA-NEW. A
/// second Ctrl-o hides `pad-3` again with the cursor still on OMEGA-NEW.
///
/// A reload that keeps the old row index lands the cursor on a gap line in
/// the longer full-file list, so the cursor claim fails.
#[test]
fn pty_compare_ctrl_o_full_file_context_keeps_hunk() {
    let (_root, workspace) = compare_regions_workspace();
    let mut tui = open_compare_regions_diff(&workspace);
    tui.wait_pred(
        compare_hunk_only,
        "compare diff opens at default context (gap between hunks hidden)",
        WAIT,
    );

    for _ in 0..40 {
        if diff_cursor_on(&tui.screen(), REGIONS_OMEGA) {
            break;
        }
        tui.letter_press('j');
        tui.wait_ms(VISUAL_KEY_GAP_MS);
    }
    tui.wait_pred(
        |screen| compare_hunk_only(screen) && diff_cursor_on(screen, REGIONS_OMEGA),
        "plain j moves the compare diff cursor to the second hunk (OMEGA-NEW)",
        WAIT,
    );

    tui.ctrl_letter('o');
    tui.wait_pred(
        |screen| compare_full_file(screen) && diff_cursor_on(screen, REGIONS_OMEGA),
        "Ctrl-o shows full-file context with the cursor still on OMEGA-NEW",
        GIT_WAIT,
    );

    tui.ctrl_letter('o');
    tui.wait_pred(
        |screen| compare_hunk_only(screen) && diff_cursor_on(screen, REGIONS_OMEGA),
        "second Ctrl-o restores hunk-only with the cursor still on OMEGA-NEW",
        GIT_WAIT,
    );
}

/// Left drag in each compare pane copies that pane's text only.
///
/// Docs: a drag in a pane body selects the text under it and copies it on
/// release (OSC 52). The selection stays in the pane where the press
/// landed. A compare tab paints the same two panes.
///
/// Live PTY on `compare_regions_workspace`: open `app ↔ origin/main` by real
/// input. First drag: press in the diff pane on the row above ALPHA-NEW,
/// drag into the file list one row below ALPHA-NEW, release. The payload
/// holds ALPHA-NEW and no file-list text or box border (the diff gutter `│`
/// is pane text). Second drag: press
/// at the start of the regions.txt file row, drag into the diff pane one
/// row down, release. The payload holds regions.txt and no diff text or
/// box border. Each release flashes the `copied` toast.
#[test]
fn pty_compare_drag_select_copies_pane_text() {
    let (_root, workspace) = compare_regions_workspace();
    let mut tui = open_compare_regions_diff(&workspace);
    let screen = tui.screen();
    let alpha = right_row_containing(&screen, REGIONS_ALPHA)
        .unwrap_or_else(|| panic!("ALPHA-NEW diff row:\n{screen}"));

    drag(
        &mut tui,
        (RIGHT_PANE_COL, alpha - 1),
        (LEFT_INNER_COL, alpha + 1),
    );
    tui.wait_clipboard_pred(
        |payloads| {
            payloads.iter().any(|text| {
                text.contains(REGIONS_ALPHA)
                    && !has_box_border(text)
                    && !text.contains(COMPARE_ADDED_FILE)
            })
        },
        "OSC 52 payload is compare diff text: ALPHA-NEW, no file list, no border",
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
        "diff drag release flashes the copied toast",
        WAIT,
    );

    let screen = tui.screen();
    let row = tree_row_containing(&screen, "regions.txt")
        .unwrap_or_else(|| panic!("regions.txt file row:\n{screen}"));
    drag(&mut tui, (LEFT_INNER_COL, row), (RIGHT_PANE_COL, row + 1));
    tui.wait_clipboard_pred(
        |payloads| {
            payloads.len() == 2
                && payloads[1].contains("regions.txt")
                && !payloads[1].contains('│')
                && !payloads[1].contains(REGIONS_ALPHA)
                && !payloads[1].contains(REGIONS_OMEGA)
                && !payloads[1].contains("COMMITTED")
        },
        "OSC 52 payload is compare file-list text: regions.txt, no diff, no border",
        WAIT,
    );
}

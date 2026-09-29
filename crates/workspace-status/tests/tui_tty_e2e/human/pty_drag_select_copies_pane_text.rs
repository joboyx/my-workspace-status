use crate::harness::{tree_row_containing, PtySession};
use crate::seed::daily_workspace;
use crate::support::{crumb_row, documented_launch_first_paint, RIGHT_PANE_COL, SETTLE_MS, WAIT};

/// xterm SGR left-button drag (`Cb` 0 + motion bit 32).
const SGR_LEFT_DRAG: u8 = 32;

/// First column inside the tree box (column 0 is its border).
const TREE_INNER_COL: u16 = 1;

fn sgr_release(tui: &mut PtySession, col: u16, row: u16) {
    let seq = format!(
        "\x1b[<0;{};{}m",
        col.saturating_add(1),
        row.saturating_add(1)
    );
    tui.send_bytes(seq.as_bytes());
}

/// Left drag inside the tree pane copies that pane's text only.
///
/// Docs: a drag in a pane body selects the text under it and copies it on
/// release. The selection stays in the pane where the press landed.
///
/// Live PTY after first paint: SGR press at the start of the README.md
/// tree row, motion-bit drag to the right pane one row down (past the
/// divider), release. The OSC 52 payload holds the README.md label and
/// no box border `│` or right-pane diff text. The breadcrumb row flashes the
/// `copied` toast.
///
/// A plain click, a copy that spills into the right pane, or a copy that
/// keeps the border cannot pass.
#[test]
fn pty_drag_select_copies_pane_text() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file-diff (drag has not run)",
        WAIT,
    );
    let row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README.md row:\n{}", tui.screen()));

    tui.sgr_click(TREE_INNER_COL, row);
    tui.wait_ms(SETTLE_MS);
    assert!(
        tui.clipboard_payloads().is_empty(),
        "plain click must not copy:\n{}",
        tui.screen()
    );

    tui.sgr_mouse(0, TREE_INNER_COL, row);
    tui.sgr_mouse(SGR_LEFT_DRAG, RIGHT_PANE_COL, row + 1);
    sgr_release(&mut tui, RIGHT_PANE_COL, row + 1);

    tui.wait_clipboard_pred(
        |payloads| {
            payloads.iter().any(|text| {
                text.contains("README.md")
                    && !text.contains('│')
                    && !text.contains("UNSTAGED")
                    && !text.contains("+dirty")
                    && !text.contains("app/README.md")
            })
        },
        "OSC 52 payload is tree text: README.md, no border, no diff pane",
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

use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{right_pane, tree_cursor_on, tree_has, WAIT};

/// A heart with its VS16 selector: `unicode-width` measures it two columns.
const EMOJI: &str = "\u{2764}\u{fe0f}";
const NAME_AFTER_EMOJI: &str = "-note.txt";
const END_OF_FRAME: &[u8] = b"\x1b[?2026l";

/// True when `bytes` starts with a cursor move (`ESC [ row ; col H`).
fn starts_with_cursor_move(bytes: &[u8]) -> bool {
    let Some(rest) = bytes.strip_prefix(b"\x1b[") else {
        return false;
    };
    let Some(end) = rest.iter().position(|byte| *byte == b'H') else {
        return false;
    };
    std::str::from_utf8(&rest[..end])
        .ok()
        .and_then(|args| args.split_once(';'))
        .is_some_and(|(row, col)| row.parse::<u16>().is_ok() && col.parse::<u16>().is_ok())
}

/// The bytes right after each emoji in `raw`.
fn after_each_emoji(raw: &[u8]) -> Vec<&[u8]> {
    (0..raw.len())
        .filter_map(|start| raw[start..].strip_prefix(EMOJI.as_bytes()))
        .collect()
}

/// True once the frame that holds the last emoji in `raw` has ended.
///
/// Bytes arrive in order, so all the bytes of that frame after the emoji are there.
fn last_emoji_frame_ended(raw: &[u8]) -> bool {
    after_each_emoji(raw).last().is_some_and(|after| {
        after
            .windows(END_OF_FRAME.len())
            .any(|bytes| bytes == END_OF_FRAME)
    })
}

/// A row keeps its columns when a VS16 emoji paints over older text.
///
/// `vt100` counts this emoji as one column and a real terminal draws it as two,
/// so the screen cannot show the shift. The claim reads the bytes instead.
///
/// The cursor moves from `README.md` to the emoji file, so the right pane's path
/// header changes from `app/README.md` to the emoji name. The cell the emoji
/// covers held `E`, and that change is what makes the frame diff consider it. A
/// plain write of that cell after the emoji lands one column late on a terminal
/// that draws the emoji wide, and the rest of the row follows it. Each emoji
/// must be followed by a cursor move.
#[test]
fn pty_vs16_emoji_row_moves_the_cursor_past_the_emoji() {
    let (_root, workspace) = daily_workspace();
    fs::write(
        workspace
            .join("app")
            .join(format!("{EMOJI}{NAME_AFTER_EMOJI}")),
        "note\n",
    )
    .unwrap();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            tree_has(screen, NAME_AFTER_EMOJI)
                && tree_cursor_on(screen, "README.md")
                && right_pane(screen).contains("app/README.md")
        },
        "launch lists the emoji file, with the README.md path header on the right",
        WAIT,
    );

    let from = tui.raw_output().len();
    tui.key('j');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, NAME_AFTER_EMOJI)
                && right_pane(screen).contains(NAME_AFTER_EMOJI)
                && !right_pane(screen).contains("app/README.md")
        },
        "j moves the cursor to the emoji file and its path header replaces README.md",
        WAIT,
    );
    tui.wait_pred(
        |_| last_emoji_frame_ended(&tui.raw_output()[from..]),
        "the frame that paints the emoji ends",
        WAIT,
    );

    let raw = tui.raw_output();
    for after in after_each_emoji(&raw[from..]) {
        assert!(
            starts_with_cursor_move(after),
            "the bytes after the emoji must move the cursor, not write the covered cell: {:?}",
            String::from_utf8_lossy(&after[..after.len().min(24)])
        );
    }
}

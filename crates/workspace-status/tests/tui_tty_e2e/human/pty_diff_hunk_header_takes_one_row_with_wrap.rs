use crate::harness::PtySession;
use crate::seed::hunk_context_markdown_workspace;
use crate::support::{right_pane, tree_cursor_on, WAIT};

/// Start of the hunk header git writes for the `notes.md` change; its
/// function context (the long first line of the file) follows.
const HUNK_HEADER: &str = "@@ -7,6 +7,6 @@ Here the second paragraph";

/// The diff shows the `notes.md` hunk with wrap on, and the right-pane row
/// under the hunk header is the hunk's first code line (`- item 7`), not
/// a wrapped tail of the function context.
fn hunk_header_on_one_row(screen: &str) -> bool {
    let pane = right_pane(screen);
    let rows: Vec<&str> = pane.lines().collect();
    let Some(at) = rows.iter().position(|row| row.contains(HUNK_HEADER)) else {
        return false;
    };
    tree_cursor_on(screen, "notes.md")
        && pane.contains("· wrap")
        && rows.get(at + 1).is_some_and(|row| row.contains("- item 7"))
}

/// With wrap on, a hunk header longer than the diff pane takes one row.
///
/// `notes.md` changes `- item 10`, so its hunk starts at line 7 and git
/// adds the long first line of the file after the second `@@` as function
/// context. Wrap is on at launch. The hunk header clips at the pane edge,
/// as it does with wrap off, and the next row is the first context line of
/// the hunk. A header that wraps puts its tail (`worktree co`) on a second
/// row with no gutter, under the header, and cannot pass.
#[test]
fn pty_diff_hunk_header_takes_one_row_with_wrap() {
    let (_root, workspace) = hunk_context_markdown_workspace("ws-tui-tty-hunk-header");
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        hunk_header_on_one_row,
        "launch paints the long notes.md hunk header on one row with wrap on",
        WAIT,
    );
}

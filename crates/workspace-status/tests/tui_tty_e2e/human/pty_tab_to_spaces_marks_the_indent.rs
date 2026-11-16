use crate::harness::PtySession;
use crate::seed::{tab_to_spaces_workspace, TAB_SPACES_TEXT};
use crate::support::{assert_word_bgs, right_pane, tree_cursor_on, WAIT};

/// Sign, indent, and first words of the `+` line (indent of 4 spaces).
const ADD_NEEDLE: &str = "+    Tab indented";
/// The same cells of the `-` line (a tab, painted as 4 spaces).
const DEL_NEEDLE: &str = "-    Tab indented";
/// Word background (`#`) or row background (`.`) of each needle cell: the
/// sign and the text keep the row background, the 4 indent columns take
/// the word background.
const INDENT_MARKED: &str = ".####............";

/// `notes.txt` focused, wrap on and no `-w`, both needles painted, and the
/// line wrapped (no row holds the whole text).
fn tab_spaces_diff(screen: &str) -> bool {
    let right = right_pane(screen);
    let header = right.lines().next().unwrap_or("");
    tree_cursor_on(screen, "notes.txt")
        && header.contains("notes.txt")
        && header.contains("· wrap")
        && !header.contains("· -w")
        && right.contains(ADD_NEEDLE)
        && right.contains(DEL_NEEDLE)
        && !right.lines().any(|row| row.contains(TAB_SPACES_TEXT))
}

/// A tab that became 4 spaces is visible while whitespace is shown.
///
/// `notes.txt` indents its one long line with a tab in HEAD and with 4
/// spaces in the worktree, so both rows paint the same text. With `w` off,
/// the 4 indent columns of the `-` and the `+` row sit on the word
/// background and the rest of the line keeps the row background. A word
/// highlight that compares the painted text (tab already expanded) finds
/// no change and leaves both indents on the row background.
#[test]
fn pty_tab_to_spaces_marks_the_indent() {
    let (_root, workspace) = tab_to_spaces_workspace("ws-tui-tty-tab-to-spaces");
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        tab_spaces_diff,
        "launch shows the wrapped notes.txt diff without -w",
        WAIT,
    );
    assert_word_bgs(
        &tui,
        (ADD_NEEDLE, INDENT_MARKED),
        (DEL_NEEDLE, INDENT_MARKED),
        "whitespace shown",
    );
}

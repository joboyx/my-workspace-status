use crate::harness::PtySession;
use crate::seed::{long_line_markdown_workspace, LONG_LINE_NEW, LONG_LINE_OLD};
use crate::support::{right_pane, tree_cursor_on, WAIT};

/// `text` without the chrome glyphs a painted row can glue to a word: the
/// `+` / `-` sign before it, punctuation, the rule or scrollbar after it.
fn trimmed(token: &str) -> &str {
    token.trim_matches(|c: char| !c.is_alphanumeric())
}

/// The trimmed words of `line`, in order.
fn words(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(trimmed)
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

/// Right-pane rows that paint code of the long line: rows with a word of
/// the line on them (the header, section label, and hunk header have
/// none).
fn code_rows(screen: &str) -> Vec<String> {
    let vocab = words(LONG_LINE_OLD);
    right_pane(screen)
        .lines()
        .filter(|row| {
            row.split_whitespace()
                .any(|token| vocab.iter().any(|word| word == trimmed(token)))
        })
        .map(str::to_string)
        .collect()
}

/// The words the code rows paint, in screen order, with line numbers and
/// bare chrome dropped.
fn painted_words(screen: &str) -> Vec<String> {
    code_rows(screen)
        .iter()
        .flat_map(|row| words(row))
        .filter(|word| !word.chars().all(|c| c.is_ascii_digit()))
        .collect()
}

/// The `-` and `+` lines both wrap onto more than one row, and the rows
/// hold every word of both lines whole and in order: no row ends inside a
/// word (`becom` / `e`, `worktree c` / `opy`, `s` / `o every`).
fn long_line_wrapped_at_words(screen: &str) -> bool {
    let mut expected = words(LONG_LINE_OLD);
    expected.extend(words(LONG_LINE_NEW));
    tree_cursor_on(screen, "notes.md")
        && right_pane(screen).contains("UNSTAGED")
        && right_pane(screen).contains("· wrap")
        && code_rows(screen).len() >= 4
        && painted_words(screen) == expected
}

/// With wrap on, a long markdown line in the diff breaks between words.
///
/// `notes.md` changes one long line by doubling two spaces. Wrap is on at
/// launch, so the `-` and `+` lines each take several rows of the narrow
/// inline diff. Every row ends with a whole word and the next row starts
/// with the next word, on both lines: the words the rows paint, in screen
/// order, are the words of the `-` line and then of the `+` line. A wrap
/// that cuts a word at the pane edge (`becom` / `e double`) paints a
/// fragment that is not a word of the line, and cannot pass.
#[test]
fn pty_diff_wraps_code_at_word_boundaries() {
    let (_root, workspace) = long_line_markdown_workspace("ws-tui-tty-word-wrap");
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        long_line_wrapped_at_words,
        "launch wraps the notes.md long line at word boundaries on - and +",
        WAIT,
    );
}

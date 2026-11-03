use std::path::PathBuf;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    cursor_key, documented_launch_first_paint, prove_keys_handled, status_row, tree_cursor_on,
    GIT_WAIT, WAIT,
};

/// Words with a doubled nav letter (`ll`). `l` is a held nav key, and in
/// Normal mode the event loop drops its queued copy, so these catch a text
/// field that loses the second `l` when the whole word arrives in one read.
const WORDS: [&str; 2] = ["full", "skill"];

/// The search-in-files dialog title for the `app` checkout (tree cursor on
/// README.md).
const SEARCH_FILES_TITLE: &str = "Search · app";
/// The Quick Open files-mode title for the `app` checkout.
const QUICK_OPEN_FILES_TITLE: &str = "Go to file · app";

/// Launch the daily seed and wait for the documented first paint: tree
/// focused, cursor on the dirty README.md, its diff loaded.
fn open_daily() -> (PathBuf, PtySession) {
    let (root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: tree cursor on the dirty README.md",
        WAIT,
    );
    (root, tui)
}

/// True when no word of [`WORDS`] shows on `text`.
fn no_word(text: &str) -> bool {
    WORDS.iter().all(|word| !text.contains(word))
}

/// Type each word of [`WORDS`] into an open text field, the whole word in
/// one PTY write, and wait until the field paints it.
///
/// Before each word, `empty` must hold (the field is open and shows no
/// typed text). After the write, `shows(screen, word)` must hold (the
/// field paints the whole word right before its caret). `clear` then
/// empties the field, or closes and opens it again, for the next word.
fn type_each_word_in_one_write(
    tui: &mut PtySession,
    field: &str,
    empty: impl Fn(&str) -> bool,
    shows: impl Fn(&str, &str) -> bool,
    clear: impl Fn(&mut PtySession),
) {
    for word in WORDS {
        tui.wait_pred(&empty, &format!("the {field} is open and empty"), WAIT);
        tui.send_bytes(word.as_bytes());
        tui.wait_pred(
            |screen| shows(screen, word),
            &format!("the {field} paints `{word}▏` after `{word}` in one write"),
            WAIT,
        );
        clear(tui);
    }
}

/// True when the `/` SEARCH prompt is open on the status row and its query
/// is `query`, followed by the `▏` caret.
fn search_prompt_shows(screen: &str, query: &str) -> bool {
    let status = status_row(screen);
    status.contains("SEARCH") && status.contains(&format!("{query}▏"))
}

/// `/` opens the SEARCH prompt; `full` and then `skill`, each in one write,
/// paint whole before the caret (`full▏`, `skill▏`).
///
/// A regression drops the second `l` as held-nav backlog, so the prompt
/// paints `ful▏` or `skil▏` and the wait times out.
#[test]
fn pty_search_prompt_types_doubled_nav_letters_in_one_write() {
    let (_root, mut tui) = open_daily();
    tui.key('/');
    type_each_word_in_one_write(
        &mut tui,
        "SEARCH prompt",
        |screen| search_prompt_shows(screen, "") && no_word(status_row(screen)),
        search_prompt_shows,
        |tui| {
            tui.esc();
            tui.wait_pred(
                |screen| {
                    !status_row(screen).contains("SEARCH") && tree_cursor_on(screen, "README.md")
                },
                "Esc closes the SEARCH prompt",
                WAIT,
            );
            tui.key('/');
        },
    );
}

/// `:` opens Quick Open commands with `>` typed; `full` and then `skill`,
/// each in one write, paint whole after the prefix (`>full▏`, `>skill▏`).
///
/// A regression drops the second `l`, so the filter paints `>ful▏` or
/// `>skil▏` and the wait times out.
#[test]
fn pty_quick_open_commands_types_doubled_nav_letters_in_one_write() {
    let (_root, mut tui) = open_daily();
    tui.key(':');
    type_each_word_in_one_write(
        &mut tui,
        "Quick Open commands filter",
        |screen| screen.contains("Enter run") && screen.contains(">▏") && no_word(screen),
        |screen, word| screen.contains("Enter run") && screen.contains(&format!(">{word}▏")),
        |tui| tui.esc(),
    );
}

/// Ctrl-p opens Quick Open files; `full` and then `skill`, each in one
/// write, paint whole before the caret (`full▏`, `skill▏`).
///
/// A regression drops the second `l`, so the query row paints `ful▏` or
/// `skil▏` and the wait times out.
#[test]
fn pty_quick_open_files_types_doubled_nav_letters_in_one_write() {
    let (_root, mut tui) = open_daily();
    tui.ctrl_letter('p');
    type_each_word_in_one_write(
        &mut tui,
        "Quick Open files query",
        |screen| {
            screen.contains(QUICK_OPEN_FILES_TITLE)
                && screen.contains("▏type a file name…")
                && no_word(screen)
        },
        |screen, word| {
            screen.contains(QUICK_OPEN_FILES_TITLE) && screen.contains(&format!("{word}▏"))
        },
        |tui| tui.esc(),
    );
}

/// Ctrl-f opens search in files; `full` and then `skill`, each in one
/// write, paint whole before the caret (`full▏`, `skill▏`).
///
/// A regression drops the second `l`, so the query row paints `ful▏` or
/// `skil▏` and the wait times out.
#[test]
fn pty_search_in_files_types_doubled_nav_letters_in_one_write() {
    let (_root, mut tui) = open_daily();
    tui.ctrl_letter('f');
    type_each_word_in_one_write(
        &mut tui,
        "search-in-files query",
        |screen| {
            screen.contains(SEARCH_FILES_TITLE)
                && screen.contains("▏type to search…")
                && no_word(screen)
        },
        |screen, word| screen.contains(SEARCH_FILES_TITLE) && screen.contains(&format!("{word}▏")),
        |tui| tui.esc(),
    );
}

/// The `;` Comment overlay is open: its `body:` row and its footer hints.
fn comment_open(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
}

/// Tab focuses the dirty README diff and `;` opens the Comment overlay on
/// its line; `full` and then `skill`, each in one write, paint whole before
/// the body caret (`full▏`, `skill▏`).
///
/// A regression drops the second `l`, so the body paints `ful▏` or `skil▏`
/// and the wait times out.
#[test]
fn pty_comment_body_types_doubled_nav_letters_in_one_write() {
    let (_root, mut tui) = open_daily();
    tui.tab();
    tui.wait_pred(
        |screen| !tree_cursor_on(screen, "README.md") && screen.contains("UNSTAGED"),
        "Tab focuses the dirty README diff",
        WAIT,
    );
    tui.key(';');
    type_each_word_in_one_write(
        &mut tui,
        "Comment body",
        |screen| comment_open(screen) && no_word(screen),
        |screen, word| comment_open(screen) && screen.contains(&format!("{word}▏")),
        |tui| {
            tui.esc();
            tui.wait_pred(
                |screen| !comment_open(screen),
                "Esc cancels the Comment overlay",
                WAIT,
            );
            tui.key(';');
        },
    );
}

/// Screen row of the focused tree cursor bar (`▌`).
fn tree_cursor_row(screen: &str) -> Option<usize> {
    cursor_key(screen).map(|(row, _)| row)
}

/// In Normal mode, `jj` in one write moves the tree cursor exactly one row,
/// and a later single `j` moves it one more row.
///
/// The event loop drops a queued copy of a held nav key, so a held `j`
/// does not flush as a burst after release. A regression that keeps the
/// queued copy moves two rows on `jj`; a `j` that no longer works leaves
/// the cursor in place on the single write.
#[test]
fn pty_normal_mode_drops_a_doubled_nav_key_in_one_write() {
    let (_root, mut tui) = open_daily();
    let start = tree_cursor_row(&tui.screen()).expect("the tree cursor bar is painted");

    tui.send_bytes(b"jj");
    tui.wait_pred(
        |screen| tree_cursor_row(screen).is_some_and(|row| row != start),
        "`jj` in one write moves the tree cursor",
        WAIT,
    );
    prove_keys_handled(&mut tui);
    let screen = tui.screen();
    assert_eq!(
        tree_cursor_row(&screen),
        Some(start + 1),
        "`jj` in one write moves the tree cursor exactly one row:\n{screen}"
    );

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_row(screen) == Some(start + 2),
        "a single `j` write moves the tree cursor one more row",
        WAIT,
    );
}

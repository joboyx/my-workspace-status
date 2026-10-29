use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Typed query. It is in no seeded file, path, or command title, so it
/// shows on screen only in the dialog's query row. It has no `h`/`j`/`k`/`l`,
/// so it types in one burst (`discard_held_nav_backlog` drops a held
/// same-letter nav burst).
const QUERY: &str = "zorbatron";
/// Search dialog title for the `app` checkout (tree cursor on README.md).
const SEARCH_TITLE: &str = "Search · app";
/// Search dialog query row while the query is empty: caret, then the
/// placeholder.
const SEARCH_EMPTY_ROW: &str = "▏type to search…";
/// Quick Open files-mode title for the `app` checkout.
const FILES_TITLE: &str = "Go to file · app";
/// Quick Open files-mode query row while the query is empty: caret, then
/// the placeholder.
const FILES_EMPTY_ROW: &str = "▏type a file name…";
/// Quick Open commands-mode query row with nothing after the `>` prefix.
const COMMANDS_EMPTY_ROW: &str = ">▏";
/// A palette command row that an empty commands filter paints in the first
/// screen of the list for the tree focus, and that [`QUERY`] filters out.
const COMMAND_ROW: &str = "Search in files";

/// Tree cursor on README.md, and neither the search dialog nor Quick Open
/// is painted.
fn first_paint(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && search_closed(screen)
        && quick_open_closed(screen)
}

/// Wait for [`first_paint`] after the seeded tree and diff load.
fn wait_first_paint(tui: &PtySession) {
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        first_paint,
        "first paint: tree cursor on README.md; no search dialog, no Quick Open",
        WAIT,
    );
}

/// True when the query row of an open dialog shows `text` followed by the
/// `▏` caret.
fn query_row_shows(screen: &str, text: &str) -> bool {
    screen.contains(&format!("{text}▏"))
}

/// The search dialog is painted: its `Search · app` title and its
/// `Enter open` footer hint.
fn search_open(screen: &str) -> bool {
    screen.contains(SEARCH_TITLE) && screen.contains("Enter open")
}

/// The search dialog is open and its query row shows [`QUERY`].
fn search_open_with_query(screen: &str) -> bool {
    search_open(screen) && query_row_shows(screen, QUERY)
}

/// The search dialog is open, its query row shows the empty-query
/// placeholder, and [`QUERY`] shows nowhere on screen.
fn search_open_empty(screen: &str) -> bool {
    search_open(screen) && screen.contains(SEARCH_EMPTY_ROW) && !screen.contains(QUERY)
}

/// No search dialog title on screen, and the tree cursor is back on
/// README.md.
fn search_closed(screen: &str) -> bool {
    !screen.contains("Search · ") && tree_cursor_on(screen, "README.md")
}

/// Quick Open is painted in files mode: its `Go to file · app` title and
/// its `Enter open` footer hint, and no commands-mode `Enter run` hint.
fn files_open(screen: &str) -> bool {
    screen.contains(FILES_TITLE) && screen.contains("Enter open") && !screen.contains("Enter run")
}

/// Quick Open files mode is open, its query row shows the empty-query
/// placeholder, and [`QUERY`] shows nowhere on screen.
fn files_open_empty(screen: &str) -> bool {
    files_open(screen) && screen.contains(FILES_EMPTY_ROW) && !screen.contains(QUERY)
}

/// Quick Open is painted in commands mode: the `Commands` title and the
/// `Enter run` footer hint, and no files-mode `Enter open` hint.
fn commands_open(screen: &str) -> bool {
    screen.contains("Commands") && screen.contains("Enter run") && !screen.contains("Enter open")
}

/// Quick Open commands mode is open, its query row shows only the `>`
/// prefix and the caret, [`QUERY`] shows nowhere on screen, and the full
/// list paints [`COMMAND_ROW`].
fn commands_open_prefix_only(screen: &str) -> bool {
    commands_open(screen)
        && screen.contains(COMMANDS_EMPTY_ROW)
        && !screen.contains(QUERY)
        && screen.contains(COMMAND_ROW)
}

/// No Quick Open on screen (neither mode's title nor footer hint), and
/// the tree cursor is back on README.md.
fn quick_open_closed(screen: &str) -> bool {
    !screen.contains("Go to file")
        && !screen.contains("Enter run")
        && !screen.contains("Enter open")
        && tree_cursor_on(screen, "README.md")
}

/// Ctrl-f opens `Search · app`, typing [`QUERY`] fills its query row, Esc
/// clears the query and keeps the dialog open, and a second Esc on the
/// empty query closes it.
///
/// Fail if the first Esc closes the dialog or leaves [`QUERY`] in the
/// query row, or if the second Esc leaves the dialog up.
#[test]
fn pty_search_in_files_esc_clears_query_then_closes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.ctrl_letter('f');
    tui.wait_pred(
        search_open_empty,
        "Ctrl-f opens `Search · app` with the empty-query placeholder",
        WAIT,
    );
    tui.keys(QUERY);
    tui.wait_pred(
        search_open_with_query,
        "the search query row shows the typed query",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        search_open_empty,
        "Esc with a typed query keeps `Search · app` open and clears the query row to the placeholder",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        search_closed,
        "Esc on the empty query closes the search dialog",
        WAIT,
    );
}

/// Ctrl-p opens Quick Open on files, typing [`QUERY`] fills its query row,
/// Esc clears the query and keeps the overlay open in files mode, and a
/// second Esc on the empty query closes it.
///
/// Fail if the first Esc closes Quick Open or leaves [`QUERY`] in the
/// query row, or if the second Esc leaves the overlay up.
#[test]
fn pty_quick_open_files_esc_clears_query_then_closes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.ctrl_letter('p');
    tui.wait_pred(
        files_open_empty,
        "Ctrl-p opens Quick Open files (`Go to file · app`) with the empty-query placeholder",
        WAIT,
    );
    tui.keys(QUERY);
    tui.wait_pred(
        |screen| files_open(screen) && query_row_shows(screen, QUERY),
        "the Quick Open files query row shows the typed query",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        files_open_empty,
        "Esc with a typed query keeps Quick Open files open and clears the query row to the placeholder",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        quick_open_closed,
        "Esc on the empty query closes Quick Open",
        WAIT,
    );
}

/// `:` opens Quick Open on commands with `>` typed, typing [`QUERY`] after
/// it filters the list, Esc clears the text back to `>` and keeps the
/// overlay open in commands mode with the full list, and a second Esc on
/// `>` closes it.
///
/// Fail if the first Esc closes Quick Open, drops the `>` prefix (files
/// mode), or leaves [`QUERY`] in the query row, or if the second Esc
/// leaves the overlay up.
#[test]
fn pty_quick_open_commands_esc_keeps_prefix_then_closes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.key(':');
    tui.wait_pred(
        commands_open_prefix_only,
        "`:` opens Quick Open commands with only `>` in the query row and the full list",
        WAIT,
    );
    tui.keys(QUERY);
    tui.wait_pred(
        |screen| {
            commands_open(screen)
                && query_row_shows(screen, &format!(">{QUERY}"))
                && !screen.contains(COMMAND_ROW)
        },
        "the commands query row shows `>` plus the typed text and the list is filtered",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        commands_open_prefix_only,
        "Esc with typed text keeps Quick Open commands open, the query row back to `>`, and the full list",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        quick_open_closed,
        "Esc on the bare `>` query closes Quick Open",
        WAIT,
    );
}

/// Ctrl-p then `#` plus [`QUERY`] hands off to `Search · app` with
/// [`QUERY`] as its query, Esc clears the query and keeps the search dialog
/// open, and a second Esc on the empty query closes it without bringing
/// Quick Open back.
///
/// Fail if the `#` hand-off paints no search dialog with the query, if
/// the first Esc closes the dialog or leaves [`QUERY`] in the query row,
/// or if the second Esc leaves the dialog or Quick Open up.
#[test]
fn pty_quick_open_hash_search_esc_clears_query_then_closes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.ctrl_letter('p');
    tui.wait_pred(
        files_open_empty,
        "Ctrl-p opens Quick Open files (`Go to file · app`) with the empty-query placeholder",
        WAIT,
    );
    tui.key('#');
    tui.keys(QUERY);
    tui.wait_pred(
        |screen| search_open_with_query(screen) && !screen.contains("Go to file"),
        "`#` plus the query closes Quick Open and opens `Search · app` with the query",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        search_open_empty,
        "Esc with a typed query keeps `Search · app` open and clears the query row to the placeholder",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| search_closed(screen) && quick_open_closed(screen),
        "Esc on the empty query closes the search dialog and does not reopen Quick Open",
        WAIT,
    );
}

/// Ctrl-f opens `Search · app`, typing [`QUERY`] fills its query row,
/// Ctrl-f with the typed query closes the dialog at once, and the next
/// Ctrl-f brings it back with [`QUERY`] still in the query row.
///
/// Fail if the second Ctrl-f leaves the dialog up, or if the third Ctrl-f
/// paints an empty query.
#[test]
fn pty_search_in_files_ctrl_f_closes_and_restores_query() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    wait_first_paint(&tui);

    tui.ctrl_letter('f');
    tui.wait_pred(
        search_open_empty,
        "Ctrl-f opens `Search · app` with the empty-query placeholder",
        WAIT,
    );
    tui.keys(QUERY);
    tui.wait_pred(
        search_open_with_query,
        "the search query row shows the typed query",
        WAIT,
    );

    tui.ctrl_letter('f');
    tui.wait_pred(
        |screen| search_closed(screen) && !screen.contains(QUERY),
        "Ctrl-f with a typed query closes the search dialog at once",
        WAIT,
    );
    tui.ctrl_letter('f');
    tui.wait_pred(
        search_open_with_query,
        "the next Ctrl-f reopens `Search · app` with the parked query",
        WAIT,
    );
}

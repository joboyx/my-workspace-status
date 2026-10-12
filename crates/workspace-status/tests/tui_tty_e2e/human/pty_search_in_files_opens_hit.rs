use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Clean, committed file under `app` with two hit lines for [`QUERY`].
const SEARCH_FILE: &str = "search-me.txt";
/// Right pane title of the file in the Explorer tab.
const SEARCH_TITLE: &str = "┐┌app/search-me.txt";
/// Strip label of the `app` checkout's Explorer tab.
const EXPLORER_APP: &str = "Explorer · app";
/// Search term. It is in no other seeded file. It has no `h`/`j`/`k`/`l`,
/// so it types in one burst (`discard_held_nav_backlog` drops a held
/// same-letter nav burst).
const QUERY: &str = "zorbatron";
/// Line 2 of [`SEARCH_FILE`]: the first hit.
const FIRST_HIT: &str = "alpha zorbatron";
/// Line 4 of [`SEARCH_FILE`]: the second hit.
const SECOND_HIT: &str = "omega zorbatron";
/// Tokyo Night (default theme) `cursor_bg` `#283457`: the file view paints
/// it on the cursor row only.
const CURSOR_BG: (u8, u8, u8) = (0x28, 0x34, 0x57);

/// Commit [`SEARCH_FILE`] in `app` so the file is clean (no tree row) and
/// the tree cursor still lands on `README.md`.
fn seed_search_file(workspace: &Path) {
    let app = workspace.join("app");
    fs::write(
        app.join(SEARCH_FILE),
        format!("first line\n{FIRST_HIT}\nmiddle line\n{SECOND_HIT}\n"),
    )
    .unwrap();
    git(&app, &["add", SEARCH_FILE]);
    git(&app, &["commit", "-q", "-m", "add search file"]);
}

/// The search dialog is up, scoped to the `app` checkout.
fn search_open(screen: &str) -> bool {
    screen.contains("Search · app") && screen.contains("Enter open")
}

fn first_paint(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && !screen.contains("Search · ")
}

/// Search hit row: the `❯` cursor chip in front of `needle`. The tree
/// cursor is `▌`, so a tree row cannot pass.
fn hit_row_on(screen: &str, needle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains('❯') && line.contains(needle))
}

/// [`SEARCH_FILE`] shows in the `Explorer · app` tab: the strip names the
/// Explorer tab (no top-level file tab), the right pane is titled with the
/// file, the tree selects it, and the search dialog is closed.
fn search_file_in_explorer(screen: &str) -> bool {
    let tab_strip = screen.lines().next().unwrap_or_default();
    let title_row = screen.lines().nth(1).unwrap_or_default();
    tab_strip.contains(EXPLORER_APP)
        && !tab_strip.contains(SEARCH_FILE)
        && title_row.contains(SEARCH_TITLE)
        && tree_cursor_on(screen, SEARCH_FILE)
        && screen.contains(FIRST_HIT)
        && screen.contains(SECOND_HIT)
        && !screen.contains("Search · ")
}

/// True when `needle` paints on the file view cursor row.
fn on_cursor_row(tui: &PtySession, needle: &str) -> bool {
    tui.needle_has_bg(needle, CURSOR_BG.0, CURSOR_BG.1, CURSOR_BG.2)
}

/// Ctrl-f opens `Search · app`, typing [`QUERY`] lists the line 2 hit,
/// Enter opens `app/search-me.txt` in the `Explorer · app` tab with the
/// cursor on line 2, and `n` moves the cursor to the line 4 hit.
///
/// Fail if Ctrl-f paints no search dialog, if no `❯ 2: alpha zorbatron`
/// hit row shows, if Enter leaves the dialog up or puts the cursor on
/// another line, or if `n` does not step to the next hit line.
#[test]
fn pty_search_in_files_enter_opens_hit_and_n_steps() {
    let (_root, workspace) = daily_workspace();
    seed_search_file(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        first_paint,
        "first paint: tree cursor on README.md; search dialog is closed",
        WAIT,
    );

    tui.ctrl_letter('f');
    tui.wait_pred(
        |screen| search_open(screen) && screen.contains("type to search"),
        "Ctrl-f opens the search dialog titled `Search · app` with an empty query",
        WAIT,
    );
    tui.keys(QUERY);
    tui.wait_pred(
        |screen| {
            search_open(screen)
                && screen.contains(&format!("{QUERY}▏"))
                && hit_row_on(screen, &format!("2: {FIRST_HIT}"))
                && screen.contains(&format!("4: {SECOND_HIT}"))
        },
        "the query lists both hit lines with ❯ on the line 2 hit",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| search_file_in_explorer(screen) && on_cursor_row(&tui, FIRST_HIT),
        "Enter closes the dialog and opens app/search-me.txt in Explorer · app with the cursor on line 2",
        GIT_WAIT,
    );
    assert!(
        !on_cursor_row(&tui, SECOND_HIT),
        "line 4 must not have the cursor before `n`; screen:\n{}",
        tui.screen()
    );

    tui.key('n');
    tui.wait_pred(
        |screen| {
            search_file_in_explorer(screen)
                && on_cursor_row(&tui, SECOND_HIT)
                && !on_cursor_row(&tui, FIRST_HIT)
        },
        "`n` moves the file view cursor to the next hit (line 4)",
        WAIT,
    );
}

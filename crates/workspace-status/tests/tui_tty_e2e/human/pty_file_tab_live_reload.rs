use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Same gap as `pty_quick_open_files_opens_file_tab`. A same-letter
/// `h`/`j`/`k`/`l` burst is dropped by `discard_held_nav_backlog` after the
/// first press.
const QUERY_NAV_LETTER_GAP_MS: u64 = 50;

/// Clean, committed file under `app` that the test opens, edits and deletes.
const LIVE_FILE: &str = "live-reload.txt";
/// File tab title on the pane's top border.
const LIVE_TITLE: &str = "┌app/live-reload.txt";
/// First line of the committed body.
const SEED_LINE: &str = "live reload seed";
/// Line appended on disk; only a watch reload can paint it.
const MARKER: &str = "live-reload-marker-1";
/// Watch status when the active file tab's file is gone.
const FILE_DELETED: &str = "file deleted";
/// File tab body while an open / `r` load is in flight.
const LOADING: &str = "loading…";
/// `FileRead::Failed` body for a missing file (io error text).
const READ_FAILED: &str = "No such file";

/// Commit [`LIVE_FILE`] in `app` so the file is clean (no tree row).
fn seed_live_file(workspace: &Path) {
    let app = workspace.join("app");
    fs::write(app.join(LIVE_FILE), format!("{SEED_LINE}\nsecond line\n")).unwrap();
    git(&app, &["add", LIVE_FILE]);
    git(&app, &["commit", "-q", "-m", "add live reload file"]);
}

fn files_open(screen: &str) -> bool {
    screen.contains("Go to file") && screen.contains("Enter open")
}

fn first_paint(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && !screen.contains("Go to file")
}

/// Type a Quick Open query at human key gaps for nav letters.
fn type_query(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(QUERY_NAV_LETTER_GAP_MS);
        }
    }
}

/// Quick Open hit row: the `❯` cursor chip in front of `needle`.
fn hit_row_on(screen: &str, needle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains('❯') && line.contains(needle))
}

/// The [`LIVE_FILE`] tab is active: tab strip names it, the pane title is
/// on the top border, and Quick Open is closed.
fn live_tab_active(screen: &str) -> bool {
    let tab_strip = screen.lines().next().unwrap_or_default();
    tab_strip.contains(LIVE_FILE)
        && screen
            .lines()
            .any(|line| line.trim_start().starts_with(LIVE_TITLE))
        && !screen.contains("Go to file")
}

/// Launch with a 500 ms watch poll and open [`LIVE_FILE`] through `F`.
fn open_live_tab(workspace: &Path) -> PtySession {
    let mut tui = PtySession::open_with_env(workspace, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        first_paint,
        "first paint: tree cursor on README.md; Quick Open is closed",
        WAIT,
    );
    tui.shift_letter('F');
    tui.wait_pred(files_open, "F opens Quick Open on files", WAIT);
    type_query(&mut tui, "live-reload");
    tui.wait_pred(
        |screen| files_open(screen) && hit_row_on(screen, LIVE_FILE),
        "`live-reload` lists live-reload.txt under the ❯ cursor",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| live_tab_active(screen) && screen.contains(SEED_LINE) && !screen.contains(MARKER),
        "Enter opens the live-reload.txt tab with its committed body",
        GIT_WAIT,
    );
    tui
}

/// Append [`MARKER`] to [`LIVE_FILE`] (the size changes, so the disk token
/// moves whatever the mtime granularity).
fn append_marker(workspace: &Path) {
    let path = workspace.join("app").join(LIVE_FILE);
    let mut body = fs::read_to_string(&path).unwrap();
    body.push_str(MARKER);
    body.push('\n');
    fs::write(&path, body).unwrap();
}

/// Watch poll reloads the active file tab of a clean, committed file.
///
/// Opens `app/live-reload.txt` through Quick Open with
/// `WS_STATUS_WATCH_MS=500`, appends a marker line on disk, and waits for
/// the marker in the tab without `r`. Fail if the marker never paints, if
/// the tab loses its `app/live-reload.txt` title, or if the body shows the
/// loading or read-error text.
#[test]
fn pty_file_tab_live_reload_paints_disk_edit() {
    let (_root, workspace) = daily_workspace();
    seed_live_file(&workspace);
    let tui = open_live_tab(&workspace);

    append_marker(&workspace);
    tui.wait_pred(
        |screen| live_tab_active(screen) && screen.contains(MARKER) && screen.contains(SEED_LINE),
        "watch poll paints the appended marker in the file tab without `r`",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert!(
        live_tab_active(&screen),
        "file tab must stay active and titled app/live-reload.txt; screen:\n{screen}"
    );
    crate::harness::assert_absent(&screen, LOADING);
    crate::harness::assert_absent(&screen, READ_FAILED);
    crate::harness::assert_absent(&screen, FILE_DELETED);
}

/// Watch poll keeps a deleted file's tab open on its last content.
///
/// After a live reload paints the marker, the file is deleted on disk. The
/// status line must show `file deleted` and the tab must keep its title and
/// last body (seed line and marker). Fail if the warning never paints, if
/// the tab closes or loses its body, or if the body turns into the
/// loading or read-error text.
#[test]
fn pty_file_tab_live_reload_keeps_deleted_file() {
    let (_root, workspace) = daily_workspace();
    seed_live_file(&workspace);
    let tui = open_live_tab(&workspace);

    append_marker(&workspace);
    tui.wait_pred(
        |screen| live_tab_active(screen) && screen.contains(MARKER),
        "watch poll paints the appended marker before the delete",
        GIT_WAIT,
    );

    fs::remove_file(workspace.join("app").join(LIVE_FILE)).unwrap();
    tui.wait_pred(
        |screen| {
            screen.contains(FILE_DELETED)
                && live_tab_active(screen)
                && screen.contains(SEED_LINE)
                && screen.contains(MARKER)
        },
        "watch poll warns `file deleted` and keeps the tab on its last content",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert!(
        live_tab_active(&screen),
        "file tab must stay open after the delete; screen:\n{screen}"
    );
    crate::harness::assert_contains(&screen, SEED_LINE);
    crate::harness::assert_contains(&screen, MARKER);
    crate::harness::assert_absent(&screen, LOADING);
    crate::harness::assert_absent(&screen, READ_FAILED);
}

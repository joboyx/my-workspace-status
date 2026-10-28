use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git};
use crate::support::{
    right_pane, tree_cursor_on, tree_has, tree_inactive_selection_on, GIT_WAIT, WAIT,
};

/// Same gap as `pty_quick_open_files_opens_in_explorer`. A same-letter
/// `h`/`j`/`k`/`l` burst is dropped by `discard_held_nav_backlog` after the
/// first press.
const QUERY_NAV_LETTER_GAP_MS: u64 = 50;

/// Clean, committed file under `app` that the test opens, edits and deletes.
const LIVE_FILE: &str = "live-reload.txt";
/// Path the file view title and the diff header both name.
const LIVE_PATH: &str = "app/live-reload.txt";
/// Strip label of the `app` checkout's Explorer tab.
const EXPLORER_APP: &str = "Explorer · app";
/// First line of the committed body.
const SEED_LINE: &str = "live reload seed";
/// Line appended on disk; only a watch reload can paint it.
const MARKER: &str = "live-reload-marker-1";
/// Watch status when the active inner file tab's file is gone.
const FILE_DELETED: &str = "file deleted";
/// File view body while an open / `r` load is in flight.
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

/// [`LIVE_FILE`] shows in the `Explorer · app` tab: the strip names the
/// Explorer tab (no top-level file tab), the tree selects the file (the
/// unfocused `▏` bar: Quick Open focuses the file pane), the
/// right pane names it (the file view title, or the diff header once the
/// edit makes it modified), and Quick Open is closed.
fn live_tab_active(screen: &str) -> bool {
    let tab_strip = screen.lines().next().unwrap_or_default();
    let title_row = screen.lines().nth(1).unwrap_or_default();
    tab_strip.contains(EXPLORER_APP)
        && !tab_strip.contains(LIVE_FILE)
        && tree_inactive_selection_on(screen, LIVE_FILE)
        && (title_row.contains(LIVE_PATH) || right_pane(screen).contains(LIVE_PATH))
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
        "Enter opens live-reload.txt in Explorer · app with its committed body",
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

/// Watch poll reloads the active Explorer inner file tab of a clean,
/// committed file.
///
/// Opens `app/live-reload.txt` through Quick Open with
/// `WS_STATUS_WATCH_MS=500`, appends a marker line on disk, and waits for
/// the marker without `r`. The edit makes the file modified, so the inner
/// tab turns into its whole-file diff (the seed line stays as context).
/// Fail if the marker never paints, if the Explorer tab stops showing
/// `app/live-reload.txt`, or if the body shows the loading or read-error
/// text.
#[test]
fn pty_explorer_file_live_reload_paints_disk_edit() {
    let (_root, workspace) = daily_workspace();
    seed_live_file(&workspace);
    let tui = open_live_tab(&workspace);

    append_marker(&workspace);
    tui.wait_pred(
        |screen| live_tab_active(screen) && screen.contains(MARKER) && screen.contains(SEED_LINE),
        "watch poll paints the appended marker in the Explorer without `r`",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert!(
        live_tab_active(&screen),
        "the Explorer tab must keep showing app/live-reload.txt; screen:\n{screen}"
    );
    assert!(
        right_pane(&screen).contains(&format!("+{MARKER}")),
        "the modified file shows as a diff with the marker added; screen:\n{screen}"
    );
    crate::harness::assert_absent(&screen, LOADING);
    crate::harness::assert_absent(&screen, READ_FAILED);
    crate::harness::assert_absent(&screen, FILE_DELETED);
}

/// Watch poll keeps a deleted file's inner tab open on its last content.
///
/// After a live reload paints the marker, the file is deleted on disk. The
/// status line must show `file deleted` and the inner tab must keep its
/// last content (seed line and marker). Fail if the warning never paints,
/// if the inner tab closes or loses its content, or if it turns into the
/// loading or read-error text.
#[test]
fn pty_explorer_file_live_reload_keeps_deleted_file() {
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
        "watch poll warns `file deleted` and keeps the inner tab on its last content",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert!(
        live_tab_active(&screen),
        "the inner tab must stay open after the delete; screen:\n{screen}"
    );
    crate::harness::assert_contains(&screen, SEED_LINE);
    crate::harness::assert_contains(&screen, MARKER);
    crate::harness::assert_absent(&screen, LOADING);
    crate::harness::assert_absent(&screen, READ_FAILED);
}

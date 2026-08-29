use std::fs;

use crate::harness::{self, assert_contains, left_tree, PtySession};
use crate::seed::daily_workspace;
use crate::support::{tree_has, tree_line_containing, GIT_WAIT, WAIT};

/// Idle first paint with live watch on: tree + file chrome, no `r` toast.
///
/// Help `r` is `refresh now`. Watch is the poll (`WS_STATUS_WATCH_MS`).
fn idle_tui_ready_for_watch(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && screen.contains(" tree")
        && screen.contains("? help")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && !screen.contains("MOVE")
        && !screen.contains("SEARCH")
        && !refresh_now_toast(screen)
}

/// `r` reload toast. Watch apply must not paint this.
fn refresh_now_toast(screen: &str) -> bool {
    screen.contains("refreshed app") || screen.contains("refreshed workspace")
}

/// New dirty path on the tree with the untracked `A` badge, not chrome-only.
fn tree_shows_watch_dirty_path(screen: &str, name: &str) -> bool {
    tree_line_containing(screen, name).is_some_and(|line| line.contains("A "))
}

/// Live watch paints a new dirty path while nav keys arrive (no `r`).
///
/// Docs: watch apply while keys arrive (no `r`). Help/keymap: `r` is
/// `refresh now` (`pty_r_refreshes_new_dirty_file`, watch off). `watch.rs`:
/// dirty paths sit in `checkout_watch_identity`, so an edit is a real poll
/// move. A no-op, a toast/status tick without the path, a frozen tree until
/// `r`, or a path that only appears after `r` cannot pass.
#[test]
fn pty_watch_applies_while_keys_arrive() {
    let (_root, workspace) = daily_workspace();
    let marker = format!(
        "watch-live-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_pred(
        idle_tui_ready_for_watch,
        "first paint: tree + file chrome, watch on, no r toast",
        WAIT,
    );
    tui.wait_pred(
        |screen| !tree_has(screen, &marker),
        "new dirty path is absent before the disk write",
        WAIT,
    );

    fs::write(workspace.join("app").join(&marker), "live-watch\n").unwrap();

    let mut down = true;
    tui.wait_pred_while(
        |screen| tree_shows_watch_dirty_path(screen, &marker) && !refresh_now_toast(screen),
        "watch paints the new dirty path on the tree while j/k arrive (no r)",
        GIT_WAIT,
        |session| {
            session.key(if down { 'j' } else { 'k' });
            down = !down;
        },
    );

    let screen = tui.screen();
    let left = left_tree(&screen);
    assert!(
        tree_shows_watch_dirty_path(&screen, &marker),
        "new dirty path must paint on the tree (toast/status-only fails); screen:\n{screen}"
    );
    assert!(
        tree_has(&screen, "README.md") && left.contains("2 changed"),
        "watch dirty-path identity must keep README and bump the workspace change count; screen:\n{screen}"
    );
    harness::assert_absent(&screen, "refreshed app");
    harness::assert_absent(&screen, "refreshed workspace");
    harness::assert_absent(&screen, "MOVE");
    assert_contains(&screen, "? help");
}

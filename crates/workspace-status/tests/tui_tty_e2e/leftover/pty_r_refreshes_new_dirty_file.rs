use std::fs;

use crate::harness::{left_tree, PtySession};
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, launch_breadcrumb_workspace_only,
    launch_panes_left_tree_right_diff, launch_status_chrome, no_wrong_overlays, tree_cursor_on,
    tree_has, tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};

const MARKER: &str = "r-live.txt";

/// New untracked path on the tree with the `A` badge, not chrome-only.
fn tree_shows_new_dirty_path(screen: &str) -> bool {
    tree_line_containing(screen, MARKER)
        .is_some_and(|line| line.contains("A ") && !line.contains('\u{258C}'))
}

/// Watch is off. Write has not reached the tree. `r` has not run.
fn idle_watch_off_before_r(screen: &str) -> bool {
    documented_launch_first_paint(screen) && !tree_has(screen, MARKER)
}

/// Focused-repo `r` reloaded `app`. New dirty file is on the tree.
///
/// Workspace change count is 2. README stays focused. File-diff stays.
/// A no-op, watch-only apply, toast-only tick, graph, or search cannot pass.
fn documented_r_refreshed_app_dirty_file(screen: &str) -> bool {
    let left = left_tree(screen);
    let readme = tree_line_containing(screen, "README.md");
    let app = tree_line_containing(screen, "@ app");
    launch_panes_left_tree_right_diff(screen)
        && left.contains("# workspace")
        && left.contains("2 changed · all current")
        && !left.contains("1 changed")
        && tree_has(screen, "app")
        && tree_has(screen, "& main")
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && tree_has(screen, "No updates")
        && tree_shows_new_dirty_path(screen)
        && readme.is_some_and(|line| line.contains('M') && line.contains('\u{258C}'))
        && app.is_some_and(|line| line.contains('2'))
        && !tree_has(screen, "lib")
        && !screen.contains("notes")
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, MARKER)
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && screen.contains("app/README.md  inline (too narrow)")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("@@ -1 +1,2 @@")
        && launch_breadcrumb_workspace_only(screen)
        && launch_status_chrome(screen)
        && !screen.contains("[workspace]")
        && !screen.contains("workspace ›")
        && !screen.contains("WIP on graph")
        && !screen.contains("Working tree")
        && !screen.contains("focus a repo for the graph")
        && !screen.contains("No matching rows")
        && !screen.contains("loading")
        && !screen.contains("┌ files")
        && no_wrong_overlays(screen)
}

/// `r` reloads the focused checkout while watch is off.
///
/// Help GIT: `r` is `refresh now`. Configuration: refresh the focused
/// repo; the whole workspace on the workspace row or the No-updates
/// group. `PtySession` sets `WS_STATUS_WATCH_MS=0`. Live watch without
/// `r` is `pty_watch_applies_while_keys_arrive`.
///
/// Live PTY after first paint (cursor already on dirty README): write
/// `r-live.txt` in `app`. The tree stays at `1 changed` until `r`. Then
/// `r` paints that path with `A `, bumps the workspace and `app` counts
/// to 2, keeps README focused, and keeps the file-diff. A no-op, a
/// watch-only apply, a toast/status tick without the tree row, a graph
/// or files pane, or `/` search cannot pass.
#[test]
fn pty_r_refreshes_new_dirty_file() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        idle_watch_off_before_r,
        "first paint: dirty README file-diff, watch off, r-live.txt absent",
        WAIT,
    );

    fs::write(workspace.join("app").join(MARKER), "refresh me\n").unwrap();
    tui.wait_pred(
        idle_watch_off_before_r,
        "watch is off: new dirty path stays off the tree until r",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        idle_watch_off_before_r,
        "still first paint after the write (not a delayed watch apply)",
        WAIT,
    );

    tui.key('r');
    tui.wait_pred(
        documented_r_refreshed_app_dirty_file,
        "r reloads app: r-live.txt on the tree with A, 2 changed, README file-diff",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_r_refreshed_app_dirty_file,
        "r refresh holds (not a flicker, toast-only, watch-only, or wrong pane)",
        WAIT,
    );
}

use crate::harness::PtySession;
use crate::seed::unfetched_behind_workspace;
use crate::support::{
    crumb_row, has_fetch_hint, has_pull_hint, status_row, syncbox_row_behind, tree_cursor_on,
    tree_has, tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};

fn no_wrong_fetch_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("no visible repos for that op")
        && !screen.contains("nothing behind to pull")
}

fn crumb_fetched_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Fetched 1 repo")
        && !crumb.contains("failed")
        && !crumb.contains("Pulled")
        && !crumb.contains("Pushed")
}

/// First paint: workspace focused, syncbox hidden under folded No updates.
/// Looks in-sync. Fetch hint. No pull. No origin tip.
fn idle_unfetched_workspace(screen: &str) -> bool {
    let status = status_row(screen);
    let no_updates = tree_line_containing(screen, "No updates");
    let Some(top) = screen.lines().next() else {
        return false;
    };
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "syncbox")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "all current")
        && !tree_has(screen, "behind")
        && !tree_has(screen, "syncbox")
        && no_updates.is_some_and(|line| line.contains('>') && line.contains('1'))
        && top.contains(" tree ")
        && top.contains(" graph")
        && screen.contains("focus a repo for the graph")
        && !screen.contains("origin-tip-commit")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && !screen.contains("Pushed")
        && has_fetch_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace"
        && no_wrong_fetch_overlays(screen)
}

/// Workspace `f` fetched remotes. Tree shows behind. Pull hint. Not a pull.
fn documented_workspace_fetch_behind(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "syncbox")
        && tree_has(screen, "1 behind")
        && !tree_has(screen, "all current")
        && !tree_has(screen, "No updates")
        && syncbox_row_behind(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("origin-tip-commit")
        && crumb_fetched_one(screen)
        && has_fetch_hint(screen)
        && has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Pulled")
        && !screen.contains("Pushed")
        && no_wrong_fetch_overlays(screen)
}

/// `j` onto syncbox after fetch: graph shows the origin tip. HEAD stays seed.
fn documented_fetch_graph_behind(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "1 behind")
        && syncbox_row_behind(screen)
        && screen.contains("origin-tip-commit")
        && screen.contains("seed syncbox")
        && screen.contains("[origin/main]")
        && screen.contains("Working tree clean")
        && screen.contains("main v1")
        && !screen.contains("focus a repo for the graph")
        && crumb_row(screen).contains("workspace › syncbox")
        && has_fetch_hint(screen)
        && has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Pulled")
        && !screen.contains("Pushed")
        && no_wrong_fetch_overlays(screen)
}

/// `f` fetches remotes against a local bare origin. Must mark behind.
///
/// Docs: Help GIT `f` = fetch remotes. Configuration: `git fetch --quiet`
/// for the focused checkout, or primary checkouts on the workspace row.
/// Live PTY after first paint (workspace cursor, folded No updates) did
/// that fetch: tree `v1` / `1 behind`, pull hint, `Fetched 1 repo`. `j`
/// onto syncbox paints `origin-tip-commit` on the graph. HEAD stays
/// `seed syncbox`. Not `p` pull. Not Shift+P push. Not a toast-only tick.
///
/// After first paint the cursor is already on the workspace row. Do not
/// `/` search. A no-op, pull, push, toast-only, missing behind mark, or
/// the wrong repo is red.
#[test]
fn pty_fetch_local_remote_marks_behind() {
    let (_root, workspace) = unfetched_behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("No updates", WAIT);
    tui.wait_pred(
        idle_unfetched_workspace,
        "first paint: workspace cursor, folded No updates, in-sync, fetch hint, no pull",
        WAIT,
    );

    tui.key('f');
    tui.wait_pred(
        documented_workspace_fetch_behind,
        "f fetches remotes: Fetched 1 repo, syncbox v1, 1 behind, pull hint, not pulled",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_workspace_fetch_behind,
        "fetched behind paint holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        documented_fetch_graph_behind,
        "j onto syncbox: graph origin-tip-commit, HEAD stays seed, still v1",
        GIT_WAIT,
    );
}

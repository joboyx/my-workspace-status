use std::time::Duration;

use crate::harness::PtySession;
use crate::seed::unfetched_behind_workspace;
use crate::slow_git::{open_with_slow_git, wait_slow_git_started, SlowFetchPullGit};
use crate::support::{
    crumb_row, has_fetch_hint, panes_tree_focused_graph_unfocused, repo_row_in_sync, status_row,
    syncbox_row_behind, tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has,
    tree_line_containing, GIT_WAIT, WAIT,
};

fn no_lock_or_search(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("index.lock")
        && !screen.contains("no visible repos for that op")
}

fn idle_unfetched_workspace(screen: &str) -> bool {
    let no_updates = tree_line_containing(screen, "No updates");
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "syncbox")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "all current")
        && !tree_has(screen, "behind")
        && !tree_has(screen, "syncbox")
        && no_updates.is_some_and(|line| line.contains('>') && line.contains('1'))
        && tree_dir_collapsed(screen, "No updates")
        && panes_tree_focused_graph_unfocused(screen)
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && has_fetch_hint(screen)
        && status_row(screen).contains(" tree")
        && crumb_row(screen).trim() == "workspace"
        && no_lock_or_search(screen)
}

fn no_updates_open_on_group(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "syncbox")
        && tree_dir_expanded(screen, "No updates")
        && tree_has(screen, "syncbox")
        && repo_row_in_sync(screen, "syncbox")
}

fn cursor_on_syncbox_in_sync(screen: &str) -> bool {
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && repo_row_in_sync(screen, "syncbox")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && no_lock_or_search(screen)
}

fn pulled_without_lock(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    screen.contains("Pulled")
        && (crumb.contains("Pulled 1 repo") || crumb.contains("Pulled"))
        && !crumb.contains("failed")
        && !screen.contains("index.lock")
        && no_lock_or_search(screen)
}

fn focus_syncbox(tui: &mut PtySession) {
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "No updates"),
        "j from workspace focuses folded No updates",
        WAIT,
    );
    tui.key('l');
    tui.wait_pred(
        no_updates_open_on_group,
        "l opens No updates (syncbox visible, cursor stays)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        cursor_on_syncbox_in_sync,
        "j onto syncbox: in-sync, focused checkout, not workspace",
        WAIT,
    );
}

/// `f` then `p` on the same checkout before fetch ends still pulls.
///
/// Same gitdir stays one mutating child: pull waits for occupy release.
/// Live PTY: unfold/focus syncbox, `f`, wait until slow git occupies, `p`
/// before fetch finishes. `Pulled` paints. Fail if `index.lock` appears
/// or if `Pulled` never appears.
#[test]
fn pty_overlapping_fetch_then_pull_on_same_repo_pulls() {
    let (_root, workspace) = unfetched_behind_workspace();
    let slow = SlowFetchPullGit::install(&workspace);
    let mut tui = open_with_slow_git(&workspace, &slow);
    tui.wait_contains("No updates", WAIT);
    tui.wait_pred(
        idle_unfetched_workspace,
        "first paint: workspace cursor, folded No updates, syncbox hidden",
        WAIT,
    );

    focus_syncbox(&mut tui);

    tui.key('f');
    wait_slow_git_started(&slow, || tui.screen(), WAIT);
    tui.wait_pred(
        |screen| crumb_row(screen).contains("Fetching") || screen.contains("Fetching"),
        "f paints Fetching (occupy) before finish",
        Duration::from_secs(2),
    );
    assert!(
        !tui.screen().contains("Pulled"),
        "p has not run yet; screen:\n{}",
        tui.screen()
    );
    assert!(
        !tui.screen().contains("index.lock"),
        "fetch must not paint index.lock; screen:\n{}",
        tui.screen()
    );

    tui.key('p');

    tui.wait_pred(
        pulled_without_lock,
        "p after occupy pulls: Pulled paints, no index.lock",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert!(
        screen.contains("Pulled"),
        "Pulled must appear after fetch-then-pull on the same repo; screen:\n{screen}"
    );
    assert!(
        !screen.contains("index.lock"),
        "serialized fetch then pull must not paint index.lock; screen:\n{screen}"
    );
    assert!(
        !syncbox_row_behind(&screen) || screen.contains("Pulled"),
        "behind mark without Pulled means pull never ran; screen:\n{screen}"
    );
}

use std::time::Duration;

use crate::harness::PtySession;
use crate::seed::two_unfetched_behind_workspace;
use crate::slow_git::{open_with_slow_git, wait_slow_git_started, SlowFetchPullGit};
use crate::support::{
    crumb_row, has_fetch_hint, panes_tree_focused_graph_unfocused, repo_row_behind,
    repo_row_in_sync, status_row, tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has,
    tree_line_containing, GIT_WAIT, WAIT,
};

fn no_wrong_fetch_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("no visible repos for that op")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("index.lock")
}

fn idle_two_unfetched_workspace(screen: &str) -> bool {
    let no_updates = tree_line_containing(screen, "No updates");
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "alpha")
        && !tree_cursor_on(screen, "beta")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "all current")
        && !tree_has(screen, "behind")
        && !tree_has(screen, "alpha")
        && !tree_has(screen, "beta")
        && no_updates.is_some_and(|line| line.contains('>') && line.contains('2'))
        && tree_dir_collapsed(screen, "No updates")
        && panes_tree_focused_graph_unfocused(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && has_fetch_hint(screen)
        && status_row(screen).contains(" tree")
        && crumb_row(screen).trim() == "workspace"
        && no_wrong_fetch_overlays(screen)
}

fn no_updates_open_on_group(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, "No updates") else {
        return false;
    };
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "alpha")
        && !tree_cursor_on(screen, "beta")
        && tree_dir_expanded(screen, "No updates")
        && !tree_dir_collapsed(screen, "No updates")
        && line.contains('v')
        && line.contains('2')
        && tree_has(screen, "alpha")
        && tree_has(screen, "beta")
        && repo_row_in_sync(screen, "alpha")
        && repo_row_in_sync(screen, "beta")
}

fn cursor_on_alpha_in_sync(screen: &str) -> bool {
    tree_cursor_on(screen, "alpha")
        && !tree_cursor_on(screen, "beta")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "beta")
        && repo_row_in_sync(screen, "alpha")
        && repo_row_in_sync(screen, "beta")
        && !screen.contains("Fetched")
}

fn cursor_on_beta_while_fetching(screen: &str) -> bool {
    tree_cursor_on(screen, "beta")
        && !tree_cursor_on(screen, "alpha")
        && tree_has(screen, "alpha")
        && !screen.contains("Fetched")
        && (crumb_row(screen).contains("Fetching") || screen.contains("Fetching"))
}

fn both_repos_behind_after_overlap(screen: &str) -> bool {
    repo_row_behind(screen, "alpha")
        && repo_row_behind(screen, "beta")
        && tree_has(screen, "2 behind")
        && !tree_has(screen, "all current")
        && !screen.contains("index.lock")
        && !crumb_row(screen).contains("failed")
        && (crumb_row(screen).contains("Fetched 2 repos")
            || (screen.contains("Fetched") && repo_row_behind(screen, "beta")))
}

fn unfold_no_updates_onto_alpha(tui: &mut PtySession) {
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "No updates"),
        "j from workspace focuses folded No updates",
        WAIT,
    );
    tui.key('l');
    tui.wait_pred(
        no_updates_open_on_group,
        "l opens No updates (v, alpha and beta visible, cursor stays)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        cursor_on_alpha_in_sync,
        "j onto alpha: in-sync, beta still in-sync, not a workspace fetch",
        WAIT,
    );
}

/// Overlapping `f` on two focused checkouts both mark behind.
///
/// Docs: Help GIT `f` = fetch remotes for the focused checkout (not the
/// workspace row). Independent gitdirs overlap. Live PTY: unfold No
/// updates, `f` on `alpha`, wait until slow git occupies, `j` onto `beta`,
/// `f` before `alpha` finishes. Both rows show `v1` / workspace `2 behind`.
/// Fail if only `Fetched 1 repo` and `beta` stays in-sync, or if `alpha`
/// is lost. Workspace-row `f` (all primaries) is not this path.
#[test]
fn pty_overlapping_fetches_on_two_repos_both_mark_behind() {
    let (_root, workspace) = two_unfetched_behind_workspace();
    let slow = SlowFetchPullGit::install(&workspace);
    let mut tui = open_with_slow_git(&workspace, &slow);
    tui.wait_contains("No updates", WAIT);
    tui.wait_pred(
        idle_two_unfetched_workspace,
        "first paint: workspace cursor, folded No updates 2, alpha/beta hidden",
        WAIT,
    );

    unfold_no_updates_onto_alpha(&mut tui);

    tui.key('f');
    wait_slow_git_started(&slow, || tui.screen(), WAIT);
    tui.wait_pred(
        |screen| crumb_row(screen).contains("Fetching") || screen.contains("Fetching"),
        "alpha f paints Fetching (occupy) before finish",
        Duration::from_secs(2),
    );
    assert!(
        !tui.screen().contains("Fetched"),
        "second f must land before alpha fetch finishes; screen:\n{}",
        tui.screen()
    );

    tui.key('j');
    tui.wait_pred(
        cursor_on_beta_while_fetching,
        "j onto beta while alpha fetch is still running",
        Duration::from_secs(2),
    );
    tui.key('f');

    tui.wait_pred(
        both_repos_behind_after_overlap,
        "both alpha and beta mark v1 / 2 behind after overlapping fetches",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert!(
        repo_row_behind(&screen, "alpha"),
        "alpha fetch must not be lost; screen:\n{screen}"
    );
    assert!(
        repo_row_behind(&screen, "beta"),
        "beta fetch must run (Fetched 1 repo only + beta in-sync is red); screen:\n{screen}"
    );
    assert!(
        !(crumb_row(&screen).contains("Fetched 1 repo") && repo_row_in_sync(&screen, "beta")),
        "second fetch never ran: Fetched 1 repo and beta still in-sync; screen:\n{screen}"
    );
    assert!(
        !screen.contains("index.lock"),
        "overlapping fetches must not paint index.lock; screen:\n{screen}"
    );
}

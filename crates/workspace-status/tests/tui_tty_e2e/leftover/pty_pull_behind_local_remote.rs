use crate::harness::PtySession;
use crate::seed::behind_workspace;
use crate::support::{
    crumb_row, graph_subject_line, graph_subject_meta_line, has_fetch_hint, has_pull_hint,
    status_row, syncbox_row_behind, syncbox_row_current, tree_cursor_on, tree_has, GIT_WAIT,
    SETTLE_MS, WAIT,
};

fn no_wrong_pull_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("no visible repos for that op")
        && !screen.contains("Stash ")
        && !screen.contains("pop stash")
}

fn crumb_pulled_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Pulled 1 repo")
        && !crumb.contains("failed")
        && !crumb.contains("Fetched")
        && !crumb.contains("Pushed")
}

/// Origin tip is a remote-only commit. HEAD is still the seed.
fn origin_tip_is_remote_not_head(screen: &str) -> bool {
    graph_subject_line(screen, "origin-tip-commit")
        .is_some_and(|line| line.contains("*  origin-tip-commit"))
        && graph_subject_meta_line(screen, "origin-tip-commit")
            .is_some_and(|line| line.contains("[origin/main]") && !line.contains("[+=main]"))
}

/// HEAD moved to the origin tip. Local and origin/main fold into `[+=main]`.
fn origin_tip_is_head(screen: &str) -> bool {
    graph_subject_line(screen, "origin-tip-commit")
        .is_some_and(|line| line.contains("@  origin-tip-commit"))
        && graph_subject_meta_line(screen, "origin-tip-commit")
            .is_some_and(|line| line.contains("[+=main]") && !line.contains("[origin/main]"))
}

fn seed_is_head(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("@  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox")
            .is_some_and(|line| line.contains("[+main]") && !line.contains("[+=main]"))
}

fn seed_is_ancestor_not_head(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("*  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox")
            .is_some_and(|line| !line.contains("[+main]") && !line.contains("[+=main]"))
}

/// First paint: cursor on behind syncbox. Graph HEAD is still the seed.
fn idle_behind_syncbox(screen: &str) -> bool {
    let status = status_row(screen);
    let Some(top) = screen.lines().next() else {
        return false;
    };
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "1 behind")
        && !tree_has(screen, "all current")
        && !tree_has(screen, "No updates")
        && syncbox_row_behind(screen)
        && top.contains(" tree ")
        && top.contains(" graph")
        && screen.contains("main v1")
        && screen.contains("Working tree clean")
        && origin_tip_is_remote_not_head(screen)
        && seed_is_head(screen)
        && !screen.contains("Pulled")
        && !screen.contains("Fetched")
        && !screen.contains("Pushed")
        && has_fetch_hint(screen)
        && has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace › syncbox"
        && no_wrong_pull_overlays(screen)
}

/// `p` pulled the focused behind checkout. HEAD is the origin tip.
fn documented_p_pulled(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "all current")
        && tree_has(screen, "No updates")
        && !tree_has(screen, "1 behind")
        && !tree_has(screen, "v1")
        && syncbox_row_current(screen)
        && !screen.contains("main v1")
        && screen.contains("Working tree clean")
        && origin_tip_is_head(screen)
        && seed_is_ancestor_not_head(screen)
        && crumb_pulled_one(screen)
        && crumb_row(screen).contains("workspace › syncbox")
        && !crumb_row(screen).contains("[syncbox]")
        && has_fetch_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Fetched")
        && !screen.contains("Pushed")
        && no_wrong_pull_overlays(screen)
}

/// `p` pulls a behind checkout against a local bare origin.
///
/// Docs: Help GIT `p` = pull behind. Configuration: focused checkout →
/// that path. Live PTY after first paint (cursor already on `syncbox`)
/// did that pull: tree drops `v1` / `1 behind` (`all current`, No
/// updates), graph HEAD is `origin-tip-commit` (`@`, `[+=main]`), seed
/// stays an ancestor, pull hint goes, `Pulled 1 repo`. Not `f` fetch.
/// Not Shift+P push. Not a toast-only tick. Not graph stash pop.
///
/// After first paint the cursor is already on the behind repo. Do not
/// `/` search. A no-op, fetch-only, push, toast-only, still-behind, or
/// the wrong repo is red.
#[test]
fn pty_pull_behind_local_remote() {
    let (_root, workspace) = behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("syncbox", WAIT);
    tui.wait_contains("origin-tip-commit", GIT_WAIT);
    tui.wait_pred(
        idle_behind_syncbox,
        "first paint: cursor on behind syncbox, main v1, HEAD is seed, pull hint",
        WAIT,
    );

    tui.key('p');
    tui.wait_pred(
        documented_p_pulled,
        "p pulls: Pulled 1 repo, HEAD origin-tip-commit [+=main], no v1, pull hint gone",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_p_pulled,
        "pulled paint holds (not a flicker or toast-only tick)",
        WAIT,
    );
}

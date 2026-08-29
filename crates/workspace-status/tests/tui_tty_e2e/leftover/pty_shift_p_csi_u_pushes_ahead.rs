use crate::harness::PtySession;
use crate::seed::ahead_workspace;
use crate::support::{
    after_syncbox_name, crumb_row, graph_subject_line, graph_subject_meta_line, has_fetch_hint,
    has_pull_hint, status_row, syncbox_row_current, tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS,
    WAIT,
};

/// Trailing ASCII ahead-by-1 (`^1`) on the syncbox tree row.
fn syncbox_row_ahead(screen: &str) -> bool {
    after_syncbox_name(screen).is_some_and(|after| after.contains("^1") && after.contains("& main"))
}

fn has_push_hint(screen: &str) -> bool {
    status_row(screen).contains("push")
}

fn no_wrong_push_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("nothing to push")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("no visible repos for that op")
        && !screen.contains("Stash ")
        && !screen.contains("pop stash")
}

fn crumb_pushed_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Pushed 1 repo")
        && !crumb.contains("failed")
        && !crumb.contains("Fetched")
        && !crumb.contains("Pulled")
}

/// HEAD is the local ahead tip. Origin still sits on the seed.
fn ahead_tip_is_head_not_origin(screen: &str) -> bool {
    graph_subject_line(screen, "ahead-tip-commit")
        .is_some_and(|line| line.contains("@  ahead-tip-commit"))
        && graph_subject_meta_line(screen, "ahead-tip-commit").is_some_and(|line| {
            line.contains("[+main]")
                && !line.contains("[+=main]")
                && !line.contains("[origin/main]")
        })
}

/// HEAD and origin/main fold into `[+=main]` on the pushed tip.
fn ahead_tip_is_head_and_origin(screen: &str) -> bool {
    graph_subject_line(screen, "ahead-tip-commit")
        .is_some_and(|line| line.contains("@  ahead-tip-commit"))
        && graph_subject_meta_line(screen, "ahead-tip-commit")
            .is_some_and(|line| line.contains("[+=main]") && !line.contains("[origin/main]"))
}

fn seed_is_origin_not_head(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("*  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox")
            .is_some_and(|line| line.contains("[origin/main]") && !line.contains("[+main]"))
}

fn seed_is_ancestor_no_origin_chip(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("*  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox").is_some_and(|line| {
            !line.contains("[origin/main]")
                && !line.contains("[+main]")
                && !line.contains("[+=main]")
        })
}

/// First paint: cursor on ahead syncbox. Graph HEAD is the local tip.
fn idle_ahead_syncbox(screen: &str) -> bool {
    let status = status_row(screen);
    let Some(top) = screen.lines().next() else {
        return false;
    };
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "1 ahead")
        && !tree_has(screen, "all current")
        && !tree_has(screen, "No updates")
        && syncbox_row_ahead(screen)
        && top.contains(" tree ")
        && top.contains(" graph")
        && screen.contains("main ^1")
        && screen.contains("Working tree clean")
        && ahead_tip_is_head_not_origin(screen)
        && seed_is_origin_not_head(screen)
        && !screen.contains("Pushed")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && has_push_hint(screen)
        && has_fetch_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace › syncbox"
        && no_wrong_push_overlays(screen)
}

/// CSI-u Shift+P pushed the focused ahead checkout. Origin matches HEAD.
fn documented_shift_p_pushed(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "all current")
        && tree_has(screen, "No updates")
        && !tree_has(screen, "1 ahead")
        && !tree_has(screen, "^1")
        && syncbox_row_current(screen)
        && !screen.contains("main ^1")
        && screen.contains("Working tree clean")
        && ahead_tip_is_head_and_origin(screen)
        && seed_is_ancestor_no_origin_chip(screen)
        && crumb_pushed_one(screen)
        && crumb_row(screen).contains("workspace › syncbox")
        && !crumb_row(screen).contains("[syncbox]")
        && has_fetch_hint(screen)
        && !has_push_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && no_wrong_push_overlays(screen)
}

/// CSI-u Shift+P pushes an ahead checkout against a local bare origin.
///
/// Docs: Help GIT `P` = push ahead/diverged/new. Configuration: repo /
/// checkout only (not workspace). Live PTY after first paint (cursor
/// already on `syncbox`) did that push: tree drops `^1` / `1 ahead`
/// (`all current`, No updates), graph HEAD stays `ahead-tip-commit`
/// (`@`, `[+=main]`), seed loses `[origin/main]`, push hint goes,
/// `Pushed 1 repo`. Not `f` fetch. Not `p` pull. Not a raw `P` byte.
/// Not a toast-only tick.
///
/// After first paint the cursor is already on the ahead repo. Do not
/// `/` search. A no-op, fetch, pull, toast-only, still-ahead, or the
/// wrong repo is red.
#[test]
fn pty_shift_p_csi_u_pushes_ahead() {
    let (_root, workspace) = ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("syncbox", WAIT);
    tui.wait_contains("ahead-tip-commit", GIT_WAIT);
    tui.wait_pred(
        idle_ahead_syncbox,
        "first paint: cursor on ahead syncbox, main ^1, HEAD is local tip, push hint",
        WAIT,
    );

    tui.shift_letter('P');
    tui.wait_pred(
        documented_shift_p_pushed,
        "Shift+P pushes: Pushed 1 repo, HEAD ahead-tip-commit [+=main], no ^1, push hint gone",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_shift_p_pushed,
        "pushed paint holds (not a flicker or toast-only tick)",
        WAIT,
    );
}

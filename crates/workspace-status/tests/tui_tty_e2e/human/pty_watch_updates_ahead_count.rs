use std::fs;

use crate::harness::{assert_contains, PtySession};
use crate::seed::{git, watch_ahead_workspace};
use crate::support::{
    after_syncbox_name, crumb_row, graph_subject_line, graph_subject_meta_line,
    panes_tree_focused_graph_unfocused, status_row, tree_cursor_on, tree_has, GIT_WAIT, WAIT,
};

/// `r` reload toast. Watch apply must not paint this.
fn refresh_now_toast(screen: &str) -> bool {
    screen.contains("refreshed app") || screen.contains("refreshed workspace")
}

/// Trailing ASCII ahead-by-N (`^N`) on the syncbox tree row.
fn syncbox_row_ahead_n(screen: &str, n: &str) -> bool {
    after_syncbox_name(screen)
        .is_some_and(|after| after.contains(&format!("^{n}")) && after.contains("& main"))
}

/// HEAD is the local ahead tip. Origin still sits on the seed.
fn ahead_tip_is_head_not_origin(screen: &str, subject: &str) -> bool {
    graph_subject_line(screen, subject)
        .is_some_and(|line| line.contains(&format!("@  {subject}")))
        && graph_subject_meta_line(screen, subject).is_some_and(|line| {
            line.contains("[+main]")
                && !line.contains("[+=main]")
                && !line.contains("[origin/main]")
        })
}

fn seed_is_origin_not_head(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("*  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox")
            .is_some_and(|line| line.contains("[origin/main]") && !line.contains("[+main]"))
}

/// First paint: cursor on syncbox ahead by 2. Graph HEAD is the local tip.
fn idle_ahead_two(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "1 ahead")
        && !tree_has(screen, "all current")
        && syncbox_row_ahead_n(screen, "2")
        && !syncbox_row_ahead_n(screen, "3")
        && panes_tree_focused_graph_unfocused(screen)
        && screen.contains("main ^2")
        && !screen.contains("main ^3")
        && screen.contains("Working tree clean")
        && ahead_tip_is_head_not_origin(screen, "watch-ahead-two")
        && seed_is_origin_not_head(screen)
        && !screen.contains("watch-ahead-three")
        && !refresh_now_toast(screen)
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace › syncbox"
        && screen.contains("? help")
}

/// Watch painted ahead-by-3 chrome and the new unique subject (no `r`).
fn paints_ahead_three(screen: &str) -> bool {
    tree_has(screen, "1 ahead")
        && syncbox_row_ahead_n(screen, "3")
        && !syncbox_row_ahead_n(screen, "2")
        && screen.contains("main ^3")
        && !screen.contains("main ^2")
        && screen.contains("watch-ahead-three")
        && ahead_tip_is_head_not_origin(screen, "watch-ahead-three")
        && !refresh_now_toast(screen)
}

/// Live watch updates ahead-by-2 to ahead-by-3 without `r`.
///
/// Tree `^2` / graph `main ^2` are the commit-count marks (workspace
/// `1 ahead` is the repo-count summary). A disk commit with subject
/// `watch-ahead-three` must move those marks to `^3` / `main ^3` and
/// paint the new subject. Count proof is that chrome, not a commit
/// message. A no-op, a toast, or a count that stays at 2 until `r`
/// cannot pass.
#[test]
fn pty_watch_updates_ahead_count() {
    let (_root, workspace) = watch_ahead_workspace();
    let repo = workspace.join("syncbox");
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_contains("syncbox", WAIT);
    tui.wait_contains("watch-ahead-two", GIT_WAIT);
    tui.wait_pred(
        idle_ahead_two,
        "first paint: syncbox ahead by 2 (main ^2 / tree ^2), watch-ahead-three absent",
        WAIT,
    );

    fs::write(repo.join("count.txt"), "three\n").unwrap();
    git(&repo, &["add", "count.txt"]);
    git(&repo, &["commit", "-q", "-m", "watch-ahead-three"]);

    tui.wait_pred(
        paints_ahead_three,
        "watch paints main ^3 / tree ^3 and watch-ahead-three without r or refresh toast",
        GIT_WAIT,
    );

    let screen = tui.screen();
    assert_contains(&screen, "watch-ahead-three");
    assert_contains(&screen, "main ^3");
    assert!(
        paints_ahead_three(&screen),
        "ahead count must move to 3 without r (frozen ^2 until reload fails); screen:\n{screen}"
    );
    crate::harness::assert_absent(&screen, "refreshed app");
    crate::harness::assert_absent(&screen, "refreshed workspace");
    crate::harness::assert_absent(&screen, "main ^2");
}

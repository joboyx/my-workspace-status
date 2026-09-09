use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, merger_graph_left_unfocused, no_wrong_overlays, right_of_split,
    right_pane, still_file_diff, title_has_files, GIT_WAIT, WAIT,
};

fn graph_row_line(screen: &str, needle: &str) -> Option<String> {
    screen.lines().find_map(|line| {
        let right = right_of_split(line);
        right.contains(needle).then_some(right)
    })
}

fn gutter_before<'a>(line: &'a str, label: &str) -> Option<&'a str> {
    let at = line.find(label)?;
    Some(&line[..at])
}

/// ASCII join elbows on merge and root gutters (`@-/`, `*-/`).
///
/// Live ASCII paint for this seed uses `/` (`╯`). `\` (`╮` / `╰`) does not
/// appear. The `/` inside `feature/graph` is in the label, not the gutter.
fn ascii_join_elbows_on_right(screen: &str) -> bool {
    let merge_line = graph_row_line(screen, "merge");
    let root_line = graph_row_line(screen, "root");
    let merge_has_slash = merge_line
        .as_deref()
        .and_then(|line| gutter_before(line, "merge"))
        .is_some_and(|g| g.contains('/'));
    let root_has_slash = root_line
        .as_deref()
        .and_then(|line| gutter_before(line, "root"))
        .is_some_and(|g| g.contains('/'));
    merge_has_slash && root_has_slash
}

/// ASCII stash node `s` on the subject-row gutter, not the `s` in `stash@{0}`.
///
/// `stash@{0}` is on the spacer; cells before that label are rails only.
fn stash_node_on_gutter(screen: &str) -> bool {
    let spacer_is_not_the_node = graph_row_line(screen, "stash@{0}")
        .is_some_and(|line| !gutter_before(&line, "stash@{0}").is_some_and(|g| g.contains('s')));
    let node = graph_row_line(screen, "WIP on graph")
        .is_some_and(|line| gutter_before(&line, "WIP on graph").is_some_and(|g| g.contains('s')));
    spacer_is_not_the_node && node
}

fn has_relative_date(screen: &str) -> bool {
    screen.contains("just now")
        || screen.contains("1m")
        || screen.contains("2m")
        || screen.contains("1h")
}

fn labels_without_lane_glyphs(screen: &str) -> bool {
    let right = right_pane(screen);
    right.contains("merge")
        && right.contains("stash@{0}")
        && (right.contains("feature/graph") || right.contains("[feature/graph]"))
        && !ascii_join_elbows_on_right(screen)
}

/// Daily merger graph: merge commit, stash spur, branch, date, author, lanes.
fn merger_multi_lane_painted(screen: &str) -> bool {
    let right = right_pane(screen);
    merger_graph_left_unfocused(screen)
        && right.contains("merge")
        && right.contains("stash@{0}")
        && (right.contains("feature/graph") || right.contains("[feature/graph]"))
        && has_relative_date(screen)
        && screen.contains("workspace-stat")
        && ascii_join_elbows_on_right(screen)
        && stash_node_on_gutter(screen)
        && !still_file_diff(screen)
        && !title_has_files(screen)
        && !screen.contains("wip.txt")
        && no_wrong_overlays(screen)
}

/// Daily `merger` graph paints merge elbows and the stash spur.
///
/// Live PTY with ASCII glyphs. The right pane shows the merge commit,
/// `stash@{0}`, `feature/graph`, a relative date, and author
/// `workspace-stat`. Join elbows are ASCII `/` on the merge and root
/// gutters (`@-/`, `*-/`); a labels-only frame cannot pass. The stash
/// node is ASCII `s` on the subject-row gutter, not the `s` inside
/// `stash@{0}`. Files drill, README file-diff, or a stash overlay cannot
/// pass. Does not apply or pop.
///
/// MYWS-005.
#[test]
fn pty_multi_lane_graph_paints_merge_and_stash_spur() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff (merger graph has not loaded)",
        WAIT,
    );
    assert!(
        labels_without_lane_glyphs(&tui.screen()) || !ascii_join_elbows_on_right(&tui.screen()),
        "launch file-diff must not already satisfy merge-lane elbows:\n{}",
        tui.screen()
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph (left focus)",
        GIT_WAIT,
    );
    tui.wait_pred(
        merger_multi_lane_painted,
        "merger graph paints merge/stash labels, relative date, author, ASCII / join elbows, and stash node s",
        WAIT,
    );
}

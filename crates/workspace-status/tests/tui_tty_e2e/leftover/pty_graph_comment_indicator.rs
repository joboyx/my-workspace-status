use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, graph_cursor_on, merger_graph_drilled_right,
    merger_graph_left_unfocused, pane_unstaged_readme, right_of_split, tree_cursor_on,
    tree_line_containing, GIT_WAIT, WAIT,
};

const COMMIT_BODY: &str = "graph-commit-mark-e2e";
const FILE_BODY: &str = "graph-file-mark-e2e";
const WT_BODY: &str = "graph-wt-mark-e2e";

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
        && !screen.contains("MOVE")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save") && !screen.contains("empty deletes")
}

fn graph_row_line(screen: &str, needle: &str) -> Option<String> {
    screen.lines().find_map(|line| {
        let right = right_of_split(line);
        right.contains(needle).then_some(right)
    })
}

fn graph_row_has_ascii_comment(screen: &str, needle: &str) -> bool {
    graph_row_line(screen, needle).is_some_and(|line| line.contains('"'))
}

fn left_row_has_ascii_comment(screen: &str, needle: &str) -> bool {
    tree_line_containing(screen, needle).is_some_and(|line| line.contains('"'))
}

fn commit_files_right(screen: &str) -> bool {
    (screen.contains("left.txt") || screen.contains("right.txt") || screen.contains("README.md"))
        && (screen.contains(" files") || screen.contains("┌ files"))
        && !screen.contains("wip.txt")
        && overlay_closed(screen)
}

fn commit_file_diff(screen: &str) -> bool {
    overlay_closed(screen)
        && (screen.contains("┌ diff") || screen.contains("UNSTAGED") || screen.contains("@@"))
        && (screen.contains("left.txt")
            || screen.contains("right.txt")
            || screen.contains("README.md"))
}

fn save_overlay_body(tui: &mut PtySession, body: &str) {
    tui.key(';');
    tui.wait_pred(comment_overlay, "; opens Comment overlay", WAIT);
    tui.keys(body);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the comment",
        WAIT,
    );
}

fn merger_merge_commit_selected(screen: &str) -> bool {
    graph_cursor_on(screen, "merge")
        && !graph_cursor_on(screen, "WIP on graph")
        && !graph_cursor_on(screen, "working tree")
}

/// `;` on a graph commit paints ASCII `"` on that row, including the
/// selected cursor. Uncommented rows stay unmarked.
///
/// Docs: comment glyph is `ICON_COMMENT` (`"`). Graph selected cursor
/// stays `▌`. A cursor-column-only mark that hides under `▌`, a toast
/// with no row glyph, or a mark on stash / working tree is red.
#[test]
fn pty_graph_commit_comment_paints_mark() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        merger_graph_drilled_right,
        "Enter on merger focuses that graph",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "WIP on graph") && !graph_cursor_on(screen, "working tree")
        },
        "first j selects stash@{0}",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        merger_merge_commit_selected,
        "second j selects the merge commit",
        WAIT,
    );
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && !graph_row_has_ascii_comment(screen, "merge")
                && !graph_row_has_ascii_comment(screen, "WIP on graph")
                && !graph_row_has_ascii_comment(screen, "working tree")
        },
        "graph rows have no comment mark before ;",
        WAIT,
    );

    save_overlay_body(&mut tui, COMMIT_BODY);
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && merger_merge_commit_selected(screen)
                && graph_row_has_ascii_comment(screen, "merge")
                && graph_cursor_on(screen, "merge")
                && !graph_row_has_ascii_comment(screen, "WIP on graph")
                && !graph_row_has_ascii_comment(screen, "working tree")
        },
        "saved commit comment paints ASCII \" on the selected merge row",
        WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && graph_cursor_on(screen, "WIP on graph")
                && !graph_cursor_on(screen, "merge")
                && graph_row_has_ascii_comment(screen, "merge")
                && !graph_row_has_ascii_comment(screen, "WIP on graph")
                && !graph_row_has_ascii_comment(screen, "working tree")
        },
        "unselected merge row keeps \"; stash and working tree stay unmarked",
        WAIT,
    );
}

/// A commit-file line comment paints `"` on that file row and on the
/// graph commit. Other files stay unmarked.
///
/// Docs + VIEW: `;` comments the focused numbered line. Esc from the
/// diff focuses the commit-file list. A paint-only overlay, a mark on
/// every file, or a graph with no mark after Esc is red.
#[test]
fn pty_graph_commit_file_comment_paints_mark() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff (graph drill has not run)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        merger_graph_drilled_right,
        "Enter on merger focuses that graph",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "WIP on graph") && !graph_cursor_on(screen, "working tree")
        },
        "first j selects stash@{0}",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        merger_merge_commit_selected,
        "second j selects the merge commit",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        commit_files_right,
        "Enter on the merge commit opens its file list",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        commit_file_diff,
        "Enter on a commit file opens the numbered commit diff",
        WAIT,
    );

    save_overlay_body(&mut tui, FILE_BODY);
    tui.esc();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && (left_row_has_ascii_comment(screen, "README.md")
                    || left_row_has_ascii_comment(screen, "left.txt")
                    || left_row_has_ascii_comment(screen, "right.txt"))
                && !(left_row_has_ascii_comment(screen, "README.md")
                    && left_row_has_ascii_comment(screen, "left.txt"))
        },
        "Esc to the commit-file list paints ASCII \" on the commented file only",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && left_row_has_ascii_comment(screen, "merge")
                && !left_row_has_ascii_comment(screen, "WIP on graph")
                && !left_row_has_ascii_comment(screen, "working tree")
        },
        "Esc to the graph paints ASCII \" on the merge commit (file comments)",
        WAIT,
    );
}

/// A working-tree line comment paints `"` on the graph uncommitted row.
///
/// Launch is the dirty README. Tab comments that line. Esc, `k`, Enter
/// open app's graph. A no-op graph or a mark on every graph row is red.
#[test]
fn pty_graph_uncommitted_marks_worktree_line_comment() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && pane_unstaged_readme(screen)
                && overlay_closed(screen)
        },
        "Tab focuses the dirty README diff",
        WAIT,
    );
    save_overlay_body(&mut tui, WT_BODY);
    tui.esc();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "app")
        },
        "Esc returns to the left tree on README",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "README.md"),
        "k from README focuses app",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && graph_cursor_on(screen, "uncommitted")
                && graph_row_has_ascii_comment(screen, "uncommitted")
                && !graph_row_has_ascii_comment(screen, "seed app")
        },
        "app graph uncommitted row paints ASCII \" for the README line comment",
        GIT_WAIT,
    );
}

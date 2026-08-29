use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, merger_graph_drilled_right, merger_graph_left_unfocused,
    GIT_WAIT, WAIT,
};

const BODY: &str = "commit-line-note-e2e";

fn comment_store(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".e2e-state").join("comments.json")
}

fn store_text(workspace: &Path) -> String {
    fs::read_to_string(comment_store(workspace)).unwrap_or_default()
}

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

fn commit_files_right(screen: &str) -> bool {
    (screen.contains("left.txt") || screen.contains("right.txt") || screen.contains("README.md"))
        && (screen.contains(" files") || screen.contains("┌ files"))
        && !comment_overlay(screen)
}

fn commit_file_diff(screen: &str) -> bool {
    overlay_closed(screen)
        && (screen.contains("left.txt")
            || screen.contains("right.txt")
            || screen.contains("README.md"))
        && (screen.contains(" files") || screen.contains("┌ files"))
        && (screen.contains("split") || screen.contains("inline"))
}

fn commit_line_commented(screen: &str) -> bool {
    overlay_closed(screen)
        && screen.contains('"')
        && screen.contains("comment saved")
        && (screen.contains("left.txt")
            || screen.contains("right.txt")
            || screen.contains("README.md"))
}

/// `;` on a commit file diff saves a line comment keyed to the SHA.
///
/// Docs + VIEW: `;` comments the focused row / line. Daily `merger`
/// drill: `j`, Enter, `j` `j`, Enter (files), Enter (diff). A stash
/// no-op, overlay-only tick, or working-tree line key is red.
#[test]
fn pty_semicolon_line_comment_commit_diff() {
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
    tui.key('j');
    tui.enter();
    tui.wait_pred(
        commit_files_right,
        "j j Enter opens a commit file list (not stash / uncommitted)",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        commit_file_diff,
        "Enter on a commit file opens the numbered commit diff",
        WAIT,
    );

    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on the commit file line",
        WAIT,
    );
    tui.keys(BODY);
    tui.enter();
    tui.wait_pred(
        commit_line_commented,
        "Enter saves: overlay gone, ASCII \" on the commit diff, toast comment saved",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && stored.contains("commitLine"),
        "store must keep a commit-line comment (not worktreeLine):\n{stored}"
    );
}

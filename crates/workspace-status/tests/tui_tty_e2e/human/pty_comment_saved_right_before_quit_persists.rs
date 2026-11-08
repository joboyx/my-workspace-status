use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, tree_cursor_on, tree_has, WAIT,
};

const BODY: &str = "saved-right-before-quit";

fn comment_store(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".e2e-state").join("comments.json")
}

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
}

fn right_diff_focused(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && !tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && !comment_overlay(screen)
}

/// Enter on the Comment overlay and `q` in one write: the save runs on a
/// worker, and quit comes before the loop spawns it. Quit must still write
/// the comment to the store.
#[test]
fn pty_comment_saved_right_before_quit_persists() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        right_diff_focused,
        "Tab focuses the dirty README diff",
        WAIT,
    );

    tui.key(';');
    tui.wait_pred(comment_overlay, "; opens the Comment overlay", WAIT);
    tui.keys(BODY);
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains(BODY),
        "typed body appears in the overlay",
        WAIT,
    );

    tui.send_bytes(b"\rq");
    tui.wait_exit(WAIT);
    let stored = fs::read_to_string(comment_store(&workspace)).unwrap_or_default();
    assert!(
        stored.contains(BODY),
        "the comment saved just before quit must reach the store:\n{stored}"
    );
}

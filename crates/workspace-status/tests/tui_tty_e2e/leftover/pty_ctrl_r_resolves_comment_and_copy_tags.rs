use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, tree_cursor_on, GIT_WAIT, WAIT,
};

const BODY: &str = "resolve-note-e2e";

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
        && !screen.contains("# Comments")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save")
        && !screen.contains("empty deletes")
        && !screen.contains("copied to clipboard")
}

fn export_overlay(screen: &str) -> bool {
    screen.contains("# Comments")
        && screen.contains("copied to clipboard")
        && screen.contains("copied · Esc close")
        && !screen.contains("MOVE")
}

fn right_diff_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn overlay_resolved(screen: &str) -> bool {
    comment_overlay(screen)
        && screen.contains("Comment · resolved")
        && screen.contains("Ctrl-R unresolve")
        && !screen.contains("Ctrl-R resolve ·")
}

fn overlay_open(screen: &str) -> bool {
    comment_overlay(screen)
        && screen.contains("Ctrl-R resolve")
        && !screen.contains("Comment · resolved")
}

/// Overlay `Ctrl-R` marks a comment resolved. `y` still copies it with
/// `[resolved]`. A second `Ctrl-R` unresolves. Hunt leftover: missing
/// overlay chrome, a copy that drops resolved comments, or a copy that
/// omits the tag is red.
#[test]
fn pty_ctrl_r_resolves_comment_and_copy_tags() {
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
    tui.wait_pred(
        overlay_open,
        "; opens Comment overlay with Ctrl-R resolve (not already resolved)",
        WAIT,
    );
    tui.keys(BODY);
    tui.wait_pred(
        |screen| overlay_open(screen) && screen.contains(BODY),
        "typed body appears in the overlay",
        WAIT,
    );
    tui.ctrl_letter('r');
    tui.wait_pred(
        |screen| overlay_resolved(screen) && screen.contains(BODY),
        "Ctrl-R marks the overlay Comment · resolved",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && pane_unstaged_readme(screen)
                && screen.contains("comment saved")
                && screen.contains('\'')
                && tree_cursor_on(screen, "README.md")
        },
        "Enter saves: overlay gone, ASCII ' resolved mark, toast comment saved",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && stored.contains("\"resolved\": true"),
        "store must keep the body and resolved flag:\n{stored}"
    );

    tui.key('y');
    tui.wait_pred(
        |screen| export_overlay(screen) && screen.contains(BODY) && screen.contains("[resolved]"),
        "y copies the resolved comment and tags it [resolved]",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(overlay_closed, "Esc closes the export overlay", WAIT);

    tui.key(';');
    tui.wait_pred(
        |screen| overlay_resolved(screen) && screen.contains(BODY),
        "; reopens the overlay still resolved",
        WAIT,
    );
    tui.ctrl_letter('r');
    tui.wait_pred(
        |screen| overlay_open(screen) && screen.contains(BODY),
        "second Ctrl-R unresolves (Ctrl-R resolve, not Comment · resolved)",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the unresolved comment",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && !stored.contains("\"resolved\": true"),
        "unresolve must drop the resolved flag and keep the body:\n{stored}"
    );

    tui.key('y');
    tui.wait_pred(
        |screen| export_overlay(screen) && screen.contains(BODY) && !screen.contains("[resolved]"),
        "y still copies the open comment and does not tag it resolved",
        WAIT,
    );
}

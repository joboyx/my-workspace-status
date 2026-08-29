use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, tree_cursor_on, GIT_WAIT, SETTLE_MS, WAIT,
};

const BODY: &str = "wt-line-note-e2e";

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

fn right_diff_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn dirty_line_commented(screen: &str) -> bool {
    overlay_closed(screen)
        && pane_unstaged_readme(screen)
        && screen.contains('"')
        && screen.contains("comment saved")
        && tree_cursor_on(screen, "README.md")
}

fn dirty_line_uncommented(screen: &str) -> bool {
    overlay_closed(screen)
        && pane_unstaged_readme(screen)
        && screen.contains("comment deleted")
        && tree_cursor_on(screen, "README.md")
}

/// `;` on a focused dirty file diff saves a line comment (ASCII `"`).
///
/// Docs + VIEW: `;` comments the focused row / line. Empty Enter deletes.
/// Launch is the README file diff. Tab focuses that numbered diff. A
/// tree-row no-op, overlay-only tick, or paint without the store is red.
#[test]
fn pty_semicolon_line_comment_dirty_file() {
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
        "Tab focuses the dirty README diff (not a tree comment)",
        WAIT,
    );

    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on the numbered dirty line",
        WAIT,
    );
    tui.keys(BODY);
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains(BODY),
        "typed body appears in the overlay",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        dirty_line_commented,
        "Enter saves: overlay gone, ASCII \" on the dirty diff, toast comment saved",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && stored.contains("worktreeLine"),
        "store must keep the dirty-line comment, not overlay-only:\n{stored}"
    );

    tui.key(';');
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains(BODY),
        "; reopens the overlay with the saved body",
        WAIT,
    );
    for _ in 0..BODY.len() {
        tui.send_bytes(b"\x7f");
    }
    tui.wait_pred(
        |screen| comment_overlay(screen) && !screen.contains(BODY),
        "backspace clears the overlay body",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        dirty_line_uncommented,
        "empty Enter deletes: overlay gone, toast comment deleted",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    let stored = store_text(&workspace);
    assert!(
        !stored.contains(BODY),
        "empty submit must drop the dirty-line comment:\n{stored}"
    );
}

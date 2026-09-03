use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, tree_cursor_on, tree_has, GIT_WAIT, WAIT,
};

const LINE1: &str = "one";
const HEADING: &str = "# heading";
const LIST: &str = "- list";
const KEY_GAP_MS: u64 = 50;

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
        && screen.contains("Shift+Enter newline")
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

fn right_diff_focused(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && !tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn shift_enter(tui: &mut PtySession) {
    tui.csi_u(13, 2, 1);
    tui.csi_u(13, 2, 3);
}

fn export_escapes_bullet(screen: &str) -> bool {
    screen.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed == HEADING || trimmed == LIST
    })
}

/// `y` copy of a Shift+Enter body stays one markdown bullet. Ctrl-R still
/// tags `[resolved]`. A raw `# heading` / `- list` line is red.
#[test]
fn pty_semicolon_comment_multiline_export() {
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
        "; opens Comment overlay with Ctrl-R resolve",
        WAIT,
    );
    tui.keys(LINE1);
    shift_enter(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    tui.keys(HEADING);
    shift_enter(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    tui.keys(LIST);
    tui.wait_pred(
        |screen| {
            overlay_open(screen)
                && screen.contains(LINE1)
                && screen.contains(HEADING)
                && screen.contains(LIST)
        },
        "typed multiline body shows heading and list lines in the overlay",
        WAIT,
    );
    tui.ctrl_letter('r');
    tui.wait_pred(
        |screen| overlay_resolved(screen) && screen.contains(LINE1),
        "Ctrl-R marks the overlay Comment · resolved (not textarea Redo)",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && pane_unstaged_readme(screen)
                && screen.contains("comment saved")
                && tree_has(screen, "README.md")
                && !tree_cursor_on(screen, "README.md")
        },
        "Enter saves: overlay gone, toast comment saved",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains("one\\n# heading\\n- list") && stored.contains("\"resolved\": true"),
        "store must keep newlines and resolved:\n{stored}"
    );

    tui.key('y');
    tui.wait_pred(
        |screen| {
            export_overlay(screen)
                && screen.contains(LINE1)
                && screen.contains("[resolved]")
                && screen.contains("> # heading")
                && screen.contains("> - list")
                && !export_escapes_bullet(screen)
        },
        "y copies one quoted bullet with [resolved]; heading/list stay nested",
        WAIT,
    );
}

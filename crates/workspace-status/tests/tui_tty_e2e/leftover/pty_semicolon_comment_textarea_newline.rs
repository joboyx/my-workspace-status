use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, status_row, tree_cursor_on, GIT_WAIT,
    SETTLE_MS, WAIT,
};

const LINE1: &str = "one";
const LINE2: &str = "two three";
/// Kitty CSI-u Left. Crossterm 0.28 `event::read` yields Char U+E006.
const KITTY_LEFT: u32 = 57350;
/// Kitty CSI-u Right. Pair of [`KITTY_LEFT`].
const KITTY_RIGHT: u32 = 57351;
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
        && screen.contains("Ctrl-A/E line")
        && screen.contains("Ctrl-Left/Right word")
        && !screen.contains("MOVE")
        && !screen.contains("# Comments")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save") && !screen.contains("empty deletes")
}

fn overlay_region(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let start = lines.iter().position(|line| {
        line.contains("Comment")
            && !line.contains("comment saved")
            && !line.contains("comment cancelled")
    });
    match start {
        Some(i) => lines[i..].join("\n"),
        None => String::new(),
    }
}

fn idle_status_occluded(screen: &str) -> bool {
    comment_overlay(screen) && {
        let overlay = overlay_region(screen);
        let status = status_row(screen);
        !overlay.contains("? help")
            && !overlay.contains("focus right")
            && !status.contains("? help")
            && !status.contains("focus right")
            && !status.contains(" tree")
    }
}

fn right_diff_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn shift_enter(tui: &mut PtySession) {
    tui.csi_u(13, 2, 1);
    tui.csi_u(13, 2, 3);
}

fn ctrl_left(tui: &mut PtySession) {
    tui.csi_u(KITTY_LEFT, 5, 1);
    tui.csi_u(KITTY_LEFT, 5, 3);
}

fn ctrl_right(tui: &mut PtySession) {
    tui.csi_u(KITTY_RIGHT, 5, 1);
    tui.csi_u(KITTY_RIGHT, 5, 3);
}

/// `;` comment overlay is a textarea. Shift+Enter inserts a newline.
/// Ctrl-A / Ctrl-E are line start / end. Ctrl-Left / Ctrl-Right move by
/// word. Append-only Enter, or a no-op advertised shortcut, fails.
#[test]
fn pty_semicolon_comment_textarea_newline() {
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
        idle_status_occluded,
        "; opens Comment overlay and covers idle status chips",
        WAIT,
    );
    tui.keys(LINE1);
    tui.wait_pred(
        |screen| {
            idle_status_occluded(screen)
                && overlay_region(screen).contains(&format!("{LINE1}▏"))
                && overlay_region(screen).matches(LINE1).count() == 1
        },
        "typed first line shows a caret at the end once",
        WAIT,
    );

    shift_enter(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    tui.wait_pred(
        |screen| {
            idle_status_occluded(screen)
                && overlay_region(screen).contains(&format!("{LINE1}"))
                && overlay_region(screen).contains("▏")
                && !overlay_region(screen).contains(&format!("{LINE1}▏"))
                && !overlay_region(screen).contains(&format!("{LINE1}{LINE2}"))
        },
        "Shift+Enter starts a second line (not append-only)",
        WAIT,
    );

    tui.keys(LINE2);
    tui.wait_pred(
        |screen| {
            let overlay = overlay_region(screen);
            idle_status_occluded(screen)
                && overlay.contains(LINE1)
                && overlay.contains(&format!("{LINE2}▏"))
                && !overlay.contains(&format!("{LINE1}{LINE2}"))
                && !overlay.contains(&format!("{LINE1}▏"))
        },
        "second line types after the newline, not glued to line 1",
        WAIT,
    );

    ctrl_left(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    tui.wait_pred(
        |screen| {
            let overlay = overlay_region(screen);
            idle_status_occluded(screen)
                && overlay.contains("two ▏three")
                && !overlay.contains(&format!("{LINE2}▏"))
                && overlay.contains(LINE1)
        },
        "Ctrl-Left jumps to the previous word (not a no-op, not char-left)",
        WAIT,
    );

    tui.ctrl_letter('a');
    tui.wait_ms(KEY_GAP_MS);
    tui.wait_pred(
        |screen| {
            let overlay = overlay_region(screen);
            idle_status_occluded(screen)
                && overlay.contains("▏two three")
                && !overlay.contains("two ▏three")
                && overlay.contains(LINE1)
                && !overlay.contains(&format!("{LINE1}▏"))
        },
        "Ctrl-A is current-line start (not buffer start)",
        WAIT,
    );

    ctrl_right(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    tui.wait_pred(
        |screen| {
            let overlay = overlay_region(screen);
            idle_status_occluded(screen)
                && overlay.contains("two ▏three")
                && !overlay.contains("▏two three")
                && overlay.contains(LINE1)
        },
        "Ctrl-Right jumps to the next word (not a no-op)",
        WAIT,
    );

    tui.ctrl_letter('e');
    tui.wait_ms(KEY_GAP_MS);
    tui.wait_pred(
        |screen| {
            let overlay = overlay_region(screen);
            idle_status_occluded(screen)
                && overlay.contains("two three▏")
                && !overlay.contains("two ▏three")
                && overlay.contains(LINE1)
                && !overlay.contains(&format!("{LINE1}▏"))
        },
        "Ctrl-E is current-line end (not buffer start)",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && pane_unstaged_readme(screen)
                && screen.contains("comment saved")
                && tree_cursor_on(screen, "README.md")
        },
        "Enter saves: overlay gone, toast comment saved",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    let stored = store_text(&workspace);
    assert!(
        stored.contains("one\\ntwo three")
            && !stored.contains("onetwo three")
            && !stored.contains("one two three"),
        "store must keep the Shift+Enter newline, not append-only:\n{stored}"
    );
}

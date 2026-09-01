use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, status_row, tree_cursor_on, GIT_WAIT,
    SETTLE_MS, WAIT,
};

const BODY: &str = "textarea-note-e2e";
const PREFIX: &str = "Z";
/// Kitty CSI-u Left. Crossterm 0.28 `event::read` yields Char U+E006.
const KITTY_LEFT: u32 = 57350;
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

fn csi_u_left(tui: &mut PtySession) {
    tui.csi_u(KITTY_LEFT, 1, 1);
    tui.csi_u(KITTY_LEFT, 1, 3);
}

fn send_delete(tui: &mut PtySession) {
    tui.send_bytes(b"\x1b[3;1:1~");
    tui.send_bytes(b"\x1b[3;1:3~");
}

/// `;` comment overlay is a caret editor. Idle status stays out of the box.
///
/// Home / Left / Delete must edit at the caret. Append-only `push` / `pop`,
/// a status echo of `body: …` inside the overlay, or idle `? help` chips
/// on the last row cannot pass.
#[test]
fn pty_semicolon_comment_textarea_cursor() {
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
    tui.keys(BODY);
    tui.wait_pred(
        |screen| {
            idle_status_occluded(screen)
                && overlay_region(screen).contains(&format!("{BODY}▏"))
                && overlay_region(screen).matches(BODY).count() == 1
                && !overlay_region(screen).contains(&format!("▏{BODY}"))
        },
        "typed body shows a caret at the end once, not a status echo",
        WAIT,
    );

    tui.home();
    tui.wait_ms(KEY_GAP_MS);
    tui.wait_pred(
        |screen| {
            idle_status_occluded(screen)
                && overlay_region(screen).contains(&format!("▏{BODY}"))
                && !overlay_region(screen).contains(&format!("{BODY}▏"))
        },
        "Home moves the caret to the start (not append-only)",
        WAIT,
    );

    csi_u_left(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    tui.wait_pred(
        |screen| {
            idle_status_occluded(screen)
                && overlay_region(screen).contains(&format!("▏{BODY}"))
                && !overlay_region(screen).contains(&format!("{BODY}▏"))
        },
        "Left at the start stays at the start",
        WAIT,
    );

    send_delete(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    let after_delete: String = BODY.chars().skip(1).collect();
    tui.wait_pred(
        |screen| {
            idle_status_occluded(screen)
                && overlay_region(screen).contains(&format!("▏{after_delete}"))
                && !overlay_region(screen).contains(BODY)
                && !overlay_region(screen).contains(&format!("{BODY}▏"))
        },
        "Delete removes the first character, not the last",
        WAIT,
    );

    tui.keys(PREFIX);
    tui.wait_pred(
        |screen| {
            idle_status_occluded(screen)
                && overlay_region(screen).contains(&format!("{PREFIX}▏{after_delete}"))
                && !overlay_region(screen).contains(&format!("{after_delete}{PREFIX}"))
        },
        "insert at the caret prefixes the remaining body",
        WAIT,
    );

    tui.enter();
    let expected = format!("{PREFIX}{after_delete}");
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
        stored.contains(&expected)
            && !stored.contains(BODY)
            && !stored.contains(&format!("{BODY}{PREFIX}")),
        "store must keep mid-string edit {expected}, not append-only {BODY}{PREFIX}:\n{stored}"
    );
}

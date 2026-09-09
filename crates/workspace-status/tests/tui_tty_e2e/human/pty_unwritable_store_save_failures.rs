use std::fs;
use std::path::PathBuf;

use crate::harness::{assert_absent, PtySession};
use crate::seed::{daily_workspace, unique_root};
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, tree_cursor_on, tree_has, WAIT,
};

const BODY: &str = "fail-note-e2e";

fn unwritable_store_path(prefix: &str) -> (PathBuf, PathBuf) {
    let root = unique_root(prefix);
    fs::create_dir_all(&root).unwrap();
    let blocker = root.join("not-a-dir");
    fs::write(&blocker, "x").unwrap();
    (root, blocker.join("store.json"))
}

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
        && !screen.contains("MOVE")
        && !screen.contains("# Comments")
}

fn right_diff_focused(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && !tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

/// CSI-u semicolon (`CSI 59 ; 1 : 1 u` press, `: 3` release).
///
/// A raw `';'` byte is a different path.
fn csi_u_semicolon(tui: &mut PtySession) {
    tui.csi_u(59, 1, 1);
    tui.csi_u(59, 1, 3);
}

/// `;` on a focused dirty file diff names persist failure when the comment
/// store parent is a regular file (`not-a-dir/store.json`).
///
/// Tab focuses the README diff. CSI-u `;` opens Comment. Enter must paint
/// `comment save failed` and must not paint `comment saved`. A no-op `;`,
/// overlay-only tick, or success toast is red.
#[test]
fn pty_semicolon_save_names_failure_when_unwritable() {
    let (_root, workspace) = daily_workspace();
    let (blocker_root, path) = unwritable_store_path("ws-pty-comment-fail");
    let store = path.to_str().expect("utf-8 store path").to_string();
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_COMMENT_STORE", &store)]);
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

    csi_u_semicolon(&mut tui);
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
        |screen| screen.contains("comment save failed"),
        "Enter names comment save failed (unwritable store, not a no-op)",
        WAIT,
    );
    assert_absent(&tui.screen(), "comment saved");
    let _ = fs::remove_dir_all(blocker_root);
}

/// Space on a dirty file names persist failure when the viewed store parent
/// is a regular file (`not-a-dir/store.json`).
///
/// After first paint the cursor is already on the dirty README. Space must
/// paint `viewed save failed` and must not paint `comment saved`. A no-op
/// Space is red. Session `*` is not required.
#[test]
fn pty_space_reviewed_names_save_failure_when_unwritable() {
    let (_root, workspace) = daily_workspace();
    let (blocker_root, path) = unwritable_store_path("ws-pty-viewed-fail");
    let store = path.to_str().expect("utf-8 store path").to_string();
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_VIEWED_STORE", &store)]);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.key(' ');
    tui.wait_pred(
        |screen| screen.contains("viewed save failed"),
        "Space names viewed save failed (unwritable store, not a no-op)",
        WAIT,
    );
    assert_absent(&tui.screen(), "comment saved");
    let _ = fs::remove_dir_all(blocker_root);
}

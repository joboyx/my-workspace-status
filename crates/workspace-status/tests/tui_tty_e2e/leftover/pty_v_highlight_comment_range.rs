use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_top, pane_unstaged_readme, tree_cursor_on, GIT_WAIT, WAIT,
};

const BODY: &str = "range-note-e2e";
const KEY_GAP_MS: u64 = 50;
/// Kitty CSI-u Down. Crossterm 0.28 `event::read` yields Char U+E009.
const KITTY_DOWN: u32 = 57353;

fn comment_store(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".e2e-state").join("comments.json")
}

fn store_text(workspace: &Path) -> String {
    fs::read_to_string(comment_store(workspace)).unwrap_or_default()
}

/// CSI-u unmodified letter (`CSI code ; 1 : 1 u` press, `: 3` release).
///
/// Two ASCII `j` bytes (`PtySession::key`) are a different path.
fn csi_u_letter(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

fn csi_u_kitty_down(tui: &mut PtySession) {
    tui.csi_u(KITTY_DOWN, 1, 1);
    tui.csi_u(KITTY_DOWN, 1, 3);
}

/// CSI-u semicolon (`CSI 59 ; 1 : 1 u` press, `: 3` release).
///
/// A raw `';'` byte is a different path.
fn csi_u_semicolon(tui: &mut PtySession) {
    tui.csi_u(59, 1, 1);
    tui.csi_u(59, 1, 3);
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

fn right_diff_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && top.contains(" diff ")
        && !top.contains(" tree ")
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
        && !screen.contains("VISUAL")
}

fn highlight_active(screen: &str) -> bool {
    right_diff_focused_allow_visual(screen)
        && screen.contains("VISUAL")
        && screen.contains("cancel highlight")
        && !comment_overlay(screen)
        && !screen.contains("MOVE")
}

fn right_diff_focused_allow_visual(screen: &str) -> bool {
    let top = pane_top(screen);
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && top.contains(" diff ")
        && !top.contains(" tree ")
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn range_overlay(screen: &str) -> bool {
    comment_overlay(screen)
        && screen.contains("README.md:1-2")
        && !screen.contains("README.md:1-2-")
}

fn range_commented(screen: &str) -> bool {
    overlay_closed(screen)
        && pane_unstaged_readme(screen)
        && screen.contains('"')
        && screen.contains("comment saved")
        && tree_cursor_on(screen, "README.md")
        && !screen.contains("VISUAL")
}

/// `V` on a focused file diff starts visual-line highlight. `j` / Down
/// extend the range. `;` opens Comment on that line span, not one line.
///
/// Docs + VIEW: `V` highlights diff lines for `;`. CSI-u Shift+V is the
/// live encoding (`CSI 118 ; 2 : 1 u` press, `: 3` release). A raw `V`
/// byte, overlay-only tick, or `worktreeLine` without `endLine` is red.
#[test]
fn pty_v_then_semicolon_comments_line_range() {
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

    tui.shift_letter('V');
    tui.wait_pred(
        highlight_active,
        "CSI-u Shift+V paints VISUAL highlight (not a no-op, not Comment)",
        WAIT,
    );

    tui.wait_ms(KEY_GAP_MS);
    csi_u_letter(&mut tui, 'j');
    tui.wait_ms(KEY_GAP_MS);
    csi_u_kitty_down(&mut tui);
    tui.wait_ms(KEY_GAP_MS);
    csi_u_letter(&mut tui, 'j');
    tui.wait_pred(
        |screen| highlight_active(screen) && screen.contains("+dirty"),
        "CSI-u j and Kitty Down keep VISUAL and still show the dirty add",
        WAIT,
    );

    csi_u_semicolon(&mut tui);
    tui.wait_pred(
        range_overlay,
        "; opens Comment on README.md:1-2 (not a single line)",
        WAIT,
    );
    tui.keys(BODY);
    tui.wait_pred(
        |screen| range_overlay(screen) && screen.contains(BODY),
        "typed body appears in the range overlay",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        range_commented,
        "Enter saves: overlay gone, VISUAL gone, ASCII \" on the dirty diff",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY)
            && stored.contains("worktreeLine")
            && stored.contains("\"endLine\": 2")
            && stored.contains("\"line\": 1"),
        "store must keep a worktree line range 1-2, not a single line:\n{stored}"
    );
}

/// Esc leaves highlight and stays on the focused diff. It does not
/// comment and does not unfocus.
///
/// CSI-u Escape (`CSI 27 u`). If `V` is a no-op, Esc unfocuses the tree
/// and this leftover is red.
#[test]
fn pty_v_esc_exits_highlight_without_comment() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.shift_letter('V');
    tui.wait_pred(
        |screen| {
            documented_launch_first_paint(screen)
                && !screen.contains("VISUAL")
                && !comment_overlay(screen)
        },
        "V on the left tree is a no-op (no VISUAL, no Comment)",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        right_diff_focused,
        "Tab focuses the dirty README diff",
        WAIT,
    );

    tui.shift_letter('V');
    tui.wait_pred(
        highlight_active,
        "CSI-u Shift+V paints VISUAL on the focused diff",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        right_diff_focused,
        "Esc drops VISUAL and stays on the focused diff (no Comment, no unfocus)",
        WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.trim().is_empty() || !stored.contains(BODY),
        "Esc must not write a comment:\n{stored}"
    );
}

use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, panes_tree_unfocused_diff_focused,
    tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT,
};

const BODY: &str = "range-note-e2e";
const WATCH_BODY: &str = "watch-span-e2e";
const WATCH_EXTRA: &str = "watch-extra-line";
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
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && panes_tree_unfocused_diff_focused(screen)
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
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && panes_tree_unfocused_diff_focused(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn range_overlay(screen: &str) -> bool {
    comment_overlay(screen)
        && screen.contains("README.md:1-2")
        && !screen.contains("README.md:1-2-")
}

fn highlight_readme_range(tui: &mut PtySession) {
    tui.shift_letter('V');
    tui.wait_pred(
        highlight_active,
        "CSI-u Shift+V paints VISUAL highlight (not a no-op, not Comment)",
        WAIT,
    );
    tui.wait_ms(KEY_GAP_MS);
    csi_u_letter(tui, 'j');
    tui.wait_ms(KEY_GAP_MS);
    csi_u_kitty_down(tui);
    tui.wait_ms(KEY_GAP_MS);
    csi_u_letter(tui, 'j');
    tui.wait_pred(
        |screen| highlight_active(screen) && screen.contains("+dirty"),
        "CSI-u j and Kitty Down keep VISUAL and still show the dirty add",
        WAIT,
    );
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

    highlight_readme_range(&mut tui);

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

/// `;` without highlight opens the covering span the `"` glyph already
/// shows. Empty Enter deletes that range, not a new empty single-line key.
///
/// CSI-u `;` (`CSI 59 ; 1 : 1 u`). A single-line overlay (`README.md:2`
/// with an empty body) is red.
#[test]
fn pty_semicolon_opens_existing_range_on_covered_line() {
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

    highlight_readme_range(&mut tui);
    csi_u_semicolon(&mut tui);
    tui.wait_pred(range_overlay, "; opens Comment on README.md:1-2", WAIT);
    tui.keys(BODY);
    tui.wait_pred(
        |screen| range_overlay(screen) && screen.contains(BODY),
        "typed body appears in the range overlay",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        range_commented,
        "Enter saves the 1-2 range (VISUAL off, ASCII \")",
        GIT_WAIT,
    );

    csi_u_semicolon(&mut tui);
    tui.wait_pred(
        |screen| range_overlay(screen) && screen.contains(BODY),
        "; without V reopens the covering 1-2 span with the saved body",
        WAIT,
    );
    for _ in 0..BODY.len() {
        tui.send_bytes(b"\x7f");
    }
    tui.wait_pred(
        |screen| range_overlay(screen) && !screen.contains(BODY),
        "backspace clears the covering-span body",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && pane_unstaged_readme(screen)
                && screen.contains("comment deleted")
                && tree_cursor_on(screen, "README.md")
                && !screen.contains("VISUAL")
        },
        "empty Enter deletes the covering range (not a no-op single-line key)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    let stored = store_text(&workspace);
    assert!(
        !stored.contains(BODY) && !stored.contains("\"endLine\": 2"),
        "empty submit must drop the 1-2 range comment:\n{stored}"
    );
}

/// Watch reload of the focused file must drop visual-line highlight.
/// `;` after that save is one numbered line, not a stale row-index span.
///
/// `WS_STATUS_WATCH_MS=500`. CSI-u Shift+V / j / Kitty Down / `;`.
#[test]
fn pty_watch_reload_clears_visual_so_semicolon_is_not_stale_span() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", "500")]);
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

    highlight_readme_range(&mut tui);

    // Git and the watch disk token key off mtime. Sleep past one second so
    // the rewrite is a new token, not the same-second size bump.
    std::thread::sleep(Duration::from_millis(1100));
    fs::write(
        workspace.join("app").join("README.md"),
        format!("# app\ndirty\n{WATCH_EXTRA}\n"),
    )
    .unwrap();
    let marker = format!(
        "watch-visual-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    fs::write(workspace.join("app").join(&marker), "force-watch\n").unwrap();

    tui.wait_pred(
        |screen| tree_has(screen, &marker) && screen.contains(WATCH_EXTRA),
        "watch paints the extra README line (new dirty path forces the poll)",
        GIT_WAIT,
    );
    tui.wait_pred(
        |screen| {
            right_diff_focused_allow_visual(screen)
                && screen.contains(WATCH_EXTRA)
                && !screen.contains("VISUAL")
                && !comment_overlay(screen)
        },
        "painted-row change drops VISUAL (no r)",
        WAIT,
    );

    csi_u_semicolon(&mut tui);
    tui.wait_pred(
        |screen| {
            comment_overlay(screen)
                && !screen.contains("README.md:1-2")
                && !screen.contains("README.md:1-3")
                && !screen.contains("MOVE")
        },
        "; after watch opens a single-line overlay, not a stale range",
        WAIT,
    );
    tui.keys(WATCH_BODY);
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains(WATCH_BODY),
        "typed body appears in the single-line overlay",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the post-watch line comment",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(WATCH_BODY)
            && stored.contains("worktreeLine")
            && !stored.contains("\"endLine\""),
        "store must be one line, not a stale visual span:\n{stored}"
    );
}

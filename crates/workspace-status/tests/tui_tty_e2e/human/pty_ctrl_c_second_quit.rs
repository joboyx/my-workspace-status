use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{screen_line_from_end, send_gesture, tree_has, GIT_WAIT, WAIT};

/// Pinned chrome copy after the first Ctrl+C (`tui/ctrl_c_exit.rs`).
const CTRL_C_EXIT_PROMPT: &str = "Press Ctrl-c again to exit";

/// Idle daily seed: tree + status pills, breadcrumb on the penultimate row.
///
/// No quit prompt yet. Status is last. A help overlay cannot pass.
fn idle_tree_before_ctrl_c(screen: &str) -> bool {
    let status = screen_line_from_end(screen, 0);
    let crumb = screen_line_from_end(screen, 1);
    tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && status.contains(" tree")
        && status.contains("? help")
        && status.contains("focus right")
        && crumb.trim() == "workspace › app"
        && !crumb.contains("Ctrl-c")
        && !status.contains(CTRL_C_EXIT_PROMPT)
        && !screen.contains(CTRL_C_EXIT_PROMPT)
        && !screen.contains("MOVE")
}

/// Second Ctrl+C within the window quits.
///
/// Docs + VIEW: `Ctrl-c Ctrl-c` / `quit (press twice)`. The first-press
/// test (`pty_ctrl_c_prompts_before_quit`) owns the first-press arm: the
/// prompt pins and the process keeps running. Help overlay lists the row
/// (`pty_help_overlay`). This claim is process exit on the second press.
///
/// Encoding: CSI-u Control+c (`CSI 99 ; 5 : 1 u` press, `: 3` release)
/// for both presses, sent in one write. The live loop requested
/// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus event types. C0 `\x03`
/// (`PtySession::ctrl`) is a different path. `q` is help `quit`, not
/// this chord.
///
/// Documented result: the second press within 2 s of the first (by
/// arrival) exits with status 0. Fail if the second press is a no-op, if
/// the prompt stays armed, or if nothing happens. Do not teardown with
/// `q`.
#[test]
fn pty_ctrl_c_second_quit() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_tree_before_ctrl_c,
        "first paint: tree + status pills, no quit prompt",
        WAIT,
    );
    tui.assert_running("before first Ctrl+C");

    // Both presses in one write: they arrive together, well inside the
    // 2 s window, however late the loop handles them.
    let tap = "\x1b[99;5:1u\x1b[99;5:3u";
    send_gesture(&mut tui, &[tap, tap]);
    tui.wait_exit(WAIT);
}

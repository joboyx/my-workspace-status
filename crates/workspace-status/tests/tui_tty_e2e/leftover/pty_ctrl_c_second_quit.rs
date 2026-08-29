use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{screen_line_from_end, tree_has, GIT_WAIT, WAIT};

/// Pinned chrome copy after the first Ctrl+C (`tui/ctrl_c_exit.rs`).
const CTRL_C_EXIT_PROMPT: &str = "Press Ctrl+C again to exit";

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
        && crumb.trim() == "workspace"
        && !crumb.contains("Ctrl+C")
        && !status.contains(CTRL_C_EXIT_PROMPT)
        && !screen.contains(CTRL_C_EXIT_PROMPT)
        && !screen.contains("MOVE")
}

/// First Ctrl+C pins the quit prompt between breadcrumb and status pills.
///
/// Fail if the copy is only a breadcrumb toast, if status pills vanish, or
/// if the tree is gone. The process-alive check sits on the caller.
fn first_ctrl_c_pinned_prompt(screen: &str) -> bool {
    let status = screen_line_from_end(screen, 0);
    let prompt = screen_line_from_end(screen, 1);
    let crumb = screen_line_from_end(screen, 2);
    prompt.trim() == CTRL_C_EXIT_PROMPT
        && crumb.trim() == "workspace"
        && !crumb.contains("Ctrl+C")
        && status.contains(" tree")
        && status.contains("? help")
        && status.contains("focus right")
        && !status.contains(CTRL_C_EXIT_PROMPT)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && !screen.contains("MOVE")
}

/// Second Ctrl+C within the window quits.
///
/// Docs + VIEW: `Ctrl-C Ctrl-C` / `quit (press twice)`. First leftover
/// (`pty_ctrl_c_prompts_before_quit`) owns the first-press arm. Help
/// overlay lists the row (`pty_help_overlay`). This claim is process
/// exit after that prompt is armed.
///
/// Encoding: CSI-u Control+c (`CSI 99 ; 5 : 1 u` press, `: 3` release)
/// for both presses. The live loop requested
/// `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus event types. C0 `\x03`
/// (`PtySession::ctrl`) is a different path. `q` is help `quit`, not
/// this chord.
///
/// Documented result: first press pins `Press Ctrl+C again to exit` and
/// keeps the process. Second press within ~2s exits with status 0.
/// Fail if the first press already quits, if the second is a no-op, if
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

    tui.ctrl_letter('c');
    tui.wait_pred(
        first_ctrl_c_pinned_prompt,
        "first CSI-u Ctrl+C pins Press Ctrl+C again to exit between breadcrumb and status",
        WAIT,
    );
    tui.assert_running("after first Ctrl+C (must not quit)");

    tui.ctrl_letter('c');
    tui.wait_exit(WAIT);
}

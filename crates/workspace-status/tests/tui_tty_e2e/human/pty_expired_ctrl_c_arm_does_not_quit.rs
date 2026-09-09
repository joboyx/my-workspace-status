use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{screen_line_from_end, tree_has, GIT_WAIT, WAIT};

/// Pinned chrome copy after the first Ctrl+C (`tui/ctrl_c_exit.rs`).
const CTRL_C_EXIT_PROMPT: &str = "Press Ctrl+C again to exit";

/// Confirm-exit window from `tui/ctrl_c_exit.rs` (`CTRL_C_EXIT_MS`).
const CTRL_C_EXIT_MS: u64 = 2000;

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

/// Expired Ctrl+C arm does not quit; a late press re-arms.
///
/// Docs + VIEW: `Ctrl-C Ctrl-C` / `quit (press twice)`. The first-press
/// test (`pty_ctrl_c_prompts_before_quit`) owns the first-press arm.
/// Second press within the window is `pty_ctrl_c_second_quit`. This claim
/// is live-loop expiry: after the window the pinned prompt clears, the
/// process stays, and a late Ctrl+C re-arms instead of quitting.
///
/// Encoding: CSI-u Control+c (`CSI 99 ; 5 : 1 u` press, `: 3` release).
/// The live loop requested `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus event
/// types. C0 `\x03` (`PtySession::ctrl`) is a different path.
///
/// The live loop sleeps `ctrl_remain` then calls
/// `AppState::expire_ctrl_c_prompt` (`tui/event_loop.rs`). This test
/// waits past `CTRL_C_EXIT_MS` (2000) with no keys. It does not inject
/// expiry or a key Release.
///
/// Documented result: first press pins `Press Ctrl+C again to exit` and
/// keeps the process. After the window, idle chrome returns (breadcrumb
/// `workspace` on the penultimate row, no pinned prompt). Late Ctrl+C
/// pins the prompt again and keeps the process. Fail if the first press
/// already quits, if the prompt never clears, if expiry quits, if the
/// late press quits, if re-arm is a no-op, or if a help overlay (`MOVE`)
/// passes as idle. Teardown sends `q` (second-within-window Ctrl+C is
/// not claimed).
#[test]
fn pty_expired_ctrl_c_arm_does_not_quit() {
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

    tui.wait_ms(CTRL_C_EXIT_MS + 200);
    tui.wait_pred(
        idle_tree_before_ctrl_c,
        "after CTRL_C_EXIT_MS the pinned quit prompt is gone; idle chrome returns",
        WAIT,
    );
    tui.assert_running("after Ctrl+C arm expired (must not quit)");

    tui.ctrl_letter('c');
    tui.wait_pred(
        first_ctrl_c_pinned_prompt,
        "late CSI-u Ctrl+C re-arms Press Ctrl+C again to exit; must not quit",
        WAIT,
    );
    tui.assert_running("after late Ctrl+C (must re-arm, not quit)");

    tui.key('q');
    tui.wait_exit(WAIT);
}

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_has, WAIT};

/// Idle first paint: tree + file chrome, no Ctrl+C quit prompt.
///
/// `q` is help `quit`, not `quit (press twice)`. The status chip may
/// truncate (`…`) before `q` / `quit`, so this checks mounted TUI chrome
/// rather than the truncated hint.
fn idle_tui_ready_for_q(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && screen.contains(" tree")
        && screen.contains("? help")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && !screen.contains("Press Ctrl+C again to exit")
        && !screen.contains("MOVE")
}

/// `q` quits immediately (help `q` quit, not the Ctrl+C chord).
///
/// Docs: process exits. Help/keymap: `q` / "quit" — not "press twice"
/// (`Ctrl-C Ctrl-C`). A no-op, a Ctrl+C arm, a twice-to-quit prompt, a
/// crash, or a still-alive process with painted chrome cannot pass.
/// `wait_exit` alone is not enough: the Ctrl+C prompt must never paint.
#[test]
fn pty_q_quits_immediately() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        idle_tui_ready_for_q,
        "first paint: tree + file chrome, no Ctrl+C quit prompt",
        WAIT,
    );

    tui.key('q');
    tui.wait_exit_without("Press Ctrl+C again to exit", WAIT);
}

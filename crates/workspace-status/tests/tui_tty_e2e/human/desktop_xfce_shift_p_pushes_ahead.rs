#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::harness::left_tree;
use crate::seed::ahead_workspace;
use crate::support::{op_finished, tree_cleared_ahead_behind, GIT_WAIT, SETTLE_MS, WAIT};

#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_shift_p_pushes_ahead() {
    let (_root, workspace) = ahead_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.key("slash");
    tui.type_text("syncbox");
    tui.key("Return");
    tui.wait_contains("/syncbox", WAIT);
    tui.wait_contains("ahead-tip-commit", WAIT);
    tui.wait_pred(
        |screen| left_tree(screen).contains("^1"),
        "tree shows ahead-by-1 before push",
        WAIT,
    );
    tui.wait_contains("push", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("shift+p");
    tui.wait_pred(
        |screen| op_finished(screen, "Pushed"),
        "Pushed 1 repo without failure",
        GIT_WAIT,
    );
    tui.wait_pred(
        tree_cleared_ahead_behind,
        "ahead mark cleared after push",
        WAIT,
    );
}

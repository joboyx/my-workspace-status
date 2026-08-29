#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::seed::focus_workspace;
use crate::support::WAIT;

/// xfce + XTEST Shift keys: search capitals, unmark-then-Enter, Shift+O.
///
/// Overlay toggle is space (`[x]` / `[ ]`), not X. Reopen after `O`
/// has no pre-mark; unmark-then-Enter runs while a focus is still on.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_shift_keys_search_and_clear_focus() {
    let (_root, workspace) = focus_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.key("slash");
    tui.key("shift+r");
    tui.key("shift+e");
    tui.key("shift+a");
    tui.key("shift+d");
    tui.key("shift+m");
    tui.key("shift+e");
    tui.wait_contains("README▏", WAIT);
    tui.key("Escape");

    tui.key("slash");
    tui.type_text("focusbox");
    tui.key("Return");
    tui.wait_contains("focusbox", WAIT);
    tui.key("Tab");
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_contains("noise-leaf-commit", WAIT);
    tui.key("o");
    tui.wait_contains("Focus branches", WAIT);
    tui.type_text("keep");
    tui.key("Return");
    tui.wait_pred(
        |screen| !screen.contains("Focus branches"),
        "focus overlay closed after apply",
        WAIT,
    );
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_pred(
        |screen| !screen.contains("noise-leaf-commit"),
        "noise-leaf-commit hidden after focus",
        WAIT,
    );

    tui.key("o");
    tui.wait_contains("Focus branches", WAIT);
    tui.wait_contains("[x]", WAIT);
    tui.key("space");
    tui.wait_pred(
        |screen| !screen.contains("[x]"),
        "[x] mark cleared after space",
        WAIT,
    );
    tui.key("Return");
    tui.wait_pred(
        |screen| !screen.contains("Focus branches"),
        "focus overlay closed after empty apply",
        WAIT,
    );
    tui.wait_contains("noise-leaf-commit", WAIT);
    tui.wait_contains("main-leaf-commit", WAIT);

    tui.key("o");
    tui.wait_contains("Focus branches", WAIT);
    tui.type_text("keep");
    tui.key("Return");
    tui.wait_pred(
        |screen| !screen.contains("noise-leaf-commit"),
        "noise-leaf-commit hidden before Shift+O",
        WAIT,
    );
    tui.key("shift+o");
    tui.wait_contains("noise-leaf-commit", WAIT);
}

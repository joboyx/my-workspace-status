#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::harness::left_tree;
use crate::seed::daily_workspace;
use crate::support::{GIT_WAIT, SETTLE_MS, WAIT};

#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_stash_create_apply_and_drop() {
    let (_root, workspace) = daily_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.key("slash");
    tui.type_text("README");
    tui.key("Return");
    tui.wait_contains("/README", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("shift+s");
    tui.wait_contains("s create", WAIT);
    tui.key("s");
    tui.wait_contains("Stashed", GIT_WAIT);
    tui.wait_pred(
        |screen| !left_tree(screen).contains("README.md"),
        "stashed README leaves the dirty tree",
        WAIT,
    );
    tui.key("Escape");
    tui.key("slash");
    tui.type_text("app");
    tui.key("Return");
    tui.wait_contains("/app", WAIT);
    tui.key("Tab");
    tui.wait_contains("Working tree", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("slash");
    tui.type_text("stash@{");
    tui.key("Return");
    tui.wait_contains("/stash@{", WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("a");
    tui.wait_contains("README.md", GIT_WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("shift+d");
    tui.wait_contains("Drop", WAIT);
    tui.key("y");
    tui.wait_contains("dropped stash@{0}", GIT_WAIT);
}

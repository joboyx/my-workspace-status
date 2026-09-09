#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_keys_help_and_search() {
    let (_root, workspace) = daily_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.key("shift+slash");
    tui.wait_contains("MOVE", WAIT);
    tui.wait_contains("GIT", WAIT);
    tui.wait_contains("VIEW", WAIT);
    tui.key("Escape");
    tui.key("slash");
    tui.type_text("merger");
    tui.key("Return");
    tui.wait_contains("merger", WAIT);
    tui.wait_contains("WIP on graph", WAIT);
}

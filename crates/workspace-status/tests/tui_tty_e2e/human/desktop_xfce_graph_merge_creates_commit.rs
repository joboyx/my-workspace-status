#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::seed::focus_workspace;
use crate::support::{GIT_WAIT, SETTLE_MS, WAIT};

#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_graph_merge_creates_commit() {
    let (_root, workspace) = focus_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.key("slash");
    tui.type_text("focusbox");
    tui.key("Return");
    tui.wait_contains("/focusbox", WAIT);
    tui.key("Tab");
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_contains("main-leaf-commit", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("slash");
    tui.type_text("main-leaf-commit");
    tui.key("Return");
    tui.wait_contains("/main-leaf-commit", WAIT);
    tui.wait_contains("merge", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("m");
    tui.wait_contains("Merge", WAIT);
    tui.wait_contains("into", WAIT);
    tui.key("y");
    tui.wait_contains("Merge branch 'main'", GIT_WAIT);
}

#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::seed::daily_workspace;
use crate::support::{GIT_WAIT, SETTLE_MS, WAIT};

#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_stash_graph_pop() {
    let (_root, workspace) = daily_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.key("slash");
    tui.wait_contains("SEARCH", WAIT);
    tui.type_text("merger");
    // Typing jumps to merger and starts graph git. Wait for that paint
    // before Return: xfce can drop Enter while the pane worker runs.
    tui.wait_pred(
        |screen| {
            screen.contains("SEARCH")
                && screen.contains("Enter arms query")
                && screen.contains("merger▏")
                && screen.contains("WIP on graph")
                && !screen.contains("/merger")
        },
        "SEARCH typing on merger after the graph jump",
        GIT_WAIT,
    );
    tui.key("Return");
    tui.wait_pred(
        |screen| screen.contains("/merger") && !screen.contains("SEARCH"),
        "Enter arms /merger; SEARCH closes",
        WAIT,
    );
    tui.key("Tab");
    tui.wait_contains("WIP on graph", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("slash");
    tui.wait_contains("SEARCH", WAIT);
    tui.type_text("stash@{");
    tui.wait_pred(
        |screen| {
            screen.contains("SEARCH")
                && screen.contains("Enter arms query")
                && screen.contains("stash@{▏")
        },
        "SEARCH typing stash@{",
        WAIT,
    );
    tui.key("Return");
    tui.wait_pred(
        |screen| screen.contains("/stash@{") && !screen.contains("SEARCH"),
        "Enter arms /stash@{",
        WAIT,
    );
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("p");
    tui.wait_contains("wip.txt", GIT_WAIT);
    tui.wait_contains("popped stash@{0}", GIT_WAIT);
}

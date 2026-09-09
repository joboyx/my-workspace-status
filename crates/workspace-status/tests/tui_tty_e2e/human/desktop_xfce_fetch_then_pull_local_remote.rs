#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::harness::left_tree;
use crate::seed::unfetched_behind_workspace;
use crate::support::{op_finished, tree_cleared_ahead_behind, GIT_WAIT, SETTLE_MS, WAIT};

#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_fetch_then_pull_local_remote() {
    let (_root, workspace) = unfetched_behind_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.key("slash");
    tui.type_text("syncbox");
    tui.key("Return");
    tui.wait_contains("/syncbox", WAIT);
    tui.wait_contains("Working tree", WAIT);
    tui.wait_contains("fetch", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("f");
    tui.wait_pred(
        |screen| op_finished(screen, "Fetched") || left_tree(screen).contains("v1"),
        "Fetched 1 repo or tree shows behind-by-1",
        GIT_WAIT,
    );
    tui.wait_pred(
        |screen| left_tree(screen).contains("v1"),
        "tree shows behind-by-1 after fetch",
        WAIT,
    );
    tui.wait_contains("origin-tip-commit", WAIT);
    tui.wait_contains("pull", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key("p");
    tui.wait_pred(
        |screen| op_finished(screen, "Pulled"),
        "Pulled 1 repo without failure",
        GIT_WAIT,
    );
    tui.wait_pred(
        tree_cleared_ahead_behind,
        "behind mark cleared after pull",
        WAIT,
    );
    tui.key("Tab");
    tui.wait_contains("origin-tip-commit", GIT_WAIT);
}

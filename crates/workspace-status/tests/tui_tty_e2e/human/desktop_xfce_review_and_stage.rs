#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_space_reviewed, idle_dirty_readme_unreviewed, tree_line_containing, GIT_WAIT, WAIT,
};

/// Space reviewed on first-paint dirty README. `s`/`u` stay a separate
/// claim; this arm only strengthens the Space-reviewed part.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_review_and_stage() {
    let (_root, workspace) = daily_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "first paint: cursor on dirty README, no reviewed mark",
        WAIT,
    );
    tui.key("space");
    tui.wait_pred(
        documented_space_reviewed,
        "Space paints ASCII `*` on the focused README row; file stays; not staged",
        WAIT,
    );
    tui.key("s");
    tui.wait_contains("STAGED", GIT_WAIT);
    tui.wait_absent("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| tree_line_containing(screen, "README.md").is_some_and(|line| line.contains("S ")),
        "staged README badge `S `",
        WAIT,
    );
    tui.key("u");
    tui.wait_contains("UNSTAGED", GIT_WAIT);
}

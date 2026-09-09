use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{GIT_WAIT, WAIT};

fn tab_close_hit(screen: &str) -> Option<(u16, u16)> {
    for (row, line) in screen.lines().enumerate() {
        if !line.contains("app · vs") {
            continue;
        }
        if let Some(at) = line.find("[x]") {
            let col = line[..at].chars().count() as u16 + 1;
            return Some((col, row as u16));
        }
    }
    None
}

fn workspace_tab_has_close(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains("Workspace") && line.contains("[x]") && !line.contains("app · vs")
    })
}

/// SGR click on compare `[x]` closes that tab. Workspace has no close hit.
#[test]
fn pty_compare_mouse_tab_close() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app · vs origin/main", GIT_WAIT);

    let screen = tui.screen();
    assert!(
        !workspace_tab_has_close(&screen),
        "Workspace must not paint a close control:\n{screen}"
    );
    let (col, row) = tab_close_hit(&screen).expect("compare [x]");
    tui.sgr_click(col, row);
    tui.wait_pred(
        |screen| {
            screen.contains("# workspace")
                && !screen.contains("app · vs origin/main")
                && !screen.contains("COMMITTED")
        },
        "click [x] closes the compare tab",
        WAIT,
    );
}

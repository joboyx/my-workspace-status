use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{GIT_WAIT, WAIT};

fn tab_hit(screen: &str, needle: &str) -> Option<(u16, u16)> {
    for (row, line) in screen.lines().enumerate() {
        if let Some(at) = line.find(needle) {
            let col = line[..at].chars().count() as u16 + 2;
            return Some((col, row as u16));
        }
    }
    None
}

/// SGR click on the tab strip switches tabs. There is no mouse close.
#[test]
fn pty_compare_mouse_tab_click() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app · vs origin/main", GIT_WAIT);

    let screen = tui.screen();
    let (ws_col, ws_row) = tab_hit(&screen, "Workspace").expect("Workspace tab");
    tui.sgr_click(ws_col, ws_row);
    tui.wait_pred(
        |screen| screen.contains("# workspace") && !screen.contains("COMMITTED"),
        "click Workspace activates that tab",
        WAIT,
    );

    let screen = tui.screen();
    let (cmp_col, cmp_row) = tab_hit(&screen, "app · vs").expect("compare tab");
    tui.sgr_click(cmp_col, cmp_row);
    tui.wait_pred(
        |screen| screen.contains("COMMITTED") && screen.contains("app · vs origin/main"),
        "click compare tab activates it",
        GIT_WAIT,
    );
}

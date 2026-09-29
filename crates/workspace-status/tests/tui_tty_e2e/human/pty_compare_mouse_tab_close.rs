use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{GIT_WAIT, TAB_CLOSE, WAIT};

fn tab_close_hit(screen: &str) -> Option<(u16, u16)> {
    for (row, line) in screen.lines().enumerate() {
        if !line.contains("app ↔") {
            continue;
        }
        if let Some(at) = line.find(TAB_CLOSE) {
            let col = line[..at].chars().count() as u16 + 1;
            return Some((col, row as u16));
        }
    }
    None
}

fn workspace_tab_has_close(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains("Workspace") && line.contains(TAB_CLOSE) && !line.contains("app ↔")
    })
}

/// SGR click on compare `[𝔁]` closes that tab. Workspace has no close hit.
#[test]
fn pty_compare_mouse_tab_close() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app ↔ origin/main", GIT_WAIT);

    let screen = tui.screen();
    assert!(
        !workspace_tab_has_close(&screen),
        "Workspace must not paint a close control:\n{screen}"
    );
    let (col, row) = tab_close_hit(&screen).expect("compare close control");
    tui.sgr_click(col, row);
    tui.wait_pred(
        |screen| {
            screen.contains("# workspace")
                && !screen.contains("app ↔ origin/main")
                && !screen.contains("COMMITTED")
        },
        "click [𝔁] closes the compare tab",
        WAIT,
    );
}

/// Tab-strip row: the line that holds the Workspace tab and a compare tab.
fn strip_row(screen: &str) -> Option<(u16, String)> {
    screen
        .lines()
        .enumerate()
        .find(|(_, line)| line.contains("Workspace") && line.contains("app ↔"))
        .map(|(row, line)| (row as u16, line.to_string()))
}

/// SGR click on the painted `x` of the second compare tab closes that tab.
///
/// Each label holds `↔`, which paints one column. The hit boxes must not
/// drift right of the painted `[𝔁]` as labels accumulate.
#[test]
fn pty_compare_mouse_second_tab_close() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.wait_contains("Diff vs default", WAIT);
    tui.enter();
    tui.wait_contains("app ↔ origin/main", GIT_WAIT);
    tui.ctrl_letter('k');
    tui.keys("vs branch");
    tui.enter();
    tui.wait_contains("Compare", WAIT);
    tui.keys("main");
    tui.enter();
    tui.wait_pred(
        |screen| {
            strip_row(screen).is_some_and(|(_, line)| {
                line.contains("app ↔ main") && line.matches(TAB_CLOSE).count() == 2
            })
        },
        "two compare tabs, each with [𝔁]",
        GIT_WAIT,
    );

    let (row, line) = strip_row(&tui.screen()).expect("tab strip");
    let second = line.rfind(TAB_CLOSE).expect("second [𝔁]");
    assert!(line[..second].contains("app ↔ main"), "{line}");
    let col = line[..second].chars().count() as u16 + 1;
    tui.sgr_click(col, row);
    tui.wait_pred(
        |screen| {
            strip_row(screen).is_some_and(|(_, line)| {
                line.contains("app ↔ origin/main")
                    && !line.contains("app ↔ main")
                    && line.matches(TAB_CLOSE).count() == 1
            })
        },
        "click on the painted x of the second tab closes that tab only",
        WAIT,
    );
}

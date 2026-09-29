use std::time::{Duration, Instant};

use crate::harness::PtySession;
use crate::seed::compare_ahead_workspace;
use crate::support::{GIT_WAIT, WAIT};

/// SGR pointer motion with no button held (`3 | 32`, any-event tracking).
const SGR_POINTER_MOVE: u8 = 3 | 32;
/// Tokyo Night `tab_close` (`#737aa2`): idle `[x]`, dimmer than `muted`.
const IDLE_FG: (u8, u8, u8) = (0x73, 0x7a, 0xa2);
/// Tokyo Night `tab_close_hover` (`#c0caf5`): hovered `[x]`.
const HOVER_FG: (u8, u8, u8) = (0xc0, 0xca, 0xf5);

/// 0-based cell of the `x` in the compare tab `[x]`.
fn tab_close_x(screen: &str) -> Option<(u16, u16)> {
    for (row, line) in screen.lines().enumerate() {
        if !line.contains("app ↔") {
            continue;
        }
        if let Some(at) = line.find("[x]") {
            let col = line[..at].chars().count() as u16 + 1;
            return Some((col, row as u16));
        }
    }
    None
}

fn wait_close_fg(tui: &PtySession, want: (u8, u8, u8), what: &str) {
    let deadline = Instant::now() + WAIT;
    loop {
        let fgs = tui.first_needle_fgs("[x]");
        if fgs
            .as_ref()
            .is_some_and(|fgs| fgs.len() == 3 && fgs.iter().all(|fg| *fg == Some(want)))
        {
            return;
        }
        if Instant::now() >= deadline {
            panic!("{what}: [x] fgs {fgs:?}, want {want:?}\n{}", tui.screen());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Buttonless SGR motion over the compare `[x]` brightens it; motion off
/// the `[x]` dims it again. Motion never closes or switches the tab.
#[test]
fn pty_compare_mouse_tab_close_hover() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_THEME", "tokyo-night")]);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.enter();
    tui.wait_contains("app ↔ origin/main", GIT_WAIT);
    wait_close_fg(&tui, IDLE_FG, "idle [x] paints tab_close");

    let (col, row) = tab_close_x(&tui.screen()).expect("compare [x]");
    tui.sgr_mouse(SGR_POINTER_MOVE, col, row);
    wait_close_fg(&tui, HOVER_FG, "motion over [x] paints tab_close_hover");

    tui.sgr_mouse(SGR_POINTER_MOVE, 2, row);
    wait_close_fg(&tui, IDLE_FG, "motion off [x] restores tab_close");
    assert!(
        tui.screen().contains("app ↔ origin/main"),
        "motion must not close the compare tab:\n{}",
        tui.screen()
    );
}

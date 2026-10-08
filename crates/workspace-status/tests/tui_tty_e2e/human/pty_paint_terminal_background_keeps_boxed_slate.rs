use std::fs;

use crate::harness::{user_config_file, PtySession, UserConfig, COLS, ROWS};
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_top, SLATE_CHROME, SLATE_HEADING, SLATE_SIDEBAR,
    SLATE_SURFACE, WAIT,
};

const SLATE_TERMINAL_CONFIG: &str =
    "{\"theme\":\"slate\",\"viewDefaults\":{\"background\":\"terminal\"}}\n";

/// `background: "terminal"` keeps the boxed panes, also on Slate.
///
/// Docs: `viewDefaults.background` `"terminal"` draws panes, tab strip,
/// breadcrumb and footer as before paint mode: boxed panes, no fills. The
/// theme still colours text and borders.
///
/// Live PTY with a test-owned config (`slate` + `terminal`): the documented
/// launch frame paints with a `┐┌` / `││` box join and a `─` title rule. The
/// focused tree border is Slate `heading`. No pane, tab strip, breadcrumb or
/// status cell carries a Slate sidebar / surface / chrome fill.
#[test]
fn pty_paint_terminal_background_keeps_boxed_slate() {
    let (_root, workspace) = daily_workspace();
    let file = user_config_file(&workspace.join(".e2e-config"));
    fs::create_dir_all(file.parent().unwrap()).unwrap();
    fs::write(&file, SLATE_TERMINAL_CONFIG).unwrap();
    let tui = PtySession::open_size_with_config(&workspace, COLS, ROWS, &[], UserConfig::Own);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: boxed tree / diff panes on the terminal background",
        WAIT,
    );
    let (r, g, b) = SLATE_HEADING;
    tui.wait_has_rgb(r, g, b, WAIT);

    let screen = tui.screen();
    let top = pane_top(&screen);
    assert!(
        top.contains('─') && (top.contains("┐┌") || screen.contains("││")),
        "terminal background keeps boxed panes:\n{screen}"
    );
    assert_eq!(
        tui.cell_paint(1, 0).and_then(|cell| cell.glyph),
        Some('┌'),
        "the tree box starts at column 0:\n{screen}"
    );
    assert_eq!(
        tui.cell_paint(1, 0).and_then(|cell| cell.fg),
        Some(SLATE_HEADING),
        "focused tree border is Slate heading:\n{screen}"
    );
    for (row, col, what) in [
        (0, COLS - 1, "tab strip"),
        (ROWS - 3, 1, "tree body"),
        (ROWS - 3, COLS - 2, "right body"),
        (ROWS - 2, COLS - 1, "breadcrumb row"),
        (ROWS - 1, COLS - 1, "status row"),
    ] {
        let bg = tui.cell_paint(row, col).and_then(|cell| cell.bg);
        assert!(
            bg != Some(SLATE_SIDEBAR) && bg != Some(SLATE_SURFACE) && bg != Some(SLATE_CHROME),
            "{what} cell ({row},{col}) has no Slate fill, got {bg:?}:\n{screen}"
        );
    }
}

use crate::harness::{
    self, tree_cursor_bar_on_row, tree_is_panned_to_tail, tree_row_containing, PtySession,
    SGR_WHEEL_DOWN, SGR_WHEEL_RIGHT,
};
use crate::seed::{daily_workspace, seed_long_path_file};
use crate::support::{tree_cursor_on, GIT_WAIT, SETTLE_MS, TREE_LABEL_COL, WAIT};

/// Tree `m` toggles mouse reporting. Not graph merge.
///
/// Docs / keymap / help: mouse is on by default. Tree `m` (raw byte)
/// flips capture and paints `Mouse off` / `Mouse on`. Off ignores click,
/// drag, and wheel. On accepts them. A focused graph commit would confirm
/// merge instead.
///
/// Default-on click must select `merger`. After `m`, click, vertical
/// wheel (`Cb` 65), and trackpad hscroll (`Cb` 67) are ignored. After
/// the second `m`, click selects README, hscroll pans the clipped tree
/// row, and vertical wheel moves the tree cursor. Toast-only, click-only,
/// or a no-op is red.
#[test]
fn pty_m_toggles_mouse_capture() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("README.md")
                && !screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && !screen.contains("fast-forward if possible")
        },
        "launch paints the tree; mouse status and merge confirm are absent",
        GIT_WAIT,
    );
    let _ = tui.wait_clipped_long_path_row(WAIT);

    // The untracked long path is the first file. Click README so the
    // default-on mouse path both selects and loads a short diff (hscroll
    // over the tree then pans the tree, not a long file-diff).
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_row);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md") && screen.contains("UNSTAGED"),
        "default mouse-on SGR click selects README (a default-off or dead mouse never loads that pane)",
        GIT_WAIT,
    );

    let merger_row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, merger_row);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("UNSTAGED")
                && (screen.contains("Working tree") || screen.contains("WIP on graph"))
        },
        "default mouse-on SGR click selects merger (a default-off or dead mouse never loads that pane)",
        GIT_WAIT,
    );

    tui.key('m');
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && tree_cursor_on(screen, "merger")
                && !screen.contains("fast-forward if possible")
                && !screen.contains("UNSTAGED")
        },
        "tree `m` paints Mouse off and does not open merge confirm",
        WAIT,
    );

    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row while mouse off:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_row);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("UNSTAGED")
                && (screen.contains("Working tree") || screen.contains("WIP on graph"))
        },
        "SGR click is ignored while Mouse off (toast-only would select README)",
        WAIT,
    );

    let merger_row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row while mouse off:\n{}", tui.screen()));
    tui.sgr_mouse(SGR_WHEEL_DOWN, TREE_LABEL_COL, merger_row);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("UNSTAGED")
        },
        "vertical wheel is ignored while Mouse off (ungated wheel would leave merger)",
        WAIT,
    );

    let hscroll_row = tui.wait_clipped_long_path_row(WAIT);
    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, hscroll_row);
    }
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && harness::clipped_long_path_row(screen).is_some()
                && tree_cursor_on(screen, "merger")
        },
        "SGR hscroll is ignored while Mouse off (ungated wheel would pan the tree)",
        WAIT,
    );

    tui.key('m');
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse on")
                && !screen.contains("Mouse off")
                && tree_cursor_on(screen, "merger")
                && !screen.contains("UNSTAGED")
        },
        "second tree `m` paints Mouse on",
        WAIT,
    );
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row after Mouse on:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_row);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "merger")
                && screen.contains("UNSTAGED")
        },
        "SGR click selects README after Mouse on (status-only would leave merger focused)",
        GIT_WAIT,
    );

    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row before hscroll on:\n{}", tui.screen()));
    let hscroll_row = tui.wait_clipped_long_path_row(WAIT);
    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, hscroll_row);
    }
    tui.wait_pred(
        |screen| tree_is_panned_to_tail(screen) && tree_cursor_bar_on_row(screen, readme_row),
        "SGR hscroll pans the tree after Mouse on and does not steal the README cursor",
        WAIT,
    );

    tui.sgr_mouse(SGR_WHEEL_DOWN, TREE_LABEL_COL, readme_row);
    tui.wait_pred(
        |screen| !tree_cursor_bar_on_row(screen, readme_row),
        "vertical wheel moves the tree cursor after Mouse on (gate-only would stay on README)",
        GIT_WAIT,
    );
}

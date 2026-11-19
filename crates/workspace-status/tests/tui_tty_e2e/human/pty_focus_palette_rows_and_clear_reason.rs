use crate::harness::PtySession;
use crate::seed::tree_focus_workspace;
use crate::support::{
    open_command_palette, tree_focus_first_paint, type_palette_filter, wait_screen, GIT_WAIT, WAIT,
};

/// The four focus rows, in palette order.
const FOCUS_ROWS: [&str; 4] = [
    "Focus: this feature",
    "Focus: this repo",
    "Focus: type a query…",
    "Focus: clear",
];

/// Screen row of the first line that contains `title`.
fn row_of(screen: &str, title: &str) -> Option<usize> {
    screen.lines().position(|line| line.contains(title))
}

/// `:` filter `focus` lists the four focus rows in order, and `Focus: clear`
/// with no focus set is disabled with the reason `no focus set`.
///
/// The launch cursor is on `notes/README.md` (`main`), so `Focus: this
/// feature` also shows `no feature on the default branch`.
#[test]
fn pty_focus_palette_rows_and_clear_reason() {
    let (_root, workspace) = tree_focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        tree_focus_first_paint,
        "first paint: every repo shows, no focus chip",
        GIT_WAIT,
    );

    open_command_palette(&mut tui);
    type_palette_filter(&mut tui, "focus");
    let screen = wait_screen(
        &tui,
        |screen| {
            screen.contains("Enter run") && FOCUS_ROWS.iter().all(|title| screen.contains(title))
        },
        "palette filter `focus` lists the four Focus rows",
        WAIT,
    );
    let rows: Vec<usize> = FOCUS_ROWS
        .iter()
        .map(|title| row_of(&screen, title).unwrap())
        .collect();
    assert!(
        rows.windows(2).all(|pair| pair[0] < pair[1]),
        "Focus rows must paint in the order {FOCUS_ROWS:?} (rows {rows:?}):\n{screen}"
    );
    let line = |title: &str| screen.lines().nth(row_of(&screen, title).unwrap()).unwrap();
    assert!(
        line("Focus: clear").contains("no focus set"),
        "`Focus: clear` with no focus shows `no focus set`:\n{screen}"
    );
    assert!(
        line("Focus: this feature").contains("no feature on the default branch"),
        "`Focus: this feature` on a main checkout shows `no feature on the default branch`:\n{screen}"
    );
}

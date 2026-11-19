use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{join_at_width, wait_join_at_width, WAIT};

/// Wait until the frame `cols` wide joins the panes at `join` (a tree of
/// `join + 1` columns).
fn wait_join(tui: &PtySession, cols: u16, join: u16, what: &str) {
    tui.wait_pred(
        |screen| join_at_width(screen, cols) == Some(join),
        what,
        WAIT,
    );
}

/// The tree pane keeps a remembered column width through terminal
/// resizes, and `<` / `>` set it.
///
/// A 160-column launch fixes a 64-column tree (join at column 63). A
/// wider terminal gives its extra columns to the right pane. `>` at 220
/// columns adds round(5% of 220) = 11 columns. An 80-column terminal
/// clamps the painted tree to 80 − 22 = 58 columns so the right pane
/// keeps its minimum, and the 75 columns come back at 220. `<` takes the
/// 11 columns away again. Fail if a resize moves the join back to a
/// share of the terminal width.
#[test]
fn pty_tree_keeps_columns_on_resize() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size(&workspace, 160, 40);
    let launch = wait_join_at_width(&tui, 160, "160-col first paint with a tree/right join");
    assert_eq!(launch, 63, "a 64-col launch tree:\n{}", tui.screen());

    tui.resize(220, 40);
    let wide = wait_join_at_width(&tui, 220, "220-col repaint after the resize");
    assert_eq!(
        wide,
        63,
        "a wider terminal keeps the tree:\n{}",
        tui.screen()
    );

    tui.key('>');
    wait_join(
        &tui,
        220,
        74,
        "`>` at 220 cols widens the tree to 75 columns",
    );

    tui.resize(80, 40);
    wait_join(&tui, 80, 57, "80 cols clamp the tree to 58 columns");

    tui.resize(220, 40);
    wait_join(&tui, 220, 74, "220 cols give the tree its 75 columns back");

    tui.key('<');
    wait_join(
        &tui,
        220,
        63,
        "`<` at 220 cols narrows the tree to 64 columns",
    );
}

/// A launch in a very wide terminal stops the tree at 80 columns (join
/// at column 79), not 40% of the terminal (120 columns at 300).
#[test]
fn pty_tree_launch_width_caps_at_eighty_columns() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open_size(&workspace, 300, 40);
    let join = wait_join_at_width(&tui, 300, "300-col first paint with a tree/right join");
    assert_eq!(join, 79, "an 80-col launch tree:\n{}", tui.screen());
}

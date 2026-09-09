use crate::harness::PtySession;
use crate::seed::worktree_workspace;
use crate::support::{
    crumb_row, no_mouse_toggle_toast, tree_cursor_on, tree_has, tree_line_containing,
    tree_pane_focused, WAIT,
};

const PRIMARY: &str = "feature/primary-open";
const LINKED: &str = "feature/linked-open";

fn no_wrong_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("Merge main into")
        && no_mouse_toggle_toast(screen)
}

/// ASCII nested primary: `& feature/primary-open`. Open `o` after the name is the bug.
///
/// Branch names contain the letter `o`. Never assert `!line.contains('o')`.
fn primary_row_omits_open_mark(screen: &str) -> bool {
    tree_line_containing(screen, PRIMARY).is_some_and(|line| {
        line.contains('&')
            && line.contains(PRIMARY)
            && !line.contains('L')
            && !line.contains(&format!("{PRIMARY} o"))
            && !line.contains(&format!("{PRIMARY} M"))
    })
}

/// ASCII linked extra: `L feature/linked-open o`. Merged `M` is the bug.
fn linked_row_keeps_open_mark(screen: &str) -> bool {
    tree_line_containing(screen, LINKED).is_some_and(|line| {
        line.contains('L')
            && line.contains(LINKED)
            && line.contains(&format!("{LINKED} o"))
            && !line.contains(&format!("{LINKED} M"))
    })
}

fn family_and_marks_on_tree(screen: &str) -> bool {
    tree_has(screen, "app")
        && tree_has(screen, PRIMARY)
        && tree_has(screen, LINKED)
        && tree_has(screen, "2 wt")
        && primary_row_omits_open_mark(screen)
        && linked_row_keeps_open_mark(screen)
}

fn family_tree_idle(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, PRIMARY)
        && !tree_cursor_on(screen, LINKED)
        && family_and_marks_on_tree(screen)
        && crumb_row(screen).contains("workspace › app")
        && no_wrong_overlays(screen)
}

/// Nested primary whose unique commits have not landed on default.
///
/// Docs: nested primary uses `&` and omits open-vs-default `o`; linked extra
/// keeps `L` plus `o`; merged `M` must not appear on these open rows.
///
/// Live PTY: first paint shows `& feature/primary-open` (no `feature/primary-open o`)
/// and `L feature/linked-open o`. The primary row has no `L`. Idle tree, no
/// overlays. A no-op or a primary painted like a linked open row cannot pass.
#[test]
fn pty_primary_omits_open_vs_default_mark() {
    let (_root, workspace) = worktree_workspace();

    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        family_tree_idle,
        "first paint: nested primary omits o, linked extra keeps L plus o",
        WAIT,
    );
}

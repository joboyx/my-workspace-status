use crate::harness::PtySession;
use crate::seed::primary_merged_workspace;
use crate::support::{
    crumb_row, no_mouse_toggle_toast, tree_cursor_on, tree_has, tree_line_containing,
    tree_pane_focused, WAIT,
};

const PRIMARY: &str = "feature/primary-merged";
const LINKED_MERGED: &str = "feature/linked-merged";
const JUST_CREATED: &str = "feature/just-created";

fn no_wrong_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("Merge main into")
        && no_mouse_toggle_toast(screen)
}

/// ASCII nested primary: `& feature/primary-merged M`. Open `o` is the bug.
fn primary_row_is_merged(screen: &str) -> bool {
    tree_line_containing(screen, PRIMARY).is_some_and(|line| {
        line.contains('&')
            && line.contains(&format!("{PRIMARY} M"))
            && !line.contains(&format!("{PRIMARY} o"))
            && !line.contains('L')
            && !line.contains('o')
    })
}

/// ASCII linked row whose unique commits landed: `L feature/linked-merged M`.
fn linked_merged_row_is_merged(screen: &str) -> bool {
    tree_line_containing(screen, LINKED_MERGED).is_some_and(|line| {
        line.contains('L')
            && line.contains(&format!("{LINKED_MERGED} M"))
            && !line.contains(&format!("{LINKED_MERGED} o"))
    })
}

/// ASCII linked row: `L feature/just-created o`.
fn just_created_row_is_open(screen: &str) -> bool {
    tree_line_containing(screen, JUST_CREATED).is_some_and(|line| {
        line.contains('L')
            && line.contains(&format!("{JUST_CREATED} o"))
            && !line.contains(&format!("{JUST_CREATED} M"))
    })
}

fn family_and_marks_on_tree(screen: &str) -> bool {
    tree_has(screen, "app")
        && tree_has(screen, PRIMARY)
        && tree_has(screen, LINKED_MERGED)
        && tree_has(screen, JUST_CREATED)
        && tree_has(screen, "3 wt")
        && primary_row_is_merged(screen)
        && linked_merged_row_is_merged(screen)
        && just_created_row_is_open(screen)
}

fn family_tree_idle(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, PRIMARY)
        && !tree_cursor_on(screen, LINKED_MERGED)
        && !tree_cursor_on(screen, JUST_CREATED)
        && family_and_marks_on_tree(screen)
        && crumb_row(screen).contains("workspace › app")
        && no_wrong_overlays(screen)
}

/// Nested primary whose unique commits landed on default.
///
/// Docs: merge checkmark is a strict ancestor of the default tip. Primary
/// checkout paints that check (`M`) and omits open (`o`). Linked extras keep
/// both marks. Nested primary uses the branch glyph (`&`), never the linked
/// worktree mark (`L`).
///
/// Live PTY: first paint shows `& feature/primary-merged M`,
/// `L feature/linked-merged M`, and `L feature/just-created o`. The primary
/// row has no `o`. Idle tree, no overlays. A missing check on the primary
/// row, an open mark on primary, a missing check on the linked merged row,
/// overlay-only, or a no-op that never paints `M`/`o` cannot pass.
#[test]
fn pty_primary_merged_branch_shows_check() {
    let (_root, workspace) = primary_merged_workspace();

    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        family_tree_idle,
        "first paint: nested primary merged (M), linked landed M, just-created o",
        WAIT,
    );
}

use crate::harness::PtySession;
use crate::seed::merge_mark_workspace;
use crate::support::{
    crumb_row, no_mouse_toggle_toast, status_row, tree_cursor_on, tree_has, tree_line_containing,
    tree_pane_focused, SETTLE_MS, WAIT,
};

const JUST_CREATED: &str = "feature/just-created";
const LANDED: &str = "feature/landed";
const PRIMARY: &str = "feature/primary-open";
const JUST_CREATED_PATH: &str = "app/.worktrees/new";

fn no_remove_confirm(screen: &str) -> bool {
    !screen.contains("Remove worktree ")
        && !screen.contains("NOT merged into default")
        && !screen.contains("clean worktree")
}

fn no_wrong_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("Merge main into")
        && no_mouse_toggle_toast(screen)
}

/// ASCII linked row: `L feature/just-created o`. Checkmark `M` is the bug.
fn just_created_row_is_open(screen: &str) -> bool {
    tree_line_containing(screen, JUST_CREATED).is_some_and(|line| {
        line.contains('L')
            && line.contains(&format!("{JUST_CREATED} o"))
            && !line.contains(&format!("{JUST_CREATED} M"))
    })
}

/// ASCII linked row whose unique commits landed: `L feature/landed M`.
fn landed_row_is_merged(screen: &str) -> bool {
    tree_line_containing(screen, LANDED).is_some_and(|line| {
        line.contains('L')
            && line.contains(&format!("{LANDED} M"))
            && !line.contains(&format!("{LANDED} o"))
    })
}

fn family_and_marks_on_tree(screen: &str) -> bool {
    tree_has(screen, "app")
        && tree_has(screen, PRIMARY)
        && tree_has(screen, JUST_CREATED)
        && tree_has(screen, LANDED)
        && tree_has(screen, "3 wt")
        && just_created_row_is_open(screen)
        && landed_row_is_merged(screen)
}

fn family_tree_idle(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, JUST_CREATED)
        && !tree_cursor_on(screen, LANDED)
        && !tree_cursor_on(screen, PRIMARY)
        && family_and_marks_on_tree(screen)
        && crumb_row(screen).contains("workspace › app")
        && no_remove_confirm(screen)
        && no_wrong_overlays(screen)
}

fn just_created_ready_to_remove(screen: &str) -> bool {
    let status = status_row(screen);
    tree_pane_focused(screen)
        && tree_cursor_on(screen, JUST_CREATED)
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, LANDED)
        && !tree_cursor_on(screen, PRIMARY)
        && family_and_marks_on_tree(screen)
        && crumb_row(screen).contains("workspace › new")
        && status.contains("W")
        && status.contains("remove worktree (open)")
        && !status.contains("remove worktree (merged)")
        && no_remove_confirm(screen)
        && no_wrong_overlays(screen)
}

/// `W` on a default-tip linked row. Confirm must not claim merged.
fn just_created_remove_confirm_open(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, JUST_CREATED)
        && family_and_marks_on_tree(screen)
        && screen.contains(&format!("Remove worktree {JUST_CREATED_PATH}?"))
        && screen.contains(&format!("branch {JUST_CREATED} — NOT merged into default"))
        && !screen.contains(&format!("branch {JUST_CREATED} — merged into default"))
        && screen.contains("clean worktree")
        && screen.contains("remove")
        && screen.contains("cancel")
        && no_wrong_overlays(screen)
}

/// `n` closed confirm. Toast is cancel. Default-tip stays open. Landed stays merged.
fn documented_cancel_keeps_open_default_tip(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    just_created_ready_to_remove(screen)
        && crumb.contains("remove worktree cancelled")
        && !crumb.contains("removed worktree")
        && !screen.contains("Remove worktree ")
}

/// Default-tip HEAD is a just-created branch, not merged.
///
/// Docs: merge checkmark is a strict ancestor of the default tip. HEAD
/// equal to that tip paints open (`o`), never merged (`M`). Help GIT `W`
/// confirm uses the same flag (`merged into default` / `NOT merged into
/// default`). Linked extras only.
///
/// Live PTY: first paint shows `L feature/just-created o` and
/// `L feature/landed M`. `j` `j` `j` land on the default-tip row with
/// `remove worktree (open)`. `W` paints `NOT merged into default`. `n`
/// cancels. A checkmark on the default-tip row, a missing check on
/// `feature/landed`, overlay-only, or a no-op that never paints `o`/`M`
/// cannot pass.
#[test]
fn pty_merge_mark_default_tip_is_open() {
    let (_root, workspace) = merge_mark_workspace();

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        family_tree_idle,
        "first paint: default-tip linked row open (o), landed merged (M)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, PRIMARY) && !tree_cursor_on(screen, JUST_CREATED),
        "first j: primary checkout (not a linked merge-mark row)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, LANDED) && landed_row_is_merged(screen),
        "second j: landed linked row still paints merged M",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        just_created_ready_to_remove,
        "third j: default-tip row; W hint is open, not merged",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        just_created_ready_to_remove,
        "default-tip open hint holds (not a delayed merged flicker)",
        WAIT,
    );

    tui.shift_letter('W');
    tui.wait_pred(
        just_created_remove_confirm_open,
        "W confirm: NOT merged into default (default-tip is not merged work)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        just_created_remove_confirm_open,
        "open confirm holds (not a merged-label flicker)",
        WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        documented_cancel_keeps_open_default_tip,
        "n cancels; default-tip stays open, landed stays merged",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_cancel_keeps_open_default_tip,
        "cancel holds (not a delayed remove or mark flip)",
        WAIT,
    );
}

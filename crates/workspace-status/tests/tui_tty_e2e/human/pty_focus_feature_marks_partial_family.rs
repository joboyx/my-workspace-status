use crate::harness::PtySession;
use crate::seed::{tree_focus_family_workspace, FOCUS_LINKED_BRANCH};
use crate::support::{
    focus_feature_from_app, tree_has, tree_has_branch, tree_line_containing, wait_screen, WAIT,
};

/// Primary branch of `app` in [`tree_focus_family_workspace`].
const PRIMARY_BRANCH: &str = "feature/ABC-123-invoice-export";

/// The family row (the only tree row with a ` wt` count) is `app` and
/// paints `mark`.
fn app_family_row_has(screen: &str, mark: &str) -> bool {
    tree_line_containing(screen, " wt")
        .is_some_and(|line| line.contains("app") && line.contains(mark))
}

/// `*` on a ticket branch keeps only the matching checkouts of a family: the
/// family row shows `1/2 wt` and the other worktree leaves the tree.
///
/// `app` has its primary checkout on `feature/ABC-123-invoice-export` and a
/// linked worktree on `feature/login-form`. At launch the family row shows
/// `2 wt` and both branches. `*` on `app` focuses `ABC-123`: the family row
/// shows `1/2 wt`, the primary branch stays, and `feature/login-form` is not
/// on the tree (it is not in the outside row either).
#[test]
fn pty_focus_feature_marks_partial_family() {
    let (_root, workspace) = tree_focus_family_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            app_family_row_has(screen, " 2 wt")
                && tree_has_branch(screen, PRIMARY_BRANCH)
                && tree_has(screen, &format!("L {FOCUS_LINKED_BRANCH}"))
        },
        "first paint: the app family shows `2 wt` and both branches",
        WAIT,
    );
    focus_feature_from_app(&mut tui);

    let screen = wait_screen(
        &tui,
        |screen| app_family_row_has(screen, "1/2 wt") && tree_has_branch(screen, PRIMARY_BRANCH),
        "the ABC-123 focus marks the app family `1/2 wt`",
        WAIT,
    );
    assert!(
        !tree_has(&screen, FOCUS_LINKED_BRANCH),
        "the linked worktree on {FOCUS_LINKED_BRANCH} must leave the tree:\n{screen}"
    );
}

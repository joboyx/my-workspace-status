use crate::harness::{left_tree, PtySession};
use crate::seed::ignored_primary_family_workspace;
use crate::support::{
    crumb_row, no_wrong_overlays, panes_tree_focused_diff_unfocused, status_row, tree_dir_expanded,
    tree_has, tree_line_containing, SETTLE_MS, WAIT,
};

const LINKED_BRANCH: &str = "feature/linked-open";
const PRIMARY_BRANCH: &str = "feature/primary-open";

fn linked_orphan_on_tree(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, LINKED_BRANCH)
        .or_else(|| tree_line_containing(screen, "app/.worktrees"))
    else {
        return false;
    };
    let family = tree_line_containing(screen, "app");
    let family_is_parent = family.is_some_and(|row| {
        row.contains("@ app") && (row.contains("2 wt") || tree_dir_expanded(screen, "app"))
    });
    (line.contains('L') || line.contains("app/.worktrees")) && !family_is_parent
}

fn ignored_family_absent(screen: &str) -> bool {
    let left = left_tree(screen);
    !left.contains(LINKED_BRANCH)
        && !left.contains("app/.worktrees")
        && !left.contains(PRIMARY_BRANCH)
        && !tree_has(screen, "2 wt")
        && !tree_has(screen, "app")
        && !linked_orphan_on_tree(screen)
}

fn lib_stays(screen: &str) -> bool {
    tree_has(screen, "lib")
        && tree_has(screen, "README.md")
        && tree_dir_expanded(screen, "lib")
        && panes_tree_focused_diff_unfocused(screen)
        && no_wrong_overlays(screen)
}

/// Cold start (no `-a`): ignored primary `app` and its linked child stay out.
fn documented_launch_hides_ignored_family(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    lib_stays(screen)
        && ignored_family_absent(screen)
        && !crumb.contains("showing ignored repos")
        && !crumb.contains("hiding ignored repos")
}

/// `.` shows the family nested: `@ app` + `L feature/linked-open`, not an orphan.
fn documented_dot_shows_ignored_family(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let linked = tree_line_containing(screen, LINKED_BRANCH);
    let family = tree_line_containing(screen, "app");
    lib_stays(screen)
        && tree_has(screen, "app")
        && tree_has(screen, "2 wt")
        && tree_has(screen, PRIMARY_BRANCH)
        && tree_has(screen, LINKED_BRANCH)
        && tree_dir_expanded(screen, "app")
        && family.is_some_and(|line| line.contains("@ app") && line.contains("2 wt"))
        && linked.is_some_and(|line| line.contains('L') && line.contains("linked-open"))
        && !linked_orphan_on_tree(screen)
        && crumb.contains("showing ignored repos")
        && !crumb.contains("hiding ignored repos")
}

/// Second `.` hides both again. The same leak as cold start must fail.
fn documented_dot_hides_ignored_family(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    lib_stays(screen)
        && ignored_family_absent(screen)
        && crumb.contains("hiding ignored repos")
        && !crumb.contains("showing ignored repos")
        && status.contains("focus right")
}

/// `.` hides linked worktrees of an ignored primary, then shows them nested.
///
/// Docs: Help VIEW `.` = show / hide ignored repos. Hidden ignored includes
/// linked children of an ignored primary. They return with `.` / `-a` as a
/// family (`2 wt`, `L` + branch), not a workspace-root `L` orphan.
///
/// Live PTY after first paint: ignored `app` and `feature/linked-open` stay
/// out. `.` inserts `@ app` with `2 wt` and nested `L feature/linked-open`.
/// A second `.` removes both. A toast-only, `app`-only, or still-orphan
/// frame cannot pass.
#[test]
fn pty_dot_hides_linked_of_ignored_primary() {
    let (_root, workspace) = ignored_primary_family_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_hides_ignored_family,
        "first paint: ignored app and linked child hidden (`.` has not run)",
        WAIT,
    );

    tui.key('.');
    tui.wait_pred(
        documented_dot_shows_ignored_family,
        "`.` shows ignored app family nested; linked is not a workspace-root L orphan",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_dot_shows_ignored_family,
        "shown ignored family holds (not a flicker, toast-only, or orphan)",
        WAIT,
    );

    tui.key('.');
    tui.wait_pred(
        documented_dot_hides_ignored_family,
        "second `.` hides app and linked child; toast hiding",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_dot_hides_ignored_family,
        "hidden family holds (not a flicker or still-orphan after hide)",
        WAIT,
    );
}

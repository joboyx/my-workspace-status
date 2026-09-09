use crate::harness::{left_tree, PtySession};
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, no_updates_group_folded, no_wrong_overlays,
    pane_unstaged_readme, panes_tree_focused_diff_unfocused, panes_tree_unfocused_diff_focused,
    status_row, tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has,
    tree_line_containing, SETTLE_MS, WAIT,
};

fn tree_readme_rows(screen: &str) -> usize {
    left_tree(screen)
        .lines()
        .filter(|line| line.contains("README.md"))
        .count()
}

fn workspace_heading(screen: &str) -> Option<String> {
    tree_line_containing(screen, "# workspace")
}

/// Daily seed after `.`: ignored `notes` is its own repo row, not a group child.
fn notes_ignored_repo_row(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, "notes") else {
        return false;
    };
    line.contains("@ notes")
        && line.contains('~')
        && line.contains("& main")
        && !line.contains("[ignored]")
        && !line.contains("No updates")
        && tree_dir_expanded(screen, "notes")
        && !tree_dir_collapsed(screen, "notes")
}

/// Seed rows that `.` must not fold, jump, or send into No-updates.
fn seed_tree_stays(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && !tree_cursor_on(screen, "notes")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_has(screen, "feature/graph")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && pane_unstaged_readme(screen)
        && no_wrong_overlays(screen)
}

/// Left-focused `.` showed ignored `notes`. Toast-only or No-updates cannot pass.
fn documented_dot_shows_ignored_notes(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    let heading = workspace_heading(screen).unwrap_or_default();
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, "README.md")
        && seed_tree_stays(screen)
        && notes_ignored_repo_row(screen)
        && tree_readme_rows(screen) == 2
        && heading.contains("2 changed · all current")
        && !heading.contains("1 changed")
        && crumb.contains("showing ignored repos")
        && !crumb.contains("hiding ignored repos")
        && !crumb.contains("[workspace]")
        && !crumb.contains('›')
        && status.contains("focus right")
        && status.contains(" tree")
        && status.contains(" split")
        && !status.contains("drill")
}

/// Second `.` hid `notes`. Hide-only / still-shown cannot pass.
fn documented_dot_hides_ignored_notes(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    let heading = workspace_heading(screen).unwrap_or_default();
    panes_tree_focused_diff_unfocused(screen)
        && tree_cursor_on(screen, "README.md")
        && seed_tree_stays(screen)
        && tree_line_containing(screen, "notes").is_none()
        && !tree_has(screen, "notes")
        && tree_readme_rows(screen) == 1
        && heading.contains("1 changed · all current")
        && !heading.contains("2 changed")
        && crumb.contains("hiding ignored repos")
        && !crumb.contains("showing ignored repos")
        && !crumb.contains("[workspace]")
        && !crumb.contains('›')
        && status.contains("focus right")
        && status.contains(" tree")
        && status.contains(" split")
        && !status.contains("drill")
}

/// Tab focused the file-diff. Ignored `notes` stays hidden until `.`.
fn right_focused_notes_still_hidden(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    let heading = workspace_heading(screen).unwrap_or_default();
    panes_tree_unfocused_diff_focused(screen)
        && seed_tree_stays(screen)
        && tree_line_containing(screen, "notes").is_none()
        && !tree_has(screen, "notes")
        && tree_readme_rows(screen) == 1
        && heading.contains("1 changed · all current")
        && crumb.contains("[workspace]")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !status.contains("focus right")
}

/// Right-focused `.` still shows ignored `notes` (not a left-list action).
fn documented_dot_shows_ignored_notes_right(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    let heading = workspace_heading(screen).unwrap_or_default();
    panes_tree_unfocused_diff_focused(screen)
        && seed_tree_stays(screen)
        && notes_ignored_repo_row(screen)
        && tree_readme_rows(screen) == 2
        && heading.contains("2 changed · all current")
        && !heading.contains("1 changed")
        && crumb.contains("[workspace]")
        && crumb.contains("showing ignored repos")
        && !crumb.contains("hiding ignored repos")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !status.contains("focus right")
}

/// `.` shows ignored repos on the tree, then hides them. Toggles either way.
///
/// Docs: Help VIEW `.` = show / hide ignored repos. Keymap: starts hidden
/// unless `-a`, rebuilds the tree, works when right-focused. Ignored
/// `notes` is a muted repo with the ignored glyph (`~` in ASCII), not
/// `[ignored]` and not a No-updates child. After `.` it is not folded.
/// Hidden ignored stay out of the tree.
///
/// Live PTY after first paint: `.` inserts expanded `@ notes ~` with its
/// dirty README, bumps the workspace heading to `2 changed`, and toasts
/// `showing ignored repos`. A second `.` removes that row and toasts
/// `hiding ignored repos`. Tab then `.` still toggles. A no-op, hide-only,
/// toast-only, or unfold-No-updates cannot pass.
#[test]
fn pty_dot_toggles_ignored_repos() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: ignored notes hidden (`.` has not run)",
        WAIT,
    );

    tui.key('.');
    tui.wait_pred(
        documented_dot_shows_ignored_notes,
        "`.` shows ignored notes as `~` repo row; not No-updates; toast showing",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_dot_shows_ignored_notes,
        "shown ignored notes hold (not a flicker, toast-only, or group unfold)",
        WAIT,
    );

    tui.key('.');
    tui.wait_pred(
        documented_dot_hides_ignored_notes,
        "second `.` hides notes; heading back to 1 changed; toast hiding",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_dot_hides_ignored_notes,
        "hidden notes hold (not a flicker or hide-only first press)",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        right_focused_notes_still_hidden,
        "Tab focuses the file-diff; notes stay hidden until `.`",
        WAIT,
    );
    tui.key('.');
    tui.wait_pred(
        documented_dot_shows_ignored_notes_right,
        "right-focused `.` still shows ignored notes (not a left-list no-op)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_dot_shows_ignored_notes_right,
        "right-focused shown notes hold",
        WAIT,
    );
}

use crate::harness::{PtySession, ROWS};
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, has_stage_hint, idle_dirty_readme_unstaged, no_wrong_overlays, pane_unstaged_readme,
    readme_unstaged_badge, status_row, tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT,
};

fn no_y_revert_path(screen: &str) -> bool {
    !screen.contains("reverted")
        && !screen.contains("deleted README")
        && !screen.contains("Working tree clean")
        && !screen.contains("working tree clean")
}

fn no_wrong_revert_overlays(screen: &str) -> bool {
    no_wrong_overlays(screen)
        && !screen.contains("Drop ")
        && !screen.contains("Remove worktree")
        && !screen.contains("Create branch")
        && !screen.contains("Merge ")
        && !screen.contains("nothing to discard")
        && !screen.contains("Nothing to discard")
        && !screen.contains("focus a file")
}

fn dirty_readme_still_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
}

/// Boxed `x` confirm: counted revert, `y`/`Y`/`n`. File is still dirty.
fn documented_revert_confirm_armed(screen: &str) -> bool {
    dirty_readme_still_focused(screen)
        && screen.contains("Revert README.md?")
        && screen.contains("1 tracked file")
        && screen.contains("discarded")
        && screen.contains("0 untracked files")
        && screen.contains("kept")
        && screen.contains("revert + delete untracked")
        && screen.contains("cancel")
        && !screen.contains("revert cancelled")
        && no_y_revert_path(screen)
        && no_wrong_revert_overlays(screen)
}

/// `n` closed the confirm. Toast is cancel. README is still unstaged.
fn documented_revert_n_cancelled(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    dirty_readme_still_focused(screen)
        && crumb.contains("revert cancelled")
        && !crumb.contains("reverted")
        && !screen.contains("Revert README.md?")
        && !screen.contains("revert + delete untracked")
        && !screen.contains("1 tracked file")
        && has_stage_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && status.contains("revert")
        && no_y_revert_path(screen)
        && no_wrong_revert_overlays(screen)
}

/// Launch is 140×32 (`PtySession::open`). A 100×24 resize paints fewer lines.
fn painted_shorter_than_launch(screen: &str) -> bool {
    screen.lines().count() < usize::from(ROWS)
}

/// Confirm still armed on the short grid (not a same-size no-op).
fn documented_revert_confirm_armed_on_short_grid(screen: &str) -> bool {
    documented_revert_confirm_armed(screen) && painted_shorter_than_launch(screen)
}

/// `j` moved the tree cursor off the dirty README (onto `app`).
fn cursor_moved_off_readme(screen: &str) -> bool {
    !tree_cursor_on(screen, "README.md") || tree_cursor_on(screen, "app")
}

/// Boxed `x` confirm stays armed across a live PTY resize, swallows `j`,
/// then `n` still cancels.
///
/// Help GIT `x` is revert (`y`/`Y`). Configuration: `x` confirms with
/// counts (`y` tracked only, `Y` also deletes untracked); `n` / Esc
/// cancel. Keymap: `x` is `Action::Revert` (opens `PendingConfirm::Revert`);
/// confirm `n` is `Action::ConfirmNo` (`revert cancelled`, no write).
/// Resize does not dismiss the overlay. Movement keys are swallowed
/// while it is open.
///
/// After first paint the cursor is already on the dirty README. Do not
/// `/` search (`n` would be next-match if confirm never armed). A no-op
/// resize, overlay drop, `j` move, immediate revert, `y` path,
/// overlay-only paint, or toast-only tick is red. This test does not
/// claim `y` / `Y` apply, pane/help relayout, theme, or scrollbar.
/// `pty_revert_confirm_n_cancels` owns the no-resize `n` path.
#[test]
fn pty_revert_confirm_survives_resize() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unstaged,
        "first paint: cursor on dirty README, unstaged, no confirm",
        WAIT,
    );

    tui.key('x');
    tui.wait_pred(
        documented_revert_confirm_armed,
        "x arms Revert README.md? with y/Y/n; file stays dirty",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_revert_confirm_armed,
        "revert confirm holds (not a flicker, y path, or toast-only tick)",
        WAIT,
    );

    assert!(
        !painted_shorter_than_launch(&tui.screen()),
        "short-grid claim must be false before resize (launch is 140x32):\n{}",
        tui.screen()
    );
    tui.resize(100, 24);
    tui.wait_pred(
        documented_revert_confirm_armed_on_short_grid,
        "100x24 resize keeps Revert README.md? y/Y/n on a shorter grid",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_revert_confirm_armed_on_short_grid,
        "confirm holds after resize (not a flicker, overlay drop, or same-size no-op)",
        WAIT,
    );

    assert!(
        !cursor_moved_off_readme(&tui.screen()),
        "cursor-moved-off-README claim must be false before j:\n{}",
        tui.screen()
    );
    tui.key('j');
    tui.wait_pred(
        documented_revert_confirm_armed,
        "j does not move the tree cursor; confirm stays armed on README",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_revert_confirm_armed,
        "confirm still armed after j (overlay swallowed movement)",
        WAIT,
    );

    assert!(
        !documented_revert_n_cancelled(&tui.screen()),
        "cancelled claim must be false before n:\n{}",
        tui.screen()
    );
    tui.key('n');
    tui.wait_pred(
        documented_revert_n_cancelled,
        "n cancels: revert cancelled toast, README still unstaged, overlay gone",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_revert_n_cancelled,
        "cancelled paint holds (not a flicker, y revert, or overlay return)",
        WAIT,
    );
}

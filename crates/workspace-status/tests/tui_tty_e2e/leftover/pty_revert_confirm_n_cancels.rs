use crate::harness::PtySession;
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

/// `x` arms revert confirm; `n` cancels and does not revert.
///
/// Docs: Help GIT `x` is revert (`y`/`Y`). Configuration: `x` confirms
/// with counts (`y` tracked only, `Y` also deletes untracked); `n` / Esc
/// cancel. Keymap: `x` is `Action::Revert` (opens `PendingConfirm::Revert`);
/// confirm `n` is `Action::ConfirmNo` (`revert cancelled`, no write).
/// `y`/`Enter` would `git restore` tracked files (`reverted …`).
///
/// After first paint the cursor is already on the dirty README. Do not
/// `/` search (`n` would be next-match if confirm never armed). A no-op,
/// immediate revert, `y` path, overlay-only paint, or toast-only tick
/// is red. This leftover does not claim `y` / `Y` apply.
#[test]
fn pty_revert_confirm_n_cancels() {
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

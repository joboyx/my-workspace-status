use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, idle_dirty_readme_unstaged, no_wrong_overlays, pane_unstaged_readme,
    readme_unstaged_badge, tree_cursor_on, GIT_WAIT, SETTLE_MS, WAIT,
};

const HEAD_README: &str = "# app\n";
const DIRTY_README: &str = "# app\ndirty\n";

fn worktree_readme(app: &Path) -> String {
    fs::read_to_string(app.join("README.md")).expect("README.md")
}

fn no_revert_applied(screen: &str) -> bool {
    !screen.contains("reverted")
        && !screen.contains("deleted README")
        && !screen.contains("revert cancelled")
}

/// Boxed `x` confirm on a tracked-only file: `y` / `n` only, no `Y`.
fn tracked_only_confirm_armed(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && screen.contains("Revert README.md?")
        && screen.contains("1 tracked file")
        && screen.contains("discarded")
        && !screen.contains("untracked")
        && !screen.contains("revert + delete untracked")
        && screen.contains("cancel")
        && no_revert_applied(screen)
        && no_wrong_overlays(screen)
}

/// `y` restored README. Overlay gone. Toast names the file.
fn readme_reverted(screen: &str) -> bool {
    crumb_row(screen).contains("reverted README.md")
        && !screen.contains("Revert README.md?")
        && !screen.contains("1 tracked file")
        && !readme_unstaged_badge(screen)
        && !pane_unstaged_readme(screen)
        && no_wrong_overlays(screen)
}

/// Shift+Y on a tracked-only revert confirm does nothing; `y` then reverts.
///
/// Configuration: `x` confirms with counts and offers only the keys that
/// apply. A tracked-only scope shows `y revert` and `n Esc cancel`; a key the
/// box does not show keeps it open. Keymap: `Y` is `Action::ConfirmYesClean`,
/// `y` is `Action::ConfirmYes`. Live TUI reads Shift+Y as CSI-u.
///
/// Git truth is the oracle: README stays dirty after Shift+Y and matches
/// HEAD after `y`. A Shift+Y revert, a closed overlay, or a toast-only
/// tick is red. `pty_revert_confirm_n_cancels` owns `n`.
#[test]
fn pty_revert_confirm_tracked_only_ignores_shift_y() {
    let (_root, workspace) = daily_workspace();
    let app = workspace.join("app");
    assert_eq!(
        worktree_readme(&app),
        DIRTY_README,
        "seed must dirty README"
    );

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
        tracked_only_confirm_armed,
        "x arms Revert README.md? with y/n only",
        WAIT,
    );

    tui.shift_letter('Y');
    tui.wait_ms(SETTLE_MS * 3);
    tui.wait_pred(
        tracked_only_confirm_armed,
        "Shift+Y keeps the tracked-only confirm open and reverts nothing",
        WAIT,
    );
    assert_eq!(
        worktree_readme(&app),
        DIRTY_README,
        "Shift+Y must not revert README:\n{}",
        tui.screen()
    );

    tui.letter_press('y');
    tui.wait_pred(
        readme_reverted,
        "y reverts README; overlay gone; toast reverted README.md",
        GIT_WAIT,
    );
    assert_eq!(
        worktree_readme(&app),
        HEAD_README,
        "y must git-restore README to HEAD:\n{}",
        tui.screen()
    );
}

use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, no_wrong_overlays, readme_unstaged_badge, tree_cursor_on, tree_has, GIT_WAIT,
    SETTLE_MS, WAIT,
};

const UNTRACKED: &str = "new.txt";

/// Confirm chip row: the box line that ends with `n Esc cancel`.
fn chip_row(screen: &str) -> String {
    screen
        .lines()
        .find(|line| line.contains(" Esc  cancel"))
        .unwrap_or_default()
        .to_string()
}

/// Boxed `x` confirm on one untracked file: `y delete` / `n` only.
fn single_untracked_confirm_armed(screen: &str) -> bool {
    let chips = chip_row(screen);
    tree_cursor_on(screen, UNTRACKED)
        && screen.contains(&format!("Revert {UNTRACKED}?"))
        && screen.contains("1 untracked file")
        && screen.contains("deleted")
        && !screen.contains(" tracked file")
        && !screen.contains("discarded")
        && chips.contains(" y  delete")
        && !chips.contains(" Y ")
        && !chips.contains("revert")
        && !chips.contains("delete untracked")
        && !crumb_row(screen).contains(&format!("deleted {UNTRACKED}"))
        && no_wrong_overlays(screen)
}

/// `y` deleted the untracked file. Overlay gone. Tracked README still dirty.
fn untracked_deleted(screen: &str) -> bool {
    crumb_row(screen).contains(&format!("deleted {UNTRACKED}"))
        && !screen.contains(&format!("Revert {UNTRACKED}?"))
        && !tree_has(screen, UNTRACKED)
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && no_wrong_overlays(screen)
}

/// `x` on one untracked file offers `y delete` and no `Y`; `y` deletes it.
///
/// Configuration: `x` confirms with counts and offers only the keys that
/// apply. One untracked file with nothing tracked shows `y delete` and
/// `n Esc cancel`; Shift+Y is not shown, so it keeps the box open.
///
/// Daily seed plus untracked `new.txt`. `j` moves from README to
/// `new.txt`. Disk truth is the oracle: the file survives Shift+Y and is
/// gone after `y`, while tracked README stays dirty.
#[test]
fn pty_revert_confirm_single_untracked_y_deletes() {
    let (_root, workspace) = daily_workspace();
    let app = workspace.join("app");
    let untracked = app.join(UNTRACKED);
    fs::write(&untracked, "delete me\n").unwrap();

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains(UNTRACKED, GIT_WAIT);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md") && tree_has(screen, UNTRACKED),
        "first paint: README focused, new.txt listed",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, UNTRACKED),
        "j focuses untracked new.txt",
        WAIT,
    );

    tui.key('x');
    tui.wait_pred(
        single_untracked_confirm_armed,
        "x arms Revert new.txt? with y delete / n only",
        WAIT,
    );

    tui.shift_letter('Y');
    tui.wait_ms(SETTLE_MS * 3);
    tui.wait_pred(
        single_untracked_confirm_armed,
        "Shift+Y keeps the single-untracked confirm open",
        WAIT,
    );
    assert!(
        untracked.is_file(),
        "Shift+Y must not delete {UNTRACKED}:\n{}",
        tui.screen()
    );

    tui.letter_press('y');
    tui.wait_pred(
        untracked_deleted,
        "y deletes new.txt; overlay gone; README still dirty",
        GIT_WAIT,
    );
    assert!(
        !untracked.exists(),
        "y must delete {UNTRACKED}:\n{}",
        tui.screen()
    );
    assert!(
        fs::read_to_string(app.join("README.md"))
            .unwrap()
            .contains("dirty"),
        "y on new.txt must not touch README:\n{}",
        tui.screen()
    );
}

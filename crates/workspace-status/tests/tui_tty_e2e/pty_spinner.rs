//! Live row spinner while remote git runs on a checkout.

use std::cell::RefCell;
use std::collections::HashSet;

use crate::seed::ahead_workspace;
use crate::slow_git::{open_with_slow_git, wait_slow_git_started, SlowFetchPullGit};
use crate::support::{after_ack, after_syncbox_name, tree_cursor_on, GIT_WAIT, WAIT};

/// ASCII spinner frames (the harness runs `WS_STATUS_GLYPHS=ascii`).
const ASCII_FRAMES: [char; 4] = ['|', '/', '-', '\\'];

/// The syncbox row paints an ASCII spinner frame after its name.
fn syncbox_row_spins(screen: &str) -> bool {
    after_syncbox_name(screen).is_some_and(|after| after.contains(ASCII_FRAMES))
}

/// Workspace `f` with a slow fetch: the syncbox row spins while git runs,
/// and the frame is gone once the fetch lands. Fail if the row never
/// spins, or keeps a frame after `Fetched 1 repo`.
#[test]
fn pty_fetch_shows_spinner_on_repo_row_then_clears() {
    let (_root, workspace) = ahead_workspace();
    let slow = SlowFetchPullGit::install(&workspace);
    let mut tui = open_with_slow_git(&workspace, &slow);
    tui.wait_pred(
        |screen| after_syncbox_name(screen).is_some_and(|after| after.contains("^1")),
        "first paint: syncbox row, one ahead",
        WAIT,
    );
    assert!(
        !syncbox_row_spins(&tui.screen()),
        "idle row has no spinner frame; screen:\n{}",
        tui.screen()
    );

    tui.gg();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "workspace"),
        "gg focuses the workspace row",
        WAIT,
    );
    tui.key('f');
    wait_slow_git_started(&slow, || tui.screen(), WAIT);
    tui.wait_pred(
        |screen| syncbox_row_spins(screen) && !screen.contains("Fetched"),
        "syncbox row spins while its fetch runs",
        WAIT,
    );
    // No key lands now: only the animation tick can change the frame.
    let seen = RefCell::new(HashSet::new());
    tui.wait_pred(
        |screen| {
            if let Some(after) = after_syncbox_name(screen) {
                seen.borrow_mut()
                    .extend(after.chars().filter(|ch| ASCII_FRAMES.contains(ch)));
            }
            seen.borrow().len() >= 2
        },
        "the spinner frame advances on the animation tick",
        WAIT,
    );

    tui.wait_pred(
        after_ack("Fetched 1 repo", |screen| !syncbox_row_spins(screen)),
        "spinner clears once the fetch lands",
        GIT_WAIT,
    );
}

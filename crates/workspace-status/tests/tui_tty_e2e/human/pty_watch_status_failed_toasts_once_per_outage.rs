use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::pr_support::tab_strip;
use crate::seed::{seed_repo, unique_root};
use crate::support::{corrupt_index, tree_has, GIT_WAIT, WAIT};

/// The outage toast text of the broken checkout.
const TOAST: &str = "git status (app): status failed";

/// The header mark at the right end of the tab strip.
const HEADER: &str = "refresh failed";

/// Write a new untracked file in `repo` and wait until the tree lists it,
/// which proves a later watch tick landed.
fn wait_for_a_tick(tui: &PtySession, repo: &Path, name: &str) {
    fs::write(repo.join(name), "tick\n").unwrap();
    tui.wait_pred(
        |screen| tree_has(screen, name),
        "a watch tick lists the new file",
        GIT_WAIT,
    );
}

/// A checkout whose `git status` fails on a watch tick shows one sticky
/// outage toast and the header mark per outage (`docs/tui-rust.md`,
/// **Background job failures**, live refresh).
///
/// Seed: two dirty checkouts, `app` and `lib`, with the live-refresh poll
/// at 500 ms. The test breaks `app` by truncating its index, then puts the
/// index back.
///
/// Proof: a tick after the break paints `git status (app): status failed ·
/// Esc dismiss` and the tab strip ends in `refresh failed`. Esc hides the
/// toast and keeps the header. Two more ticks (proved by new files in
/// `lib`) bring no toast back. Once the index is back, a tick drops the
/// header. A new break toasts again. Reverting the once-per-outage rule
/// toasts again on the next tick after Esc; reverting the recovery rule
/// keeps the header after the repair.
#[test]
fn pty_watch_status_failed_toasts_once_per_outage() {
    let root = unique_root("ws-tui-e2e-watch-status-outage");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    seed_repo(&workspace, "lib", "main", true);
    let app = workspace.join("app");
    let lib = workspace.join("lib");
    let index = app.join(".git").join("index");
    let good_index = fs::read(&index).unwrap();

    let mut tui =
        PtySession::open_size_with_env(&workspace, 140, 40, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_pred(
        |screen| tree_has(screen, "app") && tree_has(screen, "lib"),
        "launch shows both dirty checkouts",
        WAIT,
    );
    assert!(
        !tab_strip(&tui).contains(HEADER),
        "a healthy launch has no header mark: {:?}",
        tab_strip(&tui)
    );

    // Break `app`: the next tick starts the outage.
    corrupt_index(&index);
    tui.wait_pred(
        |screen| {
            screen.contains(&format!("{TOAST} · Esc dismiss"))
                && tab_strip(&tui).ends_with(HEADER)
                && tree_has(screen, "status failed")
        },
        "a tick shows the outage toast and the header mark",
        GIT_WAIT,
    );
    assert_eq!(
        tui.screen().matches(TOAST).count(),
        1,
        "one toast:\n{}",
        tui.screen()
    );

    // Esc hides the toast; the outage goes on, so the header stays.
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains(TOAST),
        "Esc dismisses the toast",
        WAIT,
    );
    assert!(
        tab_strip(&tui).ends_with(HEADER),
        "the header mark stays: {:?}",
        tab_strip(&tui)
    );

    // More ticks in the same outage: no toast comes back.
    wait_for_a_tick(&tui, &lib, "tick-one.txt");
    wait_for_a_tick(&tui, &lib, "tick-two.txt");
    let screen = tui.screen();
    assert!(!screen.contains(TOAST), "no toast again:\n{screen}");
    assert!(tab_strip(&tui).ends_with(HEADER), "{screen}");
    assert!(tree_has(&screen, "status failed"), "{screen}");

    // Repair `app`: a tick ends the outage.
    fs::write(&index, &good_index).unwrap();
    tui.wait_pred(
        |screen| !tree_has(screen, "status failed") && !tab_strip(&tui).contains(HEADER),
        "the repaired checkout drops the header mark",
        GIT_WAIT,
    );

    // A new break is a new outage: the toast shows again.
    corrupt_index(&index);
    tui.wait_pred(
        |screen| screen.contains(TOAST) && tab_strip(&tui).ends_with(HEADER),
        "a break after the recovery toasts again",
        GIT_WAIT,
    );
}

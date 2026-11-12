//! Load regression test: a watch tick over a large workspace rebuilds the
//! tree rows a fixed number of times, not once per checkout.
//!
//! Each watch tick runs one status job per checkout. When each result
//! rebuilt the workspace snapshot and the tree rows, the cost of one tick
//! grew with checkouts x dirty files, and a large workspace kept the event
//! loop thread near one full core. The test counts
//! [`AppState::rebuild_rows`] calls, not wall-clock time, so a slow runner
//! cannot fail it.
//!
//! It builds 60 git repositories, so it runs only with the `slow-tests`
//! feature: `cargo test -p workspace-status --lib --features slow-tests load_tests`.
//! CI runs it in the `cargo`, `cargo-macos` and `cargo-windows` checks.

use std::collections::HashSet;
use std::fs;
use std::path::Path;

use super::tests::opts;
use super::Interpreter;
use crate::config::WorkspaceStatusConfig;
use crate::settings::Settings;
use crate::testutil::{git, init_repo, unique_dir};
use crate::tui::action::Action;
use crate::tui::collect_full_snapshot;
use crate::tui::state::{rebuild_rows_count, reset_rebuild_rows_count, AppState};
use crate::tui::tree::NodeKind;

const CHECKOUTS: usize = 60;
const COMMITS: usize = 3;
const UNTRACKED: usize = 200;

/// The most row rebuilds one watch tick may cost: one when the focused
/// checkout's result lands, one when the collect ends.
const MAX_REBUILDS_PER_TICK: usize = 2;

/// A repository at `dir` on `main` with [`COMMITS`] commits.
fn seed_checkout(dir: &Path) {
    init_repo(dir);
    for n in 2..=COMMITS {
        fs::write(dir.join(format!("commit-{n}.txt")), format!("{n}\n")).expect("write file");
        git(dir, &["add", "-A"]);
        git(dir, &["commit", "-q", "-m", &format!("commit {n}")]);
    }
}

/// [`UNTRACKED`] new files under `dir/src`.
fn write_untracked(dir: &Path) {
    let src = dir.join("src");
    fs::create_dir_all(&src).expect("create src");
    for n in 0..UNTRACKED {
        fs::write(src.join(format!("file-{n:03}.txt")), "untracked\n").expect("write file");
    }
}

/// Dispatch one watch tick and run it to the end on this thread, the way
/// the live loop dispatches it. Returns the rebuilds the tick cost.
fn watch_tick(interp: &mut Interpreter, state: &mut AppState) -> usize {
    let tui_opts = opts(state);
    reset_rebuild_rows_count();
    let effect = state.dispatch(Action::WatchTick);
    interp.interpret_sync(state, &tui_opts, effect, &Action::WatchTick);
    assert!(!interp.sched.collection_active(), "the collect ended");
    rebuild_rows_count()
}

/// Every checkout has a row in the tree and [`UNTRACKED`] untracked
/// changes in the snapshot.
fn assert_every_checkout_applied(state: &AppState) {
    assert_eq!(state.snapshot.repos.len(), CHECKOUTS);
    for row in &state.snapshot.repos {
        assert_eq!(row.changes.len(), UNTRACKED, "changes of {}", row.repo);
        assert!(row.changes.iter().all(|change| change.untracked));
    }
    let tree_repos: HashSet<&str> = state
        .rows
        .iter()
        .filter(|row| matches!(row.kind, NodeKind::Repo | NodeKind::Checkout))
        .filter_map(|row| row.repo.as_deref())
        .collect();
    assert_eq!(tree_repos.len(), CHECKOUTS, "a tree row per checkout");
}

#[test]
fn load_watch_tick_over_60_checkouts_rebuilds_rows_at_most_twice() {
    let root = unique_dir("ws-load-watch-tick");
    let dirs: Vec<_> = (0..CHECKOUTS)
        .map(|n| root.join(format!("repo-{n:02}")))
        .collect();
    for dir in &dirs {
        seed_checkout(dir);
    }
    let settings = Settings::resolve(&Default::default(), |_| None);
    let snapshot = collect_full_snapshot(
        &root,
        &WorkspaceStatusConfig::with_defaults(),
        &[],
        false,
        false,
        settings.fetch_concurrency,
    );
    assert_eq!(snapshot.repos.len(), CHECKOUTS);
    assert!(snapshot.repos.iter().all(|row| row.changes.is_empty()));
    let mut state = AppState::new(root.clone(), snapshot, true);
    let mut interp = Interpreter::with_fetch_cap(settings.fetch_concurrency);

    // Every checkout changes after launch, so the tick must apply all 60.
    for dir in &dirs {
        write_untracked(dir);
    }
    let rebuilds = watch_tick(&mut interp, &mut state);
    assert!(
        (1..=MAX_REBUILDS_PER_TICK).contains(&rebuilds),
        "a tick that changed every checkout rebuilt the rows {rebuilds} times"
    );
    assert_every_checkout_applied(&state);

    // Nothing changed: the steady-state tick costs no more.
    let rebuilds = watch_tick(&mut interp, &mut state);
    assert!(
        rebuilds <= MAX_REBUILDS_PER_TICK,
        "an unchanged tick rebuilt the rows {rebuilds} times"
    );
    assert_every_checkout_applied(&state);

    let _ = fs::remove_dir_all(&root);
}

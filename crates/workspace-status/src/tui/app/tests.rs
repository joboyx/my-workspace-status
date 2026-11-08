use super::*;
use crate::config::WorkspaceStatusConfig;
use crate::git::{exec_git, git_binary, list_local_branches, stage_file};
use crate::parallel::FETCH_CONCURRENCY;
use crate::testutil::{app_state, git, init_repo, panic_hook_lock, repo_snapshot, unique_dir};
use crate::tui::action::{Action, Effect};
use crate::tui::branches::DIRTY_WORKTREE_STATUS;
use crate::tui::pointer_shape::PointerShape;
use crate::tui::state::PendingConfirm;
use crate::tui::status::StatusKind;
use crate::tui::term_progress::{ProgressState, TermProgress};
use std::fs;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

/// The live loop asks the terminal for the shifted character of Shift+key
/// (`REPORT_ALTERNATE_KEYS`). Without it, kitty-protocol terminals report
/// Shift+`;` as `;` + Shift and `:` acts as `;`.
#[test]
fn keyboard_enhancement_asks_for_the_shifted_key() {
    let flags = keyboard_enhancement_flags();
    assert!(
        flags.contains(KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS),
        "{flags:?}"
    );
    assert_eq!(flags.bits(), 0b1111, "{flags:?}");
}

/// Restore bytes as `restore_terminal` writes them, with `shown` as
/// the shown-bar flag and `pointer_set` as the resize-pointer flag.
fn restore_bytes(shown: &AtomicBool, pointer_set: &AtomicBool) -> String {
    let mut out = Vec::new();
    restore_terminal_to(
        &mut out,
        shown.swap(false, Ordering::SeqCst),
        pointer_set.swap(false, Ordering::SeqCst),
    );
    String::from_utf8(out).unwrap()
}

#[test]
fn single_repo_reload_keeps_the_upstream_rewrite() {
    let fixture = crate::testutil::upstream_rewrite_fixture(crate::testutil::RewriteKind::Dropped);
    let empty = build_workspace_snapshot(&[], &[], false, &[]);
    let next = compute_reload_repo(&fixture.root, &empty, "checkout", false);
    let row = next
        .repos
        .iter()
        .find(|row| row.repo == "checkout")
        .expect("reloaded row");
    let rewrite = row.upstream_rewrite.as_ref().expect("rewrite survives");
    assert_eq!((rewrite.dropped, rewrite.yours), (2, 0));
    assert_eq!(row.sync_note, "diverged (ahead 2, behind 1)");
}

#[test]
fn restore_sequence_clears_progress_first() {
    static SHOWN: AtomicBool = AtomicBool::new(false);
    static NO_POINTER: AtomicBool = AtomicBool::new(false);
    let mut progress = TermProgress::with_flag(true, &SHOWN);
    let mut sink = Vec::new();

    let idle = restore_bytes(&SHOWN, &NO_POINTER);
    assert!(
        !idle.contains("\x1b]9;4;"),
        "no bar shown: no OSC 9;4 at all: {idle:?}"
    );
    assert!(
        idle.contains("\x1b[?1049l"),
        "leaves the alt screen: {idle:?}"
    );
    assert!(idle.contains("\x1b[?25h"), "shows the cursor: {idle:?}");

    progress
        .write_if_changed(&mut sink, ProgressState::Normal(50))
        .unwrap();
    let shown = restore_bytes(&SHOWN, &NO_POINTER);
    assert!(
        shown.starts_with(PROGRESS_CLEAR),
        "a shown bar is cleared first: {shown:?}"
    );
    assert_eq!(shown, format!("{PROGRESS_CLEAR}{idle}"));

    progress
        .write_if_changed(&mut sink, ProgressState::Error(50))
        .unwrap();
    progress
        .write_if_changed(&mut sink, ProgressState::Clear)
        .unwrap();
    assert_eq!(
        restore_bytes(&SHOWN, &NO_POINTER),
        idle,
        "a batch that already cleared its bar writes no second clear"
    );
}

/// Every exit (quit, an error return, the panic hook) and the editor
/// suspend close a synchronized update before they leave the
/// alternate screen, whether or not one is open.
#[test]
fn restore_closes_a_synchronized_update_before_leaving_the_alt_screen() {
    static NO_BAR: AtomicBool = AtomicBool::new(false);
    static NO_POINTER: AtomicBool = AtomicBool::new(false);
    const SYNC_END: &str = "\x1b[?2026l";
    const LEAVE_ALT: &str = "\x1b[?1049l";
    let closes_then_leaves = |out: &str| {
        matches!(
            (out.find(SYNC_END), out.find(LEAVE_ALT)),
            (Some(end), Some(leave)) if end < leave
        )
    };

    let restore = restore_bytes(&NO_BAR, &NO_POINTER);
    assert!(
        closes_then_leaves(&restore),
        "restore ends the update, then leaves: {restore:?}"
    );

    let mut suspend = Vec::new();
    leave_alternate_screen_to(&mut suspend).unwrap();
    let suspend = String::from_utf8(suspend).unwrap();
    assert_eq!(
        suspend,
        format!("{SYNC_END}{LEAVE_ALT}"),
        "the editor suspend ends the update, then leaves"
    );
}

#[test]
fn restore_resets_the_pointer_only_after_a_resize_shape() {
    static NO_BAR: AtomicBool = AtomicBool::new(false);
    static POINTER: AtomicBool = AtomicBool::new(false);
    let mut pointer = TermPointer::with_flag(&POINTER);
    let mut sink = Vec::new();

    let idle = restore_bytes(&NO_BAR, &POINTER);
    assert!(
        !idle.contains("\x1b]22;"),
        "no resize shape shown: no OSC 22 at all: {idle:?}"
    );

    pointer
        .write_if_changed(&mut sink, PointerShape::EwResize)
        .unwrap();
    assert_eq!(
        restore_bytes(&NO_BAR, &POINTER),
        format!("{POINTER_RESET}{idle}"),
        "a shown resize shape is reset before the terminal leaves"
    );
    assert_eq!(
        restore_bytes(&NO_BAR, &POINTER),
        idle,
        "the exit reset runs once"
    );

    pointer
        .write_if_changed(&mut sink, PointerShape::NsResize)
        .unwrap();
    pointer
        .write_if_changed(&mut sink, PointerShape::Default)
        .unwrap();
    assert_eq!(
        restore_bytes(&NO_BAR, &POINTER),
        idle,
        "a pointer already back on the default writes no second reset"
    );

    pointer
        .write_if_changed(&mut sink, PointerShape::NsResize)
        .unwrap();
    pointer.reset(&mut sink).unwrap();
    assert_eq!(
        restore_bytes(&NO_BAR, &POINTER),
        idle,
        "a suspend reset leaves nothing for the exit restore"
    );
}

/// Test writer that appends to a shared buffer.
struct SharedBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl Write for SharedBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn panic_hook_keeps_worker_panics_off_the_screen_and_prints_other_threads() {
    use crate::tui::effect::{run_job, JobOutcome};
    use std::sync::atomic::AtomicUsize;

    const WORKER_PANIC: &str = "boom in a worker job (hook test)";
    const OTHER_PANIC: &str = "boom on a helper thread (hook test)";
    static NO_BAR: AtomicBool = AtomicBool::new(false);
    static NO_POINTER: AtomicBool = AtomicBool::new(false);
    static PRINTED_WORKER: AtomicUsize = AtomicUsize::new(0);
    static PRINTED_OTHER: AtomicUsize = AtomicUsize::new(0);

    let _serial = panic_hook_lock();
    // The previous hook counts the panics of this test that reach it;
    // panics of tests that run at the same time do not count.
    std::panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let text = payload
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| payload.downcast_ref::<String>().map(String::as_str));
        match text {
            Some(WORKER_PANIC) => PRINTED_WORKER.fetch_add(1, Ordering::SeqCst),
            Some(OTHER_PANIC) => PRINTED_OTHER.fetch_add(1, Ordering::SeqCst),
            _ => 0,
        };
    }));
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&seen);
    install_panic_hook(&NO_BAR, &NO_POINTER, move || {
        SharedBuf(std::sync::Arc::clone(&sink))
    });
    TERMINAL_SET_UP.store(true, Ordering::SeqCst);

    let worker =
        thread::spawn(|| run_job(Box::new(|| -> JobOutcome { panic!("{}", WORKER_PANIC) })))
            .join()
            .expect("run_job catches the panic");
    let other = thread::spawn(|| panic!("{}", OTHER_PANIC)).join();

    TERMINAL_SET_UP.store(false, Ordering::SeqCst);
    // Put the default hook back for the other tests.
    drop(std::panic::take_hook());

    let message = worker.err().expect("the job panicked");
    let at = message
        .strip_prefix(&format!("{WORKER_PANIC} at "))
        .unwrap_or_else(|| panic!("panic message, then where: {message:?}"));
    assert!(
        at.starts_with(&format!("{}:", file!())),
        "the location names this file: {at:?}"
    );
    assert_eq!(
        PRINTED_WORKER.load(Ordering::SeqCst),
        0,
        "a worker panic is not printed into the alternate screen"
    );
    assert!(other.is_err(), "the helper thread panicked");
    assert_eq!(
        PRINTED_OTHER.load(Ordering::SeqCst),
        1,
        "a panic on a thread that is not a worker job still prints"
    );
    assert!(
        seen.lock().unwrap().is_empty(),
        "no panic off the TUI thread restores the terminal"
    );
}

#[test]
fn quit_runtime_shutdown_does_not_wait_for_a_long_blocking_job() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build()
        .expect("current-thread runtime");
    let (started_tx, started_rx) = mpsc::channel();
    rt.spawn_blocking(move || {
        started_tx.send(()).unwrap();
        thread::sleep(Duration::from_secs(10));
    });
    started_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the blocking job started");
    // The real wait reads a process-wide count that other tests may hold;
    // this test checks only that the wait runs, and that it runs first.
    let waited = AtomicBool::new(false);
    let start = std::time::Instant::now();
    shut_down_runtime(rt, RUNTIME_SHUTDOWN_GRACE, || {
        assert!(
            start.elapsed() < RUNTIME_SHUTDOWN_GRACE,
            "the user-work wait runs before the worker grace"
        );
        waited.store(true, Ordering::SeqCst);
    });
    assert!(
        waited.load(Ordering::SeqCst),
        "quit waits for stashed changes to come back"
    );
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "quit waited {:?} for a blocking job",
        start.elapsed()
    );
}

#[test]
fn panic_hook_restores_terminal_and_clears_progress() {
    let _serial = panic_hook_lock();
    static SHOWN: AtomicBool = AtomicBool::new(false);
    static POINTER: AtomicBool = AtomicBool::new(false);
    TermProgress::with_flag(true, &SHOWN)
        .write_if_changed(&mut Vec::new(), ProgressState::Normal(0))
        .unwrap();
    TermPointer::with_flag(&POINTER)
        .write_if_changed(&mut Vec::new(), PointerShape::EwResize)
        .unwrap();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = std::sync::Arc::clone(&seen);
    install_panic_hook(&SHOWN, &POINTER, move || {
        SharedBuf(std::sync::Arc::clone(&sink))
    });
    TERMINAL_SET_UP.store(true, Ordering::SeqCst);
    let caught = std::panic::catch_unwind(|| panic!("boom in the TUI thread"));
    // Put the default hook back for the other tests.
    drop(std::panic::take_hook());
    assert!(caught.is_err());
    let out = String::from_utf8(seen.lock().unwrap().clone()).unwrap();
    assert!(
        out.starts_with(PROGRESS_CLEAR),
        "hook clears the shown bar first: {out:?}"
    );
    assert!(
        out.starts_with(&format!("{PROGRESS_CLEAR}{POINTER_RESET}")),
        "hook resets the resize pointer next: {out:?}"
    );
    assert!(out.contains("\x1b[?1049l"), "hook leaves the alt screen");
    assert!(
        out.find("\x1b[?2026l")
            .is_some_and(|end| out.find("\x1b[?1049l").is_some_and(|leave| end < leave)),
        "hook ends any synchronized update before it leaves: {out:?}"
    );
    assert!(out.contains("\x1b[?25h"), "hook shows the cursor");
    assert!(
        !TERMINAL_SET_UP.load(Ordering::SeqCst),
        "the hook restores once"
    );
    assert!(!SHOWN.load(Ordering::SeqCst), "the bar is gone");
    assert!(!POINTER.load(Ordering::SeqCst), "the pointer is reset");
}

/// Press-only CSI-u `gg` through the live loop's order: the TTY decode,
/// the stale-chord expiry, the echo filter, the key map, and dispatch,
/// each at the press's arrival instant.
///
/// A second press 5 ms after the first is the echo of that tap. One
/// 120 ms later completes `gg`. One 401 ms later arms a new chord.
#[test]
fn press_only_gg_runs_the_loop_decode_on_arrival_instants() {
    use crate::tui::event_loop::decode_input;
    use crate::tui::tty::parse_tty_chunk;
    use std::time::Instant;

    let press = parse_tty_chunk(b"\x1b[103;1:1u");
    let [(g, origin)] = press.as_slice() else {
        panic!("one press-only g: {press:?}");
    };
    let fresh = || {
        let mut app = app_state(&[repo_snapshot("app"), repo_snapshot("lib")]);
        app.cursor = app.rows.len() - 1;
        assert_ne!(app.cursor, 0, "the cursor starts off the workspace root");
        app
    };
    let key_at = |app: &mut AppState, at: Instant| {
        app.begin_input(at);
        let action = decode_input(app, g, *origin, at);
        if let Some(action) = action.clone() {
            app.dispatch(action);
        }
        app.end_input();
        action
    };

    let t0 = crate::testutil::instant_ago(Duration::from_secs(5));
    let mut app = fresh();
    assert_eq!(key_at(&mut app, t0), Some(Action::ArmGChord));
    assert_eq!(
        key_at(&mut app, t0 + Duration::from_millis(5)),
        None,
        "inside the echo window: the tap's own release"
    );
    assert_eq!(
        key_at(&mut app, t0 + Duration::from_millis(120)),
        Some(Action::MoveToStart)
    );
    assert_eq!(app.cursor, 0, "gg reached the workspace root");

    let mut app = fresh();
    let end = app.cursor;
    key_at(&mut app, t0);
    assert_eq!(
        key_at(&mut app, t0 + Duration::from_millis(401)),
        Some(Action::ArmGChord),
        "past the 400 ms window: a new chord, not gg"
    );
    assert_eq!(app.cursor, end);
}

#[test]
fn drop_undiscovered_checkouts_removes_vanished_paths() {
    let mut app = app_state(&[repo_snapshot("app"), repo_snapshot("linked")]);
    assert_eq!(app.snapshot.repos.len(), 2);
    drop_undiscovered_checkouts(&mut app, &["app".into()]);
    let names: Vec<&str> = app
        .snapshot
        .repos
        .iter()
        .map(|row| row.repo.as_str())
        .collect();
    assert_eq!(names, vec!["app"]);
    drop_undiscovered_checkouts(&mut app, &["app".into()]);
    assert_eq!(app.snapshot.repos.len(), 1);
}

#[test]
fn tree_picker_dirty_refuses_tracked_only() {
    let root = unique_dir("ws-tui-checkout-dirty");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    git(&repo_dir, &["checkout", "-q", "-b", "feature/x"]);
    git(&repo_dir, &["checkout", "-q", "main"]);
    fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
    fs::write(repo_dir.join("untracked.txt"), "u\n").unwrap();

    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    app.open_branch_picker("app".into(), list_local_branches(&repo_dir));
    app.dispatch(Action::BranchChar('f'));
    match app.dispatch(Action::BranchSubmit) {
        Effect::CheckoutBranch {
            repo,
            selected_name,
            fast_forward_ref,
        } => {
            assert_eq!(selected_name, "feature/x");
            assert!(fast_forward_ref.is_none());
            assert!(!run_checkout_branch(
                &mut app,
                &workspace,
                repo,
                selected_name,
                fast_forward_ref,
            ));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(app.status, DIRTY_WORKTREE_STATUS);
    assert!(app.branch_picker.is_some());
    assert!(app.confirm.is_none());
    assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "main");

    git(&repo_dir, &["checkout", "-q", "--", "README.md"]);
    assert!(!repo_has_local_changes(&repo_dir));
    app.open_branch_picker("app".into(), list_local_branches(&repo_dir));
    app.dispatch(Action::BranchChar('f'));
    match app.dispatch(Action::BranchSubmit) {
        Effect::CheckoutBranch {
            repo,
            selected_name,
            fast_forward_ref,
        } => {
            assert!(run_checkout_branch(
                &mut app,
                &workspace,
                repo,
                selected_name,
                fast_forward_ref,
            ));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        exec_git(&["branch", "--show-current"], &repo_dir),
        "feature/x"
    );
    assert!(repo_dir.join("untracked.txt").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn origin_selection_confirms_then_ff_only_local_does_not() {
    let root = unique_dir("ws-tui-checkout-ff");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    let remote = root.join("remote.git");
    Command::new(git_binary())
        .args(["init", "-q", "--bare", remote.to_str().unwrap()])
        .status()
        .unwrap();
    git(
        &repo_dir,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&repo_dir, &["push", "-u", "origin", "main", "--quiet"]);
    git(&repo_dir, &["checkout", "-q", "-b", "foo"]);
    git(&repo_dir, &["push", "-u", "origin", "foo", "--quiet"]);
    git(&repo_dir, &["checkout", "-q", "main"]);
    let other = root.join("other");
    Command::new(git_binary())
        .args([
            "clone",
            "-q",
            remote.to_str().unwrap(),
            other.to_str().unwrap(),
        ])
        .status()
        .unwrap();
    git(&other, &["checkout", "-q", "foo"]);
    fs::write(other.join("README.md"), "# origin-ahead\n").unwrap();
    git(&other, &["add", "README.md"]);
    git(&other, &["commit", "-q", "-m", "remote"]);
    git(&other, &["push", "--quiet"]);
    git(&repo_dir, &["fetch", "--quiet"]);

    let local = exec_git(&["rev-parse", "foo"], &repo_dir);
    let remote_sha = exec_git(&["rev-parse", "origin/foo"], &repo_dir);
    assert_ne!(local, remote_sha);

    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);

    assert!(run_checkout_branch(
        &mut app,
        &workspace,
        "app".into(),
        "foo".into(),
        None,
    ));
    assert!(app.confirm.is_none());
    assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "foo");
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &repo_dir), local);

    git(&repo_dir, &["checkout", "-q", "main"]);
    assert!(!run_checkout_branch(
        &mut app,
        &workspace,
        "app".into(),
        "origin/foo".into(),
        None,
    ));
    assert!(matches!(
        app.confirm,
        Some(PendingConfirm::CheckoutOutOfSync {
            ahead_behind: Some((0, 1)),
            ..
        })
    ));
    match app.dispatch(Action::ConfirmYes) {
        Effect::CheckoutBranch {
            selected_name,
            fast_forward_ref,
            repo,
        } => {
            assert_eq!(selected_name, "foo");
            assert_eq!(fast_forward_ref.as_deref(), Some("origin/foo"));
            assert!(run_checkout_branch(
                &mut app,
                &workspace,
                repo,
                selected_name,
                fast_forward_ref,
            ));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "foo");
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &repo_dir), remote_sha);
    assert_eq!(app.status.kind(), StatusKind::Ok);

    // Local-only commit: the confirm counts it, and the fast-forward
    // failure says why instead of a bare "could not".
    fs::write(repo_dir.join("local.txt"), "local\n").unwrap();
    git(&repo_dir, &["add", "local.txt"]);
    git(&repo_dir, &["commit", "-q", "-m", "local"]);
    let local_ahead = exec_git(&["rev-parse", "HEAD"], &repo_dir);
    git(&repo_dir, &["checkout", "-q", "main"]);
    assert!(!run_checkout_branch(
        &mut app,
        &workspace,
        "app".into(),
        "origin/foo".into(),
        None,
    ));
    assert!(matches!(
        app.confirm,
        Some(PendingConfirm::CheckoutOutOfSync {
            ahead_behind: Some((1, 0)),
            ..
        })
    ));
    match app.dispatch(Action::ConfirmYes) {
        Effect::CheckoutBranch {
            selected_name,
            fast_forward_ref,
            repo,
        } => assert!(run_checkout_branch(
            &mut app,
            &workspace,
            repo,
            selected_name,
            fast_forward_ref,
        )),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        app.status,
        "Checked out foo; could not fast-forward to origin/foo: local has commits origin/foo lacks"
    );
    assert_eq!(app.status.kind(), StatusKind::Warn);
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &repo_dir), local_ahead);
    let _ = fs::remove_dir_all(&root);
}

/// Status of the confirm-Yes checkout (`fast_forward_ref: origin/main`)
/// on the fixture checkout, and its tip after it.
fn checkout_after_rewrite(kind: crate::testutil::RewriteKind) -> (StatusMessage, bool) {
    let fixture = crate::testutil::upstream_rewrite_fixture(kind);
    let status = match compute_checkout(&fixture.checkout, "main", Some("origin/main")) {
        CheckoutCompute::Done { status } => status,
        _ => panic!("checkout must finish"),
    };
    let tip = exec_git(&["rev-parse", "HEAD"], &fixture.checkout);
    (status, tip == fixture.local_tip)
}

#[test]
fn checkout_ff_failure_on_a_rewritten_upstream_points_to_p() {
    use crate::testutil::RewriteKind;

    let (status, kept) = checkout_after_rewrite(RewriteKind::Dropped);
    assert_eq!(
            status,
            "Checked out main; origin/main was force-pushed (2 dropped, none yours) — press p to move main to origin/main"
        );
    assert_eq!(status.kind(), StatusKind::Warn);
    assert!(kept, "the checkout never resets");

    let (status, kept) = checkout_after_rewrite(RewriteKind::YourCommit);
    assert_eq!(
            status,
            "Checked out main; origin/main was force-pushed under your 1 commit — press p for the rebase command"
        );
    assert_eq!(status.kind(), StatusKind::Warn);
    assert!(kept);

    // A normal divergence keeps the plain reason.
    let (status, kept) = checkout_after_rewrite(RewriteKind::NormalDivergence);
    assert_eq!(
            status,
            "Checked out main; could not fast-forward to origin/main: local has commits origin/main lacks"
        );
    assert!(kept);
}

#[test]
fn graph_merge_dirty_refuses_then_ff_and_conflict_stay_uncommitted() {
    let root = unique_dir("ws-tui-merge");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    git(&repo_dir, &["config", "user.name", "workspace-status test"]);
    git(
        &repo_dir,
        &[
            "config",
            "user.email",
            "workspace-status-test@example.invalid",
        ],
    );
    git(&repo_dir, &["checkout", "-q", "-b", "topic"]);
    fs::write(repo_dir.join("topic.txt"), "topic\n").unwrap();
    git(&repo_dir, &["add", "topic.txt"]);
    git(&repo_dir, &["commit", "-q", "-m", "topic"]);
    let topic = exec_git(&["rev-parse", "HEAD"], &repo_dir);
    git(&repo_dir, &["checkout", "-q", "main"]);
    fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
    fs::write(repo_dir.join("untracked.txt"), "u\n").unwrap();

    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    assert!(!run_merge_into_head(
        &mut app,
        &workspace,
        "app".into(),
        topic.clone(),
        "topic".into(),
    ));
    assert_eq!(app.status, DIRTY_WORKTREE_STATUS);
    assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "main");
    assert_ne!(exec_git(&["rev-parse", "HEAD"], &repo_dir), topic);

    git(&repo_dir, &["checkout", "-q", "--", "README.md"]);
    assert!(!repo_has_local_changes(&repo_dir));
    assert!(run_merge_into_head(
        &mut app,
        &workspace,
        "app".into(),
        topic.clone(),
        "topic".into(),
    ));
    assert_eq!(app.status, "Fast-forwarded to topic");
    assert_eq!(exec_git(&["rev-parse", "HEAD"], &repo_dir), topic);
    assert!(repo_dir.join("untracked.txt").exists());

    git(&repo_dir, &["reset", "--hard", "--quiet", "HEAD~1"]);
    fs::write(repo_dir.join("README.md"), "# main-side\n").unwrap();
    git(&repo_dir, &["add", "README.md"]);
    git(&repo_dir, &["commit", "-q", "-m", "main-side"]);
    git(&repo_dir, &["checkout", "-q", "-B", "other", "HEAD~1"]);
    fs::write(repo_dir.join("README.md"), "# other-side\n").unwrap();
    git(&repo_dir, &["add", "README.md"]);
    git(&repo_dir, &["commit", "-q", "-m", "other-side"]);
    let other = exec_git(&["rev-parse", "HEAD"], &repo_dir);
    git(&repo_dir, &["checkout", "-q", "main"]);
    assert!(run_merge_into_head(
        &mut app,
        &workspace,
        "app".into(),
        other,
        "other".into(),
    ));
    assert_eq!(app.status, "Merge conflict — resolve in the worktree");
    assert!(rev_parse_quiet("MERGE_HEAD", &repo_dir).is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn right_pane_load_types_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<RightPaneRequest>();
    assert_send::<RightPaneLoad>();
}

#[test]
fn compute_right_pane_load_fills_graph_for_a_repo_row() {
    let root = unique_dir("ws-tui-pane-load");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    let idx = app
        .rows
        .iter()
        .position(|r| {
            r.kind == super::super::tree::NodeKind::Repo && r.repo.as_deref() == Some("app")
        })
        .expect("visible app repo row");
    app.cursor = idx;
    load_right_headless(&mut app);
    assert!(
        app.graph.is_some(),
        "repo row must load a graph from pane git"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn compute_right_pane_load_follows_graph_row_at_files_depth() {
    use super::super::drill::{CommitFileSource, DrillView};
    use super::super::state::FocusPane;

    let root = unique_dir("ws-tui-pane-follow");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    fs::write(repo_dir.join("second.txt"), "two\n").unwrap();
    git(&repo_dir, &["add", "second.txt"]);
    git(&repo_dir, &["commit", "-q", "-m", "second"]);
    git(&repo_dir, &["checkout", "-q", "-b", "feature/follow"]);
    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    let idx = app
        .rows
        .iter()
        .position(|r| {
            r.kind == super::super::tree::NodeKind::Repo && r.repo.as_deref() == Some("app")
        })
        .expect("visible app repo row");
    app.cursor = idx;
    load_right_headless(&mut app);
    assert!(app.graph.is_some());
    app.open_commit_files("app".into(), CommitFileSource::Worktree, Vec::new());
    app.focus = FocusPane::Left;
    let files_before = match &app.drill {
        DrillView::Files { files, .. } => files.clone(),
        other => panic!("expected files drill, got {other:?}"),
    };
    assert!(files_before.is_empty());
    app.graph_cursor = 1;
    load_right_headless(&mut app);
    assert_eq!(
        app.focus,
        FocusPane::Left,
        "follow must not steal left focus"
    );
    assert!(app.drill.is_files());
    let files_after = match &app.drill {
        DrillView::Files { files, .. } => files.clone(),
        other => panic!("expected files drill, got {other:?}"),
    };
    assert!(
        files_after.iter().any(|file| file.path == "second.txt"),
        "HEAD commit should list second.txt, got {files_after:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn event_thread_keeps_polling_while_right_pane_git_runs() {
    let root = unique_dir("ws-tui-pane-pump");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    let idx = app
        .rows
        .iter()
        .position(|r| {
            r.kind == super::super::tree::NodeKind::Repo && r.repo.as_deref() == Some("app")
        })
        .expect("visible app repo row");
    app.cursor = idx;
    let request = RightPaneRequest::from_state(&app);
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        let payload = request.compute();
        let _ = tx.send(payload);
    });
    let mut pumps = 0u32;
    let payload = loop {
        match rx.try_recv() {
            Ok(value) => break value,
            Err(mpsc::TryRecvError::Empty) => {
                pumps += 1;
                thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                panic!("right-pane worker disconnected")
            }
        }
    };
    assert!(
        pumps >= 5,
        "draw thread must poll while pane git runs (got {pumps} pumps); \
             inline load_right after fetch would stall until git log returns, \
             then crossterm would flush queued keys in a burst"
    );
    apply_right_pane_load(&mut app, payload);
    assert!(app.graph.is_some());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn checkout_and_merge_compute_are_send() {
    fn assert_send<T: Send>() {}
    assert_send::<CheckoutCompute>();
    assert_send::<MergeCompute>();
}

#[test]
fn event_thread_keeps_polling_while_git_write_runs() {
    let root = unique_dir("ws-tui-write-pump");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
    let dir = repo_dir.clone();
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        let result = stage_file(&dir, "README.md");
        let _ = tx.send(result);
    });
    let mut pumps = 0u32;
    let result = loop {
        match rx.try_recv() {
            Ok(value) => break value,
            Err(mpsc::TryRecvError::Empty) => {
                pumps += 1;
                thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                panic!("git-write worker disconnected")
            }
        }
    };
    assert!(
        pumps >= 5,
        "draw thread must poll while git add runs (got {pumps} pumps); \
             a sync stage_file on the loop thread freezes the TUI until the child exits"
    );
    result.expect("stage README.md");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn busy_wait_applies_nav_before_write_finishes() {
    let root = unique_dir("ws-tui-busy-nav");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    assert!(
        app.rows.len() >= 2,
        "fixture must have more than one tree row"
    );
    app.cursor = 0;
    let start = app.cursor;
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(80));
        let _ = tx.send(());
    });
    let mut moved_during = false;
    loop {
        match rx.try_recv() {
            Ok(()) => break,
            Err(mpsc::TryRecvError::Empty) => {
                let _ = app.dispatch(Action::Move(1));
                if app.cursor != start {
                    moved_during = true;
                }
                thread::sleep(Duration::from_millis(5));
            }
            Err(mpsc::TryRecvError::Disconnected) => panic!("nav worker disconnected"),
        }
    }
    assert!(
        moved_during,
        "nav must apply during the worker, not after join (BusyAction::Handle)"
    );
    assert_ne!(app.cursor, start);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn unchanged_watch_signatures_let_the_loop_skip_pane_git() {
    let root = unique_dir("ws-tui-watch-skip");
    let workspace = root.join("workspace");
    let repo_dir = workspace.join("app");
    init_repo(&repo_dir);
    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot.clone(), true);
    let before = app.signatures.clone();
    assert!(
        !apply_watch_snapshot_for_tick(&mut app, snapshot),
        "identical watch snapshot must skip load_right"
    );
    assert_eq!(
        before, app.signatures,
        "identical watch snapshot must keep signatures so load_right can be skipped"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn watch_tick_reloads_when_head_moves_without_tree_chrome_flip() {
    let root = unique_dir("ws-tui-watch-head");
    let workspace = root.join("workspace");
    let alpha = workspace.join("alpha");
    let beta = workspace.join("beta");
    init_repo(&alpha);
    init_repo(&beta);
    git(&alpha, &["checkout", "-q", "-b", "feature/watch"]);
    git(&beta, &["checkout", "-q", "-b", "feature/other"]);
    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    load_right_headless(&mut app);
    let before_head = app
        .snapshot
        .repos
        .iter()
        .find(|row| row.repo == "alpha")
        .map(|row| row.head.clone())
        .expect("alpha");
    let before_sigs = app.signatures.clone();

    fs::write(alpha.join("tick.txt"), "head-move\n").unwrap();
    git(&alpha, &["add", "tick.txt"]);
    git(&alpha, &["commit", "-q", "-m", "watch-head-move"]);
    let new_head = exec_git(&["rev-parse", "HEAD"], &alpha);
    assert_ne!(before_head, new_head);

    let next = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let alpha_row = next
        .repos
        .iter()
        .find(|row| row.repo == "alpha")
        .expect("alpha row");
    assert_eq!(alpha_row.branch, "feature/watch");
    assert_eq!(
        alpha_row.sync_status,
        crate::snapshot::SyncStatus::NoUpstream
    );
    assert!(alpha_row.changes.is_empty());
    assert_eq!(alpha_row.head, new_head);

    assert!(
        apply_watch_snapshot_for_tick(&mut app, next),
        "HEAD-only commit must not skip the pane reload"
    );
    assert_ne!(before_sigs, app.signatures);
    assert_eq!(
        app.snapshot
            .repos
            .iter()
            .find(|row| row.repo == "alpha")
            .map(|row| row.head.as_str()),
        Some(new_head.as_str())
    );

    fs::write(beta.join("dirty.txt"), "flash me\n").unwrap();
    let dirty = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let changed = app.apply_watch_snapshot(dirty);
    assert!(
        changed.iter().any(|id| id.contains("dirty.txt")),
        "dirty file on the other repo must flash: {changed:?}"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn watch_tick_reloads_when_ahead_count_moves() {
    let root = unique_dir("ws-tui-watch-ahead");
    let workspace = root.join("workspace");
    let remote = root.join("remote.git");
    let tracker = workspace.join("tracker");
    let sidecar = workspace.join("sidecar");
    fs::create_dir_all(&workspace).unwrap();
    Command::new(git_binary())
        .args(["init", "-q", "--bare", remote.to_str().unwrap()])
        .status()
        .unwrap();
    init_repo(&tracker);
    git(
        &tracker,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&tracker, &["push", "-u", "origin", "main", "--quiet"]);
    init_repo(&sidecar);
    git(&sidecar, &["checkout", "-q", "-b", "feature/sidecar"]);
    for i in 1..=2 {
        fs::write(tracker.join("count.txt"), format!("{i}\n")).unwrap();
        git(&tracker, &["add", "count.txt"]);
        git(&tracker, &["commit", "-q", "-m", &format!("ahead {i}")]);
    }
    let config = WorkspaceStatusConfig::with_defaults();
    let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let tracker_row = snapshot
        .repos
        .iter()
        .find(|row| row.repo == "tracker")
        .expect("tracker");
    assert_eq!(tracker_row.sync_status, crate::snapshot::SyncStatus::Ahead);
    assert!(
        tracker_row.sync_note.contains("ahead by 2"),
        "{}",
        tracker_row.sync_note
    );
    let mut app = AppState::new(workspace.clone(), snapshot, true);
    fs::write(tracker.join("count.txt"), "3\n").unwrap();
    git(&tracker, &["add", "count.txt"]);
    git(&tracker, &["commit", "-q", "-m", "ahead 3"]);
    let next = collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
    let after = next
        .repos
        .iter()
        .find(|row| row.repo == "tracker")
        .expect("tracker after");
    assert_eq!(after.sync_status, crate::snapshot::SyncStatus::Ahead);
    assert!(
        after.sync_note.contains("ahead by 3"),
        "{}",
        after.sync_note
    );
    assert!(
        apply_watch_snapshot_for_tick(&mut app, next),
        "ahead 2→3 must not skip the pane reload"
    );
    assert_eq!(
        app.snapshot
            .repos
            .iter()
            .find(|row| row.repo == "tracker")
            .map(|row| row.sync_note.as_str()),
        Some("ahead by 3 commits")
    );
    let _ = fs::remove_dir_all(&root);
}

fn commit_file(dir: &Path, name: &str) -> String {
    fs::write(dir.join(name), format!("{name}\n")).unwrap();
    git(dir, &["add", name]);
    git(dir, &["commit", "-q", "-m", name]);
    exec_git(&["rev-parse", "HEAD"], dir)
}

fn compare_paths(load: &CompareRangeLoad) -> Vec<(String, String)> {
    load.files
        .iter()
        .map(|file| (file.status.clone(), file.path.clone()))
        .collect()
}

#[test]
fn pinned_head_compares_a_commit_with_its_first_parent() {
    let dir = crate::testutil::unique_dir("ws-compare-pinned");
    init_repo(&dir);
    let first = commit_file(&dir, "a.txt");
    git(&dir, &["checkout", "-q", "-b", "topic"]);
    commit_file(&dir, "topic.txt");
    git(&dir, &["checkout", "-q", "main"]);
    let main_tip = commit_file(&dir, "main.txt");
    git(
        &dir,
        &["merge", "--no-ff", "-q", "-m", "merge topic", "topic"],
    );
    let merge = exec_git(&["rev-parse", "HEAD"], &dir);
    // HEAD moves past both commits: a pinned head ignores it.
    let later = commit_file(&dir, "later.txt");

    let load = compute_compare_range(&dir, &format!("{first}^"), &first).unwrap();
    assert_eq!(compare_paths(&load), vec![("A".into(), "a.txt".into())]);
    assert_eq!(load.head.as_deref(), Some(first.as_str()));
    match &load.source {
        CommitFileSource::Compare {
            base_ref,
            head_ref,
            head,
            merge_base,
            ..
        } => {
            assert_eq!(base_ref, &format!("{first}^"));
            assert_eq!(head_ref, &first);
            assert_eq!(head, &first);
            assert_eq!(merge_base, &load.base_tip, "parent is the merge base");
        }
        other => panic!("{other:?}"),
    }
    // One commit against its parent: the load reads that commit for
    // the footer, with its message, parent, and refs.
    let commit = load.commit.as_ref().expect("the head commit");
    assert_eq!(commit.id, first);
    assert_eq!(commit.subject, "a.txt");
    assert_eq!(commit.parents, vec![load.base_tip.clone()]);

    // A merge shows what it brought in versus its first parent.
    let load = compute_compare_range(&dir, &format!("{merge}^"), &merge).unwrap();
    assert_eq!(load.base_tip, main_tip);
    let commit = load.commit.as_ref().expect("the merge commit");
    assert_eq!(commit.subject, "merge topic");
    assert_eq!(commit.parents.len(), 2);
    assert_eq!(compare_paths(&load), vec![("A".into(), "topic.txt".into())]);

    // A live tab still reads the checkout's HEAD.
    let live = compute_compare_range(&dir, &first, "HEAD").unwrap();
    assert!(live.commit.is_none(), "a range has no single commit");
    let tip = compute_compare_range(&dir, &format!("{later}^"), &later).unwrap();
    let refs = tip.commit.map(|commit| commit.refs).unwrap_or_default();
    assert!(
        refs.iter().any(|r| r.name == "main"),
        "branch tip refs: {refs:?}"
    );
    let mut paths: Vec<_> = compare_paths(&live).into_iter().map(|(_, p)| p).collect();
    paths.sort();
    assert_eq!(paths, vec!["later.txt", "main.txt", "topic.txt"]);

    assert_eq!(
        compute_compare_range(&dir, "main", &"0".repeat(40)).err(),
        Some(head_ref_not_found(&"0".repeat(40)))
    );
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn worktree_range_lists_and_diffs_the_one_file_against_the_commit() {
    let dir = crate::testutil::unique_dir("ws-compare-worktree");
    init_repo(&dir);
    let base = commit_file(&dir, "old.txt");
    commit_file(&dir, "other.txt");
    let file = WorktreeFile {
        path: "old.txt".into(),
        old_path: None,
    };
    let clean = compute_worktree_compare_range(&dir, &base, &file).unwrap();
    assert!(clean.files.is_empty(), "disk equals the commit");

    // Renamed and edited after `base`; other.txt changes are not listed.
    git(&dir, &["mv", "old.txt", "new.txt"]);
    fs::write(dir.join("new.txt"), "old.txt\nmore\n").unwrap();
    fs::write(dir.join("other.txt"), "changed\n").unwrap();
    let file = WorktreeFile {
        path: "new.txt".into(),
        old_path: Some("old.txt".into()),
    };
    let load = compute_worktree_compare_range(&dir, &base, &file).unwrap();
    assert_eq!(compare_paths(&load), vec![("R".into(), "new.txt".into())]);
    assert_eq!(
        load.source,
        CommitFileSource::CommitVsWorktree {
            base: base.clone(),
            path: "new.txt".into(),
            old_path: Some("old.txt".into()),
        }
    );
    let content =
        compute_compare_diff(&dir, &load.source, "new.txt", Some("old.txt"), None).unwrap();
    assert!(content.vs_worktree && !content.is_committed);
    assert!(content.unstaged.contains("+more"), "{content:?}");

    assert_eq!(
        compute_worktree_compare_range(&dir, &"0".repeat(40), &file).err(),
        Some(base_ref_not_found(&"0".repeat(40)))
    );
    let _ = fs::remove_dir_all(&dir);
}

/// A depth-2 commit or stash drill diff labels its one section
/// `COMMITTED`, as a compare diff does. `is_committed` stays false, so
/// `build_partial_patch` answers every kind and range exactly as for
/// the plain unstaged text of the same lines, and each row keeps the
/// same line reference (blame and go to line read it).
#[test]
fn commit_and_stash_diffs_label_committed_and_keep_partial_patch_kinds() {
    use crate::tui::diff::{
        build_diff_rows, build_partial_patch, row_line_ref, DiffRow, DiffSection, PartialPatchKind,
    };
    use crate::tui::split::DiffMode;
    let cwd = crate::testutil::unique_dir("ws-d2-committed-label");
    let dir = cwd.join("app");
    init_repo(&dir);
    fs::write(dir.join("README.md"), "# seed\nold line\nkeep\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "base"]);
    fs::write(dir.join("README.md"), "# seed\nnew line\nkeep\nadded\n").unwrap();
    git(&dir, &["commit", "-q", "-am", "edit"]);
    fs::write(dir.join("README.md"), "# seed\nnew line\nstashed\n").unwrap();
    git(&dir, &["stash", "push", "-q"]);
    let cases = [
        (
            CommitFileSource::Commit {
                commit_id: exec_git(&["rev-parse", "HEAD"], &dir),
            },
            diff_commit_file_ctx(&dir, "HEAD", "README.md", None),
        ),
        (
            CommitFileSource::Stash {
                stash_ref: "stash@{0}".into(),
            },
            diff_stash_file_ctx(&dir, "stash@{0}", "README.md", None),
        ),
    ];
    for (source, lines) in cases {
        let content = compute_commit_diff(&cwd, "app", &source, "README.md", None, None);
        let plain = DiffContent::from_lines(lines);
        assert!(!content.is_committed, "{source:?}");
        assert_eq!(content.unstaged, plain.unstaged, "{source:?}");
        for mode in [DiffMode::Inline, DiffMode::SideBySide] {
            let rows = build_diff_rows(&content, mode);
            assert_eq!(
                rows.first(),
                Some(&DiffRow::Section(DiffSection::Committed)),
                "{source:?} {mode:?}"
            );
            assert!(
                !rows
                    .iter()
                    .any(|r| matches!(r, DiffRow::Section(DiffSection::Unstaged))),
                "{source:?} {mode:?}"
            );
            for row in 0..rows.len() {
                assert_eq!(
                    row_line_ref(&content, mode, row),
                    row_line_ref(&plain, mode, row),
                    "{source:?} {mode:?} row {row}"
                );
            }
            for kind in [
                PartialPatchKind::Stage,
                PartialPatchKind::Unstage,
                PartialPatchKind::Revert,
                PartialPatchKind::RevertCommitted,
            ] {
                for start in 0..rows.len() {
                    for end in start..rows.len() {
                        assert_eq!(
                            build_partial_patch(&content, mode, start, end, kind, "README.md"),
                            build_partial_patch(&plain, mode, start, end, kind, "README.md"),
                            "{source:?} {mode:?} {kind:?} {start}..={end}"
                        );
                    }
                }
            }
        }
    }
    let _ = fs::remove_dir_all(&cwd);
}

#[test]
fn probe_ignores_head_moves_for_a_pinned_head() {
    let dir = crate::testutil::unique_dir("ws-probe-pinned");
    init_repo(&dir);
    let pinned = commit_file(&dir, "a.txt");
    let base = format!("{pinned}^");
    let parent = exec_git(&["rev-parse", &base], &dir);
    commit_file(&dir, "next.txt");
    let (changed, head, base_tip) =
        probe_compare_range(&dir, &base, &pinned, Some(&pinned), Some(&parent)).unwrap();
    assert!(!changed, "HEAD moved but the pinned endpoints did not");
    assert_eq!(head.as_deref(), Some(pinned.as_str()));
    assert_eq!(base_tip.as_deref(), Some(parent.as_str()));
    let (changed, _, _) =
        probe_compare_range(&dir, &base, "HEAD", Some(&pinned), Some(&parent)).unwrap();
    assert!(changed, "a live tab on the same base reloads");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn probe_without_recorded_sha_is_not_a_change() {
    let dir = unique_dir("ws-probe-none");
    init_repo(&dir);
    let head = exec_git(&["rev-parse", "HEAD"], &dir);
    let (changed, got_head, got_base) =
        probe_compare_range(&dir, "HEAD", "HEAD", None, None).unwrap();
    assert!(!changed, "missing recorded SHAs must not force a reload");
    assert_eq!(got_head.as_deref(), Some(head.as_str()));
    assert_eq!(got_base.as_deref(), Some(head.as_str()));
    let (changed, _, _) = probe_compare_range(
        &dir,
        "HEAD",
        "HEAD",
        Some(head.as_str()),
        Some(head.as_str()),
    )
    .unwrap();
    assert!(!changed, "equal tips must not reload");
    fs::write(dir.join("dirty.txt"), "dirty\n").unwrap();
    let (changed, _, _) = probe_compare_range(
        &dir,
        "HEAD",
        "HEAD",
        Some(head.as_str()),
        Some(head.as_str()),
    )
    .unwrap();
    assert!(!changed, "dirty-only worktree must not reload compare");
    fs::write(dir.join("next.txt"), "next\n").unwrap();
    git(&dir, &["add", "next.txt"]);
    git(&dir, &["commit", "-q", "-m", "move head"]);
    let (changed, _, _) = probe_compare_range(
        &dir,
        "HEAD",
        "HEAD",
        Some(head.as_str()),
        Some(head.as_str()),
    )
    .unwrap();
    assert!(changed, "HEAD SHA change must reload");
    let _ = fs::remove_dir_all(&dir);
}

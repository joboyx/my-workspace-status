//! Event-loop helpers that keep the TUI from going deaf.
//!
//! Long git work must not sit on the draw thread. Overlay modes must not
//! start background fetch/watch ticks. Graph autoload must not run on
//! resize, mouse noise, or swallowed overlay keys.
//!
//! The live TTY loop (`event_loop.rs`) is one async `tokio::select!` over
//! terminal input, watch/fetch deadlines, `JoinSet` completions, flash /
//! Ctrl-C, and presentation. Every git/process effect runs on
//! `spawn_blocking`. Fetch / pull / push of independent checkouts share the
//! same cap (`env_fetch_concurrency`, default 4). Watch collect streams
//! `discover_checkouts` then per-repo `process_repo` and applies each
//! [`crate::snapshot::RepoSnapshot`] as it arrives. Applying a result must
//! not call `git log` / `git diff` on the loop thread.
//!
//! While exclusive writes or a remote batch are in flight, nav / pane
//! switch / cancel / quit still dispatch (`BusyAction::Handle`). Only
//! actions that would start another git write are drained (`Ignore`).
//!
//! Held nav (`h`/`j`/`k`/`l`) maps Repeat to the same move as Press. The
//! input thread drops queued copies of that key after each move so a hold
//! cannot flush as a burst after release. Pane loads coalesce by request
//! id and [`super::app::RightPaneTarget`]; a stale result is discarded and
//! the latest target is scheduled.

use super::action::Action;
use super::keys::InputMode;

/// What to do with an event while a git subprocess owns a worker thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyAction {
    /// Drain the event. Used for actions that would start another git write.
    Ignore,
    /// Dispatch and apply (nav, pane switch, cancel, overlay typing, …).
    Handle,
    /// Relayout while the worker continues.
    Resize { cols: u16, rows: u16 },
    /// Finish the current worker, then leave the TUI.
    Quit,
}

/// Classify one dispatched action during an in-flight git op.
///
/// Writes (`f` / `p` / `s` / confirm-yes / …) are [`BusyAction::Ignore`] so
/// they cannot start a second mutating child. Everything else stays live:
/// move, click, wheel, Tab, Esc, help, search, overlay cancel.
pub fn classify_busy_action(action: &Action) -> BusyAction {
    match action {
        Action::Quit => BusyAction::Quit,
        Action::Resize { cols, rows } => BusyAction::Resize {
            cols: *cols,
            rows: *rows,
        },
        Action::Fetch
        | Action::Pull
        | Action::Push
        | Action::DefaultBranch
        | Action::Stage
        | Action::Unstage
        | Action::Revert
        | Action::ConfirmYes
        | Action::ConfirmYesClean
        | Action::Edit
        | Action::WatchTick
        | Action::FetchTick
        | Action::StashMenuEnter
        | Action::GraphStashApply
        | Action::GraphStashPop
        | Action::GraphStashDrop
        | Action::GraphCheckout
        | Action::GraphMerge
        | Action::BranchSubmit
        | Action::CreateBranchSubmit
        | Action::GraphFocusSubmit
        | Action::GraphFocusClear => BusyAction::Ignore,
        _ => BusyAction::Handle,
    }
}

/// True when fetch/watch timers must not start (confirm, help, pickers, …).
pub fn overlay_blocks_background_ticks(mode: InputMode) -> bool {
    !matches!(
        mode,
        InputMode::Normal { .. } | InputMode::ZPending { .. } | InputMode::GPending { .. }
    )
}

/// True when this action may have moved the graph cursor onto the last row.
pub fn action_triggers_graph_autoload(action: &Action) -> bool {
    matches!(
        action,
        Action::Move(_)
            | Action::PageMove(_)
            | Action::MoveToStart
            | Action::MoveToEnd
            | Action::SearchNext
            | Action::SearchPrev
            | Action::FocusRight
            | Action::NavEnter
            | Action::ScrollWheel {
                horizontal: false,
                ..
            }
            | Action::Click { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_and_noise_do_not_autoload() {
        assert!(!action_triggers_graph_autoload(&Action::None));
        assert!(!action_triggers_graph_autoload(&Action::Resize {
            cols: 80,
            rows: 24
        }));
        assert!(!action_triggers_graph_autoload(&Action::WatchTick));
        assert!(!action_triggers_graph_autoload(&Action::FetchTick));
        assert!(!action_triggers_graph_autoload(&Action::ConfirmYes));
        assert!(!action_triggers_graph_autoload(&Action::ConfirmNo));
        assert!(!action_triggers_graph_autoload(&Action::Drag {
            col: 1,
            row: 1
        }));
        assert!(action_triggers_graph_autoload(&Action::Move(1)));
        assert!(action_triggers_graph_autoload(&Action::MoveToEnd));
        assert!(action_triggers_graph_autoload(&Action::PageMove(-1)));
        assert!(action_triggers_graph_autoload(&Action::ScrollWheel {
            col: 1,
            row: 1,
            delta: 1,
            horizontal: false,
        }));
        assert!(!action_triggers_graph_autoload(&Action::ScrollWheel {
            col: 1,
            row: 1,
            delta: 1,
            horizontal: true,
        }));
    }

    #[test]
    fn overlays_block_background_ticks() {
        assert!(overlay_blocks_background_ticks(InputMode::Confirm));
        assert!(overlay_blocks_background_ticks(InputMode::Help));
        assert!(overlay_blocks_background_ticks(InputMode::HelpSearch));
        assert!(overlay_blocks_background_ticks(InputMode::SearchPrompt));
        assert!(overlay_blocks_background_ticks(InputMode::StashMenu));
        assert!(overlay_blocks_background_ticks(InputMode::BranchPicker));
        assert!(overlay_blocks_background_ticks(InputMode::GraphFocusPicker));
        assert!(overlay_blocks_background_ticks(InputMode::CreateBranch));
        assert!(overlay_blocks_background_ticks(InputMode::Comment));
        assert!(overlay_blocks_background_ticks(InputMode::CommentExport));
        assert!(!overlay_blocks_background_ticks(InputMode::Normal {
            search_active: false
        }));
        assert!(!overlay_blocks_background_ticks(InputMode::ZPending {
            search_active: true
        }));
        assert!(!overlay_blocks_background_ticks(InputMode::GPending {
            search_active: false
        }));
    }

    #[test]
    fn busy_loop_keeps_nav_live_and_drops_nested_writes() {
        assert_eq!(classify_busy_action(&Action::Quit), BusyAction::Quit);
        assert_eq!(
            classify_busy_action(&Action::Resize {
                cols: 100,
                rows: 30
            }),
            BusyAction::Resize {
                cols: 100,
                rows: 30
            }
        );
        assert_eq!(classify_busy_action(&Action::Move(1)), BusyAction::Handle);
        assert_eq!(
            classify_busy_action(&Action::PanDiff(-1)),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::ScrollDiff(1)),
            BusyAction::Handle
        );
        assert_eq!(classify_busy_action(&Action::FoldClose), BusyAction::Handle);
        assert_eq!(
            classify_busy_action(&Action::FocusRight),
            BusyAction::Handle
        );
        assert_eq!(classify_busy_action(&Action::NavEsc), BusyAction::Handle);
        assert_eq!(classify_busy_action(&Action::ConfirmNo), BusyAction::Handle);
        assert_eq!(
            classify_busy_action(&Action::Click { col: 4, row: 8 }),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::ScrollWheel {
                col: 4,
                row: 8,
                delta: -1,
                horizontal: false,
            }),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::ConfirmYes),
            BusyAction::Ignore
        );
        assert_eq!(classify_busy_action(&Action::Fetch), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Pull), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Push), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Stage), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Revert), BusyAction::Ignore);
        assert_eq!(
            classify_busy_action(&Action::GraphCheckout),
            BusyAction::Ignore
        );
        assert_eq!(
            classify_busy_action(&Action::CommentSubmit),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::ExportComments),
            BusyAction::Handle
        );
    }

    /// Fails CI if the live TTY path grows a nested pump or a sync pane /
    /// snapshot / write git call on the loop thread. Headless e2e uses
    /// [`super::effect::Interpreter::interpret_sync`] (same apply as live).
    #[test]
    fn tty_event_loop_must_not_call_sync_pane_git() {
        let app = include_str!("app.rs");
        let loop_src = include_str!("event_loop.rs");
        let effect = include_str!("effect.rs");
        let headless = include_str!("headless.rs");
        let sched = include_str!("scheduler.rs");

        for pumped in [
            "fn run_loop(",
            "fn run_work_pumped",
            "fn pump_busy_events",
            "fn run_capped_pumped",
            "fn run_bulk_remote_pumped",
            "fn load_right_pumped",
            "fn apply_effect_inner",
            "fn reload_snapshot_pumped",
            "fn reload_repo_pumped",
            "fn load_commit_files_pumped",
            "fn load_commit_diff_pumped",
            "fn run_write_then_refresh_pumped",
        ] {
            assert!(
                !app.contains(pumped) && !loop_src.contains(pumped),
                "{pumped} must not return to the live TTY path"
            );
        }

        assert!(
            !app.contains("load_right(") && !loop_src.contains("load_right("),
            "TTY must not call load_right(; that puts git log / git diff on the loop thread"
        );
        assert!(
            app.contains("load_right_headless("),
            "unit tests may still call load_right_headless for pane compute"
        );
        assert!(
            !headless.contains("load_right_headless"),
            "HeadlessTui must not call load_right_headless"
        );
        assert!(
            !headless.contains("apply_headless"),
            "HeadlessTui must not keep a second apply path"
        );
        assert!(
            headless.contains("interpret_sync("),
            "HeadlessTui must call Interpreter::interpret_sync"
        );
        assert_eq!(
            app.matches("reload_snapshot(state, opts").count(),
            0,
            "sync reload_snapshot must not return"
        );
        assert_eq!(
            app.matches("reload_repo(state, opts").count(),
            0,
            "sync reload_repo must not return"
        );
        assert_eq!(
            app.matches("load_commit_files(state, opts").count(),
            0,
            "sync load_commit_files must not return"
        );
        assert_eq!(
            app.matches("load_commit_diff(state, opts").count(),
            0,
            "sync load_commit_diff must not return"
        );

        assert!(
            loop_src.contains("spawn_blocking"),
            "TTY git/process work must use spawn_blocking"
        );
        assert!(
            loop_src.contains("JoinSet"),
            "TTY must join workers on a JoinSet"
        );
        assert!(
            loop_src.contains("interp.apply("),
            "live JoinSet completions must call Interpreter::apply"
        );
        assert!(
            sched.contains("fn accept_repo_result"),
            "scheduler must gate stale collection generations"
        );
        assert!(
            !loop_src.contains(concat!("collect_full_", "snapshot("))
                && !effect.contains(concat!("collect_full_", "snapshot(")),
            "live watch/refresh must stream process_repo, not wait on collect_full_snapshot"
        );

        for banned in [
            "if let Err(err) = stage_file",
            "if let Err(err) = unstage_file",
            "if let Err(err) = revert_tracked_file",
            "if let Err(err) = remove_untracked_file",
            "match stash_push(",
            "match stash_apply(",
            "match stash_pop(",
            "match stash_drop(",
            "match create_branch_checkout(",
            "match create_branch_at(",
            "match remove_worktree(",
            "if run_checkout_branch(state",
            "if run_merge_into_head(state",
        ] {
            assert!(
                !app.contains(banned) && !loop_src.contains(banned),
                "{banned} must not run on the TTY loop thread"
            );
        }

        let comments_target = include_str!("comments/target.rs");
        for banned in ["list_local_branches", "rev_parse", "exec_git"] {
            assert!(
                !comments_target.contains(banned),
                "comment GC / targeting must not run git ({banned}); use snapshot.local_branches"
            );
        }
    }

    /// Live loop must read and enable mouse through `tui/tty.rs`. Direct
    /// `event::read` / `EnableMouseCapture` would skip the shared sequence
    /// and SGR contract Headless e2e uses.
    #[test]
    fn tty_event_loop_must_use_shared_mouse_tty() {
        let app = include_str!("app.rs");
        let loop_src = include_str!("event_loop.rs");
        for (name, src) in [("app.rs", app), ("event_loop.rs", loop_src)] {
            assert!(
                !src.contains("event::read("),
                "{name} must call read_event(); event::read skips the shared mouse module"
            );
            assert!(
                !src.contains("event::poll("),
                "{name} must call poll_event(); event::poll skips the shared mouse module"
            );
            assert!(
                !src.contains("EnableMouseCapture"),
                "{name} must call enable_mouse(); EnableMouseCapture sets exclusive 1003"
            );
        }
        assert!(
            loop_src.contains("read_event("),
            "TTY input thread must read via tty::read_event"
        );
        assert!(
            loop_src.contains("poll_event("),
            "TTY input thread must poll via tty::poll_event"
        );
        assert!(
            app.contains("enable_mouse("),
            "TTY must enable mouse via tty::enable_mouse"
        );
        assert!(
            app.contains("disable_mouse("),
            "TTY must disable mouse via tty::disable_mouse"
        );
    }
}

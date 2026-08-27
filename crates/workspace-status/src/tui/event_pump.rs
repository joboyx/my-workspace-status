//! Event-loop helpers that keep the TUI from going deaf.
//!
//! Long git work must not sit on the draw thread. Overlay modes must not
//! start background fetch/watch ticks. Graph autoload must not run on
//! resize, mouse noise, or swallowed overlay keys.
//!
//! Fetch / pull / push of independent checkouts run on a capped worker pool
//! (`run_capped_pumped`, Ink `FETCH_CONCURRENCY` = 4). Watch collect and
//! other git still use one worker (`run_work_pumped`). Applying a result
//! still used to call `load_right` (`git log` / `git diff`) on the draw
//! thread. Crossterm queued keys and clicks until that join, then the loop
//! drained them in a burst. Follow-up pane git must stay on the same busy
//! pump. Unchanged watch snapshots (tree signatures and checkout `HEAD` /
//! sync note / dirty set) skip the pane reload. The next tick is due from the
//! start of the interval.
//!
//! While a worker (or capped batch) runs, nav / pane switch / cancel / quit
//! still dispatch (`BusyAction::Handle`). Only actions that would start
//! another git write are drained (`Ignore`) so they cannot nest a second
//! mutating child.
//!
//! Held nav (`h`/`j`/`k`/`l`) maps Repeat to the same move as Press. The TTY
//! loop drops queued copies of that key after each move so a hold cannot
//! flush as a burst after release. Nested `LoadRightPane` git is not started
//! from a busy Handle; the in-flight `load_right_pumped` reloads if the
//! target moved.

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
        | Action::CreateBranchSubmit => BusyAction::Ignore,
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
        assert!(overlay_blocks_background_ticks(InputMode::CreateBranch));
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
    }

    /// Fails CI if TTY `apply_effect_inner` grows a sync `load_right(` /
    /// `reload_snapshot(state` / commit-file git call / unpumped local write
    /// (`git add` / stash / checkout / …) again. Headless e2e keeps the sync
    /// helpers (`load_right_headless`, `reload_snapshot`).
    #[test]
    fn tty_event_loop_must_not_call_sync_pane_git() {
        let src = include_str!("app.rs");
        assert!(
            !src.contains("load_right("),
            "TTY must use load_right_pumped; Headless e2e uses load_right_headless. \
             Re-adding load_right( puts git log / git diff on the draw thread."
        );
        assert!(
            src.contains("load_right_pumped("),
            "TTY pane git must stay on load_right_pumped"
        );
        assert!(
            src.contains("load_right_headless("),
            "Headless e2e must keep sync load_right_headless"
        );
        assert_eq!(
            src.matches("reload_snapshot(state, opts").count(),
            1,
            "only apply_headless_inner may call reload_snapshot(state, opts); \
             TTY must use reload_snapshot_pumped"
        );
        assert_eq!(
            src.matches("reload_repo(state, opts").count(),
            1,
            "only apply_headless_inner may call reload_repo(state, opts); \
             TTY must use reload_repo_pumped"
        );
        assert_eq!(
            src.matches("load_commit_files(state, opts").count(),
            1,
            "only apply_headless_inner may call load_commit_files(state, opts); \
             TTY must use load_commit_files_pumped"
        );
        assert_eq!(
            src.matches("load_commit_diff(state, opts").count(),
            1,
            "only apply_headless_inner may call load_commit_diff(state, opts); \
             TTY must use load_commit_diff_pumped"
        );
        assert!(
            !src.contains("if let Err(err) = stage_file"),
            "TTY Stage must pump git add; unpumped stage_file blocks the draw thread"
        );
        assert!(
            !src.contains("if let Err(err) = unstage_file"),
            "TTY Unstage must pump git restore --staged"
        );
        assert!(
            !src.contains("if let Err(err) = revert_tracked_file"),
            "TTY Revert must pump git restore"
        );
        assert!(
            !src.contains("if let Err(err) = remove_untracked_file"),
            "TTY Revert must pump git clean"
        );
        assert!(
            !src.contains("match stash_push("),
            "TTY stash create must pump git stash push"
        );
        assert!(
            !src.contains("match stash_apply("),
            "TTY stash apply must pump git stash apply"
        );
        assert!(
            !src.contains("match stash_pop("),
            "TTY stash pop must pump git stash pop"
        );
        assert!(
            !src.contains("match stash_drop("),
            "TTY stash drop must pump git stash drop"
        );
        assert!(
            !src.contains("match create_branch_checkout("),
            "TTY create-branch checkout must be pumped"
        );
        assert!(
            !src.contains("match create_branch_at("),
            "TTY create-branch-at must be pumped"
        );
        assert!(
            !src.contains("match remove_worktree("),
            "TTY remove-worktree must be pumped"
        );
        assert!(
            !src.contains("if run_checkout_branch(state"),
            "TTY checkout must pump compute_checkout; run_checkout_branch is tests/sync only"
        );
        assert!(
            !src.contains("if run_merge_into_head(state"),
            "TTY merge must pump compute_merge; run_merge_into_head is tests/sync only"
        );
        assert!(
            src.contains("run_write_then_refresh_pumped("),
            "TTY local writes must share the pumped write+refresh helper"
        );
        assert!(
            src.contains("run_capped_pumped("),
            "TTY fetch/pull/push must use run_capped_pumped so independent repos overlap"
        );
        assert!(
            src.contains("run_bulk_remote_pumped("),
            "TTY Fetch/Pull/Push must share run_bulk_remote_pumped"
        );
        assert_eq!(
            src.matches("for (i, repo) in repos.iter().enumerate()")
                .count(),
            1,
            "only DefaultBranch stays a serial per-repo loop; Fetch/Pull/Push must not"
        );
    }
}

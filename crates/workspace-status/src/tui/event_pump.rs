//! Event-loop helpers that keep the TUI from going deaf.
//!
//! Long git work must not sit on the draw thread. Overlay modes must not
//! start background fetch/watch ticks. Graph autoload must not run on
//! resize, mouse noise, or swallowed overlay keys.
//!
//! Fetch / pull / watch *children* already run on a worker (`run_work_pumped`).
//! Applying that result still used to call `load_right` (`git log` / `git diff`)
//! on the draw thread. Crossterm queued keys and clicks until that join, then
//! the loop drained them in a burst. Follow-up pane git must stay on the same
//! busy pump. Unchanged watch snapshots skip the pane reload.

use super::action::Action;
use super::keys::InputMode;

/// What to do with an event while a git subprocess owns a worker thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyAction {
    /// Keep waiting; the event is drained so the TTY buffer cannot fill.
    Ignore,
    /// Relayout while the worker continues.
    Resize { cols: u16, rows: u16 },
    /// Finish the current worker, then leave the TUI.
    Quit,
}

/// Classify one dispatched action during an in-flight git op.
pub fn classify_busy_action(action: &Action) -> BusyAction {
    match action {
        Action::Quit => BusyAction::Quit,
        Action::Resize { cols, rows } => BusyAction::Resize {
            cols: *cols,
            rows: *rows,
        },
        _ => BusyAction::Ignore,
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
            | Action::EasyMotionChar(_)
            | Action::FocusRight
            | Action::NavEnter
            | Action::ScrollWheel { .. }
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
        assert!(overlay_blocks_background_ticks(InputMode::EasyMotion));
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
    fn busy_loop_keeps_quit_and_resize_drains_the_rest() {
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
        assert_eq!(classify_busy_action(&Action::Move(1)), BusyAction::Ignore);
        assert_eq!(
            classify_busy_action(&Action::ConfirmYes),
            BusyAction::Ignore
        );
        assert_eq!(classify_busy_action(&Action::Fetch), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Pull), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Push), BusyAction::Ignore);
        assert_eq!(
            classify_busy_action(&Action::Click { col: 4, row: 8 }),
            BusyAction::Ignore
        );
        assert_eq!(
            classify_busy_action(&Action::ScrollWheel {
                col: 4,
                row: 8,
                delta: -1
            }),
            BusyAction::Ignore
        );
    }

    /// Fails CI if TTY `apply_effect_inner` grows a sync `load_right(` /
    /// `reload_snapshot(state` / commit-file git call again. Headless e2e keeps
    /// the sync helpers (`load_right_headless`, `reload_snapshot`).
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
    }
}

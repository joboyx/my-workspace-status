//! [`super::AppState::dispatch`]: map [`Action`] to session updates and [`Effect`].
//!
//! The match here is only a router. List keys live in [`super::dispatch_keymap`],
//! pan / wheel in [`super::pan`], Enter / Esc in [`super::dispatch_drill`],
//! and git writes in [`super::dispatch_write`].

use super::super::action::{Action, Effect};
use super::super::gates::dispatch_is_noop;
use super::{AppState, FocusPane};

impl AppState {
    /// Apply `action` and return the [`Effect`] the event loop should run.
    pub fn dispatch(&mut self, action: Action) -> Effect {
        if !matches!(action, Action::FoldToggle) {
            self.z_pending_at = None;
        }
        if !matches!(action, Action::ArmGChord) {
            self.g_pending_at = None;
        }
        let noop = dispatch_is_noop(
            &action,
            self.nav_depth(),
            self.focus == FocusPane::Right,
            self.list_focus_target(),
        );
        if noop && !matches!(action, Action::FoldToggle) {
            return Effect::None;
        }
        match action {
            action @ (Action::PanDiff(_) | Action::ScrollWheel { .. }) => {
                self.dispatch_hscroll(action)
            }
            action @ (Action::NavEnter | Action::NavEsc) => self.dispatch_drill(action),
            action @ (Action::Fetch
            | Action::Pull
            | Action::DefaultBranch
            | Action::Refresh
            | Action::Stage
            | Action::Unstage
            | Action::Revert
            | Action::ConfirmYes
            | Action::ConfirmYesClean
            | Action::ConfirmNo
            | Action::RemoveWorktree
            | Action::Push
            | Action::StashMenu
            | Action::StashMenuChar(_)
            | Action::StashMenuEnter
            | Action::StashMenuCancel
            | Action::Branch
            | Action::BranchMove(_)
            | Action::BranchChar(_)
            | Action::BranchBackspace
            | Action::BranchSubmit
            | Action::BranchCancel
            | Action::CreateBranchStart
            | Action::CreateBranchChar(_)
            | Action::CreateBranchBackspace
            | Action::CreateBranchSubmit
            | Action::CreateBranchCancel
            | Action::GraphStashApply
            | Action::GraphStashPop
            | Action::GraphStashDrop
            | Action::GraphCheckout
            | Action::GraphCreateBranch
            | Action::GraphMerge) => self.dispatch_write(action),
            action @ (Action::FoldToggle
            | Action::Quit
            | Action::CtrlC
            | Action::ToggleHelp
            | Action::Move(_)
            | Action::MoveToStart
            | Action::MoveToEnd
            | Action::PageMove(_)
            | Action::FoldToggleSubtree
            | Action::ArmGChord
            | Action::FoldClose
            | Action::FoldOpen
            | Action::ToggleShowIgnored
            | Action::ToggleTreeMode
            | Action::ToggleReviewed
            | Action::FocusLeft
            | Action::FocusRight
            | Action::ScrollDiff(_)
            | Action::ToggleFullContext
            | Action::Click { .. }
            | Action::Drag { .. }
            | Action::Release
            | Action::ToggleDiffMode
            | Action::ToggleMouse
            | Action::SearchStart
            | Action::SearchChar(_)
            | Action::SearchBackspace
            | Action::SearchSubmit
            | Action::SearchCancel
            | Action::SearchNext
            | Action::SearchPrev
            | Action::Edit
            | Action::WatchTick
            | Action::FetchTick
            | Action::GraphFocusBranches
            | Action::GraphFocusClear
            | Action::GraphFocusMove(_)
            | Action::GraphFocusChar(_)
            | Action::GraphFocusBackspace
            | Action::GraphFocusToggle
            | Action::GraphFocusSubmit
            | Action::GraphFocusCancel
            | Action::CycleTheme
            | Action::DiffVisualStart
            | Action::DiffVisualCancel
            | Action::CommentStart
            | Action::CommentChar(_)
            | Action::CommentBackspace
            | Action::CommentDelete
            | Action::CommentLeft
            | Action::CommentRight
            | Action::CommentHome
            | Action::CommentEnd
            | Action::CommentSubmit
            | Action::CommentCancel
            | Action::CommentToggleResolved
            | Action::ExportComments
            | Action::ExportCommentsCancel
            | Action::Resize { .. }
            | Action::None) => self.dispatch_keymap(action, noop),
        }
    }
}

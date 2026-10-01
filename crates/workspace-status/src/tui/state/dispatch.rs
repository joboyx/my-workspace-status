//! [`super::AppState::dispatch`]: map [`Action`] to session updates and [`Effect`].
//!
//! The match here is only a router. List keys live in [`super::dispatch_keymap`],
//! pan / wheel in [`super::pan`], Enter / Esc in [`super::dispatch_drill`],
//! and git writes in [`super::dispatch_write`].

use super::super::action::{Action, Effect};
use super::super::gates::{dispatch_is_noop, dispatch_noop_reason};
use super::super::status::StatusMessage;
use super::super::tabs::{CANNOT_STAGE_COMPARE, CANNOT_UNSTAGE_COMPARE, SWITCH_TO_WORKSPACE_TAB};
use super::{AppState, FocusPane};

impl AppState {
    /// Why the active compare tab refuses `action`, or `None` when it may run.
    ///
    /// The one compare gate. [`Self::dispatch`] puts the reason on the status
    /// line and the command palette paints it on the row, so a key press and
    /// a palette row always give the same copy. The Workspace tab refuses
    /// nothing here.
    pub(crate) fn compare_refusal(&self, action: &Action) -> Option<String> {
        if !self.is_compare_tab() {
            return None;
        }
        let reason = match action {
            // A compare diff is commits only: there is no index side to write.
            Action::Stage => CANNOT_STAGE_COMPARE,
            Action::Unstage => CANNOT_UNSTAGE_COMPARE,
            // Remote, branch, stash, graph, and worktree writes stay on Workspace.
            Action::Fetch
            | Action::Pull
            | Action::Push
            | Action::DefaultBranch
            | Action::Branch
            | Action::BranchSubmit
            | Action::CreateBranchStart
            | Action::CreateBranchSubmit
            | Action::RemoveWorktree
            | Action::GraphCheckout
            | Action::GraphCreateBranch
            | Action::GraphMerge
            | Action::GraphStashApply
            | Action::GraphStashPop
            | Action::GraphStashDrop
            | Action::GraphFocusBranches
            | Action::GraphFocusClear
            | Action::GraphFocusSubmit
            | Action::StashMenu
            | Action::StashMenuEnter => SWITCH_TO_WORKSPACE_TAB,
            // `x` writes the worktree only while the compare head is the
            // checked-out working tree for the file.
            Action::Revert => return self.compare_revert_target().err(),
            // Only a confirm the compare tab opened may run here.
            Action::ConfirmYes | Action::ConfirmYesClean => {
                if self
                    .confirm
                    .as_ref()
                    .is_some_and(|pending| pending.is_compare_owned())
                {
                    return None;
                }
                SWITCH_TO_WORKSPACE_TAB
            }
            _ => return None,
        };
        Some(reason.into())
    }

    /// True when `action` is a compare-tab `x`, which the right-pane
    /// no-op gate must let through: it reverts the open diff's file.
    pub(crate) fn compare_revert_runs(&self, action: &Action) -> bool {
        self.is_compare_tab() && matches!(action, Action::Revert)
    }

    /// Apply `action` and return the [`Effect`] the event loop should run.
    pub fn dispatch(&mut self, action: Action) -> Effect {
        if !matches!(action, Action::FoldToggle | Action::PointerMove { .. }) {
            self.z_pending_at = None;
        }
        if !matches!(
            action,
            Action::ArmGChord
                | Action::None
                | Action::PointerMove { .. }
                | Action::WatchTick
                | Action::FetchTick
                | Action::Release
        ) {
            self.g_pending_at = None;
        }
        if let Some(reason) = self.compare_refusal(&action) {
            self.status = StatusMessage::warn(reason);
            return Effect::None;
        }
        let visual_write = self.diff_visual_anchor.is_some()
            && matches!(action, Action::Stage | Action::Unstage | Action::Revert);
        // Compare `x` also runs from the focused diff (the open file).
        let visual_write = visual_write || self.compare_revert_runs(&action);
        let noop = dispatch_is_noop(
            &action,
            self.nav_depth(),
            self.focus == FocusPane::Right,
            self.list_focus_target(),
        );
        if noop && !matches!(action, Action::FoldToggle) && !visual_write {
            if let Some(reason) = dispatch_noop_reason(
                &action,
                self.nav_depth(),
                self.focus == FocusPane::Right,
                self.list_focus_target(),
            ) {
                self.status = StatusMessage::warn(reason);
            }
            return Effect::None;
        }
        match action {
            // The live loop stores the pointer before dispatch.
            Action::PointerMove { .. } => Effect::None,
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
            | Action::ToggleFullContext
            | Action::Click { .. }
            | Action::Drag { .. }
            | Action::Release
            | Action::ToggleDiffMode
            | Action::ToggleDiffWrap
            | Action::ToggleCommitMsgExpand
            | Action::ToggleMouse
            | Action::SearchStart
            | Action::SearchChar(_)
            | Action::SearchBackspace
            | Action::SearchSubmit
            | Action::SearchCancel
            | Action::SearchNext
            | Action::SearchPrev
            | Action::Edit
            | Action::ExternalDiff
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
            | Action::DiffVisualUnmapped
            | Action::CommentStart
            | Action::CommentInput(_)
            | Action::CommentSubmit
            | Action::CommentCancel
            | Action::CommentToggleResolved
            | Action::ExportComments
            | Action::ExportCommentsCancel
            | Action::CopyEntityReference
            | Action::ToggleCommandPalette(_)
            | Action::CommandPaletteMove(_)
            | Action::CommandPaletteChar(_)
            | Action::CommandPaletteBackspace
            | Action::CommandPaletteSubmit
            | Action::CommandPaletteCancel
            | Action::CompareVsDefault
            | Action::CompareVsBranch
            | Action::CloseCompareTab
            | Action::NextTab
            | Action::PreviousTab
            | Action::JumpToTab(_)
            | Action::ComparePickerMove(_)
            | Action::ComparePickerChar(_)
            | Action::ComparePickerBackspace
            | Action::ComparePickerSubmit
            | Action::ComparePickerCancel
            | Action::Resize { .. }
            | Action::None) => self.dispatch_keymap(action, noop),
        }
    }
}

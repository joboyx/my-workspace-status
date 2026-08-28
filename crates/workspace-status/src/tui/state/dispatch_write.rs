//! Git writes, confirms, and write overlays (stash, branch, merge).

use super::super::action::{Action, Effect};
use super::super::ops::Op;
use super::super::split::SplitDrag;
use super::super::stash::StashOpId;
use super::{AppState, FileWrite, PendingConfirm};

impl AppState {
    /// Apply a git-write [`Action`], including confirm and overlay keys.
    pub(crate) fn dispatch_write(&mut self, action: Action) -> Effect {
        match action {
            Action::Fetch => self.op_effect(Op::Fetch),
            Action::Pull => self.op_effect(Op::Pull),
            Action::DefaultBranch => self.op_effect(Op::DefaultBranch),
            Action::Refresh => self.refresh_effect(),
            Action::Stage => self.file_write_effect(FileWrite::Stage),
            Action::Unstage => self.file_write_effect(FileWrite::Unstage),
            Action::Revert => {
                self.drag = SplitDrag::None;
                self.begin_revert()
            }
            Action::ConfirmYes => self.confirm_yes(false),
            Action::ConfirmYesClean => self.confirm_yes(true),
            Action::ConfirmNo => {
                if let Some(pending) = self.confirm.take() {
                    self.status = match pending {
                        PendingConfirm::Revert { .. } => "revert cancelled".into(),
                        PendingConfirm::StashDrop { .. } => "drop cancelled".into(),
                        PendingConfirm::CheckoutOutOfSync { .. } => "checkout cancelled".into(),
                        PendingConfirm::RemoveWorktree { .. } => "remove worktree cancelled".into(),
                        PendingConfirm::MergeIntoHead { .. } => "merge cancelled".into(),
                    };
                }
                Effect::None
            }
            Action::RemoveWorktree => {
                self.drag = SplitDrag::None;
                self.begin_remove_worktree()
            }
            Action::Push => self.push_effect(),
            Action::StashMenu => {
                self.drag = SplitDrag::None;
                self.begin_stash_menu()
            }
            Action::StashMenuChar(c) => self.stash_menu_key(Some(c), false, false),
            Action::StashMenuEnter => self.stash_menu_key(None, true, false),
            Action::StashMenuCancel => self.stash_menu_key(None, false, true),
            Action::Branch => {
                self.drag = SplitDrag::None;
                self.begin_branch_picker()
            }
            Action::BranchMove(delta) => {
                if let Some(picker) = self.branch_picker.as_mut() {
                    picker.move_cursor(delta);
                }
                Effect::None
            }
            Action::BranchChar(c) => {
                if let Some(picker) = self.branch_picker.as_mut() {
                    let mut filter = picker.filter.clone();
                    filter.push(c);
                    picker.set_filter(filter);
                    self.status = format!("branch /{}", picker.filter);
                }
                Effect::None
            }
            Action::BranchBackspace => {
                if let Some(picker) = self.branch_picker.as_mut() {
                    let mut filter = picker.filter.clone();
                    filter.pop();
                    picker.set_filter(filter);
                    self.status = format!("branch /{}", picker.filter);
                }
                Effect::None
            }
            Action::BranchSubmit => self.submit_branch_picker(),
            Action::BranchCancel => {
                self.branch_picker = None;
                self.status = "branch cancelled".into();
                Effect::None
            }
            Action::CreateBranchStart => {
                self.drag = SplitDrag::None;
                self.begin_create_branch()
            }
            Action::CreateBranchChar(c) => {
                if let Some(create) = self.create_branch.as_mut() {
                    create.name.push(c);
                    self.status = format!("create {}", create.name);
                }
                Effect::None
            }
            Action::CreateBranchBackspace => {
                if let Some(create) = self.create_branch.as_mut() {
                    create.name.pop();
                    self.status = format!("create {}", create.name);
                }
                Effect::None
            }
            Action::CreateBranchSubmit => self.submit_create_branch(),
            Action::CreateBranchCancel => {
                self.create_branch = None;
                self.status = "create branch cancelled".into();
                Effect::None
            }
            Action::GraphStashApply => self.graph_stash_op(StashOpId::Apply),
            Action::GraphStashPop => self.graph_stash_op(StashOpId::Pop),
            Action::GraphStashDrop => self.graph_stash_op(StashOpId::Drop),
            Action::GraphCheckout => {
                self.drag = SplitDrag::None;
                self.begin_graph_checkout()
            }
            Action::GraphCreateBranch => {
                self.drag = SplitDrag::None;
                self.begin_graph_create_branch()
            }
            Action::GraphMerge => {
                self.drag = SplitDrag::None;
                self.begin_graph_merge()
            }
            _ => Effect::None,
        }
    }
}

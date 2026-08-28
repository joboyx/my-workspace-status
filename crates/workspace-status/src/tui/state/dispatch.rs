//! [`super::AppState::dispatch`]: map [`Action`] to session updates and [`Effect`].

use std::time::Instant;

use super::super::action::{Action, Effect};
use super::super::gates::dispatch_is_noop;
use super::super::ops::Op;
use super::super::split::SplitDrag;
use super::super::stash::StashOpId;
use super::{AppState, FileWrite, FocusPane, FoldOp, PendingConfirm};

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
            Action::FoldToggle => {
                if !noop {
                    self.fold_op(FoldOp::Toggle);
                }
                self.z_pending_at = Some(Instant::now());
                Effect::None
            }
            Action::Quit => Effect::Quit,
            Action::CtrlC => self.ctrl_c(Instant::now()),
            Action::ToggleHelp => {
                self.drag = SplitDrag::None;
                self.help_open = !self.help_open;
                self.clear_help_search();
                Effect::None
            }
            Action::Move(delta) => self.move_focused(delta),
            Action::MoveToStart => self.move_focused_edge(false),
            Action::MoveToEnd => self.move_focused_edge(true),
            Action::PageMove(pages) => {
                if self.graph_pane_focused() {
                    self.page_graph(pages)
                } else {
                    let height = self.page_step();
                    self.move_focused(pages * height)
                }
            }
            Action::FoldToggleSubtree => {
                self.fold_subtree();
                Effect::None
            }
            Action::ArmGChord => {
                self.g_pending_at = Some(Instant::now());
                Effect::None
            }
            Action::FoldClose => {
                self.fold_op(FoldOp::Close);
                Effect::None
            }
            Action::FoldOpen => {
                self.fold_op(FoldOp::Open);
                Effect::None
            }
            Action::ToggleShowIgnored => {
                self.show_ignored = !self.show_ignored;
                self.snapshot.show_ignored = self.show_ignored;
                self.rebuild_rows();
                self.status = if self.show_ignored {
                    "showing ignored repos".into()
                } else {
                    "hiding ignored repos".into()
                };
                Effect::LoadRightPane
            }
            Action::ToggleTreeMode => self.toggle_tree_mode(),
            Action::Fetch => self.op_effect(Op::Fetch),
            Action::Pull => self.op_effect(Op::Pull),
            Action::DefaultBranch => self.op_effect(Op::DefaultBranch),
            Action::Refresh => self.refresh_effect(),
            Action::ToggleReviewed => self.toggle_reviewed(),
            Action::FocusLeft => {
                self.focus = FocusPane::Left;
                Effect::None
            }
            Action::FocusRight => {
                self.focus = FocusPane::Right;
                Effect::None
            }
            Action::ScrollDiff(delta) => {
                self.scroll_right(delta);
                Effect::None
            }
            Action::PanDiff(delta) => {
                self.pan_focused(delta);
                Effect::None
            }
            Action::ToggleFullContext => self.toggle_full_context(),
            Action::Click { col, row } => {
                if !self.mouse_enabled {
                    Effect::None
                } else {
                    self.click(col, row)
                }
            }
            Action::Drag { col, row } => {
                if !self.mouse_enabled {
                    Effect::None
                } else {
                    self.drag_split(col, row)
                }
            }
            Action::Release => {
                if !self.mouse_enabled {
                    Effect::None
                } else {
                    self.drag = SplitDrag::None;
                    Effect::None
                }
            }
            Action::ToggleDiffMode => self.toggle_diff_mode(),
            Action::ToggleMouse => {
                self.mouse_enabled = !self.mouse_enabled;
                self.status = if self.mouse_enabled {
                    "Mouse on".into()
                } else {
                    "Mouse off".into()
                };
                Effect::None
            }
            Action::ScrollWheel {
                col,
                row: _,
                delta,
                horizontal,
            } => {
                if !self.mouse_enabled {
                    return Effect::None;
                }
                if horizontal {
                    self.mouse_pan(col, delta);
                    return Effect::None;
                }
                if col >= self.layout.right_x {
                    self.focus = FocusPane::Right;
                    if self.drill.is_files() {
                        self.move_file_cursor(delta)
                    } else {
                        self.scroll_right(delta);
                        Effect::None
                    }
                } else if self.drill.is_diff() {
                    self.focus = FocusPane::Left;
                    self.move_file_cursor(delta)
                } else if self.drill.is_files() {
                    self.focus = FocusPane::Left;
                    self.move_graph_cursor(delta);
                    self.follow_graph_files()
                } else {
                    self.focus = FocusPane::Left;
                    self.move_cursor(delta);
                    Effect::LoadRightPane
                }
            }
            Action::SearchStart => {
                self.drag = SplitDrag::None;
                if self.help_open {
                    self.help_search_query = Some(String::new());
                    Effect::None
                } else {
                    self.search_mode = true;
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_hit = None;
                    self.search_target = self.current_search_pane();
                    self.status = "/".into();
                    Effect::None
                }
            }
            Action::SearchChar(c) => {
                if self.help_open && self.help_search_query.is_some() {
                    if let Some(q) = &mut self.help_search_query {
                        q.push(c);
                    }
                    Effect::None
                } else if self.search_mode {
                    self.search_query.push(c);
                    self.apply_search(0)
                } else {
                    Effect::None
                }
            }
            Action::SearchBackspace => {
                if self.help_open && self.help_search_query.is_some() {
                    if let Some(q) = &mut self.help_search_query {
                        q.pop();
                    }
                    Effect::None
                } else if self.search_mode {
                    self.search_query.pop();
                    self.apply_search(0)
                } else {
                    Effect::None
                }
            }
            Action::SearchSubmit => {
                if self.help_open && self.help_search_query.is_some() {
                    Effect::None
                } else {
                    self.search_mode = false;
                    if self.search_query.trim().is_empty() {
                        self.search_active = false;
                        self.search_query.clear();
                        self.search_hit = None;
                        self.status = "search cleared".into();
                        Effect::None
                    } else {
                        self.search_active = true;
                        self.apply_search(0)
                    }
                }
            }
            Action::SearchCancel => {
                if self.help_open && self.help_search_query.is_some() {
                    self.clear_help_search();
                    self.status = "help search cleared".into();
                    Effect::None
                } else {
                    self.search_mode = false;
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_hit = None;
                    self.status = "search cancelled".into();
                    Effect::None
                }
            }
            Action::SearchNext => {
                if self.search_active && !self.search_query.trim().is_empty() {
                    self.apply_search(1)
                } else {
                    Effect::None
                }
            }
            Action::SearchPrev => {
                if self.search_active && !self.search_query.trim().is_empty() {
                    self.apply_search(-1)
                } else {
                    Effect::None
                }
            }
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
            Action::Edit => {
                if let Some((repo, path)) = self.focused_commit_edit_path() {
                    self.status = format!("edit {path}");
                    Effect::EditFile { repo, path }
                } else if let Some((repo, change)) = self.focused_file_if_shown() {
                    self.status = format!("edit {}", change.path);
                    Effect::EditFile {
                        repo,
                        path: change.path,
                    }
                } else {
                    self.status = "focus a dirty file to edit".into();
                    Effect::None
                }
            }
            Action::WatchTick => Effect::WatchRefresh,
            Action::FetchTick => self.fetch_tick_effect(),
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
            Action::NavEnter => self.nav_enter(),
            Action::NavEsc => {
                if self.search_active {
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_hit = None;
                    self.status = "search cleared".into();
                    Effect::None
                } else {
                    self.nav_esc()
                }
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
            Action::GraphFocusBranches => {
                self.drag = SplitDrag::None;
                self.begin_graph_focus_picker()
            }
            Action::GraphFocusClear => self.clear_graph_branch_focus(),
            Action::GraphFocusMove(delta) => {
                if let Some(picker) = self.graph_focus_picker.as_mut() {
                    picker.move_cursor(delta);
                }
                Effect::None
            }
            Action::GraphFocusChar(c) => {
                if let Some(picker) = self.graph_focus_picker.as_mut() {
                    let mut filter = picker.filter.clone();
                    filter.push(c);
                    picker.set_filter(filter);
                    self.status = format!("focus /{}", picker.filter);
                }
                Effect::None
            }
            Action::GraphFocusBackspace => {
                if let Some(picker) = self.graph_focus_picker.as_mut() {
                    let mut filter = picker.filter.clone();
                    filter.pop();
                    picker.set_filter(filter);
                    self.status = format!("focus /{}", picker.filter);
                }
                Effect::None
            }
            Action::GraphFocusToggle => {
                if let Some(picker) = self.graph_focus_picker.as_mut() {
                    picker.toggle_mark();
                }
                Effect::None
            }
            Action::GraphFocusSubmit => self.submit_graph_focus_picker(),
            Action::GraphFocusCancel => {
                self.graph_focus_picker = None;
                self.status = "focus cancelled".into();
                Effect::None
            }
            Action::CycleTheme => self.cycle_theme(),
            Action::Resize { cols, rows: _ } => {
                self.apply_terminal_size(cols);
                Effect::None
            }
            Action::None => Effect::None,
        }
    }
}

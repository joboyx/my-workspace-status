//! List keys, search, help, view toggles, and mouse hit-test.

use std::time::Instant;

use super::super::action::{Action, Effect, ExternalDiffKind};
use super::super::branches::can_open_branch_picker;
use super::super::command_palette::{CommandScope, PaletteCommand};
use super::super::diff::PartialPatchKind;
use super::super::gates::{
    dispatch_is_noop, dispatch_noop_reason, ListFocusTarget, FOCUS_A_FILE_DIFF,
    FOCUS_A_FILE_TO_MARK_REVIEWED, FOCUS_A_GRAPH_COMMIT, FOCUS_A_GRAPH_STASH,
    REVIEWED_MARKS_ARE_FOR_TREE_FILES, STASH_NEEDS_TREE_OR_GRAPH,
};
use super::super::graph_focus::GRAPH_FOCUS_NEED_CONTEXT;
use super::super::ops::{collect_write_files, op_is_kind_noop, op_kind_noop_reason, Op};
use super::super::split::SplitDrag;
use super::super::status::StatusMessage;
use super::super::tabs::{
    ComparePickerKind, NOT_ON_WORKTREE_COMPARE, ONLY_WORKSPACE_TAB_OPEN, WORKSPACE_TAB_CANNOT_CLOSE,
};
use super::super::tree::NodeKind;
use super::{AppState, FileWrite, FocusPane, FoldOp};

impl AppState {
    /// Apply a list / view / search / hit-test [`Action`].
    pub(crate) fn dispatch_keymap(&mut self, action: Action, fold_noop: bool) -> Effect {
        match action {
            Action::FoldToggle => {
                // Graph and file-diff rows do not fold: say so instead of
                // arming a `zz` that cannot do anything.
                if !matches!(
                    self.list_focus_target(),
                    ListFocusTarget::Tree | ListFocusTarget::CommitFiles
                ) {
                    self.status = StatusMessage::warn(super::Z_FOLDS_TREE_ROWS);
                    return Effect::None;
                }
                if !fold_noop {
                    self.fold_op(FoldOp::Toggle);
                }
                self.z_pending_at = Some(Instant::now());
                Effect::None
            }
            Action::Quit => Effect::Quit,
            Action::CtrlC => self.ctrl_c(Instant::now()),
            Action::ToggleHelp => {
                self.cancel_mouse_drag();
                self.help_open = !self.help_open;
                self.clear_help_search();
                if self.help_open {
                    self.help_scroll = 0;
                    self.clear_diff_visual();
                }
                Effect::None
            }
            Action::HelpScroll(delta) => {
                let max = self.layout.help_scroll_max as i64;
                self.help_scroll =
                    (self.help_scroll as i64 + i64::from(delta)).clamp(0, max) as usize;
                Effect::None
            }
            Action::Move(delta) => self.move_focused(delta),
            Action::MoveToStart => self.move_focused_edge(false),
            Action::MoveToEnd => self.move_focused_edge(true),
            Action::PageMove(pages) => {
                if self.graph_pane_focused() {
                    self.page_graph(pages)
                } else if self.list_focus_target() == ListFocusTarget::None {
                    let height = self.diff_body_height().saturating_sub(1).max(1) as i32;
                    self.move_focused(pages * height)
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
                // Highlight has no GPending mode: its second `g` lands here.
                if self.diff_visual_anchor.is_some() && self.chord_pending(self.g_pending_at) {
                    self.g_pending_at = None;
                    return self.move_focused_edge(false);
                }
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
            Action::ToggleReviewed => self.toggle_reviewed(),
            Action::FocusLeft => {
                self.clear_diff_visual();
                self.focus = FocusPane::Left;
                Effect::None
            }
            Action::FocusRight => {
                self.focus = FocusPane::Right;
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
            Action::CtrlClick { col, row } => {
                if !self.mouse_enabled {
                    Effect::None
                } else {
                    self.ctrl_click(col, row)
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
                    self.release_mouse()
                }
            }
            Action::BackClick => {
                if !self.mouse_enabled {
                    return Effect::None;
                }
                self.cancel_mouse_drag();
                if self.diff_visual_anchor.is_some() {
                    self.dispatch(Action::DiffVisualCancel)
                } else {
                    self.dispatch(Action::NavEsc)
                }
            }
            Action::ResizeTree(steps) => self.step_tree_width(steps),
            Action::ToggleDiffMode => self.toggle_diff_mode(),
            Action::ToggleDiffWrap => self.toggle_diff_wrap(),
            Action::ToggleCommitMsgExpand => self.toggle_commit_msg_expand(),
            Action::ResizeCommitMsg(delta) => self.resize_commit_msg(delta),
            Action::ToggleLineBlame => self.toggle_line_blame(),
            Action::ToggleMouse => {
                self.cancel_mouse_drag();
                self.mouse_enabled = !self.mouse_enabled;
                self.pointer = None;
                self.drop_peek();
                self.status = if self.mouse_enabled {
                    "Mouse on".into()
                } else {
                    "Mouse off".into()
                };
                Effect::None
            }
            Action::SearchStart => {
                self.cancel_mouse_drag();
                self.clear_diff_visual();
                if self.help_open {
                    self.help_search_query = Some(String::new());
                    Effect::None
                } else {
                    self.search_mode = true;
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_hit = None;
                    self.search_target = self.current_search_pane();
                    self.save_search_origin();
                    self.status = "/".into();
                    Effect::None
                }
            }
            Action::SearchChar(c) => {
                if self.help_open && self.help_search_query.is_some() {
                    if let Some(q) = &mut self.help_search_query {
                        q.push(c);
                    }
                    self.help_scroll = 0;
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
                    self.help_scroll = 0;
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
                        self.search_origin = None;
                        self.search_active = false;
                        self.search_query.clear();
                        self.search_hit = None;
                        self.status = "search cleared".into();
                        Effect::None
                    } else {
                        // Enter steps forward from the `/` origin, like typing.
                        self.search_active = true;
                        let effect = self.apply_search(0);
                        self.search_origin = None;
                        effect
                    }
                }
            }
            Action::SearchCancel => {
                if self.help_open && self.help_search_query.is_some() {
                    self.clear_help_search();
                    self.status = "help search cleared".into();
                    Effect::None
                } else {
                    // Typing moved the cursor match by match: put it back.
                    self.search_mode = false;
                    self.search_active = false;
                    self.search_query.clear();
                    self.search_hit = None;
                    let effect = self.restore_search_origin();
                    self.status = "search cancelled".into();
                    effect
                }
            }
            Action::SearchNext | Action::SearchPrev => {
                if self.search_is_armed() {
                    let step = if matches!(action, Action::SearchNext) {
                        1
                    } else {
                        -1
                    };
                    self.apply_search(step)
                } else {
                    self.status = StatusMessage::warn(super::NO_SEARCH_ARMED);
                    Effect::None
                }
            }
            Action::Edit => {
                if let Some((repo, path)) = self.focused_commit_edit_path() {
                    self.status = StatusMessage::progress(format!("opening {path}…"));
                    Effect::EditFile {
                        repo,
                        path,
                        line: None,
                    }
                } else if self.is_compare_tab() {
                    self.status = StatusMessage::warn("focus a file to edit");
                    Effect::None
                } else if let Some((repo, change)) = self.focused_file_if_shown() {
                    self.status = StatusMessage::progress(format!("opening {}…", change.path));
                    Effect::EditFile {
                        repo,
                        path: change.path,
                        line: None,
                    }
                } else {
                    self.status = StatusMessage::warn("focus a dirty file to edit");
                    Effect::None
                }
            }
            Action::ExternalDiff => {
                if let Some((repo, path)) = self.focused_commit_edit_path() {
                    let Some(kind) = self.external_diff_kind() else {
                        self.status = StatusMessage::warn(NOT_ON_WORKTREE_COMPARE);
                        return Effect::None;
                    };
                    self.status = StatusMessage::progress(format!("opening diff {path}…"));
                    Effect::ExternalDiff { repo, path, kind }
                } else if self.is_compare_tab() {
                    self.status = StatusMessage::warn("focus a file to diff");
                    Effect::None
                } else if let Some((repo, change)) = self.focused_file_if_shown() {
                    self.status = StatusMessage::progress(format!("opening diff {}…", change.path));
                    Effect::ExternalDiff {
                        repo,
                        path: change.path,
                        kind: ExternalDiffKind::Worktree,
                    }
                } else {
                    self.status = StatusMessage::warn("focus a file to diff");
                    Effect::None
                }
            }
            Action::WatchTick => {
                let mut effects = vec![Effect::WatchRefresh];
                effects.extend(self.compare_probe_effects());
                if effects.len() == 1 {
                    Effect::WatchRefresh
                } else {
                    Effect::Batch(effects)
                }
            }
            Action::FetchTick => self.fetch_tick_effect(),
            Action::GraphFocusBranches => {
                self.cancel_mouse_drag();
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
                    // The picker title paints the filter; keep it off the status rows.
                    self.status.clear();
                }
                Effect::None
            }
            Action::GraphFocusBackspace => {
                if let Some(picker) = self.graph_focus_picker.as_mut() {
                    let mut filter = picker.filter.clone();
                    filter.pop();
                    picker.set_filter(filter);
                    // The picker title paints the filter; keep it off the status rows.
                    self.status.clear();
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
            Action::DiffVisualStart => self.begin_diff_visual(),
            Action::DiffVisualCancel => self.cancel_diff_visual(),
            Action::DiffVisualUnmapped => {
                if self.diff_visual_anchor.is_some() {
                    self.status = StatusMessage::warn(super::ESC_EXITS_HIGHLIGHT);
                }
                Effect::None
            }
            Action::CommentStart => self.begin_comment(),
            Action::CommentInput(key) => {
                if let Some(prompt) = self.comment.as_mut() {
                    prompt.input(key);
                }
                Effect::None
            }
            Action::CommentSubmit => self.submit_comment(),
            Action::CommentToggleResolved => {
                if let Some(prompt) = self.comment.as_mut() {
                    prompt.resolved = !prompt.resolved;
                }
                Effect::None
            }
            Action::CommentCancel => {
                self.comment = None;
                self.status = "comment cancelled".into();
                Effect::None
            }
            Action::ExportComments => self.export_comments(),
            Action::CopyEntityReference => self.copy_entity_reference(),
            Action::OpenPullRequest => self.open_pull_request(),
            Action::ExportCommentsCancel => {
                self.comment_export = None;
                self.status.clear();
                Effect::None
            }
            Action::Resize { cols, rows: _ } => {
                self.text_selection = None;
                self.apply_terminal_size(cols);
                Effect::None
            }
            Action::ToggleQuickOpen(_)
            | Action::QuickOpenMove(_)
            | Action::QuickOpenChar(_)
            | Action::QuickOpenBackspace
            | Action::QuickOpenSubmit
            | Action::QuickOpenCancel => self.dispatch_quick_open(action),
            Action::CompareVsDefault => self.compare_vs_default(),
            Action::CompareVsBranch => self.prepare_compare_picker(ComparePickerKind::Branch),
            Action::CompareVsCommit => self.prepare_compare_picker(ComparePickerKind::Commit),
            Action::CompareCommitVsParent => self.compare_commit_vs_parent(),
            Action::BlameCommitVsParent => self.blame_commit_vs_parent(),
            Action::BlamePreviousChange => self.blame_previous_change(),
            Action::BlameCommitVsWorktree => self.blame_commit_vs_worktree(),
            Action::BlameRevealGraph => self.blame_reveal_graph(),
            Action::BlameMenu => self.open_blame_menu(),
            Action::BlameMenuChar(key) => self.blame_menu_pick(Some(key)),
            Action::BlameMenuEnter => self.blame_menu_pick(None),
            Action::BlameMenuCancel => {
                self.blame_menu = false;
                self.status.clear();
                Effect::None
            }
            Action::CloseTab => self.close_active_tab(),
            Action::NextTab => self.activate_relative_tab(1),
            Action::PreviousTab => self.activate_relative_tab(-1),
            Action::JumpToTab(n) => self.jump_to_tab(n),
            Action::ComparePickerMove(delta) => {
                if let Some(picker) = self.compare_picker.as_mut() {
                    picker.move_cursor(delta);
                }
                Effect::None
            }
            Action::ComparePickerChar(c) => {
                if let Some(picker) = self.compare_picker.as_mut() {
                    let mut filter = picker.filter().to_string();
                    filter.push(c);
                    picker.set_filter(filter);
                }
                Effect::None
            }
            Action::ComparePickerBackspace => {
                if let Some(picker) = self.compare_picker.as_mut() {
                    let mut filter = picker.filter().to_string();
                    filter.pop();
                    picker.set_filter(filter);
                }
                Effect::None
            }
            Action::ComparePickerSubmit => self.submit_compare_picker(),
            Action::ComparePickerCancel => {
                self.abandon_compare_picker();
                self.status = StatusMessage::info(super::super::tabs::COMPARE_CANCELLED);
                Effect::None
            }
            Action::None => Effect::None,
            _ => Effect::None,
        }
    }

    /// Why palette row `command` cannot run, or `None` if Enter should dispatch.
    ///
    /// On a file tab: [`Self::file_tab_refusal`], the blame gate
    /// ([`Self::blame_refusal`]), then [`Self::file_tab_palette_reason`].
    /// Otherwise: compare refusal ([`Self::compare_refusal`], which also gates
    /// the compare open commands on every tab), the blame gate, folder-summary refusal
    /// ([`Self::summary_refusal`]), then the row's
    /// highlight scope, then the range patch (highlighted stage / unstage /
    /// revert), then the action gate.
    pub(crate) fn palette_disabled_reason(&self, command: &PaletteCommand) -> Option<String> {
        let action = &command.action;
        if self.is_file_tab() {
            return self
                .file_tab_refusal(action)
                .or_else(|| self.blame_refusal(action))
                .or_else(|| self.file_tab_palette_reason(command));
        }
        if let Some(reason) = self
            .compare_refusal(action)
            .or_else(|| self.blame_refusal(action))
            .or_else(|| self.summary_refusal(action))
        {
            return Some(reason);
        }
        let highlighted = self.diff_visual_anchor.is_some();
        match (command.scope, highlighted) {
            (CommandScope::NoHighlight, true) => return Some("exit highlight first (Esc)".into()),
            (CommandScope::Highlight, false) => {
                return Some("highlight diff lines first (V)".into())
            }
            _ => {}
        }
        if let Some(anchor) = self.diff_visual_anchor {
            let kind = match action {
                Action::Stage => Some(PartialPatchKind::Stage),
                Action::Unstage => Some(PartialPatchKind::Unstage),
                Action::Revert => Some(PartialPatchKind::Revert),
                _ => None,
            };
            if let Some(kind) = kind {
                return self.visual_patch(anchor, kind).err();
            }
        }
        if dispatch_is_noop(
            action,
            self.nav_depth(),
            self.focus == FocusPane::Right,
            self.list_focus_target(),
        ) && !self.compare_revert_runs(action)
        {
            // Only list moves and folds have no gate reason; the palette's
            // such rows are Fold row and Fold subtree.
            return dispatch_noop_reason(
                action,
                self.nav_depth(),
                self.focus == FocusPane::Right,
                self.list_focus_target(),
            )
            .or_else(|| Some(super::Z_FOLDS_TREE_ROWS.into()));
        }
        match action {
            // Same check as compare `x` with no highlight: the compare
            // gate, then the file status (M / A / D / R only).
            Action::Revert if self.is_compare_tab() => self.compare_file_revert_refusal(),
            Action::Pull | Action::DefaultBranch | Action::Fetch => {
                let op = if matches!(action, Action::Pull) {
                    Op::Pull
                } else if matches!(action, Action::Fetch) {
                    Op::Fetch
                } else {
                    Op::DefaultBranch
                };
                self.focused_row()
                    .filter(|row| op_is_kind_noop(row.kind, op))
                    .map(|_| op_kind_noop_reason(op).into())
            }
            Action::Push => match self.focused_row().map(|row| row.kind) {
                Some(NodeKind::Repo | NodeKind::Checkout) => None,
                _ => Some("repo / checkout only".into()),
            },
            Action::RemoveWorktree => {
                if self.can_remove_focused_worktree() {
                    None
                } else {
                    Some("Focus a linked worktree to remove".into())
                }
            }
            Action::Revert => {
                let scoped =
                    collect_write_files(&self.snapshot, self.focused_row(), self.show_ignored);
                let selected = scoped
                    .into_iter()
                    .filter(|file| super::is_revertible(&file.change))
                    .count();
                if selected > 0 {
                    None
                } else {
                    let staged_only = self.focused_file_if_shown().is_some_and(|(_, change)| {
                        change.staged_status.is_some()
                            && change.unstaged_status.is_none()
                            && !change.untracked
                    });
                    Some(if staged_only {
                        "nothing to discard (staged only)".into()
                    } else if matches!(
                        self.focused_row().map(|row| row.kind),
                        Some(
                            super::super::tree::NodeKind::File
                                | super::super::tree::NodeKind::Dir
                                | super::super::tree::NodeKind::Repo
                                | super::super::tree::NodeKind::Checkout
                                | super::super::tree::NodeKind::Section
                        )
                    ) {
                        "nothing to discard".into()
                    } else {
                        "focus a file, dir, checkout, or repo to revert".into()
                    })
                }
            }
            Action::Stage => self.empty_file_write_reason(FileWrite::Stage),
            Action::Unstage => self.empty_file_write_reason(FileWrite::Unstage),
            Action::GraphStashApply | Action::GraphStashPop | Action::GraphStashDrop => {
                if self.graph_stash_focused() {
                    None
                } else {
                    Some(FOCUS_A_GRAPH_STASH.into())
                }
            }
            Action::GraphCheckout | Action::GraphCreateBranch | Action::GraphMerge => {
                if self.graph_commit_focused() {
                    None
                } else {
                    Some(FOCUS_A_GRAPH_COMMIT.into())
                }
            }
            Action::GraphFocusBranches => {
                if self.graph_focus_picker_repo().is_some() {
                    None
                } else {
                    Some(GRAPH_FOCUS_NEED_CONTEXT.into())
                }
            }
            Action::GraphFocusClear => {
                if self.graph_focus_picker_repo().is_none() {
                    Some(GRAPH_FOCUS_NEED_CONTEXT.into())
                } else if !self.graph_focus_is_active() {
                    Some("no graph focus to clear".into())
                } else {
                    None
                }
            }
            Action::ToggleFullContext => {
                if self.right_is_diff() && self.displayed_diff_id().is_some() {
                    None
                } else {
                    Some(FOCUS_A_FILE_DIFF.into())
                }
            }
            Action::DiffVisualStart => {
                if self.list_focus_target() != ListFocusTarget::None {
                    Some(FOCUS_A_FILE_DIFF.into())
                } else if self.current_diff_rows().is_empty() {
                    Some("no highlight target".into())
                } else {
                    None
                }
            }
            Action::Edit => {
                if self.focused_commit_edit_path().is_some() {
                    None
                } else if self.is_compare_tab() {
                    Some("focus a file to edit".into())
                } else if self.focused_file_if_shown().is_some() {
                    None
                } else {
                    Some("focus a dirty file to edit".into())
                }
            }
            Action::ExternalDiff => {
                if self.focused_commit_edit_path().is_some() {
                    None
                } else if self.is_compare_tab() {
                    Some("focus a file to diff".into())
                } else if self.focused_file_if_shown().is_some() {
                    None
                } else {
                    Some("focus a file to diff".into())
                }
            }
            Action::CommentStart => {
                if self.current_comment_target().is_some() {
                    None
                } else {
                    Some("no comment target".into())
                }
            }
            Action::ToggleReviewed => {
                if self.is_compare_tab() {
                    if self.focused_commit_edit_path().is_some() {
                        None
                    } else {
                        Some(FOCUS_A_FILE_TO_MARK_REVIEWED.into())
                    }
                } else if self.nav_depth() >= 1 {
                    Some(REVIEWED_MARKS_ARE_FOR_TREE_FILES.into())
                } else if self
                    .focused_row()
                    .is_some_and(|row| row.kind == super::super::tree::NodeKind::File)
                {
                    None
                } else {
                    Some(FOCUS_A_FILE_TO_MARK_REVIEWED.into())
                }
            }
            Action::CopyEntityReference => {
                if self.current_entity_reference().is_some() {
                    None
                } else {
                    Some("no copy target".into())
                }
            }
            Action::Branch => match self.focused_row() {
                Some(row) if can_open_branch_picker(&self.snapshot, row) => None,
                _ => Some("focus a checkout to pick a branch".into()),
            },
            Action::StashMenu => {
                if self.nav_depth() >= 2 {
                    Some(STASH_NEEDS_TREE_OR_GRAPH.into())
                } else if self.focused_checkout_if_shown().is_some() {
                    None
                } else {
                    Some("focus a visible repo to stash".into())
                }
            }
            Action::CloseTab => {
                if self.tabs.is_workspace() {
                    Some(WORKSPACE_TAB_CANNOT_CLOSE.into())
                } else {
                    None
                }
            }
            Action::NextTab | Action::PreviousTab => {
                (self.tabs.len() <= 1).then(|| ONLY_WORKSPACE_TAB_OPEN.into())
            }
            Action::SearchNext | Action::SearchPrev => {
                (!self.search_is_armed()).then(|| super::NO_SEARCH_ARMED.into())
            }
            Action::FoldToggleSubtree => (!matches!(
                self.list_focus_target(),
                ListFocusTarget::Tree | ListFocusTarget::CommitFiles
            ))
            .then(|| super::Z_FOLDS_TREE_ROWS.into()),
            _ => None,
        }
    }

    fn empty_file_write_reason(&self, write: FileWrite) -> Option<String> {
        let scoped = collect_write_files(&self.snapshot, self.focused_row(), self.show_ignored);
        let empty = scoped.iter().all(|file| match write {
            FileWrite::Stage => !super::is_stageable(&file.change),
            FileWrite::Unstage => !super::is_unstageable(&file.change),
        });
        empty.then(|| super::empty_write_status(self.focused_row().map(|row| row.kind), write))
    }

    fn can_remove_focused_worktree(&self) -> bool {
        let Some(row) = self.focused_row() else {
            return false;
        };
        if super::super::stash::row_is_hidden_ignored(row, self.show_ignored) {
            return false;
        }
        if !matches!(
            row.kind,
            super::super::tree::NodeKind::Checkout | super::super::tree::NodeKind::Repo
        ) {
            return false;
        }
        let Some(repo_path) = row.repo.as_deref() else {
            return false;
        };
        self.snapshot
            .repos
            .iter()
            .find(|repo| repo.repo == repo_path)
            .is_some_and(|snap| {
                snap.checkout_kind == crate::snapshot::CheckoutKind::Linked
                    && snap.primary_repo.is_some()
            })
    }
}

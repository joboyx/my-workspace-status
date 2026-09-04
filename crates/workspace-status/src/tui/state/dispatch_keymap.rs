//! List keys, search, help, view toggles, and mouse hit-test.

use std::time::Instant;

use super::super::action::{Action, Effect, ExternalDiffKind, PaletteOpenedBy};
use super::super::branches::can_open_branch_picker;
use super::super::command_palette::CommandPaletteState;
use super::super::gates::{dispatch_is_noop, ListFocusTarget};
use super::super::ops::{collect_write_files, op_is_kind_noop, Op};
use super::super::split::SplitDrag;
use super::super::tree::NodeKind;
use super::{AppState, FileWrite, FocusPane, FoldOp};

impl AppState {
    /// Apply a list / view / search / hit-test [`Action`].
    pub(crate) fn dispatch_keymap(&mut self, action: Action, fold_noop: bool) -> Effect {
        match action {
            Action::FoldToggle => {
                if !fold_noop {
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
                if self.help_open {
                    self.clear_diff_visual();
                }
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
            Action::ScrollDiff(delta) => {
                self.move_diff_cursor(delta);
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
            Action::SearchStart => {
                self.drag = SplitDrag::None;
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
            Action::ExternalDiff => {
                if let Some((repo, path)) = self.focused_commit_edit_path() {
                    let kind = self.external_diff_kind();
                    self.status = format!("diff {path}");
                    Effect::ExternalDiff { repo, path, kind }
                } else if let Some((repo, change)) = self.focused_file_if_shown() {
                    self.status = format!("diff {}", change.path);
                    Effect::ExternalDiff {
                        repo,
                        path: change.path,
                        kind: ExternalDiffKind::Worktree,
                    }
                } else {
                    self.status = "focus a file to diff".into();
                    Effect::None
                }
            }
            Action::WatchTick => Effect::WatchRefresh,
            Action::FetchTick => self.fetch_tick_effect(),
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
            Action::DiffVisualStart => self.begin_diff_visual(),
            Action::DiffVisualCancel => self.cancel_diff_visual(),
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
            Action::ExportCommentsCancel => {
                self.comment_export = None;
                self.status.clear();
                Effect::None
            }
            Action::Resize { cols, rows: _ } => {
                self.apply_terminal_size(cols);
                Effect::None
            }
            Action::ToggleCommandPalette(opened_by) => self.toggle_command_palette(opened_by),
            Action::CommandPaletteMove(delta) => {
                if let Some(palette) = self.command_palette.as_mut() {
                    palette.move_cursor(delta);
                }
                Effect::None
            }
            Action::CommandPaletteChar(c) => {
                if let Some(palette) = self.command_palette.as_mut() {
                    palette.push_char(c);
                }
                Effect::None
            }
            Action::CommandPaletteBackspace => {
                if let Some(palette) = self.command_palette.as_mut() {
                    palette.backspace();
                }
                Effect::None
            }
            Action::CommandPaletteSubmit => self.submit_command_palette(),
            Action::CommandPaletteCancel => {
                self.command_palette = None;
                Effect::None
            }
            Action::None => Effect::None,
            _ => Effect::None,
        }
    }

    fn toggle_command_palette(&mut self, opened_by: PaletteOpenedBy) -> Effect {
        if self.command_palette.is_some() {
            self.command_palette = None;
        } else {
            self.drag = SplitDrag::None;
            self.help_open = false;
            self.clear_help_search();
            self.command_palette = Some(CommandPaletteState::new(opened_by));
        }
        Effect::None
    }

    fn submit_command_palette(&mut self) -> Effect {
        let Some(command) = self
            .command_palette
            .as_ref()
            .and_then(|palette| palette.selected())
        else {
            return Effect::None;
        };
        if self.palette_disabled_reason(&command.action).is_some() {
            return Effect::None;
        }
        let action = command.action.clone();
        self.command_palette = None;
        self.dispatch(action)
    }

    /// Why the highlighted command cannot run, or `None` if Enter should dispatch.
    pub(crate) fn palette_disabled_reason(&self, action: &Action) -> Option<String> {
        if dispatch_is_noop(
            action,
            self.nav_depth(),
            self.focus == FocusPane::Right,
            self.list_focus_target(),
        ) {
            return Some("not available here".into());
        }
        match action {
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
                    .map(|_| "workspace / repo / checkout only".into())
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
                    Some("focus a graph stash row".into())
                }
            }
            Action::GraphCheckout | Action::GraphCreateBranch | Action::GraphMerge => {
                if self.graph_commit_focused() {
                    None
                } else {
                    Some("focus a graph commit".into())
                }
            }
            Action::GraphFocusBranches => {
                if self.graph_pane_focused() && self.graph_focus_repo().is_some() {
                    None
                } else {
                    Some("focus the graph pane".into())
                }
            }
            Action::GraphFocusClear => {
                if !self.graph_pane_focused() {
                    Some("focus the graph pane".into())
                } else if self.graph_branch_focus.is_none() {
                    Some("no graph focus to clear".into())
                } else {
                    None
                }
            }
            Action::ToggleFullContext => {
                if self.right_is_diff() && self.displayed_diff_id().is_some() {
                    None
                } else {
                    Some("focus a file diff".into())
                }
            }
            Action::DiffVisualStart => {
                if self.list_focus_target() != ListFocusTarget::None {
                    Some("focus a file diff".into())
                } else if self.current_diff_rows().is_empty() {
                    Some("no highlight target".into())
                } else {
                    None
                }
            }
            Action::Edit => {
                if self.focused_commit_edit_path().is_some()
                    || self.focused_file_if_shown().is_some()
                {
                    None
                } else {
                    Some("focus a dirty file to edit".into())
                }
            }
            Action::ExternalDiff => {
                if self.focused_commit_edit_path().is_some()
                    || self.focused_file_if_shown().is_some()
                {
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
                if self.nav_depth() >= 1 {
                    Some("not available here".into())
                } else if self
                    .focused_row()
                    .is_some_and(|row| row.kind == super::super::tree::NodeKind::File)
                {
                    None
                } else {
                    Some("focus a file to mark reviewed".into())
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
                    Some("not available here".into())
                } else if self.focused_checkout_if_shown().is_some() {
                    None
                } else {
                    Some("focus a visible repo to stash".into())
                }
            }
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

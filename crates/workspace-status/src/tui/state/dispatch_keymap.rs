//! List keys, search, help, view toggles, and mouse hit-test.

use std::time::Instant;

use super::super::action::{Action, Effect, ExternalDiffKind};
use super::super::gates::ListFocusTarget;
use super::super::split::SplitDrag;
use super::{AppState, FocusPane, FoldOp};

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
            Action::None => Effect::None,
            _ => Effect::None,
        }
    }
}

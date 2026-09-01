//! TUI state and Action dispatch.

mod dispatch;
mod dispatch_drill;
mod dispatch_keymap;
mod dispatch_write;
mod pan;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::style::Color;
use workspace_status_graph::{
    format_relative_date, graph_chrome_budget, paint_model, GraphChromeBudget, GraphModel,
    GraphRow, PaintedLine, ASCII, UNICODE,
};

use crate::snapshot::{
    carry_status_failed_local_branches, CheckoutKind, FileChange, WorkspaceSnapshot,
};

#[cfg(test)]
use super::action::Action;
use super::action::Effect;
use super::branches::{
    can_open_branch_picker, checkoutable_branch_names, is_valid_branch_name, merge_rev_for_commit,
    BranchPickerState, CreateBranchState, DIRTY_WORKTREE_STATUS,
};
#[cfg(not(test))]
use super::comments::comment_store_path;
use super::comments::{
    collect_live_set, comment_key_label, comments_in_focus_scope, export_markdown, gc_comments,
    load_comment_store, put_comment, resolve_comment_target, save_comment_store,
    viewport_line_number, viewport_line_range, CommentExport, CommentExportList, CommentKey,
    CommentPrompt, CommentStore,
};
use super::commit_files::{
    ancestor_dir_ids, collect_foldable_subtree_ids as collect_commit_subtree_ids,
    flatten_commit_files, CommitFileRow,
};
use super::ctrl_c_exit::{handle_ctrl_c, is_ctrl_c_exit_prompt, CTRL_C_EXIT_PROMPT};
use super::diff::{
    anchor_row_text, build_diff_rows, find_anchor_row, row_search_text, DiffContent, DiffRow,
};
use super::drill::{
    source_from_graph_row, stash_ref_from_graph_row, CommitFile, CommitFileSource, DrillView,
};
use super::fetch::background_fetch_targets;
use super::gates::ListFocusTarget;
use super::graph_focus::GraphFocusPickerState;
use super::keys::{InputMode, DOUBLE_TAP_MS};
use super::ops::{
    collect_write_files, format_running_op, op_is_kind_noop, op_targets, push_targets,
    refresh_target, should_delete_untracked, Op, RunningOp, ScopedFile,
};
use super::search::{
    focus_commit_file_search, focus_diff_search, focus_graph_search, focus_tree_search, SearchPane,
};
use super::split::{
    clamp_tree_fraction, diff_split_fraction_from_col, effective_diff_mode, graph_col_from_col,
    graph_col_from_delta, graph_scroll_from_delta, graph_scroll_from_row, hit_split,
    tree_fraction_from_col, DiffMode, SplitDrag, SplitHit, SplitLayout, DIFF_SPLIT_FRACTION,
    TREE_WIDTH_FRACTION,
};
use super::stash::{
    checkout_path, resolve_stash_menu_key, row_is_hidden_ignored, stash_dirty_for_row,
    stash_menu_status, stash_ops_for_context, StashMenuKeyResult, StashOp, StashOpId,
    StashOpsContext,
};
use super::theme::{cycle_theme_id, theme_from_env, ThemeId};
use super::tree::{
    build_tree, collect_foldable_subtree_ids, default_folds, flatten_with, visible_for_tree,
    visible_window, workspace_label_from_cwd, NodeKind, TreeNode, VisibleRow,
};
#[cfg(not(test))]
use super::viewed::viewed_store_path;
use super::viewed::{
    collect_current_fingerprints, fingerprint_file_change, is_viewed, load_viewed_store,
    reconcile_viewed, save_viewed_store, toggle_viewed, viewed_identity, viewed_row_ids,
    ViewedStore,
};
use super::watch::{
    capture_removal_ghosts, changed_row_ids, checkout_flash_ids, commit_file_identity,
    commit_file_signatures, flash_strength, flashable_row_ids, graph_flash_decision,
    graph_flash_meta, graph_row_identity, graph_row_signatures, is_new_row_set, merge_ghost_rows,
    prune_flashes, prune_ghosts, tree_signatures, GhostRow, GraphFlashDecision, GraphFlashMeta,
};
use crate::git::FULL_DIFF_CONTEXT_LINES;

/// Which pane has keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusPane {
    Left,
    Right,
}

/// Last painted layout, used for mouse hit testing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutHit {
    pub tree_x: u16,
    pub tree_y: u16,
    pub tree_width: u16,
    pub tree_height: u16,
    pub right_x: u16,
    pub list_offset: usize,
    pub term_cols: u16,
    pub pane_height: u16,
    pub outer_tree_width: u16,
    pub diff_pane_width: u16,
    /// Inner right-pane height (excludes the tree/graph border).
    pub diff_pane_height: u16,
    pub diff_content_x: u16,
    pub diff_split_rule_x: Option<u16>,
    pub right_y: u16,
    pub files_list_y: u16,
    pub files_list_offset: usize,
    /// 0-based graph scrollbar column when a graph list is painted.
    pub graph_scrollbar_x: Option<u16>,
    /// 0-based first row of the graph scrollbar track.
    pub graph_scrollbar_y: u16,
    /// Graph list height (scrollbar track).
    pub graph_scrollbar_height: u16,
    /// Painted graph line count.
    pub graph_content_len: usize,
    /// 0-based graph horizontal scrollbar row when the bar is painted.
    pub graph_hscrollbar_y: Option<u16>,
    /// 0-based first column of the graph horizontal scrollbar track.
    pub graph_hscrollbar_x: u16,
    /// Graph horizontal scrollbar track width.
    pub graph_hscrollbar_width: u16,
    /// Max horizontal pan for the painted graph.
    pub graph_col_max: u16,
    /// 0-based diff scrollbar column when a file diff is painted.
    pub diff_scrollbar_x: Option<u16>,
    /// 0-based first row of the diff scrollbar track (body, not header).
    pub diff_scrollbar_y: u16,
    /// Diff body height (vertical scrollbar track).
    pub diff_scrollbar_height: u16,
    /// 0-based diff horizontal scrollbar row when the bar is painted.
    pub diff_hscrollbar_y: Option<u16>,
    /// 0-based first column of the diff horizontal scrollbar track.
    pub diff_hscrollbar_x: u16,
    /// Diff horizontal scrollbar track width.
    pub diff_hscrollbar_width: u16,
}

impl Default for LayoutHit {
    fn default() -> Self {
        Self {
            tree_x: 0,
            tree_y: 1,
            tree_width: 40,
            tree_height: 20,
            right_x: 40,
            list_offset: 0,
            term_cols: 120,
            pane_height: 22,
            outer_tree_width: 48,
            diff_pane_width: 70,
            diff_pane_height: 20,
            diff_content_x: 50,
            diff_split_rule_x: None,
            right_y: 1,
            files_list_y: 3,
            files_list_offset: 0,
            graph_scrollbar_x: None,
            graph_scrollbar_y: 0,
            graph_scrollbar_height: 0,
            graph_content_len: 0,
            graph_hscrollbar_y: None,
            graph_hscrollbar_x: 0,
            graph_hscrollbar_width: 0,
            graph_col_max: 0,
            diff_scrollbar_x: None,
            diff_scrollbar_y: 0,
            diff_scrollbar_height: 0,
            diff_hscrollbar_y: None,
            diff_hscrollbar_x: 0,
            diff_hscrollbar_width: 0,
        }
    }
}

/// Confirm overlay before a destructive file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingConfirm {
    Revert {
        targets: Vec<RevertTarget>,
        /// Focused-row path shown in the overlay.
        label: String,
    },
    StashDrop {
        repo: String,
        stash_ref: String,
    },
    CheckoutOutOfSync {
        repo: String,
        branch: String,
        remote_ref: String,
    },
    RemoveWorktree {
        primary: String,
        path: String,
        force: bool,
        branch: String,
        merged_into_default: Option<bool>,
    },
    MergeIntoHead {
        repo: String,
        rev: String,
        label: String,
        into: String,
    },
}

/// One path in a pending revert confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevertTarget {
    pub repo: String,
    pub path: String,
    pub untracked: bool,
    pub old_path: Option<String>,
}

/// File-diff that owns `diff_cursor`, `diff_scroll`, and `diff_col_offset`.
///
/// A new identity resets those fields to the origin. A same-view reload
/// (watch, `Ctrl-o` after the hunk anchor) keeps them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DiffViewId {
    Workspace {
        repo: String,
        path: String,
    },
    Commit {
        repo: String,
        source: CommitFileSource,
        path: String,
    },
}

/// Interactive session state. Dispatch is pure besides the returned [`Effect`].
#[derive(Clone, Debug)]
pub struct AppState {
    pub cwd: PathBuf,
    pub snapshot: WorkspaceSnapshot,
    pub show_ignored: bool,
    /// Workspace directory tree vs flat paths. Session-only. Default true.
    pub tree_mode: bool,
    /// Commit-file directory tree vs flat paths. Independent of `tree_mode`.
    pub commit_tree_mode: bool,
    /// Folded commit-file dir ids for the current drill list.
    pub commit_file_folds: HashSet<String>,
    pub tree: TreeNode,
    pub folds: HashSet<String>,
    pub rows: Vec<VisibleRow>,
    pub cursor: usize,
    pub help_open: bool,
    pub help_search_query: Option<String>,
    pub focus: FocusPane,
    pub status: String,
    pub graph: Option<GraphModel>,
    pub graph_identity: Option<(String, String)>,
    pub graph_scroll: u16,
    pub graph_cursor: usize,
    /// True while the next `git log` window is fetching (`loading older…`).
    pub graph_loading_older: bool,
    /// True while listing commit files.
    pub commit_files_loading: bool,
    pub drill: DrillView,
    pub diff_content: DiffContent,
    pub diff_scroll: u16,
    /// Focused file-diff row (section, hunk, or line).
    pub diff_cursor: usize,
    pub diff_repo: Option<String>,
    pub diff_path: Option<String>,
    /// Painted file-diff identity. Viewport reset follows this, not paint.
    diff_view: Option<DiffViewId>,
    pub reviewed: HashSet<String>,
    pub viewed_store: ViewedStore,
    pub viewed_path: PathBuf,
    pub comment_store: CommentStore,
    pub comment_path: PathBuf,
    pub comment: Option<CommentPrompt>,
    pub comment_export: Option<CommentExport>,
    /// Visual-line anchor on a focused file diff (`V`). Cursor is the other end.
    pub diff_visual_anchor: Option<usize>,
    pub layout: LayoutHit,
    pub ascii: bool,
    pub search_mode: bool,
    pub search_active: bool,
    pub search_query: String,
    /// Pane bound when `/` started. `n`/`N` stay on this pane.
    pub search_target: SearchPane,
    /// Current matching diff line, when the bound pane is a diff.
    pub search_hit: Option<usize>,
    pub diff_col_offset: u16,
    /// Horizontal pan of the left list (workspace tree or drill graph/files).
    pub left_col_offset: u16,
    /// Horizontal pan of the right list (graph or commit-files).
    pub right_col_offset: u16,
    /// File identities currently shown with unlimited `-U` context.
    pub full_context: HashSet<String>,
    /// Search text of the hunk/change row to keep in view after `-U` reload.
    pending_hunk_anchor: Option<String>,
    pub confirm: Option<PendingConfirm>,
    pub stash_menu: Option<Vec<StashOp>>,
    pub stash_repo: Option<String>,
    pub branch_picker: Option<BranchPickerState>,
    pub graph_focus_picker: Option<GraphFocusPickerState>,
    /// Per-repo local branch names whose ancestors the graph shows. `None` = `--all`.
    pub graph_branch_focus: Option<(String, Vec<String>)>,
    pub create_branch: Option<CreateBranchState>,
    pub flashes: HashMap<String, Instant>,
    pub signatures: BTreeMap<String, String>,
    pub graph_signatures: BTreeMap<String, String>,
    graph_flash_meta: Option<GraphFlashMeta>,
    commit_file_signatures: BTreeMap<String, String>,
    tree_ghosts: Vec<GhostRow<VisibleRow>>,
    commit_file_ghosts: Vec<GhostRow<CommitFileRow>>,
    pub tree_fraction: f64,
    pub diff_split_fraction: f64,
    pub diff_mode: DiffMode,
    pub drag: SplitDrag,
    pub theme: ThemeId,
    pub mouse_enabled: bool,
    pub(crate) z_pending_at: Option<Instant>,
    pub(crate) g_pending_at: Option<Instant>,
    pub(crate) ctrl_c_armed_until: Option<Instant>,
    last_click: Option<(u16, u16, Instant)>,
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl AppState {
    pub fn new(cwd: PathBuf, snapshot: WorkspaceSnapshot, ascii: bool) -> Self {
        Self::with_viewed_path(cwd, snapshot, ascii, default_viewed_path())
    }

    pub(crate) fn with_viewed_path(
        cwd: PathBuf,
        snapshot: WorkspaceSnapshot,
        ascii: bool,
        viewed_path: PathBuf,
    ) -> Self {
        let show_ignored = snapshot.show_ignored;
        let tree_mode = true;
        let commit_tree_mode = true;
        let visible = visible_snapshot(&snapshot, show_ignored);
        let tree = build_tree(&visible, tree_mode, &workspace_label_from_cwd(&cwd));
        let folds = default_folds(&tree);
        let rows = flatten_with(&tree, &folds, ascii);
        let cursor = initial_cursor(&rows);
        let signatures = tree_signatures(&tree, &cwd);
        let viewed_store = load_viewed_store(&viewed_path);
        let comment_path = comment_path_for(&viewed_path);
        let comment_store = load_comment_store(&comment_path);
        let mut state = Self {
            cwd,
            snapshot,
            show_ignored,
            tree_mode,
            commit_tree_mode,
            commit_file_folds: HashSet::new(),
            tree,
            folds,
            rows,
            cursor,
            help_open: false,
            help_search_query: None,
            focus: FocusPane::Left,
            status: String::new(),
            graph: None,
            graph_identity: None,
            graph_scroll: 0,
            graph_cursor: 0,
            graph_loading_older: false,
            commit_files_loading: false,
            drill: DrillView::Graph,
            diff_content: DiffContent::default(),
            diff_scroll: 0,
            diff_cursor: 0,
            diff_repo: None,
            diff_path: None,
            diff_view: None,
            reviewed: HashSet::new(),
            viewed_store,
            viewed_path,
            comment_store,
            comment_path,
            comment: None,
            comment_export: None,
            diff_visual_anchor: None,
            layout: LayoutHit::default(),
            ascii,
            search_mode: false,
            search_active: false,
            search_query: String::new(),
            search_target: SearchPane::Tree,
            search_hit: None,
            diff_col_offset: 0,
            left_col_offset: 0,
            right_col_offset: 0,
            full_context: HashSet::new(),
            pending_hunk_anchor: None,
            confirm: None,
            stash_menu: None,
            stash_repo: None,
            branch_picker: None,
            graph_focus_picker: None,
            graph_branch_focus: None,
            create_branch: None,
            flashes: HashMap::new(),
            signatures,
            graph_signatures: BTreeMap::new(),
            graph_flash_meta: None,
            commit_file_signatures: BTreeMap::new(),
            tree_ghosts: Vec::new(),
            commit_file_ghosts: Vec::new(),
            tree_fraction: TREE_WIDTH_FRACTION,
            diff_split_fraction: DIFF_SPLIT_FRACTION,
            diff_mode: DiffMode::SideBySide,
            drag: SplitDrag::None,
            theme: theme_from_env(),
            mouse_enabled: true,
            z_pending_at: None,
            g_pending_at: None,
            ctrl_c_armed_until: None,
            last_click: None,
        };
        state.reconcile_viewed_store();
        state.reconcile_comment_store();
        state
    }

    pub fn input_mode(&self) -> InputMode {
        if self.confirm.is_some() {
            InputMode::Confirm
        } else if self.stash_menu.is_some() {
            InputMode::StashMenu
        } else if self.comment.is_some() {
            InputMode::Comment
        } else if self.comment_export.is_some() {
            InputMode::CommentExport
        } else if self.create_branch.is_some() {
            InputMode::CreateBranch
        } else if self.branch_picker.is_some() {
            InputMode::BranchPicker
        } else if self.graph_focus_picker.is_some() {
            InputMode::GraphFocusPicker
        } else if self.help_open {
            if self.help_search_query.is_some() {
                InputMode::HelpSearch
            } else {
                InputMode::Help
            }
        } else if self.search_mode {
            InputMode::SearchPrompt
        } else if self.diff_visual_anchor.is_some()
            && self.list_focus_target() == ListFocusTarget::None
        {
            InputMode::DiffVisual
        } else if self.chord_pending(self.z_pending_at) {
            InputMode::ZPending {
                search_active: self.search_active,
            }
        } else if self.chord_pending(self.g_pending_at) {
            InputMode::GPending {
                search_active: self.search_active,
            }
        } else {
            InputMode::Normal {
                search_active: self.search_active,
            }
        }
    }

    fn chord_pending(&self, at: Option<Instant>) -> bool {
        at.is_some_and(|t| t.elapsed() <= Duration::from_millis(DOUBLE_TAP_MS))
    }

    pub fn focused_row(&self) -> Option<&VisibleRow> {
        self.rows.get(self.cursor)
    }

    /// Checkout path for the focused tree row, if any.
    pub fn focused_checkout_path(&self) -> Option<String> {
        self.focused_row().and_then(|row| row.repo.clone())
    }

    pub fn right_is_diff(&self) -> bool {
        match &self.drill {
            DrillView::Diff { .. } => true,
            DrillView::Files { .. } => false,
            DrillView::Graph => matches!(self.focused_row().map(|r| r.kind), Some(NodeKind::File)),
        }
    }

    pub fn graph_stash_focused(&self) -> bool {
        self.graph_pane_focused() && self.focused_graph_stash_ref().is_some()
    }

    pub fn graph_commit_focused(&self) -> bool {
        self.graph_pane_focused()
            && matches!(self.focused_graph_row(), Some(GraphRow::Commit { .. }))
    }

    pub fn in_commit_drill(&self) -> bool {
        self.drill.is_files() || self.drill.is_diff()
    }

    /// ViewStack depth: 0 workspace, 1 commit files, 2 commit diff.
    fn nav_depth(&self) -> u8 {
        match self.drill {
            DrillView::Graph => 0,
            DrillView::Files { .. } => 1,
            DrillView::Diff { .. } => 2,
        }
    }

    /// True when unshifted `h` / `l` fold the workspace tree.
    pub(crate) fn hl_folds(&self) -> bool {
        self.list_focus_target() == ListFocusTarget::Tree
    }

    /// Which list (or diff) the focused pane is driving.
    ///
    /// Depth 0 left is the workspace tree; depth 1 left is the graph; depth 2
    /// left is the commit-file list. Right at depth 2 (and depth 0 file diffs)
    /// is `None` so `j`/`k` move the focused file-diff row. The viewport keeps
    /// that row near the vertical middle.
    pub(crate) fn list_focus_target(&self) -> ListFocusTarget {
        match &self.drill {
            DrillView::Graph => {
                if self.focus == FocusPane::Left {
                    ListFocusTarget::Tree
                } else if self.right_is_diff() {
                    ListFocusTarget::None
                } else {
                    ListFocusTarget::Graph
                }
            }
            DrillView::Files { .. } => {
                if self.focus == FocusPane::Left {
                    ListFocusTarget::Graph
                } else {
                    ListFocusTarget::CommitFiles
                }
            }
            DrillView::Diff { .. } => {
                if self.focus == FocusPane::Left {
                    ListFocusTarget::CommitFiles
                } else {
                    ListFocusTarget::None
                }
            }
        }
    }

    pub(crate) fn commit_files_list_focused(&self) -> bool {
        self.list_focus_target() == ListFocusTarget::CommitFiles
    }

    pub fn graph_pane_focused(&self) -> bool {
        self.list_focus_target() == ListFocusTarget::Graph
    }

    fn commit_drill_files(&self) -> Option<&[CommitFile]> {
        match &self.drill {
            DrillView::Files { files, .. } | DrillView::Diff { files, .. } => Some(files),
            DrillView::Graph => None,
        }
    }

    fn files_list_origin_x(&self) -> u16 {
        if self.drill.is_diff() {
            self.layout.tree_x
        } else {
            self.layout.diff_content_x
        }
    }

    fn set_commit_file_cursor(&mut self, idx: usize) {
        match &mut self.drill {
            DrillView::Files { cursor, .. } => *cursor = idx,
            DrillView::Diff { file_cursor, .. } => *file_cursor = idx,
            DrillView::Graph => {}
        }
    }

    /// Graph-row source for the right pane at depth ≥ 1.
    pub(crate) fn follow_commit_source(&self) -> Option<(String, CommitFileSource)> {
        let repo = self.focused_graph_repo()?;
        let source = source_from_graph_row(&self.focused_graph_row()?)?;
        Some((repo, source))
    }

    /// Load commit files for the focused graph row (depth 1 left).
    ///
    /// Depth 0 right is the graph itself — no follow. Same source is a no-op.
    fn follow_graph_files(&self) -> Effect {
        let DrillView::Files { repo, source, .. } = &self.drill else {
            return Effect::None;
        };
        match self.follow_commit_source() {
            Some((next_repo, next_source)) if next_repo == *repo && next_source == *source => {
                Effect::None
            }
            Some(_) => Effect::LoadRightPane,
            None => Effect::None,
        }
    }

    /// Load the focused file's commit diff through `load_right`.
    ///
    /// Directory rows and the already-shown path keep the previous diff.
    fn maybe_load_focused_commit_diff(&self) -> Effect {
        let DrillView::Diff { path, .. } = &self.drill else {
            return Effect::None;
        };
        let Some(row) = self.focused_commit_file_row() else {
            return Effect::None;
        };
        if !row.is_file() || row.path == *path {
            return Effect::None;
        }
        Effect::LoadRightPane
    }

    /// Header / footer / list split for the graph pane.
    pub fn graph_chrome(&self) -> GraphChromeBudget {
        graph_chrome_budget(
            self.layout.tree_height.max(1),
            self.graph_loading_older,
            self.graph.as_ref().is_some_and(|g| g.sync.is_some()),
        )
    }

    pub fn commit_file_rows(&self) -> Vec<CommitFileRow> {
        let files = match &self.drill {
            DrillView::Files { files, .. } | DrillView::Diff { files, .. } => files.as_slice(),
            DrillView::Graph => return Vec::new(),
        };
        flatten_commit_files(
            files,
            self.commit_tree_mode,
            &self.commit_file_folds,
            self.ascii,
        )
    }

    pub(crate) fn commit_files_cursor(&self) -> usize {
        match &self.drill {
            DrillView::Files { cursor, .. } => *cursor,
            DrillView::Diff { file_cursor, .. } => *file_cursor,
            DrillView::Graph => 0,
        }
    }

    /// Highlighted commit-file row when the files list is open.
    pub(crate) fn focused_commit_file_row(&self) -> Option<CommitFileRow> {
        if self.drill.is_graph() {
            return None;
        }
        let rows = self.commit_file_rows();
        rows.get(self.commit_files_cursor()).cloned()
    }

    /// Kind of the highlighted commit-file row, when the files list is open.
    pub(crate) fn focused_commit_file_kind(
        &self,
    ) -> Option<super::commit_files::CommitFileRowKind> {
        self.focused_commit_file_row().map(|row| row.kind)
    }

    fn focused_commit_edit_path(&self) -> Option<(String, String)> {
        let repo = match &self.drill {
            DrillView::Files { repo, .. } | DrillView::Diff { repo, .. } => repo.clone(),
            DrillView::Graph => return None,
        };
        if self.commit_files_list_focused() {
            let row = self.focused_commit_file_row()?;
            if !row.is_file() {
                return None;
            }
            return Some((repo, row.path));
        }
        match &self.drill {
            DrillView::Diff { path, .. } => Some((repo, path.clone())),
            _ => None,
        }
    }

    pub(crate) fn commit_detail_meta(&self) -> (String, Option<String>) {
        let (repo, source) = match &self.drill {
            DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. } => {
                (repo.as_str(), source)
            }
            DrillView::Graph => return (String::new(), None),
        };
        let title = repo.rsplit('/').next().unwrap_or(repo).to_string();
        let subtitle = match source {
            CommitFileSource::Worktree => Some("Uncommitted changes".into()),
            CommitFileSource::Stash { stash_ref } => {
                let subject = self.graph.as_ref().and_then(|model| {
                    model
                        .stashes
                        .iter()
                        .find(|stash| stash.stash_ref == *stash_ref)
                        .map(|stash| stash.subject.clone())
                });
                Some(match subject {
                    Some(subject) if !subject.is_empty() => format!("{stash_ref} · {subject}"),
                    _ => stash_ref.clone(),
                })
            }
            CommitFileSource::Commit { commit_id } => {
                let short = if commit_id.len() >= 7 {
                    &commit_id[..7]
                } else {
                    commit_id.as_str()
                };
                let commit = self
                    .graph
                    .as_ref()
                    .and_then(|model| model.commits.iter().find(|commit| commit.id == *commit_id));
                let refs = commit
                    .map(|commit| {
                        commit
                            .refs
                            .iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let subject = commit.map(|c| c.subject.as_str()).unwrap_or("");
                let mut bits = vec![short.to_string()];
                if !refs.is_empty() {
                    bits.push(refs);
                }
                if !subject.is_empty() {
                    bits.push(subject.to_string());
                }
                if let Some(commit) = commit {
                    if !commit.author_name.is_empty() {
                        bits.push(commit.author_name.clone());
                    }
                    if commit.author_date_unix != 0 {
                        bits.push(format_relative_date(commit.author_date_unix, unix_now()));
                    }
                }
                Some(bits.join(" · "))
            }
        };
        (title, subtitle)
    }

    fn restore_commit_file_cursor(&mut self, path: Option<&str>) {
        let rows = self.commit_file_rows();
        if rows.is_empty() {
            self.set_commit_file_cursor(0);
            return;
        }
        let idx = path
            .and_then(|path| {
                rows.iter()
                    .position(|row| row.is_file() && row.path == path)
                    .or_else(|| rows.iter().position(|row| row.path == path))
            })
            .unwrap_or(0)
            .min(rows.len() - 1);
        self.set_commit_file_cursor(idx);
    }

    fn visible_tree(&self) -> TreeNode {
        let visible = visible_snapshot(&self.snapshot, self.show_ignored);
        build_tree(
            &visible,
            self.tree_mode,
            &workspace_label_from_cwd(&self.cwd),
        )
    }

    pub fn rebuild_rows(&mut self) {
        let focus_id = self.focused_row().map(|r| r.id.clone());
        self.tree = self.visible_tree();
        self.rows = flatten_with(&self.tree, &self.folds, self.ascii);
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        if let Some(id) = focus_id {
            if let Some(idx) = self.rows.iter().position(|r| r.id == id) {
                self.cursor = idx;
                return;
            }
        }
        self.cursor = self.cursor.min(self.rows.len() - 1);
    }

    fn toggle_tree_mode(&mut self) -> Effect {
        if self.in_commit_drill() {
            return self.toggle_commit_tree_mode();
        }
        let previous_id = self.focused_row().map(|row| row.id.clone());
        self.tree_mode = !self.tree_mode;
        self.tree = self.visible_tree();
        self.folds = default_folds(&self.tree);
        self.rows = flatten_with(&self.tree, &self.folds, self.ascii);
        self.restore_focus_after_tree_rebuild(previous_id);
        self.signatures = tree_signatures(&self.tree, &self.cwd);
        self.tree_ghosts.clear();
        self.status = if self.tree_mode {
            "Directory tree".into()
        } else {
            "Flat paths".into()
        };
        Effect::LoadRightPane
    }

    fn toggle_commit_tree_mode(&mut self) -> Effect {
        let path = self.focused_commit_file_row().map(|row| row.path);
        self.commit_tree_mode = !self.commit_tree_mode;
        self.commit_file_folds.clear();
        if !self.drill.is_graph() {
            self.restore_commit_file_cursor(path.as_deref());
        }
        self.status = if self.commit_tree_mode {
            "Directory tree".into()
        } else {
            "Flat paths".into()
        };
        self.reseed_commit_file_signatures();
        Effect::None
    }

    fn restore_focus_after_tree_rebuild(&mut self, previous_id: Option<String>) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        if let Some(id) = previous_id {
            if let Some(idx) = self.rows.iter().position(|row| row.id == id) {
                self.cursor = idx;
                return;
            }
            for ancestor in focus_ancestor_ids(&id) {
                if let Some(idx) = self.rows.iter().position(|row| row.id == ancestor) {
                    self.cursor = idx;
                    return;
                }
            }
        }
        self.cursor = self.cursor.min(self.rows.len() - 1);
    }

    pub fn apply_snapshot(&mut self, snapshot: WorkspaceSnapshot) {
        let snapshot = carry_status_failed_local_branches(&self.snapshot, snapshot);
        self.snapshot = snapshot;
        self.snapshot.show_ignored = self.show_ignored;
        self.rebuild_rows();
        self.signatures = tree_signatures(&self.tree, &self.cwd);
        self.reconcile_viewed_store();
        self.reconcile_comment_store();
    }

    /// Apply a watch poll. Keeps fold / focus / scroll. Flashes only rows
    /// whose identity actually changed.
    pub fn apply_watch_snapshot(&mut self, snapshot: WorkspaceSnapshot) -> Vec<String> {
        let focus_id = self.focused_row().map(|r| r.id.clone());
        let folds = self.folds.clone();
        let graph_scroll = self.graph_scroll;
        let diff_scroll = self.diff_scroll;
        let diff_cursor = self.diff_cursor;
        let before = self.signatures.clone();
        let old_rows = self.rows.clone();
        self.apply_snapshot(snapshot);
        self.folds = folds;
        self.rebuild_rows();
        if let Some(id) = focus_id {
            if let Some(idx) = self.rows.iter().position(|r| r.id == id) {
                self.cursor = idx;
            }
        }
        self.graph_scroll = graph_scroll;
        self.diff_scroll = diff_scroll;
        self.diff_cursor = diff_cursor;
        let now = Instant::now();
        self.prune_flash_state(now);
        if is_new_row_set(&before, &self.signatures) {
            self.tree_ghosts.clear();
            return Vec::new();
        }
        self.tree_ghosts.extend(capture_removal_ghosts(
            &old_rows,
            |row| row.id.as_str(),
            |row| row.id.clone(),
            &before,
            &self.signatures,
            now,
        ));
        let changed = changed_row_ids(&before, &self.signatures);
        for id in &changed {
            self.flashes.insert(id.clone(), now);
        }
        changed
    }

    fn ctrl_c(&mut self, now: Instant) -> Effect {
        let result = handle_ctrl_c(self.ctrl_c_armed_until, now);
        self.ctrl_c_armed_until = result.armed_until;
        if result.quit {
            return Effect::Quit;
        }
        if result.prompt {
            self.status = CTRL_C_EXIT_PROMPT.into();
        }
        Effect::None
    }

    /// Disarm an expired Ctrl-C window. Clears the prompt only when it is still showing.
    pub fn expire_ctrl_c_prompt(&mut self, now: Instant) -> bool {
        let Some(until) = self.ctrl_c_armed_until else {
            return false;
        };
        if now < until {
            return false;
        }
        self.ctrl_c_armed_until = None;
        if is_ctrl_c_exit_prompt(&self.status) {
            self.status.clear();
            true
        } else {
            false
        }
    }

    /// Milliseconds left on the Ctrl-C arm, if one is active.
    pub fn ctrl_c_remaining_ms(&self, now: Instant) -> Option<u64> {
        let until = self.ctrl_c_armed_until?;
        Some(until.saturating_duration_since(now).as_millis() as u64)
    }

    fn prune_flash_state(&mut self, now: Instant) {
        prune_flashes(&mut self.flashes, now);
        prune_ghosts(&mut self.tree_ghosts, now);
        prune_ghosts(&mut self.commit_file_ghosts, now);
    }

    /// Drop expired flashes and ghosts before paint / the next tick.
    pub fn prune_expired_flashes(&mut self) {
        self.prune_flash_state(Instant::now());
    }

    /// True when a flash or ghost is still decaying.
    pub fn has_active_flashes(&self) -> bool {
        !self.flashes.is_empty()
            || !self.tree_ghosts.is_empty()
            || !self.commit_file_ghosts.is_empty()
    }

    /// Fade colour for a row id, if it is still flashing.
    pub fn flash_color(&self, id: &str) -> Option<Color> {
        let at = self.flashes.get(id)?;
        let strength = flash_strength(Instant::now().saturating_duration_since(*at));
        self.theme.palette().flash_bg(strength)
    }

    fn stamp_flashes(&mut self, ids: impl IntoIterator<Item = String>, now: Instant) {
        for id in ids {
            self.flashes.insert(id, now);
        }
    }

    /// Flash checkout chrome after fetch / pull / push / default-branch.
    pub fn stamp_checkout_flashes(&mut self, repos: &[String]) {
        let now = Instant::now();
        self.prune_flash_state(now);
        for repo in repos {
            self.stamp_flashes(checkout_flash_ids(repo), now);
        }
    }

    /// Tree rows including removal ghosts for the flash window.
    pub fn painted_tree_rows(&self) -> Vec<VisibleRow> {
        merge_ghost_rows(&self.rows, &self.tree_ghosts, |row| row.id.as_str())
    }

    /// Commit-file rows including removal ghosts for the flash window.
    pub fn painted_commit_file_rows(&self) -> Vec<CommitFileRow> {
        merge_ghost_rows(&self.commit_file_rows(), &self.commit_file_ghosts, |row| {
            row.id.as_str()
        })
    }

    /// Visible-row indexes plus fade colours for the graph widget.
    pub fn graph_flash_rows(&self) -> Vec<(usize, Color)> {
        let Some(model) = self.graph.as_ref() else {
            return Vec::new();
        };
        let Some((repo, _)) = self.graph_identity.as_ref() else {
            return Vec::new();
        };
        model
            .visible_rows()
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let id = graph_row_identity(repo, row);
                self.flash_color(&id).map(|color| (i, color))
            })
            .collect()
    }

    /// Flash colour for a commit-file row in the current drill.
    pub fn commit_file_flash_color(&self, row_id: &str) -> Option<Color> {
        let (repo, source) = match &self.drill {
            DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. } => {
                (repo, source)
            }
            DrillView::Graph => return None,
        };
        self.flash_color(&commit_file_identity(repo, source, row_id))
    }

    fn unfolded_commit_file_rows(&self, files: &[CommitFile]) -> Vec<CommitFileRow> {
        flatten_commit_files(files, self.commit_tree_mode, &HashSet::new(), self.ascii)
    }

    fn reseed_commit_file_signatures(&mut self) {
        self.commit_file_ghosts.clear();
        let (repo, source, files) = match &self.drill {
            DrillView::Files {
                repo,
                source,
                files,
                ..
            }
            | DrillView::Diff {
                repo,
                source,
                files,
                ..
            } => (repo.clone(), source.clone(), files.clone()),
            DrillView::Graph => {
                self.commit_file_signatures.clear();
                return;
            }
        };
        let unfolded = self.unfolded_commit_file_rows(&files);
        self.commit_file_signatures = commit_file_signatures(&repo, &source, &unfolded);
    }

    fn cycle_theme(&mut self) -> Effect {
        self.theme = cycle_theme_id(self.theme);
        self.status = format!("theme: {}", self.theme.label());
        Effect::None
    }

    fn op_effect(&mut self, op: Op) -> Effect {
        if self
            .focused_row()
            .is_some_and(|row| op_is_kind_noop(row.kind, op))
        {
            return Effect::None;
        }
        let targets = op_targets(&self.snapshot, self.focused_row(), self.show_ignored, op);
        if targets.is_empty() {
            self.status = "no visible repos for that op".into();
            return Effect::None;
        }
        match op {
            Op::Fetch => {
                self.status = format_running_op(RunningOp::Fetch, 0, targets.len());
                Effect::Fetch { repos: targets }
            }
            Op::Pull => {
                let behind: Vec<String> = targets
                    .into_iter()
                    .filter(|repo| {
                        self.snapshot.repos.iter().any(|r| {
                            r.repo == *repo && r.sync_status == crate::snapshot::SyncStatus::Behind
                        })
                    })
                    .collect();
                if behind.is_empty() {
                    self.status = "nothing behind to pull".into();
                    Effect::None
                } else {
                    self.status = format_running_op(RunningOp::Pull, 0, behind.len());
                    Effect::Pull { repos: behind }
                }
            }
            Op::DefaultBranch => {
                let repos = targets
                    .into_iter()
                    .filter(|repo| {
                        self.snapshot.repos.iter().any(|r| {
                            r.repo == *repo
                                && !crate::helpers::is_default_branch(
                                    &r.branch,
                                    r.default_branch_override.as_deref(),
                                )
                        })
                    })
                    .collect::<Vec<_>>();
                if repos.is_empty() {
                    self.status = "no non-default branches to switch".into();
                    Effect::None
                } else {
                    self.status = format_running_op(RunningOp::DefaultBranch, 0, repos.len());
                    Effect::DefaultBranch { repos }
                }
            }
        }
    }

    fn refresh_effect(&self) -> Effect {
        match refresh_target(self.focused_row()) {
            None => Effect::ReloadSnapshot,
            Some(repo) => Effect::ReloadRepo { repo },
        }
    }

    fn move_cursor(&mut self, delta: i32) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as i32 + delta;
        self.cursor = next.clamp(0, self.rows.len() as i32 - 1) as usize;
    }

    fn clear_help_search(&mut self) {
        self.help_search_query = None;
    }

    /// Apply the focused row's current fold state to foldable descendants.
    ///
    /// The first `z` already toggled this row and armed the 400ms window.
    /// The second `z` must not toggle it again. Descendants follow the
    /// focused row: a folded parent folds the subtree; an open parent
    /// opens it.
    fn fold_subtree(&mut self) {
        if self.commit_files_list_focused() {
            let Some(row) = self.focused_commit_file_row() else {
                return;
            };
            let id = row.id.clone();
            let path = row.path.clone();
            let Some(files) = self.commit_drill_files().map(|files| files.to_vec()) else {
                return;
            };
            let ids = collect_commit_subtree_ids(&files, self.commit_tree_mode, &id);
            if ids.is_empty() {
                return;
            }
            let fold_descendants = self.commit_file_folds.contains(&id);
            for sid in ids {
                if sid == id {
                    continue;
                }
                if fold_descendants {
                    self.commit_file_folds.insert(sid);
                } else {
                    self.commit_file_folds.remove(&sid);
                }
            }
            self.restore_commit_file_cursor(Some(&path));
            return;
        }
        if self.list_focus_target() != ListFocusTarget::Tree {
            return;
        }
        let Some(row) = self.focused_row().cloned() else {
            return;
        };
        let ids = collect_foldable_subtree_ids(&self.tree, &row.id);
        if ids.is_empty() {
            return;
        }
        let fold_descendants = self.folds.contains(&row.id);
        for sid in ids {
            if sid == row.id {
                continue;
            }
            if fold_descendants {
                self.folds.insert(sid);
            } else {
                self.folds.remove(&sid);
            }
        }
        self.rebuild_rows();
    }

    fn fold_op(&mut self, op: FoldOp) {
        if self.commit_files_list_focused() {
            self.fold_commit_file(op);
            return;
        }
        if self.list_focus_target() != ListFocusTarget::Tree {
            return;
        }
        let Some(row) = self.focused_row().cloned() else {
            return;
        };
        if !row.foldable {
            return;
        }
        match op {
            FoldOp::Toggle => {
                if !self.folds.remove(&row.id) {
                    self.folds.insert(row.id);
                }
            }
            FoldOp::Close => {
                self.folds.insert(row.id);
            }
            FoldOp::Open => {
                self.folds.remove(&row.id);
            }
        }
        self.rebuild_rows();
    }

    fn fold_commit_file(&mut self, op: FoldOp) {
        let Some(row) = self.focused_commit_file_row() else {
            return;
        };
        if !row.foldable {
            return;
        }
        match op {
            FoldOp::Toggle => {
                if !self.commit_file_folds.remove(&row.id) {
                    self.commit_file_folds.insert(row.id);
                }
            }
            FoldOp::Close => {
                self.commit_file_folds.insert(row.id);
            }
            FoldOp::Open => {
                self.commit_file_folds.remove(&row.id);
            }
        }
        self.restore_commit_file_cursor(Some(&row.path));
    }

    fn diff_body_height(&self) -> usize {
        let h_bar = u16::from(self.diff_col_offset > 0);
        self.layout
            .diff_pane_height
            .saturating_sub(1 + h_bar)
            .max(1) as usize
    }

    fn split_layout(&self) -> SplitLayout {
        SplitLayout {
            term_cols: self.layout.term_cols.max(1),
            term_rows: self.layout.pane_height.saturating_add(2).max(1),
            pane_height: self.layout.pane_height.max(1),
            tree_width: self.layout.outer_tree_width.max(1),
            diff_pane_width: self.layout.diff_pane_width,
            diff_content_x: self.layout.diff_content_x,
            diff_split_rule_x: self.layout.diff_split_rule_x,
            graph_scrollbar_x: self.layout.graph_scrollbar_x,
            graph_scrollbar_y: self.layout.graph_scrollbar_y,
            graph_scrollbar_height: self.layout.graph_scrollbar_height,
            graph_content_len: self.layout.graph_content_len,
            graph_scroll: self.graph_scroll,
            graph_hscrollbar_y: self.layout.graph_hscrollbar_y,
            graph_hscrollbar_x: self.layout.graph_hscrollbar_x,
            graph_hscrollbar_width: self.layout.graph_hscrollbar_width,
            graph_col_max: self.layout.graph_col_max,
            graph_col_offset: if self.drill.is_files() {
                self.left_col_offset
            } else {
                self.right_col_offset
            },
        }
    }

    fn apply_tree_fraction_from_col(&mut self, col: u16) {
        let cols = self.layout.term_cols.max(1);
        self.tree_fraction = clamp_tree_fraction(cols, tree_fraction_from_col(cols, col));
    }

    fn apply_diff_fraction_from_col(&mut self, col: u16) {
        self.diff_split_fraction = diff_split_fraction_from_col(
            self.layout.outer_tree_width.max(1),
            self.layout.diff_pane_width.max(1),
            col,
        );
    }

    fn click(&mut self, col: u16, row: u16) -> Effect {
        match hit_split(self.split_layout(), col, row) {
            SplitHit::Pane => {
                self.drag = SplitDrag::Pane;
                self.apply_tree_fraction_from_col(col);
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::DiffSplit => {
                self.drag = SplitDrag::Diff;
                self.apply_diff_fraction_from_col(col);
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphThumb => {
                self.focus_graph_pane();
                self.drag = SplitDrag::GraphScrollbar {
                    origin_row: row,
                    origin_scroll: self.graph_scroll,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphTrack => {
                self.focus_graph_pane();
                let jumped = graph_scroll_from_row(self.split_layout(), row);
                self.graph_scroll = jumped;
                self.drag = SplitDrag::GraphScrollbar {
                    origin_row: row,
                    origin_scroll: jumped,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphHThumb => {
                self.focus_graph_pane();
                self.drag = SplitDrag::GraphHScrollbar {
                    origin_col: col,
                    origin_offset: self.graph_col_offset(),
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphHTrack => {
                self.focus_graph_pane();
                let jumped = graph_col_from_col(self.split_layout(), col);
                self.set_graph_col_offset(jumped);
                self.drag = SplitDrag::GraphHScrollbar {
                    origin_col: col,
                    origin_offset: jumped,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::Other => {}
        }
        let now = Instant::now();
        let is_double = self.last_click.as_ref().is_some_and(|(c, r, at)| {
            *c == col && *r == row && at.elapsed() <= Duration::from_millis(400)
        });
        self.last_click = Some((col, row, now));
        if col >= self.layout.right_x {
            self.focus = FocusPane::Right;
            if self.drill.is_files() {
                self.clear_diff_visual();
                return self.click_commit_files(col, row, is_double);
            }
            if !self.right_is_diff() && self.graph.is_some() {
                self.clear_diff_visual();
                return self.click_graph(row, is_double, true);
            }
            if self.right_is_diff() || self.drill.is_diff() {
                self.click_diff(row);
                if is_double {
                    return self.nav_enter();
                }
                return Effect::None;
            }
            if is_double {
                return self.nav_enter();
            }
            return Effect::None;
        }
        self.focus = FocusPane::Left;
        self.clear_diff_visual();
        if self.drill.is_diff() {
            return self.click_commit_files(col, row, is_double);
        }
        if self.drill.is_files() {
            return self.click_graph(row, is_double, false);
        }
        if row < self.layout.tree_y {
            return Effect::None;
        }
        let idx = self.layout.list_offset + (row - self.layout.tree_y) as usize;
        let painted = self.painted_tree_rows();
        let Some(tree_row) = painted.get(idx).cloned() else {
            return Effect::None;
        };
        let Some(live) = self.rows.iter().position(|r| r.id == tree_row.id) else {
            return Effect::None;
        };
        self.cursor = live;
        if tree_row.foldable && self.is_tree_chevron(col, tree_row.depth) {
            // Two Downs are a double-click. Skip the second toggle so the
            // fold from the first click remains. Do not Enter.
            if is_double {
                return Effect::None;
            }
            self.fold_op(FoldOp::Toggle);
            return Effect::LoadRightPane;
        }
        if is_double {
            return self.nav_enter();
        }
        Effect::LoadRightPane
    }

    /// Select the file-diff row under `row` and keep it near the middle.
    fn click_diff(&mut self, row: u16) {
        let body_y = self.layout.right_y.saturating_add(1);
        if row < body_y {
            return;
        }
        if self.layout.diff_hscrollbar_y == Some(row) {
            return;
        }
        let n = self.current_diff_rows().len();
        if n == 0 {
            return;
        }
        let idx = self.diff_scroll as usize + (row - body_y) as usize;
        if idx >= n {
            return;
        }
        self.diff_cursor = idx;
        self.sync_diff_scroll();
    }

    fn is_tree_chevron(&self, col: u16, depth: usize) -> bool {
        col == self.layout.tree_x + 1 + (depth as u16) * 2
    }

    fn is_files_chevron(&self, col: u16, depth: usize) -> bool {
        col == self.files_list_origin_x() + 1 + (depth as u16) * 2
    }

    fn click_graph(&mut self, row: u16, is_double: bool, right: bool) -> Effect {
        let y = if right {
            self.layout.right_y
        } else {
            self.layout.tree_y
        };
        if row < y {
            return Effect::None;
        }
        let Some(model) = self.graph.as_ref() else {
            return Effect::None;
        };
        let chrome = graph_chrome_budget(
            self.layout.tree_height.max(1),
            self.graph_loading_older,
            model.sync.is_some(),
        );
        let mut offset = (row - y) as usize;
        if chrome.header {
            if offset == 0 {
                return Effect::None;
            }
            offset -= 1;
        }
        if offset >= chrome.list_height as usize {
            return Effect::None;
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        let Some(line) = painted.get(self.graph_scroll as usize + offset) else {
            return Effect::None;
        };
        if let Some(idx) = line.row_index {
            self.graph_cursor = idx;
            self.sync_graph_scroll();
            if is_double {
                return self.nav_enter();
            }
            return self.follow_graph_files();
        }
        Effect::None
    }

    fn click_commit_files(&mut self, col: u16, row: u16, is_double: bool) -> Effect {
        if row < self.layout.files_list_y {
            return Effect::None;
        }
        let idx = self.layout.files_list_offset + (row - self.layout.files_list_y) as usize;
        let painted = self.painted_commit_file_rows();
        let Some(file_row) = painted.get(idx).cloned() else {
            return Effect::None;
        };
        let live_rows = self.commit_file_rows();
        let Some(live) = live_rows.iter().position(|r| r.id == file_row.id) else {
            return Effect::None;
        };
        self.set_commit_file_cursor(live);
        if file_row.foldable && self.is_files_chevron(col, file_row.depth) {
            if is_double {
                return Effect::None;
            }
            self.fold_commit_file(FoldOp::Toggle);
            return Effect::None;
        }
        let load = self.maybe_load_focused_commit_diff();
        if is_double {
            return match self.nav_enter() {
                Effect::None => load,
                other => other,
            };
        }
        load
    }

    fn drag_split(&mut self, col: u16, row: u16) -> Effect {
        match self.drag {
            SplitDrag::Pane => self.apply_tree_fraction_from_col(col),
            SplitDrag::Diff => self.apply_diff_fraction_from_col(col),
            SplitDrag::GraphScrollbar {
                origin_row,
                origin_scroll,
            } => {
                self.graph_scroll =
                    graph_scroll_from_delta(self.split_layout(), origin_row, origin_scroll, row);
            }
            SplitDrag::GraphHScrollbar {
                origin_col,
                origin_offset,
            } => {
                let next =
                    graph_col_from_delta(self.split_layout(), origin_col, origin_offset, col);
                self.set_graph_col_offset(next);
            }
            SplitDrag::None => {}
        }
        Effect::None
    }

    fn focus_graph_pane(&mut self) {
        self.focus = if self.drill.is_files() {
            FocusPane::Left
        } else {
            FocusPane::Right
        };
    }

    fn toggle_diff_mode(&mut self) -> Effect {
        self.diff_mode = match self.diff_mode {
            DiffMode::SideBySide => DiffMode::Inline,
            DiffMode::Inline => DiffMode::SideBySide,
        };
        self.status = match self.diff_mode {
            DiffMode::SideBySide => "Diff: split".into(),
            DiffMode::Inline => "Diff: inline".into(),
        };
        Effect::None
    }

    pub fn set_graph(&mut self, model: GraphModel, repo: String, head: String) {
        let after = graph_row_signatures(&model, &repo);
        let next_meta = graph_flash_meta(&model, &repo);
        let focused = self.focused_graph_repo();
        let decision = graph_flash_decision(
            focused.as_deref(),
            &repo,
            &self.graph_signatures,
            &after,
            self.graph_flash_meta.as_ref(),
            &next_meta,
        );
        let now = Instant::now();
        self.prune_flash_state(now);
        match decision {
            GraphFlashDecision::Stale => {}
            GraphFlashDecision::Seed => {
                self.graph_signatures = after;
                self.graph_flash_meta = Some(next_meta);
            }
            GraphFlashDecision::Apply { include_adds } => {
                let ids = flashable_row_ids(&self.graph_signatures, &after, include_adds);
                self.stamp_flashes(ids, now);
                self.graph_signatures = after;
                self.graph_flash_meta = Some(next_meta);
            }
        }
        let identity = (repo, head);
        if self.graph_identity.as_ref() != Some(&identity) {
            self.graph_scroll = 0;
            self.graph_cursor = 0;
            self.left_col_offset = 0;
            self.right_col_offset = 0;
            if !matches!(self.drill, DrillView::Files { .. } | DrillView::Diff { .. }) {
                self.drill = DrillView::Graph;
            }
        }
        self.graph_identity = Some(identity);
        self.graph = Some(model);
        let n = self
            .graph
            .as_ref()
            .map(|g| g.visible_rows().len())
            .unwrap_or(0);
        if n == 0 {
            self.graph_cursor = 0;
        } else {
            self.graph_cursor = self.graph_cursor.min(n - 1);
        }
        if self.drill.is_graph() {
            self.diff_content = DiffContent::default();
            self.diff_repo = None;
            self.diff_path = None;
        }
    }

    /// Store workspace-file diff content for the right pane.
    pub fn set_diff(&mut self, repo: String, path: String, content: DiffContent) {
        self.adopt_diff_view(DiffViewId::Workspace {
            repo: repo.clone(),
            path: path.clone(),
        });
        self.diff_repo = Some(repo);
        self.diff_path = Some(path);
        self.diff_content = content;
        let n = self.current_diff_rows().len();
        if n == 0 {
            self.diff_cursor = 0;
        } else {
            self.diff_cursor = self.diff_cursor.min(n - 1);
        }
        self.apply_pending_hunk_anchor();
        self.sync_diff_scroll();
        if self.drill.is_graph() {
            self.graph = None;
        }
    }

    pub fn clear_right(&mut self) {
        self.graph = None;
        self.graph_identity = None;
        self.graph_cursor = 0;
        self.graph_loading_older = false;
        self.commit_files_loading = false;
        self.drill = DrillView::Graph;
        self.diff_content = DiffContent::default();
        self.diff_repo = None;
        self.diff_path = None;
        self.diff_view = None;
        self.reset_diff_viewport();
        self.graph_signatures.clear();
        self.graph_flash_meta = None;
        self.commit_file_signatures.clear();
        self.commit_file_ghosts.clear();
    }

    /// Open the commit-files drill before git returns.
    ///
    /// Paint uses `loading files…` while `commit_files_loading` is true and
    /// the list is empty.
    pub fn begin_commit_files(&mut self, repo: String, source: CommitFileSource) {
        self.commit_file_folds.clear();
        self.commit_files_loading = true;
        self.right_col_offset = 0;
        self.drill = DrillView::Files {
            repo,
            source,
            files: Vec::new(),
            cursor: 0,
        };
        self.focus = FocusPane::Right;
    }

    /// Fill the commit-file list. Enter from the graph focuses the right pane;
    /// a follow/reload that is already at files/diff keeps the current focus.
    pub fn open_commit_files(
        &mut self,
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
    ) {
        let same_source = match &self.drill {
            DrillView::Files {
                repo: r, source: s, ..
            }
            | DrillView::Diff {
                repo: r, source: s, ..
            } => r == &repo && s == &source,
            DrillView::Graph => false,
        };
        let keep_path = if same_source {
            self.focused_commit_file_row().map(|row| row.path.clone())
        } else {
            None
        };
        let before = self.commit_file_signatures.clone();
        let old_rows = if same_source {
            self.commit_file_rows()
        } else {
            Vec::new()
        };
        if !same_source {
            self.commit_file_folds.clear();
            self.right_col_offset = 0;
        }
        self.commit_files_loading = false;
        self.status = format!("files {}", files.len());
        let cursor = DrillView::files_cursor(&files, 0);
        let retain_focus = !self.drill.is_graph();
        self.drill = DrillView::Files {
            repo: repo.clone(),
            source: source.clone(),
            files,
            cursor,
        };
        if !retain_focus {
            self.focus = FocusPane::Right;
        }
        if let Some(path) = keep_path.as_deref() {
            self.restore_commit_file_cursor(Some(path));
        }
        let unfolded = match &self.drill {
            DrillView::Files { files, .. } => self.unfolded_commit_file_rows(files),
            _ => Vec::new(),
        };
        let after = commit_file_signatures(&repo, &source, &unfolded);
        let now = Instant::now();
        self.prune_flash_state(now);
        if !same_source || is_new_row_set(&before, &after) {
            self.commit_file_signatures = after;
            self.commit_file_ghosts.clear();
        } else {
            self.commit_file_ghosts.extend(capture_removal_ghosts(
                &old_rows,
                |row| row.id.as_str(),
                |row| commit_file_identity(&repo, &source, &row.id),
                &before,
                &after,
                now,
            ));
            self.stamp_flashes(flashable_row_ids(&before, &after, true), now);
            self.commit_file_signatures = after;
        }
    }

    pub fn open_commit_diff(
        &mut self,
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
        file_cursor: usize,
        path: String,
        content: DiffContent,
    ) {
        let entering = !self.drill.is_diff();
        self.adopt_diff_view(DiffViewId::Commit {
            repo: repo.clone(),
            source: source.clone(),
            path: path.clone(),
        });
        if entering {
            self.left_col_offset = 0;
        }
        self.status = format!("diff {path}");
        self.drill = DrillView::Diff {
            repo,
            source,
            files,
            file_cursor,
            path,
            content,
        };
        let n = self.current_diff_rows().len();
        if n == 0 {
            self.diff_cursor = 0;
        } else {
            self.diff_cursor = self.diff_cursor.min(n - 1);
        }
        self.apply_pending_hunk_anchor();
        self.sync_diff_scroll();
    }

    fn reset_diff_viewport(&mut self) {
        self.diff_scroll = 0;
        self.diff_cursor = 0;
        self.diff_col_offset = 0;
        self.clear_diff_visual();
    }

    fn adopt_diff_view(&mut self, id: DiffViewId) {
        if self.diff_view.as_ref() != Some(&id) {
            self.reset_diff_viewport();
            self.diff_view = Some(id);
        }
    }

    pub fn focused_file(&self) -> Option<(String, FileChange)> {
        let row = self.focused_row()?;
        Some((row.repo.clone()?, row.file.clone()?))
    }

    pub fn focused_graph_repo(&self) -> Option<String> {
        let row = self.focused_row()?;
        match row.kind {
            NodeKind::Repo | NodeKind::Checkout | NodeKind::Dir => row.repo.clone(),
            NodeKind::File => None,
            NodeKind::Workspace | NodeKind::Group => None,
        }
    }

    fn search_load_effect(&self) -> Effect {
        if self.search_target == SearchPane::Tree {
            Effect::LoadRightPane
        } else {
            Effect::None
        }
    }

    fn current_search_pane(&self) -> SearchPane {
        match self.list_focus_target() {
            ListFocusTarget::Tree => SearchPane::Tree,
            ListFocusTarget::Graph => SearchPane::Graph,
            ListFocusTarget::CommitFiles => SearchPane::CommitFiles,
            ListFocusTarget::None => SearchPane::Diff,
        }
    }

    fn current_diff_content(&self) -> &DiffContent {
        match &self.drill {
            DrillView::Diff { content, .. } => content,
            _ => &self.diff_content,
        }
    }

    /// Path shown in the diff pane header (`repo/path`).
    pub fn diff_header_path(&self) -> String {
        match &self.drill {
            DrillView::Diff { repo, path, .. } => format!("{repo}/{path}"),
            _ => match (self.diff_repo.as_deref(), self.diff_path.as_deref()) {
                (Some(repo), Some(path)) => format!("{repo}/{path}"),
                (_, Some(path)) => path.to_string(),
                _ => String::new(),
            },
        }
    }

    /// Numbered rows for the current layout (inline vs split).
    pub fn current_diff_rows(&self) -> Vec<DiffRow> {
        let mode = effective_diff_mode(self.diff_mode, self.layout.diff_pane_width);
        build_diff_rows(self.current_diff_content(), mode)
    }

    fn set_search_status(&mut self, hit: bool) {
        if self.search_query.trim().is_empty() {
            self.status.clear();
        } else if hit {
            // Armed query is a `/query` chip on the idle bar, not `n next N prev`.
            self.status.clear();
        } else {
            self.status = "no match".into();
        }
    }

    fn apply_search(&mut self, dir: i32) -> Effect {
        match self.search_target {
            SearchPane::Tree => {
                self.apply_tree_search(dir);
                self.search_load_effect()
            }
            SearchPane::Graph => {
                self.apply_graph_search(dir);
                self.follow_graph_files()
            }
            SearchPane::CommitFiles => {
                self.apply_commit_file_search(dir);
                self.maybe_load_focused_commit_diff()
            }
            SearchPane::Diff => {
                self.apply_diff_search(dir);
                Effect::None
            }
        }
    }

    fn apply_tree_search(&mut self, dir: i32) {
        let current = self.focused_row().map(|r| r.id.clone());
        let (folds, focus_id) = focus_tree_search(
            &self.tree,
            &self.folds,
            &self.search_query,
            current.as_deref(),
            dir,
        );
        self.folds = folds;
        self.rebuild_rows();
        if let Some(id) = focus_id {
            if let Some(idx) = self.rows.iter().position(|r| r.id == id) {
                self.cursor = idx;
            }
            self.set_search_status(true);
        } else {
            self.set_search_status(false);
        }
    }

    fn apply_graph_search(&mut self, dir: i32) {
        let Some(model) = self.graph.as_ref() else {
            self.set_search_status(false);
            return;
        };
        let rows = model.visible_rows();
        let Some(idx) = focus_graph_search(&rows, &self.search_query, self.graph_cursor, dir)
        else {
            self.set_search_status(false);
            return;
        };
        self.graph_cursor = idx;
        self.sync_graph_scroll();
        self.set_search_status(true);
    }

    fn apply_commit_file_search(&mut self, dir: i32) {
        let Some(files) = self.commit_drill_files() else {
            self.set_search_status(false);
            return;
        };
        let files = files.to_vec();
        let cursor = self.commit_files_cursor();
        let current_path = flatten_commit_files(
            &files,
            self.commit_tree_mode,
            &self.commit_file_folds,
            self.ascii,
        )
        .get(cursor)
        .map(|row| row.path.clone());
        let current_file_idx = current_path
            .as_deref()
            .and_then(|path| files.iter().position(|file| file.path == path))
            .unwrap_or(cursor);
        let Some(file_idx) =
            focus_commit_file_search(&files, &self.search_query, current_file_idx, dir)
        else {
            self.set_search_status(false);
            return;
        };
        let path = files[file_idx].path.clone();
        for id in ancestor_dir_ids(&path) {
            self.commit_file_folds.remove(&id);
        }
        self.restore_commit_file_cursor(Some(&path));
        self.set_search_status(true);
    }

    fn apply_diff_search(&mut self, dir: i32) {
        let rows = self.current_diff_rows();
        let texts: Vec<String> = rows.iter().map(row_search_text).collect();
        let Some(idx) = focus_diff_search(&texts, &self.search_query, self.search_hit, dir) else {
            self.search_hit = None;
            self.set_search_status(false);
            return;
        };
        self.search_hit = Some(idx);
        self.diff_cursor = idx;
        self.sync_diff_scroll();
        self.set_search_status(true);
    }

    fn file_context_id(repo: &str, path: &str) -> String {
        format!("file:{repo}:{path}")
    }

    fn commit_context_id(repo: &str, path: &str) -> String {
        format!("commit:{repo}:{path}")
    }

    fn displayed_diff_id(&self) -> Option<String> {
        match &self.drill {
            DrillView::Diff { repo, path, .. } => Some(Self::commit_context_id(repo, path)),
            DrillView::Graph => {
                let (repo, file) = self.focused_file()?;
                Some(Self::file_context_id(&repo, &file.path))
            }
            DrillView::Files { .. } => None,
        }
    }

    /// `Some(n)` means `-Un` for this identity. `None` is git's default.
    pub fn diff_context_for(&self, id: &str) -> Option<u32> {
        if self.full_context.contains(id) {
            Some(FULL_DIFF_CONTEXT_LINES)
        } else {
            None
        }
    }

    /// Workspace dirty-file context, independent of pane focus.
    pub fn workspace_diff_context(&self, repo: &str, path: &str) -> Option<u32> {
        self.diff_context_for(&Self::file_context_id(repo, path))
    }

    /// Commit-file (or stash-file) context, independent of pane focus.
    pub fn commit_diff_context(&self, repo: &str, path: &str) -> Option<u32> {
        self.diff_context_for(&Self::commit_context_id(repo, path))
    }

    /// Context for the file currently shown in the right pane.
    /// Survives tree focus so a reload does not drop unlimited `-U`.
    pub fn diff_context_lines(&self) -> Option<u32> {
        let id = self.displayed_diff_id()?;
        self.diff_context_for(&id)
    }

    pub fn full_context_active(&self) -> bool {
        self.diff_context_lines().is_some()
    }

    fn toggle_full_context(&mut self) -> Effect {
        if !self.right_is_diff() {
            return Effect::None;
        }
        let Some(id) = self.displayed_diff_id() else {
            return Effect::None;
        };
        let rows = self.current_diff_rows();
        self.pending_hunk_anchor = Some(anchor_row_text(
            &rows,
            self.diff_cursor,
            self.diff_body_height(),
        ));
        if !self.full_context.remove(&id) {
            self.full_context.insert(id);
        }
        match &self.drill {
            DrillView::Diff {
                repo, source, path, ..
            } => Effect::LoadCommitDiff {
                repo: repo.clone(),
                source: source.clone(),
                path: path.clone(),
            },
            DrillView::Graph => Effect::LoadRightPane,
            DrillView::Files { .. } => Effect::None,
        }
    }

    fn apply_pending_hunk_anchor(&mut self) {
        let Some(needle) = self.pending_hunk_anchor.take() else {
            return;
        };
        let rows = self.current_diff_rows();
        let idx = find_anchor_row(&rows, &needle);
        self.diff_cursor = idx;
        self.sync_diff_scroll();
    }

    fn focused_file_if_shown(&self) -> Option<(String, FileChange)> {
        let row = self.focused_row()?;
        if row.kind != NodeKind::File {
            return None;
        }
        if row.ignored && !self.show_ignored {
            return None;
        }
        Some((row.repo.clone()?, row.file.clone()?))
    }

    fn file_write_effect(&mut self, write: FileWrite) -> Effect {
        let scoped = collect_write_files(&self.snapshot, self.focused_row(), self.show_ignored);
        let selected: Vec<ScopedFile> = scoped
            .into_iter()
            .filter(|file| match write {
                FileWrite::Stage => is_stageable(&file.change),
                FileWrite::Unstage => is_unstageable(&file.change),
            })
            .collect();
        if selected.is_empty() {
            let kind = self.focused_row().map(|row| row.kind);
            self.status = empty_write_status(kind, write);
            return Effect::None;
        }
        let groups = group_write_paths(&selected);
        let total: usize = groups.iter().map(|(_, paths)| paths.len()).sum();
        let verb = match write {
            FileWrite::Stage => "stage",
            FileWrite::Unstage => "unstage",
        };
        self.status = if total == 1 {
            format!("{verb} {}", groups[0].1[0])
        } else {
            format!("{verb} {total} files")
        };
        let effects: Vec<Effect> = groups
            .into_iter()
            .map(|(repo, paths)| match write {
                FileWrite::Stage => Effect::Stage { repo, paths },
                FileWrite::Unstage => Effect::Unstage { repo, paths },
            })
            .collect();
        single_or_batch(effects)
    }

    fn begin_revert(&mut self) -> Effect {
        let scoped = collect_write_files(&self.snapshot, self.focused_row(), self.show_ignored);
        let selected: Vec<ScopedFile> = scoped
            .into_iter()
            .filter(|file| is_revertible(&file.change))
            .collect();
        if selected.is_empty() {
            let staged_only = self.focused_file_if_shown().is_some_and(|(_, change)| {
                change.staged_status.is_some()
                    && change.unstaged_status.is_none()
                    && !change.untracked
            });
            self.status = if staged_only {
                "nothing to discard (staged only)".into()
            } else if matches!(
                self.focused_row().map(|row| row.kind),
                Some(NodeKind::File | NodeKind::Dir | NodeKind::Repo | NodeKind::Checkout)
            ) {
                "nothing to discard".into()
            } else {
                "focus a file, dir, checkout, or repo to revert".into()
            };
            return Effect::None;
        }
        let targets: Vec<RevertTarget> = selected
            .iter()
            .map(|file| RevertTarget {
                repo: file.repo.clone(),
                path: file.change.path.clone(),
                untracked: file.change.untracked,
                old_path: file.change.old_path.clone(),
            })
            .collect();
        let label = self
            .focused_row()
            .map(|row| {
                if row.chrome.path.is_empty() {
                    row.label.clone()
                } else {
                    row.chrome.path.clone()
                }
            })
            .unwrap_or_else(|| targets.first().map(|t| t.path.clone()).unwrap_or_default());
        self.confirm = Some(PendingConfirm::Revert { targets, label });
        Effect::None
    }

    fn confirm_yes(&mut self, clean: bool) -> Effect {
        match self.confirm.take() {
            Some(PendingConfirm::Revert { targets, .. }) => {
                if targets.is_empty() {
                    return Effect::None;
                }
                let flags: Vec<bool> = targets.iter().map(|t| t.untracked).collect();
                let delete_untracked = should_delete_untracked(&flags, clean);
                let groups = group_revert_targets(&targets, delete_untracked);
                let tracked_n: usize = groups.iter().map(|(_, tracked, _)| tracked.len()).sum();
                let untracked_n: usize =
                    groups.iter().map(|(_, _, untracked)| untracked.len()).sum();
                self.status = if tracked_n + untracked_n == 1 {
                    if untracked_n == 1 {
                        format!("delete {}", groups[0].2[0])
                    } else {
                        format!("revert {}", groups[0].1[0])
                    }
                } else {
                    format!("revert {tracked_n} tracked, {untracked_n} untracked")
                };
                let effects: Vec<Effect> = groups
                    .into_iter()
                    .map(|(repo, tracked, untracked)| Effect::Revert {
                        repo,
                        tracked,
                        untracked,
                    })
                    .collect();
                single_or_batch(effects)
            }
            Some(PendingConfirm::StashDrop { repo, stash_ref }) => {
                self.status = format!("drop {stash_ref}");
                Effect::StashDrop { repo, stash_ref }
            }
            Some(PendingConfirm::CheckoutOutOfSync {
                repo,
                branch,
                remote_ref,
            }) => {
                self.status = format!("checkout {branch} then fast-forward {remote_ref}");
                Effect::CheckoutBranch {
                    repo,
                    selected_name: branch,
                    fast_forward_ref: Some(remote_ref),
                }
            }
            Some(PendingConfirm::RemoveWorktree {
                primary,
                path,
                force,
                ..
            }) => {
                self.status = format!("remove worktree {path}");
                Effect::RemoveWorktree {
                    primary,
                    path,
                    force,
                }
            }
            Some(PendingConfirm::MergeIntoHead {
                repo, rev, label, ..
            }) => {
                self.status = format!("merge {label}");
                Effect::MergeIntoHead { repo, rev, label }
            }
            None => Effect::None,
        }
    }

    fn toggle_reviewed(&mut self) -> Effect {
        if self.nav_depth() >= 1 {
            return Effect::None;
        }
        let Some(row) = self.focused_row().cloned() else {
            return Effect::None;
        };
        if row.kind != NodeKind::File {
            return Effect::None;
        }
        let Some(repo) = row.repo.as_deref() else {
            return Effect::None;
        };
        let Some(file) = row.file.as_ref() else {
            return Effect::None;
        };
        let identity = viewed_identity(repo, &file.path);
        let fingerprint = fingerprint_file_change(&self.cwd, repo, file);
        self.viewed_store = toggle_viewed(&self.viewed_store, &identity, &fingerprint);
        save_viewed_store(&self.viewed_store, &self.viewed_path);
        if is_viewed(&self.viewed_store, &identity, &fingerprint) {
            self.reviewed.insert(row.id);
        } else {
            self.reviewed.remove(&row.id);
        }
        Effect::None
    }

    fn reconcile_viewed_store(&mut self) {
        let current = collect_current_fingerprints(&self.snapshot, &self.cwd);
        let next = reconcile_viewed(&self.viewed_store, &current);
        if next != self.viewed_store {
            self.viewed_store = next;
            save_viewed_store(&self.viewed_store, &self.viewed_path);
        }
        self.reviewed = viewed_row_ids(&self.snapshot, &self.viewed_store, &self.cwd);
    }

    fn reconcile_comment_store(&mut self) {
        let live = collect_live_set(&self.snapshot, &self.comment_store);
        let next = gc_comments(&self.comment_store, &live);
        if next != self.comment_store {
            self.comment_store = next;
            save_comment_store(&self.comment_store, &self.comment_path);
        }
    }

    fn current_comment_target(&self) -> Option<CommentKey> {
        match self.list_focus_target() {
            ListFocusTarget::None => {
                let source = match &self.drill {
                    DrillView::Diff { source, .. } => Some(source),
                    _ => None,
                };
                let (repo, path) = match &self.drill {
                    DrillView::Diff { repo, path, .. } => {
                        (Some(repo.as_str()), Some(path.as_str()))
                    }
                    _ => (self.diff_repo.as_deref(), self.diff_path.as_deref()),
                };
                let rows = self.current_diff_rows();
                let (line, end_line) = if let Some(anchor) = self.diff_visual_anchor {
                    viewport_line_range(&rows, anchor, self.diff_cursor)?
                } else {
                    let line = viewport_line_number(&rows, self.diff_cursor as u16)?;
                    (line, line)
                };
                resolve_comment_target(
                    &self.snapshot,
                    None,
                    None,
                    None,
                    true,
                    repo,
                    path,
                    source,
                    Some(line),
                    Some(end_line),
                )
            }
            ListFocusTarget::Graph => {
                let repo = self.focused_graph_repo();
                let row = self.focused_graph_row();
                resolve_comment_target(
                    &self.snapshot,
                    None,
                    row.as_ref(),
                    repo.as_deref(),
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            }
            ListFocusTarget::Tree => resolve_comment_target(
                &self.snapshot,
                self.focused_row(),
                None,
                None,
                false,
                None,
                None,
                None,
                None,
                None,
            ),
            ListFocusTarget::CommitFiles => None,
        }
    }

    pub(crate) fn clear_diff_visual(&mut self) {
        self.diff_visual_anchor = None;
    }

    /// True when visual-line highlight includes painted diff row `idx`.
    pub fn diff_visual_contains(&self, idx: usize) -> bool {
        let Some(anchor) = self.diff_visual_anchor else {
            return false;
        };
        let lo = anchor.min(self.diff_cursor);
        let hi = anchor.max(self.diff_cursor);
        idx >= lo && idx <= hi
    }

    fn begin_diff_visual(&mut self) -> Effect {
        if self.list_focus_target() != ListFocusTarget::None {
            return Effect::None;
        }
        if self.current_diff_rows().is_empty() {
            self.status = "no highlight target".into();
            return Effect::None;
        }
        self.drag = SplitDrag::None;
        self.help_open = false;
        self.comment_export = None;
        self.diff_visual_anchor = Some(self.diff_cursor);
        self.status.clear();
        Effect::None
    }

    fn cancel_diff_visual(&mut self) -> Effect {
        if self.diff_visual_anchor.is_some() {
            self.clear_diff_visual();
            self.status.clear();
        }
        Effect::None
    }

    fn begin_comment(&mut self) -> Effect {
        self.drag = SplitDrag::None;
        self.help_open = false;
        self.comment_export = None;
        let Some(key) = self.current_comment_target() else {
            self.status = "no comment target".into();
            return Effect::None;
        };
        self.clear_diff_visual();
        let body = self.comment_store.get(&key).cloned().unwrap_or_default();
        let label = comment_key_label(&key);
        self.comment = Some(CommentPrompt { key, body, label });
        self.status.clear();
        Effect::None
    }

    fn submit_comment(&mut self) -> Effect {
        let Some(prompt) = self.comment.take() else {
            return Effect::None;
        };
        let empty = prompt.body.trim().is_empty();
        self.comment_store = put_comment(&self.comment_store, prompt.key, &prompt.body);
        save_comment_store(&self.comment_store, &self.comment_path);
        self.status = if empty {
            "comment deleted".into()
        } else {
            "comment saved".into()
        };
        Effect::None
    }

    fn export_comments(&mut self) -> Effect {
        self.drag = SplitDrag::None;
        self.help_open = false;
        self.comment = None;
        self.reconcile_comment_store();
        let markdown = export_markdown(&self.scoped_comment_store());
        self.comment_export = Some(CommentExport {
            markdown: markdown.clone(),
        });
        self.status = "copied".into();
        Effect::CopyClipboard { text: markdown }
    }

    fn scoped_comment_store(&self) -> CommentStore {
        let graph_repo = self.focused_graph_repo();
        let graph_row = self.focused_graph_row();
        let commit_file = self.focused_commit_file_row();
        let list = match self.list_focus_target() {
            ListFocusTarget::Tree => CommentExportList::Tree {
                row: self.focused_row(),
            },
            ListFocusTarget::Graph => CommentExportList::Graph {
                repo: graph_repo.as_deref(),
                row: graph_row.as_ref(),
            },
            ListFocusTarget::CommitFiles => match (&self.drill, commit_file.as_ref()) {
                (
                    DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. },
                    Some(row),
                ) => CommentExportList::CommitFiles {
                    repo: repo.as_str(),
                    source,
                    path: row.path.as_str(),
                    is_dir: row.is_dir(),
                },
                _ => CommentExportList::Tree {
                    row: self.focused_row(),
                },
            },
            ListFocusTarget::None => {
                let (repo, path, source) = match &self.drill {
                    DrillView::Diff {
                        repo, path, source, ..
                    } => (Some(repo.as_str()), Some(path.as_str()), Some(source)),
                    _ => (self.diff_repo.as_deref(), self.diff_path.as_deref(), None),
                };
                match (repo, path) {
                    (Some(repo), Some(path)) => CommentExportList::Diff { repo, path, source },
                    _ => CommentExportList::Tree {
                        row: self.focused_row(),
                    },
                }
            }
        };
        comments_in_focus_scope(&self.comment_store, &self.snapshot, &self.tree, list)
    }

    fn refuse_remove_worktree(&mut self) -> Effect {
        self.status = "Focus a linked worktree to remove".into();
        Effect::None
    }

    fn begin_remove_worktree(&mut self) -> Effect {
        let Some(row) = self.focused_row() else {
            return self.refuse_remove_worktree();
        };
        if row_is_hidden_ignored(row, self.show_ignored) {
            return Effect::None;
        }
        if !matches!(row.kind, NodeKind::Checkout | NodeKind::Repo) {
            return self.refuse_remove_worktree();
        }
        let Some(repo_path) = row.repo.as_deref() else {
            return self.refuse_remove_worktree();
        };
        let Some(snap) = self.snapshot.repos.iter().find(|r| r.repo == repo_path) else {
            return self.refuse_remove_worktree();
        };
        if snap.checkout_kind != CheckoutKind::Linked {
            return self.refuse_remove_worktree();
        }
        let Some(primary) = snap.primary_repo.clone() else {
            return self.refuse_remove_worktree();
        };
        let force = snap.has_unstaged || snap.has_staged || snap.has_untracked;
        let path = snap.repo.clone();
        let branch = snap.branch.clone();
        let merged_into_default = snap.merged_into_default;
        self.confirm = Some(PendingConfirm::RemoveWorktree {
            primary,
            path,
            force,
            branch,
            merged_into_default,
        });
        Effect::None
    }

    fn fetch_tick_effect(&mut self) -> Effect {
        let targets = background_fetch_targets(&self.snapshot, self.show_ignored);
        if targets.is_empty() {
            return Effect::None;
        }
        self.status = format_running_op(RunningOp::Fetch, 0, targets.len());
        Effect::Fetch { repos: targets }
    }

    fn focused_checkout_if_shown(&self) -> Option<String> {
        let row = self.focused_row()?;
        if row_is_hidden_ignored(row, self.show_ignored) {
            return None;
        }
        checkout_path(row)
    }

    fn push_effect(&mut self) -> Effect {
        let targets = push_targets(&self.snapshot, self.focused_row(), self.show_ignored);
        if targets.is_empty() {
            self.status = "nothing to push".into();
            return Effect::None;
        }
        self.status = format_running_op(RunningOp::Push, 0, targets.len());
        Effect::Push { repos: targets }
    }

    fn begin_stash_menu(&mut self) -> Effect {
        if self.nav_depth() >= 2 {
            return Effect::None;
        }
        let Some(repo) = self.focused_checkout_if_shown() else {
            self.status = "focus a visible repo to stash".into();
            return Effect::None;
        };
        self.help_open = false;
        Effect::PrepareStashMenu { repo }
    }

    /// Fill the stash overlay after git lists the latest stash.
    pub fn open_stash_menu(&mut self, repo: String, latest_stash_ref: Option<String>) {
        let focused_stash = self.focused_graph_stash_ref();
        let row = self.focused_row();
        let (dirty, dirty_paths) = if focused_stash.is_some() {
            let dirty = row
                .and_then(|r| r.repo.as_deref())
                .and_then(|repo| self.snapshot.repos.iter().find(|r| r.repo == repo))
                .map(|snap| snap.has_unstaged || snap.has_staged || snap.has_untracked)
                .unwrap_or(false);
            (dirty, None)
        } else {
            match row {
                Some(row) => stash_dirty_for_row(&self.snapshot, row),
                None => (false, None),
            }
        };
        let latest_for_ops = if self.graph_pane_focused() {
            latest_stash_ref
        } else {
            None
        };
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty,
            dirty_paths,
            focused_stash_ref: focused_stash,
            latest_stash_ref: latest_for_ops,
        });
        if ops.is_empty() {
            self.stash_menu = None;
            self.stash_repo = None;
            self.status = "nothing to stash".into();
            return;
        }
        self.status = stash_menu_status(&ops);
        self.stash_repo = Some(repo);
        self.stash_menu = Some(ops);
    }

    fn stash_menu_key(&mut self, input: Option<char>, enter: bool, escape: bool) -> Effect {
        let Some(ops) = self.stash_menu.clone() else {
            return Effect::None;
        };
        match resolve_stash_menu_key(input, enter, escape, &ops) {
            StashMenuKeyResult::Cancel => {
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = "stash cancelled".into();
                Effect::None
            }
            StashMenuKeyResult::Ignore => Effect::None,
            StashMenuKeyResult::Run(op) => self.run_stash_op(op),
        }
    }

    fn run_stash_op(&mut self, op: StashOp) -> Effect {
        let Some(repo) = self.stash_repo.clone() else {
            return Effect::None;
        };
        match op.id {
            StashOpId::Create => {
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = "stash".into();
                Effect::StashCreate {
                    repo,
                    paths: op.paths.unwrap_or_default(),
                }
            }
            StashOpId::Apply => {
                let stash_ref = op.stash_ref.unwrap_or_else(|| "stash@{0}".into());
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = format!("apply {stash_ref}");
                Effect::StashApply { repo, stash_ref }
            }
            StashOpId::Pop => {
                let stash_ref = op.stash_ref.unwrap_or_else(|| "stash@{0}".into());
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = format!("pop {stash_ref}");
                Effect::StashPop { repo, stash_ref }
            }
            StashOpId::Drop => {
                let stash_ref = op.stash_ref.unwrap_or_else(|| "stash@{0}".into());
                self.stash_menu = None;
                self.confirm = Some(PendingConfirm::StashDrop {
                    repo,
                    stash_ref: stash_ref.clone(),
                });
                Effect::None
            }
        }
    }

    fn begin_branch_picker(&mut self) -> Effect {
        let Some(row) = self.focused_row() else {
            self.status = "focus a repo to pick a branch".into();
            return Effect::None;
        };
        if row_is_hidden_ignored(row, self.show_ignored) {
            self.status = "focus a visible repo to pick a branch".into();
            return Effect::None;
        }
        if !can_open_branch_picker(&self.snapshot, row) {
            self.status = "focus a checkout to pick a branch".into();
            return Effect::None;
        }
        let Some(repo) = checkout_path(row) else {
            self.status = "focus a checkout to pick a branch".into();
            return Effect::None;
        };
        self.help_open = false;
        Effect::PrepareBranchPicker { repo }
    }

    /// Fill the branch picker after git lists local branches.
    pub fn open_branch_picker(&mut self, repo: String, branches: Vec<crate::git::LocalBranch>) {
        if branches.is_empty() {
            self.status = "no local branches".into();
            self.branch_picker = None;
            return;
        }
        let default = self
            .snapshot
            .repos
            .iter()
            .find(|r| r.repo == repo)
            .and_then(|r| r.default_branch_override.clone());
        let sorted = super::branches::sort_branches_for_picker(branches, default.as_deref());
        self.branch_picker = Some(BranchPickerState::new(repo, sorted));
        self.status.clear();
    }

    /// Local branch names whose ancestors the current graph should show.
    pub(crate) fn graph_focus_revs(&self) -> Vec<String> {
        let owned = self.focused_graph_repo();
        let repo = self
            .graph_identity
            .as_ref()
            .map(|(repo, _)| repo.as_str())
            .or(owned.as_deref());
        let Some(repo) = repo else {
            return Vec::new();
        };
        match &self.graph_branch_focus {
            Some((focus_repo, names)) if focus_repo == repo && !names.is_empty() => names.clone(),
            _ => Vec::new(),
        }
    }

    fn graph_focus_repo(&self) -> Option<String> {
        self.graph_identity
            .as_ref()
            .map(|(repo, _)| repo.clone())
            .or_else(|| self.focused_graph_repo())
    }

    fn begin_graph_focus_picker(&mut self) -> Effect {
        if !self.graph_pane_focused() {
            return Effect::None;
        }
        let Some(repo) = self.graph_focus_repo() else {
            return Effect::None;
        };
        self.help_open = false;
        Effect::PrepareGraphFocusPicker { repo }
    }

    /// Fill the graph focus overlay after git lists local branches.
    pub fn open_graph_focus_picker(
        &mut self,
        repo: String,
        branches: Vec<crate::git::LocalBranch>,
    ) {
        if branches.is_empty() {
            self.status = "no local branches".into();
            self.graph_focus_picker = None;
            return;
        }
        let default = self
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .and_then(|row| row.default_branch_override.clone());
        let sorted = super::branches::sort_branches_for_picker(branches, default.as_deref());
        let preselected = match &self.graph_branch_focus {
            Some((focus_repo, names)) if focus_repo == &repo => names.as_slice(),
            _ => &[],
        };
        self.graph_focus_picker = Some(GraphFocusPickerState::new(repo, sorted, preselected));
        self.status.clear();
    }

    fn submit_graph_focus_picker(&mut self) -> Effect {
        let Some(picker) = self.graph_focus_picker.as_ref() else {
            return Effect::None;
        };
        let repo = picker.repo.clone();
        let Some(names) = picker.apply_names() else {
            self.status = "no matching branches".into();
            return Effect::None;
        };
        self.graph_focus_picker = None;
        if names.is_empty() {
            return self.clear_graph_branch_focus();
        }
        self.apply_graph_branch_focus(repo, names)
    }

    fn clear_graph_branch_focus(&mut self) -> Effect {
        self.graph_focus_picker = None;
        if self.graph_branch_focus.is_none() {
            if self.graph_pane_focused() {
                self.status = "full graph".into();
            }
            return Effect::None;
        }
        self.apply_graph_branch_focus(self.graph_focus_repo().unwrap_or_default(), Vec::new())
    }

    fn apply_graph_branch_focus(&mut self, repo: String, names: Vec<String>) -> Effect {
        self.graph_identity = None;
        self.graph_cursor = 0;
        self.graph_scroll = 0;
        if names.is_empty() {
            self.graph_branch_focus = None;
            self.status = "full graph".into();
        } else {
            let label = names.join(", ");
            self.graph_branch_focus = Some((repo, names));
            self.status = format!("graph focus: {label}");
        }
        Effect::LoadRightPane
    }

    fn submit_branch_picker(&mut self) -> Effect {
        let Some(picker) = self.branch_picker.as_ref() else {
            return Effect::None;
        };
        let repo = picker.repo.clone();
        let filter = picker.filter.clone();
        if let Some(selected) = picker.selected().cloned() {
            if selected.current {
                self.branch_picker = None;
                self.status = format!("Already on {}", selected.name);
                return Effect::None;
            }
            return self.checkout_or_confirm(repo, selected.name);
        }
        if is_valid_branch_name(&filter) {
            self.branch_picker = None;
            self.status = format!("create {filter}");
            return Effect::CreateBranch {
                repo,
                name: filter.trim().to_string(),
            };
        }
        self.status = "no matching branches".into();
        Effect::None
    }

    /// Emit a checkout effect. Origin out-of-sync confirm is decided later
    /// (`plan_graph_checkout`) from the selected name, not from local vs origin
    /// of every checkout.
    pub fn checkout_or_confirm(&mut self, repo: String, selected_name: String) -> Effect {
        self.status = format!("checkout {selected_name}");
        Effect::CheckoutBranch {
            repo,
            selected_name,
            fast_forward_ref: None,
        }
    }

    /// Confirm checkout when a local exists and is out of sync with the selected `origin/*`.
    pub fn confirm_checkout_if_out_of_sync(
        &mut self,
        repo: String,
        branch: String,
        remote_ref: Option<String>,
    ) -> Effect {
        if let Some(remote_ref) = remote_ref {
            self.confirm = Some(PendingConfirm::CheckoutOutOfSync {
                repo,
                branch: branch.clone(),
                remote_ref: remote_ref.clone(),
            });
            Effect::None
        } else {
            self.checkout_or_confirm(repo, branch)
        }
    }

    fn begin_create_branch(&mut self) -> Effect {
        let Some(picker) = self.branch_picker.as_ref() else {
            self.status = "open the branch picker first".into();
            return Effect::None;
        };
        let repo = picker.repo.clone();
        let seed = picker.filter.clone();
        self.create_branch = Some(CreateBranchState {
            repo,
            name: seed,
            commit_id: None,
        });
        self.status.clear();
        Effect::None
    }

    fn submit_create_branch(&mut self) -> Effect {
        let Some(create) = self.create_branch.take() else {
            return Effect::None;
        };
        if !is_valid_branch_name(&create.name) {
            self.status = "invalid branch name".into();
            self.create_branch = Some(create);
            return Effect::None;
        }
        self.branch_picker = None;
        self.status = format!("create {}", create.name.trim());
        if let Some(commit_id) = create.commit_id {
            Effect::CreateBranchAt {
                repo: create.repo,
                name: create.name.trim().to_string(),
                commit_id,
            }
        } else {
            Effect::CreateBranch {
                repo: create.repo,
                name: create.name.trim().to_string(),
            }
        }
    }

    fn begin_graph_checkout(&mut self) -> Effect {
        if !self.graph_commit_focused() {
            return Effect::None;
        }
        if self.hidden_ignored_focus() {
            self.status = "focus a visible repo to checkout".into();
            return Effect::None;
        }
        let Some(repo) = self.focused_graph_repo() else {
            return Effect::None;
        };
        let Some(GraphRow::Commit { commit, .. }) = self.focused_graph_row() else {
            return Effect::None;
        };
        let names = checkoutable_branch_names(&commit.refs);
        if names.is_empty() {
            return Effect::None;
        }
        if self.graph_repo_is_dirty(&repo) {
            self.status = DIRTY_WORKTREE_STATUS.into();
            return Effect::None;
        }
        self.help_open = false;
        if names.len() == 1 {
            return self.checkout_or_confirm(repo, names[0].clone());
        }
        self.branch_picker = Some(BranchPickerState::from_names(
            repo,
            names,
            Some(commit.id.clone()),
        ));
        self.status.clear();
        Effect::None
    }

    fn begin_graph_create_branch(&mut self) -> Effect {
        if !self.graph_commit_focused() {
            return Effect::None;
        }
        if self.hidden_ignored_focus() {
            self.status = "focus a visible repo to create a branch".into();
            return Effect::None;
        }
        let Some(repo) = self.focused_graph_repo() else {
            return Effect::None;
        };
        let Some(GraphRow::Commit { commit, .. }) = self.focused_graph_row() else {
            return Effect::None;
        };
        let commit_id = commit.id.clone();
        self.help_open = false;
        self.create_branch = Some(CreateBranchState {
            repo,
            name: String::new(),
            commit_id: Some(commit_id),
        });
        self.status.clear();
        Effect::None
    }

    fn begin_graph_merge(&mut self) -> Effect {
        if !self.graph_commit_focused() {
            return Effect::None;
        }
        if self.hidden_ignored_focus() {
            self.status = "focus a visible repo to merge".into();
            return Effect::None;
        }
        let Some(repo) = self.focused_graph_repo() else {
            return Effect::None;
        };
        let Some(GraphRow::Commit { commit, .. }) = self.focused_graph_row() else {
            return Effect::None;
        };
        if self.graph_repo_is_dirty(&repo) {
            self.status = DIRTY_WORKTREE_STATUS.into();
            return Effect::None;
        }
        let (rev, label) = merge_rev_for_commit(&commit.id, &commit.refs);
        let into = self.graph_head_label(&repo);
        self.help_open = false;
        self.confirm = Some(PendingConfirm::MergeIntoHead {
            repo,
            rev,
            label,
            into,
        });
        Effect::None
    }

    fn graph_head_label(&self, repo: &str) -> String {
        self.snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .map(|row| row.branch.clone())
            .filter(|branch| !branch.is_empty())
            .unwrap_or_else(|| "HEAD".into())
    }

    fn graph_repo_is_dirty(&self, repo: &str) -> bool {
        self.snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .is_some_and(|row| row.has_unstaged || row.has_staged)
    }

    fn hidden_ignored_focus(&self) -> bool {
        self.focused_row()
            .is_some_and(|row| row_is_hidden_ignored(row, self.show_ignored))
    }

    pub fn focused_graph_row(&self) -> Option<GraphRow> {
        let rows = self.graph.as_ref()?.visible_rows();
        rows.get(self.graph_cursor).cloned()
    }

    pub fn focused_graph_stash_ref(&self) -> Option<String> {
        if !self.graph_pane_focused() {
            return None;
        }
        stash_ref_from_graph_row(&self.focused_graph_row()?)
    }

    fn move_graph_cursor(&mut self, delta: i32) {
        let n = self
            .graph
            .as_ref()
            .map(|g| g.visible_rows().len())
            .unwrap_or(0);
        if n == 0 {
            self.graph_cursor = 0;
            return;
        }
        let next = self.graph_cursor as i32 + delta;
        self.graph_cursor = next.clamp(0, n as i32 - 1) as usize;
        self.sync_graph_scroll();
    }

    /// Page in painted-list space, then
    /// snap onto a selectable row. Click stays on `visible_rows`.
    fn page_graph(&mut self, pages: i32) -> Effect {
        let Some(model) = self.graph.as_ref() else {
            return Effect::None;
        };
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        if painted.is_empty() {
            return Effect::None;
        }
        let list_h = self.graph_chrome().list_height.max(1) as usize;
        let page = list_h.saturating_sub(1).max(1) as i32;
        let current = painted
            .iter()
            .position(|line| line.row_index == Some(self.graph_cursor) && line.selectable)
            .unwrap_or(0);
        let last = painted.len() as i32 - 1;
        let target = (current as i32 + pages * page).clamp(0, last) as usize;
        let snapped = nearest_selectable_painted_index(&painted, target);
        if let Some(idx) = painted[snapped].row_index {
            self.graph_cursor = idx;
        }
        self.sync_graph_scroll();
        self.follow_graph_files()
    }

    fn apply_terminal_size(&mut self, cols: u16) {
        self.layout.term_cols = cols.max(1);
        self.tree_fraction = clamp_tree_fraction(self.layout.term_cols, self.tree_fraction);
    }

    pub(crate) fn sync_graph_scroll(&mut self) {
        let Some(model) = self.graph.as_ref() else {
            return;
        };
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        let Some(idx) = painted
            .iter()
            .position(|line| line.row_index == Some(self.graph_cursor) && line.selectable)
        else {
            return;
        };
        let list_h = self.graph_chrome().list_height.max(1) as usize;
        let (start, _) = visible_window(painted.len(), idx, list_h);
        self.graph_scroll = start as u16;
    }

    fn sync_diff_scroll(&mut self) {
        let n = self.current_diff_rows().len();
        if n == 0 {
            self.diff_cursor = 0;
            self.diff_scroll = 0;
            self.clear_diff_visual();
            return;
        }
        self.diff_cursor = self.diff_cursor.min(n - 1);
        if let Some(anchor) = self.diff_visual_anchor {
            self.diff_visual_anchor = Some(anchor.min(n - 1));
        }
        let (start, _) = visible_window(n, self.diff_cursor, self.diff_body_height());
        self.diff_scroll = start as u16;
    }

    fn move_diff_cursor(&mut self, delta: i32) {
        let n = self.current_diff_rows().len();
        if n == 0 {
            self.diff_cursor = 0;
            self.diff_scroll = 0;
            self.clear_diff_visual();
            return;
        }
        self.diff_cursor = (self.diff_cursor as i32 + delta).clamp(0, n as i32 - 1) as usize;
        self.sync_diff_scroll();
    }

    fn page_step(&self) -> i32 {
        self.layout.tree_height.max(1).saturating_sub(1).max(1) as i32
    }

    fn move_file_cursor(&mut self, delta: i32) -> Effect {
        let n = self.commit_file_rows().len();
        if n == 0 {
            self.set_commit_file_cursor(0);
            return Effect::None;
        }
        let next = (self.commit_files_cursor() as i32 + delta).clamp(0, n as i32 - 1) as usize;
        self.set_commit_file_cursor(next);
        self.maybe_load_focused_commit_diff()
    }

    fn move_focused(&mut self, delta: i32) -> Effect {
        match self.list_focus_target() {
            ListFocusTarget::CommitFiles => self.move_file_cursor(delta),
            ListFocusTarget::Graph => {
                self.move_graph_cursor(delta);
                self.follow_graph_files()
            }
            ListFocusTarget::None => {
                self.move_diff_cursor(delta);
                Effect::None
            }
            ListFocusTarget::Tree => {
                self.move_cursor(delta);
                self.drill = DrillView::Graph;
                Effect::LoadRightPane
            }
        }
    }

    fn move_focused_edge(&mut self, end: bool) -> Effect {
        match self.list_focus_target() {
            ListFocusTarget::CommitFiles => {
                let n = self.commit_file_rows().len();
                self.set_commit_file_cursor(if end { n.saturating_sub(1) } else { 0 });
                self.maybe_load_focused_commit_diff()
            }
            ListFocusTarget::Graph => {
                let n = self
                    .graph
                    .as_ref()
                    .map(|g| g.visible_rows().len())
                    .unwrap_or(0);
                self.graph_cursor = if end { n.saturating_sub(1) } else { 0 };
                self.sync_graph_scroll();
                self.follow_graph_files()
            }
            ListFocusTarget::None => {
                let n = self.current_diff_rows().len();
                self.diff_cursor = if end { n.saturating_sub(1) } else { 0 };
                self.sync_diff_scroll();
                Effect::None
            }
            ListFocusTarget::Tree => {
                if end {
                    if !self.rows.is_empty() {
                        self.cursor = self.rows.len() - 1;
                    }
                } else {
                    self.cursor = 0;
                }
                self.drill = DrillView::Graph;
                Effect::LoadRightPane
            }
        }
    }

    fn nav_enter(&mut self) -> Effect {
        if self.hidden_ignored_focus() {
            self.status = "hidden ignored stay out of drill".into();
            return Effect::None;
        }
        if self.focus == FocusPane::Left {
            self.focus = FocusPane::Right;
            return Effect::None;
        }
        match &self.drill {
            DrillView::Graph => {
                let Some(repo) = self.focused_graph_repo() else {
                    self.status = "focus a repo commit to drill".into();
                    return Effect::None;
                };
                let Some(row) = self.focused_graph_row() else {
                    self.status = "focus a graph commit to drill".into();
                    return Effect::None;
                };
                let Some(source) = source_from_graph_row(&row) else {
                    self.status = "focus a graph commit to drill".into();
                    return Effect::None;
                };
                Effect::LoadCommitFiles { repo, source }
            }
            DrillView::Files {
                repo,
                source,
                cursor,
                ..
            } => {
                let rows = self.commit_file_rows();
                let Some(row) = rows.get(*cursor) else {
                    self.status = "no files in this commit".into();
                    return Effect::None;
                };
                if row.is_dir() {
                    self.fold_commit_file(FoldOp::Toggle);
                    return Effect::None;
                }
                Effect::LoadCommitDiff {
                    repo: repo.clone(),
                    source: source.clone(),
                    path: row.path.clone(),
                }
            }
            DrillView::Diff { .. } => Effect::None,
        }
    }

    fn nav_esc(&mut self) -> Effect {
        if self.focus == FocusPane::Right {
            self.focus = FocusPane::Left;
            return Effect::None;
        }
        match &self.drill {
            DrillView::Diff {
                repo,
                source,
                files,
                file_cursor,
                ..
            } => {
                let repo = repo.clone();
                let source = source.clone();
                let files = files.clone();
                let cursor = *file_cursor;
                self.drill = DrillView::Files {
                    repo,
                    source,
                    files,
                    cursor,
                };
                self.status = "files".into();
                Effect::None
            }
            DrillView::Files { .. } => {
                self.drill = DrillView::Graph;
                self.status = "graph".into();
                Effect::LoadRightPane
            }
            DrillView::Graph => Effect::None,
        }
    }

    fn graph_stash_op(&mut self, id: StashOpId) -> Effect {
        if self.hidden_ignored_focus() {
            return Effect::None;
        }
        let Some(stash_ref) = self.focused_graph_stash_ref() else {
            self.status = "focus a graph stash row".into();
            return Effect::None;
        };
        let Some(repo) = self.focused_graph_repo() else {
            self.status = "focus a visible repo to stash".into();
            return Effect::None;
        };
        match id {
            StashOpId::Apply => {
                self.status = format!("apply {stash_ref}");
                Effect::StashApply { repo, stash_ref }
            }
            StashOpId::Pop => {
                self.status = format!("pop {stash_ref}");
                Effect::StashPop { repo, stash_ref }
            }
            StashOpId::Drop => {
                self.confirm = Some(PendingConfirm::StashDrop {
                    repo,
                    stash_ref: stash_ref.clone(),
                });
                Effect::None
            }
            StashOpId::Create => Effect::None,
        }
    }
}

enum FileWrite {
    Stage,
    Unstage,
}

fn empty_write_status(kind: Option<NodeKind>, write: FileWrite) -> String {
    let verb = match write {
        FileWrite::Stage => "stage",
        FileWrite::Unstage => "unstage",
    };
    match kind {
        Some(NodeKind::File | NodeKind::Dir | NodeKind::Repo | NodeKind::Checkout) => {
            format!("nothing to {verb}")
        }
        _ => format!("focus a file, dir, checkout, or repo to {verb}"),
    }
}

fn is_stageable(change: &FileChange) -> bool {
    change.unstaged_status.is_some() || change.untracked
}

fn is_unstageable(change: &FileChange) -> bool {
    change.staged_status.is_some()
}

fn is_revertible(change: &FileChange) -> bool {
    change.unstaged_status.is_some() || change.untracked
}

fn op_paths(change: &FileChange) -> Vec<String> {
    match &change.old_path {
        Some(old) if old != &change.path => vec![old.clone(), change.path.clone()],
        _ => vec![change.path.clone()],
    }
}

fn group_write_paths(files: &[ScopedFile]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for file in files {
        let paths = match groups.iter().position(|(repo, _)| repo == &file.repo) {
            Some(idx) => &mut groups[idx].1,
            None => {
                groups.push((file.repo.clone(), Vec::new()));
                &mut groups.last_mut().unwrap().1
            }
        };
        for path in op_paths(&file.change) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    groups
}

fn group_revert_targets(
    targets: &[RevertTarget],
    delete_untracked: bool,
) -> Vec<(String, Vec<String>, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for target in targets {
        let (tracked, untracked) = match groups.iter().position(|(repo, _, _)| repo == &target.repo)
        {
            Some(idx) => {
                let group = &mut groups[idx];
                (&mut group.1, &mut group.2)
            }
            None => {
                groups.push((target.repo.clone(), Vec::new(), Vec::new()));
                let group = groups.last_mut().unwrap();
                (&mut group.1, &mut group.2)
            }
        };
        let paths = match &target.old_path {
            Some(old) if old != &target.path => vec![old.clone(), target.path.clone()],
            _ => vec![target.path.clone()],
        };
        if target.untracked {
            if delete_untracked {
                untracked.extend(paths);
            }
        } else {
            tracked.extend(paths);
        }
    }
    groups.retain(|(_, tracked, untracked)| !tracked.is_empty() || !untracked.is_empty());
    groups
}

/// Snap a painted-list index onto a selectable line (prefer forward, then back).
/// Nearest selectable graph index at or after `start`.
fn nearest_selectable_painted_index(painted: &[PaintedLine], from: usize) -> usize {
    if painted.is_empty() {
        return 0;
    }
    let from = from.min(painted.len() - 1);
    if painted[from].selectable {
        return from;
    }
    for i in from + 1..painted.len() {
        if painted[i].selectable {
            return i;
        }
    }
    for i in (0..from).rev() {
        if painted[i].selectable {
            return i;
        }
    }
    0
}

fn single_or_batch(effects: Vec<Effect>) -> Effect {
    match effects.len() {
        0 => Effect::None,
        1 => effects.into_iter().next().expect("one effect"),
        _ => Effect::Batch(effects),
    }
}

fn initial_cursor(rows: &[VisibleRow]) -> usize {
    rows.iter()
        .position(|r| r.kind == NodeKind::File)
        .or_else(|| {
            rows.iter()
                .position(|r| r.kind == NodeKind::Repo || r.kind == NodeKind::Checkout)
        })
        .unwrap_or(0)
}

enum FoldOp {
    Toggle,
    Close,
    Open,
}

#[allow(dead_code)]
fn default_viewed_path() -> PathBuf {
    #[cfg(test)]
    {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        return std::env::temp_dir().join(format!(
            "ws-viewed-test-{}-{}.json",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
    }
    #[cfg(not(test))]
    viewed_store_path()
}

fn comment_path_for(viewed_path: &PathBuf) -> PathBuf {
    if let Ok(override_path) = std::env::var("WS_STATUS_COMMENT_STORE") {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    #[cfg(test)]
    {
        return viewed_path.with_file_name("comments.json");
    }
    #[cfg(not(test))]
    {
        let _ = viewed_path;
        comment_store_path()
    }
}

/// Ancestor ids to try when a tree-mode row disappears in flat mode.
fn focus_ancestor_ids(id: &str) -> Vec<String> {
    let Some((kind, rest)) = id.split_once(':') else {
        return Vec::new();
    };
    if rest.is_empty() {
        return Vec::new();
    }
    if kind == "checkout" {
        return vec![format!("repo:{rest}")];
    }
    if kind != "file" && kind != "dir" {
        return Vec::new();
    }
    let Some((repo, path)) = rest.split_once(':') else {
        return Vec::new();
    };
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let parent_segs = if segments.is_empty() {
        &[][..]
    } else {
        &segments[..segments.len() - 1]
    };
    let mut ids = Vec::new();
    for len in (1..=parent_segs.len()).rev() {
        ids.push(format!("dir:{repo}:{}", parent_segs[..len].join("/")));
    }
    ids.push(format!("repo:{repo}"));
    ids.push(format!("checkout:{repo}"));
    ids
}

fn visible_snapshot(snapshot: &WorkspaceSnapshot, show_ignored: bool) -> WorkspaceSnapshot {
    let mut copy = snapshot.clone();
    copy.show_ignored = show_ignored;
    visible_for_tree(&copy)
}

#[cfg(test)]
mod tests {
    use super::super::gates::ListFocusTarget;
    use super::super::keys::InputMode;
    use super::super::theme::{resolve_theme_id, ThemeId};
    use super::super::tree::list_viewport_start;
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::split::{pane_widths, side_by_side_column_widths, DIFF_SPLIT_FRACTION};
    use crate::tui::watch::watch_interval_ms;
    use workspace_status_graph::{Commit, GraphModel, GraphRef, Stash};

    fn repo(name: &str, dirty: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: dirty,
            has_staged: false,
            has_untracked: false,
            changes: if dirty {
                vec![FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                }]
            } else {
                vec![]
            },
            checkout_kind: crate::snapshot::CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
        }
    }

    fn state() -> AppState {
        let snapshot = build_workspace_snapshot(
            &[repo("app", true), repo("notes", true), repo("lib", false)],
            &["notes".into()],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    #[test]
    fn ignore_toggle_shows_and_hides_notes() {
        let mut app = state();
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
        let effect = app.dispatch(Action::ToggleShowIgnored);
        assert_eq!(effect, Effect::LoadRightPane);
        assert!(app.show_ignored);
        assert!(app.rows.iter().any(|r| r.label.contains("notes")));
        app.dispatch(Action::ToggleShowIgnored);
        assert!(!app.show_ignored);
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
    }

    #[test]
    fn hidden_ignored_skipped_on_workspace_fetch() {
        let mut app = state();
        app.cursor = 0;
        let effect = app.dispatch(Action::Fetch);
        match effect {
            Effect::Fetch { repos } => {
                assert_eq!(repos, vec!["app", "lib"]);
                assert!(!repos.iter().any(|r| r == "notes"));
            }
            other => panic!("expected fetch, got {other:?}"),
        }
        assert_eq!(app.status, "Fetching 0/2…");
    }

    #[test]
    fn workspace_pull_and_default_arm_running_op_progress() {
        let mut app_snap = repo("app", false);
        app_snap.sync_status = SyncStatus::Behind;
        app_snap.branch = "feature/x".into();
        let mut lib_snap = repo("lib", false);
        lib_snap.sync_status = SyncStatus::Behind;
        lib_snap.branch = "feature/y".into();
        let snapshot = build_workspace_snapshot(&[app_snap, lib_snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        match app.dispatch(Action::Pull) {
            Effect::Pull { repos } => assert_eq!(repos, vec!["app", "lib"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "Pulling 0/2…");
        match app.dispatch(Action::DefaultBranch) {
            Effect::DefaultBranch { repos } => assert_eq!(repos, vec!["app", "lib"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "Switching 0/2…");
    }

    #[test]
    fn shown_ignored_is_in_workspace_fetch() {
        let mut app = state();
        app.dispatch(Action::ToggleShowIgnored);
        app.cursor = 0;
        let effect = app.dispatch(Action::Fetch);
        match effect {
            Effect::Fetch { repos } => {
                assert!(repos.contains(&"app".to_string()));
                assert!(repos.contains(&"notes".to_string()));
            }
            other => panic!("expected fetch, got {other:?}"),
        }
    }

    #[test]
    fn space_toggles_reviewed_on_file_only() {
        let mut app = state();
        let file_idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file row");
        app.cursor = file_idx;
        let id = app.rows[file_idx].id.clone();
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(app.reviewed.contains(&id));
        app.cursor = 0;
        app.dispatch(Action::ToggleReviewed);
        assert!(app.reviewed.contains(&id));
    }

    fn sample_commit_files() -> Vec<CommitFile> {
        vec![CommitFile {
            status: "M".into(),
            path: "README.md".into(),
            old_path: None,
        }]
    }

    fn sample_commit_source() -> CommitFileSource {
        CommitFileSource::Commit {
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        }
    }

    fn two_commit_files() -> Vec<CommitFile> {
        vec![
            CommitFile {
                status: "M".into(),
                path: "one.rs".into(),
                old_path: None,
            },
            CommitFile {
                status: "M".into(),
                path: "two.rs".into(),
                old_path: None,
            },
        ]
    }

    fn tall_panning_diff() -> DiffContent {
        let mut body = format!("@@ -0,0 +1,41 @@\n+{}\n", "x".repeat(40));
        for i in 0..40 {
            body.push_str(&format!("+line {i}\n"));
        }
        DiffContent::from_unified(body)
    }

    fn pan_and_scroll_focused_diff(app: &mut AppState) {
        app.focus = FocusPane::Right;
        app.layout.diff_pane_width = 8;
        app.layout.diff_pane_height = 8;
        let steps = (app.diff_body_height() / 2 + 8) as i32;
        app.dispatch(Action::Move(steps));
        for _ in 0..8 {
            app.dispatch(Action::PanDiff(1));
        }
        assert!(
            app.diff_cursor > 0 && app.diff_scroll > 0 && app.diff_col_offset > 0,
            "need a stale viewport, cursor={} scroll={} pan={}",
            app.diff_cursor,
            app.diff_scroll,
            app.diff_col_offset
        );
    }

    #[test]
    fn tree_writes_noop_when_depth_at_least_one_and_right_focused() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let cursor = app.cursor;
        let id = app.rows[cursor].id.clone();
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
        );
        assert_eq!(app.focus, FocusPane::Right);
        assert!(app.in_commit_drill());
        let status = app.status.clone();
        for action in [
            Action::Stage,
            Action::Unstage,
            Action::Revert,
            Action::Fetch,
            Action::Pull,
            Action::Push,
            Action::DefaultBranch,
            Action::Branch,
            Action::RemoveWorktree,
            Action::StashMenu,
        ] {
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(
                app.cursor, cursor,
                "{action:?} must not move the tree cursor"
            );
            assert!(app.confirm.is_none(), "{action:?}");
            assert_eq!(app.status, status, "{action:?} must stay silent");
        }
        match app.dispatch(Action::Edit) {
            Effect::EditFile { path, .. } => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(!app.reviewed.contains(&id));
    }

    #[test]
    fn pull_and_default_on_file_or_dir_are_silent_fetch_stays() {
        let mut snap = tree_repo();
        snap.branch = "feature/x".into();
        snap.sync_status = SyncStatus::Behind;
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_id(&mut app, "file:app:README.md");
        let status = app.status.clone();
        assert_eq!(app.dispatch(Action::Pull), Effect::None);
        assert_eq!(app.status, status);
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert_eq!(app.status, status);
        match app.dispatch(Action::Fetch) {
            Effect::Fetch { repos } => assert_eq!(repos, vec!["app"]),
            other => panic!("{other:?}"),
        }
        focus_id(&mut app, "dir:app:src");
        let status = app.status.clone();
        assert_eq!(app.dispatch(Action::Pull), Effect::None);
        assert_eq!(app.status, status);
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert_eq!(app.status, status);
        match app.dispatch(Action::Fetch) {
            Effect::Fetch { repos } => assert_eq!(repos, vec!["app"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn space_toggles_reviewed_only_at_depth_zero_workspace_file() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        app.focus = FocusPane::Right;
        assert!(app.right_is_diff());
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(app.reviewed.contains(&id));
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
        );
        app.dispatch(Action::ToggleReviewed);
        assert!(app.reviewed.contains(&id), "depth 1 must not unmark");
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["+line".into()]),
        );
        app.dispatch(Action::ToggleReviewed);
        assert!(app.reviewed.contains(&id), "depth 2 must not unmark");
    }

    #[test]
    fn ctrl_o_fires_from_left_when_right_is_already_a_diff() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        app.focus = FocusPane::Left;
        assert!(app.right_is_diff());
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert!(app.full_context_active());

        let mut commit = state();
        focus_repo(&mut commit, "app");
        commit.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        commit.focus = FocusPane::Left;
        match commit.dispatch(Action::ToggleFullContext) {
            Effect::LoadCommitDiff { path, .. } => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
        assert!(commit.full_context_active());
    }

    #[test]
    fn remove_worktree_refuses_with_status_when_not_linked() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        assert!(app.confirm.is_none());
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        let file_idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file");
        app.cursor = file_idx;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        assert!(app.confirm.is_none());
        focus_checkout(&mut app, "app/.worktrees/feat");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert!(matches!(
            app.confirm,
            Some(PendingConfirm::RemoveWorktree { .. })
        ));
        assert!(app.confirm.is_some());
        app.dispatch(Action::ConfirmNo);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn workspace_default_branch_skips_default_and_hidden_ignored() {
        let mut app = state();
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert!(app.status.contains("no non-default"));
    }

    #[test]
    fn default_branch_on_focused_default_repo_is_noop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert!(app.status.contains("no non-default"));
        let snapshot = build_workspace_snapshot(
            &[RepoSnapshot {
                repo: "app".into(),
                branch: "feature/x".into(),
                sync_status: SyncStatus::NoUpstream,
                sync_note: String::new(),
                head: String::new(),
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
                merged_into_default: None,
                default_branch_override: None,
                local_branches: Vec::new(),
            }],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        match app.dispatch(Action::DefaultBranch) {
            Effect::DefaultBranch { repos } => assert_eq!(repos, vec!["app"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "Switching 0/1…");
    }

    fn two_file_repo() -> RepoSnapshot {
        RepoSnapshot {
            repo: "app".into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: true,
            changes: vec![
                FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                },
                FileChange {
                    path: "new.txt".into(),
                    staged_status: None,
                    unstaged_status: None,
                    untracked: true,
                    old_path: None,
                },
            ],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
        }
    }

    #[test]
    fn bulk_stage_on_repo_and_file_rows() {
        let snapshot = build_workspace_snapshot(
            &[two_file_repo(), repo("notes", true)],
            &["notes".into()],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        focus_repo(&mut app, "app");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, mut paths } => {
                assert_eq!(repo, "app");
                paths.sort();
                assert_eq!(paths, vec!["README.md", "new.txt"]);
            }
            other => panic!("{other:?}"),
        }
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Stage) {
            Effect::Stage { paths, .. } => assert_eq!(paths, vec!["README.md"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bulk_stage_on_checkout_not_family_container() {
        let mut linked = two_file_repo();
        linked.repo = ".worktrees/app/feat".into();
        linked.checkout_kind = CheckoutKind::Linked;
        linked.primary_repo = Some("app".into());
        linked.has_untracked = false;
        linked.changes = vec![FileChange {
            path: "wt.md".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        let snapshot = build_workspace_snapshot(&[two_file_repo(), linked], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        let idx = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Checkout && row.repo.as_deref() == Some("app"))
            .expect("primary checkout");
        app.cursor = idx;
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, mut paths } => {
                assert_eq!(repo, "app");
                paths.sort();
                assert_eq!(paths, vec!["README.md", "new.txt"]);
            }
            other => panic!("{other:?}"),
        }
        let wt = app
            .rows
            .iter()
            .position(|row| {
                row.kind == NodeKind::Checkout && row.repo.as_deref() == Some(".worktrees/app/feat")
            })
            .expect("linked checkout");
        app.cursor = wt;
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, ".worktrees/app/feat");
                assert_eq!(paths, vec!["wt.md"]);
            }
            other => panic!("{other:?}"),
        }
    }

    fn mixed_dirty(name: &str) -> RepoSnapshot {
        let mut snap = two_file_repo();
        snap.repo = name.into();
        snap
    }

    #[test]
    fn workspace_stage_unstage_revert_are_noop() {
        let snapshot =
            build_workspace_snapshot(&[mixed_dirty("app"), mixed_dirty("api")], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(
            app.focused_row().map(|row| row.kind),
            Some(NodeKind::Workspace)
        );
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        assert_eq!(app.dispatch(Action::Unstage), Effect::None);
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn no_updates_group_fetch_pull_default_are_noop() {
        let mut app = state();
        let idx = app
            .rows
            .iter()
            .position(|row| row.id == "group:no-updates")
            .expect("no-updates group");
        app.cursor = idx;
        assert_eq!(app.dispatch(Action::Fetch), Effect::None);
        assert_eq!(app.dispatch(Action::Pull), Effect::None);
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
    }

    #[test]
    fn refresh_on_checkout_names_that_repo() {
        let mut app = state();
        focus_repo(&mut app, "app");
        match app.dispatch(Action::Refresh) {
            Effect::ReloadRepo { repo } => assert_eq!(repo, "app"),
            other => panic!("{other:?}"),
        }
        app.cursor = 0;
        assert_eq!(
            app.focused_row().map(|row| row.kind),
            Some(NodeKind::Workspace)
        );
        assert_eq!(app.dispatch(Action::Refresh), Effect::ReloadSnapshot);
        let idx = app
            .rows
            .iter()
            .position(|row| row.id == "group:no-updates")
            .expect("no-updates group");
        app.cursor = idx;
        assert_eq!(app.dispatch(Action::Refresh), Effect::ReloadSnapshot);
    }

    #[test]
    fn refresh_on_linked_checkout_names_that_path() {
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", true),
                RepoSnapshot {
                    repo: ".worktrees/app/feat".into(),
                    branch: "feature/x".into(),
                    sync_status: SyncStatus::NoUpstream,
                    sync_note: String::new(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: None,
                    default_branch_override: None,
                    local_branches: Vec::new(),
                },
            ],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let idx = app
            .rows
            .iter()
            .position(|row| {
                row.kind == NodeKind::Checkout && row.repo.as_deref() == Some(".worktrees/app/feat")
            })
            .expect("linked checkout");
        app.cursor = idx;
        match app.dispatch(Action::Refresh) {
            Effect::ReloadRepo { repo } => assert_eq!(repo, ".worktrees/app/feat"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn revert_y_keeps_untracked_on_mixed_scope_y_clean_deletes() {
        let snapshot = build_workspace_snapshot(&[two_file_repo()], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert_eq!(tracked, vec!["README.md"]);
                assert!(untracked.is_empty());
            }
            other => panic!("{other:?}"),
        }
        focus_repo(&mut app, "app");
        app.dispatch(Action::Revert);
        match app.dispatch(Action::ConfirmYesClean) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert_eq!(tracked, vec!["README.md"]);
                assert_eq!(untracked, vec!["new.txt"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn push_in_sync_is_noop_ahead_allowed() {
        let mut in_sync = repo("app", false);
        in_sync.sync_status = SyncStatus::UpToDate;
        let mut ahead = repo("lib", false);
        ahead.sync_status = SyncStatus::Ahead;
        ahead.sync_note = "ahead 1".into();
        let snapshot = build_workspace_snapshot(&[in_sync, ahead], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.folds.remove("group:no-updates");
        app.rebuild_rows();
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Push), Effect::None);
        assert!(app.status.contains("nothing to push"));
        focus_repo(&mut app, "lib");
        match app.dispatch(Action::Push) {
            Effect::Push { repos } => assert_eq!(repos, vec!["lib"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "Pushing 0/1…");
    }

    #[test]
    fn help_toggle_does_not_quit() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::ToggleHelp), Effect::None);
        assert!(app.help_open);
        assert_eq!(app.dispatch(Action::Quit), Effect::Quit);
    }

    #[test]
    fn ctrl_c_prompts_then_quits_within_the_window() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        assert_eq!(app.status, CTRL_C_EXIT_PROMPT);
        assert!(app.ctrl_c_armed_until.is_some());
        assert_eq!(app.dispatch(Action::CtrlC), Effect::Quit);
    }

    #[test]
    fn ctrl_c_expired_arm_is_a_fresh_prompt_not_quit() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        let until = app.ctrl_c_armed_until.expect("armed");
        assert!(!app.expire_ctrl_c_prompt(until - Duration::from_millis(1)));
        assert_eq!(app.status, CTRL_C_EXIT_PROMPT);
        assert!(app.expire_ctrl_c_prompt(until));
        assert!(app.status.is_empty());
        assert!(app.ctrl_c_armed_until.is_none());
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        assert_eq!(app.status, CTRL_C_EXIT_PROMPT);
    }

    #[test]
    fn other_keys_do_not_disarm_ctrl_c() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        let _ = app.dispatch(Action::Move(1));
        assert_eq!(app.dispatch(Action::CtrlC), Effect::Quit);
    }

    #[test]
    fn q_still_quits_immediately() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::Quit), Effect::Quit);
    }

    fn focus_file(app: &mut AppState, needle: &str) {
        let idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File && r.label.contains(needle))
            .expect("file row");
        app.cursor = idx;
    }

    #[test]
    fn search_n_n_unfolds_parents() {
        let mut app = state();
        app.folds.insert("repo:app".into());
        app.rebuild_rows();
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        app.dispatch(Action::SearchChar('R'));
        app.dispatch(Action::SearchChar('E'));
        app.dispatch(Action::SearchChar('A'));
        app.dispatch(Action::SearchChar('D'));
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_active);
        assert!(!app.search_mode);
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
        assert_eq!(
            app.focused_row().map(|r| r.id.as_str()),
            Some("file:app:README.md")
        );
        let first = app.focused_row().map(|r| r.id.clone());
        app.dispatch(Action::SearchNext);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.focused_row().map(|r| r.id.clone()), first);
    }

    #[test]
    fn hidden_ignored_not_in_search_or_stage() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        for c in "notes".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert!(app.status.contains("no match"));
        app.dispatch(Action::SearchCancel);
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
        focus_repo(&mut app, "app");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                assert_eq!(paths, vec!["README.md"]);
            }
            other => panic!("{other:?}"),
        }
        app.dispatch(Action::ToggleShowIgnored);
        assert!(app.rows.iter().any(|r| r.label.contains("notes")));
        focus_file(&mut app, "README.md");
        // notes file is also README? notes has README.md in helper
        let notes = app.rows.iter().position(|r| r.id == "file:notes:README.md");
        assert!(notes.is_some());
        app.cursor = notes.unwrap();
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, .. } => assert_eq!(repo, "notes"),
            other => panic!("expected stage notes, got {other:?}"),
        }
    }

    #[test]
    fn stage_unstage_revert_on_file_only() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                assert_eq!(paths, vec!["README.md"]);
            }
            other => panic!("{other:?}"),
        }
        // synthetic staged-only
        if let Some(row) = app.rows.iter_mut().find(|r| r.kind == NodeKind::File) {
            if let Some(file) = row.file.as_mut() {
                file.staged_status = Some("M".into());
                file.unstaged_status = None;
            }
        }
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Unstage) {
            Effect::Unstage { paths, .. } => assert_eq!(paths, vec!["README.md"]),
            other => panic!("{other:?}"),
        }
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.status.contains("staged only"));
    }

    #[test]
    fn revert_confirm_cancels_and_applies() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("cancelled"));
        focus_file(&mut app, "README.md");
        app.dispatch(Action::Revert);
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                repo,
                tracked,
                untracked,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(tracked, vec!["README.md"]);
                assert!(untracked.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
    }

    #[test]
    fn resize_does_not_dismiss_revert_confirm() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(
            app.dispatch(Action::Resize {
                cols: 100,
                rows: 24
            }),
            Effect::None
        );
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn watch_zero_is_disable_and_refresh_keeps_focus() {
        assert_eq!(watch_interval_ms(Some("0")), 0);
        assert!(watch_interval_ms(Some("2000")) >= 500);
        let mut app = state();
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        let folds = app.folds.clone();
        app.graph_scroll = 3;
        app.diff_scroll = 4;
        let snapshot = app.snapshot.clone();
        let changed = app.apply_watch_snapshot(snapshot);
        assert!(changed.is_empty());
        assert_eq!(app.focused_row().unwrap().id, id);
        assert_eq!(app.folds, folds);
        assert_eq!(app.graph_scroll, 3);
        assert_eq!(app.diff_scroll, 4);
        assert_eq!(app.dispatch(Action::WatchTick), Effect::WatchRefresh);
    }

    fn mini_graph(ids: &[&str]) -> GraphModel {
        GraphModel {
            uncommitted: Some(false),
            commits: ids
                .iter()
                .map(|id| Commit {
                    id: (*id).into(),
                    subject: format!("s-{id}"),
                    ..Commit::default()
                })
                .collect(),
            window: ids.len(),
            skip: 0,
            limit: 300,
            ..GraphModel::default()
        }
    }

    #[test]
    fn watch_add_update_remove_flashes_same_ids_only() {
        let mut app = state();
        app.flashes.clear();
        let file_id = app
            .rows
            .iter()
            .find(|row| row.label.contains("README.md"))
            .map(|row| row.id.clone())
            .expect("readme row");
        let mut snapshot = app.snapshot.clone();
        snapshot.repos[0].changes.push(FileChange {
            path: "new.rs".into(),
            staged_status: None,
            unstaged_status: Some("A".into()),
            untracked: true,
            old_path: None,
        });
        let added = app.apply_watch_snapshot(snapshot.clone());
        assert!(
            added.iter().any(|id| id.contains("new.rs")),
            "added file should flash: {added:?}"
        );
        assert!(
            !added.contains(&file_id),
            "unchanged readme should not flash: {added:?}"
        );

        snapshot.repos[0].changes.retain(|c| c.path != "README.md");
        let removed = app.apply_watch_snapshot(snapshot);
        assert!(
            removed.iter().any(|id| id.contains("README.md")),
            "removed readme should flash: {removed:?}"
        );
        assert!(
            app.tree_ghosts.iter().any(|g| g.id.contains("README.md")),
            "removed readme should stay as a ghost"
        );
    }

    #[test]
    fn graph_repo_switch_seeds_without_flashing() {
        let mut app = state();
        app.flashes.clear();
        app.set_graph(mini_graph(&["aaa"]), "app".into(), "aaa".into());
        assert!(
            app.flashes.is_empty(),
            "first graph paint seeds: {:?}",
            app.flashes.keys().collect::<Vec<_>>()
        );
        app.set_graph(mini_graph(&["aaa"]), "lib".into(), "aaa".into());
        assert!(
            app.flashes.is_empty(),
            "repo switch must not flash: {:?}",
            app.flashes.keys().collect::<Vec<_>>()
        );
        let mut updated = mini_graph(&["aaa"]);
        updated.commits[0].subject = "changed".into();
        app.set_graph(updated, "lib".into(), "aaa".into());
        assert!(
            app.flashes.keys().any(|id| id.contains("commit:aaa")),
            "same-repo subject change should flash: {:?}",
            app.flashes.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn edit_keeps_fold_focus_scroll() {
        let mut app = state();
        app.folds.insert("group:no-updates".into());
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        let folds = app.folds.clone();
        app.diff_scroll = 7;
        match app.dispatch(Action::Edit) {
            Effect::EditFile { repo, path } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "README.md");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.focused_row().unwrap().id, id);
        assert_eq!(app.folds, folds);
        assert_eq!(app.diff_scroll, 7);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Edit), Effect::None);
    }

    #[test]
    fn file_writes_on_git_fixture_honor_revert_confirm() {
        use crate::config::WorkspaceStatusConfig;
        use crate::git::{revert_tracked_file, stage_file, unstage_file};
        use crate::tui::collect_full_snapshot;
        use std::fs;
        use std::process::Command;
        use std::time::{SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "ws-tui-ops-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        fs::create_dir_all(&repo_dir).unwrap();
        let env = [
            ("GIT_AUTHOR_NAME", "workspace-status test"),
            ("GIT_AUTHOR_EMAIL", "workspace-status-test@example.invalid"),
            ("GIT_COMMITTER_NAME", "workspace-status test"),
            (
                "GIT_COMMITTER_EMAIL",
                "workspace-status-test@example.invalid",
            ),
        ];
        let git = |args: &[&str]| {
            let mut cmd = Command::new("git");
            cmd.args(args).current_dir(&repo_dir);
            for (k, v) in env {
                cmd.env(k, v);
            }
            assert!(cmd.status().unwrap().success(), "{args:?}");
        };
        let init = Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&repo_dir)
            .status();
        if init.map(|s| s.success()).unwrap_or(false) == false {
            git(&["init", "-q"]);
            git(&["checkout", "-q", "-b", "main"]);
        }
        fs::write(repo_dir.join("README.md"), "# seed\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-q", "-m", "seed"]);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();

        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                stage_file(&repo_dir, &paths[0]).unwrap();
            }
            other => panic!("{other:?}"),
        }
        unstage_file(&repo_dir, "README.md").unwrap();
        focus_file(&mut app, "README.md");
        app.dispatch(Action::Revert);
        assert!(app.confirm.is_some());
        app.dispatch(Action::ConfirmNo);
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# dirty\n"
        );
        app.dispatch(Action::Revert);
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert!(untracked.is_empty());
                revert_tracked_file(&repo_dir, &tracked[0]).unwrap();
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# seed\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn focus_repo(app: &mut AppState, name: &str) {
        let idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Repo && r.repo.as_deref() == Some(name))
            .expect("repo row");
        app.cursor = idx;
    }

    #[test]
    fn push_omits_hidden_ignored_and_unfocused_worktree() {
        use crate::snapshot::CheckoutKind;
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", true),
                RepoSnapshot {
                    repo: ".worktrees/app/feat".into(),
                    branch: "feature/x".into(),
                    sync_status: SyncStatus::Ahead,
                    sync_note: "ahead 1".into(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: None,
                    default_branch_override: None,
                    local_branches: Vec::new(),
                },
                RepoSnapshot {
                    repo: "notes".into(),
                    branch: "main".into(),
                    sync_status: SyncStatus::Ahead,
                    sync_note: "ahead 1".into(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Primary,
                    primary_repo: None,
                    merged_into_default: None,
                    default_branch_override: None,
                    local_branches: Vec::new(),
                },
            ],
            &["notes".into()],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Push), Effect::None);
        focus_repo(&mut app, "app");
        match app.dispatch(Action::Push) {
            Effect::Push { repos } => {
                assert_eq!(repos, vec!["app"]);
                assert!(!repos
                    .iter()
                    .any(|r| r.contains("worktrees") || r == "notes"));
            }
            other => panic!("{other:?}"),
        }
        let wt = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some(".worktrees/app/feat"));
        if let Some(idx) = wt {
            app.cursor = idx;
            match app.dispatch(Action::Push) {
                Effect::Push { repos } => assert_eq!(repos, vec![".worktrees/app/feat"]),
                other => panic!("{other:?}"),
            }
        }
        app.dispatch(Action::ToggleShowIgnored);
        let notes = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes"))
            .expect("notes");
        app.cursor = notes;
        match app.dispatch(Action::Push) {
            Effect::Push { repos } => assert_eq!(repos, vec!["notes"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stash_pop_drop_confirm_cancel_and_apply() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Left;
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("file menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create]
        );
        assert_eq!(app.dispatch(Action::StashMenuCancel), Effect::None);
        assert!(app.stash_menu.is_none());
        assert!(app.status.contains("cancelled"));

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(
            app.dispatch(Action::StashMenuChar('s')),
            Effect::StashCreate {
                repo: "app".into(),
                paths: vec!["README.md".into()],
            }
        );
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('a')), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.dispatch(Action::StashMenuChar('p')), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.dispatch(Action::StashMenuChar('d')), Effect::None);
        assert!(app.confirm.is_none());

        app.folds.remove("group:no-updates");
        app.rebuild_rows();
        focus_repo(&mut app, "lib");
        app.focus = FocusPane::Left;
        app.open_stash_menu("lib".into(), Some("stash@{0}".into()));
        assert!(app.stash_menu.is_none());
        assert!(app.status.contains("nothing to stash"));

        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{0}".into(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{0}"))
            .expect("stash");
        app.graph_cursor = idx;
        match app.dispatch(Action::GraphStashApply) {
            Effect::StashApply { repo, stash_ref } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.dispatch(Action::GraphStashDrop), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.status.contains("drop cancelled"));
        app.dispatch(Action::GraphStashDrop);
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
            other => panic!("{other:?}"),
        }
        match app.dispatch(Action::GraphStashPop) {
            Effect::StashPop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn branch_picker_list_create_checkout() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        assert!(matches!(
            app.dispatch(Action::Branch),
            Effect::PrepareBranchPicker { repo } if repo == "app"
        ));
        app.open_branch_picker(
            "app".into(),
            vec![
                LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                },
                LocalBranch {
                    name: "feature/x".into(),
                    current: false,
                    authordate: 2,
                },
            ],
        );
        assert!(app.branch_picker.is_some());
        app.dispatch(Action::BranchChar('f'));
        assert_eq!(
            app.branch_picker
                .as_ref()
                .unwrap()
                .selected()
                .map(|b| b.name.as_str()),
            Some("feature/x")
        );
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "feature/x");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }

        app.open_branch_picker(
            "app".into(),
            vec![LocalBranch {
                name: "main".into(),
                current: true,
                authordate: 1,
            }],
        );
        app.dispatch(Action::CreateBranchStart);
        for c in "feature/new".chars() {
            app.dispatch(Action::CreateBranchChar(c));
        }
        match app.dispatch(Action::CreateBranchSubmit) {
            Effect::CreateBranch { name, repo } => {
                assert_eq!(repo, "app");
                assert_eq!(name, "feature/new");
            }
            other => panic!("{other:?}"),
        }

        assert_eq!(
            app.confirm_checkout_if_out_of_sync(
                "app".into(),
                "feature/x".into(),
                Some("origin/feature/x".into()),
            ),
            Effect::None
        );
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        app.confirm_checkout_if_out_of_sync(
            "app".into(),
            "feature/x".into(),
            Some("origin/feature/x".into()),
        );
        match app.dispatch(Action::ConfirmYes) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "feature/x");
                assert_eq!(fast_forward_ref.as_deref(), Some("origin/feature/x"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tree_picker_already_on_closes_with_no_git() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_branch_picker(
            "app".into(),
            vec![LocalBranch {
                name: "main".into(),
                current: true,
                authordate: 1,
            }],
        );
        assert_eq!(app.dispatch(Action::BranchSubmit), Effect::None);
        assert!(app.branch_picker.is_none());
        assert_eq!(app.status, "Already on main");
        assert!(app.confirm.is_none());
    }

    #[test]
    fn tree_picker_local_selection_never_confirms() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_branch_picker(
            "app".into(),
            vec![
                LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                },
                LocalBranch {
                    name: "feature/x".into(),
                    current: false,
                    authordate: 2,
                },
            ],
        );
        app.dispatch(Action::BranchChar('f'));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "feature/x");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert!(app.branch_picker.is_some());
    }

    fn graph_state(dirty: bool) -> AppState {
        let mut app_repo = repo("app", dirty);
        if !dirty {
            app_repo.branch = "feature/x".into();
        }
        let snapshot = build_workspace_snapshot(
            &[app_repo, repo("notes", true)],
            &["notes".into()],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn install_graph_commit(app: &mut AppState, refs: &[&str]) {
        install_graph_commit_refs(app, refs.iter().map(|s| GraphRef::from(*s)).collect());
    }

    fn install_graph_commit_refs(app: &mut AppState, refs: Vec<GraphRef>) {
        let id = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let model = GraphModel {
            commits: vec![Commit {
                id: id.into(),
                subject: "head".into(),
                parents: Vec::new(),
                refs,
                author_name: String::new(),
                author_date_unix: 0,
            }],
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some(id.into()),
            sync: None,
            show_ignored: app.show_ignored,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(model, "app".into(), id.into());
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    #[test]
    fn graph_commit_b_one_local_ref_checkouts() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        match app.dispatch(Action::GraphCheckout) {
            Effect::CheckoutBranch {
                repo,
                selected_name,
                fast_forward_ref,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(selected_name, "main");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_commit_b_several_refs_opens_name_picker() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["origin/z", "topic", "main", "origin/main"]);
        assert_eq!(app.dispatch(Action::GraphCheckout), Effect::None);
        let picker = app.branch_picker.as_ref().expect("picker");
        let names: Vec<&str> = picker.branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["main", "topic", "origin/main", "origin/z"]);
        app.dispatch(Action::BranchMove(2));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "origin/main");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_commit_b_dirty_refuses() {
        let mut app = graph_state(true);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        assert_eq!(app.dispatch(Action::GraphCheckout), Effect::None);
        assert!(app.branch_picker.is_none());
        assert!(app.status.contains("commit or stash"));
    }

    #[test]
    fn graph_commit_b_tag_only_is_noop() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit_refs(&mut app, vec![GraphRef::tag("v1.0")]);
        assert_eq!(app.dispatch(Action::GraphCheckout), Effect::None);
        assert!(app.branch_picker.is_none());
    }

    #[test]
    fn graph_commit_c_creates_at_commit_without_checkout() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        assert!(app.create_branch.as_ref().unwrap().commit_id.is_some());
        for c in "topic/x".chars() {
            app.dispatch(Action::CreateBranchChar(c));
        }
        match app.dispatch(Action::CreateBranchSubmit) {
            Effect::CreateBranchAt {
                repo,
                name,
                commit_id,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(name, "topic/x");
                assert_eq!(commit_id, "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_commit_m_opens_confirm_yes_merges_no_cancels() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        match &app.confirm {
            Some(PendingConfirm::MergeIntoHead {
                repo,
                rev,
                label,
                into,
            }) => {
                assert_eq!(repo, "app");
                assert_eq!(rev, "topic");
                assert_eq!(label, "topic");
                assert_eq!(into, "feature/x");
            }
            other => panic!("{other:?}"),
        }
        match app.dispatch(Action::ConfirmYes) {
            Effect::MergeIntoHead { repo, rev, label } => {
                assert_eq!(repo, "app");
                assert_eq!(rev, "topic");
                assert_eq!(label, "topic");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());

        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "merge cancelled");
    }

    #[test]
    fn graph_commit_m_dirty_refuses() {
        let mut app = graph_state(true);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("commit or stash"));
    }

    #[test]
    fn graph_commit_m_tag_merges_commit_id() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit_refs(&mut app, vec![GraphRef::tag("v1.0")]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        match &app.confirm {
            Some(PendingConfirm::MergeIntoHead { rev, label, .. }) => {
                assert_eq!(rev, "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
                assert_eq!(label, "aaa1111");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_merge_worktree_only_when_that_row_is_focused() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("commit or stash"));

        focus_checkout(&mut app, "app/.worktrees/feat");
        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        match &app.confirm {
            Some(PendingConfirm::MergeIntoHead { repo, into, .. }) => {
                assert_eq!(repo, "app/.worktrees/feat");
                assert_eq!(into, "feature/x");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_stash_m_is_noop_for_merge() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{0}".into(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(_)))
            .expect("stash");
        app.graph_cursor = idx;
        assert!(!app.graph_commit_focused());
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn tree_b_still_opens_head_picker() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        assert!(matches!(
            app.dispatch(Action::Branch),
            Effect::PrepareBranchPicker { repo } if repo == "app"
        ));
        install_graph_commit(&mut app, &["main"]);
        app.focus = FocusPane::Left;
        assert!(matches!(
            app.dispatch(Action::Branch),
            Effect::PrepareBranchPicker { repo } if repo == "app"
        ));
    }

    #[test]
    fn tree_c_is_noop_on_repo_file_and_workspace() {
        let mut app = state();
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        assert!(app.create_branch.is_none());
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
    }

    #[test]
    fn graph_stash_b_stays_tree_and_c_is_noop() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{0}".into(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(_)))
            .expect("stash");
        app.graph_cursor = idx;
        assert!(!app.graph_commit_focused());
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(app.dispatch(Action::Branch), Effect::None);
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        assert!(app.create_branch.is_none());
    }

    #[test]
    fn stash_and_branch_git_fixture_honor_confirm() {
        use crate::config::WorkspaceStatusConfig;
        use crate::git::{
            checkout_branch, create_branch_checkout, latest_stash_ref, list_local_branches,
            stash_apply, stash_drop, stash_pop, stash_push,
        };
        use crate::tui::collect_full_snapshot;
        use std::fs;
        use std::process::Command;
        use std::time::{SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "ws-tui-stash-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        fs::create_dir_all(&repo_dir).unwrap();
        let env = [
            ("GIT_AUTHOR_NAME", "workspace-status test"),
            ("GIT_AUTHOR_EMAIL", "workspace-status-test@example.invalid"),
            ("GIT_COMMITTER_NAME", "workspace-status test"),
            (
                "GIT_COMMITTER_EMAIL",
                "workspace-status-test@example.invalid",
            ),
        ];
        let git = |args: &[&str]| {
            let mut cmd = Command::new("git");
            cmd.args(args).current_dir(&repo_dir);
            for (k, v) in env {
                cmd.env(k, v);
            }
            assert!(cmd.status().unwrap().success(), "{args:?}");
        };
        let init = Command::new("git")
            .args(["init", "-q", "-b", "main"])
            .current_dir(&repo_dir)
            .status();
        if init.map(|s| s.success()).unwrap_or(false) == false {
            git(&["init", "-q"]);
            git(&["checkout", "-q", "-b", "main"]);
        }
        fs::write(repo_dir.join("README.md"), "# seed\n").unwrap();
        git(&["add", "README.md"]);
        git(&["commit", "-q", "-m", "seed"]);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();

        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        focus_file(&mut app, "README.md");
        app.open_stash_menu("app".into(), latest_stash_ref(&repo_dir));
        match app.dispatch(Action::StashMenuChar('s')) {
            Effect::StashCreate { paths, .. } => stash_push(&repo_dir, &paths).unwrap(),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# seed\n"
        );
        let latest = latest_stash_ref(&repo_dir).expect("stash");
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: latest.clone(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == latest))
            .expect("stash row");
        app.graph_cursor = idx;
        match app.dispatch(Action::GraphStashApply) {
            Effect::StashApply { stash_ref, .. } => stash_apply(&repo_dir, &stash_ref).unwrap(),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# dirty\n"
        );
        app.dispatch(Action::GraphStashDrop);
        app.dispatch(Action::ConfirmNo);
        assert_eq!(
            latest_stash_ref(&repo_dir).as_deref(),
            Some(latest.as_str())
        );
        app.dispatch(Action::GraphStashDrop);
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => stash_drop(&repo_dir, &stash_ref).unwrap(),
            other => panic!("{other:?}"),
        }
        assert!(latest_stash_ref(&repo_dir).is_none());

        fs::write(repo_dir.join("README.md"), "# again\n").unwrap();
        stash_push(&repo_dir, &[]).unwrap();
        let latest = latest_stash_ref(&repo_dir).expect("stash2");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: latest.clone(),
                subject: "again".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == latest))
            .expect("stash2 row");
        app.graph_cursor = idx;
        assert_eq!(
            app.dispatch(Action::GraphStashPop),
            Effect::StashPop {
                repo: "app".into(),
                stash_ref: latest.clone(),
            }
        );
        stash_pop(&repo_dir, &latest).unwrap();
        assert!(latest_stash_ref(&repo_dir).is_none());

        focus_repo(&mut app, "app");
        app.focus = FocusPane::Left;
        match app.dispatch(Action::Branch) {
            Effect::PrepareBranchPicker { repo } => {
                app.open_branch_picker(repo, list_local_branches(&repo_dir));
            }
            other => panic!("{other:?}"),
        }
        assert!(app
            .branch_picker
            .as_ref()
            .unwrap()
            .branches
            .iter()
            .any(|b| b.name == "main"));
        app.dispatch(Action::CreateBranchStart);
        for c in "feature/pick".chars() {
            app.dispatch(Action::CreateBranchChar(c));
        }
        match app.dispatch(Action::CreateBranchSubmit) {
            Effect::CreateBranch { name, .. } => {
                create_branch_checkout(&repo_dir, &name).unwrap();
            }
            other => panic!("{other:?}"),
        }
        assert!(checkout_branch("main", &repo_dir));
        app.open_branch_picker("app".into(), list_local_branches(&repo_dir));
        app.dispatch(Action::BranchChar('p'));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch { selected_name, .. } => {
                assert_eq!(selected_name, "feature/pick");
                assert!(checkout_branch(&selected_name, &repo_dir));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            crate::git::exec_git(&["branch", "--show-current"], &repo_dir),
            "feature/pick"
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn linked_snapshot() -> crate::snapshot::WorkspaceSnapshot {
        build_workspace_snapshot(
            &[
                repo("app", true),
                RepoSnapshot {
                    repo: "app/.worktrees/feat".into(),
                    branch: "feature/x".into(),
                    sync_status: SyncStatus::NoUpstream,
                    sync_note: String::new(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: Some(false),
                    default_branch_override: None,
                    local_branches: Vec::new(),
                },
                repo("notes", true),
            ],
            &["notes".into()],
            false,
            &[],
        )
    }

    fn focus_checkout(app: &mut AppState, name: &str) {
        let idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Checkout && r.repo.as_deref() == Some(name))
            .expect("checkout row");
        app.cursor = idx;
    }

    #[test]
    fn remove_worktree_linked_only_confirm_cancel_and_apply() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        assert!(app.confirm.is_none());
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        let file_idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file");
        app.cursor = file_idx;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        focus_checkout(&mut app, "app/.worktrees/feat");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert!(matches!(
            app.confirm,
            Some(PendingConfirm::RemoveWorktree { ref path, force, .. })
                if path == "app/.worktrees/feat" && !force
        ));
        app.dispatch(Action::ConfirmNo);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("cancelled"));
        focus_checkout(&mut app, "app/.worktrees/feat");
        app.dispatch(Action::RemoveWorktree);
        match app.dispatch(Action::ConfirmYes) {
            Effect::RemoveWorktree {
                primary,
                path,
                force,
            } => {
                assert_eq!(primary, "app");
                assert_eq!(path, "app/.worktrees/feat");
                assert!(!force);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn remove_worktree_hidden_ignored_is_noop() {
        let snapshot = build_workspace_snapshot(
            &[RepoSnapshot {
                repo: "notes/.worktrees/feat".into(),
                branch: "feature/x".into(),
                sync_status: SyncStatus::NoUpstream,
                sync_note: String::new(),
                head: String::new(),
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Linked,
                primary_repo: Some("notes".into()),
                merged_into_default: None,
                default_branch_override: None,
                local_branches: Vec::new(),
            }],
            &["notes/.worktrees/feat".into()],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        assert!(app
            .rows
            .iter()
            .all(|r| r.repo.as_deref() != Some("notes/.worktrees/feat")));
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        app.dispatch(Action::ToggleShowIgnored);
        if let Some(idx) = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes/.worktrees/feat"))
        {
            app.cursor = idx;
            app.show_ignored = false;
            assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        }
    }

    #[test]
    fn fetch_tick_includes_linked_worktrees_skips_hidden_ignored() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        match app.dispatch(Action::FetchTick) {
            Effect::Fetch { repos } => {
                assert_eq!(repos, vec!["app", "app/.worktrees/feat"]);
                assert!(!repos.iter().any(|r| r == "notes"));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "Fetching 0/2…");
        assert_eq!(app.dispatch(Action::WatchTick), Effect::WatchRefresh);
        assert_eq!(watch_interval_ms(Some("0")), 0);
        assert_ne!(
            crate::tui::fetch::fetch_interval_ms(Some("0")),
            watch_interval_ms(Some("0")) + 1
        );
        assert_eq!(crate::tui::fetch::fetch_interval_ms(Some("0")), 0);
        assert_eq!(watch_interval_ms(None), crate::tui::watch::DEFAULT_WATCH_MS);
        assert_eq!(
            crate::tui::fetch::fetch_interval_ms(None),
            crate::tui::fetch::DEFAULT_FETCH_MS
        );
        assert_ne!(
            crate::tui::fetch::DEFAULT_FETCH_MS,
            crate::tui::watch::DEFAULT_WATCH_MS
        );
    }

    #[test]
    fn reviewed_persists_and_drops_on_fingerprint_change() {
        use crate::tui::viewed::{load_viewed_store, viewed_identity};
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "ws-reviewed-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("ws");
        let repo_dir = workspace.join("app");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut app = AppState::new(workspace.clone(), snapshot.clone(), true);
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(app.reviewed.contains(&id));
        let loaded = load_viewed_store(&app.viewed_path);
        assert!(loaded.contains_key(&viewed_identity("app", "README.md")));
        fs::write(repo_dir.join("README.md"), "# changed\n").unwrap();
        app.apply_snapshot(snapshot);
        assert!(!app.reviewed.contains(&id));
        assert!(load_viewed_store(&app.viewed_path).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    fn graph_commit(id: &str, subject: &str) -> Commit {
        Commit {
            id: id.into(),
            subject: subject.into(),
            parents: Vec::new(),
            refs: Vec::new(),
            author_name: String::new(),
            author_date_unix: 0,
        }
    }

    fn install_graph(app: &mut AppState, stashes: Vec<Stash>) {
        let model = GraphModel {
            commits: vec![graph_commit(
                "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "head",
            )],
            stashes,
            worktrees: Vec::new(),
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            sync: None,
            show_ignored: app.show_ignored,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    #[test]
    fn enter_on_graph_commit_opens_files_then_diff_and_esc_pops() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        match app.dispatch(Action::NavEnter) {
            Effect::LoadCommitFiles { repo, source } => {
                assert_eq!(repo, "app");
                assert_eq!(
                    source,
                    CommitFileSource::Commit {
                        commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                },
                CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                },
            ],
        );
        assert!(app.drill.is_files());
        assert!(app.graph.is_some(), "graph stays in the model during files");
        let rows = app.commit_file_rows();
        assert!(rows.iter().any(|row| row.id == "dir:src"));
        let lib = rows
            .iter()
            .position(|row| row.path == "src/lib.rs")
            .expect("src/lib.rs row");
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = lib;
        }
        match app.dispatch(Action::NavEnter) {
            Effect::LoadCommitDiff { path, .. } => assert_eq!(path, "src/lib.rs"),
            other => panic!("{other:?}"),
        }
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
            }],
            0,
            "src/lib.rs".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        assert!(app.drill.is_diff());
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(app.list_focus_target(), ListFocusTarget::None);
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.drill.is_diff(), "right Esc unfocuses without popping");
        assert_eq!(app.focus, FocusPane::Left);
        assert!(app.commit_files_list_focused());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.drill.is_files());
        assert_eq!(app.focus, FocusPane::Left);
        assert_eq!(app.list_focus_target(), ListFocusTarget::Graph);
        assert!(app.graph.is_some());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::LoadRightPane);
        assert!(app.drill.is_graph());
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn esc_on_right_at_files_unfocuses_without_popping() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
            }],
        );
        assert_eq!(app.focus, FocusPane::Right);
        assert!(app.drill.is_files());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.drill.is_files());
        assert_eq!(app.focus, FocusPane::Left);
        assert_eq!(app.list_focus_target(), ListFocusTarget::Graph);
    }

    #[test]
    fn j_k_on_depth_2_left_moves_commit_files_not_graph() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        let files = vec![
            CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
            },
            CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
            },
        ];
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files.clone(),
        );
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files,
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert_eq!(app.focus, FocusPane::Left);
        assert!(app.drill.is_diff());
        let graph_before = app.graph_cursor;
        let rows = app.commit_file_rows();
        let lib = rows
            .iter()
            .position(|row| row.path == "src/lib.rs")
            .expect("src/lib.rs row");
        assert!(lib > 0, "src/lib.rs should not be the first row: {rows:?}");
        if let DrillView::Diff { file_cursor, .. } = &mut app.drill {
            *file_cursor = lib - 1;
        }
        match app.dispatch(Action::Move(1)) {
            Effect::LoadRightPane => {}
            other => panic!("expected LoadRightPane, got {other:?}"),
        }
        assert_eq!(app.graph_cursor, graph_before);
        assert_eq!(app.commit_files_cursor(), lib);
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn j_k_on_depth_1_left_still_moves_graph() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        if let Some(model) = app.graph.as_mut() {
            model.commits.push(graph_commit(
                "bbb2222cccccccccccccccccccccccccccccccccc",
                "other",
            ));
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
            }],
        );
        app.focus = FocusPane::Left;
        let files_before = app.commit_files_cursor();
        let graph_before = app.graph_cursor;
        assert_eq!(app.dispatch(Action::Move(1)), Effect::LoadRightPane);
        assert_eq!(app.commit_files_cursor(), files_before);
        assert_ne!(app.graph_cursor, graph_before);
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn fold_on_depth_2_left_folds_commit_files() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        let files = vec![
            CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
            },
            CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
            },
        ];
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files,
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        app.focus = FocusPane::Left;
        let dir = app
            .commit_file_rows()
            .iter()
            .position(|row| row.id == "dir:src")
            .expect("dir:src");
        if let DrillView::Diff { file_cursor, .. } = &mut app.drill {
            *file_cursor = dir;
        }
        assert!(app
            .commit_file_rows()
            .iter()
            .any(|row| row.path == "src/lib.rs"));
        app.dispatch(Action::FoldToggle);
        assert!(
            !app.commit_file_rows()
                .iter()
                .any(|row| row.path == "src/lib.rs"),
            "folding dir:src should hide src/lib.rs"
        );
    }

    #[test]
    fn hidden_ignored_is_not_drilled_unless_shown() {
        let mut app = state();
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
        app.cursor = 0;
        app.focus = FocusPane::Right;
        install_graph(&mut app, Vec::new());
        // tree still on workspace / app, not hidden notes
        app.dispatch(Action::ToggleShowIgnored);
        let notes = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes"))
            .expect("notes");
        app.cursor = notes;
        app.show_ignored = false;
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::NavEnter), Effect::None);
        assert!(app.status.contains("hidden ignored"));
        app.show_ignored = true;
        app.rebuild_rows();
        let notes = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes"))
            .expect("notes shown");
        app.cursor = notes;
        install_graph(&mut app, Vec::new());
        match app.dispatch(Action::NavEnter) {
            Effect::LoadCommitFiles { repo, .. } => assert_eq!(repo, "notes"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_stash_row_apply_pop_drop_target_that_ref() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![
                Stash {
                    stash_ref: "stash@{0}".into(),
                    subject: "latest".into(),
                    parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                    ..Stash::default()
                },
                Stash {
                    stash_ref: "stash@{1}".into(),
                    subject: "older".into(),
                    parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                    ..Stash::default()
                },
            ],
        );
        let rows = app.graph.as_ref().unwrap().visible_rows();
        let idx = rows
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{1}"))
            .expect("stash@{1}");
        app.graph_cursor = idx;
        match app.dispatch(Action::GraphStashApply) {
            Effect::StashApply { stash_ref, repo } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{1}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.dispatch(Action::GraphStashDrop), Effect::None);
        assert!(app.status.contains("stash@{1}"));
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.status.contains("drop cancelled"));
        app.dispatch(Action::GraphStashDrop);
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{1}"),
            other => panic!("{other:?}"),
        }
        match app.dispatch(Action::GraphStashPop) {
            Effect::StashPop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{1}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stash_menu_from_repo_or_file_is_create_only() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Left;
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create]
        );
        assert!(ops.iter().all(|op| op.stash_ref.is_none()));
        app.stash_menu = None;
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{1}".into(),
                subject: "older".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{1}"))
            .expect("stash");
        app.graph_cursor = idx;
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("graph menu");
        assert!(ops.iter().any(|op| op.id == StashOpId::Drop));
        assert!(
            ops.iter()
                .any(|op| op.stash_ref.as_deref() == Some("stash@{1}")),
            "{ops:?}"
        );
        assert!(ops
            .iter()
            .all(|op| { op.stash_ref.is_none() || op.stash_ref.as_deref() == Some("stash@{1}") }));
    }

    fn focus_graph_row(app: &mut AppState, pred: impl Fn(&GraphRow) -> bool) {
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(pred)
            .expect("graph row");
        app.graph_cursor = idx;
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    fn graph_stash(stash_ref: &str, subject: &str) -> Stash {
        Stash {
            stash_ref: stash_ref.into(),
            subject: subject.into(),
            parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            ..Stash::default()
        }
    }

    #[test]
    fn graph_stash_menu_on_commit_offers_latest_apply_pop_not_drop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Commit { .. }));
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("graph commit menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create, StashOpId::Apply, StashOpId::Pop]
        );
        assert!(ops
            .iter()
            .all(|op| { op.stash_ref.is_none() || op.stash_ref.as_deref() == Some("stash@{0}") }));
        assert!(!ops.iter().any(|op| op.id == StashOpId::Drop));

        match app.dispatch(Action::StashMenuChar('p')) {
            Effect::StashPop { repo, stash_ref } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert!(app.stash_menu.is_none());

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('d')), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn graph_stash_menu_dispatch_from_right_focused_graph_is_noop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Commit { .. }));
        assert_eq!(app.focus, FocusPane::Right);
        let status = app.status.clone();
        assert_eq!(app.dispatch(Action::StashMenu), Effect::None);
        assert_eq!(app.status, status);
    }

    #[test]
    fn graph_stash_menu_on_uncommitted_offers_latest_apply_pop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        if let Some(graph) = app.graph.as_mut() {
            graph.uncommitted = Some(true);
        }
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Uncommitted { .. }));
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("uncommitted menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create, StashOpId::Apply, StashOpId::Pop]
        );
        assert!(!ops.iter().any(|op| op.id == StashOpId::Drop));
        assert!(ops
            .iter()
            .filter(|op| op.id != StashOpId::Create)
            .all(|op| op.stash_ref.as_deref() == Some("stash@{0}")));
    }

    #[test]
    fn stash_menu_pop_runs_immediately_drop_still_confirms() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        focus_graph_row(
            &mut app,
            |r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{0}"),
        );
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("stash row menu");
        assert!(ops.iter().any(|op| op.id == StashOpId::Drop));
        match app.dispatch(Action::StashMenuChar('p')) {
            Effect::StashPop { repo, stash_ref } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert!(app.stash_menu.is_none());

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('d')), Effect::None);
        assert!(app.confirm.is_some());
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
            other => panic!("{other:?}"),
        }
    }

    fn wide_split_layout() -> super::LayoutHit {
        let mut layout = super::LayoutHit::default();
        layout.term_cols = 160;
        layout.pane_height = 22;
        layout.outer_tree_width = 48;
        layout.right_x = 48;
        layout.diff_pane_width = 110;
        layout.diff_content_x = 50;
        let left = side_by_side_column_widths(110, DIFF_SPLIT_FRACTION).left_width;
        layout.diff_split_rule_x = Some(50 + left);
        layout
    }

    #[test]
    fn pane_drag_updates_ratio_and_release_ends_drag() {
        let mut app = state();
        app.layout = wide_split_layout();
        let start = pane_widths(160, app.tree_fraction).tree_width;
        assert_eq!(
            app.dispatch(Action::Click { col: 47, row: 5 }),
            Effect::None
        );
        assert_eq!(app.drag, SplitDrag::Pane);
        assert_eq!(app.dispatch(Action::Drag { col: 70, row: 8 }), Effect::None);
        let after = pane_widths(160, app.tree_fraction).tree_width;
        assert!(after > start, "start={start} after={after}");
        assert_eq!(after, 71);
        assert_eq!(app.drag, SplitDrag::Pane);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(pane_widths(160, app.tree_fraction).tree_width, after);
    }

    #[test]
    fn in_diff_drag_updates_ratio_and_release_ends_drag() {
        let mut app = state();
        app.layout = wide_split_layout();
        let rule = app.layout.diff_split_rule_x.expect("rule");
        let start = side_by_side_column_widths(110, app.diff_split_fraction).left_width;
        assert_eq!(
            app.dispatch(Action::Click { col: rule, row: 6 }),
            Effect::None
        );
        assert_eq!(app.drag, SplitDrag::Diff);
        assert_eq!(
            app.dispatch(Action::Drag {
                col: rule + 20,
                row: 6
            }),
            Effect::None
        );
        let after = side_by_side_column_widths(110, app.diff_split_fraction).left_width;
        assert!(after > start, "start={start} after={after}");
        assert_eq!(app.drag, SplitDrag::Diff);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(
            side_by_side_column_widths(110, app.diff_split_fraction).left_width,
            after
        );
    }

    #[test]
    fn drag_clamp_keeps_panes_nonzero() {
        let mut app = state();
        app.layout = wide_split_layout();
        app.dispatch(Action::Click { col: 47, row: 4 });
        app.dispatch(Action::Drag { col: 0, row: 4 });
        let w = pane_widths(160, app.tree_fraction);
        assert!(w.tree_width >= 20);
        assert!(w.diff_width >= 20);
        app.dispatch(Action::Release);
        let rule = app.layout.diff_split_rule_x.expect("rule");
        app.dispatch(Action::Click { col: rule, row: 4 });
        app.dispatch(Action::Drag { col: 0, row: 4 });
        let s = side_by_side_column_widths(110, app.diff_split_fraction);
        assert!(s.left_width >= 1);
        assert!(s.right_width >= 1);
    }

    #[test]
    fn click_on_tree_is_not_a_split_drag() {
        let mut app = state();
        app.layout = wide_split_layout();
        assert!(!app.rows.is_empty());
        let effect = app.dispatch(Action::Click {
            col: 10,
            row: app.layout.tree_y,
        });
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(effect, Effect::LoadRightPane);
        assert_eq!(app.cursor, app.layout.list_offset);
    }

    #[test]
    fn i_toggles_inline_without_mouse() {
        let mut app = state();
        assert_eq!(app.diff_mode, DiffMode::SideBySide);
        app.dispatch(Action::ToggleDiffMode);
        assert_eq!(app.diff_mode, DiffMode::Inline);
        app.dispatch(Action::ToggleDiffMode);
        assert_eq!(app.diff_mode, DiffMode::SideBySide);
    }

    fn arm_graph_scrollbar(app: &mut AppState, content_len: usize) {
        app.layout.term_cols = 160;
        app.layout.pane_height = 22;
        app.layout.outer_tree_width = 48;
        app.layout.right_x = 48;
        app.layout.right_y = 1;
        app.layout.graph_scrollbar_x = Some(158);
        app.layout.graph_scrollbar_y = 2;
        app.layout.graph_scrollbar_height = 10;
        app.layout.graph_content_len = content_len;
        app.graph_scroll = 0;
        app.focus = FocusPane::Right;
    }

    #[test]
    fn graph_scrollbar_thumb_drag_updates_scroll_and_release_ends_drag() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let content_len = paint_model(app.graph.as_ref().unwrap(), glyphs, None).len();
        arm_graph_scrollbar(&mut app, content_len);
        let thumb =
            workspace_status_graph::graph_scrollbar_thumb(content_len, 0, 10).expect("thumb");
        let thumb_row = 2 + thumb.0;
        let start = app.graph_scroll;
        let cursor = app.graph_cursor;
        assert_eq!(
            app.dispatch(Action::Click {
                col: 158,
                row: thumb_row
            }),
            Effect::None
        );
        assert_eq!(
            app.drag,
            SplitDrag::GraphScrollbar {
                origin_row: thumb_row,
                origin_scroll: start
            }
        );
        assert_eq!(app.graph_scroll, start, "thumb grab must not jump");
        assert_eq!(
            app.dispatch(Action::Drag {
                col: 158,
                row: thumb_row + 6
            }),
            Effect::None
        );
        assert!(
            app.graph_scroll > start,
            "thumb drag should scroll, got {}",
            app.graph_scroll
        );
        assert_eq!(
            app.graph_cursor, cursor,
            "scrollbar must not move the graph cursor"
        );
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);

        app.dispatch(Action::Move(1));
        assert_ne!(app.graph_cursor, cursor, "j still moves the graph cursor");
        let after_j = app.graph_cursor;
        app.dispatch(Action::PageMove(1));
        assert_ne!(app.graph_cursor, after_j, "PageDown still pages the graph");
        app.dispatch(Action::PanDiff(1));
        app.dispatch(Action::PanDiff(-1));
    }

    #[test]
    fn graph_scrollbar_track_click_jumps_and_arms_drag() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let content_len = paint_model(app.graph.as_ref().unwrap(), glyphs, None).len();
        arm_graph_scrollbar(&mut app, content_len);
        let thumb =
            workspace_status_graph::graph_scrollbar_thumb(content_len, 0, 10).expect("thumb");
        let track_row = 2 + 9;
        assert!(
            track_row >= 2 + thumb.0 + thumb.1,
            "fixture track row must sit below the thumb"
        );
        let cursor = app.graph_cursor;
        assert_eq!(
            app.dispatch(Action::Click {
                col: 158,
                row: track_row
            }),
            Effect::None
        );
        assert!(
            app.graph_scroll > 0,
            "track click should jump toward the click"
        );
        assert_eq!(
            app.drag,
            SplitDrag::GraphScrollbar {
                origin_row: track_row,
                origin_scroll: app.graph_scroll
            }
        );
        assert_eq!(app.graph_cursor, cursor);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
    }

    #[test]
    fn click_on_graph_body_is_not_a_scrollbar_drag() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let content_len = paint_model(app.graph.as_ref().unwrap(), glyphs, None).len();
        arm_graph_scrollbar(&mut app, content_len);
        let effect = app.dispatch(Action::Click { col: 80, row: 4 });
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(effect, Effect::None);
        assert_eq!(app.focus, FocusPane::Right);
    }

    fn arm_graph_hscrollbar(app: &mut AppState, col_max: u16) {
        app.layout.term_cols = 160;
        app.layout.pane_height = 22;
        app.layout.outer_tree_width = 48;
        app.layout.right_x = 48;
        app.layout.graph_hscrollbar_y = Some(12);
        app.layout.graph_hscrollbar_x = 50;
        app.layout.graph_hscrollbar_width = 20;
        app.layout.graph_col_max = col_max;
        app.right_col_offset = 0;
        app.focus = FocusPane::Right;
    }

    #[test]
    fn graph_horizontal_scrollbar_thumb_drag_updates_offset() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 4);
        arm_graph_hscrollbar(&mut app, 40);
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 0, 20).expect("thumb");
        let thumb_col = 50 + thumb.0;
        let start = app.right_col_offset;
        assert_eq!(
            app.dispatch(Action::Click {
                col: thumb_col,
                row: 12
            }),
            Effect::None
        );
        assert_eq!(
            app.drag,
            SplitDrag::GraphHScrollbar {
                origin_col: thumb_col,
                origin_offset: start
            }
        );
        assert_eq!(app.right_col_offset, start, "thumb grab must not jump");
        assert_eq!(
            app.dispatch(Action::Drag {
                col: thumb_col + 10,
                row: 12
            }),
            Effect::None
        );
        assert!(
            app.right_col_offset > start,
            "thumb drag should pan, got {}",
            app.right_col_offset
        );
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        app.dispatch(Action::PanDiff(-20));
        assert_eq!(app.right_col_offset, 0, "keyboard pan still clamps at 0");
    }

    #[test]
    fn mouse_hscroll_pans_graph_under_cursor_without_focus_steal() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        let model = GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: format!("subject-{}", "y".repeat(80)),
                parents: Vec::new(),
                refs: Vec::new(),
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        app.focus = FocusPane::Left;
        app.drill = DrillView::Graph;
        app.layout.right_x = 48;
        app.layout.diff_pane_width = 20;
        assert_eq!(app.right_col_offset, 0);
        assert_eq!(
            app.dispatch(Action::ScrollWheel {
                col: 80,
                row: 4,
                delta: 5,
                horizontal: true,
            }),
            Effect::None
        );
        assert!(
            app.right_col_offset > 0,
            "hscroll over the graph must pan even when the tree is focused"
        );
        assert_eq!(
            app.focus,
            FocusPane::Left,
            "hscroll must not steal left-pane focus"
        );
        app.mouse_enabled = false;
        let panned = app.right_col_offset;
        app.dispatch(Action::ScrollWheel {
            col: 80,
            row: 4,
            delta: 5,
            horizontal: true,
        });
        assert_eq!(app.right_col_offset, panned, "mouse off ignores hscroll");
    }

    #[test]
    fn theme_cycle_wraps_and_stays_in_session() {
        let mut app = state();
        app.theme = ThemeId::TokyoNight;
        assert_eq!(app.dispatch(Action::CycleTheme), Effect::None);
        assert_eq!(app.theme, ThemeId::Monokai);
        assert!(app.status.contains("Monokai"));
        app.dispatch(Action::CycleTheme);
        app.dispatch(Action::CycleTheme);
        app.dispatch(Action::CycleTheme);
        app.dispatch(Action::CycleTheme);
        assert_eq!(app.theme, ThemeId::TokyoNight);
        assert_eq!(resolve_theme_id(Some("gruvbox-dark")), ThemeId::GruvboxDark);
        assert_eq!(resolve_theme_id(Some("nope")), ThemeId::TokyoNight);
    }

    fn tree_repo() -> RepoSnapshot {
        RepoSnapshot {
            repo: "app".into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: false,
            changes: vec![
                FileChange {
                    path: "src/lib.rs".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                },
                FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                },
            ],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
        }
    }

    fn tree_app() -> AppState {
        let snapshot = build_workspace_snapshot(&[tree_repo()], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn focus_id(app: &mut AppState, id: &str) {
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
    }

    #[test]
    fn tree_mode_default_has_dir_and_basename() {
        let app = tree_app();
        assert!(app.tree_mode);
        let dir = app
            .rows
            .iter()
            .find(|row| row.id == "dir:app:src")
            .expect("dir");
        assert_eq!(dir.kind, NodeKind::Dir);
        assert!(dir.foldable);
        let lib = app
            .rows
            .iter()
            .find(|row| row.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("lib.rs"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert!(app.rows.iter().any(|row| row.id == "file:app:README.md"));
    }

    #[test]
    fn t_toggles_flat_full_paths_without_dirs() {
        let mut app = tree_app();
        focus_id(&mut app, "file:app:src/lib.rs");
        assert_eq!(app.dispatch(Action::ToggleTreeMode), Effect::LoadRightPane);
        assert!(!app.tree_mode);
        assert_eq!(app.status, "Flat paths");
        assert!(app.rows.iter().all(|row| row.kind != NodeKind::Dir));
        let lib = app
            .rows
            .iter()
            .find(|row| row.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("lib.rs"));
        assert!(lib.label.contains("src"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert_eq!(
            app.focused_row().map(|row| row.id.as_str()),
            Some("file:app:src/lib.rs")
        );
        app.dispatch(Action::ToggleTreeMode);
        assert!(app.tree_mode);
        assert_eq!(app.status, "Directory tree");
        assert!(app.rows.iter().any(|row| row.id == "dir:app:src"));
        assert_eq!(
            app.focused_row().map(|row| row.id.as_str()),
            Some("file:app:src/lib.rs")
        );
    }

    #[test]
    fn z_on_dir_hides_files() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("dir:app:src"));
        assert!(app.rows.iter().all(|row| row.id != "file:app:src/lib.rs"));
        assert!(app.rows.iter().any(|row| row.id == "file:app:README.md"));
    }

    #[test]
    fn stage_on_dir_skips_checkout_root_sibling() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                assert_eq!(paths, vec!["src/lib.rs"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn t_in_commit_files_toggles_commit_trie_not_workspace() {
        let mut app = tree_app();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                },
                CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                },
            ],
        );
        assert!(app.commit_tree_mode);
        assert!(app.tree_mode);
        assert!(app.commit_file_rows().iter().any(|row| row.id == "dir:src"));
        assert_eq!(app.dispatch(Action::ToggleTreeMode), Effect::None);
        assert!(!app.commit_tree_mode);
        assert!(app.tree_mode, "workspace t stays workspace-only");
        assert!(app.rows.iter().any(|row| row.id == "dir:app:src"));
        let rows = app.commit_file_rows();
        assert!(rows.iter().all(|row| !row.is_dir()));
        assert!(rows.iter().any(|row| {
            row.id == "file:src/lib.rs"
                && row.label.contains("lib.rs")
                && row.label.contains("src")
                && !row.label.contains("src/lib.rs")
                && row.trailing.contains('A')
        }));
        app.dispatch(Action::ToggleTreeMode);
        assert!(app.commit_tree_mode);
        assert!(app.commit_file_rows().iter().any(|row| row.id == "dir:src"));
    }

    #[test]
    fn t_from_dir_falls_back_to_checkout() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        app.dispatch(Action::ToggleTreeMode);
        let focused = app.focused_row().expect("row");
        assert!(
            focused.id == "repo:app" || focused.id == "checkout:app",
            "{}",
            focused.id
        );
    }

    fn type_search(app: &mut AppState, query: &str) {
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        for c in query.chars() {
            app.dispatch(Action::SearchChar(c));
        }
        app.dispatch(Action::SearchSubmit);
    }

    fn install_two_graph_commits(app: &mut AppState) {
        let model = GraphModel {
            commits: vec![
                graph_commit("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "alpha unique"),
                graph_commit("ccc3333dddddddddddddddddddddddddddddd", "beta unique"),
            ],
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            sync: None,
            show_ignored: app.show_ignored,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    #[test]
    fn graph_search_enter_focuses_subject_and_n_cycles() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_two_graph_commits(&mut app);
        type_search(&mut app, "unique");
        assert!(app.search_active);
        assert_eq!(app.search_target, SearchPane::Graph);
        assert_eq!(app.graph_cursor, 0);
        app.dispatch(Action::SearchNext);
        assert_eq!(app.graph_cursor, 1);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.graph_cursor, 0);
        app.dispatch(Action::SearchNext);
        app.dispatch(Action::SearchNext);
        assert_eq!(app.graph_cursor, 0);
    }

    #[test]
    fn graph_search_focuses_author_sha_ref_and_time() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        let now = unix_now();
        let mut hit = graph_commit("aa11bb22cc33dd44ee55ff6677889900aabbccdd", "alpha unique");
        hit.author_name = "Ada SearchAuthor".into();
        hit.author_date_unix = now - 90;
        hit.refs = vec![GraphRef::tag("v9.9.9"), GraphRef::local("topic-search")];
        let miss = graph_commit("ccc3333dddddddddddddddddddddddddddddd", "beta unique");
        let model = GraphModel {
            commits: vec![hit, miss],
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some("aa11bb22cc33dd44ee55ff6677889900aabbccdd".into()),
            sync: None,
            show_ignored: app.show_ignored,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aa11bb22cc33dd44ee55ff6677889900aabbccdd".into(),
        );
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;

        for query in ["SearchAuthor", "aa11bb2", "v9.9.9", "topic-search", "1m"] {
            app.graph_cursor = 1;
            type_search(&mut app, query);
            assert_eq!(
                app.graph_cursor, 0,
                "query {query} should focus the hit row"
            );
        }
    }

    #[test]
    fn commit_files_search_focuses_matching_path() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                },
                CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                },
            ],
        );
        type_search(&mut app, "lib");
        let row = app
            .commit_file_rows()
            .get(app.commit_files_cursor())
            .cloned()
            .expect("search row");
        assert_eq!(row.path, "src/lib.rs");
        assert_eq!(app.search_target, SearchPane::CommitFiles);
    }

    #[test]
    fn diff_search_sets_hit_and_scrolls() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec![
                "diff --git a/README.md b/README.md".into(),
                "@@ -1,3 +1,4 @@".into(),
                " context".into(),
                "+needle-unique".into(),
                " more".into(),
            ]),
        );
        type_search(&mut app, "needle-unique");
        assert_eq!(app.search_target, SearchPane::Diff);
        assert_eq!(app.search_hit, Some(3));
        assert!(app.diff_scroll <= 3);
    }

    #[test]
    fn tree_search_still_unfolds_parents() {
        let mut app = state();
        app.folds.insert("repo:app".into());
        app.rebuild_rows();
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
        type_search(&mut app, "README");
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
        assert_eq!(
            app.focused_row().map(|r| r.id.as_str()),
            Some("file:app:README.md")
        );
    }

    #[test]
    fn ctrl_o_toggles_unlimited_context_and_restores() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        assert_eq!(app.diff_context_lines(), None);
        assert!(!app.full_context_active());
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.diff_context_lines(), Some(FULL_DIFF_CONTEXT_LINES));
        assert!(app.full_context_active());
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.diff_context_lines(), None);
        assert!(!app.full_context_active());
    }

    #[test]
    fn ctrl_o_reload_keeps_hunk_text_in_view() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.layout.diff_pane_height = 8;
        let hunk_only = DiffContent::from_unified(
            "@@ -46,7 +46,7 @@\n pad-a\n pad-b\n pad-c\n-WIDE-HUNK-BASE\n+WIDE-HUNK-NEEDLE\n pad-d\n pad-e\n pad-f\n",
        );
        app.set_diff("app".into(), "README.md".into(), hunk_only);
        assert_eq!(app.diff_scroll, 0);
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        let mut full = String::from("@@ -1,40 +1,40 @@\n");
        for i in 0..30 {
            full.push_str(&format!(" HEAD-{i:02}\n"));
        }
        full.push_str(
            " pad-a\n pad-b\n pad-c\n-WIDE-HUNK-BASE\n+WIDE-HUNK-NEEDLE\n pad-d\n pad-e\n pad-f\n",
        );
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(full),
        );
        let rows = app.current_diff_rows();
        let start = app.diff_scroll as usize;
        let end = (start + 8).min(rows.len());
        let visible: Vec<String> = rows[start..end].iter().map(row_search_text).collect();
        assert!(
            visible
                .iter()
                .any(|t| t.contains("WIDE-HUNK-NEEDLE") || t.contains("WIDE-HUNK-BASE")),
            "hunk must stay in view, got scroll={start} visible={visible:?}"
        );
        assert!(
            visible.iter().all(|t| !t.contains("HEAD-00")),
            "file start must not replace the hunk, got scroll={start} visible={visible:?}"
        );
        assert!(app.diff_scroll > 0, "full-file must not stay at scroll 0");
    }

    #[test]
    fn ctrl_o_on_tree_or_graph_is_noop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Left;
        assert_eq!(app.dispatch(Action::ToggleFullContext), Effect::None);
        install_graph(&mut app, Vec::new());
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::ToggleFullContext), Effect::None);
    }

    #[test]
    fn set_diff_new_path_resets_viewport() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), tall_panning_diff());
        pan_and_scroll_focused_diff(&mut app);
        app.set_diff("app".into(), "src/lib.rs".into(), tall_panning_diff());
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.diff_scroll, 0);
        assert_eq!(app.diff_col_offset, 0);
    }

    #[test]
    fn open_commit_diff_new_path_resets_viewport() {
        let mut app = state();
        focus_repo(&mut app, "app");
        let files = two_commit_files();
        let source = sample_commit_source();
        app.open_commit_diff(
            "app".into(),
            source.clone(),
            files.clone(),
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        pan_and_scroll_focused_diff(&mut app);
        app.open_commit_diff(
            "app".into(),
            source,
            files,
            1,
            "two.rs".into(),
            tall_panning_diff(),
        );
        assert_eq!(app.diff_cursor, 0, "new commit file must drop the old row");
        assert_eq!(app.diff_scroll, 0, "new commit file must start at the top");
        assert_eq!(
            app.diff_col_offset, 0,
            "new commit file must start at the left"
        );
    }

    #[test]
    fn open_commit_diff_same_path_keeps_viewport() {
        let mut app = state();
        focus_repo(&mut app, "app");
        let files = two_commit_files();
        let source = sample_commit_source();
        app.open_commit_diff(
            "app".into(),
            source.clone(),
            files.clone(),
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        pan_and_scroll_focused_diff(&mut app);
        let cursor = app.diff_cursor;
        let scroll = app.diff_scroll;
        let pan = app.diff_col_offset;
        app.open_commit_diff(
            "app".into(),
            source,
            files,
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.diff_scroll, scroll);
        assert_eq!(app.diff_col_offset, pan);
    }

    #[test]
    fn workspace_then_commit_same_path_resets_viewport() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), tall_panning_diff());
        pan_and_scroll_focused_diff(&mut app);
        app.open_commit_diff(
            "app".into(),
            sample_commit_source(),
            sample_commit_files(),
            0,
            "README.md".into(),
            tall_panning_diff(),
        );
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.diff_scroll, 0);
        assert_eq!(app.diff_col_offset, 0);
    }

    #[test]
    fn focused_diff_l_increases_pan_h_decreases_tree_still_folds() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.layout.diff_pane_width = 8;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(format!("@@ -0,0 +1,1 @@\n+{}", "x".repeat(40))),
        );
        assert_eq!(app.diff_col_offset, 0);
        app.dispatch(Action::PanDiff(1));
        assert!(app.diff_col_offset > 0, "l increases pan offset");
        let after_l = app.diff_col_offset;
        app.dispatch(Action::PanDiff(-1));
        assert!(app.diff_col_offset < after_l, "h decreases pan offset");
        app.dispatch(Action::PanDiff(-1));
        app.dispatch(Action::PanDiff(-1));
        assert_eq!(app.diff_col_offset, 0);

        let mut tree = state();
        focus_repo(&mut tree, "app");
        tree.focus = FocusPane::Left;
        tree.dispatch(Action::FoldClose);
        assert!(tree.folds.contains("repo:app"));
        tree.dispatch(Action::FoldOpen);
        assert!(!tree.folds.contains("repo:app"));
    }

    #[test]
    fn mouse_hscroll_over_left_pane_pans_long_diff() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Left;
        app.layout.right_x = 48;
        app.layout.diff_pane_width = 8;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(format!("@@ -0,0 +1,1 @@\n+{}", "x".repeat(40))),
        );
        assert_eq!(app.diff_col_offset, 0);
        assert_eq!(
            app.dispatch(Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 5,
                horizontal: true,
            }),
            Effect::None
        );
        assert!(
            app.diff_col_offset > 0,
            "hscroll over the left pane must pan a long file diff"
        );
        assert_eq!(
            app.focus,
            FocusPane::Left,
            "hscroll must not steal tree focus"
        );
        assert_eq!(app.left_col_offset, 0, "short tree labels stay unpanned");
    }

    #[test]
    fn list_pan_shifts_tree_and_graph_without_stealing_fold() {
        let mut snap = repo("app", true);
        snap.changes = vec![FileChange {
            path: format!("deep/nested/{}/tail.rs", "x".repeat(40)),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        let mut tree = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_file(&mut tree, "tail.rs");
        tree.focus = FocusPane::Left;
        tree.layout.tree_width = 16;
        assert_eq!(tree.left_col_offset, 0);
        tree.dispatch(Action::PanDiff(4));
        assert!(
            tree.left_col_offset > 0,
            "Shift/pan should reveal a long tree path"
        );
        let panned = tree.left_col_offset;
        tree.dispatch(Action::PanDiff(-20));
        assert_eq!(tree.left_col_offset, 0);
        focus_repo(&mut tree, "app");
        tree.dispatch(Action::FoldClose);
        assert!(tree.folds.contains("repo:app"));
        tree.dispatch(Action::FoldOpen);
        assert!(!tree.folds.contains("repo:app"));
        assert_eq!(tree.left_col_offset, 0);
        tree.dispatch(Action::PanDiff(1));
        assert!(panned > 0);

        let mut graph = state();
        focus_repo(&mut graph, "app");
        let model = GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: format!("subject-{}", "y".repeat(80)),
                parents: Vec::new(),
                refs: Vec::new(),
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: None,
            ..GraphModel::default()
        };
        graph.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        graph.focus = FocusPane::Right;
        graph.drill = DrillView::Graph;
        graph.layout.diff_pane_width = 20;
        graph.dispatch(Action::PanDiff(5));
        assert!(
            graph.right_col_offset > 0,
            "h/l on a focused graph should pan a long subject"
        );
    }

    #[test]
    fn tree_mouse_hscroll_pans_without_moving_cursor() {
        let mut snap = repo("app", true);
        snap.changes = vec![FileChange {
            path: format!("deep/nested/{}/tail.rs", "x".repeat(40)),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_file(&mut app, "tail.rs");
        app.focus = FocusPane::Left;
        app.layout.tree_width = 16;
        app.layout.right_x = 40;
        let cursor = app.cursor;
        let id = app.rows[cursor].id.clone();
        assert_eq!(app.left_col_offset, 0);

        assert_eq!(
            app.dispatch(Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 4,
                horizontal: true,
            }),
            Effect::None
        );
        assert_eq!(app.cursor, cursor, "hscroll must not move the tree cursor");
        assert_eq!(app.rows[app.cursor].id, id);
        assert_eq!(app.focus, FocusPane::Left);
        assert!(
            app.left_col_offset > 0,
            "tree mouse hscroll should pan a long path"
        );

        app.focus = FocusPane::Right;
        let panned = app.left_col_offset;
        app.dispatch(Action::ScrollWheel {
            col: 8,
            row: 4,
            delta: 2,
            horizontal: true,
        });
        assert_eq!(app.cursor, cursor);
        assert_eq!(
            app.focus,
            FocusPane::Right,
            "hscroll must not steal pane focus"
        );
        assert!(
            app.left_col_offset >= panned,
            "hscroll over the tree pans the tree even when the right pane is focused"
        );

        app.focus = FocusPane::Left;
        let after_pan = app.cursor;
        app.dispatch(Action::ScrollWheel {
            col: 8,
            row: 4,
            delta: -1,
            horizontal: false,
        });
        assert_ne!(
            app.cursor, after_pan,
            "vertical wheel over the tree still moves the cursor"
        );

        let mut fold = AppState::new(
            PathBuf::from("/tmp"),
            build_workspace_snapshot(&[repo("app", true)], &[], false, &[]),
            true,
        );
        focus_repo(&mut fold, "app");
        fold.dispatch(Action::FoldClose);
        assert!(fold.folds.contains("repo:app"));
        fold.dispatch(Action::FoldOpen);
        assert!(!fold.folds.contains("repo:app"));
    }

    #[test]
    fn full_context_survives_tree_focus_and_commit_file_list() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        assert_eq!(
            app.dispatch(Action::ToggleFullContext),
            Effect::LoadRightPane
        );
        app.focus = FocusPane::Left;
        assert_eq!(app.diff_context_lines(), Some(FULL_DIFF_CONTEXT_LINES));
        assert_eq!(
            app.workspace_diff_context("app", "README.md"),
            Some(FULL_DIFF_CONTEXT_LINES)
        );
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.diff_context_lines(), None);

        let mut commit = state();
        focus_repo(&mut commit, "app");
        commit.focus = FocusPane::Right;
        commit.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
            }],
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        match commit.dispatch(Action::ToggleFullContext) {
            Effect::LoadCommitDiff { path, .. } => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
        assert_eq!(commit.diff_context_lines(), Some(FULL_DIFF_CONTEXT_LINES));
        commit.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
            }],
        );
        assert_eq!(commit.diff_context_lines(), None);
        assert_eq!(
            commit.commit_diff_context("app", "README.md"),
            Some(FULL_DIFF_CONTEXT_LINES)
        );
    }

    #[test]
    fn help_then_slash_starts_help_search_not_pane_search() {
        let mut app = state();
        app.dispatch(Action::ToggleHelp);
        assert!(app.help_open);
        assert_eq!(app.input_mode(), InputMode::Help);
        assert_eq!(
            crate::tui::keys::event_to_action(
                &crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('/'),
                    crossterm::event::KeyModifiers::NONE,
                )),
                app.input_mode(),
                false,
                false,
            ),
            Action::SearchStart
        );
        app.dispatch(Action::SearchStart);
        assert!(app.help_search_query.is_some());
        assert!(!app.search_mode);
        assert!(!app.search_active);
        assert_eq!(app.input_mode(), InputMode::HelpSearch);
        app.dispatch(Action::SearchChar('q'));
        app.dispatch(Action::SearchChar('u'));
        app.dispatch(Action::SearchChar('i'));
        assert_eq!(app.help_search_query.as_deref(), Some("qui"));
        assert!(!app.search_mode);
        app.dispatch(Action::SearchChar('n'));
        assert_eq!(app.help_search_query.as_deref(), Some("quin"));
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.help_search_query.as_deref(), Some("quin"));
        app.dispatch(Action::SearchNext);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.help_search_query.as_deref(), Some("quin"));
        app.dispatch(Action::SearchCancel);
        assert!(app.help_open);
        assert!(app.help_search_query.is_none());
    }

    #[test]
    fn slash_without_help_still_starts_pane_search() {
        let mut app = state();
        assert!(!app.help_open);
        app.dispatch(Action::SearchStart);
        assert!(app.search_mode);
        assert!(app.help_search_query.is_none());
        app.dispatch(Action::SearchChar('R'));
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_active);
        assert!(!app.search_mode);
    }

    #[test]
    fn enter_keeps_graph_and_shows_commit_meta() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        if let Some(model) = app.graph.as_mut() {
            model.commits[0].author_name = "Ada".into();
            model.commits[0].author_date_unix = unix_now() - 3600;
            model.commits[0].refs = vec![GraphRef::local("main")];
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
            }],
        );
        assert!(app.graph.is_some());
        assert!(app.in_commit_drill());
        let (title, subtitle) = app.commit_detail_meta();
        assert_eq!(title, "app");
        let subtitle = subtitle.expect("subtitle");
        assert!(subtitle.contains("aaa1111"), "{subtitle}");
        assert!(subtitle.contains("main"), "{subtitle}");
        assert!(subtitle.contains("head"), "{subtitle}");
        assert!(subtitle.contains("Ada"), "{subtitle}");
        assert!(
            subtitle.contains("1h") || subtitle.contains("59m") || subtitle.contains("just now"),
            "{subtitle}"
        );
        assert!(app
            .commit_file_rows()
            .iter()
            .any(|row| row.path == "src/lib.rs"));
    }

    #[test]
    fn e_on_commit_file_and_diff_emits_edit() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
            }],
        );
        let idx = app
            .commit_file_rows()
            .iter()
            .position(|row| row.path == "src/lib.rs")
            .expect("file");
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = idx;
        }
        match app.dispatch(Action::Edit) {
            Effect::EditFile { repo, path } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "src/lib.rs");
            }
            other => panic!("{other:?}"),
        }
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
            }],
            idx,
            "src/lib.rs".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        match app.dispatch(Action::Edit) {
            Effect::EditFile { repo, path } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "src/lib.rs");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn workspace_t_unchanged_outside_commit_drill() {
        let mut app = tree_app();
        assert!(app.tree_mode);
        assert!(app.commit_tree_mode);
        app.dispatch(Action::ToggleTreeMode);
        assert!(!app.tree_mode);
        assert!(app.commit_tree_mode);
        assert!(app.drill.is_graph());
    }
    #[test]
    fn single_z_folds_only_focused_row() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("dir:app:src"));
        assert!(!app.folds.contains("repo:app"));
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn zz_second_z_is_toggle_subtree() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("repo:app"));
        assert!(!app.folds.contains("dir:app:src"));
        app.dispatch(Action::FoldToggleSubtree);
        assert!(
            app.folds.contains("repo:app"),
            "second z must not toggle the parent a second time"
        );
        assert!(
            app.folds.contains("dir:app:src"),
            "second z folds descendants to match the folded parent"
        );
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
    }

    #[test]
    fn zz_on_a_folded_parent_opens_the_subtree() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        app.dispatch(Action::FoldToggle);
        app.dispatch(Action::FoldToggleSubtree);
        assert!(app.folds.contains("repo:app"));
        assert!(app.folds.contains("dir:app:src"));
        app.dispatch(Action::FoldToggle);
        assert!(!app.folds.contains("repo:app"));
        assert!(app.folds.contains("dir:app:src"));
        app.dispatch(Action::FoldToggleSubtree);
        assert!(!app.folds.contains("repo:app"));
        assert!(
            !app.folds.contains("dir:app:src"),
            "second z opens descendants to match the open parent"
        );
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn fold_toggle_subtree_does_not_toggle_the_focused_row() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        app.dispatch(Action::FoldToggleSubtree);
        assert!(
            !app.folds.contains("repo:app"),
            "FoldToggleSubtree must not toggle the parent (first z already did)"
        );
        assert!(!app.folds.contains("dir:app:src"));
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
    }

    #[test]
    fn expired_z_is_a_new_single_toggle() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("dir:app:src"));
        app.z_pending_at = Some(Instant::now() - Duration::from_millis(500));
        app.dispatch(Action::FoldToggle);
        assert!(!app.folds.contains("dir:app:src"));
        assert!(!app.folds.contains("repo:app"));
    }

    #[test]
    fn gg_chord_arms_without_moving_then_moves_to_start() {
        let mut app = tree_app();
        app.cursor = 3.min(app.rows.len().saturating_sub(1));
        let before = app.cursor;
        app.dispatch(Action::ArmGChord);
        assert_eq!(app.cursor, before);
        assert!(app.g_pending_at.is_some());
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.cursor, 0);
        assert!(app.g_pending_at.is_none());
    }

    #[test]
    fn gg_g_on_focused_diff_scroll_to_edges() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let mut lines = vec!["@@ -1,1 +1,40 @@".into()];
        lines.extend((0..40).map(|i| format!("+line {i}")));
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(lines),
        );
        app.focus = FocusPane::Right;
        app.layout.diff_pane_height = 8;
        assert_eq!(app.list_focus_target(), ListFocusTarget::None);
        let view_h = app.diff_body_height();
        let past_mid = (view_h / 2 + 2) as i32;
        app.dispatch(Action::Move(past_mid));
        let mid = app.diff_scroll;
        assert!(
            mid > 0,
            "Move past the midpoint should leave the top, scroll={mid} cursor={}",
            app.diff_cursor
        );
        app.dispatch(Action::MoveToEnd);
        assert!(
            app.diff_scroll > mid,
            "MoveToEnd must scroll a focused diff, mid={mid} after={}",
            app.diff_scroll
        );
        assert_eq!(app.focus, FocusPane::Right);
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.diff_scroll, 0);
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.focus, FocusPane::Right);
    }

    #[test]
    fn focused_diff_j_past_midpoint_keeps_focus_near_middle() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let mut lines = vec!["@@ -1,1 +1,40 @@".into()];
        lines.extend((0..40).map(|i| format!("+line {i}")));
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(lines),
        );
        app.focus = FocusPane::Right;
        app.layout.diff_pane_height = 8;
        let n = app.current_diff_rows().len();
        let view_h = app.diff_body_height();
        assert_eq!(view_h, 7);
        let steps = view_h / 2 + 3;
        app.dispatch(Action::Move(steps as i32));
        assert_eq!(app.diff_cursor, steps);
        assert_eq!(
            app.diff_scroll as usize,
            list_viewport_start(n, app.diff_cursor, view_h)
        );
        let offset = app.diff_cursor - app.diff_scroll as usize;
        assert_eq!(
            offset,
            view_h / 2,
            "focused diff row should sit at the vertical middle, offset={offset} mid={}",
            view_h / 2
        );
        assert_ne!(
            offset,
            view_h.saturating_sub(1),
            "keep-middle must not leave the focused row on the last viewport line"
        );
    }

    #[test]
    fn gg_g_on_left_tree_with_file_diff_shown_moves_tree() {
        let mut app = tree_app();
        focus_file(&mut app, "README.md");
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["+line".into(); 20]),
        );
        app.focus = FocusPane::Left;
        app.diff_scroll = 4;
        app.cursor = 2.min(app.rows.len().saturating_sub(1));
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.cursor, 0);
        assert_eq!(
            app.diff_scroll, 4,
            "left gg must not scroll an unfocused file diff"
        );
        assert_eq!(app.focus, FocusPane::Left);
        app.dispatch(Action::MoveToEnd);
        assert_eq!(app.cursor, app.rows.len() - 1);
        assert_eq!(
            app.diff_scroll, 4,
            "left G must not scroll an unfocused file diff"
        );
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn ctrl_d_moves_five_and_pgdn_uses_page_height() {
        let mut app = tree_app();
        assert!(app.rows.len() >= 5, "need a list long enough for +5");
        app.cursor = 0;
        app.layout.tree_height = 2;
        app.dispatch(Action::Move(5));
        assert_eq!(app.cursor, 5.min(app.rows.len() - 1));
        app.cursor = 0;
        app.dispatch(Action::PageMove(1));
        assert_eq!(app.cursor, 1);
        app.cursor = 4;
        app.dispatch(Action::Move(-5));
        assert_eq!(app.cursor, 0);
    }

    fn install_linear_graph(app: &mut AppState, n: usize) {
        let commits: Vec<Commit> = (0..n)
            .map(|i| {
                let id = format!("c{i:02}{}", "a".repeat(36));
                let parents = if i + 1 < n {
                    vec![format!("c{:02}{}", i + 1, "a".repeat(36))]
                } else {
                    Vec::new()
                };
                Commit {
                    id,
                    subject: format!("commit {i}"),
                    parents,
                    refs: Vec::new(),
                    author_name: String::new(),
                    author_date_unix: 0,
                }
            })
            .collect();
        let head = commits[0].id.clone();
        let model = GraphModel {
            commits,
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some(head.clone()),
            sync: None,
            show_ignored: app.show_ignored,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(model, "app".into(), head);
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    fn painted_focus_index(app: &AppState) -> usize {
        let model = app.graph.as_ref().expect("graph");
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        painted
            .iter()
            .position(|line| line.row_index == Some(app.graph_cursor) && line.selectable)
            .expect("focused painted row")
    }

    #[test]
    fn graph_pgdn_pages_painted_lines_and_keeps_focus_in_viewport() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        // height 12, no sync header → list_height 10, page = 9 painted lines
        app.layout.tree_height = 12;
        let list_h = app.graph_chrome().list_height.max(1) as usize;
        assert_eq!(list_h, 10, "list_height from chrome budget");
        let page = list_h.saturating_sub(1).max(1);
        app.graph_cursor = 0;
        app.sync_graph_scroll();
        let before = painted_focus_index(&app);
        let selectable = app.graph.as_ref().unwrap().visible_rows().len();
        app.dispatch(Action::PageMove(1));
        let after = painted_focus_index(&app);
        let scroll = app.graph_scroll as usize;
        assert!(
            after >= scroll && after < scroll + list_h,
            "focused painted row {after} must stay in [{scroll}, {})",
            scroll + list_h
        );
        assert!(
            after >= page.saturating_sub(1) && after <= page + 1,
            "PageDown should move about one painted viewport ({page}), got painted {before} → {after}"
        );
        assert!(
            app.graph_cursor < page.min(selectable),
            "must not apply painted page ({page}) to selectable indices; cursor={}",
            app.graph_cursor
        );

        app.dispatch(Action::MoveToEnd);
        let end = painted_focus_index(&app);
        let end_scroll = app.graph_scroll as usize;
        assert!(
            end >= end_scroll && end < end_scroll + list_h,
            "End should sync scroll: painted {end} in [{end_scroll}, {})",
            end_scroll + list_h
        );
        assert!(end_scroll > 0, "tall list End should scroll");
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.graph_cursor, 0);
        assert_eq!(app.graph_scroll, 0);
    }

    #[test]
    fn graph_j_past_midpoint_keeps_focus_near_middle() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 40);
        app.layout.tree_height = 12;
        let list_h = app.graph_chrome().list_height.max(1) as usize;
        assert_eq!(list_h, 10, "list_height from chrome budget");
        app.graph_cursor = 0;
        app.sync_graph_scroll();
        let steps = list_h / 2 + 4;
        app.dispatch(Action::Move(steps as i32));
        let idx = painted_focus_index(&app);
        let scroll = app.graph_scroll as usize;
        let offset = idx.saturating_sub(scroll);
        let mid = list_h / 2;
        assert!(
            (offset as i32 - mid as i32).abs() <= 1,
            "focused painted row should sit near the middle: offset={offset} mid={mid} idx={idx} scroll={scroll} list_h={list_h}"
        );
        assert_ne!(
            offset,
            list_h.saturating_sub(1),
            "keep-middle must not leave the focused row on the last viewport line"
        );
    }

    #[test]
    fn graph_scrollbar_drag_does_not_recenter_until_focus_moves() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 40);
        app.layout.tree_height = 12;
        app.dispatch(Action::MoveToEnd);
        let cursor = app.graph_cursor;
        let scrolled = app.graph_scroll;
        assert!(scrolled > 0, "End should leave the top of the graph");
        app.graph_scroll = 0;
        assert_eq!(
            app.graph_cursor, cursor,
            "viewport-only scroll must not move the graph cursor"
        );
        assert_eq!(
            app.graph_scroll, 0,
            "scrollbar drag must keep an independent viewport until the focused row moves"
        );
        app.dispatch(Action::Move(0));
        assert_eq!(app.graph_cursor, cursor);
        assert_eq!(
            app.graph_scroll, scrolled,
            "the next focused-row move recentres the viewport"
        );
    }

    #[test]
    fn double_click_dispatches_enter() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        let idx = app
            .rows
            .iter()
            .position(|r| r.id == "repo:app")
            .expect("repo");
        let row = app.layout.tree_y + idx as u16;
        // click on the label, not the chevron (depth 1 => chevron at x=2)
        let col = 8;
        app.dispatch(Action::Click { col, row });
        assert_eq!(app.cursor, idx);
        assert_eq!(app.focus, FocusPane::Left);
        let effect = app.dispatch(Action::Click { col, row });
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn chevron_click_toggles_fold() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        let idx = app
            .rows
            .iter()
            .position(|r| r.id == "dir:app:src")
            .expect("dir");
        let depth = app.rows[idx].depth;
        let col = app.layout.tree_x + 1 + (depth as u16) * 2;
        let row = app.layout.tree_y + idx as u16;
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
        app.dispatch(Action::Click { col, row });
        assert!(app.folds.contains("dir:app:src"));
        assert_eq!(
            app.cursor,
            app.rows.iter().position(|r| r.id == "dir:app:src").unwrap()
        );
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
    }

    #[test]
    fn double_click_chevron_folds_once_and_does_not_enter() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        let idx = app
            .rows
            .iter()
            .position(|r| r.id == "dir:app:src")
            .expect("dir");
        let depth = app.rows[idx].depth;
        let col = app.layout.tree_x + 1 + (depth as u16) * 2;
        let row = app.layout.tree_y + idx as u16;
        app.dispatch(Action::Click { col, row });
        assert!(app.folds.contains("dir:app:src"));
        assert_eq!(app.focus, FocusPane::Left);
        let effect = app.dispatch(Action::Click { col, row });
        assert!(
            app.folds.contains("dir:app:src"),
            "second Down must not undo the fold"
        );
        assert_eq!(
            app.focus,
            FocusPane::Left,
            "chevron double-click is not Enter"
        );
        assert_eq!(effect, Effect::None);
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
    }

    #[test]
    fn m_toggles_mouse_and_ignores_clicks_when_off() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        assert!(app.mouse_enabled);
        app.dispatch(Action::ToggleMouse);
        assert!(!app.mouse_enabled);
        assert!(app.status.contains("off"));
        let start = app.cursor;
        app.dispatch(Action::Click {
            col: 8,
            row: app.layout.tree_y,
        });
        assert_eq!(app.cursor, start);
        app.dispatch(Action::ToggleMouse);
        assert!(app.mouse_enabled);
        assert!(app.status.contains("on"));
    }
}

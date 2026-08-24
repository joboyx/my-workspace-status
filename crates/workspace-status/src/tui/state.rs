//! TUI state and Action dispatch.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use workspace_status_graph::{
    format_relative_date, graph_chrome_budget, paint_model, GraphChromeBudget, GraphModel,
    GraphRow, PaintedLine, ASCII, UNICODE,
};

use crate::snapshot::{FileChange, WorkspaceSnapshot};

use super::action::{Action, Effect};
use super::branches::{
    can_open_branch_picker, checkoutable_branch_names, is_valid_branch_name, merge_rev_for_commit,
    BranchPickerState, CreateBranchState, DIRTY_WORKTREE_STATUS,
};
use super::commit_files::{
    ancestor_dir_ids, collect_foldable_subtree_ids as collect_commit_subtree_ids,
    flatten_commit_files, CommitFileRow,
};
use super::ctrl_c_exit::{handle_ctrl_c, is_ctrl_c_exit_prompt, CTRL_C_EXIT_PROMPT};
use super::diff::{
    anchor_row_index, build_diff_rows, cell_code_width, clamp_diff_scroll, gutter_width,
    row_search_text, scroll_to_keep_row, DiffContent, DiffRow,
};
use super::drill::{
    source_from_graph_row, stash_ref_from_graph_row, CommitFile, CommitFileSource, DrillView,
};
use super::easy_motion::{resolve_easy_motion_jump, visible_window, EasyMotionResolve};
use super::fetch::background_fetch_targets;
use super::gates::{dispatch_is_noop, ListFocusTarget};
use super::keys::{InputMode, DOUBLE_TAP_MS};
use super::ops::{
    collect_write_files, format_running_op, op_is_kind_noop, op_targets, push_targets,
    refresh_target, should_delete_untracked, Op, RunningOp, ScopedFile,
};
use super::search::{
    apply_pan, focus_commit_file_search, focus_diff_search, focus_graph_search, focus_tree_search,
    max_col_offset, SearchPane,
};
use super::split::{
    clamp_tree_fraction, diff_split_fraction_from_col, effective_diff_mode, hit_split,
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
    workspace_label_from_cwd, NodeKind, TreeNode, VisibleRow,
};
#[cfg(not(test))]
use super::viewed::viewed_store_path;
use super::viewed::{
    collect_current_fingerprints, fingerprint_file_change, is_viewed, load_viewed_store,
    reconcile_viewed, save_viewed_store, toggle_viewed, viewed_identity, viewed_row_ids,
    ViewedStore,
};
use super::watch::{changed_row_ids, tree_signatures};
use crate::git::FULL_DIFF_CONTEXT_LINES;
use crate::snapshot::CheckoutKind;

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
        }
    }
}

/// Armed EasyMotion overlay (typed prefix so far).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EasyMotion {
    pub typed: String,
}

/// Which focused list EasyMotion labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum EasyMotionList {
    Tree,
    Graph,
    CommitFiles,
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
    pub diff_repo: Option<String>,
    pub diff_path: Option<String>,
    pub reviewed: HashSet<String>,
    pub viewed_store: ViewedStore,
    pub viewed_path: PathBuf,
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
    /// File identities currently shown with unlimited `-U` context.
    pub full_context: HashSet<String>,
    pending_hunk_anchor: Option<usize>,
    pub confirm: Option<PendingConfirm>,
    pub stash_menu: Option<Vec<StashOp>>,
    pub stash_repo: Option<String>,
    pub branch_picker: Option<BranchPickerState>,
    pub create_branch: Option<CreateBranchState>,
    pub flashes: HashMap<String, Instant>,
    pub signatures: BTreeMap<String, String>,
    pub tree_fraction: f64,
    pub diff_split_fraction: f64,
    pub diff_mode: DiffMode,
    pub drag: SplitDrag,
    pub easy_motion: Option<EasyMotion>,
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
            diff_repo: None,
            diff_path: None,
            reviewed: HashSet::new(),
            viewed_store,
            viewed_path,
            layout: LayoutHit::default(),
            ascii,
            search_mode: false,
            search_active: false,
            search_query: String::new(),
            search_target: SearchPane::Tree,
            search_hit: None,
            diff_col_offset: 0,
            full_context: HashSet::new(),
            pending_hunk_anchor: None,
            confirm: None,
            stash_menu: None,
            stash_repo: None,
            branch_picker: None,
            create_branch: None,
            flashes: HashMap::new(),
            signatures,
            tree_fraction: TREE_WIDTH_FRACTION,
            diff_split_fraction: DIFF_SPLIT_FRACTION,
            diff_mode: DiffMode::SideBySide,
            drag: SplitDrag::None,
            easy_motion: None,
            theme: theme_from_env(),
            mouse_enabled: true,
            z_pending_at: None,
            g_pending_at: None,
            ctrl_c_armed_until: None,
            last_click: None,
        };
        state.reconcile_viewed_store();
        state
    }

    pub fn input_mode(&self) -> InputMode {
        if self.confirm.is_some() {
            InputMode::Confirm
        } else if self.stash_menu.is_some() {
            InputMode::StashMenu
        } else if self.create_branch.is_some() {
            InputMode::CreateBranch
        } else if self.branch_picker.is_some() {
            InputMode::BranchPicker
        } else if self.help_open {
            if self.help_search_query.is_some() {
                InputMode::HelpSearch
            } else {
                InputMode::Help
            }
        } else if self.search_mode {
            InputMode::SearchPrompt
        } else if self.easy_motion.is_some() {
            InputMode::EasyMotion
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

    /// Which list (or diff) the focused pane is driving.
    ///
    /// Depth 0 left is the workspace tree; depth 1 left is the graph; depth 2
    /// left is the commit-file list. Right at depth 2 (and depth 0 file diffs)
    /// is `None` so `j`/`k` scroll the diff.
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

    fn maybe_load_focused_commit_diff(&self) -> Effect {
        let DrillView::Diff {
            repo, source, path, ..
        } = &self.drill
        else {
            return Effect::None;
        };
        let Some(row) = self.focused_commit_file_row() else {
            return Effect::None;
        };
        if !row.is_file() || row.path == *path {
            return Effect::None;
        }
        Effect::LoadCommitDiff {
            repo: repo.clone(),
            source: source.clone(),
            path: row.path.clone(),
        }
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

    fn focused_commit_file_row(&self) -> Option<CommitFileRow> {
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
        self.snapshot = snapshot;
        self.snapshot.show_ignored = self.show_ignored;
        self.rebuild_rows();
        self.signatures = tree_signatures(&self.tree, &self.cwd);
        self.reconcile_viewed_store();
    }

    /// Apply a watch poll. Keeps fold / focus / scroll. Flashes only rows
    /// whose identity actually changed.
    pub fn apply_watch_snapshot(&mut self, snapshot: WorkspaceSnapshot) -> Vec<String> {
        let focus_id = self.focused_row().map(|r| r.id.clone());
        let folds = self.folds.clone();
        let graph_scroll = self.graph_scroll;
        let diff_scroll = self.diff_scroll;
        let before = self.signatures.clone();
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
        let changed = changed_row_ids(&before, &self.signatures);
        let now = Instant::now();
        self.flashes
            .retain(|_, at| now.duration_since(*at).as_millis() < 800);
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
                self.pan_diff(delta);
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
            Action::ScrollWheel { col, row: _, delta } => {
                if !self.mouse_enabled {
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
                    Effect::None
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
            Action::EasyMotionStart => self.start_easy_motion(),
            Action::EasyMotionChar(c) => self.easy_motion_char(c),
            Action::EasyMotionCancel => self.cancel_easy_motion(),
            Action::CycleTheme => self.cycle_theme(),
            Action::Resize { cols, rows: _ } => {
                self.apply_terminal_size(cols);
                Effect::None
            }
            Action::None => Effect::None,
        }
    }

    fn start_easy_motion(&mut self) -> Effect {
        if self.easy_motion_list().is_none() {
            return Effect::None;
        }
        self.drag = SplitDrag::None;
        self.easy_motion = Some(EasyMotion {
            typed: String::new(),
        });
        self.status = "EasyMotion".into();
        Effect::None
    }

    fn easy_motion_char(&mut self, c: char) -> Effect {
        let Some(current) = self.easy_motion.clone() else {
            return Effect::None;
        };
        let Some((start, count, list)) = self.easy_motion_window() else {
            return self.cancel_easy_motion();
        };
        let typed = format!("{}{c}", current.typed);
        match resolve_easy_motion_jump(count, start, &typed) {
            EasyMotionResolve::Miss => self.cancel_easy_motion(),
            EasyMotionResolve::Partial => {
                if let Some(motion) = self.easy_motion.as_mut() {
                    motion.typed = typed.clone();
                }
                self.status = format!("EasyMotion {typed}");
                Effect::None
            }
            EasyMotionResolve::Hit { index } => {
                self.easy_motion = None;
                self.status.clear();
                self.jump_easy_motion(list, index)
            }
        }
    }

    fn cancel_easy_motion(&mut self) -> Effect {
        self.easy_motion = None;
        self.status.clear();
        Effect::None
    }

    fn cycle_theme(&mut self) -> Effect {
        self.theme = cycle_theme_id(self.theme);
        self.status = format!("theme: {}", self.theme.label());
        Effect::None
    }

    fn easy_motion_list(&self) -> Option<EasyMotionList> {
        match self.list_focus_target() {
            ListFocusTarget::Tree => Some(EasyMotionList::Tree),
            ListFocusTarget::Graph => Some(EasyMotionList::Graph),
            ListFocusTarget::CommitFiles => Some(EasyMotionList::CommitFiles),
            ListFocusTarget::None => None,
        }
    }

    fn easy_motion_window(&self) -> Option<(usize, usize, EasyMotionList)> {
        let list = self.easy_motion_list()?;
        let height = self.layout.tree_height.max(1) as usize;
        let (start, count) = match list {
            EasyMotionList::Tree => visible_window(self.rows.len(), self.cursor, height),
            EasyMotionList::Graph => {
                let n = self
                    .graph
                    .as_ref()
                    .map(|g| g.visible_rows().len())
                    .unwrap_or(0);
                let list_h = self.graph_chrome().list_height.max(1) as usize;
                visible_window(n, self.graph_cursor, list_h)
            }
            EasyMotionList::CommitFiles => {
                let rows = self.commit_file_rows();
                visible_window(rows.len(), self.commit_files_cursor(), height)
            }
        };
        Some((start, count, list))
    }

    fn jump_easy_motion(&mut self, list: EasyMotionList, index: usize) -> Effect {
        match list {
            EasyMotionList::Tree => {
                if self.rows.is_empty() {
                    return Effect::None;
                }
                self.cursor = index.min(self.rows.len() - 1);
                self.drill = DrillView::Graph;
                Effect::LoadRightPane
            }
            EasyMotionList::Graph => {
                let n = self
                    .graph
                    .as_ref()
                    .map(|g| g.visible_rows().len())
                    .unwrap_or(0);
                if n == 0 {
                    return Effect::None;
                }
                self.graph_cursor = index.min(n - 1);
                self.sync_graph_scroll();
                Effect::None
            }
            EasyMotionList::CommitFiles => {
                let n = self.commit_file_rows().len();
                if n == 0 {
                    return Effect::None;
                }
                self.set_commit_file_cursor(index.min(n - 1));
                self.maybe_load_focused_commit_diff()
            }
        }
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
            let opening = self.commit_file_folds.contains(&id);
            for sid in ids {
                if opening {
                    self.commit_file_folds.remove(&sid);
                } else {
                    self.commit_file_folds.insert(sid);
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
        let opening = self.folds.contains(&row.id);
        for sid in ids {
            if opening {
                self.folds.remove(&sid);
            } else {
                self.folds.insert(sid);
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

    fn scroll_right(&mut self, delta: i32) {
        if self.right_is_diff() || self.drill.is_diff() {
            let max = self.diff_scroll_max() as i32;
            let next = self.diff_scroll as i32 + delta;
            self.diff_scroll = next.clamp(0, max) as u16;
        } else {
            let next = self.graph_scroll as i32 + delta;
            self.graph_scroll = next.max(0) as u16;
        }
    }

    fn diff_body_height(&self) -> usize {
        self.layout.diff_pane_height.saturating_sub(1).max(1) as usize
    }

    fn diff_scroll_max(&self) -> usize {
        clamp_diff_scroll(
            usize::MAX,
            self.current_diff_rows().len(),
            self.diff_body_height(),
        )
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
                return self.click_commit_files(col, row, is_double);
            }
            if !self.right_is_diff() && self.graph.is_some() {
                return self.click_graph(row, is_double, true);
            }
            if is_double {
                return self.nav_enter();
            }
            return Effect::None;
        }
        self.focus = FocusPane::Left;
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
        if idx >= self.rows.len() {
            return Effect::None;
        }
        let tree_row = self.rows[idx].clone();
        self.cursor = idx;
        if tree_row.foldable && self.is_tree_chevron(col, tree_row.depth) {
            self.fold_op(FoldOp::Toggle);
            return Effect::LoadRightPane;
        }
        if is_double {
            return self.nav_enter();
        }
        Effect::LoadRightPane
    }

    fn is_tree_chevron(&self, col: u16, depth: usize) -> bool {
        let prefix = if self.easy_motion.is_some() { 2 } else { 0 };
        col == self.layout.tree_x + prefix + 1 + (depth as u16) * 2
    }

    fn is_files_chevron(&self, col: u16, depth: usize) -> bool {
        let prefix = if self.easy_motion.is_some() { 2 } else { 0 };
        col == self.files_list_origin_x() + prefix + 1 + (depth as u16) * 2
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
            if is_double {
                return self.nav_enter();
            }
        }
        Effect::None
    }

    fn click_commit_files(&mut self, col: u16, row: u16, is_double: bool) -> Effect {
        if row < self.layout.files_list_y {
            return Effect::None;
        }
        let idx = self.layout.files_list_offset + (row - self.layout.files_list_y) as usize;
        let n = self.commit_file_rows().len();
        if idx >= n {
            return Effect::None;
        }
        self.set_commit_file_cursor(idx);
        let Some(file_row) = self.commit_file_rows().get(idx).cloned() else {
            return Effect::None;
        };
        if file_row.foldable && self.is_files_chevron(col, file_row.depth) {
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

    fn drag_split(&mut self, col: u16, _row: u16) -> Effect {
        match self.drag {
            SplitDrag::Pane => self.apply_tree_fraction_from_col(col),
            SplitDrag::Diff => self.apply_diff_fraction_from_col(col),
            SplitDrag::None => {}
        }
        Effect::None
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
        let identity = (repo, head);
        if self.graph_identity.as_ref() != Some(&identity) {
            self.graph_scroll = 0;
            self.graph_cursor = 0;
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
        let same = self.diff_repo.as_deref() == Some(repo.as_str())
            && self.diff_path.as_deref() == Some(path.as_str());
        if !same {
            self.diff_scroll = 0;
            self.diff_col_offset = 0;
        }
        self.diff_repo = Some(repo);
        self.diff_path = Some(path);
        self.diff_content = content;
        self.apply_pending_hunk_anchor();
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
    }

    /// Open the commit-files drill before git returns.
    ///
    /// Paint uses `loading files…` while `commit_files_loading` is true and
    /// the list is empty.
    pub fn begin_commit_files(&mut self, repo: String, source: CommitFileSource) {
        self.commit_file_folds.clear();
        self.commit_files_loading = true;
        self.drill = DrillView::Files {
            repo,
            source,
            files: Vec::new(),
            cursor: 0,
        };
        self.focus = FocusPane::Right;
    }

    pub fn open_commit_files(
        &mut self,
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
    ) {
        self.commit_file_folds.clear();
        self.commit_files_loading = false;
        self.status = format!("files {}", files.len());
        let cursor = DrillView::files_cursor(&files, 0);
        self.drill = DrillView::Files {
            repo,
            source,
            files,
            cursor,
        };
        self.focus = FocusPane::Right;
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
        self.diff_scroll = 0;
        self.diff_col_offset = 0;
        self.status = format!("diff {path}");
        self.drill = DrillView::Diff {
            repo,
            source,
            files,
            file_cursor,
            path,
            content,
        };
        self.apply_pending_hunk_anchor();
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
                Effect::None
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
        self.diff_scroll = scroll_to_keep_row(idx, self.diff_body_height(), rows.len());
        self.set_search_status(true);
    }

    fn pan_diff(&mut self, delta: i32) {
        if self.focus != FocusPane::Right || !self.right_is_diff() {
            return;
        }
        let rows = self.current_diff_rows();
        let gutter = gutter_width(&rows);
        let mut lens = Vec::new();
        for row in &rows {
            if let DiffRow::Line { left, right } = row {
                lens.push(left.text.chars().count());
                if let Some(right) = right {
                    lens.push(right.text.chars().count());
                }
            }
        }
        let pane_w = self.layout.diff_pane_width.max(1) as usize;
        let viewport = cell_code_width(pane_w, gutter);
        let max = max_col_offset(&lens, viewport);
        self.diff_col_offset = apply_pan(self.diff_col_offset, delta, max);
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
        self.pending_hunk_anchor = Some(anchor_row_index(
            &rows,
            self.diff_scroll as usize,
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
        let Some(anchor) = self.pending_hunk_anchor.take() else {
            return;
        };
        let rows = self.current_diff_rows();
        self.diff_scroll = scroll_to_keep_row(anchor, self.diff_body_height(), rows.len());
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
    /// snap onto a selectable row. EasyMotion / click stay on `visible_rows`.
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
        Effect::None
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
                Effect::None
            }
            ListFocusTarget::None => {
                self.scroll_right(delta);
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
                Effect::None
            }
            ListFocusTarget::None => Effect::None,
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
    use super::super::easy_motion::visible_window;
    use super::super::gates::ListFocusTarget;
    use super::super::keys::InputMode;
    use super::super::theme::{resolve_theme_id, ThemeId};
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::split::{pane_widths, side_by_side_column_widths, DIFF_SPLIT_FRACTION};
    use crate::tui::watch::watch_interval_ms;
    use workspace_status_graph::{Commit, GraphRef, Stash};

    fn repo(name: &str, dirty: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
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
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
                merged_into_default: None,
                default_branch_override: None,
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
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: None,
                    default_branch_override: None,
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
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: None,
                    default_branch_override: None,
                },
                RepoSnapshot {
                    repo: "notes".into(),
                    branch: "main".into(),
                    sync_status: SyncStatus::Ahead,
                    sync_note: "ahead 1".into(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Primary,
                    primary_repo: None,
                    merged_into_default: None,
                    default_branch_override: None,
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
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: Some(false),
                    default_branch_override: None,
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
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Linked,
                primary_repo: Some("notes".into()),
                merged_into_default: None,
                default_branch_override: None,
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
            Effect::LoadCommitDiff { path, .. } => assert_eq!(path, "src/lib.rs"),
            other => panic!("expected LoadCommitDiff, got {other:?}"),
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
        assert_eq!(app.dispatch(Action::Move(1)), Effect::None);
        assert_eq!(app.commit_files_cursor(), files_before);
        assert_ne!(app.graph_cursor, graph_before);
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

    #[test]
    fn easy_motion_labels_visible_rows_only_and_jumps() {
        let mut app = state();
        app.layout.tree_height = 2;
        app.cursor = 0;
        assert!(app.rows.len() > 2, "need more rows than the viewport");
        assert_eq!(app.dispatch(Action::EasyMotionStart), Effect::None);
        assert!(app.easy_motion.is_some());
        assert_eq!(app.input_mode(), InputMode::EasyMotion);
        let first = app.rows[0].id.clone();
        let second = app.rows[1].id.clone();
        assert_eq!(
            app.dispatch(Action::EasyMotionChar('b')),
            Effect::LoadRightPane
        );
        assert_eq!(
            app.focused_row().map(|r| r.id.as_str()),
            Some(second.as_str())
        );
        assert!(app.easy_motion.is_none());

        app.cursor = 0;
        app.layout.tree_height = 2;
        app.dispatch(Action::EasyMotionStart);
        app.dispatch(Action::EasyMotionChar('a'));
        assert_eq!(
            app.focused_row().map(|r| r.id.as_str()),
            Some(first.as_str())
        );

        app.cursor = app.rows.len() - 1;
        app.layout.tree_height = 2;
        let (start, count) =
            visible_window(app.rows.len(), app.cursor, app.layout.tree_height as usize);
        assert_eq!(count, 2);
        assert_eq!(start, app.rows.len() - 2);
        let target = app.rows[start].id.clone();
        app.dispatch(Action::EasyMotionStart);
        app.dispatch(Action::EasyMotionChar('a'));
        assert_eq!(
            app.focused_row().map(|r| r.id.as_str()),
            Some(target.as_str())
        );
        assert_ne!(app.cursor, 0);
    }

    #[test]
    fn easy_motion_partial_hit_miss_and_esc() {
        use super::super::easy_motion::{resolve_easy_motion_label, EasyMotionResolve};
        assert_eq!(
            resolve_easy_motion_label(&["aa".into(), "ab".into()], "a"),
            EasyMotionResolve::Partial
        );

        let mut app = state();
        app.layout.tree_height = 30;
        app.cursor = 0;
        let before = app.cursor;
        app.dispatch(Action::EasyMotionStart);
        assert_eq!(app.dispatch(Action::EasyMotionChar('z')), Effect::None);
        assert!(app.easy_motion.is_none(), "miss cancels");
        assert_eq!(app.cursor, before, "miss stays on the same row");

        app.dispatch(Action::EasyMotionStart);
        assert_eq!(app.dispatch(Action::EasyMotionCancel), Effect::None);
        assert!(app.easy_motion.is_none());
        assert_eq!(
            app.input_mode(),
            InputMode::Normal {
                search_active: false
            }
        );
        assert_eq!(app.cursor, before, "Esc stays on the same row");
    }

    #[test]
    fn easy_motion_start_is_noop_on_focused_diff() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        assert!(app.right_is_diff());
        assert_eq!(app.dispatch(Action::EasyMotionStart), Effect::None);
        assert!(app.easy_motion.is_none());
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
            !app.folds.contains("repo:app"),
            "toggleSubtree on a folded parent opens the subtree"
        );
        assert!(!app.folds.contains("dir:app:src"));
    }

    #[test]
    fn fold_toggle_subtree_closes_an_open_subtree() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        app.dispatch(Action::FoldToggleSubtree);
        assert!(app.folds.contains("repo:app"));
        assert!(app.folds.contains("dir:app:src"));
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
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

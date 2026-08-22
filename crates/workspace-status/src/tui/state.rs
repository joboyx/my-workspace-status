//! TUI state and Action dispatch.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use workspace_status_graph::{GraphModel, GraphRow};

use crate::snapshot::{FileChange, WorkspaceSnapshot};

use super::action::{Action, Effect};
use super::drill::{
    source_from_graph_row, stash_ref_from_graph_row, CommitFile, CommitFileSource, DrillView,
};
use super::branches::{
    can_open_branch_picker, is_valid_branch_name, BranchPickerState, CreateBranchState,
};
use super::keys::InputMode;
use super::fetch::background_fetch_targets;
use super::ops::{op_targets, push_targets, Op};
use super::search::focus_tree_search;
use super::split::{
    clamp_tree_fraction, diff_split_fraction_from_col, hit_split, is_side_by_side_split,
    pair_unified_lines, tree_fraction_from_col, DiffMode, SplitDrag, SplitHit, SplitLayout,
    DIFF_SPLIT_FRACTION, TREE_WIDTH_FRACTION,
};
use super::stash::{
    checkout_path, resolve_stash_menu_key, row_is_hidden_ignored, stash_dirty_for_row,
    stash_ops_for_context, StashMenuKeyResult, StashOp, StashOpId, StashOpsContext,
};
use super::tree::{
    build_tree, default_folds, flatten, visible_for_tree, NodeKind, TreeNode, VisibleRow,
};
use super::viewed::{
    collect_current_fingerprints, fingerprint_file_change, is_viewed, load_viewed_store,
    reconcile_viewed, save_viewed_store, toggle_viewed, viewed_identity, viewed_row_ids,
    ViewedStore,
};
#[cfg(not(test))]
use super::viewed::viewed_store_path;
use super::watch::{changed_row_ids, tree_signatures};
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
    pub diff_content_x: u16,
    pub diff_split_rule_x: Option<u16>,
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
            diff_content_x: 50,
            diff_split_rule_x: None,
        }
    }
}

/// Confirm overlay before a destructive file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingConfirm {
    Revert {
        repo: String,
        path: String,
        untracked: bool,
    },
    StashPop {
        repo: String,
        stash_ref: String,
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
    },
}

/// Interactive session state. Dispatch is pure besides the returned [`Effect`].
#[derive(Clone, Debug)]
pub struct AppState {
    pub cwd: PathBuf,
    pub snapshot: WorkspaceSnapshot,
    pub show_ignored: bool,
    pub tree: TreeNode,
    pub folds: HashSet<String>,
    pub rows: Vec<VisibleRow>,
    pub cursor: usize,
    pub help_open: bool,
    pub focus: FocusPane,
    pub status: String,
    pub graph: Option<GraphModel>,
    pub graph_identity: Option<(String, String)>,
    pub graph_scroll: u16,
    pub graph_cursor: usize,
    pub drill: DrillView,
    pub diff_lines: Vec<String>,
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
}

impl AppState {
    pub fn new(cwd: PathBuf, snapshot: WorkspaceSnapshot, ascii: bool) -> Self {
        let show_ignored = snapshot.show_ignored;
        let visible = visible_snapshot(&snapshot, show_ignored);
        let tree = build_tree(&visible);
        let folds = default_folds(&tree);
        let rows = flatten(&tree, &folds);
        let cursor = initial_cursor(&rows);
        let signatures = tree_signatures(&tree);
        let viewed_path = default_viewed_path();
        let viewed_store = load_viewed_store(&viewed_path);
        let mut state = Self {
            cwd,
            snapshot,
            show_ignored,
            tree,
            folds,
            rows,
            cursor,
            help_open: false,
            focus: FocusPane::Left,
            status: "q quit  ? help  . ignored  f fetch  p pull  d default".into(),
            graph: None,
            graph_identity: None,
            graph_scroll: 0,
            graph_cursor: 0,
            drill: DrillView::Graph,
            diff_lines: Vec::new(),
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
            InputMode::Help
        } else if self.search_mode {
            InputMode::SearchPrompt
        } else {
            InputMode::Normal {
                search_active: self.search_active,
            }
        }
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
        self.focus == FocusPane::Right
            && self.drill.is_graph()
            && self.focused_graph_stash_ref().is_some()
    }

    pub fn rebuild_rows(&mut self) {
        let focus_id = self.focused_row().map(|r| r.id.clone());
        let visible = visible_snapshot(&self.snapshot, self.show_ignored);
        self.tree = build_tree(&visible);
        self.rows = flatten(&self.tree, &self.folds);
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

    pub fn apply_snapshot(&mut self, snapshot: WorkspaceSnapshot) {
        self.snapshot = snapshot;
        self.snapshot.show_ignored = self.show_ignored;
        self.rebuild_rows();
        self.signatures = tree_signatures(&self.tree);
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
        self.flashes.retain(|_, at| now.duration_since(*at).as_millis() < 800);
        for id in &changed {
            self.flashes.insert(id.clone(), now);
        }
        changed
    }

    pub fn dispatch(&mut self, action: Action) -> Effect {
        match action {
            Action::Quit => Effect::Quit,
            Action::ToggleHelp => {
                self.drag = SplitDrag::None;
                self.help_open = !self.help_open;
                Effect::None
            }
            Action::Move(delta) => self.move_focused(delta),
            Action::MoveToStart => self.move_focused_edge(false),
            Action::MoveToEnd => self.move_focused_edge(true),
            Action::PageMove(pages) => {
                let height = self.layout.tree_height.max(1) as i32;
                self.move_focused(pages * height)
            }
            Action::FoldToggle => {
                self.fold_op(FoldOp::Toggle);
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
            Action::Fetch => self.op_effect(Op::Fetch),
            Action::Pull => self.op_effect(Op::Pull),
            Action::DefaultBranch => self.op_effect(Op::DefaultBranch),
            Action::Refresh => Effect::ReloadSnapshot,
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
            Action::Click { col, row } => self.click(col, row),
            Action::Drag { col, row } => self.drag_split(col, row),
            Action::Release => {
                self.drag = SplitDrag::None;
                Effect::None
            }
            Action::ToggleDiffMode => self.toggle_diff_mode(),
            Action::ScrollWheel { col, row: _, delta } => {
                if col >= self.layout.right_x {
                    self.focus = FocusPane::Right;
                    self.scroll_right(delta);
                    Effect::None
                } else {
                    self.focus = FocusPane::Left;
                    self.move_cursor(delta);
                    Effect::LoadRightPane
                }
            }
            Action::SearchStart => {
                self.drag = SplitDrag::None;
                self.search_mode = true;
                self.search_active = false;
                self.search_query.clear();
                self.status = "/".into();
                Effect::None
            }
            Action::SearchChar(c) => {
                if self.search_mode {
                    self.search_query.push(c);
                    self.apply_search(0);
                    Effect::LoadRightPane
                } else {
                    Effect::None
                }
            }
            Action::SearchBackspace => {
                if self.search_mode {
                    self.search_query.pop();
                    self.apply_search(0);
                    Effect::LoadRightPane
                } else {
                    Effect::None
                }
            }
            Action::SearchSubmit => {
                self.search_mode = false;
                if self.search_query.trim().is_empty() {
                    self.search_active = false;
                    self.search_query.clear();
                    self.status = "search cleared".into();
                } else {
                    self.search_active = true;
                    self.apply_search(0);
                }
                Effect::LoadRightPane
            }
            Action::SearchCancel => {
                self.search_mode = false;
                self.search_active = false;
                self.search_query.clear();
                self.status = "search cancelled".into();
                Effect::None
            }
            Action::SearchNext => {
                if self.search_active && !self.search_query.trim().is_empty() {
                    self.apply_search(1);
                    Effect::LoadRightPane
                } else {
                    Effect::None
                }
            }
            Action::SearchPrev => {
                if self.search_active && !self.search_query.trim().is_empty() {
                    self.apply_search(-1);
                    Effect::LoadRightPane
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
            Action::ConfirmYes => self.confirm_yes(),
            Action::ConfirmNo => {
                if let Some(pending) = self.confirm.take() {
                    self.status = match pending {
                        PendingConfirm::Revert { .. } => "revert cancelled".into(),
                        PendingConfirm::StashPop { .. } => "pop cancelled".into(),
                        PendingConfirm::StashDrop { .. } => "drop cancelled".into(),
                        PendingConfirm::CheckoutOutOfSync { .. } => "checkout cancelled".into(),
                        PendingConfirm::RemoveWorktree { .. } => "remove worktree cancelled".into(),
                    };
                }
                Effect::None
            }
            Action::Edit => {
                if let Some((repo, change)) = self.focused_file_if_shown() {
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
            Action::NavEsc => self.nav_esc(),
            Action::GraphStashApply => self.graph_stash_op(StashOpId::Apply),
            Action::GraphStashPop => self.graph_stash_op(StashOpId::Pop),
            Action::GraphStashDrop => self.graph_stash_op(StashOpId::Drop),
            Action::None => Effect::None,
        }
    }

    fn op_effect(&mut self, op: Op) -> Effect {
        let targets = op_targets(
            &self.snapshot,
            self.focused_row(),
            self.show_ignored,
            op,
        );
        if targets.is_empty() {
            self.status = "no visible repos for that op".into();
            return Effect::None;
        }
        match op {
            Op::Fetch => {
                self.status = format!("fetch {}", targets.join(" "));
                Effect::Fetch { repos: targets }
            }
            Op::Pull => {
                let behind: Vec<String> = targets
                    .into_iter()
                    .filter(|repo| {
                        self.snapshot.repos.iter().any(|r| {
                            r.repo == *repo
                                && r.sync_status == crate::snapshot::SyncStatus::Behind
                        })
                    })
                    .collect();
                if behind.is_empty() {
                    self.status = "nothing behind to pull".into();
                    Effect::None
                } else {
                    self.status = format!("pull {}", behind.join(" "));
                    Effect::Pull { repos: behind }
                }
            }
            Op::DefaultBranch => {
                let kind = self.focused_row().map(|r| r.kind);
                let repos = if matches!(kind, Some(NodeKind::Workspace | NodeKind::Group) | None) {
                    targets
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
                        .collect()
                } else {
                    targets
                };
                if repos.is_empty() {
                    self.status = "no non-default branches to switch".into();
                    Effect::None
                } else {
                    self.status = format!("default-branch {}", repos.join(" "));
                    Effect::DefaultBranch { repos }
                }
            }
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

    fn fold_op(&mut self, op: FoldOp) {
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

    fn scroll_right(&mut self, delta: i32) {
        if let DrillView::Diff { lines, .. } = &self.drill {
            let max = self.diff_scroll_max(lines) as i32;
            let next = self.diff_scroll as i32 + delta;
            self.diff_scroll = next.clamp(0, max) as u16;
            return;
        }
        if self.right_is_diff() {
            let max = self.diff_scroll_max(&self.diff_lines) as i32;
            let next = self.diff_scroll as i32 + delta;
            self.diff_scroll = next.clamp(0, max) as u16;
        } else {
            let next = self.graph_scroll as i32 + delta;
            self.graph_scroll = next.max(0) as u16;
        }
    }

    fn diff_scroll_max(&self, lines: &[String]) -> usize {
        let count = if is_side_by_side_split(self.diff_mode, self.layout.diff_pane_width) {
            pair_unified_lines(lines).len()
        } else {
            lines.len()
        };
        count.saturating_sub(1)
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
                return Effect::None;
            }
            SplitHit::DiffSplit => {
                self.drag = SplitDrag::Diff;
                self.apply_diff_fraction_from_col(col);
                return Effect::None;
            }
            SplitHit::Other => {}
        }
        if col >= self.layout.right_x {
            self.focus = FocusPane::Right;
            return Effect::None;
        }
        self.focus = FocusPane::Left;
        if row < self.layout.tree_y {
            return Effect::None;
        }
        let idx = self.layout.list_offset + (row - self.layout.tree_y) as usize;
        if idx < self.rows.len() {
            self.cursor = idx;
            return Effect::LoadRightPane;
        }
        Effect::None
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
        let n = self.graph.as_ref().map(|g| g.visible_rows().len()).unwrap_or(0);
        if n == 0 {
            self.graph_cursor = 0;
        } else {
            self.graph_cursor = self.graph_cursor.min(n - 1);
        }
        if self.drill.is_graph() {
            self.diff_lines.clear();
            self.diff_repo = None;
            self.diff_path = None;
        }
    }

    pub fn set_diff(&mut self, repo: String, path: String, lines: Vec<String>) {
        let same = self.diff_repo.as_deref() == Some(repo.as_str())
            && self.diff_path.as_deref() == Some(path.as_str());
        if !same {
            self.diff_scroll = 0;
        }
        self.diff_repo = Some(repo);
        self.diff_path = Some(path);
        self.diff_lines = lines;
        if self.drill.is_graph() {
            self.graph = None;
        }
    }

    pub fn clear_right(&mut self) {
        self.graph = None;
        self.graph_identity = None;
        self.graph_cursor = 0;
        self.drill = DrillView::Graph;
        self.diff_lines.clear();
        self.diff_repo = None;
        self.diff_path = None;
    }

    pub fn open_commit_files(&mut self, repo: String, source: CommitFileSource, files: Vec<CommitFile>) {
        let cursor = DrillView::files_cursor(&files, 0);
        self.status = format!("files {}", files.len());
        self.drill = DrillView::Files {
            repo,
            source,
            files,
            cursor,
        };
    }

    pub fn open_commit_diff(
        &mut self,
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
        file_cursor: usize,
        path: String,
        lines: Vec<String>,
    ) {
        self.diff_scroll = 0;
        self.status = format!("diff {path}");
        self.drill = DrillView::Diff {
            repo,
            source,
            files,
            file_cursor,
            path,
            lines,
        };
    }

    pub fn focused_file(&self) -> Option<(String, FileChange)> {
        let row = self.focused_row()?;
        Some((row.repo.clone()?, row.file.clone()?))
    }

    pub fn focused_graph_repo(&self) -> Option<String> {
        let row = self.focused_row()?;
        match row.kind {
            NodeKind::Repo | NodeKind::Checkout => row.repo.clone(),
            NodeKind::File => None,
            NodeKind::Workspace | NodeKind::Group => None,
        }
    }

    fn apply_search(&mut self, dir: i32) {
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
            self.status = if self.search_mode {
                format!("/{}", self.search_query)
            } else {
                format!("/{}  n next  N prev", self.search_query)
            };
        } else if self.search_query.trim().is_empty() {
            self.status = "/".into();
        } else {
            self.status = format!("/{}  no match", self.search_query);
        }
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
        let Some((repo, change)) = self.focused_file_if_shown() else {
            self.status = match write {
                FileWrite::Stage => "focus a dirty file to stage".into(),
                FileWrite::Unstage => "focus a dirty file to unstage".into(),
            };
            return Effect::None;
        };
        let paths = op_paths(&change);
        match write {
            FileWrite::Stage => {
                if !is_stageable(&change) {
                    self.status = "nothing to stage".into();
                    return Effect::None;
                }
                self.status = format!("stage {}", change.path);
                Effect::Stage { repo, paths }
            }
            FileWrite::Unstage => {
                if !is_unstageable(&change) {
                    self.status = "nothing to unstage".into();
                    return Effect::None;
                }
                self.status = format!("unstage {}", change.path);
                Effect::Unstage { repo, paths }
            }
        }
    }

    fn begin_revert(&mut self) -> Effect {
        let Some((repo, change)) = self.focused_file_if_shown() else {
            self.status = "focus a dirty file to revert".into();
            return Effect::None;
        };
        if !is_revertible(&change) {
            self.status = if change.staged_status.is_some() {
                "nothing to discard (staged only)".into()
            } else {
                "nothing to discard".into()
            };
            return Effect::None;
        }
        self.confirm = Some(PendingConfirm::Revert {
            repo,
            path: change.path.clone(),
            untracked: change.untracked,
        });
        self.status = if change.untracked {
            format!("delete {}? y/n", change.path)
        } else {
            format!("revert {}? y/n", change.path)
        };
        Effect::None
    }

    fn confirm_yes(&mut self) -> Effect {
        match self.confirm.take() {
            Some(PendingConfirm::Revert {
                repo,
                path,
                untracked,
            }) => {
                let paths = if let Some((_, change)) = self.focused_file_if_shown() {
                    if change.path == path {
                        op_paths(&change)
                    } else {
                        vec![path.clone()]
                    }
                } else {
                    vec![path.clone()]
                };
                self.status = if untracked {
                    format!("delete {path}")
                } else {
                    format!("revert {path}")
                };
                Effect::Revert {
                    repo,
                    paths,
                    untracked,
                }
            }
            Some(PendingConfirm::StashPop { repo, stash_ref }) => {
                self.status = format!("pop {stash_ref}");
                Effect::StashPop { repo, stash_ref }
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
                self.status = format!("checkout {branch} then pull {remote_ref}");
                Effect::CheckoutBranch {
                    repo,
                    branch,
                    pull_after: true,
                }
            }
            Some(PendingConfirm::RemoveWorktree {
                primary,
                path,
                force,
            }) => {
                self.status = format!("remove worktree {path}");
                Effect::RemoveWorktree {
                    primary,
                    path,
                    force,
                }
            }
            None => Effect::None,
        }
    }

    fn toggle_reviewed(&mut self) -> Effect {
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

    fn begin_remove_worktree(&mut self) -> Effect {
        let Some(row) = self.focused_row() else {
            return Effect::None;
        };
        if row_is_hidden_ignored(row, self.show_ignored) {
            return Effect::None;
        }
        if !matches!(row.kind, NodeKind::Checkout | NodeKind::Repo) {
            return Effect::None;
        }
        let Some(repo_path) = row.repo.as_deref() else {
            return Effect::None;
        };
        let Some(snap) = self.snapshot.repos.iter().find(|r| r.repo == repo_path) else {
            return Effect::None;
        };
        if snap.checkout_kind != CheckoutKind::Linked {
            return Effect::None;
        }
        let Some(primary) = snap.primary_repo.clone() else {
            return Effect::None;
        };
        let force = snap.has_unstaged || snap.has_staged || snap.has_untracked;
        let path = snap.repo.clone();
        self.confirm = Some(PendingConfirm::RemoveWorktree {
            primary,
            path: path.clone(),
            force,
        });
        self.status = format!("remove worktree {path}? y/n");
        Effect::None
    }

    fn fetch_tick_effect(&mut self) -> Effect {
        let targets = background_fetch_targets(&self.snapshot, self.show_ignored);
        if targets.is_empty() {
            return Effect::None;
        }
        self.status = format!("fetch {}", targets.join(" "));
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
            self.status = "focus a visible repo to push".into();
            return Effect::None;
        }
        self.status = format!("push {}", targets.join(" "));
        Effect::Push { repos: targets }
    }

    fn begin_stash_menu(&mut self) -> Effect {
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
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty,
            dirty_paths,
            latest_stash_ref: focused_stash.or(latest_stash_ref),
        });
        if ops.is_empty() {
            self.stash_menu = None;
            self.stash_repo = None;
            self.status = "nothing to stash".into();
            return;
        }
        self.stash_repo = Some(repo);
        self.stash_menu = Some(ops);
        self.status = "stash  s create  a apply  p pop  d drop".into();
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
                self.confirm = Some(PendingConfirm::StashPop {
                    repo,
                    stash_ref: stash_ref.clone(),
                });
                self.status = format!("pop {stash_ref}? y/n");
                Effect::None
            }
            StashOpId::Drop => {
                let stash_ref = op.stash_ref.unwrap_or_else(|| "stash@{0}".into());
                self.stash_menu = None;
                self.confirm = Some(PendingConfirm::StashDrop {
                    repo,
                    stash_ref: stash_ref.clone(),
                });
                self.status = format!("drop {stash_ref}? y/n");
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
        self.status = "j/k move  type filter  Enter checkout  C create".into();
    }

    fn submit_branch_picker(&mut self) -> Effect {
        let Some(picker) = self.branch_picker.as_ref() else {
            return Effect::None;
        };
        let repo = picker.repo.clone();
        let filter = picker.filter.clone();
        if let Some(selected) = picker.selected().cloned() {
            self.branch_picker = None;
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

    /// Ask before checkout when `origin/<branch>` exists and differs.
    pub fn checkout_or_confirm(&mut self, repo: String, branch: String) -> Effect {
        self.status = format!("checkout {branch}");
        Effect::CheckoutBranch {
            repo,
            branch,
            pull_after: false,
        }
    }

    /// Confirm checkout when local is out of sync with origin/*.
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
            self.status = format!("{branch} is not in sync with {remote_ref}. checkout then pull? y/n");
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
        self.create_branch = Some(CreateBranchState { repo, name: seed });
        self.status = "create branch  Enter confirm  Esc cancel".into();
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
        Effect::CreateBranch {
            repo: create.repo,
            name: create.name.trim().to_string(),
        }
    }

    fn tree_file_focused(&self) -> bool {
        matches!(self.focused_row().map(|r| r.kind), Some(NodeKind::File))
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
        if self.focus != FocusPane::Right || !self.drill.is_graph() {
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
        if (self.graph_cursor as u16) < self.graph_scroll {
            self.graph_scroll = self.graph_cursor as u16;
        }
    }

    fn move_file_cursor(&mut self, delta: i32) {
        if let DrillView::Files { cursor, files, .. } = &mut self.drill {
            if files.is_empty() {
                *cursor = 0;
                return;
            }
            let next = *cursor as i32 + delta;
            *cursor = next.clamp(0, files.len() as i32 - 1) as usize;
        }
    }

    fn move_focused(&mut self, delta: i32) -> Effect {
        if self.focus == FocusPane::Right {
            match &self.drill {
                DrillView::Files { .. } => {
                    self.move_file_cursor(delta);
                    Effect::None
                }
                DrillView::Diff { .. } => {
                    self.scroll_right(delta);
                    Effect::None
                }
                DrillView::Graph => {
                    if self.tree_file_focused() {
                        self.scroll_right(delta);
                    } else {
                        self.move_graph_cursor(delta);
                    }
                    Effect::None
                }
            }
        } else {
            self.move_cursor(delta);
            self.drill = DrillView::Graph;
            Effect::LoadRightPane
        }
    }

    fn move_focused_edge(&mut self, end: bool) -> Effect {
        if self.focus == FocusPane::Right {
            let tree_file = self.tree_file_focused();
            match &mut self.drill {
                DrillView::Files { cursor, files, .. } => {
                    *cursor = if end { files.len().saturating_sub(1) } else { 0 };
                    Effect::None
                }
                DrillView::Graph if !tree_file => {
                    let n = self
                        .graph
                        .as_ref()
                        .map(|g| g.visible_rows().len())
                        .unwrap_or(0);
                    self.graph_cursor = if end { n.saturating_sub(1) } else { 0 };
                    Effect::None
                }
                _ => Effect::None,
            }
        } else if end {
            if !self.rows.is_empty() {
                self.cursor = self.rows.len() - 1;
            }
            self.drill = DrillView::Graph;
            Effect::LoadRightPane
        } else {
            self.cursor = 0;
            self.drill = DrillView::Graph;
            Effect::LoadRightPane
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
                files,
                cursor,
            } => {
                let Some(file) = files.get(*cursor) else {
                    self.status = "no files in this commit".into();
                    return Effect::None;
                };
                Effect::LoadCommitDiff {
                    repo: repo.clone(),
                    source: source.clone(),
                    path: file.path.clone(),
                }
            }
            DrillView::Diff { .. } => Effect::None,
        }
    }

    fn nav_esc(&mut self) -> Effect {
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
            DrillView::Graph => {
                if self.focus == FocusPane::Right {
                    self.focus = FocusPane::Left;
                }
                Effect::None
            }
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
                self.status = format!("drop {stash_ref}? y/n");
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

fn visible_snapshot(snapshot: &WorkspaceSnapshot, show_ignored: bool) -> WorkspaceSnapshot {
    let mut copy = snapshot.clone();
    copy.show_ignored = show_ignored;
    visible_for_tree(&copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::split::{pane_widths, side_by_side_column_widths, DIFF_SPLIT_FRACTION};
    use crate::tui::watch::watch_interval_ms;
    use crate::snapshot::{build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus};
    use workspace_status_graph::{Commit, Stash};

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

    #[test]
    fn workspace_default_branch_skips_default_and_hidden_ignored() {
        let mut app = state();
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert!(app.status.contains("no non-default"));
    }

    #[test]
    fn help_toggle_does_not_quit() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::ToggleHelp), Effect::None);
        assert!(app.help_open);
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
        assert_eq!(app.focused_row().map(|r| r.id.as_str()), Some("file:app:README.md"));
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
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        assert!(app.status.contains("focus a dirty file"));
        app.dispatch(Action::ToggleShowIgnored);
        assert!(app.rows.iter().any(|r| r.label.contains("notes")));
        focus_file(&mut app, "README.md");
        // notes file is also README? notes has README.md in helper
        let notes = app
            .rows
            .iter()
            .position(|r| r.id == "file:notes:README.md");
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
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
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
                paths,
                untracked,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(paths, vec!["README.md"]);
                assert!(!untracked);
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
            ("GIT_COMMITTER_EMAIL", "workspace-status-test@example.invalid"),
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
            Effect::Revert { paths, untracked, .. } => {
                assert!(!untracked);
                revert_tracked_file(&repo_dir, &paths[0]).unwrap();
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
                assert!(!repos.iter().any(|r| r.contains("worktrees") || r == "notes"));
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
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert!(app.stash_menu.is_some());
        assert_eq!(app.dispatch(Action::StashMenuCancel), Effect::None);
        assert!(app.stash_menu.is_none());
        assert!(app.status.contains("cancelled"));

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('s')), Effect::StashCreate {
            repo: "app".into(),
            paths: vec!["README.md".into()],
        });

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        match app.dispatch(Action::StashMenuChar('a')) {
            Effect::StashApply { repo, stash_ref } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('p')), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.status.contains("pop cancelled"));

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        app.dispatch(Action::StashMenuChar('p'));
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashPop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
            other => panic!("{other:?}"),
        }

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        app.dispatch(Action::StashMenuChar('d'));
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        app.dispatch(Action::StashMenuChar('d'));
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
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
            app.branch_picker.as_ref().unwrap().selected().map(|b| b.name.as_str()),
            Some("feature/x")
        );
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch { branch, pull_after, .. } => {
                assert_eq!(branch, "feature/x");
                assert!(!pull_after);
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
                branch,
                pull_after,
                ..
            } => {
                assert_eq!(branch, "feature/x");
                assert!(pull_after);
            }
            other => panic!("{other:?}"),
        }
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
            ("GIT_COMMITTER_EMAIL", "workspace-status-test@example.invalid"),
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
        app.open_stash_menu("app".into(), Some(latest.clone()));
        match app.dispatch(Action::StashMenuChar('a')) {
            Effect::StashApply { stash_ref, .. } => stash_apply(&repo_dir, &stash_ref).unwrap(),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# dirty\n"
        );
        app.open_stash_menu("app".into(), Some(latest.clone()));
        app.dispatch(Action::StashMenuChar('d'));
        app.dispatch(Action::ConfirmNo);
        assert_eq!(latest_stash_ref(&repo_dir).as_deref(), Some(latest.as_str()));
        app.open_stash_menu("app".into(), Some(latest.clone()));
        app.dispatch(Action::StashMenuChar('d'));
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => stash_drop(&repo_dir, &stash_ref).unwrap(),
            other => panic!("{other:?}"),
        }
        assert!(latest_stash_ref(&repo_dir).is_none());

        fs::write(repo_dir.join("README.md"), "# again\n").unwrap();
        stash_push(&repo_dir, &[]).unwrap();
        let latest = latest_stash_ref(&repo_dir).expect("stash2");
        app.open_stash_menu("app".into(), Some(latest.clone()));
        app.dispatch(Action::StashMenuChar('p'));
        app.dispatch(Action::ConfirmNo);
        assert_eq!(latest_stash_ref(&repo_dir).as_deref(), Some(latest.as_str()));
        app.open_stash_menu("app".into(), Some(latest.clone()));
        app.dispatch(Action::StashMenuChar('p'));
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashPop { stash_ref, .. } => stash_pop(&repo_dir, &stash_ref).unwrap(),
            other => panic!("{other:?}"),
        }
        assert!(latest_stash_ref(&repo_dir).is_none());

        focus_repo(&mut app, "app");
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
            Effect::CheckoutBranch { branch, .. } => {
                assert_eq!(branch, "feature/pick");
                assert!(checkout_branch(&branch, &repo_dir));
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
        assert!(app.confirm.is_none());
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        let file_idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file");
        app.cursor = file_idx;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
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
        assert!(app.rows.iter().all(|r| r.repo.as_deref() != Some("notes/.worktrees/feat")));
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
    fn fetch_tick_visible_primaries_only_and_watch_stays_independent() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        match app.dispatch(Action::FetchTick) {
            Effect::Fetch { repos } => {
                assert_eq!(repos, vec!["app"]);
                assert!(!repos.iter().any(|r| r.contains("worktrees") || r == "notes"));
            }
            other => panic!("{other:?}"),
        }
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
        }
    }

    fn install_graph(app: &mut AppState, stashes: Vec<Stash>) {
        let model = GraphModel {
            commits: vec![graph_commit("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "head")],
            stashes,
            worktrees: Vec::new(),
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            sync: None,
            show_ignored: app.show_ignored,
            uncommitted: false,
        };
        app.set_graph(model, "app".into(), "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into());
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
        app.dispatch(Action::Move(1));
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
            vec!["+fn x() {}".into()],
        );
        assert!(app.drill.is_diff());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.drill.is_files());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::LoadRightPane);
        assert!(app.drill.is_graph());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert_eq!(app.focus, FocusPane::Left);
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
                },
                Stash {
                    stash_ref: "stash@{1}".into(),
                    subject: "older".into(),
                    parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
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
    fn stash_menu_from_repo_or_file_stays_latest_only() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Left;
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("menu");
        assert!(ops.iter().all(|op| {
            op.stash_ref.is_none() || op.stash_ref.as_deref() == Some("stash@{0}")
        }));
        app.stash_menu = None;
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{1}".into(),
                subject: "older".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
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
        assert!(
            ops.iter().any(|op| op.stash_ref.as_deref() == Some("stash@{1}")),
            "{ops:?}"
        );
        assert!(ops.iter().all(|op| {
            op.stash_ref.is_none() || op.stash_ref.as_deref() == Some("stash@{1}")
        }));
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
        assert_eq!(app.dispatch(Action::Click { col: 47, row: 5 }), Effect::None);
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
        assert_eq!(app.dispatch(Action::Click { col: rule, row: 6 }), Effect::None);
        assert_eq!(app.drag, SplitDrag::Diff);
        assert_eq!(app.dispatch(Action::Drag { col: rule + 20, row: 6 }), Effect::None);
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
        let effect = app.dispatch(Action::Click { col: 10, row: app.layout.tree_y });
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

}

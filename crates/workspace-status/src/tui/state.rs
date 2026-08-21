//! TUI state and Action dispatch.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use workspace_status_graph::GraphModel;

use crate::snapshot::{FileChange, WorkspaceSnapshot};

use super::action::{Action, Effect};
use super::keys::InputMode;
use super::ops::{op_targets, Op};
use super::search::focus_tree_search;
use super::tree::{
    build_tree, default_folds, flatten, visible_for_tree, NodeKind, TreeNode, VisibleRow,
};
use super::watch::{changed_row_ids, tree_signatures};

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
    pub diff_lines: Vec<String>,
    pub diff_scroll: u16,
    pub diff_repo: Option<String>,
    pub diff_path: Option<String>,
    pub reviewed: HashSet<String>,
    pub layout: LayoutHit,
    pub ascii: bool,
    pub search_mode: bool,
    pub search_active: bool,
    pub search_query: String,
    pub confirm: Option<PendingConfirm>,
    pub flashes: HashMap<String, Instant>,
    pub signatures: BTreeMap<String, String>,
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
        Self {
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
            diff_lines: Vec::new(),
            diff_scroll: 0,
            diff_repo: None,
            diff_path: None,
            reviewed: HashSet::new(),
            layout: LayoutHit::default(),
            ascii,
            search_mode: false,
            search_active: false,
            search_query: String::new(),
            confirm: None,
            flashes: HashMap::new(),
            signatures,
        }
    }

    pub fn input_mode(&self) -> InputMode {
        if self.confirm.is_some() {
            InputMode::Confirm
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
        matches!(self.focused_row().map(|r| r.kind), Some(NodeKind::File))
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
                self.help_open = !self.help_open;
                Effect::None
            }
            Action::Move(delta) => {
                self.move_cursor(delta);
                Effect::LoadRightPane
            }
            Action::MoveToStart => {
                self.cursor = 0;
                Effect::LoadRightPane
            }
            Action::MoveToEnd => {
                if !self.rows.is_empty() {
                    self.cursor = self.rows.len() - 1;
                }
                Effect::LoadRightPane
            }
            Action::PageMove(pages) => {
                let height = self.layout.tree_height.max(1) as i32;
                self.move_cursor(pages * height);
                Effect::LoadRightPane
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
            Action::ToggleReviewed => {
                if let Some(row) = self.focused_row() {
                    if row.kind == NodeKind::File {
                        let id = row.id.clone();
                        if !self.reviewed.remove(&id) {
                            self.reviewed.insert(id);
                        }
                    }
                }
                Effect::None
            }
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
            Action::Revert => self.begin_revert(),
            Action::ConfirmYes => self.confirm_yes(),
            Action::ConfirmNo => {
                if self.confirm.take().is_some() {
                    self.status = "revert cancelled".into();
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
        if self.right_is_diff() {
            let max = self.diff_lines.len().saturating_sub(1) as i32;
            let next = self.diff_scroll as i32 + delta;
            self.diff_scroll = next.clamp(0, max) as u16;
        } else {
            let next = self.graph_scroll as i32 + delta;
            self.graph_scroll = next.max(0) as u16;
        }
    }

    fn click(&mut self, col: u16, row: u16) -> Effect {
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

    pub fn set_graph(&mut self, model: GraphModel, repo: String, head: String) {
        let identity = (repo, head);
        if self.graph_identity.as_ref() != Some(&identity) {
            self.graph_scroll = 0;
        }
        self.graph_identity = Some(identity);
        self.graph = Some(model);
        self.diff_lines.clear();
        self.diff_repo = None;
        self.diff_path = None;
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
        self.graph = None;
    }

    pub fn clear_right(&mut self) {
        self.graph = None;
        self.graph_identity = None;
        self.diff_lines.clear();
        self.diff_repo = None;
        self.diff_path = None;
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
        let Some(PendingConfirm::Revert {
            repo,
            path,
            untracked,
        }) = self.confirm.take()
        else {
            return Effect::None;
        };
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

fn visible_snapshot(snapshot: &WorkspaceSnapshot, show_ignored: bool) -> WorkspaceSnapshot {
    let mut copy = snapshot.clone();
    copy.show_ignored = show_ignored;
    visible_for_tree(&copy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::watch::watch_interval_ms;
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};

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
}

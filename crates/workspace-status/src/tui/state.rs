//! TUI state and Action dispatch.

use std::collections::HashSet;
use std::path::PathBuf;

use workspace_status_graph::GraphModel;

use crate::snapshot::{FileChange, WorkspaceSnapshot};

use super::action::{Action, Effect};
use super::ops::{op_targets, Op};
use super::tree::{
    build_tree, default_folds, flatten, visible_for_tree, NodeKind, TreeNode, VisibleRow,
};

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
}

impl AppState {
    pub fn new(cwd: PathBuf, snapshot: WorkspaceSnapshot, ascii: bool) -> Self {
        let show_ignored = snapshot.show_ignored;
        let visible = visible_snapshot(&snapshot, show_ignored);
        let tree = build_tree(&visible);
        let folds = default_folds(&tree);
        let rows = flatten(&tree, &folds);
        let cursor = initial_cursor(&rows);
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
}

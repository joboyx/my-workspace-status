//! "View remotely": what the focused row opens on the forge.
//!
//! This module only names the entity ([`RemoteFocus`]). The remote check and
//! the browser open run on a worker ([`Effect::ViewRemotely`]); the result
//! lands in [`AppState::apply_remote_view`].

use std::path::{Path, PathBuf};

use workspace_status_graph::{GraphRow, PartTarget, RefKind, Worktree};

use crate::helpers::is_counted_local_branch;

use super::super::action::Effect;
use super::super::diff::{DiffCellKind, DiffRow};
use super::super::drill::CommitFileSource;
use super::super::gates::ListFocusTarget;
use super::super::icons::IconKind;
use super::super::popover::IconTarget;
use super::super::remote_view::{
    opened_status, stash_local_only_status, FileRev, RemoteTarget, NOTHING_TO_VIEW, OPEN_FAILED,
    UNCOMMITTED_LOCAL_ONLY,
};
use super::super::status::StatusMessage;
use super::super::tree::{dir_path_from_id, NodeKind, VisibleRow};
use super::AppState;

/// What "View remotely" does for the focused entity.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum RemoteFocus {
    /// Check `target` on the remote of checkout `checkout`, then open it.
    View {
        /// Checkout path (snapshot `repo`).
        checkout: PathBuf,
        /// The entity whose page opens.
        target: RemoteTarget,
    },
    /// Nothing opens; the status line shows this text.
    Refuse(String),
}

impl RemoteFocus {
    /// [`Self::View`] of `target` in checkout `checkout`.
    fn view(checkout: impl Into<PathBuf>, target: RemoteTarget) -> Self {
        Self::View {
            checkout: checkout.into(),
            target,
        }
    }

    /// [`Self::Refuse`] with [`NOTHING_TO_VIEW`].
    fn nothing() -> Self {
        Self::Refuse(NOTHING_TO_VIEW.to_string())
    }

    /// The refusal text, or `None` when this opens (or checks) a page.
    fn refusal(self) -> Option<String> {
        match self {
            Self::Refuse(text) => Some(text),
            Self::View { .. } => None,
        }
    }
}

/// "View remotely" target of graph worktree `worktree`: its branch, else
/// its HEAD commit when detached, in that worktree.
fn worktree_focus(worktree: &Worktree) -> RemoteFocus {
    let target = match (&worktree.branch, &worktree.head_id) {
        (Some(branch), _) if is_counted_local_branch(branch) => {
            RemoteTarget::Branch(branch.clone())
        }
        (_, Some(sha)) if !sha.is_empty() => RemoteTarget::Commit(sha.clone()),
        _ => return RemoteFocus::nothing(),
    };
    RemoteFocus::view(worktree.path.as_str(), target)
}

impl AppState {
    /// What "View remotely" opens for the focused row.
    ///
    /// Tree: a repository row (a family row too) opens the repository home;
    /// a checkout row its branch, or its HEAD commit when detached; a file
    /// or directory row that path at the checkout branch (or HEAD commit).
    /// Graph: a commit row its commit; a worktree row its branch (or HEAD
    /// commit). A stash or the uncommitted row is local only.
    ///
    /// Commit-file list (a drill or a compare tab): the file or folder at
    /// [`Self::source_rev`]. Diff pane: its file at that revision, with the
    /// new-side line of the focused row ([`new_side_line`]). Explorer tab:
    /// the tree row, or the open file at its cursor line, at the checkout
    /// branch. Every other row and surface has nothing to view.
    pub(super) fn remote_target_for_focus(&self) -> RemoteFocus {
        self.remote_focus(true)
    }

    /// [`Self::remote_target_for_focus`]; with `line` false a diff target
    /// has no line anchor, so no diff rows are built. The line never
    /// decides a refusal, so the refusal gate passes false.
    fn remote_focus(&self, line: bool) -> RemoteFocus {
        if self.is_explorer_tab() {
            return self.remote_target_for_explorer(line);
        }
        match self.list_focus_target() {
            ListFocusTarget::Tree => self.focused_row().map_or_else(RemoteFocus::nothing, |row| {
                self.remote_target_for_tree_row(row)
            }),
            ListFocusTarget::Graph => self.remote_target_for_graph_row(),
            ListFocusTarget::CommitFiles => self.remote_target_for_commit_file(),
            ListFocusTarget::None => self.remote_target_for_diff(line),
        }
    }

    /// The status text that "View remotely" refuses with on the focused
    /// row, or `None` when it opens (or checks) a page. Disables the
    /// palette row.
    pub(super) fn view_remotely_refusal(&self) -> Option<String> {
        self.remote_focus(false).refusal()
    }

    /// What the View remotely line of popover section `kind` on `target`
    /// opens: the entity the section describes, not the focused row.
    ///
    /// A ref chip opens its tag (`Tag`), its remote-tracking branch
    /// (`RemoteBranch`), or its local branch (`Branch`); a checkout or sync
    /// mark run its branch; `[HEAD]` the commit of its row. Graph chips use
    /// the graph's checkout ([`AppState::focused_graph_repo`]). A worktree
    /// glyph opens that worktree's branch (or its HEAD commit when
    /// detached), and the sync header HEAD's branch (HEAD's commit when
    /// detached). The Branch and Linked worktree sections of a
    /// one-checkout repo row open that checkout's branch (its HEAD commit
    /// when detached), as a checkout row does; its Repo section opens the
    /// repository home. Every other target is on the focused row:
    /// [`Self::remote_target_for_focus`].
    pub(super) fn remote_target_for_icon(
        &self,
        kind: IconKind,
        target: &IconTarget,
    ) -> RemoteFocus {
        self.remote_focus_for_icon(kind, target, true)
    }

    /// [`Self::remote_target_for_icon`]; `line` as in [`Self::remote_focus`].
    fn remote_focus_for_icon(
        &self,
        kind: IconKind,
        target: &IconTarget,
        line: bool,
    ) -> RemoteFocus {
        match target {
            IconTarget::GraphChip { row, chip } => self.remote_target_for_chip(row, chip, line),
            IconTarget::GraphWorktree(path) => self
                .graph
                .as_ref()
                .and_then(|model| model.worktrees.iter().find(|wt| wt.path == *path))
                .map_or_else(RemoteFocus::nothing, worktree_focus),
            IconTarget::GraphSync => {
                let model = self.graph.as_ref();
                let branch = model
                    .and_then(|model| model.sync.as_ref())
                    .map(|sync| sync.branch.clone())
                    .filter(|branch| is_counted_local_branch(branch));
                let target = match (branch, model.and_then(|model| model.head_id.clone())) {
                    (Some(branch), _) => RemoteTarget::Branch(branch),
                    (None, Some(sha)) if !sha.is_empty() => RemoteTarget::Commit(sha),
                    _ => return RemoteFocus::nothing(),
                };
                self.focused_graph_repo()
                    .map_or_else(RemoteFocus::nothing, |checkout| {
                        RemoteFocus::view(checkout, target)
                    })
            }
            IconTarget::TreeRow(id)
                if matches!(kind, IconKind::Branch | IconKind::LinkedWorktree) =>
            {
                // A one-checkout repo row paints its checkout's icons too.
                let checkout = self
                    .rows
                    .iter()
                    .find(|row| row.id == *id && row.kind == NodeKind::Repo)
                    .and_then(|row| row.repo.as_deref());
                match checkout {
                    Some(checkout) => self.checkout_focus(checkout),
                    None => self.remote_focus(line),
                }
            }
            IconTarget::TreeRow(_)
            | IconTarget::CommitFileRow(_)
            | IconTarget::GraphRow(_)
            | IconTarget::GraphMoreLines(_)
            | IconTarget::CommitDetailMoreLines(_)
            | IconTarget::PullRequest(_) => self.remote_focus(line),
        }
    }

    /// "View remotely" target of graph chip `chip` painted for the graph
    /// row with [`graph_row_id`] `row`. `[+N]` falls back to the focused
    /// row.
    ///
    /// [`graph_row_id`]: super::super::popover::graph_row_id
    fn remote_target_for_chip(&self, row: &str, chip: &PartTarget, line: bool) -> RemoteFocus {
        let target = match chip {
            PartTarget::Chip(refs) => match refs.first() {
                Some(chip_ref) => match chip_ref.kind {
                    RefKind::Tag => RemoteTarget::Tag(chip_ref.name.clone()),
                    RefKind::Remote => RemoteTarget::RemoteBranch(chip_ref.name.clone()),
                    RefKind::Local => RemoteTarget::Branch(chip_ref.name.clone()),
                },
                None => return RemoteFocus::nothing(),
            },
            PartTarget::Marks { branch, .. } => RemoteTarget::Branch(branch.clone()),
            PartTarget::DetachedHead => match self.graph_row_commit(row) {
                Some(sha) => RemoteTarget::Commit(sha.to_string()),
                None => return RemoteFocus::nothing(),
            },
            PartTarget::Overflow(_) => return self.remote_focus(line),
        };
        self.focused_graph_repo()
            .map_or_else(RemoteFocus::nothing, |checkout| {
                RemoteFocus::view(checkout, target)
            })
    }

    /// Commit of the graph row with [`graph_row_id`] `row`: its own, or
    /// HEAD for the uncommitted row (whose footer shows HEAD's chips).
    /// `None` for a stash or worktree row, or a commit not in the graph.
    /// Reads the model in place; no row list is built.
    ///
    /// [`graph_row_id`]: super::super::popover::graph_row_id
    fn graph_row_commit(&self, row: &str) -> Option<&str> {
        let model = self.graph.as_ref()?;
        if row == "uncommitted" {
            return model.uncommitted.and(model.head_id.as_deref());
        }
        let sha = row.strip_prefix("commit:")?;
        model
            .commits
            .iter()
            .find(|commit| commit.id == sha)
            .map(|commit| commit.id.as_str())
    }

    /// The text the View remotely line of popover section `kind` on
    /// `target` refuses with, or `None` ([`Self::view_remotely_refusal`]
    /// for that section).
    pub(super) fn view_remotely_refusal_at(
        &self,
        kind: IconKind,
        target: &IconTarget,
    ) -> Option<String> {
        self.remote_focus_for_icon(kind, target, false).refusal()
    }

    /// "View remotely" target of the focused commit-file row: that file or
    /// folder, with no line.
    fn remote_target_for_commit_file(&self) -> RemoteFocus {
        let (Some((checkout, source)), Some(row)) =
            (self.commit_drill_source(), self.focused_commit_file_row())
        else {
            return RemoteFocus::nothing();
        };
        match self.source_rev(checkout, Some(source)) {
            Ok(rev) => RemoteFocus::view(
                checkout,
                RemoteTarget::File {
                    rev,
                    dir: row.is_dir(),
                    path: row.path,
                    line: None,
                },
            ),
            Err(refuse) => refuse,
        }
    }

    /// "View remotely" target of the focused diff pane: the open file, with
    /// the new-side line of the focused row (with `line` true). A folder
    /// summary in its place opens that folder.
    fn remote_target_for_diff(&self, line: bool) -> RemoteFocus {
        if self.folder_summary().is_some() {
            return self.remote_target_for_commit_file();
        }
        let Some((checkout, path, source)) = self.open_diff_target() else {
            return RemoteFocus::nothing();
        };
        let rev = match self.source_rev(checkout, source) {
            Ok(rev) => rev,
            Err(refuse) => return refuse,
        };
        let line = if line {
            self.current_diff_rows()
                .get(self.diff_cursor)
                .and_then(new_side_line)
        } else {
            None
        };
        RemoteFocus::view(
            checkout,
            RemoteTarget::File {
                rev,
                path: path.to_string(),
                line,
                dir: false,
            },
        )
    }

    /// "View remotely" target on an Explorer tab: a diff as in
    /// [`Self::remote_target_for_diff`]; the open file at its cursor line;
    /// or, with the tree focused, its file or folder row. `line` as in
    /// [`Self::remote_target_for_diff`].
    fn remote_target_for_explorer(&self, line: bool) -> RemoteFocus {
        let Some(tab) = self.tabs.active_explorer() else {
            return RemoteFocus::nothing();
        };
        let (path, line, dir) = if self.list_focus_target() == ListFocusTarget::None {
            if self.explorer_diff().is_some() {
                return self.remote_target_for_diff(line);
            }
            let Some(file) = self.tabs.active_file_view() else {
                return RemoteFocus::nothing();
            };
            (file.rel.clone(), u32::try_from(file.cursor + 1).ok(), false)
        } else {
            match self
                .explorer_resolved_cursor_row(tab.id)
                .filter(|row| !row.placeholder)
            {
                Some(row) => (row.rel, None, row.is_dir),
                None => return RemoteFocus::nothing(),
            }
        };
        match self.checkout_rev(&tab.checkout) {
            Some(rev) => RemoteFocus::view(
                tab.checkout.as_str(),
                RemoteTarget::File {
                    rev,
                    path,
                    line,
                    dir,
                },
            ),
            None => RemoteFocus::nothing(),
        }
    }

    /// Revision a file from `source` opens at: the commit of a commit drill
    /// or a compare tab ([`CommitFileSource::line_commit`]); otherwise (the
    /// worktree, or a commit-vs-working-tree tab) the checkout revision
    /// ([`Self::checkout_rev`]). A stash is local only.
    fn source_rev(
        &self,
        checkout: &str,
        source: Option<&CommitFileSource>,
    ) -> Result<FileRev, RemoteFocus> {
        if let Some(CommitFileSource::Stash { stash_ref }) = source {
            return Err(RemoteFocus::Refuse(stash_local_only_status(stash_ref)));
        }
        if let Some(sha) = source.and_then(CommitFileSource::line_commit) {
            return Ok(FileRev::Commit(sha.to_string()));
        }
        self.checkout_rev(checkout).ok_or_else(RemoteFocus::nothing)
    }

    /// "View remotely" target of tree row `row`.
    fn remote_target_for_tree_row(&self, row: &VisibleRow) -> RemoteFocus {
        let Some(checkout) = row.repo.as_deref() else {
            return RemoteFocus::nothing();
        };
        let path = match row.kind {
            NodeKind::Repo => return RemoteFocus::view(checkout, RemoteTarget::Repo),
            NodeKind::Checkout => return self.checkout_focus(checkout),
            NodeKind::Dir if !row.chrome.path.is_empty() => Some(row.chrome.path.clone()),
            NodeKind::Dir => dir_path_from_id(&row.id, checkout),
            NodeKind::File => row
                .file
                .as_ref()
                .map(|file| file.path.clone())
                .filter(|path| !path.is_empty())
                .or_else(|| Some(row.chrome.path.clone()).filter(|path| !path.is_empty())),
            NodeKind::Workspace | NodeKind::Group | NodeKind::Section => None,
        };
        match (path, self.checkout_rev(checkout)) {
            (Some(path), Some(rev)) => RemoteFocus::view(
                checkout,
                RemoteTarget::File {
                    rev,
                    path,
                    line: None,
                    dir: row.kind == NodeKind::Dir,
                },
            ),
            _ => RemoteFocus::nothing(),
        }
    }

    /// "View remotely" target of checkout `checkout`: its branch, or its
    /// HEAD commit when detached ([`Self::checkout_rev`]).
    fn checkout_focus(&self, checkout: &str) -> RemoteFocus {
        match self.checkout_rev(checkout) {
            Some(FileRev::Branch(branch)) => {
                RemoteFocus::view(checkout, RemoteTarget::Branch(branch))
            }
            Some(FileRev::Commit(sha)) => RemoteFocus::view(checkout, RemoteTarget::Commit(sha)),
            None => RemoteFocus::nothing(),
        }
    }

    /// "View remotely" target of the focused graph row.
    fn remote_target_for_graph_row(&self) -> RemoteFocus {
        match self.focused_graph_row() {
            Some(GraphRow::Commit { commit, .. }) => match self.focused_graph_repo() {
                Some(checkout) => RemoteFocus::view(checkout, RemoteTarget::Commit(commit.id)),
                None => RemoteFocus::nothing(),
            },
            Some(GraphRow::Worktree(worktree)) => worktree_focus(&worktree),
            Some(GraphRow::Stash(stash)) => {
                RemoteFocus::Refuse(stash_local_only_status(&stash.stash_ref))
            }
            Some(GraphRow::Uncommitted { .. }) => {
                RemoteFocus::Refuse(UNCOMMITTED_LOCAL_ONLY.to_string())
            }
            None => RemoteFocus::nothing(),
        }
    }

    /// Revision of checkout `checkout` in the snapshot: its branch, or its
    /// HEAD commit when detached. `None` when the checkout is not in the
    /// snapshot, or is detached with no known HEAD.
    fn checkout_rev(&self, checkout: &str) -> Option<FileRev> {
        let repo = self
            .snapshot
            .repos
            .iter()
            .find(|repo| Path::new(&repo.repo) == Path::new(checkout))?;
        if is_counted_local_branch(&repo.branch) {
            Some(FileRev::Branch(repo.branch.clone()))
        } else if repo.head.is_empty() {
            None
        } else {
            Some(FileRev::Commit(repo.head.clone()))
        }
    }

    /// "View remotely": open the forge page of the focused entity, or say
    /// why there is none.
    pub(super) fn view_remotely(&mut self) -> Effect {
        let focus = self.remote_target_for_focus();
        self.remote_focus_effect(focus)
    }

    /// The View remotely line of popover section `kind` on `target`: open
    /// the page of the entity that section describes
    /// ([`Self::remote_target_for_icon`]), or say why there is none.
    pub(super) fn view_remotely_at(&mut self, kind: IconKind, target: &IconTarget) -> Effect {
        let focus = self.remote_target_for_icon(kind, target);
        self.remote_focus_effect(focus)
    }

    /// The effect of `focus`; a refusal sets the warn status instead.
    fn remote_focus_effect(&mut self, focus: RemoteFocus) -> Effect {
        match focus {
            RemoteFocus::View { checkout, target } => Effect::ViewRemotely { checkout, target },
            RemoteFocus::Refuse(text) => {
                self.status = StatusMessage::warn(text);
                Effect::None
            }
        }
    }

    /// Land a "View remotely" result on the status line.
    ///
    /// `page` is the page URL, or the text that says why the entity has no
    /// page on the remote; `opened` is true when the browser started.
    pub(crate) fn apply_remote_view(&mut self, page: Result<String, String>, opened: bool) {
        self.status = match page {
            Ok(url) if opened => StatusMessage::ok(opened_status(&url)),
            Ok(_) => StatusMessage::error(OPEN_FAILED),
            Err(text) => StatusMessage::warn(text),
        };
    }
}

/// The new-side (post-image) line number of diff row `row`, for a line
/// anchor: an added or context line in either layout. `None` for a row
/// that shows only a deleted line, and for section, hunk, meta, and error
/// rows. The cell number alone is not enough: an inline deleted row shows
/// its old-side number.
fn new_side_line(row: &DiffRow) -> Option<u32> {
    let DiffRow::Line { left, right } = row else {
        return None;
    };
    // Split rows keep the new side on the right; inline rows have one cell.
    let cell = right.as_ref().unwrap_or(left);
    matches!(cell.kind, DiffCellKind::Add | DiffCellKind::Ctx)
        .then_some(cell.line_no)
        .flatten()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use workspace_status_graph::{Commit, GraphModel, Stash, Worktree};

    use super::super::super::command_palette::PALETTE_COMMANDS;
    use super::super::super::diff::{DiffCell, DiffContent};
    use super::super::super::drill::{CommitFile, DrillView};
    use super::super::super::split::DiffMode;
    use super::super::super::status::StatusKind;
    use super::super::FocusPane;
    use super::*;
    use crate::helpers::DETACHED_HEAD_BRANCH;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::action::Action;

    const HEAD: &str = "0123456789abcdef0123456789abcdef01234567";

    fn repo(name: &str, branch: &str) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: branch.into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            upstream_rewrite: None,
            head: HEAD.into(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: false,
            changes: vec![FileChange {
                path: "src/lib.rs".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    fn linked(name: &str, primary: &str, branch: &str) -> RepoSnapshot {
        let mut row = repo(name, branch);
        row.checkout_kind = CheckoutKind::Linked;
        row.primary_repo = Some(primary.into());
        row
    }

    fn app_with(repos: &[RepoSnapshot]) -> AppState {
        let snapshot = build_workspace_snapshot(repos, &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn focus(app: &mut AppState, id: &str) {
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"));
    }

    /// Dispatch "View remotely" with a clear status.
    fn gw(app: &mut AppState) -> Effect {
        app.status = StatusMessage::default();
        app.dispatch(Action::ViewRemotely)
    }

    fn view(checkout: &str, target: RemoteTarget) -> Effect {
        Effect::ViewRemotely {
            checkout: PathBuf::from(checkout),
            target,
        }
    }

    fn file_at(rev: FileRev, path: &str, dir: bool) -> RemoteTarget {
        RemoteTarget::File {
            rev,
            path: path.into(),
            line: None,
            dir,
        }
    }

    fn assert_refused(app: &mut AppState, text: &str, what: &str) {
        assert_eq!(gw(app), Effect::None, "{what}");
        assert_eq!(app.status, text, "{what}");
        assert_eq!(app.status.kind(), StatusKind::Warn, "{what}");
    }

    #[test]
    fn tree_repo_and_family_rows_open_the_repository_home() {
        let mut app = app_with(&[repo("solo", "feature")]);
        focus(&mut app, "repo:solo");
        assert_eq!(gw(&mut app), view("solo", RemoteTarget::Repo));

        let mut app = app_with(&[
            repo("app", "main"),
            linked("app/.worktrees/feat", "app", "feat"),
        ]);
        focus(&mut app, "repo:app");
        assert!(app.focused_row().unwrap().chrome.is_family);
        assert_eq!(gw(&mut app), view("app", RemoteTarget::Repo));
        assert!(app.status.is_empty());
    }

    #[test]
    fn tree_checkout_rows_open_the_branch_or_the_detached_head() {
        let mut app = app_with(&[
            repo("app", "main"),
            linked("app/.worktrees/feat", "app", "feat"),
            linked("app/.worktrees/loose", "app", DETACHED_HEAD_BRANCH),
        ]);
        focus(&mut app, "checkout:app/.worktrees/feat");
        assert_eq!(
            gw(&mut app),
            view("app/.worktrees/feat", RemoteTarget::Branch("feat".into()))
        );
        focus(&mut app, "checkout:app/.worktrees/loose");
        assert_eq!(
            gw(&mut app),
            view("app/.worktrees/loose", RemoteTarget::Commit(HEAD.into()))
        );
    }

    #[test]
    fn tree_file_and_dir_rows_open_the_path_at_the_checkout_revision() {
        let mut app = app_with(&[repo("app", "feature"), repo("det", DETACHED_HEAD_BRANCH)]);
        focus(&mut app, "dir:app:src");
        assert_eq!(
            gw(&mut app),
            view(
                "app",
                file_at(FileRev::Branch("feature".into()), "src", true)
            )
        );
        focus(&mut app, "file:app:src/lib.rs");
        assert_eq!(
            gw(&mut app),
            view(
                "app",
                file_at(FileRev::Branch("feature".into()), "src/lib.rs", false)
            )
        );
        focus(&mut app, "file:det:src/lib.rs");
        assert_eq!(
            gw(&mut app),
            view(
                "det",
                file_at(FileRev::Commit(HEAD.into()), "src/lib.rs", false)
            ),
            "a detached checkout opens the path at its HEAD"
        );
    }

    #[test]
    fn tree_rows_without_an_entity_have_nothing_to_view() {
        let mut clean = repo("clean", "main");
        clean.changes.clear();
        clean.has_unstaged = false;
        clean.sync_status = SyncStatus::UpToDate;
        let mut app = app_with(&[repo("app", "main"), clean]);
        app.cursor = 0;
        assert_eq!(app.focused_row().unwrap().kind, NodeKind::Workspace);
        assert_refused(&mut app, NOTHING_TO_VIEW, "workspace");
        focus(&mut app, "group:no-updates");
        assert_refused(&mut app, NOTHING_TO_VIEW, "group");
    }

    fn file_line(rev: FileRev, path: &str, line: Option<u32>) -> RemoteTarget {
        RemoteTarget::File {
            rev,
            path: path.into(),
            line,
            dir: false,
        }
    }

    fn feature() -> FileRev {
        FileRev::Branch("feature".into())
    }

    /// Palette gate of the "View remotely" row.
    fn palette_reason(app: &AppState) -> Option<String> {
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.action == Action::ViewRemotely)
            .expect("View remotely palette row");
        app.palette_disabled_reason(command)
    }

    /// Index of the diff row whose cell (either side) shows `text`.
    fn diff_row(app: &AppState, text: &str) -> usize {
        app.current_diff_rows()
            .iter()
            .position(|row| match row {
                DiffRow::Line { left, right } => {
                    left.text == text || right.as_ref().is_some_and(|cell| cell.text == text)
                }
                _ => false,
            })
            .unwrap_or_else(|| panic!("no diff row {text}"))
    }

    /// `src/lib.rs` keeps `keep`, changes `old` to `new`, and drops `gone`.
    const LIB_DIFF: &str = "@@ -1,4 +1,3 @@\n keep\n-old\n+new\n-gone\n tail\n";

    /// Workspace `app` on `feature` with the `src/lib.rs` diff focused.
    fn tree_diff_app() -> AppState {
        let mut app = app_with(&[repo("app", "feature")]);
        focus(&mut app, "file:app:src/lib.rs");
        app.focus = FocusPane::Right;
        app.set_diff(
            "app".into(),
            "src/lib.rs".into(),
            DiffContent::from_unified(LIB_DIFF),
        );
        assert_eq!(app.list_focus_target(), ListFocusTarget::None);
        app
    }

    #[test]
    fn inline_diff_line_anchors_on_the_new_side_only() {
        let mut app = tree_diff_app();
        app.diff_mode = DiffMode::Inline;
        let lib = |line| view("app", file_line(feature(), "src/lib.rs", line));
        app.diff_cursor = diff_row(&app, "keep");
        assert_eq!(gw(&mut app), lib(Some(1)), "context line");
        app.diff_cursor = diff_row(&app, "new");
        assert_eq!(gw(&mut app), lib(Some(2)), "added line");
        app.diff_cursor = diff_row(&app, "tail");
        assert_eq!(gw(&mut app), lib(Some(3)));
        app.diff_cursor = diff_row(&app, "old");
        assert_eq!(
            gw(&mut app),
            lib(None),
            "a deleted line has no new-side line, though its cell shows old 2"
        );
        app.diff_cursor = 0;
        assert_eq!(gw(&mut app), lib(None), "a section row");
        assert!(app.status.is_empty());
        assert_eq!(palette_reason(&app), None);
    }

    #[test]
    fn split_diff_line_anchors_on_the_new_side_only() {
        let mut app = tree_diff_app();
        app.diff_mode = DiffMode::SideBySide;
        app.layout.diff_pane_width = 200;
        assert_eq!(app.diff_layout(), DiffMode::SideBySide);
        let lib = |line| view("app", file_line(feature(), "src/lib.rs", line));
        app.diff_cursor = diff_row(&app, "new");
        assert_eq!(
            app.diff_cursor,
            diff_row(&app, "old"),
            "old and new pair up"
        );
        assert_eq!(gw(&mut app), lib(Some(2)), "the pair opens the new line");
        app.diff_cursor = diff_row(&app, "gone");
        assert_eq!(gw(&mut app), lib(None), "a deletion-only row");
        app.diff_cursor = diff_row(&app, "tail");
        assert_eq!(gw(&mut app), lib(Some(3)));
    }

    fn lib_files() -> Vec<CommitFile> {
        ["src/lib.rs", "src/main.rs"]
            .into_iter()
            .map(|path| CommitFile {
                status: "M".into(),
                path: path.into(),
                old_path: None,
                stat: None,
            })
            .collect()
    }

    /// A depth-2 drill of `source` in `app` with the `src/lib.rs` diff
    /// open and focused.
    fn drill_diff(app: &mut AppState, source: CommitFileSource) {
        app.drill = DrillView::Diff {
            repo: "app".into(),
            source,
            files: lib_files(),
            file_cursor: 0,
            path: "src/lib.rs".into(),
            content: DiffContent::from_unified(LIB_DIFF),
        };
        app.focus = FocusPane::Right;
        app.diff_mode = DiffMode::Inline;
        focus_commit_file(app, "src/lib.rs", false);
        app.diff_cursor = diff_row(app, "new");
    }

    /// Put the commit-file cursor on the file (or, with `dir`, folder) row
    /// of `path`.
    fn focus_commit_file(app: &mut AppState, path: &str, dir: bool) {
        let row = app
            .commit_file_rows()
            .iter()
            .position(|row| row.path == path && row.is_dir() == dir)
            .unwrap_or_else(|| panic!("no commit-file row {path}"));
        app.set_commit_file_cursor(row);
    }

    #[test]
    fn commit_drill_diff_and_file_list_open_at_the_commit() {
        let sha = "c".repeat(40);
        let mut app = app_with(&[repo("app", "feature")]);
        focus(&mut app, "repo:app");
        drill_diff(
            &mut app,
            CommitFileSource::Commit {
                commit_id: sha.clone(),
            },
        );
        let at_commit = || FileRev::Commit(sha.clone());
        assert_eq!(
            gw(&mut app),
            view("app", file_line(at_commit(), "src/lib.rs", Some(2)))
        );

        // The commit-file list: a file row, then its folder row.
        app.commit_tree_mode = true;
        app.focus = FocusPane::Left;
        assert_eq!(app.list_focus_target(), ListFocusTarget::CommitFiles);
        assert_eq!(
            gw(&mut app),
            view("app", file_at(at_commit(), "src/lib.rs", false))
        );
        focus_commit_file(&mut app, "src", true);
        assert_eq!(
            gw(&mut app),
            view("app", file_at(at_commit(), "src", true)),
            "a folder row opens the folder page"
        );
        assert_eq!(palette_reason(&app), None);
    }

    #[test]
    fn worktree_drill_opens_at_the_branch_and_stash_drill_is_local_only() {
        let mut app = app_with(&[repo("app", "feature")]);
        focus(&mut app, "repo:app");
        drill_diff(&mut app, CommitFileSource::Worktree);
        assert_eq!(
            gw(&mut app),
            view("app", file_line(feature(), "src/lib.rs", Some(2)))
        );

        drill_diff(
            &mut app,
            CommitFileSource::Stash {
                stash_ref: "stash@{1}".into(),
            },
        );
        assert_eq!(
            palette_reason(&app).as_deref(),
            Some("stash@{1} is local only")
        );
        assert_refused(&mut app, "stash@{1} is local only", "stash diff");
        app.focus = FocusPane::Left;
        assert_eq!(app.list_focus_target(), ListFocusTarget::CommitFiles);
        assert_refused(&mut app, "stash@{1} is local only", "stash file list");
    }

    #[test]
    fn compare_tab_files_and_diff_open_at_the_compare_head() {
        let mut app = app_with(&[repo("app", "feature")]);
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        assert_refused(&mut app, NOTHING_TO_VIEW, "compare range still loading");

        let tab = app.tabs.active_compare().expect("compare tab");
        let (tab_id, gen) = (tab.id, tab.generation);
        let source = CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: "b".repeat(40),
            merge_base: "a".repeat(40),
            head: "d".repeat(40),
        };
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(crate::tui::app::CompareRangeLoad {
                source: source.clone(),
                files: lib_files(),
                patch_keys: Default::default(),
                head: Some("d".repeat(40)),
                base_tip: "b".repeat(40),
            }),
        );
        focus_commit_file(&mut app, "src/lib.rs", false);
        assert_eq!(app.list_focus_target(), ListFocusTarget::CommitFiles);
        let head = || FileRev::Commit("d".repeat(40));
        assert_eq!(
            gw(&mut app),
            view("app", file_at(head(), "src/lib.rs", false))
        );

        app.apply_compare_diff(
            tab_id,
            gen,
            &source,
            "src/lib.rs",
            Ok(DiffContent::from_unified(LIB_DIFF)),
        );
        app.focus = FocusPane::Right;
        app.diff_mode = DiffMode::Inline;
        app.diff_cursor = diff_row(&app, "new");
        assert_eq!(
            gw(&mut app),
            view("app", file_line(head(), "src/lib.rs", Some(2)))
        );
    }

    #[test]
    fn the_refusal_gate_builds_no_diff_rows() {
        let mut app = tree_diff_app();
        app.diff_cursor = diff_row(&app, "new");
        app.diff_row_builds.set(0);
        assert_eq!(palette_reason(&app), None);
        assert_eq!(app.diff_row_builds.get(), 0, "the gate skips the line");
        assert_eq!(
            gw(&mut app),
            view("app", file_line(feature(), "src/lib.rs", Some(2)))
        );
        assert_eq!(app.diff_row_builds.get(), 1, "gw reads the line");
    }

    #[test]
    fn palette_row_is_off_where_gw_refuses() {
        let mut app = app_with(&[repo("app", "feature")]);
        app.cursor = 0;
        assert_eq!(palette_reason(&app).as_deref(), Some(NOTHING_TO_VIEW));
        focus(&mut app, "repo:app");
        assert_eq!(palette_reason(&app), None);
    }

    #[test]
    fn new_side_line_skips_old_only_and_non_line_rows() {
        let cell = |kind, line_no| DiffCell {
            kind,
            text: String::new(),
            line_no,
        };
        let inline = |kind, n| DiffRow::Line {
            left: cell(kind, Some(n)),
            right: None,
        };
        assert_eq!(new_side_line(&inline(DiffCellKind::Add, 4)), Some(4));
        assert_eq!(new_side_line(&inline(DiffCellKind::Ctx, 5)), Some(5));
        assert_eq!(new_side_line(&inline(DiffCellKind::Del, 6)), None);
        assert_eq!(new_side_line(&inline(DiffCellKind::Meta, 7)), None);
        let split = DiffRow::Line {
            left: cell(DiffCellKind::Del, Some(8)),
            right: Some(cell(DiffCellKind::Empty, None)),
        };
        assert_eq!(new_side_line(&split), None);
        let hunk = DiffRow::Hunk {
            text: "@@ -1 +1 @@".into(),
        };
        assert_eq!(new_side_line(&hunk), None);
    }

    #[test]
    fn graph_rows_open_commits_and_worktree_branches_and_refuse_local_rows() {
        let mut app = app_with(&[repo("app", "main")]);
        focus(&mut app, "repo:app");
        let worktree = |path: &str, branch: Option<&str>, head: Option<&str>| Worktree {
            path: path.into(),
            head_id: head.map(str::to_string),
            branch: branch.map(str::to_string),
            hidden: false,
            is_current: false,
        };
        app.graph = Some(GraphModel {
            uncommitted: Some(true),
            commits: vec![Commit {
                id: HEAD.into(),
                subject: "seed".into(),
                ..Commit::default()
            }],
            stashes: vec![Stash {
                id: "5".repeat(40),
                stash_ref: "stash@{0}".into(),
                parent_id: Some(HEAD.into()),
                ..Stash::default()
            }],
            worktrees: vec![
                worktree("app/.worktrees/feat", Some("feat"), None),
                worktree("app/.worktrees/loose", None, Some(&"b".repeat(40))),
                worktree("app/.worktrees/gone", None, None),
            ],
            window: 1,
            ..GraphModel::default()
        });
        app.focus = FocusPane::Right;
        assert!(app.graph_pane_focused());
        let rows = app.graph.as_ref().expect("graph").visible_rows();
        let at = |pick: &dyn Fn(&GraphRow) -> bool| rows.iter().position(pick).expect("row");

        app.graph_cursor = at(&|row| matches!(row, GraphRow::Commit { .. }));
        assert_eq!(gw(&mut app), view("app", RemoteTarget::Commit(HEAD.into())));

        app.graph_cursor = at(
            &|row| matches!(row, GraphRow::Worktree(wt) if wt.branch.as_deref() == Some("feat")),
        );
        assert_eq!(
            gw(&mut app),
            view("app/.worktrees/feat", RemoteTarget::Branch("feat".into()))
        );

        app.graph_cursor =
            at(&|row| matches!(row, GraphRow::Worktree(wt) if wt.path.ends_with("loose")));
        assert_eq!(
            gw(&mut app),
            view("app/.worktrees/loose", RemoteTarget::Commit("b".repeat(40))),
            "a detached worktree opens its HEAD commit"
        );

        app.graph_cursor =
            at(&|row| matches!(row, GraphRow::Worktree(wt) if wt.path.ends_with("gone")));
        assert_refused(&mut app, NOTHING_TO_VIEW, "worktree with no branch or head");

        app.graph_cursor = at(&|row| matches!(row, GraphRow::Stash(_)));
        assert_refused(&mut app, "stash@{0} is local only", "stash");

        app.graph_cursor = at(&|row| matches!(row, GraphRow::Uncommitted { .. }));
        assert_refused(&mut app, UNCOMMITTED_LOCAL_ONLY, "uncommitted");
    }

    #[test]
    fn apply_remote_view_reports_open_refusal_and_open_failure() {
        let mut app = app_with(&[repo("app", "main")]);
        let url = "https://github.com/octo/demo";
        app.apply_remote_view(Ok(url.into()), true);
        assert_eq!(app.status, opened_status(url));
        assert_eq!(app.status.kind(), StatusKind::Ok);

        app.apply_remote_view(Ok(url.into()), false);
        assert_eq!(app.status, OPEN_FAILED);
        assert_eq!(app.status.kind(), StatusKind::Error);

        app.apply_remote_view(Err("commit 0123456 is not on origin".into()), false);
        assert_eq!(app.status, "commit 0123456 is not on origin");
        assert_eq!(app.status.kind(), StatusKind::Warn);
    }
}

//! "View remotely": what the focused row opens on the forge.
//!
//! This module only names the entity ([`RemoteFocus`]). The remote check and
//! the browser open run on a worker ([`Effect::ViewRemotely`]); the result
//! lands in [`AppState::apply_remote_view`].

use std::path::{Path, PathBuf};

use workspace_status_graph::GraphRow;

use crate::helpers::is_counted_local_branch;

use super::super::action::Effect;
use super::super::gates::ListFocusTarget;
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
    /// Open the PR of a branch (checkout, branch), as `gx` does.
    // Only a PR badge in a popover gives this; drop the allow with that hook.
    #[allow(dead_code)]
    PullRequest(PathBuf, String),
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
}

impl AppState {
    /// What "View remotely" opens for the focused row.
    ///
    /// Tree: a repository row (a family row too) opens the repository home;
    /// a checkout row its branch, or its HEAD commit when detached; a file
    /// or directory row that path at the checkout branch (or HEAD commit).
    /// Graph: a commit row its commit; a worktree row its branch (or HEAD
    /// commit). A stash or the uncommitted row is local only. Every other
    /// row and surface has nothing to view.
    pub(super) fn remote_target_for_focus(&self) -> RemoteFocus {
        if self.is_explorer_tab() || self.is_compare_tab() {
            return RemoteFocus::nothing();
        }
        match self.list_focus_target() {
            ListFocusTarget::Tree => self.focused_row().map_or_else(RemoteFocus::nothing, |row| {
                self.remote_target_for_tree_row(row)
            }),
            ListFocusTarget::Graph => self.remote_target_for_graph_row(),
            ListFocusTarget::CommitFiles | ListFocusTarget::None => RemoteFocus::nothing(),
        }
    }

    /// "View remotely" target of tree row `row`.
    fn remote_target_for_tree_row(&self, row: &VisibleRow) -> RemoteFocus {
        let Some(checkout) = row.repo.as_deref() else {
            return RemoteFocus::nothing();
        };
        let path = match row.kind {
            NodeKind::Repo => return RemoteFocus::view(checkout, RemoteTarget::Repo),
            NodeKind::Checkout => {
                return match self.checkout_rev(checkout) {
                    Some(FileRev::Branch(branch)) => {
                        RemoteFocus::view(checkout, RemoteTarget::Branch(branch))
                    }
                    Some(FileRev::Commit(sha)) => {
                        RemoteFocus::view(checkout, RemoteTarget::Commit(sha))
                    }
                    None => RemoteFocus::nothing(),
                };
            }
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

    /// "View remotely" target of the focused graph row.
    fn remote_target_for_graph_row(&self) -> RemoteFocus {
        match self.focused_graph_row() {
            Some(GraphRow::Commit { commit, .. }) => match self.focused_graph_repo() {
                Some(checkout) => RemoteFocus::view(checkout, RemoteTarget::Commit(commit.id)),
                None => RemoteFocus::nothing(),
            },
            Some(GraphRow::Worktree(worktree)) => {
                let target = match (worktree.branch, worktree.head_id) {
                    (Some(branch), _) if is_counted_local_branch(&branch) => {
                        RemoteTarget::Branch(branch)
                    }
                    (_, Some(sha)) if !sha.is_empty() => RemoteTarget::Commit(sha),
                    _ => return RemoteFocus::nothing(),
                };
                RemoteFocus::view(worktree.path, target)
            }
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
        match self.remote_target_for_focus() {
            RemoteFocus::View { checkout, target } => Effect::ViewRemotely { checkout, target },
            RemoteFocus::PullRequest(repo, branch) => Effect::OpenPullRequest { repo, branch },
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use workspace_status_graph::{Commit, GraphModel, Stash, Worktree};

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

    #[test]
    fn explorer_tab_has_nothing_to_view_for_now() {
        let mut app = app_with(&[repo("app", "feature")]);
        focus(&mut app, "repo:app");
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        assert!(app.is_explorer_tab());
        assert_refused(&mut app, NOTHING_TO_VIEW, "Explorer file");
    }

    #[test]
    fn graph_rows_open_commits_and_worktree_branches_and_refuse_local_rows() {
        let mut app = app_with(&[repo("app", "main")]);
        focus(&mut app, "repo:app");
        let worktree = |path: &str, branch: Option<&str>, head: Option<&str>| Worktree {
            path: path.into(),
            head_id: head.map(str::to_string),
            branch: branch.map(str::to_string),
            ignored: false,
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

//! PR of a branch row: the `gx` target and the badge cache.
//!
//! The forge CLI runs on a worker ([`Effect::OpenPullRequest`],
//! [`Effect::LookupPullRequests`]). This module decides what to look up and
//! keeps the answers. Answers are cached per (remote URL, branch). Each
//! checkout remembers the branch it was looked up for and the remote URL the
//! worker resolved, so a badge never shows an answer for another branch.
//!
//! A checkout is looked up again only when its branch is new or changed, and
//! after `r` forgets it. A watch tick that re-applies the same branch does
//! not ask the forge again.
//!
//! The PR popover shows richer fields ([`PullRequestDetail`]). They are
//! fetched once per (remote URL, branch), when a peek or pin first shows
//! that PR ([`AppState::request_popover_pr_details`]), never on a tick. A
//! branch switch, `r`, a vanished checkout, or a new answer for the branch
//! drops the detail.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use workspace_status_graph::{GraphRow, Worktree};

use crate::helpers::is_counted_local_branch;

use super::super::action::Effect;
use super::super::gates::ListFocusTarget;
use super::super::popover::IconTarget;
use super::super::pull_request::{
    lookup_failed_status, no_pr_status, PrDetailLookup, PrLookup, PrState, PullRequest,
    PullRequestDetail, NO_PR_FOR_ROW, OPEN_FAILED,
};
use super::super::status::StatusMessage;
use super::super::tree::NodeKind;
use super::AppState;

/// What the forge said about the PR of one (remote URL, branch).
#[derive(Clone, Debug, PartialEq, Eq)]
enum BranchPr {
    /// The forge answered: the PR that counts, or none.
    Ready(Option<PullRequest>),
    /// The CLI could not answer (missing, not signed in, bad output).
    Failed,
}

impl From<PrLookup> for BranchPr {
    fn from(lookup: PrLookup) -> Self {
        match lookup {
            PrLookup::Found(pr) => Self::Ready(Some(pr)),
            PrLookup::NoPr => Self::Ready(None),
            PrLookup::Failed => Self::Failed,
        }
    }
}

/// The branch one checkout was looked up for.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CheckoutPr {
    branch: String,
    /// Remote URL the lookup resolved. `None` while the lookup is in flight,
    /// and when the checkout has no remote (then there is no PR either).
    remote: Option<String>,
}

/// Detail fetch of one PR for the PR popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PrDetailState {
    /// Request with this id is out; the worker has not answered. Only the
    /// answer to this request lands.
    Loading(u64),
    /// The forge described the PR.
    Ready(PullRequestDetail),
    /// The fetch failed. Not retried until the detail is dropped.
    Failed,
}

impl From<PrDetailLookup> for PrDetailState {
    fn from(lookup: PrDetailLookup) -> Self {
        match lookup {
            PrDetailLookup::Found(detail) => Self::Ready(detail),
            PrDetailLookup::Failed => Self::Failed,
        }
    }
}

/// PR answers for the badge on branch rows, and PR details for the popover.
#[derive(Clone, Debug, Default)]
pub(crate) struct PrCache {
    /// Checkout path (snapshot `repo`) to the branch it was looked up for.
    checkouts: HashMap<PathBuf, CheckoutPr>,
    /// (remote URL, branch) to the forge's answer.
    answers: HashMap<(String, String), BranchPr>,
    /// (remote URL, branch) to the detail of the PR its answer names.
    details: HashMap<(String, String), PrDetailState>,
    /// Id of the last detail request.
    last_detail_request: u64,
}

impl PrCache {
    /// Drop answers and details that no checkout points at any more.
    fn prune_answers(&mut self) {
        let live: HashSet<(&str, &str)> = self
            .checkouts
            .values()
            .filter_map(|checkout| Some((checkout.remote.as_deref()?, checkout.branch.as_str())))
            .collect();
        let keep = |(remote, branch): &(String, String)| {
            live.contains(&(remote.as_str(), branch.as_str()))
        };
        self.answers.retain(|key, _| keep(key));
        self.details.retain(|key, _| keep(key));
    }

    /// Store `answer` for `key`. A different answer drops the detail, which
    /// described the old one.
    fn set_answer(&mut self, key: (String, String), answer: BranchPr) {
        if self.answers.get(&key) != Some(&answer) {
            self.details.remove(&key);
        }
        self.answers.insert(key, answer);
    }
}

impl AppState {
    /// Checkout and branch `gx` opens a PR for, or `None`.
    ///
    /// The focused tree repo / checkout row with a branch, or the focused
    /// graph worktree row with a branch. A file or compare tab, commit files,
    /// a diff, a commit, stash, dir, or file row, a repo that groups several
    /// checkouts, and a detached HEAD have no target.
    pub(super) fn pr_target_for_focus(&self) -> Option<(PathBuf, String)> {
        if self.is_file_tab() || self.is_compare_tab() {
            return None;
        }
        match self.list_focus_target() {
            ListFocusTarget::Tree => {
                let row = self.focused_row()?;
                if !matches!(row.kind, NodeKind::Repo | NodeKind::Checkout) || row.chrome.is_family
                {
                    return None;
                }
                self.pr_target_for_repo(Path::new(row.repo.as_deref()?))
            }
            ListFocusTarget::Graph => match self.focused_graph_row()? {
                GraphRow::Worktree(worktree) => {
                    let branch = worktree
                        .branch
                        .filter(|branch| is_counted_local_branch(branch))?;
                    Some((PathBuf::from(worktree.path), branch))
                }
                _ => None,
            },
            ListFocusTarget::CommitFiles | ListFocusTarget::None => None,
        }
    }

    /// Checkout and branch for a PR of checkout `repo`, from the snapshot.
    ///
    /// `None` when `repo` is not a checkout in the snapshot, or when its
    /// HEAD is detached or unknown.
    fn pr_target_for_repo(&self, repo: &Path) -> Option<(PathBuf, String)> {
        let branch = self.snapshot_branch(repo)?;
        is_counted_local_branch(branch).then(|| (repo.to_path_buf(), branch.to_string()))
    }

    /// `gx`: open the PR of the focused branch row, or say there is none.
    pub(super) fn open_pull_request(&mut self) -> Effect {
        match self.pr_target_for_focus() {
            Some((repo, branch)) => Effect::OpenPullRequest { repo, branch },
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                Effect::None
            }
        }
    }

    /// Ctrl+click: on a painted PR badge, select the row and open its PR.
    ///
    /// The row is selected exactly as a plain click selects it, and the
    /// click's own effect (the right-pane load) still runs. It pins no
    /// popover. Off every badge this is a plain click and never opens a
    /// browser.
    pub(super) fn ctrl_click(&mut self, col: u16, row: u16) -> Effect {
        let badge = self
            .layout
            .pr_badge_hits()
            .into_iter()
            .find(|hit| hit.contains(col, row))
            .map(|hit| hit.repo);
        let Some(repo) = badge else {
            return self.click(col, row);
        };
        if let Some(effect) = self.popover_click(col, row) {
            return effect;
        }
        let selected = self.click_at(col, row);
        let open = match self.pr_target_for_repo(&repo) {
            Some((repo, branch)) => {
                // A second Ctrl+click on the badge opens again; it is not
                // a double-click that drills into the row.
                self.last_click = None;
                Effect::OpenPullRequest { repo, branch }
            }
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                return selected;
            }
        };
        super::popover::then(selected, open)
    }

    /// The PR the badge of checkout `repo` shows, under the same rule as
    /// [`Self::pr_badge`].
    pub(crate) fn pull_request_for(&self, repo: &Path) -> Option<&PullRequest> {
        let checkout = self.pr_cache.checkouts.get(repo)?;
        if self.snapshot_branch(repo) != Some(checkout.branch.as_str()) {
            return None;
        }
        let key = (checkout.remote.clone()?, checkout.branch.clone());
        match self.pr_cache.answers.get(&key)? {
            BranchPr::Ready(Some(pr)) => Some(pr),
            BranchPr::Ready(None) | BranchPr::Failed => None,
        }
    }

    /// PR state for the badge of checkout `repo`.
    ///
    /// `Some` only when the cached answer is for the branch the checkout has
    /// now and names a PR. A lookup in flight, a failed lookup, no PR, or an
    /// answer for another branch give `None`.
    pub(crate) fn pr_badge(&self, repo: &Path) -> Option<PrState> {
        self.pull_request_for(repo).map(|pr| pr.state)
    }

    /// Checkouts whose PR badge needs a lookup now, marked in flight.
    ///
    /// Syncs the cache with the snapshot. A checkout with a new or changed
    /// branch is returned. A checkout already looked up for its current
    /// branch is left alone. A checkout that vanished, has a detached HEAD,
    /// or is a hidden ignored repo is dropped.
    pub(crate) fn due_pr_lookups(&mut self) -> Vec<(PathBuf, String)> {
        let wanted: Vec<(PathBuf, String)> = self
            .snapshot
            .repos
            .iter()
            .filter(|row| self.show_ignored || !row.ignored)
            .filter(|row| is_counted_local_branch(&row.branch))
            .map(|row| (PathBuf::from(&row.repo), row.branch.clone()))
            .collect();
        let cache = &mut self.pr_cache;
        let live: HashSet<&PathBuf> = wanted.iter().map(|(path, _)| path).collect();
        cache.checkouts.retain(|path, _| live.contains(path));
        let mut due = Vec::new();
        for (path, branch) in &wanted {
            if cache
                .checkouts
                .get(path)
                .is_some_and(|checkout| &checkout.branch == branch)
            {
                continue;
            }
            cache.checkouts.insert(
                path.clone(),
                CheckoutPr {
                    branch: branch.clone(),
                    remote: None,
                },
            );
            due.push((path.clone(), branch.clone()));
        }
        cache.prune_answers();
        due
    }

    /// Forget PR answers so the next applied snapshot looks them up again.
    ///
    /// `r` on a checkout passes it; a full reload passes `None` (every
    /// checkout).
    ///
    /// The PR detail of that checkout is dropped too, so the next peek or
    /// pin fetches it again.
    pub(crate) fn forget_pr_lookups(&mut self, repo: Option<&Path>) {
        let cache = &mut self.pr_cache;
        match repo {
            Some(repo) => {
                if let Some(CheckoutPr {
                    branch,
                    remote: Some(remote),
                }) = cache.checkouts.remove(repo)
                {
                    cache.details.remove(&(remote, branch));
                }
            }
            None => {
                cache.checkouts.clear();
                cache.details.clear();
            }
        }
        cache.prune_answers();
    }

    /// Land a badge lookup of `branch` in checkout `repo`.
    ///
    /// Dropped when the checkout is on another branch now, or was forgotten
    /// since the lookup was queued. A failed lookup shows no badge and no
    /// status. Returns true when the cache changed.
    pub(crate) fn apply_pr_lookup(
        &mut self,
        repo: &Path,
        branch: &str,
        remote: Option<String>,
        lookup: PrLookup,
    ) -> bool {
        if self.snapshot_branch(repo) != Some(branch) {
            return false;
        }
        let Some(checkout) = self
            .pr_cache
            .checkouts
            .get_mut(repo)
            .filter(|checkout| checkout.branch == branch)
        else {
            return false;
        };
        checkout.remote.clone_from(&remote);
        if let Some(remote) = remote {
            self.pr_cache
                .set_answer((remote, branch.to_string()), lookup.into());
        }
        true
    }

    /// Land a `gx` result: say what happened, and cache the fresh answer.
    ///
    /// `opened` is true when the browser started for a found PR.
    pub(crate) fn apply_pr_open(
        &mut self,
        repo: &Path,
        branch: &str,
        remote: Option<String>,
        lookup: PrLookup,
        opened: bool,
    ) {
        self.status = match &lookup {
            PrLookup::Found(pr) if opened => StatusMessage::ok(format!("opened PR #{}", pr.number)),
            PrLookup::Found(_) => StatusMessage::error(OPEN_FAILED),
            PrLookup::NoPr => StatusMessage::warn(no_pr_status(branch)),
            PrLookup::Failed => StatusMessage::error(lookup_failed_status(branch)),
        };
        if self.snapshot_branch(repo) == Some(branch) {
            self.pr_cache.checkouts.insert(
                repo.to_path_buf(),
                CheckoutPr {
                    branch: branch.to_string(),
                    remote: remote.clone(),
                },
            );
        }
        if let Some(remote) = remote {
            self.pr_cache
                .set_answer((remote, branch.to_string()), lookup.into());
        }
    }

    /// Detail state of the PR the badge of checkout `repo` shows, under the
    /// rule of [`Self::pull_request_for`]. `None` when no detail was
    /// requested since the last drop.
    pub(crate) fn pull_request_detail_for(&self, repo: &Path) -> Option<&PrDetailState> {
        self.pull_request_for(repo)?;
        let checkout = self.pr_cache.checkouts.get(repo)?;
        let key = (checkout.remote.clone()?, checkout.branch.clone());
        self.pr_cache.details.get(&key)
    }

    /// Detail fetches for the PRs the open popover shows, each marked
    /// loading.
    ///
    /// A PR whose (remote URL, branch) already has a detail (loading, ready,
    /// or failed) is not asked again, so a later peek or pin reads the
    /// cache. Called when a peek opens or a popover pins; never on a tick.
    pub(crate) fn request_popover_pr_details(&mut self) -> Effect {
        let Some(popover) = self.popover.as_ref() else {
            return Effect::None;
        };
        let repos: Vec<PathBuf> = popover
            .targets
            .iter()
            .filter_map(|(_, target)| match target {
                IconTarget::PullRequest(repo) => Some(repo.clone()),
                _ => None,
            })
            .collect();
        let mut effects = Vec::new();
        for repo in repos {
            let Some(number) = self.pull_request_for(&repo).map(|pr| pr.number) else {
                continue;
            };
            let Some(CheckoutPr {
                branch,
                remote: Some(remote),
            }) = self.pr_cache.checkouts.get(&repo).cloned()
            else {
                continue;
            };
            let key = (remote.clone(), branch.clone());
            if self.pr_cache.details.contains_key(&key) {
                continue;
            }
            self.pr_cache.last_detail_request += 1;
            let request = self.pr_cache.last_detail_request;
            self.pr_cache
                .details
                .insert(key, PrDetailState::Loading(request));
            effects.push(Effect::LookupPullRequestDetail {
                repo,
                branch,
                remote,
                number,
                request,
            });
        }
        match effects.len() {
            0 => Effect::None,
            1 => effects.remove(0),
            _ => Effect::Batch(effects),
        }
    }

    /// Land detail request `request` of PR `number` for `branch` of
    /// checkout `repo` at `remote`.
    ///
    /// Dropped when the checkout is on another branch now, was forgotten,
    /// resolves another remote, its answer names another PR, or the
    /// detail no longer waits for this request (dropped since, or asked
    /// again: a newer request wins over a late older one). Returns true
    /// when the cache changed.
    pub(crate) fn apply_pr_detail(
        &mut self,
        repo: &Path,
        branch: &str,
        remote: &str,
        number: u64,
        request: u64,
        lookup: PrDetailLookup,
    ) -> bool {
        if self.pull_request_for(repo).map(|pr| pr.number) != Some(number) {
            return false;
        }
        let current = self.pr_cache.checkouts.get(repo).is_some_and(|checkout| {
            checkout.branch == branch && checkout.remote.as_deref() == Some(remote)
        });
        if !current {
            return false;
        }
        let key = (remote.to_string(), branch.to_string());
        if self.pr_cache.details.get(&key) != Some(&PrDetailState::Loading(request)) {
            return false;
        }
        // The detail adds lines above Open PR: keep the pinned focus on
        // the line it was on.
        let focus = self.popover_focus_key();
        self.pr_cache.details.insert(key, lookup.into());
        self.restore_popover_focus(focus);
        true
    }

    /// Graph worktree rows that show a PR badge: visible-row index,
    /// checkout path, and PR state.
    ///
    /// A row is badged only when the branch it paints is the checkout's
    /// snapshot branch, the branch the cached answer is for. A graph loaded
    /// before a branch switch still paints the old branch, so it gets no
    /// badge. Commit rows never get one.
    pub(crate) fn graph_pr_badges(&self) -> Vec<(usize, PathBuf, PrState)> {
        let Some(model) = self.graph.as_ref() else {
            return Vec::new();
        };
        let badge = |worktree: &Worktree| {
            let path = Path::new(&worktree.path);
            let branch = worktree.branch.as_deref()?;
            if self.snapshot_branch(path) != Some(branch) {
                return None;
            }
            self.pr_badge(path)
        };
        // Skip building the row list on the common frame with no badge.
        if !model.worktrees.iter().any(|wt| badge(wt).is_some()) {
            return Vec::new();
        }
        model
            .visible_rows()
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                GraphRow::Worktree(worktree) => {
                    badge(worktree).map(|state| (index, PathBuf::from(&worktree.path), state))
                }
                _ => None,
            })
            .collect()
    }

    /// Branch of checkout `repo` in the snapshot.
    fn snapshot_branch(&self, repo: &Path) -> Option<&str> {
        self.snapshot
            .repos
            .iter()
            .find(|row| Path::new(&row.repo) == repo)
            .map(|row| row.branch.as_str())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use workspace_status_graph::{Commit, GraphModel, Worktree};

    use super::super::super::app::apply_one_repo_snapshot;
    use super::super::super::icons::IconKind;
    use super::super::super::popover::IconTarget;
    use super::super::super::status::StatusKind;
    use super::super::{FocusPane, IconHit};
    use super::*;
    use crate::helpers::DETACHED_HEAD_BRANCH;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::action::Action;

    fn repo(name: &str, branch: &str) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: branch.into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: "abc".into(),
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

    fn app_with(repos: &[RepoSnapshot], ignored: &[&str]) -> AppState {
        let ignored: Vec<String> = ignored.iter().map(|name| name.to_string()).collect();
        let snapshot = build_workspace_snapshot(repos, &ignored, false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn focus(app: &mut AppState, id: &str) {
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"));
    }

    fn gx(app: &mut AppState) -> Effect {
        app.status = StatusMessage::default();
        app.dispatch(Action::OpenPullRequest)
    }

    fn open(repo: &str, branch: &str) -> Effect {
        Effect::OpenPullRequest {
            repo: PathBuf::from(repo),
            branch: branch.into(),
        }
    }

    fn assert_no_pr_for_row(app: &mut AppState, what: &str) {
        assert_eq!(gx(app), Effect::None, "{what}");
        assert_eq!(app.status, NO_PR_FOR_ROW, "{what}");
        assert_eq!(app.status.kind(), StatusKind::Warn, "{what}");
    }

    fn found(state: PrState) -> PrLookup {
        PrLookup::Found(PullRequest {
            number: 7,
            url: "https://github.com/octo/demo/pull/7".into(),
            state,
        })
    }

    const REMOTE: &str = "git@github.com:octo/demo.git";

    #[test]
    fn gx_on_a_checkout_row_opens_that_branch() {
        let mut app = app_with(&[repo("solo", "feature")], &[]);
        focus(&mut app, "repo:solo");
        assert_eq!(gx(&mut app), open("solo", "feature"));

        let mut app = app_with(
            &[
                repo("app", "main"),
                linked("app/.worktrees/feat", "app", "feat"),
            ],
            &[],
        );
        focus(&mut app, "checkout:app");
        assert_eq!(gx(&mut app), open("app", "main"));
        focus(&mut app, "checkout:app/.worktrees/feat");
        assert_eq!(gx(&mut app), open("app/.worktrees/feat", "feat"));
        assert!(app.status.is_empty());
    }

    #[test]
    fn gx_on_rows_without_a_branch_says_no_pr_for_this_row() {
        let mut app = app_with(
            &[
                repo("app", "main"),
                linked("app/.worktrees/feat", "app", "feat"),
                repo("det", DETACHED_HEAD_BRANCH),
            ],
            &[],
        );
        focus(&mut app, "repo:app");
        assert_no_pr_for_row(&mut app, "repo that groups checkouts");
        focus(&mut app, "repo:det");
        assert_no_pr_for_row(&mut app, "detached HEAD");
        focus(&mut app, "dir:det:src");
        assert_no_pr_for_row(&mut app, "dir");
        let file = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.cursor = file;
        assert_no_pr_for_row(&mut app, "file");
        app.cursor = 0;
        assert_no_pr_for_row(&mut app, "workspace");
    }

    #[test]
    fn gx_on_a_file_tab_says_no_pr_for_this_row() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        focus(&mut app, "repo:app");
        let _ = app.open_file_tab("app".into(), "src/lib.rs".into());
        assert!(app.is_file_tab());
        assert_no_pr_for_row(&mut app, "file tab");
    }

    #[test]
    fn gx_in_the_graph_opens_worktree_rows_with_a_branch_only() {
        let mut app = app_with(&[repo("app", "main")], &[]);
        focus(&mut app, "repo:app");
        let worktree = |path: &str, branch: Option<&str>| Worktree {
            path: path.into(),
            head_id: None,
            branch: branch.map(str::to_string),
            ignored: false,
            is_current: false,
        };
        app.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa".into(),
                subject: "seed".into(),
                ..Commit::default()
            }],
            worktrees: vec![
                worktree("app/.worktrees/feat", Some("feat")),
                worktree("app/.worktrees/loose", None),
            ],
            window: 1,
            ..GraphModel::default()
        });
        app.focus = FocusPane::Right;
        assert!(app.graph_pane_focused());
        let rows = app.graph.as_ref().expect("graph").visible_rows();
        let at = |pick: &dyn Fn(&GraphRow) -> bool| rows.iter().position(pick).expect("row");

        app.graph_cursor = at(
            &|row| matches!(row, GraphRow::Worktree(wt) if wt.branch.as_deref() == Some("feat")),
        );
        assert_eq!(gx(&mut app), open("app/.worktrees/feat", "feat"));

        app.graph_cursor = at(&|row| matches!(row, GraphRow::Commit { .. }));
        assert_no_pr_for_row(&mut app, "graph commit");

        app.graph_cursor = at(&|row| matches!(row, GraphRow::Worktree(wt) if wt.branch.is_none()));
        assert_no_pr_for_row(&mut app, "detached graph worktree");
    }

    #[test]
    fn badge_shows_only_a_ready_pr_for_the_current_branch() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let path = Path::new("app");
        assert_eq!(
            app.due_pr_lookups(),
            vec![(PathBuf::from("app"), "feature".to_string())]
        );
        assert_eq!(app.pr_badge(path), None, "in flight");

        for state in [PrState::Open, PrState::Approved, PrState::Merged] {
            assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(state)));
            assert_eq!(app.pr_badge(path), Some(state));
        }
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::NoPr));
        assert_eq!(app.pr_badge(path), None, "no PR");
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::Failed));
        assert_eq!(app.pr_badge(path), None, "failed lookup");
        assert!(app.status.is_empty(), "a background failure stays silent");
        assert!(app.apply_pr_lookup(path, "feature", None, PrLookup::NoPr));
        assert_eq!(app.pr_badge(path), None, "no remote");
    }

    #[test]
    fn a_lookup_for_an_old_branch_or_a_forgotten_checkout_is_dropped() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
        assert!(!app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(PrState::Open)));
        assert_eq!(app.pr_badge(path), None);
        assert_eq!(
            app.due_pr_lookups(),
            vec![(PathBuf::from("app"), "other".to_string())],
            "the changed branch is looked up"
        );

        app.forget_pr_lookups(Some(path));
        assert!(!app.apply_pr_lookup(path, "other", Some(REMOTE.into()), found(PrState::Open)));
        assert_eq!(app.pr_badge(path), None);
    }

    #[test]
    fn a_cached_answer_is_hidden_once_the_checkout_leaves_its_branch() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(PrState::Open)));
        apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
        assert_eq!(app.pr_badge(path), None, "answer is for another branch");
    }

    #[test]
    fn due_lookups_skip_detached_and_hidden_ignored_and_drop_vanished_checkouts() {
        let mut app = app_with(
            &[
                repo("app", "feature"),
                repo("det", DETACHED_HEAD_BRANCH),
                repo("vendor", "main"),
            ],
            &["vendor"],
        );
        assert_eq!(
            app.due_pr_lookups(),
            vec![(PathBuf::from("app"), "feature".to_string())]
        );
        assert!(app.due_pr_lookups().is_empty(), "same branch again: none");

        let path = Path::new("app");
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(PrState::Open)));
        apply_one_repo_snapshot(&mut app, "app", None);
        assert!(app.due_pr_lookups().is_empty());
        assert!(
            app.pr_cache.checkouts.is_empty(),
            "vanished checkout dropped"
        );
        assert!(app.pr_cache.answers.is_empty(), "its answer too");
    }

    #[test]
    fn open_result_sets_status_and_refreshes_the_cache() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let path = Path::new("app");
        app.apply_pr_open(
            path,
            "feature",
            Some(REMOTE.into()),
            found(PrState::Merged),
            true,
        );
        assert_eq!(app.status, "opened PR #7");
        assert_eq!(app.status.kind(), StatusKind::Ok);
        assert_eq!(app.pr_badge(path), Some(PrState::Merged));
        assert!(
            app.due_pr_lookups().is_empty(),
            "the open answered this branch"
        );

        app.apply_pr_open(path, "feature", Some(REMOTE.into()), PrLookup::NoPr, false);
        assert_eq!(app.status, "no PR for feature");
        assert_eq!(app.pr_badge(path), None, "fresh answer replaces the badge");
    }

    /// Screen row of tree row `id` with the default layout (no scroll).
    fn tree_y(app: &AppState, id: &str) -> u16 {
        let index = app
            .rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"));
        app.layout.tree_y + u16::try_from(index).expect("small tree")
    }

    fn badge_at(app: &mut AppState, repo: &str, x: u16, y: u16) {
        app.layout.icon_hits = vec![IconHit {
            y,
            x,
            width: 2,
            kind: IconKind::PrOpen,
            target: IconTarget::PullRequest(PathBuf::from(repo)),
        }];
    }

    fn focused_id(app: &AppState) -> String {
        app.focused_row().expect("focused row").id.clone()
    }

    #[test]
    fn ctrl_click_on_a_badge_selects_the_row_and_opens_its_pr() {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        focus(&mut app, "repo:lib");

        for col in [20, 21] {
            focus(&mut app, "repo:lib");
            app.last_click = None;
            assert_eq!(
                app.dispatch(Action::CtrlClick { col, row: y }),
                Effect::Batch(vec![Effect::LoadRightPane, open("app", "feature")]),
                "col {col}"
            );
            assert_eq!(focused_id(&app), "repo:app", "col {col}");
            assert_eq!(app.focus, FocusPane::Left);
        }
    }

    #[test]
    fn a_second_ctrl_click_on_a_badge_opens_again_without_drilling() {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        focus(&mut app, "repo:lib");
        app.last_click = None;
        // Control: two plain clicks in the window are a double-click, which
        // moves focus to the right pane. They land off the badge: a plain
        // click on an icon pins its popover and never drills.
        app.dispatch(Action::Click { col: 10, row: y });
        app.dispatch(Action::Click { col: 10, row: y });
        assert_eq!(app.focus, FocusPane::Right);

        focus(&mut app, "repo:lib");
        app.focus = FocusPane::Left;
        app.last_click = None;
        let first = app.dispatch(Action::CtrlClick { col: 20, row: y });
        assert!(
            app.last_click.is_none(),
            "an opening Ctrl+click arms no double-click"
        );
        let second = app.dispatch(Action::CtrlClick { col: 20, row: y });
        let single = Effect::Batch(vec![Effect::LoadRightPane, open("app", "feature")]);
        assert_eq!(first, single);
        assert_eq!(second, single, "a single click again, not a double-click");
        assert_eq!(app.focus, FocusPane::Left, "no drill");
        assert_eq!(focused_id(&app), "repo:app");
    }

    #[test]
    fn ctrl_click_off_the_badge_is_a_plain_click() {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        for col in [10, 19, 22] {
            focus(&mut app, "repo:lib");
            app.last_click = None;
            let ctrl = app.dispatch(Action::CtrlClick { col, row: y });
            let ctrl_focus = focused_id(&app);
            focus(&mut app, "repo:lib");
            app.last_click = None;
            let plain = app.dispatch(Action::Click { col, row: y });
            assert_eq!(ctrl, Effect::LoadRightPane, "col {col}");
            assert_eq!(ctrl, plain, "col {col}");
            assert_eq!(ctrl_focus, focused_id(&app), "col {col}");
            assert_eq!(ctrl_focus, "repo:app", "col {col}");
        }
        // The badge on another row does not open from this row.
        let lib_y = tree_y(&app, "repo:lib");
        app.last_click = None;
        assert_eq!(
            app.dispatch(Action::CtrlClick {
                col: 20,
                row: lib_y
            }),
            Effect::LoadRightPane
        );
        assert!(app.status.is_empty());
    }

    #[test]
    fn plain_click_on_a_badge_only_selects() {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        focus(&mut app, "repo:lib");
        assert_eq!(
            app.dispatch(Action::Click { col: 20, row: y }),
            Effect::LoadRightPane
        );
        assert_eq!(focused_id(&app), "repo:app");
    }

    #[test]
    fn ctrl_click_with_the_mouse_off_does_nothing() {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        focus(&mut app, "repo:lib");
        app.mouse_enabled = false;
        assert_eq!(
            app.dispatch(Action::CtrlClick { col: 20, row: y }),
            Effect::None
        );
        assert_eq!(focused_id(&app), "repo:lib");
    }

    #[test]
    fn ctrl_click_on_a_badge_without_a_branch_says_no_pr_for_this_row() {
        let mut app = app_with(
            &[repo("app", DETACHED_HEAD_BRANCH), repo("lib", "main")],
            &[],
        );
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        focus(&mut app, "repo:lib");
        assert_eq!(
            app.dispatch(Action::CtrlClick { col: 20, row: y }),
            Effect::LoadRightPane
        );
        assert_eq!(focused_id(&app), "repo:app");
        assert_eq!(app.status, NO_PR_FOR_ROW);
        assert_eq!(app.status.kind(), StatusKind::Warn);
    }

    // ---- PR detail for the popover ----

    use super::super::super::popover::PEEK_DWELL_MS;
    use super::super::super::pull_request::{ChecksSummary, PullRequestDetail};

    /// `app` on `feature` with a ready open PR #`number` in the badge cache.
    fn app_with_pr(number: u64) -> AppState {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let _ = app.due_pr_lookups();
        set_answer(&mut app, number, PrState::Open);
        app
    }

    fn set_answer(app: &mut AppState, number: u64, state: PrState) {
        let pr = PullRequest {
            number,
            url: format!("https://github.com/octo/demo/pull/{number}"),
            state,
        };
        assert!(app.apply_pr_lookup(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            PrLookup::Found(pr)
        ));
    }

    fn fetch(number: u64, request: u64) -> Effect {
        Effect::LookupPullRequestDetail {
            repo: PathBuf::from("app"),
            branch: "feature".into(),
            remote: REMOTE.into(),
            number,
            request,
        }
    }

    fn pr_detail(number: u64) -> PrDetailLookup {
        PrDetailLookup::Found(PullRequestDetail {
            number,
            title: "Add login".into(),
            state: "OPEN".into(),
            draft: false,
            author: Some("octocat".into()),
            head: Some("feature".into()),
            base: Some("main".into()),
            review: None,
            checks: ChecksSummary::default(),
            updated_at: None,
        })
    }

    /// Land the answer to the request the cache waits for (0 when it
    /// waits for none).
    fn land(app: &mut AppState, branch: &str, number: u64, lookup: PrDetailLookup) -> bool {
        let request = app
            .pr_cache
            .details
            .values()
            .find_map(|detail| match detail {
                PrDetailState::Loading(request) => Some(*request),
                _ => None,
            })
            .unwrap_or(0);
        land_request(app, branch, number, request, lookup)
    }

    fn land_request(
        app: &mut AppState,
        branch: &str,
        number: u64,
        request: u64,
        lookup: PrDetailLookup,
    ) -> bool {
        app.apply_pr_detail(Path::new("app"), branch, REMOTE, number, request, lookup)
    }

    fn detail_of(app: &AppState) -> Option<PrDetailState> {
        app.pull_request_detail_for(Path::new("app")).cloned()
    }

    /// Pin the PR badge popover of `app` with a click on its badge.
    fn click_badge(app: &mut AppState) -> Effect {
        let y = tree_y(app, "repo:app");
        badge_at(app, "app", 20, y);
        app.close_popover();
        app.dispatch(Action::Click { col: 20, row: y })
    }

    #[test]
    fn a_peek_or_pin_requests_the_detail_once_and_later_ones_read_the_cache() {
        let mut app = app_with_pr(7);
        assert_eq!(detail_of(&app), None, "nothing asked before a popover");

        // A peek that opens on the badge asks once.
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        let at = std::time::Instant::now();
        app.set_pointer_at(Some((20, y)), at);
        assert_eq!(
            app.expire_peek_with_details(at + std::time::Duration::from_millis(PEEK_DWELL_MS)),
            (true, fetch(7, 1))
        );
        assert!(app.popover.is_some(), "peek open");
        assert_eq!(detail_of(&app), Some(PrDetailState::Loading(1)));
        assert_eq!(
            app.request_popover_pr_details(),
            Effect::None,
            "a later peek reads the cache"
        );

        // Pinning the same PR while it loads asks nothing.
        assert_eq!(click_badge(&mut app), Effect::LoadRightPane);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
        assert_eq!(
            click_badge(&mut app),
            Effect::LoadRightPane,
            "ready: cached"
        );

        // A failure is cached too; it is not retried on the next pin.
        app.forget_pr_lookups(Some(Path::new("app")));
        let _ = app.due_pr_lookups();
        set_answer(&mut app, 7, PrState::Open);
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)]),
            "a pin on a fresh answer asks"
        );
        assert!(land(&mut app, "feature", 7, PrDetailLookup::Failed));
        assert_eq!(detail_of(&app), Some(PrDetailState::Failed));
        assert_eq!(click_badge(&mut app), Effect::LoadRightPane);
    }

    #[test]
    fn a_second_peek_dwell_on_the_badge_asks_nothing() {
        use super::super::super::popover::PEEK_GRACE_MS;
        use std::time::{Duration, Instant};
        let mut app = app_with_pr(7);
        let y = tree_y(&app, "repo:app");
        badge_at(&mut app, "app", 20, y);
        let dwell = Duration::from_millis(PEEK_DWELL_MS);
        let t0 = Instant::now();
        app.set_pointer_at(Some((20, y)), t0);
        assert_eq!(
            app.expire_peek_with_details(t0 + dwell),
            (true, fetch(7, 1)),
            "the first dwell opens the peek and asks once"
        );
        // Leave: the grace closes the peek and asks nothing.
        let t1 = t0 + dwell;
        app.set_pointer_at(None, t1);
        assert_eq!(
            app.expire_peek_with_details(t1 + Duration::from_millis(PEEK_GRACE_MS)),
            (true, Effect::None)
        );
        assert!(app.popover.is_none());
        // Back on the badge: a second peek reads the cache.
        let t2 = t1 + Duration::from_secs(1);
        app.set_pointer_at(Some((20, y)), t2);
        assert_eq!(
            app.expire_peek_with_details(t2 + dwell),
            (true, Effect::None)
        );
        assert!(app.popover.is_some(), "second peek open");
        assert_eq!(
            app.expire_peek_with_details(t2 + dwell),
            (false, Effect::None),
            "nothing due"
        );
    }

    #[test]
    fn a_late_answer_to_an_older_request_loses_to_the_newer_one() {
        let mut app = app_with_pr(7);
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 1)])
        );
        // `r` while request 1 hangs, then a pin asks again.
        app.forget_pr_lookups(Some(Path::new("app")));
        let _ = app.due_pr_lookups();
        set_answer(&mut app, 7, PrState::Open);
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)])
        );
        assert!(
            !land_request(&mut app, "feature", 7, 1, PrDetailLookup::Failed),
            "the old request times out late: dropped"
        );
        assert_eq!(detail_of(&app), Some(PrDetailState::Loading(2)));
        assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
        assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
        assert!(
            !land_request(&mut app, "feature", 7, 1, PrDetailLookup::Failed),
            "and never replaces the fresh one"
        );
    }

    #[test]
    fn no_detail_is_requested_without_a_pr_answer() {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let _ = app.due_pr_lookups();
        assert_eq!(click_badge(&mut app), Effect::LoadRightPane, "in flight");
        assert!(app.apply_pr_lookup(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            PrLookup::NoPr
        ));
        assert_eq!(click_badge(&mut app), Effect::LoadRightPane, "no PR");
        assert!(app.pr_cache.details.is_empty());
    }

    #[test]
    fn a_branch_switch_r_and_a_changed_answer_drop_the_detail() {
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));

        // `gx` with the same answer keeps the detail; a new one drops it.
        app.apply_pr_open(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            found(PrState::Open),
            true,
        );
        assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
        app.apply_pr_open(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            found(PrState::Approved),
            true,
        );
        assert_eq!(detail_of(&app), None, "the answer changed under gx");
        assert!(app.pr_cache.details.is_empty());
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)])
        );

        // `r` forgets the checkout and its detail.
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        app.forget_pr_lookups(Some(Path::new("app")));
        assert!(app.pr_cache.details.is_empty(), "r drops the detail");
        let _ = app.due_pr_lookups();
        set_answer(&mut app, 7, PrState::Open);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        app.forget_pr_lookups(None);
        assert!(app.pr_cache.details.is_empty(), "a full reload drops all");

        // A branch switch drops the old branch's detail.
        let _ = app.due_pr_lookups();
        set_answer(&mut app, 7, PrState::Open);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
        assert_eq!(detail_of(&app), None, "hidden at once");
        let _ = app.due_pr_lookups();
        assert!(app.pr_cache.details.is_empty(), "pruned with the answer");
    }

    #[test]
    fn a_late_detail_for_an_old_branch_or_a_forgotten_checkout_is_dropped() {
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
        assert!(!land(&mut app, "feature", 7, pr_detail(7)), "old branch");

        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        app.forget_pr_lookups(Some(Path::new("app")));
        assert!(!land(&mut app, "feature", 7, pr_detail(7)), "forgotten");
        assert_eq!(detail_of(&app), None);

        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        set_answer(&mut app, 8, PrState::Open);
        assert!(
            !land(&mut app, "feature", 7, pr_detail(7)),
            "another PR now"
        );
        assert_eq!(detail_of(&app), None);

        let mut app = app_with_pr(7);
        assert!(!land(&mut app, "feature", 7, pr_detail(7)), "never asked");
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        assert!(
            !land(&mut app, "feature", 7, PrDetailLookup::Failed),
            "a second answer does not replace the first"
        );
        assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
    }

    #[test]
    fn the_pinned_focus_stays_on_its_line_when_the_detail_lands() {
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        assert_eq!(
            app.popover_run_action(),
            Some(Action::OpenPullRequest),
            "lands on Open PR below the loading line"
        );
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        assert_eq!(app.popover_run_action(), Some(Action::OpenPullRequest));

        // A field keeps the focus too: the URL moves down past the new
        // fields and `y` still copies it.
        let copy_url = Effect::CopyClipboard {
            text: "https://github.com/octo/demo/pull/7".into(),
            announce: true,
        };
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        app.dispatch(Action::PopoverMove(-1));
        assert_eq!(app.dispatch(Action::PopoverCopyLine), copy_url);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        assert_eq!(app.dispatch(Action::PopoverCopyLine), copy_url);
    }

    #[test]
    fn watch_ticks_never_request_a_detail() {
        let mut app = app_with_pr(7);
        focus(&mut app, "repo:app");
        let _ = click_badge(&mut app);
        assert!(app.popover_pinned());
        // The detail is dropped while the popover stays pinned.
        set_answer(&mut app, 7, PrState::Approved);
        assert_eq!(detail_of(&app), None);
        for tick in [Action::WatchTick, Action::FetchTick] {
            let effect = app.dispatch(tick.clone());
            assert!(
                !format!("{effect:?}").contains("LookupPullRequestDetail"),
                "{tick:?}: {effect:?}"
            );
        }
        assert!(app.pr_cache.details.is_empty());
        let _ = app.due_pr_lookups();
        assert!(
            app.pr_cache.details.is_empty(),
            "a status apply asks nothing"
        );
    }

    #[test]
    fn palette_open_pr_closes_the_palette_and_opens_the_focused_branch() {
        use super::super::super::action::QuickOpenEntry;
        use super::super::super::command_palette::PALETTE_COMMANDS;
        let mut app = app_with(&[repo("app", "feature")], &[]);
        focus(&mut app, "repo:app");
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == "Open PR")
            .expect("Open PR row");
        assert_eq!(app.palette_disabled_reason(command), None);

        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in "open pr".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let selected = app
            .command_palette()
            .and_then(|palette| palette.selected())
            .map(|command| command.title);
        assert_eq!(selected, Some("Open PR"));
        assert_eq!(
            app.dispatch(Action::QuickOpenSubmit),
            open("app", "feature")
        );
        assert!(app.quick_open.is_none(), "submit closes the palette");
    }

    /// `pull` also finds Open PR by its alias. On a file row Pull behind is
    /// disabled and Open PR is enabled, but the cursor stays on Pull behind,
    /// so Enter keeps the palette open instead of running `gx`.
    #[test]
    fn palette_pull_on_a_file_row_lands_on_disabled_pull_not_open_pr() {
        use super::super::super::action::QuickOpenEntry;
        let mut app = app_with(&[repo("app", "feature")], &[]);
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        let selected = |app: &mut AppState, query: &str| {
            app.quick_open = None;
            app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
            for c in query.chars() {
                app.dispatch(Action::QuickOpenChar(c));
            }
            app.command_palette()
                .and_then(|palette| palette.selected())
                .map(|command| command.title)
        };
        assert_eq!(selected(&mut app, "pull"), Some("Pull behind"));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(
            app.command_palette().is_some(),
            "disabled row keeps it open"
        );
        assert_eq!(selected(&mut app, "pull request"), Some("Open PR"));
        assert_eq!(selected(&mut app, "merge request"), Some("Open PR"));
    }
}

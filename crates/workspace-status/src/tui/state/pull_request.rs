//! PR of a branch row: the `gx` target and the badge cache.
//!
//! The forge CLI runs on a worker ([`Effect::OpenPullRequest`],
//! [`Effect::LookupPullRequests`]). This module decides what to look up and
//! keeps the answers. Answers are cached per (remote URL, branch). Each
//! checkout remembers the branch it was looked up for and the remote URL the
//! worker resolved, so a badge never shows an answer for another branch.
//!
//! A checkout is looked up when its branch is new or changed, and after `r`
//! forgets it. A watch tick that re-applies the same branch does not ask the
//! forge again. The poll cycle ([`AppState::due_pr_polls`]) looks up the
//! pollable checkouts again on its own timer, and keeps their badges on
//! screen while it runs. A failed poll backs its forge host off and keeps
//! the last answer.
//!
//! Each successful lookup is compared with the last one of the same
//! checkout in this session. A change of the same PR into CI failed, ready
//! to merge, or merged, or the PR going away, queues a [`PrTransition`] for
//! the alert surfaces.
//!
//! The PR popover shows richer fields ([`PullRequestDetail`]). They are
//! fetched once per (remote URL, branch), when a peek or pin first shows
//! that PR ([`AppState::request_popover_pr_details`]), never on a tick. A
//! branch switch, `r`, a vanished checkout, or an answer that names another
//! PR drops the detail. A changed answer for the same PR keeps it and marks
//! it for a new fetch, which the next peek, pin, or landed badge lookup
//! with the popover open asks for.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use workspace_status_graph::{GraphRow, Worktree};

use crate::helpers::is_counted_local_branch;
use crate::snapshot::{SyncStatus, WorkspaceRepoSnapshot};

use super::super::action::Effect;
use super::super::gates::ListFocusTarget;
use super::super::hyperlink::LinkHit;
use super::super::popover::IconTarget;
use super::super::pr_poll::{
    pr_answer_is_stale, pr_poll_backoff_ms, pr_poll_interval_ms, PrPollMode,
};
use super::super::pr_status::PrState;
use super::super::pull_request::{
    commit_url, lookup_failed_status, no_pr_status, parse_remote, PrDetailLookup, PrLookup,
    PullRequest, PullRequestDetail, NO_PR_FOR_ROW, OPEN_FAILED,
};
use super::super::status::StatusMessage;
use super::super::tree::{NodeKind, PrBadge, PrTotals, PR_REF_MIN_TREE_COLS};
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
            PrLookup::Failed(_) => Self::Failed,
        }
    }
}

impl BranchPr {
    /// Number and URL of the PR this answer names; `None` for no PR or a
    /// failed lookup.
    fn identity(&self) -> Option<(u64, &str)> {
        match self {
            Self::Ready(Some(pr)) => Some((pr.number, pr.url.as_str())),
            Self::Ready(None) | Self::Failed => None,
        }
    }
}

/// The branch one checkout was looked up for, and how its lookups went.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CheckoutPr {
    branch: String,
    /// Remote URL the last landed lookup resolved. `None` before the first
    /// lookup of the branch lands, and when the checkout has no remote (then
    /// there is no PR either). A poll, a refresh, `r`, and a full reload
    /// keep it, so the badge stays.
    remote: Option<String>,
    /// Forge host of [`Self::remote`] (`github.com`), parsed once when the
    /// remote is set; `None` without a GitHub or GitLab remote. The poll
    /// period, the poll cycle, and the stale test read it, so they never
    /// parse a remote.
    host: Option<String>,
    /// State of the PR this checkout's last landed answer names; `None`
    /// while no PR is known (no PR, a failed first lookup, or nothing
    /// landed). A failed later lookup keeps it, like the answer.
    pr_state: Option<PrState>,
    /// A lookup of this branch is queued or running. A poll skips it.
    in_flight: bool,
    /// `r` or a full reload asked for a new lookup: the next applied status
    /// ([`AppState::due_pr_lookups`]) queues it.
    look_again: bool,
    /// Start of the poll cycle that queued the lookup in flight; `None`
    /// when that lookup is not a poll. A failed poll backs off its forge
    /// host from this time.
    poll_cycle: Option<Instant>,
    /// When the last successful lookup of this branch landed.
    checked_at: Option<Instant>,
    /// The last lookup of this branch failed; `None` after a success.
    failure: Option<PrLookupFailure>,
}

impl CheckoutPr {
    /// A record for `branch` with nothing landed yet.
    fn new(branch: &str, in_flight: bool) -> Self {
        Self {
            branch: branch.to_string(),
            remote: None,
            host: None,
            pr_state: None,
            in_flight,
            look_again: false,
            poll_cycle: None,
            checked_at: None,
            failure: None,
        }
    }

    /// Set the resolved remote and its forge host.
    fn set_remote(&mut self, remote: Option<String>, host: Option<String>) {
        self.remote = remote;
        self.host = host;
    }

    /// True when a poll cycle may look this record up, given the snapshot
    /// row's upstream: a GitHub or GitLab remote, and an open PR, or no PR
    /// known yet on a branch that was pushed. A merged PR is not polled.
    fn pollable(&self, has_upstream: bool) -> bool {
        self.host.is_some()
            && match self.pr_state {
                Some(PrState::Merged) => false,
                Some(_) => true,
                None => has_upstream,
            }
    }

    /// True when this record shows an open PR with CI running or ready to
    /// merge, which makes the poll period hot.
    fn hot(&self) -> bool {
        self.host.is_some() && matches!(self.pr_state, Some(PrState::CiRunning | PrState::Ready))
    }
}

/// Forge host of `remote`, or `None` when it is not on GitHub or GitLab.
/// The only place this module parses a remote for polling; test builds
/// count the calls.
fn forge_host(remote: &str) -> Option<String> {
    #[cfg(test)]
    FORGE_HOST_PARSES.with(|count| count.set(count.get() + 1));
    parse_remote(remote).map(|repo| repo.host)
}

#[cfg(test)]
thread_local! {
    /// Calls of [`forge_host`] on this thread.
    static FORGE_HOST_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A PR lookup of a checkout that failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrLookupFailure {
    /// The CLI's first stderr line, when it printed one.
    // Only tests read this until the PR popover does.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) reason: Option<String>,
}

/// What a PR became in a [`PrTransition`].
// Only tests read this until the alert surfaces do.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrTransitionTo {
    /// The same PR is in this state now: [`PrState::CiFailed`],
    /// [`PrState::Ready`], or [`PrState::Merged`].
    State(PrState),
    /// The PR went away (closed without a merge, so no PR counts) while
    /// the checkout stayed on the branch and the forge answered.
    Closed,
}

/// A change of a PR that alerts: same checkout, same PR number, a state
/// from an earlier successful lookup in this session, and now CI failed,
/// ready to merge, merged, or closed. Other changes are not queued.
///
/// [`AppState::take_pr_transitions`] drains the queue.
// Only tests read this until the alert surfaces do.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrTransition {
    /// Checkout path (snapshot `repo`) whose PR changed.
    pub(crate) checkout: PathBuf,
    /// The new answer. For [`PrTransitionTo::Closed`], the last answer
    /// before the PR went away.
    pub(crate) pr: PullRequest,
    /// State of the PR in the earlier lookup.
    pub(crate) from: PrState,
    /// What the PR became.
    pub(crate) to: PrTransitionTo,
}

/// The answer of the last successful lookup of one checkout in this
/// session, the "before" of a [`PrTransition`].
#[derive(Clone, Debug, PartialEq, Eq)]
struct SeenPr {
    remote: String,
    branch: String,
    pr: Option<PullRequest>,
}

impl SeenPr {
    /// The transition from this answer to `pr`, the new answer of
    /// `checkout` for `branch` at `remote`, when it is one that alerts.
    fn transition_to(
        &self,
        checkout: &Path,
        remote: &str,
        branch: &str,
        pr: Option<&PullRequest>,
    ) -> Option<PrTransition> {
        if self.remote != remote || self.branch != branch {
            return None;
        }
        let before = self.pr.as_ref()?;
        let (pr, to) = match pr {
            Some(pr) if pr.number == before.number && pr.state != before.state => {
                if !matches!(
                    pr.state,
                    PrState::CiFailed | PrState::Ready | PrState::Merged
                ) {
                    return None;
                }
                (pr.clone(), PrTransitionTo::State(pr.state))
            }
            None if before.state != PrState::Merged => (before.clone(), PrTransitionTo::Closed),
            _ => return None,
        };
        Some(PrTransition {
            checkout: checkout.to_path_buf(),
            pr,
            from: before.state,
            to,
        })
    }
}

/// How the lookups on one forge host went, decided per poll cycle.
///
/// A poll cycle fails for the host only when none of its lookups on that
/// host succeeds: one checkout that keeps failing (a renamed repository,
/// lost access) never backs off the others.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct HostHealth {
    /// Poll cycles that failed in a row; each one doubles the period. A
    /// success resets it.
    failures: u32,
    /// Start of the last poll cycle counted as failed.
    failed_cycle: Option<Instant>,
    /// When the last successful lookup on this host landed. A failure from
    /// a cycle that started before it is not counted: the host answered in
    /// that cycle.
    ok_at: Option<Instant>,
}

impl HostHealth {
    /// Count a failed poll from the cycle that started at `cycle`, unless
    /// a lookup on the host succeeded since that cycle started, or that
    /// cycle (or a newer one) already counted.
    fn fail(&mut self, cycle: Instant) {
        let answered = self.ok_at.is_some_and(|ok| ok >= cycle);
        let counted = self.failed_cycle.is_some_and(|failed| failed >= cycle);
        if !answered && !counted {
            self.failures = self.failures.saturating_add(1);
            self.failed_cycle = Some(cycle);
        }
    }

    /// Note a successful lookup that landed at `now`: the backoff resets,
    /// which also cancels a failure counted earlier in the same cycle.
    fn succeed(&mut self, now: Instant) {
        self.failures = 0;
        self.ok_at = Some(self.ok_at.map_or(now, |ok| ok.max(now)));
    }

    /// True when a poll may run at `now` with poll period `interval_ms`:
    /// no failed cycle in a row, or the backed-off period has passed since
    /// the last one.
    fn ready(&self, now: Instant, interval_ms: u64) -> bool {
        match self.failed_cycle {
            Some(cycle) if self.failures > 0 => {
                let wait = Duration::from_millis(pr_poll_backoff_ms(interval_ms, self.failures));
                now.saturating_duration_since(cycle) >= wait
            }
            _ => true,
        }
    }
}

/// A detail that waits for a new fetch: the answer of its PR changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DetailRefresh {
    /// The next peek, pin, or landed badge or detail lookup with the
    /// popover open asks for it. While the first fetch of the detail is
    /// out, it waits for that fetch to land.
    Due,
    /// Request `request` is out; the old detail stays on screen. `again`:
    /// the answer changed after the request went out, so the detail is
    /// due again when this one lands.
    Loading { request: u64, again: bool },
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
            PrDetailLookup::Failed(_) => Self::Failed,
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
    /// (remote URL, branch) of details that wait for a new fetch.
    detail_refresh: HashMap<(String, String), DetailRefresh>,
    /// Id of the last detail request.
    last_detail_request: u64,
    /// Checkout path to its last successful answer in this session. `r`
    /// and a full reload keep it, so their lookups can still alert; a
    /// branch change, a detached HEAD, a hidden repo, or a vanished
    /// checkout drops it.
    seen: HashMap<PathBuf, SeenPr>,
    /// Forge host (`github.com`) to how its polls went.
    hosts: HashMap<String, HostHealth>,
    /// Transitions not yet drained by [`AppState::take_pr_transitions`].
    transitions: Vec<PrTransition>,
    /// How polling runs ([`crate::settings::Settings::pr_poll`]).
    poll: PrPollMode,
    /// A record shows an open PR with CI running or ready to merge, so the
    /// poll period is hot. Recomputed from the records ([`Self::rehot`])
    /// whenever a lookup lands or the records sync with the snapshot, so
    /// the poll period and the stale test cost nothing per frame.
    hot: bool,
}

impl PrCache {
    /// Remote URL the PR lookup resolved for checkout `checkout`.
    ///
    /// `None` before the lookup lands, and when the checkout has no remote.
    pub(crate) fn remote_for(&self, checkout: &Path) -> Option<&str> {
        self.checkouts.get(checkout)?.remote.as_deref()
    }

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
        self.detail_refresh.retain(|key, _| keep(key));
    }

    /// Drop the detail of `key` and any refresh it waits for.
    fn drop_detail(&mut self, key: &(String, String)) {
        self.details.remove(key);
        self.detail_refresh.remove(key);
    }

    /// Mark the detail of `key` for a new fetch, when one landed.
    fn mark_detail_for_refresh(&mut self, key: &(String, String)) {
        if !self.details.contains_key(key) {
            return;
        }
        match self.detail_refresh.get_mut(key) {
            // The new fetch went out before this change: fetch again once
            // it lands.
            Some(DetailRefresh::Loading { again, .. }) => *again = true,
            Some(DetailRefresh::Due) => {}
            // While the first fetch is out, `Due` waits for it to land.
            None => {
                self.detail_refresh.insert(key.clone(), DetailRefresh::Due);
            }
        }
    }

    /// Store `answer` for `key`.
    ///
    /// An answer that names another PR (number or URL), or no PR, drops the
    /// detail, which described the old one. A changed answer for the same
    /// PR keeps the detail on screen and marks it for a new fetch.
    fn set_answer(&mut self, key: (String, String), answer: BranchPr) {
        let old = self.answers.get(&key);
        if old != Some(&answer) {
            if old.and_then(BranchPr::identity) != answer.identity() {
                self.drop_detail(&key);
            } else {
                self.mark_detail_for_refresh(&key);
            }
        }
        self.answers.insert(key, answer);
    }

    /// True when a poll of a checkout on `host` may run at `now`, with poll
    /// period `interval_ms` ([`HostHealth::ready`]).
    fn host_ready(&self, host: &str, now: Instant, interval_ms: u64) -> bool {
        self.hosts
            .get(host)
            .is_none_or(|health| health.ready(now, interval_ms))
    }

    /// Recompute [`Self::hot`] from the records. Callers sync the records
    /// with the snapshot first ([`AppState::due_pr_lookups`]), so each
    /// record is a visible checkout on its current branch.
    fn rehot(&mut self) {
        self.hot = self.checkouts.values().any(CheckoutPr::hot);
    }

    /// Record `pr`, the new answer of `checkout` for `branch` at `remote`,
    /// as the last one seen, and queue the transition from the one before
    /// when it alerts.
    ///
    /// While the forge still computes the merge facts, ready to merge versus
    /// in review is not final: such an answer is not recorded, so the last
    /// settled one stays the "before". CI failed and merged do not depend
    /// on the merge facts, so they still alert.
    fn note_seen(&mut self, checkout: &Path, remote: &str, branch: &str, pr: Option<&PullRequest>) {
        let unsettled = pr.is_some_and(|pr| {
            !pr.merge_state_settled && matches!(pr.state, PrState::Ready | PrState::InReview)
        });
        if unsettled {
            return;
        }
        let now_seen = SeenPr {
            remote: remote.to_string(),
            branch: branch.to_string(),
            pr: pr.cloned(),
        };
        let before = self.seen.insert(checkout.to_path_buf(), now_seen);
        if let Some(transition) =
            before.and_then(|before| before.transition_to(checkout, remote, branch, pr))
        {
            self.transitions.push(transition);
        }
    }

    /// Land `lookup` of `branch` for `checkout`, whose record is for that
    /// branch. `remote` is the URL the worker resolved; `poll_cycle` is the
    /// start of the poll cycle that queued it (`None` for any other lookup).
    ///
    /// A success resets the backoff of its forge host, stamps the check
    /// time, and may queue a transition. A failure records its reason and
    /// keeps the last answer when there is one (the badge stays); a failed
    /// poll also backs the host off.
    fn land(
        &mut self,
        checkout: &Path,
        branch: &str,
        remote: Option<String>,
        lookup: PrLookup,
        poll_cycle: Option<Instant>,
        now: Instant,
    ) {
        let Some(record) = self.checkouts.get_mut(checkout) else {
            return;
        };
        // Parse the remote only when it changed; a poll lands the same one.
        let host = if record.remote == remote {
            record.host.clone()
        } else {
            remote.as_deref().and_then(forge_host)
        };
        let pr = match lookup {
            PrLookup::Failed(reason) => {
                record.failure = Some(PrLookupFailure { reason });
                let known = record.remote.clone().is_some_and(|remote| {
                    matches!(
                        self.answers.get(&(remote, branch.to_string())),
                        Some(BranchPr::Ready(_))
                    )
                });
                if !known {
                    record.set_remote(remote.clone(), host.clone());
                    record.pr_state = None;
                    if let Some(remote) = remote {
                        self.set_answer((remote, branch.to_string()), BranchPr::Failed);
                    }
                }
                if let (Some(host), Some(cycle)) = (host, poll_cycle) {
                    self.hosts.entry(host).or_default().fail(cycle);
                }
                self.rehot();
                return;
            }
            PrLookup::Found(pr) => Some(pr),
            PrLookup::NoPr => None,
        };
        record.set_remote(remote.clone(), host.clone());
        record.pr_state = pr.as_ref().filter(|_| remote.is_some()).map(|pr| pr.state);
        record.failure = None;
        record.checked_at = Some(now);
        if let Some(host) = host {
            self.hosts.entry(host).or_default().succeed(now);
        }
        self.rehot();
        let Some(remote) = remote else {
            return;
        };
        self.note_seen(checkout, &remote, branch, pr.as_ref());
        self.set_answer((remote, branch.to_string()), BranchPr::Ready(pr));
    }
}

/// A checkout a poll cycle may look up.
struct Pollable<'a> {
    path: &'a Path,
    record: &'a CheckoutPr,
    /// Forge host of the resolved remote ([`CheckoutPr::host`]).
    host: &'a str,
}

impl AppState {
    /// Checkout and branch `gx` opens a PR for, or `None`.
    ///
    /// The focused tree repo / checkout row with a branch, or the focused
    /// graph worktree row with a branch. An Explorer or compare tab, commit
    /// files, a diff, a commit, stash, dir, or file row, a repo that groups
    /// several checkouts, and a detached HEAD have no target.
    pub(super) fn pr_target_for_focus(&self) -> Option<(PathBuf, String)> {
        if self.is_explorer_tab() || self.is_compare_tab() {
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

    /// One link per PR badge glyph painted so far this frame, to the full
    /// URL of its PR, for [`LayoutHit::links`].
    ///
    /// A badge whose PR is no longer cached gets no link.
    ///
    /// [`LayoutHit::links`]: super::LayoutHit::links
    pub(crate) fn pr_badge_links(&self) -> Vec<LinkHit> {
        self.layout
            .pr_badge_hits()
            .into_iter()
            .filter_map(|hit| {
                let pr = self.pull_request_for(&hit.repo)?;
                Some(LinkHit {
                    y: hit.y,
                    x: hit.x,
                    width: hit.width,
                    url: pr.url.clone(),
                })
            })
            .collect()
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

    /// Web page of commit `sha` (full id) in checkout `checkout`, or `None`.
    ///
    /// The forge comes from the remote URL the PR badge lookup already
    /// resolved for the checkout ([`PrCache::remote_for`]); no git runs
    /// here. No resolved remote, or a remote that is not GitHub or GitLab,
    /// gives no link. An id that is not a full SHA-1 or SHA-256 hex id (an
    /// abbreviated one) gives no link either.
    pub(crate) fn commit_link(&self, checkout: &Path, sha: &str) -> Option<String> {
        let full = matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit());
        if !full {
            return None;
        }
        let remote = self.pr_cache.remote_for(checkout)?;
        Some(commit_url(&parse_remote(remote)?, sha))
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
    /// branch is returned, and one that `r` or a full reload asked to look
    /// up again (its answer stays, so the badge stays). A checkout already
    /// looked up for its current branch is left alone. A checkout that
    /// vanished, has a detached HEAD, or is a hidden repo is dropped.
    pub(crate) fn due_pr_lookups(&mut self) -> Vec<(PathBuf, String)> {
        let wanted: Vec<(PathBuf, String)> = self
            .pr_rows()
            .map(|row| (PathBuf::from(&row.repo), row.branch.clone()))
            .collect();
        let cache = &mut self.pr_cache;
        let branch_of: HashMap<&PathBuf, &String> =
            wanted.iter().map(|(path, branch)| (path, branch)).collect();
        cache
            .checkouts
            .retain(|path, _| branch_of.contains_key(path));
        // The last answer seen of another branch never makes a transition.
        cache
            .seen
            .retain(|path, seen| branch_of.get(path) == Some(&&seen.branch));
        let mut due = Vec::new();
        for (path, branch) in &wanted {
            match cache.checkouts.get_mut(path) {
                Some(record) if &record.branch == branch => {
                    if !std::mem::take(&mut record.look_again) {
                        continue;
                    }
                    // A lookup still in flight is not queued twice: the
                    // interpreter drops the repeat.
                    record.in_flight = true;
                    record.poll_cycle = None;
                }
                _ => {
                    cache
                        .checkouts
                        .insert(path.clone(), CheckoutPr::new(branch, true));
                }
            }
            due.push((path.clone(), branch.clone()));
        }
        cache.prune_answers();
        cache.rehot();
        due
    }

    /// Snapshot rows whose PR is looked up: visible (not hidden, unless
    /// hidden repos show) and on a local branch.
    fn pr_rows(&self) -> impl Iterator<Item = &WorkspaceRepoSnapshot> {
        self.snapshot
            .repos
            .iter()
            .filter(|row| self.show_hidden || !row.hidden)
            .filter(|row| is_counted_local_branch(&row.branch))
    }

    /// Set how PR polling runs, from [`crate::settings::Settings::pr_poll`].
    pub(crate) fn set_pr_poll_mode(&mut self, mode: PrPollMode) {
        self.pr_cache.poll = mode;
    }

    /// Checkouts a poll cycle may look up.
    ///
    /// A checkout is pollable when it is visible, on a local branch, its
    /// first lookup for that branch landed with a GitHub or GitLab remote,
    /// and either its answer is an open PR, or no PR is known yet and the
    /// branch has an upstream (it was pushed), so a PR opened later shows.
    /// A merged PR is not polled. Reads the facts each record keeps
    /// ([`CheckoutPr::pollable`]); it parses no remote.
    fn pollable_checkouts(&self) -> Vec<Pollable<'_>> {
        self.pr_rows()
            .filter_map(|row| {
                let path = Path::new(&row.repo);
                let record = self
                    .pr_cache
                    .checkouts
                    .get(path)
                    .filter(|record| record.branch == row.branch)?;
                if !record.pollable(row.sync_status != SyncStatus::NoUpstream) {
                    return None;
                }
                Some(Pollable {
                    path,
                    record,
                    host: record.host.as_deref()?,
                })
            })
            .collect()
    }

    /// Planned PR poll period in ms: [`PrPollMode`] with "hot" while any
    /// pollable PR has CI running or is ready to merge. `0` with polling
    /// off. Reads a flag the cache keeps up to date, so it is cheap on
    /// every loop pass.
    pub(crate) fn pr_poll_interval_ms(&self) -> u64 {
        pr_poll_interval_ms(self.pr_cache.poll, self.pr_cache.hot)
    }

    /// Pollable checkouts the poll cycle that starts at `now` looks up,
    /// marked in flight. Their badges stay on screen while the lookups run.
    ///
    /// Empty with polling off. Skips a checkout whose lookup is still in
    /// flight, and one whose forge host backs off after failed polls.
    pub(crate) fn due_pr_polls(&mut self, now: Instant) -> Vec<(PathBuf, String)> {
        let interval = self.pr_poll_interval_ms();
        if interval == 0 {
            return Vec::new();
        }
        let due: Vec<(PathBuf, String)> = self
            .pollable_checkouts()
            .into_iter()
            .filter(|pollable| !pollable.record.in_flight)
            .filter(|pollable| self.pr_cache.host_ready(pollable.host, now, interval))
            .map(|pollable| (pollable.path.to_path_buf(), pollable.record.branch.clone()))
            .collect();
        for (path, _) in &due {
            if let Some(record) = self.pr_cache.checkouts.get_mut(path) {
                record.in_flight = true;
                record.poll_cycle = Some(now);
            }
        }
        due
    }

    /// Forget that the PRs were looked up, so the next applied snapshot
    /// looks them up again.
    ///
    /// `r` on a checkout passes it; a full reload passes `None` (every
    /// checkout).
    ///
    /// The last answer and its check time stay, so the badge stays while
    /// the new lookup runs and when it fails, as for a poll; only no PR from
    /// a forge that answered, or a branch change, clears it. The PR detail
    /// is dropped, so the popover fetches it again (at once when it is open
    /// and the lookup lands). The last answer seen stays too, so the new
    /// lookup can still queue a transition.
    pub(crate) fn forget_pr_lookups(&mut self, repo: Option<&Path>) {
        let cache = &mut self.pr_cache;
        match repo {
            Some(repo) => {
                let Some(record) = cache.checkouts.get_mut(repo) else {
                    return;
                };
                record.look_again = true;
                if let Some(remote) = record.remote.clone() {
                    let key = (remote, record.branch.clone());
                    cache.drop_detail(&key);
                }
            }
            None => {
                for record in cache.checkouts.values_mut() {
                    record.look_again = true;
                }
                cache.details.clear();
                cache.detail_refresh.clear();
            }
        }
    }

    /// Land a badge lookup of `branch` in checkout `repo`, now.
    ///
    /// See [`Self::apply_pr_lookup_at`].
    pub(crate) fn apply_pr_lookup(
        &mut self,
        repo: &Path,
        branch: &str,
        remote: Option<String>,
        lookup: PrLookup,
    ) -> bool {
        self.apply_pr_lookup_at(repo, branch, remote, lookup, Instant::now())
    }

    /// Land a badge lookup of `branch` in checkout `repo` at `now`.
    ///
    /// Dropped when the checkout is on another branch now, or was forgotten
    /// since the lookup was queued. A failed lookup sets no status; it keeps
    /// the last answer of the branch, so a known badge stays, and records
    /// the failure. A failed poll backs its forge host off. A success may
    /// queue a [`PrTransition`]. Returns true when the cache changed.
    pub(crate) fn apply_pr_lookup_at(
        &mut self,
        repo: &Path,
        branch: &str,
        remote: Option<String>,
        lookup: PrLookup,
        now: Instant,
    ) -> bool {
        if self.snapshot_branch(repo) != Some(branch) {
            return false;
        }
        let Some(record) = self
            .pr_cache
            .checkouts
            .get_mut(repo)
            .filter(|record| record.branch == branch)
        else {
            return false;
        };
        record.in_flight = false;
        let poll_cycle = record.poll_cycle.take();
        self.pr_cache
            .land(repo, branch, remote, lookup, poll_cycle, now);
        true
    }

    /// Land a `gx` result: say what happened, and cache the fresh answer.
    ///
    /// `opened` is true when the browser started for a found PR. The
    /// answer lands like a badge lookup's (a failure keeps a known badge).
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
            PrLookup::Failed(_) => StatusMessage::error(lookup_failed_status(branch)),
        };
        if self.snapshot_branch(repo) == Some(branch) {
            let cache = &mut self.pr_cache;
            if cache
                .checkouts
                .get(repo)
                .is_none_or(|record| record.branch != branch)
            {
                cache
                    .checkouts
                    .insert(repo.to_path_buf(), CheckoutPr::new(branch, false));
            }
            cache.land(repo, branch, remote, lookup, None, Instant::now());
        } else if let Some(remote) = remote {
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
    /// cache, unless the detail waits for a new fetch after its answer
    /// changed: then it is asked again and the old detail stays on screen
    /// until the answer lands. A change during the first fetch waits for
    /// that fetch to land. Called when a peek opens, a popover pins, and a
    /// badge or detail lookup lands; never on a watch or fetch tick.
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
                ..
            }) = self.pr_cache.checkouts.get(&repo).cloned()
            else {
                continue;
            };
            let key = (remote.clone(), branch.clone());
            let refresh = match (
                self.pr_cache.details.get(&key),
                self.pr_cache.detail_refresh.get(&key),
            ) {
                (None, _) => false,
                // A `Due` waits until the first fetch lands.
                (Some(PrDetailState::Loading(_)), _) => continue,
                (Some(_), Some(DetailRefresh::Due)) => true,
                (Some(_), _) => continue,
            };
            self.pr_cache.last_detail_request += 1;
            let request = self.pr_cache.last_detail_request;
            if refresh {
                self.pr_cache.detail_refresh.insert(
                    key,
                    DetailRefresh::Loading {
                        request,
                        again: false,
                    },
                );
            } else {
                self.pr_cache
                    .details
                    .insert(key, PrDetailState::Loading(request));
            }
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
    /// again: a newer request wins over a late older one). A failed new
    /// fetch of a detail that had landed keeps the old detail. Returns true
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
        if !self.pr_detail_awaited(remote, branch, request) {
            return false;
        }
        let key = (remote.to_string(), branch.to_string());
        // The detail adds lines above Open PR: keep the pinned focus on
        // the line it was on.
        let focus = self.popover_focus_key();
        let refresh = self.pr_cache.detail_refresh.remove(&key);
        let keep_old = matches!(refresh, Some(DetailRefresh::Loading { .. }))
            && matches!(lookup, PrDetailLookup::Failed(_))
            && matches!(
                self.pr_cache.details.get(&key),
                Some(PrDetailState::Ready(_))
            );
        if !keep_old {
            self.pr_cache.details.insert(key.clone(), lookup.into());
        }
        // The answer changed while this fetch was out: the detail is due
        // again, so it never stays older than the badge.
        if matches!(
            refresh,
            Some(DetailRefresh::Due | DetailRefresh::Loading { again: true, .. })
        ) {
            self.pr_cache.detail_refresh.insert(key, DetailRefresh::Due);
        }
        self.restore_popover_focus(focus);
        true
    }

    /// True while the detail cache still waits on request `request` for
    /// `branch` at `remote` (a first fetch or a new fetch of a changed
    /// answer). `r`, a branch switch, or an answer that names another PR
    /// drops the wait, and a queued fetch for it is then skipped.
    pub(crate) fn pr_detail_awaited(&self, remote: &str, branch: &str, request: u64) -> bool {
        let key = (remote.to_string(), branch.to_string());
        self.pr_cache.details.get(&key) == Some(&PrDetailState::Loading(request))
            || matches!(
                self.pr_cache.detail_refresh.get(&key),
                Some(DetailRefresh::Loading { request: out, .. }) if *out == request
            )
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

    /// Lookup record of checkout `checkout` for the branch it is on now.
    fn current_pr_record(&self, checkout: &Path) -> Option<&CheckoutPr> {
        let record = self.pr_cache.checkouts.get(checkout)?;
        (self.snapshot_branch(checkout) == Some(record.branch.as_str())).then_some(record)
    }

    /// True when the badge of checkout `checkout` is stale at `now`: it
    /// shows a polled (open) PR, and the last successful lookup is older
    /// than twice the planned poll period ([`Self::pr_poll_interval_ms`]).
    /// Never with polling off.
    ///
    /// Reads only that checkout's record (no snapshot scan, no remote
    /// parse), so a painter can ask it for every row. The records follow
    /// the snapshot through [`Self::due_pr_lookups`]; a painter asks only
    /// for a badge it paints, which already matched the current branch.
    pub(crate) fn pr_is_stale(&self, checkout: &Path, now: Instant) -> bool {
        let Some(record) = self.pr_cache.checkouts.get(checkout) else {
            return false;
        };
        let polled_pr = record.host.is_some()
            && record
                .pr_state
                .is_some_and(|state| state != PrState::Merged);
        polled_pr
            && record.checked_at.is_some_and(|at| {
                pr_answer_is_stale(
                    now.saturating_duration_since(at),
                    self.pr_poll_interval_ms(),
                )
            })
    }

    /// The PR badge of checkout `checkout` in a tree `tree_width` columns
    /// wide at `now`: the state of [`Self::pull_request_for`], its ref unless
    /// the tree is narrower than [`PR_REF_MIN_TREE_COLS`], and whether it is
    /// stale ([`Self::pr_is_stale`]). `None` when the checkout shows no PR.
    pub(crate) fn tree_pr_badge(
        &self,
        checkout: &Path,
        tree_width: usize,
        now: Instant,
    ) -> Option<PrBadge> {
        let pr = self.pull_request_for(checkout)?;
        Some(PrBadge {
            state: pr.state,
            ref_text: (tree_width >= PR_REF_MIN_TREE_COLS).then(|| pr.ref_text()),
            stale: self.pr_is_stale(checkout, now),
        })
    }

    /// The workspace PR total: the PRs whose badge shows on a visible
    /// checkout (open and merged), each PR counted once, and how many of
    /// them need the author ([`PullRequest::needs_you`]).
    pub(crate) fn pr_totals(&self) -> PrTotals {
        let mut seen = HashSet::new();
        let mut totals = PrTotals::default();
        for row in self.pr_rows() {
            let Some(pr) = self.pull_request_for(Path::new(&row.repo)) else {
                continue;
            };
            if seen.insert(pr.url.as_str()) {
                totals.prs += 1;
                totals.need_you += usize::from(pr.needs_you());
            }
        }
        totals
    }

    /// The checkout and PR the graph header chip shows: the PR of the
    /// graph's checkout, when the graph's sync header paints the branch the
    /// checkout is on now (a graph loaded before a branch switch shows
    /// none).
    pub(crate) fn graph_header_pr(&self) -> Option<(PathBuf, &PullRequest)> {
        let (checkout, _) = self.graph_identity.as_ref()?;
        let checkout = Path::new(checkout);
        let sync = self.graph.as_ref()?.sync.as_ref()?;
        if self.snapshot_branch(checkout) != Some(sync.branch.as_str()) {
            return None;
        }
        Some((checkout.to_path_buf(), self.pull_request_for(checkout)?))
    }

    /// The known PR that the PR of checkout `checkout` is stacked on
    /// ([`PullRequest::stacked_on`]): among the PRs of every visible
    /// checkout whose remote is the same forge repository, with the
    /// checkout's configured default branch.
    pub(crate) fn pr_stack_base(&self, checkout: &Path) -> Option<&PullRequest> {
        let pr = self.pull_request_for(checkout)?;
        let forge_repo = |path: &Path| parse_remote(self.pr_cache.remote_for(path)?);
        let repo = forge_repo(checkout)?;
        let known: Vec<&PullRequest> = self
            .pr_rows()
            .map(|row| Path::new(&row.repo))
            .filter(|path| forge_repo(path).as_ref() == Some(&repo))
            .filter_map(|path| self.pull_request_for(path))
            .collect();
        let default_branch = self
            .snapshot
            .repos
            .iter()
            .find(|row| Path::new(&row.repo) == checkout)
            .and_then(|row| row.default_branch_override.as_deref());
        pr.stacked_on(known, default_branch)
    }
}

// Only tests read this until the PR popover, alert, and PRs tab surfaces
// do.
#[cfg_attr(not(test), allow(dead_code))]
impl AppState {
    /// When the last successful PR lookup of checkout `checkout` landed, for
    /// its current branch ("checked … ago"). `None` before one lands. `r`
    /// and a full reload keep it until the new lookup succeeds.
    pub(crate) fn pr_checked_at(&self, checkout: &Path) -> Option<Instant> {
        self.current_pr_record(checkout)?.checked_at
    }

    /// The failure of the last PR lookup of checkout `checkout` for its
    /// current branch; `None` when it succeeded or none landed yet.
    pub(crate) fn pr_lookup_failure(&self, checkout: &Path) -> Option<&PrLookupFailure> {
        self.current_pr_record(checkout)?.failure.as_ref()
    }

    /// Take the queued [`PrTransition`]s, oldest first.
    pub(crate) fn take_pr_transitions(&mut self) -> Vec<PrTransition> {
        std::mem::take(&mut self.pr_cache.transitions)
    }

    /// Look up the PR of checkout `checkout` again now, or of every checkout
    /// with `None` ("Refresh this PR", the PRs tab `r`).
    ///
    /// Takes each visible checkout on a local branch whose first lookup
    /// landed with a GitHub or GitLab remote and that has no lookup in
    /// flight. Like `r`, the badge stays on screen until the new answer
    /// lands, and when it fails; the forge host backoff does not apply.
    /// Unlike `r`, it needs no status apply first, and it keeps the detail
    /// on screen: the detail is marked for a new fetch, and a popover that
    /// shows it asks again when the answer lands.
    pub(crate) fn refresh_pull_requests(&mut self, checkout: Option<&Path>) -> Effect {
        let targets: Vec<(PathBuf, String)> = self
            .pr_rows()
            .filter(|row| checkout.is_none_or(|checkout| Path::new(&row.repo) == checkout))
            .filter_map(|row| {
                let path = Path::new(&row.repo);
                let record = self
                    .pr_cache
                    .checkouts
                    .get(path)
                    .filter(|record| record.branch == row.branch && !record.in_flight)?;
                record.host.as_ref()?;
                Some((path.to_path_buf(), row.branch.clone()))
            })
            .collect();
        if targets.is_empty() {
            return Effect::None;
        }
        let cache = &mut self.pr_cache;
        for (path, branch) in &targets {
            let Some(record) = cache.checkouts.get_mut(path) else {
                continue;
            };
            record.in_flight = true;
            if let Some(remote) = record.remote.clone() {
                cache.mark_detail_for_refresh(&(remote, branch.clone()));
            }
        }
        Effect::LookupPullRequests { targets }
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

    fn app_with(repos: &[RepoSnapshot], hidden: &[&str]) -> AppState {
        let hidden: Vec<String> = hidden.iter().map(|name| name.to_string()).collect();
        let snapshot = build_workspace_snapshot(repos, &hidden, false, &[]);
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
        PrLookup::Found(PullRequest::fixture(
            7,
            "https://github.com/octo/demo/pull/7",
            state,
        ))
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
    fn gx_on_an_explorer_file_says_no_pr_for_this_row() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        focus(&mut app, "repo:app");
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        assert!(app.is_explorer_tab());
        assert_no_pr_for_row(&mut app, "Explorer file");
    }

    #[test]
    fn gx_in_the_graph_opens_worktree_rows_with_a_branch_only() {
        let mut app = app_with(&[repo("app", "main")], &[]);
        focus(&mut app, "repo:app");
        let worktree = |path: &str, branch: Option<&str>| Worktree {
            path: path.into(),
            head_id: None,
            branch: branch.map(str::to_string),
            hidden: false,
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

        for state in [PrState::InReview, PrState::Ready, PrState::Merged] {
            assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(state)));
            assert_eq!(app.pr_badge(path), Some(state));
        }
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::NoPr));
        assert_eq!(app.pr_badge(path), None, "no PR");
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::Failed(None)));
        assert_eq!(app.pr_badge(path), None, "failed lookup after no PR");
        assert!(app.status.is_empty(), "a background failure stays silent");
        assert!(app.apply_pr_lookup(path, "feature", None, PrLookup::NoPr));
        assert_eq!(app.pr_badge(path), None, "no remote");
    }

    #[test]
    fn commit_link_needs_a_resolved_remote_and_a_full_id() {
        const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        assert_eq!(app.commit_link(path, SHA), None, "lookup in flight");
        assert!(app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), PrLookup::NoPr));
        assert_eq!(
            app.commit_link(path, SHA).as_deref(),
            Some("https://github.com/octo/demo/commit/0123456789abcdef0123456789abcdef01234567")
        );
        let sha256 = "ab".repeat(32);
        assert!(app.commit_link(path, &sha256).is_some(), "SHA-256 id");
        for short in ["0123456", &SHA[..39], "HEAD", &"g".repeat(40)] {
            assert_eq!(app.commit_link(path, short), None, "{short}");
        }
    }

    #[test]
    fn a_lookup_for_an_old_branch_is_dropped_and_one_after_r_lands() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
        assert!(!app.apply_pr_lookup(
            path,
            "feature",
            Some(REMOTE.into()),
            found(PrState::InReview)
        ));
        assert_eq!(app.pr_badge(path), None);
        assert_eq!(
            app.due_pr_lookups(),
            vec![(PathBuf::from("app"), "other".to_string())],
            "the changed branch is looked up"
        );

        // `r` keeps the record of the current branch: an answer for it
        // lands, one for the old branch is still dropped.
        app.forget_pr_lookups(Some(path));
        assert!(app.apply_pr_lookup(path, "other", Some(REMOTE.into()), found(PrState::InReview)));
        assert_eq!(app.pr_badge(path), Some(PrState::InReview));
        assert!(!app.apply_pr_lookup(path, "feature", Some(REMOTE.into()), found(PrState::Ready)));
        assert_eq!(app.pr_badge(path), Some(PrState::InReview));
    }

    #[test]
    fn a_cached_answer_is_hidden_once_the_checkout_leaves_its_branch() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        assert!(app.apply_pr_lookup(
            path,
            "feature",
            Some(REMOTE.into()),
            found(PrState::InReview)
        ));
        apply_one_repo_snapshot(&mut app, "app", Some(repo("app", "other")));
        assert_eq!(app.pr_badge(path), None, "answer is for another branch");
    }

    #[test]
    fn due_lookups_skip_detached_and_hidden_and_drop_vanished_checkouts() {
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
        assert!(app.apply_pr_lookup(
            path,
            "feature",
            Some(REMOTE.into()),
            found(PrState::InReview)
        ));
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
            kind: IconKind::PrInReview,
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
    use super::super::super::pr_status::{checks_ci_link, ChecksSummary, PrNotes};
    use super::super::super::pull_request::PullRequestDetail;

    /// `app` on `feature` with a ready open PR #`number` in the badge cache.
    fn app_with_pr(number: u64) -> AppState {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let _ = app.due_pr_lookups();
        set_answer(&mut app, number, PrState::InReview);
        app
    }

    fn set_answer(app: &mut AppState, number: u64, state: PrState) {
        let pr = PullRequest::fixture(
            number,
            &format!("https://github.com/octo/demo/pull/{number}"),
            state,
        );
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
            reviews: None,
            checks: ChecksSummary::default(),
            pipeline: None,
            check_names: Vec::new(),
            more_checks: 0,
            notes: PrNotes::default(),
            ci_link: checks_ci_link(&[], &format!("https://github.com/octo/demo/pull/{number}")),
            updated_at: None,
        })
    }

    /// Land the answer to the request the cache waits for, a first fetch or
    /// a new fetch (0 when it waits for none).
    fn land(app: &mut AppState, branch: &str, number: u64, lookup: PrDetailLookup) -> bool {
        let first = app
            .pr_cache
            .details
            .values()
            .find_map(|detail| match detail {
                PrDetailState::Loading(request) => Some(*request),
                _ => None,
            });
        let again = app
            .pr_cache
            .detail_refresh
            .values()
            .find_map(|refresh| match refresh {
                DetailRefresh::Loading { request, .. } => Some(*request),
                DetailRefresh::Due => None,
            });
        land_request(app, branch, number, first.or(again).unwrap_or(0), lookup)
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
        set_answer(&mut app, 7, PrState::InReview);
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)]),
            "a pin on a fresh answer asks"
        );
        assert!(land(&mut app, "feature", 7, PrDetailLookup::Failed(None)));
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
        set_answer(&mut app, 7, PrState::InReview);
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)])
        );
        assert!(
            !land_request(&mut app, "feature", 7, 1, PrDetailLookup::Failed(None)),
            "the old request times out late: dropped"
        );
        assert_eq!(detail_of(&app), Some(PrDetailState::Loading(2)));
        assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
        assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
        assert!(
            !land_request(&mut app, "feature", 7, 1, PrDetailLookup::Failed(None)),
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
    fn a_branch_switch_r_and_another_pr_drop_the_detail() {
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));

        // `gx` with the same PR keeps the detail, even when its answer
        // changed; another PR drops it.
        app.apply_pr_open(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            found(PrState::InReview),
            true,
        );
        assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
        assert!(app.pr_cache.detail_refresh.is_empty(), "same answer");
        app.apply_pr_open(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            found(PrState::Ready),
            true,
        );
        assert!(
            matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
            "the same PR changed under gx: the detail stays"
        );
        set_answer(&mut app, 8, PrState::Ready);
        assert_eq!(detail_of(&app), None, "another PR under gx");
        assert!(app.pr_cache.details.is_empty());
        set_answer(&mut app, 7, PrState::Ready);
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 2)])
        );

        // `r` forgets the checkout and its detail.
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        app.forget_pr_lookups(Some(Path::new("app")));
        assert!(app.pr_cache.details.is_empty(), "r drops the detail");
        let _ = app.due_pr_lookups();
        set_answer(&mut app, 7, PrState::InReview);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        app.forget_pr_lookups(None);
        assert!(app.pr_cache.details.is_empty(), "a full reload drops all");

        // A branch switch drops the old branch's detail.
        let _ = app.due_pr_lookups();
        set_answer(&mut app, 7, PrState::InReview);
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
        set_answer(&mut app, 8, PrState::InReview);
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
            !land(&mut app, "feature", 7, PrDetailLookup::Failed(None)),
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
    fn a_release_after_the_detail_lands_runs_the_pressed_line() {
        use crate::tui::popover::{flat_lines, PopoverLine};
        use ratatui::{backend::TestBackend, Terminal};
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal
            .draw(|frame| crate::tui::render::draw(frame, &mut app))
            .unwrap();
        let painted = app.layout.popover.clone().expect("painted");
        let sections = app.open_popover_sections();
        let lines = flat_lines(&sections);
        let (y, index) = *painted
            .lines
            .iter()
            .find(|(_, index)| {
                matches!(lines[*index], PopoverLine::Action(command)
                    if command.action == Action::OpenPullRequest)
            })
            .expect("Open PR line");
        app.dispatch(Action::Click {
            col: painted.inner.x + 2,
            row: y,
        });
        assert_eq!(app.popover_release_action(), Some(Action::OpenPullRequest));

        // The detail lands between press and release: the lines move.
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        let sections = app.open_popover_sections();
        assert!(
            !matches!(flat_lines(&sections)[index], PopoverLine::Action(command)
                if command.action == Action::OpenPullRequest),
            "the pressed index now holds another line"
        );
        assert_eq!(
            app.popover_release_action(),
            Some(Action::OpenPullRequest),
            "the press follows its line"
        );
        let mut twin = app.clone();
        twin.close_popover();
        let want = twin.dispatch(Action::OpenPullRequest);
        assert_eq!(app.dispatch(Action::Release), want);
        assert!(app.popover.is_none());
    }

    #[test]
    fn watch_ticks_never_request_a_detail() {
        let mut app = app_with_pr(7);
        focus(&mut app, "repo:app");
        let _ = click_badge(&mut app);
        assert!(app.popover_pinned());
        // Another PR drops the detail while the popover stays pinned.
        set_answer(&mut app, 8, PrState::Ready);
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

    // ---- polling, backoff, stale state, transitions ----

    use std::time::{Duration, Instant};

    use super::super::super::pr_poll::PR_POLL_CALM_MS;

    const GH_REMOTE: &str = "git@github.com:acme/app.git";
    const GL_REMOTE: &str = "git@gitlab.com:acme/services/api.git";

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// Checkout `name` on `branch`, pushed (it has an upstream).
    fn pushed_on(name: &str, branch: &str) -> RepoSnapshot {
        let mut row = repo(name, branch);
        row.sync_status = SyncStatus::UpToDate;
        row
    }

    fn pushed(name: &str) -> RepoSnapshot {
        pushed_on(name, "feature")
    }

    /// GitHub remote of repository `acme/<name>`. Answers are cached per
    /// (remote URL, branch), so each checkout of a test needs its own.
    fn gh(name: &str) -> String {
        format!("git@github.com:acme/{name}.git")
    }

    /// GitLab remote of project `acme/services/<name>`.
    fn gl(name: &str) -> String {
        format!("git@gitlab.com:acme/services/{name}.git")
    }

    /// PR `number` in `state` on the forge of `remote`.
    fn pr_on(remote: &str, number: u64, state: PrState) -> PullRequest {
        let url = if remote.contains("gitlab") {
            format!("https://gitlab.com/acme/services/api/-/merge_requests/{number}")
        } else {
            format!("https://github.com/acme/app/pull/{number}")
        };
        PullRequest::fixture(number, &url, state)
    }

    /// [`pr_on`] while the forge still computes its merge facts.
    fn computing(remote: &str, number: u64, state: PrState) -> PullRequest {
        PullRequest {
            merge_state_settled: false,
            ..pr_on(remote, number, state)
        }
    }

    fn open_on(remote: &str, number: u64) -> PrLookup {
        PrLookup::Found(pr_on(remote, number, PrState::InReview))
    }

    /// Land `lookup` of `feature` for checkout `name` at `remote` at `now`.
    fn land_pr(app: &mut AppState, name: &str, remote: &str, lookup: PrLookup, now: Instant) {
        assert!(
            app.apply_pr_lookup_at(Path::new(name), "feature", Some(remote.into()), lookup, now),
            "{name} lands"
        );
    }

    /// Checkouts the poll cycle at `now` looks up, by name.
    fn polled(app: &mut AppState, now: Instant) -> Vec<String> {
        let mut names: Vec<String> = app
            .due_pr_polls(now)
            .into_iter()
            .map(|(path, branch)| {
                assert_eq!(branch, "feature");
                path.display().to_string()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn pollable_checkouts_show_an_open_pr_or_a_pushed_branch_with_no_pr_yet() {
        let mut app = app_with(
            &[
                repo("open", "feature"),
                pushed("lab"),
                pushed("merged"),
                pushed("nopr"),
                repo("local", "feature"),
                pushed("failed"),
                pushed("other"),
                pushed("pending"),
                repo("det", DETACHED_HEAD_BRANCH),
                pushed("vendor"),
            ],
            &["vendor"],
        );
        app.show_hidden = true;
        assert_eq!(app.due_pr_lookups().len(), 9, "every checkout on a branch");
        let t0 = Instant::now();
        assert!(polled(&mut app, t0).is_empty(), "no remote resolved yet");
        let remote = |name: &str| match name {
            "lab" | "failed" => gl(name),
            "other" => "https://git.example.org/acme/other.git".to_string(),
            _ => gh(name),
        };
        let answer = |name: &str| match name {
            "open" | "vendor" => open_on(&remote(name), 1),
            "lab" => PrLookup::Found(pr_on(&remote(name), 2, PrState::Draft)),
            "merged" => PrLookup::Found(pr_on(&remote(name), 3, PrState::Merged)),
            "failed" => PrLookup::Failed(None),
            _ => PrLookup::NoPr,
        };
        for name in [
            "open", "lab", "merged", "nopr", "local", "failed", "other", "vendor",
        ] {
            land_pr(&mut app, name, &remote(name), answer(name), t0);
        }

        let t1 = t0 + secs(180);
        let due = polled(&mut app, t1);
        assert_eq!(
            due,
            ["failed", "lab", "nopr", "open", "vendor"],
            "open PRs (with or without an upstream), and pushed branches with no PR \
             known; not a merged PR, an unpushed branch with no PR, another forge, \
             a lookup in flight, or a detached HEAD"
        );
        for name in &due {
            let lookup = match answer(name) {
                PrLookup::Failed(_) => PrLookup::NoPr,
                other => other,
            };
            land_pr(&mut app, name, &remote(name), lookup, t1);
        }
        app.show_hidden = false;
        assert_eq!(
            polled(&mut app, t1 + secs(180)),
            ["failed", "lab", "nopr", "open"],
            "a hidden repo polls only while hidden repos show"
        );
    }

    #[test]
    fn a_poll_keeps_the_badge_and_skips_a_checkout_still_in_flight() {
        let mut app = app_with(&[pushed("app")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        land_pr(&mut app, "app", GH_REMOTE, open_on(GH_REMOTE, 7), t0);
        let t1 = t0 + secs(180);
        assert_eq!(polled(&mut app, t1), ["app"]);
        assert_eq!(
            app.pr_badge(path),
            Some(PrState::InReview),
            "the badge stays"
        );
        assert!(
            polled(&mut app, t1 + secs(180)).is_empty(),
            "a lookup in flight is not polled again"
        );
        assert!(
            app.due_pr_lookups().is_empty(),
            "a status apply on the same branch asks nothing"
        );
        assert_eq!(app.pr_badge(path), Some(PrState::InReview));
        let running = PrLookup::Found(pr_on(GH_REMOTE, 7, PrState::CiRunning));
        land_pr(&mut app, "app", GH_REMOTE, running, t1 + secs(2));
        assert_eq!(app.pr_badge(path), Some(PrState::CiRunning));
        assert_eq!(polled(&mut app, t1 + secs(240)), ["app"]);
    }

    #[test]
    fn the_poll_period_is_hot_while_a_pollable_pr_runs_ci_or_is_ready() {
        let mut app = app_with(&[pushed("app"), pushed("vendor")], &["vendor"]);
        app.show_hidden = true;
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        land_pr(
            &mut app,
            "vendor",
            &gh("vendor"),
            open_on(&gh("vendor"), 2),
            t0,
        );
        for (state, want) in [
            (PrState::InReview, 180_000),
            (PrState::Draft, 180_000),
            (PrState::CiFailed, 180_000),
            (PrState::CiRunning, 60_000),
            (PrState::Ready, 60_000),
            (PrState::Merged, 180_000),
        ] {
            let lookup = PrLookup::Found(pr_on(GH_REMOTE, 1, state));
            land_pr(&mut app, "app", GH_REMOTE, lookup, t0);
            assert_eq!(app.pr_poll_interval_ms(), want, "{state:?}");
        }
        let running = PrLookup::Found(pr_on(&gh("vendor"), 2, PrState::CiRunning));
        land_pr(&mut app, "vendor", &gh("vendor"), running, t0);
        assert_eq!(app.pr_poll_interval_ms(), 60_000, "a shown hidden repo");
        app.show_hidden = false;
        // The next status apply syncs the records with the snapshot.
        assert!(app.due_pr_lookups().is_empty());
        assert_eq!(
            app.pr_poll_interval_ms(),
            180_000,
            "a hidden repo is not pollable"
        );

        app.set_pr_poll_mode(PrPollMode::Every(300));
        assert_eq!(app.pr_poll_interval_ms(), 300);
        app.set_pr_poll_mode(PrPollMode::Off);
        assert_eq!(app.pr_poll_interval_ms(), 0);
        assert!(polled(&mut app, t0 + secs(3600)).is_empty(), "polling off");
    }

    /// Run the poll cycle at `at` over `app` (GitHub PR 1), `lib` (GitHub
    /// PR 2), and `lab` (GitLab MR 3): land each polled checkout, the GitHub
    /// ones failed when `github_fails`. Returns the polled names.
    fn run_cycle(app: &mut AppState, at: Instant, github_fails: bool) -> Vec<String> {
        let names = polled(app, at);
        for name in &names {
            let (remote, number) = match name.as_str() {
                "app" => (gh("app"), 1),
                "lib" => (gh("lib"), 2),
                _ => (gl("lab"), 3),
            };
            let lookup = if github_fails && remote.contains("github") {
                PrLookup::Failed(Some("HTTP 502: Bad Gateway".into()))
            } else {
                open_on(&remote, number)
            };
            land_pr(app, name, &remote, lookup, at + secs(1));
        }
        names
    }

    #[test]
    fn a_failed_poll_doubles_its_host_period_up_to_fifteen_minutes() {
        let mut app = app_with(&[pushed("app"), pushed("lab"), pushed("lib")], &[]);
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        land_pr(&mut app, "app", &gh("app"), open_on(&gh("app"), 1), t0);
        land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
        land_pr(&mut app, "lab", &gl("lab"), open_on(&gl("lab"), 3), t0);
        assert_eq!(app.pr_poll_interval_ms(), PR_POLL_CALM_MS);
        // Cycle k starts k × 180 s after t0. Both GitHub checkouts fail in
        // one cycle, which counts once. github.com then waits 360 s, 720 s,
        // 900 s (the cap), 900 s; GitLab keeps 180 s. A success resets it.
        let github_polled = [1, 3, 7, 12, 17, 18];
        let github_fails = [1, 3, 7, 12];
        for k in 1..=18u64 {
            let want: &[&str] = if github_polled.contains(&k) {
                &["app", "lab", "lib"]
            } else {
                &["lab"]
            };
            let at = t0 + secs(180 * k);
            assert_eq!(
                run_cycle(&mut app, at, github_fails.contains(&k)),
                want,
                "cycle {k}"
            );
            assert_eq!(
                app.pr_badge(Path::new("app")),
                Some(PrState::InReview),
                "cycle {k}: a failed poll keeps the badge"
            );
        }
    }

    #[test]
    fn a_failed_lookup_keeps_a_known_badge_and_records_why() {
        let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
        let (path, lib) = (Path::new("app"), Path::new("lib"));
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        let reason = |app: &AppState, path| {
            app.pr_lookup_failure(path)
                .map(|failure| failure.reason.clone())
        };

        land_pr(
            &mut app,
            "lib",
            &gh("lib"),
            PrLookup::Failed(Some("gh not found".into())),
            t0,
        );
        assert_eq!(app.pr_badge(lib), None, "a failed first lookup shows none");
        assert_eq!(reason(&app, lib), Some(Some("gh not found".into())));
        assert_eq!(app.pr_checked_at(lib), None);

        let ready = PrLookup::Found(pr_on(GH_REMOTE, 1, PrState::Ready));
        land_pr(&mut app, "app", GH_REMOTE, ready, t0);
        assert_eq!(app.pr_checked_at(path), Some(t0));
        assert_eq!(reason(&app, path), None);
        land_pr(
            &mut app,
            "app",
            GH_REMOTE,
            PrLookup::Failed(None),
            t0 + secs(5),
        );
        assert_eq!(app.pr_badge(path), Some(PrState::Ready), "the badge stays");
        assert_eq!(reason(&app, path), Some(None), "failed, with no reason");
        assert_eq!(app.pr_checked_at(path), Some(t0), "the last success");

        app.apply_pr_open(
            path,
            "feature",
            Some(GH_REMOTE.into()),
            PrLookup::Failed(Some("HTTP 401: Bad credentials".into())),
            false,
        );
        assert_eq!(app.status, "could not look up PR for feature");
        assert_eq!(app.pr_badge(path), Some(PrState::Ready), "gx keeps it too");
        assert_eq!(
            reason(&app, path),
            Some(Some("HTTP 401: Bad credentials".into()))
        );
        assert!(
            app.take_pr_transitions().is_empty(),
            "a failure never alerts"
        );

        let t1 = t0 + secs(60);
        land_pr(&mut app, "app", GH_REMOTE, PrLookup::NoPr, t1);
        assert_eq!(reason(&app, path), None, "a success clears it");
        assert_eq!(app.pr_checked_at(path), Some(t1));
        assert_eq!(app.pr_badge(path), None);
    }

    #[test]
    fn a_polled_badge_goes_stale_after_twice_the_planned_period() {
        let mut app = app_with(&[pushed("app")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        assert!(!app.pr_is_stale(path, t0 + secs(3600)), "no answer yet");
        land_pr(&mut app, "app", GH_REMOTE, open_on(GH_REMOTE, 1), t0);
        assert!(!app.pr_is_stale(path, t0 + secs(360)), "calm: 2 × 180 s");
        assert!(app.pr_is_stale(path, t0 + secs(361)));

        assert_eq!(polled(&mut app, t0 + secs(180)), ["app"]);
        let failed = PrLookup::Failed(Some("HTTP 502: Bad Gateway".into()));
        land_pr(&mut app, "app", GH_REMOTE, failed, t0 + secs(181));
        assert!(
            app.pr_is_stale(path, t0 + secs(400)),
            "a failed poll keeps the old check time"
        );
        assert_eq!(app.pr_badge(path), Some(PrState::InReview));

        let t1 = t0 + secs(500);
        let running = PrLookup::Found(pr_on(GH_REMOTE, 1, PrState::CiRunning));
        land_pr(&mut app, "app", GH_REMOTE, running, t1);
        assert!(!app.pr_is_stale(path, t1 + secs(120)), "hot: 2 × 60 s");
        assert!(app.pr_is_stale(path, t1 + secs(121)));

        app.set_pr_poll_mode(PrPollMode::Every(300));
        assert!(app.pr_is_stale(path, t1 + Duration::from_millis(601)));
        app.set_pr_poll_mode(PrPollMode::Off);
        assert!(!app.pr_is_stale(path, t1 + secs(9999)), "polling off");
        app.set_pr_poll_mode(PrPollMode::HotCalm);
        let merged = PrLookup::Found(pr_on(GH_REMOTE, 1, PrState::Merged));
        land_pr(&mut app, "app", GH_REMOTE, merged, t1);
        assert!(
            !app.pr_is_stale(path, t1 + secs(9999)),
            "a merged PR is not polled"
        );
    }

    /// `app` on `feature` after a first lookup of `before` at `remote`.
    fn seen_app(remote: &str, before: PrLookup) -> AppState {
        let mut app = app_with(&[pushed("app")], &[]);
        let _ = app.due_pr_lookups();
        land_pr(&mut app, "app", remote, before, Instant::now());
        assert!(
            app.take_pr_transitions().is_empty(),
            "never on the first lookup"
        );
        app
    }

    /// Land `pr` (or no PR) for `app` at `remote` and take the transitions.
    fn changes(app: &mut AppState, remote: &str, pr: Option<PullRequest>) -> Vec<PrTransition> {
        let lookup = pr.map_or(PrLookup::NoPr, PrLookup::Found);
        land_pr(app, "app", remote, lookup, Instant::now());
        app.take_pr_transitions()
    }

    fn transition(pr: PullRequest, from: PrState, to: PrTransitionTo) -> PrTransition {
        PrTransition {
            checkout: PathBuf::from("app"),
            pr,
            from,
            to,
        }
    }

    #[test]
    fn a_change_into_ci_failed_ready_or_merged_or_a_closed_pr_queues_a_transition() {
        use PrState::*;
        for remote in [GH_REMOTE, GL_REMOTE] {
            for (from, to) in [
                (InReview, CiFailed),
                (CiRunning, CiFailed),
                (CiRunning, Ready),
                (InReview, Ready),
                (Draft, Ready),
                (Ready, Merged),
                (CiFailed, Merged),
            ] {
                let mut app = seen_app(remote, PrLookup::Found(pr_on(remote, 12, from)));
                let pr = pr_on(remote, 12, to);
                assert_eq!(
                    changes(&mut app, remote, Some(pr.clone())),
                    vec![transition(pr, from, PrTransitionTo::State(to))],
                    "{remote}: {from:?} -> {to:?}"
                );
                assert!(app.take_pr_transitions().is_empty(), "drained");
            }

            let before = pr_on(remote, 12, CiRunning);
            let mut app = seen_app(remote, PrLookup::Found(before.clone()));
            assert_eq!(
                changes(&mut app, remote, None),
                vec![transition(before, CiRunning, PrTransitionTo::Closed)],
                "{remote}: closed"
            );

            // A failed lookup in between keeps the state before it.
            let mut app = seen_app(remote, open_on(remote, 12));
            land_pr(
                &mut app,
                "app",
                remote,
                PrLookup::Failed(None),
                Instant::now(),
            );
            let failed = pr_on(remote, 12, CiFailed);
            assert_eq!(
                changes(&mut app, remote, Some(failed.clone())),
                vec![transition(
                    failed,
                    InReview,
                    PrTransitionTo::State(CiFailed)
                )],
                "{remote}: across a failed lookup"
            );
        }
    }

    #[test]
    fn other_pr_changes_update_silently() {
        use PrState::*;
        for remote in [GH_REMOTE, GL_REMOTE] {
            for (from, to) in [
                (InReview, CiRunning),
                (CiFailed, CiRunning),
                (Ready, InReview),
                (InReview, Draft),
                (CiFailed, CiFailed),
                (Ready, Ready),
            ] {
                let mut app = seen_app(remote, PrLookup::Found(pr_on(remote, 12, from)));
                assert!(
                    changes(&mut app, remote, Some(pr_on(remote, 12, to))).is_empty(),
                    "{remote}: {from:?} -> {to:?}"
                );
            }
            let mut app = seen_app(remote, open_on(remote, 12));
            assert!(
                changes(&mut app, remote, Some(pr_on(remote, 13, CiFailed))).is_empty(),
                "{remote}: another PR number"
            );
            let mut app = seen_app(remote, PrLookup::Found(pr_on(remote, 12, Merged)));
            assert!(
                changes(&mut app, remote, None).is_empty(),
                "{remote}: a merged PR that goes away"
            );
            let mut app = seen_app(remote, PrLookup::NoPr);
            assert!(
                changes(&mut app, remote, Some(pr_on(remote, 12, CiFailed))).is_empty(),
                "{remote}: a new PR"
            );
        }
    }

    #[test]
    fn no_transition_after_a_branch_change_and_r_still_compares() {
        let mut app = seen_app(GH_REMOTE, open_on(GH_REMOTE, 7));
        let path = Path::new("app");
        let failed = || Some(pr_on(GH_REMOTE, 7, PrState::CiFailed));

        // feature → other → feature, each answer landed.
        apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
        let _ = app.due_pr_lookups();
        assert!(app.apply_pr_lookup(path, "other", Some(GH_REMOTE.into()), PrLookup::NoPr));
        apply_one_repo_snapshot(&mut app, "app", Some(pushed("app")));
        let _ = app.due_pr_lookups();
        assert!(
            changes(&mut app, GH_REMOTE, failed()).is_empty(),
            "the first lookup after a branch change"
        );

        // Away and back before any answer lands.
        apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
        let _ = app.due_pr_lookups();
        apply_one_repo_snapshot(&mut app, "app", Some(pushed("app")));
        let _ = app.due_pr_lookups();
        let ready = pr_on(GH_REMOTE, 7, PrState::Ready);
        assert!(
            changes(&mut app, GH_REMOTE, Some(ready.clone())).is_empty(),
            "a switch away and back"
        );

        // `r` keeps the badge while it looks again, and the last answer
        // seen: the same state is quiet, a change alerts.
        app.forget_pr_lookups(Some(path));
        assert!(app.take_pr_transitions().is_empty(), "a forget queues none");
        assert_eq!(app.pr_badge(path), Some(PrState::Ready));
        let _ = app.due_pr_lookups();
        assert!(changes(&mut app, GH_REMOTE, Some(ready)).is_empty());
        app.forget_pr_lookups(None);
        let _ = app.due_pr_lookups();
        let merged = pr_on(GH_REMOTE, 7, PrState::Merged);
        assert_eq!(
            changes(&mut app, GH_REMOTE, Some(merged.clone())),
            vec![transition(
                merged,
                PrState::Ready,
                PrTransitionTo::State(PrState::Merged)
            )],
            "a full reload compares with the last answer too"
        );
    }

    #[test]
    fn ready_versus_in_review_alerts_only_on_settled_merge_facts() {
        use PrState::*;
        for remote in [GH_REMOTE, GL_REMOTE] {
            let mut app = seen_app(remote, open_on(remote, 5));
            assert!(
                changes(&mut app, remote, Some(computing(remote, 5, Ready))).is_empty(),
                "{remote}: ready while the forge computes"
            );
            assert_eq!(
                app.pr_badge(Path::new("app")),
                Some(Ready),
                "the badge shows it"
            );
            assert!(
                changes(&mut app, remote, Some(computing(remote, 5, InReview))).is_empty(),
                "{remote}: back to in review while it computes"
            );
            let ready = pr_on(remote, 5, Ready);
            assert_eq!(
                changes(&mut app, remote, Some(ready.clone())),
                vec![transition(ready, InReview, PrTransitionTo::State(Ready))],
                "{remote}: the settled answer alerts once"
            );
            assert!(
                changes(&mut app, remote, Some(computing(remote, 5, InReview))).is_empty(),
                "{remote}: in review while it computes"
            );
            assert!(
                changes(&mut app, remote, Some(pr_on(remote, 5, Ready))).is_empty(),
                "{remote}: still ready as last settled: no flap"
            );
            let failed = computing(remote, 5, CiFailed);
            assert_eq!(
                changes(&mut app, remote, Some(failed.clone())),
                vec![transition(failed, Ready, PrTransitionTo::State(CiFailed))],
                "{remote}: CI failed still alerts"
            );

            let mut app = seen_app(remote, PrLookup::Found(computing(remote, 6, InReview)));
            assert!(
                changes(&mut app, remote, Some(pr_on(remote, 6, Ready))).is_empty(),
                "{remote}: an answer that still computes is not the before"
            );
        }
    }

    #[test]
    fn a_poll_with_the_same_pr_keeps_the_detail_and_another_pr_drops_it() {
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        let t0 = Instant::now();
        assert_eq!(
            app.due_pr_polls(t0),
            vec![(PathBuf::from("app"), "feature".to_string())]
        );
        // Same PR, new update time: a changed answer.
        let mut changed =
            PullRequest::fixture(7, "https://github.com/octo/demo/pull/7", PrState::InReview);
        changed.updated_at = Some(1_767_312_000);
        assert!(app.apply_pr_lookup_at(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            PrLookup::Found(changed),
            t0 + secs(1)
        ));
        assert!(
            matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
            "the same PR keeps its detail"
        );
        assert_eq!(
            app.pr_cache.detail_refresh.values().collect::<Vec<_>>(),
            vec![&DetailRefresh::Due],
            "marked for a new fetch"
        );
        set_answer(&mut app, 8, PrState::InReview);
        assert_eq!(detail_of(&app), None, "another PR drops it");
        assert!(app.pr_cache.details.is_empty());
        assert!(app.pr_cache.detail_refresh.is_empty());
    }

    #[test]
    fn a_pinned_popover_asks_again_after_its_pr_answer_changed() {
        let title = |app: &AppState| match detail_of(app) {
            Some(PrDetailState::Ready(detail)) => detail.title,
            other => panic!("a ready detail expected: {other:?}"),
        };
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        assert!(app.popover_pinned());
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        assert_eq!(app.request_popover_pr_details(), Effect::None, "no change");

        set_answer(&mut app, 7, PrState::Ready);
        assert_eq!(
            app.request_popover_pr_details(),
            fetch(7, 2),
            "a changed answer asks again"
        );
        assert_eq!(title(&app), "Add login", "the old detail stays meanwhile");
        assert_eq!(app.request_popover_pr_details(), Effect::None, "asked once");
        let PrDetailLookup::Found(mut newer) = pr_detail(7) else {
            unreachable!()
        };
        newer.title = "Add login and logout".into();
        assert!(land_request(
            &mut app,
            "feature",
            7,
            2,
            PrDetailLookup::Found(newer)
        ));
        assert_eq!(title(&app), "Add login and logout");
        assert!(app.pr_cache.detail_refresh.is_empty());

        // A failed new fetch keeps the detail that landed.
        set_answer(&mut app, 7, PrState::CiRunning);
        assert_eq!(app.request_popover_pr_details(), fetch(7, 3));
        assert!(land_request(
            &mut app,
            "feature",
            7,
            3,
            PrDetailLookup::Failed(None)
        ));
        assert_eq!(title(&app), "Add login and logout");

        // "Refresh this PR" marks it too, with no change in the answer.
        assert_eq!(
            app.refresh_pull_requests(Some(Path::new("app"))),
            Effect::LookupPullRequests {
                targets: vec![(PathBuf::from("app"), "feature".to_string())]
            }
        );
        assert_eq!(app.request_popover_pr_details(), fetch(7, 4));
    }

    #[test]
    fn poll_cycles_never_request_a_detail_on_their_own() {
        let mut app = app_with_pr(7);
        focus(&mut app, "repo:app");
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        assert_eq!(
            app.due_pr_polls(Instant::now()),
            vec![(PathBuf::from("app"), "feature".to_string())]
        );
        assert!(
            matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
            "a poll keeps the detail"
        );
        for tick in [Action::WatchTick, Action::FetchTick] {
            let effect = app.dispatch(tick.clone());
            assert!(
                !format!("{effect:?}").contains("LookupPullRequestDetail"),
                "{tick:?}: {effect:?}"
            );
        }
        set_answer(&mut app, 7, PrState::InReview);
        assert_eq!(
            app.request_popover_pr_details(),
            Effect::None,
            "the same answer asks nothing"
        );
        assert!(app.pr_cache.detail_refresh.is_empty());
    }

    #[test]
    fn refresh_looks_up_now_keeps_the_badge_and_skips_the_backoff() {
        let mut app = app_with(
            &[pushed("app"), pushed("lib"), repo("local", "feature")],
            &[],
        );
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        land_pr(&mut app, "app", GH_REMOTE, open_on(GH_REMOTE, 1), t0);
        land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
        land_pr(&mut app, "local", &gh("local"), PrLookup::NoPr, t0);
        assert_eq!(run_cycle(&mut app, t0 + secs(180), true), ["app", "lib"]);
        assert!(
            polled(&mut app, t0 + secs(360)).is_empty(),
            "github.com backs off"
        );

        let targets = |effect: Effect| match effect {
            Effect::LookupPullRequests { targets } => {
                let mut names: Vec<String> = targets
                    .into_iter()
                    .map(|(path, _)| path.display().to_string())
                    .collect();
                names.sort();
                names
            }
            other => panic!("lookups expected: {other:?}"),
        };
        assert_eq!(targets(app.refresh_pull_requests(Some(path))), ["app"]);
        assert_eq!(
            app.pr_badge(path),
            Some(PrState::InReview),
            "the badge stays"
        );
        assert_eq!(
            app.refresh_pull_requests(Some(path)),
            Effect::None,
            "already in flight"
        );
        assert_eq!(
            targets(app.refresh_pull_requests(None)),
            ["lib", "local"],
            "every checkout with a forge remote, polled or not"
        );
    }

    /// `app` on `feature` with an open PR 7 at `remote` checked at `t0`,
    /// then `r` (`Some`) or a full reload (`None`) and the status apply
    /// that queues the new lookup.
    fn reloaded(remote: &str, full: bool, t0: Instant) -> AppState {
        let mut app = app_with(&[pushed("app")], &[]);
        let path = Path::new("app");
        let _ = app.due_pr_lookups();
        land_pr(&mut app, "app", remote, open_on(remote, 7), t0);
        app.forget_pr_lookups((!full).then_some(path));
        assert_eq!(
            app.pr_badge(path),
            Some(PrState::InReview),
            "the badge stays while it looks again"
        );
        assert_eq!(
            app.due_pr_lookups(),
            vec![(PathBuf::from("app"), "feature".to_string())],
            "the next status apply looks it up again"
        );
        assert!(app.due_pr_lookups().is_empty(), "once");
        app
    }

    #[test]
    fn r_or_a_reload_with_a_failing_cli_keeps_the_badge_and_records_the_error() {
        let path = Path::new("app");
        for remote in [GH_REMOTE, GL_REMOTE] {
            for full in [false, true] {
                let what = format!("{remote}, full reload {full}");
                let t0 = Instant::now();
                let mut app = reloaded(remote, full, t0);
                let failed = PrLookup::Failed(Some("HTTP 401: Bad credentials".into()));
                land_pr(&mut app, "app", remote, failed, t0 + secs(30));
                assert_eq!(app.pr_badge(path), Some(PrState::InReview), "{what}");
                assert_eq!(
                    app.pr_lookup_failure(path)
                        .and_then(|failure| failure.reason.as_deref()),
                    Some("HTTP 401: Bad credentials"),
                    "{what}"
                );
                assert_eq!(app.pr_checked_at(path), Some(t0), "{what}: last success");
                assert!(!app.pr_is_stale(path, t0 + secs(360)), "{what}");
                assert!(
                    app.pr_is_stale(path, t0 + secs(361)),
                    "{what}: stale dims it like a failed poll"
                );
            }
        }
    }

    #[test]
    fn r_or_a_reload_with_no_pr_clears_the_badge() {
        let path = Path::new("app");
        for remote in [GH_REMOTE, GL_REMOTE] {
            for full in [false, true] {
                let what = format!("{remote}, full reload {full}");
                let t0 = Instant::now();
                let mut app = reloaded(remote, full, t0);
                land_pr(&mut app, "app", remote, PrLookup::NoPr, t0 + secs(30));
                assert_eq!(app.pr_badge(path), None, "{what}");
                assert_eq!(app.pr_lookup_failure(path), None, "{what}");
                assert_eq!(app.pr_checked_at(path), Some(t0 + secs(30)), "{what}");
            }
        }
    }

    /// `app` (GitHub PR 1) and `lib` (GitHub PR 2), two repos on one host,
    /// both open and looked up at `t0`.
    fn one_host_app(t0: Instant) -> AppState {
        let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
        let _ = app.due_pr_lookups();
        land_pr(&mut app, "app", &gh("app"), open_on(&gh("app"), 1), t0);
        land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), t0);
        app
    }

    #[test]
    fn one_failing_checkout_never_backs_off_a_host_that_answers_in_the_same_cycle() {
        let not_found = || PrLookup::Failed(Some("HTTP 404: Not Found".into()));
        for fail_first in [true, false] {
            let t0 = Instant::now();
            let mut app = one_host_app(t0);
            for k in 1..=4u64 {
                let at = t0 + secs(180 * k);
                assert_eq!(
                    polled(&mut app, at),
                    ["app", "lib"],
                    "fail first {fail_first}: cycle {k} polls both"
                );
                let (first, second) = (at + secs(1), at + secs(2));
                if fail_first {
                    land_pr(&mut app, "app", &gh("app"), not_found(), first);
                    land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), second);
                } else {
                    land_pr(&mut app, "lib", &gh("lib"), open_on(&gh("lib"), 2), first);
                    land_pr(&mut app, "app", &gh("app"), not_found(), second);
                }
            }
            let end = t0 + secs(180 * 4 + 3);
            assert!(
                !app.pr_is_stale(Path::new("lib"), end),
                "fail first {fail_first}: the healthy badge stays fresh"
            );
            assert!(
                app.pr_lookup_failure(Path::new("app")).is_some(),
                "fail first {fail_first}: the failing one records it"
            );
        }
    }

    #[test]
    fn a_cycle_where_every_lookup_on_a_host_fails_backs_it_off() {
        let t0 = Instant::now();
        let mut app = one_host_app(t0);
        let at = t0 + secs(180);
        assert_eq!(polled(&mut app, at), ["app", "lib"]);
        for (name, remote) in [("app", gh("app")), ("lib", gh("lib"))] {
            let failed = PrLookup::Failed(Some("HTTP 502: Bad Gateway".into()));
            land_pr(&mut app, name, &remote, failed, at + secs(1));
        }
        assert!(
            polled(&mut app, t0 + secs(360)).is_empty(),
            "github.com waits 2 × 180 s"
        );
        assert_eq!(polled(&mut app, t0 + secs(540)), ["app", "lib"]);
    }

    #[test]
    fn the_poll_period_and_the_stale_test_parse_no_remote() {
        let parses = || FORGE_HOST_PARSES.with(std::cell::Cell::get);
        let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        let running = |remote: &str| PrLookup::Found(pr_on(remote, 1, PrState::CiRunning));
        land_pr(&mut app, "app", &gh("app"), running(&gh("app")), t0);
        land_pr(&mut app, "lib", &gl("lib"), open_on(&gl("lib"), 2), t0);
        let before = parses();
        for _ in 0..100 {
            assert_eq!(app.pr_poll_interval_ms(), 60_000);
            assert!(!app.pr_is_stale(Path::new("app"), t0));
            assert!(app.pr_is_stale(Path::new("lib"), t0 + secs(121)));
        }
        assert_eq!(polled(&mut app, t0 + secs(60)), ["app", "lib"]);
        assert_eq!(parses(), before, "no remote parse per frame or per cycle");
        land_pr(
            &mut app,
            "app",
            &gh("app"),
            running(&gh("app")),
            t0 + secs(61),
        );
        assert_eq!(parses(), before, "a poll that lands the same remote");
        land_pr(
            &mut app,
            "lib",
            &gh("lib"),
            open_on(&gh("lib"), 2),
            t0 + secs(61),
        );
        assert_eq!(parses(), before + 1, "a new remote is parsed once");
    }

    #[test]
    fn a_second_change_during_a_detail_reload_asks_again_after_it_lands() {
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, pr_detail(7)));
        set_answer(&mut app, 7, PrState::Ready);
        assert_eq!(app.request_popover_pr_details(), fetch(7, 2));
        set_answer(&mut app, 7, PrState::CiRunning);
        assert_eq!(
            app.request_popover_pr_details(),
            Effect::None,
            "one fetch at a time"
        );
        assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
        assert_eq!(
            app.request_popover_pr_details(),
            fetch(7, 3),
            "that detail is older than the badge: ask again"
        );
        assert!(land_request(&mut app, "feature", 7, 3, pr_detail(7)));
        assert_eq!(app.request_popover_pr_details(), Effect::None, "up to date");
    }

    #[test]
    fn a_change_during_the_first_detail_fetch_asks_again_after_it_lands() {
        let mut app = app_with_pr(7);
        assert_eq!(
            click_badge(&mut app),
            Effect::Batch(vec![Effect::LoadRightPane, fetch(7, 1)])
        );
        set_answer(&mut app, 7, PrState::Ready);
        assert_eq!(
            app.request_popover_pr_details(),
            Effect::None,
            "waits for the first fetch"
        );
        assert!(land_request(&mut app, "feature", 7, 1, pr_detail(7)));
        assert_eq!(app.request_popover_pr_details(), fetch(7, 2));
        assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
        assert_eq!(app.request_popover_pr_details(), Effect::None);
    }
}

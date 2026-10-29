//! PR of a branch row: the `gx` / `gc` target, the PR actions, and the
//! badge cache.
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
//! that PR ([`AppState::request_pr_details`]), never on a tick. A
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

use super::super::action::{Action, Effect};
use super::super::gates::ListFocusTarget;
use super::super::hyperlink::LinkHit;
use super::super::icons::IconKind;
use super::super::popover::IconTarget;
use super::super::pr_poll::{
    pr_answer_is_stale, pr_poll_backoff_ms, pr_poll_interval_ms, PrPollMode,
};
use super::super::pr_status::PrState;
use super::super::pull_request::{
    ci_opened_status, commit_url, is_web_url, lookup_failed_status, no_ci_status, no_pr_status,
    parse_remote, Forge, PrDetailLookup, PrLookup, PrPage, PullRequest, PullRequestDetail,
    CI_OPEN_FAILED, NO_PR_FOR_ROW, OPEN_FAILED, PR_LOOKUP_RUNNING,
};
use super::super::status::StatusMessage;
use super::super::toast::{Toast, ToastMark, PR_TOAST_TTL_MS, TOAST_MAX};
use super::super::tree::{
    pr_badge_mark, pr_badge_repo, NodeKind, PrBadge, PrTotals, SegRole, PR_REF_MIN_TREE_COLS,
};
use super::super::watch::{graph_row_identity, FlashKind};
use super::prs_tab::prs_row_flash_id;
use super::AppState;

/// Error line reason when a failed PR lookup printed no reason.
const PR_LOOKUP_FAILED: &str = "lookup failed";

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

/// An action on one PR. Only these run on a PR target
/// ([`AppState::run_pr_action`]), so no other action can open a browser
/// there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PrAction {
    /// Open the PR (`gx`, Open PR).
    Open,
    /// Open its CI page (`gc`, Open PR CI).
    OpenCi,
    /// Copy its URL (Copy PR URL, the popover's Copy URL).
    CopyUrl,
    /// Look it up again now (Refresh this PR).
    Refresh,
}

impl PrAction {
    /// The PR action `action` names, or `None` for any other action.
    pub(crate) fn of(action: &Action) -> Option<Self> {
        Some(match action {
            Action::OpenPullRequest => Self::Open,
            Action::OpenPullRequestCi => Self::OpenCi,
            Action::CopyPullRequestUrl => Self::CopyUrl,
            Action::RefreshPullRequest => Self::Refresh,
            _ => return None,
        })
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
    pub(crate) reason: Option<String>,
}

/// What a PR became in a [`PrTransition`].
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
/// [`AppState::take_pr_transitions`] drains the queue;
/// [`AppState::alert_pr_transitions`] turns it into toasts and row flashes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PrTransition {
    /// Checkout path (snapshot `repo`) whose PR changed.
    pub(crate) checkout: PathBuf,
    /// The new answer. For [`PrTransitionTo::Closed`], the last answer
    /// before the PR went away.
    pub(crate) pr: PullRequest,
    /// What the PR became.
    pub(crate) to: PrTransitionTo,
}

impl PrTransition {
    /// Toast of this change, pushed at `at`, for [`PR_TOAST_TTL_MS`]:
    /// the state glyph in the state colour ([`pr_badge_mark`]; closed is
    /// [`IconKind::PrClosed`] in the deleted colour), `<ref> <state>`
    /// (`#318 CI failed`, `!12 merged`, `#45 closed`), then the first failed
    /// check's name for CI failed, else the PR's branch.
    fn toast(&self, ascii: bool, at: Instant) -> Toast {
        let (glyph, role, label) = match self.to {
            PrTransitionTo::State(state) => {
                let (glyph, role) = pr_badge_mark(ascii, state);
                (glyph, role, state.label())
            }
            PrTransitionTo::Closed => (IconKind::PrClosed.glyph(ascii), SegRole::Deleted, "closed"),
        };
        let failed_check = match self.to {
            PrTransitionTo::State(PrState::CiFailed) => self.pr.failed_check.as_deref(),
            _ => None,
        };
        Toast {
            mark: ToastMark::Glyph(glyph, role),
            text: format!("{} {label}", self.pr.ref_text()),
            detail: failed_check.unwrap_or(&self.pr.head).to_string(),
            role: SegRole::Repo,
            ttl_ms: PR_TOAST_TTL_MS,
            at,
        }
    }

    /// Flash ramp of the rows that show the checkout's PR: remove for CI
    /// failed and closed, add for ready to merge and merged.
    fn flash_kind(&self) -> FlashKind {
        match self.to {
            PrTransitionTo::State(PrState::CiFailed) | PrTransitionTo::Closed => FlashKind::Remove,
            PrTransitionTo::State(_) => FlashKind::Add,
        }
    }
}

/// The key that opens the PRs tab, named after the summary toast.
const PRS_TAB_KEY: &str = "gp";

/// The muted toast for the `more` PR changes of one alert batch past its
/// first ones, pushed at `at`: `+<more> more PR changes · gp`, the key of
/// the PRs tab that lists them all.
fn more_pr_changes_toast(more: usize, at: Instant) -> Toast {
    Toast {
        mark: ToastMark::None,
        text: format!("+{more} more PR changes"),
        detail: PRS_TAB_KEY.to_string(),
        role: SegRole::Muted,
        ttl_ms: PR_TOAST_TTL_MS,
        at,
    }
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
    /// When the lookup that queued the oldest of [`Self::transitions`]
    /// landed; `None` while the queue is empty.
    transitions_since: Option<Instant>,
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
        if self.transitions_since.is_none() && !self.transitions.is_empty() {
            self.transitions_since = Some(now);
        }
        self.set_answer((remote, branch.to_string()), BranchPr::Ready(pr));
    }
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

    /// Run PR action `action` on the PR of the focused branch row, or say
    /// there is none.
    ///
    /// The focused row is a tree repo / checkout row or a graph worktree row
    /// with a branch ([`Self::pr_target_for_focus`]); any other row says
    /// `no PR for this row`.
    pub(super) fn run_pr_action(&mut self, action: PrAction) -> Effect {
        match self.pr_target_for_focus() {
            Some((repo, branch)) => self.run_pr_action_on(action, repo, branch),
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                Effect::None
            }
        }
    }

    /// `gx` / `gc`: [`Self::run_pr_action`] on the focused row, or, inside
    /// a pinned popover, on its PR section. The popover closes first, as
    /// Enter on the line closes it. A pinned popover with no PR section
    /// says `no PR for this row` and stays open.
    pub(super) fn run_pr_key(&mut self, action: PrAction) -> Effect {
        if !self.popover_pinned() {
            return self.run_pr_action(action);
        }
        let Some(repo) = self.popover_pr_checkouts().into_iter().next() else {
            self.status = StatusMessage::warn(NO_PR_FOR_ROW);
            return Effect::None;
        };
        self.close_popover();
        self.run_pr_action_on_checkout(action, &repo)
    }

    /// Run `action` on the PR of checkout `repo`, from a line of its PR
    /// popover section: the section's own PR, which is not the focused
    /// row's when the section hangs from the graph header chip. `None` when
    /// `action` is no [`PrAction`].
    pub(super) fn run_pr_action_at(&mut self, action: &Action, repo: &Path) -> Option<Effect> {
        let action = PrAction::of(action)?;
        Some(self.run_pr_action_on_checkout(action, repo))
    }

    /// Run `action` on the PR of the branch checkout `repo` is on now, or
    /// say `no PR for this row` (detached or unknown HEAD).
    pub(super) fn run_pr_action_on_checkout(&mut self, action: PrAction, repo: &Path) -> Effect {
        match self.pr_target_for_repo(repo) {
            Some((repo, branch)) => self.run_pr_action_on(action, repo, branch),
            None => {
                self.status = StatusMessage::warn(NO_PR_FOR_ROW);
                Effect::None
            }
        }
    }

    /// Checkouts of the PR sections of the pinned popover, in section
    /// order; empty with no popover.
    fn popover_pr_checkouts(&self) -> Vec<PathBuf> {
        self.popover
            .iter()
            .flat_map(|popover| &popover.targets)
            .filter_map(|(_, target)| match target {
                IconTarget::PullRequest(repo) => Some(repo.clone()),
                _ => None,
            })
            .collect()
    }

    /// [`Self::run_pr_action`] on `branch` of checkout `repo`.
    fn run_pr_action_on(&mut self, action: PrAction, repo: PathBuf, branch: String) -> Effect {
        match action {
            PrAction::Open => Effect::OpenPullRequest {
                repo,
                branch,
                page: PrPage::Pr,
            },
            PrAction::OpenCi => self.open_pull_request_ci(repo, branch),
            PrAction::CopyUrl => match self.pull_request_for(&repo) {
                Some(pr) => Effect::CopyClipboard {
                    text: pr.url.clone(),
                    announce: true,
                },
                None => {
                    self.status = StatusMessage::warn(no_pr_status(&branch));
                    Effect::None
                }
            },
            PrAction::Refresh => self.refresh_one_pull_request(&repo),
        }
    }

    /// `gc`: open the CI page of the PR of `branch` in checkout `repo`
    /// ([`PullRequest::ci_link`]), or say why not.
    ///
    /// A PR the badge already shows opens its CI link at once (no lookup,
    /// no detail needed). A branch whose lookup answered with no PR says
    /// `no PR for <branch>`. With no answer yet (in flight or failed), a
    /// fresh lookup finds the PR first, like `gx`.
    fn open_pull_request_ci(&mut self, repo: PathBuf, branch: String) -> Effect {
        if let Some(pr) = self.pull_request_for(&repo) {
            let pr_ref = pr.ref_text();
            if !is_web_url(&pr.ci_link.url) {
                self.status = StatusMessage::warn(no_ci_status(&pr_ref));
                return Effect::None;
            }
            return Effect::OpenPullRequestCi {
                pr_ref,
                url: pr.ci_link.url.clone(),
            };
        }
        if self.pr_checked_at(&repo).is_some() {
            self.status = StatusMessage::warn(no_pr_status(&branch));
            return Effect::None;
        }
        Effect::OpenPullRequest {
            repo,
            branch,
            page: PrPage::Ci,
        }
    }

    /// Refresh this PR.
    ///
    /// Inside a pinned popover (`r`): looks up again every PR the popover
    /// shows, and the popover stays open and shows the new answer and
    /// detail when they land; a popover with no PR section does nothing.
    /// Otherwise (the palette): the PR of the focused branch row, or `no PR
    /// for this row`. Either way a PR that cannot be looked up says why
    /// ([`Self::refresh_one_pull_request`]).
    pub(super) fn refresh_pull_request(&mut self) -> Effect {
        if !self.popover_pinned() {
            return self.run_pr_action(PrAction::Refresh);
        }
        let mut effects: Vec<Effect> = self
            .popover_pr_checkouts()
            .iter()
            .map(|repo| self.refresh_one_pull_request(repo))
            .filter(|effect| *effect != Effect::None)
            .collect();
        match effects.len() {
            0 => Effect::None,
            1 => effects.remove(0),
            _ => Effect::Batch(effects),
        }
    }

    /// [`Self::refresh_pull_requests`] for checkout `repo`, or say why it
    /// looks nothing up: `PR lookup already running` while a lookup of it
    /// is queued or running, else `no PR for this row` (no GitHub or GitLab
    /// remote, no branch, or a hidden checkout).
    fn refresh_one_pull_request(&mut self, repo: &Path) -> Effect {
        let effect = self.refresh_pull_requests(Some(repo));
        if effect == Effect::None {
            let running = self
                .current_pr_record(repo)
                .is_some_and(|record| record.in_flight);
            self.status = StatusMessage::warn(if running {
                PR_LOOKUP_RUNNING
            } else {
                NO_PR_FOR_ROW
            });
        }
        effect
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
                Effect::OpenPullRequest {
                    repo,
                    branch,
                    page: PrPage::Pr,
                }
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
    pub(super) fn pr_rows(&self) -> impl Iterator<Item = &WorkspaceRepoSnapshot> {
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
    /// A checkout is pollable when it is visible, on a local branch, its
    /// first lookup for that branch landed with a GitHub or GitLab remote,
    /// and either its answer is an open PR, or no PR is known yet and the
    /// branch has an upstream (it was pushed), so a PR opened later shows.
    /// A merged PR is not polled. Reads the facts each record keeps
    /// ([`CheckoutPr::pollable`], [`CheckoutPr::host`]); it parses no
    /// remote.
    ///
    /// Empty with polling off. Skips a checkout whose lookup is still in
    /// flight, and one whose forge host backs off after failed polls.
    pub(crate) fn due_pr_polls(&mut self, now: Instant) -> Vec<(PathBuf, String)> {
        let interval = self.pr_poll_interval_ms();
        if interval == 0 {
            return Vec::new();
        }
        let due: Vec<(PathBuf, String)> = self
            .pr_rows()
            .filter_map(|row| {
                let path = Path::new(&row.repo);
                let record = self
                    .pr_cache
                    .checkouts
                    .get(path)
                    .filter(|record| record.branch == row.branch)?;
                let host = record.host.as_deref()?;
                let due = record.pollable(row.sync_status != SyncStatus::NoUpstream)
                    && !record.in_flight
                    && self.pr_cache.host_ready(host, now, interval);
                due.then(|| (path.to_path_buf(), record.branch.clone()))
            })
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
        // The answer can add or drop popover lines above the focused one
        // (notes, stack): keep the pinned focus on its line.
        let focus = self.popover_focus_key();
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
        self.restore_popover_focus(focus);
        true
    }

    /// Land a `gx` result (or a `gc` result that looked the PR up first):
    /// say what happened, and cache the fresh answer.
    ///
    /// `opened` is true when the browser started for `page` of a found PR.
    /// A `gc` whose PR has no CI web page says `no CI for <ref>`. The
    /// answer lands like a badge lookup's (a failure keeps a known badge).
    pub(crate) fn apply_pr_open(
        &mut self,
        repo: &Path,
        branch: &str,
        remote: Option<String>,
        lookup: PrLookup,
        page: PrPage,
        opened: bool,
    ) {
        self.status = match (&lookup, page) {
            (PrLookup::Found(pr), PrPage::Pr) if opened => {
                StatusMessage::ok(format!("opened PR #{}", pr.number))
            }
            (PrLookup::Found(_), PrPage::Pr) => StatusMessage::error(OPEN_FAILED),
            (PrLookup::Found(pr), PrPage::Ci) if !is_web_url(&pr.ci_link.url) => {
                StatusMessage::warn(no_ci_status(&pr.ref_text()))
            }
            (PrLookup::Found(pr), PrPage::Ci) if opened => {
                StatusMessage::ok(ci_opened_status(&pr.ref_text()))
            }
            (PrLookup::Found(_), PrPage::Ci) => StatusMessage::error(CI_OPEN_FAILED),
            (PrLookup::NoPr, _) => StatusMessage::warn(no_pr_status(branch)),
            (PrLookup::Failed(reason), _) => {
                StatusMessage::error(lookup_failed_status(branch, reason.as_deref()))
            }
        };
        if self.snapshot_branch(repo) == Some(branch) {
            let focus = self.popover_focus_key();
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
            self.restore_popover_focus(focus);
        } else if let Some(remote) = remote {
            self.pr_cache
                .set_answer((remote, branch.to_string()), lookup.into());
        }
    }

    /// Land a `gc` open of the CI page of the known PR `pr_ref`: say
    /// whether the browser started.
    pub(crate) fn apply_pr_ci_open(&mut self, pr_ref: &str, opened: bool) {
        self.status = if opened {
            StatusMessage::ok(ci_opened_status(pr_ref))
        } else {
            StatusMessage::error(CI_OPEN_FAILED)
        };
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

    /// Detail fetches for the PRs on screen, each marked loading: the PRs
    /// the open popover shows, and while the PRs tab is active every PR it
    /// lists (its review column and detail pane read the detail).
    ///
    /// A PR whose (remote URL, branch) already has a detail (loading, ready,
    /// or failed) is not asked again, so a later peek or pin reads the
    /// cache, unless the detail waits for a new fetch after its answer
    /// changed: then it is asked again and the old detail stays on screen
    /// until the answer lands. A change during the first fetch waits for
    /// that fetch to land. Called when a peek opens, a popover pins, the
    /// PRs tab opens or becomes active, and a badge or detail lookup lands;
    /// never on a watch or fetch tick.
    pub(crate) fn request_pr_details(&mut self) -> Effect {
        let mut repos: Vec<PathBuf> = self
            .popover
            .iter()
            .flat_map(|popover| &popover.targets)
            .filter_map(|(_, target)| match target {
                IconTarget::PullRequest(repo) => Some(repo.clone()),
                _ => None,
            })
            .collect();
        if self.is_prs_tab() {
            for repo in self.prs_listed_checkouts() {
                if !repos.contains(&repo) {
                    repos.push(repo);
                }
            }
        }
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

    /// Take the queued [`PrTransition`]s, oldest first.
    pub(crate) fn take_pr_transitions(&mut self) -> Vec<PrTransition> {
        self.pr_cache.transitions_since = None;
        std::mem::take(&mut self.pr_cache.transitions)
    }

    /// When the lookup that queued the oldest waiting [`PrTransition`]
    /// landed; `None` when none waits.
    pub(crate) fn pr_transitions_waiting_since(&self) -> Option<Instant> {
        self.pr_cache.transitions_since
    }

    /// Alert the queued PR changes as one batch pushed at `now`: a toast
    /// per change, oldest first ([`PrTransition::toast`]), and a flash,
    /// stamped at `now`, of each row that shows the checkout's PR
    /// ([`Self::pr_row_ids`], closed included) on the remove ramp (CI
    /// failed, closed) or the add ramp (ready to merge, merged).
    ///
    /// A batch of more than [`TOAST_MAX`] changes pushes the toasts of the
    /// first `TOAST_MAX - 1` and then `+<n> more PR changes · gp`, so the
    /// stack never drops one of its own toasts; every change still flashes.
    /// Returns true when the batch was not empty.
    pub(crate) fn alert_pr_transitions(&mut self, now: Instant) -> bool {
        let transitions = self.take_pr_transitions();
        let shown = if transitions.len() > TOAST_MAX {
            TOAST_MAX - 1
        } else {
            transitions.len()
        };
        for transition in &transitions[..shown] {
            self.toasts.push_with(transition.toast(self.ascii, now));
        }
        if transitions.len() > shown {
            self.toasts
                .push_with(more_pr_changes_toast(transitions.len() - shown, now));
        }
        let stamps: Vec<(String, FlashKind)> = transitions
            .iter()
            .flat_map(|transition| {
                let kind = transition.flash_kind();
                self.pr_row_ids(transition)
                    .into_iter()
                    .map(move |id| (id, kind))
            })
            .collect();
        self.prune_flash_state(now);
        self.stamp_flashes(stamps, now);
        !transitions.is_empty()
    }

    /// Flash ids of the rows that show the PR of `transition`, with its
    /// badge or, after the PR closed, without one: the checkout's tree row
    /// ([`pr_badge_repo`]: a checkout row, or a repo row with one checkout,
    /// never a family row), the graph's worktree row of that checkout when
    /// the graph shows its repository, and the PR's row of the PRs tab
    /// while that tab is open ([`prs_row_flash_id`]). A tree or graph row
    /// folded out of view has no id here.
    fn pr_row_ids(&self, transition: &PrTransition) -> Vec<String> {
        let checkout = transition.checkout.as_path();
        let prs_tab = self
            .tabs
            .find_prs()
            .map(|_| prs_row_flash_id(&transition.pr.url));
        let tree = self
            .rows
            .iter()
            .filter(|row| pr_badge_repo(row).is_some_and(|repo| Path::new(repo) == checkout))
            .map(|row| row.id.clone());
        let graph = match (self.graph_identity.as_ref(), self.graph.as_ref()) {
            (Some((repo, _)), Some(model)) => model
                .visible_rows()
                .iter()
                .filter(|row| {
                    matches!(row, GraphRow::Worktree(worktree) if Path::new(&worktree.path) == checkout)
                })
                .map(|row| graph_row_identity(repo, row))
                .collect(),
            _ => Vec::new(),
        };
        tree.chain(graph).chain(prs_tab).collect()
    }
}

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

    /// How far the PR lookups are, for the PRs tab loading line: the
    /// visible checkouts on a local branch whose lookup for that branch
    /// landed (an answer or a failure), and how many such checkouts there
    /// are.
    pub(crate) fn pr_lookup_progress(&self) -> (usize, usize) {
        self.pr_rows().fold((0, 0), |(done, total), row| {
            let landed = self
                .pr_cache
                .checkouts
                .get(Path::new(&row.repo))
                .filter(|record| record.branch == row.branch)
                .is_some_and(|record| record.checked_at.is_some() || record.failure.is_some());
            (done + usize::from(landed), total + 1)
        })
    }

    /// One line per forge CLI and host whose last PR lookup of a visible
    /// checkout failed, in snapshot order, with the first reason seen:
    /// `! glab (gitlab.com): 401 Unauthorized`, or `lookup failed` when the
    /// CLI printed none. A checkout whose remote is not on GitHub or GitLab
    /// has no line.
    pub(crate) fn pr_lookup_error_lines(&self) -> Vec<String> {
        let mut seen = HashSet::new();
        let mut lines = Vec::new();
        for row in self.pr_rows() {
            let Some(record) = self.current_pr_record(Path::new(&row.repo)) else {
                continue;
            };
            let Some(failure) = record.failure.as_ref() else {
                continue;
            };
            let Some(repo) = record.remote.as_deref().and_then(parse_remote) else {
                continue;
            };
            let cli = match repo.forge {
                Forge::GitHub => "gh",
                Forge::GitLab => "glab",
            };
            if !seen.insert((cli, repo.host.clone())) {
                continue;
            }
            let reason = failure.reason.as_deref().unwrap_or(PR_LOOKUP_FAILED);
            lines.push(format!("! {cli} ({}): {reason}", repo.host));
        }
        lines
    }

    /// Look up the PR of checkout `checkout` again now, or of every checkout
    /// with `None` ("Refresh this PR", palette Refresh PRs).
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
            upstream_rewrite: None,
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
            default_branch: None,
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
            page: PrPage::Pr,
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
            PrPage::Pr,
            true,
        );
        assert_eq!(app.status, "opened PR #7");
        assert_eq!(app.status.kind(), StatusKind::Ok);
        assert_eq!(app.pr_badge(path), Some(PrState::Merged));
        assert!(
            app.due_pr_lookups().is_empty(),
            "the open answered this branch"
        );

        app.apply_pr_open(
            path,
            "feature",
            Some(REMOTE.into()),
            PrLookup::NoPr,
            PrPage::Pr,
            false,
        );
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
            app.request_pr_details(),
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
            PrPage::Pr,
            true,
        );
        assert!(matches!(detail_of(&app), Some(PrDetailState::Ready(_))));
        assert!(app.pr_cache.detail_refresh.is_empty(), "same answer");
        app.apply_pr_open(
            Path::new("app"),
            "feature",
            Some(REMOTE.into()),
            found(PrState::Ready),
            PrPage::Pr,
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
                matches!(lines[*index], PopoverLine::Action { command, .. }
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
            !matches!(flat_lines(&sections)[index], PopoverLine::Action { command, .. }
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
            PrPage::Pr,
            false,
        );
        assert_eq!(
            app.status, "could not look up PR for feature: HTTP 401: Bad credentials",
            "gx names the reason"
        );
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

    fn transition(pr: PullRequest, to: PrTransitionTo) -> PrTransition {
        PrTransition {
            checkout: PathBuf::from("app"),
            pr,
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
                    vec![transition(pr, PrTransitionTo::State(to))],
                    "{remote}: {from:?} -> {to:?}"
                );
                assert!(app.take_pr_transitions().is_empty(), "drained");
            }

            let before = pr_on(remote, 12, CiRunning);
            let mut app = seen_app(remote, PrLookup::Found(before.clone()));
            assert_eq!(
                changes(&mut app, remote, None),
                vec![transition(before, PrTransitionTo::Closed)],
                "{remote}: closed"
            );

            // A failed lookup in between keeps the answer before it, so the
            // change still alerts.
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
                vec![transition(failed, PrTransitionTo::State(CiFailed))],
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
            vec![transition(merged, PrTransitionTo::State(PrState::Merged))],
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
                vec![transition(ready, PrTransitionTo::State(Ready))],
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
                vec![transition(failed, PrTransitionTo::State(CiFailed))],
                "{remote}: CI failed still alerts"
            );

            let mut app = seen_app(remote, PrLookup::Found(computing(remote, 6, InReview)));
            assert!(
                changes(&mut app, remote, Some(pr_on(remote, 6, Ready))).is_empty(),
                "{remote}: an answer that still computes is not the before"
            );
        }
    }

    // ---- change alerts: toasts and row flashes ----

    use super::super::super::toast::Toast;

    /// Mark, text, detail, text role, and TTL of `toast`.
    fn toast_parts(toast: &Toast) -> (ToastMark, String, String, SegRole, u64) {
        (
            toast.mark,
            toast.text.clone(),
            toast.detail.clone(),
            toast.role,
            toast.ttl_ms,
        )
    }

    /// Text and detail of every toast on the stack at `now`, oldest first.
    fn toast_texts(app: &AppState, now: Instant) -> Vec<(String, String)> {
        app.toasts
            .visible(now)
            .map(|(toast, _)| (toast.text.clone(), toast.detail.clone()))
            .collect()
    }

    /// Flash ramp of each painted tree row that flashes, by row id.
    fn flashed_rows(app: &AppState) -> Vec<(String, FlashKind)> {
        app.rows
            .iter()
            .filter_map(|row| Some((row.id.clone(), app.flashes.get(&row.id)?.kind)))
            .collect()
    }

    #[test]
    fn pr_change_toasts_read_glyph_ref_state_and_check_or_branch_on_both_forges() {
        use PrState::*;
        use PrTransitionTo::{Closed, State};
        let at = Instant::now();
        let branch = "feature/ABC-123-retry-ui";
        for (remote, sign) in [(GH_REMOTE, "#"), (GL_REMOTE, "!")] {
            for (to, unicode, ascii, role, label) in [
                (State(CiFailed), "✕", "x", SegRole::Deleted, "CI failed"),
                (State(Ready), "◉", "A", SegRole::Added, "ready to merge"),
                (State(Merged), "✓", "m", SegRole::BranchFeature, "merged"),
                (Closed, "⊘", "c", SegRole::Deleted, "closed"),
            ] {
                let state = match to {
                    State(state) => state,
                    Closed => InReview,
                };
                let pr = PullRequest {
                    head: branch.into(),
                    ..pr_on(remote, 318, state)
                };
                let change = transition(pr, to);
                for (look, glyph) in [(false, unicode), (true, ascii)] {
                    assert_eq!(
                        toast_parts(&change.toast(look, at)),
                        (
                            ToastMark::Glyph(glyph, role),
                            format!("{sign}318 {label}"),
                            branch.to_string(),
                            SegRole::Repo,
                            PR_TOAST_TTL_MS,
                        ),
                        "{remote}: {to:?}, ascii {look}"
                    );
                }
                assert_eq!(change.toast(false, at).at, at);
            }
        }

        // GitHub names the first failed check on CI failed; any other
        // change keeps the branch.
        let named = |state| PullRequest {
            head: branch.into(),
            failed_check: Some("test (ubuntu-latest)".into()),
            ..pr_on(GH_REMOTE, 318, state)
        };
        let failed = transition(named(CiFailed), State(CiFailed));
        assert_eq!(
            (failed.toast(false, at).text, failed.toast(false, at).detail),
            ("#318 CI failed".into(), "test (ubuntu-latest)".into())
        );
        let merged = transition(named(Merged), State(Merged));
        assert_eq!(merged.toast(false, at).detail, branch);

        // A GitLab badge answer names no job (`failed_check` is `None`), so
        // its CI failed toast names the branch.
        let gitlab = PullRequest {
            head: branch.into(),
            ..pr_on(GL_REMOTE, 12, CiFailed)
        };
        assert_eq!(gitlab.failed_check, None);
        let failed = transition(gitlab, State(CiFailed));
        assert_eq!(
            (failed.toast(false, at).text, failed.toast(false, at).detail),
            ("!12 CI failed".into(), branch.into())
        );
        assert_eq!(PR_TOAST_TTL_MS, 8000);
    }

    #[test]
    fn a_batch_of_more_than_three_pr_changes_toasts_the_first_two_and_a_summary() {
        use PrState::*;
        let names = ["a", "b", "c", "d", "e"];
        let mut app = app_with(&names.map(pushed), &[]);
        app.ascii = false;
        let _ = app.due_pr_lookups();
        let t0 = Instant::now();
        for (number, name) in (1..).zip(names) {
            land_pr(&mut app, name, &gh(name), open_on(&gh(name), number), t0);
        }
        assert!(!app.alert_pr_transitions(t0), "the first lookups: none");
        assert!(app.toasts.is_empty());

        // Five changes in one batch: two toasts and a summary of three.
        let into = [CiFailed, Ready, Merged, CiFailed];
        for ((number, name), state) in (1..).zip(names).zip(into) {
            let found = PrLookup::Found(pr_on(&gh(name), number, state));
            land_pr(&mut app, name, &gh(name), found, t0);
        }
        land_pr(&mut app, "e", &gh("e"), PrLookup::NoPr, t0);
        assert!(app.alert_pr_transitions(t0));
        assert_eq!(
            toast_texts(&app, t0),
            [
                ("#1 CI failed".into(), "feature".into()),
                ("#2 ready to merge".into(), "feature".into()),
                ("+3 more PR changes".into(), "gp".into()),
            ]
        );
        let summary = app.toasts.visible(t0).last().unwrap().0.clone();
        assert_eq!(
            toast_parts(&summary),
            (
                ToastMark::None,
                "+3 more PR changes".into(),
                "gp".into(),
                SegRole::Muted,
                PR_TOAST_TTL_MS,
            ),
            "muted, no mark, 8 s"
        );
        assert_eq!(
            flashed_rows(&app),
            [
                ("repo:a".into(), FlashKind::Remove),
                ("repo:b".into(), FlashKind::Add),
                ("repo:c".into(), FlashKind::Add),
                ("repo:d".into(), FlashKind::Remove),
                ("repo:e".into(), FlashKind::Remove),
            ],
            "every change flashes, toasted or summed up"
        );
        assert!(!app.alert_pr_transitions(t0), "drained");

        // Exactly three: three toasts, no summary.
        let mut app = app_with(&names.map(pushed), &[]);
        let _ = app.due_pr_lookups();
        for (number, name) in (1..).zip(&names[..3]) {
            land_pr(&mut app, name, &gh(name), open_on(&gh(name), number), t0);
        }
        for (number, name) in (1..).zip(&names[..3]) {
            let ready = PrLookup::Found(pr_on(&gh(name), number, Ready));
            land_pr(&mut app, name, &gh(name), ready, t0);
        }
        assert!(app.alert_pr_transitions(t0));
        let texts: Vec<String> = toast_texts(&app, t0).into_iter().map(|t| t.0).collect();
        assert_eq!(
            texts,
            [
                "#1 ready to merge",
                "#2 ready to merge",
                "#3 ready to merge"
            ]
        );
    }

    #[test]
    fn a_pr_change_flashes_the_checkout_tree_row_on_the_remove_or_add_ramp() {
        use PrState::*;
        // `app` is a repo row of its own. `api` (primary) and `api-wt`
        // (linked) are checkout rows under the `repo:api` family row, which
        // paints no badge and never flashes.
        let linked_wt = RepoSnapshot {
            sync_status: SyncStatus::UpToDate,
            ..linked("api-wt", "api", "feature")
        };
        let repos = [pushed("app"), pushed("api"), linked_wt];
        for (to, kind) in [
            (Some(CiFailed), FlashKind::Remove),
            (Some(Ready), FlashKind::Add),
            (Some(Merged), FlashKind::Add),
            (None, FlashKind::Remove),
        ] {
            for (checkout, row) in [
                ("app", "repo:app"),
                ("api", "checkout:api"),
                ("api-wt", "checkout:api-wt"),
            ] {
                let mut app = app_with(&repos, &[]);
                let _ = app.due_pr_lookups();
                let remote = gh(checkout);
                land_pr(
                    &mut app,
                    checkout,
                    &remote,
                    open_on(&remote, 9),
                    Instant::now(),
                );
                let lookup = to.map_or(PrLookup::NoPr, |state| {
                    PrLookup::Found(pr_on(&remote, 9, state))
                });
                land_pr(&mut app, checkout, &remote, lookup, Instant::now());
                assert!(app.flashes.is_empty());
                let at = Instant::now() + secs(1);
                assert!(app.alert_pr_transitions(at));
                assert_eq!(
                    flashed_rows(&app),
                    [(row.to_string(), kind)],
                    "{checkout}: {to:?}"
                );
                let stamps: Vec<(String, Instant)> = app
                    .flashes
                    .iter()
                    .map(|(id, stamp)| (id.clone(), stamp.at))
                    .collect();
                assert_eq!(
                    stamps,
                    [(row.to_string(), at)],
                    "{checkout}: only the badge row, stamped at the alert time"
                );
            }
        }
    }

    #[test]
    fn no_pr_alert_on_first_load_or_a_branch_change_and_one_after_a_poll_changes_the_state() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let mut app = app_with(&[pushed("app")], &[]);
            let path = Path::new("app");
            let t0 = Instant::now();
            let _ = app.due_pr_lookups();
            land_pr(&mut app, "app", remote, open_on(remote, 7), t0);
            assert!(!app.alert_pr_transitions(t0), "{remote}: first load");
            assert!(app.toasts.is_empty() && app.flashes.is_empty());

            // A branch change, then back: each first lookup is quiet.
            apply_one_repo_snapshot(&mut app, "app", Some(pushed_on("app", "other")));
            let _ = app.due_pr_lookups();
            let failed = PrLookup::Found(pr_on(remote, 8, PrState::CiFailed));
            assert!(app.apply_pr_lookup(path, "other", Some(remote.into()), failed));
            assert!(!app.alert_pr_transitions(t0), "{remote}: branch change");
            apply_one_repo_snapshot(&mut app, "app", Some(pushed("app")));
            let _ = app.due_pr_lookups();
            land_pr(&mut app, "app", remote, open_on(remote, 7), t0);
            assert!(!app.alert_pr_transitions(t0), "{remote}: back again");

            // A poll that lands the same state is quiet; a new state alerts.
            let t1 = t0 + secs(180);
            assert_eq!(polled(&mut app, t1), ["app"]);
            land_pr(&mut app, "app", remote, open_on(remote, 7), t1);
            assert!(!app.alert_pr_transitions(t1), "{remote}: same state");
            let t2 = t1 + secs(180);
            assert_eq!(polled(&mut app, t2), ["app"]);
            let merged = PrLookup::Found(pr_on(remote, 7, PrState::Merged));
            land_pr(&mut app, "app", remote, merged, t2);
            assert!(app.alert_pr_transitions(t2), "{remote}: merged");
            let sign = if remote == GH_REMOTE { "#" } else { "!" };
            assert_eq!(
                toast_texts(&app, t2),
                [(format!("{sign}7 merged"), "feature".to_string())]
            );
            assert_eq!(
                flashed_rows(&app),
                [("repo:app".to_string(), FlashKind::Add)]
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
        assert_eq!(app.request_pr_details(), Effect::None, "no change");

        set_answer(&mut app, 7, PrState::Ready);
        assert_eq!(
            app.request_pr_details(),
            fetch(7, 2),
            "a changed answer asks again"
        );
        assert_eq!(title(&app), "Add login", "the old detail stays meanwhile");
        assert_eq!(app.request_pr_details(), Effect::None, "asked once");
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
        assert_eq!(app.request_pr_details(), fetch(7, 3));
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
        assert_eq!(app.request_pr_details(), fetch(7, 4));
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
            app.request_pr_details(),
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
        assert_eq!(app.request_pr_details(), fetch(7, 2));
        set_answer(&mut app, 7, PrState::CiRunning);
        assert_eq!(
            app.request_pr_details(),
            Effect::None,
            "one fetch at a time"
        );
        assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
        assert_eq!(
            app.request_pr_details(),
            fetch(7, 3),
            "that detail is older than the badge: ask again"
        );
        assert!(land_request(&mut app, "feature", 7, 3, pr_detail(7)));
        assert_eq!(app.request_pr_details(), Effect::None, "up to date");
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
            app.request_pr_details(),
            Effect::None,
            "waits for the first fetch"
        );
        assert!(land_request(&mut app, "feature", 7, 1, pr_detail(7)));
        assert_eq!(app.request_pr_details(), fetch(7, 2));
        assert!(land_request(&mut app, "feature", 7, 2, pr_detail(7)));
        assert_eq!(app.request_pr_details(), Effect::None);
    }

    // ---- gc (Open CI), Copy URL, Refresh this PR ----

    use super::super::super::pr_status::{pipeline_ci_link, CiLink, CiLinkKind};

    const FAILED_JOB: &str = "https://github.com/acme/app/actions/runs/1/job/2";
    const PIPELINE: &str = "https://gitlab.com/acme/services/api/-/pipelines/99";

    /// A PR on the forge of `remote` whose CI link is a failed check
    /// (GitHub) or the head pipeline (GitLab), with that ref and URL.
    fn pr_with_ci(remote: &str, state: PrState) -> (PullRequest, &'static str, &'static str) {
        if remote.contains("gitlab") {
            let mut pr = pr_on(remote, 12, state);
            pr.ci_link = pipeline_ci_link(Some(PIPELINE), &pr.url);
            (pr, "!12", PIPELINE)
        } else {
            let mut pr = pr_on(remote, 318, state);
            pr.ci_link = CiLink {
                url: FAILED_JOB.into(),
                kind: CiLinkKind::FailedCheck,
            };
            (pr, "#318", FAILED_JOB)
        }
    }

    /// `app` on `feature` with `lookup` landed at `remote`, focused.
    fn app_answered(remote: &str, lookup: PrLookup) -> AppState {
        let mut app = app_with(&[repo("app", "feature"), repo("lib", "main")], &[]);
        let _ = app.due_pr_lookups();
        land_pr(&mut app, "app", remote, lookup, Instant::now());
        focus(&mut app, "repo:app");
        app
    }

    fn gc(app: &mut AppState) -> Effect {
        app.status = StatusMessage::default();
        app.dispatch(Action::OpenPullRequestCi)
    }

    fn ci_open(pr_ref: &str, url: &str) -> Effect {
        Effect::OpenPullRequestCi {
            pr_ref: pr_ref.into(),
            url: url.into(),
        }
    }

    /// Pin the PR popover of `app` (its badge on the focused row).
    fn pin_pr(app: &mut AppState) -> Effect {
        let kind = super::super::super::tree::pr_badge_kind(
            app.pr_badge(Path::new("app")).expect("badge"),
        );
        app.pin_icon(&IconHit {
            y: 1,
            x: 20,
            width: 2,
            kind,
            target: IconTarget::PullRequest(PathBuf::from("app")),
        })
    }

    /// Move the pinned popover focus to the action line titled `title`.
    fn focus_line(app: &mut AppState, title: &str) {
        use crate::tui::popover::{flat_lines, PopoverLine};
        let sections = app.open_popover_sections();
        let index = flat_lines(&sections)
            .iter()
            .position(|line| matches!(line, PopoverLine::Action { title: t, .. } if *t == title))
            .unwrap_or_else(|| panic!("no {title} line"));
        app.popover.as_mut().expect("pinned").focus_line = index;
    }

    #[test]
    fn gc_on_a_tree_row_opens_the_known_ci_link_on_both_forges() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            for state in [PrState::CiFailed, PrState::Ready, PrState::Merged] {
                let (pr, pr_ref, url) = pr_with_ci(remote, state);
                let mut app = app_answered(remote, PrLookup::Found(pr));
                assert_eq!(gc(&mut app), ci_open(pr_ref, url), "{remote} {state:?}");
                assert!(app.status.is_empty(), "the worker says how it went");
            }
        }
    }

    #[test]
    fn gc_says_no_pr_no_ci_or_looks_the_pr_up_first() {
        // No branch to look up.
        let mut app = app_answered(GH_REMOTE, PrLookup::NoPr);
        focus(&mut app, "repo:lib");
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        assert_eq!(gc(&mut app), Effect::None);
        assert_eq!(app.status, NO_PR_FOR_ROW);
        assert_eq!(app.status.kind(), StatusKind::Warn);

        for remote in [GH_REMOTE, GL_REMOTE] {
            // The forge answered: no PR.
            let mut app = app_answered(remote, PrLookup::NoPr);
            assert_eq!(gc(&mut app), Effect::None);
            assert_eq!(app.status, "no PR for feature");
            assert_eq!(app.status.kind(), StatusKind::Warn);

            // A PR with no CI web page.
            let (mut pr, pr_ref, _) = pr_with_ci(remote, PrState::InReview);
            pr.ci_link.url = String::new();
            let mut app = app_answered(remote, PrLookup::Found(pr));
            assert_eq!(gc(&mut app), Effect::None);
            assert_eq!(app.status, format!("no CI for {pr_ref}"));
            assert_eq!(app.status.kind(), StatusKind::Warn);

            // A failed first lookup knows nothing: look the PR up first.
            let mut app = app_answered(remote, PrLookup::Failed(None));
            assert_eq!(
                gc(&mut app),
                Effect::OpenPullRequest {
                    repo: PathBuf::from("app"),
                    branch: "feature".into(),
                    page: PrPage::Ci,
                }
            );
        }

        // The first lookup is still out.
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let _ = app.due_pr_lookups();
        focus(&mut app, "repo:app");
        assert!(matches!(
            gc(&mut app),
            Effect::OpenPullRequest {
                page: PrPage::Ci,
                ..
            }
        ));
    }

    #[test]
    fn gc_in_the_graph_opens_a_worktree_rows_ci_on_both_forges() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let wt = "app/.worktrees/feat";
            let mut app = app_with(&[repo("app", "main"), linked(wt, "app", "feature")], &[]);
            let _ = app.due_pr_lookups();
            let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiRunning);
            land_pr(&mut app, wt, remote, PrLookup::Found(pr), Instant::now());
            focus(&mut app, "repo:app");
            app.graph = Some(GraphModel {
                commits: vec![Commit {
                    id: "aaa".into(),
                    subject: "seed".into(),
                    ..Commit::default()
                }],
                worktrees: vec![Worktree {
                    path: wt.into(),
                    head_id: None,
                    branch: Some("feature".into()),
                    hidden: false,
                    is_current: false,
                }],
                window: 1,
                ..GraphModel::default()
            });
            app.focus = FocusPane::Right;
            assert!(app.graph_pane_focused());
            let rows = app.graph.as_ref().expect("graph").visible_rows();
            app.graph_cursor = rows
                .iter()
                .position(|row| matches!(row, GraphRow::Worktree(_)))
                .expect("worktree row");
            assert_eq!(gc(&mut app), ci_open(pr_ref, url), "{remote}");

            app.graph_cursor = rows
                .iter()
                .position(|row| matches!(row, GraphRow::Commit { .. }))
                .expect("commit row");
            assert_eq!(gc(&mut app), Effect::None, "gc is no longer graph c");
            assert_eq!(app.status, NO_PR_FOR_ROW);
        }
    }

    #[test]
    fn the_popover_open_ci_line_runs_gc_on_both_forges() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiFailed);
            let title = pr.ci_link.kind.label();
            let mut app = app_answered(remote, PrLookup::Found(pr));
            let _ = pin_pr(&mut app);
            focus_line(&mut app, title);
            assert_eq!(app.popover_run_action(), Some(Action::OpenPullRequestCi));
            assert_eq!(app.dispatch(Action::PopoverRun), ci_open(pr_ref, url));
            assert!(app.popover.is_none(), "Enter closes, then opens CI");
        }
    }

    /// The graph header chip pins the PR popover of the graph's checkout
    /// while a commit row has the focus: its PR lines act on that PR, not
    /// on the focused row (which has none).
    #[test]
    fn pr_popover_lines_act_on_their_own_pr_not_the_focused_graph_row() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiFailed);
            let (ci_title, pr_url) = (pr.ci_link.kind.label(), pr.url.clone());
            let mut app = app_answered(remote, PrLookup::Found(pr));
            app.graph = Some(GraphModel {
                commits: vec![Commit {
                    id: "aaa".into(),
                    subject: "seed".into(),
                    ..Commit::default()
                }],
                window: 1,
                ..GraphModel::default()
            });
            app.focus = FocusPane::Right;
            let rows = app.graph.as_ref().expect("graph").visible_rows();
            app.graph_cursor = rows
                .iter()
                .position(|row| matches!(row, GraphRow::Commit { .. }))
                .expect("commit row");
            assert_eq!(gc(&mut app), Effect::None, "the commit row has no PR");
            for (title, want) in [
                (ci_title, ci_open(pr_ref, url)),
                (
                    "Open PR",
                    Effect::OpenPullRequest {
                        repo: PathBuf::from("app"),
                        branch: "feature".into(),
                        page: PrPage::Pr,
                    },
                ),
                (
                    "Copy URL",
                    Effect::CopyClipboard {
                        text: pr_url.clone(),
                        announce: true,
                    },
                ),
                (
                    "Refresh this PR",
                    Effect::LookupPullRequests {
                        targets: vec![(PathBuf::from("app"), "feature".into())],
                    },
                ),
            ] {
                let _ = pin_pr(&mut app);
                focus_line(&mut app, title);
                assert_eq!(app.dispatch(Action::PopoverRun), want, "{remote} {title}");
            }
        }
    }

    #[test]
    fn a_ci_open_reports_each_outcome_on_the_status_line() {
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let _ = app.due_pr_lookups();
        for (pr_ref, opened, text, kind) in [
            ("#318", true, "opened CI for #318", StatusKind::Ok),
            ("!12", true, "opened CI for !12", StatusKind::Ok),
            ("#318", false, "could not open CI", StatusKind::Error),
        ] {
            app.apply_pr_ci_open(pr_ref, opened);
            assert_eq!(app.status, text);
            assert_eq!(app.status.kind(), kind, "{text}");
        }

        // `gc` that looked the PR up first.
        let path = Path::new("app");
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, pr_ref, _) = pr_with_ci(remote, PrState::CiFailed);
            let mut no_ci = pr.clone();
            no_ci.ci_link.url = String::new();
            for (lookup, opened, text, kind) in [
                (
                    PrLookup::Found(pr.clone()),
                    true,
                    format!("opened CI for {pr_ref}"),
                    StatusKind::Ok,
                ),
                (
                    PrLookup::Found(pr.clone()),
                    false,
                    "could not open CI".into(),
                    StatusKind::Error,
                ),
                (
                    PrLookup::Found(no_ci),
                    false,
                    format!("no CI for {pr_ref}"),
                    StatusKind::Warn,
                ),
                (
                    PrLookup::NoPr,
                    false,
                    "no PR for feature".into(),
                    StatusKind::Warn,
                ),
                (
                    PrLookup::Failed(None),
                    false,
                    "could not look up PR for feature".into(),
                    StatusKind::Error,
                ),
                (
                    PrLookup::Failed(Some("gh not found".into())),
                    false,
                    "could not look up PR for feature: gh not found".into(),
                    StatusKind::Error,
                ),
            ] {
                app.apply_pr_open(
                    path,
                    "feature",
                    Some(remote.into()),
                    lookup,
                    PrPage::Ci,
                    opened,
                );
                assert_eq!(app.status, text.as_str(), "{remote}");
                assert_eq!(app.status.kind(), kind, "{remote}: {text}");
            }
        }
    }

    #[test]
    fn copy_url_copies_the_pr_url_from_the_palette_and_the_popover_line() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, _, _) = pr_with_ci(remote, PrState::InReview);
            let copy = Effect::CopyClipboard {
                text: pr.url.clone(),
                announce: true,
            };
            let mut app = app_answered(remote, PrLookup::Found(pr));
            assert_eq!(app.dispatch(Action::CopyPullRequestUrl), copy, "{remote}");

            // `y` on the Copy URL line copies the URL, not the title, and
            // the popover stays open.
            let _ = pin_pr(&mut app);
            focus_line(&mut app, "Copy URL");
            assert_eq!(app.dispatch(Action::PopoverCopyLine), copy, "{remote}");
            assert!(app.popover_pinned());
            // Enter runs it like the palette row.
            assert_eq!(app.dispatch(Action::PopoverRun), copy, "{remote}");
            assert!(app.popover.is_none());
        }
        let mut app = app_answered(GH_REMOTE, PrLookup::NoPr);
        assert_eq!(app.dispatch(Action::CopyPullRequestUrl), Effect::None);
        assert_eq!(app.status, "no PR for feature");
        focus(&mut app, "repo:lib");
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        assert_eq!(app.dispatch(Action::CopyPullRequestUrl), Effect::None);
        assert_eq!(app.status, NO_PR_FOR_ROW);
    }

    #[test]
    fn r_in_a_pinned_pr_popover_refreshes_that_pr_and_keeps_it_open() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, _, _) = pr_with_ci(remote, PrState::CiRunning);
            let mut app = app_answered(remote, PrLookup::Found(pr.clone()));
            let Effect::LookupPullRequestDetail {
                repo,
                branch,
                remote: resolved,
                number,
                request,
            } = pin_pr(&mut app)
            else {
                panic!("the pin asks for the detail");
            };
            assert!(app.apply_pr_detail(
                &repo,
                &branch,
                &resolved,
                number,
                request,
                pr_detail(number)
            ));
            let refresh = Effect::LookupPullRequests {
                targets: vec![(PathBuf::from("app"), "feature".into())],
            };
            assert_eq!(
                app.dispatch(Action::RefreshPullRequest),
                refresh,
                "{remote}"
            );
            assert!(app.popover_pinned(), "r keeps the popover");
            assert_eq!(
                app.pr_badge(Path::new("app")),
                Some(PrState::CiRunning),
                "the badge stays while it looks again"
            );
            assert!(
                matches!(detail_of(&app), Some(PrDetailState::Ready(_))),
                "the detail stays on screen while it looks again"
            );
            assert_eq!(
                app.dispatch(Action::RefreshPullRequest),
                Effect::None,
                "already looking"
            );
            assert_eq!(app.status, PR_LOOKUP_RUNNING);
            assert_eq!(app.status.kind(), StatusKind::Warn);

            // The answer lands: the pinned popover asks for the detail
            // again.
            land_pr(&mut app, "app", remote, PrLookup::Found(pr), Instant::now());
            assert_eq!(
                app.request_pr_details(),
                Effect::LookupPullRequestDetail {
                    repo: PathBuf::from("app"),
                    branch: "feature".into(),
                    remote: remote.into(),
                    number,
                    request: request + 1,
                },
                "{remote}: the detail is fetched again"
            );

            // The palette row (no popover) refreshes the focused row.
            let (pr, _, _) = pr_with_ci(remote, PrState::Ready);
            let mut app = app_answered(remote, PrLookup::Found(pr));
            assert_eq!(
                app.dispatch(Action::RefreshPullRequest),
                refresh,
                "{remote}"
            );
        }

        // A pinned popover with no PR section ignores `r`.
        let mut app = app_answered(GH_REMOTE, PrLookup::NoPr);
        let _ = app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned());
        assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
        assert!(app.status.is_empty());
        assert!(app.popover_pinned());
    }

    /// Refresh this PR says why it looks nothing up: a lookup of that
    /// checkout is already out, or the checkout has no forge remote or no
    /// branch.
    #[test]
    fn refresh_this_pr_says_why_it_cannot_run() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, _, _) = pr_with_ci(remote, PrState::InReview);
            let mut app = app_answered(remote, PrLookup::Found(pr));
            assert!(matches!(
                app.dispatch(Action::RefreshPullRequest),
                Effect::LookupPullRequests { .. }
            ));
            assert!(app.status.is_empty());
            assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
            assert_eq!(app.status, PR_LOOKUP_RUNNING, "{remote}");
            assert_eq!(app.status.kind(), StatusKind::Warn);
        }

        // The first lookup is still out.
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let _ = app.due_pr_lookups();
        focus(&mut app, "repo:app");
        assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
        assert_eq!(app.status, PR_LOOKUP_RUNNING);

        // No GitHub or GitLab remote.
        let mut app = app_with(&[repo("app", "feature")], &[]);
        let _ = app.due_pr_lookups();
        assert!(app.apply_pr_lookup(Path::new("app"), "feature", None, PrLookup::NoPr));
        focus(&mut app, "repo:app");
        assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
        assert_eq!(app.status, NO_PR_FOR_ROW);
        assert_eq!(app.status.kind(), StatusKind::Warn);

        // No branch: a file row.
        app.status = StatusMessage::default();
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        assert_eq!(app.dispatch(Action::RefreshPullRequest), Effect::None);
        assert_eq!(app.status, NO_PR_FOR_ROW);
    }

    /// Dispatch the action the key `code` maps to in the current mode.
    fn press(app: &mut AppState, code: crossterm::event::KeyCode) -> Effect {
        use crossterm::event::{Event, KeyEvent, KeyModifiers};
        let event = Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
        let action = crate::tui::keys::event_to_action(&event, app.input_mode(), false, false);
        app.dispatch(action)
    }

    /// `gx` and `gc` inside a pinned PR popover act on its PR section, even
    /// while the focused graph row (a commit) has no PR; the popover closes
    /// as Enter on the line closes it.
    #[test]
    fn gx_and_gc_in_a_pinned_pr_popover_act_on_its_pr_on_both_forges() {
        use crate::tui::keys::InputMode;
        use crossterm::event::KeyCode;
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, pr_ref, url) = pr_with_ci(remote, PrState::CiFailed);
            let mut app = app_answered(remote, PrLookup::Found(pr));
            app.graph = Some(GraphModel {
                commits: vec![Commit {
                    id: "aaa".into(),
                    subject: "seed".into(),
                    ..Commit::default()
                }],
                window: 1,
                ..GraphModel::default()
            });
            app.focus = FocusPane::Right;
            let rows = app.graph.as_ref().expect("graph").visible_rows();
            app.graph_cursor = rows
                .iter()
                .position(|row| matches!(row, GraphRow::Commit { .. }))
                .expect("commit row");
            for (second, want) in [
                (KeyCode::Char('c'), ci_open(pr_ref, url)),
                (
                    KeyCode::Char('x'),
                    Effect::OpenPullRequest {
                        repo: PathBuf::from("app"),
                        branch: "feature".into(),
                        page: PrPage::Pr,
                    },
                ),
            ] {
                let _ = pin_pr(&mut app);
                assert_eq!(app.input_mode(), InputMode::Popover);
                assert_eq!(press(&mut app, KeyCode::Char('g')), Effect::None);
                assert_eq!(app.input_mode(), InputMode::PopoverGPending);
                assert_eq!(press(&mut app, second), want, "{remote} g{second:?}");
                assert!(app.popover.is_none(), "the line's key closes it");
            }
        }
    }

    /// `gc` in a pinned popover with no PR section says so and keeps the
    /// popover; `g` then Esc only ends the chord.
    #[test]
    fn g_chords_in_a_popover_without_a_pr_or_ended_by_esc() {
        use crate::tui::keys::InputMode;
        use crossterm::event::KeyCode;
        let mut app = app_answered(GH_REMOTE, open_on(GH_REMOTE, 7));
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        let _ = app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned(), "a file row popover");
        for second in [KeyCode::Char('c'), KeyCode::Char('x')] {
            app.status = StatusMessage::default();
            let _ = press(&mut app, KeyCode::Char('g'));
            assert_eq!(press(&mut app, second), Effect::None);
            assert_eq!(app.status, NO_PR_FOR_ROW);
            assert!(app.popover_pinned(), "the popover stays");
            assert_eq!(app.input_mode(), InputMode::Popover);
        }

        // `g` then Esc ends the chord and keeps the popover; a later `c`
        // is a plain popover key again (it does nothing).
        let mut app = app_answered(GH_REMOTE, open_on(GH_REMOTE, 7));
        let _ = pin_pr(&mut app);
        let _ = press(&mut app, KeyCode::Char('g'));
        assert_eq!(app.input_mode(), InputMode::PopoverGPending);
        assert_eq!(press(&mut app, KeyCode::Esc), Effect::None);
        assert_eq!(app.input_mode(), InputMode::Popover);
        assert!(app.popover_pinned(), "Esc in the chord does not close");
        assert_eq!(press(&mut app, KeyCode::Char('c')), Effect::None);
        assert!(app.popover_pinned());
        // `g` then `j` ends the chord without moving the focus.
        let focus = app.popover.as_ref().expect("pinned").focus_line;
        let _ = press(&mut app, KeyCode::Char('g'));
        let _ = press(&mut app, KeyCode::Char('j'));
        assert_eq!(app.input_mode(), InputMode::Popover);
        assert_eq!(app.popover.as_ref().expect("pinned").focus_line, focus);
    }

    /// A poll that drops a line above the focused one (the notes) keeps
    /// the pinned focus on the same action line.
    #[test]
    fn a_landed_badge_lookup_keeps_the_pinned_focus_on_its_line() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (mut pr, _, _) = pr_with_ci(remote, PrState::InReview);
            pr.notes.conflicts = true;
            let mut app = app_answered(remote, PrLookup::Found(pr.clone()));
            let _ = pin_pr(&mut app);
            focus_line(&mut app, "Copy URL");
            let notes = |app: &AppState| {
                app.open_popover_sections()
                    .iter()
                    .flat_map(|section| section.lines.iter())
                    .any(|line| line.copy_text() == "has conflicts")
            };
            assert!(notes(&app));
            pr.notes.conflicts = false;
            land_pr(&mut app, "app", remote, PrLookup::Found(pr), Instant::now());
            assert!(!notes(&app), "the poll dropped the notes line");
            assert_eq!(
                app.popover_run_action(),
                Some(Action::CopyPullRequestUrl),
                "{remote}: the focus stays on Copy URL"
            );
        }
    }

    /// A pinned PR popover does not pause polling: the event loop runs the
    /// poll tick in `Popover` and `PopoverGPending` (both pass
    /// `overlay_blocks_background_ticks`), the cycle takes the popover's
    /// checkout, and the landed answer shows in the PR section while the
    /// popover stays pinned.
    #[test]
    fn a_pinned_pr_popover_keeps_polling_and_shows_the_new_state() {
        use super::super::super::event_pump::overlay_blocks_background_ticks;
        use super::super::super::keys::InputMode;
        assert!(!overlay_blocks_background_ticks(InputMode::PopoverGPending));
        for remote in [GH_REMOTE, GL_REMOTE] {
            let (pr, _, _) = pr_with_ci(remote, PrState::CiRunning);
            let mut app = app_answered(remote, PrLookup::Found(pr.clone()));
            let _ = pin_pr(&mut app);
            assert!(app.popover_pinned(), "{remote}");
            assert_eq!(app.input_mode(), InputMode::Popover, "{remote}");
            assert!(!overlay_blocks_background_ticks(app.input_mode()));
            let heading = |app: &AppState| {
                app.open_popover_sections()
                    .iter()
                    .find(|section| section.target == IconTarget::PullRequest(PathBuf::from("app")))
                    .map(|section| section.heading().to_string())
            };
            assert_eq!(heading(&app).as_deref(), Some("PR · CI running"));

            let due = app.due_pr_polls(Instant::now());
            assert!(
                due.contains(&(PathBuf::from("app"), "feature".to_string())),
                "{remote}: the poll takes the pinned checkout: {due:?}"
            );
            let ready = PullRequest {
                state: PrState::Ready,
                ..pr
            };
            land_pr(
                &mut app,
                "app",
                remote,
                PrLookup::Found(ready),
                Instant::now(),
            );
            assert!(app.popover_pinned(), "{remote}: still pinned");
            assert_eq!(
                heading(&app).as_deref(),
                Some("PR · ready to merge"),
                "{remote}"
            );
        }
    }

    /// Two check names share the blank label: the focus on the second one
    /// stays there when a new detail adds a line above it.
    #[test]
    fn the_focus_on_a_check_name_survives_a_detail_landing() {
        use super::super::super::pr_status::{Check, CheckBucket};
        use crate::tui::popover::flat_lines;
        let with_checks = |author: Option<&str>| {
            let PrDetailLookup::Found(mut detail) = pr_detail(7) else {
                unreachable!("a found detail");
            };
            detail.author = author.map(str::to_string);
            detail.check_names = ["test", "build"]
                .iter()
                .zip([CheckBucket::Fail, CheckBucket::Pending])
                .map(|(name, bucket)| Check {
                    bucket,
                    name: name.to_string(),
                    url: None,
                })
                .collect();
            PrDetailLookup::Found(detail)
        };
        let focused_text = |app: &AppState| {
            let sections = app.open_popover_sections();
            let index = app.popover.as_ref().expect("pinned").focus_line;
            flat_lines(&sections)[index].copy_text()
        };
        let mut app = app_with_pr(7);
        let _ = click_badge(&mut app);
        assert!(land(&mut app, "feature", 7, with_checks(None)));
        let sections = app.open_popover_sections();
        let build = flat_lines(&sections)
            .iter()
            .position(|line| line.copy_text() == "build")
            .expect("build line");
        app.popover.as_mut().expect("pinned").focus_line = build;
        assert_eq!(focused_text(&app), "build");

        // The answer changes, so the detail is fetched again; the new one
        // adds `author` above the checks.
        set_answer(&mut app, 7, PrState::CiRunning);
        assert_eq!(app.request_pr_details(), fetch(7, 2));
        assert!(land(&mut app, "feature", 7, with_checks(Some("octo-user"))));
        assert_eq!(focused_text(&app), "build", "not the first check line");
    }

    #[test]
    fn palette_refresh_pull_requests_looks_every_pr_up_again() {
        let mut app = app_with(&[pushed("app"), pushed("lib")], &[]);
        let _ = app.due_pr_lookups();
        let now = Instant::now();
        land_pr(&mut app, "app", &gh("app"), open_on(GH_REMOTE, 1), now);
        land_pr(&mut app, "lib", &gl("lib"), open_on(GL_REMOTE, 2), now);
        let Effect::LookupPullRequests { mut targets } = app.dispatch(Action::RefreshPullRequests)
        else {
            panic!("a lookup of every PR");
        };
        targets.sort();
        assert_eq!(
            targets,
            vec![
                (PathBuf::from("app"), "feature".to_string()),
                (PathBuf::from("lib"), "feature".to_string()),
            ]
        );
    }
}

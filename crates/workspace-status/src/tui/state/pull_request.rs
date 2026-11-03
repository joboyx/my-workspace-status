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
//! the last answer. A checkout whose last lookup failed is polled until a
//! lookup succeeds.
//!
//! A checkout on its repository's known default branch has no PR status:
//! it is never looked up or polled, and a switch onto that branch drops its
//! record like any other branch change. An unknown default branch changes
//! nothing.
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
use super::super::icons::IconKind;
use super::super::outage::OutageSource;
use super::super::popover::IconTarget;
use super::super::pr_poll::{
    pr_answer_is_stale, pr_poll_backoff_ms, pr_poll_interval_ms, PrPollMode,
};
use super::super::pr_status::PrState;
use super::super::pull_request::{
    ci_opened_status, commit_url, default_branch_status, is_web_url, lookup_failed_status,
    no_ci_status, no_pr_status, parse_remote, Forge, PrDetailLookup, PrLookup, PrPage, PullRequest,
    PullRequestDetail, CI_OPEN_FAILED, NO_PR_FOR_ROW, OPEN_FAILED, PR_LOOKUP_RUNNING,
};
use super::super::status::StatusMessage;
use super::super::toast::{Toast, ToastMark, PR_TOAST_TTL_MS, TOAST_MAX};
use super::super::tree::{
    pr_badge_mark, pr_badge_repo, NodeKind, PrBadge, PrMark, PrTotals, SegRole,
    PR_REF_MIN_TREE_COLS,
};
use super::super::watch::{graph_row_identity, FlashKind};
use super::prs_tab::prs_row_flash_id;
use super::AppState;

/// Error line reason when a failed PR lookup printed no reason.
const PR_LOOKUP_FAILED: &str = "lookup failed";

/// True when snapshot row `row` is on its repository's default branch, as
/// far as it is known ([`WorkspaceRepoSnapshot::default_branch`]). Such a
/// checkout receives PRs and carries none. An unknown default branch is
/// never matched, so that checkout is looked up as any other.
fn on_default_branch(row: &WorkspaceRepoSnapshot) -> bool {
    row.default_branch.as_deref() == Some(row.branch.as_str())
}

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
    /// Forge CLI of [`Self::remote`] (`gh` or `glab`), parsed with
    /// [`Self::host`]; a failed lookup names it in its outage.
    tool: Option<&'static str>,
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
            tool: None,
            pr_state: None,
            in_flight,
            look_again: false,
            poll_cycle: None,
            checked_at: None,
            failure: None,
        }
    }

    /// Set the resolved remote and its forge CLI and host.
    fn set_remote(&mut self, remote: Option<String>, forge: Option<ForgeHost>) {
        self.remote = remote;
        (self.tool, self.host) = forge.unzip();
    }

    /// Forge CLI and host of [`Self::remote`], as [`forge_host`] parsed
    /// them.
    fn forge(&self) -> Option<ForgeHost> {
        self.tool.zip(self.host.clone())
    }

    /// True when a poll cycle may look this record up, given the snapshot
    /// row's upstream: a GitHub or GitLab remote, and a last lookup that
    /// failed, an open PR, or no PR known yet on a branch that was pushed.
    /// A merged PR is not polled once its lookup works. A failed lookup is
    /// polled until it succeeds, whatever the upstream or the PR state, so
    /// the outage of its forge host can end ([`AppState::settle_pr_outages`]).
    fn pollable(&self, has_upstream: bool) -> bool {
        self.host.is_some()
            && (self.failure.is_some()
                || match self.pr_state {
                    Some(PrState::Merged) => false,
                    Some(_) => true,
                    None => has_upstream,
                })
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

/// The forge CLI (`gh` or `glab`) and the forge host (`github.com`) of a
/// remote.
type ForgeHost = (&'static str, String);

/// Forge CLI and host of `remote`, or `None` when it is not on GitHub or
/// GitLab. The only place this module parses a remote for polling; test
/// builds count the calls.
fn forge_host(remote: &str) -> Option<ForgeHost> {
    #[cfg(test)]
    FORGE_HOST_PARSES.with(|count| count.set(count.get() + 1));
    parse_remote(remote).map(|repo| {
        let tool = match repo.forge {
            Forge::GitHub => "gh",
            Forge::GitLab => "glab",
        };
        (tool, repo.host)
    })
}

/// Tool of the outage of a failed lookup whose remote has no GitHub or
/// GitLab host: its toast reads `PR (unknown): <reason>`.
const UNKNOWN_PR_TOOL: &str = "PR";

/// Host of the outage of a failed lookup whose remote has no GitHub or
/// GitLab host ([`UNKNOWN_PR_TOOL`]).
const UNKNOWN_PR_HOST: &str = "unknown";

/// The outage toast text of a failed lookup through `tool` on `host`:
/// `<tool> (<host>): <reason>`, [`PR_LOOKUP_FAILED`] when the CLI printed
/// no reason. A reason is the CLI's first stderr line, which usually starts
/// with the tool name again (`glab: 401 Unauthorized`); that one leading
/// `<tool>: ` is cut so the toast spends its width on the reason.
fn pr_outage_toast_text(tool: &str, host: &str, reason: Option<&str>) -> String {
    let reason = reason.unwrap_or(PR_LOOKUP_FAILED);
    let reason = reason
        .strip_prefix(tool)
        .and_then(|rest| rest.strip_prefix(": "))
        .filter(|rest| !rest.is_empty())
        .unwrap_or(reason);
    format!("{tool} ({host}): {reason}")
}

/// What a landed lookup tells the PR lookup outages ([`PrCache::land`]).
#[derive(Clone, Debug, PartialEq, Eq)]
enum LandedLookup {
    /// No record waited for it: nothing changed.
    Dropped,
    /// The forge answered. Its record has no failure any more, so a host
    /// with no other failing record recovers.
    Answered,
    /// The lookup failed: the outage of `source` starts or goes on, with
    /// toast text `text` (`glab (gitlab.com): invalid_token`). `moved`
    /// is true when the record's last failure was on another source (its
    /// remote moved to another host), which may end that outage.
    Failed {
        source: OutageSource,
        text: String,
        moved: bool,
    },
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
    /// The outage this failure keeps going: its forge CLI and host. The
    /// outage ends once no record fails on it.
    source: OutageSource,
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
            outage: None,
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
        outage: None,
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
    /// Checkout path to the remote of a checkout on its known default
    /// branch. Such a checkout has no PR record, so this keeps the remote
    /// that commit links need ([`AppState::commit_link`]); no PR surface
    /// reads it.
    default_remotes: HashMap<PathBuf, CheckoutRemote>,
}

/// The remote of a checkout on its known default branch, resolved with git
/// only (no forge CLI) for commit links.
#[derive(Clone, Debug, PartialEq, Eq)]
struct CheckoutRemote {
    /// The default branch the checkout was on when it was resolved.
    branch: String,
    /// The remote URL; `None` until the worker answers, and when the
    /// checkout has no remote.
    remote: Option<String>,
    /// `r` or a full reload asked to resolve it again; the old remote
    /// stays until the answer lands.
    look_again: bool,
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
    /// poll also backs the host off. The result says what the outage of
    /// the forge host learns ([`AppState::note_pr_landing`]).
    fn land(
        &mut self,
        checkout: &Path,
        branch: &str,
        remote: Option<String>,
        lookup: PrLookup,
        poll_cycle: Option<Instant>,
        now: Instant,
    ) -> LandedLookup {
        let Some(record) = self.checkouts.get_mut(checkout) else {
            return LandedLookup::Dropped;
        };
        // Parse the remote only when it changed; a poll lands the same one.
        let forge = if record.remote == remote {
            record.forge()
        } else {
            remote.as_deref().and_then(forge_host)
        };
        let host = forge.as_ref().map(|(_, host)| host.clone());
        let pr = match lookup {
            PrLookup::Failed(reason) => {
                // One outage per forge CLI and host. A lookup fails only
                // after its remote parsed, so the forge is known in
                // practice; without it every such failure shares one key.
                let (tool, name) = forge
                    .clone()
                    .unwrap_or_else(|| (UNKNOWN_PR_TOOL, UNKNOWN_PR_HOST.to_string()));
                let text = pr_outage_toast_text(tool, &name, reason.as_deref());
                let source = OutageSource::PrLookup { tool, host: name };
                let failure = PrLookupFailure {
                    reason,
                    source: source.clone(),
                };
                let moved = record
                    .failure
                    .replace(failure)
                    .is_some_and(|before| before.source != source);
                let known = record.remote.clone().is_some_and(|remote| {
                    matches!(
                        self.answers.get(&(remote, branch.to_string())),
                        Some(BranchPr::Ready(_))
                    )
                });
                if !known {
                    record.set_remote(remote.clone(), forge);
                    record.pr_state = None;
                    if let Some(remote) = remote {
                        self.set_answer((remote, branch.to_string()), BranchPr::Failed);
                    }
                }
                if let (Some(host), Some(cycle)) = (host, poll_cycle) {
                    self.hosts.entry(host).or_default().fail(cycle);
                }
                self.rehot();
                return LandedLookup::Failed {
                    source,
                    text,
                    moved,
                };
            }
            PrLookup::Found(pr) => Some(pr),
            PrLookup::NoPr => None,
        };
        record.set_remote(remote.clone(), forge);
        record.pr_state = pr.as_ref().filter(|_| remote.is_some()).map(|pr| pr.state);
        record.failure = None;
        record.checked_at = Some(now);
        if let Some(host) = host {
            self.hosts.entry(host).or_default().succeed(now);
        }
        self.rehot();
        let Some(remote) = remote else {
            return LandedLookup::Answered;
        };
        self.note_seen(checkout, &remote, branch, pr.as_ref());
        if self.transitions_since.is_none() && !self.transitions.is_empty() {
            self.transitions_since = Some(now);
        }
        self.set_answer((remote, branch.to_string()), BranchPr::Ready(pr));
        LandedLookup::Answered
    }

    /// True when a record's last lookup failed on `source`.
    fn fails_on(&self, source: &OutageSource) -> bool {
        self.checkouts
            .values()
            .any(|record| record.failure.as_ref().is_some_and(|f| &f.source == source))
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

    /// [`Self::run_pr_action`] on `branch` of checkout `repo`. A checkout on
    /// its repository's known default branch has no PR: every action says
    /// `<branch> is the default branch: no PR` and asks the forge nothing.
    fn run_pr_action_on(&mut self, action: PrAction, repo: PathBuf, branch: String) -> Effect {
        if self.is_on_default_branch(&repo, &branch) {
            self.status = StatusMessage::info(default_branch_status(&branch));
            return Effect::None;
        }
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
        if let Some(effect) = self.popover_click(col, row, Self::click_at) {
            return effect;
        }
        let selected = self.click_at(col, row);
        let open = match self.pr_target_for_repo(&repo) {
            Some((repo, branch)) => {
                // A second Ctrl+click on the badge opens again; it is not
                // a double-click that drills into the row.
                self.last_click = None;
                // The same guard as `gx`: a badge painted before the
                // checkout's default branch became known opens nothing.
                self.run_pr_action_on(PrAction::Open, repo, branch)
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

    /// Web page of commit `sha` (full id) in checkout `checkout`, or `None`.
    ///
    /// The forge comes from the remote URL the PR badge lookup already
    /// resolved for the checkout ([`PrCache::remote_for`]), or, for a
    /// checkout on its known default branch (never looked up), the remote
    /// resolved for it alone ([`Self::due_remote_lookups`]); no git runs
    /// here. No resolved remote, or a remote that is not GitHub or GitLab,
    /// gives no link. An id that is not a full SHA-1 or SHA-256 hex id (an
    /// abbreviated one) gives no link either.
    pub(crate) fn commit_link(&self, checkout: &Path, sha: &str) -> Option<String> {
        let full = matches!(sha.len(), 40 | 64) && sha.bytes().all(|b| b.is_ascii_hexdigit());
        if !full {
            return None;
        }
        let remote = self.pr_cache.remote_for(checkout).or_else(|| {
            self.pr_cache
                .default_remotes
                .get(checkout)?
                .remote
                .as_deref()
        })?;
        Some(commit_url(&parse_remote(remote)?, sha))
    }

    /// PR state for the badge of checkout `repo`.
    ///
    /// `Some` only when the cached answer is for the branch the checkout has
    /// now and names a PR. A lookup in flight, a failed lookup, no PR, or an
    /// answer for another branch give `None`. Paint reads [`Self::pr_mark`].
    #[cfg(test)]
    pub(crate) fn pr_badge(&self, repo: &Path) -> Option<PrState> {
        self.pull_request_for(repo).map(|pr| pr.state)
    }

    /// The PR mark of checkout `repo`: the state of the PR the badge shows
    /// ([`Self::pull_request_for`]), or
    /// [`PrMark::LookupFailed`] when the last lookup of the branch it is on
    /// now failed and no earlier answer is known. `None` for no PR, a
    /// lookup in flight with nothing landed, or another branch. A failure
    /// after a good answer keeps that answer's state (the badge stays).
    pub(crate) fn pr_mark(&self, repo: &Path) -> Option<PrMark> {
        let checkout = self.current_pr_record(repo)?;
        let key = (checkout.remote.clone()?, checkout.branch.clone());
        match self.pr_cache.answers.get(&key)? {
            BranchPr::Ready(Some(pr)) => Some(PrMark::State(pr.state)),
            BranchPr::Ready(None) => None,
            BranchPr::Failed => Some(PrMark::LookupFailed),
        }
    }

    /// Checkouts whose PR badge needs a lookup now, marked in flight.
    ///
    /// Syncs the cache with the snapshot. A checkout with a new or changed
    /// branch is returned, and one that `r` or a full reload asked to look
    /// up again (its answer stays, so the badge stays). A checkout already
    /// looked up for its current branch is left alone. A checkout that
    /// vanished, has a detached HEAD, or is a hidden repo is dropped.
    ///
    /// A dropped or replaced record (vanished checkout, branch change)
    /// takes its failure with it, so a PR lookup outage may end here
    /// ([`Self::settle_pr_outages`]). `r` and a full reload keep the
    /// records, so they never end one.
    pub(crate) fn due_pr_lookups(&mut self) -> Vec<(PathBuf, String)> {
        let wanted: Vec<(PathBuf, String)> = self
            .pr_rows()
            .map(|row| (PathBuf::from(&row.repo), row.branch.clone()))
            .collect();
        let cache = &mut self.pr_cache;
        let branch_of: HashMap<&PathBuf, &String> =
            wanted.iter().map(|(path, branch)| (path, branch)).collect();
        let before = cache.checkouts.len();
        cache
            .checkouts
            .retain(|path, _| branch_of.contains_key(path));
        let mut dropped = cache.checkouts.len() != before;
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
                    dropped |= cache
                        .checkouts
                        .insert(path.clone(), CheckoutPr::new(branch, true))
                        .is_some();
                }
            }
            due.push((path.clone(), branch.clone()));
        }
        cache.prune_answers();
        cache.rehot();
        if dropped {
            self.settle_pr_outages();
        }
        due
    }

    /// End every PR lookup outage that no record fails on any more. Runs
    /// only after a lookup succeeded or a failure moved its record to
    /// another source ([`Self::note_pr_landing`]), or records were dropped
    /// ([`Self::due_pr_lookups`], [`Self::apply_pr_open`]):
    /// `r`, a full reload, and a lookup in flight keep each record's last
    /// failure, so they never end an outage.
    fn settle_pr_outages(&mut self) {
        let working: Vec<OutageSource> = self
            .outages
            .sources()
            .filter(|source| matches!(source, OutageSource::PrLookup { .. }))
            .filter(|source| !self.pr_cache.fails_on(source))
            .cloned()
            .collect();
        for source in &working {
            self.note_recovered(source);
        }
    }

    /// Feed a landed lookup to the PR lookup outages at `now`: a failure
    /// starts or updates the outage of its forge CLI and host (one toast
    /// per outage), and an answer ends each outage no record fails on any
    /// more. A failure that moved its record to another source does too,
    /// since the old source may have no failing record left.
    fn note_pr_landing(&mut self, landed: LandedLookup, now: Instant) {
        match landed {
            LandedLookup::Dropped => {}
            LandedLookup::Answered => self.settle_pr_outages(),
            LandedLookup::Failed {
                source,
                text,
                moved,
            } => {
                self.note_outage(source, text, now);
                if moved {
                    self.settle_pr_outages();
                }
            }
        }
    }

    /// Snapshot rows whose PR is looked up: visible (not hidden, unless
    /// hidden repos show), on a local branch, and not on the repository's
    /// known default branch ([`on_default_branch`]). Every PR surface reads
    /// these rows, so a default-branch checkout gets no lookup, poll,
    /// badge, total, popover section, alert, or PRs tab row.
    pub(super) fn pr_rows(&self) -> impl Iterator<Item = &WorkspaceRepoSnapshot> {
        self.branch_rows().filter(|row| !on_default_branch(row))
    }

    /// Snapshot rows that are visible (not hidden, unless hidden repos
    /// show) and on a local branch.
    fn branch_rows(&self) -> impl Iterator<Item = &WorkspaceRepoSnapshot> {
        self.snapshot
            .repos
            .iter()
            .filter(|row| self.show_hidden || !row.hidden)
            .filter(|row| is_counted_local_branch(&row.branch))
    }

    /// Checkouts on their known default branch whose remote needs a
    /// git-only resolve for commit links: a new or changed checkout, or one
    /// that `r` or a full reload asked to resolve again.
    ///
    /// Syncs the remote records with the snapshot like
    /// [`Self::due_pr_lookups`]: a checkout that vanished, is hidden, or
    /// left its default branch is dropped. No forge CLI runs for these.
    pub(crate) fn due_remote_lookups(&mut self) -> Vec<(PathBuf, String)> {
        let wanted: Vec<(PathBuf, String)> = self
            .branch_rows()
            .filter(|row| on_default_branch(row))
            .map(|row| (PathBuf::from(&row.repo), row.branch.clone()))
            .collect();
        let remotes = &mut self.pr_cache.default_remotes;
        remotes.retain(|path, record| {
            wanted
                .iter()
                .any(|(want, branch)| want == path && *branch == record.branch)
        });
        let mut due = Vec::new();
        for (path, branch) in wanted {
            match remotes.get_mut(&path) {
                Some(record) => {
                    if !std::mem::take(&mut record.look_again) {
                        continue;
                    }
                }
                None => {
                    remotes.insert(
                        path.clone(),
                        CheckoutRemote {
                            branch: branch.clone(),
                            remote: None,
                            look_again: false,
                        },
                    );
                }
            }
            due.push((path, branch));
        }
        due
    }

    /// Land the remote that a git-only resolve gave for checkout `repo` on
    /// its default branch `branch`. False when the checkout left that
    /// branch or vanished since the resolve was queued.
    pub(crate) fn apply_checkout_remote(
        &mut self,
        repo: &Path,
        branch: &str,
        remote: Option<String>,
    ) -> bool {
        match self.pr_cache.default_remotes.get_mut(repo) {
            Some(record) if record.branch == branch => {
                record.remote = remote;
                true
            }
            _ => false,
        }
    }

    /// True when checkout `repo` is a snapshot row on `branch`, and
    /// `branch` is its repository's known default branch.
    fn is_on_default_branch(&self, repo: &Path, branch: &str) -> bool {
        self.snapshot
            .repos
            .iter()
            .find(|row| Path::new(&row.repo) == repo)
            .is_some_and(|row| row.branch == branch && on_default_branch(row))
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
    /// A merged PR is not polled. A checkout whose last lookup failed is
    /// polled until a lookup succeeds, also with no upstream or a merged
    /// PR, so a working forge ends its outage. Reads the facts each record keeps
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
    /// lookup can still queue a transition. The remote of a checkout on its
    /// default branch is resolved again too, and stays until that lands.
    pub(crate) fn forget_pr_lookups(&mut self, repo: Option<&Path>) {
        let cache = &mut self.pr_cache;
        for (path, record) in &mut cache.default_remotes {
            if repo.is_none_or(|repo| repo == path.as_path()) {
                record.look_again = true;
            }
        }
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
    /// the last answer of the branch, so a known badge stays, records the
    /// failure, and starts or updates the outage of its forge CLI and host
    /// (one sticky toast per outage, [`Self::note_outage`]). A failed poll
    /// backs its forge host off. A success may queue a [`PrTransition`] and
    /// ends each outage no record fails on any more. Returns true when the
    /// cache changed.
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
        let landed = self
            .pr_cache
            .land(repo, branch, remote, lookup, poll_cycle, now);
        self.note_pr_landing(landed, now);
        self.restore_popover_focus(focus);
        true
    }

    /// End the badge lookup of `branch` in checkout `repo` with no answer
    /// (its worker panicked): the badge, the last failure, and the host
    /// back-off stay, and the next poll cycle may look it up again.
    pub(crate) fn abandon_pr_lookup(&mut self, repo: &Path, branch: &str) {
        if let Some(record) = self
            .pr_cache
            .checkouts
            .get_mut(repo)
            .filter(|record| record.branch == branch)
        {
            record.in_flight = false;
            record.poll_cycle = None;
        }
    }

    /// Land a `gx` result (or a `gc` result that looked the PR up first):
    /// say what happened, and cache the fresh answer.
    ///
    /// `opened` is true when the browser started for `page` of a found PR.
    /// A `gc` whose PR has no CI web page says `no CI for <ref>`. The
    /// answer lands like a badge lookup's (a failure keeps a known badge
    /// and feeds the outage of its forge host).
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
            let now = Instant::now();
            let cache = &mut self.pr_cache;
            let mut replaced = false;
            if cache
                .checkouts
                .get(repo)
                .is_none_or(|record| record.branch != branch)
            {
                replaced = cache
                    .checkouts
                    .insert(repo.to_path_buf(), CheckoutPr::new(branch, false))
                    .is_some();
            }
            let landed = cache.land(repo, branch, remote, lookup, None, now);
            self.note_pr_landing(landed, now);
            if replaced {
                // The record of the old branch took its failure with it.
                self.settle_pr_outages();
            }
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
    /// checkout path, and PR mark ([`Self::pr_mark`]: a state, or the
    /// failure mark of a failed lookup with no earlier answer).
    ///
    /// A row is badged only when the branch it paints is the checkout's
    /// snapshot branch, the branch the cached answer is for. A graph loaded
    /// before a branch switch still paints the old branch, so it gets no
    /// badge. Commit rows never get one.
    pub(crate) fn graph_pr_badges(&self) -> Vec<(usize, PathBuf, PrMark)> {
        let Some(model) = self.graph.as_ref() else {
            return Vec::new();
        };
        let badge = |worktree: &Worktree| {
            let path = Path::new(&worktree.path);
            let branch = worktree.branch.as_deref()?;
            if self.snapshot_branch(path) != Some(branch) {
                return None;
            }
            self.pr_mark(path)
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
    /// stale ([`Self::pr_is_stale`]). A failed lookup with no earlier answer
    /// gives the failure mark alone ([`Self::pr_mark`]). `None` when the
    /// checkout shows no mark.
    pub(crate) fn tree_pr_badge(
        &self,
        checkout: &Path,
        tree_width: usize,
        now: Instant,
    ) -> Option<PrBadge> {
        let Some(pr) = self.pull_request_for(checkout) else {
            return (self.pr_mark(checkout)? == PrMark::LookupFailed).then_some(PrBadge {
                mark: PrMark::LookupFailed,
                ref_text: None,
                stale: false,
            });
        };
        Some(PrBadge {
            mark: PrMark::State(pr.state),
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
        let checkout = self.graph_header_checkout()?;
        Some((checkout.to_path_buf(), self.pull_request_for(checkout)?))
    }

    /// The graph's checkout when the header chip shows the failure mark: its
    /// last lookup failed with no earlier answer ([`PrMark::LookupFailed`]),
    /// under the branch rule of [`Self::graph_header_pr`].
    pub(crate) fn graph_header_lookup_failed(&self) -> Option<PathBuf> {
        let checkout = self.graph_header_checkout()?;
        (self.pr_mark(checkout)? == PrMark::LookupFailed).then(|| checkout.to_path_buf())
    }

    /// The graph's checkout when the graph's sync header paints the branch
    /// the checkout is on now.
    fn graph_header_checkout(&self) -> Option<&Path> {
        let (checkout, _) = self.graph_identity.as_ref()?;
        let checkout = Path::new(checkout);
        let sync = self.graph.as_ref()?.sync.as_ref()?;
        (self.snapshot_branch(checkout) == Some(sync.branch.as_str())).then_some(checkout)
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
mod tests;

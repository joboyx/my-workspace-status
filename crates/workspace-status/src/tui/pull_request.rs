//! Find the pull request of a branch on its forge and open it in a browser.
//!
//! One module for GitHub pull requests and GitLab merge requests. User-facing
//! text calls both "PR".
//!
//! The logic is pure: [`parse_remote`] reads the forge from a remote URL, the
//! argv builders make the `gh` / `glab` commands, and the selection rules pick
//! one PR from the CLI's JSON. IO stays at the edges: [`lookup`] takes the
//! command runner as a parameter ([`run_cli`] is the real one), and
//! [`open_in_browser`] spawns the operator's browser. Both block, so callers
//! run them on a worker thread, never on the TTY event thread. Both take a
//! cancel flag: once it is set they stop waiting at once, so a quit does not
//! wait for a slow forge CLI or opener.
//!
//! Which PR counts: an open PR, else a merged PR, from a branch of the same
//! repository (a fork PR whose branch has the same name does not count). A
//! closed PR that was not merged counts as none. Among several candidates of the same kind, an exact
//! head-branch match wins, then the most recently updated one.
//!
//! The forge fields of the PR that counts feed the status rules in
//! [`super::pr_status`]: badge state, next action, notes, checks, and CI
//! link ([`PullRequest`]).
//!
//! [`lookup_detail`] fetches the rich fields of one known PR (title, review,
//! author, branches, checks, check names, reviews, notes, CI link, update
//! time) for the PR popover, with the same runner and argv-builder style.
//!
//! A runner answers `Ok(stdout)`, or `Err` with the reason when there is
//! one: the CLI's first stderr line, or `<program> not found`.

use std::collections::HashMap;
use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;

use super::pr_status::{
    badge_state, can_stack_on, checks_ci_link, named_checks, named_failed_check, next_action,
    pipeline_ci_link, Check, CheckBucket, ChecksSummary, CiLink, NextAction, PrNotes, PrState,
    StatusFacts,
};

/// Status line when a branch has no PR that counts.
pub fn no_pr_status(branch: &str) -> String {
    format!("no PR for {branch}")
}

/// Status line when the focused row has no branch to look up.
pub const NO_PR_FOR_ROW: &str = "no PR for this row";

/// Status line when the forge CLI could not answer for `branch`.
pub fn lookup_failed_status(branch: &str) -> String {
    format!("could not look up PR for {branch}")
}

/// Status line when the browser could not be started.
pub const OPEN_FAILED: &str = "could not open PR";

/// Status line when PR `pr_ref` (`#318`, `!12`) has no CI page to open.
pub fn no_ci_status(pr_ref: &str) -> String {
    format!("no CI for {pr_ref}")
}

/// Status line when the browser could not be started for a CI page.
pub const CI_OPEN_FAILED: &str = "could not open CI";

/// Status line after the browser started for the CI page of PR `pr_ref`.
pub fn ci_opened_status(pr_ref: &str) -> String {
    format!("opened CI for {pr_ref}")
}

/// The page of a PR that a `gx` / `gc` open shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrPage {
    /// The PR itself (`gx`).
    Pr,
    /// Its CI page, [`PullRequest::ci_link`] (`gc`).
    Ci,
}

/// Code-hosting service a remote points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Forge {
    /// GitHub or GitHub Enterprise, queried with `gh`.
    GitHub,
    /// GitLab (hosted or self-managed), queried with `glab`.
    GitLab,
}

/// A repository on a forge, read from a git remote URL.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForgeRepo {
    /// Which forge CLI answers for this repository.
    pub forge: Forge,
    /// Lowercase web host, for example `github.com`.
    pub host: String,
    /// `owner/repo`, or `group/sub/repo` on GitLab. No `.git` suffix.
    pub path: String,
}

/// The PR that counts for a branch: the badge answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    /// GitHub PR number or GitLab MR iid.
    pub number: u64,
    /// Web URL of the PR page.
    pub url: String,
    /// Forge of the PR; picks the ref form ([`Self::ref_text`]).
    pub forge: Forge,
    /// Title; empty when the forge sent none.
    pub title: String,
    /// Head (source) branch; empty when the forge sent none.
    pub head: String,
    /// Base (target) branch; empty when the forge sent none.
    pub base: String,
    /// Badge state.
    pub state: PrState,
    /// Merge and review blockers; all false on a merged PR.
    pub notes: PrNotes,
    /// Checks of the head commit (GitLab: the head pipeline).
    pub checks: ChecksSummary,
    /// Name of the failed GitHub check that [`Self::ci_link`] opens
    /// ([`super::pr_status::named_failed_check`]). GitLab names jobs only in the
    /// detail, so a GitLab PR has none here.
    pub failed_check: Option<String>,
    /// CI page; [`CiLink::kind`] names its action.
    pub ci_link: CiLink,
    /// Last update, in Unix seconds.
    pub updated_at: Option<i64>,
    /// False while the forge still computes the merge facts (GitHub
    /// `mergeStateStatus: UNKNOWN`; GitLab `detailed_merge_status`
    /// `checking`, `unchecked`, `approvals_syncing`, or `preparing`, or on
    /// an older server `merge_status` `unchecked`, `checking`, or
    /// `cannot_be_merged_recheck`). Ready to merge versus in review is then
    /// not final. Always true for a merged PR.
    pub merge_state_settled: bool,
}

impl PullRequest {
    /// `#318` on GitHub, `!12` on GitLab.
    pub fn ref_text(&self) -> String {
        match self.forge {
            Forge::GitHub => format!("#{}", self.number),
            Forge::GitLab => format!("!{}", self.number),
        }
    }

    /// What the author does next; `None` when merged.
    ///
    /// `stacked_on` is the PR [`Self::stacked_on`] found. Only an open one
    /// changes the answer: a ready PR then says `merge <its ref> first`.
    pub fn next_action(&self, stacked_on: Option<&PullRequest>) -> Option<NextAction> {
        let open_base = stacked_on
            .filter(|base| base.state != PrState::Merged)
            .map(PullRequest::ref_text);
        next_action(self.state, self.notes, open_base.as_deref())
    }

    /// True when the next action is the author's: fix CI, fix conflicts,
    /// address review, or merge (the base first, or this PR). The stack
    /// does not change it.
    pub fn needs_you(&self) -> bool {
        self.next_action(None)
            .is_some_and(|action| action.needs_you())
    }

    /// The PR this one is stacked on: among `known`, every PR of the same
    /// repository that is known, one whose head branch is this PR's base
    /// branch. An open one wins over a merged one, then the highest number.
    /// The caller passes PRs of this repository only; this PR itself never
    /// counts.
    ///
    /// A base that is the repository's default branch `default_branch` (the
    /// configured one, when set) or a long-lived branch such as `main` or
    /// `develop` is never a stack ([`can_stack_on`]), even when some PR has
    /// it as its head.
    pub fn stacked_on<'a>(
        &self,
        known: impl IntoIterator<Item = &'a PullRequest>,
        default_branch: Option<&str>,
    ) -> Option<&'a PullRequest> {
        if !can_stack_on(&self.base, default_branch) {
            return None;
        }
        known
            .into_iter()
            .filter(|pr| pr.number != self.number && pr.head == self.base)
            .max_by_key(|pr| (pr.state != PrState::Merged, pr.number))
    }

    /// A PR with only `number`, `url`, and `state` set: forge from the URL,
    /// branch `feature` into `main`, no notes or checks.
    #[cfg(test)]
    pub(crate) fn fixture(number: u64, url: &str, state: PrState) -> Self {
        let forge = if url.contains("gitlab") {
            Forge::GitLab
        } else {
            Forge::GitHub
        };
        let ci_link = match forge {
            Forge::GitHub => checks_ci_link(&[], url),
            Forge::GitLab => pipeline_ci_link(None, url),
        };
        Self {
            number,
            url: url.to_string(),
            forge,
            title: String::new(),
            head: "feature".into(),
            base: "main".into(),
            state,
            notes: PrNotes::default(),
            checks: ChecksSummary::default(),
            failed_check: None,
            ci_link,
            updated_at: None,
            merge_state_settled: true,
        }
    }
}

/// Result of one PR lookup for a branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrLookup {
    /// The open PR, else the merged PR, for the branch.
    Found(PullRequest),
    /// The forge answered and no PR counts: no remote, unsupported forge, no
    /// PR, or only closed PRs that were not merged.
    NoPr,
    /// No answer: the CLI is missing, is not authenticated, exited non-zero,
    /// or printed output that does not parse. Holds the reason when there is
    /// one (the CLI's first stderr line, or `gh not found` / `glab not
    /// found`).
    Failed(Option<String>),
}

/// Review decision of a PR, as the PR popover shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrReview {
    /// GitHub `APPROVED`, or a GitLab MR whose approval rules are met.
    Approved,
    /// GitHub `CHANGES_REQUESTED`.
    ChangesRequested,
    /// GitHub `REVIEW_REQUIRED`.
    ReviewRequired,
    /// An open GitLab MR whose approval rules are not met (or whose
    /// approvals could not be read).
    NotApproved,
}

impl PrReview {
    /// Upper-case label, in the GitHub words.
    pub fn label(self) -> &'static str {
        match self {
            Self::Approved => "APPROVED",
            Self::ChangesRequested => "CHANGES_REQUESTED",
            Self::ReviewRequired => "REVIEW_REQUIRED",
            Self::NotApproved => "NOT_APPROVED",
        }
    }
}

/// How many reviewers approved a PR, as the PR popover counts them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReviewCount {
    /// GitHub: distinct authors whose latest review is `APPROVED`. The
    /// GitHub CLI does not expose how many approvals are required.
    Approved(usize),
    /// GitLab: approvals that count toward the rules
    /// (`approvals_required − approvals_left`, at least 0) and approvals the
    /// rules require (`approvals_required`). An approval from a user the
    /// rules do not count is not given. Whether the rules are met is
    /// [`PullRequestDetail::review`].
    Approvals {
        /// Approvals that count toward the rules.
        given: usize,
        /// Approvals the rules require.
        required: usize,
    },
}

/// Rich fields of one PR, for the PR popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequestDetail {
    /// GitHub PR number or GitLab MR iid.
    pub number: u64,
    /// Title; empty when the forge sent none.
    pub title: String,
    /// `OPEN`, `MERGED`, `CLOSED`, or another forge state in upper case.
    pub state: String,
    /// True for a draft PR.
    pub draft: bool,
    /// Author login (GitHub) or username (GitLab).
    pub author: Option<String>,
    /// Head (source) branch.
    pub head: Option<String>,
    /// Base (target) branch.
    pub base: Option<String>,
    /// Review decision; `None` when the forge reports none.
    pub review: Option<PrReview>,
    /// Approval count; `None` when the forge did not report it.
    pub reviews: Option<ReviewCount>,
    /// Checks of the head commit.
    pub checks: ChecksSummary,
    /// GitLab head pipeline status as GitLab prints it (`failed`,
    /// `running`, …); `None` on GitHub and for no pipeline.
    pub pipeline: Option<String>,
    /// Failed, then running, checks (GitHub) or jobs (GitLab) to name: at
    /// most [`super::pr_status::NAMED_CHECKS_MAX`].
    pub check_names: Vec<Check>,
    /// Failed or running checks beyond [`Self::check_names`].
    pub more_checks: usize,
    /// Merge and review blockers; all false unless the PR is open.
    pub notes: PrNotes,
    /// CI page; [`CiLink::kind`] names its action.
    pub ci_link: CiLink,
    /// Last update, in Unix seconds.
    pub updated_at: Option<i64>,
}

/// Result of one PR detail fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrDetailLookup {
    /// The forge described the PR.
    Found(PullRequestDetail),
    /// No answer: unsupported remote, CLI missing or failed, or output that
    /// does not parse. Holds the reason when there is one, as
    /// [`PrLookup::Failed`] does.
    Failed(Option<String>),
}

/// Read the forge, host, and repository path from a git remote URL.
///
/// Accepts scp-like `git@host:owner/repo.git`, `ssh://git@host[:port]/…`,
/// `https://host/…`, and `http://host/…`, with or without `.git` and a
/// trailing slash. An SSH port is dropped because it is not the web port. A
/// host that contains `github` is GitHub (exactly `owner/repo`); one that
/// contains `gitlab` is GitLab (subgroups allowed). Anything else is `None`.
pub fn parse_remote(url: &str) -> Option<ForgeRepo> {
    let url = url.trim();
    let (host, path) = match url.split_once("://") {
        Some((scheme, rest)) => {
            let scheme = scheme.to_ascii_lowercase();
            let web = match scheme.as_str() {
                "https" | "http" => true,
                "ssh" | "git+ssh" | "git" => false,
                _ => return None,
            };
            let (authority, path) = rest.split_once('/')?;
            let host = authority.rsplit_once('@').map_or(authority, |(_, h)| h);
            let host = if web {
                host
            } else {
                host.split(':').next().unwrap_or(host)
            };
            (host, path)
        }
        None => {
            let (left, path) = url.split_once(':')?;
            if left.contains('/') {
                return None;
            }
            (left.rsplit_once('@').map_or(left, |(_, h)| h), path)
        }
    };
    let host = host.to_ascii_lowercase();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    let path = path.split(['?', '#']).next().unwrap_or_default();
    let path = path.trim_matches('/');
    let path = path.strip_suffix(".git").unwrap_or(path);
    let segments: Vec<&str> = path.split('/').collect();
    if segments.len() < 2 || segments.iter().any(|s| s.is_empty()) {
        return None;
    }
    let forge = if host.contains("github") {
        if segments.len() != 2 {
            return None;
        }
        Forge::GitHub
    } else if host.contains("gitlab") {
        Forge::GitLab
    } else {
        return None;
    };
    Some(ForgeRepo {
        forge,
        host,
        path: path.to_string(),
    })
}

/// Web page of commit `sha` in `repo`.
///
/// GitHub: `https://{host}/{path}/commit/{sha}`. GitLab:
/// `https://{host}/{path}/-/commit/{sha}`. Pass the full commit id, not
/// the short one shown on screen.
pub fn commit_url(repo: &ForgeRepo, sha: &str) -> String {
    let sep = match repo.forge {
        Forge::GitHub => "",
        Forge::GitLab => "-/",
    };
    format!("https://{}/{}/{sep}commit/{sha}", repo.host, repo.path)
}

/// Percent-encode everything except RFC 3986 unreserved characters.
///
/// `/` becomes `%2F`, which GitLab needs for a project path in the API.
pub(crate) fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `gh pr list` argv (program first) for every PR whose head is `branch`.
pub fn github_list_argv(repo: &ForgeRepo, branch: &str) -> Vec<String> {
    [
        "gh",
        "pr",
        "list",
        "-R",
        &format!("{}/{}", repo.host, repo.path),
        "--head",
        branch,
        "--state",
        "all",
        "--json",
        "number,state,url,title,headRefName,baseRefName,isCrossRepository,isDraft,\
         reviewDecision,mergeStateStatus,statusCheckRollup,updatedAt",
        "--limit",
        "30",
    ]
    .map(str::to_string)
    .to_vec()
}

/// `glab api` argv (program first) for every MR whose source is `branch`.
pub fn gitlab_list_argv(repo: &ForgeRepo, branch: &str) -> Vec<String> {
    let endpoint = format!(
        "projects/{}/merge_requests?source_branch={}&state=all&per_page=30",
        percent_encode(&repo.path),
        percent_encode(branch)
    );
    gitlab_api_argv(repo, endpoint)
}

/// `glab api` argv (program first) for the approval state of MR `iid`.
pub fn gitlab_approvals_argv(repo: &ForgeRepo, iid: u64) -> Vec<String> {
    let endpoint = format!(
        "projects/{}/merge_requests/{iid}/approvals",
        percent_encode(&repo.path)
    );
    gitlab_api_argv(repo, endpoint)
}

/// `gh pr view` argv (program first) for the detail fields of PR `number`.
pub fn github_view_argv(repo: &ForgeRepo, number: u64) -> Vec<String> {
    [
        "gh",
        "pr",
        "view",
        &number.to_string(),
        "-R",
        &format!("{}/{}", repo.host, repo.path),
        "--json",
        "number,url,title,state,isDraft,author,headRefName,baseRefName,\
         reviewDecision,mergeStateStatus,statusCheckRollup,latestReviews,updatedAt",
    ]
    .map(str::to_string)
    .to_vec()
}

/// `glab api` argv (program first) for MR `iid`: its detail fields and its
/// head pipeline.
pub fn gitlab_mr_argv(repo: &ForgeRepo, iid: u64) -> Vec<String> {
    let endpoint = format!(
        "projects/{}/merge_requests/{iid}",
        percent_encode(&repo.path)
    );
    gitlab_api_argv(repo, endpoint)
}

/// `glab api` argv (program first) for the failed and running jobs of
/// pipeline `pipeline`.
pub fn gitlab_jobs_argv(repo: &ForgeRepo, pipeline: u64) -> Vec<String> {
    let endpoint = format!(
        "projects/{}/pipelines/{pipeline}/jobs?scope[]=failed&scope[]=running",
        percent_encode(&repo.path)
    );
    gitlab_api_argv(repo, endpoint)
}

/// Web page of PR `number` in `repo`, for a detail answer that left out
/// its URL.
fn pr_page_url(repo: &ForgeRepo, number: u64) -> String {
    let page = match repo.forge {
        Forge::GitHub => "pull",
        Forge::GitLab => "-/merge_requests",
    };
    format!("https://{}/{}/{page}/{number}", repo.host, repo.path)
}

fn gitlab_api_argv(repo: &ForgeRepo, endpoint: String) -> Vec<String> {
    vec![
        "glab".to_string(),
        "api".to_string(),
        "--hostname".to_string(),
        repo.host.clone(),
        endpoint,
    ]
}

/// A forge CLI runner: argv (program first) in; stdout of a zero exit, or
/// the reason the call failed when there is one.
type Runner<'a> = &'a dyn Fn(&[String]) -> Result<String, Option<String>>;

/// Parse forge JSON. Output that does not parse fails with no reason.
fn parse<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, Option<String>> {
    serde_json::from_str(json).map_err(|_| None)
}

/// Status fields that `gh pr list` and `gh pr view` both print.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubStatus {
    #[serde(default)]
    is_draft: Option<bool>,
    #[serde(default)]
    review_decision: Option<String>,
    /// `CLEAN`, `DIRTY` (conflicts), `BLOCKED`, `BEHIND`, `UNSTABLE`, or
    /// `UNKNOWN` while GitHub still computes it.
    #[serde(default)]
    merge_state_status: Option<String>,
    #[serde(default)]
    status_check_rollup: Option<Vec<GitHubCheck>>,
}

/// One `gh pr list --json` entry.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPr {
    number: u64,
    state: String,
    url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    head_ref_name: Option<String>,
    #[serde(default)]
    base_ref_name: Option<String>,
    /// True for a PR from a fork; `--head` matches the branch name only.
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(flatten)]
    status: GitHubStatus,
}

/// Merge fields that the GitLab MR list and the single MR both print.
#[derive(Deserialize)]
struct GitLabMerge {
    #[serde(default)]
    draft: Option<bool>,
    /// Older GitLab name of `draft`.
    #[serde(default)]
    work_in_progress: Option<bool>,
    /// GitLab 15.6 and later: `mergeable`, `conflict`,
    /// `discussions_not_resolved`, `ci_still_running`, …
    #[serde(default)]
    detailed_merge_status: Option<String>,
    /// Older coarse status: `can_be_merged`, `cannot_be_merged`, …
    #[serde(default)]
    merge_status: Option<String>,
    #[serde(default)]
    has_conflicts: Option<bool>,
    #[serde(default)]
    blocking_discussions_resolved: Option<bool>,
}

impl GitLabMerge {
    fn draft(&self) -> bool {
        self.draft.or(self.work_in_progress).unwrap_or(false)
    }

    /// `detailed_merge_status`, or `None` when the server does not send it.
    fn detailed(&self) -> Option<&str> {
        self.detailed_merge_status
            .as_deref()
            .filter(|status| !status.is_empty())
    }

    /// These fields, each one taken from `older` when this answer leaves it
    /// out (missing, `null`, or an empty status).
    fn or(self, older: GitLabMerge) -> GitLabMerge {
        let status = |text: Option<String>| text.filter(|text| !text.is_empty());
        GitLabMerge {
            draft: self.draft.or(older.draft),
            work_in_progress: self.work_in_progress.or(older.work_in_progress),
            detailed_merge_status: status(self.detailed_merge_status)
                .or(status(older.detailed_merge_status)),
            merge_status: status(self.merge_status).or(status(older.merge_status)),
            has_conflicts: self.has_conflicts.or(older.has_conflicts),
            blocking_discussions_resolved: self
                .blocking_discussions_resolved
                .or(older.blocking_discussions_resolved),
        }
    }

    /// False while GitLab still computes the merge status (see
    /// [`PullRequest::merge_state_settled`]).
    fn settled(&self) -> bool {
        match self.detailed() {
            Some(status) => !matches!(
                status,
                "checking" | "unchecked" | "approvals_syncing" | "preparing"
            ),
            None => !matches!(
                self.merge_status.as_deref(),
                Some("unchecked" | "checking" | "cannot_be_merged_recheck")
            ),
        }
    }
}

/// One GitLab `merge_requests` list entry.
#[derive(Deserialize)]
struct GitLabMr {
    iid: u64,
    state: String,
    web_url: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    source_branch: Option<String>,
    #[serde(default)]
    target_branch: Option<String>,
    /// Differs from `target_project_id` for an MR from a fork.
    #[serde(default)]
    source_project_id: Option<u64>,
    #[serde(default)]
    target_project_id: Option<u64>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(flatten)]
    merge: GitLabMerge,
}

/// GitLab `merge_requests/<iid>/approvals` response.
#[derive(Deserialize)]
struct GitLabApprovals {
    #[serde(default)]
    approved: Option<bool>,
    #[serde(default)]
    approvals_required: Option<u64>,
    #[serde(default)]
    approvals_left: Option<u64>,
}

/// One GitLab pipeline job.
#[derive(Deserialize)]
struct GitLabJob {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
}

/// Open or merged: the only PR kinds that count.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Merged,
}

/// A PR that counts, with what [`pick`] ranks it by.
struct Candidate<T> {
    kind: Kind,
    exact_head: bool,
    /// RFC 3339 timestamp as the forge prints it; compared as text.
    updated_at: String,
    pr: T,
}

/// Open before merged, then exact head match, then most recently updated.
///
/// Both forges print `updated_at` in UTC with a fixed layout, so text order
/// is time order.
fn pick<T>(candidates: Vec<Candidate<T>>) -> Option<Candidate<T>> {
    candidates.into_iter().max_by(|a, b| {
        let rank = |c: &Candidate<T>| (c.kind == Kind::Open, c.exact_head);
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.updated_at.cmp(&b.updated_at))
    })
}

/// Open or merged from the forge state; `None` for a PR that does not
/// count (closed, locked).
fn kind_of(state: &str) -> Option<Kind> {
    match state {
        "OPEN" | "opened" => Some(Kind::Open),
        "MERGED" | "merged" => Some(Kind::Merged),
        _ => None,
    }
}

/// Bucket of one GitHub rollup entry.
///
/// Failed: a check run conclusion `FAILURE`, `TIMED_OUT`, `CANCELLED`,
/// `ACTION_REQUIRED`, `STARTUP_FAILURE`, or `ERROR`, or a status context
/// state `FAILURE` / `ERROR`. Else pending: a check run whose status is not
/// `COMPLETED`, or a status context state `PENDING` / `EXPECTED`. Anything
/// else passes.
fn github_check_bucket(check: &GitHubCheck) -> CheckBucket {
    let failed = matches!(
        check.conclusion.as_deref(),
        Some(
            "FAILURE" | "TIMED_OUT" | "CANCELLED" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "ERROR"
        )
    ) || matches!(check.state.as_deref(), Some("FAILURE" | "ERROR"));
    if failed {
        return CheckBucket::Fail;
    }
    let running = check
        .status
        .as_deref()
        .is_some_and(|status| !status.is_empty() && status != "COMPLETED")
        || matches!(check.state.as_deref(), Some("PENDING" | "EXPECTED"));
    if running {
        CheckBucket::Pending
    } else {
        CheckBucket::Pass
    }
}

/// Every rollup entry as a [`Check`], in forge order.
fn github_checks(status: &GitHubStatus) -> Vec<Check> {
    let non_empty = |text: &Option<String>| text.clone().filter(|text| !text.is_empty());
    status
        .status_check_rollup
        .iter()
        .flatten()
        .map(|check| Check {
            bucket: github_check_bucket(check),
            name: non_empty(&check.name)
                .or_else(|| non_empty(&check.context))
                .unwrap_or_default(),
            url: non_empty(&check.details_url).or_else(|| non_empty(&check.target_url)),
        })
        .collect()
}

/// Notes of a GitHub PR: conflicts on `mergeStateStatus: DIRTY`, changes
/// requested on `reviewDecision: CHANGES_REQUESTED`. None unless `open`.
fn github_notes(status: &GitHubStatus, open: bool) -> PrNotes {
    if !open {
        return PrNotes::default();
    }
    PrNotes {
        conflicts: status.merge_state_status.as_deref() == Some("DIRTY"),
        unresolved_threads: false,
        changes_requested: status.review_decision.as_deref() == Some("CHANGES_REQUESTED"),
    }
}

/// Badge state of a GitHub PR with `checks`.
///
/// Ready to merge: `mergeStateStatus: CLEAN`, or `reviewDecision: APPROVED`
/// with no failed or running check and a merge state that is not `DIRTY`,
/// `BLOCKED`, or `BEHIND` (branch protection blocks the merge until the
/// branch is updated). `UNKNOWN`, still computing, is none of them.
fn github_state(status: &GitHubStatus, merged: bool, checks: ChecksSummary) -> PrState {
    let merge_state = status.merge_state_status.as_deref();
    let ready = merge_state == Some("CLEAN")
        || (status.review_decision.as_deref() == Some("APPROVED")
            && checks.fail == 0
            && checks.pending == 0
            && !matches!(merge_state, Some("DIRTY" | "BLOCKED" | "BEHIND")));
    badge_state(&StatusFacts {
        merged,
        draft: status.is_draft.unwrap_or(false),
        checks,
        ready,
    })
}

/// Name of the failed check the PR names: [`named_failed_check`], the same
/// check the CI link opens. `None` when no check failed or it has no name.
fn failed_check_name(checks: &[Check]) -> Option<String> {
    named_failed_check(checks)
        .map(|check| check.name.clone())
        .filter(|name| !name.is_empty())
}

fn github_lookup(
    repo: &ForgeRepo,
    branch: &str,
    run: Runner,
) -> Result<Option<PullRequest>, Option<String>> {
    let prs: Vec<GitHubPr> = parse(&run(&github_list_argv(repo, branch))?)?;
    let candidates = prs
        .into_iter()
        .filter(|pr| !pr.is_cross_repository)
        .filter_map(|pr| {
            Some(Candidate {
                kind: kind_of(&pr.state)?,
                exact_head: pr.head_ref_name.as_deref() == Some(branch),
                updated_at: pr.updated_at.clone().unwrap_or_default(),
                pr,
            })
        })
        .collect();
    Ok(pick(candidates).map(|picked| github_pull_request(picked.pr, picked.kind)))
}

fn github_pull_request(pr: GitHubPr, kind: Kind) -> PullRequest {
    let checks = github_checks(&pr.status);
    let summary = ChecksSummary::of(&checks);
    PullRequest {
        number: pr.number,
        forge: Forge::GitHub,
        title: pr.title.unwrap_or_default(),
        head: pr.head_ref_name.unwrap_or_default(),
        base: pr.base_ref_name.unwrap_or_default(),
        state: github_state(&pr.status, kind == Kind::Merged, summary),
        notes: github_notes(&pr.status, kind == Kind::Open),
        checks: summary,
        failed_check: failed_check_name(&checks),
        ci_link: checks_ci_link(&checks, &pr.url),
        updated_at: pr.updated_at.as_deref().and_then(rfc3339_unix),
        merge_state_settled: kind == Kind::Merged
            || pr.status.merge_state_status.as_deref() != Some("UNKNOWN"),
        url: pr.url,
    }
}

/// Bucket of a GitLab pipeline or job status; `None` for no CI state
/// (`manual`, no pipeline, or a status GitLab may add later).
///
/// Failed: `failed`, `canceled`. Running: `created`,
/// `waiting_for_resource`, `preparing`, `pending`, `running`, `scheduled`.
/// Passed: `success`, `skipped`.
fn gitlab_ci_bucket(status: &str) -> Option<CheckBucket> {
    match status {
        "failed" | "canceled" => Some(CheckBucket::Fail),
        "created" | "waiting_for_resource" | "preparing" | "pending" | "running" | "scheduled" => {
            Some(CheckBucket::Pending)
        }
        "success" | "skipped" => Some(CheckBucket::Pass),
        _ => None,
    }
}

/// Checks summary of a GitLab head pipeline: one bucket, or none.
fn gitlab_checks(pipeline_status: Option<&str>) -> ChecksSummary {
    let mut summary = ChecksSummary::default();
    if let Some(bucket) = pipeline_status.and_then(gitlab_ci_bucket) {
        summary.add(bucket);
    }
    summary
}

/// Notes of a GitLab MR: conflicts on `has_conflicts` or
/// `detailed_merge_status: conflict`; unresolved threads on
/// `detailed_merge_status: discussions_not_resolved` or
/// `blocking_discussions_resolved: false`; changes requested on
/// `detailed_merge_status: requested_changes`. None unless `open`.
fn gitlab_notes(merge: &GitLabMerge, open: bool) -> PrNotes {
    if !open {
        return PrNotes::default();
    }
    let detailed = merge.detailed();
    PrNotes {
        conflicts: merge.has_conflicts == Some(true) || detailed == Some("conflict"),
        unresolved_threads: detailed == Some("discussions_not_resolved")
            || merge.blocking_discussions_resolved == Some(false),
        changes_requested: detailed == Some("requested_changes"),
    }
}

/// Badge state of a GitLab MR with `checks`.
///
/// Ready to merge: `detailed_merge_status: mergeable`. A server that does
/// not send `detailed_merge_status` (older than 15.6) is ready on
/// `merge_status: can_be_merged` with `approvals_left == 0` and a pipeline
/// that neither failed nor runs.
fn gitlab_state(
    merge: &GitLabMerge,
    merged: bool,
    checks: ChecksSummary,
    approvals_left: Option<u64>,
) -> PrState {
    let ready = match merge.detailed() {
        Some(status) => status == "mergeable",
        None => {
            merge.merge_status.as_deref() == Some("can_be_merged")
                && approvals_left == Some(0)
                && checks.fail == 0
                && checks.pending == 0
        }
    };
    badge_state(&StatusFacts {
        merged,
        draft: merge.draft(),
        checks,
        ready,
    })
}

/// The approvals of MR `iid`; `None` when the call fails or does not parse.
fn gitlab_approvals(repo: &ForgeRepo, iid: u64, run: Runner) -> Option<GitLabApprovals> {
    let json = run(&gitlab_approvals_argv(repo, iid)).ok()?;
    serde_json::from_str(&json).ok()
}

/// Failed and running jobs of `pipeline`, in GitLab order; none when the
/// call fails or does not parse.
fn gitlab_jobs(repo: &ForgeRepo, pipeline: u64, run: Runner) -> Vec<Check> {
    let jobs: Vec<GitLabJob> = run(&gitlab_jobs_argv(repo, pipeline))
        .ok()
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    jobs.into_iter()
        .filter_map(|job| {
            Some(Check {
                bucket: gitlab_ci_bucket(job.status.as_deref()?)?,
                name: job.name.unwrap_or_default(),
                url: job.web_url.filter(|url| !url.is_empty()),
            })
        })
        .collect()
}

/// List, pick, then for an open MR the MR call (head pipeline) and, when
/// neither answer has a `detailed_merge_status`, the approvals call.
///
/// The merge fields (`detailed_merge_status`, `has_conflicts`,
/// `blocking_discussions_resolved`, `draft`) of an open MR come from the MR
/// call, which is fresher than the list (the list can still say
/// `unchecked`); a field the MR call leaves out comes from the list. A
/// failed MR call fails the lookup. A failed approvals call leaves the MR
/// not ready.
fn gitlab_lookup(
    repo: &ForgeRepo,
    branch: &str,
    run: Runner,
) -> Result<Option<PullRequest>, Option<String>> {
    let mrs: Vec<GitLabMr> = parse(&run(&gitlab_list_argv(repo, branch))?)?;
    let candidates = mrs
        .into_iter()
        .filter(|mr| mr.source_project_id == mr.target_project_id)
        .filter_map(|mr| {
            Some(Candidate {
                kind: kind_of(&mr.state)?,
                exact_head: mr.source_branch.as_deref() == Some(branch),
                updated_at: mr.updated_at.clone().unwrap_or_default(),
                pr: mr,
            })
        })
        .collect();
    let Some(Candidate { kind, pr: mr, .. }) = pick(candidates) else {
        return Ok(None);
    };
    let mut pipeline = None;
    let mut approvals_left = None;
    let mut merge = mr.merge;
    if kind == Kind::Open {
        let view: GitLabMrView = parse(&run(&gitlab_mr_argv(repo, mr.iid))?)?;
        pipeline = view.head_pipeline;
        // The single MR is fresher than the list, which can still say
        // `unchecked`.
        merge = view.merge.or(merge);
        if merge.detailed().is_none() {
            approvals_left = gitlab_approvals(repo, mr.iid, run).and_then(|a| a.approvals_left);
        }
    }
    let pipeline = pipeline.unwrap_or_default();
    let checks = gitlab_checks(pipeline.status.as_deref());
    Ok(Some(PullRequest {
        number: mr.iid,
        forge: Forge::GitLab,
        title: mr.title.unwrap_or_default(),
        head: mr.source_branch.unwrap_or_default(),
        base: mr.target_branch.unwrap_or_default(),
        state: gitlab_state(&merge, kind == Kind::Merged, checks, approvals_left),
        notes: gitlab_notes(&merge, kind == Kind::Open),
        checks,
        failed_check: None,
        ci_link: pipeline_ci_link(pipeline.web_url.as_deref(), &mr.web_url),
        updated_at: mr.updated_at.as_deref().and_then(rfc3339_unix),
        merge_state_settled: kind == Kind::Merged || merge.settled(),
        url: mr.web_url,
    }))
}

/// Find the PR that counts for `branch` on the forge behind `remote_url`.
///
/// Blocks on the forge CLI through `run` (argv with the program first; `Ok`
/// is stdout of a zero exit, `Err` the reason when there is one). Pass
/// [`run_cli`] in production and a fake in tests. GitHub is one `gh pr
/// list`. GitLab is the MR list, then for an open MR the MR call (head
/// pipeline), plus the approvals call when the server sends no
/// `detailed_merge_status`.
pub fn lookup(remote_url: Option<&str>, branch: &str, run: Runner) -> PrLookup {
    let Some(repo) = remote_url.and_then(parse_remote) else {
        return PrLookup::NoPr;
    };
    let found = match repo.forge {
        Forge::GitHub => github_lookup(&repo, branch, run),
        Forge::GitLab => gitlab_lookup(&repo, branch, run),
    };
    match found {
        Ok(Some(pr)) => PrLookup::Found(pr),
        Ok(None) => PrLookup::NoPr,
        Err(why) => PrLookup::Failed(why),
    }
}

/// `gh pr view --json` detail object. Every field but the number may be
/// missing or `null`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPrView {
    number: u64,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    author: Option<ForgeUser>,
    #[serde(default)]
    head_ref_name: Option<String>,
    #[serde(default)]
    base_ref_name: Option<String>,
    #[serde(default)]
    latest_reviews: Option<Vec<GitHubReview>>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(flatten)]
    status: GitHubStatus,
}

/// Author object: `login` on GitHub, `username` on GitLab.
#[derive(Deserialize)]
struct ForgeUser {
    #[serde(default)]
    login: Option<String>,
    #[serde(default)]
    username: Option<String>,
}

impl ForgeUser {
    fn name(self) -> Option<String> {
        self.login.or(self.username).filter(|name| !name.is_empty())
    }
}

/// One `latestReviews` entry.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubReview {
    #[serde(default)]
    author: Option<ForgeUser>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    submitted_at: Option<String>,
}

/// One `statusCheckRollup` entry: a check run (`name`, `status`,
/// `conclusion`, `detailsUrl`) or a commit status context (`context`,
/// `state`, `targetUrl`).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubCheck {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    details_url: Option<String>,
    #[serde(default)]
    target_url: Option<String>,
}

/// GitLab `merge_requests/<iid>` object. Every field but the iid may be
/// missing or `null`.
#[derive(Deserialize)]
struct GitLabMrView {
    iid: u64,
    #[serde(default)]
    web_url: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    author: Option<ForgeUser>,
    #[serde(default)]
    source_branch: Option<String>,
    #[serde(default)]
    target_branch: Option<String>,
    #[serde(default)]
    head_pipeline: Option<GitLabPipeline>,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(flatten)]
    merge: GitLabMerge,
}

#[derive(Default, Deserialize)]
struct GitLabPipeline {
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
}

fn github_review(decision: &str) -> Option<PrReview> {
    match decision {
        "APPROVED" => Some(PrReview::Approved),
        "CHANGES_REQUESTED" => Some(PrReview::ChangesRequested),
        "REVIEW_REQUIRED" => Some(PrReview::ReviewRequired),
        _ => None,
    }
}

/// Distinct authors whose latest review (by `submittedAt`, then list
/// order) is `APPROVED`. A review with no author does not count.
fn approved_authors(reviews: Vec<GitHubReview>) -> usize {
    let mut latest: HashMap<String, (String, bool)> = HashMap::new();
    for review in reviews {
        let Some(author) = review.author.and_then(ForgeUser::name) else {
            continue;
        };
        let at = review.submitted_at.unwrap_or_default();
        let approved = review.state.as_deref() == Some("APPROVED");
        if latest.get(&author).is_none_or(|(seen, _)| *seen <= at) {
            latest.insert(author, (at, approved));
        }
    }
    latest.values().filter(|(_, approved)| *approved).count()
}

fn github_detail(json: &str, repo: &ForgeRepo) -> Result<PullRequestDetail, Option<String>> {
    let pr: GitHubPrView = parse(json)?;
    let state = pr.state.unwrap_or_default().to_ascii_uppercase();
    let checks = github_checks(&pr.status);
    let (check_names, more_checks) = named_checks(&checks);
    let url = pr
        .url
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| pr_page_url(repo, pr.number));
    Ok(PullRequestDetail {
        number: pr.number,
        title: pr.title.unwrap_or_default(),
        draft: pr.status.is_draft.unwrap_or(false),
        author: pr.author.and_then(ForgeUser::name),
        head: pr.head_ref_name.filter(|name| !name.is_empty()),
        base: pr.base_ref_name.filter(|name| !name.is_empty()),
        review: pr.status.review_decision.as_deref().and_then(github_review),
        reviews: pr
            .latest_reviews
            .map(|reviews| ReviewCount::Approved(approved_authors(reviews))),
        checks: ChecksSummary::of(&checks),
        pipeline: None,
        check_names,
        more_checks,
        notes: github_notes(&pr.status, state == "OPEN"),
        ci_link: checks_ci_link(&checks, &url),
        updated_at: pr.updated_at.as_deref().and_then(rfc3339_unix),
        state,
    })
}

/// The MR call, then for an open MR the approvals call and, when the head
/// pipeline failed or runs, the jobs call. Only a failed MR call fails the
/// detail.
fn gitlab_detail(
    repo: &ForgeRepo,
    iid: u64,
    run: Runner,
) -> Result<PullRequestDetail, Option<String>> {
    let mr: GitLabMrView = parse(&run(&gitlab_mr_argv(repo, iid))?)?;
    let state = mr.state.unwrap_or_default();
    let open = state == "opened";
    let pipeline = mr.head_pipeline.unwrap_or_default();
    let status = pipeline.status.filter(|status| !status.is_empty());
    let checks = gitlab_checks(status.as_deref());
    let (mut review, mut reviews, mut jobs) = (None, None, Vec::new());
    if open {
        let approvals = gitlab_approvals(repo, mr.iid, run);
        let approved = approvals.as_ref().and_then(|a| a.approved) == Some(true);
        review = Some(if approved {
            PrReview::Approved
        } else {
            PrReview::NotApproved
        });
        reviews = approvals.and_then(|a| {
            let required = a.approvals_required?;
            Some(ReviewCount::Approvals {
                given: usize::try_from(required.saturating_sub(a.approvals_left?)).ok()?,
                required: usize::try_from(required).ok()?,
            })
        });
        if checks.fail > 0 || checks.pending > 0 {
            if let Some(id) = pipeline.id {
                jobs = gitlab_jobs(repo, id, run);
            }
        }
    }
    let (check_names, more_checks) = named_checks(&jobs);
    let url = mr
        .web_url
        .filter(|url| !url.is_empty())
        .unwrap_or_else(|| pr_page_url(repo, mr.iid));
    Ok(PullRequestDetail {
        number: mr.iid,
        title: mr.title.unwrap_or_default(),
        state: if open {
            "OPEN".to_string()
        } else {
            state.to_ascii_uppercase()
        },
        draft: mr.merge.draft(),
        author: mr.author.and_then(ForgeUser::name),
        head: mr.source_branch.filter(|name| !name.is_empty()),
        base: mr.target_branch.filter(|name| !name.is_empty()),
        review,
        reviews,
        checks,
        check_names,
        more_checks,
        notes: gitlab_notes(&mr.merge, open),
        ci_link: pipeline_ci_link(pipeline.web_url.as_deref(), &url),
        pipeline: status,
        updated_at: mr.updated_at.as_deref().and_then(rfc3339_unix),
    })
}

/// Fetch the detail fields of PR `number` on the forge behind `remote_url`.
///
/// Blocks on the forge CLI through `run`, the same injectable runner as
/// [`lookup`]. GitHub is one `gh pr view`. GitLab is the MR endpoint, then,
/// for an open MR, the approvals endpoint ([`PrReview::Approved`] or
/// [`PrReview::NotApproved`], and the approval count) and, when the head
/// pipeline failed or runs, the jobs endpoint (the job names). An
/// unsupported remote fails without a CLI call.
pub fn lookup_detail(remote_url: &str, number: u64, run: Runner) -> PrDetailLookup {
    let Some(repo) = parse_remote(remote_url) else {
        return PrDetailLookup::Failed(None);
    };
    let detail = match repo.forge {
        Forge::GitHub => {
            run(&github_view_argv(&repo, number)).and_then(|json| github_detail(&json, &repo))
        }
        Forge::GitLab => gitlab_detail(&repo, number, run),
    };
    match detail {
        Ok(detail) => PrDetailLookup::Found(detail),
        Err(why) => PrDetailLookup::Failed(why),
    }
}

/// Unix seconds of an RFC 3339 timestamp as the forges print it
/// (`2026-01-02T03:04:05Z`, fractional seconds, or a `±HH:MM` offset).
fn rfc3339_unix(text: &str) -> Option<i64> {
    let (date, rest) = text.trim().split_once(['T', 't', ' '])?;
    let mut ymd = date.splitn(3, '-');
    let year: i64 = ymd.next()?.parse().ok()?;
    let month: i64 = ymd.next()?.parse().ok()?;
    let day: i64 = ymd.next()?.parse().ok()?;
    let (clock, offset) = match rest.strip_suffix(['Z', 'z']) {
        Some(clock) => (clock, 0),
        None => {
            let at = rest.rfind(['+', '-'])?;
            let (clock, zone) = rest.split_at(at);
            let (hours, minutes) = zone[1..].split_once(':')?;
            let secs = hours.parse::<i64>().ok()? * 3600 + minutes.parse::<i64>().ok()? * 60;
            (clock, if zone.starts_with('-') { -secs } else { secs })
        }
    };
    let clock = clock.split('.').next()?;
    let mut hms = clock.splitn(3, ':');
    let hour: i64 = hms.next()?.parse().ok()?;
    let minute: i64 = hms.next()?.parse().ok()?;
    let second: i64 = hms.next()?.parse().ok()?;
    let valid = (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second <= 60;
    if !valid {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3600 + minute * 60 + second - offset)
}

/// Days from 1970-01-01 to a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_index = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Longest a forge CLI may run before [`run_cli`] kills it.
const CLI_TIME_LIMIT: Duration = Duration::from_secs(20);

/// How long [`open_in_browser`] waits for a platform opener's exit code.
const OPENER_EXIT_WINDOW: Duration = Duration::from_secs(3);

/// Poll `child` until it exits, `deadline` passes, or `cancel` is set.
/// `None` on timeout, on cancel, or when its state cannot be read.
fn wait_until(child: &mut Child, deadline: Instant, cancel: &AtomicBool) -> Option<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Some(status),
            Ok(None) => {}
            Err(_) => return None,
        }
        let now = Instant::now();
        if now >= deadline || cancel.load(Ordering::Relaxed) {
            return None;
        }
        std::thread::sleep((deadline - now).min(Duration::from_millis(25)));
    }
}

/// Wait for `child` on a detached thread so it does not stay a zombie.
fn reap_in_background(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

/// Run a forge CLI and return its stdout.
///
/// `Err` when it cannot start, exits non-zero, or runs longer than 20
/// seconds (it is then killed); `Err` at once when `cancel` is set, before
/// the start or while it runs (it is then killed). The error holds the
/// reason when there is one: `<program> not found` (for example `gh not
/// found`) when the program is missing, else the first non-blank stderr
/// line of a non-zero exit, trimmed, without escape sequences, and at most
/// 200 characters ([`first_stderr_line`]). A time limit, a cancel, or a
/// silent failure has no reason.
///
/// Stdin is null and stderr is captured, so the CLI cannot read the TUI's
/// terminal or print over it. `GH_PROMPT_DISABLED` and `NO_PROMPT` (read by
/// `glab`) make the CLIs fail instead of asking for input; `NO_COLOR` keeps
/// escape codes out of the JSON. No token is passed; the CLIs use their own
/// login. The time limit keeps a stalled CLI from holding a worker; the
/// cancel flag keeps a running one from holding the TUI's exit.
pub fn run_cli(argv: &[String], cancel: &AtomicBool) -> Result<String, Option<String>> {
    run_cli_within(argv, CLI_TIME_LIMIT, cancel)
}

/// Read `pipe` to its end on its own thread, so a full pipe cannot stall
/// the child. The receiver gets the bytes once the pipe closes.
fn read_on_thread(mut pipe: impl Read + Send + 'static) -> mpsc::Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        if pipe.read_to_end(&mut bytes).is_ok() {
            let _ = tx.send(bytes);
        }
    });
    rx
}

/// Longest failure reason [`first_stderr_line`] keeps, in characters.
const STDERR_LINE_MAX: usize = 200;

/// `line` without CSI escape sequences (`ESC [`, parameter and
/// intermediate bytes, one final byte in `@`..`~`), with every other
/// control character as a space, so the text cannot drive a terminal.
fn terminal_safe(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            for byte in chars.by_ref() {
                if ('@'..='~').contains(&byte) {
                    break;
                }
            }
            continue;
        }
        out.push(if c.is_control() { ' ' } else { c });
    }
    out
}

/// First non-blank line of `stderr`: [`terminal_safe`], trimmed, and cut to
/// [`STDERR_LINE_MAX`] characters with a trailing `…` when longer.
fn first_stderr_line(stderr: &[u8]) -> Option<String> {
    let line = String::from_utf8_lossy(stderr)
        .lines()
        .map(|line| terminal_safe(line).trim().to_string())
        .find(|line| !line.is_empty())?;
    if line.chars().count() <= STDERR_LINE_MAX {
        return Some(line);
    }
    let mut cut: String = line.chars().take(STDERR_LINE_MAX - 1).collect();
    cut.push('…');
    Some(cut)
}

fn run_cli_within(
    argv: &[String],
    limit: Duration,
    cancel: &AtomicBool,
) -> Result<String, Option<String>> {
    if cancel.load(Ordering::Relaxed) {
        return Err(None);
    }
    let deadline = Instant::now() + limit;
    let (program, args) = argv.split_first().ok_or(None)?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_PROMPT", "1")
        .env("NO_COLOR", "1")
        .spawn()
        .map_err(|error| {
            (error.kind() == std::io::ErrorKind::NotFound).then(|| format!("{program} not found"))
        })?;
    let stdout = read_on_thread(child.stdout.take().ok_or(None)?);
    let stderr = read_on_thread(child.stderr.take().ok_or(None)?);
    let Some(status) = wait_until(&mut child, deadline, cancel) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(None);
    };
    // A grandchild that keeps a pipe open must not stall us either.
    let grace = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(200));
    if !status.success() {
        let stderr = stderr.recv_timeout(grace).unwrap_or_default();
        return Err(first_stderr_line(&stderr));
    }
    let bytes = stdout.recv_timeout(grace).map_err(|_| None)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// The command that opens a URL, and how to read its outcome.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BrowserCommand {
    /// Argv, program first.
    pub argv: Vec<String>,
    /// True for a platform opener (`open`, `cmd /c start`, `xdg-open`): it
    /// exits once the browser has the URL, so its exit code is the outcome.
    /// False for `$BROWSER`, which may be the browser itself: a successful
    /// start is the outcome.
    pub check_exit: bool,
}

/// The command that opens `url` in the operator's browser.
///
/// `browser_env` is `$BROWSER`: its first non-empty `:`-separated entry wins,
/// split on whitespace, with `%s` replaced by `url` or `url` appended.
/// Otherwise `os` (a `std::env::consts::OS` value) picks the platform opener:
/// `open` on macOS, `cmd /c start "" <url>` on Windows, and `xdg-open`
/// elsewhere.
pub fn browser_command(url: &str, browser_env: Option<&str>, os: &str) -> BrowserCommand {
    let entry = browser_env.and_then(|env| {
        env.split(':')
            .map(str::trim)
            .find(|entry| !entry.is_empty())
    });
    if let Some(entry) = entry {
        let mut argv: Vec<String> = entry.split_whitespace().map(str::to_string).collect();
        if argv.iter().any(|token| token.contains("%s")) {
            for token in &mut argv {
                *token = token.replace("%s", url);
            }
        } else {
            argv.push(url.to_string());
        }
        return BrowserCommand {
            argv,
            check_exit: false,
        };
    }
    let argv: &[&str] = match os {
        "macos" => &["open", url],
        // `start` takes the first quoted argument as a window title, so pass
        // an empty one; Windows quotes an empty argument as `""`.
        "windows" => &["cmd", "/c", "start", "", url],
        _ => &["xdg-open", url],
    };
    BrowserCommand {
        argv: argv.iter().map(|s| s.to_string()).collect(),
        check_exit: true,
    }
}

/// Characters `cmd.exe` reads as syntax even inside one argument (`&` and
/// `|` chain commands, `%` expands variables). The Windows opener runs
/// through `cmd /c start`, and no forge PR URL needs them, so every opener
/// refuses them.
const CMD_SPECIAL: &[char] = &['&', '|', '^', '%', '<', '>', '"'];

/// True for an `http://` or `https://` URL with no whitespace, control
/// characters, or [`CMD_SPECIAL`] characters: the only kind handed to a
/// browser command.
pub(crate) fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("https://") || lower.starts_with("http://"))
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || CMD_SPECIAL.contains(&c))
}

/// Open `url` in the operator's browser, as [`browser_command`] picks it from
/// `$BROWSER` and the OS.
///
/// `Err` without starting anything when `url` is not `http(s)://`. Blocks up
/// to 3 seconds, so call it off the TTY event thread. The child gets null
/// stdio and, on Unix, its own process group, so it never reads, writes, or
/// gets signals from the TUI's terminal. A platform opener that exits
/// non-zero within that window is `Err`; one still running after it is `Ok`
/// (some `xdg-open` setups run until the browser closes). A `$BROWSER`
/// command is `Ok` once it starts. A detached thread reaps any child still
/// running, so none stays a zombie. When `cancel` is set, nothing starts,
/// or the wait for a platform opener stops at once with `Err`.
pub fn open_in_browser(url: &str, cancel: &AtomicBool) -> Result<(), ()> {
    if !is_web_url(url) {
        return Err(());
    }
    let browser_env = std::env::var("BROWSER").ok();
    let command = browser_command(url, browser_env.as_deref(), std::env::consts::OS);
    spawn_opener(&command, OPENER_EXIT_WINDOW, cancel)
}

/// Start `command` detached from the terminal. When it has `check_exit`,
/// wait up to `window` for its exit code, or until `cancel` is set (`Err`).
fn spawn_opener(command: &BrowserCommand, window: Duration, cancel: &AtomicBool) -> Result<(), ()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(());
    }
    let (program, args) = command.argv.split_first().ok_or(())?;
    let mut cmd = Command::new(program);
    cmd.args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(|_| ())?;
    if command.check_exit {
        if let Some(status) = wait_until(&mut child, Instant::now() + window, cancel) {
            return if status.success() { Ok(()) } else { Err(()) };
        }
    }
    reap_in_background(child);
    // No one paints the outcome after a quit; report the open as unconfirmed.
    if command.check_exit && cancel.load(Ordering::Relaxed) {
        return Err(());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    use super::super::pr_status::{CiLinkKind, LONG_LIVED_BRANCHES};

    fn repo(forge: Forge, host: &str, path: &str) -> ForgeRepo {
        ForgeRepo {
            forge,
            host: host.to_string(),
            path: path.to_string(),
        }
    }

    fn strings(argv: &[&str]) -> Vec<String> {
        argv.iter().map(|s| s.to_string()).collect()
    }

    // ---- parse_remote ----

    #[test]
    fn parse_remote_scp_ssh_and_https_forms() {
        let gh = Some(repo(Forge::GitHub, "github.com", "octo/demo"));
        for url in [
            "git@github.com:octo/demo.git",
            "git@github.com:octo/demo",
            "github.com:octo/demo.git",
            "ssh://git@github.com/octo/demo.git",
            "ssh://git@github.com:22/octo/demo.git",
            "https://github.com/octo/demo.git",
            "https://github.com/octo/demo",
            "https://github.com/octo/demo/",
            "http://github.com/octo/demo.git",
            "https://user@GitHub.com/octo/demo.git",
            "  https://github.com/octo/demo.git\n",
        ] {
            assert_eq!(parse_remote(url), gh, "{url}");
        }
    }

    #[test]
    fn parse_remote_ssh_port_is_dropped_https_port_kept() {
        assert_eq!(
            parse_remote("ssh://git@gitlab.example.com:2222/group/demo.git"),
            Some(repo(Forge::GitLab, "gitlab.example.com", "group/demo"))
        );
        assert_eq!(
            parse_remote("https://gitlab.example.com:8443/group/demo.git"),
            Some(repo(Forge::GitLab, "gitlab.example.com:8443", "group/demo"))
        );
    }

    #[test]
    fn parse_remote_gitlab_subgroups_and_enterprise_github() {
        assert_eq!(
            parse_remote("git@gitlab.com:group/sub/team/demo.git"),
            Some(repo(Forge::GitLab, "gitlab.com", "group/sub/team/demo"))
        );
        assert_eq!(
            parse_remote("https://gitlab.com/group/sub/demo/"),
            Some(repo(Forge::GitLab, "gitlab.com", "group/sub/demo"))
        );
        assert_eq!(
            parse_remote("git@github.example.com:octo/demo.git"),
            Some(repo(Forge::GitHub, "github.example.com", "octo/demo"))
        );
    }

    #[test]
    fn commit_url_for_github_and_gitlab_subgroups() {
        const SHA: &str = "0123456789abcdef0123456789abcdef01234567";
        let github = parse_remote("git@github.com:example/demo.git").unwrap();
        assert_eq!(
            commit_url(&github, SHA),
            format!("https://github.com/example/demo/commit/{SHA}")
        );
        let gitlab = parse_remote("https://gitlab.com/group/sub/demo.git").unwrap();
        assert_eq!(
            commit_url(&gitlab, SHA),
            format!("https://gitlab.com/group/sub/demo/-/commit/{SHA}")
        );
    }

    #[test]
    fn parse_remote_unsupported_forge_and_garbage_are_none() {
        for url in [
            "git@bitbucket.org:octo/demo.git",
            "https://codeberg.org/octo/demo.git",
            "https://github.com/octo/demo/extra",
            "https://github.com/octo",
            "git@github.com:demo.git",
            "/srv/git/demo.git",
            "../demo",
            "file:///srv/github/octo/demo.git",
            "C:\\src\\github\\demo",
            "",
            "not a url",
            "https://github.com//demo",
        ] {
            assert_eq!(parse_remote(url), None, "{url:?}");
        }
    }

    // ---- argv ----

    #[test]
    fn github_argv_names_repo_branch_and_fields() {
        let r = repo(Forge::GitHub, "github.com", "octo/demo");
        assert_eq!(
            github_list_argv(&r, "feature/login"),
            strings(&[
                "gh",
                "pr",
                "list",
                "-R",
                "github.com/octo/demo",
                "--head",
                "feature/login",
                "--state",
                "all",
                "--json",
                "number,state,url,title,headRefName,baseRefName,isCrossRepository,isDraft,\
                 reviewDecision,mergeStateStatus,statusCheckRollup,updatedAt",
                "--limit",
                "30",
            ])
        );
        assert_eq!(
            github_view_argv(&r, 7),
            strings(&[
                "gh",
                "pr",
                "view",
                "7",
                "-R",
                "github.com/octo/demo",
                "--json",
                "number,url,title,state,isDraft,author,headRefName,baseRefName,\
                 reviewDecision,mergeStateStatus,statusCheckRollup,latestReviews,updatedAt",
            ])
        );
    }

    #[test]
    fn gitlab_argv_encodes_subgroup_path_and_branch() {
        let r = repo(Forge::GitLab, "gitlab.example.com", "group/sub/demo");
        let api = |endpoint: &str| {
            strings(&["glab", "api", "--hostname", "gitlab.example.com", endpoint])
        };
        assert_eq!(
            gitlab_list_argv(&r, "feature/a b+c"),
            api("projects/group%2Fsub%2Fdemo/merge_requests\
                 ?source_branch=feature%2Fa%20b%2Bc&state=all&per_page=30")
        );
        assert_eq!(
            gitlab_mr_argv(&r, 42),
            api("projects/group%2Fsub%2Fdemo/merge_requests/42")
        );
        assert_eq!(
            gitlab_approvals_argv(&r, 42),
            api("projects/group%2Fsub%2Fdemo/merge_requests/42/approvals")
        );
        assert_eq!(
            gitlab_jobs_argv(&r, 99),
            api("projects/group%2Fsub%2Fdemo/pipelines/99/jobs?scope[]=failed&scope[]=running")
        );
    }

    #[test]
    fn percent_encode_keeps_unreserved_only() {
        assert_eq!(percent_encode("a-Z_0.9~"), "a-Z_0.9~");
        assert_eq!(percent_encode("a/b&c=d#é"), "a%2Fb%26c%3Dd%23%C3%A9");
    }

    // ---- fixtures in the real CLI JSON shape ----

    /// `object` (JSON) with each `(key, raw JSON value)` of `fields` set.
    fn json_with(object: &str, fields: &[(&str, &str)]) -> String {
        let mut map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(object).expect("JSON object");
        for (key, raw) in fields {
            let value = serde_json::from_str(raw).unwrap_or_else(|_| panic!("JSON value {raw}"));
            map.insert(key.to_string(), value);
        }
        serde_json::Value::Object(map).to_string()
    }

    /// `object` (JSON) without `key`, as an older server prints it.
    fn json_without(object: &str, key: &str) -> String {
        let mut map: serde_json::Map<String, serde_json::Value> =
            serde_json::from_str(object).expect("JSON object");
        map.remove(key);
        serde_json::Value::Object(map).to_string()
    }

    fn slug(name: &str) -> String {
        name.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }

    /// A `statusCheckRollup` check run as `gh` prints it.
    fn check_run(name: &str, status: &str, conclusion: &str) -> String {
        format!(
            r#"{{"__typename":"CheckRun","name":"{name}","workflowName":"CI","status":"{status}","conclusion":"{conclusion}","startedAt":"2026-01-02T00:00:00Z","completedAt":"2026-01-02T00:05:00Z","detailsUrl":"https://github.com/octo/demo/actions/runs/1/job/{}"}}"#,
            slug(name)
        )
    }

    /// A `statusCheckRollup` commit status context as `gh` prints it.
    fn status_context(context: &str, state: &str) -> String {
        format!(
            r#"{{"__typename":"StatusContext","context":"{context}","state":"{state}","startedAt":"2026-01-02T00:00:00Z","targetUrl":"https://ci.example.com/{}"}}"#,
            slug(context)
        )
    }

    fn rollup(checks: &[String]) -> String {
        format!("[{}]", checks.join(","))
    }

    fn details_url(name: &str) -> String {
        format!(
            "https://github.com/octo/demo/actions/runs/1/job/{}",
            slug(name)
        )
    }

    // ---- lookup: GitHub ----

    const GH_REMOTE: &str = "git@github.com:octo/demo.git";
    const BRANCH: &str = "feature/login";

    fn gh_url(number: u64) -> String {
        format!("https://github.com/octo/demo/pull/{number}")
    }

    /// One `gh pr list --json` entry. The merge state is `UNKNOWN` (still
    /// computing) and the rollup is empty, so `decision` alone decides.
    fn gh_pr(number: u64, state: &str, head: &str, decision: &str, updated: &str) -> String {
        format!(
            r#"{{"number":{number},"state":"{state}","url":"{}","title":"Add login","headRefName":"{head}","baseRefName":"main","isCrossRepository":false,"isDraft":false,"reviewDecision":"{decision}","mergeStateStatus":"UNKNOWN","statusCheckRollup":[],"updatedAt":"{updated}"}}"#,
            gh_url(number)
        )
    }

    /// Open PR 7 on [`BRANCH`], review required, with `fields` set.
    fn gh_open(fields: &[(&str, &str)]) -> String {
        json_with(
            &gh_pr(7, "OPEN", BRANCH, "REVIEW_REQUIRED", "2026-01-02T00:00:00Z"),
            fields,
        )
    }

    fn gh_lookup(prs: &[String]) -> PrLookup {
        let json = format!("[{}]", prs.join(","));
        let calls = RefCell::new(Vec::new());
        let result = lookup(Some(GH_REMOTE), BRANCH, &|argv: &[String]| {
            calls.borrow_mut().push(argv.to_vec());
            Ok(json.clone())
        });
        let r = repo(Forge::GitHub, "github.com", "octo/demo");
        assert_eq!(calls.into_inner(), vec![github_list_argv(&r, BRANCH)]);
        result
    }

    fn found_pr(result: PrLookup) -> PullRequest {
        match result {
            PrLookup::Found(pr) => pr,
            other => panic!("a PR expected: {other:?}"),
        }
    }

    /// Number, URL, and state of a found PR.
    fn picked(result: PrLookup) -> (u64, String, PrState) {
        let pr = found_pr(result);
        (pr.number, pr.url, pr.state)
    }

    #[test]
    fn github_open_pr_reads_every_list_field() {
        let entry = gh_open(&[
            (
                "statusCheckRollup",
                &rollup(&[check_run("lint", "COMPLETED", "SUCCESS")]),
            ),
            ("updatedAt", r#""2026-01-02T00:00:00Z""#),
        ]);
        assert_eq!(
            found_pr(gh_lookup(&[entry])),
            PullRequest {
                number: 7,
                url: gh_url(7),
                forge: Forge::GitHub,
                title: "Add login".into(),
                head: BRANCH.into(),
                base: "main".into(),
                state: PrState::InReview,
                notes: PrNotes::default(),
                checks: ChecksSummary {
                    pass: 1,
                    fail: 0,
                    pending: 0
                },
                failed_check: None,
                ci_link: CiLink {
                    url: details_url("lint"),
                    kind: CiLinkKind::Checks,
                },
                updated_at: Some(1_767_312_000),
                merge_state_settled: false,
            }
        );
    }

    #[test]
    fn github_state_ladder_rows() {
        let failing = |conclusion: &str| rollup(&[check_run("test", "COMPLETED", conclusion)]);
        let mut cases: Vec<(String, String, PrState)> = Vec::new();
        let mut row = |what: String, entry: String, want: PrState| cases.push((what, entry, want));
        row(
            "merged, even with a failed check".into(),
            json_with(
                &gh_pr(7, "MERGED", BRANCH, "APPROVED", "2026-01-02T00:00:00Z"),
                &[("statusCheckRollup", &failing("FAILURE"))],
            ),
            PrState::Merged,
        );
        for conclusion in [
            "FAILURE",
            "TIMED_OUT",
            "CANCELLED",
            "ACTION_REQUIRED",
            "STARTUP_FAILURE",
            "ERROR",
        ] {
            row(
                format!("conclusion {conclusion}"),
                gh_open(&[("statusCheckRollup", &failing(conclusion))]),
                PrState::CiFailed,
            );
        }
        for state in ["FAILURE", "ERROR"] {
            row(
                format!("status context {state}"),
                gh_open(&[(
                    "statusCheckRollup",
                    &rollup(&[status_context("ci/jenkins", state)]),
                )]),
                PrState::CiFailed,
            );
        }
        let failed_and_running = rollup(&[
            check_run("build", "IN_PROGRESS", ""),
            check_run("test", "COMPLETED", "FAILURE"),
        ]);
        row(
            "CI failed beats draft and running".into(),
            gh_open(&[
                ("isDraft", "true"),
                ("statusCheckRollup", &failed_and_running),
            ]),
            PrState::CiFailed,
        );
        let running = rollup(&[check_run("build", "IN_PROGRESS", "")]);
        row(
            "draft beats CI running".into(),
            gh_open(&[("isDraft", "true"), ("statusCheckRollup", &running)]),
            PrState::Draft,
        );
        for status in ["IN_PROGRESS", "QUEUED", "WAITING", "PENDING", "REQUESTED"] {
            row(
                format!("check run {status}"),
                gh_open(&[(
                    "statusCheckRollup",
                    &rollup(&[check_run("build", status, "")]),
                )]),
                PrState::CiRunning,
            );
        }
        for state in ["PENDING", "EXPECTED"] {
            row(
                format!("status context {state}"),
                gh_open(&[(
                    "statusCheckRollup",
                    &rollup(&[status_context("ci/jenkins", state)]),
                )]),
                PrState::CiRunning,
            );
        }
        row(
            "approved but running: CI running, not ready".into(),
            gh_open(&[
                ("reviewDecision", r#""APPROVED""#),
                ("mergeStateStatus", r#""BEHIND""#),
                ("statusCheckRollup", &running),
            ]),
            PrState::CiRunning,
        );
        row(
            "clean but running: CI running, not ready".into(),
            gh_open(&[
                ("mergeStateStatus", r#""CLEAN""#),
                ("statusCheckRollup", &running),
            ]),
            PrState::CiRunning,
        );
        row(
            "clean".into(),
            gh_open(&[("mergeStateStatus", r#""CLEAN""#)]),
            PrState::Ready,
        );
        for merge_state in ["UNKNOWN", "UNSTABLE", "HAS_HOOKS"] {
            row(
                format!("approved, {merge_state}"),
                gh_open(&[
                    ("reviewDecision", r#""APPROVED""#),
                    ("mergeStateStatus", &format!(r#""{merge_state}""#)),
                ]),
                PrState::Ready,
            );
        }
        for merge_state in ["BLOCKED", "DIRTY", "BEHIND"] {
            row(
                format!("approved but {merge_state}"),
                gh_open(&[
                    ("reviewDecision", r#""APPROVED""#),
                    ("mergeStateStatus", &format!(r#""{merge_state}""#)),
                ]),
                PrState::InReview,
            );
        }
        row(
            "approved, behind, a failed check: CI failed".into(),
            gh_open(&[
                ("reviewDecision", r#""APPROVED""#),
                ("mergeStateStatus", r#""BEHIND""#),
                ("statusCheckRollup", &failing("FAILURE")),
            ]),
            PrState::CiFailed,
        );
        let passed = rollup(&[
            check_run("lint", "COMPLETED", "SUCCESS"),
            check_run("docs", "COMPLETED", "SKIPPED"),
            check_run("bench", "COMPLETED", "NEUTRAL"),
            check_run("old", "COMPLETED", "STALE"),
            status_context("ci/jenkins", "SUCCESS"),
        ]);
        row(
            "finished checks, review required".into(),
            gh_open(&[
                ("mergeStateStatus", r#""BLOCKED""#),
                ("statusCheckRollup", &passed),
            ]),
            PrState::InReview,
        );
        row(
            "no decision, no checks".into(),
            gh_open(&[
                ("reviewDecision", "null"),
                ("mergeStateStatus", "null"),
                ("statusCheckRollup", "null"),
            ]),
            PrState::InReview,
        );
        for (what, entry, want) in cases {
            assert_eq!(found_pr(gh_lookup(&[entry])).state, want, "{what}");
        }
    }

    /// Next action text and needs-you of a found PR, with no stack.
    fn next(pr: &PullRequest) -> (Option<String>, bool) {
        (
            pr.next_action(None).map(|action| action.text()),
            pr.needs_you(),
        )
    }

    #[test]
    fn github_next_action_rows() {
        let fail = rollup(&[check_run("test", "COMPLETED", "FAILURE")]);
        let run = rollup(&[check_run("build", "IN_PROGRESS", "")]);
        let cases = [
            (
                gh_open(&[("statusCheckRollup", &fail)]),
                Some("fix CI"),
                true,
            ),
            (
                gh_open(&[
                    ("mergeStateStatus", r#""DIRTY""#),
                    ("statusCheckRollup", &run),
                ]),
                Some("fix conflicts"),
                true,
            ),
            (gh_open(&[("isDraft", "true")]), Some("undraft"), false),
            (
                gh_open(&[("reviewDecision", r#""CHANGES_REQUESTED""#)]),
                Some("address review"),
                true,
            ),
            (
                gh_open(&[("mergeStateStatus", r#""CLEAN""#)]),
                Some("merge"),
                true,
            ),
            (
                gh_open(&[
                    ("reviewDecision", r#""APPROVED""#),
                    ("mergeStateStatus", r#""BEHIND""#),
                ]),
                Some("get review"),
                false,
            ),
            (
                gh_open(&[("statusCheckRollup", &run)]),
                Some("wait for CI"),
                false,
            ),
            (gh_open(&[]), Some("get review"), false),
            (
                gh_pr(
                    7,
                    "MERGED",
                    BRANCH,
                    "CHANGES_REQUESTED",
                    "2026-01-02T00:00:00Z",
                ),
                None,
                false,
            ),
        ];
        for (entry, text, needs_you) in cases {
            let pr = found_pr(gh_lookup(std::slice::from_ref(&entry)));
            assert_eq!(next(&pr), (text.map(str::to_string), needs_you), "{entry}");
        }
    }

    #[test]
    fn github_notes_conflicts_and_changes_requested_on_open_prs_only() {
        let both = [
            ("mergeStateStatus", r#""DIRTY""#),
            ("reviewDecision", r#""CHANGES_REQUESTED""#),
        ];
        let open = found_pr(gh_lookup(&[gh_open(&both)]));
        assert_eq!(
            open.notes.texts(),
            vec!["has conflicts", "changes requested"]
        );
        let merged = json_with(
            &gh_pr(7, "MERGED", BRANCH, "", "2026-01-02T00:00:00Z"),
            &both,
        );
        assert_eq!(found_pr(gh_lookup(&[merged])).notes, PrNotes::default());
    }

    #[test]
    fn github_first_failed_check_and_ci_link_order() {
        let link = |checks: &[String]| {
            let pr = found_pr(gh_lookup(&[gh_open(&[(
                "statusCheckRollup",
                &rollup(checks),
            )])]));
            (pr.failed_check, pr.ci_link.url, pr.ci_link.kind.label())
        };
        let lint = check_run("lint", "COMPLETED", "SUCCESS");
        let build = check_run("build", "IN_PROGRESS", "");
        let test = check_run("test (ubuntu-latest)", "COMPLETED", "FAILURE");
        assert_eq!(
            link(&[lint.clone(), build.clone(), test.clone()]),
            (
                Some("test (ubuntu-latest)".into()),
                details_url("test (ubuntu-latest)"),
                "Open failed check"
            )
        );
        assert_eq!(
            link(&[status_context("ci/jenkins", "ERROR"), test]),
            (
                Some("ci/jenkins".into()),
                "https://ci.example.com/ci-jenkins".into(),
                "Open failed check"
            )
        );
        assert_eq!(
            link(&[lint.clone(), build]),
            (None, details_url("build"), "Open running check")
        );
        assert_eq!(
            link(&[lint.clone()]),
            (None, details_url("lint"), "Open checks")
        );
        assert_eq!(
            link(&[]),
            (None, format!("{}/checks", gh_url(7)), "Open checks")
        );

        // The alert names the check the link opens.
        let a_no_link = json_with(
            &check_run("a", "COMPLETED", "FAILURE"),
            &[("detailsUrl", "null")],
        );
        let b = check_run("b", "COMPLETED", "FAILURE");
        assert_eq!(
            link(&[a_no_link.clone(), b]),
            (Some("b".into()), details_url("b"), "Open failed check")
        );
        let running = check_run("build", "IN_PROGRESS", "");
        assert_eq!(
            link(&[lint, a_no_link, running]),
            (
                Some("a".into()),
                format!("{}/checks", gh_url(7)),
                "Open checks"
            ),
            "no failed check has a link: the first one's name and the checks tab"
        );
    }

    #[test]
    fn github_merged_pr_with_a_failed_check_opens_that_check() {
        let merged = json_with(
            &gh_pr(7, "MERGED", BRANCH, "APPROVED", "2026-01-02T00:00:00Z"),
            &[(
                "statusCheckRollup",
                &rollup(&[check_run("test", "COMPLETED", "FAILURE")]),
            )],
        );
        let pr = found_pr(gh_lookup(&[merged]));
        assert_eq!(pr.state, PrState::Merged);
        assert_eq!(
            (pr.ci_link.kind, pr.ci_link.url, pr.failed_check),
            (
                CiLinkKind::FailedCheck,
                details_url("test"),
                Some("test".into())
            )
        );
    }

    #[test]
    fn github_closed_unmerged_and_empty_are_no_pr() {
        let prs = [gh_pr(
            3,
            "CLOSED",
            BRANCH,
            "APPROVED",
            "2026-01-02T00:00:00Z",
        )];
        assert_eq!(gh_lookup(&prs), PrLookup::NoPr);
        assert_eq!(gh_lookup(&[]), PrLookup::NoPr);
    }

    #[test]
    fn github_open_wins_over_newer_merged_and_closed() {
        let prs = [
            gh_pr(9, "MERGED", BRANCH, "", "2026-03-01T00:00:00Z"),
            gh_pr(10, "CLOSED", BRANCH, "", "2026-04-01T00:00:00Z"),
            gh_pr(8, "OPEN", BRANCH, "", "2026-01-01T00:00:00Z"),
        ];
        assert_eq!(picked(gh_lookup(&prs)), (8, gh_url(8), PrState::InReview));
    }

    #[test]
    fn github_latest_merged_when_none_open() {
        let prs = [
            gh_pr(4, "MERGED", BRANCH, "", "2026-01-01T00:00:00Z"),
            gh_pr(6, "MERGED", BRANCH, "", "2026-02-01T00:00:00Z"),
            gh_pr(5, "MERGED", BRANCH, "", "2026-01-15T00:00:00Z"),
        ];
        assert_eq!(picked(gh_lookup(&prs)), (6, gh_url(6), PrState::Merged));
    }

    #[test]
    fn github_two_open_exact_head_then_latest_updated() {
        let prs = [
            gh_pr(11, "OPEN", "Feature/Login", "", "2026-05-01T00:00:00Z"),
            gh_pr(12, "OPEN", BRANCH, "", "2026-02-01T00:00:00Z"),
            gh_pr(13, "OPEN", BRANCH, "APPROVED", "2026-03-01T00:00:00Z"),
        ];
        assert_eq!(picked(gh_lookup(&prs)), (13, gh_url(13), PrState::Ready));
        let prs = [
            gh_pr(12, "OPEN", "other", "", "2026-02-01T00:00:00Z"),
            gh_pr(14, "OPEN", "other", "", "2026-04-01T00:00:00Z"),
        ];
        assert_eq!(picked(gh_lookup(&prs)), (14, gh_url(14), PrState::InReview));
    }

    #[test]
    fn github_fork_pr_with_the_same_branch_name_does_not_count() {
        let fork = gh_pr(15, "OPEN", BRANCH, "APPROVED", "2026-06-01T00:00:00Z").replace(
            r#""isCrossRepository":false"#,
            r#""isCrossRepository":true"#,
        );
        assert_eq!(gh_lookup(std::slice::from_ref(&fork)), PrLookup::NoPr);
        let prs = [fork, gh_pr(6, "MERGED", BRANCH, "", "2026-01-01T00:00:00Z")];
        assert_eq!(picked(gh_lookup(&prs)), (6, gh_url(6), PrState::Merged));
    }

    #[test]
    fn github_list_with_only_the_old_fields_still_parses() {
        let old = r#"[{"number":7,"state":"OPEN","url":"https://github.com/octo/demo/pull/7","headRefName":"feature/login","reviewDecision":null,"updatedAt":null}]"#;
        let pr = found_pr(lookup(Some(GH_REMOTE), BRANCH, &|_: &[String]| {
            Ok(old.to_string())
        }));
        assert_eq!(
            (pr.state, pr.base.as_str(), pr.updated_at),
            (PrState::InReview, "", None)
        );
    }

    #[test]
    fn runner_error_keeps_its_reason_and_bad_json_has_none() {
        assert_eq!(
            lookup(Some(GH_REMOTE), BRANCH, &|_: &[String]| Err(Some(
                "gh not found".into()
            ))),
            PrLookup::Failed(Some("gh not found".into()))
        );
        assert_eq!(
            lookup(Some(GH_REMOTE), BRANCH, &|_: &[String]| Err(None)),
            PrLookup::Failed(None)
        );
        for bad in [
            "",
            "not json",
            "{}",
            r#"[{"number":"x"}]"#,
            r#"[{"state":"OPEN"}]"#,
        ] {
            assert_eq!(
                lookup(Some(GH_REMOTE), BRANCH, &|_: &[String]| Ok(bad.to_string())),
                PrLookup::Failed(None),
                "{bad:?}"
            );
        }
    }

    #[test]
    fn no_remote_and_unsupported_forge_are_no_pr_without_running_a_cli() {
        let run =
            |_: &[String]| -> Result<String, Option<String>> { panic!("no CLI call expected") };
        assert_eq!(lookup(None, BRANCH, &run), PrLookup::NoPr);
        assert_eq!(
            lookup(Some("git@bitbucket.org:octo/demo.git"), BRANCH, &run),
            PrLookup::NoPr
        );
    }

    // ---- lookup: GitLab ----

    const GL_REMOTE: &str = "https://gitlab.com/group/sub/demo.git";

    fn gl_url(iid: u64) -> String {
        format!("https://gitlab.com/group/sub/demo/-/merge_requests/{iid}")
    }

    fn gl_repo() -> ForgeRepo {
        repo(Forge::GitLab, "gitlab.com", "group/sub/demo")
    }

    /// One GitLab MR list entry, same project, waiting for approval.
    fn gl_mr(iid: u64, state: &str, updated: &str) -> String {
        format!(
            r#"{{"iid":{iid},"state":"{state}","web_url":"{}","title":"Add login","source_branch":"{BRANCH}","target_branch":"main","source_project_id":7,"target_project_id":7,"draft":false,"work_in_progress":false,"detailed_merge_status":"not_approved","merge_status":"can_be_merged","has_conflicts":false,"blocking_discussions_resolved":true,"updated_at":"{updated}"}}"#,
            gl_url(iid)
        )
    }

    /// Open MR 21 with `fields` set.
    fn gl_open(fields: &[(&str, &str)]) -> String {
        json_with(&gl_mr(21, "opened", "2026-01-02T00:00:00.000Z"), fields)
    }

    /// `head_pipeline` of the single MR: pipeline 99 in `status`, or `null`
    /// for `""`.
    fn gl_pipeline(status: &str) -> String {
        if status.is_empty() {
            "null".to_string()
        } else {
            format!(
                r#"{{"id":99,"iid":5,"status":"{status}","web_url":"https://gitlab.com/group/sub/demo/-/pipelines/99"}}"#
            )
        }
    }

    /// The single MR answer for list `entry`: the same fields (GitLab
    /// sends both), plus the author and the head pipeline in `pipeline`
    /// status.
    fn gl_view_of(entry: &str, pipeline: &str) -> String {
        json_with(
            entry,
            &[
                ("author", r#"{"username":"octo-user"}"#),
                ("head_pipeline", &gl_pipeline(pipeline)),
            ],
        )
    }

    /// The single MR 21 as `glab api` prints it, with head pipeline 99 in
    /// `pipeline` status (`""` for no pipeline).
    fn gl_view(state: &str, pipeline: &str) -> String {
        gl_view_of(&gl_mr(21, state, "2026-03-04T13:06:07.000+08:00"), pipeline)
    }

    /// Which GitLab endpoint `argv` calls.
    fn gl_endpoint(argv: &[String]) -> &'static str {
        let endpoint = argv.last().expect("endpoint");
        if endpoint.contains("/jobs?") {
            "jobs"
        } else if endpoint.ends_with("/approvals") {
            "approvals"
        } else if endpoint.contains("merge_requests?") {
            "list"
        } else {
            "mr"
        }
    }

    type Answer = Result<String, Option<String>>;

    /// A fake `glab api`: one answer per endpoint.
    struct GlForge {
        list: Answer,
        /// The single MR object the MR call answers, unless `mr` is set.
        view: String,
        /// An MR call answer that replaces `view` (a failure, bad JSON).
        mr: Option<Answer>,
        approvals: Answer,
        jobs: Answer,
    }

    impl GlForge {
        /// List `mrs`. The MR call answers the first of them (else open MR
        /// 21) with the same fields and no pipeline. The approvals and jobs
        /// calls fail.
        fn new(mrs: &[String]) -> Self {
            let first = mrs
                .first()
                .cloned()
                .unwrap_or_else(|| gl_mr(21, "opened", "2026-03-04T13:06:07.000+08:00"));
            Self {
                list: Ok(format!("[{}]", mrs.join(","))),
                view: gl_view_of(&first, ""),
                mr: None,
                approvals: Err(None),
                jobs: Err(None),
            }
        }

        fn mr(self, mr: Answer) -> Self {
            Self {
                mr: Some(mr),
                ..self
            }
        }

        /// The MR call answers `fields` on top of the list's.
        fn mr_fields(self, fields: &[(&str, &str)]) -> Self {
            Self {
                view: json_with(&self.view, fields),
                ..self
            }
        }

        /// The MR call leaves `key` out.
        fn mr_without(self, key: &str) -> Self {
            Self {
                view: json_without(&self.view, key),
                ..self
            }
        }

        fn pipeline(self, status: &str) -> Self {
            self.mr_fields(&[("head_pipeline", &gl_pipeline(status))])
        }

        fn approvals(self, json: &str) -> Self {
            Self {
                approvals: Ok(json.into()),
                ..self
            }
        }

        fn jobs(self, jobs: Answer) -> Self {
            Self { jobs, ..self }
        }

        fn run(&self, calls: &RefCell<Vec<Vec<String>>>, argv: &[String]) -> Answer {
            calls.borrow_mut().push(argv.to_vec());
            match gl_endpoint(argv) {
                "list" => self.list.clone(),
                "mr" => self.mr.clone().unwrap_or_else(|| Ok(self.view.clone())),
                "approvals" => self.approvals.clone(),
                _ => self.jobs.clone(),
            }
        }

        /// Badge lookup of [`BRANCH`], and the endpoints it called.
        fn lookup(&self) -> (PrLookup, Vec<&'static str>) {
            let calls = RefCell::new(Vec::new());
            let result = lookup(Some(GL_REMOTE), BRANCH, &|argv: &[String]| {
                self.run(&calls, argv)
            });
            (
                result,
                calls
                    .into_inner()
                    .iter()
                    .map(|argv| gl_endpoint(argv))
                    .collect(),
            )
        }

        /// The PR the badge lookup found.
        fn found(&self) -> PullRequest {
            found_pr(self.lookup().0)
        }

        /// Detail fetch of MR 21, and the argv of every call.
        fn detail(&self) -> (PrDetailLookup, Vec<Vec<String>>) {
            let calls = RefCell::new(Vec::new());
            let result = lookup_detail(GL_REMOTE, 21, &|argv: &[String]| self.run(&calls, argv));
            (result, calls.into_inner())
        }
    }

    #[test]
    fn gitlab_open_mr_reads_the_list_and_the_head_pipeline() {
        let r = gl_repo();
        let calls = RefCell::new(Vec::new());
        let forge = GlForge::new(&[gl_open(&[])]).pipeline("failed");
        let result = lookup(Some(GL_REMOTE), BRANCH, &|argv: &[String]| {
            forge.run(&calls, argv)
        });
        assert_eq!(
            calls.into_inner(),
            vec![gitlab_list_argv(&r, BRANCH), gitlab_mr_argv(&r, 21)],
            "no approvals call when detailed_merge_status is sent"
        );
        assert_eq!(
            found_pr(result),
            PullRequest {
                number: 21,
                url: gl_url(21),
                forge: Forge::GitLab,
                title: "Add login".into(),
                head: BRANCH.into(),
                base: "main".into(),
                state: PrState::CiFailed,
                notes: PrNotes::default(),
                checks: ChecksSummary {
                    pass: 0,
                    fail: 1,
                    pending: 0
                },
                failed_check: None,
                ci_link: CiLink {
                    url: "https://gitlab.com/group/sub/demo/-/pipelines/99".into(),
                    kind: CiLinkKind::Pipeline,
                },
                updated_at: Some(1_767_312_000),
                merge_state_settled: true,
            }
        );
    }

    #[test]
    fn gitlab_state_ladder_rows() {
        let mut cases: Vec<(String, GlForge, PrState)> = Vec::new();
        let mut row = |what: String, forge: GlForge, want: PrState| cases.push((what, forge, want));
        row(
            "merged".into(),
            GlForge::new(&[gl_mr(20, "merged", "2026-03-01T00:00:00.000Z")]).pipeline("failed"),
            PrState::Merged,
        );
        for status in ["failed", "canceled"] {
            row(
                format!("pipeline {status}"),
                GlForge::new(&[gl_open(&[("draft", "true")])]).pipeline(status),
                PrState::CiFailed,
            );
        }
        row(
            "draft beats CI running".into(),
            GlForge::new(&[gl_open(&[("draft", "true")])]).pipeline("running"),
            PrState::Draft,
        );
        row(
            "older work_in_progress flag".into(),
            GlForge::new(&[json_without(
                &gl_open(&[("work_in_progress", "true")]),
                "draft",
            )]),
            PrState::Draft,
        );
        for status in [
            "created",
            "waiting_for_resource",
            "preparing",
            "pending",
            "running",
            "scheduled",
        ] {
            row(
                format!("pipeline {status}, mergeable: CI running, not ready"),
                GlForge::new(&[gl_open(&[("detailed_merge_status", r#""mergeable""#)])])
                    .pipeline(status),
                PrState::CiRunning,
            );
        }
        for status in ["manual", "success", "skipped", ""] {
            row(
                format!("pipeline {status:?} has no CI state: mergeable"),
                GlForge::new(&[gl_open(&[("detailed_merge_status", r#""mergeable""#)])])
                    .pipeline(status),
                PrState::Ready,
            );
            row(
                format!("pipeline {status:?} has no CI state: not approved"),
                GlForge::new(&[gl_open(&[])]).pipeline(status),
                PrState::InReview,
            );
        }
        for detailed in [
            "not_approved",
            "discussions_not_resolved",
            "ci_still_running",
            "unchecked",
        ] {
            row(
                format!("detailed {detailed}"),
                GlForge::new(&[gl_open(&[(
                    "detailed_merge_status",
                    &format!(r#""{detailed}""#),
                )])]),
                PrState::InReview,
            );
        }
        for (what, forge, want) in cases {
            assert_eq!(forge.found().state, want, "{what}");
        }
    }

    #[test]
    fn merge_state_settled_is_false_while_the_forge_still_computes_it() {
        let gh = |fields: &[(&str, &str)]| found_pr(gh_lookup(&[gh_open(fields)]));
        assert!(!gh(&[]).merge_state_settled, "GitHub UNKNOWN");
        for status in [r#""CLEAN""#, r#""BLOCKED""#, r#""BEHIND""#, "null"] {
            assert!(
                gh(&[("mergeStateStatus", status)]).merge_state_settled,
                "GitHub {status}"
            );
        }
        let merged = gh_pr(7, "MERGED", BRANCH, "APPROVED", "2026-01-02T00:00:00Z");
        assert!(
            found_pr(gh_lookup(&[merged])).merge_state_settled,
            "a merged GitHub PR is settled even with UNKNOWN"
        );

        let gl = |fields: &[(&str, &str)]| GlForge::new(&[gl_open(fields)]).found();
        for status in ["checking", "unchecked", "approvals_syncing", "preparing"] {
            let pr = gl(&[("detailed_merge_status", &format!(r#""{status}""#))]);
            assert!(!pr.merge_state_settled, "GitLab {status}");
        }
        for status in ["mergeable", "not_approved", "conflict", "ci_still_running"] {
            let pr = gl(&[("detailed_merge_status", &format!(r#""{status}""#))]);
            assert!(pr.merge_state_settled, "GitLab {status}");
        }
        for (merge_status, settled) in [
            ("unchecked", false),
            ("checking", false),
            ("cannot_be_merged_recheck", false),
            ("can_be_merged", true),
            ("cannot_be_merged", true),
        ] {
            let pr = gl(&[
                ("detailed_merge_status", "null"),
                ("merge_status", &format!(r#""{merge_status}""#)),
            ]);
            assert_eq!(
                pr.merge_state_settled, settled,
                "older GitLab {merge_status}"
            );
        }
        // The MR call is fresher than the list: its status decides.
        let pr = GlForge::new(&[gl_open(&[("detailed_merge_status", r#""unchecked""#)])])
            .mr_fields(&[("detailed_merge_status", r#""mergeable""#)])
            .found();
        assert!(
            pr.merge_state_settled,
            "list unchecked, MR call mergeable: settled, so no flapping"
        );
        assert_eq!(pr.state, PrState::Ready);
        let pr = GlForge::new(&[gl_open(&[("detailed_merge_status", r#""mergeable""#)])])
            .mr_fields(&[("detailed_merge_status", r#""checking""#)])
            .found();
        assert!(
            !pr.merge_state_settled,
            "list mergeable, MR call checking: not settled"
        );
        let merged = json_with(
            &gl_mr(20, "merged", "2026-03-01T00:00:00.000Z"),
            &[("detailed_merge_status", r#""checking""#)],
        );
        assert!(
            GlForge::new(&[merged]).found().merge_state_settled,
            "a merged GitLab MR is settled"
        );
    }

    #[test]
    fn gitlab_missing_detailed_merge_status_falls_back_to_merge_status_and_approvals() {
        let old = |merge_status: &str| {
            json_without(
                &gl_open(&[("merge_status", &format!(r#""{merge_status}""#))]),
                "detailed_merge_status",
            )
        };
        let approvals = |left: u64| {
            format!(
                r#"{{"approved":{},"approvals_required":1,"approvals_left":{left},"approved_by":[]}}"#,
                left == 0
            )
        };
        let ready = GlForge::new(&[old("can_be_merged")]).approvals(&approvals(0));
        let (result, calls) = ready.lookup();
        assert_eq!(picked(result).2, PrState::Ready);
        assert_eq!(calls, ["list", "mr", "approvals"]);

        let cases = [
            (
                "approvals left",
                GlForge::new(&[old("can_be_merged")]).approvals(&approvals(1)),
            ),
            (
                "approvals call failed",
                GlForge::new(&[old("can_be_merged")]),
            ),
            (
                "approvals without the field",
                GlForge::new(&[old("can_be_merged")]).approvals("{}"),
            ),
            (
                "cannot be merged",
                GlForge::new(&[old("cannot_be_merged")]).approvals(&approvals(0)),
            ),
        ];
        for (what, forge) in cases {
            assert_eq!(forge.found().state, PrState::InReview, "{what}");
        }
        let null_detailed =
            GlForge::new(&[gl_open(&[("detailed_merge_status", "null")])]).approvals(&approvals(0));
        assert_eq!(
            null_detailed.lookup().1,
            ["list", "mr", "approvals"],
            "null reads as missing"
        );
        assert_eq!(null_detailed.found().state, PrState::Ready);
        let running = GlForge::new(&[old("can_be_merged")])
            .approvals(&approvals(0))
            .pipeline("running");
        assert_eq!(running.found().state, PrState::CiRunning);
    }

    #[test]
    fn gitlab_next_action_rows() {
        let cases = [
            (
                GlForge::new(&[gl_open(&[("has_conflicts", "true")])]).pipeline("failed"),
                Some("fix CI"),
                true,
            ),
            (
                GlForge::new(&[gl_open(&[("has_conflicts", "true")])]).pipeline("running"),
                Some("fix conflicts"),
                true,
            ),
            (
                GlForge::new(&[gl_open(&[("detailed_merge_status", r#""conflict""#)])]),
                Some("fix conflicts"),
                true,
            ),
            (
                GlForge::new(&[gl_open(&[("draft", "true")])]),
                Some("undraft"),
                false,
            ),
            (
                GlForge::new(&[gl_open(&[(
                    "detailed_merge_status",
                    r#""discussions_not_resolved""#,
                )])]),
                Some("address review"),
                true,
            ),
            (
                GlForge::new(&[gl_open(&[("blocking_discussions_resolved", "false")])]),
                Some("address review"),
                true,
            ),
            (
                GlForge::new(&[gl_open(&[(
                    "detailed_merge_status",
                    r#""requested_changes""#,
                )])]),
                Some("address review"),
                true,
            ),
            (
                GlForge::new(&[gl_open(&[("detailed_merge_status", r#""mergeable""#)])]),
                Some("merge"),
                true,
            ),
            (
                GlForge::new(&[gl_open(&[])]).pipeline("pending"),
                Some("wait for CI"),
                false,
            ),
            (GlForge::new(&[gl_open(&[])]), Some("get review"), false),
            (
                GlForge::new(&[json_with(
                    &gl_mr(20, "merged", "2026-01-01T00:00:00.000Z"),
                    &[("has_conflicts", "true")],
                )]),
                None,
                false,
            ),
        ];
        for (i, (forge, text, needs_you)) in cases.into_iter().enumerate() {
            assert_eq!(
                next(&forge.found()),
                (text.map(str::to_string), needs_you),
                "row {i}"
            );
        }
    }

    #[test]
    fn gitlab_notes_conflicts_threads_and_changes_requested_on_open_mrs_only() {
        let fields = [
            ("has_conflicts", "true"),
            ("blocking_discussions_resolved", "false"),
        ];
        let open = GlForge::new(&[gl_open(&fields)]).found();
        assert_eq!(
            open.notes.texts(),
            vec!["has conflicts", "unresolved threads"]
        );
        let requested = GlForge::new(&[gl_open(&[(
            "detailed_merge_status",
            r#""requested_changes""#,
        )])])
        .found();
        assert_eq!(requested.notes.texts(), vec!["changes requested"]);
        assert_eq!(requested.state, PrState::InReview);
        let merged = json_with(&gl_mr(20, "merged", "2026-01-01T00:00:00.000Z"), &fields);
        assert_eq!(GlForge::new(&[merged]).found().notes, PrNotes::default());
    }

    #[test]
    fn gitlab_open_mr_reads_merge_facts_from_the_mr_call_before_the_list() {
        let stale = GlForge::new(&[gl_open(&[("detailed_merge_status", r#""unchecked""#)])])
            .mr_fields(&[("detailed_merge_status", r#""mergeable""#)]);
        let (result, calls) = stale.lookup();
        assert_eq!(picked(result).2, PrState::Ready, "the MR call wins");
        assert_eq!(calls, ["list", "mr"], "no approvals call");

        let fresher = GlForge::new(&[gl_open(&[])]).mr_fields(&[
            ("draft", "true"),
            ("has_conflicts", "true"),
            ("blocking_discussions_resolved", "false"),
        ]);
        let pr = fresher.found();
        assert_eq!(pr.state, PrState::Draft);
        assert_eq!(
            pr.notes.texts(),
            vec!["has conflicts", "unresolved threads"]
        );

        let missing = GlForge::new(&[gl_open(&[
            ("detailed_merge_status", r#""mergeable""#),
            ("has_conflicts", "true"),
        ])])
        .mr_without("detailed_merge_status")
        .mr_without("has_conflicts");
        let (result, calls) = missing.lookup();
        let pr = found_pr(result);
        assert_eq!(
            (pr.state, pr.notes.conflicts),
            (PrState::Ready, true),
            "a field the MR call leaves out comes from the list"
        );
        assert_eq!(calls, ["list", "mr"]);

        let empty = GlForge::new(&[gl_open(&[("detailed_merge_status", r#""mergeable""#)])])
            .mr_fields(&[("detailed_merge_status", r#""""#)]);
        assert_eq!(
            empty.found().state,
            PrState::Ready,
            "an empty status is left out"
        );
    }

    #[test]
    fn gitlab_ci_link_is_the_head_pipeline_else_the_pipelines_tab() {
        let link = |forge: GlForge| {
            let pr = forge.found();
            (pr.ci_link.url, pr.ci_link.kind.label())
        };
        assert_eq!(
            link(GlForge::new(&[gl_open(&[])]).pipeline("success")),
            (
                "https://gitlab.com/group/sub/demo/-/pipelines/99".into(),
                "Open pipeline"
            )
        );
        assert_eq!(
            link(GlForge::new(&[gl_open(&[])])),
            (format!("{}/pipelines", gl_url(21)), "Open pipeline")
        );
        let merged = GlForge::new(&[gl_mr(20, "merged", "2026-01-01T00:00:00.000Z")]);
        assert_eq!(
            link(merged),
            (format!("{}/pipelines", gl_url(20)), "Open pipeline"),
            "a merged MR asks no MR call"
        );
    }

    #[test]
    fn gitlab_merged_asks_only_the_list_and_open_wins() {
        let merged = GlForge::new(&[gl_mr(20, "merged", "2026-03-01T00:00:00.000Z")]);
        let (result, calls) = merged.lookup();
        assert_eq!(picked(result), (20, gl_url(20), PrState::Merged));
        assert_eq!(calls, ["list"]);

        let mrs = [
            gl_mr(20, "merged", "2026-03-01T00:00:00.000Z"),
            gl_mr(22, "opened", "2026-01-01T00:00:00.000Z"),
            gl_mr(23, "opened", "2026-02-01T00:00:00.000Z"),
        ];
        let calls = RefCell::new(Vec::new());
        let forge = GlForge::new(&mrs);
        let result = lookup(Some(GL_REMOTE), BRANCH, &|argv: &[String]| {
            forge.run(&calls, argv)
        });
        assert_eq!(picked(result).0, 23);
        assert_eq!(calls.into_inner()[1], gitlab_mr_argv(&gl_repo(), 23));
    }

    #[test]
    fn gitlab_closed_locked_and_empty_are_no_pr() {
        let mrs = [
            gl_mr(24, "closed", "2026-01-01T00:00:00.000Z"),
            gl_mr(25, "locked", "2026-01-02T00:00:00.000Z"),
        ];
        assert_eq!(GlForge::new(&mrs).lookup(), (PrLookup::NoPr, vec!["list"]));
        assert_eq!(GlForge::new(&[]).lookup(), (PrLookup::NoPr, vec!["list"]));
    }

    #[test]
    fn gitlab_fork_mr_with_the_same_branch_name_does_not_count() {
        let fork = gl_open(&[("source_project_id", "99")]);
        let (result, calls) = GlForge::new(std::slice::from_ref(&fork)).lookup();
        assert_eq!(result, PrLookup::NoPr);
        assert_eq!(calls, ["list"], "no MR call for a fork MR");
        let mrs = [fork, gl_mr(20, "merged", "2026-01-01T00:00:00.000Z")];
        assert_eq!(
            picked(GlForge::new(&mrs).lookup().0),
            (20, gl_url(20), PrState::Merged)
        );
    }

    #[test]
    fn gitlab_list_or_mr_failure_is_failed_with_its_reason() {
        let reason = Some("glab: 404 Project Not Found (HTTP 404)".to_string());
        let forge = GlForge {
            list: Err(reason.clone()),
            ..GlForge::new(&[])
        };
        assert_eq!(forge.lookup().0, PrLookup::Failed(reason.clone()));
        let forge = GlForge {
            list: Ok(r#"{"message":"404 Project Not Found"}"#.into()),
            ..GlForge::new(&[])
        };
        assert_eq!(forge.lookup().0, PrLookup::Failed(None));
        let forge = GlForge::new(&[gl_open(&[])]).mr(Err(reason.clone()));
        assert_eq!(
            forge.lookup(),
            (PrLookup::Failed(reason), vec!["list", "mr"])
        );
        let forge = GlForge::new(&[gl_open(&[])]).mr(Ok("not json".into()));
        assert_eq!(forge.lookup().0, PrLookup::Failed(None));
    }

    // ---- ref, stack, needs you ----

    #[test]
    fn ref_text_is_hash_on_github_and_bang_on_gitlab() {
        let gh = PullRequest::fixture(318, &gh_url(318), PrState::InReview);
        let gl = PullRequest::fixture(12, &gl_url(12), PrState::InReview);
        assert_eq!(
            (gh.forge, gh.ref_text()),
            (Forge::GitHub, "#318".to_string())
        );
        assert_eq!(
            (gl.forge, gl.ref_text()),
            (Forge::GitLab, "!12".to_string())
        );
    }

    fn stacked(number: u64, url: &str, head: &str, base: &str, state: PrState) -> PullRequest {
        PullRequest {
            head: head.into(),
            base: base.into(),
            ..PullRequest::fixture(number, url, state)
        }
    }

    #[test]
    fn stack_finds_an_open_base_and_says_merge_it_first() {
        let base = stacked(
            318,
            &gh_url(318),
            "feature/ABC-123-retry",
            "main",
            PrState::CiFailed,
        );
        let top = stacked(
            319,
            &gh_url(319),
            "feature/ABC-125-retry-ui",
            "feature/ABC-123-retry",
            PrState::Ready,
        );
        let known = [base.clone(), top.clone()];
        assert_eq!(top.stacked_on(&known, None), Some(&base));
        assert_eq!(base.stacked_on(&known, None), None, "no PR has head main");
        let action = top.next_action(top.stacked_on(&known, None)).expect("open");
        assert_eq!(action.text(), "merge #318 first");
        assert!(top.needs_you());

        let gl_base = stacked(12, &gl_url(12), "feature/a", "main", PrState::InReview);
        let gl_top = stacked(13, &gl_url(13), "feature/b", "feature/a", PrState::Ready);
        let known = [gl_base, gl_top.clone()];
        assert_eq!(
            gl_top
                .next_action(gl_top.stacked_on(&known, None))
                .map(|a| a.text()),
            Some("merge !12 first".into())
        );
    }

    #[test]
    fn stack_keeps_a_merged_base_but_only_an_open_one_blocks_merge() {
        let merged = stacked(318, &gh_url(318), "feature/a", "main", PrState::Merged);
        let top = stacked(319, &gh_url(319), "feature/b", "feature/a", PrState::Ready);
        let known = [merged.clone(), top.clone()];
        assert_eq!(top.stacked_on(&known, None), Some(&merged));
        assert_eq!(
            top.next_action(top.stacked_on(&known, None))
                .map(|a| a.text()),
            Some("merge".into())
        );
    }

    #[test]
    fn stack_prefers_an_open_base_then_the_highest_number_and_skips_itself() {
        let old_merged = stacked(300, &gh_url(300), "feature/a", "main", PrState::Merged);
        let open = stacked(301, &gh_url(301), "feature/a", "main", PrState::InReview);
        let newer_merged = stacked(305, &gh_url(305), "feature/a", "main", PrState::Merged);
        let top = stacked(310, &gh_url(310), "feature/b", "feature/a", PrState::Ready);
        let known = [
            old_merged.clone(),
            open.clone(),
            newer_merged.clone(),
            top.clone(),
        ];
        assert_eq!(top.stacked_on(&known, None), Some(&open));
        assert_eq!(
            top.stacked_on([&old_merged, &newer_merged], None),
            Some(&newer_merged)
        );
        let loops = stacked(320, &gh_url(320), "feature/c", "feature/c", PrState::Ready);
        assert_eq!(loops.stacked_on([&loops], None), None);
        let no_base = stacked(321, &gh_url(321), "feature/d", "", PrState::Ready);
        let empty_head = stacked(322, &gh_url(322), "", "main", PrState::InReview);
        assert_eq!(no_base.stacked_on([&empty_head], None), None);
    }

    #[test]
    fn a_long_lived_or_default_base_is_never_a_stack() {
        for state in [PrState::InReview, PrState::Merged] {
            let release = stacked(400, &gh_url(400), "develop", "main", state);
            let feature = stacked(401, &gh_url(401), "feature/a", "develop", PrState::Ready);
            assert_eq!(
                feature.stacked_on([&release, &feature], None),
                None,
                "a {state:?} develop → main PR does not stack a PR into develop"
            );
            assert_eq!(
                feature
                    .next_action(feature.stacked_on([&release], None))
                    .map(|a| a.text()),
                Some("merge".into())
            );
        }
        for base in LONG_LIVED_BRANCHES {
            let below = stacked(410, &gl_url(410), base, "other", PrState::InReview);
            let top = stacked(411, &gl_url(411), "feature/b", base, PrState::Ready);
            assert_eq!(top.stacked_on([&below], None), None, "{base}");
        }
        let release = stacked(420, &gh_url(420), "release", "main", PrState::InReview);
        let top = stacked(421, &gh_url(421), "feature/c", "release", PrState::Ready);
        assert_eq!(
            top.stacked_on([&release], Some("release")),
            None,
            "the configured default branch"
        );
        assert_eq!(top.stacked_on([&release], Some("main")), Some(&release));
    }

    // ---- lookup_detail ----

    /// Runs a GitHub detail fetch of PR 7 that answers `json`.
    fn gh_detail(json: &str) -> PrDetailLookup {
        let calls = RefCell::new(Vec::new());
        let result = lookup_detail(GH_REMOTE, 7, &|argv: &[String]| {
            calls.borrow_mut().push(argv.to_vec());
            Ok(json.to_string())
        });
        let r = repo(Forge::GitHub, "github.com", "octo/demo");
        assert_eq!(calls.into_inner(), vec![github_view_argv(&r, 7)]);
        result
    }

    fn detail(result: PrDetailLookup) -> PullRequestDetail {
        match result {
            PrDetailLookup::Found(detail) => detail,
            PrDetailLookup::Failed(why) => panic!("detail expected: {why:?}"),
        }
    }

    fn named(bucket: CheckBucket, name: &str, url: &str) -> Check {
        Check {
            bucket,
            name: name.into(),
            url: Some(url.into()),
        }
    }

    /// A `latestReviews` entry as `gh` prints it.
    fn review(login: &str, state: &str, at: &str) -> String {
        format!(
            r#"{{"id":"","author":{{"login":"{login}"}},"authorAssociation":"MEMBER","body":"","submittedAt":"{at}","includesCreatedEdit":false,"reactionGroups":[],"state":"{state}","commit":{{"oid":""}}}}"#
        )
    }

    #[test]
    fn github_detail_reads_every_field() {
        let checks = rollup(&[
            check_run("lint", "COMPLETED", "SUCCESS"),
            check_run("docs", "COMPLETED", "SKIPPED"),
            check_run("bench", "COMPLETED", "NEUTRAL"),
            check_run("build", "IN_PROGRESS", ""),
            check_run("test (ubuntu-latest)", "COMPLETED", "FAILURE"),
            check_run("e2e", "COMPLETED", "TIMED_OUT"),
            check_run("deploy", "QUEUED", ""),
            status_context("ci/jenkins", "SUCCESS"),
            status_context("ci/legacy", "ERROR"),
            status_context("ci/slow", "PENDING"),
        ]);
        let reviews = format!(
            "[{}]",
            [
                review("octo-user", "APPROVED", "2026-01-01T00:00:00Z"),
                review("reviewer-b", "CHANGES_REQUESTED", "2026-01-01T00:00:00Z"),
                review("reviewer-c", "COMMENTED", "2026-01-01T00:00:00Z"),
            ]
            .join(",")
        );
        let json = json_with(
            r#"{"number":7,"title":"Add login","state":"OPEN","isDraft":false,
                "author":{"login":"octocat"},"headRefName":"feature/login","baseRefName":"main",
                "reviewDecision":"CHANGES_REQUESTED","mergeStateStatus":"DIRTY",
                "updatedAt":"2026-01-02T00:00:00Z","url":"https://github.com/octo/demo/pull/7"}"#,
            &[("statusCheckRollup", &checks), ("latestReviews", &reviews)],
        );
        assert_eq!(
            detail(gh_detail(&json)),
            PullRequestDetail {
                number: 7,
                title: "Add login".into(),
                state: "OPEN".into(),
                draft: false,
                author: Some("octocat".into()),
                head: Some("feature/login".into()),
                base: Some("main".into()),
                review: Some(PrReview::ChangesRequested),
                reviews: Some(ReviewCount::Approved(1)),
                checks: ChecksSummary {
                    pass: 4,
                    fail: 3,
                    pending: 3,
                },
                pipeline: None,
                check_names: vec![
                    named(
                        CheckBucket::Fail,
                        "test (ubuntu-latest)",
                        &details_url("test (ubuntu-latest)")
                    ),
                    named(CheckBucket::Fail, "e2e", &details_url("e2e")),
                ],
                more_checks: 4,
                notes: PrNotes {
                    conflicts: true,
                    unresolved_threads: false,
                    changes_requested: true,
                },
                ci_link: CiLink {
                    url: details_url("test (ubuntu-latest)"),
                    kind: CiLinkKind::FailedCheck,
                },
                updated_at: Some(1_767_312_000),
            }
        );
    }

    #[test]
    fn github_review_count_takes_each_authors_latest_review() {
        let count = |reviews: &[String]| {
            let json = json_with(r#"{"number":7}"#, &[("latestReviews", &rollup(reviews))]);
            detail(gh_detail(&json)).reviews
        };
        assert_eq!(
            count(&[
                review("a", "APPROVED", "2026-01-01T00:00:00Z"),
                review("a", "COMMENTED", "2026-01-02T00:00:00Z"),
                review("b", "CHANGES_REQUESTED", "2026-01-01T00:00:00Z"),
                review("b", "APPROVED", "2026-01-03T00:00:00Z"),
                review("c", "APPROVED", "2026-01-01T00:00:00Z"),
                review("c", "APPROVED", "2026-01-01T00:00:00Z"),
            ]),
            Some(ReviewCount::Approved(2)),
            "a's latest is a comment; b and c approve"
        );
        let no_author =
            r#"{"author":null,"state":"APPROVED","submittedAt":"2026-01-01T00:00:00Z"}"#;
        assert_eq!(
            count(&[no_author.to_string()]),
            Some(ReviewCount::Approved(0))
        );
        assert_eq!(count(&[]), Some(ReviewCount::Approved(0)));
    }

    #[test]
    fn github_detail_tolerates_null_and_missing_fields() {
        let partial = detail(gh_detail(
            r#"{"number":7,"state":"MERGED","isDraft":true,"author":null,
                "reviewDecision":null,"statusCheckRollup":null,"updatedAt":null,
                "latestReviews":null,"mergeStateStatus":"DIRTY"}"#,
        ));
        assert_eq!(
            partial,
            PullRequestDetail {
                number: 7,
                title: String::new(),
                state: "MERGED".into(),
                draft: true,
                author: None,
                head: None,
                base: None,
                review: None,
                reviews: None,
                checks: ChecksSummary::default(),
                pipeline: None,
                check_names: Vec::new(),
                more_checks: 0,
                notes: PrNotes::default(),
                ci_link: CiLink {
                    url: format!("{}/checks", gh_url(7)),
                    kind: CiLinkKind::Checks,
                },
                updated_at: None,
            },
            "no URL: the checks tab of the PR page"
        );
        let empty_checks = detail(gh_detail(
            r#"{"number":7,"statusCheckRollup":[],"reviewDecision":"","author":{}}"#,
        ));
        assert_eq!(empty_checks.checks, ChecksSummary::default());
        assert_eq!(empty_checks.review, None);
        assert_eq!(empty_checks.author, None);
        for decision in ["APPROVED", "REVIEW_REQUIRED"] {
            let json = format!(r#"{{"number":7,"reviewDecision":"{decision}"}}"#);
            let review = detail(gh_detail(&json)).review.expect("review");
            assert_eq!(review.label(), decision);
        }
    }

    #[test]
    fn detail_failures_keep_the_reason() {
        assert_eq!(
            lookup_detail(GH_REMOTE, 7, &|_: &[String]| Err(Some(
                "could not find pull request".into()
            ))),
            PrDetailLookup::Failed(Some("could not find pull request".into()))
        );
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"title":"x"}"#,
            r#"{"number":"7"}"#,
        ] {
            assert_eq!(gh_detail(bad), PrDetailLookup::Failed(None), "{bad:?}");
        }
        let run =
            |_: &[String]| -> Result<String, Option<String>> { panic!("no CLI call expected") };
        assert_eq!(
            lookup_detail("git@bitbucket.org:octo/demo.git", 7, &run),
            PrDetailLookup::Failed(None)
        );
        let forge = GlForge::new(&[]).mr(Err(Some("glab not found".into())));
        assert_eq!(
            forge.detail().0,
            PrDetailLookup::Failed(Some("glab not found".into()))
        );
    }

    /// A GitLab job as `glab api` prints it.
    fn job(id: u64, name: &str, status: &str) -> String {
        format!(
            r#"{{"id":{id},"name":"{name}","stage":"test","status":"{status}","web_url":"https://gitlab.com/group/sub/demo/-/jobs/{id}"}}"#
        )
    }

    fn job_url(id: u64) -> String {
        format!("https://gitlab.com/group/sub/demo/-/jobs/{id}")
    }

    #[test]
    fn gitlab_detail_of_a_failed_open_mr_asks_approvals_and_jobs() {
        let r = gl_repo();
        let jobs = format!(
            "[{}]",
            [
                job(3, "build", "running"),
                job(4, "test:unit", "failed"),
                job(5, "lint", "failed"),
                job(6, "e2e", "running"),
            ]
            .join(",")
        );
        let view = json_with(
            &gl_view("opened", "failed"),
            &[("draft", "true"), ("has_conflicts", "true")],
        );
        let forge = GlForge::new(&[])
            .mr(Ok(view))
            .approvals(r#"{"approved":false,"approvals_required":2,"approvals_left":1,"approved_by":[{"user":{"username":"octo-user"}}]}"#)
            .jobs(Ok(jobs));
        let (result, calls) = forge.detail();
        assert_eq!(
            calls,
            vec![
                gitlab_mr_argv(&r, 21),
                gitlab_approvals_argv(&r, 21),
                gitlab_jobs_argv(&r, 99),
            ]
        );
        assert_eq!(
            detail(result),
            PullRequestDetail {
                number: 21,
                title: "Add login".into(),
                state: "OPEN".into(),
                draft: true,
                author: Some("octo-user".into()),
                head: Some(BRANCH.into()),
                base: Some("main".into()),
                review: Some(PrReview::NotApproved),
                reviews: Some(ReviewCount::Approvals {
                    given: 1,
                    required: 2
                }),
                checks: ChecksSummary {
                    pass: 0,
                    fail: 1,
                    pending: 0,
                },
                pipeline: Some("failed".into()),
                check_names: vec![
                    named(CheckBucket::Fail, "test:unit", &job_url(4)),
                    named(CheckBucket::Fail, "lint", &job_url(5)),
                ],
                more_checks: 2,
                notes: PrNotes {
                    conflicts: true,
                    unresolved_threads: false,
                    changes_requested: false,
                },
                ci_link: CiLink {
                    url: "https://gitlab.com/group/sub/demo/-/pipelines/99".into(),
                    kind: CiLinkKind::Pipeline,
                },
                updated_at: Some(1_772_600_767),
            }
        );
    }

    #[test]
    fn gitlab_detail_asks_jobs_only_while_the_pipeline_failed_or_runs() {
        let approved = r#"{"approved":true,"approvals_required":1,"approvals_left":0}"#;
        let running = GlForge::new(&[])
            .pipeline("running")
            .approvals(approved)
            .jobs(Ok(format!("[{}]", job(3, "build", "running"))));
        let (result, calls) = running.detail();
        assert_eq!(calls.len(), 3);
        let running = detail(result);
        assert_eq!(
            running.check_names,
            vec![named(CheckBucket::Pending, "build", &job_url(3))]
        );
        assert_eq!(running.review, Some(PrReview::Approved));
        assert_eq!(
            running.reviews,
            Some(ReviewCount::Approvals {
                given: 1,
                required: 1
            })
        );
        for status in ["success", "manual", ""] {
            let (result, calls) = GlForge::new(&[])
                .pipeline(status)
                .approvals(approved)
                .detail();
            let names: Vec<&str> = calls.iter().map(|argv| gl_endpoint(argv)).collect();
            assert_eq!(names, ["mr", "approvals"], "{status:?}");
            assert_eq!(
                detail(result).pipeline,
                Some(status.to_string()).filter(|s| !s.is_empty())
            );
        }
        let (result, _) = GlForge::new(&[])
            .pipeline("failed")
            .jobs(Err(None))
            .detail();
        let failed_jobs_call = detail(result);
        assert_eq!(
            (failed_jobs_call.check_names, failed_jobs_call.more_checks),
            (vec![], 0)
        );
        assert_eq!(
            failed_jobs_call.review,
            Some(PrReview::NotApproved),
            "approvals call failed"
        );
        assert_eq!(failed_jobs_call.reviews, None);
    }

    #[test]
    fn gitlab_approvals_given_is_required_minus_left() {
        let reviews = |approvals: &str| {
            let detail = detail(GlForge::new(&[]).approvals(approvals).detail().0);
            (detail.review, detail.reviews)
        };
        let not_met = Some(PrReview::NotApproved);
        let one_of = |given| Some(ReviewCount::Approvals { given, required: 1 });
        assert_eq!(
            reviews(
                r#"{"approved":false,"approvals_required":1,"approvals_left":1,
                    "approved_by":[{"user":{"username":"octo-user"}}]}"#
            ),
            (not_met, one_of(0)),
            "an approval the rules do not count is not given"
        );
        assert_eq!(
            reviews(r#"{"approved":false,"approvals_required":1,"approvals_left":3}"#),
            (not_met, one_of(0)),
            "clamped at 0"
        );
        assert_eq!(
            reviews(r#"{"approved":true,"approvals_required":2,"approvals_left":0}"#),
            (
                Some(PrReview::Approved),
                Some(ReviewCount::Approvals {
                    given: 2,
                    required: 2
                })
            )
        );
        for missing in [
            r#"{"approved":false,"approvals_required":1}"#,
            r#"{"approved":false,"approvals_left":1}"#,
            r#"{"approved":false,"approvals_required":1,"approvals_left":null}"#,
        ] {
            assert_eq!(reviews(missing), (not_met, None), "{missing}");
        }
    }

    #[test]
    fn gitlab_detail_of_a_merged_mr_skips_approvals_and_jobs_and_tolerates_nulls() {
        let forge =
            GlForge::new(&[]).mr(Ok(r#"{"iid":21,"state":"merged","work_in_progress":true,
            "author":null,"head_pipeline":{"id":99,"status":"failed"},"updated_at":null,
            "has_conflicts":true}"#
                .into()));
        let (result, calls) = forge.detail();
        assert_eq!(calls.len(), 1, "no approvals or jobs call for a merged MR");
        let merged = detail(result);
        assert_eq!(merged.state, "MERGED");
        assert!(merged.draft, "older work_in_progress flag");
        assert_eq!((merged.review, merged.reviews), (None, None));
        assert_eq!(merged.author, None);
        assert_eq!(merged.notes, PrNotes::default());
        assert_eq!(
            merged.ci_link.url,
            format!("{}/pipelines", gl_url(21)),
            "no URL anywhere: the pipelines tab of the MR page"
        );
        assert_eq!(merged.updated_at, None);
    }

    #[test]
    fn gitlab_pipeline_status_maps_to_one_bucket() {
        let bucket = |status: &str| {
            let json =
                format!(r#"{{"iid":21,"state":"merged","head_pipeline":{{"status":"{status}"}}}}"#);
            detail(GlForge::new(&[]).mr(Ok(json)).detail().0).checks
        };
        let one = |pass, fail, pending| ChecksSummary {
            pass,
            fail,
            pending,
        };
        for status in ["success", "skipped"] {
            assert_eq!(bucket(status), one(1, 0, 0), "{status}");
        }
        for status in ["failed", "canceled"] {
            assert_eq!(bucket(status), one(0, 1, 0), "{status}");
        }
        for status in [
            "created",
            "waiting_for_resource",
            "preparing",
            "pending",
            "running",
            "scheduled",
        ] {
            assert_eq!(bucket(status), one(0, 0, 1), "{status}");
        }
        for status in ["manual", "", "something_new"] {
            assert_eq!(bucket(status), ChecksSummary::default(), "{status:?}");
        }
    }

    #[test]
    fn rfc3339_reads_utc_fractions_and_offsets() {
        assert_eq!(rfc3339_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339_unix("2026-01-02T00:00:00Z"), Some(1_767_312_000));
        assert_eq!(
            rfc3339_unix("2026-03-04T05:06:07.123Z"),
            Some(1_772_600_767)
        );
        assert_eq!(
            rfc3339_unix("2026-03-04T00:06:07-05:00"),
            Some(1_772_600_767)
        );
        assert_eq!(rfc3339_unix("2024-02-29T23:59:59Z"), Some(1_709_251_199));
        assert_eq!(rfc3339_unix("1969-12-31T23:00:00Z"), Some(-3600));
        for bad in [
            "",
            "2026-01-02",
            "2026-01-02T00:00:00",
            "2026-13-02T00:00:00Z",
            "2026-01-02T24:00:00Z",
            "yesterday",
        ] {
            assert_eq!(rfc3339_unix(bad), None, "{bad:?}");
        }
    }

    // ---- run_cli / opener (real processes, never gh or glab) ----

    #[cfg(unix)]
    fn sh(script: &str) -> Vec<String> {
        strings(&["sh", "-c", script])
    }

    /// A cancel flag that is never set.
    fn live() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// A cancel flag set after `delay`, as a quit would set it.
    #[cfg(unix)]
    fn cancel_after(delay: Duration) -> std::sync::Arc<AtomicBool> {
        let cancel = std::sync::Arc::new(AtomicBool::new(false));
        let flag = std::sync::Arc::clone(&cancel);
        std::thread::spawn(move || {
            std::thread::sleep(delay);
            flag.store(true, Ordering::Relaxed);
        });
        cancel
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_returns_stdout_and_fails_on_exit_or_missing_program() {
        let live = live();
        assert_eq!(
            run_cli(&sh("printf 'a\\nb'"), &live),
            Ok("a\nb".to_string())
        );
        assert_eq!(
            run_cli(&sh("printf 'a'; echo 'warning: old token' >&2"), &live),
            Ok("a".to_string()),
            "stderr of a zero exit is ignored"
        );
        assert_eq!(run_cli(&sh("printf x; exit 3"), &live), Err(None));
        assert_eq!(
            run_cli(&strings(&["ws-no-such-program-for-tests"]), &live),
            Err(Some("ws-no-such-program-for-tests not found".into()))
        );
        assert_eq!(run_cli(&[], &live), Err(None));
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_keeps_the_first_stderr_line_of_a_failure() {
        let live = live();
        assert_eq!(
            run_cli(
                &sh("echo '{\"message\":\"404\"}'; \
                     printf '\\n   \\n  glab: 404 Not found (HTTP 404)  \\nsecond line\\n' >&2; \
                     exit 1"),
                &live
            ),
            Err(Some("glab: 404 Not found (HTTP 404)".into()))
        );
        assert_eq!(
            run_cli(
                &sh("printf '\\033[31mHTTP 401:\\033[0m\\033[1;4m Bad\\tcredentials\\r\\n' >&2; exit 4"),
                &live
            ),
            Err(Some("HTTP 401: Bad credentials".into())),
            "escape sequences go, other control characters become spaces"
        );
        assert_eq!(
            run_cli(&sh("printf '%0300d\\n' 0 >&2; exit 1"), &live),
            Err(Some(format!("{}…", "0".repeat(199)))),
            "cut to 200 characters"
        );
    }

    #[test]
    fn first_stderr_line_strips_csi_and_caps_the_length() {
        assert_eq!(
            first_stderr_line("\u{1b}[38;5;196merror:\u{1b}[m bad\u{7}".as_bytes()),
            Some("error: bad".into())
        );
        assert_eq!(
            first_stderr_line("lone \u{1b} escape".as_bytes()),
            Some("lone   escape".into()),
            "an ESC that starts no CSI sequence is a space"
        );
        assert_eq!(
            first_stderr_line("cut \u{1b}[1".as_bytes()),
            Some("cut".into()),
            "an unfinished sequence runs to the end"
        );
        let exact = "é".repeat(200);
        assert_eq!(first_stderr_line(exact.as_bytes()), Some(exact.clone()));
        let long = format!("{exact}x");
        let cut = first_stderr_line(long.as_bytes()).expect("line");
        assert_eq!(cut.chars().count(), 200);
        assert!(cut.ends_with("é…"), "{cut}");
        assert_eq!(first_stderr_line(b" \n\t\n"), None);
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_nulls_stdin_and_disables_prompts_and_colour() {
        let out = run_cli(
            &sh("if [ -t 0 ]; then echo tty; fi; \
             printf '%s,%s,%s' \"$GH_PROMPT_DISABLED\" \"$NO_PROMPT\" \"$NO_COLOR\""),
            &live(),
        );
        assert_eq!(out, Ok("1,1,1".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_kills_a_cli_that_runs_past_the_limit() {
        let started = Instant::now();
        let out = run_cli_within(&sh("exec sleep 5"), Duration::from_millis(200), &live());
        assert_eq!(out, Err(None));
        assert!(started.elapsed() < Duration::from_secs(3), "not bounded");
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_with_cancel_set_fails_without_starting() {
        let started = Instant::now();
        let out = run_cli(&sh("exec sleep 5"), &AtomicBool::new(true));
        assert_eq!(out, Err(None));
        assert!(started.elapsed() < Duration::from_secs(1), "waited");
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_kills_a_running_cli_once_cancel_is_set() {
        let cancel = cancel_after(Duration::from_millis(150));
        let started = Instant::now();
        // The full time limit (20 s) would outlast the sleep; only the
        // cancel can end this early.
        let out = run_cli(&sh("exec sleep 5"), &cancel);
        assert_eq!(out, Err(None));
        assert!(started.elapsed() < Duration::from_secs(3), "not cancelled");
    }

    fn opener(argv: &[&str], check_exit: bool) -> BrowserCommand {
        BrowserCommand {
            argv: strings(argv),
            check_exit,
        }
    }

    #[cfg(unix)]
    #[test]
    fn spawn_opener_reads_platform_opener_exit_and_ignores_browser_env_exit() {
        let window = Duration::from_secs(3);
        let live = live();
        let run =
            |argv: &[&str], check_exit| spawn_opener(&opener(argv, check_exit), window, &live);
        assert_eq!(run(&["true"], true), Ok(()));
        assert_eq!(run(&["false"], true), Err(()));
        let missing = "ws-no-such-browser-for-tests";
        assert_eq!(run(&[missing], true), Err(()));
        // A `$BROWSER` command is not waited for: its exit code is not ours.
        assert_eq!(run(&["false"], false), Ok(()));
        assert_eq!(run(&[missing], false), Err(()));
        assert_eq!(run(&[], true), Err(()));
    }

    #[cfg(unix)]
    #[test]
    fn spawn_opener_stops_waiting_for_a_platform_opener_after_the_window() {
        let started = Instant::now();
        let slow = opener(&["sh", "-c", "exec sleep 5"], true);
        assert_eq!(
            spawn_opener(&slow, Duration::from_millis(200), &live()),
            Ok(())
        );
        assert!(started.elapsed() < Duration::from_secs(3), "not bounded");
    }

    #[cfg(unix)]
    #[test]
    fn spawn_opener_stops_waiting_once_cancel_is_set() {
        let slow = opener(&["sh", "-c", "exec sleep 5"], true);
        let started = Instant::now();
        assert_eq!(
            spawn_opener(&slow, Duration::from_secs(4), &AtomicBool::new(true)),
            Err(()),
            "a set flag starts nothing"
        );
        assert!(started.elapsed() < Duration::from_secs(1), "waited");
        let cancel = cancel_after(Duration::from_millis(150));
        let started = Instant::now();
        assert_eq!(
            spawn_opener(&slow, Duration::from_secs(4), &cancel),
            Err(())
        );
        assert!(started.elapsed() < Duration::from_secs(2), "not cancelled");
    }

    #[test]
    fn open_in_browser_refuses_non_web_urls_without_spawning() {
        for url in [
            "file:///etc/passwd",
            "javascript:alert(1)",
            "ftp://example.com/pr/1",
            "github.com/octo/demo/pull/7",
            "https://example.com/a b",
            "https://example.com/a\nb",
            "",
        ] {
            assert_eq!(open_in_browser(url, &live()), Err(()), "{url:?}");
        }
        assert!(is_web_url("https://github.com/octo/demo/pull/7"));
        assert!(is_web_url(
            "HTTP://gitlab.example.com/g/r/-/merge_requests/3"
        ));
    }

    #[test]
    fn open_in_browser_refuses_urls_with_cmd_syntax_characters() {
        for special in ['&', '|', '^', '%', '<', '>', '"'] {
            let url = format!("https://github.com/octo/demo/pull/7{special}calc");
            assert!(!is_web_url(&url), "{url:?}");
            assert_eq!(open_in_browser(&url, &live()), Err(()), "{url:?}");
        }
    }

    // ---- browser_command ----

    const URL: &str = "https://github.com/octo/demo/pull/7";

    #[test]
    fn browser_env_with_and_without_placeholder() {
        assert_eq!(
            browser_command(URL, Some("firefox"), "linux"),
            opener(&["firefox", URL], false)
        );
        assert_eq!(
            browser_command(URL, Some("firefox --new-tab %s"), "linux"),
            opener(&["firefox", "--new-tab", URL], false)
        );
        assert_eq!(
            browser_command(URL, Some("chromium --profile-directory=Work"), "macos"),
            opener(&["chromium", "--profile-directory=Work", URL], false)
        );
        assert_eq!(
            browser_command(URL, Some("open-url --target=%s"), "linux"),
            opener(&["open-url", &format!("--target={URL}")], false)
        );
    }

    #[test]
    fn browser_env_colon_list_uses_first_non_empty_entry() {
        assert_eq!(
            browser_command(URL, Some(":  :firefox %s:chromium"), "linux"),
            opener(&["firefox", URL], false)
        );
    }

    #[test]
    fn empty_browser_env_falls_back_to_os_opener() {
        for env in [None, Some(""), Some("   "), Some("::")] {
            assert_eq!(
                browser_command(URL, env, "linux"),
                opener(&["xdg-open", URL], true),
                "{env:?}"
            );
        }
        assert_eq!(
            browser_command(URL, None, "macos"),
            opener(&["open", URL], true)
        );
        assert_eq!(
            browser_command(URL, Some(""), "windows"),
            opener(&["cmd", "/c", "start", "", URL], true)
        );
        assert_eq!(
            browser_command(URL, None, "freebsd"),
            opener(&["xdg-open", URL], true)
        );
    }

    // ---- live forge (real gh / glab, public read-only PRs; `--ignored`) ----

    /// The real runner with a cancel flag that is never set.
    fn real_run(argv: &[String]) -> Result<String, Option<String>> {
        run_cli(argv, &AtomicBool::new(false))
    }

    /// True when `program` is installed and logged in to `host`; else
    /// prints why the live test skips.
    fn live_cli_ready(program: &str, host: &str) -> bool {
        match real_run(&strings(&[program, "auth", "status", "--hostname", host])) {
            Ok(_) => true,
            Err(why) => {
                println!(
                    "skip: {program} is missing or not logged in to {host} ({})",
                    why.unwrap_or_default()
                );
                false
            }
        }
    }

    /// Head branches of the same-repository GitHub PRs of `cli/cli` in
    /// `state`, newest first (at most 50 PRs).
    fn live_github_branches(state: &str) -> Vec<String> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Entry {
            head_ref_name: String,
            is_cross_repository: bool,
        }
        let json = real_run(&strings(&[
            "gh",
            "pr",
            "list",
            "-R",
            "github.com/cli/cli",
            "--state",
            state,
            "--limit",
            "50",
            "--json",
            "headRefName,isCrossRepository,number",
        ]))
        .expect("gh pr list");
        let entries: Vec<Entry> = serde_json::from_str(&json).expect("gh pr list JSON");
        entries
            .into_iter()
            .filter(|entry| !entry.is_cross_repository)
            .map(|entry| entry.head_ref_name)
            .collect()
    }

    /// Source branches of the same-project GitLab MRs of `gitlab-org/cli`
    /// in `state`, newest first (at most 50 MRs).
    fn live_gitlab_branches(state: &str) -> Vec<String> {
        #[derive(Deserialize)]
        struct Entry {
            source_branch: String,
            source_project_id: u64,
            target_project_id: u64,
        }
        let json = real_run(&strings(&[
            "glab",
            "api",
            "--hostname",
            "gitlab.com",
            &format!("projects/gitlab-org%2Fcli/merge_requests?state={state}&per_page=50"),
        ]))
        .expect("glab api merge_requests");
        let entries: Vec<Entry> = serde_json::from_str(&json).expect("merge_requests JSON");
        entries
            .into_iter()
            .filter(|entry| entry.source_project_id == entry.target_project_id)
            .map(|entry| entry.source_branch)
            .collect()
    }

    /// Look up `branch` at `remote` and fetch its detail with the real CLI;
    /// check the PR class (merged or open), URL, next action, CI link, and
    /// that the detail parses.
    fn live_check(remote: &str, branch: &str, merged: bool, url_prefix: &str) {
        let pr = match lookup(Some(remote), branch, &real_run) {
            PrLookup::Found(pr) => pr,
            other => panic!("{branch}: a PR expected, got {other:?}"),
        };
        println!(
            "{branch}: {} {} · next: {} · {}: {} · checks {:?} · notes {:?}",
            pr.ref_text(),
            pr.state.label(),
            pr.next_action(None)
                .map_or("none".into(), |action| action.text()),
            pr.ci_link.kind.label(),
            pr.ci_link.url,
            pr.checks,
            pr.notes.texts(),
        );
        assert!(pr.url.starts_with(url_prefix), "{}", pr.url);
        assert_eq!(pr.state == PrState::Merged, merged, "{:?}", pr.state);
        assert_eq!(pr.next_action(None).is_some(), !merged);
        assert!(pr.ci_link.url.starts_with("https://"), "{}", pr.ci_link.url);
        let detail = match lookup_detail(remote, pr.number, &real_run) {
            PrDetailLookup::Found(detail) => detail,
            PrDetailLookup::Failed(why) => panic!("{branch}: detail failed: {why:?}"),
        };
        println!(
            "  detail: {} · {} · reviews {:?} · checks {:?} · named {:?} +{} · pipeline {:?} · {}",
            detail.state,
            detail.title,
            detail.reviews,
            detail.checks,
            detail
                .check_names
                .iter()
                .map(|check| check.name.as_str())
                .collect::<Vec<_>>(),
            detail.more_checks,
            detail.pipeline,
            detail.ci_link.kind.label(),
        );
        assert_eq!(detail.number, pr.number);
        assert_eq!(detail.state, if merged { "MERGED" } else { "OPEN" });
    }

    const LIVE_GITHUB: &str = "https://github.com/cli/cli.git";
    const LIVE_GITLAB: &str = "https://gitlab.com/gitlab-org/cli.git";

    #[test]
    #[ignore = "runs the real gh against github.com/cli/cli"]
    fn live_github_open_pr_parses() {
        if !live_cli_ready("gh", "github.com") {
            return;
        }
        let Some(branch) = live_github_branches("open").into_iter().next() else {
            println!("skip: cli/cli has no open same-repository PR");
            return;
        };
        live_check(
            LIVE_GITHUB,
            &branch,
            false,
            "https://github.com/cli/cli/pull/",
        );
    }

    #[test]
    #[ignore = "runs the real gh against github.com/cli/cli"]
    fn live_github_merged_pr_parses() {
        if !live_cli_ready("gh", "github.com") {
            return;
        }
        // A branch that also has an open PR would pick the open one.
        let open = live_github_branches("open");
        let Some(branch) = live_github_branches("merged")
            .into_iter()
            .find(|branch| !open.contains(branch))
        else {
            println!("skip: cli/cli has no merged same-repository PR");
            return;
        };
        live_check(
            LIVE_GITHUB,
            &branch,
            true,
            "https://github.com/cli/cli/pull/",
        );
    }

    #[test]
    #[ignore = "runs the real glab against gitlab.com/gitlab-org/cli"]
    fn live_gitlab_open_mr_parses() {
        if !live_cli_ready("glab", "gitlab.com") {
            return;
        }
        let Some(branch) = live_gitlab_branches("opened").into_iter().next() else {
            println!("skip: gitlab-org/cli has no open same-project MR");
            return;
        };
        live_check(
            LIVE_GITLAB,
            &branch,
            false,
            "https://gitlab.com/gitlab-org/cli/-/merge_requests/",
        );
    }

    #[test]
    #[ignore = "runs the real glab against gitlab.com/gitlab-org/cli"]
    fn live_gitlab_merged_mr_parses() {
        if !live_cli_ready("glab", "gitlab.com") {
            return;
        }
        let open = live_gitlab_branches("opened");
        let Some(branch) = live_gitlab_branches("merged")
            .into_iter()
            .find(|branch| !open.contains(branch))
        else {
            println!("skip: gitlab-org/cli has no merged same-project MR");
            return;
        };
        live_check(
            LIVE_GITLAB,
            &branch,
            true,
            "https://gitlab.com/gitlab-org/cli/-/merge_requests/",
        );
    }

    // ---- status strings ----

    #[test]
    fn status_strings_are_exact() {
        assert_eq!(no_pr_status("feature/login"), "no PR for feature/login");
        assert_eq!(NO_PR_FOR_ROW, "no PR for this row");
        assert_eq!(
            lookup_failed_status("feature/login"),
            "could not look up PR for feature/login"
        );
        assert_eq!(OPEN_FAILED, "could not open PR");
    }
}

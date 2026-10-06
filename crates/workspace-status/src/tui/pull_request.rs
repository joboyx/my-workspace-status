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
//! [`lookup_detail`] fetches the rich fields of one known PR (title, review,
//! author, branches, checks, update time) for the PR popover, with the same
//! runner and argv-builder style.

use std::io::Read;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde::Deserialize;

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

/// State of the PR that counts for a branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    /// Open, and the forge does not report it as approved.
    Open,
    /// Open, and the forge reports it as approved.
    Approved,
    /// Merged.
    Merged,
}

/// The PR that counts for a branch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PullRequest {
    /// GitHub PR number or GitLab MR iid.
    pub number: u64,
    /// Web URL of the PR page.
    pub url: String,
    /// Open, approved, or merged.
    pub state: PrState,
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
    /// or printed output that does not parse.
    Failed,
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

/// Checks of a PR head, counted per outcome.
///
/// GitHub counts each `statusCheckRollup` entry. GitLab has one head
/// pipeline status, so at most one bucket is 1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ChecksSummary {
    /// Passed, neutral, or skipped.
    pub pass: usize,
    /// Failed, errored, cancelled, or timed out.
    pub fail: usize,
    /// Queued, running, or waiting.
    pub pending: usize,
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
    /// Checks of the head commit.
    pub checks: ChecksSummary,
    /// Last update, in Unix seconds.
    pub updated_at: Option<i64>,
}

/// Result of one PR detail fetch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrDetailLookup {
    /// The forge described the PR.
    Found(PullRequestDetail),
    /// No answer: unsupported remote, CLI missing or failed, or output that
    /// does not parse.
    Failed,
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

/// Percent-encode everything except RFC 3986 unreserved characters.
///
/// `/` becomes `%2F`, which GitLab needs for a project path in the API.
fn percent_encode(text: &str) -> String {
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
        "number,state,url,headRefName,isCrossRepository,reviewDecision,updatedAt",
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
        "number,title,state,isDraft,author,headRefName,baseRefName,\
         reviewDecision,statusCheckRollup,updatedAt",
    ]
    .map(str::to_string)
    .to_vec()
}

/// `glab api` argv (program first) for MR `iid` with its detail fields.
pub fn gitlab_mr_argv(repo: &ForgeRepo, iid: u64) -> Vec<String> {
    let endpoint = format!(
        "projects/{}/merge_requests/{iid}",
        percent_encode(&repo.path)
    );
    gitlab_api_argv(repo, endpoint)
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

/// One `gh pr list --json` entry.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPr {
    number: u64,
    state: String,
    url: String,
    #[serde(default)]
    head_ref_name: Option<String>,
    /// True for a PR from a fork; `--head` matches the branch name only.
    #[serde(default)]
    is_cross_repository: bool,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    updated_at: Option<String>,
}

/// One GitLab `merge_requests` API entry.
#[derive(Deserialize)]
struct GitLabMr {
    iid: u64,
    state: String,
    web_url: String,
    #[serde(default)]
    source_branch: Option<String>,
    /// Differs from `target_project_id` for an MR from a fork.
    #[serde(default)]
    source_project_id: Option<u64>,
    #[serde(default)]
    target_project_id: Option<u64>,
    #[serde(default)]
    updated_at: Option<String>,
}

/// GitLab `merge_requests/<iid>/approvals` response.
#[derive(Deserialize)]
struct GitLabApprovals {
    #[serde(default)]
    approved: Option<bool>,
}

/// Open or merged: the only PR kinds that count.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Open,
    Merged,
}

/// A PR that counts, in a forge-neutral shape for [`pick`].
struct Candidate {
    kind: Kind,
    exact_head: bool,
    /// RFC 3339 timestamp as the forge prints it; compared as text.
    updated_at: String,
    number: u64,
    url: String,
    /// GitHub only; GitLab asks a second endpoint after the pick.
    approved: bool,
}

/// Open before merged, then exact head match, then most recently updated.
///
/// Both forges print `updated_at` in UTC with a fixed layout, so text order
/// is time order.
fn pick(candidates: Vec<Candidate>) -> Option<Candidate> {
    candidates.into_iter().max_by(|a, b| {
        let rank = |c: &Candidate| (c.kind == Kind::Open, c.exact_head);
        rank(a)
            .cmp(&rank(b))
            .then_with(|| a.updated_at.cmp(&b.updated_at))
    })
}

fn github_candidates(json: &str, branch: &str) -> Result<Vec<Candidate>, ()> {
    let prs: Vec<GitHubPr> = serde_json::from_str(json).map_err(|_| ())?;
    Ok(prs
        .into_iter()
        .filter(|pr| !pr.is_cross_repository)
        .filter_map(|pr| {
            let kind = match pr.state.as_str() {
                "OPEN" => Kind::Open,
                "MERGED" => Kind::Merged,
                _ => return None,
            };
            Some(Candidate {
                kind,
                exact_head: pr.head_ref_name.as_deref() == Some(branch),
                updated_at: pr.updated_at.unwrap_or_default(),
                number: pr.number,
                url: pr.url,
                approved: pr.review_decision.as_deref() == Some("APPROVED"),
            })
        })
        .collect())
}

fn gitlab_candidates(json: &str, branch: &str) -> Result<Vec<Candidate>, ()> {
    let mrs: Vec<GitLabMr> = serde_json::from_str(json).map_err(|_| ())?;
    Ok(mrs
        .into_iter()
        .filter(|mr| mr.source_project_id == mr.target_project_id)
        .filter_map(|mr| {
            let kind = match mr.state.as_str() {
                "opened" => Kind::Open,
                "merged" => Kind::Merged,
                _ => return None,
            };
            Some(Candidate {
                kind,
                exact_head: mr.source_branch.as_deref() == Some(branch),
                updated_at: mr.updated_at.unwrap_or_default(),
                number: mr.iid,
                url: mr.web_url,
                approved: false,
            })
        })
        .collect())
}

fn to_pull_request(candidate: Candidate) -> PullRequest {
    let state = match (candidate.kind, candidate.approved) {
        (Kind::Merged, _) => PrState::Merged,
        (Kind::Open, true) => PrState::Approved,
        (Kind::Open, false) => PrState::Open,
    };
    PullRequest {
        number: candidate.number,
        url: candidate.url,
        state,
    }
}

/// Find the PR that counts for `branch` on the forge behind `remote_url`.
///
/// Blocks on the forge CLI through `run` (argv with the program first; `Ok`
/// is stdout of a zero exit). Pass [`run_cli`] in production and a fake in
/// tests. GitLab asks a second endpoint for approval of an open MR; when that
/// call fails the MR stays [`PrState::Open`] and the lookup still succeeds.
pub fn lookup(
    remote_url: Option<&str>,
    branch: &str,
    run: &dyn Fn(&[String]) -> Result<String, ()>,
) -> PrLookup {
    let Some(repo) = remote_url.and_then(parse_remote) else {
        return PrLookup::NoPr;
    };
    let found = match repo.forge {
        Forge::GitHub => run(&github_list_argv(&repo, branch))
            .and_then(|json| github_candidates(&json, branch))
            .map(pick),
        Forge::GitLab => run(&gitlab_list_argv(&repo, branch))
            .and_then(|json| gitlab_candidates(&json, branch))
            .map(pick)
            .map(|picked| {
                picked.map(|mut mr| {
                    if mr.kind == Kind::Open {
                        mr.approved = gitlab_approved(&repo, mr.number, run);
                    }
                    mr
                })
            }),
    };
    match found {
        Ok(Some(candidate)) => PrLookup::Found(to_pull_request(candidate)),
        Ok(None) => PrLookup::NoPr,
        Err(()) => PrLookup::Failed,
    }
}

/// True only when the approvals endpoint answers `"approved": true`.
fn gitlab_approved(
    repo: &ForgeRepo,
    iid: u64,
    run: &dyn Fn(&[String]) -> Result<String, ()>,
) -> bool {
    run(&gitlab_approvals_argv(repo, iid))
        .ok()
        .and_then(|json| serde_json::from_str::<GitLabApprovals>(&json).ok())
        .and_then(|approvals| approvals.approved)
        .unwrap_or(false)
}

/// `gh pr view --json` detail object. Every field but the number may be
/// missing or `null`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct GitHubPrView {
    number: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    is_draft: Option<bool>,
    #[serde(default)]
    author: Option<ForgeUser>,
    #[serde(default)]
    head_ref_name: Option<String>,
    #[serde(default)]
    base_ref_name: Option<String>,
    #[serde(default)]
    review_decision: Option<String>,
    #[serde(default)]
    status_check_rollup: Option<Vec<GitHubCheck>>,
    #[serde(default)]
    updated_at: Option<String>,
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

/// One `statusCheckRollup` entry: a check run (`status`, `conclusion`) or a
/// commit status context (`state`).
#[derive(Deserialize)]
struct GitHubCheck {
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

/// GitLab `merge_requests/<iid>` object. Every field but the iid may be
/// missing or `null`.
#[derive(Deserialize)]
struct GitLabMrView {
    iid: u64,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    draft: Option<bool>,
    /// Older GitLab name of `draft`.
    #[serde(default)]
    work_in_progress: Option<bool>,
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
}

#[derive(Deserialize)]
struct GitLabPipeline {
    #[serde(default)]
    status: Option<String>,
}

/// Outcome bucket of one check.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CheckBucket {
    Pass,
    Fail,
    Pending,
}

impl ChecksSummary {
    fn add(&mut self, bucket: CheckBucket) {
        match bucket {
            CheckBucket::Pass => self.pass += 1,
            CheckBucket::Fail => self.fail += 1,
            CheckBucket::Pending => self.pending += 1,
        }
    }
}

/// Bucket of one GitHub rollup entry. A status context reads `state`; a
/// check run that is not `COMPLETED`, or has no conclusion yet, is
/// pending.
fn github_check_bucket(check: &GitHubCheck) -> CheckBucket {
    if let Some(state) = check.state.as_deref() {
        return match state {
            "SUCCESS" => CheckBucket::Pass,
            "FAILURE" | "ERROR" => CheckBucket::Fail,
            _ => CheckBucket::Pending,
        };
    }
    if check
        .status
        .as_deref()
        .is_some_and(|status| status != "COMPLETED")
    {
        return CheckBucket::Pending;
    }
    match check.conclusion.as_deref() {
        Some("SUCCESS" | "NEUTRAL" | "SKIPPED") => CheckBucket::Pass,
        Some(
            "FAILURE" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED" | "STARTUP_FAILURE" | "STALE",
        ) => CheckBucket::Fail,
        _ => CheckBucket::Pending,
    }
}

/// Bucket of a GitLab head pipeline status; `None` for no pipeline.
fn gitlab_pipeline_bucket(status: &str) -> Option<CheckBucket> {
    match status {
        "" => None,
        "success" | "skipped" => Some(CheckBucket::Pass),
        "failed" | "canceled" => Some(CheckBucket::Fail),
        _ => Some(CheckBucket::Pending),
    }
}

fn github_review(decision: &str) -> Option<PrReview> {
    match decision {
        "APPROVED" => Some(PrReview::Approved),
        "CHANGES_REQUESTED" => Some(PrReview::ChangesRequested),
        "REVIEW_REQUIRED" => Some(PrReview::ReviewRequired),
        _ => None,
    }
}

fn github_detail(json: &str) -> Result<PullRequestDetail, ()> {
    let pr: GitHubPrView = serde_json::from_str(json).map_err(|_| ())?;
    let mut checks = ChecksSummary::default();
    for check in pr.status_check_rollup.unwrap_or_default() {
        checks.add(github_check_bucket(&check));
    }
    Ok(PullRequestDetail {
        number: pr.number,
        title: pr.title.unwrap_or_default(),
        state: pr.state.unwrap_or_default().to_ascii_uppercase(),
        draft: pr.is_draft.unwrap_or(false),
        author: pr.author.and_then(ForgeUser::name),
        head: pr.head_ref_name.filter(|name| !name.is_empty()),
        base: pr.base_ref_name.filter(|name| !name.is_empty()),
        review: pr.review_decision.as_deref().and_then(github_review),
        checks,
        updated_at: pr.updated_at.as_deref().and_then(rfc3339_unix),
    })
}

fn gitlab_detail(json: &str) -> Result<(PullRequestDetail, bool), ()> {
    let mr: GitLabMrView = serde_json::from_str(json).map_err(|_| ())?;
    let state = mr.state.unwrap_or_default();
    let open = state == "opened";
    let mut checks = ChecksSummary::default();
    if let Some(bucket) = mr
        .head_pipeline
        .and_then(|pipeline| pipeline.status)
        .as_deref()
        .and_then(gitlab_pipeline_bucket)
    {
        checks.add(bucket);
    }
    let detail = PullRequestDetail {
        number: mr.iid,
        title: mr.title.unwrap_or_default(),
        state: if open {
            "OPEN".to_string()
        } else {
            state.to_ascii_uppercase()
        },
        draft: mr.draft.or(mr.work_in_progress).unwrap_or(false),
        author: mr.author.and_then(ForgeUser::name),
        head: mr.source_branch.filter(|name| !name.is_empty()),
        base: mr.target_branch.filter(|name| !name.is_empty()),
        review: None,
        checks,
        updated_at: mr.updated_at.as_deref().and_then(rfc3339_unix),
    };
    Ok((detail, open))
}

/// Fetch the detail fields of PR `number` on the forge behind `remote_url`.
///
/// Blocks on the forge CLI through `run`, the same injectable runner as
/// [`lookup`]. GitHub is one `gh pr view`. GitLab is the MR endpoint, then,
/// for an open MR, the approvals endpoint ([`PrReview::Approved`] or
/// [`PrReview::NotApproved`]). An unsupported remote fails without a CLI
/// call.
pub fn lookup_detail(
    remote_url: &str,
    number: u64,
    run: &dyn Fn(&[String]) -> Result<String, ()>,
) -> PrDetailLookup {
    let Some(repo) = parse_remote(remote_url) else {
        return PrDetailLookup::Failed;
    };
    let detail = match repo.forge {
        Forge::GitHub => {
            run(&github_view_argv(&repo, number)).and_then(|json| github_detail(&json))
        }
        Forge::GitLab => run(&gitlab_mr_argv(&repo, number))
            .and_then(|json| gitlab_detail(&json))
            .map(|(mut detail, open)| {
                if open {
                    detail.review = Some(if gitlab_approved(&repo, number, run) {
                        PrReview::Approved
                    } else {
                        PrReview::NotApproved
                    });
                }
                detail
            }),
    };
    match detail {
        Ok(detail) => PrDetailLookup::Found(detail),
        Err(()) => PrDetailLookup::Failed,
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

/// Run a forge CLI and return its stdout. `Err` when it cannot start, exits
/// non-zero, or runs longer than 20 seconds (it is then killed). `Err` at
/// once when `cancel` is set, before the start or while it runs (it is then
/// killed).
///
/// Stdin and stderr are null, so the CLI cannot read the TUI's terminal or
/// print over it. `GH_PROMPT_DISABLED` and `NO_PROMPT` (read by `glab`) make
/// the CLIs fail instead of asking for input; `NO_COLOR` keeps escape codes
/// out of the JSON. No token is passed; the CLIs use their own login. The
/// time limit keeps a stalled CLI from holding a worker; the cancel flag
/// keeps a running one from holding the TUI's exit.
pub fn run_cli(argv: &[String], cancel: &AtomicBool) -> Result<String, ()> {
    run_cli_within(argv, CLI_TIME_LIMIT, cancel)
}

fn run_cli_within(argv: &[String], limit: Duration, cancel: &AtomicBool) -> Result<String, ()> {
    if cancel.load(Ordering::Relaxed) {
        return Err(());
    }
    let deadline = Instant::now() + limit;
    let (program, args) = argv.split_first().ok_or(())?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env("GH_PROMPT_DISABLED", "1")
        .env("NO_PROMPT", "1")
        .env("NO_COLOR", "1")
        .spawn()
        .map_err(|_| ())?;
    // Read stdout on its own thread so a full pipe cannot stall the child.
    let mut stdout = child.stdout.take().ok_or(())?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let read = stdout.read_to_end(&mut bytes).map(|_| bytes);
        let _ = tx.send(read);
    });
    let Some(status) = wait_until(&mut child, deadline, cancel) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(());
    };
    if !status.success() {
        return Err(());
    }
    // A grandchild that keeps stdout open must not stall us either.
    let grace = deadline
        .saturating_duration_since(Instant::now())
        .max(Duration::from_millis(200));
    let bytes = rx.recv_timeout(grace).map_err(|_| ())?.map_err(|_| ())?;
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
fn is_web_url(url: &str) -> bool {
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
                "number,state,url,headRefName,isCrossRepository,reviewDecision,updatedAt",
                "--limit",
                "30",
            ])
        );
    }

    #[test]
    fn gitlab_argv_encodes_subgroup_path_and_branch() {
        let r = repo(Forge::GitLab, "gitlab.example.com", "group/sub/demo");
        assert_eq!(
            gitlab_list_argv(&r, "feature/a b+c"),
            strings(&[
                "glab",
                "api",
                "--hostname",
                "gitlab.example.com",
                "projects/group%2Fsub%2Fdemo/merge_requests\
                 ?source_branch=feature%2Fa%20b%2Bc&state=all&per_page=30",
            ])
        );
        assert_eq!(
            gitlab_approvals_argv(&r, 42),
            strings(&[
                "glab",
                "api",
                "--hostname",
                "gitlab.example.com",
                "projects/group%2Fsub%2Fdemo/merge_requests/42/approvals",
            ])
        );
    }

    #[test]
    fn percent_encode_keeps_unreserved_only() {
        assert_eq!(percent_encode("a-Z_0.9~"), "a-Z_0.9~");
        assert_eq!(percent_encode("a/b&c=d#é"), "a%2Fb%26c%3Dd%23%C3%A9");
    }

    // ---- lookup: GitHub ----

    const GH_REMOTE: &str = "git@github.com:octo/demo.git";
    const BRANCH: &str = "feature/login";

    fn gh_pr(number: u64, state: &str, head: &str, decision: &str, updated: &str) -> String {
        format!(
            r#"{{"number":{number},"state":"{state}","url":"https://github.com/octo/demo/pull/{number}","headRefName":"{head}","isCrossRepository":false,"reviewDecision":"{decision}","updatedAt":"{updated}"}}"#
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

    fn found(number: u64, url_kind: &str, state: PrState) -> PrLookup {
        let url = match url_kind {
            "gh" => format!("https://github.com/octo/demo/pull/{number}"),
            _ => format!("https://gitlab.com/group/sub/demo/-/merge_requests/{number}"),
        };
        PrLookup::Found(PullRequest { number, url, state })
    }

    #[test]
    fn github_open_pr_is_open() {
        let prs = [gh_pr(7, "OPEN", BRANCH, "", "2026-01-02T00:00:00Z")];
        assert_eq!(gh_lookup(&prs), found(7, "gh", PrState::Open));
    }

    #[test]
    fn github_review_required_or_changes_requested_stays_open() {
        for decision in ["REVIEW_REQUIRED", "CHANGES_REQUESTED", ""] {
            let prs = [gh_pr(7, "OPEN", BRANCH, decision, "2026-01-02T00:00:00Z")];
            assert_eq!(gh_lookup(&prs), found(7, "gh", PrState::Open), "{decision}");
        }
        let null_decision = r#"[{"number":7,"state":"OPEN","url":"https://github.com/octo/demo/pull/7","headRefName":"feature/login","reviewDecision":null,"updatedAt":null}]"#;
        let result = lookup(Some(GH_REMOTE), BRANCH, &|_: &[String]| {
            Ok(null_decision.to_string())
        });
        assert_eq!(result, found(7, "gh", PrState::Open));
    }

    #[test]
    fn github_approved_open_pr_is_approved() {
        let prs = [gh_pr(7, "OPEN", BRANCH, "APPROVED", "2026-01-02T00:00:00Z")];
        assert_eq!(gh_lookup(&prs), found(7, "gh", PrState::Approved));
    }

    #[test]
    fn github_merged_pr_is_merged_even_if_it_was_approved() {
        let prs = [gh_pr(
            5,
            "MERGED",
            BRANCH,
            "APPROVED",
            "2026-01-02T00:00:00Z",
        )];
        assert_eq!(gh_lookup(&prs), found(5, "gh", PrState::Merged));
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
        assert_eq!(gh_lookup(&prs), found(8, "gh", PrState::Open));
    }

    #[test]
    fn github_latest_merged_when_none_open() {
        let prs = [
            gh_pr(4, "MERGED", BRANCH, "", "2026-01-01T00:00:00Z"),
            gh_pr(6, "MERGED", BRANCH, "", "2026-02-01T00:00:00Z"),
            gh_pr(5, "MERGED", BRANCH, "", "2026-01-15T00:00:00Z"),
        ];
        assert_eq!(gh_lookup(&prs), found(6, "gh", PrState::Merged));
    }

    #[test]
    fn github_two_open_exact_head_then_latest_updated() {
        let prs = [
            gh_pr(11, "OPEN", "Feature/Login", "", "2026-05-01T00:00:00Z"),
            gh_pr(12, "OPEN", BRANCH, "", "2026-02-01T00:00:00Z"),
            gh_pr(13, "OPEN", BRANCH, "APPROVED", "2026-03-01T00:00:00Z"),
        ];
        assert_eq!(gh_lookup(&prs), found(13, "gh", PrState::Approved));
        let prs = [
            gh_pr(12, "OPEN", "other", "", "2026-02-01T00:00:00Z"),
            gh_pr(14, "OPEN", "other", "", "2026-04-01T00:00:00Z"),
        ];
        assert_eq!(gh_lookup(&prs), found(14, "gh", PrState::Open));
    }

    #[test]
    fn github_fork_pr_with_the_same_branch_name_does_not_count() {
        let fork = gh_pr(15, "OPEN", BRANCH, "APPROVED", "2026-06-01T00:00:00Z").replace(
            r#""isCrossRepository":false"#,
            r#""isCrossRepository":true"#,
        );
        assert_eq!(gh_lookup(std::slice::from_ref(&fork)), PrLookup::NoPr);
        let prs = [fork, gh_pr(6, "MERGED", BRANCH, "", "2026-01-01T00:00:00Z")];
        assert_eq!(gh_lookup(&prs), found(6, "gh", PrState::Merged));
    }

    #[test]
    fn runner_error_and_bad_json_are_failed() {
        assert_eq!(
            lookup(Some(GH_REMOTE), BRANCH, &|_: &[String]| Err(())),
            PrLookup::Failed
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
                PrLookup::Failed,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn no_remote_and_unsupported_forge_are_no_pr_without_running_a_cli() {
        let run = |_: &[String]| -> Result<String, ()> { panic!("no CLI call expected") };
        assert_eq!(lookup(None, BRANCH, &run), PrLookup::NoPr);
        assert_eq!(
            lookup(Some("git@bitbucket.org:octo/demo.git"), BRANCH, &run),
            PrLookup::NoPr
        );
    }

    // ---- lookup: GitLab ----

    const GL_REMOTE: &str = "https://gitlab.com/group/sub/demo.git";

    fn gl_mr(iid: u64, state: &str, updated: &str) -> String {
        format!(
            r#"{{"iid":{iid},"state":"{state}","web_url":"https://gitlab.com/group/sub/demo/-/merge_requests/{iid}","source_branch":"{BRANCH}","source_project_id":7,"target_project_id":7,"updated_at":"{updated}"}}"#
        )
    }

    /// Runs a GitLab lookup; `approvals` answers the approvals endpoint.
    fn gl_lookup(mrs: &[String], approvals: Result<&str, ()>) -> (PrLookup, Vec<Vec<String>>) {
        let list = format!("[{}]", mrs.join(","));
        let calls = RefCell::new(Vec::new());
        let result = lookup(Some(GL_REMOTE), BRANCH, &|argv: &[String]| {
            calls.borrow_mut().push(argv.to_vec());
            let endpoint = argv.last().expect("endpoint");
            if endpoint.ends_with("/approvals") {
                approvals.map(str::to_string)
            } else {
                Ok(list.clone())
            }
        });
        (result, calls.into_inner())
    }

    #[test]
    fn gitlab_open_mr_asks_approvals_and_maps_state() {
        let r = repo(Forge::GitLab, "gitlab.com", "group/sub/demo");
        let mrs = [gl_mr(21, "opened", "2026-01-02T00:00:00.000Z")];
        let (result, calls) = gl_lookup(&mrs, Ok(r#"{"approved":true}"#));
        assert_eq!(result, found(21, "gl", PrState::Approved));
        assert_eq!(
            calls,
            vec![gitlab_list_argv(&r, BRANCH), gitlab_approvals_argv(&r, 21)]
        );
        let (result, _) = gl_lookup(&mrs, Ok(r#"{"approved":false}"#));
        assert_eq!(result, found(21, "gl", PrState::Open));
    }

    #[test]
    fn gitlab_approvals_failure_or_missing_field_stays_open() {
        let mrs = [gl_mr(21, "opened", "2026-01-02T00:00:00.000Z")];
        for approvals in [
            Err(()),
            Ok("{}"),
            Ok("not json"),
            Ok(r#"{"approved":null}"#),
        ] {
            let (result, _) = gl_lookup(&mrs, approvals);
            assert_eq!(result, found(21, "gl", PrState::Open), "{approvals:?}");
        }
    }

    #[test]
    fn gitlab_merged_skips_approvals_and_open_wins() {
        let mrs = [gl_mr(20, "merged", "2026-03-01T00:00:00.000Z")];
        let (result, calls) = gl_lookup(&mrs, Ok(r#"{"approved":true}"#));
        assert_eq!(result, found(20, "gl", PrState::Merged));
        assert_eq!(calls.len(), 1, "no approvals call for a merged MR");

        let mrs = [
            gl_mr(20, "merged", "2026-03-01T00:00:00.000Z"),
            gl_mr(22, "opened", "2026-01-01T00:00:00.000Z"),
            gl_mr(23, "opened", "2026-02-01T00:00:00.000Z"),
        ];
        let (result, _) = gl_lookup(&mrs, Err(()));
        assert_eq!(result, found(23, "gl", PrState::Open));
    }

    #[test]
    fn gitlab_closed_locked_and_empty_are_no_pr() {
        let mrs = [
            gl_mr(24, "closed", "2026-01-01T00:00:00.000Z"),
            gl_mr(25, "locked", "2026-01-02T00:00:00.000Z"),
        ];
        assert_eq!(gl_lookup(&mrs, Err(())).0, PrLookup::NoPr);
        assert_eq!(gl_lookup(&[], Err(())).0, PrLookup::NoPr);
    }

    #[test]
    fn gitlab_fork_mr_with_the_same_branch_name_does_not_count() {
        let fork = gl_mr(26, "opened", "2026-06-01T00:00:00.000Z")
            .replace(r#""source_project_id":7"#, r#""source_project_id":99"#);
        let (result, calls) = gl_lookup(std::slice::from_ref(&fork), Ok(r#"{"approved":true}"#));
        assert_eq!(result, PrLookup::NoPr);
        assert_eq!(calls.len(), 1, "no approvals call for a fork MR");
        let mrs = [fork, gl_mr(20, "merged", "2026-01-01T00:00:00.000Z")];
        assert_eq!(gl_lookup(&mrs, Err(())).0, found(20, "gl", PrState::Merged));
    }

    #[test]
    fn gitlab_list_failure_is_failed() {
        let result = lookup(Some(GL_REMOTE), BRANCH, &|_: &[String]| Err(()));
        assert_eq!(result, PrLookup::Failed);
        let result = lookup(Some(GL_REMOTE), BRANCH, &|_: &[String]| {
            Ok(r#"{"message":"404 Project Not Found"}"#.to_string())
        });
        assert_eq!(result, PrLookup::Failed);
    }

    // ---- lookup_detail ----

    #[test]
    fn detail_argv_names_repo_number_and_fields() {
        let r = repo(Forge::GitHub, "github.com", "octo/demo");
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
                "number,title,state,isDraft,author,headRefName,baseRefName,\
                 reviewDecision,statusCheckRollup,updatedAt",
            ])
        );
        let r = repo(Forge::GitLab, "gitlab.example.com", "group/sub/demo");
        assert_eq!(
            gitlab_mr_argv(&r, 42),
            strings(&[
                "glab",
                "api",
                "--hostname",
                "gitlab.example.com",
                "projects/group%2Fsub%2Fdemo/merge_requests/42",
            ])
        );
    }

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
            PrDetailLookup::Failed => panic!("detail expected"),
        }
    }

    #[test]
    fn github_detail_reads_every_field_and_buckets_checks() {
        let json = r#"{"number":7,"title":"Add login","state":"OPEN","isDraft":false,
            "author":{"login":"octocat"},"headRefName":"feature/login","baseRefName":"main",
            "reviewDecision":"CHANGES_REQUESTED","updatedAt":"2026-01-02T00:00:00Z",
            "url":"https://github.com/octo/demo/pull/7","statusCheckRollup":[
              {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SUCCESS"},
              {"__typename":"CheckRun","status":"COMPLETED","conclusion":"SKIPPED"},
              {"__typename":"CheckRun","status":"COMPLETED","conclusion":"NEUTRAL"},
              {"__typename":"CheckRun","status":"COMPLETED","conclusion":"FAILURE"},
              {"__typename":"CheckRun","status":"COMPLETED","conclusion":"TIMED_OUT"},
              {"__typename":"CheckRun","status":"IN_PROGRESS","conclusion":""},
              {"__typename":"CheckRun","status":"QUEUED","conclusion":null},
              {"__typename":"StatusContext","state":"SUCCESS"},
              {"__typename":"StatusContext","state":"ERROR"},
              {"__typename":"StatusContext","state":"PENDING"}]}"#;
        assert_eq!(
            detail(gh_detail(json)),
            PullRequestDetail {
                number: 7,
                title: "Add login".into(),
                state: "OPEN".into(),
                draft: false,
                author: Some("octocat".into()),
                head: Some("feature/login".into()),
                base: Some("main".into()),
                review: Some(PrReview::ChangesRequested),
                checks: ChecksSummary {
                    pass: 4,
                    fail: 3,
                    pending: 3,
                },
                updated_at: Some(1_767_312_000),
            }
        );
    }

    #[test]
    fn github_detail_tolerates_null_and_missing_fields() {
        let partial = detail(gh_detail(
            r#"{"number":7,"state":"MERGED","isDraft":true,"author":null,
                "reviewDecision":null,"statusCheckRollup":null,"updatedAt":null}"#,
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
                checks: ChecksSummary::default(),
                updated_at: None,
            }
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
    fn detail_failures_are_failed() {
        assert_eq!(
            lookup_detail(GH_REMOTE, 7, &|_: &[String]| Err(())),
            PrDetailLookup::Failed
        );
        for bad in [
            "",
            "not json",
            "[]",
            r#"{"title":"x"}"#,
            r#"{"number":"7"}"#,
        ] {
            assert_eq!(gh_detail(bad), PrDetailLookup::Failed, "{bad:?}");
        }
        let run = |_: &[String]| -> Result<String, ()> { panic!("no CLI call expected") };
        assert_eq!(
            lookup_detail("git@bitbucket.org:octo/demo.git", 7, &run),
            PrDetailLookup::Failed
        );
        assert_eq!(
            lookup_detail(GL_REMOTE, 21, &|_: &[String]| Err(())),
            PrDetailLookup::Failed
        );
    }

    /// Runs a GitLab detail fetch of MR 21; `approvals` answers the
    /// approvals endpoint.
    fn gl_detail(json: &str, approvals: Result<&str, ()>) -> (PrDetailLookup, Vec<Vec<String>>) {
        let calls = RefCell::new(Vec::new());
        let result = lookup_detail(GL_REMOTE, 21, &|argv: &[String]| {
            calls.borrow_mut().push(argv.to_vec());
            if argv.last().expect("endpoint").ends_with("/approvals") {
                approvals.map(str::to_string)
            } else {
                Ok(json.to_string())
            }
        });
        (result, calls.into_inner())
    }

    #[test]
    fn gitlab_detail_reads_the_mr_and_asks_approvals_when_open() {
        let r = repo(Forge::GitLab, "gitlab.com", "group/sub/demo");
        let json = r#"{"iid":21,"title":"Add login","state":"opened","draft":true,
            "author":{"username":"octo"},"source_branch":"feature/login","target_branch":"main",
            "head_pipeline":{"status":"running"},"updated_at":"2026-03-04T13:06:07.000+08:00",
            "web_url":"https://gitlab.com/group/sub/demo/-/merge_requests/21"}"#;
        let (result, calls) = gl_detail(json, Ok(r#"{"approved":true}"#));
        assert_eq!(
            calls,
            vec![gitlab_mr_argv(&r, 21), gitlab_approvals_argv(&r, 21)]
        );
        assert_eq!(
            detail(result),
            PullRequestDetail {
                number: 21,
                title: "Add login".into(),
                state: "OPEN".into(),
                draft: true,
                author: Some("octo".into()),
                head: Some("feature/login".into()),
                base: Some("main".into()),
                review: Some(PrReview::Approved),
                checks: ChecksSummary {
                    pass: 0,
                    fail: 0,
                    pending: 1,
                },
                updated_at: Some(1_772_600_767),
            }
        );
        let (result, _) = gl_detail(json, Err(()));
        assert_eq!(detail(result).review, Some(PrReview::NotApproved));
    }

    #[test]
    fn gitlab_detail_of_a_merged_mr_skips_approvals_and_tolerates_nulls() {
        let (result, calls) = gl_detail(
            r#"{"iid":21,"state":"merged","work_in_progress":true,"author":null,
                "head_pipeline":null,"updated_at":null}"#,
            Ok(r#"{"approved":true}"#),
        );
        assert_eq!(calls.len(), 1, "no approvals call for a merged MR");
        let merged = detail(result);
        assert_eq!(merged.state, "MERGED");
        assert!(merged.draft, "older work_in_progress flag");
        assert_eq!(merged.review, None);
        assert_eq!(merged.author, None);
        assert_eq!(merged.checks, ChecksSummary::default());
        assert_eq!(merged.updated_at, None);
    }

    #[test]
    fn gitlab_pipeline_status_maps_to_one_bucket() {
        let bucket = |status: &str| {
            let json =
                format!(r#"{{"iid":21,"state":"merged","head_pipeline":{{"status":"{status}"}}}}"#);
            detail(gl_detail(&json, Err(())).0).checks
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
        for status in ["running", "pending", "created", "manual", "scheduled"] {
            assert_eq!(bucket(status), one(0, 0, 1), "{status}");
        }
        assert_eq!(bucket(""), ChecksSummary::default());
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
        assert_eq!(run_cli(&sh("printf x; exit 3"), &live), Err(()));
        assert_eq!(
            run_cli(&strings(&["ws-no-such-program-for-tests"]), &live),
            Err(())
        );
        assert_eq!(run_cli(&[], &live), Err(()));
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
        assert_eq!(out, Err(()));
        assert!(started.elapsed() < Duration::from_secs(3), "not bounded");
    }

    #[cfg(unix)]
    #[test]
    fn run_cli_with_cancel_set_fails_without_starting() {
        let started = Instant::now();
        let out = run_cli(&sh("exec sleep 5"), &AtomicBool::new(true));
        assert_eq!(out, Err(()));
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
        assert_eq!(out, Err(()));
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

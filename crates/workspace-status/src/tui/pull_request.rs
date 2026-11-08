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

/// Status line when a PR action targets a checkout on its repository's
/// default branch `branch`, which receives PRs and carries none: `main is
/// the default branch: no PR`.
pub fn default_branch_status(branch: &str) -> String {
    format!("{branch} is the default branch: no PR")
}

/// Status line when the forge CLI could not answer for `branch`, with its
/// `reason` when it gave one ([`run_cli`]): `could not look up PR for
/// feature: gh not found`.
pub fn lookup_failed_status(branch: &str, reason: Option<&str>) -> String {
    match reason {
        Some(reason) => format!("could not look up PR for {branch}: {reason}"),
        None => format!("could not look up PR for {branch}"),
    }
}

/// Status line when the browser could not be started.
pub const OPEN_FAILED: &str = "could not open PR";

/// Status line when Refresh this PR finds a lookup of that PR already
/// queued or running.
pub const PR_LOOKUP_RUNNING: &str = "PR lookup already running";

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
    /// address review, update the branch, or merge (the base first, or this
    /// PR). The stack does not change it.
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
/// trailing slash. An SSH port is dropped because it is not the web port. An
/// SSH host alias of a public forge (`github.com-work`, see
/// [`ssh_alias_forge_host`]) is read as that forge's host. A host that
/// contains `github` is GitHub (exactly `owner/repo`); one that contains
/// `gitlab` is GitLab (subgroups allowed). Anything else is `None`.
pub fn parse_remote(url: &str) -> Option<ForgeRepo> {
    let url = url.trim();
    let (host, path, ssh) = match url.split_once("://") {
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
            (host, path, scheme != "git" && !web)
        }
        None => {
            let (left, path) = url.split_once(':')?;
            if left.contains('/') {
                return None;
            }
            (left.rsplit_once('@').map_or(left, |(_, h)| h), path, true)
        }
    };
    let mut host = host.to_ascii_lowercase();
    if host.is_empty() || host.contains(char::is_whitespace) {
        return None;
    }
    if ssh {
        if let Some(forge_host) = ssh_alias_forge_host(&host) {
            host = forge_host.to_string();
        }
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

/// Public forge hosts whose SSH aliases [`ssh_alias_forge_host`] resolves.
const PUBLIC_FORGE_HOSTS: [&str; 2] = ["github.com", "gitlab.com"];

/// The public forge host behind an SSH host alias, or `None`.
///
/// Users with two accounts on one forge add an `~/.ssh/config` entry such
/// as `Host github.com-work` with `HostName github.com`, so each remote
/// picks its own key. `gh` / `glab` know only the real host. A lower-case
/// `host` that is a public forge host followed by `-` and a suffix is read
/// as that forge host. `~/.ssh/config` itself is not read: the rule needs no
/// process and works the same on every machine.
fn ssh_alias_forge_host(host: &str) -> Option<&'static str> {
    PUBLIC_FORGE_HOSTS.into_iter().find(|forge| {
        host.strip_prefix(forge)
            .and_then(|rest| rest.strip_prefix('-'))
            .is_some_and(|alias| !alias.is_empty())
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
/// requested on `reviewDecision: CHANGES_REQUESTED`, behind on
/// `mergeStateStatus: BEHIND` with `reviewDecision: APPROVED`. None unless
/// `open`.
fn github_notes(status: &GitHubStatus, open: bool) -> PrNotes {
    if !open {
        return PrNotes::default();
    }
    let merge_state = status.merge_state_status.as_deref();
    let decision = status.review_decision.as_deref();
    PrNotes {
        conflicts: merge_state == Some("DIRTY"),
        unresolved_threads: false,
        changes_requested: decision == Some("CHANGES_REQUESTED"),
        behind: merge_state == Some("BEHIND") && decision == Some("APPROVED"),
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
/// `detailed_merge_status: requested_changes`; behind on
/// `detailed_merge_status: need_rebase` with `approvals_left == Some(0)`
/// (GitLab reports `need_rebase` before `not_approved`, so the status alone
/// does not prove approval). None unless `open`.
fn gitlab_notes(merge: &GitLabMerge, open: bool, approvals_left: Option<u64>) -> PrNotes {
    if !open {
        return PrNotes::default();
    }
    let detailed = merge.detailed();
    PrNotes {
        conflicts: merge.has_conflicts == Some(true) || detailed == Some("conflict"),
        unresolved_threads: detailed == Some("discussions_not_resolved")
            || merge.blocking_discussions_resolved == Some(false),
        changes_requested: detailed == Some("requested_changes"),
        behind: detailed == Some("need_rebase") && approvals_left == Some(0),
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
/// neither answer has a `detailed_merge_status` or the status is
/// `need_rebase`, the approvals call.
///
/// The merge fields (`detailed_merge_status`, `has_conflicts`,
/// `blocking_discussions_resolved`, `draft`) of an open MR come from the MR
/// call, which is fresher than the list (the list can still say
/// `unchecked`); a field the MR call leaves out comes from the list. A
/// failed MR call fails the lookup. A failed approvals call leaves the MR
/// not ready, and a `need_rebase` MR not behind ([`PrNotes::behind`]).
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
        // `need_rebase` comes before `not_approved` in GitLab's checks, so
        // only the approvals call says whether the MR is approved.
        if matches!(merge.detailed(), None | Some("need_rebase")) {
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
        notes: gitlab_notes(&merge, kind == Kind::Open, approvals_left),
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
/// `detailed_merge_status` or sends `need_rebase`.
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
    let mut approvals_left = None;
    if open {
        let approvals = gitlab_approvals(repo, mr.iid, run);
        approvals_left = approvals.as_ref().and_then(|a| a.approvals_left);
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
        notes: gitlab_notes(&mr.merge, open, approvals_left),
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
/// `None` on timeout, on cancel, or when its state cannot be read. The
/// forge CLI runs and the PR title command (`tui/pr_groups.rs`) wait here.
pub(crate) fn wait_until(
    child: &mut Child,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Option<ExitStatus> {
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
/// Browser openers and detached GUI editors (`tui/editor.rs`) use it.
pub(crate) fn reap_in_background(mut child: Child) {
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
/// 200 characters ([`first_stderr_line`]), or `timed out after 20 s` when
/// the time limit killed it. A cancel or a silent failure has no reason.
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

/// Failure reason of a CLI killed by the time limit `limit`:
/// `timed out after 20 s` (`timed out after 200 ms` below a whole second).
fn timed_out_reason(limit: Duration) -> String {
    if limit.subsec_millis() == 0 {
        format!("timed out after {} s", limit.as_secs())
    } else {
        format!("timed out after {} ms", limit.as_millis())
    }
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
        // The time limit names itself; a cancel (quit) says nothing.
        let timed_out = !cancel.load(Ordering::Relaxed) && Instant::now() >= deadline;
        return Err(timed_out.then(|| timed_out_reason(limit)));
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
mod tests;

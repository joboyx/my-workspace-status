//! Fake forge answers for the six PR badge states, on GitHub and on GitLab.
//!
//! The JSON follows what `gh pr list` / `gh pr view` and the GitLab merge
//! request API print for each state. [`put_github`] and [`put_gitlab`] write
//! every answer the app asks for into a [`FakeForge`], so one call moves a
//! checkout to a state, also in the middle of a run.

use super::fake_forge::{FakeForge, Reply};

/// Repository the GitHub checkouts of the e2e fixtures point at.
pub const GITHUB_REPO: &str = "acme/app";

/// Project the GitLab checkouts of the e2e fixtures point at.
pub const GITLAB_PROJECT: &str = "acme/services/api";

/// Failed check of a GitHub PR: the second check, after `lint`.
pub const FAILED_CHECK: &str = "test (ubuntu-latest)";

/// One of the six badge states of a PR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// A check failed, or the head pipeline failed.
    CiFailed,
    /// Mergeable now.
    Ready,
    /// A check or the head pipeline still runs.
    CiRunning,
    /// Draft.
    Draft,
    /// Open, with nothing else to say.
    InReview,
    /// Merged.
    Merged,
}

impl Badge {
    /// Every state, in the order of the state table in `docs/tui-rust.md`
    /// (**Pull request badge**): CI failed, ready, CI running, draft, in
    /// review, merged.
    pub const ALL: [Badge; 6] = [
        Self::CiFailed,
        Self::Ready,
        Self::CiRunning,
        Self::Draft,
        Self::InReview,
        Self::Merged,
    ];

    /// Glyph in ASCII mode (the PTY harness default).
    pub fn ascii(self) -> char {
        match self {
            Self::CiFailed => 'x',
            Self::Ready => 'A',
            Self::CiRunning => 'r',
            Self::Draft => 'd',
            Self::InReview => 'P',
            Self::Merged => 'm',
        }
    }

    /// Glyph in Nerd mode.
    pub fn nerd(self) -> char {
        match self {
            Self::CiFailed => '\u{2715}',
            Self::Ready => '\u{25C9}',
            Self::CiRunning => '\u{25D0}',
            Self::Draft => '\u{25CC}',
            Self::InReview => '\u{25CB}',
            Self::Merged => '\u{2713}',
        }
    }

    /// Short name for fixture directory and branch names.
    pub fn slug(self) -> &'static str {
        match self {
            Self::CiFailed => "failed",
            Self::Ready => "ready",
            Self::CiRunning => "running",
            Self::Draft => "draft",
            Self::InReview => "review",
            Self::Merged => "merged",
        }
    }
}

/// URL of the PR page of GitHub PR `number`.
pub fn github_pr_url(number: u64) -> String {
    format!("https://github.com/{GITHUB_REPO}/pull/{number}")
}

/// URL of check `index` (1 = `lint`, 2 = the test check) of GitHub PR
/// `number`.
pub fn github_check_url(number: u64, index: u64) -> String {
    format!("https://github.com/{GITHUB_REPO}/actions/runs/{number}/job/{number}{index}")
}

/// URL of the merge request page of GitLab merge request `iid`.
pub fn gitlab_mr_url(iid: u64) -> String {
    format!("https://gitlab.com/{GITLAB_PROJECT}/-/merge_requests/{iid}")
}

/// Id of the head pipeline of GitLab merge request `iid`.
pub fn gitlab_pipeline_id(iid: u64) -> u64 {
    9000 + iid
}

/// URL of the head pipeline of GitLab merge request `iid`.
pub fn gitlab_pipeline_url(iid: u64) -> String {
    format!(
        "https://gitlab.com/{GITLAB_PROJECT}/-/pipelines/{}",
        gitlab_pipeline_id(iid)
    )
}

/// One `statusCheckRollup` entry of a GitHub check run.
fn check_run(number: u64, index: u64, name: &str, status: &str, conclusion: &str) -> String {
    format!(
        r#"{{"__typename":"CheckRun","completedAt":"2026-10-29T00:50:00Z","conclusion":"{conclusion}","detailsUrl":"{}","name":"{name}","startedAt":"2026-10-29T00:48:00Z","status":"{status}","workflowName":"CI"}}"#,
        github_check_url(number, index)
    )
}

/// The checks of a GitHub PR in `badge`: `lint` always passes, the test
/// check fails, runs, or passes.
fn github_checks(number: u64, badge: Badge) -> String {
    let lint = check_run(number, 1, "lint", "COMPLETED", "SUCCESS");
    let test = match badge {
        Badge::CiFailed => check_run(number, 2, FAILED_CHECK, "COMPLETED", "FAILURE"),
        Badge::CiRunning => check_run(number, 2, FAILED_CHECK, "IN_PROGRESS", ""),
        _ => check_run(number, 2, FAILED_CHECK, "COMPLETED", "SUCCESS"),
    };
    format!("[{lint},{test}]")
}

/// `(isDraft, state, reviewDecision, mergeStateStatus)` of a GitHub PR.
fn github_facts(badge: Badge) -> (bool, &'static str, &'static str, &'static str) {
    match badge {
        Badge::CiFailed => (false, "OPEN", "REVIEW_REQUIRED", "BLOCKED"),
        Badge::Ready => (false, "OPEN", "APPROVED", "CLEAN"),
        Badge::CiRunning => (false, "OPEN", "APPROVED", "BLOCKED"),
        Badge::Draft => (true, "OPEN", "REVIEW_REQUIRED", "BLOCKED"),
        Badge::InReview => (false, "OPEN", "REVIEW_REQUIRED", "BLOCKED"),
        Badge::Merged => (false, "MERGED", "APPROVED", "UNKNOWN"),
    }
}

/// `gh pr list --head <branch>` answer: one PR in `badge`.
pub fn github_list_json(number: u64, branch: &str, badge: Badge) -> String {
    let (draft, state, review, merge) = github_facts(badge);
    format!(
        r#"[{{"number":{number},"state":"{state}","url":"{}","title":"Refresh auth tokens","headRefName":"{branch}","baseRefName":"main","isCrossRepository":false,"isDraft":{draft},"reviewDecision":"{review}","mergeStateStatus":"{merge}","statusCheckRollup":{},"updatedAt":"2026-10-29T01:00:00Z"}}]"#,
        github_pr_url(number),
        github_checks(number, badge)
    )
}

/// `gh pr view <number>` answer: the PR in `badge`, with its reviews.
pub fn github_view_json(number: u64, branch: &str, badge: Badge) -> String {
    let (draft, state, review, merge) = github_facts(badge);
    let reviews = if review == "APPROVED" {
        r#"[{"author":{"login":"octo-reviewer"},"state":"APPROVED","submittedAt":"2026-10-29T00:40:00Z"}]"#
    } else {
        "[]"
    };
    format!(
        r#"{{"number":{number},"title":"Refresh auth tokens","state":"{state}","isDraft":{draft},"author":{{"login":"octo-user"}},"headRefName":"{branch}","baseRefName":"main","reviewDecision":"{review}","mergeStateStatus":"{merge}","statusCheckRollup":{},"latestReviews":{reviews},"updatedAt":"2026-10-29T01:00:00Z"}}"#,
        github_checks(number, badge)
    )
}

/// Make every `gh` answer for PR `number` on `branch` in `repo` the PR in
/// `badge`.
pub fn put_github(forge: &FakeForge, repo: &str, number: u64, branch: &str, badge: Badge) {
    forge.github_pr_list(
        repo,
        branch,
        Reply::Json(&github_list_json(number, branch, badge)),
    );
    forge.github_pr_view(
        repo,
        number,
        Reply::Json(&github_view_json(number, branch, badge)),
    );
}

/// `(draft, state, detailed_merge_status, pipeline status)` of a GitLab
/// merge request; no pipeline status means no head pipeline.
fn gitlab_facts(badge: Badge) -> (bool, &'static str, &'static str, &'static str) {
    match badge {
        Badge::CiFailed => (false, "opened", "ci_must_pass", "failed"),
        Badge::Ready => (false, "opened", "mergeable", "success"),
        Badge::CiRunning => (false, "opened", "ci_still_running", "running"),
        Badge::Draft => (true, "opened", "draft_status", "success"),
        Badge::InReview => (false, "opened", "not_approved", "success"),
        Badge::Merged => (false, "merged", "not_open", "success"),
    }
}

/// The merge request object, with the head pipeline when `pipeline`.
fn gitlab_mr_json(iid: u64, branch: &str, badge: Badge, pipeline: bool) -> String {
    let (draft, state, merge, ci) = gitlab_facts(badge);
    let head = if pipeline {
        format!(
            r#","head_pipeline":{{"id":{},"status":"{ci}","web_url":"{}"}}"#,
            gitlab_pipeline_id(iid),
            gitlab_pipeline_url(iid)
        )
    } else {
        String::new()
    };
    format!(
        r#"{{"iid":{iid},"state":"{state}","title":"Rate limit the public API","draft":{draft},"source_branch":"{branch}","target_branch":"main","web_url":"{}","source_project_id":1,"target_project_id":1,"has_conflicts":false,"blocking_discussions_resolved":true,"detailed_merge_status":"{merge}","merge_status":"can_be_merged","author":{{"username":"octo-user"}},"updated_at":"2026-10-29T01:00:00Z"{head}}}"#,
        gitlab_mr_url(iid)
    )
}

/// Make every `glab api` answer for merge request `iid` on `branch` in
/// `project` the merge request in `badge`.
///
/// The list answer has no `head_pipeline`, like the real list: the badge
/// reads the pipeline and the merge facts from the single-merge-request
/// answer.
pub fn put_gitlab(forge: &FakeForge, project: &str, iid: u64, branch: &str, badge: Badge) {
    let (_, _, _, ci) = gitlab_facts(badge);
    forge.gitlab_mr_list(
        project,
        branch,
        Reply::Json(&format!("[{}]", gitlab_mr_json(iid, branch, badge, false))),
    );
    forge.gitlab_mr(
        project,
        iid,
        Reply::Json(&gitlab_mr_json(iid, branch, badge, true)),
    );
    let approved = matches!(badge, Badge::Ready | Badge::CiRunning | Badge::Merged);
    forge.gitlab_approvals(
        project,
        iid,
        Reply::Json(if approved {
            r#"{"approved":true,"approvals_required":1,"approvals_left":0,"approved_by":[{"user":{"username":"octo-reviewer"}}]}"#
        } else {
            r#"{"approved":false,"approvals_required":2,"approvals_left":2,"approved_by":[]}"#
        }),
    );
    let jobs = match ci {
        "failed" => format!(
            r#"[{{"id":3{iid},"name":"test","stage":"test","status":"failed","web_url":"https://gitlab.com/{GITLAB_PROJECT}/-/jobs/3{iid}"}}]"#
        ),
        "running" => format!(
            r#"[{{"id":3{iid},"name":"test","stage":"test","status":"running","web_url":"https://gitlab.com/{GITLAB_PROJECT}/-/jobs/3{iid}"}}]"#
        ),
        _ => "[]".to_string(),
    };
    forge.gitlab_pipeline_jobs(project, gitlab_pipeline_id(iid), Reply::Json(&jobs));
}

/// Make every `gh` answer for PR `number` on `branch` in `repo` a closed,
/// unmerged PR. The badge treats it as no PR.
pub fn put_github_closed(forge: &FakeForge, repo: &str, number: u64, branch: &str) {
    let list = github_list_json(number, branch, Badge::InReview)
        .replace(r#""state":"OPEN""#, r#""state":"CLOSED""#);
    forge.github_pr_list(repo, branch, Reply::Json(&list));
    let view = github_view_json(number, branch, Badge::InReview)
        .replace(r#""state":"OPEN""#, r#""state":"CLOSED""#);
    forge.github_pr_view(repo, number, Reply::Json(&view));
}

/// Make every `glab api` answer for merge request `iid` on `branch` in
/// `project` a closed, unmerged merge request. The badge treats it as no PR.
pub fn put_gitlab_closed(forge: &FakeForge, project: &str, iid: u64, branch: &str) {
    let mr = gitlab_mr_json(iid, branch, Badge::InReview, true)
        .replace(r#""state":"opened""#, r#""state":"closed""#);
    let list = gitlab_mr_json(iid, branch, Badge::InReview, false)
        .replace(r#""state":"opened""#, r#""state":"closed""#);
    forge.gitlab_mr_list(project, branch, Reply::Json(&format!("[{list}]")));
    forge.gitlab_mr(project, iid, Reply::Json(&mr));
}

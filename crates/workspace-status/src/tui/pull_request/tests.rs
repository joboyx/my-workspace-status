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
fn parse_remote_ssh_alias_of_a_public_forge_maps_to_the_forge_host() {
    for url in [
        "git@github.com-work:octo/demo.git",
        "ssh://git@github.com-work/octo/demo.git",
        "git@GitHub.com-Work:octo/demo",
    ] {
        assert_eq!(
            parse_remote(url),
            Some(repo(Forge::GitHub, "github.com", "octo/demo")),
            "{url}"
        );
    }
    assert_eq!(
        parse_remote("git@gitlab.com-work:group/sub/demo.git"),
        Some(repo(Forge::GitLab, "gitlab.com", "group/sub/demo"))
    );
    // Only the ssh forms: an https host is a real DNS name.
    assert_eq!(
        parse_remote("https://github.com-work/octo/demo.git"),
        Some(repo(Forge::GitHub, "github.com-work", "octo/demo"))
    );
    // Other hosts keep their name.
    for (url, host) in [
        ("git@github-work:octo/demo.git", "github-work"),
        ("git@github.example.com:octo/demo.git", "github.example.com"),
        (
            "git@github.com.example.com:octo/demo.git",
            "github.com.example.com",
        ),
    ] {
        assert_eq!(parse_remote(url).unwrap().host, host, "{url}");
    }
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
    let api =
        |endpoint: &str| strings(&["glab", "api", "--hostname", "gitlab.example.com", endpoint]);
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
            Some("update branch"),
            true,
        ),
        (
            gh_open(&[("mergeStateStatus", r#""BEHIND""#)]),
            Some("get review"),
            false,
        ),
        (
            gh_open(&[
                ("reviewDecision", r#""APPROVED""#),
                ("mergeStateStatus", r#""BEHIND""#),
                ("statusCheckRollup", &run),
            ]),
            Some("wait for CI"),
            false,
        ),
        (
            gh_open(&[
                ("reviewDecision", r#""APPROVED""#),
                ("mergeStateStatus", r#""BEHIND""#),
                ("statusCheckRollup", &fail),
            ]),
            Some("fix CI"),
            true,
        ),
        (
            gh_open(&[
                ("reviewDecision", r#""APPROVED""#),
                ("mergeStateStatus", r#""BEHIND""#),
                ("isDraft", "true"),
            ]),
            Some("undraft"),
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
fn github_behind_needs_an_approved_open_pr() {
    let behind = |entry: String| found_pr(gh_lookup(&[entry])).notes.behind;
    let approved = found_pr(gh_lookup(&[gh_open(&[
        ("reviewDecision", r#""APPROVED""#),
        ("mergeStateStatus", r#""BEHIND""#),
    ])]));
    assert!(approved.notes.behind);
    assert!(approved.notes.texts().is_empty(), "no note text");
    assert!(!behind(gh_open(&[("mergeStateStatus", r#""BEHIND""#)])));
    assert!(!behind(gh_open(&[
        ("reviewDecision", r#""APPROVED""#),
        ("mergeStateStatus", r#""BLOCKED""#),
    ])));
    let merged = json_with(
        &gh_pr(7, "MERGED", BRANCH, "APPROVED", "2026-01-02T00:00:00Z"),
        &[("mergeStateStatus", r#""BEHIND""#)],
    );
    assert!(!behind(merged), "a merged PR is never behind");
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
    let run = |_: &[String]| -> Result<String, Option<String>> { panic!("no CLI call expected") };
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

/// The approvals answer of an MR that needs 1 approval and has `left`
/// left.
fn gl_approvals(left: u64) -> String {
    format!(
        r#"{{"approved":{},"approvals_required":1,"approvals_left":{left},"approved_by":[]}}"#,
        left == 0
    )
}

#[test]
fn gitlab_missing_detailed_merge_status_falls_back_to_merge_status_and_approvals() {
    let old = |merge_status: &str| {
        json_without(
            &gl_open(&[("merge_status", &format!(r#""{merge_status}""#))]),
            "detailed_merge_status",
        )
    };
    let approvals = gl_approvals;
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
fn gitlab_need_rebase_runs_the_approvals_call_and_is_behind_only_when_approved() {
    let need_rebase = || GlForge::new(&[gl_open(&[("detailed_merge_status", r#""need_rebase""#)])]);
    let (result, calls) = need_rebase().approvals(&gl_approvals(0)).lookup();
    assert_eq!(calls, ["list", "mr", "approvals"]);
    let pr = found_pr(result);
    assert!(pr.notes.behind);
    assert!(pr.notes.texts().is_empty(), "no note text");
    assert_eq!(pr.state, PrState::InReview, "need_rebase is not ready");
    assert!(
        !need_rebase()
            .approvals(&gl_approvals(1))
            .found()
            .notes
            .behind
    );
    assert!(!need_rebase().found().notes.behind, "approvals call failed");
    let merged = json_with(
        &gl_mr(20, "merged", "2026-01-01T00:00:00.000Z"),
        &[("detailed_merge_status", r#""need_rebase""#)],
    );
    let (result, calls) = GlForge::new(&[merged]).approvals(&gl_approvals(0)).lookup();
    assert!(
        !found_pr(result).notes.behind,
        "a merged MR is never behind"
    );
    assert_eq!(calls, ["list"]);
}

#[test]
fn gitlab_next_action_rows() {
    let need_rebase = |fields: &[(&str, &str)]| {
        let mut fields = fields.to_vec();
        fields.push(("detailed_merge_status", r#""need_rebase""#));
        GlForge::new(&[gl_open(&fields)])
    };
    let cases = [
        (
            need_rebase(&[]).approvals(&gl_approvals(0)),
            Some("update branch"),
            true,
        ),
        (
            need_rebase(&[]).approvals(&gl_approvals(1)),
            Some("get review"),
            false,
        ),
        (need_rebase(&[]), Some("get review"), false),
        (
            need_rebase(&[])
                .approvals(&gl_approvals(0))
                .pipeline("running"),
            Some("wait for CI"),
            false,
        ),
        (
            need_rebase(&[("draft", "true")]).approvals(&gl_approvals(0)),
            Some("undraft"),
            false,
        ),
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
                behind: false,
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
    let no_author = r#"{"author":null,"state":"APPROVED","submittedAt":"2026-01-01T00:00:00Z"}"#;
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
    let run = |_: &[String]| -> Result<String, Option<String>> { panic!("no CLI call expected") };
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
                behind: false,
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
    let forge = GlForge::new(&[]).mr(Ok(r#"{"iid":21,"state":"merged","work_in_progress":true,
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
            &sh(
                "printf '\\033[31mHTTP 401:\\033[0m\\033[1;4m Bad\\tcredentials\\r\\n' >&2; exit 4"
            ),
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
    assert_eq!(out, Err(Some("timed out after 200 ms".to_string())));
    assert!(started.elapsed() < Duration::from_secs(3), "not bounded");
    assert_eq!(timed_out_reason(CLI_TIME_LIMIT), "timed out after 20 s");
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
    let run = |argv: &[&str], check_exit| spawn_opener(&opener(argv, check_exit), window, &live);
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
        lookup_failed_status("feature/login", None),
        "could not look up PR for feature/login"
    );
    assert_eq!(
        lookup_failed_status("feature/login", Some("timed out after 20 s")),
        "could not look up PR for feature/login: timed out after 20 s"
    );
    assert_eq!(OPEN_FAILED, "could not open PR");
}

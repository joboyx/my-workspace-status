//! PR status rules shared by GitHub and GitLab: the badge state ladder, the
//! next action, the CI link, and the named checks.
//!
//! Everything here is pure. [`super::pull_request`] reads the forge JSON into
//! the forge-neutral facts these rules take ([`StatusFacts`], [`Check`],
//! [`PrNotes`]); the badge, the popover, the alerts, and the PR list read the
//! answers. Each ladder is first match wins, in the order the variants are
//! declared.

/// Badge state of an open or merged PR, in ladder order.
///
/// [`badge_state`] picks the first state that matches. A closed PR that was
/// not merged has no state: it counts as no PR.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrState {
    /// Merged.
    Merged,
    /// A check failed (GitHub), or the head pipeline failed or was canceled
    /// (GitLab).
    CiFailed,
    /// Draft.
    Draft,
    /// A check is not finished (GitHub), or the head pipeline has not
    /// finished (GitLab).
    CiRunning,
    /// Mergeable now: the forge says so, and no check failed or runs.
    Ready,
    /// Any other open PR.
    InReview,
}

// Only tests read this until the PR badge, popover, and alert surfaces do.
#[cfg_attr(not(test), allow(dead_code))]
impl PrState {
    /// Lower-case state label: `merged`, `CI failed`, `draft`, `CI running`,
    /// `ready to merge`, or `in review`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Merged => "merged",
            Self::CiFailed => "CI failed",
            Self::Draft => "draft",
            Self::CiRunning => "CI running",
            Self::Ready => "ready to merge",
            Self::InReview => "in review",
        }
    }
}

/// Outcome class of one check, one job, or one head pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CheckBucket {
    /// Passed, neutral, or skipped.
    Pass,
    /// Failed, errored, cancelled, or timed out.
    Fail,
    /// Queued, running, or waiting.
    Pending,
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

impl ChecksSummary {
    /// Count one more check in `bucket`.
    pub fn add(&mut self, bucket: CheckBucket) {
        match bucket {
            CheckBucket::Pass => self.pass += 1,
            CheckBucket::Fail => self.fail += 1,
            CheckBucket::Pending => self.pending += 1,
        }
    }

    /// Counts of `checks`.
    pub fn of(checks: &[Check]) -> Self {
        let mut summary = Self::default();
        for check in checks {
            summary.add(check.bucket);
        }
        summary
    }
}

/// One GitHub check run or status context, or one GitLab job.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    /// Outcome class.
    pub bucket: CheckBucket,
    /// Check run name, status context, or job name; empty when the forge
    /// sent none.
    pub name: String,
    /// Web page of the check or job, when the forge sent one.
    pub url: Option<String>,
}

/// Forge-neutral facts that decide the badge state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatusFacts {
    /// The PR is merged.
    pub merged: bool,
    /// The PR is a draft.
    pub draft: bool,
    /// Checks of the PR head.
    pub checks: ChecksSummary,
    /// The forge's own "mergeable now" rule holds.
    pub ready: bool,
}

/// The badge state for `facts`: merged, CI failed, draft, CI running, ready
/// to merge, in review; first match wins.
///
/// CI running comes before ready to merge, so an approved PR whose checks
/// still run is not ready.
pub fn badge_state(facts: &StatusFacts) -> PrState {
    if facts.merged {
        PrState::Merged
    } else if facts.checks.fail > 0 {
        PrState::CiFailed
    } else if facts.draft {
        PrState::Draft
    } else if facts.checks.pending > 0 {
        PrState::CiRunning
    } else if facts.ready {
        PrState::Ready
    } else {
        PrState::InReview
    }
}

/// Merge and review blockers the forge reports on an open PR.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrNotes {
    /// GitHub `mergeStateStatus: DIRTY`; GitLab `has_conflicts`, or
    /// `detailed_merge_status: conflict`.
    pub conflicts: bool,
    /// GitLab `detailed_merge_status: discussions_not_resolved`, or
    /// `blocking_discussions_resolved: false`.
    pub unresolved_threads: bool,
    /// GitHub `reviewDecision: CHANGES_REQUESTED`; GitLab
    /// `detailed_merge_status: requested_changes`.
    pub changes_requested: bool,
}

// Only tests read this until the PR badge, popover, and alert surfaces do.
#[cfg_attr(not(test), allow(dead_code))]
impl PrNotes {
    /// `has conflicts`, `unresolved threads`, `changes requested`: the notes
    /// that hold, in that order.
    pub fn texts(self) -> Vec<&'static str> {
        [
            (self.conflicts, "has conflicts"),
            (self.unresolved_threads, "unresolved threads"),
            (self.changes_requested, "changes requested"),
        ]
        .into_iter()
        .filter_map(|(holds, text)| holds.then_some(text))
        .collect()
    }

    /// True when a reviewer's ask is open: changes requested or unresolved
    /// threads.
    fn review_asks(self) -> bool {
        self.unresolved_threads || self.changes_requested
    }
}

/// What the author of an open PR does next, in ladder order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NextAction {
    /// The PR is in [`PrState::CiFailed`].
    FixCi,
    /// [`PrNotes::conflicts`].
    FixConflicts,
    /// The PR is a draft.
    Undraft,
    /// Changes requested or unresolved threads.
    AddressReview,
    /// Ready, but the base branch is the head of another open PR; holds the
    /// ref of that PR (`#318`, `!12`).
    MergeBaseFirst(String),
    /// Ready to merge.
    Merge,
    /// CI still runs.
    WaitForCi,
    /// Anything else open.
    GetReview,
}

// Only tests read this until the PR badge, popover, and alert surfaces do.
#[cfg_attr(not(test), allow(dead_code))]
impl NextAction {
    /// Exact text: `fix CI`, `fix conflicts`, `undraft`, `address review`,
    /// `merge <base ref> first`, `merge`, `wait for CI`, or `get review`.
    pub fn text(&self) -> String {
        match self {
            Self::FixCi => "fix CI".into(),
            Self::FixConflicts => "fix conflicts".into(),
            Self::Undraft => "undraft".into(),
            Self::AddressReview => "address review".into(),
            Self::MergeBaseFirst(base) => format!("merge {base} first"),
            Self::Merge => "merge".into(),
            Self::WaitForCi => "wait for CI".into(),
            Self::GetReview => "get review".into(),
        }
    }

    /// True when the move is the author's: fix CI, fix conflicts, address
    /// review, merge the base first, or merge.
    pub fn needs_you(&self) -> bool {
        matches!(
            self,
            Self::FixCi
                | Self::FixConflicts
                | Self::AddressReview
                | Self::MergeBaseFirst(_)
                | Self::Merge
        )
    }
}

/// The next action of a PR in `state` with `notes`; `None` when merged.
///
/// `open_base_ref` is the ref of the open PR whose head branch is this PR's
/// base branch, if there is one: a ready PR then says `merge <ref> first`.
// Only tests read this until the PR badge, popover, and alert surfaces do.
#[cfg_attr(not(test), allow(dead_code))]
pub fn next_action(
    state: PrState,
    notes: PrNotes,
    open_base_ref: Option<&str>,
) -> Option<NextAction> {
    Some(match state {
        PrState::Merged => return None,
        PrState::CiFailed => NextAction::FixCi,
        _ if notes.conflicts => NextAction::FixConflicts,
        PrState::Draft => NextAction::Undraft,
        _ if notes.review_asks() => NextAction::AddressReview,
        PrState::Ready => match open_base_ref {
            Some(base) => NextAction::MergeBaseFirst(base.to_string()),
            None => NextAction::Merge,
        },
        PrState::CiRunning => NextAction::WaitForCi,
        PrState::InReview => NextAction::GetReview,
    })
}

/// What a CI link opens, which names its action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CiLinkKind {
    /// The failed GitHub check [`named_failed_check`] picks.
    FailedCheck,
    /// The first running GitHub check.
    RunningCheck,
    /// A GitHub check page, or the PR's checks tab (also when no failed
    /// check has a link).
    Checks,
    /// The GitLab head pipeline, or the MR's pipelines tab.
    Pipeline,
}

// Only tests read this until the PR badge, popover, and alert surfaces do.
#[cfg_attr(not(test), allow(dead_code))]
impl CiLinkKind {
    /// Action label: `Open failed check`, `Open running check`,
    /// `Open checks`, or `Open pipeline`.
    pub fn label(self) -> &'static str {
        match self {
            Self::FailedCheck => "Open failed check",
            Self::RunningCheck => "Open running check",
            Self::Checks => "Open checks",
            Self::Pipeline => "Open pipeline",
        }
    }
}

/// The CI page of a PR and what it shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CiLink {
    /// Web page to open.
    pub url: String,
    /// What the page shows; [`CiLinkKind::label`] names the action.
    pub kind: CiLinkKind,
}

/// The failed check a PR names (in an alert) and links (`Open failed
/// check`): the first failed check that has a link, else the first failed
/// check. `None` when no check failed.
///
/// [`checks_ci_link`] opens this same check, so the name and the link never
/// point at two different checks.
pub fn named_failed_check(checks: &[Check]) -> Option<&Check> {
    let failed = || {
        checks
            .iter()
            .filter(|check| check.bucket == CheckBucket::Fail)
    };
    failed()
        .find(|check| check.url.is_some())
        .or_else(|| failed().next())
}

/// GitHub CI link.
///
/// With a failed check: the link of [`named_failed_check`]
/// ([`CiLinkKind::FailedCheck`]), or `<pr_url>/checks` when no failed check
/// has a link. With no failed check: the first running check's link, else
/// the first check with a link, else `<pr_url>/checks`.
pub fn checks_ci_link(checks: &[Check], pr_url: &str) -> CiLink {
    let checks_tab = || CiLink {
        url: format!("{}/checks", pr_url.trim_end_matches('/')),
        kind: CiLinkKind::Checks,
    };
    if let Some(failed) = named_failed_check(checks) {
        return match &failed.url {
            Some(url) => CiLink {
                url: url.clone(),
                kind: CiLinkKind::FailedCheck,
            },
            None => checks_tab(),
        };
    }
    let first = |bucket: Option<CheckBucket>| {
        checks
            .iter()
            .filter(|check| bucket.is_none_or(|bucket| check.bucket == bucket))
            .find_map(|check| check.url.clone())
    };
    if let Some(url) = first(Some(CheckBucket::Pending)) {
        CiLink {
            url,
            kind: CiLinkKind::RunningCheck,
        }
    } else if let Some(url) = first(None) {
        CiLink {
            url,
            kind: CiLinkKind::Checks,
        }
    } else {
        checks_tab()
    }
}

/// Branch names that never count as a stack base, whatever the repository's
/// default branch is: long-lived branches that many PRs target. Holds every
/// [`crate::helpers::DEFAULT_BRANCH_NAMES`] entry.
pub const LONG_LIVED_BRANCHES: [&str; 7] = [
    "main",
    "master",
    "develop",
    "dev",
    "trunk",
    "staging",
    "production",
];

/// True when a PR whose base branch is `base` can be stacked on another PR:
/// `base` is not empty, not the repository's default branch
/// `default_branch` (when one is configured), and not one of
/// [`LONG_LIVED_BRANCHES`].
pub fn can_stack_on(base: &str, default_branch: Option<&str>) -> bool {
    !base.is_empty() && default_branch != Some(base) && !LONG_LIVED_BRANCHES.contains(&base)
}

/// GitLab CI link: the head pipeline's page, else `<mr_url>/pipelines`.
pub fn pipeline_ci_link(pipeline_url: Option<&str>, mr_url: &str) -> CiLink {
    let url = match pipeline_url.filter(|url| !url.is_empty()) {
        Some(url) => url.to_string(),
        None => format!("{}/pipelines", mr_url.trim_end_matches('/')),
    };
    CiLink {
        url,
        kind: CiLinkKind::Pipeline,
    }
}

/// Most checks [`named_checks`] names.
pub const NAMED_CHECKS_MAX: usize = 2;

/// Checks to name in the popover: failed ones first, then running ones,
/// each group in forge order, at most [`NAMED_CHECKS_MAX`]. The count is how
/// many more failed or running checks there are.
pub fn named_checks(checks: &[Check]) -> (Vec<Check>, usize) {
    let wanted: Vec<&Check> = [CheckBucket::Fail, CheckBucket::Pending]
        .into_iter()
        .flat_map(|bucket| checks.iter().filter(move |check| check.bucket == bucket))
        .collect();
    let more = wanted.len().saturating_sub(NAMED_CHECKS_MAX);
    let shown = wanted.into_iter().take(NAMED_CHECKS_MAX).cloned().collect();
    (shown, more)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checks(fail: usize, pending: usize) -> ChecksSummary {
        ChecksSummary {
            pass: 1,
            fail,
            pending,
        }
    }

    fn facts(merged: bool, draft: bool, checks: ChecksSummary, ready: bool) -> StatusFacts {
        StatusFacts {
            merged,
            draft,
            checks,
            ready,
        }
    }

    #[test]
    fn badge_state_is_first_match_in_ladder_order() {
        let cases = [
            (facts(true, true, checks(1, 1), true), PrState::Merged),
            (facts(false, true, checks(1, 1), true), PrState::CiFailed),
            (facts(false, true, checks(0, 1), true), PrState::Draft),
            (facts(false, false, checks(0, 1), true), PrState::CiRunning),
            (facts(false, false, checks(0, 0), true), PrState::Ready),
            (facts(false, false, checks(0, 0), false), PrState::InReview),
            (
                facts(false, false, ChecksSummary::default(), false),
                PrState::InReview,
            ),
        ];
        for (facts, want) in cases {
            assert_eq!(badge_state(&facts), want, "{facts:?}");
        }
    }

    #[test]
    fn running_checks_beat_ready() {
        assert_eq!(
            badge_state(&facts(false, false, checks(0, 2), true)),
            PrState::CiRunning
        );
    }

    #[test]
    fn state_labels_are_exact() {
        let labels = [
            (PrState::Merged, "merged"),
            (PrState::CiFailed, "CI failed"),
            (PrState::Draft, "draft"),
            (PrState::CiRunning, "CI running"),
            (PrState::Ready, "ready to merge"),
            (PrState::InReview, "in review"),
        ];
        for (state, label) in labels {
            assert_eq!(state.label(), label);
        }
    }

    const CONFLICTS: PrNotes = PrNotes {
        conflicts: true,
        unresolved_threads: false,
        changes_requested: false,
    };
    const THREADS: PrNotes = PrNotes {
        conflicts: false,
        unresolved_threads: true,
        changes_requested: false,
    };
    const CHANGES: PrNotes = PrNotes {
        conflicts: false,
        unresolved_threads: false,
        changes_requested: true,
    };
    const NONE: PrNotes = PrNotes {
        conflicts: false,
        unresolved_threads: false,
        changes_requested: false,
    };

    /// State, notes, open base ref → action, its text, needs you.
    type ActionRow = (
        PrState,
        PrNotes,
        Option<&'static str>,
        NextAction,
        &'static str,
        bool,
    );

    #[test]
    fn next_action_rows_n1_to_n8_with_texts_and_needs_you() {
        use NextAction::*;
        let cases: [ActionRow; 13] = [
            (PrState::CiFailed, CONFLICTS, None, FixCi, "fix CI", true),
            (
                PrState::InReview,
                CONFLICTS,
                None,
                FixConflicts,
                "fix conflicts",
                true,
            ),
            (
                PrState::Draft,
                CONFLICTS,
                None,
                FixConflicts,
                "fix conflicts",
                true,
            ),
            (PrState::Draft, CHANGES, None, Undraft, "undraft", false),
            (
                PrState::InReview,
                CHANGES,
                None,
                AddressReview,
                "address review",
                true,
            ),
            (
                PrState::CiRunning,
                THREADS,
                None,
                AddressReview,
                "address review",
                true,
            ),
            (
                PrState::Ready,
                NONE,
                Some("#318"),
                MergeBaseFirst("#318".into()),
                "merge #318 first",
                true,
            ),
            (
                PrState::Ready,
                NONE,
                Some("!12"),
                MergeBaseFirst("!12".into()),
                "merge !12 first",
                true,
            ),
            (PrState::Ready, NONE, None, Merge, "merge", true),
            (
                PrState::CiRunning,
                NONE,
                Some("#318"),
                WaitForCi,
                "wait for CI",
                false,
            ),
            (
                PrState::InReview,
                NONE,
                Some("#318"),
                GetReview,
                "get review",
                false,
            ),
            (
                PrState::Ready,
                CHANGES,
                Some("#318"),
                AddressReview,
                "address review",
                true,
            ),
            (PrState::CiFailed, NONE, None, FixCi, "fix CI", true),
        ];
        for (state, notes, base, want, text, needs_you) in cases {
            let got = next_action(state, notes, base).expect("open PR");
            assert_eq!(got, want, "{state:?} {notes:?} {base:?}");
            assert_eq!(got.text(), text);
            assert_eq!(got.needs_you(), needs_you, "{text}");
        }
    }

    #[test]
    fn merged_has_no_next_action() {
        for notes in [NONE, CONFLICTS, THREADS, CHANGES] {
            assert_eq!(next_action(PrState::Merged, notes, Some("#1")), None);
        }
    }

    #[test]
    fn notes_texts_are_exact_and_ordered() {
        assert!(NONE.texts().is_empty());
        let all = PrNotes {
            conflicts: true,
            unresolved_threads: true,
            changes_requested: true,
        };
        assert_eq!(
            all.texts(),
            vec!["has conflicts", "unresolved threads", "changes requested"]
        );
        assert_eq!(THREADS.texts(), vec!["unresolved threads"]);
    }

    fn check(bucket: CheckBucket, name: &str, url: Option<&str>) -> Check {
        Check {
            bucket,
            name: name.into(),
            url: url.map(str::to_string),
        }
    }

    const PR: &str = "https://github.com/acme/app/pull/318";

    #[test]
    fn checks_ci_link_prefers_failed_then_running_then_any_then_checks_tab() {
        use CheckBucket::*;
        let pass = check(Pass, "lint", Some("https://ci/pass"));
        let run = check(Pending, "build", Some("https://ci/run"));
        let fail = check(Fail, "test", Some("https://ci/fail"));
        let fail_no_link = check(Fail, "old", None);
        let link = |checks: &[Check]| {
            let link = checks_ci_link(checks, PR);
            (link.url, link.kind)
        };
        assert_eq!(
            link(&[pass.clone(), run.clone(), fail_no_link.clone(), fail]),
            ("https://ci/fail".into(), CiLinkKind::FailedCheck)
        );
        assert_eq!(
            link(&[pass.clone(), run.clone()]),
            ("https://ci/run".into(), CiLinkKind::RunningCheck)
        );
        assert_eq!(
            link(&[pass.clone()]),
            ("https://ci/pass".into(), CiLinkKind::Checks)
        );
        assert_eq!(
            link(&[pass, fail_no_link.clone(), run]),
            (format!("{PR}/checks"), CiLinkKind::Checks),
            "a failed check with no link: the checks tab, not a running check"
        );
        assert_eq!(
            link(&[fail_no_link]),
            (format!("{PR}/checks"), CiLinkKind::Checks)
        );
        assert_eq!(link(&[]), (format!("{PR}/checks"), CiLinkKind::Checks));
        assert_eq!(
            checks_ci_link(&[], &format!("{PR}/")).url,
            format!("{PR}/checks")
        );
    }

    #[test]
    fn the_named_failed_check_is_the_one_the_link_opens() {
        use CheckBucket::*;
        let a = check(Fail, "a (no link)", None);
        let b = check(Fail, "b", Some("https://ci/b"));
        let c = check(Fail, "c", Some("https://ci/c"));
        let run = check(Pending, "build", Some("https://ci/run"));
        let all = [run.clone(), a.clone(), b.clone(), c];
        assert_eq!(named_failed_check(&all), Some(&b));
        assert_eq!(
            checks_ci_link(&all, PR),
            CiLink {
                url: "https://ci/b".into(),
                kind: CiLinkKind::FailedCheck
            }
        );
        assert_eq!(named_failed_check(&[run.clone(), a.clone()]), Some(&a));
        assert_eq!(named_failed_check(&[run]), None);
    }

    #[test]
    fn long_lived_and_default_branches_never_count_as_a_stack_base() {
        for base in LONG_LIVED_BRANCHES {
            assert!(!can_stack_on(base, None), "{base}");
        }
        for name in crate::helpers::DEFAULT_BRANCH_NAMES {
            assert!(LONG_LIVED_BRANCHES.contains(&name), "{name}");
        }
        assert!(!can_stack_on("", None));
        assert!(!can_stack_on("release", Some("release")));
        assert!(can_stack_on("release", Some("main")));
        assert!(can_stack_on("feature/ABC-123-retry", None));
        assert!(!can_stack_on("main", Some("release")), "long-lived wins");
    }

    #[test]
    fn pipeline_ci_link_is_the_head_pipeline_else_the_pipelines_tab() {
        const MR: &str = "https://gitlab.com/acme/services/api/-/merge_requests/12";
        let pipeline = "https://gitlab.com/acme/services/api/-/pipelines/99";
        assert_eq!(
            pipeline_ci_link(Some(pipeline), MR),
            CiLink {
                url: pipeline.into(),
                kind: CiLinkKind::Pipeline
            }
        );
        for none in [None, Some("")] {
            assert_eq!(
                pipeline_ci_link(none, MR),
                CiLink {
                    url: format!("{MR}/pipelines"),
                    kind: CiLinkKind::Pipeline
                }
            );
        }
    }

    #[test]
    fn ci_link_labels_are_exact() {
        let labels = [
            (CiLinkKind::FailedCheck, "Open failed check"),
            (CiLinkKind::RunningCheck, "Open running check"),
            (CiLinkKind::Checks, "Open checks"),
            (CiLinkKind::Pipeline, "Open pipeline"),
        ];
        for (kind, label) in labels {
            assert_eq!(kind.label(), label);
        }
    }

    #[test]
    fn named_checks_list_failed_then_running_up_to_two_with_a_more_count() {
        use CheckBucket::*;
        let all = [
            check(Pending, "build", None),
            check(Pass, "lint", None),
            check(Fail, "test (ubuntu-latest)", None),
            check(Pending, "docs", None),
            check(Fail, "test (macos-latest)", None),
        ];
        let (shown, more) = named_checks(&all);
        let names: Vec<&str> = shown.iter().map(|check| check.name.as_str()).collect();
        assert_eq!(names, ["test (ubuntu-latest)", "test (macos-latest)"]);
        assert_eq!(more, 2);
        let (shown, more) = named_checks(&all[..2]);
        assert_eq!(shown, vec![all[0].clone()]);
        assert_eq!(more, 0);
        assert_eq!(named_checks(&[check(Pass, "lint", None)]), (vec![], 0));
    }

    #[test]
    fn checks_summary_counts_each_bucket() {
        use CheckBucket::*;
        let all = [
            check(Pass, "a", None),
            check(Fail, "b", None),
            check(Pending, "c", None),
            check(Pending, "d", None),
        ];
        assert_eq!(
            ChecksSummary::of(&all),
            ChecksSummary {
                pass: 1,
                fail: 1,
                pending: 2
            }
        );
    }
}

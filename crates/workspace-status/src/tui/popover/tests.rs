use super::*;
use crate::tui::command_palette::PALETTE_COMMANDS;

fn action_line(title: &str) -> PopoverLine {
    let command = PALETTE_COMMANDS
        .iter()
        .find(|command| command.title == title)
        .expect("palette row");
    PopoverLine::Action {
        command,
        title: command.title,
        keys: command.keys,
    }
}

fn field(value: &str) -> PopoverLine {
    super::field("x", value)
}

fn sync_value(lines: &[PopoverLine]) -> Option<&str> {
    lines.iter().find_map(|line| match line {
        PopoverLine::Field {
            label: "sync",
            value,
            ..
        } => Some(value.as_str()),
        _ => None,
    })
}

#[test]
fn diverged_popover_shows_the_force_push_text_when_classified() {
    let rewrite = crate::git::UpstreamRewrite {
        upstream: "origin/main".into(),
        fork_point: "f".repeat(40),
        tip: "t".repeat(40),
        yours: 1,
        dropped: 2,
    };
    let note = "diverged (ahead 3, behind 1)";
    let mut lines = Vec::new();
    let actions = sync_fields(IconKind::Diverged, note, Some(&rewrite), &mut lines);
    assert_eq!(
        sync_value(&lines),
        Some("origin/main was force-pushed \u{b7} 2 dropped, 1 yours")
    );
    assert_eq!(actions, vec![Action::Fetch, Action::Pull, Action::Push]);

    let mut plain = Vec::new();
    sync_fields(IconKind::Diverged, note, None, &mut plain);
    assert_eq!(sync_value(&plain), Some("ahead 3, behind 1"));
}

#[test]
fn worktree_branch_honours_the_configured_default() {
    let role = |branch, default_override| {
        let mut lines = Vec::new();
        worktree_fields(
            &WorktreeFacts {
                path: "/w/wt",
                primary: None,
                branch: Some(branch),
                default_override,
                current: false,
                hidden: false,
            },
            &mut lines,
        );
        lines.iter().find_map(|line| match line {
            PopoverLine::Field {
                label: "branch",
                role,
                ..
            } => Some(*role),
            _ => None,
        })
    };
    assert_eq!(role("trunk", Some("trunk")), Some(SegRole::BranchDefault));
    assert_eq!(role("main", Some("trunk")), Some(SegRole::BranchFeature));
    assert_eq!(role("main", None), Some(SegRole::BranchDefault));
    assert_eq!(role("trunk", None), Some(SegRole::BranchFeature));
}

#[test]
fn rect_opens_below_flips_above_and_clamps_right() {
    let bounds = Rect::new(0, 1, 80, 20);
    let icon = |x, y| Rect::new(x, y, 1, 1);
    assert_eq!(
        popover_rect(icon(10, 3), bounds, 30, 6),
        Rect::new(10, 4, 30, 6),
        "below the icon"
    );
    assert_eq!(
        popover_rect(icon(10, 20), bounds, 30, 6),
        Rect::new(10, 14, 30, 6),
        "bottom row flips above"
    );
    assert_eq!(
        popover_rect(icon(75, 3), bounds, 30, 6),
        Rect::new(50, 4, 30, 6),
        "right edge clamps left"
    );
    assert_eq!(
        popover_rect(icon(5, 10), bounds, 30, 30),
        Rect::new(5, 1, 30, 20),
        "taller than the bounds shrinks and sits at the top"
    );
    assert_eq!(
        popover_rect(icon(5, 4), Rect::new(0, 1, 80, 8), 20, 6),
        Rect::new(5, 3, 20, 6),
        "no room either side sits at the bottom"
    );
}

#[test]
fn focus_moves_over_fields_and_actions_only() {
    let text = PopoverLine::Text("meaning".into());
    let url = field("https://example.test/pull/7");
    let pull = action_line("Pull behind");
    let fetch = action_line("Fetch remotes");
    let lines = vec![&text, &url, &pull, &fetch];
    assert_eq!(landing_line(&lines), 2, "lands on the first action");
    assert_eq!(step_line(&lines, 2, 1), 3);
    assert_eq!(step_line(&lines, 3, 1), 3, "stops at the last line");
    assert_eq!(step_line(&lines, 2, -1), 1);
    assert_eq!(step_line(&lines, 1, -1), 1, "the meaning never focuses");
    assert_eq!(focused_line(&lines, 0), Some(1));
    assert_eq!(focused_line(&lines, 9), Some(3), "past the end clamps");
    let only_text = vec![&text];
    assert_eq!(focused_line(&only_text, 0), None);
    assert_eq!(landing_line(&only_text), 0);
    assert_eq!(landing_line(&[&text, &url]), 1, "no action: first field");
}

#[test]
fn lines_copy_the_value_or_title() {
    assert_eq!(field("#7").copy_text(), "#7");
    assert_eq!(action_line("Push").copy_text(), "Push");
    assert_eq!(PopoverLine::Text("m".into()).copy_text(), "m");
    assert_eq!(
        PopoverLine::action(&Action::Push),
        Some(action_line("Push"))
    );
    assert_eq!(PopoverLine::action(&Action::PopoverClose), None);
}

// ---- PR section: field lines (number … url) and the actions ----

mod pr {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use super::super::*;
    use crate::snapshot::RepoSnapshot;
    use crate::testutil::app_state;
    use crate::tui::action::Effect;
    use crate::tui::pr_status::{
        checks_ci_link, pipeline_ci_link, Check, CiLink, CiLinkKind, PrNotes,
    };
    use crate::tui::pull_request::{Forge, PrDetailLookup, PrLookup, PullRequest};
    use crate::tui::state::prs_fixtures::checkout;
    use crate::tui::state::unix_now;

    const GH_REMOTE: &str = "git@github.com:acme/app.git";
    const GL_REMOTE: &str = "git@gitlab.com:acme/services/api.git";
    const GH_PR: &str = "https://github.com/acme/app/pull/318";
    const GL_MR: &str = "https://gitlab.com/acme/services/api/-/merge_requests/12";
    const FAILED_JOB: &str = "https://github.com/acme/app/actions/runs/1/job/2";
    const PIPELINE: &str = "https://gitlab.com/acme/services/api/-/pipelines/99";

    /// Both forges, as (remote, a PR in `state` on it).
    fn forges(state: PrState) -> [(&'static str, PullRequest); 2] {
        [(GH_REMOTE, gh_pr(state)), (GL_REMOTE, gl_mr(state))]
    }

    fn gh_pr(state: PrState) -> PullRequest {
        let mut pr = PullRequest::fixture(318, GH_PR, state);
        pr.title = "Retry failed uploads with backoff".into();
        pr.head = "feature/ABC-123-retry".into();
        pr
    }

    fn gl_mr(state: PrState) -> PullRequest {
        let mut pr = PullRequest::fixture(12, GL_MR, state);
        pr.title = "Add the retry API".into();
        pr.head = "feature/ABC-123-retry".into();
        pr
    }

    /// One checkout per PR (named `c<i>`, on the PR's head branch),
    /// each PR the badge answer at `remote`, in ASCII mode.
    fn app_with(remote: &str, prs: &[PullRequest]) -> AppState {
        let names: Vec<String> = (0..prs.len()).map(|i| format!("c{i}")).collect();
        let repos: Vec<RepoSnapshot> = names
            .iter()
            .zip(prs)
            .map(|(name, pr)| checkout(name, &pr.head))
            .collect();
        let mut app = app_state(&repos);
        let _ = app.due_pr_lookups();
        for (name, pr) in names.iter().zip(prs) {
            assert!(app.apply_pr_lookup(
                Path::new(name),
                &pr.head,
                Some(remote.into()),
                PrLookup::Found(pr.clone()),
            ));
        }
        app
    }

    /// What the popover holds open: a pinned PR section on checkout
    /// `name`.
    fn pin(app: &mut AppState, name: &str) -> (IconKind, IconTarget) {
        let kind = pr_badge_kind(app.pr_badge(Path::new(name)).expect("badge"));
        let target = (kind, IconTarget::PullRequest(PathBuf::from(name)));
        app.popover = Some(PopoverState {
            origin: PopoverOrigin::Pinned,
            targets: vec![target.clone()],
            anchor: None,
            focus_line: 0,
            owner: app.popover_owner(),
        });
        target
    }

    /// The PR section of checkout `name`, pinned: the detail fetch
    /// asked for (left loading), and `detail` landed when given.
    fn section_of(
        app: &mut AppState,
        name: &str,
        detail: Option<PrDetailLookup>,
    ) -> PopoverSection {
        let target = pin(app, name);
        let fetch = app.request_pr_details();
        if let Some(detail) = detail {
            let Effect::LookupPullRequestDetail {
                repo,
                branch,
                remote,
                number,
                request,
            } = fetch
            else {
                panic!("the pin asks for the detail: {fetch:?}");
            };
            assert!(app.apply_pr_detail(&repo, &branch, &remote, number, request, detail));
        }
        popover_sections(app, &[target], unix_now())
            .pop()
            .expect("PR section")
    }

    /// A line as text: `label: value` (`label: mark value` with a
    /// mark), `(note)`, `---` for the rule, `[title] keys` for an
    /// action.
    fn line_text(line: &PopoverLine) -> String {
        match line {
            PopoverLine::Field {
                label,
                value,
                mark: Some((mark, _)),
                ..
            } => format!("{label}: {mark} {value}"),
            PopoverLine::Field { label, value, .. } => format!("{label}: {value}"),
            PopoverLine::Text(text) | PopoverLine::Note(text) => format!("({text})"),
            PopoverLine::Rule => "---".into(),
            PopoverLine::Action { title, keys, .. } => format!("[{title}] {keys}"),
        }
    }

    fn texts(section: &PopoverSection) -> Vec<String> {
        section.lines.iter().map(line_text).collect()
    }

    /// The field whose [`line_text`] starts with `text`.
    fn field_of<'a>(section: &'a PopoverSection, text: &str) -> &'a PopoverLine {
        section
            .lines
            .iter()
            .find(|line| {
                matches!(line, PopoverLine::Field { .. }) && line_text(line).starts_with(text)
            })
            .unwrap_or_else(|| panic!("no line {text:?} in {:?}", texts(section)))
    }

    /// Colour and bold head of the value of the field that starts with
    /// `text`.
    fn style_of(section: &PopoverSection, text: &str) -> (SegRole, usize) {
        match field_of(section, text) {
            PopoverLine::Field { role, bold, .. } => (*role, *bold),
            _ => unreachable!("a field"),
        }
    }

    /// Colour of the mark of the field that starts with `text`.
    fn mark_role_of(section: &PopoverSection, text: &str) -> Option<SegRole> {
        match field_of(section, text) {
            PopoverLine::Field { mark, .. } => mark.map(|(_, role)| role),
            _ => unreachable!("a field"),
        }
    }

    fn actions(section: &PopoverSection) -> Vec<String> {
        let lines = texts(section);
        let rule = lines.iter().position(|line| line == "---").expect("rule");
        lines[rule + 1..].to_vec()
    }

    fn check(bucket: CheckBucket, name: &str, url: Option<&str>) -> Check {
        Check {
            bucket,
            name: name.into(),
            url: url.map(str::to_string),
        }
    }

    fn gh_detail() -> PullRequestDetail {
        PullRequestDetail {
            number: 318,
            title: "Retry failed uploads with backoff".into(),
            state: "OPEN".into(),
            draft: false,
            author: Some("octo-user".into()),
            head: Some("feature/ABC-123-retry".into()),
            base: Some("main".into()),
            review: Some(PrReview::ReviewRequired),
            reviews: Some(ReviewCount::Approved(1)),
            checks: ChecksSummary {
                pass: 6,
                fail: 1,
                pending: 2,
            },
            pipeline: None,
            check_names: vec![
                check(CheckBucket::Fail, "test (ubuntu-latest)", Some(FAILED_JOB)),
                check(CheckBucket::Pending, "build", None),
            ],
            more_checks: 1,
            notes: PrNotes::default(),
            ci_link: checks_ci_link(&[], GH_PR),
            updated_at: Some(unix_now() - 2 * 3600),
        }
    }

    fn gl_detail() -> PullRequestDetail {
        PullRequestDetail {
            number: 12,
            title: "Add the retry API".into(),
            state: "OPEN".into(),
            draft: false,
            author: Some("octo-user".into()),
            head: Some("feature/ABC-123-retry".into()),
            base: Some("main".into()),
            review: Some(PrReview::Approved),
            reviews: Some(ReviewCount::Approvals {
                given: 2,
                required: 2,
            }),
            checks: ChecksSummary {
                pass: 1,
                fail: 0,
                pending: 0,
            },
            pipeline: Some("success".into()),
            check_names: Vec::new(),
            more_checks: 0,
            notes: PrNotes::default(),
            ci_link: pipeline_ci_link(Some(PIPELINE), GL_MR),
            updated_at: None,
        }
    }

    #[test]
    fn a_failed_github_pr_lists_every_line_in_order() {
        let mut pr = gh_pr(PrState::CiFailed);
        pr.failed_check = Some("test (ubuntu-latest)".into());
        pr.ci_link = CiLink {
            url: FAILED_JOB.into(),
            kind: CiLinkKind::FailedCheck,
        };
        let mut app = app_with(GH_REMOTE, &[pr]);
        let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(gh_detail())));
        assert_eq!(section.heading(), "PR · CI failed");
        assert_eq!(section.icon, IconKind::PrCiFailed);
        assert_eq!(section.role, SegRole::Deleted);
        let lines = texts(&section);
        assert_eq!(
            lines,
            vec![
                "number: #318 Retry failed uploads with backoff",
                "state: OPEN · REVIEW_REQUIRED",
                "next: > fix CI",
                "author: octo-user",
                "branch: feature/ABC-123-retry -> main",
                "checks: 6 pass · 1 fail · 2 pending",
                ": fail test (ubuntu-latest)",
                ": pending build",
                ": +1 more",
                "reviews: 1 approved · REVIEW_REQUIRED",
                lines[10].as_str(),
                "checked: just now",
                "url: https://github.com/acme/app/pull/318",
                "---",
                "[Open PR] gx",
                "[Open failed check] gc",
                "[Copy URL] y",
                "[Refresh this PR] r",
            ],
        );
        assert!(lines[10].starts_with("updated: ") && lines[10].ends_with(" (2h)"));
        assert_eq!(style_of(&section, "number"), (SegRole::Repo, "#318".len()));
        assert_eq!(style_of(&section, "state"), (SegRole::Deleted, 0));
        assert_eq!(
            style_of(&section, "next"),
            (SegRole::Deleted, "> fix CI".len())
        );
        assert_eq!(style_of(&section, "branch"), (SegRole::BranchFeature, 0));
        assert_eq!(style_of(&section, "checks"), (SegRole::Deleted, 0));
        // The mark takes the bucket colour; the check name stays plain.
        assert_eq!(style_of(&section, ": fail"), (SegRole::File, 0));
        assert_eq!(mark_role_of(&section, ": fail"), Some(SegRole::Deleted));
        assert_eq!(style_of(&section, ": pending"), (SegRole::File, 0));
        assert_eq!(mark_role_of(&section, ": pending"), Some(SegRole::Modified));
        assert_eq!(mark_role_of(&section, ": +1"), None);
        assert_eq!(style_of(&section, ": +1"), (SegRole::Muted, 0));
        assert_eq!(style_of(&section, "reviews"), (SegRole::Modified, 0));
        assert_eq!(style_of(&section, "checked"), (SegRole::Muted, 0));

        // Unicode glyph mode: the arrow, the next mark, and the check
        // marks.
        app.ascii = false;
        let section = section_of(&mut app, "c0", None);
        let lines = texts(&section);
        for want in [
            "next: ▸ fix CI",
            "branch: feature/ABC-123-retry → main",
            "checks: ✓ 6 · ✗ 1 · … 2",
            ": ✗ test (ubuntu-latest)",
            ": … build",
        ] {
            assert!(lines.iter().any(|line| line == want), "{want} in {lines:?}");
        }
    }

    #[test]
    fn a_ready_gitlab_mr_shows_its_pipeline_and_approvals() {
        let mut mr = gl_mr(PrState::Ready);
        mr.ci_link = pipeline_ci_link(Some(PIPELINE), GL_MR);
        let mut app = app_with(GL_REMOTE, &[mr]);
        let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(gl_detail())));
        assert_eq!(section.heading(), "PR · ready to merge");
        assert_eq!(section.icon, IconKind::PrReady);
        assert_eq!(
            texts(&section),
            vec![
                "number: !12 Add the retry API",
                "state: OPEN · APPROVED",
                "next: > merge",
                "author: octo-user",
                "branch: feature/ABC-123-retry -> main",
                "checks: pipeline success",
                "approvals: 2 of 2",
                "checked: just now",
                "url: https://gitlab.com/acme/services/api/-/merge_requests/12",
                "---",
                "[Open PR] gx",
                "[Open pipeline] gc",
                "[Copy URL] y",
                "[Refresh this PR] r",
            ]
        );
        assert_eq!(style_of(&section, "number"), (SegRole::Repo, "!12".len()));
        assert_eq!(
            style_of(&section, "next"),
            (SegRole::Added, "> merge".len())
        );
        assert_eq!(style_of(&section, "checks"), (SegRole::Added, 0));
        assert_eq!(style_of(&section, "approvals"), (SegRole::Added, 0));

        // Approvals that do not meet the rules colour modified, even
        // when the numbers match (an approver the rules do not count).
        let mut not_met = gl_detail();
        not_met.review = Some(PrReview::NotApproved);
        not_met.reviews = Some(ReviewCount::Approvals {
            given: 0,
            required: 1,
        });
        not_met.pipeline = Some("failed".into());
        not_met.checks = ChecksSummary {
            pass: 0,
            fail: 1,
            pending: 0,
        };
        not_met.check_names = vec![check(CheckBucket::Fail, "rspec 1/2", None)];
        let mut app = app_with(GL_REMOTE, &[gl_mr(PrState::CiFailed)]);
        let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(not_met)));
        assert_eq!(
            style_of(&section, "approvals: 0 of 1"),
            (SegRole::Modified, 0)
        );
        assert_eq!(
            style_of(&section, "checks: pipeline failed"),
            (SegRole::Deleted, 0)
        );
        // The mark takes the bucket colour; the job name stays plain.
        assert_eq!(style_of(&section, ": fail rspec 1/2"), (SegRole::File, 0));
        assert_eq!(
            mark_role_of(&section, ": fail rspec 1/2"),
            Some(SegRole::Deleted)
        );
    }

    /// A forge that sent no approval count (GitLab with a missing
    /// approvals field; GitHub likewise) shows no review line.
    #[test]
    fn no_review_count_drops_the_review_line_on_both_forges() {
        for (remote, pr) in forges(PrState::InReview) {
            let mut detail = if remote == GL_REMOTE {
                gl_detail()
            } else {
                gh_detail()
            };
            detail.reviews = None;
            let mut app = app_with(remote, &[pr]);
            let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(detail)));
            let lines = texts(&section);
            assert!(
                !lines
                    .iter()
                    .any(|line| line.starts_with("approvals") || line.starts_with("reviews")),
                "{remote}: {lines:?}"
            );
            assert!(
                lines.iter().any(|line| line.starts_with("checks")),
                "{lines:?}"
            );
        }
    }

    #[test]
    fn github_reviews_leave_out_a_missing_decision_and_colour_by_approval() {
        let mut app = app_with(GH_REMOTE, &[gh_pr(PrState::Ready)]);
        let mut detail = gh_detail();
        detail.review = None;
        detail.reviews = Some(ReviewCount::Approved(0));
        let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(detail)));
        assert_eq!(
            style_of(&section, "reviews: 0 approved"),
            (SegRole::Modified, 0)
        );
        assert!(texts(&section).contains(&"reviews: 0 approved".to_string()));

        let mut app = app_with(GH_REMOTE, &[gh_pr(PrState::Ready)]);
        let mut detail = gh_detail();
        detail.review = Some(PrReview::Approved);
        detail.reviews = Some(ReviewCount::Approved(2));
        let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(detail)));
        assert_eq!(
            style_of(&section, "reviews: 2 approved · APPROVED"),
            (SegRole::Added, 0)
        );
    }

    /// Every state on both forges: the heading, the next line, the CI
    /// action, and Refresh this PR on open PRs only.
    #[test]
    fn each_state_names_its_next_step_and_actions_on_both_forges() {
        for (state, label, next) in [
            (PrState::CiFailed, "CI failed", "next: > fix CI"),
            (PrState::Ready, "ready to merge", "next: > merge"),
            (PrState::CiRunning, "CI running", "next: > wait for CI"),
            (PrState::Draft, "draft", "next: > undraft"),
            (PrState::InReview, "in review", "next: > get review"),
            (PrState::Merged, "merged", "next: merged"),
        ] {
            for (remote, pr) in forges(state) {
                let ci = match pr.forge {
                    Forge::GitHub => "[Open checks] gc",
                    Forge::GitLab => "[Open pipeline] gc",
                };
                let mut app = app_with(remote, &[pr]);
                let section = section_of(&mut app, "c0", None);
                let what = format!("{state:?} at {remote}");
                assert_eq!(section.heading(), format!("PR · {label}"), "{what}");
                assert_eq!(section.icon, pr_badge_kind(state), "{what}");
                let lines = texts(&section);
                assert!(lines.contains(&next.to_string()), "{what}: {lines:?}");
                assert_eq!(
                    lines[1], "(loading…)",
                    "{what}: the note under the number line"
                );
                let mut want = vec!["[Open PR] gx", ci, "[Copy URL] y"];
                if state != PrState::Merged {
                    want.push("[Refresh this PR] r");
                }
                assert_eq!(actions(&section), want, "{what}");
            }
        }
    }

    #[test]
    fn a_merged_pr_says_when_and_keeps_its_ci_label_rule() {
        for (remote, mut pr) in forges(PrState::Merged) {
            pr.updated_at = Some(unix_now() - 3 * 3600);
            if pr.forge == Forge::GitHub {
                pr.ci_link = CiLink {
                    url: FAILED_JOB.into(),
                    kind: CiLinkKind::FailedCheck,
                };
            }
            let github = pr.forge == Forge::GitHub;
            let mut app = app_with(remote, &[pr]);
            let section = section_of(&mut app, "c0", None);
            assert_eq!(
                style_of(&section, "next: merged 3h ago"),
                (SegRole::BranchFeature, "merged 3h ago".len())
            );
            let ci = if github {
                "[Open failed check] gc"
            } else {
                "[Open pipeline] gc"
            };
            assert_eq!(
                actions(&section),
                vec!["[Open PR] gx", ci, "[Copy URL] y"],
                "{remote}"
            );
        }
    }

    #[test]
    fn a_stacked_pr_names_its_base_and_merges_it_first() {
        for remote in [GH_REMOTE, GL_REMOTE] {
            let mut base = if remote == GH_REMOTE {
                gh_pr(PrState::CiFailed)
            } else {
                gl_mr(PrState::CiFailed)
            };
            base.head = "feature/ABC-123-retry".into();
            let mut top = if remote == GH_REMOTE {
                PullRequest::fixture(319, "https://github.com/acme/app/pull/319", PrState::Ready)
            } else {
                PullRequest::fixture(
                    13,
                    "https://gitlab.com/acme/services/api/-/merge_requests/13",
                    PrState::Ready,
                )
            };
            top.head = "feature/ABC-125-retry-ui".into();
            top.base = "feature/ABC-123-retry".into();
            let base_ref = base.ref_text();
            let mut app = app_with(remote, &[base, top]);
            let section = section_of(&mut app, "c1", None);
            let lines = texts(&section);
            let stack = format!("stack: after {base_ref} (CI failed)");
            assert!(lines.contains(&stack), "{lines:?}");
            assert_eq!(style_of(&section, &stack), (SegRole::Modified, 0));
            let next = format!("next: > merge {base_ref} first");
            assert!(lines.contains(&next), "{lines:?}");
            let at = |text: &str| lines.iter().position(|line| line.starts_with(text));
            assert!(at("next") < at("stack") && at("stack") < at("checked"));

            // The base itself is no stack.
            let section = section_of(&mut app, "c0", None);
            assert!(!texts(&section).iter().any(|line| line.starts_with("stack")));
        }
    }

    #[test]
    fn conflicts_and_threads_show_as_notes_and_the_next_step() {
        for (remote, mut pr) in forges(PrState::InReview) {
            pr.notes = PrNotes {
                conflicts: true,
                unresolved_threads: true,
                changes_requested: false,
                behind: false,
            };
            let mut app = app_with(remote, &[pr]);
            let section = section_of(&mut app, "c0", None);
            let lines = texts(&section);
            assert!(
                lines.contains(&"next: > fix conflicts".to_string()),
                "{lines:?}"
            );
            assert_eq!(
                style_of(&section, "notes: has conflicts · unresolved threads"),
                (SegRole::Modified, 0),
                "{remote}"
            );
        }
    }

    #[test]
    fn a_failing_lookup_keeps_the_pr_and_says_why_on_checked() {
        for (remote, pr) in forges(PrState::CiRunning) {
            let mut app = app_state(&[checkout("c0", &pr.head)]);
            let _ = app.due_pr_lookups();
            let then = Instant::now()
                .checked_sub(Duration::from_secs(12 * 60))
                .expect("clock");
            let head = pr.head.clone();
            assert!(app.apply_pr_lookup_at(
                Path::new("c0"),
                &head,
                Some(remote.into()),
                PrLookup::Found(pr),
                then,
            ));
            let section = section_of(&mut app, "c0", None);
            assert_eq!(style_of(&section, "checked: 12m ago"), (SegRole::Muted, 0));

            for (reason, want) in [
                (
                    Some("HTTP 401: Bad credentials".to_string()),
                    "checked: 12m ago · last lookup failed: HTTP 401: Bad credentials",
                ),
                (None, "checked: 12m ago · last lookup failed"),
            ] {
                assert!(app.apply_pr_lookup(
                    Path::new("c0"),
                    &head,
                    Some(remote.into()),
                    PrLookup::Failed(reason),
                ));
                let section = popover_sections(
                    &app,
                    &app.popover.clone().expect("pinned").targets,
                    unix_now(),
                )
                .pop()
                .expect("the badge stays, so does the section");
                let lines = texts(&section);
                assert!(lines.contains(&want.to_string()), "{remote}: {lines:?}");
                assert_eq!(style_of(&section, want), (SegRole::Modified, 0));
            }
        }
    }

    #[test]
    fn a_failed_first_lookup_pins_the_failure_section_with_its_reason() {
        for (remote, pr) in forges(PrState::InReview) {
            let mut app = app_state(&[checkout("c0", &pr.head)]);
            let _ = app.due_pr_lookups();
            let target = (
                IconKind::PrLookupFailed,
                IconTarget::PullRequest(PathBuf::from("c0")),
            );
            assert!(
                popover_sections(&app, std::slice::from_ref(&target), unix_now()).is_empty(),
                "{remote}: no section while the first lookup runs"
            );
            assert!(app.apply_pr_lookup(
                Path::new("c0"),
                &pr.head,
                Some(remote.into()),
                PrLookup::Failed(Some("HTTP 401: Bad credentials".into())),
            ));
            let section = popover_sections(&app, std::slice::from_ref(&target), unix_now())
                .pop()
                .expect("the failure mark has a section");
            assert_eq!(section.icon, IconKind::PrLookupFailed, "{remote}");
            assert_eq!(section.role, SegRole::Deleted, "{remote}");
            assert_eq!(section.title.as_deref(), Some("PR · lookup failed"));
            assert_eq!(
                texts(&section),
                [
                    "checked: last lookup failed: HTTP 401: Bad credentials",
                    "---",
                    "[Refresh this PR] r",
                ],
                "{remote}"
            );

            // A good answer replaces the failure section with the PR's.
            assert!(app.apply_pr_lookup(
                Path::new("c0"),
                &pr.head,
                Some(remote.into()),
                PrLookup::Found(pr.clone()),
            ));
            let section = popover_sections(&app, &[target], unix_now())
                .pop()
                .expect("the PR section");
            assert_eq!(section.title.as_deref(), Some("PR · in review"));
        }
    }

    #[test]
    fn a_failed_detail_keeps_the_badge_lines_and_says_so() {
        for (remote, pr) in forges(PrState::InReview) {
            let pr_ref = pr.ref_text();
            let mut app = app_with(remote, &[pr]);
            let section = section_of(&mut app, "c0", Some(PrDetailLookup::Failed(None)));
            let lines = texts(&section);
            assert_eq!(lines[0], format!("number: {pr_ref}"));
            assert_eq!(lines[1], format!("({PR_DETAIL_FAILED})"));
            assert_eq!(lines[2], "next: > get review");
        }
    }
}

#[test]
fn sync_counts_read_the_note() {
    assert_eq!(note_count("ahead by 3 commits", "ahead by "), Some(3));
    assert_eq!(note_count("behind by 12 commits", "behind by "), Some(12));
    assert_eq!(note_count("", "behind by "), None);
    assert_eq!(commits(Some(1), "to pull"), "1 commit to pull");
    assert_eq!(commits(Some(4), "not pushed"), "4 commits not pushed");
    assert_eq!(commits(None, "to pull"), "commits to pull");
}

mod commit_popovers {
    use super::super::*;
    use crate::testutil::app_state;
    use workspace_status_graph::{GraphModel, Person};

    const AUTHORED: i64 = 1_700_000_000;
    const COMMITTED: i64 = 1_700_003_600;

    fn id(c: char) -> String {
        c.to_string().repeat(40)
    }

    fn ada() -> CommitPeople {
        CommitPeople {
            author_email: "ada@example.com".into(),
            author_offset_secs: Some(2 * 3600),
            committer: Signature {
                name: "Bob Builder".into(),
                email: "bob@example.com".into(),
                unix: COMMITTED,
                offset_secs: Some(-5 * 3600),
            },
            co_authors: vec![
                Person {
                    name: "Cy Coder".into(),
                    email: "cy@example.com".into(),
                },
                Person {
                    name: "Bob Builder".into(),
                    email: String::new(),
                },
            ],
        }
    }

    /// A merge `a` of `b` and `c` with a branch, a remote, and a tag,
    /// authored by Ada, committed by Bob, with two co-authors.
    fn merge() -> Commit {
        Commit {
            id: id('a'),
            subject: "Merge feature".into(),
            parents: vec![id('b'), id('c')],
            refs: vec![
                GraphRef::local("main"),
                GraphRef::tag("v1.0"),
                GraphRef::remote("origin/main"),
            ],
            author_name: "Ada Lovelace".into(),
            author_date_unix: AUTHORED,
            people: ada(),
            ..Commit::default()
        }
    }

    /// Root commit `c`: no trailer, no author offset, committer
    /// unknown.
    fn root() -> Commit {
        Commit {
            id: id('c'),
            subject: "root".into(),
            author_name: "Ada Lovelace".into(),
            author_date_unix: AUTHORED,
            people: CommitPeople {
                author_email: "ada@example.com".into(),
                ..CommitPeople::default()
            },
            ..Commit::default()
        }
    }

    fn stash() -> Stash {
        Stash {
            id: id('d'),
            stash_ref: "stash@{0}".into(),
            subject: "WIP on main".into(),
            author_name: "Ada Lovelace".into(),
            author_date_unix: AUTHORED,
            parent_id: Some(id('a')),
            people: ada(),
            ..Stash::default()
        }
    }

    /// No repos; the graph of `app` holds [`merge`], [`root`], and
    /// [`stash`].
    fn graph_state() -> AppState {
        let mut state = app_state(&[]);
        state.graph = Some(GraphModel {
            commits: vec![merge(), root()],
            stashes: vec![stash()],
            ..GraphModel::default()
        });
        state.graph_identity = Some(("app".into(), id('a')));
        state
    }

    fn target(repo: &str, sha: &str) -> IconTarget {
        IconTarget::Commit {
            repo: repo.into(),
            sha: sha.into(),
        }
    }

    fn section(state: &AppState, kind: IconKind, sha: &str) -> Option<PopoverSection> {
        popover_sections(state, &[(kind, target("app", sha))], 0)
            .into_iter()
            .next()
    }

    /// Each line as `label|value`, `text|…`, or `note|…`.
    fn rows(section: &PopoverSection) -> Vec<String> {
        section
            .lines
            .iter()
            .map(|line| match line {
                PopoverLine::Text(text) => format!("text|{text}"),
                PopoverLine::Note(text) => format!("note|{text}"),
                PopoverLine::Field { label, value, .. } => format!("{label}|{value}"),
                other => panic!("no actions in a commit popover: {other:?}"),
            })
            .collect()
    }

    fn meaning(kind: IconKind) -> String {
        format!("text|{}", spec(kind).meaning)
    }

    fn when(unix: i64, offset: &str) -> String {
        format!("{} local · orig {offset}", format_local_exact(unix))
    }

    #[test]
    fn commit_hash_card_lists_full_sha_subject_parents_refs_and_dates() {
        let state = graph_state();
        let card = section(&state, IconKind::CommitHash, &id('a')).expect("card");
        assert_eq!(card.heading(), "commit aaaaaaa");
        assert_eq!(card.icon, IconKind::CommitHash);
        assert_eq!(card.role, SegRole::Muted);
        assert_eq!(card.target, target("app", &id('a')));
        assert_eq!(
            rows(&card),
            vec![
                meaning(IconKind::CommitHash),
                format!("sha|{}", id('a')),
                "subject|Merge feature".into(),
                format!("parent|{}", id('b')),
                format!("|{}", id('c')),
                "branches|main, origin/main".into(),
                "tags|v1.0".into(),
                "author|Ada Lovelace <ada@example.com>".into(),
                format!("authored|{}", when(AUTHORED, "UTC+02:00")),
                format!("committed|{}", when(COMMITTED, "UTC-05:00")),
            ]
        );
        assert!(
            matches!(&card.lines[1], PopoverLine::Field { bold, value, .. } if *bold == value.len()),
            "the whole sha is bold: {:?}",
            card.lines[1]
        );
        let lines: Vec<&PopoverLine> = card.lines.iter().collect();
        assert_eq!(landing_line(&lines), 1, "lands on the sha");

        let root = section(&state, IconKind::CommitHash, &id('c')).expect("root");
        assert_eq!(
            rows(&root),
            vec![
                meaning(IconKind::CommitHash),
                format!("sha|{}", id('c')),
                "subject|root".into(),
                "parent|none (root commit)".into(),
                "author|Ada Lovelace <ada@example.com>".into(),
                format!("authored|{} local", format_local_exact(AUTHORED)),
            ]
        );

        let stash = section(&state, IconKind::CommitHash, &id('d')).expect("stash");
        assert_eq!(stash.heading(), "commit ddddddd");
        assert_eq!(
            rows(&stash),
            vec![
                meaning(IconKind::CommitHash),
                format!("sha|{}", id('d')),
                "ref|stash@{0}".into(),
                "subject|WIP on main".into(),
                format!("parent|{}", id('a')),
                "author|Ada Lovelace <ada@example.com>".into(),
                format!("authored|{}", when(AUTHORED, "UTC+02:00")),
                format!("committed|{}", when(COMMITTED, "UTC-05:00")),
            ]
        );
    }

    #[test]
    fn commit_time_and_author_sections_show_both_people_and_co_authors() {
        let state = graph_state();
        let times = vec![
            "author|Ada Lovelace <ada@example.com>".to_string(),
            format!("authored|{}", when(AUTHORED, "UTC+02:00")),
            "committer|Bob Builder <bob@example.com>".into(),
            format!("committed|{}", when(COMMITTED, "UTC-05:00")),
        ];
        let time = section(&state, IconKind::CommitDate, &id('a')).expect("time");
        assert_eq!(time.heading(), "commit time");
        assert_eq!(time.role, SegRole::Muted);
        assert_eq!(
            rows(&time),
            [vec![meaning(IconKind::CommitDate)], times.clone()].concat()
        );

        let author = section(&state, IconKind::CommitAuthor, &id('a')).expect("author");
        assert_eq!(author.heading(), "commit author");
        assert_eq!(
            rows(&author),
            [
                vec![meaning(IconKind::CommitAuthor)],
                times,
                vec![
                    "co-author|Cy Coder <cy@example.com>".into(),
                    "|Bob Builder".into(),
                ],
            ]
            .concat()
        );

        // No trailer: no co-author row. No offset: local only. Unix 0
        // and no committer: those rows are left out.
        let root = section(&state, IconKind::CommitAuthor, &id('c')).expect("root");
        assert_eq!(
            rows(&root),
            vec![
                meaning(IconKind::CommitAuthor),
                "author|Ada Lovelace <ada@example.com>".into(),
                format!("authored|{} local", format_local_exact(AUTHORED)),
            ]
        );

        assert_eq!(who("", ""), None);
        assert_eq!(who("Ada Lovelace", "").as_deref(), Some("Ada Lovelace"));
        assert_eq!(super::super::when(0, Some(3600)), None);
    }

    #[test]
    fn commit_meta_icon_names_the_row_commit_a_parent_or_the_stash() {
        use workspace_status_graph::GraphIconKind as G;
        let commit = GraphRow::Commit {
            commit: merge(),
            is_head: true,
            worktrees: Vec::new(),
        };
        let stash_row = GraphRow::Stash(stash());
        let icon = |kind, part, row: &GraphRow| commit_meta_icon(kind, part, row, "app");
        assert_eq!(
            icon(G::CommitHash, None, &commit),
            Some((IconKind::CommitHash, target("app", &id('a'))))
        );
        assert_eq!(
            icon(G::CommitHash, Some(1), &commit),
            Some((IconKind::CommitHash, target("app", &id('c'))))
        );
        assert_eq!(icon(G::CommitHash, Some(2), &commit), None);
        assert_eq!(
            icon(G::CommitDate, None, &commit),
            Some((IconKind::CommitDate, target("app", &id('a'))))
        );
        assert_eq!(
            icon(G::CommitAuthor, None, &stash_row),
            Some((IconKind::CommitAuthor, target("app", &id('d'))))
        );
        assert_eq!(icon(G::CommitHash, Some(0), &stash_row), None);
        let no_id = GraphRow::Stash(Stash {
            id: String::new(),
            ..stash()
        });
        assert_eq!(icon(G::CommitHash, None, &no_id), None);
        let uncommitted = GraphRow::Uncommitted { has_changes: true };
        assert_eq!(icon(G::CommitHash, None, &uncommitted), None);
        assert_eq!(icon(G::CommitNode, None, &commit), None);
        // gh lists row icons through graph_icon: none for commit text.
        for kind in [G::CommitHash, G::CommitDate, G::CommitAuthor] {
            assert_eq!(graph_icon(kind, None, None, &commit), None);
        }
    }

    #[test]
    fn commit_card_off_the_graph_shows_loading_then_the_cached_card_or_the_failure() {
        let mut state = graph_state();
        let missing = id('e');
        let hash = section(&state, IconKind::CommitHash, &missing).expect("loading");
        assert_eq!(hash.heading(), "commit eeeeeee");
        assert_eq!(
            rows(&hash),
            vec![
                meaning(IconKind::CommitHash),
                format!("sha|{missing}"),
                "note|loading…".into(),
            ]
        );
        for kind in [IconKind::CommitDate, IconKind::CommitAuthor] {
            let loading = section(&state, kind, &missing).expect("loading");
            assert_eq!(rows(&loading), vec![meaning(kind), "note|loading…".into()]);
        }
        // The graph of `app` does not answer for another checkout.
        let other = popover_sections(
            &state,
            &[(IconKind::CommitHash, target("lib", &id('a')))],
            0,
        );
        assert_eq!(
            rows(&other[0]).last().map(String::as_str),
            Some("note|loading…")
        );

        state.commit_cards.insert(
            "app".into(),
            missing.clone(),
            Ok(Commit {
                id: missing.clone(),
                ..merge()
            }),
        );
        let card = section(&state, IconKind::CommitHash, &missing).expect("card");
        assert_eq!(
            rows(&card)[1..4],
            [
                format!("sha|{missing}"),
                "subject|Merge feature".to_string(),
                format!("parent|{}", id('b')),
            ]
        );
        assert_eq!(rows(&card).len(), 10, "the full card");

        let failed = id('f');
        state
            .commit_cards
            .insert("app".into(), failed.clone(), Err("boom".into()));
        let hash = section(&state, IconKind::CommitHash, &failed).expect("failed");
        assert_eq!(
            rows(&hash),
            vec![
                meaning(IconKind::CommitHash),
                format!("sha|{failed}"),
                "note|no commit details: boom".into(),
            ]
        );
        let author = section(&state, IconKind::CommitAuthor, &failed).expect("failed");
        assert_eq!(
            rows(&author),
            vec![
                meaning(IconKind::CommitAuthor),
                "note|no commit details: boom".into(),
            ]
        );

        assert!(
            section(&state, IconKind::CommitHash, "").is_none(),
            "empty sha"
        );
        assert!(
            section(&state, IconKind::GraphCommit, &id('a')).is_none(),
            "another kind"
        );
    }
}

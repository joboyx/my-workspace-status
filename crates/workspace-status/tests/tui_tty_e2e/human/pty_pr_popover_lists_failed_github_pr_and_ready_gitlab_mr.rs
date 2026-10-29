use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, click_badge, forge_env, named_workspace, seed_checkouts, Forge, PrCheckout,
};
use crate::support::{GIT_WAIT, WAIT};

/// A screen line with runs of blanks folded to one space, so a label column
/// of any width compares the same.
fn folded(line: &str) -> String {
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Index of the first screen line that, folded, contains `needle`.
fn line_with(screen: &str, needle: &str) -> Option<usize> {
    screen
        .lines()
        .position(|line| folded(line).contains(needle))
}

/// The popover prints `fields` on consecutive lines starting at the line
/// with `header`, then the `actions` in order below them.
fn popover_reads(screen: &str, header: &str, fields: &[&str], actions: &[&str]) -> bool {
    let Some(top) = line_with(screen, header) else {
        return false;
    };
    let lines: Vec<&str> = screen.lines().collect();
    let fields_ok = fields.iter().enumerate().all(|(i, field)| {
        lines
            .get(top + 1 + i)
            .is_some_and(|line| folded(line).contains(field))
    });
    let mut below = top + 1 + fields.len();
    let actions_ok = actions.iter().all(|action| {
        let found = lines
            .iter()
            .enumerate()
            .skip(below)
            .find(|(_, line)| folded(line).contains(action))
            .map(|(i, _)| i);
        below = found.map_or(below, |i| i + 1);
        found.is_some()
    });
    fields_ok && actions_ok
}

/// A pinned PR popover lists its lines in order, for a failed GitHub PR and
/// for a ready GitLab merge request (spec §8.4 and AC3's popover half).
///
/// Seed: `appbox` (GitHub `#7`, CI failed) and `apibox` (GitLab `!5`, ready
/// to merge). A click on the badge pins the popover; the fake `gh pr view`
/// and the GitLab approvals answer.
///
/// Proof, GitHub: `x PR · CI failed`, then `number`, `state`, `next`,
/// `author`, `branch`, `checks` with the failed check on the line below,
/// `reviews`, `updated`, `checked`, `url`, then the actions Open PR, Open
/// failed check, Copy URL, Refresh this PR. GitLab: `A PR · ready to
/// merge`, the same lines with `checks pipeline success` and `approvals 1
/// of 1`, and Open pipeline in place of Open failed check. Reverting the
/// popover lines leaves the old three-line section and fails the field
/// order; reverting a state's next action or CI action fails its line.
#[test]
fn pty_pr_popover_lists_failed_github_pr_and_ready_gitlab_mr() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-popover", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::Ready);

    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#7"), 'x') && badge_is(&tui, "apibox", Some("!5"), 'A'),
        "appbox shows `#7 x` and apibox shows `!5 A`",
        GIT_WAIT,
    );

    click_badge(&mut tui, "appbox");
    tui.wait_pred(
        |screen| {
            popover_reads(
                screen,
                "x PR · CI failed",
                &[
                    "number #7 Refresh auth tokens",
                    "state OPEN · REVIEW_REQUIRED",
                    "next > fix CI",
                    "author octo-user",
                    "branch feature/auth-refresh -> main",
                    "checks 1 pass · 1 fail · 0 pending",
                    "fail test (ubuntu-latest)",
                    "reviews 0 approved · REVIEW_REQUIRED",
                    "updated 2026-10-",
                    "checked ",
                    "url https://github.com/acme/app/pull/7",
                ],
                &[
                    "Open PR gx",
                    "Open failed check gc",
                    "Copy URL y",
                    "Refresh this PR r",
                ],
            )
        },
        "the failed GitHub PR popover lists its lines in order",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| line_with(screen, "x PR · CI failed").is_none(),
        "Esc closes the GitHub popover",
        WAIT,
    );

    click_badge(&mut tui, "apibox");
    tui.wait_pred(
        |screen| {
            popover_reads(
                screen,
                "A PR · ready to merge",
                &[
                    "number !5 Rate limit the public API",
                    "state OPEN · APPROVED",
                    "next > merge",
                    "author octo-user",
                    "branch feature/rate-limit -> main",
                    "checks pipeline success",
                    "approvals 1 of 1",
                    "updated 2026-10-",
                    "checked ",
                    // The popover is 64 columns wide: a long URL ends in `…`.
                    "url https://gitlab.com/acme/services/api/-/merge_",
                ],
                &[
                    "Open PR gx",
                    "Open pipeline gc",
                    "Copy URL y",
                    "Refresh this PR r",
                ],
            )
        },
        "the ready GitLab popover lists its lines in order",
        WAIT,
    );
}

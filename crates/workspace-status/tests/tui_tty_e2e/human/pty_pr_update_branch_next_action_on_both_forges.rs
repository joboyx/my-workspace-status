use crate::common::fake_forge::FakeForge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, click_badge, forge_env, line_with, named_workspace, popover_reads, prs_headers,
    prs_refs, seed_checkouts, total_is, Forge, PrCheckout,
};
use crate::support::{GIT_WAIT, WAIT};

/// An approved PR whose branch is behind its base gets the next action
/// `update branch` on both forges, and it needs you (`docs/tui-rust.md`,
/// **Pull request badge**, next action).
///
/// Seed: `ws` holds `appbox` (GitHub `#7`: approved, green checks,
/// `mergeStateStatus: BEHIND`) and `apibox` (GitLab `!5`: pipeline success,
/// approvals left 0, `detailed_merge_status: need_rebase`). The branches
/// carry the keys `ABC-140` and `ABC-141`, so each PR heads its own PRs tab
/// group.
///
/// Proof: both badges stay in review (`#7 P`, `!5 P`). The pinned popover
/// of `#7` reads `P PR · in review`, `state OPEN · APPROVED`, `next >
/// update branch`; the one of `!5` reads the same next action and
/// `approvals 1 of 1`, so the approvals call ran for `need_rebase`. The
/// workspace row ends in `2 PRs · 2 need you`. `gp` lists the groups
/// `ABC-140 · > update branch #7` and `ABC-141 · > update branch !5`.
/// Reverting the behind rule shows `next > get review` and `2 PRs` with no
/// one needing you; dropping the GitLab approvals call for `need_rebase`
/// leaves `!5` unapproved, so its popover never reads `update branch`.
#[test]
fn pty_pr_update_branch_next_action_on_both_forges() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-update-branch", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/ABC-140-auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/ABC-141-rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set_behind(&forge);
    gitlab.set_behind(&forge);

    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'P')
                && badge_is(&tui, "apibox", Some("!5"), 'P')
                && total_is(&tui, "# ws", "2 PRs · 2 need you")
        },
        "both badges read in review and the workspace row ends in `2 PRs · 2 need you`",
        GIT_WAIT,
    );

    click_badge(&mut tui, "appbox");
    tui.wait_pred(
        |screen| {
            popover_reads(
                screen,
                "P PR · in review",
                &[
                    "number #7 Refresh auth tokens",
                    "state OPEN · APPROVED",
                    "next > update branch",
                ],
                &["Refresh this PR r"],
            )
        },
        "the behind GitHub PR popover reads `next > update branch`",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| line_with(screen, "P PR · in review").is_none(),
        "Esc closes the GitHub popover",
        WAIT,
    );

    click_badge(&mut tui, "apibox");
    tui.wait_pred(
        |screen| {
            popover_reads(
                screen,
                "P PR · in review",
                &[
                    "number !5 Rate limit the public API",
                    "state OPEN · APPROVED",
                    "next > update branch",
                    "author octo-user",
                    "branch feature/ABC-141-rate-limit -> main",
                    "checks pipeline success",
                    "approvals 1 of 1",
                ],
                &["Open pipeline gc", "Refresh this PR r"],
            )
        },
        "the behind GitLab MR popover reads `next > update branch` and `approvals 1 of 1`",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| line_with(screen, "P PR · in review").is_none(),
        "Esc closes the GitLab popover",
        WAIT,
    );

    tui.send_bytes(b"gp");
    tui.wait_pred(
        |_| prs_refs(&tui).len() == 2,
        "gp opens the PRs tab with both PRs listed",
        WAIT,
    );
    let headers: Vec<String> = prs_headers(&tui).into_iter().map(|(_, h)| h).collect();
    assert_eq!(
        headers,
        [
            "ABC-140 · > update branch #7",
            "ABC-141 · > update branch !5"
        ],
        "each group header names `update branch`:\n{}",
        tui.screen()
    );
}

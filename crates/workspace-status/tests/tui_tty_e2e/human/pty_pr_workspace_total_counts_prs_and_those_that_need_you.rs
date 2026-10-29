use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    forge_env, named_workspace, seed_checkouts, tree_row_text, tree_text_paint, Forge, PrCheckout,
    MODIFIED, MUTED, POLL_MS,
};
use crate::support::GIT_WAIT;

/// Workspace row text, or an empty string until the row is on screen.
fn total_row(tui: &PtySession) -> String {
    tree_row_text(tui, "# ws").unwrap_or_default()
}

/// The workspace row ends in `total` (two spaces after the sync summary).
fn total_is(tui: &PtySession, total: &str) -> bool {
    total_row(tui).trim_end().ends_with(&format!("  {total}"))
}

/// The workspace row counts PRs and those that need you (AC1 and spec §8.2).
///
/// Seed: `ws` holds `appbox` (GitHub `#7`) and `apibox` (GitLab `!5`), both
/// polled every 300 ms. A failed PR and a ready PR need you; a PR in
/// review does not.
///
/// Proof: the row ends in `2 PRs · 2 need you`, with `2 PRs` muted and
/// `2 need you` amber and bold. A narrow tree (120 columns, under the
/// 50-column rule) shows only `2 need you`. After the swaps the total reads
/// `2 PRs · 1 needs you` (singular verb), `2 PRs` with none needing you, and
/// `1 PR` when one checkout loses its PR. Reverting the total drops the
/// text from the row; reverting the singular, narrow, or none rules fails
/// the matching wait.
#[test]
fn pty_pr_workspace_total_counts_prs_and_those_that_need_you() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-total", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::Ready);

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL_MS)]),
    );
    tui.wait_pred(
        |_| total_is(&tui, "2 PRs · 2 need you"),
        "the workspace row ends in `2 PRs · 2 need you`",
        GIT_WAIT,
    );
    assert_eq!(
        tree_text_paint(&tui, "# ws", "2 PRs"),
        Some((MUTED, false)),
        "`2 PRs` paints muted"
    );
    assert_eq!(
        tree_text_paint(&tui, "# ws", "2 need you"),
        Some((MODIFIED, true)),
        "`2 need you` paints amber and bold"
    );

    tui.resize(120, 32);
    tui.wait_pred(
        |_| total_is(&tui, "2 need you") && !total_row(&tui).contains("PRs"),
        "a tree under 50 columns shows only `2 need you`",
        GIT_WAIT,
    );
    tui.resize(140, 32);

    gitlab.set(&forge, Badge::InReview);
    tui.wait_pred(
        |_| total_is(&tui, "2 PRs · 1 needs you"),
        "one PR in review: `2 PRs · 1 needs you`",
        GIT_WAIT,
    );

    github.set(&forge, Badge::InReview);
    tui.wait_pred(
        |_| total_is(&tui, "2 PRs") && !total_row(&tui).contains("need"),
        "no PR needs you: `2 PRs` alone",
        GIT_WAIT,
    );

    gitlab.set_none(&forge);
    tui.wait_pred(|_| total_is(&tui, "1 PR"), "one PR left: `1 PR`", GIT_WAIT);
}

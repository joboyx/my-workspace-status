use std::cell::Cell;

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_flashes, badge_is, click_badge, forge_env, named_workspace, seed_checkouts, toast_text,
    total_is, Forge, PrCheckout, ADD_RAMP, POLL_MS,
};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// A PR in review that turns ready to merge raises the alert on both
/// forges: a toast, a row flash, the new badge, the new total, the graph
/// chip and the popover (`docs/tui-rust.md`, **PR change toasts**).
///
/// Seed: `apibox` (GitLab `!5`) and `appbox` (GitHub `#7`), both in review,
/// polled every 300 ms. The test swaps both answers to ready to merge and
/// presses no key.
///
/// Proof: the workspace total is `2 PRs` (nobody needs you) before the
/// swap, and no toast shows. After it the next poll shows `A #7 ready to
/// merge · feature/auth-refresh` and `A !5 ready to merge ·
/// feature/rate-limit`, the badge glyph cells sit on the add flash ramp in
/// the frame that carries the toast, both rows end in `A`, the total reads
/// `2 PRs · 2 need you`, the graph chip of the focused checkout reads `A
/// <ref> ready to merge  > merge` (the whole chip, which a toast cannot
/// contain), and the pinned popover reads `A PR · ready to merge`.
/// Reverting the ready alert leaves no toast; reverting the total update
/// leaves `2 PRs`.
#[test]
fn pty_pr_poll_review_to_ready_alerts_on_both_forges() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-ready", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::InReview);
    gitlab.set(&forge, Badge::InReview);

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        40,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL_MS)]),
    );
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'P')
                && badge_is(&tui, "apibox", Some("!5"), 'P')
                && total_is(&tui, "# ws", "2 PRs")
        },
        "both checkouts show a PR in review and the total is `2 PRs`",
        GIT_WAIT,
    );
    assert!(
        !toast_text(&tui.screen()).contains("ready to merge"),
        "no toast on first load:\n{}",
        tui.screen()
    );

    let github_toast = "A #7 ready to merge · feature/auth-refresh";
    let gitlab_toast = "A !5 ready to merge · feature/rate-limit";
    github.set(&forge, Badge::Ready);
    gitlab.set(&forge, Badge::Ready);
    let flashed = (Cell::new(false), Cell::new(false));
    tui.wait_pred(
        |screen| {
            flashed
                .0
                .set(flashed.0.get() || badge_flashes(&tui, "appbox", &ADD_RAMP));
            flashed
                .1
                .set(flashed.1.get() || badge_flashes(&tui, "apibox", &ADD_RAMP));
            let toasts = toast_text(screen);
            toasts.contains(github_toast) && toasts.contains(gitlab_toast)
        },
        "the next poll toasts both ready PRs",
        GIT_WAIT,
    );
    assert!(
        flashed.0.get() && flashed.1.get(),
        "both rows flash on the add ramp with the toast: github={} gitlab={}\n{}",
        flashed.0.get(),
        flashed.1.get(),
        tui.screen()
    );
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'A')
                && badge_is(&tui, "apibox", Some("!5"), 'A')
                && total_is(&tui, "# ws", "2 PRs · 2 need you")
        },
        "both rows end in `A` and the total reads `2 PRs · 2 need you`",
        WAIT,
    );

    // The whole chip, with its two spaces and the next action, is matched:
    // the toast text (`A !5 ready to merge · ...`) cannot contain it.
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "apibox") && screen.contains("A !5 ready to merge  > merge")
        },
        "the graph chip of apibox reads `A !5 ready to merge  > merge`",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "appbox") && screen.contains("A #7 ready to merge  > merge")
        },
        "the graph chip of appbox reads `A #7 ready to merge  > merge`",
        WAIT,
    );

    click_badge(&mut tui, "appbox");
    tui.wait_pred(
        |screen| screen.contains("A PR · ready to merge") && screen.contains("next    > merge"),
        "the GitHub popover reads ready to merge and merge",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("A PR · ready to merge"),
        "Esc closes the popover",
        WAIT,
    );
    click_badge(&mut tui, "apibox");
    tui.wait_pred(
        |screen| {
            screen.contains("A PR · ready to merge")
                && screen.contains("next      > merge")
                && screen.contains("pipeline success")
        },
        "the GitLab popover reads ready to merge, merge and the passed pipeline",
        WAIT,
    );
}

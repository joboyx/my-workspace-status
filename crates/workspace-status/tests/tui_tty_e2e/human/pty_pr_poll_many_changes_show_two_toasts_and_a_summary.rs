use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, list_calls, named_workspace, seed_checkouts, toast_count, toast_text,
    Forge, PrCheckout,
};
use crate::support::{GIT_WAIT, WAIT};

/// Poll period of this test. Longer than the other polling tests, so the
/// quiet gap between two poll cycles is wide enough to swap four answers in.
const POLL: &str = "800";

/// Four PRs that change in one poll cycle raise two toasts and a summary
/// (spec §7, phase A: no key hint on the summary).
///
/// Seed: two GitHub checkouts (`#7`, `#8`) and two GitLab checkouts (`!5`,
/// `!6`), all CI running, polled every 800 ms. The test waits for a whole
/// poll cycle to end (four list calls), then swaps all four answers to CI
/// failed in the quiet gap, so one cycle finds four changes.
///
/// Proof: the toast stack holds two `CI failed ·` toasts and the summary
/// `+2 more PR changes`, and the summary carries no key hint (`gp` is not
/// bound in phase A). Reverting the batch cap shows four toasts (or a
/// stack that pushes the others out); reverting the summary drops the
/// text.
#[test]
fn pty_pr_poll_many_changes_show_two_toasts_and_a_summary() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-summary", "ws");
    let forge = FakeForge::new(&root);
    let checkouts = [
        PrCheckout::new("app-a", "feature/a", Forge::GitHub, 7),
        PrCheckout::new("app-b", "feature/b", Forge::GitHub, 8),
        PrCheckout::new("api-a", "feature/c", Forge::GitLab, 5),
        PrCheckout::new("api-b", "feature/d", Forge::GitLab, 6),
    ];
    seed_checkouts(&workspace, &checkouts);
    for checkout in &checkouts {
        checkout.set(&forge, Badge::CiRunning);
    }

    let tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL)]),
    );
    tui.wait_pred(
        |_| {
            checkouts
                .iter()
                .all(|c| badge_is(&tui, &c.dir, Some(&c.ref_text()), 'r'))
        },
        "all four checkouts show a running PR",
        GIT_WAIT,
    );

    // A whole poll cycle just ended when the log holds a multiple of four
    // list calls past the first lookups; swap in the gap before the next.
    let cycle_end = list_calls(&forge).len() + 4;
    tui.wait_pred(
        |_| list_calls(&forge).len() >= cycle_end,
        "a poll cycle ran",
        GIT_WAIT,
    );
    tui.wait_ms(200);
    for checkout in &checkouts {
        checkout.set(&forge, Badge::CiFailed);
    }

    tui.wait_pred(
        |screen| toast_count(screen, "more PR changes") == 1,
        "the summary toast shows",
        GIT_WAIT,
    );
    let screen = tui.screen();
    let toasts = toast_text(&screen);
    assert_eq!(
        toast_count(&screen, "CI failed ·"),
        2,
        "two alert toasts, not four:\n{toasts}"
    );
    assert!(
        toasts.contains("+2 more PR changes"),
        "the summary counts the two left out:\n{toasts}"
    );
    assert!(
        !toasts.contains("gp"),
        "phase A has no key hint on the summary:\n{toasts}"
    );
    tui.wait_pred(
        |_| {
            checkouts
                .iter()
                .all(|c| badge_is(&tui, &c.dir, Some(&c.ref_text()), 'x'))
        },
        "all four rows end in `x`",
        WAIT,
    );
}

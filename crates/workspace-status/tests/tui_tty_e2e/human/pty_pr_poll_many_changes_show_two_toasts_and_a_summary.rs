use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, list_calls, named_workspace, seed_checkouts, toast_count, toast_text,
    Forge, PrCheckout,
};
use crate::support::{open_command_palette, type_palette_filter, GIT_WAIT, WAIT};

/// Four PRs that change in one batch raise two toasts and a summary
/// (`docs/tui-rust.md`, **PR change toasts**: a batch of more than three
/// changes shows the first two and `+N more PR changes`, with no key hint).
///
/// Seed: two GitHub checkouts (`#7`, `#8`) and two GitLab checkouts (`!5`,
/// `!6`), all CI running, polling off. Once the four badges show, the test
/// swaps all four answers to CI failed and runs palette **Refresh PRs**
/// once: the four lookups queue together, so their changes alert as one
/// batch. No poll timing is involved.
///
/// Proof: the toast stack holds two `CI failed ·` toasts and the summary
/// `+2 more PR changes`, and the summary carries no key hint. Reverting the
/// batch cap shows four toasts (or a stack that pushes the others out);
/// reverting the summary drops the text.
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

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", "0")]),
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
    let before = list_calls(&forge).len();

    for checkout in &checkouts {
        checkout.set(&forge, Badge::CiFailed);
    }
    open_command_palette(&mut tui);
    type_palette_filter(&mut tui, "refresh prs");
    tui.wait_contains("Refresh PRs", WAIT);
    tui.enter();

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
        "the summary has no key hint:\n{toasts}"
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
    assert_eq!(
        list_calls(&forge).len(),
        before + 4,
        "one Refresh PRs looked each checkout up once, and nothing polled"
    );
}

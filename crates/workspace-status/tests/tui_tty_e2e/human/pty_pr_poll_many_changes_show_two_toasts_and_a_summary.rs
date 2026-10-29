use std::cell::Cell;
use std::time::{Duration, Instant};

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, list_calls, named_workspace, seed_checkouts, toast_count, toast_text,
    Forge, PrCheckout,
};
use crate::support::{GIT_WAIT, WAIT};

/// Poll period of this test. Much longer than a poll cycle, so the quiet gap
/// between two cycles dwarfs a scheduling stall of the test thread on a
/// loaded machine.
const POLL: &str = "2000";

/// The whole call log must stay this still before the answers are swapped.
const STILL: Duration = Duration::from_millis(300);

/// Wait until a poll cycle has ended: `floor` list calls or more, a multiple
/// of four (one per checkout), and no call of any kind for `STILL`. The
/// next cycle is then about `POLL` minus the cycle away.
fn wait_between_cycles(tui: &PtySession, forge: &FakeForge, floor: usize) {
    let last = Cell::new((forge.calls().len(), Instant::now()));
    tui.wait_pred(
        |_| {
            let total = forge.calls().len();
            let (seen, since) = last.get();
            if total != seen {
                last.set((total, Instant::now()));
                return false;
            }
            let lists = list_calls(forge).len();
            lists >= floor && lists % 4 == 0 && since.elapsed() >= STILL
        },
        "a poll cycle ended and the call log is still",
        GIT_WAIT,
    );
}

/// Four PRs that change in one poll cycle raise two toasts and a summary
/// (`docs/tui-rust.md`, **PR change toasts**: a batch of more than three
/// changes shows the first two and `+N more PR changes`, with no key hint).
///
/// Seed: two GitHub checkouts (`#7`, `#8`) and two GitLab checkouts (`!5`,
/// `!6`), all CI running, polled every 2 s. The test waits for a whole poll
/// cycle to end (a multiple of four list calls, then a still call log), and
/// swaps all four answers to CI failed in the gap, so the next cycle finds
/// four changes.
///
/// Proof: the toast stack holds two `CI failed ·` toasts and the summary
/// `+2 more PR changes`, and the summary carries no key hint (`gp` is not
/// bound in this build). Reverting the batch cap shows four toasts (or a
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

    // The launch lookups made the first four list calls. The first whole
    // poll cycle after them ends at the next multiple of four.
    let first_cycle = list_calls(&forge).len() + 4;
    wait_between_cycles(&tui, &forge, first_cycle);
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
        "the summary has no key hint in this build:\n{toasts}"
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

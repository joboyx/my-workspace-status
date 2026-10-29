use std::thread;
use std::time::{Duration, Instant};

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, list_calls, named_workspace, seed_checkouts, Forge, PrCheckout,
};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// How long no list call may appear: about seven 300 ms poll periods.
const QUIET: Duration = Duration::from_millis(2000);

/// The call log keeps its size for `QUIET` while the app runs.
fn calls_stay_at(forge: &FakeForge, count: usize) {
    let start = Instant::now();
    while start.elapsed() < QUIET {
        assert_eq!(
            list_calls(forge).len(),
            count,
            "a list call appeared with polling off: {:?}",
            forge.calls()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

/// `WS_STATUS_PR_POLL_MS=0` turns polling off, and `r` still looks a PR up
/// (`docs/configuration.md`, `WS_STATUS_PR_POLL_MS`).
///
/// Seed: `apibox` (GitLab `!5`) and `appbox` (GitHub `#7`), both in review,
/// launched with `WS_STATUS_PR_POLL_MS=0`.
///
/// Proof: the first lookups make one list call per checkout (a `gh pr list`
/// and one merge request list call). For the next 2 s, with nothing else
/// asked, the call log gets no further list call, although the answers
/// changed to CI failed in the meantime. `r` on each row then makes exactly
/// one more list call for that checkout, the badge turns to `x`, and the log
/// stays still again afterwards. Reverting the switch (polling on, as the
/// sibling tests run it at 300 ms) fills the log within the first window.
#[test]
fn pty_pr_poll_off_stops_polling_and_r_still_looks_up() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-poll-off", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::InReview);
    gitlab.set(&forge, Badge::InReview);

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", "0")]),
    );
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#7"), 'P') && badge_is(&tui, "apibox", Some("!5"), 'P'),
        "both checkouts show a PR in review",
        GIT_WAIT,
    );
    // Both badges show, so both launch list calls are logged. A third,
    // late call would break `calls_stay_at` below.
    tui.wait_pred(
        |_| list_calls(&forge).len() >= 2,
        "both launch list calls are in the log",
        WAIT,
    );
    let launch = list_calls(&forge);
    assert_eq!(
        (
            launch
                .iter()
                .filter(|c| c.starts_with("gh pr list "))
                .count(),
            launch.iter().filter(|c| c.starts_with("glab api ")).count()
        ),
        (1, 1),
        "one list call per checkout at launch: {launch:?}"
    );

    // The answers change; nobody asks, so nothing happens.
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::CiFailed);
    calls_stay_at(&forge, 2);
    assert!(
        badge_is(&tui, "appbox", Some("#7"), 'P') && badge_is(&tui, "apibox", Some("!5"), 'P'),
        "the badges did not move:\n{}",
        tui.screen()
    );

    // `r` on the focused row looks it up.
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "apibox"),
        "cursor on apibox",
        WAIT,
    );
    tui.key('r');
    tui.wait_pred(
        |_| badge_is(&tui, "apibox", Some("!5"), 'x'),
        "r on apibox shows the failed MR",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "appbox"),
        "cursor on appbox",
        WAIT,
    );
    tui.key('r');
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#7"), 'x'),
        "r on appbox shows the failed PR",
        GIT_WAIT,
    );
    assert_eq!(
        list_calls(&forge).len(),
        4,
        "each r made exactly one list call: {:?}",
        forge.calls()
    );
    calls_stay_at(&forge, 4);
}

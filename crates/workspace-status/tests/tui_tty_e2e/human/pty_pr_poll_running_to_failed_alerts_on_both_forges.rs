use std::cell::Cell;
use std::thread;
use std::time::{Duration, Instant};

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::{Badge, FAILED_CHECK};
use crate::harness::PtySession;
use crate::pr_support::{
    badge_flashes, badge_is, click_badge, forge_env, named_workspace, seed_checkouts, toast_text,
    tree_row_text, Forge, PrCheckout, POLL_MS, REMOVE_RAMP,
};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// The workspace row ends in `total`.
fn total_is(tui: &PtySession, total: &str) -> bool {
    tree_row_text(tui, "# ws").is_some_and(|row| row.trim_end().ends_with(&format!("  {total}")))
}

/// The toast rows hold no PR alert text: no ref and no state word.
fn assert_no_alert(tui: &PtySession, when: &str) {
    let toasts = toast_text(&tui.screen());
    assert!(
        !["#7", "!5", "CI failed", "ready to merge", "merged"]
            .iter()
            .any(|text| toasts.contains(text)),
        "no PR toast {when}:\n{}",
        tui.screen()
    );
}

/// A running PR that turns CI failed raises the alert on both forges (AC3,
/// the running to failed half).
///
/// Seed: `apibox` (GitLab `!5`) and `appbox` (GitHub `#7`), both in review,
/// polled every 300 ms. The test swaps both answers to CI running, then to
/// CI failed, and presses no key.
///
/// Proof: no toast shows on first load. The swap to CI running updates the
/// glyph to `r` and raises no toast (only failed, ready, merged and closed
/// alert). After the swap to CI failed the
/// next poll shows `x #7 CI failed · test (ubuntu-latest)` (the failed
/// check) and `x !5 CI failed · feature/rate-limit` (GitLab names the
/// branch), the badge glyph cells sit on the remove flash ramp in the frame
/// that carries the toast, both rows end in `x`, the workspace total reads
/// `2 PRs · 2 need you`, the graph chip of the focused checkout reads
/// `x <ref> CI failed  > fix CI` and the pinned popover reads `x PR · CI
/// failed`. The toast outlives the 2.5 s of an acknowledgement and is gone
/// within the wait budget. Reverting the alert leaves no toast or no flash;
/// reverting polling leaves the badges on `r`.
#[test]
fn pty_pr_poll_running_to_failed_alerts_on_both_forges() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-fail", "ws");
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
        |_| badge_is(&tui, "appbox", Some("#7"), 'P') && badge_is(&tui, "apibox", Some("!5"), 'P'),
        "both checkouts show a PR in review",
        GIT_WAIT,
    );
    assert_no_alert(&tui, "on first load");

    // Review to running is not an alert: the badge changes and nothing
    // else does.
    github.set(&forge, Badge::CiRunning);
    gitlab.set(&forge, Badge::CiRunning);
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#7"), 'r') && badge_is(&tui, "apibox", Some("!5"), 'r'),
        "both badges turn to running",
        GIT_WAIT,
    );
    assert_no_alert(&tui, "on review to running");

    let github_toast = format!("x #7 CI failed · {FAILED_CHECK}");
    let gitlab_toast = "x !5 CI failed · feature/rate-limit";
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::CiFailed);
    let flashed = (Cell::new(false), Cell::new(false));
    let first_seen = Cell::new(None::<Instant>);
    tui.wait_pred(
        |screen| {
            flashed
                .0
                .set(flashed.0.get() || badge_flashes(&tui, "appbox", &REMOVE_RAMP));
            flashed
                .1
                .set(flashed.1.get() || badge_flashes(&tui, "apibox", &REMOVE_RAMP));
            let toasts = toast_text(screen);
            let shown = toasts.contains(&github_toast) && toasts.contains(gitlab_toast);
            if shown && first_seen.get().is_none() {
                first_seen.set(Some(Instant::now()));
            }
            shown
        },
        "the next poll toasts both CI failures",
        GIT_WAIT,
    );
    assert!(
        flashed.0.get() && flashed.1.get(),
        "both rows flash on the remove ramp with the toast: github={} gitlab={}\n{}",
        flashed.0.get(),
        flashed.1.get(),
        tui.screen()
    );
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'x')
                && badge_is(&tui, "apibox", Some("!5"), 'x')
                && total_is(&tui, "2 PRs · 2 need you")
        },
        "both rows end in `x` and the total reads `2 PRs · 2 need you`",
        WAIT,
    );

    // The graph chip of the focused checkout (apibox) and then of appbox.
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "apibox")
                && screen.contains("x !5 CI failed")
                && screen.contains("> fix CI")
        },
        "the graph chip of apibox reads `x !5 CI failed  > fix CI`",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "appbox")
                && screen.contains("x #7 CI failed")
                && screen.contains("> fix CI")
        },
        "the graph chip of appbox reads `x #7 CI failed  > fix CI`",
        WAIT,
    );

    // A toast lives 8 s, not the 2.5 s of an acknowledgement.
    let seen = first_seen.get().expect("toast seen");
    thread::sleep((seen + Duration::from_secs(3)).saturating_duration_since(Instant::now()));
    assert!(
        toast_text(&tui.screen()).contains(&github_toast),
        "the toast is still up 3 s after it appeared:\n{}",
        tui.screen()
    );
    tui.wait_pred(
        |screen| {
            let toasts = toast_text(screen);
            !toasts.contains(&github_toast) && !toasts.contains(gitlab_toast)
        },
        "the toasts go away",
        GIT_WAIT,
    );

    // The popover reads the new state.
    click_badge(&mut tui, "appbox");
    tui.wait_pred(
        |screen| {
            screen.contains("x PR · CI failed")
                && screen.contains("next    > fix CI")
                && screen.contains("fail test (ubuntu-latest)")
        },
        "the GitHub popover reads CI failed and fix CI",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("x PR · CI failed"),
        "Esc closes the popover",
        WAIT,
    );
    click_badge(&mut tui, "apibox");
    tui.wait_pred(
        |screen| {
            screen.contains("x PR · CI failed")
                && screen.contains("next      > fix CI")
                && screen.contains("pipeline failed")
        },
        "the GitLab popover reads CI failed, fix CI and the failed pipeline",
        WAIT,
    );
}

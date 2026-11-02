use crate::common::fake_forge::{FakeForge, Reply};
use crate::common::pr_scenarios::{Badge, GITLAB_PROJECT};
use crate::common::seed::git;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_glyph_paint, badge_is, forge_env, list_calls, named_workspace, seed_checkouts, tab_strip,
    Forge, PrCheckout, DELETED,
};
use crate::support::{GIT_WAIT, WAIT};

/// Poll period. A failing host backs off (2 s, 4 s, ...); a success resets
/// it.
const POLL: &str = "1000";

/// The outage toast text of the failing GitLab host: the reason's leading
/// `glab: ` is cut.
const TOAST: &str = "glab (gitlab.com): 401 invalid_token";

/// The header mark at the right end of the tab strip.
const HEADER: &str = "PR lookup failed";

/// Make the fake `glab` list call of `checkout` exit 1 with an expired
/// token's stderr.
fn fail(forge: &FakeForge, checkout: &PrCheckout) {
    forge.gitlab_mr_list(
        GITLAB_PROJECT,
        &checkout.branch,
        Reply::Fail {
            code: 1,
            stderr: "glab: 401 invalid_token\n",
        },
    );
}

/// GitLab list calls in the fake forge log.
fn gitlab_lookups(forge: &FakeForge) -> usize {
    list_calls(forge)
        .iter()
        .filter(|call| call.contains("/merge_requests?"))
        .count()
}

/// Give `checkout` an upstream (`origin/<branch>` at `HEAD`), with local
/// refs only, so a branch with no PR answer yet is polled.
fn track_origin(checkout: &std::path::Path, branch: &str) {
    git(
        checkout,
        &[
            "update-ref",
            &format!("refs/remotes/origin/{branch}"),
            "HEAD",
        ],
    );
    git(
        checkout,
        &["config", &format!("branch.{branch}.remote"), "origin"],
    );
    git(
        checkout,
        &[
            "config",
            &format!("branch.{branch}.merge"),
            &format!("refs/heads/{branch}"),
        ],
    );
}

/// A failing forge host shows one sticky outage toast and the header mark
/// per outage, and a checkout with no earlier answer paints the failure
/// mark (`docs/tui-rust.md`, **Pull request jobs**, **Pull request badge**).
///
/// Seed: `apibox` (GitLab, pushed) whose `glab` list call fails from launch
/// with `glab: 401 invalid_token`, and `appbox` (GitHub `#7`, in review),
/// polled every second.
///
/// Proof: at launch the toast reads `glab (gitlab.com): 401 invalid_token
/// · Esc dismiss`, the tab strip ends in `PR lookup failed`,
/// and `apibox` ends in a bold red `!` while `appbox` keeps `#7 P`. Esc
/// hides the toast and keeps the header. Two more failed polls bring no
/// toast back. Once `glab` answers, a poll lands `!5 P` and the header
/// goes. A new failure starts a new outage: the toast shows again.
/// Reverting the once-per-outage rule toasts on every failed poll;
/// reverting the recovery rule keeps the header after the good answer;
/// reverting the failure mark leaves the `apibox` badge column blank.
#[test]
fn pty_pr_failing_host_toasts_once_per_outage_and_marks_the_badge() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-host-outage", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    track_origin(&workspace.join(&gitlab.dir), &gitlab.branch);
    github.set(&forge, Badge::InReview);
    fail(&forge, &gitlab);

    let mut tui = PtySession::open_size_with_env(
        &workspace,
        140,
        40,
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL)]),
    );
    tui.wait_pred(
        |screen| {
            screen.contains(&format!("{TOAST} · Esc dismiss"))
                && tab_strip(&tui).ends_with(HEADER)
                && badge_is(&tui, "apibox", None, '!')
                && badge_is(&tui, "appbox", Some("#7"), 'P')
        },
        "the outage toast, the header mark, and the failure mark",
        GIT_WAIT,
    );
    assert_eq!(
        badge_glyph_paint(&tui, "apibox"),
        Some((DELETED, true)),
        "the failure mark paints bold in the theme's deleted colour"
    );
    assert_eq!(
        tui.screen().matches(TOAST).count(),
        1,
        "one toast:\n{}",
        tui.screen()
    );

    // Esc hides the toast; the outage goes on, so the header stays.
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains(TOAST),
        "Esc dismisses the toast",
        WAIT,
    );
    assert!(
        tab_strip(&tui).ends_with(HEADER),
        "the header mark stays: {:?}",
        tab_strip(&tui)
    );

    // More failed polls in the same outage: no toast comes back.
    let before = gitlab_lookups(&forge);
    tui.wait_pred(
        |_| gitlab_lookups(&forge) >= before + 2,
        "two more polls run against the failing glab",
        GIT_WAIT,
    );
    let screen = tui.screen();
    assert!(!screen.contains(TOAST), "no toast again:\n{screen}");
    assert!(tab_strip(&tui).ends_with(HEADER), "{screen}");
    assert!(badge_is(&tui, "apibox", None, '!'), "{screen}");

    // glab answers again: a poll lands the PR and the outage ends.
    gitlab.set(&forge, Badge::InReview);
    tui.wait_pred(
        |_| badge_is(&tui, "apibox", Some("!5"), 'P') && !tab_strip(&tui).contains(HEADER),
        "the badge is back and the header mark is gone",
        GIT_WAIT,
    );

    // A new failure is a new outage: the toast shows again.
    fail(&forge, &gitlab);
    tui.wait_pred(
        |screen| screen.contains(TOAST) && tab_strip(&tui).ends_with(HEADER),
        "a failure after the recovery toasts again",
        GIT_WAIT,
    );
    assert!(
        badge_is(&tui, "apibox", Some("!5"), 'P'),
        "a failure after a good answer keeps the badge:\n{}",
        tui.screen()
    );
}

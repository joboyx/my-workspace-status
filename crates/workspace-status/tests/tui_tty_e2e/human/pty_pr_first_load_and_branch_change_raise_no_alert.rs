use std::time::Duration;

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, named_workspace, seed_checkouts, watch_quiet, Forge, PrCheckout, POLL_MS,
};
use crate::seed::git;
use crate::support::GIT_WAIT;

/// How long the screen must stay free of alerts: several poll periods.
const QUIET: Duration = Duration::from_millis(1800);

/// Opening a workspace and switching a checkout's branch raise no PR alert:
/// only a change of a PR already seen in this session alerts (`docs/tui-model.md`,
/// **PR transitions**).
///
/// Seed: `appbox` (GitHub) on `feature/auth-refresh` with `#7` CI failed,
/// `apibox` (GitLab) on `feature/rate-limit` with `!5` ready, and `donebox`
/// (GitHub) on `feature/shipped` with `#9` merged: the three states that
/// alert when a PR changes into them. Polling runs every 300 ms. Each of
/// `appbox` and `apibox` has a second branch with a PR in another state
/// (`#8` ready, `!6` merged).
///
/// Proof: from launch until the three badges are up and for 1.8 s after
/// (about six polls), the toast rows never carry an alert word. Then
/// `git checkout` moves `appbox` to the branch with `#8` and `apibox` to the
/// one with `!6`; the badges follow (`#8 A`, `!6 m`) and the toast rows stay
/// free of alerts for another 1.8 s. An alert on first load fires on the
/// first window; an alert on a branch change fires on the second.
#[test]
fn pty_pr_first_load_and_branch_change_raise_no_alert() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-quiet", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    let done = PrCheckout::new("donebox", "feature/shipped", Forge::GitHub, 9);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone(), done.clone()]);
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::Ready);
    done.set(&forge, Badge::Merged);

    // The branches the checkouts move to, each with a PR in another state.
    let github_next = PrCheckout::new("appbox", "feature/auth-v2", Forge::GitHub, 8);
    let gitlab_next = PrCheckout::new("apibox", "feature/rate-limit-v2", Forge::GitLab, 6);
    git(&workspace.join("appbox"), &["branch", &github_next.branch]);
    git(&workspace.join("apibox"), &["branch", &gitlab_next.branch]);
    github_next.set(&forge, Badge::Ready);
    gitlab_next.set(&forge, Badge::Merged);

    let tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(
            &forge,
            &[
                ("WS_STATUS_PR_POLL_MS", POLL_MS),
                ("WS_STATUS_WATCH_MS", "500"),
            ],
        ),
    );
    watch_quiet(
        &tui,
        |tui| {
            badge_is(tui, "appbox", Some("#7"), 'x')
                && badge_is(tui, "apibox", Some("!5"), 'A')
                && badge_is(tui, "donebox", Some("#9"), 'm')
        },
        QUIET,
        "on first load",
    );

    git(
        &workspace.join("appbox"),
        &["checkout", "-q", &github_next.branch],
    );
    git(
        &workspace.join("apibox"),
        &["checkout", "-q", &gitlab_next.branch],
    );
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#8"), 'A') && badge_is(&tui, "apibox", Some("!6"), 'm'),
        "the badges follow the new branches",
        GIT_WAIT,
    );
    watch_quiet(
        &tui,
        |tui| badge_is(tui, "appbox", Some("#8"), 'A') && badge_is(tui, "apibox", Some("!6"), 'm'),
        QUIET,
        "on a branch change",
    );
}

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, forge_env, list_calls, named_workspace, seed_checkouts, tree_badge, Forge, PrCheckout,
};
use crate::support::{ack_rows, tree_cursor_on, GIT_WAIT, WAIT};

/// `gc` on the focused tree row, then wait until `forge` has recorded one
/// more open and the acknowledgement `ack` shows.
fn gc_opens(tui: &mut PtySession, forge: &FakeForge, ack: &str, want_url: &str) {
    let before = forge.opened().len();
    tui.send_bytes(b"gc");
    tui.wait_pred(
        |screen| ack_rows(screen).contains(ack) && forge.opened().len() == before + 1,
        &format!("gc acknowledges `{ack}` and records one open"),
        GIT_WAIT,
    );
    assert_eq!(
        forge.opened().last().map(String::as_str),
        Some(want_url),
        "gc opens the CI page"
    );
}

/// `gc` opens the failed check of a GitHub PR and the pipeline of a GitLab
/// merge request, and says so (`docs/tui-rust.md`, **Pull request badge**,
/// the CI link).
///
/// Seed: `apibox` (GitLab `!5`, pipeline failed) above `appbox` (GitHub
/// `#7`, checks `lint` passed and `test (ubuntu-latest)` failed), with the
/// fake `$BROWSER` recorder.
///
/// Proof: `gc` on `appbox` records the link of the failed check (the second
/// check, `.../job/72`, not the first one listed, `lint`) and says `opened CI
/// for #7`; `gc` on `apibox` records the pipeline URL and says `opened CI
/// for !5`. With the GitHub PR ready, `gc` opens the first check with a link.
/// With no PR, `gc` says `no PR for feature/auth-refresh` and records
/// nothing. Reverting `gc` leaves the keys to `g` then `c` (nothing opens);
/// a CI link that takes the first listed check opens `lint` and fails the
/// failed-check URL.
#[test]
fn pty_pr_gc_opens_failed_check_and_pipeline() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-gc", "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::CiFailed);
    gitlab.set(&forge, Badge::CiFailed);

    let mut tui = PtySession::open_size_with_env(&workspace, 140, 32, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#7"), 'x') && badge_is(&tui, "apibox", Some("!5"), 'x'),
        "both checkouts show a failed PR",
        GIT_WAIT,
    );
    assert!(forge.opened().is_empty(), "nothing opened yet");

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "appbox"),
        "j moves the cursor to appbox",
        WAIT,
    );
    gc_opens(
        &mut tui,
        &forge,
        "opened CI for #7",
        &github.ci_url(Badge::CiFailed),
    );

    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "apibox"),
        "k moves the cursor to apibox",
        WAIT,
    );
    gc_opens(
        &mut tui,
        &forge,
        "opened CI for !5",
        &gitlab.ci_url(Badge::CiFailed),
    );

    // Ready GitHub PR: no failed or running check, so the first check with
    // a link opens (`lint`).
    github.set(&forge, Badge::Ready);
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "appbox"),
        "j moves the cursor back to appbox",
        WAIT,
    );
    tui.key('r');
    tui.wait_pred(
        |_| badge_is(&tui, "appbox", Some("#7"), 'A'),
        "after r, appbox shows the ready PR",
        GIT_WAIT,
    );
    gc_opens(
        &mut tui,
        &forge,
        "opened CI for #7",
        &github.ci_url(Badge::Ready),
    );

    // No PR: gc says so and opens nothing.
    github.set_none(&forge);
    let lists = list_calls(&forge).len();
    tui.key('r');
    tui.wait_pred(
        |_| {
            list_calls(&forge).len() > lists
                && tree_badge(&tui, "appbox")
                    .is_some_and(|b| b.ref_text.is_none() && b.glyph != 'A')
        },
        "after r, the lookup ran and appbox shows no PR",
        GIT_WAIT,
    );
    let opened = forge.opened().len();
    tui.send_bytes(b"gc");
    tui.wait_pred(
        |screen| ack_rows(screen).contains("no PR for feature/auth-refresh"),
        "gc says there is no PR for the branch",
        WAIT,
    );
    assert_eq!(forge.opened().len(), opened, "gc opened nothing");
}

use std::time::Duration;

use crate::common::fake_forge::{FakeForge, Reply};
use crate::common::pr_scenarios::{Badge, GITHUB_REPO, GITLAB_PROJECT};
use crate::harness::PtySession;
use crate::pr_support::{
    assert_holds_for, badge_glyph_paint, badge_is, badge_ref_fg, click_badge, forge_env,
    glyph_colour, has_alert, list_calls, named_workspace, seed_checkouts, Forge, PrCheckout,
    MODIFIED, MUTED,
};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// Poll period of this test. A failing host doubles it (1 s, 2 s, 4 s), so
/// after the second failed poll the next poll is 4 s away: long enough that
/// the recovery steps below are the `r` key and not a poll.
const POLL: &str = "1000";

/// How long a kept badge must stay on screen after an `r` against the
/// failing CLI.
const KEPT: Duration = Duration::from_millis(600);

/// Wait until `forge` has logged `more` list lookups past the `before`
/// count: a poll (or `r`) ran against the failing CLI.
fn wait_lookups(tui: &PtySession, forge: &FakeForge, before: usize, more: usize, what: &str) {
    tui.wait_pred(|_| list_calls(forge).len() >= before + more, what, GIT_WAIT);
}

/// The badge of `name` paints its ref muted and its bold glyph in the colour
/// of a PR in review (`stale` false) or muted (`stale` true).
fn badge_paints(tui: &PtySession, name: &str, stale: bool) -> bool {
    let want = if stale {
        MUTED
    } else {
        glyph_colour(Badge::InReview)
    };
    badge_glyph_paint(tui, name) == Some((want, true)) && badge_ref_fg(tui, name) == Some(MUTED)
}

/// A failing `gh` and `glab` keep the last badge, dim it once it is stale,
/// and show the first stderr line in the popover (`docs/tui-rust.md`,
/// **Pull request badge**, stale and failed lookups).
///
/// Seed: `appbox` (GitHub `#7`) and `apibox` (GitLab `!5`), both in review,
/// polled every second. The test then makes both fake CLIs exit 1 with a
/// two-line stderr.
///
/// Proof: after two failed polls per forge both rows still end in `#7 P`
/// and `!5 P`, no alert shows, and the glyphs paint muted (stale: the last
/// success is older than two poll periods) while staying bold. The pinned
/// popover `checked` line reads `last lookup failed: <first stderr line>`
/// in amber, without the second stderr line. `r` against the failing CLI
/// keeps the badge: once its call is logged, the badge stays on screen for
/// 600 ms. Once the answers come back, the glyphs are still stale (the
/// failed host is backed off, so no poll comes for 4 s) until `r` looks each
/// checkout up, and then each paints its state colour again. Reverting the
/// keep-last-badge rule drops the badge on the first failed poll (the old
/// behaviour: a failed lookup hides it); reverting the stale rule leaves the
/// blue glyph; reverting the stderr line leaves the `checked` line without
/// the reason.
#[test]
fn pty_pr_failing_cli_keeps_the_badge_and_names_the_error() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-cli-fails", "ws");
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
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL)]),
    );
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'P')
                && badge_is(&tui, "apibox", Some("!5"), 'P')
                && badge_paints(&tui, "appbox", false)
                && badge_paints(&tui, "apibox", false)
        },
        "both checkouts show a PR in review in its state colour",
        GIT_WAIT,
    );

    let before = list_calls(&forge).len();
    forge.github_pr_list(
        GITHUB_REPO,
        &github.branch,
        Reply::Fail {
            code: 1,
            stderr: "gh: HTTP 502\nsecond line of stderr\n",
        },
    );
    forge.gitlab_mr_list(
        GITLAB_PROJECT,
        &gitlab.branch,
        Reply::Fail {
            code: 1,
            stderr: "glab: HTTP 502\nsecond line of stderr\n",
        },
    );
    wait_lookups(
        &tui,
        &forge,
        before,
        4,
        "two polls per forge run against the failing CLIs",
    );
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'P')
                && badge_is(&tui, "apibox", Some("!5"), 'P')
                && badge_paints(&tui, "appbox", true)
                && badge_paints(&tui, "apibox", true)
        },
        "the badges stay and turn muted (stale)",
        GIT_WAIT,
    );
    assert!(!has_alert(&tui.screen()), "no alert:\n{}", tui.screen());

    // `r` against the failing CLI keeps the badge: once the call is logged,
    // the failure lands within milliseconds, and the badge must be there
    // through the whole window, not only before the answer.
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "apibox"),
        "cursor on apibox",
        WAIT,
    );
    let before = list_calls(&forge).len();
    tui.key('r');
    wait_lookups(&tui, &forge, before, 1, "r looks the checkout up");
    assert_holds_for(
        &tui,
        |tui| badge_is(tui, "apibox", Some("!5"), 'P'),
        KEPT,
        "r against the failing CLI keeps the badge",
    );

    // The popover names the reason: first stderr line only, in amber.
    click_badge(&mut tui, "appbox");
    tui.wait_pred(
        |screen| screen.contains("last lookup failed: gh: HTTP 502"),
        "the GitHub popover names the failure",
        WAIT,
    );
    assert!(
        !tui.screen().contains("second line of stderr"),
        "only the first stderr line shows:\n{}",
        tui.screen()
    );
    let fgs = tui
        .first_needle_fgs("last lookup failed")
        .expect("failure text on screen");
    assert!(
        fgs.iter().all(|fg| *fg == Some(MODIFIED)),
        "the failure reads amber: {fgs:?}"
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("last lookup failed"),
        "Esc closes the popover",
        WAIT,
    );
    click_badge(&mut tui, "apibox");
    tui.wait_pred(
        |screen| screen.contains("last lookup failed: glab: HTTP 502"),
        "the GitLab popover names the failure",
        WAIT,
    );
    assert!(
        !tui.screen().contains("second line of stderr"),
        "only the first stderr line shows:\n{}",
        tui.screen()
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("last lookup failed"),
        "Esc closes the popover",
        WAIT,
    );

    tui.wait_pred(
        |screen| tree_cursor_on(screen, "apibox"),
        "the cursor is on apibox again",
        WAIT,
    );

    // The forge answers again. The failed hosts are backed off (the next
    // poll is seconds away), so the glyphs stay stale until `r` looks the
    // checkout up: the colour comes back with the keys, not with a poll. The
    // swap, the stale check and the three keys (`r` on apibox, `j`, `r` on
    // appbox) follow each other without a wait, so no poll fits between
    // them; the two lookups in the log are the two `r` keys.
    github.set(&forge, Badge::InReview);
    gitlab.set(&forge, Badge::InReview);
    assert!(
        badge_paints(&tui, "apibox", true) && badge_paints(&tui, "appbox", true),
        "stale until a lookup succeeds:\n{}",
        tui.screen()
    );
    let before = list_calls(&forge).len();
    tui.keys("rjr");
    tui.wait_pred(
        |_| badge_paints(&tui, "apibox", false) && badge_paints(&tui, "appbox", false),
        "the two r keys bring both state colours back",
        GIT_WAIT,
    );
    assert_eq!(
        list_calls(&forge).len(),
        before + 2,
        "the two r keys made the two lookups: {:?}",
        forge.calls()
    );
}

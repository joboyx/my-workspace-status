use crate::common::fake_forge::{FakeForge, Reply};
use crate::common::pr_scenarios::{Badge, GITHUB_REPO, GITLAB_PROJECT};
use crate::harness::PtySession;
use crate::pr_support::{
    badge_glyph_paint, badge_is, badge_ref_fg, click_badge, forge_env, glyph_colour, has_alert,
    list_calls, named_workspace, seed_checkouts, Forge, PrCheckout, MODIFIED, MUTED, POLL_MS,
};
use crate::support::{tree_cursor_on, GIT_WAIT, WAIT};

/// Wait until `forge` has logged `more` list lookups past the `before`
/// count: a poll (or `r`) ran against the failing CLI.
fn wait_lookups(tui: &PtySession, forge: &FakeForge, before: usize, more: usize, what: &str) {
    tui.wait_pred(
        |_| crate::pr_support::list_calls(forge).len() >= before + more,
        what,
        GIT_WAIT,
    );
}

/// Both badges still paint their state: ref and glyph, in the colour of
/// `badge` (`stale` false) or muted (`stale` true), glyph bold.
fn badges_paint(tui: &PtySession, stale: bool) -> bool {
    let want = if stale {
        MUTED
    } else {
        glyph_colour(Badge::InReview)
    };
    ["appbox", "apibox"].iter().all(|name| {
        badge_glyph_paint(tui, name) == Some((want, true)) && badge_ref_fg(tui, name) == Some(MUTED)
    })
}

/// A failing `gh` and `glab` keep the last badge, dim it once it is stale,
/// and show the first stderr line in the popover (AC5).
///
/// Seed: `appbox` (GitHub `#7`) and `apibox` (GitLab `!5`), both in review,
/// polled every 300 ms. The test then makes both fake CLIs exit 1 with a
/// two-line stderr.
///
/// Proof: after several failed polls both rows still end in `#7 P` and
/// `!5 P`, no alert shows, and the glyphs paint muted (stale: the last
/// success is older than two poll periods) while staying bold. The pinned
/// popover `checked` line reads `last lookup failed: <first stderr line>`
/// in amber, without the second stderr line. `r` while the CLI fails keeps
/// the badge. Once the answers come back, `r` brings the state colour back.
/// Reverting the keep-last-badge rule drops the badge on the first failed
/// poll (the old behaviour: a failed lookup hides it); reverting the stale
/// rule leaves the blue glyph; reverting the stderr line leaves the
/// `checked` line without the reason.
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
        &forge_env(&forge, &[("WS_STATUS_PR_POLL_MS", POLL_MS)]),
    );
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'P')
                && badge_is(&tui, "apibox", Some("!5"), 'P')
                && badges_paint(&tui, false)
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
    wait_lookups(&tui, &forge, before, 4, "the CLIs are called and fail");
    tui.wait_pred(
        |_| {
            badge_is(&tui, "appbox", Some("#7"), 'P')
                && badge_is(&tui, "apibox", Some("!5"), 'P')
                && badges_paint(&tui, true)
        },
        "the badges stay and turn muted (stale)",
        GIT_WAIT,
    );
    assert!(!has_alert(&tui.screen()), "no alert:\n{}", tui.screen());

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

    // `r` against the failing CLI keeps the badge too.
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "apibox"),
        "cursor on apibox",
        WAIT,
    );
    let before = list_calls(&forge).len();
    tui.key('r');
    wait_lookups(&tui, &forge, before, 1, "r looks the checkout up");
    tui.wait_pred(
        |_| badge_is(&tui, "apibox", Some("!5"), 'P'),
        "r against the failing CLI keeps the badge",
        WAIT,
    );

    // The forge answers again: `r` brings the state colour back.
    github.set(&forge, Badge::InReview);
    gitlab.set(&forge, Badge::InReview);
    tui.key('r');
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "appbox"),
        "cursor on appbox",
        WAIT,
    );
    tui.key('r');
    tui.wait_pred(
        |_| badges_paint(&tui, false),
        "after a good answer the glyphs paint in their state colour again",
        GIT_WAIT,
    );
}

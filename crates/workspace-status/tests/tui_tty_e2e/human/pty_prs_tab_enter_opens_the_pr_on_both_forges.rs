use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::{github_pr_url, gitlab_mr_url};
use crate::harness::PtySession;
use crate::pr_support::{forge_env, prs_list, prs_move_to, seed_prs, total_is, PRS_BOARD};
use crate::support::{ack_rows, GIT_WAIT, WAIT};

/// Enter on the focused row, then wait until `forge` has recorded one more
/// open and the acknowledgement `ack` shows.
fn enter_opens(tui: &mut PtySession, forge: &FakeForge, ack: &str, want_url: &str) {
    let before = forge.opened().len();
    tui.enter();
    tui.wait_pred(
        |screen| ack_rows(screen).contains(ack) && forge.opened().len() == before + 1,
        &format!("Enter acknowledges `{ack}` and records one open"),
        WAIT,
    );
    assert_eq!(
        forge.opened().last().map(String::as_str),
        Some(want_url),
        "Enter opens the PR page"
    );
}

/// Enter on a PRs tab row opens that PR in the browser, on GitHub and on
/// GitLab (`docs/tui-rust.md`, **PRs tab**).
///
/// Seed: [`PRS_BOARD`], with the fake `$BROWSER` recorder. The first group
/// lists GitHub `#40` above GitLab `!31`.
///
/// Proof: Enter on `#40` records `https://github.com/acme/app/pull/40` and
/// says `opened PR #40`; Enter on `!31` records the merge request URL
/// `https://gitlab.com/acme/services/api/-/merge_requests/31` and says
/// `opened PR #31`. Each Enter records exactly one open. An Enter that
/// acted on the header's PR or on the tree cursor's checkout would open
/// `!31` first, or the PR of `api-cache`.
#[test]
fn pty_prs_tab_enter_opens_the_pr_on_both_forges() {
    let (_root, workspace, forge) = seed_prs("ws-tui-tty-prs-enter", &PRS_BOARD);
    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "9 PRs · 2 need you"),
        "every lookup lands: the workspace row counts nine PRs",
        GIT_WAIT,
    );
    tui.keys("gp");
    tui.wait_pred(|_| prs_list(&tui).is_some(), "gp opens the PRs tab", WAIT);
    assert!(forge.opened().is_empty(), "nothing opened yet");

    prs_move_to(&mut tui, "#40");
    enter_opens(&mut tui, &forge, "opened PR #40", &github_pr_url(40));
    prs_move_to(&mut tui, "!31");
    enter_opens(&mut tui, &forge, "opened PR #31", &gitlab_mr_url(31));
}

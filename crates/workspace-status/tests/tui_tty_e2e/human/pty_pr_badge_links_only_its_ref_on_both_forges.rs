use std::collections::{BTreeMap, BTreeSet};

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::{github_pr_url, gitlab_mr_url, Badge};
use crate::harness::PtySession;
use crate::pr_support::{
    badge_is, click_badge, forge_env, named_workspace, seed_checkouts, Forge, PrCheckout,
};
use crate::support::{links_since, tree_cursor_on, GIT_WAIT, WAIT};

/// Full PR URL of `checkout` on its forge.
fn pr_url(checkout: &PrCheckout) -> String {
    match checkout.forge {
        Forge::GitHub => github_pr_url(checkout.number),
        Forge::GitLab => gitlab_mr_url(checkout.number),
    }
}

/// The graph header chip of a CI failed PR: `x <ref> CI failed  > fix CI`.
fn failed_chip(checkout: &PrCheckout) -> String {
    format!("x {} CI failed  > fix CI", checkout.ref_text())
}

/// The terminal hyperlink of a PR badge wraps only the PR ref, on both
/// forges and on both places the badge paints: the tree row (`<ref>
/// <glyph>`) and the graph header chip (`<glyph> <ref> <label>`). The
/// glyph stays a plain cell, and an in-app click on it still pins the PR
/// popover (`docs/tui-rust.md`, **Pull request badge**).
///
/// Seed: `apibox` (GitLab `!20`) and `appbox` (GitHub `#10`), both CI
/// failed. The tree cursor visits each checkout so its graph header chip
/// paints too, then goes back to `apibox`, so the `appbox` chip frame and
/// its links are flushed before the raw output is read.
///
/// Proof: the OSC 8 links read from the raw PTY bytes (`vt100` drops OSC 8)
/// to each full PR URL wrap exactly one text, the ref (`!20`, `#10`); at
/// least one such link exists, so the link writer is live. A click on the
/// `appbox` glyph cell pins the `x PR · CI failed` popover. Reverting the
/// link span to the whole badge or chip makes the link text `#10 x` or
/// `x #10` and fails the link check.
#[test]
fn pty_pr_badge_links_only_its_ref_on_both_forges() {
    let (root, workspace) = named_workspace("ws-tui-tty-pr-link-ref", "ws");
    let forge = FakeForge::new(&root);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 20);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 10);
    seed_checkouts(&workspace, &[gitlab.clone(), github.clone()]);
    gitlab.set(&forge, Badge::CiFailed);
    github.set(&forge, Badge::CiFailed);

    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| {
            badge_is(&tui, "apibox", Some("!20"), 'x') && badge_is(&tui, "appbox", Some("#10"), 'x')
        },
        "apibox shows `!20 x` and appbox shows `#10 x`",
        GIT_WAIT,
    );

    // The tree cursor starts on apibox: its graph header chip paints.
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "apibox") && screen.contains(&failed_chip(&gitlab)),
        "the graph chip of apibox reads `x !20 CI failed  > fix CI`",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "appbox") && screen.contains(&failed_chip(&github)),
        "j moves onto appbox; its graph chip reads `x #10 CI failed  > fix CI`",
        GIT_WAIT,
    );
    // A later frame, so the links of the appbox chip frame are in the raw
    // output too.
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "apibox") && screen.contains(&failed_chip(&gitlab)),
        "k moves back onto apibox and its graph chip",
        GIT_WAIT,
    );

    // Distinct wrapped texts per PR URL, read before the click so the
    // popover cannot add links.
    let checkouts = [&gitlab, &github];
    let mut seen: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (url, text) in links_since(&tui, 0) {
        if checkouts.iter().any(|c| pr_url(c) == url) {
            seen.entry(url).or_default().insert(text);
        }
    }

    // The glyph cell still pins the PR popover on click.
    click_badge(&mut tui, "appbox");
    tui.wait_pred(
        |screen| screen.contains("x PR · CI failed"),
        "a click on the appbox glyph pins the `x PR · CI failed` popover",
        WAIT,
    );

    let expected: BTreeMap<String, BTreeSet<String>> = checkouts
        .iter()
        .map(|c| (pr_url(c), BTreeSet::from([c.ref_text()])))
        .collect();
    assert!(
        !seen.is_empty(),
        "no OSC 8 link to a PR URL was written; links: {:?}",
        links_since(&tui, 0)
    );
    assert_eq!(
        seen, expected,
        "each PR badge link must wrap only its ref (PR URL -> distinct link texts)"
    );
}

use crate::harness::PtySession;
use crate::pr_support::{
    badge_glyph_paint, badge_is, badge_ref_fg, forge_env, glyph_colour, state_board, MUTED,
};
use crate::support::GIT_WAIT;

/// Each of the six PR states paints its own glyph, colour and ref, on a
/// GitHub checkout and on a GitLab checkout (AC1).
///
/// Seed: twelve checkouts on one workspace, one per state on each forge
/// (`gh-failed` is `#10`, `gl-failed` is `!20`, and so on). The fake `gh`
/// and `glab` answer each branch in its state.
///
/// Proof: every tree row ends in `<ref> <glyph>` with the ASCII letter
/// (`x A r d P m`) in the last cell before the pane border. The glyph paints
/// bold in the Tokyo Night colour of its state (red, green, amber, muted,
/// blue, purple) and the ref paints muted. A GitHub row and a GitLab row of
/// the same state share glyph and colour. Reverting the six-state badge
/// fails the glyph or the colour of the draft, running, or failed rows (the
/// old badge had three states); reverting the ref placement fails the
/// `<ref> <glyph>` match.
#[test]
fn pty_pr_badge_shows_every_state_on_both_forges() {
    let (_root, workspace, forge, board) = state_board("ws-tui-tty-pr-states");
    let tui = PtySession::open_size_with_env(&workspace, 140, 32, &forge_env(&forge, &[]));

    for (checkout, badge) in &board {
        let want = (checkout.ref_text(), badge.ascii());
        tui.wait_pred(
            |_| badge_is(&tui, &checkout.dir, Some(&want.0), want.1),
            &format!("{} ends in `{} {}`", checkout.dir, want.0, want.1),
            GIT_WAIT,
        );
    }

    for (checkout, badge) in &board {
        let paint = badge_glyph_paint(&tui, &checkout.dir)
            .unwrap_or_else(|| panic!("no badge paint for {}:\n{}", checkout.dir, tui.screen()));
        assert_eq!(
            paint,
            (glyph_colour(*badge), true),
            "{} glyph `{}` paints bold in the colour of its state",
            checkout.dir,
            badge.ascii()
        );
        assert_eq!(
            badge_ref_fg(&tui, &checkout.dir),
            Some(MUTED),
            "{} ref paints muted",
            checkout.dir
        );
    }

    let calls = forge.calls();
    assert!(
        calls.iter().any(|call| call.starts_with("gh pr list ")),
        "gh was called: {calls:?}"
    );
    assert!(
        calls.iter().any(|call| call.starts_with("glab api ")),
        "glab was called: {calls:?}"
    );
}

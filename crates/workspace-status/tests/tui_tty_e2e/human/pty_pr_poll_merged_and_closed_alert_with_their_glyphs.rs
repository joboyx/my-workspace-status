use std::cell::Cell;

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::PtySession;
use crate::pr_support::{
    badge_flashes, badge_is, forge_env, named_workspace, row_flashes, seed_checkouts, toast_text,
    tree_badge, Forge, PrCheckout, ADD_RAMP, POLL_MS, REMOVE_RAMP,
};
use crate::support::GIT_WAIT;

/// Run one merged and one closed transition, one per forge, with the glyph
/// set `glyphs` (`ascii` or `nerd`).
///
/// `merged_on` is the forge whose PR merges; the other forge's PR closes.
/// `merged` and `closed` are the toast glyphs of the set.
fn merged_and_closed(prefix: &str, glyphs: &str, merged_on: Forge, merged: char, closed: char) {
    let (root, workspace) = named_workspace(prefix, "ws");
    let forge = FakeForge::new(&root);
    let github = PrCheckout::new("appbox", "feature/auth-refresh", Forge::GitHub, 7);
    let gitlab = PrCheckout::new("apibox", "feature/rate-limit", Forge::GitLab, 5);
    seed_checkouts(&workspace, &[github.clone(), gitlab.clone()]);
    github.set(&forge, Badge::Ready);
    gitlab.set(&forge, Badge::Ready);
    let (merging, closing) = match merged_on {
        Forge::GitHub => (&github, &gitlab),
        Forge::GitLab => (&gitlab, &github),
    };

    let tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(
            &forge,
            &[
                ("WS_STATUS_PR_POLL_MS", POLL_MS),
                ("WS_STATUS_GLYPHS", glyphs),
            ],
        ),
    );
    let ready = if glyphs == "nerd" {
        Badge::Ready.nerd()
    } else {
        Badge::Ready.ascii()
    };
    tui.wait_pred(
        |_| {
            badge_is(&tui, &github.dir, Some(&github.ref_text()), ready)
                && badge_is(&tui, &gitlab.dir, Some(&gitlab.ref_text()), ready)
        },
        "both checkouts show a ready PR",
        GIT_WAIT,
    );

    merging.set(&forge, Badge::Merged);
    closing.set_closed(&forge);
    let merged_toast = format!(
        "{merged} {} merged · {}",
        merging.ref_text(),
        merging.branch
    );
    let closed_toast = format!(
        "{closed} {} closed · {}",
        closing.ref_text(),
        closing.branch
    );
    let flashed = (Cell::new(false), Cell::new(false));
    tui.wait_pred(
        |screen| {
            flashed
                .0
                .set(flashed.0.get() || badge_flashes(&tui, &merging.dir, &ADD_RAMP));
            flashed
                .1
                .set(flashed.1.get() || row_flashes(&tui, &closing.dir, &REMOVE_RAMP));
            let toasts = toast_text(screen);
            toasts.contains(&merged_toast) && toasts.contains(&closed_toast)
        },
        "the next poll toasts the merge and the close",
        GIT_WAIT,
    );
    assert!(
        flashed.0.get() && flashed.1.get(),
        "the merged row flashes on the add ramp ({}) and the closed row on the remove ramp ({}):\n{}",
        flashed.0.get(),
        flashed.1.get(),
        tui.screen()
    );
    let merged_glyph = if glyphs == "nerd" {
        Badge::Merged.nerd()
    } else {
        Badge::Merged.ascii()
    };
    tui.wait_pred(
        |_| {
            badge_is(&tui, &merging.dir, Some(&merging.ref_text()), merged_glyph)
                && tree_badge(&tui, &closing.dir)
                    .is_some_and(|b| b.ref_text.is_none() && b.glyph != ready)
        },
        "the merged row shows the merged glyph and the closed row loses its badge",
        GIT_WAIT,
    );
}

/// A merged GitHub PR and a closed GitLab merge request alert with the
/// ASCII glyphs (`docs/tui-rust.md`, **PR change toasts**: into merged, and
/// closed, whose badge goes away).
///
/// Seed: `appbox` (`#7`) and `apibox` (`!5`), both ready to merge, polled
/// every 300 ms; the test swaps the GitHub PR to merged and the GitLab one
/// to closed without merging.
///
/// Proof: toasts `m #7 merged · feature/auth-refresh` and `c !5 closed ·
/// feature/rate-limit`; the merged row flashes on the add ramp and ends in
/// `m`; the closed row flashes on the remove ramp and loses its badge.
/// Reverting the merged or closed alert leaves its toast out; reverting the
/// closed flash leaves the row plain.
#[test]
fn pty_pr_poll_github_merge_and_gitlab_close_alert_in_ascii() {
    merged_and_closed("ws-tui-tty-pr-done-ascii", "ascii", Forge::GitHub, 'm', 'c');
}

/// A merged GitLab merge request and a closed GitHub PR alert with the
/// Nerd glyphs `✓` and `⊘` (`docs/tui-rust.md`, **PR change toasts**).
///
/// Seed and proof as the ASCII test with the forges swapped: toasts
/// `✓ !5 merged · feature/rate-limit` and `⊘ #7 closed ·
/// feature/auth-refresh`. Reverting the closed glyph `⊘` fails the closed
/// toast.
#[test]
fn pty_pr_poll_gitlab_merge_and_github_close_alert_in_nerd() {
    merged_and_closed(
        "ws-tui-tty-pr-done-nerd",
        "nerd",
        Forge::GitLab,
        '\u{2713}',
        '\u{2298}',
    );
}

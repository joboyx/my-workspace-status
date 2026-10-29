use crate::harness::PtySession;
use crate::pr_support::{
    forge_env, prs_cursor_on, prs_list, prs_move_to, seed_prs, tab_strip, total_is, PRS_BOARD,
};
use crate::support::{crumb_row, tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// `o` on a PRs tab row switches to the Workspace tab with the tree cursor
/// on the checkout of that PR, unfolding its parents; a second `gp` goes
/// back to the one PRs tab where it was (`docs/tui-rust.md`, **PRs tab**).
///
/// Seed: [`PRS_BOARD`]. Before `gp`, `h` on the workspace row folds it, so
/// no checkout row is on screen.
///
/// Proof: with the cursor on `#45`, `o` paints the tree again (no `prs`
/// pane), the workspace row is unfolded, the cursor bar is on
/// `app-export-ui` (the checkout of `#45`, which is not the first or last
/// row), and the breadcrumb reads `ws › app-export-ui`. `gp` then shows the
/// PRs tab again with the cursor still on `#45`, and the tab strip still has
/// one PRs tab. Reverting the unfold leaves the workspace row folded and the
/// cursor on it; an `o` that only switches tabs leaves the cursor on the
/// workspace row.
#[test]
fn pty_prs_tab_o_shows_the_checkout_in_the_tree() {
    let (_root, workspace, forge) = seed_prs("ws-tui-tty-prs-o", &PRS_BOARD);
    let mut tui = PtySession::open_size_with_env(&workspace, 140, 40, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| total_is(&tui, "# ws", "9 PRs · 2 need you"),
        "every lookup lands: the workspace row counts nine PRs",
        GIT_WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "# ws"),
        "k moves the cursor to the workspace row",
        WAIT,
    );
    tui.key('h');
    tui.wait_pred(
        |screen| !tree_has(screen, "app-export-ui") && !tree_has(screen, "api-cache"),
        "h folds the workspace row: no checkout row shows",
        WAIT,
    );

    tui.keys("gp");
    tui.wait_pred(|_| prs_list(&tui).is_some(), "gp opens the PRs tab", WAIT);
    prs_move_to(&mut tui, "#45");

    tui.key('o');
    tui.wait_pred(
        |screen| {
            prs_list(&tui).is_none()
                && tree_cursor_on(screen, "app-export-ui")
                && tree_has(screen, "api-cache")
                && tree_has(screen, "app-retry")
        },
        "o shows the Workspace tab, unfolded, with the cursor on the checkout of #45",
        WAIT,
    );
    tui.wait_pred(
        |screen| crumb_row(screen).contains("ws › app-export-ui"),
        "the breadcrumb names the checkout",
        WAIT,
    );

    tui.keys("gp");
    tui.wait_pred(
        |_| prs_cursor_on(&tui, "#45"),
        "a second gp goes back to the PRs tab, cursor still on #45",
        WAIT,
    );
    let strip = tab_strip(&tui);
    assert_eq!(
        strip.matches("PRs").count(),
        1,
        "one PRs tab in the strip: {strip:?}"
    );
}

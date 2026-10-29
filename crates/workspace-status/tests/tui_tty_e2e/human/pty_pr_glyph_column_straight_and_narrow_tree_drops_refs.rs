use std::fs;

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::{PtySession, UserConfig};
use crate::pr_support::{
    badge_is, forge_env, frame_fits_terminal, seed_checkouts, state_board, tree_edge, Forge,
    PrCheckout,
};
use crate::support::GIT_WAIT;

/// Narrowest tree (inner columns) that paints refs. A narrower tree paints
/// the glyph alone on every row (`docs/tui-rust.md`, **Pull request badge**).
const REF_MIN_COLS: u16 = 50;

/// Two more checkouts whose rows carry other marks before the badge: a
/// dirty GitHub checkout in review and a dirty GitLab one in draft.
fn extra_checkouts() -> Vec<(PrCheckout, Badge)> {
    vec![
        (
            PrCheckout::new(
                "gh-dirty",
                "feature/gh-dirty-with-a-longer-name",
                Forge::GitHub,
                30,
            ),
            Badge::InReview,
        ),
        (
            PrCheckout::new("gl-dirty", "feature/gl-dirty", Forge::GitLab, 40),
            Badge::Draft,
        ),
    ]
}

/// Every row of the board and the extras must show its badge: the ref (when
/// `refs`) and the ASCII glyph in the tree's edge cell, which
/// [`tree_edge`] reads from the pane border. So every glyph sits in one
/// column, and a badge that sits earlier in the trailing run leaves another
/// mark in that cell and fails.
fn board_matches(tui: &PtySession, board: &[(PrCheckout, Badge)], refs: bool) -> bool {
    board.iter().all(|(checkout, badge)| {
        let ref_text = checkout.ref_text();
        badge_is(
            tui,
            &checkout.dir,
            refs.then_some(ref_text.as_str()),
            badge.ascii(),
        )
    })
}

/// The board plus the extras, seeded and answered by the fake forge.
fn fixture(
    prefix: &str,
) -> (
    std::path::PathBuf,
    std::path::PathBuf,
    FakeForge,
    Vec<(PrCheckout, Badge)>,
) {
    let (root, workspace, forge, mut board) = state_board(prefix);
    for (checkout, badge) in extra_checkouts() {
        seed_checkouts(&workspace, std::slice::from_ref(&checkout));
        fs::write(workspace.join(&checkout.dir).join("README.md"), "dirty\n").unwrap();
        checkout.set(&forge, badge);
        board.push((checkout, badge));
    }
    (root, workspace, forge, board)
}

/// One fresh launch at `cols` columns: the inner tree is `inner` columns
/// (read off the cell grid), every row's glyph sits in the tree's edge
/// cell, and the refs are on every row exactly when `inner` is 50 or more.
///
/// A fresh launch, not a resize: the split a resize leaves does not follow
/// the width (the tree keeps the share it had), so only a launch gives a
/// known inner width.
fn launch_at(prefix: &str, cols: u16, inner: u16) {
    let (_root, workspace, forge, board) = fixture(prefix);
    let tui = PtySession::open_size_with_env(&workspace, cols, 36, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| {
            frame_fits_terminal(&tui)
                && tree_edge(&tui).is_some_and(|edge| {
                    edge.glyph_col == inner && board_matches(&tui, &board, inner >= REF_MIN_COLS)
                })
        },
        &format!(
            "{cols} columns give an inner tree of {inner}: every row ends at the edge, refs {}",
            if inner >= REF_MIN_COLS { "on" } else { "off" }
        ),
        GIT_WAIT,
    );
}

/// The glyph column is straight at every tree width, and a tree under 50
/// columns drops the refs on every row.
///
/// Seed: fourteen checkouts (six states on each forge, plus two dirty rows
/// whose other marks differ), fake `gh` / `glab` answers per state.
///
/// Proof: at 140 columns each row ends in `<ref> <glyph>`. The terminal then
/// resizes to 100 and 120 columns and back to 140. At each width every
/// row's glyph sits in the pane's last cell (one column, read from the pane
/// border), and the refs are on every row when the inner tree is 50 columns
/// or wider and on no row when narrower. Reverting the ref drop keeps refs
/// on a narrow tree and fails the narrow wait; moving the badge earlier in
/// the trailing run leaves another mark in the edge cell of the dirty rows
/// and fails the badge check. The 50-column boundary itself is the next two
/// tests.
#[test]
fn pty_pr_glyph_column_straight_and_narrow_tree_drops_refs() {
    let (_root, workspace, forge, board) = fixture("ws-tui-tty-pr-column");
    let mut tui = PtySession::open_size_with_env(&workspace, 140, 36, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| board_matches(&tui, &board, true),
        "every row ends in `<ref> <glyph>` at 140 columns",
        GIT_WAIT,
    );

    for cols in [100, 120, 140] {
        tui.resize(cols, 36);
        // One sample must satisfy all of it: the frame of the width before
        // the resize cannot.
        tui.wait_pred(
            |_| {
                frame_fits_terminal(&tui)
                    && tree_edge(&tui).is_some_and(|edge| {
                        board_matches(&tui, &board, edge.glyph_col >= REF_MIN_COLS)
                    })
            },
            &format!(
                "{cols} columns: every row ends at the tree edge, refs on every row only when the tree is {REF_MIN_COLS}+ wide"
            ),
            GIT_WAIT,
        );
    }
}

/// A tree of 49 columns (128 terminal columns) drops the refs.
///
/// Seed and proof as `launch_at`: the grid confirms the inner width is 49,
/// and every row ends in the glyph alone at the tree edge. A rule that
/// lets refs in at 49 columns keeps them here and fails.
#[test]
fn pty_pr_tree_of_49_columns_drops_refs() {
    launch_at("ws-tui-tty-pr-edge-49", 128, REF_MIN_COLS - 1);
}

/// A tree of exactly 50 columns (130 terminal columns) paints the refs.
///
/// Seed and proof as `launch_at`: the grid confirms the inner width is 50,
/// and every row ends in `<ref> <glyph>` at the tree edge. A rule that
/// needs 51 columns drops the refs here and fails.
#[test]
fn pty_pr_tree_of_50_columns_paints_refs() {
    launch_at("ws-tui-tty-pr-edge-50", 130, REF_MIN_COLS);
}

/// The flat look (shipped defaults, no pane border) keeps one glyph column
/// and drops the refs on a narrow tree.
///
/// Seed: as the boxed test. Proof: at 140 columns every row ends in
/// `<ref> <glyph>` with the glyph in the tree's edge cell; at 100 columns
/// every row ends in the glyph alone, still in that cell. Reverting the
/// flat edge placement or the ref drop fails the matching wait.
#[test]
fn pty_pr_glyph_column_straight_in_the_flat_look() {
    let (_root, workspace, forge, board) = fixture("ws-tui-tty-pr-column-flat");
    let mut tui = PtySession::open_size_with_config(
        &workspace,
        140,
        36,
        &forge_env(&forge, &[]),
        UserConfig::Own,
    );
    tui.wait_pred(
        |_| board_matches(&tui, &board, true),
        "flat look, 140 columns: every row ends in `<ref> <glyph>` at the edge",
        GIT_WAIT,
    );
    tui.resize(100, 36);
    tui.wait_pred(
        |_| board_matches(&tui, &board, false),
        "flat look, 100 columns: glyphs only, at the edge",
        GIT_WAIT,
    );
}

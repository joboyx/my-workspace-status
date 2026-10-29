use std::fs;

use crate::common::fake_forge::FakeForge;
use crate::common::pr_scenarios::Badge;
use crate::harness::{PtySession, UserConfig};
use crate::pr_support::{
    badge_is, forge_env, seed_checkouts, state_board, tree_badge, tree_edge, Forge, PrCheckout,
};
use crate::support::GIT_WAIT;

/// Narrowest tree (inner columns) that paints refs: spec §8.1.
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

/// Every row of the board and the extras must show `want` as its badge: the
/// ref (when `refs`) and the ASCII glyph, flush with the tree edge.
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

/// All PR glyphs share one column, which is the tree's last content cell.
fn glyph_columns(tui: &PtySession, board: &[(PrCheckout, Badge)]) -> Vec<u16> {
    let mut cols: Vec<u16> = board
        .iter()
        .filter_map(|(checkout, _)| tree_badge(tui, &checkout.dir))
        .map(|badge| badge.col)
        .collect();
    cols.dedup();
    cols
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

/// The glyph column is straight at every tree width, and a tree under 50
/// columns drops the refs on every row (AC2).
///
/// Seed: fourteen checkouts (six states on each forge, plus two dirty rows
/// whose other marks differ), fake `gh` / `glab` answers per state.
///
/// Proof: at 140 columns each row ends in `<ref> <glyph>` and every glyph
/// is in one column, the last tree cell. The terminal then resizes through
/// widths on both sides of the 50-column rule (the inner tree width is read
/// off the cell grid): each time, the glyph column is still one column
/// and the refs are on every row when the tree is 50 columns or wider, and
/// on no row when narrower. Reverting the ref drop keeps refs on a narrow
/// tree and fails the narrow wait; reverting the end-of-run placement moves
/// the glyph column per row and fails the single-column check.
#[test]
fn pty_pr_glyph_column_straight_and_narrow_tree_drops_refs() {
    let (_root, workspace, forge, board) = fixture("ws-tui-tty-pr-column");
    let mut tui = PtySession::open_size_with_env(&workspace, 140, 36, &forge_env(&forge, &[]));
    tui.wait_pred(
        |_| board_matches(&tui, &board, true),
        "every row ends in `<ref> <glyph>` at 140 columns",
        GIT_WAIT,
    );
    let wide = glyph_columns(&tui, &board);
    assert_eq!(wide.len(), 1, "one glyph column at 140 columns: {wide:?}");

    for cols in [128, 129, 130, 100, 120, 132, 140] {
        tui.resize(cols, 36);
        // One sample must satisfy all of it: a frame that is half drawn
        // cannot.
        tui.wait_pred(
            |_| {
                tree_edge(&tui).is_some_and(|edge| {
                    board_matches(&tui, &board, edge.glyph_col >= REF_MIN_COLS)
                        && glyph_columns(&tui, &board) == [edge.glyph_col]
                })
            },
            &format!(
                "{cols} columns: one glyph column at the tree edge, refs on every row only when the tree is {REF_MIN_COLS}+ wide"
            ),
            GIT_WAIT,
        );
    }
}

/// The flat look (shipped defaults, no pane border) keeps one glyph column
/// and drops the refs on a narrow tree (AC2, second look of the parity row).
///
/// Seed: as the boxed test. Proof: at 140 columns every row ends in
/// `<ref> <glyph>` in one column; at 100 columns every row ends in the glyph
/// alone, still in one column. Reverting the flat edge placement or the ref
/// drop fails the matching wait.
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
        |_| board_matches(&tui, &board, true) && glyph_columns(&tui, &board).len() == 1,
        "flat look, 140 columns: every row ends in `<ref> <glyph>`, one glyph column",
        GIT_WAIT,
    );
    tui.resize(100, 36);
    tui.wait_pred(
        |_| board_matches(&tui, &board, false) && glyph_columns(&tui, &board).len() == 1,
        "flat look, 100 columns: glyphs only, one glyph column",
        GIT_WAIT,
    );
}

use crate::harness::PtySession;
use crate::pr_support::{
    badge_glyph_paint, badge_is, forge_env, glyph_colour, state_board, tree_badge,
};
use crate::support::GIT_WAIT;

/// In Nerd mode the six states paint their shapes `✕ ◉ ◐ ◌ ○ ✓`, one cell
/// wide, flush with the tree edge, on both forges (`docs/tui-rust.md`,
/// **Pull request badge**, Nerd glyph set).
///
/// Seed: the twelve-checkout board of the ASCII test, launched with
/// `WS_STATUS_GLYPHS=nerd`.
///
/// Proof: each row ends in `<ref> <shape>`; the cell right of the glyph is
/// the pane border, so no shape spills into a second column; the shape
/// paints bold in the same colour as its ASCII letter does. Reverting the
/// Nerd shapes (the old `\u{f407}` family or the ASCII letters) fails the
/// glyph match.
#[test]
fn pty_pr_badge_nerd_glyphs_on_both_forges() {
    let (_root, workspace, forge, board) = state_board("ws-tui-tty-pr-nerd");
    let tui = PtySession::open_size_with_env(
        &workspace,
        140,
        32,
        &forge_env(&forge, &[("WS_STATUS_GLYPHS", "nerd")]),
    );

    for (checkout, badge) in &board {
        let ref_text = checkout.ref_text();
        tui.wait_pred(
            |_| badge_is(&tui, &checkout.dir, Some(&ref_text), badge.nerd()),
            &format!("{} ends in `{ref_text} {}`", checkout.dir, badge.nerd()),
            GIT_WAIT,
        );
    }
    for (checkout, badge) in &board {
        let cells = tree_badge(&tui, &checkout.dir).expect("tree row");
        assert_eq!(
            tui.grid_cell_char(cells.row, cells.col + 1),
            Some('│'),
            "{}: `{}` fits one cell, the pane border follows\n{}",
            checkout.dir,
            badge.nerd(),
            tui.screen()
        );
        assert_eq!(
            badge_glyph_paint(&tui, &checkout.dir),
            Some((glyph_colour(*badge), true)),
            "{} glyph paints bold in the colour of its state",
            checkout.dir
        );
    }
}

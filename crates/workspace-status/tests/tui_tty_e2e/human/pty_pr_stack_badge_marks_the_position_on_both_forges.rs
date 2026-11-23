use crate::harness::PtySession;
use crate::pr_support::{
    badge_text_is, forge_env, seed_stacks, tree_badge, tree_text_paint, MUTED,
};
use crate::support::GIT_WAIT;

/// The tree badge of a PR in a stacked chain paints its stack position
/// between the ref and the glyph, on both forges and in both glyph sets
/// (`docs/tui-rust.md`, **Pull request badge**).
///
/// Seed: [`seed_stacks`]. GitHub chain `#6` → `#7` → `#8` → `#9` with `#7`
/// (`app-auth`, CI failed) and `#8` (`app-login`, in review) checked out;
/// GitLab chain `!4` → `!5` → `!6` with `!5` (`api-limit`, CI running)
/// checked out. The other members exist only on the fake forge, so only
/// the stack walk finds them.
///
/// Proof, ASCII: the rows end in `#7 2/4 x`, `#8 3/4 P`, and `!5 2/3 r`;
/// the position paints muted and the glyph stays the last tree cell (the
/// pane border follows it). Nerd: `#7 ⧗2/4 ✕`, `#8 ⧗3/4 ○`, `!5 ⧗2/3 ◐`.
/// A walk that skips the members no checkout has gives `1/2` / `2/2` (or no
/// position on `!5`); a position after the glyph breaks the glyph column.
#[test]
fn pty_pr_stack_badge_marks_the_position_on_both_forges() {
    let (_root, workspace, forge) = seed_stacks("ws-tui-tty-pr-stack-badge");
    let ascii = [
        ("app-auth", "#7 2/4 x", "2/4"),
        ("app-login", "#8 3/4 P", "3/4"),
        ("api-limit", "!5 2/3 r", "2/3"),
    ];
    {
        let tui = PtySession::open_size_with_env(&workspace, 160, 40, &forge_env(&forge, &[]));
        for (dir, badge, _) in ascii {
            tui.wait_pred(
                |_| badge_text_is(&tui, dir, badge),
                &format!("{dir} ends in `{badge}`"),
                GIT_WAIT,
            );
        }
        for (dir, _, position) in ascii {
            let cells = tree_badge(&tui, dir).expect("tree row");
            assert_eq!(
                tui.grid_cell_char(cells.row, cells.col + 1),
                Some('│'),
                "{dir}: the glyph is the last tree cell\n{}",
                tui.screen()
            );
            assert_eq!(
                tree_text_paint(&tui, dir, position).map(|(fg, _)| fg),
                Some(MUTED),
                "{dir}: the stack position `{position}` paints muted"
            );
        }
    }

    // A closed session removes its temp root, so the Nerd run seeds anew.
    let (_root, workspace, forge) = seed_stacks("ws-tui-tty-pr-stack-badge-nerd");
    let tui = PtySession::open_size_with_env(
        &workspace,
        160,
        40,
        &forge_env(&forge, &[("WS_STATUS_GLYPHS", "nerd")]),
    );
    for (dir, badge) in [
        ("app-auth", "#7 \u{29D7}2/4 \u{2715}"),
        ("app-login", "#8 \u{29D7}3/4 \u{25CB}"),
        ("api-limit", "!5 \u{29D7}2/3 \u{25D0}"),
    ] {
        tui.wait_pred(
            |_| badge_text_is(&tui, dir, badge),
            &format!("Nerd: {dir} ends in `{badge}`"),
            GIT_WAIT,
        );
    }
}

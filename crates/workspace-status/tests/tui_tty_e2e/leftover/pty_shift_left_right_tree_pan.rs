use crate::common::hscroll::TREE_HSCROLL_TAIL;
use crate::harness::{
    self, assert_tree_clipped_long_path, left_tree, status_has_tree_hscroll_tail,
    tree_cursor_bar_on_row, tree_is_panned_to_tail, tree_row_containing, PtySession,
};
use crate::seed::{daily_workspace, seed_long_path_file};
use crate::support::{
    launch_panes_left_tree_right_diff, no_updates_group_folded, no_wrong_overlays, status_row,
    tree_cursor_on, tree_dir_expanded, tree_has, GIT_WAIT, SETTLE_MS, TREE_LABEL_COL, WAIT,
};

/// Kitty CSI-u Left. Crossterm 0.28 `event::read` yields Char U+E006.
const KITTY_LEFT: u32 = 57350;
/// Kitty CSI-u Right. Crossterm 0.28 `event::read` yields Char U+E007.
const KITTY_RIGHT: u32 = 57351;

/// Same gap as `pty_key_repeat_j_reaches_no_updates`. A burst of the same
/// nav key is dropped by `discard_held_nav_backlog` after the first press.
const NAV_KEY_GAP_MS: u64 = 50;

fn csi_u_arrow(tui: &mut PtySession, codepoint: u32, modifier: u8) {
    tui.csi_u(codepoint, modifier, 1);
    tui.csi_u(codepoint, modifier, 3);
}

fn csi_u_shift_arrow_pan(tui: &mut PtySession, codepoint: u32) {
    csi_u_arrow(tui, codepoint, 2);
    tui.wait_ms(NAV_KEY_GAP_MS);
}

fn help_lists_shift_arrows_tree_pan(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("h l")
        && compact.contains("fold")
        && compact.contains("pan lists/diff")
        && compact.contains("Shift+")
        && compact.contains("tree")
}

/// Tree focused on dirty README, long path clipped, No updates folded.
fn tree_clipped_readme_focus(screen: &str, readme_row: u16) -> bool {
    harness::clipped_long_path_row(screen).is_some()
        && tree_cursor_bar_on_row(screen, readme_row)
        && launch_panes_left_tree_right_diff(screen)
        && tree_has(screen, "README.md")
        && tree_dir_expanded(screen, "app")
        && no_updates_group_folded(screen)
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !screen.contains("Mouse on")
        && !status_has_tree_hscroll_tail(screen)
        && no_wrong_overlays(screen)
        && status_row(screen).contains("focus right")
}

/// Documented Shift+←/→ tree pan: `TAIL99` on the tree row, prefix gone.
fn documented_shift_arrows_panned(screen: &str, readme_row: u16) -> bool {
    tree_is_panned_to_tail(screen)
        && tree_row_containing(screen, TREE_HSCROLL_TAIL).is_some()
        && tree_cursor_bar_on_row(screen, readme_row)
        && launch_panes_left_tree_right_diff(screen)
        && tree_dir_expanded(screen, "app")
        && no_updates_group_folded(screen)
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !status_has_tree_hscroll_tail(screen)
        && no_wrong_overlays(screen)
        && status_row(screen).contains("focus right")
}

/// CSI-u Shift+←/→ pan a clipped tree path. Unshifted arrows still fold.
///
/// Docs + MOVE: `h l` fold · pan lists/diff · Shift+←→ tree. Live PTY
/// writes kitty CSI-u Shift+Right (`CSI 57351 ; 2 : 1 u`) into `event::read`.
/// Shared oracle: clipped `very-long` prefix on the **tree row**, then
/// `TAIL99` after pan, prefix gone. Unshifted CSI-u Right must not pan
/// (file fold is a no-op). Cursor stays on README. App stays open. No
/// updates stays folded. Tree focus stays. A no-op, a fold, a focus
/// steal, or chrome-only (`SEARCH` / Mouse toast) is red.
#[test]
fn pty_shift_left_right_tree_pan() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("README.md")
                && screen.contains("? help")
                && !screen.contains("SEARCH")
                && !screen.contains("MOVE")
                && !screen.contains("Mouse off")
                && !screen.contains("Mouse on")
        },
        "launch paints the tree; SEARCH, help overlay, and mouse toast are closed",
        WAIT,
    );
    let _ = tui.wait_clipped_long_path_row(WAIT);

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && help_lists_shift_arrows_tree_pan(screen)
                && screen.contains("h   l")
        },
        "help MOVE lists h/l fold and Shift+←→ tree pan",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("pan lists/diff")
                && screen.contains("README.md")
                && screen.contains("? help")
        },
        "Esc closes help so Shift+arrows pan the tree, not help",
        WAIT,
    );

    // Launch focuses the untracked long path (NEW). Click README so the
    // right pane is a short diff and tree-focused Shift+arrows pan the
    // tree. Click is setup, not the click-to-select claim.
    let readme_hit = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_hit);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && screen.contains("+dirty")
                && screen.contains("app/README.md")
                && !screen.contains("SEARCH")
                && !screen.contains("NEW")
        },
        "click loads a short README diff (not the long-path NEW file)",
        GIT_WAIT,
    );
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row before Shift+arrows:\n{}", tui.screen()));
    let _ = tui.wait_clipped_long_path_row(WAIT);
    assert_tree_clipped_long_path(&tui.screen());
    assert!(
        tree_clipped_readme_focus(&tui.screen(), readme_row),
        "clipped long path, README cursor, app open, No updates folded:\n{}",
        tui.screen()
    );

    for _ in 0..8 {
        csi_u_arrow(&mut tui, KITTY_RIGHT, 1);
        csi_u_arrow(&mut tui, KITTY_LEFT, 1);
    }
    tui.wait_ms(SETTLE_MS);
    assert!(
        tree_clipped_readme_focus(&tui.screen(), readme_row),
        "unshifted CSI-u Left/Right must not pan (file fold is a no-op):\n{}",
        tui.screen()
    );

    for _ in 0..40 {
        csi_u_shift_arrow_pan(&mut tui, KITTY_RIGHT);
    }
    tui.wait_pred(
        |screen| documented_shift_arrows_panned(screen, readme_row),
        "CSI-u Shift+Right shows TAIL99, drops very-long, keeps README cursor and short diff",
        WAIT,
    );
    crate::common::hscroll::assert_panned_to_tail(&left_tree(&tui.screen()));

    for _ in 0..40 {
        csi_u_shift_arrow_pan(&mut tui, KITTY_LEFT);
    }
    tui.wait_pred(
        |screen| tree_clipped_readme_focus(screen, readme_row),
        "CSI-u Shift+Left restores the clipped prefix without folding or stealing focus",
        WAIT,
    );
    assert_tree_clipped_long_path(&tui.screen());
}

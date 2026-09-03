use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{left_tree, PtySession};
use crate::seed::{daily_workspace, seed_long_diff_file};
use crate::support::{
    crumb_row, launch_breadcrumb_workspace_only, no_updates_group_folded, no_wrong_overlays,
    panes_tree_focused_diff_unfocused, panes_tree_unfocused_diff_focused, right_pane, status_row,
    title_has_files, tree_cursor_on, tree_dir_expanded, tree_has, SETTLE_MS, WAIT,
};

const FILE: &str = "unique-diffline.rs";
/// Repeats after the first CSI-u press. Live hunt: one press is `· pan 1`.
const PAN_REPEATS: usize = 40;
/// Gap so the input thread does not drain the held-nav backlog as one move.
const REPEAT_GAP_MS: u64 = 50;

fn status_has_diff_tail(screen: &str) -> bool {
    status_row(screen).contains(DIFF_HSCROLL_TAIL)
}

fn help_lists_hl_pan(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    screen.contains("MOVE")
        && compact.contains("h l")
        && compact.contains("fold · pan lists/diff")
        && compact.contains("toggle fold")
}

fn tree_stays_on_long_file(screen: &str) -> bool {
    tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, FILE)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
}

fn clipped_new_diff(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    left.contains(FILE)
        && !left.contains(DIFF_HSCROLL_TAIL)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains("nnnn")
        && right.contains("line 0")
        && right.contains("inline (too narrow)")
        && !right.contains("inline (too narrow) ·")
        && !right.contains(DIFF_HSCROLL_TAIL)
        && !right.contains('█')
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !screen.contains("WIP on graph")
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
}

fn panned_new_diff(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    left.contains(FILE)
        && !left.contains(DIFF_HSCROLL_TAIL)
        && right.contains(FILE)
        && right.contains("NEW")
        && right.contains(DIFF_HSCROLL_TAIL)
        && right.contains("inline (too narrow) ·")
        && right.contains('█')
        && !right.contains("app/README.md")
        && !right.contains("UNSTAGED")
        && !screen.contains("WIP on graph")
        && !title_has_files(screen)
        && !status_has_diff_tail(screen)
}

fn idle_chrome_left(screen: &str) -> bool {
    let status = status_row(screen);
    launch_breadcrumb_workspace_only(screen)
        && status.contains("focus right")
        && !status.contains("drill")
        && !status.contains("Esc")
        && no_wrong_overlays(screen)
}

fn idle_chrome_right(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    crumb.trim() == "[workspace]"
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !status.contains("focus right")
        && no_wrong_overlays(screen)
}

/// Long NEW file-diff is loaded and clipped. Tree is still focused.
fn long_diff_clipped_tree_focus(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen)
        && tree_stays_on_long_file(screen)
        && clipped_new_diff(screen)
        && idle_chrome_left(screen)
}

/// Tab focused the overflowing file-diff. The long line is still clipped.
fn long_diff_clipped_diff_focus(screen: &str) -> bool {
    panes_tree_unfocused_diff_focused(screen)
        && tree_stays_on_long_file(screen)
        && clipped_new_diff(screen)
        && idle_chrome_right(screen)
}

/// One CSI-u `l` press: content moved one column, tail still clipped.
fn documented_hl_panned_one_col(screen: &str) -> bool {
    let right = right_pane(screen);
    panes_tree_unfocused_diff_focused(screen)
        && tree_stays_on_long_file(screen)
        && idle_chrome_right(screen)
        && right.contains("inline (too narrow) · pan 1")
        && right.contains('█')
        && right.contains("ine 0")
        && !right.contains("line 0")
        && !right.contains(DIFF_HSCROLL_TAIL)
        && !status_has_diff_tail(screen)
}

/// Held `l` revealed the overflowing tail. Content moved, not chrome only.
fn documented_hl_panned_file_diff(screen: &str) -> bool {
    panes_tree_unfocused_diff_focused(screen)
        && tree_stays_on_long_file(screen)
        && panned_new_diff(screen)
        && idle_chrome_right(screen)
}

/// CSI-u press, spaced Repeat, release. The input thread drains a held-nav
/// burst after each move, so Repeats must not be written as one flush.
fn csi_u_hold_letter(tui: &mut PtySession, letter: char, repeats: usize) {
    tui.letter_press(letter);
    tui.wait_ms(REPEAT_GAP_MS);
    for _ in 0..repeats {
        tui.letter_repeat(letter);
        tui.wait_ms(REPEAT_GAP_MS);
    }
    tui.csi_u(u32::from(letter.to_ascii_lowercase()), 1, 3);
}

/// CSI-u `h` / `l` pan a focused overflowing file-diff.
///
/// Docs + help MOVE: `h l` is fold on the tree and pan on lists/diff.
/// Graph, commit-file list, or a file diff focused: `h` / `l` pan. Header
/// shows `· pan N`. A 1-row horizontal bar paints after the viewport
/// leaves the left edge. Tree-focused `h` / `l` still fold and must not
/// pan this diff.
///
/// Live PTY (default 140×32 so help paints `lists/diff`; the NEW line
/// still clips). `/unique-diffline` loads the file. CSI-u held `l` while
/// the tree is focused leaves the tail clipped. Tab focuses the diff.
/// One CSI-u `l` press (`CSI 108 ; 1 : 1 u`) pans one column (`· pan 1`,
/// `line 0` → `ine 0`, h-bar). Spaced Repeat (`: 2`) reveals
/// `UNIQUE_DIFF_TAIL`. CSI-u held `h` restores the clip. A raw `'l'`
/// byte is a different path. The input thread drops a held-nav burst,
/// so Repeats are spaced like `pty_key_repeat_j`. A no-op, a tree fold,
/// a chrome-only pan suffix, or a tree pan cannot pass.
#[test]
fn pty_h_l_pan_graph_or_file_diff() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace, FILE, DIFF_HSCROLL_TAIL);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(
        help_lists_hl_pan,
        "help MOVE lists h l as fold · pan lists/diff",
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
        "Esc closes help so h/l are fold/pan, not help keys",
        WAIT,
    );

    tui.search("unique-diffline");
    tui.wait_pred(
        long_diff_clipped_tree_focus,
        "search loads the clipped NEW file-diff; tree stays focused on the file",
        WAIT,
    );

    csi_u_hold_letter(&mut tui, 'l', 3);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        long_diff_clipped_tree_focus,
        "CSI-u held l on the tree-focused file must not pan the overflowing diff (fold, not pan)",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        long_diff_clipped_diff_focus,
        "Tab focuses the overflowing file-diff; the long line is still clipped",
        WAIT,
    );

    tui.letter_press('l');
    tui.wait_pred(
        documented_hl_panned_one_col,
        "one CSI-u l press pans one column (line 0 → ine 0, · pan 1, h-bar; tail still clipped)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_hl_panned_one_col,
        "one-column pan holds (not a delayed jump to the tail or a chrome-only flicker)",
        WAIT,
    );

    for _ in 0..PAN_REPEATS {
        tui.letter_repeat('l');
        tui.wait_ms(REPEAT_GAP_MS);
    }
    tui.csi_u(u32::from('l'), 1, 3);
    tui.wait_pred(
        documented_hl_panned_file_diff,
        "CSI-u l Repeat pans the overflowing file-diff to UNIQUE_DIFF_TAIL (tree stays)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_hl_panned_file_diff,
        "file-diff pan holds (not a flicker, tree fold, or chrome-only suffix)",
        WAIT,
    );

    csi_u_hold_letter(&mut tui, 'h', PAN_REPEATS);
    tui.wait_pred(
        long_diff_clipped_diff_focus,
        "CSI-u held h pans the focused file-diff back to the left edge (tail and pan chrome gone)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        long_diff_clipped_diff_focus,
        "restored clip holds (not a no-op h, a fold, or a leftover pan bar)",
        WAIT,
    );
}

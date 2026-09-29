use crate::seed::{regions_diff, two_hunk_regions_workspace, REGIONS_ALPHA, REGIONS_OMEGA};
use crate::support::{
    crumb_row, open_regions_first_hunk_highlight, revert_range_confirm, GIT_WAIT, SETTLE_MS, WAIT,
};

fn reverted_range_toast(screen: &str) -> bool {
    crumb_row(screen).contains("reverted range regions.txt")
        && !screen.contains("Discard highlighted lines")
        && !screen.contains("VISUAL")
}

/// `V` over the first hunk, `x`, then `y` discards only that hunk.
///
/// Two separable hunks in `regions.txt`. Whole-file `x` would restore
/// both ALPHA and OMEGA. Highlight + `x` must open the range confirm, and
/// `y` must drop ALPHA from the worktree, keep OMEGA, and leave the index
/// untouched. Git on disk is the oracle, not the toast.
#[test]
fn pty_v_x_y_reverts_one_hunk() {
    let (_root, workspace) = two_hunk_regions_workspace("ws-tui-tty-visual-revert");
    let repo = workspace.join("app");
    let mut tui = open_regions_first_hunk_highlight(&workspace);

    tui.letter_press('x');
    tui.wait_pred(
        revert_range_confirm,
        "x opens the range revert confirm and clears VISUAL",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert!(
        regions_diff(&repo, false).contains(REGIONS_ALPHA),
        "the confirm alone must not touch the worktree"
    );

    tui.letter_press('y');
    tui.wait_pred(
        reverted_range_toast,
        "y reverts the highlighted range (reverted range toast)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let unstaged = regions_diff(&repo, false);
    let cached = regions_diff(&repo, true);
    assert!(
        !unstaged.contains(REGIONS_ALPHA) && unstaged.contains(REGIONS_OMEGA),
        "worktree must drop ALPHA and keep OMEGA:\nunstaged={unstaged}"
    );
    assert!(
        cached.is_empty(),
        "range revert must not touch the index:\ncached={cached}"
    );
}

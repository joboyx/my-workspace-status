use crate::harness::PtySession;
use crate::seed::{regions_diff, two_hunk_regions_workspace, REGIONS_ALPHA, REGIONS_OMEGA};
use crate::support::{
    crumb_row, open_regions_first_hunk_highlight, regions_highlight_active, revert_range_confirm,
    GIT_WAIT, SETTLE_MS, VISUAL_KEY_GAP_MS, WAIT,
};

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

fn palette_highlight_rows(screen: &str) -> bool {
    palette_open(screen)
        && screen.contains("HIGHLIGHT")
        && screen.contains("Stage highlighted lines")
        && screen.contains("Revert highlighted lines")
        && screen.contains("Exit highlight")
}

/// Type a palette filter with a gap after nav letters (held-nav backlog).
fn type_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(VISUAL_KEY_GAP_MS);
        }
    }
}

/// The palette status line (not the footer, not a catalog row) shows `reason`.
fn palette_status_shows(screen: &str, reason: &str) -> bool {
    screen.lines().any(|line| {
        line.contains(reason) && !line.contains("Enter run") && !line.contains("Fetch remotes")
    })
}

/// The disabled Fetch remotes row paints `reason` at its right edge.
fn fetch_row_shows(screen: &str, reason: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains("Fetch remotes") && line.contains(reason))
}

/// `V` over the first hunk, `:` (commands), then "Revert highlighted lines".
///
/// `:` in highlight mode opens Quick Open on commands. The HIGHLIGHT rows
/// show first. A whole-file row shows why it waits
/// (`exit highlight first`) on the row and in the footer. Esc returns to
/// highlight with the same range. Enter on "Revert highlighted lines"
/// opens the range confirm (`pty_v_x_y_reverts_one_hunk` covers the git
/// result of that confirm).
#[test]
fn pty_v_colon_palette_revert_opens_range_confirm() {
    let (_root, workspace) = two_hunk_regions_workspace("ws-tui-tty-visual-palette");
    let mut tui = open_regions_first_hunk_highlight(&workspace);

    tui.key(':');
    tui.wait_pred(
        palette_highlight_rows,
        "`:` in highlight opens the commands with the HIGHLIGHT rows",
        WAIT,
    );
    type_filter(&mut tui, "fetch");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Fetch remotes")
                && screen.contains("exit highlight first")
        },
        "a whole-file row shows the exit-highlight reason in the footer",
        WAIT,
    );
    let screen = tui.screen();
    assert!(
        fetch_row_shows(&screen, "exit highlight first (Esc)")
            && !palette_status_shows(&screen, "exit highlight first (Esc)"),
        "before Enter the reason is on the row and in the footer, not the status line"
    );
    tui.enter();
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| palette_open(screen) && palette_status_shows(screen, "exit highlight first (Esc)"),
        "Enter on a disabled row keeps the palette open and puts the reason on its status line",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| !palette_open(screen) && regions_highlight_active(screen),
        "Esc closes the palette and keeps the highlight",
        WAIT,
    );

    tui.key(':');
    tui.wait_pred(palette_highlight_rows, "`:` reopens the commands", WAIT);
    type_filter(&mut tui, "revert highlighted");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("revert highlighted")
                && screen.contains("Revert highlighted lines")
                && !screen.contains("Stage highlighted lines")
        },
        "filter narrows to Revert highlighted lines",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        revert_range_confirm,
        "Enter opens the range revert confirm",
        WAIT,
    );
}

/// `V` over the first hunk, `:` (commands), then "Stage highlighted lines".
///
/// Enter stages only the highlighted hunk. Git on disk is the oracle: the
/// index gains ALPHA only, and OMEGA stays unstaged.
#[test]
fn pty_v_colon_palette_stages_highlighted_range() {
    let (_root, workspace) = two_hunk_regions_workspace("ws-tui-tty-visual-palette");
    let repo = workspace.join("app");
    let mut tui = open_regions_first_hunk_highlight(&workspace);

    tui.key(':');
    tui.wait_pred(
        palette_highlight_rows,
        "`:` in highlight opens the commands with the HIGHLIGHT rows",
        WAIT,
    );
    type_filter(&mut tui, "stage highlighted");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("❯ Stage highlighted lines")
                && !screen.contains("Revert highlighted lines")
        },
        "filter narrows to the stage rows with the cursor on Stage highlighted lines",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| crumb_row(screen).contains("staged range regions.txt") && !palette_open(screen),
        "Enter stages the highlighted range (staged range toast)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let cached = regions_diff(&repo, true);
    let unstaged = regions_diff(&repo, false);
    assert!(
        cached.contains(REGIONS_ALPHA) && !cached.contains(REGIONS_OMEGA),
        "index must gain ALPHA only:\ncached={cached}"
    );
    assert!(
        unstaged.contains(REGIONS_OMEGA) && !unstaged.contains(REGIONS_ALPHA),
        "OMEGA must stay unstaged:\nunstaged={unstaged}"
    );
}

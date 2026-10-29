use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{regions_diff, two_hunk_regions_workspace, REGIONS_ALPHA, REGIONS_OMEGA};
use crate::support::{
    ack_rows, after_ack, panes_tree_unfocused_diff_focused, regions_diff_focused,
    regions_first_paint, regions_highlight_active, GIT_WAIT, SETTLE_MS, VISUAL_KEY_GAP_MS, WAIT,
};

const ALPHA: &str = REGIONS_ALPHA;
const OMEGA: &str = REGIONS_OMEGA;

fn csi_u_letter(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

/// VISUAL off and the STAGED section painted. The wait checks the `staged range`
/// toast (`after_ack`), since it can expire before the diff reload lands.
fn range_staged(screen: &str) -> bool {
    !screen.contains("VISUAL") && screen.contains("STAGED")
}

fn unstaged_range_toast(screen: &str) -> bool {
    ack_rows(screen).contains("unstaged range") && !screen.contains("VISUAL")
}

fn repo_cached(workspace: &Path) -> String {
    regions_diff(&workspace.join("app"), true)
}

fn repo_unstaged(workspace: &Path) -> String {
    regions_diff(&workspace.join("app"), false)
}

fn highlight_first_hunk(tui: &mut PtySession) {
    tui.shift_letter('V');
    tui.wait_pred(
        regions_highlight_active,
        "CSI-u Shift+V paints VISUAL highlight",
        WAIT,
    );
    tui.wait_ms(VISUAL_KEY_GAP_MS);
    for _ in 0..6 {
        csi_u_letter(tui, 'j');
        tui.wait_ms(VISUAL_KEY_GAP_MS);
    }
    tui.wait_pred(
        |screen| regions_highlight_active(screen) && screen.contains(ALPHA),
        "j extends VISUAL over the first hunk (ALPHA)",
        WAIT,
    );
}

/// `V` then `s` stages only the highlighted hunk. `u` unstages that range.
///
/// Two separable hunks in `regions.txt`. Whole-file `git add` would put
/// both ALPHA and OMEGA in the index. Highlight + `s` must stage ALPHA
/// only. `u` on the staged hunk must drop ALPHA from the index and leave
/// OMEGA unstaged. Git on disk is the oracle, not the toast.
#[test]
fn pty_v_s_stages_one_hunk_u_unstages_that_range() {
    let (_root, workspace) = two_hunk_regions_workspace("ws-tui-tty-visual-stage");
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        regions_first_paint,
        "launch is the two-hunk dirty regions.txt file diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        regions_diff_focused,
        "Tab focuses the two-hunk regions.txt diff",
        WAIT,
    );

    highlight_first_hunk(&mut tui);
    tui.key('s');
    tui.wait_pred(
        after_ack("staged range", range_staged),
        "s stages the highlighted range (VISUAL off, staged range toast)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let cached = repo_cached(&workspace);
    let unstaged = repo_unstaged(&workspace);
    assert!(
        cached.contains(ALPHA) && !cached.contains(OMEGA),
        "index must contain ALPHA only, not a whole-file stage:\ncached={cached}\nunstaged={unstaged}"
    );
    assert!(
        unstaged.contains(OMEGA) && !unstaged.contains(ALPHA),
        "worktree diff must still show OMEGA unstaged:\ncached={cached}\nunstaged={unstaged}"
    );

    // One write: both `g` share one arrival, so the 400 ms window holds.
    tui.send_bytes(b"gg");
    tui.wait_pred(
        |screen| {
            panes_tree_unfocused_diff_focused(screen)
                && screen.contains("STAGED")
                && screen.contains(ALPHA)
                && !screen.contains("VISUAL")
        },
        "gg on the focused diff keeps STAGED ALPHA in view",
        WAIT,
    );

    tui.shift_letter('V');
    tui.wait_pred(
        |screen| regions_highlight_active(screen) && screen.contains("STAGED"),
        "V on the staged hunk paints VISUAL",
        WAIT,
    );
    tui.wait_ms(VISUAL_KEY_GAP_MS);
    for _ in 0..6 {
        csi_u_letter(&mut tui, 'j');
        tui.wait_ms(VISUAL_KEY_GAP_MS);
    }
    tui.key('u');
    tui.wait_pred(
        unstaged_range_toast,
        "u unstages the highlighted staged range",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    let cached = repo_cached(&workspace);
    let unstaged = repo_unstaged(&workspace);
    assert!(
        !cached.contains(ALPHA) && !cached.contains(OMEGA),
        "index must drop ALPHA after range unstage:\ncached={cached}"
    );
    assert!(
        unstaged.contains(ALPHA) && unstaged.contains(OMEGA),
        "both hunks must be unstaged after u:\nunstaged={unstaged}"
    );
}

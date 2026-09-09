use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, merger_graph_drilled_right, merger_graph_left_unfocused,
    GIT_WAIT, WAIT,
};

/// Enter on a graph-capable tree row focuses the graph. Esc pops back.
///
/// Docs + VIEW: Enter is `focus right / drill`; Esc is `back / unfocus`
/// and never quits. Launch is the README file diff. `j` moves onto
/// `merger` (the graph-capable row). Enter must paint the focused graph
/// for that repo, not keep the file diff and not push commit files.
/// Esc is CSI-u (`CSI 27 u`). It must restore the left tree and the
/// merger row. A no-op, a screen-delta-only check, `/` search, Tab, or
/// `o`/`O` cannot pass.
#[test]
fn pty_graph_drill_enter_esc() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff (graph drill has not run)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph (left focus, not yet Enter)",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        merger_graph_drilled_right,
        "Enter on merger focuses that graph (file-diff / files drill / no-op cannot pass)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        merger_graph_left_unfocused,
        "CSI-u Esc restores the left tree and the merger row",
        WAIT,
    );
}

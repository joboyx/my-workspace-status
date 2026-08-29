use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{apply_current_keep_graph_focus, graph_focus_cleared_full, GIT_WAIT};

/// Graph `o` / `O`: overlay, filter-apply `feature`, CSI-u Shift+O clears.
///
/// Docs + VIEW: `o` opens the local-branch overlay. Type to filter. Enter
/// with no marks applies the cursor row. While focus is on, the graph is
/// ancestors of those tips. `O` restores `--all`. Shift+O is CSI-u
/// (`CSI 111 ; 2 : 1 u` press, `: 3` release), not a raw `'O'` byte. A
/// no-op, an Enter files drill, `/` SEARCH, overlay Enter on `main`, or
/// another Shift binding (stash / theme / `G`) cannot pass.
/// Unmark-then-Enter stays on `pty_graph_focus_unmark_enter_clears`.
#[test]
fn pty_graph_branch_focus_overlay() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    apply_current_keep_graph_focus(&mut tui);

    tui.shift_letter('O');
    tui.wait_pred(
        graph_focus_cleared_full,
        "CSI-u Shift+O restores the full graph and drops the clear-focus hint",
        GIT_WAIT,
    );
}

#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_graph_stash_pop, documented_launch_first_paint,
    graph_focused_merger_before_stash_pop, merger_graph_left_unfocused, stash_row_ready_to_pop,
    GIT_WAIT, SETTLE_MS, WAIT,
};

/// Graph `p` pops the focused stash (apply + drop) on xfce / xdotool.
///
/// Docs: Help GIT `a p D` = focused stash apply/pop/drop. Keymap: graph
/// stash row `p` is `Action::GraphStashPop` (`git stash pop` of that
/// `stash@{n}`). Workspace / tree `p` is `Action::Pull`. Overlay `S`
/// then `a` / `D` is leftover `desktop_xfce_stash_create_apply_and_drop`.
/// Pop runs immediately. Drop still confirms.
///
/// After first paint, `j` lands on merger. Tab focuses the graph. `j`
/// selects `stash@{0}` (`WIP on graph`). `p` must restore `wip.txt` on
/// the merger tree, drop that stash from the graph, and toast `popped
/// stash@{0}`. A no-op, workspace pull, apply-only (stash stays),
/// drop-only (no `wip.txt`), overlay, SEARCH chip, or toast-only is red.
///
/// This path does not `/` search. Typing `merger` jumps the tree and
/// starts graph git; xfce can drop Enter while that worker runs, so a
/// `/merger` chip wait is not a pop claim. After Tab, `stash@{0}` is
/// already on the graph, so waiting for that string does not prove the
/// stash row is selected. `p` on the working-tree row is Pull.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_stash_graph_pop() {
    let (_root, workspace) = daily_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file diff (graph stash pop has not run)",
        WAIT,
    );

    tui.key("j");
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph (left focus, stash still listed)",
        GIT_WAIT,
    );

    tui.key("Tab");
    tui.wait_pred(
        graph_focused_merger_before_stash_pop,
        "Tab focuses the merger graph: working tree clean, stash@{0} listed, pop idle",
        GIT_WAIT,
    );

    tui.key("j");
    tui.wait_pred(
        stash_row_ready_to_pop,
        "j selects stash@{0}: graph cursor on WIP on graph; p pop stash hint",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        stash_row_ready_to_pop,
        "stash row holds (not a flicker); overlay closed; not pull",
        WAIT,
    );

    tui.key("p");
    tui.wait_pred(
        documented_graph_stash_pop,
        "graph p pops stash@{0}: wip.txt A on merger, stash gone, popped toast",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_graph_stash_pop,
        "popped paint holds (not a flicker, toast-only tick, or apply-only)",
        WAIT,
    );
}

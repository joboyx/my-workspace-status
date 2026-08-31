#![cfg(target_os = "linux")]

use crate::desktop::DesktopSession;
use crate::seed::daily_workspace;
use crate::support::{
    app_focused_stash_visible, app_graph_stash_row_focused, app_graph_working_tree_focused,
    documented_stash_applied, documented_stash_created, documented_stash_dropped,
    idle_dirty_readme_unstaged, no_updates_unfolded_after_stash, stash_create_overlay_open,
    stash_drop_confirm_open, GIT_WAIT, SETTLE_MS, WAIT,
};

/// xfce Shift+S creates a stash, graph `a` applies it, Shift+D drops it.
///
/// Docs: Help GIT `S` = stash menu, `a p D` = focused stash apply/pop/drop.
/// Configuration: tree dirty file is create-only (`s`). Graph stash row
/// `a` applies and keeps the entry. `D` asks `y`/`n`. After first paint
/// the cursor is already on dirty README. xfce Shift+S (keyboard
/// enhancement CSI-u) then overlay `s` must paint `Stash app` / `s create`
/// and stash that file (`Stashed 1 file`). `l`/`j`/Tab/`j` focuses app's
/// `stash@{0}`. Graph `a` restores README (`applied stash@{0}`) and keeps
/// the stash. Shift+D then `y` drops it (`dropped stash@{0}`); README
/// stays dirty. A no-op, SEARCH typing, `s` stage, toast-only, overlay-
/// open-only, pop, or merger `WIP on graph` is red.
///
/// This path does not `/` search. `wait_contains("/README")` matches the
/// diff header `app/README.md` while SEARCH is still typing, so Enter-arm
/// is never proven. xfce can drop Enter while right-pane git runs. Shift+S
/// while SEARCH is open types into the query and must not open the stash
/// overlay. Overlay `S` then `a` / `D` is this leftover. Graph `p` pop is
/// leftover `desktop_xfce_stash_graph_pop`.
#[cfg(target_os = "linux")]
#[test]
#[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
fn desktop_xfce_stash_create_apply_and_drop() {
    let (_root, workspace) = daily_workspace();
    let tui = DesktopSession::open(&workspace);
    tui.wait_pred(
        idle_dirty_readme_unstaged,
        "first paint: cursor on dirty README, unstaged, no stash overlay",
        WAIT,
    );

    tui.key("shift+s");
    tui.wait_pred(
        stash_create_overlay_open,
        "CSI-u Shift+S opens create-only stash overlay on dirty README",
        WAIT,
    );

    tui.key("s");
    tui.wait_pred(
        documented_stash_created,
        "overlay s stashes README: Stashed 1 file, file leaves tree, overlay closes",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_stash_created,
        "stashed paint holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key("l");
    tui.wait_pred(
        no_updates_unfolded_after_stash,
        "l unfolds No updates and shows app (still hidden README)",
        WAIT,
    );
    tui.key("j");
    tui.wait_pred(
        app_focused_stash_visible,
        "j focuses app under No updates; app graph shows stash@{0}",
        GIT_WAIT,
    );

    tui.key("Tab");
    tui.wait_pred(
        app_graph_working_tree_focused,
        "Tab focuses app graph on working tree; stash stays the next row",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        app_graph_working_tree_focused,
        "working-tree row holds (not a flicker); overlay closed",
        WAIT,
    );
    tui.key("j");
    tui.wait_pred(
        app_graph_stash_row_focused,
        "j focuses app stash@{0}; a apply / D drop hints; not merger",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        app_graph_stash_row_focused,
        "stash row holds (not a flicker); overlay closed; not pull",
        WAIT,
    );

    tui.key("a");
    tui.wait_pred(
        documented_stash_applied,
        "graph a applies: README returns dirty, stash@{0} stays, applied toast",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_stash_applied,
        "applied paint holds (not pop, not a flicker)",
        WAIT,
    );

    tui.key("shift+d");
    tui.wait_pred(
        stash_drop_confirm_open,
        "CSI-u Shift+D opens Drop stash@{0}? confirm; stash and README stay",
        WAIT,
    );
    tui.key("y");
    tui.wait_pred(
        documented_stash_dropped,
        "y drops stash@{0}; README stays dirty; graph stash row is gone",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_stash_dropped,
        "dropped paint holds (not pop, not a flicker)",
        WAIT,
    );
}

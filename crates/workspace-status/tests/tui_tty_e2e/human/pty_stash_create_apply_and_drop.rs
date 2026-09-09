use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    app_focused_stash_visible, app_graph_stash_row_focused, app_graph_working_tree_focused,
    documented_stash_applied, documented_stash_created, documented_stash_dropped,
    idle_dirty_readme_unstaged, no_updates_unfolded_after_stash, stash_create_overlay_open,
    stash_drop_confirm_open, GIT_WAIT, SETTLE_MS, WAIT,
};

/// CSI-u Shift+S creates a stash, graph `a` applies it, CSI-u Shift+D
/// drops it.
///
/// Docs: Help GIT `S` = stash menu, `a p D` = focused stash apply/pop/drop.
/// Configuration: tree dirty file is create-only (`s`). Graph stash row
/// `a` applies and keeps the entry. `D` asks `y`/`n`. Live PTY after
/// first paint (cursor already on dirty README) did that create with
/// CSI-u Shift+S then overlay `s`: toast `Stashed 1 file`, README left
/// the tree, overlay closed. `l`/`j`/Tab/`j` focused app's `stash@{0}`.
/// Graph `a` restored README (`applied stash@{0}`) and kept the stash.
/// CSI-u Shift+D then `y` dropped it (`dropped stash@{0}`); README stayed
/// dirty. Not overlay-open only. Not `s` stage (`S `). Not Space `*`.
/// Not graph `p` pop. Not merger `WIP on graph`. Not a raw `S`/`D` byte.
///
/// After first paint the cursor is already on the dirty README. Do not
/// `/` search. A no-op, toast-only, stage, pop, overlay-open-only, or
/// the merger stash is red.
#[test]
fn pty_stash_create_apply_and_drop() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unstaged,
        "first paint: cursor on dirty README, unstaged, no stash overlay",
        WAIT,
    );

    tui.shift_letter('S');
    tui.wait_pred(
        stash_create_overlay_open,
        "CSI-u Shift+S opens create-only stash overlay on dirty README",
        WAIT,
    );

    tui.key('s');
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

    tui.key('l');
    tui.wait_pred(
        no_updates_unfolded_after_stash,
        "l unfolds No updates and shows app (still hidden README)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        app_focused_stash_visible,
        "j focuses app under No updates; app graph shows stash@{0}",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        app_graph_working_tree_focused,
        "Tab focuses app graph on working tree; stash stays the next row",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        app_graph_stash_row_focused,
        "j focuses app stash@{0}; a apply / D drop hints; not merger",
        WAIT,
    );

    tui.key('a');
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

    tui.shift_letter('D');
    tui.wait_pred(
        stash_drop_confirm_open,
        "CSI-u Shift+D opens Drop stash@{0}? confirm; stash and README stay",
        WAIT,
    );
    tui.key('y');
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

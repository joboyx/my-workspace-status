use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, graph_cursor_on, graph_pane_focused, idle_dirty_readme_unstaged,
    pane_unstaged_readme, readme_unstaged_badge, status_row, tree_cursor_on, tree_has,
    tree_pane_focused, GIT_WAIT, SETTLE_MS, WAIT,
};

fn no_stash_wrong_ops(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("popped")
        && !crumb_row(screen).contains("staged")
}

fn app_stash_on_graph(screen: &str) -> bool {
    screen.contains("WIP on main")
        && screen.contains("stash@{0}")
        && screen.contains("seed app")
        && !screen.contains("WIP on graph")
}

fn has_graph_stash_hints(screen: &str) -> bool {
    let status = status_row(screen);
    status.contains("apply stash") && status.contains("drop stash") && status.contains("pop stash")
}

/// CSI-u Shift+S opened the create-only overlay on the dirty README.
fn stash_create_overlay_open(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && screen.contains("Stash app")
        && screen.contains("s create")
        && screen.contains("Esc cancel")
        && !screen.contains("a apply")
        && !screen.contains("p pop")
        && !screen.contains("d drop")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on main")
}

/// Overlay `s` created a path-scoped stash. README left the tree.
fn documented_stash_created(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_has(screen, "README.md")
        && !tree_has(screen, "app")
        && tree_has(screen, "No updates")
        && tree_has(screen, "0 changed")
        && crumb.contains("Stashed 1 file")
        && !screen.contains("Stash app")
        && !screen.contains("s create")
        && !screen.contains("UNSTAGED")
        && !screen.contains("WIP on main")
        && !crumb.contains("staged")
        && !crumb.contains("applied")
        && !crumb.contains("popped")
        && !crumb.contains("dropped")
        && no_stash_wrong_ops(screen)
}

/// `l` then `j`: app is focused under No updates. App graph shows the stash.
fn app_focused_stash_visible(screen: &str) -> bool {
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "lib")
        && tree_has(screen, "No updates")
        && app_stash_on_graph(screen)
        && screen.contains("working tree clean")
        && crumb_row(screen).contains("workspace › app")
        && !crumb_row(screen).contains("[app]")
        && tree_pane_focused(screen)
        && no_stash_wrong_ops(screen)
}

/// Tab focused the app graph on the working-tree row. Stash is the next row.
fn app_graph_working_tree_focused(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && graph_cursor_on(screen, "working tree clean")
        && !graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && crumb_row(screen).contains("[app]")
        && no_stash_wrong_ops(screen)
}

/// `j` landed on the app stash row. Graph `a` / `D` hints. Not merger.
fn app_graph_stash_row_focused(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && has_graph_stash_hints(screen)
        && crumb_row(screen).contains("[app]")
        && !screen.contains("Drop stash@{0}?")
        && no_stash_wrong_ops(screen)
}

/// Graph `a` applied. README is dirty again. Stash stays (not pop).
fn documented_stash_applied(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "1 changed")
        && readme_unstaged_badge(screen)
        && screen.contains("uncommitted changes")
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && has_graph_stash_hints(screen)
        && crumb.contains("applied stash@{0}")
        && !crumb.contains("popped")
        && !crumb.contains("dropped")
        && !crumb.contains("Stashed")
        && !screen.contains("Drop stash@{0}?")
        && no_stash_wrong_ops(screen)
}

/// CSI-u Shift+D opened drop confirm. Stash and dirty README stay until `y`.
fn stash_drop_confirm_open(screen: &str) -> bool {
    graph_pane_focused(screen)
        && screen.contains("Drop stash@{0}?")
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && screen.contains("uncommitted changes")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("popped")
}

/// Confirm `y` dropped the stash. Dirty README stays. Not pop.
fn documented_stash_dropped(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "1 changed")
        && readme_unstaged_badge(screen)
        && screen.contains("uncommitted changes")
        && graph_cursor_on(screen, "seed app")
        && !screen.contains("WIP on main")
        && !screen.contains("Drop stash@{0}?")
        && !status.contains("apply stash")
        && !status.contains("drop stash")
        && crumb.contains("dropped stash@{0}")
        && !crumb.contains("popped")
        && !crumb.contains("applied")
        && no_stash_wrong_ops(screen)
}

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
        |screen| {
            tree_cursor_on(screen, "No updates")
                && tree_has(screen, "app")
                && tree_has(screen, "lib")
                && !tree_has(screen, "README.md")
        },
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

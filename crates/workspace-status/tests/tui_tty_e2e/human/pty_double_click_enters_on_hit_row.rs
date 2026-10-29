use crate::harness::{tree_row_containing, PtySession};
use crate::seed::daily_workspace;
use crate::support::{
    prove_keys_handled, sgr_double_click, title_has_diff, title_has_files, title_has_graph,
    tree_has, GIT_WAIT, RIGHT_PANE_COL, TREE_DEPTH1_CHEVRON_COL, TREE_LABEL_COL, WAIT,
};

/// Column shift for a setup single click: the same row, one cell right of
/// where the double-click pair lands.
///
/// A double-click needs both presses on the same cell, so a click on the
/// next cell can never pair with the pair's first press, however close in
/// time they arrive. No sleep has to outlast the 400 ms window.
const SETUP_COL_SHIFT: u16 = 1;

/// 0-based screen row whose full line contains `needle` (right pane included).
fn screen_row_containing(screen: &str, needle: &str) -> Option<u16> {
    screen
        .lines()
        .enumerate()
        .find_map(|(i, line)| line.contains(needle).then_some(i as u16))
}

/// Double-click is Enter on the hit row (help: `Enter dblclick`).
///
/// Docs: left Enter focuses right on the same stack; right Enter drills
/// graph → commit files → commit diff; a chevron double-click still folds
/// and must not Enter. A single click only selects. Keyboard Enter on the
/// graph stash is the drill oracle the mouse pair must match.
#[test]
fn pty_double_click_enters_on_hit_row() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| tree_has(screen, "No updates") && !tree_has(screen, "lib"),
        "lib stays under the folded No-updates group",
        WAIT,
    );

    let nu_row = tree_row_containing(&tui.screen(), "No updates")
        .unwrap_or_else(|| panic!("No updates row:\n{}", tui.screen()));
    sgr_double_click(&mut tui, TREE_DEPTH1_CHEVRON_COL, nu_row);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "lib")
                && tree_has(screen, "No updates")
                && !screen.contains("[workspace]")
                && !title_has_files(screen)
        },
        "chevron double-click folds once (a second toggle hides lib; Enter would focus right)",
        WAIT,
    );

    let merger_row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL + SETUP_COL_SHIFT, merger_row);
    tui.wait_contains("WIP on graph", GIT_WAIT);
    prove_keys_handled(&mut tui);
    tui.wait_pred(
        |screen| {
            title_has_graph(screen)
                && screen.contains("WIP on graph")
                && !screen.contains("[merger]")
                && !title_has_files(screen)
        },
        "single-click merger selects the graph only (left focus)",
        WAIT,
    );

    sgr_double_click(&mut tui, TREE_LABEL_COL, merger_row);
    tui.wait_pred(
        |screen| {
            screen.contains("[merger]")
                && screen.contains("WIP on graph")
                && title_has_graph(screen)
                && !title_has_files(screen)
                && !screen.contains("wip.txt")
        },
        "tree double-click is Enter on the hit repo: focus right, do not drill",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| screen.contains("WIP on graph") && !screen.contains("[merger]"),
        "Esc unfocuses without popping the graph",
        WAIT,
    );

    tui.tab();
    tui.wait_contains("Working tree", WAIT);
    tui.key('j');
    tui.enter();
    tui.wait_pred(
        |screen| {
            title_has_files(screen) && screen.contains("wip.txt") && screen.contains("[stash@{0}]")
        },
        "keyboard Enter on the stash row drills to commit files (oracle)",
        GIT_WAIT,
    );
    tui.esc();
    tui.esc();
    tui.wait_pred(
        |screen| {
            screen.contains("WIP on graph")
                && !title_has_files(screen)
                && !screen.contains("[stash@{0}]")
        },
        "Esc Esc returns to the graph after the keyboard oracle",
        WAIT,
    );

    let graph_row = screen_row_containing(&tui.screen(), "WIP on graph")
        .unwrap_or_else(|| panic!("graph WIP row:\n{}", tui.screen()));
    tui.sgr_click(RIGHT_PANE_COL + SETUP_COL_SHIFT, graph_row);
    prove_keys_handled(&mut tui);
    tui.wait_pred(
        |screen| {
            screen.contains("WIP on graph")
                && !title_has_files(screen)
                && !screen.contains("wip.txt")
                && !screen.contains("[stash@{0}]")
        },
        "single-click the stash row selects it (no files drill)",
        WAIT,
    );

    sgr_double_click(&mut tui, RIGHT_PANE_COL, graph_row);
    tui.wait_pred(
        |screen| {
            title_has_files(screen)
                && screen.contains("wip.txt")
                && screen.contains("[stash@{0}]")
                && screen.contains("workspace › merger")
                && !screen.contains("[merger]")
        },
        "graph double-click matches keyboard Enter: drill to that stash's files",
        GIT_WAIT,
    );

    let file_row = screen_row_containing(&tui.screen(), "wip.txt")
        .unwrap_or_else(|| panic!("wip.txt row:\n{}", tui.screen()));
    tui.sgr_click(RIGHT_PANE_COL + SETUP_COL_SHIFT, file_row);
    prove_keys_handled(&mut tui);
    tui.wait_pred(
        |screen| {
            title_has_files(screen)
                && screen.contains("wip.txt")
                && !title_has_diff(screen)
                && !screen.contains("@@")
                && !screen.contains("+stash me")
        },
        "single-click the commit-file row stays on the files list",
        WAIT,
    );

    sgr_double_click(&mut tui, RIGHT_PANE_COL, file_row);
    tui.wait_pred(
        |screen| {
            title_has_diff(screen)
                && screen.contains("[wip.txt]")
                && screen.contains("@@")
                && screen.contains("+stash me")
        },
        "files double-click is Enter: open that file's commit diff",
        GIT_WAIT,
    );

    // A click on the next cell first, so the leaf pair below cannot pair
    // with the last press of the pair above.
    tui.sgr_click(RIGHT_PANE_COL + SETUP_COL_SHIFT, file_row);
    sgr_double_click(&mut tui, RIGHT_PANE_COL, file_row);
    prove_keys_handled(&mut tui);
    tui.wait_pred(
        |screen| {
            title_has_diff(screen)
                && screen.contains("[wip.txt]")
                && screen.contains("@@")
                && screen.contains("+stash me")
                && !title_has_graph(screen)
        },
        "double-click at the diff leaf is a no-op (still that diff)",
        WAIT,
    );
}

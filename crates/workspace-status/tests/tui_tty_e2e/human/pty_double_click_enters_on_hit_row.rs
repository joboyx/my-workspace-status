use crate::harness::{tree_row_containing, PtySession};
use crate::seed::daily_workspace;
use crate::support::{
    title_has_diff, title_has_files, title_has_graph, tree_has, GIT_WAIT, RIGHT_PANE_COL,
    SETTLE_MS, TREE_DEPTH1_CHEVRON_COL, TREE_LABEL_COL, WAIT,
};

/// Live `last_click` window is 400ms. Wait past it so the next pair is a
/// fresh double-click, not a continuation of a setup click.
const DOUBLE_CLICK_EXPIRE_MS: u64 = 500;

/// 0-based screen row whose full line contains `needle` (right pane included).
fn screen_row_containing(screen: &str, needle: &str) -> Option<u16> {
    screen
        .lines()
        .enumerate()
        .find_map(|(i, line)| line.contains(needle).then_some(i as u16))
}

/// Two xterm SGR left press+release reports on the same cell.
///
/// The live loop does not decode a double-click button. It treats two
/// left Downs at the same cell within 400ms as Enter (`nav_enter`).
fn sgr_double_click(tui: &mut PtySession, col: u16, row: u16) {
    tui.sgr_click(col, row);
    tui.sgr_click(col, row);
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

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let merger_row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, merger_row);
    tui.wait_contains("WIP on graph", GIT_WAIT);
    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
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
    tui.wait_ms(SETTLE_MS);
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

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let graph_row = screen_row_containing(&tui.screen(), "WIP on graph")
        .unwrap_or_else(|| panic!("graph WIP row:\n{}", tui.screen()));
    tui.sgr_click(RIGHT_PANE_COL, graph_row);
    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
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

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let file_row = screen_row_containing(&tui.screen(), "wip.txt")
        .unwrap_or_else(|| panic!("wip.txt row:\n{}", tui.screen()));
    tui.sgr_click(RIGHT_PANE_COL, file_row);
    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
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

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    sgr_double_click(&mut tui, RIGHT_PANE_COL, file_row);
    tui.wait_ms(SETTLE_MS);
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

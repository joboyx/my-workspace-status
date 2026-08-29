use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    page_file_body_visible, seed_tree_page_files, tree_cursor_on, tree_has, GIT_WAIT, WAIT,
};

fn help_lists_gg_g_top_bottom(screen: &str) -> bool {
    screen.contains("gg   G") && screen.contains("top / bottom of focused")
}

/// Help MOVE lists `gg G` as top/bottom of the focused pane. CSI-u `G`
/// jumps the tree. Two `g` bytes return to the root.
///
/// Docs + MOVE: `gg` (second `g` within ~400ms) is the start of the
/// focused list. Lone `g` expires with no move. `G` is the end. Live PTY
/// after first paint (cursor on dirty README) left last rows below the
/// fold. CSI-u `G` (`CSI 103 ; 2 : 1 u` press, `: 3` release) landed on
/// folded No updates. Two `g` bytes landed on `# workspace`. A raw `G`
/// byte is a different path. PageDown is one viewport. Extra dirty files
/// keep the last row off-screen at launch, so a no-op, PageDown, or a
/// jump onto `page-29` / merger cannot pass. Cursor bar, right pane, and
/// fold must all move.
#[test]
fn pty_gg_and_g_jump_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_page_files(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && tree_has(screen, "workspace")
                && !tree_cursor_on(screen, "workspace")
                && !tree_has(screen, "No updates")
                && !tree_has(screen, "page-29")
                && !page_file_body_visible(screen)
                && screen.contains("? help")
                && screen.contains("focus right")
                && !screen.contains("[workspace]")
        },
        "launch cursor is README; last tree rows stay below the fold",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && help_lists_gg_g_top_bottom(screen)
                && screen.contains("top / bottom of focused")
        },
        "help MOVE lists gg G as top/bottom of focused pane",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
                && screen.contains("UNSTAGED")
        },
        "Esc closes help so gg/G are tree jumps, not help keys",
        WAIT,
    );

    tui.key('g');
    tui.wait_ms(500);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "No updates")
                && screen.contains("UNSTAGED")
                && tree_has(screen, "workspace")
                && !tree_has(screen, "No updates")
        },
        "lone g expires with no move (gg would land on workspace; G would land on No updates)",
        WAIT,
    );

    tui.shift_letter('G');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "page-29")
                && !tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "workspace")
                && !tree_has(screen, "README.md")
                && !tree_has(screen, "workspace")
                && tree_has(screen, "page-29")
                && screen.contains("focus a repo for the graph")
                && !screen.contains("UNSTAGED")
                && !page_file_body_visible(screen)
                && !screen.contains("WIP on graph")
                && screen.contains("? help")
                && screen.contains("focus right")
                && !screen.contains("fetch")
                && !screen.contains("[workspace]")
                && !screen.contains("drill")
        },
        "CSI-u G jumps to the last tree row (a no-op stays on README; PgDn stays mid-list; merger would load its graph)",
        GIT_WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib") && tree_cursor_on(screen, "No updates"),
        "G then l opens No updates (G actually selected that row)",
        WAIT,
    );

    tui.gg();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "page-29")
                && tree_has(screen, "workspace")
                && !tree_has(screen, "No updates")
                && !tree_has(screen, "page-29")
                && screen.contains("focus a repo for the graph")
                && screen.contains("? help")
                && screen.contains("focus right")
                && screen.contains("fetch")
                && !screen.contains("[workspace]")
                && !screen.contains("UNSTAGED")
                && !page_file_body_visible(screen)
        },
        "gg jumps to the first tree row (a no-op stays on No updates; PgUp from the end lands on a page file)",
        GIT_WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        |screen| {
            !tree_has(screen, "app")
                && !tree_has(screen, "lib")
                && !tree_has(screen, "README.md")
                && tree_cursor_on(screen, "workspace")
        },
        "gg then h folds the workspace root (not only No updates)",
        WAIT,
    );
}

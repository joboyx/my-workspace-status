use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    page_file_body_visible, seed_tree_page_files, tree_cursor_on, tree_has, GIT_WAIT, WAIT,
};

fn help_lists_home_end_top_bottom(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains("Home")
            && line.contains("End")
            && line.contains("top / bottom")
            && !line.contains("page")
            && !line.contains("focused pane")
    })
}

/// Help MOVE lists Home/End as list top/bottom. CSI `1~` / `4~` jump the tree.
///
/// Documented result: first / last **tree** row, not pane chrome. `gg` / `G`
/// are the same edges via letters. PageDown is one viewport. Extra dirty
/// files keep the last row off-screen at launch, so a no-op, PageDown, or
/// a jump onto `page-29` / merger cannot pass. Cursor bar, right pane, and
/// fold must all move.
#[test]
fn pty_home_and_end_jump_workspace_tree() {
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
                && help_lists_home_end_top_bottom(screen)
                && screen.contains("gg   G")
                && screen.contains("top / bottom of focused")
                && screen.contains("PgUp   PgDn")
                && screen.contains("page focused pane")
        },
        "help MOVE lists Home/End as top/bottom, not page and not gg/G chrome",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("Home")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
                && screen.contains("UNSTAGED")
        },
        "Esc closes help so Home/End are tree jumps, not help keys",
        WAIT,
    );

    tui.end();
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
        "End jumps to the last tree row (a no-op stays on README; PgDn stays mid-list; merger would load its graph)",
        GIT_WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib") && tree_cursor_on(screen, "No updates"),
        "End then l opens No updates (End actually selected that row)",
        WAIT,
    );

    tui.home();
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
        "Home jumps to the first tree row (a no-op stays on No updates; PgUp from the end lands on a page file)",
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
        "Home then h folds the workspace root (not only No updates)",
        WAIT,
    );
}

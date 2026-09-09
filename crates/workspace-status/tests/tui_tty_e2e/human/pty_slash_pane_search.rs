use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// MOVE lists `/` as pane search, not help-local search.
fn help_lists_slash_search_focused_pane(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("MOVE")
        && compact.contains("/ search focused pane")
        && compact.contains("(Enter")
        && compact.contains("arms)")
        && compact.contains("/ search help")
}

/// Empty `/` prompt on the tree. Help overlay and an armed chip are other keys.
fn pane_search_prompt_on_tree(screen: &str) -> bool {
    screen.contains("SEARCH")
        && screen.contains("▏")
        && screen.contains("Enter arms query")
        && screen.contains("n/N after Enter")
        && !screen.contains("MOVE")
        && !screen.contains("HELP  /")
        && !screen.contains("/ search help")
        && !screen.contains("/merger")
        && !screen.contains("no match")
        && !screen.contains("drill")
        && !screen.contains("[workspace]")
        && !screen.contains("[merger]")
}

/// Typing `/merger` on the tree. Help `/` paints `HELP  /merger`.
fn typing_merger_tree_hit(screen: &str) -> bool {
    pane_search_prompt_on_tree(screen)
        && screen.contains("merger▏")
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "workspace")
        && screen.contains("workspace › merger")
        && !screen.contains("[merger]")
        && screen.contains("WIP on graph")
        && screen.contains("Working tree clean")
        && !screen.contains("+dirty")
        && !screen.contains("UNSTAGED")
        && !screen.contains("app/README.md")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "No updates")
        && !tree_has(screen, "lib")
        && !screen.contains("notes")
}

/// Enter arms `/merger` on the tree. SEARCH is gone. Stay left.
fn armed_merger_search_left(screen: &str) -> bool {
    screen.contains("/merger")
        && screen.contains("? help")
        && screen.contains("focus right")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && !screen.contains("MOVE")
        && !screen.contains("HELP  /")
        && !screen.contains("drill")
        && !screen.contains("[merger]")
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && screen.contains("workspace › merger")
        && screen.contains("WIP on graph")
        && !screen.contains("+dirty")
        && !screen.contains("UNSTAGED")
        && tree_has(screen, "README.md")
        && !tree_has(screen, "lib")
}

/// Help `/`, then live `/` + query on the focused tree.
///
/// Docs + MOVE: search the focused pane by substring (rows stay visible).
/// `/` paints SEARCH on that pane. Typing jumps the cursor. Enter arms
/// `/query`. Help `/` is a different overlay (`HELP  /…`). A no-op, help
/// search, the launch README row, Tab (`[merger]` / `drill`), a filter
/// that hides README, or a paint-changed-only assert cannot pass.
#[test]
fn pty_slash_pane_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "README.md")
                && tree_has(screen, "merger")
                && tree_has(screen, "No updates")
                && !tree_has(screen, "lib")
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "merger")
                && screen.contains("+dirty")
                && screen.contains("UNSTAGED")
                && !screen.contains("WIP on graph")
                && !screen.contains("SEARCH")
                && !screen.contains("workspace › merger")
        },
        "launch focuses README; merger is visible; graph subject is not loaded",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        help_lists_slash_search_focused_pane,
        "help MOVE lists / search focused pane (Enter arms)",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("/ search help")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("focus right")
        },
        "Esc closes help so / is pane search, not help search",
        WAIT,
    );

    tui.key('/');
    tui.wait_pred(
        |screen| {
            pane_search_prompt_on_tree(screen)
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "merger")
                && !screen.contains("workspace › merger")
                && screen.contains("+dirty")
        },
        "/ arms SEARCH on the tree; empty query must not jump or open help",
        WAIT,
    );

    tui.keys("merger");
    tui.wait_pred(
        typing_merger_tree_hit,
        "/merger jumps to merger (a no-op stays on README; help search is HELP  /merger)",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        armed_merger_search_left,
        "Enter arms /merger on the tree; SEARCH closes; stay left on merger",
        GIT_WAIT,
    );
}

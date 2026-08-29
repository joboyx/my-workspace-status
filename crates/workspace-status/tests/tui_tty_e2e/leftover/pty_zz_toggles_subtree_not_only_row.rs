use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has, SETTLE_MS, WAIT,
};

fn app_children_hidden(screen: &str) -> bool {
    tree_has(screen, "app")
        && tree_dir_collapsed(screen, "app")
        && !tree_has(screen, "zz-leaf.rs")
        && !tree_has(screen, "README.md")
        && !tree_has(screen, "src")
}

fn app_open_src_folded(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_has(screen, "src")
        && tree_dir_collapsed(screen, "src")
        && tree_dir_expanded(screen, "app")
        && !tree_has(screen, "zz-leaf.rs")
}

fn app_subtree_expanded(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_has(screen, "src")
        && tree_has(screen, "zz-leaf.rs")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "src")
}

/// `zz` subtree-folds the focused repo. Nested leaf stays hidden after `l`.
///
/// First `z` toggles this row immediately and paints `z…`. Second `z`
/// within 400ms is `toggleSubtree` (no extra row toggle): descendants
/// follow the focused row. A no-op, a row-only fold, or a chord that
/// unfolds the parent again cannot pass. `z` / fold is a different key.
#[test]
fn pty_zz_toggles_subtree_not_only_row() {
    let (_root, workspace) = daily_workspace();
    fs::create_dir_all(workspace.join("app").join("src")).unwrap();
    fs::write(
        workspace.join("app").join("src").join("zz-leaf.rs"),
        "fn zz_leaf() {}\n",
    )
    .unwrap();

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| app_subtree_expanded(screen) && tree_cursor_on(screen, "zz-leaf.rs"),
        "nested leaf is visible and focused at launch",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && screen.contains("toggle fold")
                && screen.contains("toggle subtree")
                && screen.contains("zz")
        },
        "help MOVE lists zz toggle subtree distinct from z toggle fold",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("MOVE") && app_subtree_expanded(screen),
        "Esc closes help so z is not swallowed",
        WAIT,
    );

    tui.key('k');
    tui.wait_ms(SETTLE_MS);
    tui.key('k');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "src")
                && app_subtree_expanded(screen)
                && screen.contains("? help")
        },
        "k k from the leaf focuses the expanded app repo",
        WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        |screen| {
            app_children_hidden(screen)
                && tree_cursor_on(screen, "app")
                && screen.contains("z…")
                && !screen.contains("? help")
        },
        "first z folds only this row and arms the 400ms zz window",
        WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        |screen| {
            app_children_hidden(screen)
                && tree_cursor_on(screen, "app")
                && screen.contains("? help")
                && !screen.contains("z…")
        },
        "second z within 400ms keeps the repo folded (a subtree unfold is a no-op)",
        WAIT,
    );

    tui.tab();
    tui.wait_contains("workspace › [app]", WAIT);
    tui.zz();
    tui.wait_ms(SETTLE_MS);
    tui.tab();
    tui.wait_pred(
        |screen| {
            app_children_hidden(screen)
                && tree_cursor_on(screen, "app")
                && screen.contains("workspace › app")
                && !screen.contains("[app]")
        },
        "zz on the graph is a no-op; the hidden workspace tree stays folded",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| app_open_src_folded(screen) && tree_cursor_on(screen, "app"),
        "l opens the repo with src still folded (a row-only fold would show zz-leaf.rs)",
        WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        |screen| app_children_hidden(screen) && screen.contains("z…"),
        "late-z setup: first z folds the repo again",
        WAIT,
    );
    tui.wait_ms(500);
    tui.key('z');
    tui.wait_pred(
        |screen| app_open_src_folded(screen) && tree_cursor_on(screen, "app"),
        "a late second z is a new row toggle, not subtree (src stays folded)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "src") && tree_dir_collapsed(screen, "src"),
        "j from app lands on the folded src dir",
        WAIT,
    );
    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "zz-leaf.rs") && tree_cursor_on(screen, "src"),
        "l on src reveals zz-leaf.rs (src was folded, not missing)",
        WAIT,
    );
    tui.key('h');
    tui.wait_pred(
        |screen| app_open_src_folded(screen) && tree_cursor_on(screen, "src"),
        "h folds src again before the expand chord",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && app_open_src_folded(screen),
        "k returns to app with src still folded",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        |screen| app_children_hidden(screen) && tree_cursor_on(screen, "app"),
        "h folds the repo so zz can expand the subtree",
        WAIT,
    );
    tui.key('z');
    tui.wait_pred(
        |screen| {
            app_open_src_folded(screen) && tree_cursor_on(screen, "app") && screen.contains("z…")
        },
        "first z of expand zz opens the repo; src stays folded",
        WAIT,
    );
    tui.key('z');
    tui.wait_pred(
        |screen| app_subtree_expanded(screen) && tree_cursor_on(screen, "app"),
        "second z opens foldable descendants (zz-leaf.rs visible without l on src)",
        WAIT,
    );
}

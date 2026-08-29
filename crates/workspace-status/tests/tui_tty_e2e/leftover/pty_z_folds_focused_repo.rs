use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, pane_unstaged_readme, readme_unstaged_badge,
    status_row, tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has, GIT_WAIT,
    SETTLE_MS, WAIT,
};

/// Daily seed: dirty README stays, `app` stays expanded, No updates stays folded.
fn z_file_row_leaves_tree_open(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && tree_dir_expanded(screen, "app")
        && !tree_dir_collapsed(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && tree_dir_collapsed(screen, "No updates")
        && !tree_has(screen, "lib")
        && crumb_row(screen).trim() == "workspace"
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
}

/// Tree-focused expanded `app` repo. README still visible. Graph loaded.
fn app_repo_expanded_before_z(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "merger")
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && readme_unstaged_badge(screen)
        && tree_dir_expanded(screen, "app")
        && !tree_dir_collapsed(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && tree_dir_collapsed(screen, "No updates")
        && !tree_has(screen, "lib")
        && screen.contains("uncommitted changes")
        && screen.contains("seed app")
        && !screen.contains("UNSTAGED")
        && crumb.contains("workspace › app")
        && !crumb.contains("[app]")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
}

/// `z` folded the focused `app` repo. Cursor stays. Graph stays. Not `zz`.
fn documented_z_folds_focused_repo(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "merger")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && !tree_has(screen, "README.md")
        && tree_dir_collapsed(screen, "app")
        && !tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && tree_dir_collapsed(screen, "No updates")
        && !tree_has(screen, "lib")
        && screen.contains("uncommitted changes")
        && screen.contains("seed app")
        && !screen.contains("UNSTAGED")
        && crumb.contains("workspace › app")
        && !crumb.contains("[app]")
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
}

/// Graph-focused `z` must not unfold the hidden workspace tree.
fn graph_z_leaves_repo_folded(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_cursor_on(screen, "app")
        && tree_dir_collapsed(screen, "app")
        && !tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && !tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && tree_dir_collapsed(screen, "No updates")
        && !tree_has(screen, "lib")
        && screen.contains("uncommitted changes")
        && screen.contains("seed app")
        && crumb.contains("workspace › [app]")
        && status.contains("drill")
        && !screen.contains("UNSTAGED")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
}

/// `z` folds the focused repo. Not Space (reviewed). Not `s`/`u` stage.
/// Not `h`/`l`. Not `zz` subtree.
///
/// Docs: Help MOVE `z` = toggle fold (instant; no-op on graph/diff).
/// Keymap: `z` is `Action::FoldToggle` on the focused list row. `zz` is
/// `pty_zz_toggles_subtree_not_only_row`. `h`/`l` is
/// `pty_fold_h_l_toggles_no_updates_group`.
///
/// After first paint the cursor is on the dirty README. `z` on that file
/// is a no-op (file stays). `k` focuses expanded `app`. `z` must collapse
/// that row (`>`, README gone, cursor stays). Graph-focused `z` must not
/// fold or unfold the hidden tree. A late `z` toggles the row open. A
/// no-op, `/` hide, Space `*`, stage, or workspace-root fold cannot pass.
#[test]
fn pty_z_folds_focused_repo() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file diff (z fold has not run)",
        WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        z_file_row_leaves_tree_open,
        "z on the dirty file is a no-op: README stays, app stays expanded",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        z_file_row_leaves_tree_open,
        "file-row z hold (not a delayed parent fold)",
        WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        app_repo_expanded_before_z,
        "k focuses expanded app; README still visible; graph loaded",
        GIT_WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        documented_z_folds_focused_repo,
        "z folds focused app: chevron >, README gone, cursor stays, graph stays",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_z_folds_focused_repo,
        "folded paint holds (not a flicker or workspace-root fold)",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_z_leaves_repo_folded,
        "Tab focuses the app graph; hidden tree stays folded",
        WAIT,
    );
    tui.key('z');
    tui.wait_pred(
        graph_z_leaves_repo_folded,
        "graph-focused z is a no-op (does not unfold the hidden tree)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        graph_z_leaves_repo_folded,
        "graph z hold (not a delayed unfold)",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        documented_z_folds_focused_repo,
        "Tab returns to the folded app row",
        WAIT,
    );
    tui.wait_ms(500);
    tui.key('z');
    tui.wait_pred(
        app_repo_expanded_before_z,
        "late z toggles the repo open (README back; not zz subtree)",
        WAIT,
    );
}

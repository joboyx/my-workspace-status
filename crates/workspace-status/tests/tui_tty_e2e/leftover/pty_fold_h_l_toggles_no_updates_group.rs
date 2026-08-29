use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, idle_dirty_readme_unstaged, no_updates_group_folded, no_wrong_overlays, status_row,
    tree_cursor_on, tree_dir_collapsed, tree_dir_expanded, tree_has, tree_line_containing,
    GIT_WAIT, WAIT,
};

/// Open No-updates group: expanded chevron, `lib` on the tree.
fn no_updates_group_open(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, "No updates") else {
        return false;
    };
    let Some(lib) = tree_line_containing(screen, "lib") else {
        return false;
    };
    tree_dir_expanded(screen, "No updates")
        && !tree_dir_collapsed(screen, "No updates")
        && line.contains('v')
        && line.contains('1')
        && lib.contains("@ lib")
        && lib.contains("& main")
}

fn fold_hl_no_wrong_chrome(screen: &str) -> bool {
    no_wrong_overlays(screen)
        && !screen.contains("z…")
        && !screen.contains("[workspace]")
        && crumb_row(screen).trim() == "workspace"
        && status_row(screen).contains(" tree")
        && status_row(screen).contains(" split")
        && status_row(screen).contains("focus right")
}

/// Tree-focused dirty file: `h`/`l` must not open No updates or fold `app`.
fn file_hl_leaves_group_folded(screen: &str) -> bool {
    idle_dirty_readme_unstaged(screen)
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "merger")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && fold_hl_no_wrong_chrome(screen)
        && !screen.contains("focus a repo for the graph")
}

/// Group focused and folded. `app` / README stay. Right pane is not a file diff.
fn group_hl_folded(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "lib")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && fold_hl_no_wrong_chrome(screen)
        && status_row(screen).contains("other pane")
}

/// Group focused and open. Cursor stays on the group. Not `z` toggle.
fn group_hl_open(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "lib")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_open(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && fold_hl_no_wrong_chrome(screen)
        && status_row(screen).contains("other pane")
}

/// `h`/`l` fold the No-updates group. `l` on a dirty file must not open it.
///
/// Docs + MOVE: tree-focused `h` / `l` close / open fold. Help `z` is a
/// separate toggle. Live PTY after first paint (cursor already on dirty
/// README) left the group folded. `G` then `l` opened it (`v`, `lib`
/// visible). A second `l` stayed open. `h` folded it (`>`, `lib` hidden).
/// A second `h` stayed folded. Not `z` toggle (`z…`). Not parent-repo
/// fold. Not Enter drill. Not chevron click.
///
/// After first paint the cursor is already on the dirty file. Do not `/`
/// search. A no-op on the group, pan, `z` toggle, file-`l` that opens the
/// group, or `h` that folds `app` / workspace cannot pass.
#[test]
fn pty_fold_h_l_toggles_no_updates_group() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        file_hl_leaves_group_folded,
        "first paint: cursor on dirty README, No updates folded, no lib",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        file_hl_leaves_group_folded,
        "`l` on the dirty file must not expand No updates (file, cursor, diff stay)",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        file_hl_leaves_group_folded,
        "`h` on the dirty file must not fold app or open No updates",
        WAIT,
    );

    tui.shift_letter('G');
    tui.wait_pred(
        group_hl_folded,
        "G focuses folded No updates (a no-op stays on README; l has not opened yet)",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        group_hl_open,
        "l on No updates opens the group (v, lib visible, cursor stays)",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        group_hl_open,
        "second l stays open (z toggle would hide lib)",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        group_hl_folded,
        "h folds No updates (>, lib hidden; app and README stay)",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        group_hl_folded,
        "second h stays folded (z toggle would reveal lib)",
        WAIT,
    );
}

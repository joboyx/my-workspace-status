use crate::harness::{tree_row_containing, PtySession};
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, documented_launch_first_paint, no_mouse_toggle_toast, no_updates_group_folded,
    no_wrong_overlays, status_row, title_has_files, tree_cursor_on, tree_dir_collapsed,
    tree_dir_expanded, tree_has, tree_line_containing, tree_pane_focused, GIT_WAIT, SETTLE_MS,
    TREE_DEPTH1_CHEVRON_COL, TREE_LABEL_COL, WAIT,
};

/// Two Downs on the same cell within 400ms are a double-click. Wait past
/// that window so the next chevron click is a fresh toggle, not a skip.
const DOUBLE_CLICK_EXPIRE_MS: u64 = 500;

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

fn chevron_no_wrong_chrome(screen: &str) -> bool {
    no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
        && !screen.contains("z…")
        && !screen.contains("[workspace]")
        && !screen.contains("[app]")
        && !screen.contains("[merger]")
        && !title_has_files(screen)
        && !screen.contains("wip.txt")
        && !screen.contains("WIP on graph")
        && status_row(screen).contains(" tree")
        && status_row(screen).contains(" split")
        && status_row(screen).contains("focus right")
        && status_row(screen).contains("other pane")
}

/// Group focused and open. Cursor stays on the group. Not Enter. Not `z`.
fn chevron_opens_no_updates(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "lib")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_open(screen)
        && tree_pane_focused(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && crumb_row(screen).trim() == "workspace"
        && chevron_no_wrong_chrome(screen)
}

/// Group focused and folded. `app` / README stay. Right pane is not a file diff.
fn chevron_folds_no_updates(screen: &str) -> bool {
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
        && tree_pane_focused(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && crumb_row(screen).trim() == "workspace"
        && chevron_no_wrong_chrome(screen)
}

/// Chevron on expanded `app` folded that row. Graph loaded. Stay left.
fn chevron_folds_app_repo(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "merger")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && !tree_has(screen, "README.md")
        && tree_dir_collapsed(screen, "app")
        && !tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && tree_pane_focused(screen)
        && screen.contains("uncommitted changes")
        && screen.contains("seed app")
        && !screen.contains("UNSTAGED")
        && crumb.contains("workspace › app")
        && !crumb.contains("[app]")
        && status.contains("focus right")
        && status.contains(" tree")
        && status.contains(" split")
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
        && !screen.contains("[workspace]")
        && !title_has_files(screen)
}

/// SGR click on a tree fold chevron toggles that row.
///
/// Docs / keymap: click the fold chevron to toggle that row's fold. Help
/// lists `Enter dblclick` as focus right / drill. A chevron click must
/// not Enter. Label click and right-pane click are other leftovers.
///
/// Live PTY after first paint (cursor on dirty README, No updates folded):
/// SGR press+release on the No-updates chevron opened the group (`v`,
/// `lib` visible), moved the cursor there, and replaced the file diff
/// with the empty graph hint. Stay left. A second click after the
/// double-click window folded it (`>`, `lib` hidden). The `app` chevron
/// then folded that repo (README gone, app graph loaded).
///
/// A no-op, click-to-select (label, no fold), right-pane click
/// (`[workspace]`), Enter drill, or paint-only change cannot pass.
#[test]
fn pty_click_chevron_toggles_fold() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: README file-diff, No updates folded (chevron has not run)",
        WAIT,
    );

    assert_ne!(
        TREE_LABEL_COL, TREE_DEPTH1_CHEVRON_COL,
        "chevron click must not hit the depth-1 label"
    );
    let nu_row = tree_row_containing(&tui.screen(), "No updates")
        .unwrap_or_else(|| panic!("No updates row:\n{}", tui.screen()));
    tui.sgr_click(TREE_DEPTH1_CHEVRON_COL, nu_row);
    tui.wait_pred(
        chevron_opens_no_updates,
        "SGR click on No updates chevron opens that group (v, lib, cursor stays, not Enter)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        chevron_opens_no_updates,
        "opened No updates holds (not a flicker, click-to-select, or toast-only)",
        WAIT,
    );

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let nu_row = tree_row_containing(&tui.screen(), "No updates")
        .unwrap_or_else(|| panic!("No updates row after open:\n{}", tui.screen()));
    tui.sgr_click(TREE_DEPTH1_CHEVRON_COL, nu_row);
    tui.wait_pred(
        chevron_folds_no_updates,
        "second chevron click folds No updates (>, lib hidden; app and README stay)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        chevron_folds_no_updates,
        "folded No updates holds (not a delayed second toggle or Enter)",
        WAIT,
    );

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let app_row = tree_row_containing(&tui.screen(), "app")
        .unwrap_or_else(|| panic!("app row:\n{}", tui.screen()));
    tui.sgr_click(TREE_DEPTH1_CHEVRON_COL, app_row);
    tui.wait_pred(
        chevron_folds_app_repo,
        "app chevron folds that repo (README gone, graph loaded, stay left)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        chevron_folds_app_repo,
        "folded app holds (not zz subtree, Enter, or a delayed unfold)",
        WAIT,
    );
}

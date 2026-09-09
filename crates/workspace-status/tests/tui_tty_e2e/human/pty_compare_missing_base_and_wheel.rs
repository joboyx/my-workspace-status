use crate::harness::{pane_body_start, tree_row_containing, PtySession, SGR_WHEEL_DOWN};
use crate::seed::{compare_ahead_workspace, git};
use crate::support::{
    panes_tree_unfocused_diff_focused, right_cursor_on, right_diff_has_focused_cursor,
    right_of_split, right_pane, status_row, tree_cursor_on, tree_has, tree_inactive_selection_on,
    GIT_WAIT, RIGHT_PANE_COL, TREE_LABEL_COL, WAIT,
};

/// Same gap as `pty_workspace_command_palette`. A same-letter `h`/`j`/`k`/`l`
/// burst is dropped by `discard_held_nav_backlog` after the first press.
const PALETTE_NAV_LETTER_GAP_MS: u64 = 50;

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

/// Type a palette filter at human key gaps for nav letters.
///
/// `keys("vs default")` can drop the `l` in `default`.
fn type_palette_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(PALETTE_NAV_LETTER_GAP_MS);
        }
    }
}

fn open_vs_default(tui: &mut PtySession) {
    tui.ctrl_letter('k');
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains("Ctrl-k"),
        "Ctrl-k opens the command palette (a no-op leaves idle chrome without Enter run)",
        WAIT,
    );
    type_palette_filter(tui, "vs default");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs default")
                && screen.contains("vs default")
        },
        "palette filter `vs default` shows `Diff vs default` (Enter before the filter lands would run the first catalog row; a dropped nav letter would keep a truncated query)",
        WAIT,
    );
    tui.enter();
}

fn first_paint_has_app(screen: &str) -> bool {
    tree_has(screen, "app")
}

fn cursor_on_app(screen: &str) -> bool {
    tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "workspace")
}

fn compare_vs_origin_main(screen: &str) -> bool {
    screen.contains("app · vs origin/main")
        && screen.contains("COMMITTED")
        && screen.contains("· vs")
        && screen.contains("alpha.txt")
}

fn workspace_only_without_compare(screen: &str) -> bool {
    screen.contains("# workspace") && !screen.contains("· vs")
}

fn missing_base_keeps_compare_tab(screen: &str) -> bool {
    screen.contains("Base ref not found: origin/main")
        && screen.contains("app · vs")
        && screen.contains("· vs")
        && !workspace_only_without_compare(screen)
}

fn cursor_on_alpha_first_file(screen: &str) -> bool {
    compare_vs_origin_main(screen)
        && tree_cursor_on(screen, "alpha.txt")
        && !tree_cursor_on(screen, "beta.txt")
        && !screen.contains("Mouse off")
}

fn cursor_on_beta_not_parked_tree(screen: &str) -> bool {
    tree_cursor_on(screen, "beta.txt")
        && !tree_cursor_on(screen, "alpha.txt")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "README.md")
}

fn beta_file_selected(screen: &str) -> bool {
    (tree_cursor_on(screen, "beta.txt") || tree_inactive_selection_on(screen, "beta.txt"))
        && !tree_cursor_on(screen, "alpha.txt")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "README.md")
}

fn compare_diff_has_focused_cursor(screen: &str) -> bool {
    right_pane(screen).contains('\u{258C}')
        && (right_cursor_on(screen, "COMMITTED")
            || right_cursor_on(screen, "@@")
            || right_cursor_on(screen, "alpha-body")
            || right_cursor_on(screen, "beta-body")
            || right_diff_has_focused_cursor(screen))
}

fn compare_diff_pane_focused(screen: &str) -> bool {
    let status = status_row(screen);
    beta_file_selected(screen)
        && compare_diff_has_focused_cursor(screen)
        && !status.contains("focus right")
        && (panes_tree_unfocused_diff_focused(screen)
            || status.contains("drill")
            || status.contains("Esc")
            || status.contains("back"))
}

fn right_cursor_line(screen: &str) -> Option<String> {
    right_pane(screen)
        .lines()
        .find(|line| line.contains('\u{258C}'))
        .map(str::to_string)
}

/// 0-based screen row of the focused diff bar, else a right-pane body line.
fn right_pane_body_row(screen: &str) -> Option<u16> {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    let start = pane_body_start(lines.len());
    let mut fallback = None;
    for (i, line) in lines.iter().enumerate().take(end).skip(start) {
        let right = right_of_split(line);
        if right.contains('\u{258C}') {
            return Some(i as u16);
        }
        if fallback.is_none()
            && (right.contains("COMMITTED")
                || right.contains("@@")
                || right.contains("alpha-body")
                || right.contains("beta-body"))
        {
            fallback = Some(i as u16);
        }
    }
    fallback
}

fn open_app_vs_default(tui: &mut PtySession) {
    tui.wait_pred(first_paint_has_app, "first paint: app is on the tree", WAIT);
    tui.search("app");
    tui.wait_pred(
        cursor_on_app,
        "/app keeps the tree cursor on app (a miss jumps to workspace)",
        WAIT,
    );
    open_vs_default(tui);
    tui.wait_pred(
        compare_vs_origin_main,
        "Diff vs default paints app · vs origin/main, COMMITTED, and alpha.txt",
        GIT_WAIT,
    );
}

/// Refresh after the compare base ref disappears keeps the compare tab.
///
/// Diff vs default on `app` paints `app · vs origin/main` and the ahead
/// files. Deleting `origin/main` then `r` paints `Base ref not found:
/// origin/main`. The tab stays. Workspace-only (`# workspace` without
/// `· vs`) fails. A no-op refresh that keeps the happy compare (no error)
/// fails.
#[test]
fn pty_compare_missing_base_after_refresh_keeps_tab() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    open_app_vs_default(&mut tui);

    git(
        &workspace.join("app"),
        &["update-ref", "-d", "refs/remotes/origin/main"],
    );
    tui.key('r');
    tui.wait_pred(
        missing_base_keeps_compare_tab,
        "r after deleting origin/main paints Base ref not found and keeps app · vs (a no-op refresh that stays on the happy compare fails)",
        GIT_WAIT,
    );
}

/// Vertical wheel moves the compare file list, then the focused DiffPane.
///
/// Diff vs default starts on `alpha.txt`. Wheel down over that left-pane
/// row selects `beta.txt` and must not land on the parked workspace tree.
/// Enter focuses the right DiffPane (`▌`, status drops `focus right`).
/// Wheel down over the right pane moves the focused diff row; `beta.txt`
/// stays selected. A no-op that stays on `alpha.txt` or the same `▌` line
/// fails.
#[test]
fn pty_compare_wheel_moves_file_list_and_diff() {
    let (_root, workspace) = compare_ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    open_app_vs_default(&mut tui);
    tui.wait_pred(
        cursor_on_alpha_first_file,
        "compare vs origin/main starts on alpha.txt, not beta.txt",
        GIT_WAIT,
    );

    let alpha_row = tree_row_containing(&tui.screen(), "alpha.txt")
        .unwrap_or_else(|| panic!("left compare alpha.txt row:\n{}", tui.screen()));
    tui.sgr_mouse(SGR_WHEEL_DOWN, TREE_LABEL_COL, alpha_row);
    tui.wait_pred(
        cursor_on_beta_not_parked_tree,
        "left compare wheel moves the file list to beta.txt (a no-op stays on alpha.txt; landing on app / workspace / README.md steals the parked tree)",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        compare_diff_pane_focused,
        "Enter focuses the compare DiffPane (▌, status drops focus right) and keeps beta.txt selected",
        WAIT,
    );

    let before_cursor = right_cursor_line(&tui.screen())
        .unwrap_or_else(|| panic!("focused ▌ on the compare DiffPane:\n{}", tui.screen()));
    let body_row = right_pane_body_row(&tui.screen())
        .unwrap_or_else(|| panic!("right pane body row:\n{}", tui.screen()));
    tui.sgr_mouse(SGR_WHEEL_DOWN, RIGHT_PANE_COL, body_row);
    tui.wait_pred(
        |screen| {
            beta_file_selected(screen)
                && right_cursor_line(screen).is_some_and(|line| line != before_cursor)
        },
        "right compare wheel moves the focused ▌ line (a no-op keeps the same DiffPane row; leaving beta.txt moves the file list)",
        WAIT,
    );
}

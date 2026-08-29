use std::fs;

use crate::harness::{left_tree, PtySession};
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, no_updates_group_folded, no_wrong_overlays, pane_top, right_pane, status_row,
    tree_cursor_on, tree_dir_expanded, tree_has, tree_line_containing, SETTLE_MS, WAIT,
};

/// Wide enough that split actually paints (`NARROW_SXS` is 100). Default 140
/// falls back to `inline (too narrow)` so `i` would only flip a header word.
const WIDE_COLS: u16 = 200;
const WIDE_ROWS: u16 = 32;

fn seed_nested_dirty(workspace: &std::path::Path) {
    fs::create_dir_all(workspace.join("app").join("src")).unwrap();
    fs::write(
        workspace.join("app").join("src").join("view.rs"),
        "fn view() {}\n",
    )
    .unwrap();
}

fn help_lists_t_and_i(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("VIEW")
        && screen.lines().any(|line| {
            line.contains("i")
                && line.contains("inline / split")
                && !line.contains("flat / tree")
                && !line.contains("cycle theme")
        })
        && screen.lines().any(|line| {
            line.contains("t")
                && line.contains("flat / tree")
                && !line.contains("inline / split")
                && !line.contains("cycle theme")
        })
        && screen.lines().any(|line| {
            line.contains("T") && line.contains("cycle theme") && !line.contains("flat / tree")
        })
}

fn panes_tree_focused_diff_unfocused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" tree ")
        && top.contains(" diff")
        && !top.contains(" diff ")
        && !top.contains(" graph")
        && !top.contains(" files")
}

fn on_view_rs_file(screen: &str) -> bool {
    tree_cursor_on(screen, "view.rs")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "src")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && no_updates_group_folded(screen)
        && panes_tree_focused_diff_unfocused(screen)
        && no_wrong_overlays(screen)
}

fn src_dir_row(screen: &str) -> Option<String> {
    left_tree(screen)
        .lines()
        .find(|line| line.contains("/ src") && !line.contains("view.rs"))
        .map(str::to_string)
}

fn view_rs_row(screen: &str) -> Option<String> {
    tree_line_containing(screen, "view.rs")
}

/// Directory trie: `src` is its own foldable row; the file is basename only.
fn tree_presents_directory(screen: &str) -> bool {
    let Some(dir) = src_dir_row(screen) else {
        return false;
    };
    let Some(file) = view_rs_row(screen) else {
        return false;
    };
    tree_dir_expanded(screen, "src")
        && dir.contains('v')
        && dir.contains("/ src")
        && !dir.contains("view.rs")
        && file.contains("view.rs")
        && file.contains('A')
        && !file.contains("view.rs  src")
        && status_row(screen).contains(" tree")
        && !status_row(screen).contains(" flat")
}

/// Flat paths: no `src` dir row; the file carries `view.rs  src`.
fn tree_presents_flat(screen: &str) -> bool {
    let Some(file) = view_rs_row(screen) else {
        return false;
    };
    src_dir_row(screen).is_none()
        && file.contains("view.rs  src")
        && file.contains('A')
        && !file.contains("/ src")
        && status_row(screen).contains(" flat")
        && !status_row(screen).contains(" tree")
}

fn split_body_line(right: &str) -> bool {
    right
        .lines()
        .any(|line| line.contains("│ 1 │ +fn view()") && !line.trim_start().starts_with("1 │"))
}

fn inline_body_line(right: &str) -> bool {
    right.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("1 │ +fn view()") && !line.contains("│ 1 │ +fn view()")
    })
}

fn file_diff_header(right: &str, mode: &str) -> bool {
    right.lines().next().is_some_and(|line| {
        line.contains("app/src/view.rs")
            && line.contains(mode)
            && !line.contains("too narrow")
            && (mode != "split" || !line.contains("inline"))
            && (mode != "inline" || !line.contains("split"))
    })
}

/// Preferred split actually paints two columns (empty left, add on the right).
fn file_diff_is_split(screen: &str) -> bool {
    let right = right_pane(screen);
    file_diff_header(&right, "split")
        && split_body_line(&right)
        && !inline_body_line(&right)
        && right.contains("NEW")
        && status_row(screen).contains(" split")
        && !status_row(screen).contains(" inline")
}

/// Preferred inline paints one unified add row (no empty left column).
fn file_diff_is_inline(screen: &str) -> bool {
    let right = right_pane(screen);
    file_diff_header(&right, "inline")
        && inline_body_line(&right)
        && !split_body_line(&right)
        && right.contains("NEW")
        && status_row(screen).contains(" inline")
        && !status_row(screen).contains(" split")
}

fn documented_tree_split_file(screen: &str) -> bool {
    on_view_rs_file(screen)
        && tree_presents_directory(screen)
        && file_diff_is_split(screen)
        && !crumb_row(screen).contains("Flat paths")
        && !crumb_row(screen).contains("Diff: inline")
}

fn documented_flat_split_file(screen: &str) -> bool {
    on_view_rs_file(screen)
        && tree_presents_flat(screen)
        && file_diff_is_split(screen)
        && crumb_row(screen).contains("Flat paths")
        && !crumb_row(screen).contains("Directory tree")
}

fn documented_tree_after_restore(screen: &str) -> bool {
    on_view_rs_file(screen)
        && tree_presents_directory(screen)
        && file_diff_is_split(screen)
        && crumb_row(screen).contains("Directory tree")
        && !crumb_row(screen).contains("Flat paths")
}

fn documented_inline_file(screen: &str) -> bool {
    on_view_rs_file(screen)
        && tree_presents_directory(screen)
        && file_diff_is_inline(screen)
        && crumb_row(screen).contains("Diff: inline")
        && !crumb_row(screen).contains("Diff: split")
}

fn documented_split_after_restore(screen: &str) -> bool {
    on_view_rs_file(screen)
        && tree_presents_directory(screen)
        && file_diff_is_split(screen)
        && crumb_row(screen).contains("Diff: split")
        && !crumb_row(screen).contains("Diff: inline")
}

/// `t` flips directory tree / flat paths. `i` flips split / inline on a file diff.
///
/// Docs + help VIEW: `t` = flat / tree, `i` = inline / split. Keymap: `t`
/// rebuilds the workspace trie vs flat paths (status `Directory tree` /
/// `Flat paths`). `i` toggles preferred diff layout on a file diff.
/// Split falls back to inline below 100 columns.
///
/// Live PTY (200 cols so split can paint; CSI-u `t` / `i` press
/// `CSI 116 ; 1 : 1 u` / `CSI 105 ; 1 : 1 u`): first paint is the `src`
/// dir row plus basename `view.rs`, header `app/src/view.rs  split`, and
/// a two-column NEW add. CSI-u `t` drops the dir row, paints
/// `view.rs  src`, pills `flat`, toasts `Flat paths`. A second `t`
/// restores the dir row and `Directory tree`. CSI-u `i` on that file
/// collapses the empty left column to unified `1 │ +fn view()`, pills
/// `inline`, toasts `Diff: inline`. A second `i` restores split.
/// Help does not refuse `i` off a file diff. A toast-only, pill-only,
/// or no-op cannot pass.
#[test]
fn pty_t_and_i_toggle_view_modes() {
    let (_root, workspace) = daily_workspace();
    seed_nested_dirty(&workspace);
    let mut tui = PtySession::open_size(&workspace, WIDE_COLS, WIDE_ROWS);
    tui.wait_pred(
        documented_tree_split_file,
        "first paint: directory tree, nested view.rs focused, split file diff",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        help_lists_t_and_i,
        "help VIEW lists i inline/split and t flat/tree as distinct from T theme",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("MOVE") && documented_tree_split_file(screen),
        "Esc closes help so t/i are view toggles, not help keys",
        WAIT,
    );

    tui.letter_press('t');
    tui.wait_pred(
        documented_flat_split_file,
        "CSI-u t flattens src into view.rs  src; pill flat; toast Flat paths",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_flat_split_file,
        "flat paths hold (not a flicker, pill-only, or tree that never dropped src)",
        WAIT,
    );

    tui.letter_press('t');
    tui.wait_pred(
        documented_tree_after_restore,
        "second CSI-u t restores the src dir row; toast Directory tree",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_tree_after_restore,
        "directory tree holds (not a no-op second t or a leftover flat file row)",
        WAIT,
    );

    tui.letter_press('i');
    tui.wait_pred(
        documented_inline_file,
        "CSI-u i on the file diff paints unified inline; pill inline; toast Diff: inline",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_inline_file,
        "inline layout holds (not a flicker, header-only, or split that never collapsed)",
        WAIT,
    );

    tui.letter_press('i');
    tui.wait_pred(
        documented_split_after_restore,
        "second CSI-u i restores two-column split; toast Diff: split",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_split_after_restore,
        "split layout holds (not a no-op second i or a leftover inline add row)",
        WAIT,
    );
}

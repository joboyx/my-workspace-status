//! Shared WAIT, tree, and crumb helpers for human-path TTY e2e.

use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::harness::{left_tree, PtySession};
use crate::seed::{REGIONS_ALPHA, REGIONS_OMEGA};

pub const WAIT: Duration = Duration::from_secs(12);

/// Compare-tab close control as painted: brackets around U+2717 (ballot x).
/// One terminal cell per char.
pub const TAB_CLOSE: &str = "[\u{2717}]";

pub const GIT_WAIT: Duration = Duration::from_secs(20);

pub const SETTLE_MS: u64 = 200;

pub fn tree_line_containing(screen: &str, needle: &str) -> Option<String> {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(needle))
        .map(str::to_string)
}

/// ASCII linked-worktree kind icon `L` immediately before `name`.
///
/// Trailing sync / counts stay to the right of the name. A trailing `L`
/// after `name` fails.
pub fn tree_line_has_leading_worktree_icon(line: &str, name: &str) -> bool {
    let Some(icon) = line.find('L') else {
        return false;
    };
    let Some(at) = line.find(name) else {
        return false;
    };
    icon < at && !line[at + name.len()..].contains('L')
}

/// Cells after `README.md` on the left-tree file row (trailing chrome).
pub fn after_readme_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "README.md")?;
    let at = line.find("README.md")?;
    Some(line[at + "README.md".len()..].to_string())
}

/// Trailing ASCII reviewed `*` after `README.md` on the left-tree file row.
///
/// The glyph is right-aligned before the status badge. A `*` elsewhere on
/// the row (or a full-screen substring) must not pass. `UNSTAGED` contains
/// the letters `STAGED`, so the badge on this row is the stage oracle.
pub fn readme_row_reviewed(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| after.contains('*') && after.contains('M') && !after.contains('S'))
}

pub fn readme_row_unreviewed_unstaged(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| !after.contains('*') && after.contains('M') && !after.contains('S'))
}

/// First paint: dirty README focused, no reviewed `*`. Not a repo row.
pub fn idle_dirty_readme_unreviewed(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && readme_row_unreviewed_unstaged(screen)
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("[x]")
}

/// Space marked the focused dirty file. File stays. Not fold. Not stage.
pub fn documented_space_reviewed(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && readme_row_reviewed(screen)
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("[x]")
}

pub fn op_finished(screen: &str, verb: &str) -> bool {
    screen.contains(&format!("{verb} 1 repo")) && !screen.contains("failed")
}

pub fn tree_cleared_ahead_behind(screen: &str) -> bool {
    let left = left_tree(screen);
    !left.contains("v1") && !left.contains("^1")
}

pub fn tree_has(screen: &str, needle: &str) -> bool {
    left_tree(screen).contains(needle)
}

/// Left-tree cursor bar (`▌`) on the row that contains `needle`.
pub fn tree_cursor_on(screen: &str, needle: &str) -> bool {
    tree_line_containing(screen, needle).is_some_and(|line| line.contains('\u{258C}'))
}

/// Thinner unfocused selection marker (`▏`) on the left-tree row that contains `needle`.
pub fn tree_inactive_selection_on(screen: &str, needle: &str) -> bool {
    tree_line_containing(screen, needle).is_some_and(|line| line.contains('\u{258F}'))
}

/// Breadcrumb is the workspace plus the focused file's repo (`workspace › app`).
pub fn launch_breadcrumb_workspace_app(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(crumb) = lines.get(lines.len().saturating_sub(2)) else {
        return false;
    };
    crumb.trim() == "workspace › app"
}

/// Idle status: directory-tree pill, split preferred but painted inline
/// (`split→inline`), help, file hints.
pub fn launch_status_chrome(screen: &str) -> bool {
    let Some(status) = screen.lines().last() else {
        return false;
    };
    status.contains(" tree")
        && status.contains(" split→inline ")
        && status.contains("? help")
        && status.contains("focus right")
        && status.contains("stage")
        && status.contains("revert")
        && status.contains("fetch")
        && status.contains("edit")
        && status.contains("reviewed")
        && !status.contains("drill")
        && !status.contains("SEARCH")
        && !status.contains("Flat paths")
}

/// Left tree focused, right diff unfocused.
pub fn launch_panes_left_tree_right_diff(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen)
}

/// Documented first paint on the daily seed. A blank, graph-first, ignored-
/// shown, unfolded No-updates, or paint-changed-only frame cannot pass.
pub fn documented_launch_first_paint(screen: &str) -> bool {
    let left = left_tree(screen);
    let readme = tree_line_containing(screen, "README.md");
    let no_updates = tree_line_containing(screen, "No updates");
    launch_panes_left_tree_right_diff(screen)
        && left.contains("# workspace")
        && left.contains("1 changed · all current")
        && tree_has(screen, "app")
        && tree_has(screen, "& main")
        && tree_has(screen, "README.md")
        && tree_has(screen, "merger")
        && tree_has(screen, "feature/graph")
        && tree_has(screen, "No updates")
        && readme.is_some_and(|line| line.contains('M'))
        && no_updates.is_some_and(|line| line.contains('>') && line.contains('1'))
        && !tree_has(screen, "lib")
        && !screen.contains("notes")
        && tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "No updates")
        && screen.contains("app/README.md  inline (too narrow)")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("@@ -1 +1,2 @@")
        && launch_breadcrumb_workspace_app(screen)
        && launch_status_chrome(screen)
        && !screen.contains("[workspace]")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("Working tree")
        && !screen.contains("focus a repo for the graph")
        && !screen.contains("No matching rows")
        && !screen.contains("loading")
}

/// Pane title row (`tree` / `graph` / `files` / `diff`). Skips the tab strip.
pub fn pane_top(screen: &str) -> &str {
    screen
        .lines()
        .find(|line| {
            (line.contains("tree") || line.contains("graph") || line.contains("files"))
                && (line.contains('─') || line.contains('┐') || line.contains('┌'))
        })
        .or_else(|| screen.lines().nth(1))
        .unwrap_or("")
}

/// Title row must not mark focus with `*` or `●`. PTY e2e is ASCII.
fn title_row_has_no_focus_glyph(screen: &str) -> bool {
    let top = pane_top(screen);
    !top.contains("* tree")
        && !top.contains("* graph")
        && !top.contains("* files")
        && !top.contains("* diff")
        && !top.contains('●')
}

fn title_row_names(screen: &str, left: &str, right: &str, absent: &[&str]) -> bool {
    let top = pane_top(screen);
    top.contains(left)
        && top.contains(right)
        && absent.iter().all(|name| !top.contains(name))
        && title_row_has_no_focus_glyph(screen)
}

/// Rounded overlay box. Confirm / picker / stash chrome covers the status row.
fn overlay_box_open(screen: &str) -> bool {
    screen.contains('╭')
}

/// Right-focused last crumb segment is `[name]`. Idle chrome or above an overlay.
fn breadcrumb_marks_right_focus(screen: &str) -> bool {
    screen.lines().any(|line| {
        let t = line.trim();
        if t == "[workspace]" || t.starts_with("[workspace]") {
            return true;
        }
        t.contains(" › ")
            && t.rsplit(" › ")
                .next()
                .is_some_and(|last| last.trim_start().starts_with('['))
    })
}

/// Left focus: status still offers `focus right`, or the crumb has no
/// `[brackets]` (a narrow row with an armed search can cut every hint).
fn left_pane_focused(screen: &str) -> bool {
    let status = status_line(screen);
    status.contains("focus right")
        || ((overlay_box_open(screen) || status.contains("? help"))
            && !status.contains("Esc ←")
            && !breadcrumb_marks_right_focus(screen))
}

/// Right focus: drill / Esc on status, or crumb `[brackets]`.
fn right_pane_focused(screen: &str) -> bool {
    let status = status_line(screen);
    breadcrumb_marks_right_focus(screen)
        || (!status.contains("focus right") && (status.contains("drill") || status.contains("Esc")))
}

/// True when a pane title on the top row names `files`.
pub fn title_has_files(screen: &str) -> bool {
    pane_top(screen).contains("files")
}

/// True when a pane title on the top row names `graph`.
pub fn title_has_graph(screen: &str) -> bool {
    pane_top(screen).contains("graph")
}

/// True when a pane title on the top row names `diff`.
pub fn title_has_diff(screen: &str) -> bool {
    pane_top(screen).contains("diff")
}

/// Breadcrumb sits on the penultimate row (status is last).
pub fn crumb_line(screen: &str) -> &str {
    let lines: Vec<&str> = screen.lines().collect();
    lines
        .get(lines.len().saturating_sub(2))
        .copied()
        .unwrap_or("")
}

pub fn status_line(screen: &str) -> &str {
    screen.lines().last().unwrap_or("")
}

/// Left tree focused, right file-diff unfocused. Not graph / not files.
pub fn panes_tree_focused_diff_unfocused(screen: &str) -> bool {
    title_row_names(screen, "tree", "diff", &["graph", "files"]) && left_pane_focused(screen)
}

/// Left tree unfocused, right file-diff focused. Not graph / not files.
pub fn panes_tree_unfocused_diff_focused(screen: &str) -> bool {
    title_row_names(screen, "tree", "diff", &["graph", "files"]) && right_pane_focused(screen)
}

/// Left tree focused, right graph unfocused. Not files / not a file diff.
pub fn panes_tree_focused_graph_unfocused(screen: &str) -> bool {
    title_row_names(screen, "tree", "graph", &["files", "diff"]) && left_pane_focused(screen)
}

/// Left tree unfocused, right graph focused. Not files / not a file diff.
pub fn panes_tree_unfocused_graph_focused(screen: &str) -> bool {
    title_row_names(screen, "tree", "graph", &["files", "diff"]) && right_pane_focused(screen)
}

/// Right commit-files focused (left is graph). Not a file diff.
pub fn panes_files_focused(screen: &str) -> bool {
    title_row_names(screen, "graph", "files", &["diff"]) && right_pane_focused(screen)
}

/// Left graph focused, right commit-files unfocused. Not a file diff.
pub fn panes_graph_focused_files_unfocused(screen: &str) -> bool {
    title_row_names(screen, "graph", "files", &["diff"]) && left_pane_focused(screen)
}

/// Left commit-files focused, right commit-diff unfocused. Not graph.
pub fn panes_files_focused_diff_unfocused(screen: &str) -> bool {
    title_row_names(screen, "files", "diff", &["graph"]) && left_pane_focused(screen)
}

/// Left commit-files unfocused, right commit-diff focused. Not graph.
pub fn panes_files_unfocused_diff_focused(screen: &str) -> bool {
    title_row_names(screen, "files", "diff", &["graph"]) && right_pane_focused(screen)
}

/// Merger graph body. Files drill (`wip.txt` / files title) cannot pass.
pub fn merger_graph_body(screen: &str) -> bool {
    screen.contains("WIP on graph")
        && screen.contains("stash@{0}")
        && screen.contains("feature/graph")
        && screen.contains("working tree clean")
        && !screen.contains("wip.txt")
        && !title_has_files(screen)
        && !screen.contains("[stash@{0}]")
}

/// File-diff chrome from the launch README row. Must be gone after drill.
pub fn still_file_diff(screen: &str) -> bool {
    screen.contains("app/README.md")
        || screen.contains("UNSTAGED")
        || screen.contains("@@ -1 +1,2 @@")
        || screen.contains("+dirty")
        || launch_panes_left_tree_right_diff(screen)
}

/// Left-focused merger row with its graph loaded. Enter has not run.
pub fn merger_graph_left_unfocused(screen: &str) -> bool {
    let crumb = crumb_line(screen);
    let status = status_line(screen);
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && crumb.contains("workspace › merger")
        && !crumb.contains("[merger]")
        && status.contains("focus right")
        && !status.contains("drill")
        && !status.contains("← tree")
        && merger_graph_body(screen)
        && !still_file_diff(screen)
        && !screen.contains("SEARCH")
}

/// Documented Enter on a graph-capable tree row: focus the graph, stay
/// on merger. A no-op, a files drill, or README/file-diff cannot pass.
pub fn merger_graph_drilled_right(screen: &str) -> bool {
    let crumb = crumb_line(screen);
    let status = status_line(screen);
    panes_tree_unfocused_graph_focused(screen)
        && tree_has(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "merger")
        && tree_inactive_selection_on(screen, "merger")
        && crumb.contains("workspace › [merger]")
        && status.contains("drill")
        && status.contains("Esc")
        && (status.contains("← tree") || status.contains("Esc   clear"))
        && !status.contains("focus right")
        && merger_graph_body(screen)
        && !still_file_diff(screen)
        && !screen.contains("SEARCH")
}

/// Wrong keys: Enter drills files, `/` types SEARCH, Shift+S opens stash.
pub fn not_files_search_or_stash(screen: &str) -> bool {
    !title_has_files(screen)
        && !screen.contains("keep.txt")
        && !screen.contains("SEARCH")
        && !screen.contains("Stash ")
}

/// `--all` graph for focusbox. Keep, main, and noise tips are all visible.
pub fn focusbox_full_graph_body(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("noise-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("focus-root-commit")
        && screen.contains("working tree clean")
        && screen.contains("[+feature/keep]")
        && screen.contains("[topic/noise]")
        && screen.contains("[main]")
}

/// Ancestors of `feature/keep` only. A no-op or `--all` cannot pass.
pub fn focusbox_keep_only_graph_body(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("focus-root-commit")
        && screen.contains("[+feature/keep]")
        && !screen.contains("noise-leaf-commit")
        && !screen.contains("main-leaf-commit")
        && !screen.contains("[topic/noise]")
        && !screen.contains("[main]")
}

/// Graph loaded, left focus, `--all`. Tab / `o` have not run.
pub fn focusbox_graph_left_full(screen: &str) -> bool {
    let crumb = crumb_line(screen);
    let status = status_line(screen);
    panes_tree_focused_graph_unfocused(screen)
        && tree_cursor_on(screen, "focusbox")
        && crumb.contains("workspace › focusbox")
        && !crumb.contains("[focusbox]")
        && !crumb.contains("graph focus:")
        && status.contains("focus right")
        && !status.contains("drill")
        && !status.contains("clear focus")
        && !screen.contains("Focus branches")
        && focusbox_full_graph_body(screen)
        && not_files_search_or_stash(screen)
}

/// Tab focused the graph. Full `--all` history. Branch focus is off.
pub fn focusbox_graph_right_full(screen: &str) -> bool {
    let crumb = crumb_line(screen);
    let status = status_line(screen);
    panes_tree_unfocused_graph_focused(screen)
        && tree_has(screen, "focusbox")
        && crumb.contains("workspace › [focusbox]")
        && !crumb.contains("graph focus:")
        && !crumb.contains("full graph")
        && status.contains("drill")
        && status.contains("Esc")
        && (status.contains("← tree") || status.contains("Esc   clear"))
        && status.contains("focus branches")
        && !status.contains("clear focus")
        && !status.contains("focus right")
        && !screen.contains("Focus branches")
        && focusbox_full_graph_body(screen)
        && not_files_search_or_stash(screen)
}

/// `o` overlay is open. Cursor may sit on any local branch (sort is
/// authordate). Current checkout still shows `* feature/keep`.
///
/// The overlay covers the status row, so crumb/status helpers do not apply.
pub fn graph_focus_overlay_open(screen: &str) -> bool {
    graph_focus_overlay_chrome(screen) && screen.contains("topic/noise")
}

/// Focus branches dialog chrome, whatever the filter hides. The centered
/// dialog covers the graph rows behind it, so `topic/noise` shows only
/// while the list itself has that row.
fn graph_focus_overlay_chrome(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && screen.contains("Focus branches")
        && screen.contains("filter:")
        && screen.contains("* feature/keep")
        && screen.contains("Enter apply")
        && screen.contains("Ctrl-o clear")
        && screen.contains("Esc cancel")
        && screen.contains("workspace › [focusbox]")
        && !screen.contains("graph focus:")
        && !screen.contains("drill")
        && not_files_search_or_stash(screen)
}

/// Overlay filter `feature`: cursor on `feature/keep`. Not `main`.
///
/// Every letter types into the overlay filter. `feature` is unique to
/// `feature/keep`.
pub fn graph_focus_overlay_filtered_keep(screen: &str) -> bool {
    graph_focus_overlay_chrome(screen)
        && screen.contains("filter: feature")
        && screen
            .lines()
            .any(|line| line.contains('❯') && line.contains("feature/keep"))
        && !screen.contains("[ ]   main")
}

/// Applied keep focus: toast, `O` clear-focus hint, keep-only graph.
pub fn graph_focus_applied_keep(screen: &str) -> bool {
    let crumb = crumb_line(screen);
    let status = status_line(screen);
    panes_tree_unfocused_graph_focused(screen)
        && crumb.contains("[focusbox]")
        && crumb.contains("graph focus: feature/keep")
        && status.contains("drill")
        && status.contains("Esc")
        && (status.contains("← tree") || status.contains("Esc   clear"))
        && status.contains("focus branches")
        && status.contains("clear focus")
        && !status.contains("focus right")
        && !screen.contains("Focus branches")
        && !screen.contains("Enter apply")
        && focusbox_keep_only_graph_body(screen)
        && not_files_search_or_stash(screen)
}

/// CSI-u Shift+O restored `--all`. Clear-focus hint is gone. Stay on graph.
pub fn graph_focus_cleared_full(screen: &str) -> bool {
    let crumb = crumb_line(screen);
    let status = status_line(screen);
    panes_tree_unfocused_graph_focused(screen)
        && crumb.contains("[focusbox]")
        && crumb.contains("full graph")
        && !crumb.contains("graph focus:")
        && status.contains("drill")
        && status.contains("focus branches")
        && !status.contains("clear focus")
        && !status.contains("focus right")
        && !screen.contains("Focus branches")
        && focusbox_full_graph_body(screen)
        && not_files_search_or_stash(screen)
}

/// Tab to the graph, `o`, filter `feature`, Enter applies `feature/keep`.
///
/// Overlay sort is authordate, so cursor 0 is not always the current `*`
/// row. Filter-then-Enter is the documented apply path.
pub fn apply_current_keep_graph_focus(tui: &mut PtySession) {
    tui.wait_pred(
        focusbox_graph_left_full,
        "focusbox graph loaded on the left (full --all; o / Tab have not run)",
        GIT_WAIT,
    );
    tui.tab();
    tui.wait_pred(
        focusbox_graph_right_full,
        "Tab focuses the graph; full history; branch focus is off",
        WAIT,
    );
    tui.key('o');
    tui.wait_pred(
        graph_focus_overlay_open,
        "o opens Focus branches (files drill / no-op / SEARCH cannot pass)",
        WAIT,
    );
    tui.keys("feature");
    tui.wait_pred(
        graph_focus_overlay_filtered_keep,
        "typing feature filters the overlay onto feature/keep (cursor 0 / main cannot pass)",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        graph_focus_applied_keep,
        "Enter applies feature/keep: toast, clear-focus hint, keep-only graph",
        GIT_WAIT,
    );
}

/// Depth-1 fold chevron column when the tree inner origin is x=1.
pub const TREE_DEPTH1_CHEVRON_COL: u16 = 4;

/// Label column past the chevron (same as the tree-hscroll setup click).
pub const TREE_LABEL_COL: u16 = 8;

/// Right pane on the default 140-col layout (tree fraction 0.4).
pub const RIGHT_PANE_COL: u16 = 90;

/// ASCII collapsed chevron (`>`) on the left-tree row that contains `name`.
pub fn tree_dir_collapsed(screen: &str, name: &str) -> bool {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(name))
        .is_some_and(|line| line.contains('>'))
}

/// ASCII expanded chevron (`v`) on the left-tree row that contains `name`.
pub fn tree_dir_expanded(screen: &str, name: &str) -> bool {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(name))
        .is_some_and(|line| line.contains('v') && !line.contains('>'))
}

/// Folded No-updates group: collapsed chevron, count 1, `lib` hidden.
pub fn no_updates_group_folded(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, "No updates") else {
        return false;
    };
    tree_dir_collapsed(screen, "No updates")
        && !tree_dir_expanded(screen, "No updates")
        && line.contains('>')
        && line.contains('1')
        && !tree_has(screen, "lib")
}

/// Last status row (mode pills + hint chips).
pub fn status_row(screen: &str) -> &str {
    screen_line_from_end(screen, 0)
}

/// Breadcrumb row (path left, toast right).
pub fn crumb_row(screen: &str) -> &str {
    screen_line_from_end(screen, 1)
}

/// Trailing `M ` badge, not staged `S `, not reviewed `*`.
pub fn readme_unstaged_badge(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| after.contains("M ") && !after.contains('S') && !after.contains('*'))
}

pub fn has_stage_hint(screen: &str) -> bool {
    let status = status_row(screen);
    status.contains("stage") && !status.contains("unstage")
}

pub fn has_unstage_hint(screen: &str) -> bool {
    status_row(screen).contains("unstage")
}

pub fn pane_unstaged_readme(screen: &str) -> bool {
    screen.contains("UNSTAGED") && screen.contains("app/README.md") && screen.contains("+dirty")
}

pub fn no_wrong_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("[x]")
        && !screen.contains("nothing to stage")
        && !screen.contains("nothing to unstage")
}

/// First paint: dirty README focused, unstaged. Not a repo row.
pub fn idle_dirty_readme_unstaged(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && has_stage_hint(screen)
        && !has_unstage_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace › app"
        && no_wrong_overlays(screen)
}

/// Cells after `syncbox` on the left-tree repo row (branch + sync mark).
pub fn after_syncbox_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "syncbox")?;
    let at = line.find("syncbox")?;
    Some(line[at + "syncbox".len()..].to_string())
}

/// Trailing clean `.` on the syncbox row after it lands in No updates.
pub fn syncbox_row_current(screen: &str) -> bool {
    after_syncbox_name(screen).is_some_and(|after| {
        after.contains("& main")
            && after.contains('.')
            && !after.contains("^1")
            && !after.contains("v1")
    })
}

/// Trailing ASCII behind-by-1 (`v1`) on the syncbox tree row.
///
/// A `v1` on the graph header or a full-screen substring must not pass.
pub fn syncbox_row_behind(screen: &str) -> bool {
    after_syncbox_name(screen).is_some_and(|after| after.contains("v1") && after.contains("& main"))
}

/// Cells after `name` on the left-tree row (branch + sync mark).
pub fn after_repo_name(screen: &str, name: &str) -> Option<String> {
    let line = tree_line_containing(screen, name)?;
    let at = line.find(name)?;
    Some(line[at + name.len()..].to_string())
}

/// Trailing ASCII behind-by-1 (`v1`) on the named left-tree repo row.
pub fn repo_row_behind(screen: &str, name: &str) -> bool {
    after_repo_name(screen, name)
        .is_some_and(|after| after.contains("v1") && after.contains("& main"))
}

/// Trailing in-sync mark on the named left-tree repo row (No updates `.`).
pub fn repo_row_in_sync(screen: &str, name: &str) -> bool {
    after_repo_name(screen, name).is_some_and(|after| {
        after.contains("& main")
            && after.contains('.')
            && !after.contains("^1")
            && !after.contains("v1")
    })
}

pub fn has_fetch_hint(screen: &str) -> bool {
    status_row(screen).contains("fetch")
}

pub fn has_pull_hint(screen: &str) -> bool {
    status_row(screen).contains("pull")
}

pub fn graph_subject_line(screen: &str, subject: &str) -> Option<String> {
    screen
        .lines()
        .find(|line| line.contains(subject))
        .map(str::to_string)
}

pub fn graph_subject_meta_line(screen: &str, subject: &str) -> Option<String> {
    let mut lines = screen.lines();
    lines.find(|line| line.contains(subject))?;
    lines.next().map(str::to_string)
}

pub fn seed_tree_page_files(workspace: &Path) {
    let app = workspace.join("app");
    for i in 0..30 {
        fs::write(
            app.join(format!("page-{i:02}.txt")),
            format!("page-{i:02}-body\n"),
        )
        .unwrap();
    }
}

pub fn page_file_body_visible(screen: &str) -> bool {
    (0..30).any(|i| screen.contains(&format!("page-{i:02}-body")))
}

/// Left tree focused, right graph unfocused. Not files / not a file diff.
pub fn tree_pane_focused(screen: &str) -> bool {
    panes_tree_focused_graph_unfocused(screen)
}

/// Left tree unfocused, right graph focused. Not files / not a file diff.
pub fn graph_pane_focused(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
}

pub fn graph_cursor_on(screen: &str, needle: &str) -> bool {
    screen.lines().any(|line| {
        let right = right_of_split(line);
        right.contains('\u{258C}') && right.contains(needle)
    })
}

/// Focused list cursor bar (`▌`) on the right-pane row that contains `needle`.
pub fn right_cursor_on(screen: &str, needle: &str) -> bool {
    right_pane(screen)
        .lines()
        .any(|line| line.contains('\u{258C}') && line.contains(needle))
}

/// Thinner unfocused selection marker (`▏`) on the right-pane row that contains `needle`.
pub fn right_inactive_selection_on(screen: &str, needle: &str) -> bool {
    right_pane(screen)
        .lines()
        .any(|line| line.contains('\u{258F}') && line.contains(needle))
}

pub fn right_diff_has_focused_cursor(screen: &str) -> bool {
    right_cursor_on(screen, "UNSTAGED")
        || right_cursor_on(screen, "+dirty")
        || right_cursor_on(screen, "@@")
}

pub fn right_diff_has_inactive_selection(screen: &str) -> bool {
    right_inactive_selection_on(screen, "UNSTAGED")
        || right_inactive_selection_on(screen, "+dirty")
        || right_inactive_selection_on(screen, "@@")
}

pub fn no_mouse_toggle_toast(screen: &str) -> bool {
    !screen.contains("Mouse off") && !screen.contains("Mouse on")
}

pub fn screen_line_from_end(screen: &str, from_end: usize) -> &str {
    let lines: Vec<&str> = screen.lines().collect();
    lines
        .get(lines.len().saturating_sub(from_end + 1))
        .copied()
        .unwrap_or("")
}

/// Right-pane vertical scrollbar thumb at the top of its track: the first
/// right-pane row whose last cell is the bar (`█` thumb or `║` track) is the
/// thumb. The bar shows whenever the content overflows, at the origin too.
pub fn right_vbar_at_top(screen: &str) -> bool {
    right_pane(screen)
        .lines()
        .filter_map(|line| line.trim_end_matches('│').chars().last())
        .find(|ch| matches!(ch, '█' | '║'))
        == Some('█')
}

/// Right-pane cells, excluding top/bottom chrome (same rows as [`left_tree`]).
pub fn right_pane(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    let start = crate::harness::pane_body_start(lines.len());
    lines[start..end]
        .iter()
        .map(|line| right_of_split(line))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn right_of_split(line: &str) -> String {
    for sep in ["││", "┐┌", "┘└"] {
        if let Some(idx) = line.find(sep) {
            return line[idx + sep.len()..].to_string();
        }
    }
    String::new()
}

/// Cells after `wip.txt` on the left-tree file row (trailing chrome).
pub fn after_wip_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "wip.txt")?;
    let at = line.find("wip.txt")?;
    Some(line[at + "wip.txt".len()..].to_string())
}

/// Restored `wip.txt` on the merger tree. Badge `A` is the staged add.
pub fn merger_wip_added(screen: &str) -> bool {
    tree_has(screen, "wip.txt")
        && tree_has(screen, "merger")
        && after_wip_name(screen).is_some_and(|after| after.contains('A'))
}

/// Right pane still lists merger `stash@{0}` (`WIP on graph`).
pub fn graph_stash_still_listed(screen: &str) -> bool {
    let right = right_pane(screen);
    right.contains("WIP on graph") || right.contains("stash@{0}")
}

/// Pull, apply-only, drop-only, or stash-push toasts. Graph pop is `popped`.
pub fn no_pull_or_other_stash_write(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    !crumb.contains("Pulled")
        && !crumb.contains("applied")
        && !crumb.contains("dropped")
        && !crumb.contains("Stashed")
        && !crumb.contains("failed")
}

/// SEARCH / help / stash-menu / pull-idle toasts that are not graph pop.
pub fn no_wrong_stash_pop_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("Drop stash@{")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("no visible repos for that op")
        && no_mouse_toggle_toast(screen)
}

/// Tab focused the merger graph. HEAD is clean. Stash is listed. Pop idle.
pub fn graph_focused_merger_stash_listed(screen: &str) -> bool {
    merger_graph_drilled_right(screen)
        && graph_pane_focused(screen)
        && graph_stash_still_listed(screen)
        && !tree_has(screen, "wip.txt")
        && !crumb_row(screen).contains("popped")
        && no_pull_or_other_stash_write(screen)
        && no_wrong_stash_pop_overlays(screen)
}

/// Tab lands on the uncommitted row. Stash is the next `j`.
pub fn graph_focused_merger_before_stash_pop(screen: &str) -> bool {
    graph_focused_merger_stash_listed(screen)
        && graph_cursor_on(screen, "working tree")
        && !graph_cursor_on(screen, "WIP on graph")
}

/// Graph cursor on `stash@{0}` (`WIP on graph`). Hint `p` is pop stash.
pub fn stash_row_ready_to_pop(screen: &str) -> bool {
    let status = status_row(screen);
    graph_focused_merger_stash_listed(screen)
        && graph_cursor_on(screen, "WIP on graph")
        && !graph_cursor_on(screen, "working tree")
        && status.contains("apply stash")
        && status.contains("pop stash")
        && status.contains("drop stash")
        && !status.contains("pull")
}

/// Graph `p` popped `stash@{0}`: apply + drop. Apply-only / drop-only fail.
pub fn documented_graph_stash_pop(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "merger")
        && merger_wip_added(screen)
        && !graph_stash_still_listed(screen)
        && screen.contains("uncommitted changes")
        && !screen.contains("working tree clean")
        && crumb.contains("popped stash@{0}")
        && no_pull_or_other_stash_write(screen)
        && !status.contains("pop stash")
        && !status.contains("apply stash")
        && !status.contains("drop stash")
        && status.contains("drill")
        && status.contains(" tree")
        && !status.contains(" split")
        && no_wrong_stash_pop_overlays(screen)
}

/// SEARCH / help / merger-stash / stage toasts that are not app stash create.
pub fn no_stash_wrong_ops(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("popped")
        && !crumb_row(screen).contains("staged")
}

/// App graph lists `stash@{0}` (`WIP on main`). Merger stash cannot pass.
pub fn app_stash_on_graph(screen: &str) -> bool {
    screen.contains("WIP on main")
        && screen.contains("stash@{0}")
        && screen.contains("seed app")
        && !screen.contains("WIP on graph")
}

/// Graph stash row hints on the status row, not the overlay op rows.
pub fn has_graph_stash_hints(screen: &str) -> bool {
    let status = status_row(screen);
    status.contains("apply stash") && status.contains("drop stash") && status.contains("pop stash")
}

/// Stash overlay create row: ` s ` key chip, then the `stash` label.
const STASH_CREATE_OP_ROW: &str = " s  stash";

/// CSI-u Shift+S opened the create-only overlay on the dirty README.
pub fn stash_create_overlay_open(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && screen.contains("Stash app")
        && screen.contains(STASH_CREATE_OP_ROW)
        && screen.contains("Esc cancel")
        && !screen.contains("apply stash")
        && !screen.contains("pop stash")
        && !screen.contains("drop stash")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on main")
}

/// Overlay `s` created a path-scoped stash. README left the tree.
pub fn documented_stash_created(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_has(screen, "README.md")
        && !tree_has(screen, "app")
        && tree_has(screen, "No updates")
        && tree_has(screen, "0 changed")
        && crumb.contains("Stashed 1 file")
        && !screen.contains("Stash app")
        && !screen.contains(STASH_CREATE_OP_ROW)
        && !screen.contains("UNSTAGED")
        && !screen.contains("WIP on main")
        && !crumb.contains("staged")
        && !crumb.contains("applied")
        && !crumb.contains("popped")
        && !crumb.contains("dropped")
        && no_stash_wrong_ops(screen)
}

/// `l` unfolded No updates. App is listed. README is gone.
pub fn no_updates_unfolded_after_stash(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && tree_has(screen, "app")
        && tree_has(screen, "lib")
        && !tree_has(screen, "README.md")
}

/// `l` then `j`: app is focused under No updates. App graph shows the stash.
pub fn app_focused_stash_visible(screen: &str) -> bool {
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "lib")
        && tree_has(screen, "No updates")
        && app_stash_on_graph(screen)
        && screen.contains("working tree clean")
        && crumb_row(screen).contains("workspace › app")
        && !crumb_row(screen).contains("[app]")
        && tree_pane_focused(screen)
        && no_stash_wrong_ops(screen)
}

/// Tab focused the app graph on the working-tree row. Stash is the next row.
pub fn app_graph_working_tree_focused(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_has(screen, "app")
        && graph_cursor_on(screen, "working tree clean")
        && !graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && crumb_row(screen).contains("[app]")
        && no_stash_wrong_ops(screen)
}

/// `j` landed on the app stash row. Graph `a` / `D` hints. Not merger.
pub fn app_graph_stash_row_focused(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_has(screen, "app")
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && has_graph_stash_hints(screen)
        && crumb_row(screen).contains("[app]")
        && !screen.contains("Drop stash@{0}?")
        && no_stash_wrong_ops(screen)
}

/// Graph `a` applied. README is dirty again. Stash stays (not pop).
pub fn documented_stash_applied(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "1 changed")
        && readme_unstaged_badge(screen)
        && screen.contains("uncommitted changes")
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && has_graph_stash_hints(screen)
        && crumb.contains("applied stash@{0}")
        && !crumb.contains("popped")
        && !crumb.contains("dropped")
        && !crumb.contains("Stashed")
        && !screen.contains("Drop stash@{0}?")
        && no_stash_wrong_ops(screen)
}

/// CSI-u Shift+D opened drop confirm. Stash and dirty README stay until `y`.
pub fn stash_drop_confirm_open(screen: &str) -> bool {
    graph_pane_focused(screen)
        && screen.contains("Drop stash@{0}?")
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && screen.contains("uncommitted changes")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("popped")
}

/// Confirm `y` dropped the stash. Dirty README stays. Not pop.
pub fn documented_stash_dropped(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "1 changed")
        && readme_unstaged_badge(screen)
        && screen.contains("uncommitted changes")
        && graph_cursor_on(screen, "seed app")
        && !screen.contains("WIP on main")
        && !screen.contains("Drop stash@{0}?")
        && !status.contains("apply stash")
        && !status.contains("drop stash")
        && crumb.contains("dropped stash@{0}")
        && !crumb.contains("popped")
        && !crumb.contains("applied")
        && no_stash_wrong_ops(screen)
}

/// Make every later `git` call in this repo fail by truncating the index.
///
/// Fixtures used to `chmod 0o000` the index. Root ignores the mode bits,
/// so the repo stayed healthy under `uid 0` and the status-failed
/// assertions timed out in any root container (CI images, devcontainers,
/// agent sandboxes). A short, invalid index header fails for every uid:
/// `fatal: .git/index: index file smaller than expected`.
pub fn corrupt_index(index: &Path) {
    assert!(index.is_file(), "index must exist: {}", index.display());
    fs::write(index, b"DIRC\0\0\0")
        .unwrap_or_else(|err| panic!("corrupt {}: {err}", index.display()));
}

/// Gap between visual-highlight keys so held-nav does not coalesce them.
pub const VISUAL_KEY_GAP_MS: u64 = 50;

/// Launch paint of [`crate::seed::two_hunk_regions_workspace`]: cursor on
/// `regions.txt`, both hunks in the UNSTAGED diff.
pub fn regions_first_paint(screen: &str) -> bool {
    tree_cursor_on(screen, "regions.txt")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && screen.contains(REGIONS_ALPHA)
        && screen.contains(REGIONS_OMEGA)
        && !screen.contains("VISUAL")
        && !screen.contains("MOVE")
}

/// The regions.txt diff has focus, with no highlight and no overlay.
pub fn regions_diff_focused(screen: &str) -> bool {
    tree_has(screen, "regions.txt")
        && !tree_cursor_on(screen, "regions.txt")
        && panes_tree_unfocused_diff_focused(screen)
        && screen.contains("UNSTAGED")
        && screen.contains(REGIONS_ALPHA)
        && !screen.contains("VISUAL")
        && !screen.contains("MOVE")
}

/// `V` highlight is on the focused regions.txt diff (VISUAL hint row).
pub fn regions_highlight_active(screen: &str) -> bool {
    screen.contains("VISUAL")
        && screen.contains("stage / unstage")
        && screen.contains("revert")
        && screen.contains("cancel highlight")
        && panes_tree_unfocused_diff_focused(screen)
        && tree_has(screen, "regions.txt")
        && !tree_cursor_on(screen, "regions.txt")
        && !screen.contains("MOVE")
}

/// Boxed `y` / `n` confirm of a highlighted-range revert.
pub fn revert_range_confirm(screen: &str) -> bool {
    screen.contains("Discard highlighted lines in regions.txt?")
        && screen.contains("cancel")
        && !screen.contains("VISUAL")
        && !screen.contains("revert + delete untracked")
}

/// Launch on the two-hunk regions.txt diff, Tab to it, `V`, then plain
/// `j` presses until the highlight covers the first hunk (ALPHA).
pub fn open_regions_first_hunk_highlight(workspace: &Path) -> PtySession {
    let mut tui = PtySession::open(workspace);
    tui.wait_pred(
        regions_first_paint,
        "launch is the two-hunk dirty regions.txt file diff",
        WAIT,
    );
    tui.tab();
    tui.wait_pred(
        regions_diff_focused,
        "Tab focuses the two-hunk regions.txt diff",
        WAIT,
    );
    tui.shift_letter('V');
    tui.wait_pred(regions_highlight_active, "Shift+V paints VISUAL", WAIT);
    tui.wait_ms(VISUAL_KEY_GAP_MS);
    for _ in 0..6 {
        tui.letter_press('j');
        tui.wait_ms(VISUAL_KEY_GAP_MS);
    }
    tui.wait_pred(
        |screen| regions_highlight_active(screen) && screen.contains(REGIONS_ALPHA),
        "j extends VISUAL over the first hunk (ALPHA)",
        WAIT,
    );
    tui
}

/// Type a command-palette filter with a gap after nav letters.
///
/// A same-letter `h`/`j`/`k`/`l` burst is dropped by the held-nav backlog,
/// so `keys("vs default")` can lose the `l`.
pub fn type_palette_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(VISUAL_KEY_GAP_MS);
        }
    }
}

/// The compare tab shows the regions.txt diff with the diff pane focused.
pub fn compare_regions_diff_focused(screen: &str) -> bool {
    screen.contains("app ↔ origin/main")
        && screen.contains("COMMITTED")
        && screen.contains(REGIONS_ALPHA)
        && screen.contains(REGIONS_OMEGA)
        && panes_files_unfocused_diff_focused(screen)
        && !screen.contains("Enter run")
}

/// Open `app ↔ origin/main` by real input on
/// `seed::compare_regions_workspace` and focus the regions.txt diff.
///
/// `/app`, `Ctrl-k`, "vs default", Enter opens the tab on regions.txt (the
/// first file). A second Enter focuses the compare diff.
pub fn open_compare_regions_diff(workspace: &Path) -> PtySession {
    let mut tui = PtySession::open(workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    open_compare_regions_in(&mut tui);
    tui
}

/// Open `app ↔ origin/main` in a running session and focus its
/// regions.txt diff, as [`open_compare_regions_diff`] does at launch.
///
/// The Workspace tree must hold focus on a row of `app` (the repo row or
/// one of its files): "Diff vs default in new tab" compares the focused checkout.
pub fn open_compare_regions_in(tui: &mut PtySession) {
    tui.ctrl_letter('k');
    tui.wait_pred(
        |screen| screen.contains("Enter run"),
        "Ctrl-k opens the command palette",
        WAIT,
    );
    type_palette_filter(tui, "vs default");
    tui.wait_pred(
        |screen| screen.contains("Diff vs default in new tab") && screen.contains("vs default"),
        "palette filter lands on Diff vs default in new tab",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            screen.contains("app ↔ origin/main")
                && screen.contains("regions.txt")
                && screen.contains(REGIONS_ALPHA)
        },
        "Diff vs default in new tab opens the compare tab on regions.txt",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        compare_regions_diff_focused,
        "Enter focuses the compare regions.txt diff",
        WAIT,
    );
}

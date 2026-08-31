//! Shared WAIT, tree, and crumb helpers for leftover TTY e2e.

use std::fs;
use std::path::Path;
use std::time::Duration;

use crate::harness::{left_tree, PtySession};

pub const WAIT: Duration = Duration::from_secs(12);

pub const GIT_WAIT: Duration = Duration::from_secs(20);

pub const SETTLE_MS: u64 = 200;

pub fn tree_line_containing(screen: &str, needle: &str) -> Option<String> {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(needle))
        .map(str::to_string)
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

/// Breadcrumb is the workspace basename only (file-focused; no repo crumb).
pub fn launch_breadcrumb_workspace_only(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(crumb) = lines.get(lines.len().saturating_sub(2)) else {
        return false;
    };
    crumb.trim() == "workspace"
}

/// Idle status: directory-tree + preferred split pills, help, file hints.
pub fn launch_status_chrome(screen: &str) -> bool {
    let Some(status) = screen.lines().last() else {
        return false;
    };
    status.contains(" tree")
        && status.contains(" split")
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

/// Left tree focused, right diff unfocused (title padding).
pub fn launch_panes_left_tree_right_diff(screen: &str) -> bool {
    let Some(top) = screen.lines().next() else {
        return false;
    };
    top.contains(" tree ") && top.contains(" diff") && !top.contains(" diff ")
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
        && launch_breadcrumb_workspace_only(screen)
        && launch_status_chrome(screen)
        && !screen.contains("[workspace]")
        && !screen.contains("workspace ›")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("Working tree")
        && !screen.contains("focus a repo for the graph")
        && !screen.contains("No matching rows")
        && !screen.contains("loading")
}

/// First paint row: pane titles (focused titles pad both sides).
pub fn pane_top(screen: &str) -> &str {
    screen.lines().next().unwrap_or("")
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

/// Left tree focused, right graph unfocused. Not files / not a file diff.
pub fn panes_tree_focused_graph_unfocused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" tree ")
        && top.contains(" graph")
        && !top.contains(" graph ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

/// Left tree unfocused, right graph focused. Not files / not a file diff.
pub fn panes_tree_unfocused_graph_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" graph ")
        && top.contains(" tree")
        && !top.contains(" tree ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

/// Merger graph body. Files drill (`wip.txt` / `┌ files`) cannot pass.
pub fn merger_graph_body(screen: &str) -> bool {
    screen.contains("WIP on graph")
        && screen.contains("stash@{0}")
        && screen.contains("feature/graph")
        && screen.contains("working tree clean")
        && !screen.contains("wip.txt")
        && !screen.contains("┌ files")
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
        && !status.contains("Esc")
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
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && crumb.contains("workspace › [merger]")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !status.contains("focus right")
        && merger_graph_body(screen)
        && !still_file_diff(screen)
        && !screen.contains("SEARCH")
}

/// Wrong keys: Enter drills files, `/` types SEARCH, Shift+S opens stash.
pub fn not_files_search_or_stash(screen: &str) -> bool {
    !screen.contains("┌ files")
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
        && tree_cursor_on(screen, "focusbox")
        && crumb.contains("workspace › [focusbox]")
        && !crumb.contains("graph focus:")
        && !crumb.contains("full graph")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
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
    panes_tree_unfocused_graph_focused(screen)
        && screen.contains("Focus branches")
        && screen.contains("filter:")
        && screen.contains("* feature/keep")
        && screen.contains("topic/noise")
        && screen.contains("Enter apply")
        && screen.contains("O clear")
        && screen.contains("Esc cancel")
        && screen.contains("workspace › [focusbox]")
        && !screen.contains("graph focus:")
        && !screen.contains("drill")
        && not_files_search_or_stash(screen)
}

/// Overlay filter `feature`: cursor on `feature/keep`. Not `main`.
///
/// Overlay `j`/`k` move the cursor, so a query that starts with `k` is
/// not the filter text. `feature` is unique to `feature/keep`.
pub fn graph_focus_overlay_filtered_keep(screen: &str) -> bool {
    graph_focus_overlay_open(screen)
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
        && status.contains("back")
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
        && crumb_row(screen).trim() == "workspace"
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
    let top = pane_top(screen);
    top.contains(" tree ")
        && top.contains(" graph")
        && !top.contains(" graph ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

/// Left tree unfocused, right graph focused. Not files / not a file diff.
pub fn graph_pane_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" graph ")
        && top.contains(" tree")
        && !top.contains(" tree ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

pub fn graph_cursor_on(screen: &str, needle: &str) -> bool {
    screen.lines().any(|line| {
        let right = right_of_split(line);
        right.contains('\u{258C}') && right.contains(needle)
    })
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

/// Right-pane cells, excluding top/bottom chrome (same rows as [`left_tree`]).
pub fn right_pane(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    let start = usize::from(end > 1);
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
        && tree_cursor_on(screen, "merger")
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
        && tree_cursor_on(screen, "merger")
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
        && status.contains(" split")
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

/// Graph stash row hints. Status chips, not overlay `a apply`.
pub fn has_graph_stash_hints(screen: &str) -> bool {
    let status = status_row(screen);
    status.contains("apply stash") && status.contains("drop stash") && status.contains("pop stash")
}

/// CSI-u Shift+S opened the create-only overlay on the dirty README.
pub fn stash_create_overlay_open(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && screen.contains("Stash app")
        && screen.contains("s create")
        && screen.contains("Esc cancel")
        && !screen.contains("a apply")
        && !screen.contains("p pop")
        && !screen.contains("d drop")
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
        && !screen.contains("s create")
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
        && tree_cursor_on(screen, "app")
        && graph_cursor_on(screen, "working tree clean")
        && !graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && crumb_row(screen).contains("[app]")
        && no_stash_wrong_ops(screen)
}

/// `j` landed on the app stash row. Graph `a` / `D` hints. Not merger.
pub fn app_graph_stash_row_focused(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "app")
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

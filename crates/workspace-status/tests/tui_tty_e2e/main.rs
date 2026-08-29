//! Real-TTY e2e for the ratatui TUI.
//!
//! Spawns the `workspace-status` binary on a PTY so the live loop's
//! `event::read` sees keys and xterm SGR mouse bytes. This is not the
//! TestBackend suite (`tui_headless_e2e.rs`) and not screenshot capture
//! (`scripts/capture-demo-stills.sh`).
//!
//! Unix only (PTY). Windows `cargo test --workspace` compiles this crate
//! with no tests.

#[cfg(unix)]
#[path = "../common/mod.rs"]
mod common;
#[cfg(unix)]
mod desktop;
#[cfg(unix)]
mod harness;
#[cfg(unix)]
mod operator;
#[cfg(unix)]
mod seed;

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use harness::{assert_contains, left_tree, tree_is_panned_to_tail, PtySession, COLS};
#[cfg(unix)]
use seed::{daily_workspace, focus_workspace, unfetched_behind_workspace};

#[cfg(unix)]
const WAIT: Duration = Duration::from_secs(12);
#[cfg(unix)]
const GIT_WAIT: Duration = Duration::from_secs(20);
#[cfg(unix)]
const SETTLE_MS: u64 = 200;

#[cfg(unix)]
fn tree_line_containing(screen: &str, needle: &str) -> Option<String> {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(needle))
        .map(str::to_string)
}

/// Cells after `README.md` on the left-tree file row (trailing chrome).
#[cfg(unix)]
fn after_readme_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "README.md")?;
    let at = line.find("README.md")?;
    Some(line[at + "README.md".len()..].to_string())
}

/// Trailing ASCII reviewed `*` after `README.md` on the left-tree file row.
///
/// The glyph is right-aligned before the status badge. A `*` elsewhere on
/// the row (or a full-screen substring) must not pass. `UNSTAGED` contains
/// the letters `STAGED`, so the badge on this row is the stage oracle.
#[cfg(unix)]
fn readme_row_reviewed(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| after.contains('*') && after.contains('M') && !after.contains('S'))
}

#[cfg(unix)]
fn readme_row_unreviewed_unstaged(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| !after.contains('*') && after.contains('M') && !after.contains('S'))
}

/// First paint: dirty README focused, no reviewed `*`. Not a repo row.
#[cfg(unix)]
fn idle_dirty_readme_unreviewed(screen: &str) -> bool {
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
#[cfg(unix)]
fn documented_space_reviewed(screen: &str) -> bool {
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

#[cfg(unix)]
fn op_finished(screen: &str, verb: &str) -> bool {
    screen.contains(&format!("{verb} 1 repo")) && !screen.contains("failed")
}

#[cfg(unix)]
fn tree_cleared_ahead_behind(screen: &str) -> bool {
    let left = left_tree(screen);
    !left.contains("v1") && !left.contains("^1")
}

#[cfg(unix)]
fn tree_has(screen: &str, needle: &str) -> bool {
    left_tree(screen).contains(needle)
}

/// Left-tree cursor bar (`▌`) on the row that contains `needle`.
#[cfg(unix)]
fn tree_cursor_on(screen: &str, needle: &str) -> bool {
    tree_line_containing(screen, needle).is_some_and(|line| line.contains('\u{258C}'))
}

/// Breadcrumb is the workspace basename only (file-focused; no repo crumb).
#[cfg(unix)]
fn launch_breadcrumb_workspace_only(screen: &str) -> bool {
    let lines: Vec<&str> = screen.lines().collect();
    let Some(crumb) = lines.get(lines.len().saturating_sub(2)) else {
        return false;
    };
    crumb.trim() == "workspace"
}

/// Idle status: directory-tree + preferred split pills, help, file hints.
#[cfg(unix)]
fn launch_status_chrome(screen: &str) -> bool {
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
#[cfg(unix)]
fn launch_panes_left_tree_right_diff(screen: &str) -> bool {
    let Some(top) = screen.lines().next() else {
        return false;
    };
    top.contains(" tree ") && top.contains(" diff") && !top.contains(" diff ")
}

/// Documented first paint on the daily seed. A blank, graph-first, ignored-
/// shown, unfolded No-updates, or paint-changed-only frame cannot pass.
#[cfg(unix)]
fn documented_launch_first_paint(screen: &str) -> bool {
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

/// Spawn paints the documented first chrome. No keys.
///
/// Docs: left tree focused, first file selected, file diff on the right,
/// ignored repos hidden, No updates folded, breadcrumb is the workspace
/// basename while the right pane is a diff. Right-pane git is a worker, so
/// a tree-only frame or a `+dirty` substring is not enough. A no-op, a
/// blank screen, a graph-first launch, or a paint-changed-only assert
/// cannot pass.
#[cfg(unix)]
#[test]
fn pty_launch_paints_tree_diff_and_chrome() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "documented first paint: focused tree, README cursor, file diff, breadcrumb, status, seed rows",
        WAIT,
    );
}

/// Compact painted help text so wrapped description fragments rejoin.
#[cfg(unix)]
fn help_compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Documented `?` overlay rows (`tui/help.rs` MOVE / GIT / VIEW).
///
/// Independent of `HELP_GROUPS` so a wrong overlay still fails.
#[cfg(unix)]
const HELP_MOVE_ROWS: &[(&str, &str)] = &[
    ("j k", "down / up"),
    ("h l", "fold · pan lists/diff · Shift+←→ tree"),
    ("z", "toggle fold (instant; no-op on graph/diff)"),
    ("zz", "toggle subtree (no-op on graph/diff)"),
    ("gg G", "top / bottom of focused pane"),
    ("Home End", "top / bottom"),
    ("/", "search focused pane (Enter arms)"),
    ("n N", "next / prev match (after Enter)"),
];
#[cfg(unix)]
const HELP_GIT_ROWS: &[(&str, &str)] = &[
    ("s", "stage scope"),
    ("S", "stash menu"),
    ("u", "unstage scope"),
    ("x", "revert (y/Y)"),
    ("e", "open in editor"),
    ("space", "mark dirty file reviewed (eye)"),
    ("f", "fetch remotes"),
    ("p", "pull behind"),
    ("P", "push ahead/diverged/new"),
    ("d", "default branch"),
    ("b", "depth 0 picker · graph local/origin/*"),
    ("m", "graph merge into HEAD"),
    ("C", "create (in picker)"),
    ("W", "remove linked worktree"),
    ("r", "refresh now"),
    ("a p D", "focused stash apply/pop/drop"),
];
#[cfg(unix)]
const HELP_VIEW_ROWS: &[(&str, &str)] = &[
    ("i", "inline / split"),
    ("t", "flat / tree"),
    (".", "show / hide ignored repos"),
    ("T", "cycle theme"),
    ("Ctrl-o", "full-file · keep hunk in view"),
    ("o O", "graph focus branches / clear"),
    ("PgUp PgDn", "page focused pane"),
    ("Ctrl-u Ctrl-d", "page focused ±5"),
    ("m", "mouse · drag pane, split, or graph scrollbars"),
    ("Esc", "back / unfocus · never quit"),
    ("Enter dblclick", "focus right / drill"),
    ("?", "this help"),
    ("Tab", "other pane"),
    ("q", "quit"),
    ("Ctrl-C Ctrl-C", "quit (press twice)"),
];

/// Split the painted overlay into MOVE / GIT / VIEW columns.
///
/// Inner width and column width follow `help_inner_width` /
/// `help_column_width` at the PTY default. Footer is excluded so
/// `/ search help` does not leak into the keymap columns.
#[cfg(unix)]
fn help_group_columns(screen: &str) -> Option<[String; 3]> {
    let lines: Vec<&str> = screen.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains("MOVE") && line.contains("GIT") && line.contains("VIEW"))?;
    let inner_w = (COLS as usize).saturating_sub(4);
    let col_w = inner_w / 3;
    let mut cols = [String::new(), String::new(), String::new()];
    for line in &lines[start..] {
        if line.contains("/ search help") || line.contains("Esc closes") {
            break;
        }
        let chars: Vec<char> = line.chars().collect();
        if chars.len() < 2 + col_w {
            continue;
        }
        let inner: Vec<char> = chars.into_iter().skip(2).take(inner_w).collect();
        for (idx, col) in cols.iter_mut().enumerate() {
            let from = idx * col_w;
            let to = (from + col_w).min(inner.len());
            if from < inner.len() {
                col.extend(inner[from..to].iter());
                col.push('\n');
            }
        }
    }
    Some(cols)
}

#[cfg(unix)]
fn help_column_has_row(column: &str, keys: &str, desc: &str) -> bool {
    let compact = help_compact(column);
    compact.contains(&help_compact(keys)) && compact.contains(&help_compact(desc))
}

#[cfg(unix)]
fn help_version_lower_right(screen: &str) -> bool {
    let version = workspace_status::APP_VERSION;
    let Some(line) = screen.lines().rev().find(|line| line.contains(version)) else {
        return false;
    };
    let Some(idx) = line.rfind(version) else {
        return false;
    };
    line[idx + version.len()..]
        .chars()
        .all(|c| c.is_whitespace() || matches!(c, '│' | '╯' | '╮' | '┘' | '┐' | '║' | '┤'))
}

/// Full documented overlay: groups, key rows, footer, version.
///
/// A no-op (`? help` chrome only), MOVE/GIT/VIEW titles without rows, or
/// a clipped last GIT wrap must fail.
#[cfg(unix)]
fn documented_help_overlay(screen: &str) -> bool {
    let Some(header) = screen
        .lines()
        .find(|line| line.contains("MOVE") && line.contains("GIT") && line.contains("VIEW"))
    else {
        return false;
    };
    let move_at = match header.find("MOVE") {
        Some(idx) => idx,
        None => return false,
    };
    let git_at = match header.find("GIT") {
        Some(idx) => idx,
        None => return false,
    };
    let view_at = match header.find("VIEW") {
        Some(idx) => idx,
        None => return false,
    };
    if !(move_at < git_at && git_at < view_at) {
        return false;
    }
    let Some([move_col, git_col, view_col]) = help_group_columns(screen) else {
        return false;
    };
    let move_c = help_compact(&move_col);
    let git_c = help_compact(&git_col);
    let view_c = help_compact(&view_col);
    if !move_c.contains("MOVE") || move_c.contains("GIT") || move_c.contains("VIEW") {
        return false;
    }
    if !git_c.contains("GIT") || git_c.contains("MOVE") || git_c.contains("VIEW") {
        return false;
    }
    if !view_c.contains("VIEW") || view_c.contains("MOVE") || view_c.contains("GIT") {
        return false;
    }
    if HELP_MOVE_ROWS
        .iter()
        .any(|(keys, desc)| !help_column_has_row(&move_col, keys, desc))
        || HELP_GIT_ROWS
            .iter()
            .any(|(keys, desc)| !help_column_has_row(&git_col, keys, desc))
        || HELP_VIEW_ROWS
            .iter()
            .any(|(keys, desc)| !help_column_has_row(&view_col, keys, desc))
    {
        return false;
    }
    // Group membership: page keys stay VIEW; git writes stay GIT.
    if move_c.contains("page focused pane")
        || move_c.contains("stage scope")
        || move_c.contains("cycle theme")
        || git_c.contains("down / up")
        || git_c.contains("cycle theme")
        || view_c.contains("stage scope")
        || view_c.contains("down / up")
        || view_c.contains("open in editor")
    {
        return false;
    }
    screen.contains("/ search help")
        && screen.contains("Esc closes")
        && !screen.contains("? help")
        && help_version_lower_right(screen)
}

/// `?` paints the documented MOVE / GIT / VIEW overlay on a live TTY.
///
/// Fail if `?` is a no-op, if the groups are wrong, or if a documented
/// key row is missing (including a clipped last GIT wrap). Help `/`
/// search and Enter-arm stay on `pty_help_enter_does_not_arm_pane_search`.
#[cfg(unix)]
#[test]
fn pty_help_overlay() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("? help")
                && screen.contains("README.md")
                && !screen.contains("MOVE")
                && !screen.contains("open in editor")
                && !screen.contains("/ search help")
        },
        "idle chrome shows ? help and the overlay is closed",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        documented_help_overlay,
        "documented MOVE / GIT / VIEW overlay (groups, key rows, footer, version)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("open in editor")
                && !screen.contains("/ search help")
                && screen.contains("? help")
                && screen.contains("README.md")
        },
        "Esc closes help and restores idle ? help chrome",
        WAIT,
    );
}

/// First paint row: pane titles (focused titles pad both sides).
#[cfg(unix)]
fn pane_top(screen: &str) -> &str {
    screen.lines().next().unwrap_or("")
}

/// Breadcrumb sits on the penultimate row (status is last).
#[cfg(unix)]
fn crumb_line(screen: &str) -> &str {
    let lines: Vec<&str> = screen.lines().collect();
    lines
        .get(lines.len().saturating_sub(2))
        .copied()
        .unwrap_or("")
}

#[cfg(unix)]
fn status_line(screen: &str) -> &str {
    screen.lines().last().unwrap_or("")
}

/// Left tree focused, right graph unfocused. Not files / not a file diff.
#[cfg(unix)]
fn panes_tree_focused_graph_unfocused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" tree ")
        && top.contains(" graph")
        && !top.contains(" graph ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

/// Left tree unfocused, right graph focused. Not files / not a file diff.
#[cfg(unix)]
fn panes_tree_unfocused_graph_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" graph ")
        && top.contains(" tree")
        && !top.contains(" tree ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

/// Merger graph body. Files drill (`wip.txt` / `┌ files`) cannot pass.
#[cfg(unix)]
fn merger_graph_body(screen: &str) -> bool {
    screen.contains("WIP on graph")
        && screen.contains("stash@{0}")
        && screen.contains("feature/graph")
        && screen.contains("working tree clean")
        && !screen.contains("wip.txt")
        && !screen.contains("┌ files")
        && !screen.contains("[stash@{0}]")
}

/// File-diff chrome from the launch README row. Must be gone after drill.
#[cfg(unix)]
fn still_file_diff(screen: &str) -> bool {
    screen.contains("app/README.md")
        || screen.contains("UNSTAGED")
        || screen.contains("@@ -1 +1,2 @@")
        || screen.contains("+dirty")
        || launch_panes_left_tree_right_diff(screen)
}

/// Left-focused merger row with its graph loaded. Enter has not run.
#[cfg(unix)]
fn merger_graph_left_unfocused(screen: &str) -> bool {
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
#[cfg(unix)]
fn merger_graph_drilled_right(screen: &str) -> bool {
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

/// Enter on a graph-capable tree row focuses the graph. Esc pops back.
///
/// Docs + VIEW: Enter is `focus right / drill`; Esc is `back / unfocus`
/// and never quits. Launch is the README file diff. `j` moves onto
/// `merger` (the graph-capable row). Enter must paint the focused graph
/// for that repo, not keep the file diff and not push commit files.
/// Esc is CSI-u (`CSI 27 u`). It must restore the left tree and the
/// merger row. A no-op, a screen-delta-only check, `/` search, Tab, or
/// `o`/`O` cannot pass.
#[cfg(unix)]
#[test]
fn pty_graph_drill_enter_esc() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff (graph drill has not run)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph (left focus, not yet Enter)",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        merger_graph_drilled_right,
        "Enter on merger focuses that graph (file-diff / files drill / no-op cannot pass)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        merger_graph_left_unfocused,
        "CSI-u Esc restores the left tree and the merger row",
        WAIT,
    );
}

/// Wrong keys: Enter drills files, `/` types SEARCH, Shift+S opens stash.
#[cfg(unix)]
fn not_files_search_or_stash(screen: &str) -> bool {
    !screen.contains("┌ files")
        && !screen.contains("keep.txt")
        && !screen.contains("SEARCH")
        && !screen.contains("Stash ")
}

/// `--all` graph for focusbox. Keep, main, and noise tips are all visible.
#[cfg(unix)]
fn focusbox_full_graph_body(screen: &str) -> bool {
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
#[cfg(unix)]
fn focusbox_keep_only_graph_body(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("focus-root-commit")
        && screen.contains("[+feature/keep]")
        && !screen.contains("noise-leaf-commit")
        && !screen.contains("main-leaf-commit")
        && !screen.contains("[topic/noise]")
        && !screen.contains("[main]")
}

/// Graph loaded, left focus, `--all`. Tab / `o` have not run.
#[cfg(unix)]
fn focusbox_graph_left_full(screen: &str) -> bool {
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
#[cfg(unix)]
fn focusbox_graph_right_full(screen: &str) -> bool {
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
#[cfg(unix)]
fn graph_focus_overlay_open(screen: &str) -> bool {
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
#[cfg(unix)]
fn graph_focus_overlay_filtered_keep(screen: &str) -> bool {
    graph_focus_overlay_open(screen)
        && screen.contains("filter: feature")
        && screen
            .lines()
            .any(|line| line.contains('❯') && line.contains("feature/keep"))
        && !screen.contains("[ ]   main")
}

/// Applied keep focus: toast, `O` clear-focus hint, keep-only graph.
#[cfg(unix)]
fn graph_focus_applied_keep(screen: &str) -> bool {
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
#[cfg(unix)]
fn graph_focus_cleared_full(screen: &str) -> bool {
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
#[cfg(unix)]
fn apply_current_keep_graph_focus(tui: &mut PtySession) {
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

/// Graph `o` / `O`: overlay, filter-apply `feature`, CSI-u Shift+O clears.
///
/// Docs + VIEW: `o` opens the local-branch overlay. Type to filter. Enter
/// with no marks applies the cursor row. While focus is on, the graph is
/// ancestors of those tips. `O` restores `--all`. Shift+O is CSI-u
/// (`CSI 111 ; 2 : 1 u` press, `: 3` release), not a raw `'O'` byte. A
/// no-op, an Enter files drill, `/` SEARCH, overlay Enter on `main`, or
/// another Shift binding (stash / theme / `G`) cannot pass.
/// Unmark-then-Enter stays on `pty_graph_focus_unmark_enter_clears`.
#[cfg(unix)]
#[test]
fn pty_graph_branch_focus_overlay() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    apply_current_keep_graph_focus(&mut tui);

    tui.shift_letter('O');
    tui.wait_pred(
        graph_focus_cleared_full,
        "CSI-u Shift+O restores the full graph and drops the clear-focus hint",
        GIT_WAIT,
    );
}

/// Painted SEARCH status line with this query and the typing cursor.
///
/// The capital glyphs must sit on that line. A lowercase type-in, an
/// armed `/{query}` chip, or a global Shift binding cannot pass.
#[cfg(unix)]
fn search_prompt_has_query(screen: &str, query: &str) -> bool {
    let Some(status) = screen.lines().last() else {
        return false;
    };
    status.contains("SEARCH")
        && status.contains(&format!("{query}▏"))
        && status.contains("Enter arms query")
        && status.contains("Esc clears")
        && status.contains("n/N after Enter")
        && (query.is_empty() || !status.contains(&format!("/{query}")))
}

/// Global Shift+letter bindings that must not fire while `/` is typing.
#[cfg(unix)]
fn search_did_not_fire_global_shift(screen: &str) -> bool {
    !screen.contains("Stash ")
        && !screen.contains("theme: Monokai")
        && !screen.contains("theme: Dracula")
        && !screen.contains("theme: Gruvbox")
        && !screen.contains("theme: Catppuccin")
        && !screen.contains("Flat paths")
        && !screen.contains("Focus branches")
        && !screen.contains("full graph")
        && !tree_cursor_on(screen, "No updates")
}

/// CSI-u Shift+letters in an armed `/` query type capitals.
///
/// Docs + help: `/` search, characters append while typing. Global
/// Shift+O clears graph focus, Shift+S opens stash, Shift+G jumps to
/// the last row, Shift+T cycles theme. Those must not fire. Raw `'O'`
/// is a different path. Enter-arm / pane search stay on other tests.
#[cfg(unix)]
#[test]
fn pty_shift_letters_csi_u_type_into_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("? help")
                && tree_cursor_on(screen, "README.md")
                && !screen.contains("SEARCH")
                && !screen.contains("MOVE")
        },
        "idle chrome before /; SEARCH is closed",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && screen.contains("search focused pane")
                && screen.contains("stash menu")
                && screen.contains("top / bottom")
                && screen.contains("cycle theme")
                && screen.contains("focus branches")
        },
        "help lists / search and the global Shift+O/S/G/T bindings",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("search focused pane")
                && screen.contains("? help")
                && tree_cursor_on(screen, "README.md")
        },
        "Esc closes help so Shift+letters go to pane search, not help",
        WAIT,
    );

    tui.key('/');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("O▏")
                && tree_cursor_on(screen, "README.md")
        },
        "/ opens SEARCH; query is empty; Shift bindings have not fired",
        WAIT,
    );

    tui.shift_letter('O');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "O")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("o▏")
        },
        "CSI-u Shift+O types O; it must not clear graph focus",
        WAIT,
    );

    tui.shift_letter('S');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "OS")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("os▏")
        },
        "CSI-u Shift+S types S; it must not open the stash menu",
        WAIT,
    );

    tui.shift_letter('G');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "OSG")
                && search_did_not_fire_global_shift(screen)
                && !tree_cursor_on(screen, "No updates")
        },
        "CSI-u Shift+G types G; it must not jump to the last tree row",
        WAIT,
    );

    tui.shift_letter('T');
    tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "OSGT")
                && search_did_not_fire_global_shift(screen)
                && !screen.contains("/OSGT")
                && !screen.contains("osgt▏")
        },
        "CSI-u Shift+T types T; it must not cycle theme or arm the query",
        WAIT,
    );

    let (_root2, focused) = focus_workspace();
    let mut focused_tui = PtySession::open(&focused);
    focused_tui.wait_contains("focusbox", WAIT);
    focused_tui.tab();
    focused_tui.wait_pred(
        |screen| {
            screen.contains("keep-leaf-commit")
                && screen.contains("noise-leaf-commit")
                && screen.contains("o   focus branches")
        },
        "graph shows both leaves; o opens focus (O clear is hidden until a focus is on)",
        GIT_WAIT,
    );

    focused_tui.key('o');
    focused_tui.wait_contains("Focus branches", GIT_WAIT);
    focused_tui.keys("keep");
    focused_tui.enter();
    focused_tui.wait_pred(
        |screen| {
            !screen.contains("Focus branches")
                && screen.contains("keep-leaf-commit")
                && !screen.contains("noise-leaf-commit")
                && screen.contains("O   clear focus")
                && screen.contains("[+feature/keep]")
        },
        "graph focus applied; O still bound as clear focus",
        GIT_WAIT,
    );

    focused_tui.key('/');
    focused_tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "")
                && screen.contains("keep-leaf-commit")
                && !screen.contains("noise-leaf-commit")
        },
        "/ on the focused graph opens SEARCH; focus filter stays",
        WAIT,
    );
    focused_tui.shift_letter('O');
    focused_tui.wait_pred(
        |screen| {
            search_prompt_has_query(screen, "O")
                && search_did_not_fire_global_shift(screen)
                && screen.contains("keep-leaf-commit")
                && !screen.contains("noise-leaf-commit")
                && screen.contains("[+feature/keep]")
                && !screen.contains("o▏")
        },
        "CSI-u Shift+O types O into SEARCH; it must not restore --all",
        WAIT,
    );
}

/// Shift+S via CSI-u opens the stash overlay (not `s` stage).
#[cfg(unix)]
#[test]
fn pty_shift_s_csi_u_opens_stash_menu() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("README.md", WAIT);
    tui.shift_letter('S');
    tui.wait_contains("Stash ", WAIT);
    tui.wait_contains("stash", WAIT);
}

/// Cursor row in the focus overlay (`❯` plus the mark box).
#[cfg(unix)]
fn graph_focus_overlay_cursor_row(screen: &str) -> Option<&str> {
    screen
        .lines()
        .find(|line| line.contains('❯') && (line.contains("[x]") || line.contains("[ ]")))
}

/// Reopen after a keep focus: current focus is pre-marked on the cursor row.
/// Overlay stays open. Keep-only graph behind must not reload yet.
#[cfg(unix)]
fn graph_focus_keep_row_premarked(screen: &str) -> bool {
    let Some(row) = graph_focus_overlay_cursor_row(screen) else {
        return false;
    };
    graph_focus_overlay_open(screen)
        && screen.contains("space toggle")
        && row.contains("[x]")
        && row.contains("feature/keep")
        && focusbox_keep_only_graph_body(screen)
}

/// Docs: Space clears `[x]` on the focused overlay row. Overlay stays open.
/// The keep-only graph must not reload until Enter.
#[cfg(unix)]
fn graph_focus_keep_row_unmarked(screen: &str) -> bool {
    let Some(row) = graph_focus_overlay_cursor_row(screen) else {
        return false;
    };
    graph_focus_overlay_open(screen)
        && screen.contains("space toggle")
        && row.contains("[ ]")
        && row.contains("feature/keep")
        && !row.contains("[x]")
        && !screen.contains("[x]")
        && focusbox_keep_only_graph_body(screen)
}

/// Unmark every `[x]` then Enter restores `--all`. Must not re-apply the
/// cursor row. Overlay Space/Enter, not `O`. A no-op or `[x]`-gone-only
/// screen delta cannot pass.
#[cfg(unix)]
#[test]
fn pty_graph_focus_unmark_enter_clears() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    apply_current_keep_graph_focus(&mut tui);

    tui.key('o');
    tui.wait_pred(
        graph_focus_keep_row_premarked,
        "reopen pre-marks [x] on the focused keep row; overlay stays; keep-only graph",
        WAIT,
    );
    tui.key(' ');
    tui.wait_pred(
        graph_focus_keep_row_unmarked,
        "Space clears [x] on the focused keep row; overlay stays; graph does not reload",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        graph_focus_cleared_full,
        "Enter after unmark restores --all / full graph (does not re-drill keep)",
        GIT_WAIT,
    );
}

/// Create (`S` then `s`), apply (`a`), drop (`D` then `y`) — not menu-open only.
#[cfg(unix)]
#[test]
fn pty_stash_create_apply_and_drop() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.shift_letter('S');
    tui.wait_contains("s create", WAIT);
    tui.key('s');
    tui.wait_contains("Stashed", GIT_WAIT);
    tui.wait_pred(
        |screen| !left_tree(screen).contains("README.md"),
        "stashed README leaves the dirty tree",
        WAIT,
    );

    tui.esc();
    tui.search("app");
    tui.wait_contains("/app", WAIT);
    tui.tab();
    tui.wait_contains("Working tree", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('/');
    tui.keys("stash@{");
    tui.enter();
    tui.wait_contains("/stash@{", WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('a');
    tui.wait_contains("README.md", GIT_WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.shift_letter('D');
    tui.wait_contains("Drop", WAIT);
    tui.wait_contains("stash@{0}", WAIT);
    tui.key('y');
    tui.wait_contains("dropped stash@{0}", GIT_WAIT);
    tui.wait_contains("README.md", WAIT);
}

#[cfg(target_os = "linux")]
mod xfce {
    use super::*;
    use crate::common::hscroll::TREE_HSCROLL_TAIL;
    use crate::desktop::DesktopSession;
    use crate::harness::{
        assert_tree_clipped_long_path, left_tree, status_has_tree_hscroll_tail,
        tree_cursor_bar_on_row, tree_row_containing,
    };
    use crate::seed::{ahead_workspace, seed_long_path_file};

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_keys_help_and_search() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("shift+slash");
        tui.wait_contains("MOVE", WAIT);
        tui.wait_contains("GIT", WAIT);
        tui.wait_contains("VIEW", WAIT);
        tui.key("Escape");
        tui.key("slash");
        tui.type_text("merger");
        tui.key("Return");
        tui.wait_contains("merger", WAIT);
        tui.wait_contains("WIP on graph", WAIT);
    }

    /// xfce + XTEST Shift keys: search capitals, unmark-then-Enter, Shift+O.
    ///
    /// Overlay toggle is space (`[x]` / `[ ]`), not X. Reopen after `O`
    /// has no pre-mark; unmark-then-Enter runs while a focus is still on.
    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_shift_keys_search_and_clear_focus() {
        let (_root, workspace) = focus_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.key("shift+r");
        tui.key("shift+e");
        tui.key("shift+a");
        tui.key("shift+d");
        tui.key("shift+m");
        tui.key("shift+e");
        tui.wait_contains("README▏", WAIT);
        tui.key("Escape");

        tui.key("slash");
        tui.type_text("focusbox");
        tui.key("Return");
        tui.wait_contains("focusbox", WAIT);
        tui.key("Tab");
        tui.wait_contains("keep-leaf-commit", WAIT);
        tui.wait_contains("noise-leaf-commit", WAIT);
        tui.key("o");
        tui.wait_contains("Focus branches", WAIT);
        tui.type_text("keep");
        tui.key("Return");
        tui.wait_pred(
            |screen| !screen.contains("Focus branches"),
            "focus overlay closed after apply",
            WAIT,
        );
        tui.wait_contains("keep-leaf-commit", WAIT);
        tui.wait_pred(
            |screen| !screen.contains("noise-leaf-commit"),
            "noise-leaf-commit hidden after focus",
            WAIT,
        );

        tui.key("o");
        tui.wait_contains("Focus branches", WAIT);
        tui.wait_contains("[x]", WAIT);
        tui.key("space");
        tui.wait_pred(
            |screen| !screen.contains("[x]"),
            "[x] mark cleared after space",
            WAIT,
        );
        tui.key("Return");
        tui.wait_pred(
            |screen| !screen.contains("Focus branches"),
            "focus overlay closed after empty apply",
            WAIT,
        );
        tui.wait_contains("noise-leaf-commit", WAIT);
        tui.wait_contains("main-leaf-commit", WAIT);

        tui.key("o");
        tui.wait_contains("Focus branches", WAIT);
        tui.type_text("keep");
        tui.key("Return");
        tui.wait_pred(
            |screen| !screen.contains("noise-leaf-commit"),
            "noise-leaf-commit hidden before Shift+O",
            WAIT,
        );
        tui.key("shift+o");
        tui.wait_contains("noise-leaf-commit", WAIT);
    }

    /// XTEST wheel right. Must fail if the tree does not pan.
    ///
    /// XTEST `click 7` (no `--window`) after a root-coordinate warp, in
    /// xterm (VTE 0.76 does not report buttons 6/7). Same clipped-prefix vs
    /// tail tree-row oracle as the PTY case (`common::hscroll`). No `/`
    /// search. Wait for a clipped tree row on the same frame. Click README
    /// so the tree pans, not a long file-diff. Cursor stays on that row.
    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; xterm encodes XTEST button 7"]
    fn desktop_xterm_xtest_trackpad_hscroll() {
        let (_root, workspace) = daily_workspace();
        seed_long_path_file(&workspace);
        let tui = DesktopSession::open_xterm_size(&workspace, 64, 24);
        let _ = tui.wait_clipped_long_path_row(WAIT);

        let readme_hit = tree_row_containing(&tui.screen(), "README.md")
            .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));
        tui.click_cell(6, readme_hit);
        tui.wait_pred(
            |screen| {
                tree_cursor_on(screen, "README.md")
                    && screen.contains("UNSTAGED")
                    && !screen.contains("SEARCH")
            },
            "XTEST click loads a short README diff (not the long path)",
            GIT_WAIT,
        );
        let readme_row = tree_row_containing(&tui.screen(), "README.md")
            .unwrap_or_else(|| panic!("README row before hscroll:\n{}", tui.screen()));
        let row = tui.wait_clipped_long_path_row(WAIT);
        assert_tree_clipped_long_path(&tui.screen());
        tui.wheel_right_at_cell(6, row, 40);
        tui.wait_pred(
            |screen| {
                tree_is_panned_to_tail(screen)
                    && tree_row_containing(screen, TREE_HSCROLL_TAIL).is_some()
                    && tree_cursor_bar_on_row(screen, readme_row)
                    && !status_has_tree_hscroll_tail(screen)
            },
            "tree row shows TAIL99, drops very-long, keeps README cursor",
            WAIT,
        );
        crate::common::hscroll::assert_panned_to_tail(&left_tree(&tui.screen()));
    }

    /// Space reviewed on first-paint dirty README. `s`/`u` stay a separate
    /// claim; this arm only strengthens the Space-reviewed part.
    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_review_and_stage() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.wait_contains("README.md", WAIT);
        tui.wait_contains("UNSTAGED", GIT_WAIT);
        tui.wait_pred(
            idle_dirty_readme_unreviewed,
            "first paint: cursor on dirty README, no reviewed mark",
            WAIT,
        );
        tui.key("space");
        tui.wait_pred(
            documented_space_reviewed,
            "Space paints ASCII `*` on the focused README row; file stays; not staged",
            WAIT,
        );
        tui.key("s");
        tui.wait_contains("STAGED", GIT_WAIT);
        tui.wait_absent("UNSTAGED", WAIT);
        tui.wait_pred(
            |screen| {
                tree_line_containing(screen, "README.md").is_some_and(|line| line.contains("S "))
            },
            "staged README badge `S `",
            WAIT,
        );
        tui.key("u");
        tui.wait_contains("UNSTAGED", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_fetch_then_pull_local_remote() {
        let (_root, workspace) = unfetched_behind_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("syncbox");
        tui.key("Return");
        tui.wait_contains("/syncbox", WAIT);
        tui.wait_contains("Working tree", WAIT);
        tui.wait_contains("fetch", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("f");
        tui.wait_pred(
            |screen| op_finished(screen, "Fetched") || left_tree(screen).contains("v1"),
            "Fetched 1 repo or tree shows behind-by-1",
            GIT_WAIT,
        );
        tui.wait_pred(
            |screen| left_tree(screen).contains("v1"),
            "tree shows behind-by-1 after fetch",
            WAIT,
        );
        tui.wait_contains("origin-tip-commit", WAIT);
        tui.wait_contains("pull", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("p");
        tui.wait_pred(
            |screen| op_finished(screen, "Pulled"),
            "Pulled 1 repo without failure",
            GIT_WAIT,
        );
        tui.wait_pred(
            tree_cleared_ahead_behind,
            "behind mark cleared after pull",
            WAIT,
        );
        tui.key("Tab");
        tui.wait_contains("origin-tip-commit", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_shift_p_pushes_ahead() {
        let (_root, workspace) = ahead_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("syncbox");
        tui.key("Return");
        tui.wait_contains("/syncbox", WAIT);
        tui.wait_contains("ahead-tip-commit", WAIT);
        tui.wait_pred(
            |screen| left_tree(screen).contains("^1"),
            "tree shows ahead-by-1 before push",
            WAIT,
        );
        tui.wait_contains("push", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("shift+p");
        tui.wait_pred(
            |screen| op_finished(screen, "Pushed"),
            "Pushed 1 repo without failure",
            GIT_WAIT,
        );
        tui.wait_pred(
            tree_cleared_ahead_behind,
            "ahead mark cleared after push",
            WAIT,
        );
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_graph_merge_creates_commit() {
        let (_root, workspace) = focus_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("focusbox");
        tui.key("Return");
        tui.wait_contains("/focusbox", WAIT);
        tui.key("Tab");
        tui.wait_contains("keep-leaf-commit", WAIT);
        tui.wait_contains("main-leaf-commit", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("slash");
        tui.type_text("main-leaf-commit");
        tui.key("Return");
        tui.wait_contains("/main-leaf-commit", WAIT);
        tui.wait_contains("merge", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("m");
        tui.wait_contains("Merge", WAIT);
        tui.wait_contains("into", WAIT);
        tui.key("y");
        tui.wait_contains("Merge branch 'main'", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_stash_create_apply_and_drop() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.type_text("README");
        tui.key("Return");
        tui.wait_contains("/README", WAIT);
        tui.wait_contains("UNSTAGED", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("shift+s");
        tui.wait_contains("s create", WAIT);
        tui.key("s");
        tui.wait_contains("Stashed", GIT_WAIT);
        tui.wait_pred(
            |screen| !left_tree(screen).contains("README.md"),
            "stashed README leaves the dirty tree",
            WAIT,
        );
        tui.key("Escape");
        tui.key("slash");
        tui.type_text("app");
        tui.key("Return");
        tui.wait_contains("/app", WAIT);
        tui.key("Tab");
        tui.wait_contains("Working tree", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("slash");
        tui.type_text("stash@{");
        tui.key("Return");
        tui.wait_contains("/stash@{", WAIT);
        tui.wait_contains("stash@{0}", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("a");
        tui.wait_contains("README.md", GIT_WAIT);
        tui.wait_contains("stash@{0}", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("shift+d");
        tui.wait_contains("Drop", WAIT);
        tui.key("y");
        tui.wait_contains("dropped stash@{0}", GIT_WAIT);
    }

    #[test]
    #[ignore = "GitHub Actions tui-tty-desktop job; needs DISPLAY, xfce4-terminal, xdotool"]
    fn desktop_xfce_stash_graph_pop() {
        let (_root, workspace) = daily_workspace();
        let tui = DesktopSession::open(&workspace);
        tui.key("slash");
        tui.wait_contains("SEARCH", WAIT);
        tui.type_text("merger");
        // Typing jumps to merger and starts graph git. Wait for that paint
        // before Return: xfce can drop Enter while the pane worker runs.
        tui.wait_pred(
            |screen| {
                screen.contains("SEARCH")
                    && screen.contains("Enter arms query")
                    && screen.contains("merger▏")
                    && screen.contains("WIP on graph")
                    && !screen.contains("/merger")
            },
            "SEARCH typing on merger after the graph jump",
            GIT_WAIT,
        );
        tui.key("Return");
        tui.wait_pred(
            |screen| screen.contains("/merger") && !screen.contains("SEARCH"),
            "Enter arms /merger; SEARCH closes",
            WAIT,
        );
        tui.key("Tab");
        tui.wait_contains("WIP on graph", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("slash");
        tui.wait_contains("SEARCH", WAIT);
        tui.type_text("stash@{");
        tui.wait_pred(
            |screen| {
                screen.contains("SEARCH")
                    && screen.contains("Enter arms query")
                    && screen.contains("stash@{▏")
            },
            "SEARCH typing stash@{",
            WAIT,
        );
        tui.key("Return");
        tui.wait_pred(
            |screen| screen.contains("/stash@{") && !screen.contains("SEARCH"),
            "Enter arms /stash@{",
            WAIT,
        );
        tui.wait_contains("stash@{0}", WAIT);
        tui.wait_ms(SETTLE_MS);
        tui.key("p");
        tui.wait_contains("wip.txt", GIT_WAIT);
        tui.wait_contains("popped stash@{0}", GIT_WAIT);
    }
}

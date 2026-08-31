use std::fs;
use std::path::PathBuf;

use crate::harness::{PtySession, SGR_WHEEL_DOWN};
use crate::seed::{seed_many_commit_files, seed_repo, seed_tall_graph, unique_root};
use crate::support::{
    graph_cursor_on, graph_pane_focused, pane_top, right_pane, tree_cursor_on, GIT_WAIT,
    RIGHT_PANE_COL, SETTLE_MS, WAIT,
};

/// Wheel down + 1003 motion bit (`65 | 32`). crossterm 0.28 drops this.
const SGR_WHEEL_DOWN_MOTION: u8 = 65 | 32;

/// Gap so the input thread does not drain a burst of `j` as one move.
const KEY_GAP_MS: u64 = 50;

/// Past a default pane midpoint (~13) and short of the last row.
const STEPS: usize = 22;

fn j_steps(tui: &mut PtySession, n: usize) {
    for _ in 0..n {
        tui.key('j');
        tui.wait_ms(KEY_GAP_MS);
    }
}

fn tree_keyboard_focus(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" tree ")
        && !top.contains(" graph ")
        && !top.contains(" files ")
        && !top.contains(" diff ")
}

fn return_to_tree(tui: &mut PtySession) {
    tui.esc();
    tui.wait_ms(120);
    tui.esc();
    tui.wait_pred(
        tree_keyboard_focus,
        "Esc returns keyboard focus to the workspace tree (search clear, then unfocus)",
        WAIT,
    );
}

fn keep_middle_workspace() -> (PathBuf, PathBuf) {
    let root = unique_root("ws-tui-tty-keep-mid");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    let mut body = String::new();
    for i in 0..40 {
        body.push_str(&format!("keepmid-line-{i}\n"));
    }
    fs::write(workspace.join("app").join("keepmid-diff.rs"), body).unwrap();
    seed_tall_graph(&workspace, "history");
    seed_many_commit_files(&workspace, "bundle", 40);
    (root, workspace)
}

fn help_lists_jk_down_up(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("MOVE") && compact.contains("j k") && compact.contains("down / up")
}

fn right_cursor_index(screen: &str) -> Option<(usize, usize)> {
    let right = right_pane(screen);
    let lines: Vec<&str> = right.lines().collect();
    let n = lines.len();
    let idx = lines.iter().position(|line| line.contains('\u{258C}'))?;
    Some((idx, n))
}

/// Focused right-pane row sits near the vertical middle, not the first or last line.
fn right_focus_near_middle(screen: &str) -> bool {
    let Some((idx, n)) = right_cursor_index(screen) else {
        return false;
    };
    if n < 8 {
        return false;
    }
    let mid = n / 2;
    let slack = (n / 4).max(3);
    idx >= 2 && idx + 2 < n && idx.abs_diff(mid) <= slack
}

fn panes_diff_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" diff ")
        && top.contains(" tree")
        && !top.contains(" tree ")
        && !top.contains(" graph")
        && !top.contains(" files")
}

fn panes_files_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" files ")
        && top.contains(" graph")
        && !top.contains(" graph ")
        && !top.contains(" diff")
}

fn diff_launch_top(screen: &str) -> bool {
    panes_diff_focused(screen)
        && tree_cursor_on(screen, "keepmid-diff.rs")
        && right_pane(screen).contains("keepmid-line-0")
        && right_pane(screen).contains('\u{258C}')
}

fn diff_kept_middle(screen: &str) -> bool {
    panes_diff_focused(screen)
        && right_focus_near_middle(screen)
        && right_pane(screen).contains("keepmid-line-")
        && !right_pane(screen).contains("keepmid-line-0")
}

fn graph_launch_top(screen: &str) -> bool {
    graph_pane_focused(screen)
        && graph_cursor_on(screen, "working tree")
        && right_pane(screen).contains("count 29")
        && !graph_cursor_on(screen, "count 0")
}

fn graph_kept_middle(screen: &str) -> bool {
    graph_pane_focused(screen)
        && right_focus_near_middle(screen)
        && right_pane(screen).contains("count ")
        && !right_pane(screen).contains("working tree")
        && !graph_cursor_on(screen, "working tree")
        && !graph_cursor_on(screen, "count 29")
}

fn files_launch_top(screen: &str) -> bool {
    panes_files_focused(screen)
        && screen.contains("┌ files")
        && right_pane(screen).contains("keepmid-00.txt")
        && right_pane(screen).contains('\u{258C}')
}

fn files_kept_middle(screen: &str) -> bool {
    panes_files_focused(screen)
        && right_focus_near_middle(screen)
        && right_pane(screen).contains("keepmid-")
        && !right_pane(screen).contains("keepmid-00.txt")
}

/// Right-pane `j` / `k` / wheel keep the focused row near the vertical middle.
///
/// Help MOVE lists `j k` as "down / up". Docs: the workspace tree already
/// recentres the focused row; graph commits, file-diff rows, and the
/// commit-file list use the same viewport rule. Vertical wheel over the
/// right pane moves that focused row (not a viewport-only scroll). Motion-bit
/// `CSI < 97` must not move. This is not hscroll.
///
/// Three overflowing right-pane lists: a 40-line dirty file, a 30-commit
/// graph, and a 40-file commit. After 22 down steps the early unique marker
/// leaves (`keepmid-line-0`, working tree / `count 29`, `keepmid-00.txt`)
/// and the cursor bar sits near the pane middle. A no-op stays on the first
/// row. Edge-stuck (last visible line) is the old graph/diff wheel. `G`
/// would land on the last row.
#[test]
fn pty_right_pane_keeps_focus_middle() {
    let (_root, workspace) = keep_middle_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(
        help_lists_jk_down_up,
        "help MOVE lists j k as down / up",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("down / up") && screen.contains("? help"),
        "Esc closes help so j is not swallowed",
        WAIT,
    );

    tui.search("keepmid-diff");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "keepmid-diff.rs") && screen.contains("keepmid-line-0"),
        "search lands on the tall dirty file",
        GIT_WAIT,
    );
    tui.tab();
    tui.wait_pred(
        diff_launch_top,
        "Tab focuses the file-diff; keepmid-line-0 is still at the top",
        WAIT,
    );
    j_steps(&mut tui, STEPS);
    tui.wait_pred(
        diff_kept_middle,
        "j on a focused file-diff recentres the row (a no-op keeps keepmid-line-0; edge-stuck is the last pane line; G would hit the last line)",
        WAIT,
    );

    return_to_tree(&mut tui);
    tui.search("history");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "history") && screen.contains("count 29"),
        "search lands on the tall graph repo",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        graph_launch_top,
        "Enter focuses the graph on working tree; count 29 is still in view",
        GIT_WAIT,
    );
    for _ in 0..STEPS {
        tui.sgr_mouse(SGR_WHEEL_DOWN_MOTION, RIGHT_PANE_COL, 8);
    }
    tui.wait_ms(SETTLE_MS);
    assert!(
        graph_launch_top(&tui.screen()),
        "motion-bit CSI < 97 must not move the graph cursor:\n{}",
        tui.screen()
    );
    for _ in 0..STEPS {
        tui.sgr_mouse(SGR_WHEEL_DOWN, RIGHT_PANE_COL, 8);
    }
    tui.wait_pred(
        graph_kept_middle,
        "vertical wheel over the graph recentres the focused commit (a no-op stays on working tree; viewport-only scroll drops the cursor bar; G would hit count 0)",
        GIT_WAIT,
    );

    return_to_tree(&mut tui);
    tui.search("bundle");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "bundle") && screen.contains("keepmid-files-commit"),
        "search lands on the many-file repo",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| graph_pane_focused(screen) && graph_cursor_on(screen, "working tree"),
        "Enter focuses the bundle graph on working tree",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "keepmid-files-commit")
                && !graph_cursor_on(screen, "working tree")
        },
        "j selects the many-file commit",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        files_launch_top,
        "Enter drills to commit files; keepmid-00.txt is still at the top",
        GIT_WAIT,
    );
    j_steps(&mut tui, STEPS);
    tui.wait_pred(
        files_kept_middle,
        "j on the commit-file list recentres the row (a no-op keeps keepmid-00.txt; edge-stuck is the last pane line; G would hit keepmid-39.txt)",
        WAIT,
    );
}

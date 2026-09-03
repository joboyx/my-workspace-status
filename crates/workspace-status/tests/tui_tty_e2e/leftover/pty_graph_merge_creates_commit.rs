use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{
    crumb_row, graph_cursor_on, graph_pane_focused, no_mouse_toggle_toast, status_row,
    title_has_files, tree_cursor_on, tree_has, tree_pane_focused, GIT_WAIT, SETTLE_MS, WAIT,
};

fn no_merge_confirm(screen: &str) -> bool {
    !screen.contains("fast-forward if possible")
        && !screen.contains("Merge main into")
        && !screen.contains("otherwise a merge commit")
}

fn no_wrong_merge_overlays(screen: &str) -> bool {
    !screen.contains("MOVE")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("Stash ")
        && !title_has_files(screen)
        && no_mouse_toggle_toast(screen)
}

fn focusbox_diverged_graph_body(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("noise-leaf-commit")
        && screen.contains("focus-root-commit")
        && screen.contains("[+feature/keep]")
        && screen.contains("[main]")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
}

/// First paint: focusbox on the tree. Graph `m` has not run.
fn idle_focusbox_before_graph_merge(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "focusbox")
        && tree_pane_focused(screen)
        && tree_has(screen, "feature/keep")
        && crumb.contains("workspace › focusbox")
        && !crumb.contains("[focusbox]")
        && !crumb.contains("Merged")
        && status.contains("focus right")
        && !status.contains("drill")
        && !screen.contains("Merge branch")
        && no_merge_confirm(screen)
        && no_wrong_merge_overlays(screen)
}

/// Tab focused the graph. HEAD is still `keep-leaf-commit`. Merge is idle.
fn graph_focused_diverged_before_merge(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "focusbox")
        && focusbox_diverged_graph_body(screen)
        && crumb.contains("workspace › [focusbox]")
        && !crumb.contains("Merged")
        && !crumb.contains("Fast-forwarded")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !screen.contains("Merge branch")
        && no_merge_confirm(screen)
        && no_wrong_merge_overlays(screen)
}

/// Graph cursor on `main-leaf-commit`. Hint `m` is merge. Overlay closed.
fn main_leaf_ready_to_merge(screen: &str) -> bool {
    let status = status_row(screen);
    graph_focused_diverged_before_merge(screen)
        && graph_cursor_on(screen, "main-leaf-commit")
        && !graph_cursor_on(screen, "keep-leaf-commit")
        && !graph_cursor_on(screen, "working tree")
        && status.contains("checkout")
        && status.contains("create branch")
        && status.contains("merge")
        && screen.contains("/main-leaf-commit")
}

/// `PendingConfirm::MergeIntoHead` boxed overlay. Not mouse. Not a write yet.
fn merge_into_head_confirm(screen: &str) -> bool {
    graph_pane_focused(screen)
        && screen.contains("Merge main into feature/keep?")
        && screen.contains("fast-forward if possible, otherwise a merge commit")
        && screen.contains("merge")
        && screen.contains("cancel")
        && screen.contains("main-leaf-commit")
        && screen.contains("keep-leaf-commit")
        && !screen.contains("Merged")
        && !screen.contains("Fast-forwarded")
        && !screen.contains("Already up to date")
        && !screen.contains("Merge branch")
        && no_mouse_toggle_toast(screen)
        && !screen.contains("MOVE")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !title_has_files(screen)
}

/// `y` created a merge commit into HEAD. Fast-forward / no-op cannot pass.
fn documented_graph_merge_commit(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && screen.contains("Merge branch 'main' into feature/keep")
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("[+feature/keep]")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
        && tree_has(screen, "feature/keep")
        && crumb.contains("Merged main")
        && !crumb.contains("failed")
        && !crumb.contains("Fast-forwarded")
        && !crumb.contains("Already up to date")
        && !screen.contains("Merge main into feature/keep?")
        && no_merge_confirm(screen)
        && no_mouse_toggle_toast(screen)
        && !screen.contains("MOVE")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !title_has_files(screen)
        && status.contains("drill")
        && status.contains(" tree")
        && status.contains(" split")
}

/// Graph `m` merges the focused commit into HEAD.
///
/// Docs: Help GIT `m` = graph merge into HEAD. Keymap: graph-focused `m`
/// is `Action::GraphMerge`. Confirm is `PendingConfirm::MergeIntoHead`.
/// Yes runs `merge_into_head` (fast-forward when possible, otherwise a
/// merge commit). Tree `m` is `ToggleMouse` (`pty_m_toggles_mouse_capture`).
///
/// After first paint the cursor is already on `focusbox`. Tab focuses the
/// graph. `/` lands on diverged `main-leaf-commit` (HEAD is
/// `keep-leaf-commit`, so this cannot fast-forward or be already up to
/// date). `m` then `y` must paint the merge-commit subject and `Merged
/// main`. A no-op, mouse toggle, overlay-only, toast-only, fast-forward,
/// or already-up-to-date is red.
#[test]
fn pty_graph_merge_creates_commit() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("focusbox", WAIT);
    tui.wait_pred(
        idle_focusbox_before_graph_merge,
        "first paint: focusbox on the tree, no merge confirm, not mouse toggle",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_focused_diverged_before_merge,
        "Tab focuses the graph: keep and main tips, HEAD still keep, merge idle",
        GIT_WAIT,
    );

    tui.search("main-leaf-commit");
    tui.wait_pred(
        main_leaf_ready_to_merge,
        "graph cursor on main-leaf-commit; m merge hint; overlay closed",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.key('m');
    tui.wait_pred(
        merge_into_head_confirm,
        "graph m opens Merge main into feature/keep confirm (not mouse, not a write)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        merge_into_head_confirm,
        "merge confirm holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key('y');
    tui.wait_pred(
        documented_graph_merge_commit,
        "y creates merge commit into HEAD: Merge branch 'main' into feature/keep, Merged main",
        GIT_WAIT,
    );
}

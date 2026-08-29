use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{
    crumb_row, focusbox_graph_left_full, graph_cursor_on, graph_pane_focused,
    graph_subject_meta_line, no_mouse_toggle_toast, status_row, tree_cursor_on, tree_has,
    tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};

const BRANCH: &str = "e2e-at-commit";

fn hex7_after(haystack: &str, prefix: &str) -> Option<String> {
    let at = haystack.find(prefix)?;
    let rest = &haystack[at + prefix.len()..];
    let hash: String = rest.chars().take(7).collect();
    (hash.len() == 7 && hash.chars().all(|c| c.is_ascii_hexdigit())).then_some(hash)
}

fn overlay_at_hash(screen: &str) -> Option<String> {
    screen
        .lines()
        .find_map(|line| hex7_after(line, "Create branch at "))
}

fn no_wrong_create_overlays(screen: &str) -> bool {
    !screen.contains("MOVE")
        && !screen.contains("Focus branches")
        && !screen.contains("Stash ")
        && !screen.contains("┌ files")
        && !screen.contains("commit message")
        && !screen.contains("fast-forward if possible")
        && !screen.contains("Merge main into")
        && !screen.contains("SEARCH")
        && no_mouse_toggle_toast(screen)
}

fn focusbox_still_on_keep(screen: &str) -> bool {
    tree_has(screen, "feature/keep")
        && tree_line_containing(screen, "focusbox")
            .is_some_and(|line| line.contains("feature/keep") && !line.contains(BRANCH))
}

fn main_leaf_chip_has_new_ref(screen: &str) -> bool {
    graph_subject_meta_line(screen, "main-leaf-commit").is_some_and(|line| {
        line.contains(&format!("[{BRANCH}]")) && !line.contains(&format!("[+{BRANCH}]"))
    })
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

/// Tab focused the graph. HEAD is still `keep-leaf-commit`. Create is idle.
fn graph_focused_diverged_before_create(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && focusbox_diverged_graph_body(screen)
        && crumb.contains("workspace › [focusbox]")
        && !crumb.contains(&format!("created {BRANCH}"))
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && overlay_at_hash(screen).is_none()
        && !screen.contains("Create branch")
        && !screen.contains("Enter confirm")
        && focusbox_still_on_keep(screen)
        && no_wrong_create_overlays(screen)
}

/// Graph cursor on `main-leaf-commit`. Hint `c` is create branch. Overlay closed.
fn main_leaf_ready_to_create(screen: &str) -> bool {
    let status = status_row(screen);
    graph_focused_diverged_before_create(screen)
        && graph_cursor_on(screen, "main-leaf-commit")
        && !graph_cursor_on(screen, "keep-leaf-commit")
        && !graph_cursor_on(screen, "working tree")
        && status.contains("checkout")
        && status.contains("create branch")
        && status.contains("merge")
        && screen.contains("/main-leaf-commit")
}

fn create_branch_overlay(screen: &str, name_line: &str) -> bool {
    graph_pane_focused(screen)
        && overlay_at_hash(screen).is_some()
        && screen.contains("Create branch")
        && screen.contains(name_line)
        && screen.contains("Enter confirm")
        && screen.contains("Esc cancel")
        && graph_cursor_on(screen, "main-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("keep-leaf-commit")
        && !screen.contains(&format!("created {BRANCH}"))
        && !screen.contains("Switched")
        && focusbox_still_on_keep(screen)
        && no_wrong_create_overlays(screen)
}

fn empty_create_branch_at_overlay(screen: &str) -> bool {
    create_branch_overlay(screen, "name: …") && !screen.contains(BRANCH)
}

fn named_create_branch_at_overlay(screen: &str) -> bool {
    create_branch_overlay(screen, &format!("name: {BRANCH}"))
}

/// `create_branch_at`: new ref on `main-leaf-commit`. HEAD stays `feature/keep`.
fn documented_graph_create_branch_at(screen: &str, overlay_hash: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && graph_cursor_on(screen, "main-leaf-commit")
        && !graph_cursor_on(screen, "keep-leaf-commit")
        && hex7_after(crumb, &format!("created {BRANCH} at ")).as_deref() == Some(overlay_hash)
        && !crumb.contains("failed")
        && !crumb.contains("Switched")
        && !crumb.contains("Already on")
        && overlay_at_hash(screen).is_none()
        && !screen.contains("Create branch")
        && !screen.contains("Enter confirm")
        && !screen.contains("Esc cancel")
        && main_leaf_chip_has_new_ref(screen)
        && screen.contains("[+feature/keep]")
        && screen.contains("[main]")
        && focusbox_still_on_keep(screen)
        && tree_cursor_on(screen, "focusbox")
        && status.contains("drill")
        && status.contains(" tree")
        && status.contains(" split")
        && status.contains("create branch")
        && no_wrong_create_overlays(screen)
}

/// Graph `c` creates a local ref at the focused commit (no checkout).
///
/// Docs: Help GIT `C` is picker create+checkout. Keymap: graph-focused `c`
/// on a commit is `Action::GraphCreateBranch`. Overlay is the name prompt
/// with `Create branch at <short>`. Enter runs `create_branch_at`
/// (`git branch -- name commitId`). HEAD stays on the current branch.
/// Tree-file `c` is a no-op (`pty_c_on_tree_file_is_not_commit`). Picker
/// `C` stays `pty_branch_picker_shift_c_creates`.
///
/// After first paint the cursor is already on `focusbox`. Tab focuses
/// the graph. `/` lands on diverged `main-leaf-commit` (not HEAD
/// `keep-leaf-commit`). `c` then a name then Enter must paint the new chip
/// on that commit and toast `created … at <short>`. A no-op, tree-file
/// `c`, picker checkout, overlay-only, or toast-only is red.
#[test]
fn pty_graph_c_creates_branch_at_commit() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("focusbox", WAIT);
    tui.wait_pred(
        focusbox_graph_left_full,
        "first paint: focusbox on the tree, full graph, create-branch overlay closed",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_focused_diverged_before_create,
        "Tab focuses the graph: keep and main tips, HEAD still keep, overlay closed",
        GIT_WAIT,
    );

    tui.search("main-leaf-commit");
    tui.wait_pred(
        main_leaf_ready_to_create,
        "graph cursor on main-leaf-commit; c create branch hint; overlay closed",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.key('c');
    tui.wait_pred(
        empty_create_branch_at_overlay,
        "graph c opens Create branch at <short> (not picker C, not a write)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        empty_create_branch_at_overlay,
        "create-branch overlay holds (not a flicker or toast-only tick)",
        WAIT,
    );
    let overlay_hash = overlay_at_hash(&tui.screen()).expect("Create branch at <short>");

    tui.keys(BRANCH);
    tui.wait_pred(
        named_create_branch_at_overlay,
        "typed name is in the overlay; Enter has not created the ref yet",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| documented_graph_create_branch_at(screen, &overlay_hash),
        "Enter creates e2e-at-commit at the focused commit; HEAD stays feature/keep",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| documented_graph_create_branch_at(screen, &overlay_hash),
        "created paint holds (not a flicker, toast-only tick, or checkout)",
        WAIT,
    );
}

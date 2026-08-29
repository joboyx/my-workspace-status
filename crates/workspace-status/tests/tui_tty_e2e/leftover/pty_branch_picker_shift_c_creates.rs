use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{
    crumb_row, focusbox_graph_left_full, graph_subject_meta_line, no_mouse_toggle_toast,
    status_row, tree_cursor_on, tree_has, tree_line_containing, tree_pane_focused, GIT_WAIT,
    SETTLE_MS, WAIT,
};

const BRANCH: &str = "e2e-from-picker";

fn no_wrong_create_overlays(screen: &str) -> bool {
    !screen.contains("MOVE")
        && !screen.contains("Focus branches")
        && !screen.contains("Stash ")
        && !screen.contains("┌ files")
        && !screen.contains("commit message")
        && !screen.contains("fast-forward if possible")
        && !screen.contains("Merge main into")
        && !screen.contains("SEARCH")
        && !screen.contains("Checkout at")
        && no_mouse_toggle_toast(screen)
}

fn focusbox_on_keep(screen: &str) -> bool {
    tree_has(screen, "feature/keep")
        && tree_line_containing(screen, "focusbox")
            .is_some_and(|line| line.contains("feature/keep") && !line.contains(BRANCH))
}

fn focusbox_checked_out_new_branch(screen: &str) -> bool {
    tree_has(screen, BRANCH)
        && tree_line_containing(screen, "focusbox").is_some_and(|line| {
            line.contains(BRANCH) && !line.contains("feature/keep") && !line.contains("& main")
        })
}

fn keep_leaf_has_checked_out_new_ref(screen: &str) -> bool {
    graph_subject_meta_line(screen, "keep-leaf-commit").is_some_and(|line| {
        line.contains(&format!("[+{BRANCH}]")) && line.contains("[feature/keep]")
    })
}

fn main_leaf_lacks_new_ref(screen: &str) -> bool {
    graph_subject_meta_line(screen, "main-leaf-commit")
        .is_some_and(|line| line.contains("[main]") && !line.contains(BRANCH))
}

fn picker_create_overlay(screen: &str, name_line: &str) -> bool {
    let overlay_title = screen
        .lines()
        .any(|line| line.contains("Create branch") && !line.contains("Create branch at"));
    tree_pane_focused(screen)
        && overlay_title
        && !screen.contains("Create branch at")
        && screen.contains(name_line)
        && screen.contains("Enter confirm")
        && screen.contains("Esc cancel")
        && !screen.contains("C create")
        && !screen.contains("Enter checkout")
        && !screen.contains(&format!("created {BRANCH}"))
        && focusbox_on_keep(screen)
        && no_wrong_create_overlays(screen)
}

fn empty_picker_create_overlay(screen: &str) -> bool {
    picker_create_overlay(screen, "name: …") && !screen.contains(BRANCH)
}

fn named_picker_create_overlay(screen: &str) -> bool {
    picker_create_overlay(screen, &format!("name: {BRANCH}"))
}

/// Tree picker is open on `focusbox`. `C` is create. Overlay is not graph `c`.
fn tree_picker_open_on_keep(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && focusbox_on_keep(screen)
        && screen.contains("Branch ")
        && screen.contains("filter:")
        && screen.contains("* feature/keep")
        && screen.contains("C create")
        && screen.contains("Enter checkout")
        && screen.contains("Esc close")
        && !screen.contains("Create branch")
        && !screen.contains("Enter confirm")
        && !screen.contains("Create branch at")
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && no_wrong_create_overlays(screen)
}

/// Picker `C` + Enter ran `checkout -b` at HEAD. Not graph `c`, not tree-file `c`.
fn documented_picker_create_checkout(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && crumb.contains(&format!("created {BRANCH}"))
        && !crumb.contains(&format!("created {BRANCH} at"))
        && !crumb.contains("failed")
        && !crumb.contains("Switched")
        && !crumb.contains("Already on")
        && !screen.contains("Create branch")
        && !screen.contains("Enter confirm")
        && !screen.contains("Esc cancel")
        && !screen.contains("C create")
        && focusbox_checked_out_new_branch(screen)
        && keep_leaf_has_checked_out_new_ref(screen)
        && main_leaf_lacks_new_ref(screen)
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
        && status.contains("focus right")
        && status.contains(" tree")
        && status.contains(" split")
        && !status.contains("create branch")
        && no_wrong_create_overlays(screen)
}

/// Picker `C` creates a branch at HEAD and checks it out.
///
/// Docs: Help GIT `C` is create in the picker. Keymap: picker `C` is
/// `Action::CreateBranchStart`. Overlay is the name prompt with
/// `Create branch` (no `at <short>`) and `Enter confirm · Esc cancel`.
/// Enter runs `create_branch_checkout` (`git checkout -b name`). HEAD
/// moves to the new branch. Graph `c` is ref-only at the focused commit
/// (`pty_graph_c_creates_branch_at_commit`). Tree-file `c` is a no-op
/// (`pty_c_on_tree_file_is_not_commit`).
///
/// After first paint the cursor is already on `focusbox` (`feature/keep`).
/// `b` opens the local picker. Shift+C then a name then Enter must toast
/// `created …` (not `created … at <short>`), check out the new name, and
/// leave HEAD on `keep-leaf-commit`. A no-op, graph `c`, picker Enter
/// onto `main`, overlay-only, or toast-only is red.
#[test]
fn pty_branch_picker_shift_c_creates() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("focusbox", WAIT);
    tui.wait_pred(
        focusbox_graph_left_full,
        "first paint: focusbox on the tree, full graph, picker closed",
        WAIT,
    );

    tui.key('b');
    tui.wait_pred(
        tree_picker_open_on_keep,
        "b opens the local picker on focusbox; C create; HEAD still feature/keep",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.shift_letter('C');
    tui.wait_pred(
        empty_picker_create_overlay,
        "picker Shift+C opens Create branch (not graph c, not a write)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        empty_picker_create_overlay,
        "create-branch overlay holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.keys(BRANCH);
    tui.wait_pred(
        named_picker_create_overlay,
        "typed name is in the overlay; Enter has not created the ref yet",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        documented_picker_create_checkout,
        "Enter creates e2e-from-picker at HEAD and checks it out",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_picker_create_checkout,
        "created checkout paint holds (not a flicker, toast-only tick, or graph-c)",
        WAIT,
    );
}

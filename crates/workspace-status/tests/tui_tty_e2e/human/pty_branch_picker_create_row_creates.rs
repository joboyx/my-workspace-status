use crate::harness::PtySession;
use crate::seed::focus_workspace;
use crate::support::{
    crumb_row, focusbox_graph_left_full, graph_subject_meta_line, no_mouse_toggle_toast,
    status_row, title_has_files, tree_cursor_on, tree_has, tree_line_containing, tree_pane_focused,
    GIT_WAIT, SETTLE_MS, WAIT,
};

/// `j`, `k`, and `C` used to move or open a prompt; in the picker they type.
const BRANCH: &str = "e2e-jk-Created";

fn no_wrong_create_overlays(screen: &str) -> bool {
    !screen.contains("MOVE")
        && !screen.contains("Focus branches")
        && !screen.contains("Stash ")
        && !title_has_files(screen)
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

/// Tree picker is open on `focusbox`, its filter holds the typed name, and
/// the list ends with the create row. No name prompt opened.
fn picker_with_create_row(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && focusbox_on_keep(screen)
        && screen.contains(&format!("filter: {BRANCH}"))
        && screen
            .lines()
            .any(|line| line.contains(&format!("❯   + create branch {BRANCH}")))
        && !screen.contains("No matching branches")
        && !screen.contains("Create branch")
        && !screen.contains(&format!("created {BRANCH}"))
        && no_wrong_create_overlays(screen)
}

/// Tree picker is open on `focusbox` with an empty filter: no create row.
fn tree_picker_open_on_keep(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && focusbox_on_keep(screen)
        && screen.contains("Branch ")
        && screen.contains("filter:")
        && screen.contains("* feature/keep")
        && screen.contains("Enter checkout")
        && screen.contains("Esc close")
        && !screen.contains("+ create branch")
        && !screen.contains("Create branch")
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && no_wrong_create_overlays(screen)
}

/// The create row + Enter ran `checkout -b` at HEAD. Not graph `c`, not tree-file `c`.
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
        && !screen.contains("Enter create")
        && !screen.contains("+ create branch")
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

/// The branch picker create row creates a branch at HEAD and checks it out.
///
/// Docs: every printable key types into the picker filter (`j`, `k`, `C`
/// too). A filter that is a valid new name and not an exact local branch
/// ends the list with `+ create branch <name>`. Enter on it runs
/// `create_branch_checkout` (`git checkout -b name`) and HEAD moves to the
/// new branch. Graph `c` is ref-only at the focused commit
/// (`pty_graph_c_creates_branch_at_commit`). Tree-file `c` is a no-op
/// (`pty_c_on_tree_file_is_not_commit`).
///
/// After first paint the cursor is already on `focusbox` (`feature/keep`).
/// `b` opens the local picker. Typing the name must land in the filter
/// with the create row selected (a `j` / `k` that moved, or a `C` that
/// opened a prompt, is red). Enter must toast `created …` (not `created …
/// at <short>`), check out the new name, and leave HEAD on
/// `keep-leaf-commit`.
#[test]
fn pty_branch_picker_create_row_creates() {
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
        "b opens the local picker on focusbox; no create row yet; HEAD still feature/keep",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.keys(BRANCH);
    tui.wait_pred(
        picker_with_create_row,
        "j / k / C type into the filter and the create row takes the cursor",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        picker_with_create_row,
        "the create row holds; Enter has not created the ref yet",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        documented_picker_create_checkout,
        "Enter on the create row creates the branch at HEAD and checks it out",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_picker_create_checkout,
        "created checkout paint holds (not a flicker, toast-only tick, or graph-c)",
        WAIT,
    );
}

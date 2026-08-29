use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{git_env, worktree_workspace};
use crate::support::{
    crumb_row, no_mouse_toggle_toast, status_row, tree_cursor_on, tree_has, tree_line_containing,
    tree_pane_focused, GIT_WAIT, SETTLE_MS, WAIT,
};

const LINKED_PATH: &str = "app/.worktrees/feat";
const LINKED_BRANCH: &str = "feature/linked-open";
const PRIMARY_BRANCH: &str = "feature/primary-open";

fn git_worktree_porcelain(repo: &Path) -> String {
    let mut cmd = Command::new("git");
    cmd.args(["worktree", "list", "--porcelain"])
        .current_dir(repo);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().expect("git worktree list");
    assert!(
        out.status.success(),
        "git worktree list failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn linked_worktree_registered(repo: &Path) -> bool {
    git_worktree_porcelain(repo).contains(".worktrees/feat")
}

fn no_remove_confirm(screen: &str) -> bool {
    !screen.contains("Remove worktree ")
        && !screen.contains("NOT merged into default")
        && !screen.contains("clean worktree")
        && !screen.contains("dirty worktree — will use --force")
}

fn no_wrong_remove_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("Merge main into")
        && !screen.contains("Drop stash@{")
        && !screen.contains("Revert ")
        && !screen.contains("┌ files")
        && no_mouse_toggle_toast(screen)
}

fn family_and_linked_on_tree(screen: &str) -> bool {
    let linked = tree_line_containing(screen, LINKED_BRANCH);
    tree_has(screen, "app")
        && tree_has(screen, PRIMARY_BRANCH)
        && tree_has(screen, LINKED_BRANCH)
        && tree_has(screen, "2 wt")
        && linked.is_some_and(|line| line.contains('L') && line.contains("linked-open o"))
}

fn family_tree_idle(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, LINKED_BRANCH)
        && !tree_cursor_on(screen, PRIMARY_BRANCH)
        && !tree_cursor_on(screen, "workspace")
        && family_and_linked_on_tree(screen)
        && crumb_row(screen).contains("workspace › app")
        && no_remove_confirm(screen)
        && no_wrong_remove_overlays(screen)
}

/// First paint: family `@ app` focused. Linked row is visible. `W` has not run.
fn idle_family_before_remove(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    family_tree_idle(screen)
        && !crumb.contains("removed worktree")
        && !crumb.contains("Focus a linked worktree to remove")
}

/// Family-row `W` refuses. Overlay stays closed. Linked checkout stays.
fn family_w_refuses_without_confirm(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    family_tree_idle(screen)
        && crumb.contains("Focus a linked worktree to remove")
        && !crumb.contains("removed worktree")
}

/// Cursor on the linked checkout. Hint `W` is remove. Overlay closed.
fn linked_ready_to_remove(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    tree_pane_focused(screen)
        && tree_cursor_on(screen, LINKED_BRANCH)
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, PRIMARY_BRANCH)
        && family_and_linked_on_tree(screen)
        && crumb.contains("workspace › feat")
        && !crumb.contains("removed worktree")
        && status.contains("W")
        && status.contains("remove worktree")
        && no_remove_confirm(screen)
        && no_wrong_remove_overlays(screen)
}

/// `PendingConfirm::RemoveWorktree` boxed overlay. Linked checkout still there.
fn remove_worktree_confirm(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_pane_focused(screen)
        && tree_cursor_on(screen, LINKED_BRANCH)
        && family_and_linked_on_tree(screen)
        && screen.contains(&format!("Remove worktree {LINKED_PATH}?"))
        && screen.contains(&format!("branch {LINKED_BRANCH} — NOT merged into default"))
        && screen.contains("clean worktree")
        && screen.contains("remove")
        && screen.contains("cancel")
        && !screen.contains("dirty worktree — will use --force")
        && !crumb.contains("removed worktree")
        && no_wrong_remove_overlays(screen)
}

/// Confirm `y` ran `git worktree remove`. Ghost / overlay-only cannot pass.
fn documented_worktree_removed(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_pane_focused(screen)
        && tree_has(screen, "app")
        && tree_has(screen, PRIMARY_BRANCH)
        && !tree_has(screen, LINKED_BRANCH)
        && !tree_has(screen, "2 wt")
        && crumb.contains(&format!("removed worktree {LINKED_PATH}"))
        && !crumb.contains("failed")
        && !crumb.contains("Focus a linked worktree to remove")
        && !crumb.contains("remove worktree cancelled")
        && no_remove_confirm(screen)
        && no_wrong_remove_overlays(screen)
}

/// `W` on a linked worktree asks, then removes.
///
/// Docs: Help GIT `W` = remove linked worktree. Keymap: `w` / `W` opens a
/// boxed confirm (`y` / `n`, merge status, `--force` when dirty). Other
/// rows refuse with `Focus a linked worktree to remove`. Yes runs
/// `remove_worktree` (`git worktree remove`).
///
/// Live PTY after first paint (cursor already on family `@ app`): CSI-u
/// Shift+W refuses and does not open confirm or drop the linked checkout.
/// `j` then `j` land on `L feature/linked-open`. CSI-u Shift+W then paints
/// `Remove worktree app/.worktrees/feat?` with open-vs-default, clean
/// worktree, `y` remove / `n` cancel. `y` toasts `removed worktree
/// app/.worktrees/feat` and drops that row. Git no longer lists the
/// linked path. A no-op, family remove, immediate remove, overlay-only,
/// toast-only, or a search-armed `/linked-open` tick cannot pass.
#[test]
fn pty_worktree_w_remove_confirm() {
    let (_root, workspace) = worktree_workspace();
    let app = workspace.join("app");
    let linked_dir = app.join(".worktrees").join("feat");
    assert!(
        linked_worktree_registered(&app),
        "seed must register the linked worktree:\n{}",
        git_worktree_porcelain(&app)
    );
    assert!(
        linked_dir.is_dir(),
        "seed linked dir missing: {}",
        linked_dir.display()
    );

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        idle_family_before_remove,
        "first paint: family app focused, linked row visible, W has not run",
        WAIT,
    );

    tui.shift_letter('W');
    tui.wait_pred(
        family_w_refuses_without_confirm,
        "family W refuses with Focus a linked worktree to remove; overlay closed",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        family_w_refuses_without_confirm,
        "family refuse holds (not a delayed confirm or immediate remove)",
        WAIT,
    );
    assert!(
        linked_worktree_registered(&app),
        "family W must not run git worktree remove:\n{}",
        git_worktree_porcelain(&app)
    );

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, PRIMARY_BRANCH) && !tree_cursor_on(screen, LINKED_BRANCH),
        "first j: cursor on primary checkout (not linked)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        linked_ready_to_remove,
        "second j: cursor on linked checkout; W remove hint; overlay closed",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.shift_letter('W');
    tui.wait_pred(
        remove_worktree_confirm,
        "linked W opens Remove worktree confirm (not a write, not family refuse)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        remove_worktree_confirm,
        "remove confirm holds (not a flicker, immediate remove, or toast-only)",
        WAIT,
    );
    assert!(
        linked_worktree_registered(&app),
        "confirm must not remove until y:\n{}",
        git_worktree_porcelain(&app)
    );
    assert!(
        linked_dir.is_dir(),
        "confirm must leave the linked dir: {}",
        linked_dir.display()
    );

    tui.key('y');
    tui.wait_pred(
        documented_worktree_removed,
        "y removes the linked worktree: toast, row gone, primary stays",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_worktree_removed,
        "removed worktree holds (not a ghost row or overlay-only tick)",
        WAIT,
    );
    assert!(
        !linked_worktree_registered(&app),
        "git still lists the linked worktree after y:\n{}",
        git_worktree_porcelain(&app)
    );
    assert!(
        !linked_dir.exists(),
        "linked dir still on disk after y: {}",
        linked_dir.display()
    );
}

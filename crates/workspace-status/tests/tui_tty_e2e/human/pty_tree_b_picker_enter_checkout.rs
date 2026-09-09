use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{focus_workspace, git_env};
use crate::support::{
    crumb_row, focusbox_graph_left_full, has_fetch_hint, no_mouse_toggle_toast, status_row,
    title_has_files, tree_cursor_on, tree_has, tree_line_containing, tree_pane_focused, GIT_WAIT,
    SETTLE_MS, WAIT,
};

fn git_stdout(repo: &Path, args: &[&str]) -> String {
    let mut cmd = Command::new("git");
    cmd.args(args).current_dir(repo);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().expect("git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

fn head_branch(repo: &Path) -> String {
    git_stdout(repo, &["rev-parse", "--abbrev-ref", "HEAD"])
}

fn head_subject(repo: &Path) -> String {
    git_stdout(repo, &["log", "-1", "--pretty=%s"])
}

fn porcelain(repo: &Path) -> String {
    git_stdout(repo, &["status", "-sb"])
}

fn local_branch_names(repo: &Path) -> String {
    git_stdout(
        repo,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )
}

fn assert_head(repo: &Path, branch: &str, subject: &str, screen: &str) {
    assert_eq!(
        head_branch(repo),
        branch,
        "HEAD branch must be {branch}:\n{}\n{screen}",
        porcelain(repo)
    );
    assert_eq!(
        head_subject(repo),
        subject,
        "HEAD commit must be {subject}:\n{}\n{screen}",
        porcelain(repo)
    );
}

fn no_wrong_picker_overlays(screen: &str) -> bool {
    !screen.contains("MOVE")
        && !screen.contains("Focus branches")
        && !screen.contains("Stash ")
        && !title_has_files(screen)
        && !screen.contains("commit message")
        && !screen.contains("fast-forward if possible")
        && !screen.contains("Merge ")
        && !screen.contains("SEARCH")
        && !screen.contains("Checkout at")
        && !screen.contains("Create branch")
        && no_mouse_toggle_toast(screen)
}

fn focusbox_on_keep(screen: &str) -> bool {
    tree_has(screen, "feature/keep")
        && tree_line_containing(screen, "focusbox")
            .is_some_and(|line| line.contains("feature/keep") && !line.contains("& main"))
}

fn focusbox_tree_on_main(screen: &str) -> bool {
    tree_has(screen, "& main")
        && tree_line_containing(screen, "focusbox")
            .is_some_and(|line| line.contains("& main") && !line.contains("feature/keep"))
        && !tree_has(screen, "feature/keep")
}

fn keep_is_checked_out(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("[+feature/keep]")
        && !screen.contains("[+main]")
}

fn main_is_checked_out(screen: &str) -> bool {
    screen.contains("main-leaf-commit")
        && screen.contains("[+main]")
        && !screen.contains("[+feature/keep]")
}

fn picker_cursor_on_main(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains('❯')
            && line.contains("main")
            && !line.contains("feature")
            && !line.contains('*')
    })
}

/// Tree picker chrome on `focusbox`. Overlay is checkout, not graph `b` / create.
fn tree_picker_checkout_chrome(screen: &str) -> bool {
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && focusbox_on_keep(screen)
        && keep_is_checked_out(screen)
        && screen.contains("Branch ")
        && screen.contains("filter:")
        && screen.contains("C create")
        && screen.contains("Enter checkout")
        && screen.contains("Esc close")
        && !screen.contains("Create branch")
        && !screen.contains("Enter confirm")
        && !screen.contains("Create branch at")
        && !screen.contains("Checkout at")
        && !crumb_row(screen).contains("Checked out")
        && !crumb_row(screen).contains("created ")
        && !crumb_row(screen).contains("Already on")
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && no_wrong_picker_overlays(screen)
}

/// Tree picker is open on `focusbox`. Enter is checkout. Overlay is not graph `b`.
fn tree_picker_open_on_keep(screen: &str) -> bool {
    tree_picker_checkout_chrome(screen) && screen.contains("* feature/keep")
}

/// Filter isolated existing `main`. Enter has not run. Overlay is still checkout.
fn tree_picker_filtered_to_main(screen: &str) -> bool {
    tree_picker_checkout_chrome(screen)
        && screen.contains("filter: main")
        && picker_cursor_on_main(screen)
        && !screen.contains("* feature/keep")
        && !screen.contains("No matching branches")
        && !screen.contains("name: ")
}

/// Picker Enter ran `git checkout` of selected `main`. Not create. Not already-on.
fn documented_tree_picker_enter_checkout(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && crumb.contains("Checked out main")
        && !crumb.contains("created ")
        && !crumb.contains("Already on")
        && !crumb.contains("failed")
        && !crumb.contains("Dirty worktree")
        && !screen.contains("Create branch")
        && !screen.contains("Enter confirm")
        && !screen.contains("Enter checkout")
        && !screen.contains("C create")
        && !screen.contains("filter:")
        && !screen.contains("Branch focusbox")
        && focusbox_tree_on_main(screen)
        && main_is_checked_out(screen)
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
        && has_fetch_hint(screen)
        && status.contains("focus right")
        && status.contains(" tree")
        && status.contains(" split")
        && !status.contains("create branch")
        && no_wrong_picker_overlays(screen)
}

/// First paint: cursor on clean `focusbox` (`feature/keep`). Picker closed.
fn idle_focusbox_picker_closed(screen: &str) -> bool {
    let status = status_row(screen);
    focusbox_graph_left_full(screen)
        && focusbox_on_keep(screen)
        && keep_is_checked_out(screen)
        && !screen.contains("Branch ")
        && !screen.contains("Enter checkout")
        && !screen.contains("C create")
        && !screen.contains("filter:")
        && !crumb_row(screen).contains("Checked out")
        && !crumb_row(screen).contains("created ")
        && status.contains(" tree")
        && status.contains(" split")
        && status.contains("branch")
        && has_fetch_hint(screen)
        && no_wrong_picker_overlays(screen)
}

/// Tree `b` opens the local picker. Enter checks out the selected branch.
///
/// Docs: Help GIT `b` is `depth 0 picker · graph local/origin/*`. Keymap:
/// tree `b` is `Action::Branch` (local picker). Overlay title is
/// `Branch` (not graph `Checkout at`). Footer is `Enter checkout` /
/// `C create` / `Esc close`. Type to filter. Enter runs
/// `checkout_branch` of the selected local name. `C` is create
/// (`pty_branch_picker_shift_c_creates`). Graph `b` is refs on a
/// commit (`pty_graph_b_checkout`).
///
/// After first paint the cursor is already on `focusbox`
/// (`feature/keep`). `b` opens the picker. Filter `main` then Enter
/// must toast `Checked out main`, move git HEAD to `main` /
/// `main-leaf-commit`, close the overlay, and leave
/// `feature/keep` as a side ref. A no-op, picker-Enter create,
/// already-on, overlay-only, or toast-only is red.
#[test]
fn pty_tree_b_picker_enter_checkout() {
    let (_root, workspace) = focus_workspace();
    let repo = workspace.join("focusbox");
    assert_eq!(head_branch(&repo), "feature/keep");
    assert_eq!(head_subject(&repo), "keep-leaf-commit");
    let names = local_branch_names(&repo);
    assert!(
        names.lines().any(|name| name == "main")
            && names.lines().any(|name| name == "feature/keep")
            && names.lines().any(|name| name == "topic/noise"),
        "focusbox must have main, feature/keep, and topic/noise:\n{names}"
    );
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("focusbox", WAIT);
    tui.wait_pred(
        idle_focusbox_picker_closed,
        "first paint: focusbox on feature/keep, graph HEAD keep-leaf, picker closed",
        WAIT,
    );
    assert_head(&repo, "feature/keep", "keep-leaf-commit", &tui.screen());

    tui.key('b');
    tui.wait_pred(
        tree_picker_open_on_keep,
        "tree b opens the local picker on focusbox; Enter checkout; HEAD still feature/keep",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        tree_picker_open_on_keep,
        "picker holds (not a flicker, graph b, or no-op)",
        WAIT,
    );
    assert_head(&repo, "feature/keep", "keep-leaf-commit", &tui.screen());
    assert_eq!(
        porcelain(&repo),
        "## feature/keep",
        "opening the picker must not checkout:\n{}",
        tui.screen()
    );

    tui.keys("main");
    tui.wait_pred(
        tree_picker_filtered_to_main,
        "filter main isolates existing main; Enter has not checked out yet",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        tree_picker_filtered_to_main,
        "filtered picker holds (not create, not a write)",
        WAIT,
    );
    assert_head(&repo, "feature/keep", "keep-leaf-commit", &tui.screen());

    tui.enter();
    tui.wait_pred(
        documented_tree_picker_enter_checkout,
        "Enter checks out main: Checked out main, tree & main, [+main], picker closed",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_tree_picker_enter_checkout,
        "checkout paint holds (not a flicker, toast-only tick, create, or still feature/keep)",
        WAIT,
    );
    assert_head(&repo, "main", "main-leaf-commit", &tui.screen());
    assert_eq!(porcelain(&repo), "## main", "{}", tui.screen());
    let names_after = local_branch_names(&repo);
    assert!(
        names_after.lines().any(|name| name == "main")
            && names_after.lines().any(|name| name == "feature/keep")
            && names_after.lines().any(|name| name == "topic/noise")
            && names_after.lines().all(|name| name != "e2e-from-picker"),
        "Enter must checkout existing main, not create a branch:\n{names_after}\n{}",
        tui.screen()
    );
}

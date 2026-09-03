use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{focus_workspace, git_env};
use crate::support::{
    crumb_row, focusbox_graph_left_full, graph_cursor_on, graph_pane_focused,
    graph_subject_meta_line, no_mouse_toggle_toast, status_row, title_has_files, tree_cursor_on,
    tree_has, tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};

const BRANCH: &str = "topic/noise";
const SUBJECT: &str = "noise-leaf-commit";

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

fn head_sha(repo: &Path) -> String {
    git_stdout(repo, &["rev-parse", "HEAD"])
}

fn porcelain(repo: &Path) -> String {
    git_stdout(repo, &["status", "-sb"])
}

fn assert_head(repo: &Path, branch: &str, subject: &str, sha: &str, screen: &str) {
    assert_eq!(
        head_branch(repo),
        branch,
        "HEAD branch must be {branch} (not detached, not another ref):\n{}\n{screen}",
        porcelain(repo)
    );
    assert_eq!(
        head_subject(repo),
        subject,
        "HEAD commit must be {subject}:\n{}\n{screen}",
        porcelain(repo)
    );
    assert_eq!(
        head_sha(repo),
        sha,
        "HEAD SHA must match {branch} ({subject}):\n{}\n{screen}",
        porcelain(repo)
    );
}

fn no_picker_or_wrong_overlays(screen: &str) -> bool {
    !screen.contains("MOVE")
        && !screen.contains("Focus branches")
        && !screen.contains("Stash ")
        && !title_has_files(screen)
        && !screen.contains("Create branch")
        && !screen.contains("Checkout at")
        && !screen.contains("Branch ")
        && !screen.contains("Enter checkout")
        && !screen.contains("C create")
        && !screen.contains("Enter confirm")
        && !screen.contains("fast-forward if possible")
        && !screen.contains("Merge main into")
        && !screen.contains("SEARCH")
        && no_mouse_toggle_toast(screen)
}

fn focusbox_tree_on_keep(screen: &str) -> bool {
    tree_has(screen, "feature/keep")
        && tree_line_containing(screen, "focusbox")
            .is_some_and(|line| line.contains("feature/keep") && !line.contains(BRANCH))
}

fn focusbox_tree_on_noise(screen: &str) -> bool {
    tree_has(screen, BRANCH)
        && tree_line_containing(screen, "focusbox").is_some_and(|line| {
            line.contains(BRANCH) && !line.contains("feature/keep") && !line.contains("& main")
        })
}

fn keep_is_checked_out(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("[+feature/keep]")
        && !screen.contains("[+topic/noise]")
        && !screen.contains("[+main]")
}

fn noise_is_checked_out(screen: &str) -> bool {
    graph_subject_meta_line(screen, SUBJECT)
        .is_some_and(|line| line.contains("[+topic/noise]") && !line.contains("[+feature/keep]"))
        && screen.contains("keep-leaf-commit")
        && screen.contains("[feature/keep]")
        && !screen.contains("[+feature/keep]")
        && screen.contains("[main]")
        && !screen.contains("[+main]")
}

fn focusbox_diverged_graph_body(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("noise-leaf-commit")
        && screen.contains("focus-root-commit")
        && screen.contains("[+feature/keep]")
        && screen.contains("[main]")
        && screen.contains("[topic/noise]")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
}

/// Tab focused the graph. HEAD is still `keep-leaf-commit`. Checkout is idle.
fn graph_focused_diverged_before_checkout(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && focusbox_tree_on_keep(screen)
        && keep_is_checked_out(screen)
        && focusbox_diverged_graph_body(screen)
        && crumb.contains("workspace › [focusbox]")
        && !crumb.contains("Checked out")
        && !crumb.contains("Switched")
        && !crumb.contains("Already on")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && no_picker_or_wrong_overlays(screen)
}

/// Graph cursor on `noise-leaf-commit`. Hint `b` is checkout. Overlay closed.
fn noise_leaf_ready_to_checkout(screen: &str) -> bool {
    let status = status_row(screen);
    graph_focused_diverged_before_checkout(screen)
        && graph_cursor_on(screen, SUBJECT)
        && !graph_cursor_on(screen, "keep-leaf-commit")
        && !graph_cursor_on(screen, "working tree")
        && status.contains("checkout")
        && status.contains("create branch")
        && status.contains("merge")
        && screen.contains("/noise-leaf-commit")
}

/// Graph `b` checked out the one local name on the focused commit.
///
/// Checkout changes graph identity (`repo`, HEAD), so `set_graph`
/// resets the list cursor to the working-tree row. Stay on that paint.
/// Cursor still on `keep-leaf-commit` is a no-op.
fn documented_graph_b_checkout(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && crumb.contains(&format!("Checked out {BRANCH}"))
        && !crumb.contains("failed")
        && !crumb.contains("Switched")
        && !crumb.contains("Already on")
        && !crumb.contains("Dirty worktree")
        && focusbox_tree_on_noise(screen)
        && noise_is_checked_out(screen)
        && !graph_cursor_on(screen, "keep-leaf-commit")
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("topic/noise no-upstream")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && status.contains(" tree")
        && status.contains(" split")
        && no_picker_or_wrong_overlays(screen)
}

/// Graph `b` checks out the focused commit's local / `origin/*` ref.
///
/// Docs: Help GIT `b` is `depth 0 picker · graph local/origin/*`.
/// Keymap: graph-focused `b` on a commit is `Action::GraphCheckout`.
/// One local or `origin/*` name checks out (`checkout_branch`). Several
/// names open a picker of those names only (`Checkout at <short>`).
/// Tree `b` is the local picker (`pty_tree_b_picker_enter_checkout`).
/// Tree `d` is default-branch (`pty_d_switches_to_default_branch`).
///
/// After first paint the cursor is already on `focusbox`. Tab focuses
/// the graph. `/` lands on `noise-leaf-commit` (one name `topic/noise`,
/// not HEAD `feature/keep` and not default `main`). `b` must checkout
/// that ref with no picker and no Enter. Git HEAD is `topic/noise` /
/// `noise-leaf-commit`. Tree shows `& topic/noise`. Graph chip is
/// `[+topic/noise]`. Toast is `Checked out topic/noise`, not
/// `Switched 1 repo`. A no-op, tree picker, files drill, `d` onto
/// `main`, overlay-only, toast-only, or the wrong ref is red.
#[test]
fn pty_graph_b_checkout() {
    let (_root, workspace) = focus_workspace();
    let repo = workspace.join("focusbox");
    let keep_sha = git_stdout(&repo, &["rev-parse", "feature/keep"]);
    let main_sha = git_stdout(&repo, &["rev-parse", "main"]);
    let noise_sha = git_stdout(&repo, &["rev-parse", BRANCH]);
    assert_ne!(keep_sha, main_sha);
    assert_ne!(keep_sha, noise_sha);
    assert_ne!(main_sha, noise_sha);
    assert_eq!(head_branch(&repo), "feature/keep");
    assert_eq!(head_subject(&repo), "keep-leaf-commit");
    assert_eq!(head_sha(&repo), keep_sha);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("focusbox", WAIT);
    tui.wait_pred(
        focusbox_graph_left_full,
        "first paint: focusbox on the tree, full graph, checkout idle",
        WAIT,
    );
    assert_head(
        &repo,
        "feature/keep",
        "keep-leaf-commit",
        &keep_sha,
        &tui.screen(),
    );

    tui.tab();
    tui.wait_pred(
        graph_focused_diverged_before_checkout,
        "Tab focuses the graph: keep and noise tips, HEAD still keep, overlay closed",
        GIT_WAIT,
    );
    assert_head(
        &repo,
        "feature/keep",
        "keep-leaf-commit",
        &keep_sha,
        &tui.screen(),
    );

    tui.search(SUBJECT);
    tui.wait_pred(
        noise_leaf_ready_to_checkout,
        "graph cursor on noise-leaf-commit; b checkout hint; overlay closed",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        noise_leaf_ready_to_checkout,
        "noise-leaf ready holds (not a flicker, picker, or files drill)",
        WAIT,
    );
    assert_head(
        &repo,
        "feature/keep",
        "keep-leaf-commit",
        &keep_sha,
        &tui.screen(),
    );

    tui.key('b');
    tui.wait_pred(
        documented_graph_b_checkout,
        "graph b checks out topic/noise at noise-leaf-commit (not picker, not d, not no-op)",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_graph_b_checkout,
        "checked-out paint holds (not a flicker, toast-only tick, tree picker, or wrong ref)",
        WAIT,
    );
    assert_head(&repo, BRANCH, SUBJECT, &noise_sha, &tui.screen());
    assert_eq!(porcelain(&repo), format!("## {BRANCH}"), "{}", tui.screen());
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "feature/keep"]),
        keep_sha,
        "graph b must not move feature/keep:\n{}",
        tui.screen()
    );
    assert_eq!(
        git_stdout(&repo, &["rev-parse", "main"]),
        main_sha,
        "graph b must not check out main (`d`):\n{}",
        tui.screen()
    );
}

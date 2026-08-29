use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{daily_workspace, focus_workspace, git_env};
use crate::support::{
    crumb_row, documented_launch_first_paint, focusbox_graph_left_full, has_fetch_hint,
    not_files_search_or_stash, status_row, tree_cursor_on, tree_has, tree_line_containing,
    GIT_WAIT, SETTLE_MS, WAIT,
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

fn no_wrong_d_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("┌ files")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("Merge ")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && !screen.contains("Pushed")
}

fn has_default_branch_hint(screen: &str) -> bool {
    status_row(screen).contains("default branch")
}

fn focusbox_tree_on_keep(screen: &str) -> bool {
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

/// Graph chips: HEAD is `feature/keep`. Not a gutter `@` / `*` on the subject row.
fn keep_is_checked_out(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("[+feature/keep]")
        && !screen.contains("[+main]")
}

/// Graph chips: HEAD is `main`. Live switch paints `[+main]` + `main-leaf-commit`.
///
/// Do not require a gutter `@` on the `main-leaf-commit` row. That glyph
/// shares a screen line with `@ focusbox` and is not a stable CI oracle.
fn main_is_checked_out(screen: &str) -> bool {
    screen.contains("main-leaf-commit")
        && screen.contains("[+main]")
        && !screen.contains("[+feature/keep]")
}

fn crumb_switched_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Switched 1 repo")
        && !crumb.contains("failed")
        && !crumb.contains("no non-default")
        && crumb.contains("workspace › focusbox")
        && !crumb.contains("[focusbox]")
}

fn crumb_already_on_default(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("no non-default branches to switch")
        && !crumb.contains("Switched")
        && !crumb.contains("failed")
        && crumb.contains("workspace › focusbox")
}

fn crumb_dirty_skip(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Switched 1 repo (1 failed)")
        && !crumb.contains("no non-default")
        && crumb.contains("workspace › focusbox")
}

/// First paint: cursor on clean `focusbox` (`feature/keep`). `d` has not run.
fn idle_focusbox_on_keep(screen: &str) -> bool {
    let status = status_row(screen);
    focusbox_graph_left_full(screen)
        && focusbox_tree_on_keep(screen)
        && keep_is_checked_out(screen)
        && !tree_has(screen, "No updates")
        && !tree_has(screen, "& main")
        && has_default_branch_hint(screen)
        && has_fetch_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !crumb_row(screen).contains("Switched")
        && !crumb_row(screen).contains("no non-default")
        && not_files_search_or_stash(screen)
        && no_wrong_d_overlays(screen)
}

/// Checkout paint after `d`: tree `& main`, graph chip `[+main]`.
fn switched_checkout_paint(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "focusbox")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && focusbox_tree_on_main(screen)
        && tree_has(screen, "No updates")
        && main_is_checked_out(screen)
        && screen.contains("main no-upstream")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
        && !has_default_branch_hint(screen)
        && has_fetch_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && status.contains("focus right")
        && not_files_search_or_stash(screen)
        && no_wrong_d_overlays(screen)
}

/// `d` checked out `main`. Tree left `feature/keep`. Graph chip is `[+main]`.
fn documented_d_switched_to_main(screen: &str) -> bool {
    switched_checkout_paint(screen) && crumb_switched_one(screen)
}

/// Second `d` on already-default. HEAD stays `main`. No second checkout.
fn documented_d_already_on_default(screen: &str) -> bool {
    switched_checkout_paint(screen) && crumb_already_on_default(screen)
}

/// Dirty `keep.txt` after first paint. `d` skipped checkout. HEAD stays keep.
fn documented_d_skips_dirty_keep(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "focusbox")
        && focusbox_tree_on_keep(screen)
        && tree_has(screen, "keep.txt")
        && tree_line_containing(screen, "keep.txt").is_some_and(|line| line.contains('M'))
        && keep_is_checked_out(screen)
        && (screen.contains("uncommitted changes") || screen.contains("Uncommitted changes"))
        && crumb_dirty_skip(screen)
        && has_default_branch_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && no_wrong_d_overlays(screen)
}

/// File-row `d` on daily README. Silent. `app` stays on `main`. `merger` stays off default.
fn documented_d_file_row_silent(screen: &str) -> bool {
    documented_launch_first_paint(screen)
        && !crumb_row(screen).contains("Switched")
        && !crumb_row(screen).contains("no non-default")
        && !crumb_row(screen).contains("failed")
        && tree_has(screen, "feature/graph")
        && tree_cursor_on(screen, "README.md")
}

/// `d` switches a clean non-default checkout to its default branch.
///
/// Docs: Help GIT `d` is `default branch`. Keymap / status: `d` /
/// "default branch" on workspace, repo, and checkout rows. Configuration:
/// focused checkout (or primaries on workspace / family) off default;
/// already-default is a no-op and does not pull; dirty trees skip via
/// `repo_has_local_changes`; file and dir rows are a silent no-op.
///
/// Live PTY after first paint (cursor already on `focusbox`,
/// `feature/keep`): raw `d` ran `git checkout` of `main`. Git HEAD is
/// `main` / `main-leaf-commit`. Tree shows `& main` and leaves
/// `feature/keep`. Graph shows `main-leaf-commit` and `[+main]` (not a
/// gutter `@` on that subject row). Toast is `Switched 1 repo`. The `d`
/// hint goes. A second `d` toasts
/// `no non-default branches to switch` and HEAD stays `main`.
///
/// Dirty `keep.txt` with the repo focused: `d` toasts `Switched 1 repo
/// (1 failed)` and HEAD stays `feature/keep`. File-row `d` on daily
/// README is silent: `app` stays `main`, `merger` stays `feature/graph`.
/// `/` search, a toast-only tick, or still-on-keep cannot pass.
#[test]
fn pty_d_switches_to_default_branch() {
    let (_root, workspace) = focus_workspace();
    let repo = workspace.join("focusbox");
    assert_eq!(head_branch(&repo), "feature/keep");
    assert_eq!(head_subject(&repo), "keep-leaf-commit");
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("focusbox", WAIT);
    tui.wait_pred(
        idle_focusbox_on_keep,
        "first paint: focusbox on feature/keep, graph HEAD keep-leaf, d hint",
        WAIT,
    );
    assert_head(&repo, "feature/keep", "keep-leaf-commit", &tui.screen());

    tui.key('d');
    tui.wait_pred(
        documented_d_switched_to_main,
        "d checks out main: Switched 1 repo, tree & main, [+main]",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_d_switched_to_main,
        "switched paint holds (not a flicker, toast-only tick, or still feature/keep)",
        WAIT,
    );
    assert_head(&repo, "main", "main-leaf-commit", &tui.screen());
    assert_eq!(porcelain(&repo), "## main", "{}", tui.screen());

    tui.key('d');
    tui.wait_pred(
        documented_d_already_on_default,
        "second d on main: no non-default branches to switch; HEAD stays main",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_d_already_on_default,
        "already-default refuse holds (not a second checkout or toast-only tick)",
        WAIT,
    );
    assert_head(&repo, "main", "main-leaf-commit", &tui.screen());
    assert_eq!(porcelain(&repo), "## main", "{}", tui.screen());

    let (_root_dirty, workspace_dirty) = focus_workspace();
    let dirty_repo = workspace_dirty.join("focusbox");
    let mut dirty_tui = PtySession::open(&workspace_dirty);
    dirty_tui.wait_contains("focusbox", WAIT);
    dirty_tui.wait_pred(
        idle_focusbox_on_keep,
        "dirty session first paint: clean focusbox on feature/keep (`d` has not run)",
        WAIT,
    );
    fs::write(dirty_repo.join("keep.txt"), "keep\ndirty\n").unwrap();
    assert!(
        porcelain(&dirty_repo).contains("M keep.txt"),
        "tracked keep.txt must be dirty before d:\n{}",
        porcelain(&dirty_repo)
    );
    assert_head(
        &dirty_repo,
        "feature/keep",
        "keep-leaf-commit",
        &dirty_tui.screen(),
    );
    dirty_tui.wait_ms(SETTLE_MS);
    dirty_tui.key('d');
    dirty_tui.wait_pred(
        documented_d_skips_dirty_keep,
        "d on dirty keep: Switched 1 repo (1 failed); HEAD stays feature/keep",
        GIT_WAIT,
    );
    dirty_tui.wait_ms(SETTLE_MS);
    dirty_tui.wait_pred(
        documented_d_skips_dirty_keep,
        "dirty skip holds (not a checkout of main or a silent no-op)",
        WAIT,
    );
    assert_head(
        &dirty_repo,
        "feature/keep",
        "keep-leaf-commit",
        &dirty_tui.screen(),
    );
    let dirty_status = porcelain(&dirty_repo);
    assert!(
        dirty_status.contains("## feature/keep") && dirty_status.contains("M keep.txt"),
        "dirty skip must leave feature/keep and keep.txt dirty:\n{dirty_status}\n{}",
        dirty_tui.screen()
    );

    let (_root_file, workspace_file) = daily_workspace();
    let app = workspace_file.join("app");
    let merger = workspace_file.join("merger");
    assert_eq!(head_branch(&app), "main");
    assert_eq!(head_branch(&merger), "feature/graph");
    let mut file_tui = PtySession::open(&workspace_file);
    file_tui.wait_pred(
        documented_d_file_row_silent,
        "daily first paint: cursor on dirty README (`d` has not run)",
        WAIT,
    );

    file_tui.key('d');
    file_tui.wait_pred(
        documented_d_file_row_silent,
        "file-row d is silent: README file-diff, merger still feature/graph",
        WAIT,
    );
    file_tui.wait_ms(SETTLE_MS);
    file_tui.wait_pred(
        documented_d_file_row_silent,
        "file-row silence holds (not a workspace switch of merger or a toast)",
        WAIT,
    );
    assert_eq!(head_branch(&app), "main", "{}", file_tui.screen());
    assert_eq!(
        head_branch(&merger),
        "feature/graph",
        "file-row d must not switch merger:\n{}",
        file_tui.screen()
    );
    assert!(
        porcelain(&app).contains("M README.md"),
        "file-row d must not revert README:\n{}\n{}",
        porcelain(&app),
        file_tui.screen()
    );
}

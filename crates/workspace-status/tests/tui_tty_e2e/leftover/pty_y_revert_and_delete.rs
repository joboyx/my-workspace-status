use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::{left_tree, PtySession};
use crate::seed::{daily_workspace, git_env};
use crate::support::{
    crumb_row, has_stage_hint, no_wrong_overlays, pane_unstaged_readme, readme_unstaged_badge,
    status_row, tree_cursor_on, tree_has, tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};

const UNTRACKED: &str = "new.txt";
const README_COMMITTED: &str = "# app\n";
const README_DIRTY: &str = "# app\ndirty\n";

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
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn porcelain(repo: &Path) -> String {
    git_stdout(repo, &["status", "--porcelain"])
}

fn help_compact(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn untracked_a_badge(screen: &str) -> bool {
    tree_line_containing(screen, UNTRACKED)
        .is_some_and(|line| line.contains("A ") && !line.contains('\u{258C}'))
}

fn no_y_apply_path(screen: &str) -> bool {
    !screen.contains("reverted")
        && !screen.contains("deleted README")
        && !screen.contains(&format!("deleted {UNTRACKED}"))
        && !screen.contains("Working tree clean")
        && !screen.contains("working tree clean")
}

fn no_wrong_revert_overlays(screen: &str) -> bool {
    no_wrong_overlays(screen)
        && !screen.contains("Drop ")
        && !screen.contains("Remove worktree")
        && !screen.contains("Create branch")
        && !screen.contains("Merge ")
        && !screen.contains("nothing to discard")
        && !screen.contains("Nothing to discard")
        && !screen.contains("focus a file")
}

fn mixed_dirty_readme_and_untracked(screen: &str) -> bool {
    let left = left_tree(screen);
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, UNTRACKED)
        && tree_has(screen, "README.md")
        && tree_has(screen, UNTRACKED)
        && tree_has(screen, "app")
        && left.contains("2 changed")
        && !left.contains("1 changed")
        && readme_unstaged_badge(screen)
        && untracked_a_badge(screen)
        && pane_unstaged_readme(screen)
        && has_stage_hint(screen)
        && no_wrong_revert_overlays(screen)
        && no_y_apply_path(screen)
}

/// Help GIT lists `x` as revert (`y`/`Y`). Overlay is not the apply path.
fn documented_help_lists_y_revert_delete(screen: &str) -> bool {
    let compact = help_compact(screen);
    compact.contains("GIT")
        && compact.contains("revert (y/Y)")
        && compact.contains("MOVE")
        && compact.contains("VIEW")
        && compact.contains("stage scope")
}

/// Tree-focused expanded `app`. Mixed dirty files stay. Graph loaded.
fn app_repo_mixed_dirty(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, UNTRACKED)
        && tree_has(screen, "README.md")
        && tree_has(screen, UNTRACKED)
        && readme_unstaged_badge(screen)
        && untracked_a_badge(screen)
        && screen.contains("uncommitted changes")
        && !screen.contains("UNSTAGED")
        && crumb.contains("workspace › app")
        && !crumb.contains("[app]")
        && no_wrong_revert_overlays(screen)
        && no_y_apply_path(screen)
}

/// Boxed `x` confirm on mixed scope: `y` tracked only, `Y` also deletes.
///
/// Confirm owns the bottom rows, so do not read `crumb_row` here.
fn documented_revert_confirm_mixed_y_deletes(screen: &str) -> bool {
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, UNTRACKED)
        && tree_has(screen, "README.md")
        && tree_has(screen, UNTRACKED)
        && readme_unstaged_badge(screen)
        && untracked_a_badge(screen)
        && screen.contains("Revert app?")
        && screen.contains("1 tracked file")
        && screen.contains("discarded")
        && screen.contains("1 untracked file")
        && screen.contains("kept")
        && screen.contains("revert + delete untracked")
        && screen.contains("cancel")
        && !screen.contains("revert cancelled")
        && no_wrong_revert_overlays(screen)
        && no_y_apply_path(screen)
}

/// CSI-u Shift+Y applied mixed revert+delete. Overlay gone. Toast is counts.
fn documented_y_reverted_and_deleted(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    crumb.contains("reverted 1 tracked, 1 untracked")
        && !crumb.contains("revert cancelled")
        && !crumb.contains("reverted README")
        && !crumb.contains(&format!("deleted {UNTRACKED}"))
        && !screen.contains("Revert app?")
        && !screen.contains("revert + delete untracked")
        && !screen.contains("1 tracked file")
        && !tree_has(screen, UNTRACKED)
        && !readme_unstaged_badge(screen)
        && !pane_unstaged_readme(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && no_wrong_revert_overlays(screen)
}

fn assert_mixed_dirty_on_disk(app: &Path, untracked: &Path, screen: &str) {
    let status = porcelain(app);
    assert!(
        status.contains("README.md") && status.contains(UNTRACKED),
        "app must still be mixed dirty before Y:\n{status}\n{screen}"
    );
    assert!(
        untracked.is_file(),
        "untracked {UNTRACKED} must exist before Y:\n{status}\n{screen}"
    );
    assert_eq!(
        fs::read_to_string(app.join("README.md")).unwrap(),
        README_DIRTY,
        "README must still be dirty before Y:\n{status}\n{screen}"
    );
}

fn assert_mixed_clean_on_disk(app: &Path, untracked: &Path, screen: &str) {
    let status = porcelain(app);
    assert!(
        status.trim().is_empty(),
        "Y must leave a clean worktree (not y-keep-untracked, not a no-op):\n{status}\n{screen}"
    );
    assert!(
        !untracked.exists(),
        "Y must delete untracked {UNTRACKED}, not only paint:\n{status}\n{screen}"
    );
    assert!(
        app.join("README.md").is_file(),
        "Y must restore README, not delete it:\n{status}\n{screen}"
    );
    assert_eq!(
        fs::read_to_string(app.join("README.md")).unwrap(),
        README_COMMITTED,
        "Y must git-restore tracked README:\n{status}\n{screen}"
    );
}

/// CSI-u Shift+Y on mixed revert confirm restores tracked and deletes untracked.
///
/// Docs: Help GIT `x` is revert (`y`/`Y`). Configuration: `x` confirms
/// with counts (`y` tracked only, `Y` also deletes untracked). Keymap:
/// confirm `Y` is `Action::ConfirmYesClean` (`git restore` tracked plus
/// `git clean -f` per untracked path). Live TUI reads Shift+Y as CSI-u,
/// not a raw `'Y'` byte.
///
/// Daily seed plus untracked `new.txt`. `k` focuses `app` so `x` is mixed
/// scope (file-row `Y` cannot prove delete). Git truth is the oracle: a
/// toast, overlay-only paint, `y` keep-untracked, no-op, or wrong path
/// is red. `pty_revert_confirm_n_cancels` owns `n`. `pty_x_then_y_discard_tracked`
/// owns plain `y`.
#[test]
fn pty_y_revert_and_delete() {
    let (_root, workspace) = daily_workspace();
    let app = workspace.join("app");
    let untracked = app.join(UNTRACKED);
    fs::write(&untracked, "delete me\n").unwrap();

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains(UNTRACKED, GIT_WAIT);
    tui.wait_pred(
        mixed_dirty_readme_and_untracked,
        "first paint: README focused, new.txt untracked A, 2 changed",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        documented_help_lists_y_revert_delete,
        "help GIT lists x revert (y/Y)",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        mixed_dirty_readme_and_untracked,
        "Esc closes help; mixed dirty README still focused",
        WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        app_repo_mixed_dirty,
        "k focuses expanded app; mixed dirty files stay; graph loaded",
        GIT_WAIT,
    );

    tui.key('x');
    tui.wait_pred(
        documented_revert_confirm_mixed_y_deletes,
        "x arms Revert app? with 1 tracked / 1 untracked kept; Y deletes",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_revert_confirm_mixed_y_deletes,
        "mixed revert confirm holds (not a flicker, y path, or toast-only tick)",
        WAIT,
    );
    assert_mixed_dirty_on_disk(&app, &untracked, &tui.screen());

    tui.shift_letter('Y');
    tui.wait_pred(
        documented_y_reverted_and_deleted,
        "CSI-u Shift+Y: reverted 1 tracked, 1 untracked; overlay gone; new.txt off tree",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_y_reverted_and_deleted,
        "Y apply holds (not a flicker, y keep-untracked, or overlay return)",
        WAIT,
    );
    assert_mixed_clean_on_disk(&app, &untracked, &tui.screen());
}

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
const HEAD_README: &str = "# app\n";
const DIRTY_README: &str = "# app\ndirty\n";

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

fn worktree_readme(repo: &Path) -> String {
    fs::read_to_string(repo.join("README.md")).expect("README.md")
}

fn untracked_a_badge(screen: &str) -> bool {
    tree_line_containing(screen, UNTRACKED)
        .is_some_and(|line| line.contains("A ") && !line.contains('\u{258C}'))
}

fn no_y_apply_or_delete(screen: &str) -> bool {
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
        && no_y_apply_or_delete(screen)
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
        && no_y_apply_or_delete(screen)
}

/// Boxed `x` confirm on mixed repo scope: `y` keeps untracked, `Y` would delete.
///
/// Confirm owns the bottom rows, so do not read `crumb_row` here.
fn documented_revert_confirm_mixed_y_keeps(screen: &str) -> bool {
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, UNTRACKED)
        && tree_has(screen, "README.md")
        && tree_has(screen, UNTRACKED)
        && readme_unstaged_badge(screen)
        && untracked_a_badge(screen)
        && screen.contains("Revert app?")
        && !screen.contains("Revert README.md?")
        && screen.contains("1 tracked file")
        && screen.contains("discarded")
        && screen.contains("1 untracked file")
        && screen.contains("kept")
        && !screen.contains("0 untracked")
        && screen.contains("revert + delete untracked")
        && screen.contains("cancel")
        && !screen.contains("revert cancelled")
        && no_wrong_revert_overlays(screen)
        && no_y_apply_or_delete(screen)
}

/// CSI-u `y` restored tracked README and kept untracked `new.txt`.
///
/// `Y` would toast counts, drop `new.txt`, and leave `0 changed`.
fn documented_y_discarded_tracked_kept_untracked(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    let left = left_tree(screen);
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, UNTRACKED)
        && !tree_cursor_on(screen, "No updates")
        && !tree_has(screen, "README.md")
        && tree_has(screen, UNTRACKED)
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_has(screen, "No updates")
        && left.contains("1 changed")
        && !left.contains("2 changed")
        && !left.contains("0 changed")
        && untracked_a_badge(screen)
        && !readme_unstaged_badge(screen)
        && !pane_unstaged_readme(screen)
        && crumb.contains("reverted README.md")
        && crumb.contains("workspace › app")
        && !crumb.contains("reverted 1 tracked")
        && !crumb.contains("1 untracked")
        && !crumb.contains("revert cancelled")
        && !crumb.contains("deleted")
        && !screen.contains("Revert app?")
        && !screen.contains("Revert README.md?")
        && !screen.contains("revert + delete untracked")
        && !screen.contains("1 tracked file")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && screen.contains("uncommitted changes")
        && status.contains(" tree")
        && status.contains(" split")
        && status.contains("? help")
        && status.contains("focus right")
        && has_stage_hint(screen)
        && status.contains("revert")
        && no_wrong_revert_overlays(screen)
}

fn assert_mixed_dirty_on_disk(app: &Path, untracked: &Path, screen: &str) {
    let status = porcelain(app);
    assert!(
        status.contains("README.md") && status.contains(UNTRACKED),
        "app must still be mixed dirty before y:\n{status}\n{screen}"
    );
    assert!(
        untracked.is_file(),
        "untracked {UNTRACKED} must exist before y:\n{status}\n{screen}"
    );
    assert_eq!(
        worktree_readme(app),
        DIRTY_README,
        "README must still be dirty before y:\n{status}\n{screen}"
    );
}

fn assert_y_kept_untracked_on_disk(app: &Path, untracked: &Path, screen: &str) {
    let status = porcelain(app);
    assert!(
        !status.contains("README.md"),
        "y must restore tracked README (not a no-op):\n{status}\n{screen}"
    );
    assert!(
        status.contains(UNTRACKED),
        "y must keep untracked {UNTRACKED} (Y would delete it):\n{status}\n{screen}"
    );
    assert!(
        untracked.is_file(),
        "y must leave untracked {UNTRACKED} on disk (Y would delete it):\n{status}\n{screen}"
    );
    assert_eq!(
        fs::read_to_string(untracked).unwrap(),
        "keep me\n",
        "y must not rewrite untracked {UNTRACKED}:\n{status}\n{screen}"
    );
    assert_eq!(
        worktree_readme(app),
        HEAD_README,
        "y must git-restore tracked README to HEAD:\n{status}\nworktree:\n{}\n{screen}",
        worktree_readme(app)
    );
}

/// CSI-u `x` then CSI-u `y` on mixed repo scope restores tracked and keeps untracked.
///
/// Docs: Help GIT `x` is revert (`y`/`Y`). Configuration: `x` confirms
/// with counts (`y` tracked only, `Y` also deletes untracked). Keymap:
/// `x` is `Action::Revert`; confirm `y` / Enter is `Action::ConfirmYes`
/// (`git restore`, toast `reverted …`). Live TUI reads `x` / `y` as CSI-u.
///
/// Daily seed plus untracked `new.txt`. `k` focuses `app` so `x` is mixed
/// scope. File-row `y` and `Y` both restore the sole tracked README, so
/// that path cannot prove `y` is not `Y`. Git truth is the oracle: porcelain
/// still lists `new.txt` after `y`. A no-op, `n` cancel, Shift+Y delete,
/// overlay-only paint, or toast-only tick is red. `pty_y_revert_and_delete`
/// owns Shift+Y. `pty_revert_confirm_n_cancels` owns `n`.
#[test]
fn pty_x_then_y_discard_tracked() {
    let (_root, workspace) = daily_workspace();
    let app = workspace.join("app");
    let untracked = app.join(UNTRACKED);
    fs::write(&untracked, "keep me\n").unwrap();
    assert_eq!(
        worktree_readme(&app),
        DIRTY_README,
        "seed must dirty tracked README.md"
    );
    assert!(
        porcelain(&app).contains("README.md") && porcelain(&app).contains(UNTRACKED),
        "seed git must be mixed dirty:\n{}",
        porcelain(&app)
    );

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains(UNTRACKED, GIT_WAIT);
    tui.wait_pred(
        mixed_dirty_readme_and_untracked,
        "first paint: README focused, new.txt untracked A, 2 changed",
        WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        app_repo_mixed_dirty,
        "k focuses expanded app; mixed dirty files stay; graph loaded",
        GIT_WAIT,
    );

    tui.letter_press('x');
    tui.wait_pred(
        documented_revert_confirm_mixed_y_keeps,
        "CSI-u x arms Revert app? with 1 tracked / 1 untracked kept",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_revert_confirm_mixed_y_keeps,
        "mixed revert confirm holds (not a flicker, y path, or toast-only tick)",
        WAIT,
    );
    assert_mixed_dirty_on_disk(&app, &untracked, &tui.screen());

    tui.letter_press('y');
    tui.wait_pred(
        documented_y_discarded_tracked_kept_untracked,
        "CSI-u y restores README, keeps new.txt, overlay gone, not Y-delete",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_y_discarded_tracked_kept_untracked,
        "y keep-untracked paint holds (not a flicker, Y-delete, or overlay return)",
        WAIT,
    );
    assert_y_kept_untracked_on_disk(&app, &untracked, &tui.screen());
}

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git_env};
use crate::support::{
    crumb_row, idle_dirty_readme_unstaged, no_wrong_overlays, pane_unstaged_readme,
    readme_unstaged_badge, status_row, tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT,
};

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
    git_stdout(repo, &["status", "-sb"])
}

fn head_readme(repo: &Path) -> String {
    git_stdout(repo, &["show", "HEAD:README.md"])
}

fn worktree_readme(repo: &Path) -> String {
    fs::read_to_string(repo.join("README.md")).expect("README.md")
}

fn readme_is_dirty(repo: &Path) -> bool {
    porcelain(repo).contains("README.md") && worktree_readme(repo) == DIRTY_README
}

fn readme_matches_head(repo: &Path) -> bool {
    let head = head_readme(repo);
    worktree_readme(repo) == head
        && head == HEAD_README
        && !porcelain(repo).contains("README.md")
}

fn no_y_delete_or_cancel(screen: &str) -> bool {
    !screen.contains("deleted README")
        && !crumb_row(screen).contains("revert cancelled")
        && !crumb_row(screen).contains("deleted")
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

fn dirty_readme_still_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
}

/// Boxed `x` confirm: counted revert, `y`/`Y`/`n`. File is still dirty.
fn documented_revert_confirm_armed(screen: &str) -> bool {
    dirty_readme_still_focused(screen)
        && screen.contains("Revert README.md?")
        && screen.contains("1 tracked file")
        && screen.contains("discarded")
        && screen.contains("0 untracked files")
        && screen.contains("kept")
        && screen.contains("revert + delete untracked")
        && screen.contains("cancel")
        && !screen.contains("reverted")
        && !screen.contains("revert cancelled")
        && no_y_delete_or_cancel(screen)
        && no_wrong_revert_overlays(screen)
}

/// Confirm `y` ran `git restore`. Overlay-only / cancel / Y-delete cannot pass.
fn documented_y_discarded_tracked(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_has(screen, "README.md")
        && !tree_has(screen, "app")
        && tree_has(screen, "No updates")
        && tree_has(screen, "merger")
        && tree_has(screen, "0 changed")
        && crumb.contains("reverted README.md")
        && !crumb.contains("revert cancelled")
        && !crumb.contains("deleted")
        && !screen.contains("Revert README.md?")
        && !screen.contains("revert + delete untracked")
        && !screen.contains("1 tracked file")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && screen.contains("focus a repo for the graph")
        && status.contains(" tree")
        && status.contains(" split")
        && status.contains("? help")
        && status.contains("focus right")
        && !status.contains("stage")
        && !status.contains("revert")
        && no_y_delete_or_cancel(screen)
        && no_wrong_revert_overlays(screen)
}

/// CSI-u `x` arms boxed revert confirm; CSI-u `y` discards the tracked file.
///
/// Docs: Help GIT `x` is revert (`y`/`Y`). Configuration: `x` confirms
/// with counts (`y` tracked only, `Y` also deletes untracked); `n` / Esc
/// cancel. Keymap: `x` is `Action::Revert` (opens `PendingConfirm::Revert`);
/// confirm `y` / Enter is `Action::ConfirmYes` (`git restore`, toast
/// `reverted …`).
///
/// Live PTY after first paint (cursor already on dirty README): CSI-u `x`
/// (`CSI 120 ; 1 : 1 u`) paints `Revert README.md?` with 1 tracked →
/// discarded, 0 untracked → kept, `y` revert / `Y` revert + delete / `n`
/// cancel. Git still has the dirty README. CSI-u `y` (`CSI 121 ; 1 : 1 u`)
/// closes the overlay, toasts `reverted README.md`, drops the file from
/// the tree (`0 changed`, cursor on folded No updates, empty graph). Git
/// `README.md` matches `HEAD` (`# app`). A no-op, immediate restore, `n`
/// cancel, `Y` delete, overlay-only paint, or toast-only tick cannot pass.
///
/// After first paint the cursor is already on the dirty README. Do not
/// `/` search (`y` would be a search char if confirm never armed).
#[test]
fn pty_x_then_y_discard_tracked() {
    let (_root, workspace) = daily_workspace();
    let app = workspace.join("app");
    assert_eq!(
        worktree_readme(&app),
        DIRTY_README,
        "seed must dirty tracked README.md"
    );
    assert_eq!(head_readme(&app), HEAD_README, "HEAD README must be the seed");
    assert!(
        readme_is_dirty(&app),
        "seed git must show dirty README.md:\n{}",
        porcelain(&app)
    );

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unstaged,
        "first paint: cursor on dirty README, unstaged, no confirm",
        WAIT,
    );

    tui.letter_press('x');
    tui.wait_pred(
        documented_revert_confirm_armed,
        "CSI-u x arms Revert README.md? with y/Y/n; file stays dirty",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_revert_confirm_armed,
        "revert confirm holds (not a flicker, y path, or toast-only tick)",
        WAIT,
    );
    assert!(
        readme_is_dirty(&app),
        "confirm must not git restore until y:\n{}\n{}",
        porcelain(&app),
        tui.screen()
    );

    tui.letter_press('y');
    tui.wait_pred(
        documented_y_discarded_tracked,
        "CSI-u y discards tracked README: reverted toast, file leaves tree, overlay gone",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_y_discarded_tracked,
        "discarded paint holds (not a flicker, cancel, or overlay return)",
        WAIT,
    );
    assert!(
        readme_matches_head(&app),
        "git README.md must match HEAD after y:\nporcelain:\n{}\nworktree:\n{}\nHEAD:\n{}\nscreen:\n{}",
        porcelain(&app),
        worktree_readme(&app),
        head_readme(&app),
        tui.screen()
    );
}

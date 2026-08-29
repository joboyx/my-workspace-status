use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git_env};
use crate::support::{documented_launch_first_paint, status_row, SETTLE_MS, WAIT};

fn local_branch_names(repo: &Path) -> String {
    let mut cmd = Command::new("git");
    cmd.args(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .current_dir(repo);
    for (k, v) in git_env() {
        cmd.env(k, v);
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let out = cmd.output().expect("git for-each-ref");
    assert!(
        out.status.success(),
        "git for-each-ref failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Wrong `c` results: graph create-branch, picker `C`, commit overlay.
fn tree_file_c_is_not_create_or_commit(screen: &str) -> bool {
    !screen.contains("Create branch")
        && !status_row(screen).contains("create branch")
        && !screen.contains("commit message")
        && !screen.contains("Branch ")
        && !screen.contains("Checkout ")
        && !screen.contains("name: …")
        && !screen.contains("filter:")
}

/// Documented idle file-diff after tree-file `c`. Overlay or graph cannot pass.
fn tree_file_c_left_file_diff(screen: &str) -> bool {
    documented_launch_first_paint(screen) && tree_file_c_is_not_create_or_commit(screen)
}

/// `?` after tree-file `c` is help, not a commit or create-branch overlay.
fn help_after_tree_file_c(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("GIT")
        && screen.contains("VIEW")
        && screen.contains("create (in picker)")
        && screen.contains("graph merge into HEAD")
        && !screen.contains("Create branch")
        && !screen.contains("commit message")
        && !screen.contains("name: …")
}

/// `c` on a focused tree file is not create-branch and not commit.
///
/// Docs: `c` creates a branch on a focused graph commit (name overlay,
/// ref only, no checkout). It is a no-op on a tree, file, or workspace
/// row. Help GIT lists picker `C` ("create (in picker)"); lowercase `c`
/// is not a commit key. Keymap shows `c` / "create branch" only on a
/// graph commit.
///
/// Live PTY after first paint (cursor already on dirty README, file
/// diff on the right): `c` left that paint. Status kept file hints
/// (stage / revert / edit / reviewed), not "create branch". No Create
/// branch overlay, no picker `C`, no commit overlay. `app` still had
/// only `main`. `?` still opened help with picker `C`.
///
/// Graph create-branch (`pty_graph_c_creates_branch_at_commit`) and
/// picker `C` (`pty_branch_picker_shift_c_creates`) are other leftovers.
/// `/README` then "Create branch" absent, a paint-only tick, or a silent
/// new ref cannot pass.
#[test]
fn pty_c_on_tree_file_is_not_commit() {
    let (_root, workspace) = daily_workspace();
    let app = workspace.join("app");
    let refs_before = local_branch_names(&app);
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        tree_file_c_left_file_diff,
        "first paint: dirty README file-diff (`c` has not run)",
        WAIT,
    );

    tui.key('c');
    tui.wait_pred(
        tree_file_c_left_file_diff,
        "tree-file `c` stays on the README file-diff; not create-branch, picker C, or commit",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        tree_file_c_left_file_diff,
        "file-diff after tree-file `c` holds (not a flicker or overlay tick)",
        WAIT,
    );
    assert_eq!(
        local_branch_names(&app),
        refs_before,
        "tree-file `c` must not create a git ref:\n{}",
        tui.screen()
    );

    tui.key('?');
    tui.wait_pred(
        help_after_tree_file_c,
        "`?` after tree-file `c` opens help (picker C); not create-branch or commit",
        WAIT,
    );
}

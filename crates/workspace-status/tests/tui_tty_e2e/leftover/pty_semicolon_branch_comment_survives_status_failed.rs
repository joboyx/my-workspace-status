use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{git, seed_repo, unique_root};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT};

const BODY: &str = "status-failed-keep-e2e";
const BRANCH: &str = "topic/keep";

fn comment_store(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".e2e-state").join("comments.json")
}

fn store_text(workspace: &Path) -> String {
    fs::read_to_string(comment_store(workspace)).unwrap_or_default()
}

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
        && !screen.contains("MOVE")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save") && !screen.contains("empty deletes")
}

fn export_overlay(screen: &str) -> bool {
    screen.contains("# Comments")
        && screen.contains("copied to clipboard")
        && screen.contains("copied · Esc close")
        && !screen.contains("MOVE")
}

fn make_status_fail(repo: &Path) {
    let index = repo.join(".git").join("index");
    let mut perms = fs::metadata(&index).expect("index").permissions();
    perms.set_mode(0o000);
    fs::set_permissions(&index, perms).expect("chmod index");
}

fn copy_tree(src: &Path, dst: &Path) {
    fs::create_dir_all(dst).unwrap();
    for entry in fs::read_dir(src).unwrap() {
        let entry = entry.unwrap();
        let to = dst.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&entry.path(), &to);
        } else {
            fs::copy(entry.path(), &to).unwrap_or_else(|err| {
                panic!("copy {} -> {}: {err}", entry.path().display(), to.display())
            });
        }
    }
}

fn store_kept_branch_comment(workspace: &Path) {
    let stored = store_text(workspace);
    assert!(
        stored.contains(BODY) && stored.contains("\"kind\": \"branch\"") && stored.contains(BRANCH),
        "branch comment must survive a failed-status snapshot:\n{stored}"
    );
}

/// Branch comments stay when `git status` fails. A lock or unreadable
/// index is not "the branch is gone."
///
/// Docs: failed porcelain is `status failed` with empty `local_branches`
/// and branch `(unknown)`. Watch apply and launch GC must not wipe a
/// saved branch comment. A no-op overlay or a store that drops the body
/// is red.
#[test]
fn pty_semicolon_branch_comment_survives_status_failed() {
    let root = unique_root("ws-tui-e2e-comment-status-failed");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    let app = workspace.join("app");
    git(&app, &["branch", BRANCH]);

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "app") && tree_has(screen, "README.md"),
        "launch shows the dirty app checkout",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && tree_has(screen, "README.md"),
        "k focuses default-branch app (exactly one non-default)",
        WAIT,
    );
    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on the attached branch",
        WAIT,
    );
    tui.keys(BODY);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the branch object comment",
        WAIT,
    );
    store_kept_branch_comment(&workspace);

    let launch_root = unique_root("ws-tui-e2e-comment-status-failed-launch");
    let launch_ws = launch_root.join("workspace");
    copy_tree(&workspace, &launch_ws);
    assert!(
        launch_ws.is_dir(),
        "launch copy must exist: {}",
        launch_ws.display()
    );

    make_status_fail(&app);
    tui.key('r');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && tree_has(screen, "app")
                && (screen.contains("(unknown)") || screen.contains("status failed"))
        },
        "r apply shows status failed / unknown HEAD",
        GIT_WAIT,
    );
    store_kept_branch_comment(&workspace);
    tui.key('y');
    tui.wait_pred(
        |screen| export_overlay(screen) && screen.contains(BODY) && !screen.contains("No comments"),
        "y export still lists the branch comment after status failed",
        WAIT,
    );
    drop(tui);

    make_status_fail(&launch_ws.join("app"));
    let mut tui = PtySession::open(&launch_ws);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "app")
                && (screen.contains("(unknown)") || screen.contains("status failed"))
        },
        "relaunch still shows the failed-status checkout",
        WAIT,
    );
    store_kept_branch_comment(&launch_ws);
    tui.key('y');
    tui.wait_pred(
        |screen| export_overlay(screen) && screen.contains(BODY) && !screen.contains("No comments"),
        "launch GC must keep the branch comment after status failed",
        WAIT,
    );
}

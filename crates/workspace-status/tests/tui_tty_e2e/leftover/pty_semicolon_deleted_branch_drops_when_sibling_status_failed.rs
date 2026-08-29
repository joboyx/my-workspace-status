use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::harness::PtySession;
use crate::seed::{git, seed_primary_and_linked_family, unique_root};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT};

const BODY: &str = "sibling-failed-drop-e2e";
const BRANCH: &str = "doomed";
const PRIMARY: &str = "feature/primary-open";
const LINKED: &str = "feature/linked-open";

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

fn linked_index(linked: &Path) -> PathBuf {
    let gitfile = fs::read_to_string(linked.join(".git")).expect("linked gitfile");
    let raw = gitfile
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))
        .map(str::trim)
        .expect("gitdir line");
    let gitdir = if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        linked.join(raw)
    };
    gitdir.join("index")
}

fn make_status_fail(index: &Path) {
    let mut perms = fs::metadata(index).expect("index").permissions();
    perms.set_mode(0o000);
    fs::set_permissions(index, perms).expect("chmod index");
}

/// A status-failed linked checkout must not keep a branch the healthy
/// primary already dropped.
///
/// Docs: skip-wipe and last-good carry apply only when every checkout of
/// that identity has an empty counted list. Two checkouts, `git branch -D`
/// on the healthy primary, failed sibling, refresh: the deleted-branch
/// comment must be gone. A leftover that still finds it is red.
#[test]
fn pty_semicolon_deleted_branch_drops_when_sibling_status_failed() {
    let root = unique_root("ws-tui-e2e-comment-sibling-failed");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_primary_and_linked_family(&workspace);
    let app = workspace.join("app");
    let linked = app.join(".worktrees").join("feat");
    git(&app, &["checkout", "-q", "-b", BRANCH]);

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "app") && tree_has(screen, LINKED) && tree_has(screen, BRANCH),
        "launch shows family with doomed primary and linked checkout",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, BRANCH) && !tree_cursor_on(screen, LINKED),
        "j focuses the primary checkout on doomed",
        WAIT,
    );
    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on the doomed branch",
        WAIT,
    );
    tui.keys(BODY);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the doomed branch comment",
        WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && stored.contains("\"kind\": \"branch\"") && stored.contains(BRANCH),
        "doomed branch comment must persist before delete:\n{stored}"
    );

    git(&app, &["checkout", "-q", PRIMARY]);
    git(&app, &["branch", "-D", BRANCH]);
    make_status_fail(&linked_index(&linked));

    tui.gg();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "workspace") && overlay_closed(screen),
        "gg focuses the workspace root for a full refresh",
        WAIT,
    );
    tui.key('r');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && tree_has(screen, "app")
                && (screen.contains("(unknown)") || screen.contains("status failed"))
        },
        "workspace r shows the failed linked checkout",
        GIT_WAIT,
    );
    tui.key('y');
    tui.wait_pred(
        |screen| export_overlay(screen) && !screen.contains(BODY) && screen.contains("No comments"),
        "y export must omit the deleted doomed comment",
        WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        !stored.contains(BODY) && !stored.contains(BRANCH),
        "GC must drop the doomed branch comment while a sibling status-failed:\n{stored}"
    );
}

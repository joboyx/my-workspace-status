use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git, git_env, seed_repo, unique_root, worktree_workspace};
use crate::support::{
    documented_launch_first_paint, merger_graph_drilled_right, merger_graph_left_unfocused,
    tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT,
};

const BRANCH_BODY: &str = "branch-obj-note-e2e";
const COMMIT_BODY: &str = "commit-obj-note-e2e";
const WORKTREE_BODY: &str = "worktree-obj-note-e2e";
const ATTACH_BODY: &str = "doomed-attach-note-e2e";
const LINKED_BRANCH: &str = "feature/linked-open";

fn comment_store(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".e2e-state").join("comments.json")
}

fn store_text(workspace: &Path) -> String {
    fs::read_to_string(comment_store(workspace)).unwrap_or_default()
}

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

fn no_comment_overlay(screen: &str) -> bool {
    overlay_closed(screen) && !screen.contains("Enter save")
}

fn save_overlay_body(tui: &mut PtySession, body: &str) {
    tui.key(';');
    tui.wait_pred(comment_overlay, "; opens Comment overlay", WAIT);
    tui.keys(body);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the object comment",
        WAIT,
    );
}

/// Object comments on commit / branch / worktree, repo-root no-op,
/// default-branch attach, GC, and `y` markdown export.
///
/// Docs + VIEW: `;` comments the focused row; `y` copies live comments
/// as markdown. Workspace and default-branch rows with more than one
/// (or zero) non-default branches are no-ops. A paint-only overlay or
/// export that still lists a deleted branch is red.
#[test]
fn pty_semicolon_object_comments_and_export() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.gg();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "workspace") && !tree_cursor_on(screen, "README.md"),
        "gg focuses the workspace root",
        WAIT,
    );
    tui.key(';');
    tui.wait_pred(
        |screen| no_comment_overlay(screen) && screen.contains("no comment target"),
        "; on the workspace root is a no-op (no Comment overlay)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "README.md"),
        "j from workspace lands on default-branch app",
        WAIT,
    );
    tui.key(';');
    tui.wait_pred(
        |screen| no_comment_overlay(screen) && screen.contains("no comment target"),
        "; on default-branch app with no extra branch is a no-op",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger (feature/graph)",
        GIT_WAIT,
    );
    save_overlay_body(&mut tui, BRANCH_BODY);
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BRANCH_BODY) && stored.contains("feature/graph"),
        "branch object comment must persist:\n{stored}"
    );

    tui.enter();
    tui.wait_pred(
        merger_graph_drilled_right,
        "Enter on merger focuses that graph",
        WAIT,
    );
    tui.key('j');
    tui.key('j');
    save_overlay_body(&mut tui, COMMIT_BODY);
    let stored = store_text(&workspace);
    assert!(
        stored.contains(COMMIT_BODY) && stored.contains("\"kind\": \"commit\""),
        "commit object comment must persist:\n{stored}"
    );

    tui.key('y');
    tui.wait_pred(
        |screen| {
            export_overlay(screen)
                && screen.contains(BRANCH_BODY)
                && screen.contains(COMMIT_BODY)
                && screen.contains("feature/graph")
        },
        "y copies live branch + commit comments as markdown",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(overlay_closed, "Esc closes the export overlay", WAIT);
    drop(tui);

    let (_root_wt, wt_workspace) = worktree_workspace();
    let mut tui = PtySession::open(&wt_workspace);
    tui.wait_contains("feature/linked-open", WAIT);
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "feature/primary-open"),
        "j focuses the primary checkout",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, LINKED_BRANCH),
        "j focuses the linked worktree",
        WAIT,
    );
    save_overlay_body(&mut tui, WORKTREE_BODY);
    let stored = store_text(&wt_workspace);
    assert!(
        stored.contains(WORKTREE_BODY) && stored.contains("worktree"),
        "worktree object comment must persist:\n{stored}"
    );
    tui.key('y');
    tui.wait_pred(
        |screen| export_overlay(screen) && screen.contains(WORKTREE_BODY),
        "y export includes the live worktree comment",
        WAIT,
    );
    tui.esc();
    drop(tui);

    let root = unique_root("ws-tui-e2e-comment-gc");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    let app = workspace.join("app");
    git(&app, &["branch", "doomed"]);
    assert!(
        git_stdout(&app, &["branch", "--list"]).contains("doomed"),
        "seed must have doomed plus main"
    );

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && tree_has(screen, "README.md"),
        "k focuses default-branch app (exactly one non-default: doomed)",
        WAIT,
    );
    save_overlay_body(&mut tui, ATTACH_BODY);
    let stored = store_text(&workspace);
    assert!(
        stored.contains(ATTACH_BODY) && stored.contains("doomed"),
        "repo-scope comment on default-branch app must attach to doomed:\n{stored}"
    );
    tui.key('y');
    tui.wait_pred(
        |screen| {
            export_overlay(screen) && screen.contains(ATTACH_BODY) && screen.contains("doomed")
        },
        "y export includes the attached doomed branch comment",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(overlay_closed, "Esc closes export before GC", WAIT);

    git(&app, &["branch", "-D", "doomed"]);
    tui.key('r');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| overlay_closed(screen) && tree_has(screen, "app"),
        "r refresh after deleting doomed",
        GIT_WAIT,
    );
    tui.key('y');
    tui.wait_pred(
        |screen| {
            export_overlay(screen)
                && !screen.contains(ATTACH_BODY)
                && !screen.contains("doomed")
                && (screen.contains("No comments") || screen.contains("# Comments"))
        },
        "y export after GC omits the deleted branch comment",
        WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        !stored.contains(ATTACH_BODY) && !stored.contains("doomed"),
        "GC on refresh must drop branch comments for a gone branch:\n{stored}"
    );
}

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::harness::PtySession;
use crate::seed::{git, git_env, seed_repo, unique_root};
use crate::support::{
    graph_cursor_on, graph_pane_focused, tree_cursor_on, tree_has, GIT_WAIT, SETTLE_MS, WAIT,
};

const BODY: &str = "commit-survives-branch-e2e";
const SUBJECT: &str = "doomed-leaf-e2e";

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
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
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

/// Commit object comments survive `git branch -D` of a side branch.
///
/// Docs: commit comments stay while the repo identity is still in the
/// snapshot. `gc_drops_gone_branch_keeps_commit` is not enough: leftover
/// must save via `;`, delete the branch, refresh, then still see the SHA
/// in the store and the `y` overlay. A no-op or a store that drops the
/// body is red.
#[test]
fn pty_semicolon_commit_comment_survives_branch_delete() {
    let root = unique_root("ws-tui-e2e-comment-commit-keep");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    let app = workspace.join("app");
    git(&app, &["checkout", "-q", "-b", "doomed"]);
    fs::write(app.join("doomed.txt"), "doomed leaf\n").unwrap();
    git(&app, &["add", "doomed.txt"]);
    git(&app, &["commit", "-q", "-m", SUBJECT]);
    let sha = git_stdout(&app, &["rev-parse", "HEAD"]);
    git(&app, &["checkout", "-q", "main"]);
    assert!(
        git_stdout(&app, &["branch", "--list"]).contains("doomed"),
        "seed must have doomed plus main"
    );

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "app") && tree_has(screen, "README.md"),
        "launch shows the dirty app checkout",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && tree_has(screen, "README.md"),
        "k focuses app",
        WAIT,
    );
    tui.tab();
    tui.wait_pred(
        |screen| graph_pane_focused(screen) && screen.contains(SUBJECT),
        "Tab focuses the graph with the doomed leaf visible",
        GIT_WAIT,
    );
    tui.search(SUBJECT);
    tui.wait_pred(
        |screen| graph_cursor_on(screen, SUBJECT) && !graph_cursor_on(screen, "working tree"),
        "search selects the doomed leaf commit",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on the commit",
        WAIT,
    );
    tui.keys(BODY);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the commit object comment",
        WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && stored.contains("\"kind\": \"commit\"") && stored.contains(&sha),
        "commit object comment must persist with the SHA:\n{stored}"
    );

    git(&app, &["branch", "-D", "doomed"]);
    tui.key('r');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| overlay_closed(screen) && tree_has(screen, "app"),
        "r refresh after deleting doomed",
        GIT_WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| overlay_closed(screen) && !screen.contains("SEARCH"),
        "Esc clears graph search so y is not typed into /",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "app")
                && overlay_closed(screen)
                && screen.contains("focus right")
        },
        "Esc returns to the app tree row so y copies that checkout",
        WAIT,
    );
    tui.key('y');
    tui.wait_pred(
        |screen| export_overlay(screen) && screen.contains(BODY) && !screen.contains("No comments"),
        "y export still lists the commit comment after the branch is gone",
        WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && stored.contains("\"kind\": \"commit\"") && stored.contains(&sha),
        "GC on refresh must keep the commit comment after branch -D:\n{stored}"
    );
}

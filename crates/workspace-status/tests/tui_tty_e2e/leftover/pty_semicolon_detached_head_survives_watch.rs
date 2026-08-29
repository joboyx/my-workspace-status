use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{git, seed_repo, unique_root};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

const BODY: &str = "detached-wt-note-e2e";

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

/// `;` on a primary detached HEAD is a worktree path key. Watch GC must
/// not drop it (branch `HEAD (detached)` is not a live `refs/heads` name).
///
/// Docs: primary detached HEAD is a worktree path key. Graph `b` is a
/// typical way to land here; this leftover seeds that checkout so watch
/// apply is the oracle. A Branch key, a no-op overlay, or a store that
/// empties after the poll is red.
#[test]
fn pty_semicolon_detached_head_survives_watch() {
    let root = unique_root("ws-tui-e2e-comment-detached");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", false);
    let app = workspace.join("app");
    git(&app, &["checkout", "--detach", "--quiet"]);

    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "app")
                && (screen.contains("HEAD (detached)") || screen.contains("(detached)"))
        },
        "launch shows primary app on detached HEAD",
        WAIT,
    );
    tui.gg();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "workspace") && !tree_cursor_on(screen, "app"),
        "gg focuses the workspace root",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "workspace"),
        "j focuses the detached primary checkout",
        WAIT,
    );

    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on detached HEAD",
        WAIT,
    );
    tui.keys(BODY);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the detached checkout comment",
        WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY)
            && stored.contains("\"kind\": \"worktree\"")
            && !stored.contains("HEAD (detached)"),
        "detached HEAD must persist as a worktree key, not Branch HEAD (detached):\n{stored}"
    );

    tui.wait_ms(2500);
    tui.wait_pred(
        |screen| overlay_closed(screen) && tree_has(screen, "app"),
        "watch ticks keep the TUI mounted after the save",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(BODY) && stored.contains("\"kind\": \"worktree\""),
        "watch GC must keep the detached worktree comment:\n{stored}"
    );
}

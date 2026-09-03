use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, tree_cursor_on, tree_has, GIT_WAIT, WAIT,
};

const FOREIGN_BODY: &str = "FOREIGN-WS-COMMENT-E2E";
const LIVE_BODY: &str = "live-ws-comment-e2e";

fn comment_store(workspace: &Path) -> std::path::PathBuf {
    workspace.join(".e2e-state").join("comments.json")
}

fn store_text(workspace: &Path) -> String {
    fs::read_to_string(comment_store(workspace)).unwrap_or_default()
}

fn seed_foreign_bucket(workspace: &Path) {
    let dir = workspace.join(".e2e-state");
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        comment_store(workspace),
        format!(
            r#"{{
  "version": 2,
  "workspaces": {{
    "foreign-ws": {{
      "entries": [
        {{
          "kind": "branch",
          "repo": "ghost-repo",
          "branch": "ghost-branch",
          "body": "{FOREIGN_BODY}"
        }}
      ]
    }}
  }}
}}
"#
        ),
    )
    .unwrap();
}

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
        && !screen.contains("MOVE")
        && !screen.contains("# Comments")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save")
        && !screen.contains("empty deletes")
        && !screen.contains("copied to clipboard")
}

fn right_diff_focused(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && !tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn live_comment_saved(screen: &str) -> bool {
    overlay_closed(screen)
        && pane_unstaged_readme(screen)
        && screen.contains("comment saved")
        && tree_has(screen, "README.md")
        && !tree_cursor_on(screen, "README.md")
}

/// Shared `comments.json` GC/save keeps another workspace's bucket.
///
/// Persist is version 2 namespaced by workspace. First paint plus a live
/// `;` save must not drop a foreign workspace body. Whole-file GC of a
/// ghost repo/branch would.
#[test]
fn pty_comment_store_keeps_foreign_workspace() {
    let (_root, workspace) = daily_workspace();
    seed_foreign_bucket(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        right_diff_focused,
        "Tab focuses the dirty README diff (not a tree comment)",
        WAIT,
    );

    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on the numbered dirty line",
        WAIT,
    );
    tui.keys(LIVE_BODY);
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains(LIVE_BODY),
        "typed body appears in the overlay",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        live_comment_saved,
        "Enter saves: overlay gone, toast comment saved",
        GIT_WAIT,
    );
    let stored = store_text(&workspace);
    assert!(
        stored.contains(LIVE_BODY),
        "live save must persist this session's comment:\n{stored}"
    );
    assert!(
        stored.contains(FOREIGN_BODY),
        "GC/save of this workspace must keep the foreign bucket:\n{stored}"
    );
}

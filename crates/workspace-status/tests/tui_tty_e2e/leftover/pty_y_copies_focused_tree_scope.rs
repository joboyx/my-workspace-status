use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{seed_repo, unique_root};
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

const FILE1_BODY: &str = "scope-file1-note-e2e";
const FILE2_BODY: &str = "scope-file2-note-e2e";
const README_BODY: &str = "scope-readme-note-e2e";

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
        && !screen.contains("# Comments")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save")
        && !screen.contains("empty deletes")
        && !screen.contains("copied to clipboard")
}

fn export_overlay(screen: &str) -> bool {
    screen.contains("# Comments")
        && screen.contains("copied to clipboard")
        && screen.contains("copied · Esc close")
        && !screen.contains("MOVE")
}

fn tree_ready(screen: &str) -> bool {
    tree_has(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "folder1")
        && tree_has(screen, "file1.txt")
        && tree_has(screen, "file2.txt")
}

fn save_line_comment(tui: &mut PtySession, name: &str, body: &str) {
    tui.tab();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, name)
                && overlay_closed(screen)
                && (screen.contains("UNSTAGED")
                    || screen.contains("UNTRACKED")
                    || screen.contains("NEW")
                    || screen.contains("@@"))
        },
        &format!("Tab focuses the {name} diff"),
        WAIT,
    );
    tui.key(';');
    tui.wait_pred(comment_overlay, "; opens Comment overlay", WAIT);
    tui.keys(body);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the line comment",
        GIT_WAIT,
    );
    tui.tab();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, name) && overlay_closed(screen) && screen.contains("focus right")
        },
        "Tab returns to the tree",
        WAIT,
    );
}

/// `y` copies comments for the focused tree row and descendants only.
///
/// Docs + VIEW: focus `file2.txt` copies that path; focus `folder1`
/// copies both files under it; a sibling `README.md` stays out.
/// Confirm-mode `y` and object-comment leftovers are other tests.
#[test]
fn pty_y_copies_focused_tree_scope() {
    let root = unique_root("ws-tui-e2e-y-scope");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    let folder = workspace.join("app").join("folder1");
    fs::create_dir_all(&folder).unwrap();
    fs::write(folder.join("file1.txt"), "file1 line\n").unwrap();
    fs::write(folder.join("file2.txt"), "file2 line\n").unwrap();

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_ready(screen) && tree_cursor_on(screen, "file1.txt"),
        "launch focuses folder1/file1.txt",
        WAIT,
    );

    save_line_comment(&mut tui, "file1.txt", FILE1_BODY);
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "file2.txt") && !tree_cursor_on(screen, "file1.txt"),
        "j from file1 lands on file2",
        WAIT,
    );
    save_line_comment(&mut tui, "file2.txt", FILE2_BODY);
    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md") && !tree_cursor_on(screen, "file2.txt"),
        "j from file2 lands on README.md",
        WAIT,
    );
    save_line_comment(&mut tui, "README.md", README_BODY);
    let stored = store_text(&workspace);
    assert!(
        stored.contains(FILE1_BODY) && stored.contains(FILE2_BODY) && stored.contains(README_BODY),
        "all three line comments must persist:\n{stored}"
    );

    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "file2.txt") && !tree_cursor_on(screen, "README.md"),
        "k from README lands on file2",
        WAIT,
    );
    tui.key('y');
    tui.wait_pred(
        |screen| {
            export_overlay(screen)
                && screen.contains(FILE2_BODY)
                && !screen.contains(FILE1_BODY)
                && !screen.contains(README_BODY)
        },
        "y on file2 copies that path only",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(overlay_closed, "Esc closes the file2 export", WAIT);

    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "file1.txt") && !tree_cursor_on(screen, "file2.txt"),
        "k from file2 lands on file1",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "folder1") && !tree_cursor_on(screen, "file1.txt"),
        "k from file1 lands on folder1",
        WAIT,
    );
    tui.key('y');
    tui.wait_pred(
        |screen| {
            export_overlay(screen)
                && screen.contains(FILE1_BODY)
                && screen.contains(FILE2_BODY)
                && !screen.contains(README_BODY)
        },
        "y on folder1 copies both files under it, not the sibling README",
        WAIT,
    );
}

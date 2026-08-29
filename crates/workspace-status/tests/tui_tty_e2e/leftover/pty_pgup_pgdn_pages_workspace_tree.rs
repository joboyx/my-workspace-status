use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{seed_tree_page_files, tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// CSI PageDown then PageUp pages the workspace tree by one viewport.
///
/// Help VIEW lists `PgUp PgDn` as "page focused pane", distinct from
/// `Ctrl-u Ctrl-d` ±5. This suite sends xterm CSI `ESC [6~` / `ESC [5~`.
/// Launch focuses `README.md`. Files sort as README then `page-00`…
/// `page-29`. The default PTY paints 28 tree rows, so one page is 27
/// (`visible − 1` overlap) and lands on `page-26.txt`. Cursor bar and
/// the right-pane file body must both move. A no-op stays on README.
/// `j` would land on `page-00`. Ctrl-d would land on `page-04`. `G` /
/// End would land on No updates. Home would land on the workspace root.
#[test]
fn pty_pgup_pgdn_pages_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_page_files(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("VIEW")
                && screen.contains("PgUp   PgDn")
                && screen.contains("page focused pane")
                && screen.contains("Ctrl-u   Ctrl-d")
                && screen.contains("page focused ±5")
        },
        "help VIEW lists PgUp/PgDn as a viewport page, distinct from Ctrl-u/d ±5",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("page focused pane")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
        },
        "Esc closes help so PageDown is not swallowed",
        WAIT,
    );

    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "page-00.txt")
                && !tree_cursor_on(screen, "page-04.txt")
                && !tree_cursor_on(screen, "page-26.txt")
                && !tree_cursor_on(screen, "workspace")
                && !tree_has(screen, "page-29")
                && !screen.contains("page-26-body")
                && screen.contains("UNSTAGED")
        },
        "launch cursor is README; page-26 is not focused",
        GIT_WAIT,
    );

    tui.page_down();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "page-26.txt")
                && screen.contains("page-26-body")
                && screen.contains("NEW")
                && !tree_cursor_on(screen, "README.md")
                && !tree_has(screen, "README.md")
                && !tree_cursor_on(screen, "page-00.txt")
                && !tree_cursor_on(screen, "page-04.txt")
                && !tree_cursor_on(screen, "page-29.txt")
                && !tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "workspace")
        },
        "PageDown pages +27 to page-26 (a no-op stays on README; j would hit page-00; Ctrl-d would hit page-04; G would hit No updates)",
        GIT_WAIT,
    );

    tui.page_up();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && !tree_cursor_on(screen, "page-26.txt")
                && !screen.contains("page-26-body")
                && !tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "No updates")
        },
        "PageUp returns to README (a no-op keeps page-26; Home would land on workspace)",
        GIT_WAIT,
    );
}

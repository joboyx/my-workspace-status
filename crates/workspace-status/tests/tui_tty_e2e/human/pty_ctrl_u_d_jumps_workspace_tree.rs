use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// Extra dirty files so Ctrl-d ±5 cannot clamp to the last row (that is `G`).
fn seed_tree_half_page_files(workspace: &Path) {
    let app = workspace.join("app");
    for i in 0..10 {
        fs::write(
            app.join(format!("jump-{i:02}.txt")),
            format!("jump-{i:02}-body\n"),
        )
        .unwrap();
    }
}

/// VIEW lists Ctrl-u/d as ±5. Same line as `page focused ±5`, not PgUp/PgDn.
fn help_lists_ctrl_u_d_half_page(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains("Ctrl-u")
            && line.contains("Ctrl-d")
            && line.contains("page focused ±5")
            && !line.contains("PgUp")
            && !line.contains("page focused pane")
    })
}

/// Cursor bar, file-diff body, and left focus for one half-page landing.
///
/// `j` is `jump-00`. A 12-row PageDown is `jump-06`. End / `G` / a fitting
/// PageDown is No updates. Tab paints `[workspace]` / `drill`.
fn half_page_on(screen: &str, name: &str, body: &str) -> bool {
    tree_cursor_on(screen, name)
        && screen.contains(body)
        && screen.contains("NEW")
        && screen.contains("focus right")
        && !screen.contains("[workspace]")
        && !screen.contains("drill")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "workspace")
        && !screen.contains("WIP on graph")
        && !screen.contains("UNSTAGED")
}

/// Help VIEW lists Ctrl-u/d as ±5. CSI-u Control jumps the tree half a page.
///
/// Documented result: ±5 rows on the focused list, not `j`/`k` (±1), not
/// PageDown (viewport), not Home/End. Launch focuses README.md. Files sort
/// as README then `jump-00`…`jump-09`, so +5 lands on `jump-04.txt` and a
/// second +5 on `jump-09.txt` (not merger). Cursor bar and the right-pane
/// body must both move. Stay left. A no-op stays on README.
#[test]
fn pty_ctrl_u_d_jumps_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_half_page_files(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && tree_has(screen, "jump-00.txt")
                && tree_has(screen, "jump-04.txt")
                && tree_has(screen, "jump-09.txt")
                && tree_has(screen, "No updates")
                && !tree_cursor_on(screen, "jump-00.txt")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !tree_cursor_on(screen, "jump-06.txt")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !tree_cursor_on(screen, "No updates")
                && !screen.contains("jump-04-body")
                && screen.contains("? help")
                && screen.contains("focus right")
                && !screen.contains("[workspace]")
        },
        "launch cursor is README; half-page and end rows are not focused",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("VIEW")
                && help_lists_ctrl_u_d_half_page(screen)
                && screen.contains("Ctrl-u   Ctrl-d")
                && screen.contains("PgUp   PgDn")
                && screen.contains("page focused pane")
                && screen.contains("j   k")
                && screen.contains("Home   End")
                && screen.contains("top / bottom")
        },
        "help VIEW lists Ctrl-u/d as ±5, not PgUp/PgDn and not j/Home/End",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("page focused ±5")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
                && screen.contains("UNSTAGED")
                && screen.contains("focus right")
        },
        "Esc closes help so Ctrl-u/d are tree jumps, not help keys",
        WAIT,
    );

    tui.ctrl_letter('d');
    tui.wait_pred(
        |screen| {
            half_page_on(screen, "jump-04.txt", "jump-04-body")
                && !tree_cursor_on(screen, "jump-00.txt")
                && !tree_cursor_on(screen, "jump-03.txt")
                && !tree_cursor_on(screen, "jump-05.txt")
                && !tree_cursor_on(screen, "jump-06.txt")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !screen.contains("jump-00-body")
                && !screen.contains("jump-06-body")
                && !screen.contains("jump-09-body")
        },
        "CSI-u Ctrl-d moves +5 to jump-04 (j is jump-00; 12-row PgDn is jump-06; End/PgDn on this tree is No updates)",
        GIT_WAIT,
    );

    tui.ctrl_letter('d');
    tui.wait_pred(
        |screen| {
            half_page_on(screen, "jump-09.txt", "jump-09-body")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !tree_cursor_on(screen, "jump-08.txt")
                && !screen.contains("jump-04-body")
        },
        "second CSI-u Ctrl-d moves +5 to jump-09 (a no-op stays on jump-04; End is No updates; merger is +6)",
        GIT_WAIT,
    );

    tui.ctrl_letter('u');
    tui.wait_pred(
        |screen| {
            half_page_on(screen, "jump-04.txt", "jump-04-body")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("jump-09-body")
        },
        "CSI-u Ctrl-u returns +5 to jump-04 (a no-op stays on jump-09; Home/two Ctrl-u would hit README)",
        GIT_WAIT,
    );

    tui.ctrl_letter('u');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && screen.contains("focus right")
                && !screen.contains("[workspace]")
                && !screen.contains("drill")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !tree_cursor_on(screen, "jump-00.txt")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !tree_cursor_on(screen, "No updates")
                && !screen.contains("jump-04-body")
                && !screen.contains("NEW")
        },
        "second CSI-u Ctrl-u returns to README (a no-op keeps jump-04; j would stay on jump-00)",
        GIT_WAIT,
    );
}

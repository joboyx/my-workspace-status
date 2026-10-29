use std::fs;

use crate::harness::PtySession;
use crate::seed::{compare_ahead_workspace, git};
use crate::support::{csi_u_tap, send_gesture, tree_cursor_on, tree_has, GIT_WAIT, WAIT};

/// CSI-u `gg` chord (press + release per tap) in one write, so both taps
/// share one arrival and stay inside the 400ms window.
fn csi_u_gg(tui: &mut PtySession) {
    send_gesture(tui, &[&csi_u_tap('g'), &csi_u_tap('g')]);
}

/// Ahead of `origin/main` with only nested files under `src/`.
///
/// Tree mode paints `src`, then `auth.ts`, then `session.ts`. A root-level
/// ahead file would hide the directory-row `gg` bug.
fn seed_nested_compare_files(workspace: &std::path::Path) {
    let app = workspace.join("app");
    git(&app, &["reset", "--hard", "HEAD~1"]);
    fs::create_dir_all(app.join("src")).unwrap();
    fs::write(app.join("src").join("auth.ts"), "export const token = 1;\n").unwrap();
    fs::write(
        app.join("src").join("session.ts"),
        "export const session = 1;\n",
    )
    .unwrap();
    git(&app, &["add", "src/auth.ts", "src/session.ts"]);
    git(&app, &["commit", "-q", "-m", "nested compare files"]);
}

fn on_compare_origin_main(screen: &str) -> bool {
    screen.contains("app ↔ origin/main")
        && screen.contains("COMMITTED")
        && tree_has(screen, "src")
        && tree_has(screen, "auth.ts")
        && tree_has(screen, "session.ts")
}

fn cursor_on_auth_ts(screen: &str) -> bool {
    on_compare_origin_main(screen)
        && tree_cursor_on(screen, "auth.ts")
        && !tree_cursor_on(screen, "session.ts")
        && !tree_cursor_on(screen, "src")
}

fn cursor_on_session_ts(screen: &str) -> bool {
    on_compare_origin_main(screen)
        && tree_cursor_on(screen, "session.ts")
        && !tree_cursor_on(screen, "auth.ts")
        && !tree_cursor_on(screen, "src")
}

/// Compare `gg` lands on the first file leaf, not the parent directory.
///
/// New compare load already selects the first file. `gg` must reuse that
/// index. A jump onto `src` fails.
#[test]
fn pty_compare_gg_first_file_not_dir() {
    let (_root, workspace) = compare_ahead_workspace();
    seed_nested_compare_files(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.key(':');
    tui.keys("vs default");
    tui.enter();
    tui.wait_pred(
        cursor_on_auth_ts,
        "Diff vs default in new tab selects the first file leaf (auth.ts), not src",
        GIT_WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        cursor_on_session_ts,
        "j moves from auth.ts onto session.ts",
        WAIT,
    );

    csi_u_gg(&mut tui);
    tui.wait_pred(
        cursor_on_auth_ts,
        "CSI-u gg returns to the first file leaf (auth.ts); landing on src fails",
        WAIT,
    );
}

use std::fs;

use crate::harness::PtySession;
use crate::seed::{whitespace_workspace, INDENT_LINE};
use crate::support::{after_ack, right_pane, tree_cursor_on, WAIT};

/// `indent.txt` focused and shown with (`dash_w`) or without `-w`.
fn indent_diff(screen: &str, dash_w: bool) -> bool {
    let pane = right_pane(screen);
    let header = pane.lines().next().unwrap_or("");
    tree_cursor_on(screen, "indent.txt")
        && header.contains("indent.txt")
        && header.contains("· -w") == dash_w
        && pane.contains(INDENT_LINE) != dash_w
}

/// The last-used whitespace setting is saved and wins at the next launch.
///
/// Launch with an explicit empty UI-state store: the whitespace-only
/// `indent.txt` hunk shows. `w` hides it and saves
/// `"ignoreWhitespace": true` to the store at once; quit. A relaunch on the
/// same store loads `indent.txt` with `-w` from the first paint: the
/// header has `· -w` and the hunk is hidden, with no key pressed.
#[test]
fn pty_ignore_whitespace_survives_restart() {
    let (_root, workspace) = whitespace_workspace("ws-tui-tty-whitespace-restart");
    let ui_store = workspace
        .join(".e2e-state")
        .join("ui-state-whitespace.json");
    let env = [("WS_STATUS_UI_STORE", ui_store.to_str().unwrap())];

    let mut tui = PtySession::open_with_env(&workspace, &env);
    tui.wait_pred(
        |screen| indent_diff(screen, false),
        "launch on an empty store shows the indent.txt hunk",
        WAIT,
    );
    assert!(!ui_store.exists(), "launch writes no store");
    tui.key('w');
    tui.wait_pred(
        after_ack("whitespace hidden", |screen| indent_diff(screen, true)),
        "w hides the whitespace-only hunk",
        WAIT,
    );
    tui.key('q');
    tui.wait_exit(WAIT);
    let stored = fs::read_to_string(&ui_store).expect("w saves the UI-state store");
    assert!(
        stored.contains("\"ignoreWhitespace\": true"),
        "the store keeps the last-used value:\n{stored}"
    );

    let tui = PtySession::open_with_env(&workspace, &env);
    tui.wait_pred(
        |screen| indent_diff(screen, true),
        "relaunch on the same store loads the diff with -w",
        WAIT,
    );
}

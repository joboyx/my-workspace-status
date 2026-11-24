use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{documented_launch_first_paint, prove_keys_handled, WAIT};
use crate::update_support::{startup_update_prompt_blocking, UpdateApp};

use workspace_status::update_check::UPDATE_PROMPT_SUFFIX;

/// The current TUI painted the documented first paint. The prompt is gone.
fn current_tui_up(screen: &str) -> bool {
    documented_launch_first_paint(screen) && !screen.contains(UPDATE_PROMPT_SUFFIX)
}

/// Startup GitHub Release prompt on a TTY. `y` with no new install opens
/// the current version's TUI.
///
/// Docs: CLI long_about, README, architecture, tui-rust. `y` runs the
/// updater; when it fails (exit 1, app file unchanged), nothing starts
/// again and this process opens the TUI.
///
/// Setup: a copy of the binary in a temp `bin/` with a fake sibling
/// updater that exits 1 and leaves the copy as it is. Encoding as in
/// `pty_update_prompt_n_opens_tui`.
///
/// An update that exits after the failure, or starts the "new version"
/// script, cannot pass.
#[test]
fn pty_update_prompt_y_no_install_opens_tui() {
    let (root, workspace) = daily_workspace();
    let app = UpdateApp::new(&root);
    app.updater(false, 1);
    let path = app.path_env();
    let mut tui = PtySession::open_pending_bin(&workspace, &[("PATH", &path)], 0, &app.app, &[]);

    tui.wait_pred(
        startup_update_prompt_blocking,
        "newer release prints the versioned update prompt on the primary screen; TUI not mounted",
        WAIT,
    );
    tui.assert_running("blocked on update prompt (must not skip or auto-mount)");

    tui.key('y');
    tui.enter();
    tui.wait_pred(
        current_tui_up,
        "y + failed update: the current TUI opens with the documented first paint",
        WAIT,
    );
    tui.assert_running("after y + failed update (must open the TUI, not exit)");
    assert!(app.sidecar_marker.exists(), "y must run the updater");
    assert!(
        !app.relaunch_log.exists(),
        "no new install: the new version must not start"
    );

    prove_keys_handled(&mut tui);
    tui.wait_pred(current_tui_up, "current TUI holds after keys", WAIT);
    assert!(
        !app.relaunch_log.exists(),
        "no new install: the new version must not start later either"
    );
}

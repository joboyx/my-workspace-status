use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{documented_launch_first_paint, prove_keys_handled, WAIT};
use crate::update_support::{startup_update_prompt_blocking, UpdateApp};

use workspace_status::update_check::UPDATE_PROMPT_SUFFIX;

/// The started new version painted the documented first paint. The prompt
/// is gone and no relaunch error shows.
fn new_version_tui_up(screen: &str) -> bool {
    documented_launch_first_paint(screen)
        && !screen.contains(UPDATE_PROMPT_SUFFIX)
        && !screen.contains("failed to run")
}

/// Startup GitHub Release prompt on a TTY. `y` with a new install starts
/// the new version with the same arguments and cwd.
///
/// Docs: CLI long_about, README, architecture, tui-rust. `y` runs the
/// updater; when it installs a new version (success and the app file
/// changed), the app starts again from its path with the original
/// arguments (none is `--update` here) in the same cwd, and that new
/// version opens the TUI. It does not prompt again: the prompt saved the
/// check time before it asked.
///
/// Setup: a copy of the binary in a temp `bin/` with a fake sibling
/// updater that moves a "new version" script over the copy. The script
/// logs its args and cwd, then execs a second untouched copy, so the real
/// TUI paints. Encoding as in `pty_update_prompt_n_opens_tui`: `y` then
/// Enter is one `read_line` answer.
///
/// An update that exits, opens the old TUI in this process, starts the new
/// version with other args or another cwd, or prompts again cannot pass.
#[test]
fn pty_update_prompt_y_starts_new_version() {
    let (root, workspace) = daily_workspace();
    let app = UpdateApp::new(&root);
    app.updater(true, 0);
    let path = app.path_env();
    let mut tui =
        PtySession::open_pending_bin(&workspace, &[("PATH", &path)], 0, &app.app, &["--tui"]);

    tui.wait_pred(
        startup_update_prompt_blocking,
        "newer release prints the versioned update prompt on the primary screen; TUI not mounted",
        WAIT,
    );
    tui.assert_running("blocked on update prompt (must not skip or auto-mount)");
    assert!(
        !app.sidecar_marker.exists(),
        "updater must not run while the prompt is up"
    );

    tui.key('y');
    tui.enter();
    tui.wait_pred(
        new_version_tui_up,
        "y + install: the new version starts and paints the documented first paint",
        WAIT,
    );
    tui.assert_running("after y + install (the new version must keep running)");
    assert!(app.sidecar_marker.exists(), "y must run the updater");
    let cwd = workspace.canonicalize().unwrap();
    assert_eq!(
        fs::read_to_string(&app.relaunch_log).unwrap_or_default(),
        format!("args=[--tui] cwd={}\n", cwd.display()),
        "the new version starts once, with the original args in the workspace cwd"
    );

    prove_keys_handled(&mut tui);
    tui.wait_pred(
        new_version_tui_up,
        "new version TUI holds (not a flicker or a second prompt)",
        WAIT,
    );
    let curl_log = fs::read_to_string(app.bin_dir.join("curl.log")).unwrap_or_default();
    assert_eq!(
        curl_log
            .lines()
            .filter(|line| line.contains("/releases/latest"))
            .count(),
        1,
        "only the first launch checks for a release; the new version must not ask again:\n{curl_log}"
    );
}

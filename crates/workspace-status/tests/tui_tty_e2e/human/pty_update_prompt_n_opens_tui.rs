use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{documented_launch_first_paint, prove_keys_handled, WAIT};
use crate::update_support::{
    startup_update_prompt_blocking, write_fake_updater, write_release_curl,
};

use workspace_status::update_check::UPDATE_PROMPT_SUFFIX;

/// `n` declined the update. Documented first paint is up. Prompt is gone.
///
/// Fail if `n` is a no-op (still on the prompt), if the TUI never mounts,
/// or if the `y` path ran (notes / sidecar).
fn declined_update_opened_tui(screen: &str) -> bool {
    documented_launch_first_paint(screen)
        && !screen.contains(UPDATE_PROMPT_SUFFIX)
        && !screen.contains("Updating ")
        && !screen.contains("SIDECAR_RAN")
        && !screen.contains("failed to run workspace-status-update")
}

fn curl_log_is_latest_only(log: &str) -> bool {
    log.contains("/releases/latest") && !log.contains("per_page=")
}

/// Startup GitHub Release prompt on a TTY. `n` declines and opens the TUI.
///
/// Docs: CLI long_about, architecture, tui-rust. A TTY launch with a
/// newer published release asks `workspace-status <current> → 99.0.0
/// available. Update? [y/N]` on the primary screen before the TUI. `y` runs the update (notes,
/// then `workspace-status-update`; the `pty_update_prompt_y_*` tests).
/// `n` / `no` open the TUI. Offline /
/// current / missing curl stay quiet and never print the prompt.
///
/// Encoding: `stdin.read_line` before ratatui mounts. That is not the
/// TUI keymap. On a live PTY, bare `n` echoes and stays on the prompt;
/// CSI-u `n` stays on the prompt; `n` then Enter (`\n` or `\r`) declines.
/// `PtySession::key('n')` plus [`PtySession::enter`] (`\r`) is that
/// line. This test does not claim `y`.
///
/// A skipped check, a no-op `n`, an auto-mount without answering, or a
/// `y` install cannot pass.
#[test]
fn pty_update_prompt_n_opens_tui() {
    let (_root, workspace) = daily_workspace();
    let shim_dir = workspace.join(".e2e-curl-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let curl_log = write_release_curl(&shim_dir);
    let sidecar_marker = shim_dir.join("sidecar.ran");
    write_fake_updater(
        &shim_dir.join("workspace-status-update"),
        &sidecar_marker,
        None,
        0,
    );
    let path = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut tui = PtySession::open_pending(&workspace, &[("PATH", &path)], 0);

    tui.wait_pred(
        startup_update_prompt_blocking,
        "newer release prints the versioned update prompt on the primary screen; TUI not mounted",
        WAIT,
    );
    tui.assert_running("blocked on update prompt (must not skip or auto-mount)");
    // The `n` below answers this prompt, which proves it stayed up.
    assert!(
        !sidecar_marker.exists(),
        "sidecar must not run while the prompt is up"
    );

    tui.key('n');
    tui.enter();
    tui.wait_pred(
        declined_update_opened_tui,
        "n declines: documented first paint, prompt gone, no update/install",
        WAIT,
    );
    tui.assert_running("after n (must open TUI, not run the sidecar)");
    prove_keys_handled(&mut tui);
    tui.wait_pred(
        declined_update_opened_tui,
        "declined TUI holds (not a flicker, prompt return, or y path)",
        WAIT,
    );

    let log = fs::read_to_string(&curl_log).unwrap_or_default();
    assert!(
        curl_log_is_latest_only(&log),
        "n must fetch /releases/latest only, not the --update notes list:\n{log}"
    );
    assert!(
        !sidecar_marker.exists(),
        "n must not run workspace-status-update"
    );
}

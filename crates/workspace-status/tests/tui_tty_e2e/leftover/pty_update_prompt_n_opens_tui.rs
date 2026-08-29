use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{documented_launch_first_paint, SETTLE_MS, WAIT};

use workspace_status::update_check::UPDATE_PROMPT;

/// Startup prompt on the primary screen. TUI chrome must not be up yet.
///
/// Docs: a newer GitHub Release prints `UPDATE_PROMPT` before the TUI
/// mounts. A skipped check, a quiet failure, or an already-mounted TUI
/// cannot pass. `y` would print `Updating ` and run the sidecar.
fn startup_update_prompt_blocking(screen: &str) -> bool {
    screen.contains(UPDATE_PROMPT)
        && !documented_launch_first_paint(screen)
        && !screen.contains("? help")
        && !screen.contains(" tree")
        && !screen.contains("Updating ")
        && !screen.contains("SIDECAR_RAN")
        && !screen.contains("failed to run workspace-status-update")
}

/// `n` declined the update. Documented first paint is up. Prompt is gone.
///
/// Fail if `n` is a no-op (still on the prompt), if the TUI never mounts,
/// or if the `y` path ran (notes / sidecar).
fn declined_update_opened_tui(screen: &str) -> bool {
    documented_launch_first_paint(screen)
        && !screen.contains(UPDATE_PROMPT)
        && !screen.contains("Updating ")
        && !screen.contains("SIDECAR_RAN")
        && !screen.contains("failed to run workspace-status-update")
}

fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

fn curl_log_is_latest_only(log: &str) -> bool {
    log.contains("/releases/latest") && !log.contains("per_page=")
}

/// Startup GitHub Release prompt on a TTY. `n` declines and opens the TUI.
///
/// Docs: CLI long_about, architecture, tui-rust. A TTY launch with a
/// newer published release asks `new version available, update? [y/n]`
/// on the primary screen before the TUI. `y` runs `--update` (notes,
/// then `workspace-status-update`). `n` / `no` open the TUI. Offline /
/// current / missing curl stay quiet and never print the prompt.
///
/// Encoding: `stdin.read_line` before ratatui mounts. That is not the
/// TUI keymap. A live PTY hunt: bare `n` echoes and stays on the prompt;
/// CSI-u `n` stays on the prompt; `n` then Enter (`\n` or `\r`) declines.
/// `PtySession::key('n')` plus [`PtySession::enter`] (`\r`) is that
/// line. This leftover does not claim `y`.
///
/// A skipped check, a no-op `n`, an auto-mount without answering, or a
/// `y` install cannot pass.
#[test]
fn pty_update_prompt_n_opens_tui() {
    let (_root, workspace) = daily_workspace();
    let shim_dir = workspace.join(".e2e-curl-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let curl_log = shim_dir.join("curl.log");
    let sidecar_marker = shim_dir.join("sidecar.ran");
    write_executable(
        &shim_dir.join("curl"),
        &format!(
            "#!/bin/sh\n\
             printf '%s\\n' \"$*\" >> \"{log}\"\n\
             for a in \"$@\"; do\n\
               case \"$a\" in\n\
                 *releases/latest*)\n\
                   printf '%s\\n' '{{\"tag_name\":\"v99.0.0\"}}'\n\
                   exit 0\n\
                   ;;\n\
                 *releases*)\n\
                   printf '%s\\n' '[ {{\"tag_name\":\"v99.0.0\",\"draft\":false,\"prerelease\":false,\"body\":\"## [99.0.0]\\n\\n### Features\\n\\n- leftover y notes\\n\\n## Install workspace-status 99.0.0\\n\"}} ]'\n\
                   exit 0\n\
                   ;;\n\
               esac\n\
             done\n\
             exit 1\n",
            log = curl_log.display()
        ),
    );
    write_executable(
        &shim_dir.join("workspace-status-update"),
        &format!(
            "#!/bin/sh\n\
             printf sidecar-ran > \"{marker}\"\n\
             printf 'SIDECAR_RAN\\n'\n\
             exit 0\n",
            marker = sidecar_marker.display()
        ),
    );
    let path = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut tui = PtySession::open_pending(&workspace, &[("PATH", &path)], 0);

    tui.wait_pred(
        startup_update_prompt_blocking,
        "newer release prints UPDATE_PROMPT on the primary screen; TUI not mounted",
        WAIT,
    );
    tui.assert_running("blocked on update prompt (must not skip or auto-mount)");
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        startup_update_prompt_blocking,
        "update prompt holds (not a skip, auto-mount, or y path)",
        WAIT,
    );
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
    tui.assert_running("after n (must open TUI, not exec the sidecar)");
    tui.wait_ms(SETTLE_MS);
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

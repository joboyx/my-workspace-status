use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

use workspace_status::update_check::UPDATE_PROMPT;

/// Startup GitHub Release prompt on a TTY. `n` continues into the TUI.
#[test]
fn pty_update_prompt_n_opens_tui() {
    let (_root, workspace) = daily_workspace();
    let shim_dir = workspace.join(".e2e-curl-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let shim = shim_dir.join("curl");
    fs::write(
        &shim,
        "#!/bin/sh\n\
         for a in \"$@\"; do\n\
           case \"$a\" in\n\
             *releases/latest*)\n\
               printf '%s\\n' '{\"tag_name\":\"v99.0.0\"}'\n\
               exit 0\n\
               ;;\n\
           esac\n\
         done\n\
         exit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&shim).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&shim, perms).unwrap();
    let path = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut tui = PtySession::open_pending(&workspace, &[("PATH", &path)], 0);
    tui.wait_contains(UPDATE_PROMPT, WAIT);
    tui.send_bytes(b"n\n");
    tui.wait_ready();
    tui.wait_contains("app", WAIT);
    tui.wait_contains("README.md", WAIT);
}

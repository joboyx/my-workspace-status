//! Shared fixtures for the startup update prompt PTY tests.
//!
//! The tests never run a real updater: a fake `curl` answers GitHub with a
//! newer release (`v99.0.0`), and a fake `workspace-status-update` prints
//! `SIDECAR_RAN`. The `y` tests run a copy of the binary in a temp `bin/`
//! dir, so a fake install swaps a file there and never touches `target/`.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use crate::support::documented_launch_first_paint;

use workspace_status::update_check::UPDATE_PROMPT_SUFFIX;

/// Write `body` to `path` and make it executable.
pub fn write_executable(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

/// Write a fake `curl` into `shim_dir` that answers a newer GitHub Release
/// (`v99.0.0`): `/releases/latest` for the startup check and the release list
/// for the `--update` notes. Every call appends its argv to the returned log.
pub fn write_release_curl(shim_dir: &Path) -> PathBuf {
    let curl_log = shim_dir.join("curl.log");
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
                   printf '%s\\n' '[ {{\"tag_name\":\"v99.0.0\",\"draft\":false,\"prerelease\":false,\"body\":\"## [99.0.0]\\n\\n### Features\\n\\n- release notes\\n\\n## Install workspace-status 99.0.0\\n\"}} ]'\n\
                   exit 0\n\
                   ;;\n\
               esac\n\
             done\n\
             exit 1\n",
            log = curl_log.display()
        ),
    );
    curl_log
}

/// Write a fake `workspace-status-update` at `path`: it writes `marker`,
/// prints `SIDECAR_RAN`, moves `install.0` over `install.1` when given (a
/// new install), then exits with `code`.
pub fn write_fake_updater(path: &Path, marker: &Path, install: Option<(&Path, &Path)>, code: i32) {
    let install = install.map_or_else(String::new, |(from, to)| {
        format!("/bin/mv \"{}\" \"{}\"\n", from.display(), to.display())
    });
    write_executable(
        path,
        &format!(
            "#!/bin/sh\n\
             printf sidecar-ran > \"{marker}\"\n\
             printf 'SIDECAR_RAN\\n'\n\
             {install}exit {code}\n",
            marker = marker.display()
        ),
    );
}

/// Startup prompt on the primary screen. TUI chrome must not be up yet.
///
/// Docs: a newer GitHub Release prints the versioned update prompt before the TUI
/// mounts. A skipped check, a quiet failure, or an already-mounted TUI
/// cannot pass. `y` would print `Updating ` and run the sidecar.
pub fn startup_update_prompt_blocking(screen: &str) -> bool {
    screen.contains("→ 99.0.0 available. Update? [y/N]")
        && screen.contains(UPDATE_PROMPT_SUFFIX)
        && !documented_launch_first_paint(screen)
        && !screen.contains("? help")
        && !screen.contains(" tree")
        && !screen.contains("Updating ")
        && !screen.contains("SIDECAR_RAN")
        && !screen.contains("failed to run workspace-status-update")
}

/// A copy of the `workspace-status` binary in `<root>/update-bin/` with a
/// fake sibling updater and a fake `curl`, for the prompt's `y` answer.
///
/// The copy is the file the updater may replace. `new_version` is a script
/// that logs its args and cwd to `relaunch_log`, then execs a second,
/// untouched copy, so the started "new version" paints the real TUI.
pub struct UpdateApp {
    /// The copied binary the test spawns (the app path).
    pub app: PathBuf,
    /// Dir of the app copy, the fake updater, and the fake `curl`.
    pub bin_dir: PathBuf,
    /// Written by the fake updater each time it runs.
    pub sidecar_marker: PathBuf,
    /// Written by the "new version" script when it starts.
    pub relaunch_log: PathBuf,
    new_version: PathBuf,
}

impl UpdateApp {
    /// Copy the binary under `root` and write the fake `curl` and the
    /// "new version" script. The updater comes from [`UpdateApp::updater`].
    pub fn new(root: &Path) -> Self {
        let bin_dir = root.join("update-bin");
        let next_dir = root.join("update-next");
        fs::create_dir_all(&bin_dir).unwrap();
        fs::create_dir_all(&next_dir).unwrap();
        let src = env!("CARGO_BIN_EXE_workspace-status");
        let app = bin_dir.join("workspace-status");
        fs::copy(src, &app).unwrap();
        let next_app = next_dir.join("workspace-status");
        fs::copy(src, &next_app).unwrap();
        write_release_curl(&bin_dir);
        let relaunch_log = root.join("relaunch.log");
        let new_version = next_dir.join("new-version");
        write_executable(
            &new_version,
            &format!(
                "#!/bin/sh\n\
                 printf 'args=[%s] cwd=%s\\n' \"$*\" \"$(pwd -P)\" >> \"{log}\"\n\
                 exec \"{next}\" \"$@\"\n",
                log = relaunch_log.display(),
                next = next_app.display()
            ),
        );
        Self {
            app,
            sidecar_marker: bin_dir.join("sidecar.ran"),
            bin_dir,
            relaunch_log,
            new_version,
        }
    }

    /// Write the fake `workspace-status-update` beside the app
    /// ([`write_fake_updater`]): when `install`, it moves the "new version"
    /// over the app.
    pub fn updater(&self, install: bool, code: i32) {
        write_fake_updater(
            &self.bin_dir.join("workspace-status-update"),
            &self.sidecar_marker,
            install.then_some((self.new_version.as_path(), self.app.as_path())),
            code,
        );
    }

    /// `PATH` with the fake `curl` dir first.
    pub fn path_env(&self) -> String {
        format!(
            "{}:{}",
            self.bin_dir.display(),
            std::env::var("PATH").unwrap_or_default()
        )
    }
}

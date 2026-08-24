//! cargo-dist sidecar (`workspace-status-update`) used by `--update`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// File name of the cargo-dist updater next to `ws` / `workspace-status`.
pub(crate) fn updater_file_name() -> &'static str {
    if cfg!(windows) {
        "workspace-status-update.exe"
    } else {
        "workspace-status-update"
    }
}

fn sibling_updater(current_exe: &Path) -> Option<PathBuf> {
    let path = current_exe.parent()?.join(updater_file_name());
    path.is_file().then_some(path)
}

fn updater_command(current_exe: &Path) -> Command {
    if let Some(path) = sibling_updater(current_exe) {
        Command::new(path)
    } else {
        Command::new("workspace-status-update")
    }
}

/// Exec `workspace-status-update` (sibling of this binary, then PATH).
///
/// On Unix this replaces the process. If exec fails, or on Windows after the
/// sidecar returns, the caller receives an [`ExitCode`].
pub(crate) fn run_self_update() -> ExitCode {
    let exe = match env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("failed to resolve current executable: {err}");
            return ExitCode::from(1);
        }
    };
    let mut cmd = updater_command(&exe);
    exec_updater(&mut cmd)
}

#[cfg(unix)]
fn exec_updater(cmd: &mut Command) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    eprintln!("failed to run workspace-status-update: {err}");
    ExitCode::from(1)
}

#[cfg(not(unix))]
fn exec_updater(cmd: &mut Command) -> ExitCode {
    match cmd.status() {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map(ExitCode::from)
            .unwrap_or(ExitCode::from(1)),
        Err(err) => {
            eprintln!("failed to run workspace-status-update: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "ws-update-sidecar-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn sibling_updater_finds_file_next_to_exe() {
        let dir = temp_dir();
        let exe = dir.join("ws");
        fs::write(&exe, b"").unwrap();
        let sidecar = dir.join(updater_file_name());
        fs::write(&sidecar, b"").unwrap();
        assert_eq!(sibling_updater(&exe).as_deref(), Some(sidecar.as_path()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sibling_updater_none_without_sidecar() {
        let dir = temp_dir();
        let exe = dir.join("ws");
        fs::write(&exe, b"").unwrap();
        assert_eq!(sibling_updater(&exe), None);
        let _ = fs::remove_dir_all(dir);
    }
}

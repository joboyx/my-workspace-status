//! `--update` execs the cargo-dist sidecar and does not open the TUI.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_workspace-status"))
}

fn ws_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ws"))
}

fn temp_dir() -> PathBuf {
    let path = std::env::temp_dir().join(format!(
        "ws-update-e2e-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::create_dir_all(&path).unwrap();
    path
}

#[cfg(unix)]
fn write_script(path: &Path, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, body).unwrap();
    let mut perms = fs::metadata(path).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(path, perms).unwrap();
}

#[test]
fn help_documents_update_flag() {
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("workspace-status --help");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--update"), "{text}");
    assert!(!text.contains("-U,"), "update should be long-only: {text}");
}

#[cfg(unix)]
#[test]
fn update_runs_path_sidecar_and_forwards_exit() {
    let dir = temp_dir();
    write_script(
        &dir.join("workspace-status-update"),
        "#!/bin/sh\necho path-sidecar\nexit 19\n",
    );
    let out = Command::new(bin())
        .arg("--update")
        .env("PATH", &dir)
        .output()
        .expect("workspace-status --update");
    assert_eq!(out.status.code(), Some(19));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "path-sidecar\n");
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn ws_update_runs_path_sidecar() {
    let dir = temp_dir();
    write_script(
        &dir.join("workspace-status-update"),
        "#!/bin/sh\necho ws-sidecar\nexit 0\n",
    );
    let out = Command::new(ws_bin())
        .arg("--update")
        .env("PATH", &dir)
        .output()
        .expect("ws --update");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "ws-sidecar\n");
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn update_prefers_sibling_over_path() {
    let dir = temp_dir();
    let sibling_dir = dir.join("bin");
    let decoy = dir.join("decoy");
    fs::create_dir_all(&sibling_dir).unwrap();
    fs::create_dir_all(&decoy).unwrap();
    let dest = sibling_dir.join("workspace-status");
    fs::copy(bin(), &dest).unwrap();
    write_script(
        &sibling_dir.join("workspace-status-update"),
        "#!/bin/sh\necho sibling\nexit 7\n",
    );
    write_script(
        &decoy.join("workspace-status-update"),
        "#!/bin/sh\necho path\nexit 3\n",
    );
    let out = Command::new(&dest)
        .arg("--update")
        .env("PATH", &decoy)
        .output()
        .expect("copied workspace-status --update");
    assert_eq!(out.status.code(), Some(7));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "sibling\n");
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn update_skips_tui_and_repo_filters() {
    let dir = temp_dir();
    write_script(
        &dir.join("workspace-status-update"),
        "#!/bin/sh\necho updated\nexit 0\n",
    );
    let out = Command::new(bin())
        .args(["--update", "--tui", "--plain", "missing-repo"])
        .env("PATH", &dir)
        .output()
        .expect("workspace-status --update with other flags");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "updated\n");
    assert!(
        !String::from_utf8_lossy(&out.stderr).contains("Unknown repo"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn update_missing_sidecar_is_nonzero() {
    let empty = temp_dir();
    let out = Command::new(bin())
        .arg("--update")
        .env("PATH", &empty)
        .output()
        .expect("workspace-status --update with empty PATH");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("workspace-status-update"),
        "stderr should mention the sidecar: {err}"
    );
    let _ = fs::remove_dir_all(empty);
}

#[test]
fn help_documents_startup_release_check() {
    let out = Command::new(bin())
        .arg("--help")
        .output()
        .expect("workspace-status --help");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains("6 hours"),
        "help should mention the 6h TUI startup check: {text}"
    );
    assert!(
        text.contains("newer published release") || text.contains("GitHub Release"),
        "help should mention the GitHub Release check: {text}"
    );
}

#[test]
fn plain_does_not_run_startup_update_check() {
    let dir = temp_dir();
    let store = dir.join("update-check.json");
    let state = dir.join("xdg-state");
    let out = Command::new(bin())
        .arg("--plain")
        .current_dir(&dir)
        .env("WS_STATUS_UPDATE_CHECK_STORE", &store)
        .env("XDG_STATE_HOME", &state)
        .env("HOME", &dir)
        .env("TERM", "dumb")
        .output()
        .expect("workspace-status --plain");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !store.exists(),
        "--plain must not write the update-check store"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn json_does_not_run_startup_update_check() {
    let dir = temp_dir();
    let store = dir.join("update-check.json");
    let state = dir.join("xdg-state");
    let out = Command::new(bin())
        .arg("--json")
        .current_dir(&dir)
        .env("WS_STATUS_UPDATE_CHECK_STORE", &store)
        .env("XDG_STATE_HOME", &state)
        .env("HOME", &dir)
        .env("TERM", "dumb")
        .output()
        .expect("workspace-status --json");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !store.exists(),
        "--json must not write the update-check store"
    );
    let _ = fs::remove_dir_all(dir);
}

#[cfg(unix)]
#[test]
fn update_flag_does_not_run_startup_check() {
    let dir = temp_dir();
    let store = dir.join("update-check.json");
    let state = dir.join("xdg-state");
    write_script(
        &dir.join("workspace-status-update"),
        "#!/bin/sh\necho updated\nexit 0\n",
    );
    let out = Command::new(bin())
        .arg("--update")
        .env("PATH", &dir)
        .env("WS_STATUS_UPDATE_CHECK_STORE", &store)
        .env("XDG_STATE_HOME", &state)
        .env("HOME", &dir)
        .output()
        .expect("workspace-status --update");
    assert!(out.status.success());
    assert_eq!(String::from_utf8_lossy(&out.stdout), "updated\n");
    assert!(
        !store.exists(),
        "--update must not write the update-check store"
    );
    let _ = fs::remove_dir_all(dir);
}

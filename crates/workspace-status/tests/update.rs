//! `--update` prints newer GitHub Release notes, then execs the cargo-dist sidecar.

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
    let dir = temp_dir();
    let out = Command::new(bin())
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
        .arg("--help")
        .output()
        .expect("workspace-status --help");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--update"), "{text}");
    assert!(!text.contains("-U,"), "update should be long-only: {text}");
    assert!(
        text.contains("GitHub Release notes") || text.contains("notes since this version"),
        "help should mention Release notes before the updater: {text}"
    );
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn help_documents_workspace_flag() {
    let dir = temp_dir();
    let out = Command::new(bin())
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
        .arg("--help")
        .output()
        .expect("workspace-status --help");
    assert!(out.status.success());
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("--workspace"), "{text}");
    assert!(text.contains("-C"), "{text}");
    let _ = fs::remove_dir_all(dir);
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
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
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
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
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
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
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
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
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
        .env("XDG_CONFIG_HOME", empty.join("xdg-config"))
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
    let dir = temp_dir();
    let out = Command::new(bin())
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
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
    let _ = fs::remove_dir_all(dir);
}

#[test]
fn plain_does_not_run_startup_update_check() {
    let dir = temp_dir();
    let store = dir.join("update-check.json");
    let state = dir.join("xdg-state");
    let out = Command::new(bin())
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
        .arg("--plain")
        .current_dir(&dir)
        .env("WS_STATUS_UPDATE_CHECK_STORE", &store)
        .env("XDG_STATE_HOME", &state)
        .env("HOME", &dir)
        .env("TERM", "dumb")
        .env_remove("WS_STATUS_WORKSPACE")
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
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
        .arg("--json")
        .current_dir(&dir)
        .env("WS_STATUS_UPDATE_CHECK_STORE", &store)
        .env("XDG_STATE_HOME", &state)
        .env("HOME", &dir)
        .env("TERM", "dumb")
        .env_remove("WS_STATUS_WORKSPACE")
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
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
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

#[cfg(unix)]
#[test]
fn update_prints_release_notes_then_runs_sidecar() {
    let dir = temp_dir();
    let current = env!("CARGO_PKG_VERSION");
    let json = r###"[
  {
    "tag_name": "v99.0.0",
    "draft": false,
    "prerelease": false,
    "body": "## [99.0.0] - 2026-08-28\n\n### Features\n\n- Show changelog on update\n\n## Install workspace-status 99.0.0\n\ncurl install\n"
  },
  {
    "tag_name": "v98.0.0",
    "draft": false,
    "prerelease": false,
    "body": "## [98.0.0] - 2026-08-27\n\n### Bug Fixes\n\n- Older fix\n\n## Install workspace-status 98.0.0\n\ncurl install\n"
  },
  {
    "tag_name": "v0.0.1",
    "draft": false,
    "prerelease": false,
    "body": "## [0.0.1]\n\n### Features\n\n- Ancient\n\n## Install workspace-status 0.0.1\n"
  }
]"###;
    let fixture = dir.join("releases.json");
    fs::write(&fixture, json.as_bytes()).unwrap();
    write_script(
        &dir.join("curl"),
        &format!("#!/bin/sh\nexec /bin/cat '{}'\n", fixture.display()),
    );
    write_script(
        &dir.join("workspace-status-update"),
        "#!/bin/sh\necho path-sidecar\nexit 0\n",
    );
    let out = Command::new(bin())
        .env("XDG_CONFIG_HOME", dir.join("xdg-config"))
        .arg("--update")
        .env("PATH", &dir)
        .output()
        .expect("workspace-status --update with fake curl");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        text.contains(&format!("Updating {current} -> 99.0.0")),
        "{text}"
    );
    assert!(text.contains("Show changelog on update"), "{text}");
    assert!(text.contains("Older fix"), "{text}");
    assert!(!text.contains("Ancient"), "{text}");
    assert!(!text.contains("curl install"), "{text}");
    assert!(
        text.contains("path-sidecar"),
        "sidecar should still run after notes: {text}"
    );
    let notes_idx = text.find("Show changelog on update").expect("notes");
    let sidecar_idx = text.find("path-sidecar").expect("sidecar");
    assert!(
        notes_idx < sidecar_idx,
        "notes should print before the sidecar: {text}"
    );
    let _ = fs::remove_dir_all(dir);
}

/// Run `--update` with a fake `curl` that logs its argv and stdin, with
/// `token` as `WORKSPACE_STATUS_GITHUB_TOKEN` (`None` removes it). Returns
/// the logged argv and stdin.
#[cfg(unix)]
fn update_curl_argv_and_stdin(token: Option<&str>) -> (String, String) {
    let dir = temp_dir();
    let argv_log = dir.join("curl-argv.log");
    let stdin_log = dir.join("curl-stdin.log");
    let fixture = dir.join("releases.json");
    fs::write(&fixture, "[]").unwrap();
    write_script(
        &dir.join("curl"),
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\n/bin/cat > '{}'\nexec /bin/cat '{}'\n",
            argv_log.display(),
            stdin_log.display(),
            fixture.display()
        ),
    );
    write_script(&dir.join("workspace-status-update"), "#!/bin/sh\nexit 0\n");
    let mut cmd = Command::new(bin());
    cmd.env("XDG_CONFIG_HOME", dir.join("xdg-config"))
        .arg("--update")
        .env("PATH", &dir);
    match token {
        Some(token) => cmd.env("WORKSPACE_STATUS_GITHUB_TOKEN", token),
        None => cmd.env_remove("WORKSPACE_STATUS_GITHUB_TOKEN"),
    };
    let out = cmd
        .output()
        .expect("workspace-status --update with fake curl");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let argv = fs::read_to_string(&argv_log).expect("fake curl ran");
    let stdin = fs::read_to_string(&stdin_log).expect("fake curl read stdin");
    let _ = fs::remove_dir_all(dir);
    (argv, stdin)
}

#[cfg(unix)]
#[test]
fn update_passes_github_token_to_curl_on_stdin_not_argv() {
    let token = "ghp_fake_token_for_argv_test";
    let (argv, stdin) = update_curl_argv_and_stdin(Some(&format!("  {token}  ")));
    assert!(
        !argv.contains(token),
        "token must not be in curl argv:\n{argv}"
    );
    assert!(
        argv.lines().any(|arg| arg == "@-"),
        "curl reads headers from stdin (-H @-):\n{argv}"
    );
    assert_eq!(stdin, format!("Authorization: Bearer {token}\n"));

    let (argv, stdin) = update_curl_argv_and_stdin(None);
    assert!(
        !argv.lines().any(|arg| arg == "@-"),
        "no token, no stdin header:\n{argv}"
    );
    assert!(!argv.contains("Authorization"), "{argv}");
    assert_eq!(stdin, "", "no token, nothing on stdin");

    let (argv, stdin) = update_curl_argv_and_stdin(Some("   "));
    assert!(
        !argv.lines().any(|arg| arg == "@-"),
        "blank token counts as unset:\n{argv}"
    );
    assert_eq!(stdin, "", "blank token, nothing on stdin");
}

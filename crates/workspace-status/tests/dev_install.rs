//! CI guard for the side-by-side dev install (`scripts/install-dev.sh`).
//!
//! The dev install must never write or remove the released names (`ws`,
//! `workspace-status`, `workspace-status-update`). It must stamp the build
//! with `WS_STATUS_DEV_BUILD` and build into its own target dir so the dev
//! marker never leaks into `target/release`. The released `[[bin]]` names
//! stay exactly `workspace-status` and `ws`.

const INSTALL_DEV_SH: &str = include_str!("../../../scripts/install-dev.sh");
const CARGO_TOML: &str = include_str!("../Cargo.toml");

/// Values of the `DEV_NAMES=(...)` array in the script.
fn dev_names(src: &str) -> Vec<&str> {
    let line = src
        .lines()
        .find(|line| line.contains("DEV_NAMES=("))
        .expect("install-dev.sh declares DEV_NAMES");
    let inner = line
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(inner, _)| inner)
        .expect("DEV_NAMES=(...) on one line");
    inner.split_whitespace().collect()
}

/// `name = "..."` values under each `[[bin]]` table.
fn bin_names(toml: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut in_bin = false;
    for line in toml.lines().map(str::trim) {
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
            continue;
        }
        if !in_bin {
            continue;
        }
        if let Some(value) = line
            .strip_prefix("name")
            .map(str::trim_start)
            .and_then(|rest| rest.strip_prefix('='))
        {
            names.push(value.trim().trim_matches('"'));
        }
    }
    names
}

#[test]
fn install_dev_only_names_dev_targets() {
    let names = dev_names(INSTALL_DEV_SH);
    assert_eq!(
        names,
        [
            "ws-dev",
            "workspace-status-dev",
            "workspace-status-update-dev"
        ]
    );
    for name in &names {
        assert!(name.ends_with("-dev"), "non-dev target: {name}");
    }
    for line in INSTALL_DEV_SH.lines().map(str::trim) {
        if line.starts_with('#') || line.starts_with("echo") {
            continue;
        }
        for released in ["ws", "workspace-status", "workspace-status-update"] {
            assert!(
                !line.contains(&format!("$bin_dir/{released}\""))
                    && !line.contains(&format!("$bin_dir/{released} ")),
                "install-dev.sh touches released {released}: {line}"
            );
        }
    }
    assert!(
        INSTALL_DEV_SH.contains("!= *-dev"),
        "install-dev.sh must refuse non -dev names"
    );
}

#[test]
fn install_dev_stamps_marker_and_uses_separate_target_dir() {
    assert!(INSTALL_DEV_SH.contains("WS_STATUS_DEV_BUILD=\"$marker\""));
    assert!(INSTALL_DEV_SH.contains("CARGO_TARGET_DIR=\"$target_dir\""));
    assert!(INSTALL_DEV_SH.contains("${WS_DEV_TARGET_DIR:-$repo_root/target/dev-install}"));
    assert!(INSTALL_DEV_SH.contains("${WS_DEV_BIN_DIR:-$HOME/.local/bin}"));
    assert!(INSTALL_DEV_SH.contains("cargo build --release --locked -p workspace-status"));
    assert!(
        !INSTALL_DEV_SH.contains("CARGO_BUILD_JOBS="),
        "install-dev.sh must not hard-code CARGO_BUILD_JOBS"
    );
}

#[test]
fn released_bin_names_are_unchanged() {
    assert_eq!(bin_names(CARGO_TOML), ["workspace-status", "ws"]);
}

#[test]
fn parsers_reject_drift() {
    assert_eq!(dev_names("readonly DEV_NAMES=(a-dev b)\n"), ["a-dev", "b"]);
    assert_eq!(
        bin_names(
            "[package]\nname = \"pkg\"\n\n[[bin]]\nname = \"a\"\n[dependencies]\nname = \"x\"\n"
        ),
        ["a"]
    );
}

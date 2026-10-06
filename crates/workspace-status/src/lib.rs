//! Workspace-status library: discovery, snapshot, --plain/--json, ratatui TUI.

/// Cargo package version compiled into this crate (`CARGO_PKG_VERSION`).
///
/// Same string `ws --update` compares to GitHub release tags. The `?` help
/// overlay paints this in the lower-right. Do not add a second version literal.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Dev-build marker compiled in from `WS_STATUS_DEV_BUILD`.
///
/// `scripts/install-dev.sh` sets it to the short git sha of the checkout,
/// plus `-dirty` when the tree has local changes. Release and plain
/// `cargo install` builds leave it unset (`None`). A dev build skips the
/// TUI-startup release prompt, and `--update` rebuilds the checkout through
/// `workspace-status-update-dev` instead of the cargo-dist sidecar.
pub const DEV_BUILD: Option<&str> = option_env!("WS_STATUS_DEV_BUILD");

/// Version label shown by the `?` help footer and `--version`.
///
/// `v{APP_VERSION}` for a release build, `v{APP_VERSION}-dev ({sha})` for a
/// dev build. Built from [`APP_VERSION`] and [`DEV_BUILD`] only.
pub fn version_label() -> String {
    version_label_for(APP_VERSION, DEV_BUILD)
}

/// [`version_label`] with injected inputs (unit-tested).
fn version_label_for(version: &str, dev_build: Option<&str>) -> String {
    match dev_build {
        Some(sha) => format!("v{version}-dev ({sha})"),
        None => format!("v{version}"),
    }
}

pub mod actions;
pub mod cli;
pub mod config;
pub mod discovery;
pub mod file_index;
pub mod git;
pub mod helpers;
pub(crate) mod parallel;
pub mod render;
pub mod settings;
pub mod snapshot;
#[cfg(test)]
pub(crate) mod testutil;
pub mod tui;
pub mod update;
pub mod update_check;
pub mod worktrees;

pub use cli::cli_main;
pub use config::{load_workspace_status_config, WorkspaceStatusConfig};
pub use discovery::{collect_snapshots, validate_filter_repos};
pub use snapshot::{
    build_workspace_snapshot, serialize_workspace_snapshot, visible_workspace_snapshot,
    WorkspaceSnapshot,
};
pub use tui::{should_open_tui, HeadlessFlags};

#[cfg(test)]
mod version_tests {
    use super::*;

    #[test]
    fn version_label_release_and_dev() {
        assert_eq!(version_label_for("0.1.224", None), "v0.1.224");
        assert_eq!(
            version_label_for("0.1.224", Some("abc1234")),
            "v0.1.224-dev (abc1234)"
        );
        assert_eq!(
            version_label_for("0.1.224", Some("abc1234-dirty")),
            "v0.1.224-dev (abc1234-dirty)"
        );
        assert_eq!(version_label(), version_label_for(APP_VERSION, DEV_BUILD));
    }
}

//! Workspace-status library: discovery, snapshot, --plain/--json, ratatui TUI.

/// Cargo package version compiled into this crate (`CARGO_PKG_VERSION`).
///
/// Same string `ws --update` compares to GitHub release tags. The `?` help
/// overlay paints this in the lower-right. Do not add a second version literal.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub mod actions;
pub mod cli;
pub mod config;
pub mod discovery;
pub mod git;
pub mod helpers;
pub mod render;
pub mod snapshot;
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

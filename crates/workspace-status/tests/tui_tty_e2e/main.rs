//! Real-TTY e2e for the ratatui TUI.
//!
//! Spawns the `workspace-status` binary on a PTY so the live loop's
//! `event::read` sees keys and xterm SGR mouse bytes. This is not the
//! TestBackend suite (`tui_headless_e2e.rs`) and not screenshot capture
//! (`scripts/capture-demo-stills.sh`).
//!
//! Unix only (PTY). Windows `cargo test --workspace` compiles this crate
//! with no tests.

#[cfg(unix)]
#[path = "../common/mod.rs"]
mod common;
#[cfg(unix)]
mod desktop;
#[cfg(unix)]
mod harness;
#[cfg(unix)]
mod leftover;
#[cfg(unix)]
mod seed;
#[cfg(unix)]
mod support;

//! Real-TTY e2e for the ratatui TUI.
//!
//! Spawns the `workspace-status` binary on a PTY so the live loop's
//! `event::read` sees keys and xterm SGR mouse bytes. This is not
//! screenshot capture (`scripts/capture-demo-stills.sh`).
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
mod human;
#[cfg(unix)]
mod pty_live_g_chords;
#[cfg(unix)]
mod seed;
#[cfg(unix)]
mod slow_git;
#[cfg(unix)]
mod support;

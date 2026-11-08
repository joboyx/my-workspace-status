//! Shared TUI e2e fixtures for the PTY suite.
//!
//! `tui_tty_e2e` mods this directory. It is not a second harness.
//! `tag_release` also mods it for the git helpers in `seed.rs`.
//! `fixture.rs` (git env, unique temp dirs) is std-only, so other test
//! binaries mod that one file alone.
//! Demo GIF clips stay in `scripts/capture-demo-stills.sh`.

pub mod fake_forge;
pub mod fixture;
pub mod hscroll;
pub mod pr_scenarios;
pub mod seed;

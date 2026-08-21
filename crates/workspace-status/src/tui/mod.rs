//! Ratatui TUI for `workspace-status` / `ws`.
//!
//! Opens only when stdout is a TTY and the user did not pass
//! `--plain`, `--json`, `-v`, `-p`, or `-d`.

mod action;
mod app;
mod branches;
mod diff;
pub(crate) mod editor;
mod graph_load;
mod keys;
mod ops;
mod render;
pub(crate) mod search;
mod stash;
mod state;
mod tree;
pub(crate) mod viewed;
pub(crate) mod fetch;
pub(crate) mod watch;

pub use app::{collect_full_snapshot, run_tui, TuiOpts};

/// CLI flags that decide TUI vs headless. Testable without a real TTY.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeadlessFlags {
    pub plain: bool,
    pub json: bool,
    pub verbose: bool,
    pub pull: bool,
    pub default_branch: bool,
}

/// True when the Rust binary should open the ratatui TUI.
///
/// A TTY with none of `--plain` / `--json` / `-v` / `-p` / `-d` opens the TUI.
/// `-a` and `-f` still open the TUI. `-f` fetches after the first paint.
pub fn should_open_tui(stdout_is_tty: bool, flags: HeadlessFlags) -> bool {
    if !stdout_is_tty {
        return false;
    }
    if flags.plain || flags.json || flags.verbose || flags.pull || flags.default_branch {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_without_flags_opens_tui() {
        assert!(should_open_tui(true, HeadlessFlags::default()));
    }

    #[test]
    fn non_tty_stays_headless() {
        assert!(!should_open_tui(false, HeadlessFlags::default()));
    }

    #[test]
    fn headless_flags_block_tui_even_on_tty() {
        for flags in [
            HeadlessFlags {
                plain: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                json: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                verbose: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                pull: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                default_branch: true,
                ..HeadlessFlags::default()
            },
        ] {
            assert!(!should_open_tui(true, flags), "{flags:?}");
        }
    }
}

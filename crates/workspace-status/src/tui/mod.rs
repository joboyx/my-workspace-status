//! Ratatui TUI for `workspace-status` / `ws`.
//!
//! Opens when stdout is a TTY (or `-i` / `--tui`) and the user did not pass
//! `--plain`, `--json`, `-v`, `-p`, or `-d`.

mod action;
mod app;
mod branches;
mod chrome;
mod commit_files;
mod ctrl_c_exit;
mod diff;
mod drill;
pub(crate) mod editor;
mod event_pump;
pub(crate) mod fetch;
mod gates;
mod graph_focus;
mod graph_load;
mod headless;
mod help;
mod icons;
mod keys;
mod ops;
mod render;
pub(crate) mod search;
mod split;
mod stash;
mod state;
mod theme;
mod tree;
pub(crate) mod tty;
pub(crate) mod viewed;
pub(crate) mod watch;

pub use app::{collect_full_snapshot, run_tui, TuiOpts};
pub use headless::HeadlessTui;

/// CLI flags that decide TUI vs headless. Testable without a real TTY.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeadlessFlags {
    pub plain: bool,
    pub json: bool,
    pub verbose: bool,
    pub pull: bool,
    pub default_branch: bool,
    /// `-i` / `--tui`: open the TUI even when stdout is not a TTY.
    pub force_tui: bool,
}

/// True when the Rust binary should open the ratatui TUI.
///
/// A TTY (or `-i` / `--tui`) with none of `--plain` / `--json` / `-v` / `-p` / `-d`
/// opens the TUI. Those headless flags still win over `--tui`.
/// `-a` and `-f` still open the TUI. `-f` fetches after the first paint.
pub fn should_open_tui(stdout_is_tty: bool, flags: HeadlessFlags) -> bool {
    if flags.plain || flags.json || flags.verbose || flags.pull || flags.default_branch {
        return false;
    }
    stdout_is_tty || flags.force_tui
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
    fn force_tui_opens_on_non_tty() {
        assert!(should_open_tui(
            false,
            HeadlessFlags {
                force_tui: true,
                ..HeadlessFlags::default()
            },
        ));
    }

    #[test]
    fn headless_flags_win_over_force_tui() {
        for flags in [
            HeadlessFlags {
                force_tui: true,
                plain: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                force_tui: true,
                json: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                force_tui: true,
                verbose: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                force_tui: true,
                pull: true,
                ..HeadlessFlags::default()
            },
            HeadlessFlags {
                force_tui: true,
                default_branch: true,
                ..HeadlessFlags::default()
            },
        ] {
            assert!(!should_open_tui(false, flags), "{flags:?}");
            assert!(!should_open_tui(true, flags), "{flags:?}");
        }
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

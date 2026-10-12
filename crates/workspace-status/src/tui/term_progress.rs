//! OSC 9;4 terminal progress for multi-repo remote batches.
//!
//! Terminals that support it (for example Windows Terminal, ConEmu, and
//! Ghostty) show the bar in the tab or taskbar. Others ignore the
//! sequence. Ratatui cannot emit it, so `draw_frame` writes it after each
//! frame, and only when the state changes. `restore_terminal` writes
//! [`PROGRESS_CLEAR`] on exit only while a bar shows ([`PROGRESS_SHOWN`]):
//! some terminals show any OSC 9 as a desktop notification, so a launch
//! with no batch writes no OSC 9;4 at all.

use std::io::{self, Write};
use std::sync::atomic::{AtomicBool, Ordering};

use super::anim::BatchProgress;

/// OSC 9;4 sequence that removes the terminal progress bar.
pub const PROGRESS_CLEAR: &str = "\x1b]9;4;0;0\x07";

/// True while the terminal shows a progress bar: the last state a
/// [`TermProgress::new`] writer wrote was not [`ProgressState::Clear`].
/// The exit restore and the panic hook write [`PROGRESS_CLEAR`] only then.
pub static PROGRESS_SHOWN: AtomicBool = AtomicBool::new(false);

/// One terminal progress state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgressState {
    /// No progress bar (OSC 9;4 state 0).
    Clear,
    /// Normal progress bar at a percent (OSC 9;4 state 1).
    Normal(u8),
    /// Error progress bar at a percent (OSC 9;4 state 2).
    Error(u8),
}

/// Progress state for the current batch: [`ProgressState::Clear`] with no
/// batch, else `done * 100 / total` percent, [`ProgressState::Error`] once a
/// repo in the batch failed.
pub fn progress_for(batch: Option<&BatchProgress>) -> ProgressState {
    let Some(batch) = batch.filter(|batch| batch.total > 0) else {
        return ProgressState::Clear;
    };
    let percent = u8::try_from((batch.done * 100 / batch.total).min(100)).unwrap_or(100);
    if batch.failed {
        ProgressState::Error(percent)
    } else {
        ProgressState::Normal(percent)
    }
}

/// The OSC 9;4 bytes for `state`: `ESC ] 9 ; 4 ; <st> ; <pr> BEL`.
pub fn osc_9_4(state: ProgressState) -> String {
    let (st, pr) = match state {
        ProgressState::Clear => (0, 0),
        ProgressState::Normal(percent) => (1, percent),
        ProgressState::Error(percent) => (2, percent),
    };
    format!("\x1b]9;4;{st};{pr}\x07")
}

/// Writes the progress state to the terminal when it changes.
#[derive(Debug)]
pub struct TermProgress {
    /// `termProgress` setting. Off writes nothing.
    enabled: bool,
    /// The state the terminal shows now.
    last: ProgressState,
    /// Set while `last` is not clear; [`PROGRESS_SHOWN`] outside tests.
    shown: &'static AtomicBool,
}

impl TermProgress {
    /// A writer that starts from a terminal with no progress bar and keeps
    /// [`PROGRESS_SHOWN`] up to date.
    pub fn new(enabled: bool) -> Self {
        Self::with_flag(enabled, &PROGRESS_SHOWN)
    }

    /// [`Self::new`] that keeps `shown` instead of [`PROGRESS_SHOWN`], so a
    /// test does not share the process-wide flag.
    pub(super) fn with_flag(enabled: bool, shown: &'static AtomicBool) -> Self {
        Self {
            enabled,
            last: ProgressState::Clear,
            shown,
        }
    }

    /// Write `next` to `out` and flush when it differs from the last state
    /// written. A disabled writer writes nothing.
    pub fn write_if_changed<W: Write>(
        &mut self,
        out: &mut W,
        next: ProgressState,
    ) -> io::Result<()> {
        if !self.enabled || next == self.last {
            return Ok(());
        }
        out.write_all(osc_9_4(next).as_bytes())?;
        out.flush()?;
        self.last = next;
        self.shown
            .store(next != ProgressState::Clear, Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(done: usize, total: usize, failed: bool) -> BatchProgress {
        BatchProgress {
            done,
            total,
            failed,
        }
    }

    #[test]
    fn osc_9_4_bytes_for_normal_error_and_clear() {
        assert_eq!(osc_9_4(ProgressState::Normal(50)), "\x1b]9;4;1;50\x07");
        assert_eq!(osc_9_4(ProgressState::Error(33)), "\x1b]9;4;2;33\x07");
        assert_eq!(osc_9_4(ProgressState::Clear), "\x1b]9;4;0;0\x07");
        assert_eq!(osc_9_4(ProgressState::Clear), PROGRESS_CLEAR);
    }

    #[test]
    fn progress_for_none_is_clear_and_counts_percent() {
        assert_eq!(progress_for(None), ProgressState::Clear);
        assert_eq!(
            progress_for(Some(&batch(0, 0, false))),
            ProgressState::Clear,
            "an empty batch has no percent"
        );
        assert_eq!(
            progress_for(Some(&batch(0, 2, false))),
            ProgressState::Normal(0)
        );
        assert_eq!(
            progress_for(Some(&batch(1, 3, false))),
            ProgressState::Normal(33)
        );
        assert_eq!(
            progress_for(Some(&batch(3, 3, false))),
            ProgressState::Normal(100)
        );
        assert_eq!(
            progress_for(Some(&batch(1, 4, true))),
            ProgressState::Error(25)
        );
    }

    #[test]
    fn writer_emits_only_on_change() {
        static WRITER_SHOWN: AtomicBool = AtomicBool::new(false);
        let mut progress = TermProgress::with_flag(true, &WRITER_SHOWN);
        let mut out = Vec::new();
        progress
            .write_if_changed(&mut out, ProgressState::Clear)
            .unwrap();
        assert!(out.is_empty(), "the terminal starts clear");
        assert!(!WRITER_SHOWN.load(Ordering::SeqCst));
        for (state, shown) in [
            (ProgressState::Normal(0), true),
            (ProgressState::Normal(0), true),
            (ProgressState::Normal(50), true),
            (ProgressState::Error(50), true),
            (ProgressState::Error(50), true),
            (ProgressState::Clear, false),
            (ProgressState::Clear, false),
        ] {
            progress.write_if_changed(&mut out, state).unwrap();
            assert_eq!(
                WRITER_SHOWN.load(Ordering::SeqCst),
                shown,
                "shown flag after {state:?}"
            );
        }
        assert_eq!(
            String::from_utf8(out).unwrap(),
            "\x1b]9;4;1;0\x07\x1b]9;4;1;50\x07\x1b]9;4;2;50\x07\x1b]9;4;0;0\x07"
        );
    }

    #[test]
    fn disabled_writer_emits_nothing() {
        static DISABLED_SHOWN: AtomicBool = AtomicBool::new(false);
        let mut progress = TermProgress::with_flag(false, &DISABLED_SHOWN);
        let mut out = Vec::new();
        for state in [
            ProgressState::Normal(10),
            ProgressState::Error(90),
            ProgressState::Clear,
        ] {
            progress.write_if_changed(&mut out, state).unwrap();
        }
        assert!(out.is_empty());
        assert!(
            !DISABLED_SHOWN.load(Ordering::SeqCst),
            "a disabled writer never shows a bar"
        );
    }
}

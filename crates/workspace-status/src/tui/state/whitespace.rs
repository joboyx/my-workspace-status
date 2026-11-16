//! The "ignore whitespace" diff setting (`w`): one global flag for every
//! diff pane and tab.
//!
//! [`AppState::toggle_ignore_whitespace`] flips
//! [`AppState::ignore_whitespace`], saves the last-used value to the
//! UI-state store, and reloads the diff on screen. Every diff load reads
//! the live flag through [`AppState::diff_opts`], so the reload (and any
//! later watch reload) gets `-w` or drops it. A tab that was parked while
//! the flag changed reloads when it becomes active
//! ([`AppState::stale_whitespace_reload`]); an Explorer tab does it
//! through its live-reload stamp, which records the flag for a changed
//! file. The reload itself is shared with Ctrl-o (`diff_reload.rs`).

use super::super::action::Effect;
use super::super::status::StatusMessage;
use super::super::ui_state_store::save_ignore_whitespace;
use super::{persist_failed_status, AppState};

/// Status after `w` turns the setting on.
pub(crate) const WHITESPACE_HIDDEN: &str = "whitespace hidden";
/// Status after `w` turns the setting off.
pub(crate) const WHITESPACE_SHOWN: &str = "whitespace shown";

impl AppState {
    /// `w` / palette Ignore / show whitespace: flip the setting, save it as
    /// the last-used value, and reload the shown diff.
    ///
    /// A failed save keeps the flip and says so on the status line, like
    /// the activity-sparkline toggle. The cursor stays near the same text
    /// (the Ctrl-O anchor), and a DiffVisual range is dropped because the
    /// rows change. With no diff on screen only the flag flips.
    pub(super) fn toggle_ignore_whitespace(&mut self) -> Effect {
        let on = !self.ignore_whitespace;
        self.ignore_whitespace = on;
        self.status = if on {
            WHITESPACE_HIDDEN.into()
        } else {
            WHITESPACE_SHOWN.into()
        };
        if let Err(err) = save_ignore_whitespace(&self.ui_store_path, on) {
            self.status = StatusMessage::error(persist_failed_status("whitespace", err));
        }
        self.clear_diff_visual();
        if self.is_explorer_tab() {
            // The stamp of a changed file records the flag, so the active
            // inner diff reloads; a clean file view does not.
            return self.active_explorer_live_reload();
        }
        self.anchored_shown_diff_reload()
    }

    /// Reload of the shown diff when it was loaded with the other
    /// whitespace setting, else [`Effect::None`].
    ///
    /// Runs when a tab becomes active: a compare tab or the Workspace tab
    /// that was parked while `w` flipped still holds the old content. The
    /// loaded content's own marker decides, so the painted diff and the
    /// setting always end up the same. Explorer tabs reload through their
    /// stamp instead.
    pub(super) fn stale_whitespace_reload(&mut self) -> Effect {
        if self.is_explorer_tab() || !self.right_is_diff() {
            return Effect::None;
        }
        if self.current_diff_content().ignore_whitespace == self.ignore_whitespace {
            return Effect::None;
        }
        self.anchored_shown_diff_reload()
    }
}

#[cfg(test)]
mod tests;

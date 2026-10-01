//! Status message with a kind: the breadcrumb trailing slot and overlay status rows.
//!
//! [`StatusKind::Info`] and [`StatusKind::Ok`] expire after [`STATUS_TTL_MS`]
//! once they show in the slot. Progress, warn, and error stay until another
//! message replaces them. A plain `String` / `&str` converts to an info
//! message, so `state.status = "…".into()` keeps working.

use std::fmt;
use std::ops::Deref;
use std::time::{Duration, Instant};

use ratatui::style::Color;

use super::theme::Palette;

/// How long an info / ok message stays after it first shows (ms).
pub const STATUS_TTL_MS: u64 = 4000;

/// What a status message reports. Picks the paint color and the expiry.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StatusKind {
    /// Neutral note (`showing ignored repos`). Expires.
    #[default]
    Info,
    /// Work in flight (`Fetching 1/3…`, `reverting foo…`). Stays until replaced.
    Progress,
    /// A finished action (`Fetched 3 repos`). Expires.
    Ok,
    /// A refusal or a key that did nothing (`busy: …`, `nothing to push`). Stays.
    Warn,
    /// A failed git or IO operation. Stays.
    Error,
}

impl StatusKind {
    /// True when a message of this kind clears itself after [`STATUS_TTL_MS`].
    pub fn expires(self) -> bool {
        matches!(self, Self::Info | Self::Ok)
    }

    /// Theme color for this kind.
    pub fn color(self, palette: Palette) -> Color {
        match self {
            Self::Info | Self::Progress => palette.muted,
            Self::Ok => palette.added,
            Self::Warn => palette.modified,
            Self::Error => palette.deleted,
        }
    }
}

/// Status text plus its [`StatusKind`] and the instant it first showed.
///
/// Derefs to `str`, compares equal to `&str` / `String` by text, and
/// converts from `String` / `&str` as [`StatusKind::Info`].
#[derive(Clone, Debug, Default)]
pub struct StatusMessage {
    text: String,
    kind: StatusKind,
    /// Stamped by [`Self::expiry_ms`] the first time the message is visible.
    shown_at: Option<Instant>,
}

impl StatusMessage {
    /// Message of `kind`. The expiry clock starts when it first shows.
    pub fn new(kind: StatusKind, text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            kind,
            shown_at: None,
        }
    }

    /// [`StatusKind::Info`] message.
    pub fn info(text: impl Into<String>) -> Self {
        Self::new(StatusKind::Info, text)
    }

    /// [`StatusKind::Progress`] message.
    pub fn progress(text: impl Into<String>) -> Self {
        Self::new(StatusKind::Progress, text)
    }

    /// [`StatusKind::Ok`] message.
    pub fn ok(text: impl Into<String>) -> Self {
        Self::new(StatusKind::Ok, text)
    }

    /// [`StatusKind::Warn`] message.
    pub fn warn(text: impl Into<String>) -> Self {
        Self::new(StatusKind::Warn, text)
    }

    /// [`StatusKind::Error`] message.
    pub fn error(text: impl Into<String>) -> Self {
        Self::new(StatusKind::Error, text)
    }

    /// Kind of the current message.
    pub fn kind(&self) -> StatusKind {
        self.kind
    }

    /// Clear the text. The kind falls back to [`StatusKind::Info`].
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Milliseconds until this message expires, stamping the start on first call.
    ///
    /// `None` when the message is empty or its kind does not expire.
    pub fn expiry_ms(&mut self, now: Instant) -> Option<u64> {
        if self.text.is_empty() || !self.kind.expires() {
            return None;
        }
        let shown = *self.shown_at.get_or_insert(now);
        let until = shown + Duration::from_millis(STATUS_TTL_MS);
        Some(until.saturating_duration_since(now).as_millis() as u64)
    }

    /// Clear the message when its time is up. Returns true when it cleared.
    pub fn expire(&mut self, now: Instant) -> bool {
        if self.expiry_ms(now) == Some(0) {
            self.clear();
            true
        } else {
            false
        }
    }
}

impl Deref for StatusMessage {
    type Target = str;

    fn deref(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for StatusMessage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl From<String> for StatusMessage {
    fn from(text: String) -> Self {
        Self::info(text)
    }
}

impl From<&str> for StatusMessage {
    fn from(text: &str) -> Self {
        Self::info(text)
    }
}

impl PartialEq for StatusMessage {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.kind == other.kind
    }
}

impl Eq for StatusMessage {}

impl PartialEq<str> for StatusMessage {
    fn eq(&self, other: &str) -> bool {
        self.text == other
    }
}

impl PartialEq<&str> for StatusMessage {
    fn eq(&self, other: &&str) -> bool {
        self.text == *other
    }
}

impl PartialEq<String> for StatusMessage {
    fn eq(&self, other: &String) -> bool {
        &self.text == other
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::theme::ThemeId;

    #[test]
    fn plain_strings_convert_to_info() {
        let status: StatusMessage = "refreshed".into();
        assert_eq!(status.kind(), StatusKind::Info);
        assert_eq!(status, "refreshed");
        let owned: StatusMessage = String::from("x").into();
        assert_eq!(owned.kind(), StatusKind::Info);
    }

    #[test]
    fn info_and_ok_expire_after_ttl_from_first_show() {
        let t0 = Instant::now();
        for mut status in [StatusMessage::info("a"), StatusMessage::ok("b")] {
            assert_eq!(status.expiry_ms(t0), Some(STATUS_TTL_MS));
            assert!(!status.expire(t0 + Duration::from_millis(STATUS_TTL_MS - 1)));
            assert!(!status.is_empty());
            assert!(status.expire(t0 + Duration::from_millis(STATUS_TTL_MS)));
            assert!(status.is_empty());
            assert_eq!(status.kind(), StatusKind::Info);
        }
    }

    #[test]
    fn warn_error_and_progress_stay_until_replaced() {
        let t0 = Instant::now();
        let late = t0 + Duration::from_secs(3600);
        for mut status in [
            StatusMessage::warn("busy"),
            StatusMessage::error("push failed"),
            StatusMessage::progress("Fetching 1/3…"),
        ] {
            assert_eq!(status.expiry_ms(t0), None);
            assert!(!status.expire(late));
            assert!(!status.is_empty());
        }
    }

    #[test]
    fn empty_message_never_schedules_a_wake() {
        let mut status = StatusMessage::default();
        assert_eq!(status.expiry_ms(Instant::now()), None);
    }

    #[test]
    fn kind_picks_theme_color() {
        let palette = ThemeId::default().palette();
        assert_eq!(StatusKind::Info.color(palette), palette.muted);
        assert_eq!(StatusKind::Progress.color(palette), palette.muted);
        assert_eq!(StatusKind::Ok.color(palette), palette.added);
        assert_eq!(StatusKind::Warn.color(palette), palette.modified);
        assert_eq!(StatusKind::Error.color(palette), palette.deleted);
    }
}

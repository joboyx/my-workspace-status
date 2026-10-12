//! Toast stack: one-shot acks (`copied`, `Fetched 3 repos`) shown in the
//! bottom-right corner of the panes, fading out through the add-flash ramp.
//!
//! [`super::state::AppState::route_ack_to_toast`] moves a
//! [`super::status::StatusKind::Ok`] status here at the top of each draw.
//! Errors, warnings, progress, and info notes stay in the status slot.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::watch::FLASH_MS;

/// How long a toast shows, fade included (ms).
pub const TOAST_TTL_MS: u64 = 2500;

/// The last part of [`TOAST_TTL_MS`] in which the toast fades out (ms).
/// Strength drops from 1.0 to 0.0 over this window.
pub const TOAST_FADE_MS: u64 = FLASH_MS;

/// Most toasts on screen at once. A push past it drops the oldest.
pub const TOAST_MAX: usize = 3;

/// One acknowledgement and the instant it was pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// Text shown in the toast (`copied`, `Fetched 3 repos`).
    pub text: String,
    /// When the toast was pushed. Its TTL counts from here.
    pub at: Instant,
}

/// Live toasts, oldest first. At most [`TOAST_MAX`].
#[derive(Clone, Debug, Default)]
pub struct ToastStack {
    items: VecDeque<Toast>,
}

impl ToastStack {
    /// Add a toast pushed at `now`. Drops the oldest past [`TOAST_MAX`].
    pub fn push(&mut self, text: impl Into<String>, now: Instant) {
        self.items.push_back(Toast {
            text: text.into(),
            at: now,
        });
        while self.items.len() > TOAST_MAX {
            self.items.pop_front();
        }
    }

    /// Drop every toast whose [`TOAST_TTL_MS`] has run out at `now`.
    pub fn prune(&mut self, now: Instant) {
        self.items
            .retain(|toast| toast_strength(now.saturating_duration_since(toast.at)) > 0.0);
    }

    /// True when no toast is held.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Toasts still showing at `now`, oldest first, each with its fade
    /// strength (1.0 = full, falling toward 0.0 in the last
    /// [`TOAST_FADE_MS`]).
    pub fn visible(&self, now: Instant) -> impl Iterator<Item = (&Toast, f32)> {
        self.items.iter().filter_map(move |toast| {
            let strength = toast_strength(now.saturating_duration_since(toast.at));
            (strength > 0.0).then_some((toast, strength))
        })
    }
}

/// Fade strength of a toast `age` old: 1.0 until the fade window, then
/// linear down to 0.0 at [`TOAST_TTL_MS`].
pub fn toast_strength(age: Duration) -> f32 {
    let ms = age.as_millis() as u64;
    let fade_start = TOAST_TTL_MS - TOAST_FADE_MS;
    if ms >= TOAST_TTL_MS {
        0.0
    } else if ms <= fade_start {
        1.0
    } else {
        1.0 - (ms - fade_start) as f32 / TOAST_FADE_MS as f32
    }
}

#[cfg(test)]
mod tests {
    use super::super::theme::ThemeId;
    use super::super::watch::FlashKind;
    use super::*;

    #[test]
    fn toast_fades_through_the_flash_ramp_then_expires() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut stack = ToastStack::default();
        stack.push("copied", t0);
        let strength = |ms: u64| {
            stack
                .visible(at(ms))
                .next()
                .map(|(toast, s)| {
                    assert_eq!(toast.text, "copied");
                    s
                })
                .unwrap_or(0.0)
        };
        assert_eq!(strength(0), 1.0);
        assert_eq!(strength(TOAST_TTL_MS - TOAST_FADE_MS), 1.0, "holds full");
        let mid = strength(TOAST_TTL_MS - TOAST_FADE_MS / 2);
        assert!(mid > 0.4 && mid < 0.6, "half way through the fade: {mid}");
        assert!(strength(TOAST_TTL_MS - 1) > 0.0);
        assert_eq!(strength(TOAST_TTL_MS), 0.0, "gone at the TTL");

        let pal = ThemeId::default().palette();
        let bg = |ms: u64| pal.flash_bg_for(FlashKind::Add, strength(ms));
        let steps = [
            bg(0),
            bg(TOAST_TTL_MS - TOAST_FADE_MS * 5 / 8),
            bg(TOAST_TTL_MS - TOAST_FADE_MS * 3 / 8),
            bg(TOAST_TTL_MS - TOAST_FADE_MS / 8),
        ];
        assert_eq!(steps, pal.flash_ramp.map(Some), "walks every ramp step");
        assert_eq!(bg(TOAST_TTL_MS), None);

        stack.prune(at(TOAST_TTL_MS - 1));
        assert!(!stack.is_empty(), "still fading");
        stack.prune(at(TOAST_TTL_MS));
        assert!(stack.is_empty(), "pruned at the TTL");
    }

    #[test]
    fn stack_keeps_newest_three() {
        let t0 = Instant::now();
        let mut stack = ToastStack::default();
        for (i, text) in ["a", "b", "c", "d"].into_iter().enumerate() {
            stack.push(text, t0 + Duration::from_millis(i as u64 * 10));
        }
        let texts: Vec<&str> = stack
            .visible(t0 + Duration::from_millis(40))
            .map(|(toast, _)| toast.text.as_str())
            .collect();
        assert_eq!(TOAST_MAX, 3);
        assert_eq!(texts, ["b", "c", "d"], "oldest dropped, newest last");
    }
}

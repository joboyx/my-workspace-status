//! Toast stack: one-shot acks (`copied`, `Fetched 3 repos`), PR change
//! alerts (`✕ #318 CI failed · test (ubuntu-latest)`), and background job
//! outages (`✕ glab (gitlab.com): invalid_token · Esc dismiss`) shown in
//! the bottom-right corner of the panes.
//!
//! [`super::state::AppState::route_ack_to_toast`] moves a
//! [`super::status::StatusKind::Ok`] status here at the top of each draw.
//! [`super::state::AppState::alert_pr_transitions`] pushes the PR change
//! toasts. Acks and PR changes fade out after their time. A background job
//! that starts to fail pushes one sticky outage toast ([`Toast::outage`],
//! `AppState::note_outage`): it never fades, and it stays until Esc
//! dismisses it or the job works again. Foreground errors, warnings,
//! progress, and info notes stay in the status slot.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use super::outage::{outage_glyph, OutageSource};
use super::tree::SegRole;
use super::watch::FLASH_MS;

/// How long an acknowledgement toast shows, fade included (ms).
pub const TOAST_TTL_MS: u64 = 2500;

/// How long a PR change toast shows, fade included (ms).
pub const PR_TOAST_TTL_MS: u64 = 8000;

/// The last part of a toast's life in which it fades out (ms). Strength
/// drops from 1.0 to 0.0 over this window, whatever the toast's TTL.
pub const TOAST_FADE_MS: u64 = FLASH_MS;

/// Most toasts on screen at once. A push past it drops the oldest toast
/// that is not an outage toast (the oldest outage toast only when every
/// held toast is one).
pub const TOAST_MAX: usize = 3;

/// The muted detail of an outage toast: the key that dismisses it.
pub const OUTAGE_DISMISS_HINT: &str = "Esc dismiss";

/// The mark a toast paints before its text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastMark {
    /// The acknowledgement check ` ✓ ` (ASCII ` ok `), in the text colour.
    Ack,
    /// ` <glyph> `, bold in its own colour role: the state mark of a PR
    /// change.
    Glyph(&'static str, SegRole),
    /// No mark: the text starts after one space.
    None,
}

/// One toast and the instant it was pushed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    /// Mark painted before the text.
    pub mark: ToastMark,
    /// Text shown in the toast (`copied`, `#318 CI failed`).
    pub text: String,
    /// Muted tail painted after the text as ` · <detail>` (a failed check
    /// or a branch); empty for none.
    pub detail: String,
    /// Colour role of the text: [`SegRole::Repo`], or [`SegRole::Muted`]
    /// for a summary.
    pub role: SegRole,
    /// How long the toast shows, fade included (ms). An outage toast has
    /// no time: it ignores this.
    pub ttl_ms: u64,
    /// When the toast was pushed. Its TTL counts from here.
    pub at: Instant,
    /// The outage this toast reports, for an outage toast: it never fades
    /// or expires, and it goes only through
    /// [`ToastStack::dismiss_outages`] or [`ToastStack::drop_outage`].
    pub outage: Option<OutageSource>,
}

impl Toast {
    /// An acknowledgement pushed at `at`: the ack mark and `text` in the
    /// repo colour, for [`TOAST_TTL_MS`].
    pub fn ack(text: impl Into<String>, at: Instant) -> Self {
        Self {
            mark: ToastMark::Ack,
            text: text.into(),
            detail: String::new(),
            role: SegRole::Repo,
            ttl_ms: TOAST_TTL_MS,
            at,
            outage: None,
        }
    }

    /// The sticky toast of `source`'s outage, pushed at `at`: the failure
    /// mark (`✕`, ASCII `x`) and `text` in the deleted colour, then the
    /// muted [`OUTAGE_DISMISS_HINT`].
    pub fn outage(source: OutageSource, text: impl Into<String>, ascii: bool, at: Instant) -> Self {
        Self {
            mark: ToastMark::Glyph(outage_glyph(ascii), SegRole::Deleted),
            text: text.into(),
            detail: OUTAGE_DISMISS_HINT.to_string(),
            role: SegRole::Deleted,
            ttl_ms: 0,
            at,
            outage: Some(source),
        }
    }

    /// True for an outage toast ([`Self::outage`]).
    pub fn is_outage(&self) -> bool {
        self.outage.is_some()
    }

    /// Fade strength at `now`: 1.0 until the last [`TOAST_FADE_MS`] of its
    /// TTL, then linear down to 0.0 at the TTL. An outage toast is always
    /// 1.0.
    pub fn strength(&self, now: Instant) -> f32 {
        if self.is_outage() {
            return 1.0;
        }
        toast_strength(now.saturating_duration_since(self.at), self.ttl_ms)
    }
}

/// Live toasts, oldest first. At most [`TOAST_MAX`].
#[derive(Clone, Debug, Default)]
pub struct ToastStack {
    items: VecDeque<Toast>,
}

impl ToastStack {
    /// Add an acknowledgement pushed at `now` ([`Toast::ack`]). Drops the
    /// oldest past [`TOAST_MAX`].
    pub fn push(&mut self, text: impl Into<String>, now: Instant) {
        self.push_with(Toast::ack(text, now));
    }

    /// Add `toast`, pushed at its `at`. Past [`TOAST_MAX`] it drops the
    /// oldest toast that is not an outage toast, of either other kind; an
    /// outage toast goes only when every held toast is one.
    pub fn push_with(&mut self, toast: Toast) {
        self.items.push_back(toast);
        while self.items.len() > TOAST_MAX {
            match self.items.iter().position(|held| !held.is_outage()) {
                Some(index) => {
                    self.items.remove(index);
                }
                None => {
                    self.items.pop_front();
                }
            }
        }
    }

    /// Drop every toast whose own TTL has run out at `now`. Outage toasts
    /// stay.
    pub fn prune(&mut self, now: Instant) {
        self.items.retain(|toast| toast.strength(now) > 0.0);
    }

    /// True when no toast is held.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// True when an outage toast is held.
    pub fn has_outage(&self) -> bool {
        self.items.iter().any(Toast::is_outage)
    }

    /// Drop every outage toast (Esc). True when one was held.
    pub fn dismiss_outages(&mut self) -> bool {
        let held = self.items.len();
        self.items.retain(|toast| !toast.is_outage());
        self.items.len() != held
    }

    /// Drop the toast of `source`'s outage, if it is held.
    pub fn drop_outage(&mut self, source: &OutageSource) {
        self.items
            .retain(|toast| toast.outage.as_ref() != Some(source));
    }

    /// Show `text` in the held toast of `source`'s outage, in place: no
    /// new toast, and no toast comes back once dismissed.
    pub fn set_outage_text(&mut self, source: &OutageSource, text: &str) {
        for toast in &mut self.items {
            if toast.outage.as_ref() == Some(source) {
                toast.text = text.to_string();
            }
        }
    }

    /// Toasts still showing at `now`, oldest first, each with its fade
    /// strength (1.0 = full, falling toward 0.0 in the last
    /// [`TOAST_FADE_MS`] of its TTL).
    pub fn visible(&self, now: Instant) -> impl Iterator<Item = (&Toast, f32)> {
        self.items.iter().filter_map(move |toast| {
            let strength = toast.strength(now);
            (strength > 0.0).then_some((toast, strength))
        })
    }
}

/// The redraw the toast stack needs after a frame painted at `now`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastTick {
    /// No toast left: no timer redraw.
    Idle,
    /// A toast is fading: redraw on every animation frame.
    Frames,
    /// Every toast holds full strength: redraw once, this many ms from
    /// now, when the first fade starts (under `NO_COLOR`, when the first
    /// toast expires).
    WakeIn(u64),
}

impl ToastStack {
    /// What the loop must do after a frame painted at `now`, from each
    /// toast's own TTL. `no_color` toasts have no fade, so they only wake
    /// at their expiry. An outage toast never changes on its own: it asks
    /// for no redraw.
    pub fn next_tick(&self, now: Instant, no_color: bool) -> ToastTick {
        let mut wake: Option<u64> = None;
        for toast in self.items.iter().filter(|toast| !toast.is_outage()) {
            let age = now.saturating_duration_since(toast.at).as_millis() as u64;
            if age >= toast.ttl_ms {
                continue;
            }
            let until = if no_color {
                toast.ttl_ms - age
            } else {
                let fade_start = toast.ttl_ms.saturating_sub(TOAST_FADE_MS);
                if age >= fade_start {
                    return ToastTick::Frames;
                }
                fade_start - age
            };
            wake = Some(wake.map_or(until, |ms| ms.min(until)));
        }
        wake.map_or(ToastTick::Idle, ToastTick::WakeIn)
    }
}

/// Fade strength of a toast `age` old that lives `ttl_ms`: 1.0 until the
/// fade window, then linear down to 0.0 at `ttl_ms`.
fn toast_strength(age: Duration, ttl_ms: u64) -> f32 {
    let ms = age.as_millis() as u64;
    let fade_start = ttl_ms.saturating_sub(TOAST_FADE_MS);
    if ms >= ttl_ms {
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

    /// A PR change toast pushed at `at`, with the CI failed mark.
    fn pr_toast(text: &str, at: Instant) -> Toast {
        Toast {
            mark: ToastMark::Glyph("✕", SegRole::Deleted),
            text: text.into(),
            detail: "feature".into(),
            role: SegRole::Repo,
            ttl_ms: PR_TOAST_TTL_MS,
            at,
            outage: None,
        }
    }

    /// The outage toast of the `gitlab.com` PR lookups, pushed at `at`.
    fn outage_toast(host: &str, at: Instant) -> Toast {
        Toast::outage(
            OutageSource::PrLookup {
                tool: "glab",
                host: host.into(),
            },
            format!("glab ({host}): invalid_token"),
            false,
            at,
        )
    }

    fn texts(stack: &ToastStack, now: Instant) -> Vec<String> {
        stack
            .visible(now)
            .map(|(toast, _)| toast.text.clone())
            .collect()
    }

    #[test]
    fn an_outage_toast_never_fades_never_prunes_and_asks_for_no_wake() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let mut stack = ToastStack::default();
        stack.push_with(outage_toast("gitlab.com", t0));
        let toast = stack.visible(t0).next().unwrap().0.clone();
        assert_eq!(toast.mark, ToastMark::Glyph("✕", SegRole::Deleted));
        assert_eq!(toast.role, SegRole::Deleted);
        assert_eq!(toast.detail, OUTAGE_DISMISS_HINT);
        assert_eq!(
            Toast::outage(OutageSource::ScheduledFetch, "f", true, t0).mark,
            ToastMark::Glyph("x", SegRole::Deleted),
            "ASCII glyph mode"
        );
        let late = at(PR_TOAST_TTL_MS * 100);
        assert_eq!(
            stack.visible(late).map(|(_, s)| s).collect::<Vec<_>>(),
            [1.0],
            "full strength long after any TTL"
        );
        stack.prune(late);
        assert!(stack.has_outage(), "not pruned");
        for no_color in [false, true] {
            assert_eq!(stack.next_tick(t0, no_color), ToastTick::Idle);
            assert_eq!(stack.next_tick(late, no_color), ToastTick::Idle);
        }
        // Beside an ack, the stack wakes for the ack only.
        stack.push("copied", t0);
        assert_eq!(
            stack.next_tick(t0, false),
            ToastTick::WakeIn(TOAST_TTL_MS - TOAST_FADE_MS)
        );
        stack.prune(at(TOAST_TTL_MS));
        assert_eq!(
            texts(&stack, at(TOAST_TTL_MS)),
            ["glab (gitlab.com): invalid_token"]
        );
        assert_eq!(stack.next_tick(at(TOAST_TTL_MS), false), ToastTick::Idle);
    }

    #[test]
    fn eviction_keeps_outage_toasts_until_every_held_toast_is_one() {
        let t0 = Instant::now();
        let mut stack = ToastStack::default();
        stack.push_with(outage_toast("a.example", t0));
        stack.push("copied", t0);
        stack.push_with(pr_toast("#1 CI failed", t0));
        stack.push("staged README.md", t0);
        assert_eq!(
            texts(&stack, t0),
            [
                "glab (a.example): invalid_token",
                "#1 CI failed",
                "staged README.md"
            ],
            "the oldest non-outage toast goes"
        );
        stack.push_with(outage_toast("b.example", t0));
        stack.push_with(outage_toast("c.example", t0));
        assert_eq!(
            texts(&stack, t0),
            [
                "glab (a.example): invalid_token",
                "glab (b.example): invalid_token",
                "glab (c.example): invalid_token"
            ]
        );
        stack.push("copied", t0);
        assert_eq!(texts(&stack, t0).len(), 3);
        assert!(
            !texts(&stack, t0).contains(&"copied".to_string()),
            "a new ack is the only non-outage toast, so it goes"
        );
        stack.push_with(outage_toast("d.example", t0));
        assert_eq!(
            texts(&stack, t0),
            [
                "glab (b.example): invalid_token",
                "glab (c.example): invalid_token",
                "glab (d.example): invalid_token"
            ],
            "all outage toasts: the oldest goes"
        );
    }

    #[test]
    fn outage_toasts_dismiss_drop_and_update_in_place() {
        let t0 = Instant::now();
        let source = |host: &str| OutageSource::PrLookup {
            tool: "glab",
            host: host.into(),
        };
        let mut stack = ToastStack::default();
        assert!(!stack.dismiss_outages(), "nothing to dismiss");
        stack.push_with(outage_toast("a.example", t0));
        stack.push_with(outage_toast("b.example", t0));
        stack.push("copied", t0);
        stack.set_outage_text(&source("a.example"), "glab (a.example): HTTP 401");
        assert_eq!(
            texts(&stack, t0),
            [
                "glab (a.example): HTTP 401",
                "glab (b.example): invalid_token",
                "copied"
            ]
        );
        stack.drop_outage(&source("a.example"));
        assert_eq!(
            texts(&stack, t0),
            ["glab (b.example): invalid_token", "copied"]
        );
        assert!(stack.has_outage());
        assert!(stack.dismiss_outages());
        assert!(!stack.has_outage());
        assert_eq!(texts(&stack, t0), ["copied"], "other toasts stay");
        stack.set_outage_text(&source("b.example"), "glab (b.example): HTTP 401");
        assert_eq!(
            texts(&stack, t0),
            ["copied"],
            "a dismissed toast stays gone"
        );
    }

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
    fn hold_wakes_once_at_fade_start_fade_ticks_frames_expiry_is_idle() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let fade_start = TOAST_TTL_MS - TOAST_FADE_MS;
        let mut stack = ToastStack::default();
        assert_eq!(stack.next_tick(t0, false), ToastTick::Idle);
        stack.push("copied", t0);
        assert_eq!(stack.next_tick(at(0), false), ToastTick::WakeIn(fade_start));
        assert_eq!(
            stack.next_tick(at(500), false),
            ToastTick::WakeIn(fade_start - 500),
            "one wake at fade start, not a frame tick"
        );
        assert_eq!(stack.next_tick(at(fade_start), false), ToastTick::Frames);
        assert_eq!(
            stack.next_tick(at(TOAST_TTL_MS - 1), false),
            ToastTick::Frames
        );
        assert_eq!(stack.next_tick(at(TOAST_TTL_MS), false), ToastTick::Idle);

        // A newer holding toast does not hide an older one's fade.
        stack.push("Fetched 1 repo", at(fade_start));
        assert_eq!(
            stack.next_tick(at(fade_start + 10), false),
            ToastTick::Frames
        );
        // Once the older one is gone, the newer one wakes at its own fade.
        assert_eq!(
            stack.next_tick(at(TOAST_TTL_MS), false),
            ToastTick::WakeIn(fade_start - (TOAST_TTL_MS - fade_start))
        );

        // NO_COLOR: no fade, one wake at expiry.
        let mut plain = ToastStack::default();
        plain.push("copied", t0);
        assert_eq!(
            plain.next_tick(at(500), true),
            ToastTick::WakeIn(TOAST_TTL_MS - 500)
        );
        assert_eq!(
            plain.next_tick(at(TOAST_TTL_MS - 100), true),
            ToastTick::WakeIn(100)
        );
        assert_eq!(plain.next_tick(at(TOAST_TTL_MS), true), ToastTick::Idle);
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

    #[test]
    fn a_pr_toast_lives_8_s_with_the_ack_fade_beside_a_2_5_s_ack() {
        let t0 = Instant::now();
        let at = |ms: u64| t0 + Duration::from_millis(ms);
        let pr_fade = PR_TOAST_TTL_MS - TOAST_FADE_MS;
        let ack_fade = TOAST_TTL_MS - TOAST_FADE_MS;
        assert_eq!((PR_TOAST_TTL_MS, TOAST_FADE_MS), (8000, 800));
        let mut stack = ToastStack::default();
        stack.push("copied", t0);
        stack.push_with(pr_toast("#318 CI failed", t0));
        let strengths = |ms: u64| -> Vec<(String, f32)> {
            stack
                .visible(at(ms))
                .map(|(toast, s)| (toast.text.clone(), s))
                .collect()
        };
        assert_eq!(
            strengths(ack_fade),
            [("copied".into(), 1.0), ("#318 CI failed".into(), 1.0)]
        );
        assert_eq!(
            stack.next_tick(at(0), false),
            ToastTick::WakeIn(ack_fade),
            "the ack fades first"
        );
        assert_eq!(
            strengths(TOAST_TTL_MS),
            [("#318 CI failed".into(), 1.0)],
            "the ack is gone at 2.5 s, the PR toast holds"
        );
        assert_eq!(
            stack.next_tick(at(TOAST_TTL_MS), false),
            ToastTick::WakeIn(pr_fade - TOAST_TTL_MS),
            "then one wake at the PR toast's own fade start"
        );
        assert_eq!(strengths(pr_fade)[0].1, 1.0, "holds until 7.2 s");
        let mid = strengths(PR_TOAST_TTL_MS - TOAST_FADE_MS / 2)[0].1;
        assert!(mid > 0.4 && mid < 0.6, "half way through its fade: {mid}");
        assert_eq!(stack.next_tick(at(pr_fade), false), ToastTick::Frames);
        assert!(strengths(PR_TOAST_TTL_MS).is_empty(), "gone at 8 s");
        assert_eq!(stack.next_tick(at(PR_TOAST_TTL_MS), false), ToastTick::Idle);

        // NO_COLOR: one wake at its own expiry.
        assert_eq!(
            stack.next_tick(at(TOAST_TTL_MS), true),
            ToastTick::WakeIn(PR_TOAST_TTL_MS - TOAST_TTL_MS)
        );

        stack.prune(at(TOAST_TTL_MS));
        assert_eq!(stack.items.len(), 1, "the ack is pruned on its own TTL");
        stack.prune(at(PR_TOAST_TTL_MS - 1));
        assert!(!stack.is_empty(), "the PR toast still fades");
        stack.prune(at(PR_TOAST_TTL_MS));
        assert!(stack.is_empty(), "pruned at 8 s");
    }

    #[test]
    fn a_push_past_three_drops_the_oldest_of_either_kind() {
        let t0 = Instant::now();
        let mut stack = ToastStack::default();
        stack.push_with(pr_toast("#1 CI failed", t0));
        stack.push("copied", t0);
        stack.push_with(pr_toast("#2 CI failed", t0));
        stack.push_with(pr_toast("#3 CI failed", t0));
        let texts: Vec<&str> = stack
            .visible(t0)
            .map(|(toast, _)| toast.text.as_str())
            .collect();
        assert_eq!(texts, ["copied", "#2 CI failed", "#3 CI failed"]);
        stack.push("staged README.md", t0);
        let texts: Vec<&str> = stack
            .visible(t0)
            .map(|(toast, _)| toast.text.as_str())
            .collect();
        assert_eq!(texts, ["#2 CI failed", "#3 CI failed", "staged README.md"]);
    }
}

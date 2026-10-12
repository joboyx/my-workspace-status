//! Animation clock: spinner frames and the presenter tick rule.
//!
//! A painter that shows a time-based frame (spinner, toast fade, scroll
//! glide) sets [`super::state::LayoutHit::animating`] or
//! [`super::state::LayoutHit::gliding`] during the draw. The event loop then
//! asks [`next_tick_ms`] for the next redraw. With nothing animating there
//! is no timer redraw.

use std::collections::HashSet;
use std::time::Instant;

use super::watch::FLASH_TICK_MS;

/// Braille spinner frames, one per [`SPINNER_FRAME_MS`].
pub const SPINNER_FRAMES: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// ASCII spinner frames (`WS_STATUS_GLYPHS=ascii`).
pub const SPINNER_FRAMES_ASCII: [&str; 4] = ["|", "/", "-", "\\"];

/// How long each spinner frame shows.
pub const SPINNER_FRAME_MS: u64 = 80;

/// Scroll-glide frame period. Equals the presenter draw floor
/// (`DRAW_MIN_MS` in `tui/event_loop.rs`), so a glide draws every frame
/// the loop allows.
pub const GLIDE_FRAME_MS: u64 = 16;

/// Length of one Ctrl-d / Ctrl-u viewport glide: 3 frames at
/// [`GLIDE_FRAME_MS`].
pub const GLIDE_MS: u64 = 48;

/// Ease-out cubic: `1 - (1 - t)^3` for `t` clamped to `0.0..=1.0`. Starts
/// fast, settles slowly; `0` at the start, `1` at the end.
pub fn ease_out_cubic(t: f32) -> f32 {
    let rest = 1.0 - t.clamp(0.0, 1.0);
    1.0 - rest * rest * rest
}

/// The pane whose viewport top a [`ScrollGlide`] moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlidePane {
    /// Workspace tree ([`super::state::LayoutHit::list_offset`]).
    Tree,
    /// Commit graph list (`AppState::graph_scroll`).
    Graph,
    /// File diff (`AppState::diff_scroll`).
    Diff,
    /// Explorer file view (the file tab's `scroll`).
    FileView,
}

/// A live viewport glide after Ctrl-d / Ctrl-u. Visual only: the cursor
/// already sits on its new row; paint moves the viewport top from `from`
/// to the new top over [`GLIDE_MS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScrollGlide {
    /// Pane the glide moves.
    pub pane: GlidePane,
    /// Viewport top painted when the chord came in.
    pub from: usize,
    /// When the chord came in.
    pub started: Instant,
    /// Viewport top the last glide frame painted, so a chord mid-glide
    /// starts from what is on screen.
    pub last: usize,
}

/// One glide frame for `pane` at `now`, toward the jump top `target`.
///
/// Returns the viewport top to paint and whether this frame is a glide
/// step (the loop then ticks at [`GLIDE_FRAME_MS`]). With no glide on
/// `pane` in `glide` the top is `target`. While the glide runs the top is
/// `from + (target - from) * ease_out_cubic(t)`, clamped so `cursor` stays
/// inside `[top, top + height)`. At `t >= 1`, when the top does not move,
/// or once that clamp puts the top on `target`, `glide` is cleared and
/// `target` comes back as a plain (not glide) frame.
pub fn glide_step(
    glide: &mut Option<ScrollGlide>,
    pane: GlidePane,
    target: usize,
    cursor: usize,
    height: usize,
    now: Instant,
) -> (usize, bool) {
    let Some(live) = glide.filter(|live| live.pane == pane) else {
        return (target, false);
    };
    let elapsed = glide_elapsed_ms(&live, now);
    if elapsed >= GLIDE_MS || live.from == target {
        *glide = None;
        return (target, false);
    }
    let t = elapsed as f32 / GLIDE_MS as f32;
    let span = target as f32 - live.from as f32;
    let eased = (live.from as f32 + span * ease_out_cubic(t))
        .round()
        .max(0.0) as usize;
    let top = eased
        .max((cursor + 1).saturating_sub(height.max(1)))
        .min(cursor);
    if top == target {
        // The cursor clamp already lands on the jump top: no step is left
        // to paint, so the loop must not tick for it.
        *glide = None;
        return (target, false);
    }
    *glide = Some(ScrollGlide { last: top, ..live });
    (top, true)
}

/// Milliseconds since `glide` started (0 for a clock before the start).
pub fn glide_elapsed_ms(glide: &ScrollGlide, now: Instant) -> u64 {
    now.saturating_duration_since(glide.started).as_millis() as u64
}

/// Spinner frame for `now`, counted from `epoch`.
///
/// Frame index is `elapsed_ms / SPINNER_FRAME_MS % len`. `ascii` picks
/// [`SPINNER_FRAMES_ASCII`], else [`SPINNER_FRAMES`].
pub fn spinner_glyph(epoch: Instant, now: Instant, ascii: bool) -> &'static str {
    let frames: &[&'static str] = if ascii {
        &SPINNER_FRAMES_ASCII
    } else {
        &SPINNER_FRAMES
    };
    let elapsed = now.saturating_duration_since(epoch).as_millis() as u64;
    frames[(elapsed / SPINNER_FRAME_MS) as usize % frames.len()]
}

/// What the last frame painted that needs a timer redraw.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnimNeeds {
    /// A row flash or ghost is still decaying.
    pub flashes: bool,
    /// A spinner or fading toast frame was painted.
    pub frames: bool,
    /// A scroll glide is in progress.
    pub glide: bool,
    /// One redraw wanted this many ms after the last draw, with no frame
    /// ticks before it (a toast holding full strength until its fade).
    pub wake_ms: Option<u64>,
}

/// Milliseconds until the next timer redraw for `needs`: the fastest active
/// need ([`GLIDE_FRAME_MS`], [`SPINNER_FRAME_MS`], `FLASH_TICK_MS`) or the
/// one-shot `wake_ms`, whichever is sooner. `None` when nothing animates
/// and no wake is due, so the loop sleeps until input or a job.
pub fn next_tick_ms(needs: AnimNeeds) -> Option<u64> {
    [
        (needs.glide, GLIDE_FRAME_MS),
        (needs.frames, SPINNER_FRAME_MS),
        (needs.flashes, FLASH_TICK_MS),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, ms)| ms)
    .chain(needs.wake_ms)
    .min()
}

/// Remote git work the panes show while it runs.
///
/// The interpreter republishes it after each spawn and each applied
/// result; paint only reads it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RemoteActivity {
    /// Checkouts with a fetch, pull, or push in flight.
    pub running: HashSet<String>,
    /// Progress of the key-started bulk remote work over two or more
    /// repos. `None` for a single repo and for the background fetch tick.
    pub batch: Option<BatchProgress>,
}

/// Progress of a bulk remote batch, summed over its live kinds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BatchProgress {
    /// Repos finished (ok, skipped, or failed).
    pub done: usize,
    /// Repos in the batch: done, in flight, and queued.
    pub total: usize,
    /// A repo in the batch failed.
    pub failed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn spinner_cycles_braille_frames_every_80ms() {
        let epoch = Instant::now();
        let at = |ms: u64| spinner_glyph(epoch, epoch + Duration::from_millis(ms), false);
        assert_eq!(at(0), "⠋");
        assert_eq!(at(79), "⠋");
        assert_eq!(at(80), "⠙");
        assert_eq!(at(160), "⠹");
        assert_eq!(at(9 * 80), "⠏");
        assert_eq!(at(10 * 80), "⠋", "wraps after the last frame");
        let seen: Vec<&str> = (0..10).map(|i| at(i * SPINNER_FRAME_MS)).collect();
        assert_eq!(seen, SPINNER_FRAMES);
        assert_eq!(
            spinner_glyph(epoch + Duration::from_secs(1), epoch, false),
            "⠋",
            "a clock before the epoch shows the first frame"
        );
    }

    #[test]
    fn spinner_ascii_fallback_uses_bar_slash_dash_backslash() {
        let epoch = Instant::now();
        let seen: Vec<&str> = (0..5)
            .map(|i| spinner_glyph(epoch, epoch + Duration::from_millis(i * 80), true))
            .collect();
        assert_eq!(seen, ["|", "/", "-", "\\", "|"]);
        assert!(seen.iter().all(|glyph| glyph.is_ascii()));
    }

    #[test]
    fn ease_out_cubic_starts_at_zero_ends_at_one_and_is_monotonic() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_out_cubic(-1.0), 0.0, "clamped below");
        assert_eq!(ease_out_cubic(2.0), 1.0, "clamped above");
        assert!(ease_out_cubic(0.5) > 0.5, "ease-out runs ahead of linear");
        let samples: Vec<f32> = (0..=100)
            .map(|i| ease_out_cubic(i as f32 / 100.0))
            .collect();
        assert!(
            samples.windows(2).all(|pair| pair[0] < pair[1]),
            "strictly increasing"
        );
        assert_eq!(GLIDE_MS, 3 * GLIDE_FRAME_MS, "three frames at the floor");
    }

    /// A glide whose cursor clamp already puts the top on the jump top
    /// paints that top as a plain frame and ends: no tick runs for a frame
    /// that would not change.
    #[test]
    fn glide_step_pinned_on_target_ends_without_a_tick() {
        let t0 = Instant::now();
        let mut glide = Some(ScrollGlide {
            pane: GlidePane::FileView,
            from: 0,
            started: t0,
            last: 0,
        });
        // Cursor 29 in a 10-line view keeps the top at 20 or more: the
        // jump top 20 is the only top that shows it.
        let step = glide_step(&mut glide, GlidePane::FileView, 20, 29, 10, t0);
        assert_eq!(step, (20, false));
        assert_eq!(glide, None, "the glide ends");

        // With room to move, the same glide paints a step and ticks.
        let mut glide = Some(ScrollGlide {
            pane: GlidePane::FileView,
            from: 0,
            started: t0,
            last: 0,
        });
        let (top, gliding) = glide_step(&mut glide, GlidePane::FileView, 20, 25, 10, t0);
        assert!(top < 20 && gliding, "top {top} glides");
    }

    #[test]
    fn next_tick_is_none_when_nothing_animates() {
        assert_eq!(next_tick_ms(AnimNeeds::default()), None);
    }

    #[test]
    fn next_tick_takes_the_fastest_active_need() {
        let needs = |flashes, frames, glide| AnimNeeds {
            flashes,
            frames,
            glide,
            wake_ms: None,
        };
        assert_eq!(next_tick_ms(needs(true, false, false)), Some(FLASH_TICK_MS));
        assert_eq!(
            next_tick_ms(needs(false, true, false)),
            Some(SPINNER_FRAME_MS)
        );
        assert_eq!(
            next_tick_ms(needs(true, true, false)),
            Some(SPINNER_FRAME_MS)
        );
        assert_eq!(next_tick_ms(needs(true, true, true)), Some(GLIDE_FRAME_MS));
        assert_eq!(next_tick_ms(needs(false, false, true)), Some(16));
    }

    #[test]
    fn a_wake_alone_is_one_deadline_and_frames_beat_a_later_wake() {
        let wake = |wake_ms, frames| AnimNeeds {
            frames,
            wake_ms,
            ..AnimNeeds::default()
        };
        assert_eq!(next_tick_ms(wake(Some(1700), false)), Some(1700));
        assert_eq!(next_tick_ms(wake(Some(5), true)), Some(5));
        assert_eq!(next_tick_ms(wake(Some(1700), true)), Some(SPINNER_FRAME_MS));
        assert_eq!(next_tick_ms(wake(None, false)), None);
    }
}

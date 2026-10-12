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
    /// A spinner or toast frame was painted.
    pub frames: bool,
    /// A scroll glide is in progress.
    pub glide: bool,
}

/// Milliseconds between timer redraws for `needs`: the fastest active
/// need ([`GLIDE_FRAME_MS`], [`SPINNER_FRAME_MS`], `FLASH_TICK_MS`).
/// `None` when nothing animates, so the loop sleeps until input or a job.
pub fn next_tick_ms(needs: AnimNeeds) -> Option<u64> {
    [
        (needs.glide, GLIDE_FRAME_MS),
        (needs.frames, SPINNER_FRAME_MS),
        (needs.flashes, FLASH_TICK_MS),
    ]
    .into_iter()
    .filter(|(on, _)| *on)
    .map(|(_, ms)| ms)
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
    fn next_tick_is_none_when_nothing_animates() {
        assert_eq!(next_tick_ms(AnimNeeds::default()), None);
    }

    #[test]
    fn next_tick_takes_the_fastest_active_need() {
        let needs = |flashes, frames, glide| AnimNeeds {
            flashes,
            frames,
            glide,
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
}

//! Light or dark theme from the terminal background (`themeMode`).
//!
//! With `themeMode` `auto`, `run_tui` asks the terminal for its background
//! colour once at startup, after raw mode and before the alternate screen
//! and the input thread. [`query_background`] writes an OSC 11 query
//! (`ESC ] 11 ; ? BEL`) and then a primary device attributes request (DA1,
//! `ESC [ c`). Terminals answer in order, so a DA1 reply with no OSC 11
//! reply before it means the terminal does not report its background: the
//! wait stops early. The wait never passes [`QUERY_TIMEOUT`]. Stdin bytes
//! that are not part of either reply go back to the live reader
//! ([`super::tty::push_pending_input`]), so a key typed during the query is
//! not lost.
//!
//! [`pick_theme`] then swaps the resolved theme for its light or dark twin.
//! A terminal that does not answer keeps the resolved theme.

use std::ops::Range;
use std::time::Duration;

use ratatui::style::Color;

use super::syntax::relative_luminance;
use super::theme::ThemeId;
use crate::config::ThemeMode;

/// Longest startup wait for the terminal's OSC 11 / DA1 reply.
pub(crate) const QUERY_TIMEOUT: Duration = Duration::from_millis(150);

/// OSC 11 background query (BEL-terminated) followed by DA1.
#[cfg(unix)]
const QUERY: &[u8] = b"\x1b]11;?\x07\x1b[c";

/// Start of an OSC 11 reply.
const OSC11_INTRO: &[u8] = b"\x1b]11;";

/// Start of a DA1 reply (`ESC [ ? Ps ; ... c`).
const DA1_INTRO: &[u8] = b"\x1b[?";

/// Luminance at or above which a background counts as light.
///
/// 0.18 is (about) where black text and white text have the same WCAG
/// contrast ratio on the background: `(L + 0.05) / 0.05 == 1.05 / (L + 0.05)`
/// gives `L = sqrt(0.0525) - 0.05 ≈ 0.179`. Above it dark text reads
/// better, so a light theme fits; below it light text does.
const LIGHT_LUMINANCE: f64 = 0.18;

/// Parse the first OSC 11 reply in `bytes`: `ESC ] 11 ; rgb:R/G/B` ended by
/// BEL or `ESC \`.
///
/// Each channel has 1 to 4 hex digits and scales to 8 bits
/// (`value * 255 / (16^n - 1)`). `None` when there is no complete reply or
/// its body is not that form.
pub(crate) fn parse_osc11_reply(bytes: &[u8]) -> Option<(u8, u8, u8)> {
    let (_, body) = find_osc11(bytes)?;
    parse_rgb_body(&bytes[body])
}

/// True when `rgb` is a light background (WCAG relative luminance at or
/// above [`LIGHT_LUMINANCE`]).
pub(crate) fn is_light_background(rgb: (u8, u8, u8)) -> bool {
    let (r, g, b) = rgb;
    relative_luminance(Color::Rgb(r, g, b)).is_some_and(|l| l >= LIGHT_LUMINANCE)
}

/// The launch theme for `mode`.
///
/// [`ThemeMode::Dark`] and [`ThemeMode::Light`] pin the dark or light twin
/// of `theme`. [`ThemeMode::Auto`] picks the twin that matches `light_bg`
/// (the terminal's answer), and keeps `theme` when the terminal did not
/// answer (`None`).
pub(crate) fn pick_theme(theme: ThemeId, mode: ThemeMode, light_bg: Option<bool>) -> ThemeId {
    match (mode, light_bg) {
        (ThemeMode::Dark, _) | (ThemeMode::Auto, Some(false)) => theme.dark_variant(),
        (ThemeMode::Light, _) | (ThemeMode::Auto, Some(true)) => theme.light_variant(),
        (ThemeMode::Auto, None) => theme,
    }
}

/// Ask the terminal for its background colour. See the module docs.
///
/// Returns the colour from the OSC 11 reply, or `None` when stdin or stdout
/// is not a terminal, the terminal answers DA1 only, or `timeout` passes.
#[cfg(unix)]
pub(crate) fn query_background(timeout: Duration) -> Option<(u8, u8, u8)> {
    use std::io::{self, IsTerminal, Write};
    use std::time::Instant;

    use super::tty::{poll_stdin, push_pending_input};

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return None;
    }
    let mut out = io::stdout();
    if out.write_all(QUERY).and_then(|()| out.flush()).is_err() {
        return None;
    }
    let deadline = Instant::now() + timeout;
    let mut got = Vec::new();
    let mut chunk = [0u8; 256];
    loop {
        if split_replies(&got).done {
            break;
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        match poll_stdin(left) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => break,
        }
        // SAFETY: `chunk` is a live buffer of `chunk.len()` writable bytes.
        let n = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                chunk.as_mut_ptr().cast::<libc::c_void>(),
                chunk.len(),
            )
        };
        let Ok(n) = usize::try_from(n) else {
            break;
        };
        if n == 0 {
            break;
        }
        got.extend_from_slice(&chunk[..n]);
    }
    let replies = split_replies(&got);
    push_pending_input(&replies.rest);
    replies.rgb
}

/// Non-Unix builds never query: the theme stays as resolved.
#[cfg(not(unix))]
pub(crate) fn query_background(_timeout: Duration) -> Option<(u8, u8, u8)> {
    None
}

/// What [`split_replies`] found in the stdin bytes read so far.
#[derive(Debug, PartialEq, Eq)]
struct Replies {
    /// Colour of a complete, well-formed OSC 11 reply.
    rgb: Option<(u8, u8, u8)>,
    /// Stop waiting: the OSC 11 reply parsed, or the DA1 reply arrived.
    done: bool,
    /// Every byte outside the OSC 11 and DA1 replies (typed input).
    rest: Vec<u8>,
}

/// Split stdin bytes into the OSC 11 reply, the DA1 reply, and the rest.
fn split_replies(bytes: &[u8]) -> Replies {
    let rgb = parse_osc11_reply(bytes);
    let da1 = find_da1(bytes);
    let mut spans: Vec<Range<usize>> = find_osc11(bytes)
        .map(|(span, _)| span)
        .into_iter()
        .collect();
    spans.extend(da1.clone());
    let rest = bytes
        .iter()
        .enumerate()
        .filter(|(i, _)| !spans.iter().any(|span| span.contains(i)))
        .map(|(_, b)| *b)
        .collect();
    Replies {
        rgb,
        done: rgb.is_some() || da1.is_some(),
        rest,
    }
}

/// Whole span and body span (after `11;`, before the terminator) of the
/// first complete OSC 11 reply.
fn find_osc11(bytes: &[u8]) -> Option<(Range<usize>, Range<usize>)> {
    let start = find_bytes(bytes, OSC11_INTRO)?;
    let body_start = start + OSC11_INTRO.len();
    let (body_end, end) = osc_terminator(bytes, body_start)?;
    Some((start..end, body_start..body_end))
}

/// Body end and sequence end of an OSC string that starts at `from`: BEL
/// or `ESC \`. `None` when the string is cut off (no terminator, or an ESC
/// that starts some other sequence).
fn osc_terminator(bytes: &[u8], from: usize) -> Option<(usize, usize)> {
    let i = from
        + bytes
            .get(from..)?
            .iter()
            .position(|b| *b == 0x07 || *b == 0x1b)?;
    match bytes[i] {
        0x07 => Some((i, i + 1)),
        _ if bytes.get(i + 1) == Some(&b'\\') => Some((i, i + 2)),
        _ => None,
    }
}

/// Parse `rgb:R/G/B` with 1 to 4 hex digits per channel.
fn parse_rgb_body(body: &[u8]) -> Option<(u8, u8, u8)> {
    let text = std::str::from_utf8(body).ok()?.strip_prefix("rgb:")?;
    let mut channels = text.split('/').map(scale_channel);
    let rgb = (channels.next()??, channels.next()??, channels.next()??);
    channels.next().is_none().then_some(rgb)
}

/// Scale one hex channel of `n` digits (1 to 4) to 8 bits.
fn scale_channel(hex: &str) -> Option<u8> {
    let n = hex.len();
    if !(1..=4).contains(&n) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(hex, 16).ok()?;
    let max = 16u32.pow(n as u32) - 1;
    u8::try_from(value * 255 / max).ok()
}

/// Span of the first complete DA1 reply (`ESC [ ? <digits and ;> c`).
fn find_da1(bytes: &[u8]) -> Option<Range<usize>> {
    let mut from = 0;
    while let Some(at) = find_bytes(&bytes[from..], DA1_INTRO) {
        let start = from + at;
        let params = start + DA1_INTRO.len();
        let len = bytes[params..]
            .iter()
            .take_while(|b| b.is_ascii_digit() || **b == b';')
            .count();
        if bytes.get(params + len) == Some(&b'c') {
            return Some(start..params + len + 1);
        }
        from = start + 1;
    }
    None
}

/// Index of the first `needle` in `hay`.
fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_four_digit_rgb_with_bel() {
        assert_eq!(
            parse_osc11_reply(b"\x1b]11;rgb:ffff/8080/0000\x07"),
            Some((255, 128, 0))
        );
        assert_eq!(
            parse_osc11_reply(b"typed\x1b]11;rgb:1a1a/1b1b/2626\x07\x1b[?62;22c"),
            Some((26, 27, 38)),
            "a reply between typed bytes and DA1 still parses"
        );
    }

    #[test]
    fn parses_two_digit_rgb_with_st() {
        assert_eq!(
            parse_osc11_reply(b"\x1b]11;rgb:e1/e2/e7\x1b\\"),
            Some((0xe1, 0xe2, 0xe7))
        );
        assert_eq!(
            parse_osc11_reply(b"\x1b]11;rgb:f/8/0\x1b\\"),
            Some((255, 136, 0)),
            "one digit per channel scales by 255 / 15"
        );
    }

    #[test]
    fn rejects_garbage_and_truncated_reply() {
        for bytes in [
            &b""[..],
            b"\x1b]11;rgb:ffff/ffff/ffff",
            b"\x1b]11;rgb:ffff/ffff\x07",
            b"\x1b]11;rgb:ffff/ffff/ffff/ffff\x07",
            b"\x1b]11;rgb:fffff/0/0\x07",
            b"\x1b]11;rgb:gg/00/00\x07",
            b"\x1b]11;rgb:/00/00\x07",
            b"\x1b]11;rgba:ffff/ffff/ffff/ffff\x07",
            b"\x1b]10;rgb:ffff/ffff/ffff\x07",
            b"\x1b]11;rgb:ff\x1b[?62c",
            b"\x1b[?62;22c",
        ] {
            assert_eq!(parse_osc11_reply(bytes), None, "{bytes:?}");
        }
    }

    #[test]
    fn white_is_light_and_black_is_dark() {
        assert!(is_light_background((255, 255, 255)));
        assert!(!is_light_background((0, 0, 0)));
        // Theme surfaces land on their own side of the line.
        assert!(is_light_background((0xe1, 0xe2, 0xe7)), "tokyo-night-day");
        assert!(!is_light_background((0x1a, 0x1b, 0x26)), "tokyo-night");
        // Mid gray #777 (L ~ 0.184) is light; #737373 (L ~ 0.171) is dark.
        assert!(is_light_background((0x77, 0x77, 0x77)));
        assert!(!is_light_background((0x73, 0x73, 0x73)));
    }

    #[test]
    fn pick_theme_auto_uses_reply_and_keeps_theme_without_one() {
        use ThemeId::*;
        assert_eq!(
            pick_theme(TokyoNight, ThemeMode::Auto, Some(true)),
            TokyoNightDay
        );
        assert_eq!(
            pick_theme(TokyoNight, ThemeMode::Auto, Some(false)),
            TokyoNight
        );
        assert_eq!(
            pick_theme(GithubLight, ThemeMode::Auto, Some(false)),
            GithubDarkDimmed
        );
        assert_eq!(
            pick_theme(GithubLight, ThemeMode::Auto, Some(true)),
            GithubLight
        );
        assert_eq!(pick_theme(TokyoNight, ThemeMode::Auto, None), TokyoNight);
        assert_eq!(pick_theme(GithubLight, ThemeMode::Auto, None), GithubLight);
    }

    #[test]
    fn pick_theme_dark_and_light_pin_the_twin() {
        for id in crate::tui::theme::THEME_IDS {
            for light_bg in [None, Some(true), Some(false)] {
                assert_eq!(pick_theme(id, ThemeMode::Dark, light_bg), id.dark_variant());
                assert_eq!(
                    pick_theme(id, ThemeMode::Light, light_bg),
                    id.light_variant()
                );
            }
            assert!(!pick_theme(id, ThemeMode::Dark, Some(true)).is_light());
            assert!(pick_theme(id, ThemeMode::Light, Some(false)).is_light());
            // Auto must follow the reply, not act like Dark or Light.
            assert!(
                pick_theme(id, ThemeMode::Auto, Some(true)).is_light(),
                "{id:?}"
            );
            assert!(
                !pick_theme(id, ThemeMode::Auto, Some(false)).is_light(),
                "{id:?}"
            );
        }
    }

    #[test]
    fn split_replies_keeps_typed_bytes_and_stops_on_da1() {
        let got = split_replies(b"j\x1b]11;rgb:ffff/ffff/ffff\x07k\x1b[?62;22cq");
        assert_eq!(
            got,
            Replies {
                rgb: Some((255, 255, 255)),
                done: true,
                rest: b"jkq".to_vec(),
            }
        );
        let da1_only = split_replies(b"g\x1b[?1;2cg");
        assert_eq!(
            da1_only,
            Replies {
                rgb: None,
                done: true,
                rest: b"gg".to_vec(),
            },
            "a terminal without OSC 11 answers DA1 only: stop early"
        );
        let waiting = split_replies(b"j\x1b[A");
        assert!(!waiting.done, "no reply yet: keep waiting");
        assert_eq!(waiting.rest, b"j\x1b[A");
    }
}

//! Terminal capability queries for the Doctor setup checkup.
//!
//! The Doctor asks the terminal what it supports instead of guessing from
//! environment variables. [`QUERY`] is one write: XTVERSION (name and
//! version), DECRQM for SGR mouse (1006), bracketed paste (2004), and
//! synchronized output (2026), the kitty keyboard flags (`CSI ? u`), the
//! OSC 11 background colour, and then DA1 (`ESC [ c`). Terminals answer in
//! order, so the DA1 reply ends the wait early; the wait never passes
//! [`CAPS_QUERY_TIMEOUT`].
//!
//! The live TTY reader (`tui/tty.rs`) parses each reply into a
//! [`TermReply`] and queues it, so a reply never reads as keys, also when it
//! lands late. [`TermCaps`] collects the replies of one query, and
//! [`capability_checks`] turns it into the Doctor's capability rows. Both
//! are pure. [`query_capabilities`] is the blocking form for the `ws doctor`
//! CLI only: the TUI must not call it (it reads stdin with a wait).

use std::collections::BTreeMap;
use std::time::Duration;

use super::doctor::{hint, row, Check, CheckGroup, CheckStatus, Os};
use super::term_bg::{find_da1, is_light_background};

/// Longest wait for the replies to [`QUERY`].
pub(crate) const CAPS_QUERY_TIMEOUT: Duration = Duration::from_millis(200);

/// The capability query, in reply order: XTVERSION, DECRQM 1006 / 2004 /
/// 2026, kitty keyboard flags, OSC 11 (ST-terminated), then DA1 as the end
/// marker.
pub(crate) const QUERY: &[u8] =
    b"\x1b[>0q\x1b[?1006$p\x1b[?2004$p\x1b[?2026$p\x1b[?u\x1b]11;?\x1b\\\x1b[c";

/// One parsed terminal reply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum TermReply {
    /// XTVERSION: `ESC P > | <text> ESC \`, for example `xterm(390)`.
    XtVersion(String),
    /// DECRPM: `ESC [ ? <mode> ; <value> $ y`. `value` 0 not recognized,
    /// 1 set, 2 reset, 3 permanently set, 4 permanently reset.
    ModeReport {
        /// The private mode number.
        mode: u16,
        /// The mode state, 0 to 4.
        value: u8,
    },
    /// Kitty keyboard: `ESC [ ? <flags> u`.
    KittyFlags(u32),
    /// OSC 11: the background colour scaled to 8 bits per channel.
    Background((u8, u8, u8)),
    /// DA1: `ESC [ ? ... c`, the end marker of the query.
    DeviceAttributes,
}

/// Parse a complete `ESC [ ? <params> [$] <final>` reply: DECRPM, kitty
/// keyboard flags, or DA1. `None` for any other sequence, and for a DECRPM
/// value above 4.
pub(crate) fn parse_csi_reply(seq: &[u8]) -> Option<TermReply> {
    let body = seq.strip_prefix(b"\x1b[?")?;
    let (last, params) = body.split_last()?;
    let params = std::str::from_utf8(params).ok()?;
    let digits = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
    match last {
        b'c' if find_da1(seq) == Some(0..seq.len()) => Some(TermReply::DeviceAttributes),
        b'u' if digits(params) => params.parse().ok().map(TermReply::KittyFlags),
        b'y' => {
            let (mode, value) = params.strip_suffix('$')?.split_once(';')?;
            if !digits(mode) || !digits(value) {
                return None;
            }
            let value: u8 = value.parse().ok().filter(|v| *v <= 4)?;
            Some(TermReply::ModeReport {
                mode: mode.parse().ok()?,
                value,
            })
        }
        _ => None,
    }
}

/// How the wait for the replies ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum QueryEnd {
    /// Still waiting.
    #[default]
    Waiting,
    /// The DA1 reply arrived: every reply the terminal sends came before it.
    DeviceAttributes,
    /// [`CAPS_QUERY_TIMEOUT`] passed with no DA1 reply.
    Timeout,
    /// The query was not sent (stdin or stdout is not a terminal, or
    /// Windows).
    NotAsked,
}

/// The replies to one [`QUERY`], and how the wait ended.
///
/// [`TermCaps::default`] is a query that waits. [`TermCaps::apply`] records
/// a reply; the DA1 reply ends the wait. [`TermCaps::time_out`] ends it at
/// the deadline. Once it ended, later replies are ignored, so rows built
/// from it never change after they paint.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TermCaps {
    /// XTVERSION text.
    version: Option<String>,
    /// DECRPM value by mode.
    modes: BTreeMap<u16, u8>,
    /// Kitty keyboard flags.
    kitty_flags: Option<u32>,
    /// OSC 11 background colour.
    background: Option<(u8, u8, u8)>,
    /// How the wait ended.
    end: QueryEnd,
}

impl TermCaps {
    /// A query that was not sent: every capability row is unknown.
    pub(crate) fn not_asked() -> Self {
        TermCaps {
            end: QueryEnd::NotAsked,
            ..TermCaps::default()
        }
    }

    /// Record `reply`. The DA1 reply ends the wait. No-op once it ended.
    pub(crate) fn apply(&mut self, reply: TermReply) {
        if self.is_done() {
            return;
        }
        match reply {
            TermReply::XtVersion(text) => self.version = Some(text),
            TermReply::ModeReport { mode, value } => {
                self.modes.insert(mode, value);
            }
            TermReply::KittyFlags(flags) => self.kitty_flags = Some(flags),
            TermReply::Background(rgb) => self.background = Some(rgb),
            TermReply::DeviceAttributes => self.end = QueryEnd::DeviceAttributes,
        }
    }

    /// End the wait at the deadline. No-op once it ended.
    pub(crate) fn time_out(&mut self) {
        if !self.is_done() {
            self.end = QueryEnd::Timeout;
        }
    }

    /// True once the DA1 reply arrived, the deadline passed, or the query
    /// was not sent.
    pub(crate) fn is_done(&self) -> bool {
        self.end != QueryEnd::Waiting
    }
}

/// Row names in paint order.
const XTVERSION_ROW: &str = "Terminal (XTVERSION)";
const KITTY_ROW: &str = "Kitty keyboard (CSI u)";
const BACKGROUND_ROW: &str = "Background (OSC 11)";

/// One DECRQM row: the mode, its row name, and what its answers mean.
struct ModeRow {
    /// Private mode number.
    mode: u16,
    /// Row name.
    name: &'static str,
    /// Why text for DECRPM 1, 2, or 3.
    supported: &'static str,
    /// Status for DECRPM 0 or 4.
    unsupported_status: CheckStatus,
    /// Why text for DECRPM 0 or 4.
    unsupported: &'static str,
    /// Hint for DECRPM 0 or 4.
    unsupported_hint: Option<&'static str>,
}

/// The DECRQM rows, in paint order.
const MODE_ROWS: [ModeRow; 3] = [
    ModeRow {
        mode: 1006,
        name: "SGR mouse (1006)",
        supported: "Supported.",
        unsupported_status: CheckStatus::Warn,
        unsupported: "Not supported: mouse reports past column 223 can be lost.",
        unsupported_hint: Some("Use a terminal with SGR mouse reports (1006)."),
    },
    ModeRow {
        mode: 2004,
        name: "Bracketed paste (2004)",
        supported: "Supported.",
        unsupported_status: CheckStatus::Ok,
        unsupported: "Not supported. The app does not use bracketed paste.",
        unsupported_hint: None,
    },
    ModeRow {
        mode: 2026,
        name: "Synchronized output (2026)",
        supported: "Supported: frames paint without tearing.",
        unsupported_status: CheckStatus::Ok,
        unsupported: "Not supported (optional): a large redraw can flicker.",
        unsupported_hint: None,
    },
];

/// Hint every capability row gets inside tmux, unless it has another one.
const TMUX_HINT: &str = "Inside tmux, tmux answers for the outer terminal.";

/// The capability rows (group TERMINAL) for `caps`, in paint order:
/// XTVERSION, SGR mouse, bracketed paste, synchronized output, kitty
/// keyboard, background. Pure.
///
/// A row with no reply is unknown, never a failure; its why ends with
/// `(no answer within 200 ms)` when the deadline ended the wait and
/// `(no answer)` when the DA1 reply did. On Windows, or when the query was
/// not sent, every row is unknown. Inside tmux (`in_tmux`) each row of a
/// sent query that has no hint gets [`TMUX_HINT`].
pub(crate) fn capability_checks(caps: &TermCaps, os: Os, in_tmux: bool) -> Vec<Check> {
    let not_asked = match (os, caps.end) {
        (Os::Windows, _) => Some("Not asked on Windows."),
        (_, QueryEnd::NotAsked) => Some("Not asked: stdin or stdout is not a terminal."),
        _ => None,
    };
    if let Some(why) = not_asked {
        return row_names()
            .map(|name| row(CheckGroup::Terminal, name, CheckStatus::Unknown, why, None))
            .collect();
    }
    let suffix = match caps.end {
        QueryEnd::Timeout => format!("(no answer within {} ms)", CAPS_QUERY_TIMEOUT.as_millis()),
        QueryEnd::Waiting => "(no answer yet)".to_string(),
        QueryEnd::DeviceAttributes | QueryEnd::NotAsked => "(no answer)".to_string(),
    };
    let no_answer = |name, base: &str| {
        row(
            CheckGroup::Terminal,
            name,
            CheckStatus::Unknown,
            format!("{base} {suffix}"),
            None,
        )
    };
    let ok = |name, why: String| row(CheckGroup::Terminal, name, CheckStatus::Ok, why, None);

    let mut rows = Vec::with_capacity(MODE_ROWS.len() + 3);
    rows.push(match &caps.version {
        Some(text) => ok(XTVERSION_ROW, text.clone()),
        None => no_answer(
            XTVERSION_ROW,
            "The terminal did not report its name and version.",
        ),
    });
    for mode in &MODE_ROWS {
        rows.push(match caps.modes.get(&mode.mode) {
            Some(1..=3) => ok(mode.name, mode.supported.to_string()),
            Some(_) => row(
                CheckGroup::Terminal,
                mode.name,
                mode.unsupported_status,
                mode.unsupported,
                mode.unsupported_hint.and_then(hint),
            ),
            None => no_answer(
                mode.name,
                &format!("The terminal did not report mode {}.", mode.mode),
            ),
        });
    }
    rows.push(match (caps.kitty_flags, caps.end) {
        (Some(flags), _) => ok(KITTY_ROW, format!("Supported (flags {flags}).")),
        (None, QueryEnd::DeviceAttributes) => ok(
            KITTY_ROW,
            "Not supported (optional): keys use the legacy encoding.".to_string(),
        ),
        (None, _) => no_answer(
            KITTY_ROW,
            "The terminal did not report its kitty keyboard flags.",
        ),
    });
    rows.push(match caps.background {
        Some(rgb) => {
            let (r, g, b) = rgb;
            let tone = if is_light_background(rgb) {
                "light"
            } else {
                "dark"
            };
            ok(BACKGROUND_ROW, format!("#{r:02x}{g:02x}{b:02x}, {tone}."))
        }
        None => no_answer(
            BACKGROUND_ROW,
            "The terminal did not report its background: themeMode auto keeps the \
             configured theme.",
        ),
    });
    if in_tmux {
        for check in &mut rows {
            if check.fix.is_none() {
                check.fix = hint(TMUX_HINT);
            }
        }
    }
    rows
}

/// Every capability row name, in paint order.
fn row_names() -> impl Iterator<Item = &'static str> {
    [XTVERSION_ROW]
        .into_iter()
        .chain(MODE_ROWS.iter().map(|mode| mode.name))
        .chain([KITTY_ROW, BACKGROUND_ROW])
}

/// Send [`QUERY`] and wait for the replies. For the `ws doctor` CLI only;
/// the TUI sends the query from its draw path and reads the replies on its
/// input thread.
///
/// Returns [`TermCaps::not_asked`] when stdin or stdout is not a terminal.
/// Turns raw mode on for the wait when it is off, and turns it off again on
/// every return. Reads stdin with a poll wait until the DA1 reply or
/// `timeout`. Stdin bytes that are not replies are dropped: the CLI reads no
/// keys. A failed write, poll, or read ends the wait as a timeout. When the
/// wait ends with no DA1 reply, pending stdin is discarded before raw mode
/// goes off.
#[cfg(unix)]
#[cfg_attr(
    test,
    expect(
        dead_code,
        reason = "needs a real terminal: the `ws doctor` PTY test covers it"
    )
)]
pub(crate) fn query_capabilities(timeout: Duration) -> TermCaps {
    use std::io::{self, IsTerminal, Write};
    use std::time::Instant;

    use super::tty::{read_stdin_until, term_replies_in};

    /// Turns raw mode off on drop when it turned it on.
    struct RawGuard(bool);
    impl Drop for RawGuard {
        fn drop(&mut self) {
            if self.0 {
                let _ = crossterm::terminal::disable_raw_mode();
            }
        }
    }

    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return TermCaps::not_asked();
    }
    let was_raw = crossterm::terminal::is_raw_mode_enabled().unwrap_or(false);
    let _raw = RawGuard(!was_raw && crossterm::terminal::enable_raw_mode().is_ok());
    let mut out = io::stdout();
    if out.write_all(QUERY).and_then(|()| out.flush()).is_err() {
        let mut caps = TermCaps::default();
        caps.time_out();
        return caps;
    }
    // Parse from the start each time: a reply cut between two reads parses
    // once its tail arrives.
    let caps_of = |got: &[u8]| {
        let mut caps = TermCaps::default();
        for reply in term_replies_in(got) {
            caps.apply(reply);
        }
        caps
    };
    let got = read_stdin_until(Instant::now() + timeout, |got| caps_of(got).is_done());
    let mut caps = caps_of(&got);
    if !caps.is_done() {
        // No DA1 reply yet: the rest of the answer may still come. Drop
        // what the terminal already queued, so a reply that lands now does
        // not show up as typed text at the shell prompt once raw mode is
        // off again.
        // SAFETY: `tcflush` only acts on the open stdin descriptor.
        unsafe {
            libc::tcflush(libc::STDIN_FILENO, libc::TCIFLUSH);
        }
    }
    caps.time_out();
    caps
}

/// Non-Unix builds never send the query.
#[cfg(not(unix))]
#[cfg_attr(
    test,
    expect(dead_code, reason = "the `ws doctor` CLI is its first caller")
)]
pub(crate) fn query_capabilities(_timeout: Duration) -> TermCaps {
    TermCaps::not_asked()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::doctor::Fix;

    /// A query that the DA1 reply ended after `replies`.
    fn answered(replies: Vec<TermReply>) -> TermCaps {
        let mut caps = TermCaps::default();
        for reply in replies {
            caps.apply(reply);
        }
        caps.apply(TermReply::DeviceAttributes);
        caps
    }

    fn full_answer() -> TermCaps {
        answered(vec![
            TermReply::XtVersion("xterm(390)".to_string()),
            TermReply::ModeReport {
                mode: 1006,
                value: 1,
            },
            TermReply::ModeReport {
                mode: 2004,
                value: 2,
            },
            TermReply::ModeReport {
                mode: 2026,
                value: 3,
            },
            TermReply::KittyFlags(15),
            TermReply::Background((255, 255, 255)),
        ])
    }

    fn whys(rows: &[Check]) -> Vec<(&str, CheckStatus, &str)> {
        rows.iter()
            .map(|check| (check.name, check.status, check.why.as_str()))
            .collect()
    }

    fn hint_of(check: &Check) -> Option<&str> {
        check.fix.as_ref().and_then(|fix| fix.hint.as_deref())
    }

    #[test]
    fn query_asks_every_capability_then_da1() {
        assert_eq!(
            QUERY,
            b"\x1b[>0q\x1b[?1006$p\x1b[?2004$p\x1b[?2026$p\x1b[?u\x1b]11;?\x1b\\\x1b[c"
        );
        assert_eq!(CAPS_QUERY_TIMEOUT, Duration::from_millis(200));
    }

    #[test]
    fn parses_decrpm_values_zero_to_four() {
        for value in 0..=4u8 {
            let seq = format!("\x1b[?2026;{value}$y");
            assert_eq!(
                parse_csi_reply(seq.as_bytes()),
                Some(TermReply::ModeReport { mode: 2026, value }),
                "{seq:?}"
            );
        }
        for bad in [
            &b"\x1b[?2026;5$y"[..],
            b"\x1b[?2026;1y",
            b"\x1b[?2026$y",
            b"\x1b[?;1$y",
            b"\x1b[?20a6;1$y",
        ] {
            assert_eq!(parse_csi_reply(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn parses_kitty_flags_and_da1() {
        assert_eq!(
            parse_csi_reply(b"\x1b[?15u"),
            Some(TermReply::KittyFlags(15))
        );
        assert_eq!(parse_csi_reply(b"\x1b[?0u"), Some(TermReply::KittyFlags(0)));
        assert_eq!(parse_csi_reply(b"\x1b[?u"), None, "no flags");
        assert_eq!(
            parse_csi_reply(b"\x1b[?62;22c"),
            Some(TermReply::DeviceAttributes)
        );
        assert_eq!(
            parse_csi_reply(b"\x1b[?1;2c"),
            Some(TermReply::DeviceAttributes)
        );
        assert_eq!(parse_csi_reply(b"\x1b[?1$c"), None);
        assert_eq!(parse_csi_reply(b"\x1b[62c"), None, "not a private reply");
    }

    #[test]
    fn da1_ends_the_wait_and_later_replies_are_ignored() {
        let mut caps = TermCaps::default();
        assert!(!caps.is_done());
        caps.apply(TermReply::KittyFlags(1));
        assert!(!caps.is_done());
        caps.apply(TermReply::DeviceAttributes);
        assert!(caps.is_done());
        assert_eq!(caps.end, QueryEnd::DeviceAttributes);
        caps.apply(TermReply::XtVersion("late".to_string()));
        caps.time_out();
        assert_eq!(caps.version, None, "a reply after the end is dropped");
        assert_eq!(caps.end, QueryEnd::DeviceAttributes);
        assert_eq!(caps.kitty_flags, Some(1));
    }

    #[test]
    fn timeout_ends_the_wait_and_later_replies_are_ignored() {
        let mut caps = TermCaps::default();
        caps.apply(TermReply::Background((0, 0, 0)));
        caps.time_out();
        assert!(caps.is_done());
        assert_eq!(caps.end, QueryEnd::Timeout);
        caps.apply(TermReply::DeviceAttributes);
        caps.apply(TermReply::KittyFlags(3));
        assert_eq!(caps.end, QueryEnd::Timeout);
        assert_eq!(caps.kitty_flags, None);
        assert_eq!(caps.background, Some((0, 0, 0)));
        assert!(TermCaps::not_asked().is_done());
    }

    #[test]
    fn answered_query_rows_are_ok_with_the_answers() {
        let rows = capability_checks(&full_answer(), Os::Linux, false);
        assert_eq!(
            whys(&rows),
            [
                ("Terminal (XTVERSION)", CheckStatus::Ok, "xterm(390)"),
                ("SGR mouse (1006)", CheckStatus::Ok, "Supported."),
                ("Bracketed paste (2004)", CheckStatus::Ok, "Supported."),
                (
                    "Synchronized output (2026)",
                    CheckStatus::Ok,
                    "Supported: frames paint without tearing."
                ),
                (
                    "Kitty keyboard (CSI u)",
                    CheckStatus::Ok,
                    "Supported (flags 15)."
                ),
                ("Background (OSC 11)", CheckStatus::Ok, "#ffffff, light."),
            ]
        );
        assert!(rows.iter().all(|check| check.group == CheckGroup::Terminal));
        assert!(rows.iter().all(|check| check.fix.is_none()), "{rows:#?}");

        let dark = answered(vec![TermReply::Background((0x1a, 0x1b, 0x26))]);
        let rows = capability_checks(&dark, Os::Macos, false);
        assert_eq!(rows[5].why, "#1a1b26, dark.");
    }

    #[test]
    fn unsupported_modes_warn_only_for_sgr_mouse() {
        for value in [0, 4] {
            let caps = answered(
                [1006, 2004, 2026]
                    .into_iter()
                    .map(|mode| TermReply::ModeReport { mode, value })
                    .collect(),
            );
            let rows = capability_checks(&caps, Os::Linux, false);
            assert_eq!(
                whys(&rows[1..4]),
                [
                    (
                        "SGR mouse (1006)",
                        CheckStatus::Warn,
                        "Not supported: mouse reports past column 223 can be lost."
                    ),
                    (
                        "Bracketed paste (2004)",
                        CheckStatus::Ok,
                        "Not supported. The app does not use bracketed paste."
                    ),
                    (
                        "Synchronized output (2026)",
                        CheckStatus::Ok,
                        "Not supported (optional): a large redraw can flicker."
                    ),
                ],
                "DECRPM {value}"
            );
            assert_eq!(
                rows[1].fix,
                Some(Fix {
                    hint: Some("Use a terminal with SGR mouse reports (1006).".to_string()),
                    copy: None,
                })
            );
            assert_eq!(rows[2].fix, None);
        }
    }

    #[test]
    fn da1_with_no_other_reply_is_unknown_except_kitty() {
        let rows = capability_checks(&answered(Vec::new()), Os::Linux, false);
        assert_eq!(
            whys(&rows),
            [
                (
                    "Terminal (XTVERSION)",
                    CheckStatus::Unknown,
                    "The terminal did not report its name and version. (no answer)"
                ),
                (
                    "SGR mouse (1006)",
                    CheckStatus::Unknown,
                    "The terminal did not report mode 1006. (no answer)"
                ),
                (
                    "Bracketed paste (2004)",
                    CheckStatus::Unknown,
                    "The terminal did not report mode 2004. (no answer)"
                ),
                (
                    "Synchronized output (2026)",
                    CheckStatus::Unknown,
                    "The terminal did not report mode 2026. (no answer)"
                ),
                (
                    "Kitty keyboard (CSI u)",
                    CheckStatus::Ok,
                    "Not supported (optional): keys use the legacy encoding."
                ),
                (
                    "Background (OSC 11)",
                    CheckStatus::Unknown,
                    "The terminal did not report its background: themeMode auto keeps the \
                     configured theme. (no answer)"
                ),
            ]
        );
    }

    #[test]
    fn timeout_with_no_reply_is_unknown_with_the_wait() {
        let mut caps = TermCaps::default();
        caps.apply(TermReply::XtVersion("tmux 3.4".to_string()));
        caps.time_out();
        let rows = capability_checks(&caps, Os::Linux, false);
        assert_eq!(rows[0].status, CheckStatus::Ok);
        assert_eq!(rows[0].why, "tmux 3.4");
        for check in &rows[1..] {
            assert_eq!(check.status, CheckStatus::Unknown, "{}", check.name);
            assert!(
                check.why.ends_with(" (no answer within 200 ms)"),
                "{}: {}",
                check.name,
                check.why
            );
        }
        assert_eq!(
            rows[4].why,
            "The terminal did not report its kitty keyboard flags. (no answer within 200 ms)",
            "no DA1: kitty support is not known"
        );
    }

    #[test]
    fn windows_and_not_asked_rows_are_unknown_with_no_hint() {
        let names = [
            "Terminal (XTVERSION)",
            "SGR mouse (1006)",
            "Bracketed paste (2004)",
            "Synchronized output (2026)",
            "Kitty keyboard (CSI u)",
            "Background (OSC 11)",
        ];
        for (caps, os, why) in [
            (full_answer(), Os::Windows, "Not asked on Windows."),
            (TermCaps::not_asked(), Os::Windows, "Not asked on Windows."),
            (
                TermCaps::not_asked(),
                Os::Linux,
                "Not asked: stdin or stdout is not a terminal.",
            ),
        ] {
            let rows = capability_checks(&caps, os, true);
            let got: Vec<&str> = rows.iter().map(|check| check.name).collect();
            assert_eq!(got, names, "{os:?}");
            for check in &rows {
                assert_eq!(check.status, CheckStatus::Unknown, "{}", check.name);
                assert_eq!(check.why, why, "{}", check.name);
                assert_eq!(check.fix, None, "{}", check.name);
            }
        }
    }

    #[test]
    fn tmux_hint_goes_on_every_row_without_another_hint() {
        let caps = answered(vec![TermReply::ModeReport {
            mode: 1006,
            value: 0,
        }]);
        let rows = capability_checks(&caps, Os::Linux, true);
        for check in &rows {
            let want = if check.name == "SGR mouse (1006)" {
                "Use a terminal with SGR mouse reports (1006)."
            } else {
                "Inside tmux, tmux answers for the outer terminal."
            };
            assert_eq!(hint_of(check), Some(want), "{}", check.name);
            assert_eq!(
                check.fix.as_ref().and_then(|fix| fix.copy.as_deref()),
                None,
                "{}: a hint, not a copy line",
                check.name
            );
        }
        let outside = capability_checks(&caps, Os::Linux, false);
        assert_eq!(hint_of(&outside[0]), None, "no tmux: no hint");
    }
}

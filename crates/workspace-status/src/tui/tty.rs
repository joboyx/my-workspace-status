//! Shared TTY mouse enable sequence, mouse decode, and live byte reader.
//!
//! The live event loop reads with [`poll_event`] / [`read_event`]. On Unix
//! the reader tags each key as [`KeyStrokeOrigin::LegacyByte`] or
//! [`KeyStrokeOrigin::Protocol`] from the bytes. It decodes SGR, X10, and
//! rxvt 1015 mouse like crossterm 0.28, except wheel reports with the
//! any-event motion bit, which decode as the plain wheel. A lone ESC waits one
//! poll timeout with no further stdin before it becomes Escape, so a split
//! CSI / CSI-u report is not an Escape plus leftover keys. A hangup or
//! 0-byte read is a read error. [`decode_sgr_mouse`] matches crossterm's
//! `parse_cb` / `parse_csi_sgr_mouse` with one change: a wheel report with
//! the any-event motion bit decodes as the plain wheel.
//!
//! Each event also carries the instant its bytes were read from stdin.
//! Events from one read share that instant, so key gestures (`gg`, `gt`,
//! the typeless CSI-u echo, double-click) are timed by when the keys
//! arrived, not by when a busy event loop got to them.
//!
//! Terminal replies to the Doctor's capability query (XTVERSION DCS,
//! DECRPM, kitty keyboard flags, OSC 11, DA1) are parsed into
//! [`TermReply`] values and queued, never read as keys, also when they land
//! late. [`take_term_replies`] drains that queue. Other OSC replies are
//! skipped.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(not(unix))]
use crossterm::event;
use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};

use super::keys::KeyStrokeOrigin;
use super::term_bg::parse_osc11_reply;
use super::term_caps::{parse_csi_reply, TermReply};

/// ANSI written to enable mouse capture.
///
/// xterm mouse *protocol* modes 1000 / 1002 / 1003 are mutually exclusive
/// on some terminals; the last SET wins. This sequence sets click (1000),
/// button-event (1002), then any-event (1003) tracking, so any-event is the
/// active mode. Then it sets rxvt 1015 and SGR (`1006h` last). Any-event
/// tracking reports pointer motion with no button held. The tab strip uses
/// it for the `[✗]` hover. Under 1003 some terminals add the motion bit (32)
/// to wheel reports (`96`/`97` vertical, `98`/`99` horizontal). The Unix
/// reader decodes those as the plain wheel. [`MOUSE_DISABLE`] resets every
/// mode, 1003 included.
pub const MOUSE_ENABLE: &[u8] = b"\x1b[?1000h\x1b[?1002h\x1b[?1003h\x1b[?1015h\x1b[?1006h";

/// ANSI written to disable mouse capture: the inverse of [`MOUSE_ENABLE`], in
/// reverse order.
pub const MOUSE_DISABLE: &[u8] = b"\x1b[?1006l\x1b[?1015l\x1b[?1003l\x1b[?1002l\x1b[?1000l";

#[cfg(test)]
/// xterm SGR button for wheel right (trackpad hscroll).
pub(crate) const SGR_WHEEL_RIGHT: u8 = 67;
#[cfg(test)]
/// xterm SGR button for Shift+wheel down (common trackpad hscroll encoding).
pub(crate) const SGR_SHIFT_WHEEL_DOWN: u8 = 69;
#[cfg(test)]
/// Wheel right with the 1003 motion bit (`67 | 32`). Decodes as wheel right.
pub(crate) const SGR_WHEEL_RIGHT_MOTION: u8 = 67 | 32;
#[cfg(test)]
/// Pointer motion with no button held (`3 | 32`). Any-event tracking only.
pub(crate) const SGR_POINTER_MOVE: u8 = 3 | 32;

/// Enable mouse capture for the live TTY. See [`MOUSE_ENABLE`].
pub fn enable_mouse(out: &mut impl Write) -> io::Result<()> {
    out.write_all(MOUSE_ENABLE)?;
    out.flush()
}

/// Disable mouse capture. See [`MOUSE_DISABLE`].
///
/// Writes the bytes itself, as [`enable_mouse`] does. On Windows crossterm's
/// `DisableMouseCapture` restores the console mode that its own
/// `EnableMouseCapture` saved instead. [`enable_mouse`] never calls that, so
/// the command fails and the terminal keeps reporting the mouse after exit.
pub fn disable_mouse(out: &mut impl Write) -> io::Result<()> {
    out.write_all(MOUSE_DISABLE)?;
    out.flush()
}

/// Poll for a TTY event. Live loop only; same reader as [`read_event`].
pub fn poll_event(timeout: Duration) -> io::Result<bool> {
    #[cfg(unix)]
    {
        unix_poll(timeout)
    }
    #[cfg(not(unix))]
    {
        event::poll(timeout)
    }
}

/// Read one TTY event. Live loop only.
pub fn read_event() -> io::Result<Event> {
    read_event_origin().map(|(event, _, _)| event)
}

/// One TTY event, the origin of its bytes, and the instant those bytes
/// were read from stdin.
pub type TtyEvent = (Event, KeyStrokeOrigin, Instant);

/// Read one TTY event, the origin of its bytes, and when they arrived.
///
/// Unix tags raw printable / control bytes as
/// [`KeyStrokeOrigin::LegacyByte`]. CSI-u and other escape sequences
/// are [`KeyStrokeOrigin::Protocol`]. The instant is the stdin read that
/// completed the event, so events from one read share it. Windows keeps
/// `event::read`, tags every event as Protocol, and stamps the read time.
pub fn read_event_origin() -> io::Result<TtyEvent> {
    #[cfg(unix)]
    {
        unix_read()
    }
    #[cfg(not(unix))]
    {
        event::read().map(|event| (event, KeyStrokeOrigin::Protocol, Instant::now()))
    }
}

struct TtyBuf {
    raw: Vec<u8>,
    events: VecDeque<TtyEvent>,
    /// When the last stdin bytes were appended to `raw`. Events parsed
    /// from `raw` carry this instant.
    read_at: Instant,
    last_winsize: Option<(u16, u16)>,
    /// Set after [`unix_poll`] waited one timeout with a held prefix (a lone
    /// ESC, or `ESC P` / `ESC P >` that may start an XTVERSION reply) and
    /// no further stdin. [`unix_read`] turns a held prefix into keys
    /// (Escape, or Alt+`P` and `>`) only when this is set.
    lone_esc_ready: bool,
    /// Terminal replies parsed from `raw`, oldest first, until
    /// [`take_term_replies`] drains them. At most [`REPLY_QUEUE_MAX`].
    replies: Vec<TermReply>,
}

/// Most terminal replies [`TtyBuf::replies`] keeps. A full answer to the
/// Doctor's query is 7 replies; past the cap the oldest reply goes, so a
/// terminal that sends replies while nothing drains them cannot grow memory.
const REPLY_QUEUE_MAX: usize = 64;

/// Queue `reply`, dropping the oldest one past [`REPLY_QUEUE_MAX`].
fn push_reply(replies: &mut Vec<TermReply>, reply: TermReply) {
    if replies.len() >= REPLY_QUEUE_MAX {
        replies.remove(0);
    }
    replies.push(reply);
}

fn empty_tty_buf() -> TtyBuf {
    TtyBuf {
        raw: Vec::new(),
        events: VecDeque::new(),
        read_at: Instant::now(),
        last_winsize: None,
        lone_esc_ready: false,
        replies: Vec::new(),
    }
}

fn tty_buf() -> &'static Mutex<TtyBuf> {
    static BUF: OnceLock<Mutex<TtyBuf>> = OnceLock::new();
    BUF.get_or_init(|| Mutex::new(empty_tty_buf()))
}

fn lock_tty_buf() -> std::sync::MutexGuard<'static, TtyBuf> {
    tty_buf()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

/// Hand stdin bytes that a startup query read back to the live reader.
///
/// The bytes go in front of any stdin not read yet, so a key typed while
/// [`super::term_bg::query_background`] waited for the terminal is read as
/// that key by [`poll_event`] / [`read_event`].
#[cfg(unix)]
pub(crate) fn push_pending_input(bytes: &[u8]) {
    if bytes.is_empty() {
        return;
    }
    let mut buf = lock_tty_buf();
    buf.raw.extend_from_slice(bytes);
    buf.read_at = Instant::now();
    buf.lone_esc_ready = false;
}

/// Take the terminal replies the reader parsed since the last call, oldest
/// first.
///
/// The input thread calls it after each [`poll_event`] /
/// [`read_event_origin`] attempt and forwards the replies to the loop
/// thread. A reply is never an event: a read that brought only replies
/// ends in `WouldBlock`. The loop thread also calls it, to drop the replies
/// still queued, right before it writes the Doctor's query.
pub(crate) fn take_term_replies() -> Vec<TermReply> {
    std::mem::take(&mut lock_tty_buf().replies)
}

/// The terminal replies in `bytes`, parsed the way the live reader parses
/// them. Keys, mouse reports, and a cut-off tail are dropped. Used by
/// [`super::term_caps::query_capabilities`].
#[cfg(unix)]
pub(crate) fn term_replies_in(bytes: &[u8]) -> Vec<TermReply> {
    let mut buf = empty_tty_buf();
    buf.raw.extend_from_slice(bytes);
    drain_parsed(&mut buf, false);
    buf.replies
}

#[cfg(unix)]
fn unix_poll(timeout: Duration) -> io::Result<bool> {
    {
        let mut buf = lock_tty_buf();
        drain_parsed(&mut buf, stdin_pending()? > 0);
        if !buf.events.is_empty() {
            return Ok(true);
        }
        if stdin_pending()? > 0 {
            return Ok(true);
        }
        if enqueue_resize_if_changed(&mut buf) {
            return Ok(true);
        }
        // Lone ESC is not ready yet. CSI may still arrive on the next
        // poll timeout (REPORT_ALL_KEYS_AS_ESCAPE_CODES can split).
    }
    // Do not call event::poll. Crossterm poll reads the TTY into its
    // own parser, so a later FIONREAD sees 0 and event::read tags a
    // raw `g` as Protocol.
    if poll_stdin(timeout)? {
        return Ok(true);
    }
    let mut buf = lock_tty_buf();
    let pending = stdin_pending()?;
    drain_parsed(&mut buf, pending > 0);
    if !buf.events.is_empty() || pending > 0 {
        return Ok(true);
    }
    if arm_lone_esc_after_wait(&mut buf, timeout) {
        return Ok(true);
    }
    Ok(enqueue_resize_if_changed(&mut buf))
}

#[cfg(unix)]
fn unix_read() -> io::Result<TtyEvent> {
    let pending = stdin_pending()?;
    let mut buf = lock_tty_buf();
    if pending > 0 {
        let bytes = read_stdin(pending)?;
        buf.raw.extend_from_slice(&bytes);
        buf.read_at = Instant::now();
        buf.lone_esc_ready = false;
    }
    if let Some(event) = take_ready_event(&mut buf, stdin_pending()? > 0) {
        return Ok(event);
    }
    if enqueue_resize_if_changed(&mut buf) {
        if let Some(event) = buf.events.pop_front() {
            return Ok(event);
        }
    }
    Err(io::Error::new(
        io::ErrorKind::WouldBlock,
        "no TTY event ready",
    ))
}

#[cfg(unix)]
fn stdin_pending() -> io::Result<usize> {
    let mut n: libc::c_int = 0;
    // SAFETY: `FIONREAD` writes one `c_int` through the pointer, and `n` is a
    // live `c_int` that outlives the call. A bad fd only makes it return -1.
    let rc = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::FIONREAD, &mut n) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n.max(0) as usize)
}

/// Wait up to `timeout` for stdin to be readable. `Ok(false)` on timeout or
/// `EINTR`; a hangup is an error.
#[cfg(unix)]
pub(crate) fn poll_stdin(timeout: Duration) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: `pfd` is one live `pollfd`, and the count passed is 1. `poll`
    // only writes `revents` in that struct.
    let n = unsafe { libc::poll(&mut pfd, 1, ms) };
    if n < 0 {
        let err = io::Error::last_os_error();
        if err.kind() == io::ErrorKind::Interrupted {
            return Ok(false);
        }
        return Err(err);
    }
    classify_poll(n, pfd.revents)
}

/// Read stdin with a poll wait until `done` says the bytes so far are
/// enough, `deadline` passes, or a poll or read fails or hangs up. Returns
/// every byte read.
///
/// For the blocking terminal queries before the input thread runs
/// ([`super::term_bg::query_background`]) and in the CLI
/// ([`super::term_caps::query_capabilities`]); never on the TTY event
/// thread.
#[cfg(unix)]
pub(crate) fn read_stdin_until(deadline: Instant, mut done: impl FnMut(&[u8]) -> bool) -> Vec<u8> {
    let mut got = Vec::new();
    let mut chunk = [0u8; 256];
    while !done(&got) {
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
        match usize::try_from(n) {
            Ok(n) if n > 0 => got.extend_from_slice(&chunk[..n]),
            _ => break,
        }
    }
    got
}

/// Map `poll` return + revents. POLLHUP without POLLIN is hangup, not idle.
#[cfg(unix)]
fn classify_poll(n: i32, revents: libc::c_short) -> io::Result<bool> {
    if n == 0 {
        return Ok(false);
    }
    if revents & libc::POLLIN != 0 {
        return Ok(true);
    }
    if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
        return Err(tty_hangup_error());
    }
    Ok(false)
}

fn tty_hangup_error() -> io::Error {
    io::Error::new(io::ErrorKind::UnexpectedEof, "TTY hangup")
}

/// Map `read` byte count. 0 is hangup, not an empty success.
fn classify_read(nread: isize) -> io::Result<usize> {
    if nread < 0 {
        return Err(io::Error::last_os_error());
    }
    if nread == 0 {
        return Err(tty_hangup_error());
    }
    Ok(nread as usize)
}

#[cfg(unix)]
fn read_winsize() -> io::Result<(u16, u16)> {
    // SAFETY: `winsize` is plain integers, so the all-zero value is valid.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    // SAFETY: `TIOCGWINSZ` writes one `winsize` through the pointer, and `ws`
    // is a live `winsize`. A bad fd only makes it return -1.
    let rc = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::TIOCGWINSZ, &mut ws) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok((ws.ws_col, ws.ws_row))
}

fn enqueue_resize_if_changed(buf: &mut TtyBuf) -> bool {
    #[cfg(unix)]
    {
        let Ok(size) = read_winsize() else {
            return false;
        };
        match buf.last_winsize {
            Some(prev) if prev != size => {
                buf.last_winsize = Some(size);
                buf.events.push_back((
                    Event::Resize(size.0, size.1),
                    KeyStrokeOrigin::Protocol,
                    Instant::now(),
                ));
                true
            }
            None => {
                buf.last_winsize = Some(size);
                false
            }
            Some(_) => false,
        }
    }
    #[cfg(not(unix))]
    {
        let _ = buf;
        false
    }
}

#[cfg(unix)]
fn read_stdin(n: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; n.max(1)];
    // SAFETY: `buf` is a live allocation of `buf.len()` writable bytes, and
    // `read` writes at most that many.
    let nread = unsafe {
        libc::read(
            libc::STDIN_FILENO,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
        )
    };
    if nread < 0 {
        return Err(io::Error::last_os_error());
    }
    let nread = classify_read(nread)?;
    buf.truncate(nread);
    Ok(buf)
}

fn take_ready_event(buf: &mut TtyBuf, more_stdin: bool) -> Option<TtyEvent> {
    drain_parsed(buf, more_stdin);
    if let Some(event) = buf.events.pop_front() {
        buf.lone_esc_ready = false;
        return Some(event);
    }
    if !(buf.lone_esc_ready && can_finish_lone_esc(buf) && !more_stdin) {
        return None;
    }
    buf.lone_esc_ready = false;
    if buf.raw == [0x1b] {
        buf.raw.clear();
        return Some((
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            KeyStrokeOrigin::Protocol,
            buf.read_at,
        ));
    }
    // `ESC P` or `ESC P >` that no reply followed: Alt+P, then `>` as typed.
    if let Take::Ready(n, event, origin) = take_esc_keys(&buf.raw, false) {
        buf.raw.drain(..n.min(buf.raw.len()));
        let read_at = buf.read_at;
        buf.events.push_back((event, origin, read_at));
    }
    drain_parsed(buf, false);
    buf.events.pop_front()
}

/// True when `raw` is only a held prefix and no event waits: a lone ESC,
/// or `ESC P` / `ESC P >` (the start of an XTVERSION reply).
fn can_finish_lone_esc(buf: &TtyBuf) -> bool {
    is_held_prefix(&buf.raw) && buf.events.is_empty()
}

/// A lone ESC, `ESC P`, or `ESC P >`: bytes that may be the start of a
/// longer sequence, held until more stdin arrives or one poll timeout
/// passes with none.
fn is_held_prefix(raw: &[u8]) -> bool {
    !raw.is_empty() && raw.len() < XTVERSION_INTRO.len() && XTVERSION_INTRO.starts_with(raw)
}

/// Arm a held prefix (Escape, or Alt+`P`) only after a non-zero poll wait
/// with no further stdin.
///
/// `poll_event(0)` (nav backlog drain) must not count as that wait.
fn arm_lone_esc_after_wait(buf: &mut TtyBuf, timeout: Duration) -> bool {
    if timeout.is_zero() || !can_finish_lone_esc(buf) {
        return false;
    }
    buf.lone_esc_ready = true;
    true
}

fn drain_parsed(buf: &mut TtyBuf, more: bool) {
    loop {
        if buf.raw.is_empty() {
            return;
        }
        match take_event(&buf.raw, more) {
            Take::NeedMore => return,
            Take::Skip(n) => {
                let n = n.min(buf.raw.len()).max(1);
                buf.raw.drain(..n);
            }
            Take::Ready(n, event, origin) => {
                let n = n.min(buf.raw.len()).max(1);
                buf.raw.drain(..n);
                let read_at = buf.read_at;
                buf.events.push_back((event, origin, read_at));
            }
            Take::Reply(n, reply) => {
                let n = n.min(buf.raw.len()).max(1);
                buf.raw.drain(..n);
                push_reply(&mut buf.replies, reply);
            }
        }
    }
}

enum Take {
    NeedMore,
    Skip(usize),
    Ready(usize, Event, KeyStrokeOrigin),
    /// A terminal reply of this many bytes: queued, never an event.
    Reply(usize, TermReply),
}

/// Parse the next event from `bytes`. `more` is true when more stdin
/// bytes are already waiting (lone ESC must not become Escape yet).
fn take_event(bytes: &[u8], more: bool) -> Take {
    if bytes.is_empty() {
        return Take::NeedMore;
    }
    match bytes[0] {
        0x1b => take_esc(bytes, more),
        b'\r' => key_legacy(1, KeyCode::Enter, KeyModifiers::NONE),
        b'\t' => key_legacy(1, KeyCode::Tab, KeyModifiers::NONE),
        0x7f => key_legacy(1, KeyCode::Backspace, KeyModifiers::NONE),
        c @ 0x01..=0x1a => key_legacy(
            1,
            KeyCode::Char((c - 1 + b'a') as char),
            KeyModifiers::CONTROL,
        ),
        c @ 0x1c..=0x1f => key_legacy(
            1,
            KeyCode::Char((c - 0x1c + b'4') as char),
            KeyModifiers::CONTROL,
        ),
        0x00 => key_legacy(1, KeyCode::Char(' '), KeyModifiers::CONTROL),
        _ => take_utf8(bytes),
    }
}

fn key_legacy(n: usize, code: KeyCode, modifiers: KeyModifiers) -> Take {
    Take::Ready(
        n,
        Event::Key(KeyEvent::new(code, modifiers)),
        KeyStrokeOrigin::LegacyByte,
    )
}

fn key_protocol(n: usize, key: KeyEvent) -> Take {
    Take::Ready(n, Event::Key(key), KeyStrokeOrigin::Protocol)
}

fn take_utf8(bytes: &[u8]) -> Take {
    for n in 1..=bytes.len().min(4) {
        if let Ok(text) = std::str::from_utf8(&bytes[..n]) {
            if let Some(c) = text.chars().next() {
                if c.len_utf8() == n {
                    let modifiers = if c.is_uppercase() {
                        KeyModifiers::SHIFT
                    } else {
                        KeyModifiers::NONE
                    };
                    return key_legacy(n, KeyCode::Char(c), modifiers);
                }
            }
        }
    }
    if bytes.len() < 4 {
        Take::NeedMore
    } else {
        Take::Skip(1)
    }
}

fn take_esc(bytes: &[u8], more: bool) -> Take {
    if bytes.len() == 1 {
        // Never promote Escape here. Wait one poll timeout with no
        // further stdin so a split CSI / CSI-u is not leftover keys.
        return Take::NeedMore;
    }
    let reply = match bytes[1] {
        b']' => take_osc_reply(bytes, more),
        b'P' => take_dcs_reply(bytes),
        _ => None,
    };
    reply.unwrap_or_else(|| take_esc_keys(bytes, more))
}

/// An escape sequence that is not a terminal reply: SS3, CSI, a doubled
/// ESC, or Alt plus the next key.
fn take_esc_keys(bytes: &[u8], more: bool) -> Take {
    match bytes[1] {
        b'O' => take_ss3(bytes),
        b'[' => take_csi(bytes),
        0x1b => key_protocol(1, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
        _ => match take_event(&bytes[1..], more) {
            Take::NeedMore => Take::NeedMore,
            Take::Skip(n) => Take::Skip(n + 1),
            Take::Ready(n, Event::Key(mut key), _) => {
                key.modifiers |= KeyModifiers::ALT;
                Take::Ready(n + 1, Event::Key(key), KeyStrokeOrigin::Protocol)
            }
            Take::Ready(n, event, _) => Take::Ready(n + 1, event, KeyStrokeOrigin::Protocol),
            Take::Reply(n, reply) => Take::Reply(n + 1, reply),
        },
    }
}

/// Longest unterminated OSC or DCS reply the reader holds before it treats
/// the bytes as keys again. An OSC 11 colour reply is about 30 bytes, an
/// XTVERSION reply about 20.
const STRING_REPLY_MAX: usize = 256;

/// Take a terminal OSC reply (`ESC ] Ps ; ... BEL` or `... ESC \`).
///
/// An OSC 11 background reply parses into [`TermReply::Background`]; any
/// other OSC reply is skipped. A reply that lands after a query stopped
/// waiting ([`super::term_bg::query_background`]) must not read as Alt+`]`
/// and a run of typed keys. `None` when the bytes do not start `ESC ] Ps ;`
/// (Alt+`]` stays a key), or when an unterminated reply passes
/// [`STRING_REPLY_MAX`] bytes.
fn take_osc_reply(bytes: &[u8], more: bool) -> Option<Take> {
    let body = &bytes[2..];
    let digits = body.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits > 0 && body.get(digits) == Some(&b';') {
        if let Some(end) = string_end(bytes) {
            return Some(match parse_osc11_reply(&bytes[..end]) {
                Some(rgb) => Take::Reply(end, TermReply::Background(rgb)),
                None => Take::Skip(end),
            });
        }
        return (bytes.len() < STRING_REPLY_MAX).then_some(Take::NeedMore);
    }
    // `ESC ]` plus digits so far: wait only while more stdin is queued.
    (digits == body.len() && more).then_some(Take::NeedMore)
}

/// Start of an XTVERSION reply (`ESC P > | <text> ESC \`).
const XTVERSION_INTRO: &[u8] = b"\x1bP>|";

/// Take an XTVERSION reply (`ESC P > | <text> ESC \`) as
/// [`TermReply::XtVersion`]. In the text each control character becomes a
/// space, runs of spaces collapse to one, and the ends are trimmed; an
/// empty text is skipped.
///
/// `ESC P` or `ESC P >` alone is a held prefix: it waits for more bytes,
/// and [`take_ready_event`] turns it into Alt+`P` only after one poll
/// timeout with no further stdin, as a lone ESC becomes Escape. `None`
/// when the bytes do not start `ESC P > |` (Alt+`P` stays a key), or when
/// an unterminated reply passes [`STRING_REPLY_MAX`] bytes.
fn take_dcs_reply(bytes: &[u8]) -> Option<Take> {
    if !bytes.starts_with(XTVERSION_INTRO) {
        return is_held_prefix(bytes).then_some(Take::NeedMore);
    }
    let Some(end) = string_end(bytes) else {
        return (bytes.len() < STRING_REPLY_MAX).then_some(Take::NeedMore);
    };
    let body_end = if bytes[end - 1] == 0x07 {
        end - 1
    } else {
        end - 2
    };
    let text = String::from_utf8_lossy(&bytes[XTVERSION_INTRO.len()..body_end])
        .replace(char::is_control, " ")
        .split(' ')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    Some(if text.is_empty() {
        Take::Skip(end)
    } else {
        Take::Reply(end, TermReply::XtVersion(text))
    })
}

/// Byte length of an OSC or DCS string through its BEL or `ESC \`
/// terminator.
fn string_end(bytes: &[u8]) -> Option<usize> {
    (2..bytes.len()).find_map(|i| match bytes[i] {
        0x07 => Some(i + 1),
        0x1b if bytes.get(i + 1) == Some(&b'\\') => Some(i + 2),
        _ => None,
    })
}

fn take_ss3(bytes: &[u8]) -> Take {
    if bytes.len() < 3 {
        return Take::NeedMore;
    }
    let code = match bytes[2] {
        b'D' => KeyCode::Left,
        b'C' => KeyCode::Right,
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        val @ b'P'..=b'S' => KeyCode::F(1 + val - b'P'),
        _ => return Take::Skip(3),
    };
    key_protocol(3, KeyEvent::new(code, KeyModifiers::NONE))
}

fn take_csi(bytes: &[u8]) -> Take {
    if bytes.len() < 3 {
        return Take::NeedMore;
    }
    match bytes[2] {
        b'[' => {
            if bytes.len() < 4 {
                return Take::NeedMore;
            }
            match bytes[3] {
                val @ b'A'..=b'E' => key_protocol(
                    4,
                    KeyEvent::new(KeyCode::F(1 + val - b'A'), KeyModifiers::NONE),
                ),
                _ => Take::Skip(4),
            }
        }
        b'D' => key_protocol(3, KeyEvent::new(KeyCode::Left, KeyModifiers::NONE)),
        b'C' => key_protocol(3, KeyEvent::new(KeyCode::Right, KeyModifiers::NONE)),
        b'A' => key_protocol(3, KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        b'B' => key_protocol(3, KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        b'H' => key_protocol(3, KeyEvent::new(KeyCode::Home, KeyModifiers::NONE)),
        b'F' => key_protocol(3, KeyEvent::new(KeyCode::End, KeyModifiers::NONE)),
        b'Z' => key_protocol(
            3,
            KeyEvent::new_with_kind(KeyCode::BackTab, KeyModifiers::SHIFT, KeyEventKind::Press),
        ),
        b'M' => take_normal_mouse(bytes),
        b'<' => take_sgr(bytes),
        b'I' => Take::Ready(3, Event::FocusGained, KeyStrokeOrigin::Protocol),
        b'O' => Take::Ready(3, Event::FocusLost, KeyStrokeOrigin::Protocol),
        b'P' => key_protocol(3, KeyEvent::new(KeyCode::F(1), KeyModifiers::NONE)),
        b'Q' => key_protocol(3, KeyEvent::new(KeyCode::F(2), KeyModifiers::NONE)),
        b'S' => key_protocol(3, KeyEvent::new(KeyCode::F(4), KeyModifiers::NONE)),
        b'?' => take_csi_query(bytes),
        b';' | b'0'..=b'9' => take_csi_numbered(bytes),
        _ => Take::Skip(3),
    }
}

fn take_normal_mouse(bytes: &[u8]) -> Take {
    if bytes.len() < 6 {
        return Take::NeedMore;
    }
    // X10: ESC [ M Cb Cx Cy. Each payload byte is value + 32.
    let Some(cb) = bytes[3].checked_sub(32) else {
        return Take::Skip(6);
    };
    let column = u16::from(bytes[4].saturating_sub(32)).saturating_sub(1);
    let row = u16::from(bytes[5].saturating_sub(32)).saturating_sub(1);
    match mouse_from_cb(cb, column, row) {
        Some(event) => Take::Ready(6, event, KeyStrokeOrigin::Protocol),
        None => Take::Skip(6),
    }
}

fn take_rxvt_mouse(bytes: &[u8]) -> Take {
    // rxvt / DECSET 1015: ESC [ Cb ; Cx ; Cy [;] M. Cb is value + 32.
    let Ok(body) = std::str::from_utf8(&bytes[2..bytes.len() - 1]) else {
        return Take::Skip(bytes.len());
    };
    let mut split = body.split(';');
    let Some(cb) = split
        .next()
        .and_then(|field| field.parse::<u8>().ok())
        .and_then(|value| value.checked_sub(32))
    else {
        return Take::Skip(bytes.len());
    };
    let Some(cx) = split
        .next()
        .and_then(|field| field.parse::<u16>().ok())
        .filter(|value| *value > 0)
    else {
        return Take::Skip(bytes.len());
    };
    let Some(cy) = split
        .next()
        .and_then(|field| field.parse::<u16>().ok())
        .filter(|value| *value > 0)
    else {
        return Take::Skip(bytes.len());
    };
    match mouse_from_cb(cb, cx - 1, cy - 1) {
        Some(event) => Take::Ready(bytes.len(), event, KeyStrokeOrigin::Protocol),
        None => Take::Skip(bytes.len()),
    }
}

fn mouse_from_cb(cb: u8, column: u16, row: u16) -> Option<Event> {
    let (kind, modifiers) = sgr_button_kind(cb)?;
    Some(Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers,
    }))
}

fn take_sgr(bytes: &[u8]) -> Take {
    let Some(end) = bytes.iter().position(|b| *b == b'M' || *b == b'm') else {
        return Take::NeedMore;
    };
    let seq = &bytes[..=end];
    match decode_sgr_mouse(seq) {
        Some(event) => Take::Ready(seq.len(), event, KeyStrokeOrigin::Protocol),
        None => Take::Skip(seq.len()),
    }
}

fn csi_final_index(bytes: &[u8]) -> Option<usize> {
    bytes
        .iter()
        .enumerate()
        .skip(2)
        .find(|(_, b)| (64..=126).contains(*b))
        .map(|(i, _)| i)
}

/// Take a `CSI ? ... final` sequence: a DECRPM, kitty keyboard flags, or
/// DA1 reply becomes a [`TermReply`]; any other one is skipped.
fn take_csi_query(bytes: &[u8]) -> Take {
    let Some(end) = csi_final_index(bytes) else {
        return Take::NeedMore;
    };
    match parse_csi_reply(&bytes[..=end]) {
        Some(reply) => Take::Reply(end + 1, reply),
        None => Take::Skip(end + 1),
    }
}

fn take_csi_numbered(bytes: &[u8]) -> Take {
    let Some(end) = csi_final_index(bytes) else {
        return Take::NeedMore;
    };
    let seq = &bytes[..=end];
    match seq[end] {
        b'~' => take_special_key(seq),
        b'u' => take_csi_u(seq),
        b'M' => take_rxvt_mouse(seq),
        b'R' => Take::Skip(seq.len()),
        _ => take_modified_arrow(seq),
    }
}

fn take_special_key(bytes: &[u8]) -> Take {
    let Ok(body) = std::str::from_utf8(&bytes[2..bytes.len() - 1]) else {
        return Take::Skip(bytes.len());
    };
    let mut split = body.split(';');
    let Ok(first) = split.next().unwrap_or("").parse::<u8>() else {
        return Take::Skip(bytes.len());
    };
    let (modifiers, kind) = parse_mod_kind(split.next());
    let code = match first {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        v @ 11..=15 => KeyCode::F(v - 10),
        v @ 17..=21 => KeyCode::F(v - 11),
        v @ 23..=26 => KeyCode::F(v - 12),
        v @ 28..=29 => KeyCode::F(v - 15),
        v @ 31..=34 => KeyCode::F(v - 17),
        _ => return Take::Skip(bytes.len()),
    };
    key_protocol(bytes.len(), KeyEvent::new_with_kind(code, modifiers, kind))
}

fn take_modified_arrow(bytes: &[u8]) -> Take {
    let (modifiers, kind) = {
        let Ok(body) = std::str::from_utf8(&bytes[2..bytes.len() - 1]) else {
            return Take::Skip(bytes.len());
        };
        let mut split = body.split(';');
        split.next();
        parse_mod_kind(split.next())
    };
    let code = match bytes[bytes.len() - 1] {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'F' => KeyCode::End,
        b'H' => KeyCode::Home,
        b'P' => KeyCode::F(1),
        b'Q' => KeyCode::F(2),
        b'R' => KeyCode::F(3),
        b'S' => KeyCode::F(4),
        _ => return Take::Skip(bytes.len()),
    };
    key_protocol(bytes.len(), KeyEvent::new_with_kind(code, modifiers, kind))
}

fn take_csi_u(bytes: &[u8]) -> Take {
    let Ok(body) = std::str::from_utf8(&bytes[2..bytes.len() - 1]) else {
        return Take::Skip(bytes.len());
    };
    let mut split = body.split(';');
    let Some(code_field) = split.next() else {
        return Take::Skip(bytes.len());
    };
    let mut codes = code_field.split(':');
    let Ok(codepoint) = codes.next().unwrap_or("").parse::<u32>() else {
        return Take::Skip(bytes.len());
    };
    let (mut modifiers, kind) = parse_mod_kind(split.next());
    let mut code = if let Some(mapped) = translate_functional(codepoint) {
        mapped
    } else if let Some(c) = char::from_u32(codepoint) {
        match c {
            '\x1b' => KeyCode::Esc,
            '\r' => KeyCode::Enter,
            '\t' if modifiers.contains(KeyModifiers::SHIFT) => KeyCode::BackTab,
            '\t' => KeyCode::Tab,
            '\x7f' => KeyCode::Backspace,
            _ => KeyCode::Char(c),
        }
    } else {
        return Take::Skip(bytes.len());
    };
    if let Some(shifted) = shifted_key(code, modifiers, codes.next()) {
        code = KeyCode::Char(shifted);
        // Same modifiers as the legacy byte of that character: SHIFT only
        // on an uppercase letter (`O`), none on a symbol (`:`).
        modifiers.set(KeyModifiers::SHIFT, shifted.is_uppercase());
    }
    key_protocol(bytes.len(), KeyEvent::new_with_kind(code, modifiers, kind))
}

/// The character a Shift+key report names in its shifted-key field
/// (`CSI 59:58;2u` is Shift+`;` = `:`).
///
/// The terminal sends that field because the live loop asks for
/// `REPORT_ALTERNATE_KEYS`. It knows the keyboard layout, so `:`, `?`,
/// `>`, `_`, `!` (and `/` on a layout where that is Shift+7) come from it,
/// not from a guessed US layout. Used only for a text key with Shift and no
/// Ctrl / Alt / Super: Ctrl-Shift-u keeps `u`, as the legacy C0 byte does.
fn shifted_key(code: KeyCode, modifiers: KeyModifiers, field: Option<&str>) -> Option<char> {
    if !matches!(code, KeyCode::Char(_))
        || !modifiers.contains(KeyModifiers::SHIFT)
        || modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER)
    {
        return None;
    }
    field
        .and_then(|field| field.parse::<u32>().ok())
        .and_then(char::from_u32)
        .filter(|c| !c.is_control())
}

fn translate_functional(codepoint: u32) -> Option<KeyCode> {
    Some(match codepoint {
        57417 => KeyCode::Left,
        57418 => KeyCode::Right,
        57419 => KeyCode::Up,
        57420 => KeyCode::Down,
        57421 => KeyCode::PageUp,
        57422 => KeyCode::PageDown,
        57423 => KeyCode::Home,
        57424 => KeyCode::End,
        57425 => KeyCode::Insert,
        57426 => KeyCode::Delete,
        57414 => KeyCode::Enter,
        _ => return None,
    })
}

fn parse_mod_kind(field: Option<&str>) -> (KeyModifiers, KeyEventKind) {
    let Some(field) = field else {
        return (KeyModifiers::NONE, KeyEventKind::Press);
    };
    let mut parts = field.split(':');
    let Ok(mask) = parts.next().unwrap_or("").parse::<u8>() else {
        return (KeyModifiers::NONE, KeyEventKind::Press);
    };
    let kind = match parts.next().and_then(|part| part.parse::<u8>().ok()) {
        Some(2) => KeyEventKind::Repeat,
        Some(3) => KeyEventKind::Release,
        _ => KeyEventKind::Press,
    };
    (parse_modifiers(mask), kind)
}

fn parse_modifiers(mask: u8) -> KeyModifiers {
    let bits = mask.saturating_sub(1);
    let mut modifiers = KeyModifiers::empty();
    if bits & 1 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if bits & 2 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if bits & 4 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    if bits & 8 != 0 {
        modifiers |= KeyModifiers::SUPER;
    }
    modifiers
}

/// Parse a byte burst the way the live Unix reader does.
#[cfg(test)]
pub(crate) fn parse_tty_chunk(bytes: &[u8]) -> Vec<(Event, KeyStrokeOrigin)> {
    let mut buf = empty_tty_buf();
    buf.raw.extend_from_slice(bytes);
    drain_parsed(&mut buf, false);
    buf.events
        .into_iter()
        .map(|(event, origin, _)| (event, origin))
        .collect()
}

#[cfg(test)]
/// Encode one xterm SGR mouse report (`CSI < Cb ; Cx ; Cy M`).
///
/// `col` / `row` are 0-based cells, matching crossterm. The sequence uses
/// 1-based coordinates, which is what a TTY sends for trackpad hscroll.
pub(crate) fn sgr_mouse_report(button: u8, col: u16, row: u16) -> Vec<u8> {
    format!(
        "\x1b[<{button};{};{}M",
        col.saturating_add(1),
        row.saturating_add(1)
    )
    .into_bytes()
}

/// Decode one xterm SGR mouse report the way the live reader does.
///
/// This follows crossterm 0.28 `parse_csi_sgr_mouse` / `parse_cb`. Wheel
/// left/right are buttons 6/7 (`Cb` 66/67). Shift+wheel is the vertical
/// wheel plus bit 2 (`Cb` 68/69). Bit 5 is motion. Under any-event tracking
/// some terminals set it on wheel reports (`96`..`99`); those decode as the
/// plain wheel. crossterm 0.28 would report `Moved` or a parse error there.
/// Unknown reports are `None` so callers no-op.
pub(crate) fn decode_sgr_mouse(seq: &[u8]) -> Option<Event> {
    if seq.len() < 8 || !seq.starts_with(&[0x1b, b'[', b'<']) {
        return None;
    }
    let last = *seq.last()?;
    if last != b'M' && last != b'm' {
        return None;
    }
    let body = std::str::from_utf8(&seq[3..seq.len() - 1]).ok()?;
    let mut parts = body.split(';');
    let cb: u8 = parts.next()?.parse().ok()?;
    let cx: u16 = parts.next()?.parse().ok()?;
    let cy: u16 = parts.next()?.parse().ok()?;
    if cx == 0 || cy == 0 {
        return None;
    }
    let (kind, modifiers) = sgr_button_kind(cb)?;
    let kind = if last == b'm' {
        match kind {
            MouseEventKind::Down(button) => MouseEventKind::Up(button),
            other => other,
        }
    } else {
        kind
    };
    Some(Event::Mouse(MouseEvent {
        kind,
        column: cx.saturating_sub(1),
        row: cy.saturating_sub(1),
        modifiers,
    }))
}

/// Decode the SGR `Cb` field like crossterm 0.28 `parse_cb`.
///
/// Match arms are the live reader contract. A wheel button with the motion
/// bit is the plain wheel, so wheel and trackpad hscroll keep working under
/// any-event tracking (DECSET 1003). The motion bit is normalized only for
/// wheel buttons (bit 64 set, buttons 4..=7). Button drags (`Cb` 32..=63 with
/// low bits 0..=2) keep `Drag`, and buttonless motion (low bits 3) keeps
/// `Moved`, with or without Shift / Alt / Ctrl. Neither becomes a scroll.
fn sgr_button_kind(cb: u8) -> Option<(MouseEventKind, KeyModifiers)> {
    let button_number = (cb & 0b0000_0011) | ((cb & 0b1100_0000) >> 4);
    let dragging = cb & 0b0010_0000 == 0b0010_0000;
    let kind = match (button_number, dragging) {
        (0, false) => MouseEventKind::Down(MouseButton::Left),
        (1, false) => MouseEventKind::Down(MouseButton::Middle),
        (2, false) => MouseEventKind::Down(MouseButton::Right),
        (0, true) => MouseEventKind::Drag(MouseButton::Left),
        (1, true) => MouseEventKind::Drag(MouseButton::Middle),
        (2, true) => MouseEventKind::Drag(MouseButton::Right),
        (3, false) => MouseEventKind::Up(MouseButton::Left),
        (3, true) => MouseEventKind::Moved,
        (4, _) => MouseEventKind::ScrollUp,
        (5, _) => MouseEventKind::ScrollDown,
        (6, _) => MouseEventKind::ScrollLeft,
        (7, _) => MouseEventKind::ScrollRight,
        _ => return None,
    };
    let mut modifiers = KeyModifiers::empty();
    if cb & 0b0000_0100 != 0 {
        modifiers |= KeyModifiers::SHIFT;
    }
    if cb & 0b0000_1000 != 0 {
        modifiers |= KeyModifiers::ALT;
    }
    if cb & 0b0001_0000 != 0 {
        modifiers |= KeyModifiers::CONTROL;
    }
    Some((kind, modifiers))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mouse_enable_sets_any_event_tracking_before_sgr() {
        let ours = std::str::from_utf8(MOUSE_ENABLE).unwrap();
        for mode in ["1000h", "1002h", "1003h", "1015h", "1006h"] {
            assert!(ours.contains(mode), "missing {mode}: {ours:?}");
        }
        assert!(!ours.contains("1003l"), "must not reset 1003: {ours:?}");
        let at = |mode: &str| ours.find(mode).unwrap();
        assert!(
            at("1000h") < at("1003h") && at("1002h") < at("1003h"),
            "any-event must be the last protocol mode set: {ours:?}"
        );
        assert!(
            ours.ends_with("\x1b[?1006h"),
            "SGR must stay last: {ours:?}"
        );

        let mut disable = Vec::new();
        disable_mouse(&mut disable).unwrap();
        let disable = String::from_utf8(disable).unwrap();
        for mode in ["1000l", "1002l", "1003l", "1015l", "1006l"] {
            assert!(disable.contains(mode), "disable misses {mode}: {disable:?}");
        }
        #[cfg(unix)]
        assert!(
            super::super::term_restore::RESTORE_SEQ.starts_with(MOUSE_DISABLE),
            "the exit-signal restore turns the mouse off the same way"
        );
    }

    #[test]
    fn decode_sgr_mouse_matches_crossterm_parse_cb() {
        let click = decode_sgr_mouse(&sgr_mouse_report(0, 8, 4)).unwrap();
        assert_eq!(
            click,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 8,
                row: 4,
                modifiers: KeyModifiers::NONE,
            })
        );
        let drag = decode_sgr_mouse(&sgr_mouse_report(32, 8, 4)).unwrap();
        assert_eq!(
            drag,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Drag(MouseButton::Left),
                column: 8,
                row: 4,
                modifiers: KeyModifiers::NONE,
            })
        );
        let right = decode_sgr_mouse(&sgr_mouse_report(SGR_WHEEL_RIGHT, 8, 4)).unwrap();
        assert_eq!(
            right,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollRight,
                column: 8,
                row: 4,
                modifiers: KeyModifiers::NONE,
            })
        );
        let shift_wheel = decode_sgr_mouse(&sgr_mouse_report(SGR_SHIFT_WHEEL_DOWN, 8, 4)).unwrap();
        assert_eq!(
            shift_wheel,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollDown,
                column: 8,
                row: 4,
                modifiers: KeyModifiers::SHIFT,
            })
        );
    }

    #[test]
    fn motion_bit_wheel_decodes_as_plain_wheel() {
        let cases = [
            (96, MouseEventKind::ScrollUp, KeyModifiers::NONE),
            (97, MouseEventKind::ScrollDown, KeyModifiers::NONE),
            (98, MouseEventKind::ScrollLeft, KeyModifiers::NONE),
            (
                SGR_WHEEL_RIGHT_MOTION,
                MouseEventKind::ScrollRight,
                KeyModifiers::NONE,
            ),
            (
                SGR_SHIFT_WHEEL_DOWN | 32,
                MouseEventKind::ScrollDown,
                KeyModifiers::SHIFT,
            ),
            (99 | 16, MouseEventKind::ScrollRight, KeyModifiers::CONTROL),
        ];
        for (cb, kind, modifiers) in cases {
            let want = Event::Mouse(MouseEvent {
                kind,
                column: 8,
                row: 4,
                modifiers,
            });
            assert_eq!(
                decode_sgr_mouse(&sgr_mouse_report(cb, 8, 4)),
                Some(want.clone()),
                "SGR {cb}"
            );
            assert_eq!(
                decode_sgr_mouse(&sgr_mouse_report(cb & !32, 8, 4)),
                Some(want),
                "SGR {cb} must match its plain wheel"
            );
        }
        let rxvt = parse_tty_chunk(b"\x1b[131;9;5M");
        assert_eq!(
            rxvt[0].0,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::ScrollRight,
                column: 8,
                row: 4,
                modifiers: KeyModifiers::NONE,
            }),
            "1015 wheel right + motion (99 + 32)"
        );
    }

    #[test]
    fn buttonless_motion_decodes_as_moved() {
        for (cb, modifiers) in [
            (SGR_POINTER_MOVE, KeyModifiers::NONE),
            (SGR_POINTER_MOVE | 4, KeyModifiers::SHIFT),
            (SGR_POINTER_MOVE | 8, KeyModifiers::ALT),
            (SGR_POINTER_MOVE | 16, KeyModifiers::CONTROL),
        ] {
            assert_eq!(
                decode_sgr_mouse(&sgr_mouse_report(cb, 8, 4)),
                Some(Event::Mouse(MouseEvent {
                    kind: MouseEventKind::Moved,
                    column: 8,
                    row: 4,
                    modifiers,
                })),
                "SGR {cb}"
            );
        }
    }

    /// Every `Cb` with the motion bit but not the wheel bit is a drag or
    /// plain motion. Every `Cb` with the wheel and motion bits is a wheel.
    /// Modifier bits (4 Shift, 8 Alt, 16 Ctrl) never change that split.
    #[test]
    fn modifier_drags_and_motion_never_scroll() {
        fn mouse(kind: MouseEventKind, modifiers: KeyModifiers) -> Option<Event> {
            Some(Event::Mouse(MouseEvent {
                kind,
                column: 8,
                row: 4,
                modifiers,
            }))
        }
        fn mods(cb: u8) -> KeyModifiers {
            let mut modifiers = KeyModifiers::empty();
            if cb & 4 != 0 {
                modifiers |= KeyModifiers::SHIFT;
            }
            if cb & 8 != 0 {
                modifiers |= KeyModifiers::ALT;
            }
            if cb & 16 != 0 {
                modifiers |= KeyModifiers::CONTROL;
            }
            modifiers
        }
        for cb in 32u8..=63 {
            let kind = match cb & 3 {
                0 => MouseEventKind::Drag(MouseButton::Left),
                1 => MouseEventKind::Drag(MouseButton::Middle),
                2 => MouseEventKind::Drag(MouseButton::Right),
                _ => MouseEventKind::Moved,
            };
            assert_eq!(
                decode_sgr_mouse(&sgr_mouse_report(cb, 8, 4)),
                mouse(kind, mods(cb)),
                "SGR {cb} is a drag or motion, never a scroll"
            );
        }
        for cb in 96u8..=127 {
            let kind = match cb & 3 {
                0 => MouseEventKind::ScrollUp,
                1 => MouseEventKind::ScrollDown,
                2 => MouseEventKind::ScrollLeft,
                _ => MouseEventKind::ScrollRight,
            };
            assert_eq!(
                decode_sgr_mouse(&sgr_mouse_report(cb, 8, 4)),
                mouse(kind, mods(cb)),
                "SGR {cb} is a wheel with the motion bit"
            );
        }

        // Spot checks with literal values, so the loops above cannot share a
        // mistake with `mods`.
        for (cb, kind, modifiers) in [
            (
                36,
                MouseEventKind::Drag(MouseButton::Left),
                KeyModifiers::SHIFT,
            ),
            (
                40,
                MouseEventKind::Drag(MouseButton::Left),
                KeyModifiers::ALT,
            ),
            (
                48,
                MouseEventKind::Drag(MouseButton::Left),
                KeyModifiers::CONTROL,
            ),
            (39, MouseEventKind::Moved, KeyModifiers::SHIFT),
            (101, MouseEventKind::ScrollDown, KeyModifiers::SHIFT),
        ] {
            assert_eq!(
                decode_sgr_mouse(&sgr_mouse_report(cb, 8, 4)),
                mouse(kind, modifiers),
                "SGR {cb}"
            );
        }

        // X10 (ESC [ M, each byte + 32) and rxvt 1015 (Cb + 32) reach the
        // same decoder after removing the offset.
        let shift_drag = mouse(MouseEventKind::Drag(MouseButton::Left), KeyModifiers::SHIFT);
        let x10 = [0x1b, b'[', b'M', 36 + 32, 9 + 32, 5 + 32];
        let events = parse_tty_chunk(&x10);
        assert_eq!(events.len(), 1, "X10 Shift+drag parses to one event");
        assert_eq!(Some(events[0].0.clone()), shift_drag, "X10 Shift+drag");
        let events = parse_tty_chunk(b"\x1b[68;9;5M");
        assert_eq!(events.len(), 1, "1015 Shift+drag parses to one event");
        assert_eq!(Some(events[0].0.clone()), shift_drag, "1015 Shift+drag");
    }

    fn key_code(event: &Event) -> KeyCode {
        match event {
            Event::Key(key) => key.code,
            other => panic!("expected key, got {other:?}"),
        }
    }

    #[test]
    fn raw_g_bytes_are_legacy_and_csi_u_g_is_protocol() {
        let raw = parse_tty_chunk(b"gg");
        assert_eq!(raw.len(), 2);
        assert_eq!(key_code(&raw[0].0), KeyCode::Char('g'));
        assert_eq!(raw[0].1, KeyStrokeOrigin::LegacyByte);
        assert_eq!(raw[1].1, KeyStrokeOrigin::LegacyByte);

        let typed = parse_tty_chunk(b"\x1b[103;1:1u\x1b[103;1:3u");
        assert_eq!(typed.len(), 2);
        assert_eq!(key_code(&typed[0].0), KeyCode::Char('g'));
        assert_eq!(typed[0].1, KeyStrokeOrigin::Protocol);
        match &typed[1].0 {
            Event::Key(key) => assert_eq!(key.kind, KeyEventKind::Release),
            other => panic!("{other:?}"),
        }

        let typeless = parse_tty_chunk(b"\x1b[103;1u\x1b[103;1u");
        assert_eq!(typeless.len(), 2);
        assert!(typeless
            .iter()
            .all(|(_, origin)| *origin == KeyStrokeOrigin::Protocol));
        match &typeless[0].0 {
            Event::Key(key) => assert_eq!(key.kind, KeyEventKind::Press),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_tty_chunk_keeps_home_and_sgr_mouse() {
        let home = parse_tty_chunk(b"\x1b[1;1:1~");
        assert_eq!(home.len(), 1);
        assert_eq!(key_code(&home[0].0), KeyCode::Home);
        assert_eq!(home[0].1, KeyStrokeOrigin::Protocol);

        let mouse = parse_tty_chunk(&sgr_mouse_report(SGR_WHEEL_RIGHT, 8, 4));
        assert_eq!(mouse.len(), 1);
        assert!(matches!(mouse[0].0, Event::Mouse(_)));
        assert_eq!(mouse[0].1, KeyStrokeOrigin::Protocol);
    }

    #[test]
    fn parse_tty_chunk_keeps_x10_and_rxvt_mouse() {
        // Same fixtures as crossterm 0.28 parse_csi_normal_mouse / rxvt.
        let x10 = parse_tty_chunk(b"\x1b[M0\x60\x70");
        assert_eq!(x10.len(), 1);
        assert_eq!(
            x10[0].0,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 63,
                row: 79,
                modifiers: KeyModifiers::CONTROL,
            })
        );
        assert_eq!(x10[0].1, KeyStrokeOrigin::Protocol);

        let rxvt = parse_tty_chunk(b"\x1b[32;30;40;M");
        assert_eq!(rxvt.len(), 1);
        assert_eq!(
            rxvt[0].0,
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 29,
                row: 39,
                modifiers: KeyModifiers::NONE,
            })
        );
        assert_eq!(rxvt[0].1, KeyStrokeOrigin::Protocol);

        let incomplete = parse_tty_chunk(b"\x1b[M0");
        assert!(
            incomplete.is_empty(),
            "partial X10 must wait for the 6-byte report"
        );
    }

    #[test]
    fn lone_esc_waits_for_idle_timeout_before_escape() {
        let mut buf = empty_tty_buf();
        buf.raw.extend_from_slice(&[0x1b]);
        assert!(
            take_ready_event(&mut buf, false).is_none(),
            "FIONREAD 0 must not promote Escape before one poll timeout"
        );
        assert_eq!(buf.raw, [0x1b]);

        buf.lone_esc_ready = true;
        let (event, origin, _) = take_ready_event(&mut buf, false).expect("idle timeout");
        assert_eq!(key_code(&event), KeyCode::Esc);
        assert_eq!(origin, KeyStrokeOrigin::Protocol);
        assert!(buf.raw.is_empty());
    }

    #[test]
    fn zero_poll_timeout_does_not_arm_lone_esc() {
        let mut buf = empty_tty_buf();
        buf.raw.extend_from_slice(&[0x1b]);
        assert!(
            !arm_lone_esc_after_wait(&mut buf, Duration::ZERO),
            "poll_event(0) must not treat a leftover ESC as idle"
        );
        assert!(!buf.lone_esc_ready);
        assert!(take_ready_event(&mut buf, false).is_none());
        assert!(arm_lone_esc_after_wait(&mut buf, Duration::from_millis(16)));
        assert!(buf.lone_esc_ready);
    }

    #[test]
    fn split_csi_after_esc_is_not_escape_plus_keys() {
        let mut arrow = empty_tty_buf();
        arrow.raw.extend_from_slice(&[0x1b]);
        assert!(take_ready_event(&mut arrow, false).is_none());
        arrow.raw.extend_from_slice(b"[A");
        arrow.lone_esc_ready = false;
        let (event, origin, _) = take_ready_event(&mut arrow, false).expect("Up");
        assert_eq!(key_code(&event), KeyCode::Up);
        assert_eq!(origin, KeyStrokeOrigin::Protocol);
        assert!(take_ready_event(&mut arrow, false).is_none());

        let mut csi_u = empty_tty_buf();
        csi_u.raw.extend_from_slice(&[0x1b]);
        assert!(take_ready_event(&mut csi_u, false).is_none());
        csi_u.raw.extend_from_slice(b"[103;1u");
        let (event, origin, _) = take_ready_event(&mut csi_u, false).expect("g");
        assert_eq!(key_code(&event), KeyCode::Char('g'));
        assert_eq!(origin, KeyStrokeOrigin::Protocol);
        assert!(take_ready_event(&mut csi_u, false).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn pushed_pending_input_is_read_as_keys() {
        push_pending_input(b"jq\x1b[A");
        let mut buf = lock_tty_buf();
        let mut keys = Vec::new();
        while let Some((event, origin, _)) = take_ready_event(&mut buf, false) {
            keys.push((key_code(&event), origin));
        }
        assert_eq!(
            keys,
            [
                (KeyCode::Char('j'), KeyStrokeOrigin::LegacyByte),
                (KeyCode::Char('q'), KeyStrokeOrigin::LegacyByte),
                (KeyCode::Up, KeyStrokeOrigin::Protocol),
            ],
            "bytes a startup query read back reach the live reader in order"
        );
        assert!(buf.raw.is_empty());
    }

    #[test]
    fn events_from_one_read_share_its_instant() {
        let mut buf = empty_tty_buf();
        let first = buf.read_at;
        buf.raw.extend_from_slice(b"g\x1b[116;1u");
        let later = first + Duration::from_millis(500);
        buf.read_at = later;
        let (_, _, g_at) = take_ready_event(&mut buf, false).expect("g");
        let (_, _, t_at) = take_ready_event(&mut buf, false).expect("t");
        assert_eq!(g_at, later, "an event carries the read that brought it");
        assert_eq!(
            g_at, t_at,
            "keys from one read share one instant, however late the loop takes them"
        );
    }

    #[test]
    fn late_osc_reply_is_skipped_not_typed() {
        for reply in [
            &b"\x1b]11;rgb:ffff/ffff/ffff\x07j"[..],
            &b"\x1b]11;rgb:00/00/00\x1b\\j"[..],
        ] {
            let events = parse_tty_chunk(reply);
            assert_eq!(events.len(), 1, "{reply:?}: {events:?}");
            assert_eq!(key_code(&events[0].0), KeyCode::Char('j'));
        }
        let alt = parse_tty_chunk(b"\x1b]x");
        assert_eq!(alt.len(), 2, "{alt:?}");
        match &alt[0].0 {
            Event::Key(key) => {
                assert_eq!(key.code, KeyCode::Char(']'));
                assert!(key.modifiers.contains(KeyModifiers::ALT));
            }
            other => panic!("expected Alt+], got {other:?}"),
        }
        assert_eq!(key_code(&alt[1].0), KeyCode::Char('x'));
    }

    /// A terminal that floods replies while nothing drains them keeps at
    /// most `REPLY_QUEUE_MAX` of them, the newest ones, and still types
    /// the key after them.
    #[test]
    fn reply_queue_keeps_only_the_newest_replies() {
        let mut bytes = Vec::new();
        for flags in 0..REPLY_QUEUE_MAX as u32 + 6 {
            bytes.extend_from_slice(format!("\x1b[?{flags}u").as_bytes());
        }
        bytes.push(b'j');
        let (events, replies) = parse_with_replies(&bytes);
        assert_eq!(replies.len(), REPLY_QUEUE_MAX);
        assert_eq!(
            replies.first(),
            Some(&TermReply::KittyFlags(6)),
            "oldest dropped"
        );
        assert_eq!(
            replies.last(),
            Some(&TermReply::KittyFlags(REPLY_QUEUE_MAX as u32 + 5))
        );
        assert_eq!(events.len(), 1, "{events:?}");
        assert_eq!(key_code(&events[0]), KeyCode::Char('j'));
    }

    /// Parse `bytes` as one read: the events and the queued replies.
    fn parse_with_replies(bytes: &[u8]) -> (Vec<Event>, Vec<TermReply>) {
        let mut buf = empty_tty_buf();
        buf.raw.extend_from_slice(bytes);
        drain_parsed(&mut buf, false);
        let events = buf.events.into_iter().map(|(event, _, _)| event).collect();
        (events, buf.replies)
    }

    fn alt_key(event: &Event) -> Option<KeyCode> {
        match event {
            Event::Key(key) if key.modifiers.contains(KeyModifiers::ALT) => Some(key.code),
            _ => None,
        }
    }

    #[test]
    fn capability_replies_are_queued_not_typed() {
        let mut cases: Vec<(Vec<u8>, TermReply)> = (0..=4u8)
            .map(|value| {
                (
                    format!("\x1b[?1006;{value}$y").into_bytes(),
                    TermReply::ModeReport { mode: 1006, value },
                )
            })
            .collect();
        cases.extend([
            (b"\x1b[?15u".to_vec(), TermReply::KittyFlags(15)),
            (b"\x1b[?62;22c".to_vec(), TermReply::DeviceAttributes),
            (
                b"\x1bP>|xterm(390)\x1b\\".to_vec(),
                TermReply::XtVersion("xterm(390)".to_string()),
            ),
            (
                b"\x1b]11;rgb:ffff/8080/0000\x07".to_vec(),
                TermReply::Background((255, 128, 0)),
            ),
            (
                b"\x1b]11;rgb:1a1a/1b1b/2626\x1b\\".to_vec(),
                TermReply::Background((26, 27, 38)),
            ),
        ]);
        for (reply, want) in cases {
            let mut bytes = reply.clone();
            bytes.push(b'j');
            let (events, replies) = parse_with_replies(&bytes);
            assert_eq!(replies, [want], "{reply:?}");
            assert_eq!(events.len(), 1, "{reply:?}: {events:?}");
            assert_eq!(key_code(&events[0]), KeyCode::Char('j'), "{reply:?}");
        }
    }

    #[test]
    fn whole_query_answer_queues_in_order_between_typed_keys() {
        let answer = b"\x1bP>|WezTerm 20240203\x1b\\\x1b[?1006;2$y\x1b[?2004;2$yj\
            \x1b[?2026;0$y\x1b[?1u\x1b]11;rgb:0000/0000/0000\x1b\\k\x1b[?65;1;9c";
        let (events, replies) = parse_with_replies(answer);
        assert_eq!(
            replies,
            [
                TermReply::XtVersion("WezTerm 20240203".to_string()),
                TermReply::ModeReport {
                    mode: 1006,
                    value: 2
                },
                TermReply::ModeReport {
                    mode: 2004,
                    value: 2
                },
                TermReply::ModeReport {
                    mode: 2026,
                    value: 0
                },
                TermReply::KittyFlags(1),
                TermReply::Background((0, 0, 0)),
                TermReply::DeviceAttributes,
            ]
        );
        let keys: Vec<KeyCode> = events.iter().map(key_code).collect();
        assert_eq!(keys, [KeyCode::Char('j'), KeyCode::Char('k')]);
    }

    #[test]
    fn other_osc_and_csi_query_replies_are_skipped_without_a_reply() {
        for bytes in [
            &b"\x1b]10;rgb:ffff/ffff/ffff\x07j"[..],
            b"\x1b]11;rgba:ffff/ffff/ffff/ffff\x07j",
            b"\x1b[?2026;9$yj",
            b"\x1b[?1;2Rj",
            b"\x1bP>|\x1b\\j",
        ] {
            let (events, replies) = parse_with_replies(bytes);
            assert!(replies.is_empty(), "{bytes:?}: {replies:?}");
            assert_eq!(events.len(), 1, "{bytes:?}: {events:?}");
            assert_eq!(key_code(&events[0]), KeyCode::Char('j'), "{bytes:?}");
        }
    }

    #[test]
    fn xtversion_text_turns_control_characters_into_one_space() {
        for (bytes, want) in [
            (&b"\x1bP>|tmux\x013.4\x1b\\"[..], "tmux 3.4"),
            (b"\x1bP>| tmux \x01\x02 3.4 \x1b\\", "tmux 3.4"),
            (b"\x1bP>|\x01xterm(390)\x7f\x1b\\", "xterm(390)"),
        ] {
            let (_, replies) = parse_with_replies(bytes);
            assert_eq!(
                replies,
                [TermReply::XtVersion(want.to_string())],
                "{bytes:?}"
            );
        }
        let (_, replies) = parse_with_replies(b"\x1bP>| \x01 \x1b\\");
        assert!(replies.is_empty(), "only blanks: no reply");
    }

    #[test]
    fn reply_split_across_two_reads_parses_once_whole() {
        for (head, tail, want) in [
            (
                &b"\x1bP>|xte"[..],
                &b"rm(390)\x1b\\j"[..],
                TermReply::XtVersion("xterm(390)".to_string()),
            ),
            (
                b"\x1b[?2026;2",
                b"$yj",
                TermReply::ModeReport {
                    mode: 2026,
                    value: 2,
                },
            ),
            (
                b"\x1b]11;rgb:ffff/ff",
                b"ff/ffff\x1b\\j",
                TermReply::Background((255, 255, 255)),
            ),
            (b"\x1b[?62;", b"22cj", TermReply::DeviceAttributes),
        ] {
            let mut buf = empty_tty_buf();
            buf.raw.extend_from_slice(head);
            assert!(take_ready_event(&mut buf, false).is_none(), "{head:?}");
            assert!(buf.replies.is_empty(), "{head:?}: waits for the tail");
            buf.raw.extend_from_slice(tail);
            let (event, _, _) = take_ready_event(&mut buf, false).expect("j");
            assert_eq!(key_code(&event), KeyCode::Char('j'), "{head:?}");
            assert_eq!(buf.replies, [want], "{head:?}");
            assert!(take_ready_event(&mut buf, false).is_none(), "{head:?}");
        }
    }

    /// A read cut right after `ESC P` or `ESC P >`, with no more stdin
    /// queued, holds those bytes: the next read completes the reply, and no
    /// key comes out.
    #[test]
    fn xtversion_cut_after_esc_p_is_held_not_typed() {
        for (head, tail) in [
            (&b"\x1bP"[..], &b">|xterm(390)\x1b\\"[..]),
            (b"\x1bP>", b"|xterm(390)\x1b\\"),
        ] {
            let mut buf = empty_tty_buf();
            buf.raw.extend_from_slice(head);
            assert!(take_ready_event(&mut buf, false).is_none(), "{head:?}");
            assert_eq!(buf.raw, head, "{head:?}: held");
            buf.raw.extend_from_slice(tail);
            assert!(take_ready_event(&mut buf, false).is_none(), "{head:?}");
            assert!(buf.events.is_empty(), "{head:?}: {:?}", buf.events);
            assert!(buf.raw.is_empty(), "{head:?}");
            assert_eq!(
                buf.replies,
                [TermReply::XtVersion("xterm(390)".to_string())],
                "{head:?}"
            );
        }
    }

    /// A typed Alt+P (`ESC P`) or `ESC P >` with nothing after it becomes
    /// keys only after one non-zero poll wait with no further stdin.
    #[test]
    fn lone_esc_p_is_alt_p_only_after_the_wait() {
        let mut buf = empty_tty_buf();
        buf.raw.extend_from_slice(b"\x1bP");
        assert!(take_ready_event(&mut buf, false).is_none(), "no wait yet");
        assert!(
            !arm_lone_esc_after_wait(&mut buf, Duration::ZERO),
            "a 0 ms drain is not the wait"
        );
        assert!(take_ready_event(&mut buf, false).is_none());
        assert!(arm_lone_esc_after_wait(&mut buf, Duration::from_millis(16)));
        let (event, origin, _) = take_ready_event(&mut buf, false).expect("Alt+P");
        assert_eq!(alt_key(&event), Some(KeyCode::Char('P')));
        assert_eq!(origin, KeyStrokeOrigin::Protocol);
        assert!(take_ready_event(&mut buf, false).is_none());
        assert!(buf.raw.is_empty() && buf.replies.is_empty());

        let mut buf = empty_tty_buf();
        buf.raw.extend_from_slice(b"\x1bP>");
        assert!(take_ready_event(&mut buf, false).is_none());
        assert!(arm_lone_esc_after_wait(&mut buf, Duration::from_millis(16)));
        let (event, _, _) = take_ready_event(&mut buf, false).expect("Alt+P");
        assert_eq!(alt_key(&event), Some(KeyCode::Char('P')));
        let (event, origin, _) = take_ready_event(&mut buf, false).expect(">");
        assert_eq!(key_code(&event), KeyCode::Char('>'));
        assert_eq!(origin, KeyStrokeOrigin::LegacyByte);
        assert!(buf.raw.is_empty());

        // More stdin queued: the wait does not finish the prefix.
        let mut buf = empty_tty_buf();
        buf.raw.extend_from_slice(b"\x1bP");
        assert!(arm_lone_esc_after_wait(&mut buf, Duration::from_millis(16)));
        assert!(take_ready_event(&mut buf, true).is_none());
    }

    #[test]
    fn esc_p_without_xtversion_intro_stays_alt_p() {
        let (events, replies) = parse_with_replies(b"\x1bPx");
        assert!(replies.is_empty());
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(alt_key(&events[0]), Some(KeyCode::Char('P')));
        assert_eq!(key_code(&events[1]), KeyCode::Char('x'));

        let (events, _) = parse_with_replies(b"\x1bp");
        assert_eq!(alt_key(&events[0]), Some(KeyCode::Char('p')), "Alt+p");

        // An unterminated reply past the cap reads as keys again.
        let mut long = b"\x1bP>|".to_vec();
        long.resize(STRING_REPLY_MAX + 1, b'x');
        let (events, replies) = parse_with_replies(&long);
        assert!(replies.is_empty());
        assert_eq!(alt_key(&events[0]), Some(KeyCode::Char('P')));
    }

    #[cfg(unix)]
    #[test]
    fn term_replies_in_keeps_replies_and_drops_keys_and_a_cut_tail() {
        assert_eq!(
            term_replies_in(b"j\x1b[?2004;1$y\x1b[A\x1b[?62c\x1b[?2026;"),
            [
                TermReply::ModeReport {
                    mode: 2004,
                    value: 1
                },
                TermReply::DeviceAttributes,
            ]
        );
        assert!(term_replies_in(b"jk\x1b[<64;1;1M").is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn take_term_replies_drains_the_live_reader_queue() {
        push_pending_input(b"\x1b[?2004;1$y\x1b[?62c");
        drain_parsed(&mut lock_tty_buf(), false);
        assert_eq!(
            take_term_replies(),
            [
                TermReply::ModeReport {
                    mode: 2004,
                    value: 1
                },
                TermReply::DeviceAttributes,
            ]
        );
        assert!(take_term_replies().is_empty(), "drained");
    }

    #[test]
    fn zero_byte_read_is_hangup() {
        let err = classify_read(0).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert_eq!(classify_read(4).unwrap(), 4);
    }

    #[cfg(unix)]
    #[test]
    fn poll_hup_without_pollin_is_hangup() {
        let err = classify_poll(1, libc::POLLHUP).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::UnexpectedEof);
        assert!(classify_poll(1, libc::POLLIN).unwrap());
        assert!(classify_poll(1, libc::POLLIN | libc::POLLHUP).unwrap());
        assert!(
            !classify_poll(0, libc::POLLHUP).unwrap(),
            "timeout (n=0) stays idle even if revents is leftover hangup"
        );
    }
}

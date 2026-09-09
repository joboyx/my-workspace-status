//! Shared TTY mouse enable sequence, mouse decode, and live byte reader.
//!
//! The live event loop reads with [`poll_event`] / [`read_event`]. On Unix
//! the reader tags each key as [`KeyStrokeOrigin::LegacyByte`] or
//! [`KeyStrokeOrigin::Protocol`] from the bytes. It decodes SGR, X10, and
//! rxvt 1015 mouse the same way crossterm 0.28 does. A lone ESC waits one
//! poll timeout with no further stdin before it becomes Escape, so a split
//! CSI / CSI-u report is not an Escape plus leftover keys. A hangup or
//! 0-byte read is a read error. Headless e2e cannot call those (no TTY),
//! so it feeds SGR bytes through [`decode_sgr_mouse`], which matches
//! crossterm's `parse_cb` / `parse_csi_sgr_mouse` including reports the
//! live reader drops. A kinder clone would go green while a real TTY
//! no-ops.

use std::collections::VecDeque;
use std::io::{self, Write};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

#[cfg(not(unix))]
use crossterm::event;
use crossterm::event::{
    DisableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton,
    MouseEvent, MouseEventKind,
};
use crossterm::execute;

use super::keys::KeyStrokeOrigin;

/// ANSI written to enable mouse capture.
///
/// xterm mouse *protocol* modes 1000 / 1002 / 1003 are mutually exclusive.
/// Crossterm's `EnableMouseCapture` sets all three (`1000h` `1002h` `1003h`);
/// the last SET wins (any-event). Resetting only `1003` can then leave
/// tracking off, so clicks, drag, and wheel die. This sequence resets 1003
/// first, then enables click + button-event tracking, rxvt 1015, and SGR
/// (`1006h` last). It never sets `1003h`. The Unix reader still decodes
/// X10 and 1015 when a terminal speaks those instead of SGR. Wheel reports
/// are `66`/`67` without the motion bit.
pub const MOUSE_ENABLE: &[u8] = b"\x1b[?1003l\x1b[?1000h\x1b[?1002h\x1b[?1015h\x1b[?1006h";

/// xterm SGR button for wheel right (trackpad hscroll).
pub(crate) const SGR_WHEEL_RIGHT: u8 = 67;
/// xterm SGR button for Shift+wheel down (common trackpad hscroll encoding).
pub(crate) const SGR_SHIFT_WHEEL_DOWN: u8 = 69;
/// Wheel right with the 1003 motion bit (`67 | 32`). crossterm 0.28 drops this.
pub(crate) const SGR_WHEEL_RIGHT_MOTION: u8 = 67 | 32;

/// Enable mouse capture for the live TTY. See [`MOUSE_ENABLE`].
pub fn enable_mouse(out: &mut impl Write) -> io::Result<()> {
    out.write_all(MOUSE_ENABLE)?;
    out.flush()
}

/// Disable mouse capture (crossterm inverse of every DECSET it turns on).
pub fn disable_mouse(out: &mut impl Write) -> io::Result<()> {
    execute!(out, DisableMouseCapture)
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
    read_event_origin().map(|(event, _)| event)
}

/// Read one TTY event and the origin of its bytes.
///
/// Unix tags raw printable / control bytes as
/// [`KeyStrokeOrigin::LegacyByte`]. CSI-u and other escape sequences
/// are [`KeyStrokeOrigin::Protocol`]. Windows keeps `event::read` and
/// tags every event as Protocol.
pub fn read_event_origin() -> io::Result<(Event, KeyStrokeOrigin)> {
    #[cfg(unix)]
    {
        unix_read()
    }
    #[cfg(not(unix))]
    {
        event::read().map(|event| (event, KeyStrokeOrigin::Protocol))
    }
}

struct TtyBuf {
    raw: Vec<u8>,
    events: VecDeque<(Event, KeyStrokeOrigin)>,
    last_winsize: Option<(u16, u16)>,
    /// Set after [`unix_poll`] waited one timeout with a lone ESC and no
    /// further stdin. [`unix_read`] emits Escape only when this is set.
    lone_esc_ready: bool,
}

fn empty_tty_buf() -> TtyBuf {
    TtyBuf {
        raw: Vec::new(),
        events: VecDeque::new(),
        last_winsize: None,
        lone_esc_ready: false,
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
fn unix_read() -> io::Result<(Event, KeyStrokeOrigin)> {
    let pending = stdin_pending()?;
    let mut buf = lock_tty_buf();
    if pending > 0 {
        let bytes = read_stdin(pending)?;
        buf.raw.extend_from_slice(&bytes);
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
    let rc = unsafe { libc::ioctl(libc::STDIN_FILENO, libc::FIONREAD, &mut n) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(n.max(0) as usize)
}

#[cfg(unix)]
fn poll_stdin(timeout: Duration) -> io::Result<bool> {
    let mut pfd = libc::pollfd {
        fd: libc::STDIN_FILENO,
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = i32::try_from(timeout.as_millis()).unwrap_or(i32::MAX);
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
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
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
                buf.events
                    .push_back((Event::Resize(size.0, size.1), KeyStrokeOrigin::Protocol));
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

fn take_ready_event(buf: &mut TtyBuf, more_stdin: bool) -> Option<(Event, KeyStrokeOrigin)> {
    drain_parsed(buf, more_stdin);
    if let Some(event) = buf.events.pop_front() {
        buf.lone_esc_ready = false;
        return Some(event);
    }
    if buf.lone_esc_ready && can_finish_lone_esc(buf) && !more_stdin {
        buf.raw.clear();
        buf.lone_esc_ready = false;
        return Some((
            Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE)),
            KeyStrokeOrigin::Protocol,
        ));
    }
    None
}

fn can_finish_lone_esc(buf: &TtyBuf) -> bool {
    buf.raw == [0x1b] && buf.events.is_empty()
}

/// Arm Escape only after a non-zero poll wait with no further stdin.
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
                buf.events.push_back((event, origin));
            }
        }
    }
}

enum Take {
    NeedMore,
    Skip(usize),
    Ready(usize, Event, KeyStrokeOrigin),
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
        },
    }
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

fn take_csi_query(bytes: &[u8]) -> Take {
    match csi_final_index(bytes) {
        Some(end) => Take::Skip(end + 1),
        None => Take::NeedMore,
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
    if modifiers.contains(KeyModifiers::SHIFT) {
        if let Some(shifted) = codes
            .next()
            .and_then(|field| field.parse::<u32>().ok())
            .and_then(char::from_u32)
        {
            code = KeyCode::Char(shifted);
            modifiers.remove(KeyModifiers::SHIFT);
        }
    }
    key_protocol(bytes.len(), KeyEvent::new_with_kind(code, modifiers, kind))
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
    buf.events.into_iter().collect()
}

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
/// This is a byte-accurate clone of crossterm 0.28 `parse_csi_sgr_mouse` /
/// `parse_cb`. Wheel left/right are buttons 6/7 (`Cb` 66/67). Shift+wheel is
/// the vertical wheel plus bit 2 (`Cb` 68/69). Bit 5 is motion; crossterm
/// 0.28 returns a parse error for wheel reports that include it (`98`/`99`),
/// and the live `event::read` loop drops those bytes. Unknown reports are
/// `None` so Headless e2e no-ops the same way.
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

/// Decode the SGR `Cb` field the way crossterm 0.28 `parse_cb` does.
///
/// Match arms are the live reader contract. Do not accept motion+wheel
/// (`(6, true)` / `(7, true)`): that is kinder than `event::read`.
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
        (3, true) | (4, true) | (5, true) => MouseEventKind::Moved,
        (4, false) => MouseEventKind::ScrollUp,
        (5, false) => MouseEventKind::ScrollDown,
        (6, false) => MouseEventKind::ScrollLeft,
        (7, false) => MouseEventKind::ScrollRight,
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
    use crossterm::event::EnableMouseCapture;
    use crossterm::Command;

    #[test]
    fn mouse_enable_never_sets_any_event_tracking() {
        let mut crossterm_enable = String::new();
        EnableMouseCapture
            .write_ansi(&mut crossterm_enable)
            .unwrap();
        assert!(
            crossterm_enable.contains("1003h"),
            "crossterm EnableMouseCapture still sets DECSET 1003: {crossterm_enable:?}"
        );
        let ours = std::str::from_utf8(MOUSE_ENABLE).unwrap();
        assert!(
            !ours.contains("1003h"),
            "live enable must not set any-event tracking: {ours:?}"
        );
        assert!(ours.contains("1003l"));
        assert!(ours.contains("1000h"));
        assert!(ours.contains("1002h"));
        assert!(ours.contains("1015h"));
        assert!(ours.contains("1006h"));
        assert!(
            ours.find("1003l").unwrap() < ours.find("1002h").unwrap(),
            "reset 1003 before enabling 1002 so exclusive-level terminals keep tracking: {ours:?}"
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
        assert!(
            decode_sgr_mouse(&sgr_mouse_report(SGR_WHEEL_RIGHT_MOTION, 8, 4)).is_none(),
            "crossterm 0.28 event::read drops SGR 99 (wheel right + motion); e2e must not pan on it"
        );
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
        let (event, origin) = take_ready_event(&mut buf, false).expect("idle timeout");
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
        let (event, origin) = take_ready_event(&mut arrow, false).expect("Up");
        assert_eq!(key_code(&event), KeyCode::Up);
        assert_eq!(origin, KeyStrokeOrigin::Protocol);
        assert!(take_ready_event(&mut arrow, false).is_none());

        let mut csi_u = empty_tty_buf();
        csi_u.raw.extend_from_slice(&[0x1b]);
        assert!(take_ready_event(&mut csi_u, false).is_none());
        csi_u.raw.extend_from_slice(b"[103;1u");
        let (event, origin) = take_ready_event(&mut csi_u, false).expect("g");
        assert_eq!(key_code(&event), KeyCode::Char('g'));
        assert_eq!(origin, KeyStrokeOrigin::Protocol);
        assert!(take_ready_event(&mut csi_u, false).is_none());
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

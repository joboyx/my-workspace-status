//! Shared TTY mouse enable sequence and SGR decode used by the live loop.
//!
//! The live event loop reads with [`poll_event`] / [`read_event`] (crossterm
//! 0.28 `event::poll` / `event::read`). Headless e2e cannot call those (no
//! TTY), so it feeds the same bytes through [`decode_sgr_mouse`], which matches
//! crossterm's `parse_cb` / `parse_csi_sgr_mouse` including reports the live
//! reader drops. A kinder clone would go green while a real TTY no-ops.

use std::io::{self, Write};
use std::time::Duration;

use crossterm::event::{
    self, DisableMouseCapture, Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use crossterm::execute;

/// ANSI written to enable mouse capture.
///
/// xterm mouse *protocol* modes 1000 / 1002 / 1003 are mutually exclusive.
/// Crossterm's `EnableMouseCapture` sets all three (`1000h` `1002h` `1003h`);
/// the last SET wins (any-event). Resetting only `1003` can then leave
/// tracking off, so clicks, drag, and wheel die. This sequence resets 1003
/// first, then enables click + button-event tracking and SGR encoding. It
/// never sets `1003h`. Wheel reports are `66`/`67` without the motion bit.
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
    event::poll(timeout)
}

/// Read one TTY event. Live loop only; crossterm 0.28 `event::read`.
pub fn read_event() -> io::Result<Event> {
    event::read()
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
}

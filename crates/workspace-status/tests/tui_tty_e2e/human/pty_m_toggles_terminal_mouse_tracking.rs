use vt100::{MouseProtocolEncoding, MouseProtocolMode};

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// Gap after a nav letter typed into the palette filter. A same-letter burst
/// is dropped by `discard_held_nav_backlog` (see `pty_workspace_command_palette`).
const PALETTE_NAV_LETTER_GAP_MS: u64 = 50;

fn wait_tracking(tui: &PtySession, mode: MouseProtocolMode, what: &str) {
    tui.wait_pred(
        |_| {
            let (now, encoding) = tui.mouse_tracking();
            now == mode
                && (mode == MouseProtocolMode::None || encoding == MouseProtocolEncoding::Sgr)
        },
        what,
        WAIT,
    );
}

/// Tree `m` and the palette `Toggle mouse` entry turn terminal mouse
/// tracking off and on, not only the app's own mouse gate.
///
/// The terminal side is the `vt100` state built from the child's DECSET /
/// DECRST output. Launch must hold any-event tracking with SGR encoding.
/// `m` (Mouse off) must reset it to none, so the terminal stops sending
/// motion reports. The second `m` restores any-event tracking. The palette
/// entry must do the same. An app-only flag flip leaves the mode on.
#[test]
fn pty_m_toggles_terminal_mouse_tracking() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    wait_tracking(
        &tui,
        MouseProtocolMode::AnyMotion,
        "launch enables any-event mouse tracking with SGR encoding",
    );
    assert_eq!(
        tui.mouse_tracking(),
        (MouseProtocolMode::AnyMotion, MouseProtocolEncoding::Sgr)
    );

    tui.key('m');
    tui.wait_pred(
        |screen| screen.contains("Mouse off"),
        "`m` paints Mouse off",
        WAIT,
    );
    wait_tracking(
        &tui,
        MouseProtocolMode::None,
        "`m` disables terminal mouse tracking (an app-only flag leaves any-event motion on)",
    );

    tui.key('m');
    tui.wait_pred(
        |screen| screen.contains("Mouse on"),
        "second `m` paints Mouse on",
        WAIT,
    );
    wait_tracking(
        &tui,
        MouseProtocolMode::AnyMotion,
        "second `m` re-enables any-event tracking with SGR encoding",
    );

    // Palette path: `:`, filter, Enter.
    for (want_paint, want_mode) in [
        ("Mouse off", MouseProtocolMode::None),
        ("Mouse on", MouseProtocolMode::AnyMotion),
    ] {
        tui.key(':');
        tui.wait_pred(
            |screen| screen.contains("Enter run"),
            "`:` opens Quick Open commands",
            WAIT,
        );
        for c in "toggle mouse".chars() {
            tui.key(c);
            if matches!(c, 'l' | 'k' | 'j' | 'h') {
                tui.wait_ms(PALETTE_NAV_LETTER_GAP_MS);
            }
        }
        tui.wait_pred(
            |screen| screen.contains("Enter run") && screen.contains("Toggle mouse"),
            "palette filter shows `Toggle mouse`",
            WAIT,
        );
        tui.enter();
        tui.wait_pred(
            |screen| screen.contains(want_paint) && !screen.contains("Enter run"),
            &format!("palette Toggle mouse paints {want_paint}"),
            WAIT,
        );
        wait_tracking(
            &tui,
            want_mode,
            &format!("palette Toggle mouse leaves tracking {want_mode:?}"),
        );
    }
}

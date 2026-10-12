//! `themeMode`: the startup OSC 11 background query picks the light or dark
//! twin of the launch theme.

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::WAIT;

/// OSC 11 background query the TUI writes in `themeMode` `auto`.
const OSC11_QUERY: &[u8] = b"\x1b]11;?";

/// Heading and mode-pill colours of one theme. Each is unique across the
/// built-ins.
struct ThemeChrome {
    id: &'static str,
    heading: (u8, u8, u8),
    pill: (u8, u8, u8),
}

/// Baseline launch theme (harness user config).
const TOKYO_NIGHT: ThemeChrome = ThemeChrome {
    id: "tokyo-night",
    heading: (0x7d, 0xcf, 0xff),
    pill: (0x3d, 0x59, 0xa1),
};

/// Light twin of [`TOKYO_NIGHT`].
const TOKYO_NIGHT_DAY: ThemeChrome = ThemeChrome {
    id: "tokyo-night-day",
    heading: (0x00, 0x63, 0x85),
    pill: (0x13, 0x59, 0xb8),
};

/// Wait until `active` paints its heading and pill, then fail if `other`
/// paints either of its colours.
fn wait_theme(tui: &PtySession, active: &ThemeChrome, other: &ThemeChrome) {
    tui.wait_contains(" tree", WAIT);
    for (r, g, b) in [active.heading, active.pill] {
        tui.wait_has_rgb(r, g, b, WAIT);
    }
    for (r, g, b) in [other.heading, other.pill] {
        assert!(
            !tui.has_rgb(r, g, b),
            "{} colour rgb({r},{g},{b}) must not paint when {} is active:\n{}",
            other.id,
            active.id,
            tui.screen()
        );
    }
}

fn wrote_query(tui: &PtySession) -> bool {
    tui.raw_output()
        .windows(OSC11_QUERY.len())
        .any(|w| w == OSC11_QUERY)
}

/// `auto` on a white terminal: the baseline Tokyo Night launches as its
/// light twin, Tokyo Night Day. Fail if the TUI never asks OSC 11 or ignores
/// the reply.
#[test]
fn pty_auto_theme_mode_picks_light_twin_from_osc11_reply() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open_with_bg_reply(
        &workspace,
        &[("WS_STATUS_THEME_MODE", "auto")],
        "ffff/ffff/ffff",
    );
    wait_theme(&tui, &TOKYO_NIGHT_DAY, &TOKYO_NIGHT);
    assert!(
        wrote_query(&tui),
        "auto asks the terminal for its background"
    );
}

/// The harness pins `themeMode` `dark`: no query, and a terminal that
/// would answer white does not change the theme.
#[test]
fn pty_dark_theme_mode_ignores_light_reply() {
    let (_root, workspace) = daily_workspace();
    let tui = PtySession::open_with_bg_reply(&workspace, &[], "ffff/ffff/ffff");
    wait_theme(&tui, &TOKYO_NIGHT, &TOKYO_NIGHT_DAY);
    assert!(!wrote_query(&tui), "dark mode never queries the terminal");
}

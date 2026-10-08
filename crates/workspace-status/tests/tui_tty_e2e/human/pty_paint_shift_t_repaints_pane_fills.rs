use crate::harness::{PtySession, UserConfig, COLS, ROWS};
use crate::seed::daily_workspace;
use crate::support::{
    crumb_row, flat_join_col, flat_pane_last_row, FLAT_TITLE_ROW, SLATE_CHROME, SLATE_SIDEBAR,
    SLATE_SURFACE, WAIT,
};

/// Solarized Dark, the theme after Slate in the `T` cycle. It sets no
/// sidebar / chrome, so those follow the default rule: `surface` mixed 22%
/// (sidebar) and 38% (chrome) toward black.
const SOLARIZED_SURFACE: (u8, u8, u8) = (0x00, 0x2b, 0x36);
const SOLARIZED_SIDEBAR: (u8, u8, u8) = (0x00, 0x22, 0x2a);
const SOLARIZED_CHROME: (u8, u8, u8) = (0x00, 0x1b, 0x21);

fn bg_at(tui: &PtySession, row: u16, col: u16) -> Option<(u8, u8, u8)> {
    tui.cell_paint(row, col).and_then(|cell| cell.bg)
}

/// Left fill, right fill, and chrome fill on a settled flat frame.
fn pane_fills(
    tui: &PtySession,
    sidebar: (u8, u8, u8),
    surface: (u8, u8, u8),
    chrome: (u8, u8, u8),
) -> bool {
    let Some(join) = flat_join_col(tui, sidebar) else {
        return false;
    };
    let last = flat_pane_last_row(tui);
    bg_at(tui, FLAT_TITLE_ROW, 0) == Some(sidebar)
        && bg_at(tui, last, 0) == Some(sidebar)
        && bg_at(tui, FLAT_TITLE_ROW, join) == Some(surface)
        && bg_at(tui, last, COLS - 1) == Some(surface)
        && bg_at(tui, 0, COLS - 1) == Some(chrome)
        && bg_at(tui, ROWS - 1, COLS - 1) == Some(chrome)
}

/// Shift+T in paint mode repaints the pane fills with the next theme.
///
/// Docs: `T` cycles the colour theme; in paint mode each theme fills the
/// left pane with `sidebar`, the right pane with `surface`, and the bars
/// with `chrome`.
///
/// Live PTY on the shipped look (no user config, Slate): CSI-u Shift+T
/// toasts `theme: Solarized Dark`, and the left, right and chrome cells take
/// Solarized's fills. No Slate fill remains. A toast-only no-op, or a theme
/// that changes text but keeps the pane fills, cannot pass.
#[test]
fn pty_paint_shift_t_repaints_pane_fills() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size_with_config(&workspace, COLS, ROWS, &[], UserConfig::Own);
    tui.wait_pred(
        |screen| {
            screen.contains("+dirty")
                && crumb_row(screen).contains("workspace › app")
                && pane_fills(&tui, SLATE_SIDEBAR, SLATE_SURFACE, SLATE_CHROME)
        },
        "first paint: Slate fills on the flat panes and bars",
        WAIT,
    );

    tui.shift_letter('T');
    tui.wait_pred(
        |screen| {
            screen.contains("theme: Solarized Dark")
                && pane_fills(&tui, SOLARIZED_SIDEBAR, SOLARIZED_SURFACE, SOLARIZED_CHROME)
        },
        "Shift+T: Solarized Dark toast and Solarized sidebar / surface / chrome fills",
        WAIT,
    );
    for (r, g, b) in [SLATE_SIDEBAR, SLATE_SURFACE, SLATE_CHROME] {
        assert!(
            !tui.has_rgb(r, g, b),
            "Slate fill rgb({r},{g},{b}) must not remain after Shift+T:\n{}",
            tui.screen()
        );
    }
}

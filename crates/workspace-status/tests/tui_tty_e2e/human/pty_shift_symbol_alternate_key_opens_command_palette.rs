use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{tree_cursor_on, WAIT};

/// Kitty keyboard flags the TUI pushes: disambiguate (1), event types (2),
/// alternate keys (4), all keys as escape codes (8).
const PUSH_FLAGS: &[u8] = b"\x1b[>15u";

/// Shift+`;` on a kitty-protocol terminal is `:`, the command palette.
///
/// The TUI asks for the shifted-key field (`REPORT_ALTERNATE_KEYS`), so the
/// terminal reports `CSI 59:58;2u` and the key acts as `:`. Without that
/// flag the report is `;` + Shift and `:` acts as `;`.
#[test]
fn pty_shift_symbol_alternate_key_opens_command_palette() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| screen.contains("? help") && tree_cursor_on(screen, "README.md"),
        "idle tree with the cursor on README.md",
        WAIT,
    );
    let raw = tui.raw_output();
    assert!(
        raw.windows(PUSH_FLAGS.len()).any(|w| w == PUSH_FLAGS),
        "the TUI must push kitty flags 15 (with REPORT_ALTERNATE_KEYS)"
    );

    tui.shift_symbol(';', ':');
    tui.keys("sparkline");
    tui.wait_contains("Toggle activity sparkline", WAIT);
}

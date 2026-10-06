use crate::harness::{tree_row_containing, PtySession};
use crate::seed::behind_workspace;
use crate::support::{crumb_row, syncbox_row_behind, GIT_WAIT, SETTLE_MS, WAIT};

/// SGR pointer motion with no button held (`3 | 32`, any-event tracking).
const SGR_POINTER_MOVE: u8 = 3 | 32;
/// Catalog meaning of the behind mark: the first line of its popover.
const BEHIND_MEANING: &str = "Upstream commits not pulled";
/// Field line of a one-commit behind mark.
const BEHIND_COUNT: &str = "1 commit to pull";
/// Footer of a pinned popover. A peek has none.
const PINNED_FOOTER: &str = "Esc close";

/// 0-based cell of the `v1` behind mark on the syncbox tree row.
fn behind_mark_cell(screen: &str) -> Option<(u16, u16)> {
    let row = tree_row_containing(screen, "syncbox")?;
    let line = screen.lines().nth(usize::from(row))?;
    let name = line.find("syncbox")?;
    let at = name + line[name..].find("v1")?;
    // ASCII glyph mode: one column per char.
    Some((line[..at].chars().count() as u16, row))
}

fn peek_shown(screen: &str) -> bool {
    screen.contains(BEHIND_MEANING)
        && screen.contains(BEHIND_COUNT)
        && screen.contains("Pull behind")
        && !screen.contains(PINNED_FOOTER)
}

fn pinned(screen: &str) -> bool {
    screen.contains(BEHIND_MEANING)
        && screen.contains(BEHIND_COUNT)
        && screen.contains("y copy line · Enter run · Esc close")
}

fn closed(screen: &str) -> bool {
    !screen.contains(BEHIND_MEANING) && !screen.contains(PINNED_FOOTER)
}

/// Hovering the tree behind mark peeks; a click pins; `j` / Enter run the
/// focused action; `gh` pins again; Esc closes.
///
/// Docs: an icon peek opens after the pointer rests on it and takes no
/// keys. A click on the icon selects its row and pins the popover: `j` / `k`
/// move, Enter runs the focused action, Esc closes. `gh` pins the popover of
/// every icon on the focused row.
///
/// Live PTY on a fetched, one-commit-behind `syncbox`: SGR motion onto
/// `v1` paints the peek (meaning, `1 commit to pull`, `Pull behind`, no
/// footer). A click pins it (footer). `j` moves to Fetch remotes and Enter
/// runs it: the popover closes and the breadcrumb says `Fetched 1 repo`.
/// `gh` pins it again; Esc closes it and the row stays behind.
#[test]
fn pty_icon_popover_peek_pin_esc() {
    let (_root, workspace) = behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(syncbox_row_behind, "syncbox row shows v1", WAIT);
    let (col, row) = behind_mark_cell(&tui.screen())
        .unwrap_or_else(|| panic!("behind mark cell:\n{}", tui.screen()));

    tui.sgr_mouse(SGR_POINTER_MOVE, col, row);
    tui.wait_pred(peek_shown, "pointer rest on v1 opens the peek", WAIT);

    tui.sgr_click(col, row);
    tui.wait_pred(pinned, "a click on v1 pins the popover", WAIT);
    // Move the pointer off: a pinned popover has no leave grace.
    tui.sgr_mouse(SGR_POINTER_MOVE, 2, row);
    tui.wait_ms(SETTLE_MS * 3);
    assert!(pinned(&tui.screen()), "pinned stays:\n{}", tui.screen());

    tui.key('j');
    tui.wait_pred(
        |screen| screen.contains("❯ Fetch remotes"),
        "j focuses Fetch remotes",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| closed(screen) && crumb_row(screen).contains("Fetched 1 repo"),
        "Enter closes the popover and runs fetch",
        GIT_WAIT,
    );

    tui.keys("gh");
    tui.wait_pred(pinned, "gh pins the focused row's popover", WAIT);
    tui.esc();
    tui.wait_pred(closed, "Esc closes the pinned popover", WAIT);
    tui.wait_pred(syncbox_row_behind, "the row is still behind", WAIT);
}

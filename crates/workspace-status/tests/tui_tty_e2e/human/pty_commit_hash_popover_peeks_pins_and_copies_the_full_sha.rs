use crate::harness::PtySession;
use crate::seed::{daily_workspace, git_stdout};
use crate::support::{idle_dirty_readme_unreviewed, merger_graph_left_unfocused, GIT_WAIT, WAIT};

/// SGR pointer motion with no button held (`3 | 32`, any-event tracking).
const SGR_POINTER_MOVE: u8 = 3 | 32;
/// Catalog meaning of the commit hash: the first line of the hash card.
const COMMIT_MEANING: &str = "Commit: full id, message, parents, refs, and dates";
/// Footer of a pinned popover. A peek has none.
const PINNED_FOOTER: &str = "y copy line · Enter run · Esc close";

/// 0-based cell of the first painted `short` on screen.
fn short_hash_cell(screen: &str, short: &str) -> Option<(u16, u16)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        let at = line.find(short)?;
        // ASCII glyph mode: one column per char.
        Some((line[..at].chars().count() as u16, row as u16))
    })
}

/// A graph commit's short hash peeks, pins, and copies its full id.
///
/// Docs: every painted commit short hash is a commit popover hit. The
/// pointer resting on it peeks the hash card (full sha first, then the
/// subject, parents, refs, author, and both dates); a click pins it, `y`
/// copies the focused field (the full sha), and Esc closes it.
///
/// Live PTY on the daily seed, ASCII glyphs: `j` loads the `merger` graph.
/// Pointer rest on the merge commit's short hash on its graph row peeks
/// the card: the full 40-char id and `authored`, no pinned footer. A click
/// pins it (footer). `y` copies the full id (OSC 52). Esc closes it. A
/// card that shows only the short id, or a copy of the short id, cannot
/// pass.
#[test]
fn pty_commit_hash_popover_peeks_pins_and_copies_the_full_sha() {
    let (_root, workspace) = daily_workspace();
    let sha = git_stdout(&workspace.join("merger"), &["rev-parse", "HEAD"]);
    assert_eq!(sha.len(), 40, "full sha: {sha}");
    let short = sha[..7].to_string();
    let card = |screen: &str| {
        screen.contains(COMMIT_MEANING) && screen.contains(&sha) && screen.contains("authored")
    };
    let peek = |screen: &str| card(screen) && !screen.contains(PINNED_FOOTER);
    let pinned = |screen: &str| card(screen) && screen.contains(PINNED_FOOTER);
    let closed = |screen: &str| !screen.contains(COMMIT_MEANING) && !screen.contains(PINNED_FOOTER);

    let mut tui = PtySession::open_size(&workspace, 120, 40);
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "first paint: cursor on the dirty README",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| merger_graph_left_unfocused(screen) && screen.contains(&short),
        "j loads the merger graph with the merge commit's short hash",
        GIT_WAIT,
    );
    let (col, row) = short_hash_cell(&tui.screen(), &short)
        .unwrap_or_else(|| panic!("short hash cell:\n{}", tui.screen()));

    tui.sgr_mouse(SGR_POINTER_MOVE, col, row);
    tui.wait_pred(peek, "pointer rest on the short hash peeks the card", WAIT);

    tui.sgr_click(col, row);
    tui.wait_pred(pinned, "a click on the short hash pins the card", WAIT);

    tui.key('y');
    tui.wait_clipboard_pred(
        |payloads| payloads.last() == Some(&sha),
        "y copies the full sha",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(closed, "Esc closes the card", WAIT);
}

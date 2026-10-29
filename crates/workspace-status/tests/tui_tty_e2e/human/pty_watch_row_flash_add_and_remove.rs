use std::fs;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, merger_graph_left_unfocused, tree_cursor_on, tree_has,
    tree_line_containing, GIT_WAIT, WAIT,
};

/// Tokyo Night add flash ramp, peak first (`flash_ramp`). The whole ramp
/// counts: every recorded frame is checked, so a flash that starts late
/// (at a lower step) is still found.
const ADD_RAMP: [(u8, u8, u8); 4] = [
    (0x51, 0x66, 0x43),
    (0x3f, 0x4d, 0x39),
    (0x2f, 0x38, 0x31),
    (0x25, 0x29, 0x2b),
];
/// Tokyo Night update flash ramp, peak first (`flash_update_ramp`).
const UPDATE_RAMP: [(u8, u8, u8); 4] = [
    (0x6d, 0x59, 0x42),
    (0x51, 0x44, 0x38),
    (0x3a, 0x33, 0x31),
    (0x2a, 0x27, 0x2b),
];
/// Tokyo Night remove flash ramp, peak first (`flash_remove_ramp`).
const REMOVE_RAMP: [(u8, u8, u8); 4] = [
    (0x77, 0x41, 0x52),
    (0x58, 0x34, 0x43),
    (0x3d, 0x2a, 0x37),
    (0x2c, 0x22, 0x2e),
];

/// ASCII file glyph plus name. Unique to the tree README row (`app/README.md`
/// in the diff header and the diff-pane `▌` are other first-match hits).
const TREE_README: &str = "· README.md";

/// Tokyo Night `cursor_bg` `#283457`. Flash must not leave this on the row.
const CURSOR_BG: (u8, u8, u8) = (0x28, 0x34, 0x57);

fn needle_on_ramp(tui: &PtySession, needle: &str, ramp: &[(u8, u8, u8)]) -> bool {
    ramp.iter()
        .any(|&(r, g, b)| tui.needle_has_bg(needle, r, g, b))
}

fn tree_shows_untracked(screen: &str, name: &str) -> bool {
    tree_line_containing(screen, name).is_some_and(|line| line.contains("A "))
}

/// Live watch paints add / update / remove 24-bit flash backgrounds.
///
/// Glyphs on the tree are not enough: a row that appears or vanishes with
/// only the default / surface / cursor background must fail. Add and remove
/// run on unfocused `app/` files after `j` to merger. Update runs first on
/// the focused README so flash wins over `cursor_bg`. Remove keeps a ghost
/// with the remove flash, then drops it. How long each lasts is a timer,
/// pinned by `flash_strength_ramps_to_zero` and
/// `a_ghost_lives_for_the_flash_then_drops` (`tui::watch` unit tests).
///
/// Each flash claim reads the frame record started before the disk write
/// ([`PtySession::wait_recorded_frame`]), so a flash that is over by the
/// time this test thread runs again is still found.
#[test]
fn pty_watch_row_flash_add_and_remove() {
    let (_root, workspace) = daily_workspace();
    let marker = format!(
        "rowflash-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let app = workspace.join("app");
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_WATCH_MS", "500"),
            ("WS_STATUS_THEME", "tokyo-night"),
        ],
    );
    tui.wait_pred(
        documented_launch_first_paint,
        "first paint: dirty README focused, watch on, Tokyo Night",
        WAIT,
    );
    tui.wait_pred(
        |screen| !tree_has(screen, &marker),
        "new dirty path is absent before the disk write",
        WAIT,
    );

    let readme = app.join("README.md");
    let mut body = fs::read_to_string(&readme).unwrap();
    body.push_str("flash-update\n");
    tui.record_frames();
    fs::write(&readme, body).unwrap();
    // Flash over the cursor: README stays focused, its row is on the
    // update ramp, and cursor_bg does not hide it.
    tui.wait_recorded_frame(
        |tui| {
            tree_cursor_on(&tui.screen(), "README.md")
                && needle_on_ramp(tui, TREE_README, &UPDATE_RAMP)
                && !tui.needle_has_bg(TREE_README, CURSOR_BG.0, CURSOR_BG.1, CURSOR_BG.2)
        },
        "focused README tree row uses update flash bg (not cursor_bg)",
        GIT_WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger so add/remove rows are not focused",
        GIT_WAIT,
    );

    tui.record_frames();
    fs::write(app.join(&marker), "live-add-flash\n").unwrap();
    tui.wait_recorded_frame(
        |tui| {
            tree_shows_untracked(&tui.screen(), &marker)
                && needle_on_ramp(tui, &marker, &ADD_RAMP)
                && !needle_on_ramp(tui, &marker, &REMOVE_RAMP)
        },
        "watch paints the new untracked path (A) with add flash bg, not remove (glyphs-only is red)",
        GIT_WAIT,
    );

    // The flash ends on its own timer; wait for that paint. Its length is
    // pinned by the `tui::watch` unit tests.
    tui.wait_pred(
        |screen| tree_shows_untracked(screen, &marker) && !needle_on_ramp(&tui, &marker, &ADD_RAMP),
        "add flash expired; untracked path stays on the tree",
        WAIT,
    );

    tui.record_frames();
    fs::remove_file(app.join(&marker)).unwrap();
    tui.wait_recorded_frame(
        |tui| tree_has(&tui.screen(), &marker) && needle_on_ramp(tui, &marker, &REMOVE_RAMP),
        "removed path stays as a ghost with remove flash bg (instant vanish is red)",
        GIT_WAIT,
    );
    // The ghost with the remove flash painted above; how long it stays is
    // a timer, pinned by the `tui::watch` unit tests. Here: it shows, then
    // the row goes away on its own.
    tui.wait_pred(
        |screen| !tree_has(screen, &marker),
        "ghost dropped after the flash",
        WAIT,
    );
}

use std::fs;
use std::thread;
use std::time::{Duration, Instant};

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, merger_graph_left_unfocused, tree_cursor_on, tree_has,
    tree_line_containing, GIT_WAIT, WAIT,
};

/// Matches `watch::FLASH_MS`. Remove ghosts drop when this window ends.
const FLASH_MS: u64 = 800;
/// Extra wait after `FLASH_MS` before the ghost must be gone.
const FLASH_GONE_SLACK_MS: u64 = 200;

/// Tokyo Night add ramp[0] `#516643`.
const ADD_PEAK: (u8, u8, u8) = (0x51, 0x66, 0x43);
/// Tokyo Night add ramp[1] `#3f4d39`. Peak is only the first ~200ms.
const ADD_STEP1: (u8, u8, u8) = (0x3f, 0x4d, 0x39);
/// Tokyo Night update ramp[0] `#6d5942`.
const UPDATE_PEAK: (u8, u8, u8) = (0x6d, 0x59, 0x42);
/// Tokyo Night update ramp[1] `#514438`.
const UPDATE_STEP1: (u8, u8, u8) = (0x51, 0x44, 0x38);
/// Tokyo Night remove ramp[0] `#774152`.
const REMOVE_PEAK: (u8, u8, u8) = (0x77, 0x41, 0x52);
/// Tokyo Night remove ramp[1] `#583443`.
const REMOVE_STEP1: (u8, u8, u8) = (0x58, 0x34, 0x43);

/// Peak plus next step. Watch is 500ms; ramp[0] lasts ~200ms (`strength > 0.75`).
const ADD_RAMP: [(u8, u8, u8); 2] = [ADD_PEAK, ADD_STEP1];
const UPDATE_RAMP: [(u8, u8, u8); 2] = [UPDATE_PEAK, UPDATE_STEP1];
const REMOVE_RAMP: [(u8, u8, u8); 2] = [REMOVE_PEAK, REMOVE_STEP1];

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

fn wait_needle_flash(
    tui: &PtySession,
    needle: &str,
    ramp: &[(u8, u8, u8)],
    what: &str,
    timeout: Duration,
) {
    let start = Instant::now();
    loop {
        let screen = tui.screen();
        if tree_has(&screen, needle) && needle_on_ramp(tui, needle, ramp) {
            return;
        }
        if start.elapsed() >= timeout {
            panic!(
                "timeout waiting for {what}; on_tree={} bgs={:?}; screen:\n{screen}",
                tree_has(&screen, needle),
                tui.first_needle_bgs(needle)
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn wait_cursor_flash(tui: &PtySession, ramp: &[(u8, u8, u8)], what: &str, timeout: Duration) {
    let start = Instant::now();
    let mut bgs_when_diff_applied: Option<Vec<Option<(u8, u8, u8)>>> = None;
    loop {
        let screen = tui.screen();
        if screen.contains("flash-update") && bgs_when_diff_applied.is_none() {
            bgs_when_diff_applied = tui.first_needle_bgs(TREE_README);
        }
        if tree_cursor_on(&screen, "README.md") && needle_on_ramp(tui, TREE_README, ramp) {
            return;
        }
        if start.elapsed() >= timeout {
            panic!(
                "timeout waiting for {what}; cursor_on_readme={} apply_bgs={bgs_when_diff_applied:?} now_bgs={:?}; screen:\n{screen}",
                tree_cursor_on(&screen, "README.md"),
                tui.first_needle_bgs(TREE_README)
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// Live watch paints add / update / remove 24-bit flash backgrounds.
///
/// Glyphs on the tree are not enough: a row that appears or vanishes with
/// only the default / surface / cursor background must fail. Add and remove
/// run on unfocused `app/` files after `j` to merger. Update runs first on
/// the focused README so flash wins over `cursor_bg`. Remove keeps a ghost
/// for `FLASH_MS`, then drops.
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
    fs::write(&readme, body).unwrap();
    wait_cursor_flash(
        &tui,
        &UPDATE_RAMP,
        "focused README tree row uses update flash bg (not cursor_bg)",
        GIT_WAIT,
    );
    assert!(
        tree_cursor_on(&tui.screen(), "README.md"),
        "update flash is paint-over-cursor; README must stay focused; screen:\n{}",
        tui.screen()
    );
    assert!(
        needle_on_ramp(&tui, TREE_README, &UPDATE_RAMP),
        "update flash must paint over the cursor; bgs={:?}; screen:\n{}",
        tui.first_needle_bgs(TREE_README),
        tui.screen()
    );
    assert!(
        !tui.needle_has_bg(TREE_README, CURSOR_BG.0, CURSOR_BG.1, CURSOR_BG.2),
        "cursor_bg #283457 must not hide the update flash; bgs={:?}; screen:\n{}",
        tui.first_needle_bgs(TREE_README),
        tui.screen()
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger so add/remove rows are not focused",
        GIT_WAIT,
    );

    fs::write(app.join(&marker), "live-add-flash\n").unwrap();
    wait_needle_flash(
        &tui,
        &marker,
        &ADD_RAMP,
        "watch paints the new dirty path with add flash bg (glyphs-only is red)",
        GIT_WAIT,
    );
    assert!(
        tree_shows_untracked(&tui.screen(), &marker),
        "added path must keep the untracked A badge; screen:\n{}",
        tui.screen()
    );
    assert!(
        needle_on_ramp(&tui, &marker, &ADD_RAMP),
        "add flash bg on `{marker}`; bgs={:?}; screen:\n{}",
        tui.first_needle_bgs(&marker),
        tui.screen()
    );
    assert!(
        !needle_on_ramp(&tui, &marker, &REMOVE_RAMP),
        "add must not use the remove ramp; bgs={:?}; screen:\n{}",
        tui.first_needle_bgs(&marker),
        tui.screen()
    );

    tui.wait_ms(FLASH_MS + FLASH_GONE_SLACK_MS);
    tui.wait_pred(
        |screen| tree_shows_untracked(screen, &marker) && !needle_on_ramp(&tui, &marker, &ADD_RAMP),
        "add flash expired; untracked path stays on the tree",
        WAIT,
    );

    fs::remove_file(app.join(&marker)).unwrap();
    wait_needle_flash(
        &tui,
        &marker,
        &REMOVE_RAMP,
        "removed path stays as a ghost with remove flash bg (instant vanish is red)",
        GIT_WAIT,
    );
    let remove_seen = Instant::now();
    assert!(
        tree_has(&tui.screen(), &marker),
        "ghost must remain during FLASH_MS; screen:\n{}",
        tui.screen()
    );
    assert!(
        needle_on_ramp(&tui, &marker, &REMOVE_RAMP),
        "remove flash bg on `{marker}`; bgs={:?}; screen:\n{}",
        tui.first_needle_bgs(&marker),
        tui.screen()
    );

    tui.wait_ms(200);
    assert!(
        tree_has(&tui.screen(), &marker) && needle_on_ramp(&tui, &marker, &REMOVE_RAMP),
        "ghost + remove flash still painted 200ms in; bgs={:?}; screen:\n{}",
        tui.first_needle_bgs(&marker),
        tui.screen()
    );

    let elapsed_ms = remove_seen.elapsed().as_millis() as u64;
    let remain = (FLASH_MS + FLASH_GONE_SLACK_MS).saturating_sub(elapsed_ms);
    tui.wait_ms(remain);
    tui.wait_pred(
        |screen| !tree_has(screen, &marker),
        "ghost dropped after FLASH_MS + slack",
        WAIT,
    );
}

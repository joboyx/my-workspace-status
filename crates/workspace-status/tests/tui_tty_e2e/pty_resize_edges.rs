//! Resize-edge hover highlight, OSC 22 pointer shape, and the saved
//! commit-message footer height, through real SGR mouse reports.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::thread;
use std::time::{Duration, Instant};

use crate::harness::{
    tree_row_containing, user_config_file, PtySession, BASELINE_USER_CONFIG, COLS,
};
use crate::seed::{daily_workspace, seed_multiline_message_repo};
use crate::support::{
    crumb_row, documented_launch_first_paint, graph_cursor_on, no_mouse_toggle_toast,
    no_wrong_overlays, panes_tree_unfocused_graph_focused, right_of_split, tree_cursor_on,
    RIGHT_PANE_COL, WAIT,
};

/// xterm SGR left-button drag (`Cb` 0 + motion bit 32).
const SGR_LEFT_DRAG: u8 = 32;
/// xterm SGR motion with no button held (`Cb` 3 + motion bit 32). The TUI
/// asks for any-event tracking (1003), so a terminal sends this on hover.
const SGR_MOTION: u8 = 35;

/// Tokyo Night (the harness baseline theme) `cursor`: the focus accent the
/// hovered edge takes.
const ACCENT: (u8, u8, u8) = (0x7a, 0xa2, 0xf7);

const OSC_22: &[u8] = b"\x1b]22;";
const EW_RESIZE: &[u8] = b"\x1b]22;ew-resize\x1b\\";
const NS_RESIZE: &[u8] = b"\x1b]22;ns-resize\x1b\\";
const POINTER_DEFAULT: &[u8] = b"\x1b]22;default\x1b\\";
const ALT_SCREEN_ON: &[u8] = b"\x1b[?1049h";
const ALT_SCREEN_OFF: &[u8] = b"\x1b[?1049l";

/// Repo with a commit message taller than the footer.
const REPO: &str = "longmsg";
/// Rows the commit-message drag adds.
const DRAG_ROWS: u16 = 3;
/// `viewDefaults.commitMessageLines` default.
const DEFAULT_MSG_LINES: u16 = 4;

/// Byte offsets of every `needle` in `hay`.
fn positions(hay: &[u8], needle: &[u8]) -> Vec<usize> {
    hay.windows(needle.len())
        .enumerate()
        .filter_map(|(i, w)| (w == needle).then_some(i))
        .collect()
}

/// First offset of `needle` at or after `from`.
fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    positions(hay, needle).into_iter().find(|at| *at >= from)
}

/// Every OSC 22 payload (`ew-resize`, `default`, ...) in `raw`, in order.
fn pointer_shapes(raw: &[u8]) -> Vec<String> {
    positions(raw, OSC_22)
        .into_iter()
        .filter_map(|at| {
            let body = &raw[at + OSC_22.len()..];
            let end = body.windows(2).position(|w| w == b"\x1b\\")?;
            Some(String::from_utf8_lossy(&body[..end]).into_owned())
        })
        .collect()
}

/// Wait until `pred` holds for the raw PTY output stream.
fn wait_raw(tui: &PtySession, pred: impl Fn(&[u8]) -> bool, what: &str) {
    let start = Instant::now();
    loop {
        let raw = tui.raw_output();
        if pred(&raw) {
            return;
        }
        assert!(
            start.elapsed() < WAIT,
            "timeout waiting for {what}; OSC 22 shapes: {:?}; screen:\n{}",
            pointer_shapes(&raw),
            tui.screen()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

fn sgr_release(tui: &mut PtySession, col: u16, row: u16) {
    let seq = format!(
        "\x1b[<0;{};{}m",
        col.saturating_add(1),
        row.saturating_add(1)
    );
    tui.send_bytes(seq.as_bytes());
}

/// 0-based column of the tree pane's right border (`┐` of the `┐┌` join).
fn pane_join_col(screen: &str) -> Option<u16> {
    screen.lines().find_map(|line| {
        ["┐┌", "││"].iter().find_map(|sep| {
            line.find(sep)
                .map(|byte_at| line[..byte_at].chars().count() as u16)
        })
    })
}

/// True when the hovered-edge accent is the fg of `(col, row)`.
fn lit(tui: &PtySession, col: u16, row: u16) -> bool {
    tui.cell_paint(row, col)
        .is_some_and(|cell| cell.fg == Some(ACCENT))
}

/// 0-based screen row of the graph footer rule: a right-pane row that is
/// all `_` (ASCII glyphs) between the pane borders.
fn footer_rule_row(screen: &str) -> Option<u16> {
    screen.lines().enumerate().find_map(|(row, line)| {
        let right = right_of_split(line);
        let inner = right.trim_matches(|c: char| c == '\u{2502}' || c.is_whitespace());
        (inner.chars().count() >= 10 && inner.chars().all(|c| c == '_')).then_some(row as u16)
    })
}

/// 0-based screen row of the boxed panes' bottom border (`┘└` join).
fn pane_bottom_row(screen: &str) -> Option<u16> {
    screen
        .lines()
        .position(|line| line.contains("┘└"))
        .map(|row| row as u16)
}

/// Message rows the graph footer shows: the footer is the rule, the meta
/// row, then the message rows down to the last body row above the border.
fn footer_msg_rows(screen: &str) -> Option<u16> {
    let rule = footer_rule_row(screen)?;
    pane_bottom_row(screen)?.checked_sub(rule)?.checked_sub(2)
}

/// Graph focused on the long-message commit with its footer expanded.
fn long_msg_selected(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && graph_cursor_on(screen, "nnnn")
        && footer_rule_row(screen).is_some()
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
}

/// `/longmsg`, Tab to the graph, `j` onto the long-message commit. Returns
/// the footer message rows that frame shows.
fn open_long_msg_graph(tui: &mut PtySession) -> u16 {
    tui.wait_contains("README.md", WAIT);
    tui.search(REPO);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, REPO) && screen.contains("nnnn"),
        "search loads the long-message graph",
        WAIT,
    );
    tui.tab();
    tui.wait_pred(
        panes_tree_unfocused_graph_focused,
        "Tab focuses the graph",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(long_msg_selected, "j onto the long-message commit", WAIT);
    footer_msg_rows(&tui.screen())
        .unwrap_or_else(|| panic!("footer rule and pane bottom:\n{}", tui.screen()))
}

/// Hovering the tree/right divider lights it and sets the resize pointer;
/// dragging it still resizes the panes; moving away clears both.
///
/// Live PTY, boxed baseline (Tokyo Night). Launch writes no OSC 22. SGR
/// motion (`Cb` 35) onto the divider recolours the tree pane's right
/// border (body rows) to the theme `cursor` accent and writes
/// `OSC 22 ; ew-resize ST` once; motion to the next band column writes
/// nothing new. A press-drag-release of 10 columns moves the `┐┌` join (the
/// adjacent drag path), and the edge stays lit under the pointer. Motion
/// into the tree writes `OSC 22 ; default ST` and the border loses the
/// accent. A paint-only hover, a shape on every motion, or a lost drag
/// fails.
#[test]
fn pty_pane_divider_hover_lights_edge_sets_pointer_and_drags() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(documented_launch_first_paint, "first paint", WAIT);
    let screen = tui.screen();
    let join = pane_join_col(&screen).unwrap_or_else(|| panic!("tree/right join:\n{screen}"));
    let row = tree_row_containing(&screen, "README.md")
        .unwrap_or_else(|| panic!("README row:\n{screen}"));
    assert!(
        !lit(&tui, join, row),
        "the divider is not lit before any hover:\n{screen}"
    );
    assert!(
        pointer_shapes(&tui.raw_output()).is_empty(),
        "launch writes no OSC 22"
    );

    tui.sgr_mouse(SGR_MOTION, join, row);
    tui.wait_pred(
        |_| lit(&tui, join, row) && lit(&tui, join, row + 1),
        "hover lights the divider body rows with the cursor accent",
        WAIT,
    );
    wait_raw(
        &tui,
        |raw| positions(raw, EW_RESIZE).len() == 1,
        "hover writes OSC 22 ew-resize",
    );
    assert!(
        !lit(&tui, join + 1, row),
        "the right pane's left border keeps its colour"
    );

    // Same band, next column: no new shape. The drag below runs after this
    // motion, so the shapes read once the drag has painted include any
    // shape the motion wrote.
    tui.sgr_mouse(SGR_MOTION, join + 1, row);

    let dest = join + 10;
    tui.sgr_mouse(0, join, row);
    tui.sgr_mouse(SGR_LEFT_DRAG, dest, row);
    sgr_release(&mut tui, dest, row);
    tui.wait_pred(
        |screen| pane_join_col(screen) == Some(dest) && lit(&tui, dest, row),
        "the divider drag moves the join under the pointer and it stays lit",
        WAIT,
    );
    assert_eq!(
        pointer_shapes(&tui.raw_output()),
        vec!["ew-resize".to_string()],
        "motion inside the band (and the drag on it) writes no second shape"
    );

    tui.sgr_mouse(SGR_MOTION, 2, row);
    tui.wait_pred(
        |_| !lit(&tui, dest, row) && !lit(&tui, dest, row + 1),
        "motion off the divider clears the highlight",
        WAIT,
    );
    wait_raw(
        &tui,
        |raw| {
            let ew = positions(raw, EW_RESIZE);
            ew.last()
                .is_some_and(|at| find_from(raw, POINTER_DEFAULT, *at).is_some())
        },
        "motion off the divider writes OSC 22 default after ew-resize",
    );
    assert_eq!(
        pointer_shapes(&tui.raw_output()),
        vec!["ew-resize".to_string(), "default".to_string()],
        "one shape per change"
    );
}

/// A blocking `$EDITOR` resets the pointer before it leaves the alternate
/// screen, and the first frame after the resume sends the shape again.
///
/// Hover the divider (`ew-resize`), then `e` on README.md with a stub
/// editor that exits at once. After that `ew-resize`, the stream holds
/// `OSC 22 ; default ST`, then the suspend's `ESC [?1049l`, then the
/// resume's `ESC [?1049h`, then `ew-resize` again (the stored pointer is
/// still on the divider). An editor that inherits the resize shape fails.
#[test]
fn pty_editor_suspend_resets_and_resume_restores_pointer_shape() {
    let (_root, workspace) = daily_workspace();
    let shim_dir = workspace.join(".e2e-editor-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let stub = shim_dir.join("stub-editor");
    fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    let editor = stub.display().to_string();
    let mut tui = PtySession::open_with_env(&workspace, &[("EDITOR", editor.as_str())]);
    tui.wait_pred(documented_launch_first_paint, "first paint", WAIT);
    let screen = tui.screen();
    let join = pane_join_col(&screen).unwrap_or_else(|| panic!("tree/right join:\n{screen}"));
    let row = tree_row_containing(&screen, "README.md")
        .unwrap_or_else(|| panic!("README row:\n{screen}"));

    tui.sgr_mouse(SGR_MOTION, join, row);
    wait_raw(
        &tui,
        |raw| !positions(raw, EW_RESIZE).is_empty(),
        "hover writes OSC 22 ew-resize",
    );
    tui.key('e');
    wait_raw(
        &tui,
        |raw| {
            let Some(hover) = positions(raw, EW_RESIZE).first().copied() else {
                return false;
            };
            let Some(reset) = find_from(raw, POINTER_DEFAULT, hover) else {
                return false;
            };
            let Some(leave) = find_from(raw, ALT_SCREEN_OFF, reset) else {
                return false;
            };
            let Some(back) = find_from(raw, ALT_SCREEN_ON, leave) else {
                return false;
            };
            find_from(raw, EW_RESIZE, back).is_some()
        },
        "ew-resize, default, leave alt screen, re-enter, ew-resize",
    );
    let raw = tui.raw_output();
    let hover = positions(&raw, EW_RESIZE)[0];
    let leave = find_from(&raw, ALT_SCREEN_OFF, hover).unwrap();
    assert!(
        find_from(&raw, POINTER_DEFAULT, hover).is_some_and(|reset| reset < leave),
        "the reset lands before the suspend leaves the alternate screen"
    );
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md") && screen.contains("? help"),
        "the TUI remounts after the editor",
        WAIT,
    );
}

/// Dragging the graph commit-message rule resizes the footer, saves the
/// rows in the UI-state store (never the config file), and a relaunch
/// with that store starts at the saved height.
///
/// Live PTY, boxed baseline config (no `commitMessageLines`), empty store
/// at a shared `WS_STATUS_UI_STORE`. The footer starts at 4 message rows.
/// Motion onto the `_` rule writes `OSC 22 ; ns-resize ST` and lights the
/// rule. Press, drag up 3 rows, release: the rule moves up 3, the crumb
/// shows `msg lines 7`, the store holds `commitMessageLines` 7, and the
/// config file bytes are unchanged. `q`, relaunch with the same store: the
/// footer shows 7 rows with no drag. A session-only drag, a config write,
/// or a store that loses the value fails.
#[test]
fn pty_commit_msg_rule_drag_saves_rows_and_relaunch_keeps_them() {
    let (_root, workspace) = daily_workspace();
    seed_multiline_message_repo(&workspace, REPO);
    let ui_store = workspace
        .join(".e2e-state")
        .join("ui-state-commit-msg.json");
    let store = ui_store.to_str().expect("utf-8 store path").to_string();
    let env = [("WS_STATUS_UI_STORE", store.as_str())];
    let config = user_config_file(&workspace.join(".e2e-config"));

    let mut first = PtySession::open_with_env(&workspace, &env);
    let rows = open_long_msg_graph(&mut first);
    assert_eq!(
        rows,
        DEFAULT_MSG_LINES,
        "no saved value, no config value: the default footer:\n{}",
        first.screen()
    );
    let rule = footer_rule_row(&first.screen()).unwrap();
    let col = RIGHT_PANE_COL;
    assert!(
        col < COLS - 2,
        "grab column inside the right pane:\n{}",
        first.screen()
    );

    first.sgr_mouse(SGR_MOTION, col, rule);
    first.wait_pred(
        |_| lit(&first, col, rule),
        "hover lights the commit-message rule",
        WAIT,
    );
    wait_raw(
        &first,
        |raw| !positions(raw, NS_RESIZE).is_empty(),
        "hover on the rule writes OSC 22 ns-resize",
    );

    let target = rule - DRAG_ROWS;
    first.sgr_mouse(0, col, rule);
    first.sgr_mouse(SGR_LEFT_DRAG, col, target);
    sgr_release(&mut first, col, target);
    let want = DEFAULT_MSG_LINES + DRAG_ROWS;
    first.wait_pred(
        |screen| {
            footer_rule_row(screen) == Some(target)
                && footer_msg_rows(screen) == Some(want)
                && crumb_row(screen).contains(&format!("msg lines {want}"))
                && long_msg_selected(screen)
        },
        "dragging the rule up 3 rows grows the footer by 3",
        WAIT,
    );
    wait_store(&ui_store, want);
    assert_eq!(
        fs::read_to_string(&config).unwrap(),
        BASELINE_USER_CONFIG,
        "a commit-message drag never writes the config file"
    );
    first.key('q');
    first.wait_exit(WAIT);

    // `first` stays bound: its Drop removes the fixture root.
    let mut restarted = PtySession::open_with_env(&workspace, &env);
    let rows = open_long_msg_graph(&mut restarted);
    assert_eq!(
        rows,
        want,
        "relaunch with the saved store starts at the dragged height:\n{}",
        restarted.screen()
    );
    assert!(
        !crumb_row(&restarted.screen()).contains("msg lines"),
        "the saved height needs no resize this session:\n{}",
        restarted.screen()
    );
}

/// Wait until the UI-state store at `path` holds `commitMessageLines`
/// `lines` (pretty JSON as the binary writes it on a fresh file).
fn wait_store(path: &Path, lines: u16) {
    let want =
        format!("{{\n  \"version\": 1,\n  \"activitySparkline\": false,\n  \"commitMessageLines\": {lines}\n}}\n");
    let start = Instant::now();
    loop {
        let body = fs::read_to_string(path).unwrap_or_default();
        if body == want {
            return;
        }
        assert!(
            start.elapsed() < WAIT,
            "store {} must hold commitMessageLines {lines}; got:\n{body}",
            path.display()
        );
        thread::sleep(Duration::from_millis(25));
    }
}

/// Config `viewDefaults.commitMessageLines` with no saved value sets the
/// launch footer height.
///
/// The test writes its own user config (the baseline look plus
/// `commitMessageLines` 6) and starts with an empty UI-state store. The
/// graph footer shows 6 message rows on launch with no resize, and the
/// launch writes no store. A config value that is ignored fails.
#[test]
fn pty_config_commit_message_lines_sets_the_launch_footer() {
    let (_root, workspace) = daily_workspace();
    seed_multiline_message_repo(&workspace, REPO);
    let config = user_config_file(&workspace.join(".e2e-config"));
    fs::create_dir_all(config.parent().unwrap()).unwrap();
    fs::write(
        &config,
        "{\"theme\":\"tokyo-night\",\"themeMode\":\"dark\",\"viewDefaults\":{\"background\":\"terminal\",\"commitMessageLines\":6}}\n",
    )
    .unwrap();
    let ui_store = workspace
        .join(".e2e-state")
        .join("ui-state-config-msg.json");
    let store = ui_store.to_str().expect("utf-8 store path").to_string();

    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_UI_STORE", store.as_str())]);
    let rows = open_long_msg_graph(&mut tui);
    assert_eq!(
        rows,
        6,
        "config commitMessageLines 6 sets the footer:\n{}",
        tui.screen()
    );
    assert!(
        !crumb_row(&tui.screen()).contains("msg lines"),
        "no resize ran:\n{}",
        tui.screen()
    );
    assert!(!ui_store.exists(), "a launch with no drag writes no store");
}

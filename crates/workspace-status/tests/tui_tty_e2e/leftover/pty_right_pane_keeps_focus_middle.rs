use std::fs;
use std::path::PathBuf;

use crate::harness::{PtySession, SGR_WHEEL_DOWN};
use crate::seed::{seed_many_commit_files, seed_repo, seed_tall_graph, unique_root};
use crate::support::{
    graph_cursor_on, graph_pane_focused, panes_files_focused, panes_graph_focused_files_unfocused,
    panes_tree_focused_diff_unfocused, panes_tree_focused_graph_unfocused,
    panes_tree_unfocused_diff_focused, right_pane, title_has_files, tree_cursor_on, GIT_WAIT,
    RIGHT_PANE_COL, SETTLE_MS, WAIT,
};

/// Wheel up (`ScrollUp`, `Cb` 64). Wheel down is [`SGR_WHEEL_DOWN`] (65).
const SGR_WHEEL_UP: u8 = 64;

/// Wheel down + 1003 motion bit (`65 | 32`). crossterm 0.28 drops this.
const SGR_WHEEL_DOWN_MOTION: u8 = 65 | 32;

/// Kitty CSI-u Up / Down. Crossterm 0.28 `event::read` yields Char U+E008 / U+E009.
const KITTY_UP: u32 = 57352;
const KITTY_DOWN: u32 = 57353;

/// Gap so the input thread does not drain a burst of nav as one move.
const KEY_GAP_MS: u64 = 50;

#[derive(Clone, Copy)]
enum RightList {
    Diff,
    Graph,
    Files,
}

fn tree_keyboard_focus(screen: &str) -> bool {
    panes_tree_focused_diff_unfocused(screen) || panes_tree_focused_graph_unfocused(screen)
}

fn return_to_tree(tui: &mut PtySession) {
    unfocus_right(
        tui,
        |screen| !tree_keyboard_focus(screen),
        tree_keyboard_focus,
        "Esc returns keyboard focus to the workspace tree (search clear, then unfocus)",
    );
}

/// First Esc may clear an armed pane search. A second Esc runs only while
/// the right pane still holds keyboard focus, so files drill does not pop.
fn unfocus_right(
    tui: &mut PtySession,
    still_right: impl Fn(&str) -> bool,
    now_left: impl Fn(&str) -> bool,
    why: &str,
) {
    tui.esc();
    tui.wait_ms(120);
    if still_right(&tui.screen()) {
        tui.esc();
    }
    tui.wait_pred(now_left, why, WAIT);
}

fn keep_middle_workspace() -> (PathBuf, PathBuf) {
    let root = unique_root("ws-tui-tty-keep-mid");
    let workspace = root.join("workspace");
    fs::create_dir_all(&workspace).unwrap();
    seed_repo(&workspace, "app", "main", true);
    let mut body = String::new();
    for i in 0..40 {
        body.push_str(&format!("keepmid-line-{i}\n"));
    }
    fs::write(workspace.join("app").join("keepmid-diff.rs"), body).unwrap();
    seed_tall_graph(&workspace, "history");
    seed_many_commit_files(&workspace, "bundle", 40);
    (root, workspace)
}

fn help_lists_jk_down_up(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("MOVE") && compact.contains("j k") && compact.contains("down / up")
}

fn is_graph_list_row(line: &str) -> bool {
    line.contains('\u{258C}')
        || line.contains("working tree")
        || line.contains('|')
        || line.contains(" o ")
        || line.contains(" * ")
        || line.contains(" @ ")
}

fn is_bottom_border(line: &str) -> bool {
    let t = line.trim();
    !t.is_empty()
        && !t.contains('\u{258C}')
        && t.chars().all(|c| {
            matches!(
                c,
                '─' | '━'
                    | '└'
                    | '┘'
                    | '┤'
                    | '├'
                    | '┴'
                    | '┼'
                    | '│'
                    | ' '
                    | '█'
                    | '▄'
                    | '▀'
            )
        })
}

fn right_inner_lines(screen: &str) -> Vec<String> {
    let mut lines: Vec<String> = right_pane(screen).lines().map(str::to_string).collect();
    while lines.last().is_some_and(|line| is_bottom_border(line)) {
        lines.pop();
    }
    lines
}

/// List body only: skip pane chrome so Y is compared to `height / 2`.
fn list_body(screen: &str, kind: RightList) -> Vec<String> {
    let lines = right_inner_lines(screen);
    match kind {
        RightList::Diff => lines.into_iter().skip(1).collect(),
        RightList::Graph => {
            let start = lines
                .iter()
                .position(|line| is_graph_list_row(line))
                .unwrap_or(0);
            let list = &lines[start..];
            if list.len() >= 3 {
                list[..list.len() - 2].to_vec()
            } else {
                list.to_vec()
            }
        }
        RightList::Files => {
            let start = lines
                .iter()
                .position(|line| line.contains('\u{258C}') || line.contains(".txt"))
                .unwrap_or(0);
            lines[start..].to_vec()
        }
    }
}

struct FocusGeom {
    y: usize,
    body_h: usize,
    mid: usize,
    marker: String,
}

fn focus_geom(screen: &str, kind: RightList) -> Option<FocusGeom> {
    let body = list_body(screen, kind);
    let body_h = body.len();
    let y = body.iter().position(|line| line.contains('\u{258C}'))?;
    Some(FocusGeom {
        y,
        body_h,
        mid: body_h / 2,
        marker: body[y].clone(),
    })
}

fn geom_or(screen: &str, kind: RightList, why: &str) -> FocusGeom {
    focus_geom(screen, kind)
        .unwrap_or_else(|| panic!("{why}: no cursor bar in the list body\n{screen}"))
}

/// Row 0 clamps to the top of the list body (`list_viewport_start`).
fn assert_top_clamped(screen: &str, kind: RightList, why: &str) {
    let g = geom_or(screen, kind, why);
    assert!(
        g.body_h >= 8,
        "{why}: list body too short ({})\n{screen}",
        g.body_h
    );
    assert_eq!(
        g.y, 0,
        "{why}: row 0 must clamp to the top of the list body (y={} mid={} body_h={})\n{screen}",
        g.y, g.mid, g.body_h
    );
}

/// Focused row sits at list-body `height / 2`. A quarter-pane offset is red.
fn assert_list_middle(screen: &str, kind: RightList, why: &str) {
    let g = geom_or(screen, kind, why);
    assert!(
        g.body_h >= 8,
        "{why}: list body too short ({})\n{screen}",
        g.body_h
    );
    let quarter = g.body_h / 4;
    assert_ne!(
        g.y, quarter,
        "{why}: a ~25% row must not pass (y={} mid={} body_h={})\n{screen}",
        g.y, g.mid, g.body_h
    );
    assert_ne!(
        g.y,
        g.body_h.saturating_sub(quarter.max(1)),
        "{why}: a ~75% row must not pass (y={} mid={} body_h={})\n{screen}",
        g.y,
        g.mid,
        g.body_h
    );
    assert_ne!(
        g.y,
        g.body_h.saturating_sub(1),
        "{why}: edge-stuck on the last list-body line (y={} body_h={})\n{screen}",
        g.y,
        g.body_h
    );
    assert_eq!(
        g.y, g.mid,
        "{why}: focused row must sit at list-body height/2 (y={} mid={} body_h={})\n{screen}",
        g.y, g.mid, g.body_h
    );
}

fn csi_u_arrow(tui: &mut PtySession, codepoint: u32) {
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

fn send_j(tui: &mut PtySession) {
    tui.key('j');
}

fn send_k(tui: &mut PtySession) {
    tui.key('k');
}

fn send_down(tui: &mut PtySession) {
    csi_u_arrow(tui, KITTY_DOWN);
}

fn send_up(tui: &mut PtySession) {
    csi_u_arrow(tui, KITTY_UP);
}

fn send_wheel_down(tui: &mut PtySession) {
    tui.sgr_mouse(SGR_WHEEL_DOWN, RIGHT_PANE_COL, 8);
}

fn send_wheel_up(tui: &mut PtySession) {
    tui.sgr_mouse(SGR_WHEEL_UP, RIGHT_PANE_COL, 8);
}

/// One nav step must change the focused marker. After the row is past the
/// midpoint, Y must be `body_h / 2`. End-clamp is only the last rows.
fn step_focus(
    tui: &mut PtySession,
    kind: RightList,
    send: fn(&mut PtySession),
    down: bool,
    why: &str,
) {
    let before = geom_or(&tui.screen(), kind, why);
    send(tui);
    tui.wait_ms(KEY_GAP_MS);
    let screen = tui.screen();
    let after = geom_or(&screen, kind, why);
    assert_ne!(
        after.marker, before.marker,
        "{why}: no-op (focused marker unchanged)\nbefore={}\nafter={}\n{screen}",
        before.marker, after.marker
    );
    if after.y == after.mid {
        assert_list_middle(&screen, kind, why);
        return;
    }
    assert_ne!(
        after.y,
        after.body_h.saturating_sub(1),
        "{why}: edge-stuck on the last list-body line\n{screen}"
    );
    if down {
        assert!(
            after.y > before.y,
            "{why}: down must move the cursor toward the middle ({} -> {})\n{screen}",
            before.y,
            after.y
        );
        assert!(
            after.y < after.mid,
            "{why}: still climbing; Y must stay below height/2 until it lands on it (y={} mid={})\n{screen}",
            after.y, after.mid
        );
    } else {
        assert!(
            after.y < before.y,
            "{why}: up must move the cursor toward the middle ({} -> {})\n{screen}",
            before.y,
            after.y
        );
        assert!(
            after.y > after.mid,
            "{why}: still descending; Y must stay above height/2 until it lands on it (y={} mid={})\n{screen}",
            after.y, after.mid
        );
    }
}

fn drive_to_middle(tui: &mut PtySession, kind: RightList, send: fn(&mut PtySession), why: &str) {
    let launch = geom_or(&tui.screen(), kind, why);
    let steps = launch.mid + 3;
    for i in 0..steps {
        step_focus(
            tui,
            kind,
            send,
            true,
            &format!("{why} toward middle, step {i}"),
        );
    }
    assert_list_middle(&tui.screen(), kind, why);
}

fn move_stays_middle(tui: &mut PtySession, kind: RightList, send: fn(&mut PtySession), why: &str) {
    let before = geom_or(&tui.screen(), kind, why);
    send(tui);
    tui.wait_ms(KEY_GAP_MS);
    let screen = tui.screen();
    let after = geom_or(&screen, kind, why);
    assert_ne!(
        after.marker, before.marker,
        "{why}: no-op (focused marker unchanged)\n{screen}"
    );
    assert_list_middle(&screen, kind, why);
}

/// Right-pane `j` / `k` / Down / Up / vertical wheel keep the focused row
/// at list-body height/2.
///
/// Help MOVE lists `j k` as "down / up". Docs: the workspace tree already
/// recentres the focused row (`list_viewport_start`); graph commits,
/// file-diff rows, and the commit-file list use the same viewport rule.
/// Vertical wheel over the right pane moves that focused row. Motion-bit
/// `CSI < 97` must not move. This is not hscroll.
///
/// Y is the cursor-bar line inside the list body (not the whole right pane).
/// After the focused row is past the midpoint, Y must equal `body_h / 2`.
/// A ~25% / ~75% row is red. Tab / Enter onto row 0 clamps to the top.
/// Tab / Enter onto an already-mid row must keep it in the middle.
#[test]
fn pty_right_pane_keeps_focus_middle() {
    let (_root, workspace) = keep_middle_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(
        help_lists_jk_down_up,
        "help MOVE lists j k as down / up",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("down / up") && screen.contains("? help"),
        "Esc closes help so j is not swallowed",
        WAIT,
    );

    tui.search("keepmid-diff");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "keepmid-diff.rs") && screen.contains("keepmid-line-0"),
        "search lands on the tall dirty file",
        GIT_WAIT,
    );
    tui.tab();
    tui.wait_pred(
        |screen| {
            panes_tree_unfocused_diff_focused(screen)
                && tree_cursor_on(screen, "keepmid-diff.rs")
                && right_pane(screen).contains("keepmid-line-0")
                && right_pane(screen).contains('\u{258C}')
        },
        "Tab focuses the file-diff on row 0",
        WAIT,
    );
    assert_top_clamped(
        &tui.screen(),
        RightList::Diff,
        "Tab onto a file-diff at row 0 clamps to the top of the list body",
    );
    drive_to_middle(
        &mut tui,
        RightList::Diff,
        send_j,
        "j on a focused file-diff",
    );
    assert!(
        !right_pane(&tui.screen()).contains("keepmid-line-0"),
        "j past the midpoint must leave keepmid-line-0:\n{}",
        tui.screen()
    );
    move_stays_middle(
        &mut tui,
        RightList::Diff,
        send_k,
        "k on a focused file-diff",
    );
    move_stays_middle(
        &mut tui,
        RightList::Diff,
        send_down,
        "Down on a focused file-diff",
    );
    move_stays_middle(
        &mut tui,
        RightList::Diff,
        send_up,
        "Up on a focused file-diff",
    );
    move_stays_middle(
        &mut tui,
        RightList::Diff,
        send_wheel_down,
        "vertical wheel down on a focused file-diff",
    );
    move_stays_middle(
        &mut tui,
        RightList::Diff,
        send_wheel_up,
        "vertical wheel up on a focused file-diff",
    );
    unfocus_right(
        &mut tui,
        panes_tree_unfocused_diff_focused,
        tree_keyboard_focus,
        "Esc unfocuses the file-diff onto the tree",
    );
    tui.tab();
    tui.wait_pred(
        panes_tree_unfocused_diff_focused,
        "Tab returns to the file-diff",
        WAIT,
    );
    assert_list_middle(
        &tui.screen(),
        RightList::Diff,
        "Tab onto a mid-list file-diff must recentre, not jump to the top",
    );

    return_to_tree(&mut tui);
    tui.search("history");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "history") && screen.contains("count 29"),
        "search lands on the tall graph repo",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            graph_pane_focused(screen)
                && graph_cursor_on(screen, "working tree")
                && right_pane(screen).contains("count 29")
                && !graph_cursor_on(screen, "count 0")
        },
        "Enter focuses the graph on working tree (row 0)",
        GIT_WAIT,
    );
    assert_top_clamped(
        &tui.screen(),
        RightList::Graph,
        "Enter onto the graph at row 0 clamps to the top of the list body",
    );
    for _ in 0..8 {
        tui.sgr_mouse(SGR_WHEEL_DOWN_MOTION, RIGHT_PANE_COL, 8);
    }
    tui.wait_ms(SETTLE_MS);
    assert!(
        graph_cursor_on(&tui.screen(), "working tree"),
        "motion-bit CSI < 97 must not move the graph cursor:\n{}",
        tui.screen()
    );
    drive_to_middle(
        &mut tui,
        RightList::Graph,
        send_wheel_down,
        "vertical wheel down on the graph",
    );
    assert!(
        !graph_cursor_on(&tui.screen(), "working tree"),
        "wheel past the midpoint must leave working tree:\n{}",
        tui.screen()
    );
    move_stays_middle(&mut tui, RightList::Graph, send_j, "j on the graph");
    move_stays_middle(&mut tui, RightList::Graph, send_k, "k on the graph");
    move_stays_middle(&mut tui, RightList::Graph, send_down, "Down on the graph");
    move_stays_middle(&mut tui, RightList::Graph, send_up, "Up on the graph");
    move_stays_middle(
        &mut tui,
        RightList::Graph,
        send_wheel_up,
        "vertical wheel up on the graph",
    );
    unfocus_right(
        &mut tui,
        graph_pane_focused,
        panes_tree_focused_graph_unfocused,
        "Esc unfocuses the graph onto the tree",
    );
    tui.enter();
    tui.wait_pred(graph_pane_focused, "Enter returns to the graph", GIT_WAIT);
    assert_list_middle(
        &tui.screen(),
        RightList::Graph,
        "Enter onto a mid-list graph must recentre, not jump to the top",
    );

    return_to_tree(&mut tui);
    tui.search("bundle");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "bundle") && screen.contains("keepmid-files-commit"),
        "search lands on the many-file repo",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| graph_pane_focused(screen) && graph_cursor_on(screen, "working tree"),
        "Enter focuses the bundle graph on working tree",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "keepmid-files-commit")
                && !graph_cursor_on(screen, "working tree")
        },
        "j selects the many-file commit",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            panes_files_focused(screen)
                && title_has_files(screen)
                && right_pane(screen).contains("keepmid-00.txt")
                && right_pane(screen).contains('\u{258C}')
        },
        "Enter drills to commit files on row 0",
        GIT_WAIT,
    );
    assert_top_clamped(
        &tui.screen(),
        RightList::Files,
        "Enter onto commit files at row 0 clamps to the top of the list body",
    );
    drive_to_middle(
        &mut tui,
        RightList::Files,
        send_j,
        "j on the commit-file list",
    );
    assert!(
        !right_pane(&tui.screen()).contains("keepmid-00.txt"),
        "j past the midpoint must leave keepmid-00.txt:\n{}",
        tui.screen()
    );
    move_stays_middle(
        &mut tui,
        RightList::Files,
        send_k,
        "k on the commit-file list",
    );
    move_stays_middle(
        &mut tui,
        RightList::Files,
        send_down,
        "Down on the commit-file list",
    );
    move_stays_middle(
        &mut tui,
        RightList::Files,
        send_up,
        "Up on the commit-file list",
    );
    move_stays_middle(
        &mut tui,
        RightList::Files,
        send_wheel_down,
        "vertical wheel down on the commit-file list",
    );
    move_stays_middle(
        &mut tui,
        RightList::Files,
        send_wheel_up,
        "vertical wheel up on the commit-file list",
    );
    unfocus_right(
        &mut tui,
        panes_files_focused,
        panes_graph_focused_files_unfocused,
        "Esc unfocuses commit files onto the graph",
    );
    tui.tab();
    tui.wait_pred(panes_files_focused, "Tab returns to commit files", WAIT);
    assert_list_middle(
        &tui.screen(),
        RightList::Files,
        "Tab onto a mid-list commit-file row must recentre, not jump to the top",
    );
}

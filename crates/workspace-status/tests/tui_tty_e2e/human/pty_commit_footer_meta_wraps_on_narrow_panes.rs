use std::fs;

use crate::harness::{user_config_file, PtySession, FLAT_USER_CONFIG};
use crate::seed::{git_stdout, seed_demo_workspace, unique_root};
use crate::support::{
    panes_files_focused, panes_files_unfocused_diff_focused, panes_tree_focused_graph_unfocused,
    panes_tree_unfocused_graph_focused, title_has_diff, GIT_WAIT, WAIT,
};

/// Terminal size of the repro: a 58-col right pane, a 38-col left pane.
const COLS: u16 = 100;
const ROWS: u16 = 30;
/// Subject of the commit whose meta line does not fit one row.
const SUBJECT: &str = "Tighten session expiry";
/// Its ref chip (ASCII glyphs: `+` marks the checkout).
const CHIP: &str = "[+feature/auth-refresh]";
/// Its author and date (`TZ=UTC`; the seed clock is +08).
const AUTHOR: &str = "Demo User";
const DATE: &str = "2026-08-17 03:00";
/// SGR pointer motion with no button held (`3 | 32`, any-event tracking).
const SGR_POINTER_MOVE: u8 = 3 | 32;
/// Footer of a pinned popover. A peek has none.
const PINNED_FOOTER: &str = "y copy line · Enter run · Esc close";
/// Title of the author and time popovers.
const AUTHOR_TITLE: &str = "commit author";
const TIME_TITLE: &str = "commit time";
/// First field of the author popover.
const AUTHOR_FIELD: &str = "Demo User <demo@example.invalid>";
/// First inner column of the left pane, in both looks (the box border or
/// the flat pad cell is column 0).
const LEFT_X: usize = 1;
/// Columns of the left pane's 38 inner cells.
const LEFT_COLS: std::ops::Range<usize> = 1..39;
/// Columns of the right pane, from the cell where its cursor bar paints
/// (the flat pad cell, or the boxed pane's first inner column) to its end.
const RIGHT_PANE: std::ops::Range<usize> = 40..100;
/// Columns of the right pane's 58 content cells, in both looks.
const RIGHT_COLS: std::ops::Range<usize> = 41..99;
/// First right-pane content column.
const RIGHT_X: usize = 41;

/// Pane look of a run.
#[derive(Clone, Copy)]
enum Look {
    /// The harness baseline config: `viewDefaults.background` `"terminal"`.
    Boxed,
    /// [`FLAT_USER_CONFIG`]: `viewDefaults.background` `"paint"`.
    Flat,
}

impl Look {
    /// Screen row of both panes' last inner row in a 30-row terminal.
    /// Boxed: row 27 is the box bottom border. Flat: no border, so row 27
    /// (above the breadcrumb and the status row) is the last pane row.
    fn last_row(self) -> usize {
        match self {
            Look::Boxed => 26,
            Look::Flat => 27,
        }
    }
}

/// Cells `cols` of screen row `row`, trailing blanks cut. ASCII glyphs:
/// one char per cell, so the char index is the column.
fn cells(screen: &str, row: usize, cols: std::ops::Range<usize>) -> String {
    let line = screen.lines().nth(row).unwrap_or("");
    line.chars()
        .skip(cols.start)
        .take(cols.len())
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// The last `n` inner rows of the pane over `cols`, top to bottom.
fn bottom_rows(screen: &str, look: Look, cols: std::ops::Range<usize>, n: usize) -> Vec<String> {
    let last = look.last_row();
    (last + 1 - n..=last)
        .map(|row| cells(screen, row, cols.clone()))
        .collect()
}

/// Focused cursor bar (`▌`) on the first row of a run of bar rows over
/// `cols` that holds `needle` (the selected commit also paints the bar on
/// its spacer row).
fn cursor_on(screen: &str, cols: std::ops::Range<usize>, needle: &str) -> bool {
    let mut above_has_bar = false;
    screen.lines().any(|line| {
        let pane: String = line.chars().skip(cols.start).take(cols.len()).collect();
        let bar = pane.contains('\u{258C}');
        let node = bar && !above_has_bar;
        above_has_bar = bar;
        node && pane.contains(needle)
    })
}

/// The meta rows at depth 0: the 58-col right pane wraps the meta line
/// after the parent group; the author and date take the pane's last row.
fn depth_0_meta(screen: &str, look: Look, tip: &str, parent: &str) -> bool {
    bottom_rows(screen, look, RIGHT_COLS, 2)
        == [
            format!("{CHIP} · {tip} · parent {parent}"),
            format!("{AUTHOR} · {DATE}"),
        ]
}

/// The meta rows of a 38-col left pane (graph footer at depth 1,
/// commit-files footer at depth 2), under the rule and the 4 message
/// rows (subject first).
fn left_meta(screen: &str, look: Look, tip: &str, parent: &str) -> bool {
    let rows = bottom_rows(screen, look, LEFT_COLS, 8);
    let rule = &rows[0];
    rule.chars().count() == LEFT_COLS.len()
        && rule.chars().all(|c| c == '_')
        && rows[1] == SUBJECT
        && rows[5..]
            == [
                format!("{CHIP} · {tip}"),
                format!("parent {parent} · {AUTHOR}"),
                DATE.to_string(),
            ]
}

/// No meta row is cut with `…`.
fn uncut(rows: &[String]) -> bool {
    rows.iter().all(|row| !row.contains('…'))
}

/// The commit footer wraps its meta line on narrow panes instead of
/// cutting it, at every depth, with hits on the wrapped rows.
///
/// Docs: the meta line (`chips · hash · parent · author · date`) wraps
/// onto as many rows as it needs at the footer width, pinned under the
/// message rows; hover and click hits keep working on every row.
///
/// Live PTY 100x30 on the demo seed, `TZ=UTC`, in both pane looks
/// ([`Look`]). `/app` loads the app graph, Tab focuses it, `j` walks to
/// `Tighten session expiry` (its chip `[+feature/auth-refresh]`, hash and
/// parent ids from `git rev-parse`). Depth 0 (58-col right pane): the
/// meta rows are `<chip> · <tip> · parent <parent>` and `Demo User ·
/// 2026-08-17 03:00`, the pane's last row; a pointer rest on `Demo User`
/// peeks the author popover. Enter (depth 1, graph on the 38-col left
/// pane) and Enter on `session.ts` (depth 2, commit-files footer): the
/// left pane's last rows are the rule, the subject, and the three meta
/// rows `<chip> · <tip>`, `parent <parent> · Demo User`, `2026-08-17
/// 03:00`; the diff on the right paints no meta. At depth 2 a rest on the
/// date peeks the time popover, and a click on the parent id on the
/// second meta row pins its hash card; Esc closes it. Each claim is false
/// on the screen before its key, so a stale frame cannot pass. The old
/// one-row meta (`... · parent 8dba2d9 · Demo…`), a dropped date, or a
/// hit left on the bottom row cannot pass.
#[test]
fn pty_commit_footer_meta_wraps_on_narrow_panes_boxed() {
    commit_footer_meta_wraps_on_narrow_panes(Look::Boxed);
}

/// [`pty_commit_footer_meta_wraps_on_narrow_panes_boxed`] with flat panes
/// (`viewDefaults.background` `"paint"`, the shipped look): no pane
/// borders, a pad cell on each side of the content.
#[test]
fn pty_commit_footer_meta_wraps_on_narrow_panes_flat() {
    commit_footer_meta_wraps_on_narrow_panes(Look::Flat);
}

fn commit_footer_meta_wraps_on_narrow_panes(look: Look) {
    let root = unique_root("ws-tui-demo-meta-wrap");
    let dest = root.join("workspace");
    seed_demo_workspace(&dest);
    let app = dest.join("app");
    let short = |spec: &str| git_stdout(&app, &["rev-parse", spec])[..7].to_string();
    let tip = short("feature/auth-refresh");
    let parent = short("feature/auth-refresh^");
    if let Look::Flat = look {
        let file = user_config_file(&dest.join(".e2e-config"));
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, FLAT_USER_CONFIG).unwrap();
    }
    let mut tui = PtySession::open_size_with_env(&dest, COLS, ROWS, &[("TZ", "UTC")]);
    tui.wait_pred(
        |screen| screen.contains("session.ts") && screen.contains("? help"),
        "demo first paint",
        GIT_WAIT,
    );
    if let Look::Flat = look {
        let screen = tui.screen();
        assert!(
            !screen.contains('\u{250C}') && !screen.contains('\u{2510}'),
            "flat panes paint no box corners:\n{screen}"
        );
    }

    tui.search("app");
    tui.wait_pred(
        |screen| {
            panes_tree_focused_graph_unfocused(screen)
                && cells(screen, 2, RIGHT_PANE).contains("feature/auth-refresh")
                && screen.contains(SUBJECT)
        },
        "/app loads the app graph on the right",
        GIT_WAIT,
    );
    tui.tab();
    tui.wait_pred(
        panes_tree_unfocused_graph_focused,
        "Tab focuses the graph",
        WAIT,
    );

    // `j` one row at a time; each waits for the footer meta to change, so
    // a half-painted frame never sends an extra `j`.
    for _ in 0..4 {
        let screen = tui.screen();
        if cursor_on(&screen, RIGHT_PANE, SUBJECT) {
            break;
        }
        assert!(
            !depth_0_meta(&screen, look, &tip, &parent),
            "the wrapped meta is not on screen before the cursor is on {SUBJECT}:\n{screen}"
        );
        let before = bottom_rows(&screen, look, RIGHT_COLS, 2);
        tui.key('j');
        tui.wait_pred(
            |screen| bottom_rows(screen, look, RIGHT_COLS, 2) != before,
            "j moves the cursor and the footer meta follows",
            WAIT,
        );
    }

    // Depth 0: two meta rows, the author and date on the pane's last row.
    tui.wait_pred(
        |screen| {
            cursor_on(screen, RIGHT_PANE, SUBJECT)
                && depth_0_meta(screen, look, &tip, &parent)
                && uncut(&bottom_rows(screen, look, RIGHT_COLS, 2))
        },
        "depth 0: the meta wraps onto two rows at the bottom of the right pane",
        WAIT,
    );
    let last = look.last_row() as u16;
    let screen = tui.screen();
    assert!(
        !screen.contains(AUTHOR_TITLE),
        "no author popover before the pointer rests:\n{screen}"
    );
    tui.sgr_mouse(SGR_POINTER_MOVE, RIGHT_X as u16 + 1, last);
    tui.wait_pred(
        |screen| {
            screen.contains(AUTHOR_TITLE)
                && screen.contains(AUTHOR_FIELD)
                && !screen.contains(PINNED_FOOTER)
        },
        "a pointer rest on the wrapped author peeks the author popover",
        WAIT,
    );
    // Onto the title row, off every hit and the peek: the grace closes it.
    tui.sgr_mouse(SGR_POINTER_MOVE, RIGHT_X as u16 + 1, 1);
    tui.wait_pred(
        |screen| !screen.contains(AUTHOR_TITLE),
        "the author peek closes after the pointer leaves",
        WAIT,
    );

    // Depth 1: the graph footer on the 38-col left pane.
    let screen = tui.screen();
    assert!(
        !left_meta(&screen, look, &tip, &parent),
        "no left-pane meta at depth 0:\n{screen}"
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            panes_files_focused(screen)
                && left_meta(screen, look, &tip, &parent)
                && uncut(&bottom_rows(screen, look, LEFT_COLS, 3))
        },
        "depth 1: the left graph footer wraps the meta onto three rows",
        WAIT,
    );

    // Depth 2: the commit-files footer on the same left pane.
    tui.wait_pred(
        |screen| cursor_on(screen, RIGHT_PANE, "src"),
        "the files cursor starts on the src folder",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| cursor_on(screen, RIGHT_PANE, "session.ts"),
        "j onto session.ts",
        WAIT,
    );
    assert!(
        !panes_files_unfocused_diff_focused(&tui.screen()),
        "no depth-2 diff before Enter:\n{}",
        tui.screen()
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            let right: Vec<String> = (2..=look.last_row())
                .map(|row| cells(screen, row, RIGHT_COLS))
                .collect();
            panes_files_unfocused_diff_focused(screen)
                && title_has_diff(screen)
                && right.iter().any(|row| row.contains("@@"))
                && !right.iter().any(|row| {
                    row.contains(CHIP)
                        || row.contains(&format!("parent {parent}"))
                        || row.contains(DATE)
                })
                && left_meta(screen, look, &tip, &parent)
                && uncut(&bottom_rows(screen, look, LEFT_COLS, 3))
        },
        "depth 2: the commit-files footer wraps the meta; the diff on the right has none",
        WAIT,
    );

    let screen = tui.screen();
    assert!(
        !screen.contains(TIME_TITLE),
        "no time popover before the pointer rests:\n{screen}"
    );
    tui.sgr_mouse(SGR_POINTER_MOVE, LEFT_X as u16 + 1, last);
    tui.wait_pred(
        |screen| screen.contains(TIME_TITLE) && !screen.contains(PINNED_FOOTER),
        "a pointer rest on the wrapped date peeks the time popover",
        WAIT,
    );
    tui.sgr_mouse(SGR_POINTER_MOVE, LEFT_X as u16 + 1, 1);
    tui.wait_pred(
        |screen| !screen.contains(TIME_TITLE),
        "the time peek closes after the pointer leaves",
        WAIT,
    );

    let card = format!("commit {parent}");
    let screen = tui.screen();
    assert!(
        !screen.contains(&card),
        "no hash card before the click:\n{screen}"
    );
    let parent_x = LEFT_X + "parent ".len();
    tui.sgr_click(parent_x as u16 + 1, last - 1);
    tui.wait_pred(
        |screen| screen.contains(&card) && screen.contains(PINNED_FOOTER),
        "a click on the parent id on the second meta row pins its hash card",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains(&card) && !screen.contains(PINNED_FOOTER),
        "Esc closes the hash card",
        WAIT,
    );
}

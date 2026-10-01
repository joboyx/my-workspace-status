use crate::harness::PtySession;
use crate::seed::{daily_workspace, git_stdout};
use crate::support::{
    graph_cursor_on, merger_graph_drilled_right, merger_graph_left_unfocused, GIT_WAIT, WAIT,
};

/// Wide enough that the right-pane footer meta line is not clipped.
const COLS: u16 = 240;
const ROWS: u16 = 32;

/// Short commit id, as the footer paints it.
fn short(id: &str) -> String {
    id.chars().take(7).collect()
}

/// One selectable graph row: list subject plus the footer meta it paints.
struct Row {
    subject: &'static str,
    footer: String,
}

/// Index of the row whose footer meta is on screen.
///
/// The footer is painted below the list, so its meta for a new row means
/// that frame's list rows (cursor bar included) are already painted. Every
/// row has a distinct meta, so at most one matches.
fn footer_row(screen: &str, rows: &[Row]) -> Option<usize> {
    rows.iter().position(|row| screen.contains(&row.footer))
}

/// The graph selection footer shows the focused commit's parents.
///
/// Daily `merger` seed: `root` ← `right` (first parent) and `left`
/// (second parent) ← `merge`. Enter drills into the merger graph. `j`
/// walks the list. The merge commit footer must read
/// `<merge> · parents <right> <left> · ` in git parent order. The
/// one-parent `left` commit must read `<left> · parent <root> · `. The
/// root commit must read `<root> · root commit · `. Ids come from
/// `git rev-parse` on the seed, so a swapped parent order, a missing
/// group, or a footer that shows only the hash cannot pass.
#[test]
fn pty_graph_footer_shows_commit_parents() {
    let (_root, workspace) = daily_workspace();
    let repo = workspace.join("merger");
    let rev = |spec: &str| short(&git_stdout(&repo, &["rev-parse", spec]));
    let merge = rev("main");
    let right = rev("main^1");
    let left = rev("main^2");
    let root = rev("main^1^1");

    let mut tui = PtySession::open_size(&workspace, COLS, ROWS);
    tui.wait_contains("README.md", WAIT);
    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        merger_graph_drilled_right,
        "Enter focuses the merger graph",
        WAIT,
    );

    let stash = rev("stash@{0}");
    let rows = [
        Row {
            subject: "WIP on graph",
            footer: format!("stash@{{0}} · {stash} · "),
        },
        Row {
            subject: "merge",
            footer: format!("{merge} · parents {right} {left} · "),
        },
        Row {
            subject: "right",
            footer: format!("{right} · parent {root} · "),
        },
        Row {
            subject: "left",
            footer: format!("{left} · parent {root} · "),
        },
        Row {
            subject: "root",
            footer: format!("{root} · root commit · "),
        },
    ];
    walk_down_to(&mut tui, &rows, 1, "merge commit footer lists both parents");
    walk_down_to(
        &mut tui,
        &rows,
        3,
        "left commit footer lists its one parent",
    );
    walk_down_to(&mut tui, &rows, 4, "root commit footer shows root commit");
}

/// Press `j` one row at a time until the cursor and footer are on
/// `rows[target]`.
///
/// Each `j` waits until the footer meta names a known row other than the
/// previous one, so a half-painted frame never triggers an extra `j`. The
/// uncommitted row (where the drill can start) is not in `rows`.
fn walk_down_to(tui: &mut PtySession, rows: &[Row], target: usize, what: &str) {
    let mut at = footer_row(&tui.screen(), rows);
    for _ in 0..=rows.len() {
        if at == Some(target) {
            break;
        }
        let prev = at;
        tui.key('j');
        tui.wait_pred(
            |s| footer_row(s, rows).is_some_and(|now| Some(now) != prev),
            &format!("{what}: footer settles on the next row"),
            WAIT,
        );
        at = footer_row(&tui.screen(), rows);
    }
    let row = &rows[target];
    tui.wait_pred(
        |s| graph_cursor_on(s, row.subject) && s.contains(&row.footer),
        what,
        WAIT,
    );
}

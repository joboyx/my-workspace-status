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

/// Graph cursor on `subject` and the footer meta reads `<hash> · <parents>`.
fn footer_on(screen: &str, subject: &str, hash: &str, parents: &str) -> bool {
    let meta = format!("{hash} · {parents} · ");
    graph_cursor_on(screen, subject) && screen.lines().any(|line| line.contains(&meta))
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

    let merge_parents = format!("parents {right} {left}");
    walk_down_to(&mut tui, "merge commit footer lists both parents", |s| {
        footer_on(s, "merge", &merge, &merge_parents)
    });

    let left_parent = format!("parent {root}");
    walk_down_to(&mut tui, "left commit footer lists its one parent", |s| {
        footer_on(s, "left", &left, &left_parent)
    });

    walk_down_to(&mut tui, "root commit footer shows root commit", |s| {
        footer_on(s, "root", &root, "root commit")
    });
}

/// Press `j` until `pred` holds on the painted frame.
fn walk_down_to(tui: &mut PtySession, what: &str, pred: impl Fn(&str) -> bool) {
    for _ in 0..12 {
        if pred(&tui.screen()) {
            return;
        }
        let before = tui.screen();
        tui.key('j');
        tui.wait_pred(|s| s != before || pred(s), what, WAIT);
    }
    tui.wait_pred(pred, what, WAIT);
}

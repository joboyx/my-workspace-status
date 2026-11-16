use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{
    regions_diff, set_view_default, whitespace_workspace, REGIONS_ALPHA, REGIONS_OMEGA,
};
use crate::support::{
    ack_rows, after_ack, regions_highlight_active, right_pane, step_cursor, tree_cursor_on,
    GIT_WAIT, WAIT,
};

const REFUSAL: &str = "turn whitespace on to stage hunks";

fn repo_cached(workspace: &Path) -> String {
    regions_diff(&workspace.join("app"), true)
}

/// `regions.txt` focused in the tree, shown with `-w`.
fn regions_tree_with_dash_w(screen: &str) -> bool {
    let pane = right_pane(screen);
    tree_cursor_on(screen, "regions.txt")
        && pane
            .lines()
            .next()
            .is_some_and(|header| header.contains("· -w"))
        && pane.contains(REGIONS_ALPHA)
        && !screen.contains("VISUAL")
}

/// While whitespace is ignored a highlighted range does not stage; the
/// whole file still does.
///
/// `viewDefaults.whitespace: "ignore"` starts with `-w`. On `regions.txt`
/// (a real change) Tab focuses the diff, `V` and `j` highlight the ALPHA
/// hunk, and `s` refuses with `turn whitespace on to stage hunks`: the
/// index stays empty (`git diff --cached`). Esc, Tab back to the tree, and
/// a whole-file `s` stages both hunks. A range stage that slips through,
/// or a refusal that blocks the whole-file stage, cannot pass.
#[test]
fn pty_ignore_whitespace_refuses_hunk_stage() {
    let (_root, workspace) = whitespace_workspace("ws-tui-tty-whitespace-stage");
    set_view_default(&workspace, "whitespace", "ignore");
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "indent.txt")
                && right_pane(screen).contains("(no diff)")
                && right_pane(screen)
                    .lines()
                    .next()
                    .is_some_and(|header| header.contains("· -w"))
        },
        "config whitespace ignore: launch loads indent.txt with -w",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        regions_tree_with_dash_w,
        "j focuses regions.txt; its real change shows with -w",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        |screen| !tree_cursor_on(screen, "regions.txt") && screen.contains(REGIONS_ALPHA),
        "Tab focuses the regions.txt diff",
        WAIT,
    );
    tui.shift_letter('V');
    tui.wait_pred(regions_highlight_active, "V paints VISUAL", WAIT);
    for _ in 0..6 {
        step_cursor(&mut tui, |tui| tui.key('j'), regions_highlight_active);
    }
    tui.key('s');
    tui.wait_pred(
        |screen| ack_rows(screen).contains(REFUSAL) && screen.contains("VISUAL"),
        "s on the highlight refuses while whitespace is ignored",
        WAIT,
    );
    assert_eq!(
        repo_cached(&workspace),
        "",
        "a refused range stages nothing"
    );

    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("VISUAL") && screen.contains(REGIONS_ALPHA),
        "Esc leaves the highlight",
        WAIT,
    );
    tui.tab();
    tui.wait_pred(
        regions_tree_with_dash_w,
        "Tab returns to the tree on regions.txt",
        WAIT,
    );
    tui.key('s');
    tui.wait_pred(
        after_ack("staged regions.txt", |screen| !screen.contains("VISUAL")),
        "whole-file s stages regions.txt",
        GIT_WAIT,
    );
    let cached = repo_cached(&workspace);
    assert!(
        cached.contains(REGIONS_ALPHA) && cached.contains(REGIONS_OMEGA),
        "whole-file stage puts both hunks in the index:\n{cached}"
    );
}

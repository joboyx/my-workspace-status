use crate::harness::{left_tree, PtySession};
use crate::seed::{compare_history_workspace, git_stdout, set_view_default, HISTORY_SUBJECTS};
use crate::support::{panes_tree_unfocused_diff_focused, tree_cursor_on, GIT_WAIT, WAIT};

/// `A` on a focused committed line opens the blame menu headed by that
/// line's annotation; `c` closes it and opens the line's commit versus
/// its parent in a new tab `app ↔ <sha7>^`.
///
/// `second.txt` gets an uncommitted line, so the diff's context line
/// `second note` is the middle commit's. A menu that does not show the
/// line's commit, a `c` that does not close it, or a tab of the wrong
/// commit cannot pass.
#[test]
fn pty_blame_menu_opens_commit_changes() {
    let (_root, workspace) = compare_history_workspace();
    set_view_default(&workspace, "diff", "inline");
    let [_, middle, newest] = HISTORY_SUBJECTS;
    let app = workspace.join("app");
    let sha = git_stdout(&app, &["rev-parse", "HEAD~1"]);
    let head = git_stdout(&app, &["rev-parse", "HEAD"]);
    std::fs::write(app.join("second.txt"), format!("{middle}\nextra line\n")).unwrap();
    let note = format!(" · {} · {middle}", &sha[..7]);
    let label = format!("app ↔ {}^", &sha[..7]);

    let mut tui = PtySession::open_size(&workspace, 160, 30);
    tui.wait_contains("app", WAIT);
    tui.search("second");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "second.txt") && screen.contains("+extra line"),
        "tree cursor on second.txt with its working-tree diff",
        GIT_WAIT,
    );
    tui.tab();
    tui.key('G');
    tui.key('k');
    tui.wait_pred(
        |screen| {
            panes_tree_unfocused_diff_focused(screen)
                && screen
                    .lines()
                    .any(|line| line.contains(&format!(" {middle}")) && line.contains(&note))
        },
        "the focused context line shows the middle commit",
        GIT_WAIT,
    );

    tui.key('A');
    tui.wait_pred(
        |screen| {
            screen
                .lines()
                .any(|line| line.contains("Blame ") && line.contains(&note))
                && screen.contains("open commit changes")
                && screen.contains("show commit in graph")
                && screen.contains("Esc cancel")
        },
        "A opens the menu headed by the line's blame",
        WAIT,
    );
    tui.key('c');
    tui.wait_pred(
        |screen| {
            let left = left_tree(screen);
            !screen.contains("Esc cancel")
                && screen
                    .lines()
                    .any(|line| line.contains("Workspace") && line.contains(&label))
                && screen.contains("COMMITTED")
                && left.contains("second.txt")
                && !left.contains("third.txt")
                && !screen.contains(newest)
        },
        "c closes the menu and a new tab shows the middle commit's own file",
        GIT_WAIT,
    );
    assert_eq!(git_stdout(&app, &["rev-parse", "HEAD"]), head);
}

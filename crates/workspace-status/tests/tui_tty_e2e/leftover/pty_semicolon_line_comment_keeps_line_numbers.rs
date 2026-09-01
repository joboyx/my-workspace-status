use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{
    documented_launch_first_paint, pane_unstaged_readme, right_pane, tree_cursor_on, GIT_WAIT, WAIT,
};

const BODY: &str = "keep-nums-e2e";

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
        && !screen.contains("MOVE")
        && !screen.contains("# Comments")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save")
        && !screen.contains("empty deletes")
        && !screen.contains("copied to clipboard")
}

fn right_diff_focused(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && pane_unstaged_readme(screen)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

/// Byte index of ` │ ` on each numbered leftover row (mark + numbers sit
/// immediately before that rule).
fn numbered_rule_cols(pane: &str) -> Vec<usize> {
    pane.lines().filter_map(|line| line.find(" │ ")).collect()
}

fn dirty_line_commented(screen: &str) -> bool {
    overlay_closed(screen)
        && pane_unstaged_readme(screen)
        && screen.contains('"')
        && screen.contains("comment saved")
        && tree_cursor_on(screen, "README.md")
}

/// `;` on a focused dirty file diff must not shift line numbers.
///
/// Hunt leftover: ` │ ` after the numbers stays in the same columns when
/// the ASCII `"` mark appears. A paint that stuffs the mark into the
/// number width, or a lock-in that ignores the leftover, is red.
#[test]
fn pty_semicolon_line_comment_keeps_line_numbers() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        right_diff_focused,
        "Tab focuses the dirty README diff (not a tree comment)",
        WAIT,
    );

    let before = tui.screen();
    let before_pane = right_pane(&before);
    let before_cols = numbered_rule_cols(&before_pane);
    assert!(
        !before_cols.is_empty(),
        "expected numbered leftover rows before a comment:\n{before_pane}"
    );

    tui.key(';');
    tui.wait_pred(
        comment_overlay,
        "; opens Comment overlay on the numbered dirty line",
        WAIT,
    );
    tui.keys(BODY);
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains(BODY),
        "typed body appears in the overlay",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        dirty_line_commented,
        "Enter saves: overlay gone, ASCII \" on the dirty diff, toast comment saved",
        GIT_WAIT,
    );

    let after = tui.screen();
    let after_pane = right_pane(&after);
    let after_cols = numbered_rule_cols(&after_pane);
    assert_eq!(
        before_cols, after_cols,
        "comment mark leftover must not shift │ after line numbers:\nbefore={before_pane}\nafter={after_pane}"
    );
    let marked = after_pane.lines().any(|line| {
        line.find(" │ ")
            .is_some_and(|at| line[..at].contains('"'))
    });
    assert!(
        marked,
        "ASCII \" leftover must occupy the reserved mark column:\n{after_pane}"
    );
}

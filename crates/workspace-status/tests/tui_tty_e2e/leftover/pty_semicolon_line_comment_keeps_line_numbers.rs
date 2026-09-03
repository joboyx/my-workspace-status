use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{pane_unstaged_readme, right_pane, tree_cursor_on, tree_has, GIT_WAIT, WAIT};

const FILE: &str = "keep-nums.rs";
const NEEDLE: &str = "KEEP-NUMS-10";
const BODY: &str = "keep-nums-e2e";

/// Untracked file whose line 10 fills the 2-column number gutter.
///
/// Daily README is 1-digit with floor 2, so stuffed `"1 │` and reserved
/// `" 1 │` keep `find(" │ ")` put. Line 10 is the same shape as
/// `comment_mark_does_not_shift_line_numbers`.
fn seed_two_digit_line(workspace: &Path) {
    let mut body = String::new();
    for i in 1..=12 {
        if i == 10 {
            body.push_str(&format!("{NEEDLE}\n"));
        } else {
            body.push_str(&format!("pad-{i}\n"));
        }
    }
    fs::write(workspace.join("app").join(FILE), body).unwrap();
}

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

fn file_diff_focused(screen: &str) -> bool {
    tree_has(screen, FILE)
        && !tree_cursor_on(screen, FILE)
        && !tree_cursor_on(screen, "README.md")
        && screen.contains("NEW")
        && screen.contains(NEEDLE)
        && screen.contains("[workspace]")
        && !comment_overlay(screen)
}

fn needle_line(pane: &str) -> Option<&str> {
    pane.lines().find(|line| line.contains(NEEDLE))
}

/// Byte index of ` │ ` on the 2-digit leftover row (`KEEP-NUMS-10`).
fn needle_rule_col(pane: &str) -> Option<usize> {
    needle_line(pane).and_then(|line| line.find(" │ "))
}

/// Reserved blank: mark space plus a full 2-digit number (`▌ 10 │`).
/// Stuffed unmarked is `▌10 │` (no mark column).
fn reserved_blank_line10(pane: &str) -> bool {
    needle_line(pane).is_some_and(|line| line.contains("▌ 10 │") && !line.contains("▌10 │"))
}

/// Reserved mark on the filled number column (`▌"10 │`).
fn reserved_marked_line10(pane: &str) -> bool {
    needle_line(pane).is_some_and(|line| line.contains("▌\"10 │"))
}

fn line10_commented(screen: &str) -> bool {
    overlay_closed(screen)
        && screen.contains("NEW")
        && screen.contains("comment saved")
        && tree_has(screen, FILE)
        && !tree_cursor_on(screen, FILE)
        && reserved_marked_line10(&right_pane(screen))
}

/// `;` on a 2-digit numbered diff line must not shift line numbers.
///
/// Hunt leftover: ` │ ` after `10` stays put, and the row is reserved
/// `▌ 10 │` / `▌"10 │` rather than stuffed `▌10 │` / `"10 │` grown from
/// a 2-column number gutter. Daily 1-digit README cannot catch stuffing.
#[test]
fn pty_semicolon_line_comment_keeps_line_numbers() {
    let (_root, workspace) = daily_workspace();
    seed_two_digit_line(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| pane_unstaged_readme(screen) && tree_has(screen, FILE),
        "launch paints README; keep-nums.rs is in the tree",
        WAIT,
    );

    tui.search("keep-nums");
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, FILE)
                && !tree_cursor_on(screen, "README.md")
                && screen.contains("NEW")
                && screen.contains(NEEDLE)
                && !comment_overlay(screen)
        },
        "search focuses keep-nums.rs and loads the NEW 12-line diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        file_diff_focused,
        "Tab focuses the keep-nums.rs diff (not a tree comment)",
        WAIT,
    );

    tui.search(NEEDLE);
    tui.wait_pred(
        |screen| file_diff_focused(screen) && reserved_blank_line10(&right_pane(screen)),
        "diff search lands on line 10 leftover `▌ 10 │` (reserved blank, not stuffed `▌10 │`)",
        WAIT,
    );

    let before = tui.screen();
    let before_pane = right_pane(&before);
    let before_col = needle_rule_col(&before_pane).expect("line 10 leftover before comment");
    assert!(
        reserved_blank_line10(&before_pane),
        "reserved blank leftover must be `▌ 10 │`, not stuffed `▌10 │`:\n{before_pane}"
    );

    tui.key(';');
    tui.wait_pred(
        |screen| comment_overlay(screen) && screen.contains("keep-nums.rs:10"),
        "; opens Comment overlay on keep-nums.rs line 10",
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
        line10_commented,
        "Enter saves: overlay gone, leftover `▌\"10 │` on KEEP-NUMS-10, toast comment saved",
        GIT_WAIT,
    );

    let after = tui.screen();
    let after_pane = right_pane(&after);
    let after_col = needle_rule_col(&after_pane).expect("line 10 leftover after comment");
    assert_eq!(
        before_col, after_col,
        "comment mark leftover must not shift │ after the filled number column:\nbefore={before_pane}\nafter={after_pane}"
    );
    assert!(
        reserved_marked_line10(&after_pane),
        "ASCII \" leftover must occupy the reserved mark column on line 10:\n{after_pane}"
    );
}

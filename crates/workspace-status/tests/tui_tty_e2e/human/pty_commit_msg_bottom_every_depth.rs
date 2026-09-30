use crate::harness::{pane_body_start, PtySession};
use crate::seed::{daily_workspace, seed_multiline_message_repo, COMMIT_MSG_BODY};
use crate::support::{
    crumb_row, no_wrong_overlays, panes_files_focused, panes_files_focused_diff_unfocused,
    panes_files_unfocused_diff_focused, panes_graph_focused_files_unfocused,
    panes_tree_unfocused_graph_focused, right_of_split, right_pane, title_has_files, SETTLE_MS,
    WAIT,
};

const REPO: &str = "longmsg";
/// The commit's only file; one row in the commit-files list.
const FILE: &str = "wip.txt";
/// Prefix of the long `nnnn…` subject (graph list row and both footers).
const SUBJECT: &str = "nnnnnnnnnn";
/// Branch on the commit meta line (`<sha> · feature/long-msg · <author>`).
const META_BRANCH: &str = "long-msg";
/// Added line of the file diff at depth 2.
const DIFF_HUNK: &str = "@@ -0,0 +1 @@";

/// 0-based screen row of the first right-pane body line that `hit` accepts.
///
/// Uses the same rows as [`right_pane`]: skips the tab strip, the pane title
/// row, and the crumb and status rows, so chrome cannot match.
fn right_row(screen: &str, hit: impl Fn(&str) -> bool) -> Option<usize> {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    (pane_body_start(lines.len())..end).find(|&row| hit(&right_of_split(lines[row])))
}

fn right_row_of(screen: &str, needle: &str) -> Option<usize> {
    right_row(screen, |cells| cells.contains(needle))
}

/// Selected graph list row (focused `▌` or unfocused `▏`) on the long subject.
fn selected_subject_row(screen: &str) -> Option<usize> {
    right_row(screen, |cells| {
        (cells.contains('\u{258C}') || cells.contains('\u{258F}')) && cells.contains(SUBJECT)
    })
}

/// Commit meta line: `·` separators plus the branch name.
fn meta_row(screen: &str) -> Option<usize> {
    right_row(screen, |cells| {
        cells.contains(META_BRANCH) && cells.contains('\u{00B7}') && !cells.contains(FILE)
    })
}

/// `row` is painted, below `anchor`.
fn below(anchor: usize, row: Option<usize>) -> bool {
    row.is_some_and(|row| row > anchor)
}

/// Depth 0: the graph footer paints the body under the selected list row.
fn graph_msg_at_bottom(screen: &str) -> bool {
    let Some(selected) = selected_subject_row(screen) else {
        return false;
    };
    crumb_row(screen).contains(REPO)
        && !title_has_files(screen)
        && below(selected, right_row_of(screen, COMMIT_MSG_BODY))
        && no_wrong_overlays(screen)
}

/// Depth 1: the file list comes first, then subject, meta, and body.
///
/// `need_body` is false when the pane is too short for the body to fit
/// under the list; the meta line must still sit under the file row.
fn files_msg_at_bottom(screen: &str, need_body: bool) -> bool {
    let Some(file) = right_row_of(screen, FILE) else {
        return false;
    };
    let body = right_row_of(screen, COMMIT_MSG_BODY);
    let subject = right_row(screen, |cells| {
        cells.contains(SUBJECT) && !cells.contains(FILE)
    });
    title_has_files(screen)
        && below(file, meta_row(screen))
        && subject.is_none_or(|row| row > file)
        && body.is_none_or(|row| row > file)
        && (!need_body || body.is_some())
        && no_wrong_overlays(screen)
}

fn files_focused_expanded_at_bottom(screen: &str) -> bool {
    panes_files_focused(screen)
        && right_pane(screen).contains(SUBJECT)
        && files_msg_at_bottom(screen, true)
        && !crumb_row(screen).contains("msg off")
}

fn files_focused_collapsed_at_bottom(screen: &str) -> bool {
    panes_files_focused(screen)
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && crumb_row(screen).contains("msg off")
        && files_msg_at_bottom(screen, false)
}

/// Depth 2: the right pane is the file diff. No commit message is painted
/// there today; if one ever is, it must sit under the diff, not above it.
fn diff_without_msg_above(screen: &str) -> bool {
    let Some(hunk) = right_row_of(screen, DIFF_HUNK) else {
        return false;
    };
    let above = |row: Option<usize>| row.is_some_and(|row| row < hunk);
    panes_files_unfocused_diff_focused(screen)
        && right_pane(screen).contains(&format!("{REPO}/{FILE}"))
        && !above(right_row_of(screen, COMMIT_MSG_BODY))
        && !above(meta_row(screen))
        && no_wrong_overlays(screen)
}

/// The selected commit message sits at the bottom of the right pane at every
/// drill depth, like the graph selection footer.
///
/// Depth 0 (graph): body under the selected `nnnn` row. Depth 1 (Enter,
/// commit files): the `wip.txt` row comes first, then subject, meta line,
/// and body, both expanded and collapsed (`M`), and on short 80×16 and
/// 80×14 terminals where the file row must stay visible. Depth 2 (Enter, file
/// diff): the right pane is the diff; it carries no commit message, and none
/// may appear above the hunk. Esc walks back to depth 1 and depth 0 with the
/// message still at the bottom.
///
/// Live PTY 80×28. A header painted above the file list, a short terminal
/// that drops the file row for the message, or a message above the depth 2
/// diff cannot pass.
#[test]
fn pty_commit_msg_bottom_every_depth() {
    let (_root, workspace) = daily_workspace();
    seed_multiline_message_repo(&workspace, REPO);
    let mut tui = PtySession::open_size(&workspace, 80, 28);
    tui.wait_contains("README.md", WAIT);

    tui.search(REPO);
    tui.wait_pred(
        |screen| right_pane(screen).contains(SUBJECT),
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
    tui.wait_pred(
        |screen| panes_tree_unfocused_graph_focused(screen) && graph_msg_at_bottom(screen),
        "depth 0: body sits below the selected graph row",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        files_focused_expanded_at_bottom,
        "depth 1: wip.txt row above the subject, meta line, and body",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        files_focused_collapsed_at_bottom,
        "depth 1 collapsed: meta line stays below the wip.txt row",
        WAIT,
    );
    tui.key('M');
    tui.wait_pred(
        files_focused_expanded_at_bottom,
        "depth 1: second M expands the message below wip.txt again",
        WAIT,
    );

    tui.resize(80, 16);
    tui.wait_pred(
        |screen| {
            screen.lines().count() == 16
                && panes_files_focused(screen)
                && files_msg_at_bottom(screen, false)
        },
        "depth 1 at 80x16: wip.txt row stays visible, meta line below it",
        WAIT,
    );
    tui.resize(80, 14);
    tui.wait_pred(
        |screen| {
            screen.lines().count() == 14
                && panes_files_focused(screen)
                && files_msg_at_bottom(screen, false)
        },
        "depth 1 at 80x14: wip.txt row stays visible, meta line below it",
        WAIT,
    );
    tui.resize(80, 28);
    tui.wait_pred(
        |screen| screen.lines().count() == 28 && files_focused_expanded_at_bottom(screen),
        "depth 1 back at 80x28: expanded message below wip.txt",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        diff_without_msg_above,
        "depth 2: file diff on the right, no commit message above the hunk",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        diff_without_msg_above,
        "depth 2 holds (no late header paint above the diff)",
        WAIT,
    );

    // First Esc may only clear the armed search chip; the next unfocuses.
    tui.esc();
    tui.wait_ms(120);
    if panes_files_unfocused_diff_focused(&tui.screen()) {
        tui.esc();
    }
    tui.wait_pred(
        panes_files_focused_diff_unfocused,
        "Esc unfocuses the depth 2 diff",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| panes_graph_focused_files_unfocused(screen) && files_msg_at_bottom(screen, true),
        "Esc pops to depth 1: message still below wip.txt",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| !title_has_files(screen) && graph_msg_at_bottom(screen),
        "Esc pops to depth 0: body still below the selected graph row",
        WAIT,
    );
}

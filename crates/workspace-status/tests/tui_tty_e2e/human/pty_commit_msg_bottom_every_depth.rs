use crate::harness::{left_tree, pane_body_start, PtySession};
use crate::seed::{daily_workspace, seed_multiline_message_repo, COMMIT_MSG_BODY};
use crate::support::{
    crumb_row, no_wrong_overlays, panes_files_focused, panes_files_focused_diff_unfocused,
    panes_files_unfocused_diff_focused, panes_graph_focused_files_unfocused,
    panes_tree_unfocused_graph_focused, right_pane, title_has_diff, title_has_files, SETTLE_MS,
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

#[derive(Clone, Copy)]
enum Pane {
    Left,
    Right,
}

/// Body cells of `pane`, one entry per screen row from [`pane_body_start`].
///
/// Uses the same rows as [`left_tree`] / [`right_pane`]: skips the tab strip,
/// the pane title row, and the crumb and status rows, so chrome cannot match.
fn pane_cells(screen: &str, pane: Pane) -> Vec<String> {
    let text = match pane {
        Pane::Left => left_tree(screen),
        Pane::Right => right_pane(screen),
    };
    text.lines().map(str::to_string).collect()
}

/// 0-based screen row of the first `pane` body line that `hit` accepts.
fn pane_row(screen: &str, pane: Pane, hit: impl Fn(&str) -> bool) -> Option<usize> {
    let start = pane_body_start(screen.lines().count());
    pane_cells(screen, pane)
        .iter()
        .position(|cells| hit(cells))
        .map(|i| start + i)
}

fn pane_row_of(screen: &str, pane: Pane, needle: &str) -> Option<usize> {
    pane_row(screen, pane, |cells| cells.contains(needle))
}

/// Selected graph list row (focused `▌` or unfocused `▏`) on the long subject.
fn selected_subject_row(screen: &str, pane: Pane) -> Option<usize> {
    pane_row(screen, pane, |cells| {
        (cells.contains('\u{258C}') || cells.contains('\u{258F}')) && cells.contains(SUBJECT)
    })
}

/// Commit meta line: `·` separators plus the branch name.
fn meta_row(screen: &str, pane: Pane) -> Option<usize> {
    pane_row(screen, pane, |cells| {
        cells.contains(META_BRANCH) && cells.contains('\u{00B7}') && !cells.contains(FILE)
    })
}

/// Footer rule: the pane row is all `▁` (`_` in ASCII mode) inside its
/// borders.
fn is_rule(cells: &str) -> bool {
    let inner = cells.trim_matches(|c: char| c == '\u{2502}' || c.is_whitespace());
    inner.chars().count() >= 10
        && (inner.chars().all(|c| c == '\u{2581}') || inner.chars().all(|c| c == '_'))
}

/// 0-based screen row of the footer rule on `pane`.
fn rule_row(screen: &str, pane: Pane) -> Option<usize> {
    pane_row(screen, pane, is_rule)
}

/// `row` is painted, below `anchor`.
fn below(anchor: usize, row: Option<usize>) -> bool {
    row.is_some_and(|row| row > anchor)
}

/// The graph footer paints the body under the selected list row of `pane`
/// (right at depth 0, left at depth 1), with the footer rule between the
/// list and the footer's subject row.
fn graph_msg_at_bottom(screen: &str, pane: Pane) -> bool {
    let Some(selected) = selected_subject_row(screen, pane) else {
        return false;
    };
    let rule = rule_row(screen, pane);
    let start = pane_body_start(screen.lines().count());
    let subject_under_rule = rule.is_some_and(|rule| {
        pane_cells(screen, pane)
            .get(rule + 1 - start)
            .is_some_and(|cells| cells.contains(SUBJECT))
    });
    crumb_row(screen).contains(REPO)
        && below(selected, rule)
        && subject_under_rule
        && rule.is_some_and(|rule| below(rule, pane_row_of(screen, pane, COMMIT_MSG_BODY)))
        && no_wrong_overlays(screen)
}

/// Depth 1: the left graph footer carries the message; the right files pane
/// is the plain file list with no subject, meta line, or body under it.
fn depth_1_msg_on_graph_only(screen: &str, need_body: bool) -> bool {
    let right = right_pane(screen);
    title_has_files(screen)
        && right.contains(FILE)
        && !right.contains(COMMIT_MSG_BODY)
        && !right.contains(SUBJECT)
        && meta_row(screen, Pane::Right).is_none()
        && (!need_body || graph_msg_at_bottom(screen, Pane::Left))
        && no_wrong_overlays(screen)
}

/// The last painted left-pane body row is the pane's last inner row (just
/// above the bottom border `└`), and it belongs to the message: at or below
/// the meta line, never the file row.
///
/// A blank row inside the message is fine; blank rows under it are not.
fn msg_pinned_to_left_bottom(screen: &str, file: usize, meta: usize) -> bool {
    let start = pane_body_start(screen.lines().count());
    let cells = pane_cells(screen, Pane::Left);
    let Some(border) = cells
        .iter()
        .position(|cells| cells.starts_with('\u{2514}'))
        .map(|i| start + i)
    else {
        return false;
    };
    let painted = (start..border).rev().find(|&row| {
        !cells[row - start]
            .trim_matches(|c: char| c == '\u{2502}' || c.is_whitespace())
            .is_empty()
    });
    painted.is_some_and(|row| row + 1 == border && row >= meta && row > file)
}

/// Depth 2: the left file list comes first, then the footer rule, title,
/// meta, subject, and body, with the message block pinned to the left pane
/// bottom. The right diff pane carries no commit message.
///
/// `need_body` is false when the pane is too short for the body to fit
/// under the list; the meta line must still sit under the file row, and the
/// last painted row must still be the pane's last inner row.
fn files_msg_at_left_bottom(screen: &str, need_body: bool) -> bool {
    let Some(file) = pane_row_of(screen, Pane::Left, FILE) else {
        return false;
    };
    let Some(meta) = meta_row(screen, Pane::Left) else {
        return false;
    };
    let body = pane_row_of(screen, Pane::Left, COMMIT_MSG_BODY);
    let subject = pane_row(screen, Pane::Left, |cells| {
        cells.contains(SUBJECT) && !cells.contains(FILE)
    });
    // The footer's first row is the rule, then the title row, then meta.
    let rule_on_top =
        rule_row(screen, Pane::Left).is_some_and(|rule| rule > file && rule + 2 == meta);
    title_has_diff(screen)
        && meta > file
        && rule_on_top
        && msg_pinned_to_left_bottom(screen, file, meta)
        && subject.is_none_or(|row| row > file)
        && body.is_none_or(|row| row > file)
        && (!need_body || body.is_some())
        && diff_without_msg(screen)
        && no_wrong_overlays(screen)
}

/// The right file diff paints the hunk and no commit message.
fn diff_without_msg(screen: &str) -> bool {
    pane_row_of(screen, Pane::Right, DIFF_HUNK).is_some()
        && right_pane(screen).contains(&format!("{REPO}/{FILE}"))
        && !right_pane(screen).contains(COMMIT_MSG_BODY)
        && meta_row(screen, Pane::Right).is_none()
}

fn diff_expanded_at_left_bottom(screen: &str) -> bool {
    panes_files_unfocused_diff_focused(screen)
        && files_msg_at_left_bottom(screen, true)
        && !crumb_row(screen).contains("msg off")
}

fn diff_collapsed_at_left_bottom(screen: &str) -> bool {
    panes_files_unfocused_diff_focused(screen)
        && !left_tree(screen).contains(COMMIT_MSG_BODY)
        && crumb_row(screen).contains("msg off")
        && files_msg_at_left_bottom(screen, false)
}

/// The selected commit message sits at the bottom of exactly one pane at
/// every drill depth, like the graph selection footer.
///
/// Both footers start with a full-width rule (`▁`, `_` in ASCII mode)
/// right under their list.
/// Depth 0 (graph): body under the selected `nnnn` row on the right. Depth
/// 1 (Enter, commit files): the left graph keeps the body under its
/// selected row; the right files pane is the `wip.txt` list with no
/// subject, meta line, or body. Depth 2 (Enter, file diff): the left pane
/// lists `wip.txt` first, then subject, meta line, and body, pinned to the
/// pane bottom (its last painted row is the pane's last inner row), both
/// expanded and collapsed (`M`), and on short 80×16 and 80×14 terminals
/// where the file row must stay visible. The right diff carries no commit
/// message. Esc walks back to depth 1 and depth 0 with the message on the
/// graph footer.
///
/// Live PTY 80×28. A message on the depth 1 files pane or the depth 2 diff,
/// a footer with no rule above it,
/// a header painted above the depth 2 file list, a message painted directly
/// under the list with blank rows below it, or a short terminal that drops
/// the file row for the message cannot pass.
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
        |screen| {
            panes_tree_unfocused_graph_focused(screen)
                && !title_has_files(screen)
                && graph_msg_at_bottom(screen, Pane::Right)
        },
        "depth 0: body sits below the selected graph row",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| panes_files_focused(screen) && depth_1_msg_on_graph_only(screen, true),
        "depth 1: body on the left graph footer, none on the files pane",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        |screen| {
            panes_files_focused(screen)
                && crumb_row(screen).contains("msg off")
                && !left_tree(screen).contains(COMMIT_MSG_BODY)
                && depth_1_msg_on_graph_only(screen, false)
        },
        "depth 1 collapsed: no body on either pane",
        WAIT,
    );
    tui.key('M');
    tui.wait_pred(
        |screen| panes_files_focused(screen) && depth_1_msg_on_graph_only(screen, true),
        "depth 1: second M expands the graph footer again",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        diff_expanded_at_left_bottom,
        "depth 2: wip.txt row above the subject, meta line, and body on the left",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        diff_expanded_at_left_bottom,
        "depth 2 holds (no late message paint on the diff)",
        WAIT,
    );

    tui.key('M');
    tui.wait_pred(
        diff_collapsed_at_left_bottom,
        "depth 2 collapsed: meta line stays below the wip.txt row",
        WAIT,
    );
    tui.key('M');
    tui.wait_pred(
        diff_expanded_at_left_bottom,
        "depth 2: second M expands the message below wip.txt again",
        WAIT,
    );

    tui.resize(80, 16);
    tui.wait_pred(
        |screen| {
            screen.lines().count() == 16
                && panes_files_unfocused_diff_focused(screen)
                && files_msg_at_left_bottom(screen, false)
        },
        "depth 2 at 80x16: wip.txt row stays visible, meta line below it",
        WAIT,
    );
    tui.resize(80, 14);
    tui.wait_pred(
        |screen| {
            screen.lines().count() == 14
                && panes_files_unfocused_diff_focused(screen)
                && files_msg_at_left_bottom(screen, false)
        },
        "depth 2 at 80x14: wip.txt row stays visible, meta line below it",
        WAIT,
    );
    tui.resize(80, 28);
    tui.wait_pred(
        |screen| screen.lines().count() == 28 && diff_expanded_at_left_bottom(screen),
        "depth 2 back at 80x28: expanded message below wip.txt",
        WAIT,
    );

    // First Esc may only clear the armed search chip; the next unfocuses.
    tui.esc();
    tui.wait_ms(120);
    if panes_files_unfocused_diff_focused(&tui.screen()) {
        tui.esc();
    }
    tui.wait_pred(
        |screen| {
            panes_files_focused_diff_unfocused(screen) && files_msg_at_left_bottom(screen, true)
        },
        "Esc unfocuses the depth 2 diff: message still below wip.txt",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            panes_graph_focused_files_unfocused(screen) && depth_1_msg_on_graph_only(screen, true)
        },
        "Esc pops to depth 1: body on the graph footer, none on the files pane",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| !title_has_files(screen) && graph_msg_at_bottom(screen, Pane::Right),
        "Esc pops to depth 0: body still below the selected graph row",
        WAIT,
    );
}

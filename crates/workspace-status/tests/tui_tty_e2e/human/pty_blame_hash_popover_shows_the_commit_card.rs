use crate::harness::PtySession;
use crate::seed::{compare_history_workspace, git_stdout, set_view_default, HISTORY_SUBJECTS};
use crate::support::{graph_cursor_on, graph_pane_focused, GIT_WAIT, WAIT};

/// Catalog meaning of the commit hash: the first line of the hash card.
const COMMIT_MEANING: &str = "Commit: full id, message, parents, refs, and dates";
/// Footer of a pinned popover. A peek has none.
const PINNED_FOOTER: &str = "y copy line · Enter run · Esc close";

/// The short hash of the line-blame note on the focused diff row: the
/// row that paints `+<subject>` and `note`. 0-based cell.
fn note_hash_cell(screen: &str, subject: &str, note: &str) -> Option<(u16, u16)> {
    let added = format!("+{subject}");
    screen.lines().enumerate().find_map(|(row, line)| {
        if !line.contains(&added) {
            return None;
        }
        // The note starts ` · <sha7> · `: the hash follows the first ` · `.
        let at = line.find(note)? + " · ".len();
        // ASCII glyph mode: one column per char.
        Some((line[..at].chars().count() as u16, row as u16))
    })
}

/// The short hash in a line-blame note pins that commit's card.
///
/// Docs: the line-blame note's author, age, and short hash are commit
/// popover hits. A click on the hash pins the hash card: the full id and
/// both dates, from the loaded graph or, for a commit off it, from a
/// lookup off the event thread (`loading…` until it lands). Esc closes it.
///
/// Live PTY on the middle commit's own diff (palette Diff commit vs
/// parent), inline so the note is not cut. A click on the note's short
/// hash pins the card with the blamed commit's full 40-char id
/// (`rev-parse HEAD~1`) and `committed`. Esc closes it. A card of the
/// wrong commit, or one that stays on `loading…`, cannot pass.
#[test]
fn pty_blame_hash_popover_shows_the_commit_card() {
    let (_root, workspace) = compare_history_workspace();
    set_view_default(&workspace, "diff", "inline");
    let [_, middle, newest] = HISTORY_SUBJECTS;
    let sha = git_stdout(&workspace.join("app"), &["rev-parse", "HEAD~1"]);
    assert_eq!(sha.len(), 40, "full sha: {sha}");
    let note = format!(" · {} · {middle}", &sha[..7]);
    let annotated = |screen: &str| {
        screen
            .lines()
            .any(|line| line.contains(&format!("+{middle}")) && line.contains(&note))
    };
    let pinned = |screen: &str| {
        screen.contains(COMMIT_MEANING)
            && screen.contains(&sha)
            && screen.contains("committed")
            && screen.contains(PINNED_FOOTER)
    };
    let closed = |screen: &str| !screen.contains(COMMIT_MEANING) && !screen.contains(PINNED_FOOTER);

    let mut tui = PtySession::open_size(&workspace, 160, 30);
    tui.wait_contains("app", WAIT);
    tui.search("app");
    tui.wait_contains("feature/history", GIT_WAIT);
    tui.tab();
    tui.wait_pred(
        |screen| graph_pane_focused(screen) && screen.contains(newest) && screen.contains(middle),
        "Tab focuses the app graph with the history commits",
        GIT_WAIT,
    );
    tui.search(middle);
    tui.wait_pred(
        |screen| graph_cursor_on(screen, middle) && !graph_cursor_on(screen, newest),
        "graph cursor on the middle commit",
        WAIT,
    );
    tui.key(':');
    tui.keys("vs parent");
    tui.wait_contains("Diff commit vs parent in new tab", WAIT);
    tui.enter();
    tui.wait_pred(
        |screen| screen.contains("COMMITTED") && screen.contains(&format!("+{middle}")),
        "the middle commit's diff opens in a compare tab",
        GIT_WAIT,
    );
    tui.tab();
    tui.key('G');
    tui.wait_pred(annotated, "focused added line shows its commit", GIT_WAIT);
    let (col, row) = note_hash_cell(&tui.screen(), middle, &note)
        .unwrap_or_else(|| panic!("note hash cell:\n{}", tui.screen()));

    tui.sgr_click(col, row);
    tui.wait_pred(
        pinned,
        "a click on the note's short hash pins the commit card",
        GIT_WAIT,
    );
    tui.esc();
    tui.wait_pred(closed, "Esc closes the card", WAIT);
}

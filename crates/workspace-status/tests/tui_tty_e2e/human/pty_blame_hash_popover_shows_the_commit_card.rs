use crate::harness::PtySession;
use crate::seed::{daily_workspace, git_stdout};
use crate::support::{idle_dirty_readme_unreviewed, merger_graph_left_unfocused, GIT_WAIT, WAIT};

/// Catalog meaning of the commit hash: the first line of the hash card.
const COMMIT_MEANING: &str = "Commit: full id, message, parents, refs, and dates";
/// Footer of a pinned popover. A peek has none.
const PINNED_FOOTER: &str = "y copy line · Enter run · Esc close";
/// The committed line of `app/README.md` on the daily seed.
const COMMITTED_LINE: &str = "# app";

/// The short hash of the line-blame note on the focused diff row: the
/// row that paints [`COMMITTED_LINE`] and `note`. 0-based cell.
fn note_hash_cell(screen: &str, note: &str) -> Option<(u16, u16)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        if !line.contains(COMMITTED_LINE) {
            return None;
        }
        // The note starts ` · <sha7> · `: the hash follows the first ` · `.
        let at = line.find(note)? + " · ".len();
        // ASCII glyph mode: one column per char.
        Some((line[..at].chars().count() as u16, row as u16))
    })
}

/// The short hash in a line-blame note pins that commit's card, loaded
/// off the event thread.
///
/// Docs: the line-blame note's author, age, and short hash are commit
/// popover hits. A click on the hash pins the hash card. A commit off the
/// loaded graph (another checkout, or past the window) shows `loading…`
/// until one lookup on the blocking pool lands; then the card shows the
/// full id and both dates. Esc closes it.
///
/// Live PTY on the daily seed: `j` loads the `merger` graph, `k` goes back
/// to `app/README.md`, so the blamed `app` commit is in no loaded graph
/// and only the lookup can fill the card. Tab focuses the diff, `G` then
/// `k` put the cursor on the committed `# app` line, whose note names the
/// `seed app` commit. A click on the note's short hash pins the card with
/// that commit's full 40-char id (`rev-parse HEAD` of `app`) and
/// `committed`. Esc closes it. A card of the wrong commit, or one that
/// stays on `loading…` (no `committed` row), cannot pass.
#[test]
fn pty_blame_hash_popover_shows_the_commit_card() {
    let (_root, workspace) = daily_workspace();
    let sha = git_stdout(&workspace.join("app"), &["rev-parse", "HEAD"]);
    assert_eq!(sha.len(), 40, "full sha: {sha}");
    let note = format!(" · {} · seed app", &sha[..7]);
    let annotated = |screen: &str| {
        screen
            .lines()
            .any(|line| line.contains(COMMITTED_LINE) && line.contains(&note))
    };
    let pinned = |screen: &str| {
        screen.contains(COMMIT_MEANING)
            && screen.contains(&sha)
            && screen.contains("committed")
            && screen.contains(PINNED_FOOTER)
    };
    let closed = |screen: &str| !screen.contains(COMMIT_MEANING) && !screen.contains(PINNED_FOOTER);

    let mut tui = PtySession::open_size(&workspace, 120, 40);
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "first paint: cursor on the dirty README",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j loads the merger graph",
        GIT_WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "k goes back to the dirty app README",
        GIT_WAIT,
    );
    tui.tab();
    tui.key('G');
    tui.key('k');
    tui.wait_pred(annotated, "the committed line shows its commit", GIT_WAIT);
    let (col, row) = note_hash_cell(&tui.screen(), &note)
        .unwrap_or_else(|| panic!("note hash cell:\n{}", tui.screen()));

    tui.sgr_click(col, row);
    tui.wait_pred(
        pinned,
        "a click on the note's short hash pins the looked-up commit card",
        GIT_WAIT,
    );
    tui.esc();
    tui.wait_pred(closed, "Esc closes the card", WAIT);
}

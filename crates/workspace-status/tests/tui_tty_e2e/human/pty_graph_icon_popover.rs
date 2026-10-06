use crate::harness::PtySession;
use crate::seed::daily_workspace;
use crate::support::{idle_dirty_readme_unreviewed, merger_graph_left_unfocused, GIT_WAIT, WAIT};

/// SGR pointer motion with no button held (`3 | 32`, any-event tracking).
const SGR_POINTER_MOVE: u8 = 3 | 32;
/// Catalog meanings: the first line of each graph node section.
const STASH_MEANING: &str = "Stash (one-node side-branch tip)";
const HEAD_MEANING: &str = "HEAD commit";
/// Footer of a pinned popover. A peek has none.
const PINNED_FOOTER: &str = "y copy line · Enter run · Esc close";

/// 0-based cell of the ASCII node `node` in the graph gutter of the line
/// whose label contains `label`.
fn node_cell(screen: &str, label: &str, node: char) -> Option<(u16, u16)> {
    screen.lines().enumerate().find_map(|(row, line)| {
        let split = ["││", "┐┌", "┘└"]
            .iter()
            .find_map(|sep| line.find(sep).map(|at| at + sep.len()))?;
        let at = split + line[split..].find(label)?;
        let node_at = split + line[split..at].rfind(node)?;
        // ASCII glyph mode: one column per char.
        Some((line[..node_at].chars().count() as u16, row as u16))
    })
}

fn stash_peek(screen: &str) -> bool {
    screen.contains(STASH_MEANING)
        && screen.contains("Apply stash")
        && screen.contains("Pop stash")
        && !screen.contains(PINNED_FOOTER)
}

fn stash_pinned(screen: &str) -> bool {
    screen.contains(STASH_MEANING)
        && screen.contains("Drop stash")
        && screen.contains(PINNED_FOOTER)
}

fn head_pinned(screen: &str) -> bool {
    screen.contains(HEAD_MEANING)
        && screen.contains("(merge)")
        && screen.contains("Create branch at commit")
        && screen.contains(PINNED_FOOTER)
        && !screen.contains(STASH_MEANING)
}

fn closed(screen: &str) -> bool {
    !screen.contains(STASH_MEANING) && !screen.contains(PINNED_FOOTER)
}

/// Graph nodes peek and pin like tree icons; `gh` covers the focused graph
/// row.
///
/// Docs: every graph node (commit, HEAD, stash), the uncommitted and
/// worktree glyphs, and the comment mark have a popover. Rails are chrome.
/// A click on a node selects its row and pins the popover; `gh` pins the
/// popover of every icon on the focused graph row.
///
/// Live PTY on the daily seed, ASCII glyphs: `j` loads the `merger` graph.
/// Pointer rest on the stash node `s` peeks the stash section (meaning,
/// Apply / Pop stash, no footer). A click pins it (footer, Drop stash). Esc
/// closes it. A click on the HEAD node `@` of the merge commit pins the
/// HEAD section (meaning, `(merge)` parents, Create branch). Esc closes it,
/// and `gh` on that focused row pins it again.
#[test]
fn pty_graph_icon_popover_peeks_pins_and_gh_on_nodes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_size(&workspace, 120, 40);
    tui.wait_pred(
        idle_dirty_readme_unreviewed,
        "first paint: cursor on the dirty README",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            merger_graph_left_unfocused(screen)
                && node_cell(screen, "WIP on graph", 's').is_some()
                && node_cell(screen, "merge", '@').is_some()
        },
        "j loads the merger graph with its stash and HEAD nodes",
        GIT_WAIT,
    );
    let (col, row) = node_cell(&tui.screen(), "WIP on graph", 's')
        .unwrap_or_else(|| panic!("stash node cell:\n{}", tui.screen()));

    tui.sgr_mouse(SGR_POINTER_MOVE, col, row);
    tui.wait_pred(stash_peek, "pointer rest on s peeks the stash", WAIT);

    tui.sgr_click(col, row);
    tui.wait_pred(stash_pinned, "a click on s pins the stash popover", WAIT);
    tui.esc();
    tui.wait_pred(closed, "Esc closes the stash popover", WAIT);

    let (col, row) = node_cell(&tui.screen(), "merge", '@')
        .unwrap_or_else(|| panic!("HEAD node cell:\n{}", tui.screen()));
    tui.sgr_click(col, row);
    tui.wait_pred(head_pinned, "a click on @ pins the HEAD popover", WAIT);
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains(PINNED_FOOTER),
        "Esc closes the HEAD popover",
        WAIT,
    );
    tui.keys("gh");
    tui.wait_pred(head_pinned, "gh pins the focused HEAD row", WAIT);
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains(PINNED_FOOTER),
        "Esc closes the gh popover",
        WAIT,
    );
}

use std::fs;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, seed_many_commit_files, seed_tall_graph};
use crate::support::{
    graph_cursor_on, graph_pane_focused, page_file_body_visible, panes_files_focused,
    panes_tree_focused_diff_unfocused, panes_tree_unfocused_diff_focused, right_pane,
    seed_tree_page_files, title_has_files, tree_cursor_on, tree_has, tree_pane_focused, GIT_WAIT,
    WAIT,
};

const DIFF_FILE: &str = "keepmid-diff.rs";
const DIFF_TOP: &str = "keepmid-line-0";
const FILES_TOP: &str = "keepmid-00.txt";
const KEY_GAP_MS: u64 = 50;
const LEAVE_TOP_TRIES: usize = 48;

/// CSI-u unmodified letter (`CSI code ; 1 : 1 u` press, `: 3` release).
///
/// The live loop requested `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus event
/// types. Two ASCII `g` bytes (`PtySession::gg`) are a different path.
fn csi_u_letter(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

/// CSI-u `gg` chord: two `g` press+release pairs inside the 400ms window.
fn csi_u_gg(tui: &mut PtySession) {
    csi_u_letter(tui, 'g');
    csi_u_letter(tui, 'g');
}

/// CSI-u Enter (`CSI 13 ; 1 : 1 u` press, `: 3` release).
///
/// `PtySession::enter` sends CR (`\r`), which is a different path.
fn csi_u_enter(tui: &mut PtySession) {
    tui.csi_u(13, 1, 1);
    tui.csi_u(13, 1, 3);
}

fn help_lists_gg_g_top_bottom(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("gg   G")
        && screen.contains("top / bottom of focused")
}

fn right_cursor_is_first_body_row(screen: &str) -> bool {
    let pane = right_pane(screen);
    let body: Vec<&str> = pane.lines().skip(1).collect();
    body.first().is_some_and(|line| line.contains('\u{258C}'))
}

fn right_cursor_on(screen: &str, needle: &str) -> bool {
    right_pane(screen)
        .lines()
        .any(|line| line.contains('\u{258C}') && line.contains(needle))
}

fn send_j(tui: &mut PtySession) {
    tui.key('j');
    tui.wait_ms(KEY_GAP_MS);
}

/// First Esc may clear an armed pane search. A second Esc runs only while
/// the right pane still holds keyboard focus.
fn unfocus_right(
    tui: &mut PtySession,
    still_right: impl Fn(&str) -> bool,
    now_left: impl Fn(&str) -> bool,
    why: &str,
) {
    tui.esc();
    tui.wait_ms(120);
    if still_right(&tui.screen()) {
        tui.esc();
    }
    tui.wait_pred(now_left, why, WAIT);
}

fn leave_until(tui: &mut PtySession, still_at_top: impl Fn(&str) -> bool, why: &str) {
    for _ in 0..LEAVE_TOP_TRIES {
        if !still_at_top(&tui.screen()) {
            return;
        }
        send_j(tui);
    }
    panic!(
        "{why}: still at the top after {LEAVE_TOP_TRIES} j\n{}",
        tui.screen()
    );
}

fn seed_tall_diff(workspace: &std::path::Path) {
    let mut body = String::new();
    for i in 0..40 {
        body.push_str(&format!("keepmid-line-{i}\n"));
    }
    fs::write(workspace.join("app").join(DIFF_FILE), body).unwrap();
}

fn tree_at_workspace_root(screen: &str) -> bool {
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, DIFF_FILE)
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "page-29")
        && tree_has(screen, "workspace")
        && screen.contains("# workspace")
        && !tree_has(screen, "No updates")
        && !tree_has(screen, "page-29")
        && !page_file_body_visible(screen)
        && screen.contains("focus a repo for the graph")
        && screen.contains("? help")
        && screen.contains("focus right")
        && !screen.contains("[workspace]")
        && !screen.contains("UNSTAGED")
}

fn tree_on_readme_not_root(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && screen.contains("UNSTAGED")
        && tree_has(screen, "workspace")
        && !tree_has(screen, "No updates")
        && !tree_has(screen, "page-29")
        && panes_tree_focused_diff_unfocused(screen)
}

fn diff_at_first_row(screen: &str) -> bool {
    panes_tree_unfocused_diff_focused(screen)
        && right_cursor_is_first_body_row(screen)
        && right_pane(screen).contains(DIFF_TOP)
        && tree_cursor_on(screen, DIFF_FILE)
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "README.md")
}

fn graph_at_working_tree(screen: &str) -> bool {
    graph_pane_focused(screen)
        && graph_cursor_on(screen, "working tree")
        && right_pane(screen).contains("count 29")
        && !graph_cursor_on(screen, "count 0")
        && !graph_cursor_on(screen, "count 20")
        && tree_cursor_on(screen, "history")
}

fn files_at_first_row(screen: &str) -> bool {
    panes_files_focused(screen)
        && title_has_files(screen)
        && right_cursor_on(screen, FILES_TOP)
        && right_pane(screen).contains(FILES_TOP)
}

/// CSI-u `gg` jumps to the start of the focused list or file-diff.
///
/// Docs + MOVE: `gg` (second `g` within ~400ms) is the start of the
/// focused list or file-diff. Lone `g` expires with no move. The
/// viewport follows. Home / End are a different path. Two ASCII `g`
/// bytes (`PtySession::gg`) can stay green while this encoding is a
/// no-op.
///
/// Live loop requested event types, so each `g` is CSI-u press then
/// release. Cover Tab (file-diff) and CSI-u Enter (graph, then commit
/// files) so the right pane is actually focused. Each surface leaves
/// its unique top marker before `gg`. A no-op, PageDown, Home, or a
/// jump onto a mid-list row cannot pass.
#[test]
fn pty_gg_jumps_focused_list_or_file_diff() {
    let (_root, workspace) = daily_workspace();
    seed_tree_page_files(&workspace);
    seed_tall_diff(&workspace);
    seed_tall_graph(&workspace, "history");
    seed_many_commit_files(&workspace, "bundle", 40);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        tree_on_readme_not_root,
        "launch cursor is README; last tree rows stay below the fold",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        help_lists_gg_g_top_bottom,
        "help MOVE lists gg G as top/bottom of focused pane",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("MOVE") && tree_on_readme_not_root(screen),
        "Esc closes help so gg is a tree jump, not a help key",
        WAIT,
    );

    csi_u_letter(&mut tui, 'g');
    tui.wait_ms(500);
    tui.wait_pred(
        tree_on_readme_not_root,
        "lone CSI-u g expires with no move (gg would land on workspace)",
        WAIT,
    );

    csi_u_gg(&mut tui);
    tui.wait_pred(
        tree_at_workspace_root,
        "CSI-u gg jumps to the first tree row (a no-op stays on README; PgUp from here is still mid-list)",
        GIT_WAIT,
    );

    tui.search("keepmid-diff");
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, DIFF_FILE)
                && panes_tree_focused_diff_unfocused(screen)
                && right_pane(screen).contains(DIFF_TOP)
        },
        "search lands on the tall dirty file with its diff shown (left still focused)",
        GIT_WAIT,
    );
    tui.tab();
    tui.wait_pred(
        |screen| {
            panes_tree_unfocused_diff_focused(screen)
                && tree_cursor_on(screen, DIFF_FILE)
                && right_pane(screen).contains(DIFF_TOP)
                && right_pane(screen).contains('\u{258C}')
        },
        "Tab focuses the file-diff on the first row",
        WAIT,
    );
    leave_until(
        &mut tui,
        |screen| right_pane(screen).contains(DIFF_TOP),
        "j on a focused file-diff must leave keepmid-line-0",
    );
    tui.wait_pred(
        |screen| {
            panes_tree_unfocused_diff_focused(screen)
                && !right_pane(screen).contains(DIFF_TOP)
                && right_pane(screen).contains('\u{258C}')
        },
        "file-diff has left the unique top marker (a later no-op gg cannot pass)",
        WAIT,
    );
    csi_u_gg(&mut tui);
    tui.wait_pred(
        diff_at_first_row,
        "CSI-u gg on a Tab-focused file-diff jumps to the first row (a no-op stays mid-list)",
        WAIT,
    );

    unfocus_right(
        &mut tui,
        panes_tree_unfocused_diff_focused,
        |screen| panes_tree_focused_diff_unfocused(screen) && tree_cursor_on(screen, DIFF_FILE),
        "Esc returns keyboard focus to the tree on the tall file",
    );

    tui.search("history");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "history") && screen.contains("count 29"),
        "search lands on the tall graph repo",
        GIT_WAIT,
    );
    csi_u_enter(&mut tui);
    tui.wait_pred(
        graph_at_working_tree,
        "CSI-u Enter focuses the graph on working tree (row 0)",
        GIT_WAIT,
    );
    leave_until(
        &mut tui,
        |screen| {
            graph_cursor_on(screen, "working tree")
                || right_pane(screen).contains("working tree clean")
        },
        "j on a focused graph must leave working tree",
    );
    tui.wait_pred(
        |screen| {
            graph_pane_focused(screen)
                && !graph_cursor_on(screen, "working tree")
                && !right_pane(screen).contains("working tree clean")
        },
        "graph has left the unique top marker (a later no-op gg cannot pass)",
        WAIT,
    );
    csi_u_gg(&mut tui);
    tui.wait_pred(
        graph_at_working_tree,
        "CSI-u gg on an Enter-focused graph jumps to working tree (a no-op stays mid-list)",
        GIT_WAIT,
    );

    unfocus_right(
        &mut tui,
        graph_pane_focused,
        |screen| {
            !graph_pane_focused(screen)
                && tree_pane_focused(screen)
                && tree_cursor_on(screen, "history")
        },
        "Esc returns keyboard focus to the tree on history",
    );

    tui.search("bundle");
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "bundle") && screen.contains("keepmid-files-commit"),
        "search lands on the many-file repo",
        GIT_WAIT,
    );
    csi_u_enter(&mut tui);
    tui.wait_pred(
        |screen| graph_pane_focused(screen) && graph_cursor_on(screen, "working tree"),
        "CSI-u Enter focuses the bundle graph on working tree",
        GIT_WAIT,
    );
    send_j(&mut tui);
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "keepmid-files-commit")
                && !graph_cursor_on(screen, "working tree")
        },
        "j selects the many-file commit",
        WAIT,
    );
    csi_u_enter(&mut tui);
    tui.wait_pred(
        |screen| {
            panes_files_focused(screen)
                && title_has_files(screen)
                && right_pane(screen).contains(FILES_TOP)
                && right_pane(screen).contains('\u{258C}')
        },
        "CSI-u Enter drills to commit files on row 0",
        GIT_WAIT,
    );
    leave_until(
        &mut tui,
        |screen| right_pane(screen).contains(FILES_TOP),
        "j on the commit-file list must leave keepmid-00.txt",
    );
    tui.wait_pred(
        |screen| {
            panes_files_focused(screen)
                && !right_pane(screen).contains(FILES_TOP)
                && right_pane(screen).contains('\u{258C}')
        },
        "commit-file list has left the unique top marker (a later no-op gg cannot pass)",
        WAIT,
    );
    csi_u_gg(&mut tui);
    tui.wait_pred(
        files_at_first_row,
        "CSI-u gg on an Enter-focused commit-file list jumps to the first row (a no-op stays mid-list)",
        WAIT,
    );
}

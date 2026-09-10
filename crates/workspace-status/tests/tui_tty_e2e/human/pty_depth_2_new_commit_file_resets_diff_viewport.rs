use crate::harness::PtySession;
use crate::seed::{daily_workspace, seed_two_tall_commit_files};
use crate::support::{
    graph_cursor_on, panes_files_focused, panes_files_focused_diff_unfocused,
    panes_files_unfocused_diff_focused, panes_tree_unfocused_graph_focused, right_pane,
    title_has_diff, title_has_files, title_has_graph, tree_cursor_on, tree_has, GIT_WAIT,
    SETTLE_MS, WAIT,
};

const ALPHA: &str = "alpha.rs";
const BETA: &str = "beta.rs";
const ALPHA_TOP: &str = "alpha-line-0";
const COMMIT: &str = "tall-pair-scroll-reset";
const REPO: &str = "scrollbox";
/// Repeats after the first CSI-u press.
const PAN_REPEATS: usize = 40;
/// Gap so the input thread does not drain the held-nav backlog as one move.
const REPEAT_GAP_MS: u64 = 50;

/// CSI-u Enter (`CSI 13 ; 1 : 1 u` press, `: 3` release).
///
/// `PtySession::enter` sends CR (`\r`), which is a different path.
fn csi_u_enter(tui: &mut PtySession) {
    tui.csi_u(13, 1, 1);
    tui.csi_u(13, 1, 3);
}

/// CSI-u press, spaced Repeat, release. The input thread drains a held-nav
/// burst after each move, so Repeats must not be written as one flush.
fn csi_u_hold_letter(tui: &mut PtySession, letter: char, repeats: usize) {
    tui.letter_press(letter);
    tui.wait_ms(REPEAT_GAP_MS);
    for _ in 0..repeats {
        tui.letter_repeat(letter);
        tui.wait_ms(REPEAT_GAP_MS);
    }
    tui.csi_u(u32::from(letter.to_ascii_lowercase()), 1, 3);
}

fn right_selection_on_first_body_row(screen: &str) -> bool {
    let pane = right_pane(screen);
    let body: Vec<&str> = pane.lines().skip(1).collect();
    let first = body.first().copied().unwrap_or("");
    let rest_clear = body
        .iter()
        .skip(1)
        .all(|line| !line.contains('\u{258C}') && !line.contains('\u{258F}'));
    rest_clear && (first.contains('\u{258C}') || first.contains('\u{258F}'))
}

fn scrollbox_graph_loaded(screen: &str) -> bool {
    tree_cursor_on(screen, REPO)
        && tree_has(screen, REPO)
        && (screen.contains("working tree") || screen.contains(COMMIT))
        && !title_has_files(screen)
}

fn scrollbox_graph_focused(screen: &str) -> bool {
    panes_tree_unfocused_graph_focused(screen)
        && tree_has(screen, REPO)
        && !tree_cursor_on(screen, REPO)
        && (screen.contains("working tree") || screen.contains(COMMIT))
        && !title_has_files(screen)
}

fn tall_pair_commit_selected(screen: &str) -> bool {
    graph_cursor_on(screen, COMMIT)
        && !graph_cursor_on(screen, "working tree")
        && scrollbox_graph_focused(screen)
}

fn tall_pair_files_right(screen: &str) -> bool {
    panes_files_focused(screen)
        && title_has_files(screen)
        && title_has_graph(screen)
        && !title_has_diff(screen)
        && screen.contains(ALPHA)
        && screen.contains(BETA)
}

fn alpha_diff_right(screen: &str) -> bool {
    panes_files_unfocused_diff_focused(screen)
        && title_has_diff(screen)
        && title_has_files(screen)
        && !title_has_graph(screen)
        && right_pane(screen).contains(ALPHA)
        && !right_pane(screen).contains(BETA)
        && right_pane(screen).contains(ALPHA_TOP)
}

fn alpha_diff_left_top(screen: &str) -> bool {
    alpha_diff_right(screen) && !right_pane(screen).contains("pan ")
}

fn alpha_scrolled_off_top(screen: &str) -> bool {
    panes_files_unfocused_diff_focused(screen)
        && right_pane(screen).contains(ALPHA)
        && !right_pane(screen).contains(ALPHA_TOP)
}

fn alpha_panned(screen: &str) -> bool {
    panes_files_unfocused_diff_focused(screen)
        && right_pane(screen).contains(ALPHA)
        && right_pane(screen).contains("pan ")
}

fn files_list_left_focused(screen: &str) -> bool {
    panes_files_focused_diff_unfocused(screen)
        && title_has_files(screen)
        && title_has_diff(screen)
        && !title_has_graph(screen)
}

fn beta_diff_at_origin(screen: &str) -> bool {
    let right = right_pane(screen);
    panes_files_focused_diff_unfocused(screen)
        && title_has_diff(screen)
        && title_has_files(screen)
        && !title_has_graph(screen)
        && right.contains(BETA)
        && !right.contains(ALPHA)
        && !right.contains("pan ")
        && !right.contains('█')
        && right_selection_on_first_body_row(screen)
}

/// First Esc may clear an armed pane search. A second Esc runs only while
/// the right pane still holds keyboard focus, so the diff does not pop.
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

/// Switching the focused commit file resets the diff viewport to the origin.
///
/// Docs: a new commit-file diff starts at the top and left. `G` then `l`
/// leave the first file. Esc focuses the file list. `j` loads the next
/// file with no leftover pan chrome and no vertical scrollbar. A no-op
/// `G` / `l` / `j` cannot pass.
#[test]
fn pty_depth_2_new_commit_file_resets_diff_viewport() {
    let (_root, workspace) = daily_workspace();
    seed_two_tall_commit_files(&workspace);
    let mut tui = PtySession::open_size(&workspace, 80, 24);
    tui.wait_contains("README.md", WAIT);

    tui.search(REPO);
    tui.wait_pred(
        scrollbox_graph_loaded,
        "search lands on scrollbox and loads its graph",
        GIT_WAIT,
    );
    csi_u_enter(&mut tui);
    tui.wait_pred(
        scrollbox_graph_focused,
        "CSI-u Enter focuses the scrollbox graph",
        WAIT,
    );

    tui.search(COMMIT);
    tui.wait_pred(
        tall_pair_commit_selected,
        "search selects the tall-pair commit (not working tree)",
        GIT_WAIT,
    );
    csi_u_enter(&mut tui);
    tui.wait_pred(
        tall_pair_files_right,
        "CSI-u Enter opens the commit file list with alpha.rs and beta.rs",
        GIT_WAIT,
    );

    csi_u_enter(&mut tui);
    tui.wait_pred(
        alpha_diff_left_top,
        "CSI-u Enter opens the first commit file diff (alpha.rs at the origin)",
        WAIT,
    );

    tui.shift_letter('G');
    tui.wait_pred(
        alpha_scrolled_off_top,
        "G on the first commit diff leaves alpha-line-0 (a no-op G cannot pass)",
        WAIT,
    );

    csi_u_hold_letter(&mut tui, 'l', PAN_REPEATS);
    tui.wait_pred(
        alpha_panned,
        "CSI-u l pans the first commit diff (header shows pan N; a no-op l cannot pass)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        alpha_panned,
        "file-diff pan holds (not a flicker or chrome-only suffix)",
        WAIT,
    );

    unfocus_right(
        &mut tui,
        panes_files_unfocused_diff_focused,
        files_list_left_focused,
        "Esc leaves the commit-file list focused on the left (diff stays on the right)",
    );

    tui.key('j');
    tui.wait_pred(
        beta_diff_at_origin,
        "j loads beta.rs at the origin (no leftover pan, no vertical thumb, first-row cursor)",
        WAIT,
    );
}

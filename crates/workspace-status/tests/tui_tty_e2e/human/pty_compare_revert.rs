use std::fs;
use std::path::{Path, PathBuf};

use crate::harness::PtySession;
use crate::seed::{compare_regions_workspace, git_stdout, COMPARE_ADDED_FILE, REGIONS_OMEGA};
use crate::support::{
    compare_regions_diff_focused, crumb_row, open_compare_regions_diff, tree_cursor_on,
    type_palette_filter, GIT_WAIT, SETTLE_MS, VISUAL_KEY_GAP_MS, WAIT,
};

const RANGE_CONFIRM: &str =
    "Revert highlighted lines in regions.txt to the origin/main merge base?";
const FILE_CONFIRM: &str = "Revert regions.txt to the origin/main merge base?";
const ADDED_CONFIRM: &str = "Revert summary.txt to the origin/main merge base?";
const ADDED_FATE: &str = "added on HEAD → summary.txt will be deleted";
const DIRTY_REASON: &str = "regions.txt has uncommitted changes";
/// Worktree edit of `keep-b` for the dirty-file refusal.
const DIRTY_B: &str = "wt-dirty-b";

fn app(workspace: &Path) -> PathBuf {
    workspace.join("app")
}

/// Git snapshot of the checkout: HEAD, index vs HEAD, and `status`.
fn git_state(workspace: &Path) -> (String, String, String) {
    let repo = app(workspace);
    (
        git_stdout(&repo, &["rev-parse", "HEAD"]),
        git_stdout(&repo, &["diff", "--cached"]),
        git_stdout(&repo, &["status", "--porcelain"]),
    )
}

/// `path` in the compare merge base (`origin/main...HEAD`).
fn merge_base_blob(workspace: &Path, path: &str) -> String {
    let repo = app(workspace);
    let mb = git_stdout(&repo, &["merge-base", "origin/main", "HEAD"]);
    git_stdout(&repo, &["show", &format!("{mb}:{path}")])
}

fn worktree_file(workspace: &Path, path: &str) -> String {
    fs::read_to_string(app(workspace).join(path)).unwrap_or_default()
}

/// The diff cursor bar (`▌`) sits on the row that shows `needle`.
fn diff_cursor_on(screen: &str, needle: &str) -> bool {
    screen
        .lines()
        .any(|line| line.contains(needle) && line.contains('▌'))
}

fn press_j(tui: &mut PtySession, times: usize) {
    for _ in 0..times {
        tui.letter_press('j');
        tui.wait_ms(VISUAL_KEY_GAP_MS);
    }
}

fn confirm_open(screen: &str, title: &str) -> bool {
    screen.contains(title) && screen.contains("cancel") && !screen.contains("VISUAL")
}

fn confirm_closed(screen: &str) -> bool {
    !screen.contains("merge base?")
}

/// `V` over the ALPHA pair of the compare diff (rows 5-6).
fn highlight_alpha(tui: &mut PtySession) {
    press_j(tui, 5);
    tui.shift_letter('V');
    tui.wait_pred(
        |screen| screen.contains("VISUAL") && screen.contains("cancel highlight"),
        "Shift+V paints VISUAL on the compare diff",
        WAIT,
    );
    tui.wait_ms(VISUAL_KEY_GAP_MS);
    press_j(tui, 1);
    tui.wait_pred(
        |screen| screen.contains("VISUAL") && diff_cursor_on(screen, "ALPHA-NEW"),
        "the highlight covers ALPHA-OLD and ALPHA-NEW",
        WAIT,
    );
}

/// `V` over the first compare hunk, `x`, then `y`.
///
/// The confirm names the merge base and writes nothing. `y` puts
/// ALPHA-OLD back in the worktree and keeps OMEGA-NEW. HEAD and the index
/// do not move. Git and the file on disk are the oracle.
#[test]
fn pty_compare_v_x_y_reverts_range_to_merge_base() {
    let (_root, workspace) = compare_regions_workspace();
    let (head, cached, porcelain) = git_state(&workspace);
    assert_eq!(
        (cached.as_str(), porcelain.as_str()),
        ("", ""),
        "clean seed"
    );
    let committed = worktree_file(&workspace, "regions.txt");
    let mut tui = open_compare_regions_diff(&workspace);
    highlight_alpha(&mut tui);
    assert!(
        tui.screen().contains("revert to merge base"),
        "the VISUAL hint row offers x:\n{}",
        tui.screen()
    );
    assert!(
        !tui.screen().contains("stage / unstage"),
        "the VISUAL hint row drops s / u on a compare tab:\n{}",
        tui.screen()
    );

    tui.letter_press('x');
    tui.wait_pred(
        |screen| confirm_open(screen, RANGE_CONFIRM) && screen.contains("worktree only"),
        "x opens the range confirm that names the merge base",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert_eq!(
        worktree_file(&workspace, "regions.txt"),
        committed,
        "the confirm alone must not write"
    );

    tui.letter_press('y');
    tui.wait_pred(
        |screen| {
            confirm_closed(screen)
                && crumb_row(screen).contains("reverted range regions.txt to merge base")
        },
        "y reverts the range (status names the merge base)",
        GIT_WAIT,
    );
    let want = merge_base_blob(&workspace, "regions.txt").replace("OMEGA-OLD", REGIONS_OMEGA);
    assert_eq!(
        worktree_file(&workspace, "regions.txt").trim_end(),
        want.trim_end(),
        "only the ALPHA hunk goes back to the merge base"
    );
    let (head_after, cached_after, _) = git_state(&workspace);
    assert_eq!(head_after, head, "HEAD does not move");
    assert_eq!(cached_after, "", "the index stays untouched");
}

/// `x` on the open compare diff: `n` and Esc cancel; `y` restores the file
/// to its merge-base content. HEAD and the index do not move.
#[test]
fn pty_compare_x_restores_modified_file_to_merge_base() {
    let (_root, workspace) = compare_regions_workspace();
    let (head, _, _) = git_state(&workspace);
    let committed = worktree_file(&workspace, "regions.txt");
    let mut tui = open_compare_regions_diff(&workspace);

    for cancel in ["n", "Esc"] {
        tui.letter_press('x');
        tui.wait_pred(
            |screen| confirm_open(screen, FILE_CONFIRM) && screen.contains("worktree only"),
            "x on the compare diff opens the whole-file confirm",
            WAIT,
        );
        if cancel == "n" {
            tui.letter_press('n');
        } else {
            tui.esc();
        }
        tui.wait_pred(
            |screen| {
                confirm_closed(screen)
                    && compare_regions_diff_focused(screen)
                    && crumb_row(screen).contains("revert cancelled")
            },
            "n / Esc cancels the confirm",
            WAIT,
        );
        tui.wait_ms(SETTLE_MS);
        assert_eq!(
            worktree_file(&workspace, "regions.txt"),
            committed,
            "{cancel} must not write"
        );
        assert_eq!(git_state(&workspace).2, "", "{cancel} leaves git clean");
    }

    tui.letter_press('x');
    tui.wait_pred(
        |screen| confirm_open(screen, FILE_CONFIRM),
        "x reopens the whole-file confirm",
        WAIT,
    );
    tui.letter_press('y');
    tui.wait_pred(
        |screen| {
            confirm_closed(screen)
                && crumb_row(screen).contains("reverted regions.txt to merge base")
        },
        "y restores regions.txt from the merge base",
        GIT_WAIT,
    );
    assert_eq!(
        worktree_file(&workspace, "regions.txt").trim_end(),
        merge_base_blob(&workspace, "regions.txt").trim_end(),
        "regions.txt equals its merge-base content"
    );
    let (head_after, cached_after, _) = git_state(&workspace);
    assert_eq!(head_after, head, "HEAD does not move");
    assert_eq!(cached_after, "", "the index stays untouched");
}

/// `x` on the added file row: the confirm says the file will be deleted,
/// and `y` removes it from the worktree (index untouched).
#[test]
fn pty_compare_x_deletes_file_added_on_head() {
    let (_root, workspace) = compare_regions_workspace();
    assert!(app(&workspace).join(COMPARE_ADDED_FILE).exists());
    let mut tui = open_compare_regions_diff(&workspace);
    tui.tab();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "regions.txt"),
        "Tab focuses the compare file list on regions.txt",
        WAIT,
    );
    press_j(&mut tui, 1);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, COMPARE_ADDED_FILE),
        "j moves the file cursor to summary.txt",
        WAIT,
    );

    tui.letter_press('x');
    tui.wait_pred(
        |screen| confirm_open(screen, ADDED_CONFIRM) && screen.contains(ADDED_FATE),
        "x on the added file says it will be deleted",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert!(
        app(&workspace).join(COMPARE_ADDED_FILE).exists(),
        "the confirm alone must not delete"
    );
    tui.letter_press('y');
    tui.wait_pred(
        |screen| confirm_closed(screen) && crumb_row(screen).contains("deleted summary.txt"),
        "y deletes summary.txt",
        GIT_WAIT,
    );
    assert!(
        !app(&workspace).join(COMPARE_ADDED_FILE).exists(),
        "summary.txt is gone from the worktree"
    );
    assert_eq!(
        git_state(&workspace).1,
        "",
        "the delete stays out of the index"
    );
}

/// A dirty regions.txt refuses `x`, highlighted `x`, and both palette
/// Revert rows with the reason. Nothing is written.
#[test]
fn pty_compare_x_refuses_dirty_file() {
    let (_root, workspace) = compare_regions_workspace();
    let path = app(&workspace).join("regions.txt");
    let dirty = fs::read_to_string(&path)
        .unwrap()
        .replace("keep-b\n", &format!("{DIRTY_B}\n"));
    fs::write(&path, &dirty).unwrap();
    let before = git_state(&workspace);
    let mut tui = open_compare_regions_diff(&workspace);

    tui.letter_press('x');
    tui.wait_pred(
        |screen| crumb_row(screen).contains(DIRTY_REASON) && confirm_closed(screen),
        "x on a dirty file shows the reason and opens no confirm",
        WAIT,
    );

    tui.ctrl_letter('k');
    tui.wait_pred(
        |screen| screen.contains("Enter run"),
        "Ctrl-k opens the palette",
        WAIT,
    );
    type_palette_filter(&mut tui, "revert");
    tui.wait_pred(
        |screen| {
            screen.contains("❯ Revert")
                && screen.contains(&format!("Enter run · Esc close · {DIRTY_REASON}"))
        },
        "the palette Revert row is disabled with the reason",
        WAIT,
    );
    tui.enter();
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| screen.contains("Enter run") && screen.contains(DIRTY_REASON),
        "Enter on the disabled row keeps the palette",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("Enter run") && compare_regions_diff_focused(screen),
        "Esc closes the palette",
        WAIT,
    );

    highlight_alpha(&mut tui);
    assert!(
        !tui.screen().contains("revert to merge base"),
        "the VISUAL hint row hides x on a dirty file:\n{}",
        tui.screen()
    );
    tui.letter_press('x');
    tui.wait_pred(
        |screen| {
            screen.contains("VISUAL")
                && crumb_row(screen).contains(DIRTY_REASON)
                && confirm_closed(screen)
        },
        "highlighted x shows the reason and keeps the highlight",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert_eq!(fs::read_to_string(&path).unwrap(), dirty, "nothing written");
    assert_eq!(git_state(&workspace), before, "git state unchanged");
}

/// The worker checks the file again right before it writes.
///
/// The confirm opens on a clean file. The file changes on disk before `y`.
/// `y` then aborts with the reason and keeps the local edit.
#[test]
fn pty_compare_y_aborts_when_file_changes_after_confirm() {
    let (_root, workspace) = compare_regions_workspace();
    let path = app(&workspace).join("regions.txt");
    let mut tui = open_compare_regions_diff(&workspace);
    tui.letter_press('x');
    tui.wait_pred(
        |screen| confirm_open(screen, FILE_CONFIRM),
        "x opens the whole-file confirm on a clean file",
        WAIT,
    );
    let dirty = fs::read_to_string(&path)
        .unwrap()
        .replace("keep-b\n", &format!("{DIRTY_B}\n"));
    fs::write(&path, &dirty).unwrap();

    tui.letter_press('y');
    tui.wait_pred(
        |screen| {
            confirm_closed(screen)
                && crumb_row(screen).contains("revert aborted: regions.txt has uncommitted changes")
        },
        "y aborts on the worker because regions.txt changed",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    assert_eq!(
        fs::read_to_string(&path).unwrap(),
        dirty,
        "the edit is kept"
    );
    assert_eq!(git_state(&workspace).1, "", "the index stays untouched");
}

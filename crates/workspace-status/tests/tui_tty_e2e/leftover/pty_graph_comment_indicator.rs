use std::fs;
use std::path::Path;

use crate::harness::PtySession;
use crate::seed::{daily_workspace, git, seed_repo};
use crate::support::{
    documented_launch_first_paint, graph_cursor_on, merger_graph_drilled_right,
    merger_graph_left_unfocused, pane_unstaged_readme, right_of_split, right_pane, title_has_diff,
    title_has_files, tree_cursor_on, tree_line_containing, GIT_WAIT, WAIT,
};

const COMMIT_BODY: &str = "graph-commit-mark-e2e";
const FILE_BODY: &str = "graph-file-mark-e2e";
const WT_BODY: &str = "graph-wt-mark-e2e";
const PAIR_REPO: &str = "pair";
const PAIR_COMMIT: &str = "readme-lib-commit";
const README_FILE: &str = "README.md";
const LIB_FILE: &str = "lib.rs";

fn comment_overlay(screen: &str) -> bool {
    screen.contains("Comment")
        && screen.contains("body:")
        && screen.contains("Enter save")
        && screen.contains("empty deletes")
        && !screen.contains("MOVE")
}

fn overlay_closed(screen: &str) -> bool {
    !screen.contains("Enter save") && !screen.contains("empty deletes")
}

fn graph_row_line(screen: &str, needle: &str) -> Option<String> {
    screen.lines().find_map(|line| {
        let right = right_of_split(line);
        right.contains(needle).then_some(right)
    })
}

fn graph_row_has_ascii_comment(screen: &str, needle: &str) -> bool {
    graph_row_line(screen, needle).is_some_and(|line| line.contains('"'))
}

fn after_name(line: &str, name: &str) -> Option<String> {
    let at = line.find(name)?;
    Some(line[at + name.len()..].to_string())
}

fn left_after_name(screen: &str, name: &str) -> Option<String> {
    let line = tree_line_containing(screen, name)?;
    after_name(&line, name)
}

fn left_file_row_marked(screen: &str, name: &str) -> bool {
    left_after_name(screen, name).is_some_and(|after| after.contains('"'))
}

fn left_row_has_ascii_comment(screen: &str, needle: &str) -> bool {
    tree_line_containing(screen, needle).is_some_and(|line| line.contains('"'))
}

/// One commit lists `README.md` (M) and `src/lib.rs` (A), matching the
/// commit-file paint unit. Feature branch keeps the repo visible.
fn seed_readme_and_lib_commit(workspace: &Path) {
    seed_repo(workspace, PAIR_REPO, "main", false);
    let repo = workspace.join(PAIR_REPO);
    fs::create_dir_all(repo.join("src")).unwrap();
    fs::write(repo.join("src/lib.rs"), "pub fn n() {}\n").unwrap();
    fs::write(repo.join("README.md"), format!("# {PAIR_REPO}\nnote\n")).unwrap();
    git(&repo, &["add", "README.md", "src/lib.rs"]);
    git(&repo, &["commit", "-q", "-m", PAIR_COMMIT]);
    git(&repo, &["checkout", "-q", "-b", "feature/pair"]);
}

fn pair_files_on_right(screen: &str) -> bool {
    let right = right_pane(screen);
    overlay_closed(screen)
        && title_has_files(screen)
        && right.contains(README_FILE)
        && right.contains(LIB_FILE)
        && right.contains(PAIR_COMMIT)
        && !screen.contains("wip.txt")
}

fn files_cursor_on(screen: &str, name: &str) -> bool {
    graph_cursor_on(screen, name)
}

fn readme_commit_diff(screen: &str) -> bool {
    overlay_closed(screen)
        && (title_has_diff(screen) || screen.contains("@@"))
        && right_pane(screen).contains(README_FILE)
        && !right_pane(screen).contains(LIB_FILE)
        && left_after_name(screen, README_FILE).is_some()
        && left_after_name(screen, LIB_FILE).is_some()
        && !left_file_row_marked(screen, README_FILE)
        && !left_file_row_marked(screen, LIB_FILE)
}

fn readme_only_file_mark_on_left(screen: &str) -> bool {
    overlay_closed(screen)
        && left_after_name(screen, README_FILE).is_some()
        && left_after_name(screen, LIB_FILE).is_some()
        && left_file_row_marked(screen, README_FILE)
        && !left_file_row_marked(screen, LIB_FILE)
}

fn save_overlay_body(tui: &mut PtySession, body: &str) {
    tui.key(';');
    tui.wait_pred(comment_overlay, "; opens Comment overlay", WAIT);
    tui.keys(body);
    tui.enter();
    tui.wait_pred(
        |screen| overlay_closed(screen) && screen.contains("comment saved"),
        "Enter saves the comment",
        WAIT,
    );
}

fn merger_merge_commit_selected(screen: &str) -> bool {
    graph_cursor_on(screen, "merge")
        && !graph_cursor_on(screen, "WIP on graph")
        && !graph_cursor_on(screen, "working tree")
}

/// `;` on a graph commit paints ASCII `"` on that row, including the
/// selected cursor. Uncommented rows stay unmarked.
///
/// Docs: comment glyph is `ICON_COMMENT` (`"`). Graph selected cursor
/// stays `▌`. A cursor-column-only mark that hides under `▌`, a toast
/// with no row glyph, or a mark on stash / working tree is red.
#[test]
fn pty_graph_commit_comment_paints_mark() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph",
        GIT_WAIT,
    );
    tui.enter();
    tui.wait_pred(
        merger_graph_drilled_right,
        "Enter on merger focuses that graph",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "WIP on graph") && !graph_cursor_on(screen, "working tree")
        },
        "first j selects stash@{0}",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        merger_merge_commit_selected,
        "second j selects the merge commit",
        WAIT,
    );
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && !graph_row_has_ascii_comment(screen, "merge")
                && !graph_row_has_ascii_comment(screen, "WIP on graph")
                && !graph_row_has_ascii_comment(screen, "working tree")
        },
        "graph rows have no comment mark before ;",
        WAIT,
    );

    save_overlay_body(&mut tui, COMMIT_BODY);
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && merger_merge_commit_selected(screen)
                && graph_row_has_ascii_comment(screen, "merge")
                && graph_cursor_on(screen, "merge")
                && !graph_row_has_ascii_comment(screen, "WIP on graph")
                && !graph_row_has_ascii_comment(screen, "working tree")
        },
        "saved commit comment paints ASCII \" on the selected merge row",
        WAIT,
    );

    tui.key('k');
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && graph_cursor_on(screen, "WIP on graph")
                && !graph_cursor_on(screen, "merge")
                && graph_row_has_ascii_comment(screen, "merge")
                && !graph_row_has_ascii_comment(screen, "WIP on graph")
                && !graph_row_has_ascii_comment(screen, "working tree")
        },
        "unselected merge row keeps \"; stash and working tree stay unmarked",
        WAIT,
    );
}

/// A commit-file line comment paints `"` after `README.md` only.
///
/// Unit `commit_file_row_paints_comment_mark_when_file_has_comments`
/// pins `README.md` vs `src/lib.rs`. This leftover hunts that README row
/// on a live commit-file list (`j` from the `src` dir, then `lib.rs`).
/// Esc from the diff focuses the list. A mark on `lib.rs`, every file,
/// a different listed file, or a toast with no glyph after the name is
/// red.
#[test]
fn pty_graph_commit_file_comment_paints_mark() {
    let (_root, workspace) = daily_workspace();
    seed_readme_and_lib_commit(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the README file diff (graph drill has not run)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        merger_graph_left_unfocused,
        "j lands on merger and loads its graph",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, PAIR_REPO)
                && !tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, README_FILE)
                && !tree_cursor_on(screen, "app")
        },
        "j from merger lands on pair (not merger / README)",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && graph_cursor_on(screen, "working tree")
                && right_pane(screen).contains(PAIR_COMMIT)
                && !screen.contains("wip.txt")
        },
        "Enter on pair focuses that graph",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, PAIR_COMMIT)
                && !graph_cursor_on(screen, "working tree")
                && !graph_cursor_on(screen, "uncommitted")
        },
        "j selects the readme-lib-commit (not working tree)",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        pair_files_on_right,
        "Enter on readme-lib-commit lists README.md and lib.rs",
        GIT_WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            pair_files_on_right(screen)
                && files_cursor_on(screen, LIB_FILE)
                && !files_cursor_on(screen, README_FILE)
        },
        "j from src lands on lib.rs (not README.md)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        |screen| {
            pair_files_on_right(screen)
                && files_cursor_on(screen, README_FILE)
                && !files_cursor_on(screen, LIB_FILE)
                && !files_cursor_on(screen, "src")
        },
        "j from lib.rs hunts the README.md file row",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        readme_commit_diff,
        "Enter on README.md opens that numbered commit diff (lib.rs stays on the list)",
        WAIT,
    );

    save_overlay_body(&mut tui, FILE_BODY);
    tui.esc();
    tui.wait_pred(
        readme_only_file_mark_on_left,
        "Esc to the commit-file list paints ASCII \" after README.md only",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && left_row_has_ascii_comment(screen, PAIR_COMMIT)
                && tree_line_containing(screen, README_FILE).is_none()
                && tree_line_containing(screen, LIB_FILE).is_none()
                && !left_row_has_ascii_comment(screen, "working tree")
                && !left_row_has_ascii_comment(screen, "uncommitted")
        },
        "Esc to the graph paints ASCII \" on readme-lib-commit (file comments)",
        WAIT,
    );
}

/// A working-tree line comment paints `"` on the graph uncommitted row.
///
/// Launch is the dirty README. Tab comments that line. Esc, `k`, Enter
/// open app's graph. A no-op graph or a mark on every graph row is red.
#[test]
fn pty_graph_uncommitted_marks_worktree_line_comment() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        documented_launch_first_paint,
        "launch is the dirty README file diff",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && pane_unstaged_readme(screen)
                && overlay_closed(screen)
        },
        "Tab focuses the dirty README diff",
        WAIT,
    );
    save_overlay_body(&mut tui, WT_BODY);
    tui.esc();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "app")
        },
        "Esc returns to the left tree on README",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && !tree_cursor_on(screen, "README.md"),
        "k from README focuses app",
        WAIT,
    );
    tui.enter();
    tui.wait_pred(
        |screen| {
            overlay_closed(screen)
                && graph_cursor_on(screen, "uncommitted")
                && graph_row_has_ascii_comment(screen, "uncommitted")
                && !graph_row_has_ascii_comment(screen, "seed app")
        },
        "app graph uncommitted row paints ASCII \" for the README line comment",
        GIT_WAIT,
    );
}

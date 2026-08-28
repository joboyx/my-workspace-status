//! Additional real-TTY operator paths not claimed by `main.rs`.
//!
//! Fold (`h`/`l`, `z`, `zz` subtree), click-to-select, double-click Enter,
//! `gg`/`G`, Home/End, `n`/`N` pane search, PgUp/PgDn, Ctrl-u/d, graph `c`
//! create-branch, `r`, ignored repos, `e` editor, CSI-u `T` theme, plus other
//! session keys the help overlay lists that a person actually types. Same PTY
//! harness: bytes in, painted screen out.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::{
    assert_contains, op_finished, tree_has, tree_row_containing, GIT_WAIT, SETTLE_MS, WAIT,
};
use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{self, left_tree, PtySession, SGR_WHEEL_RIGHT};
use crate::seed::{
    daily_workspace, focus_workspace, seed_long_diff_file, seed_repo, worktree_workspace,
};
use workspace_status::update_check::UPDATE_PROMPT;

/// Depth-1 fold chevron column when the tree inner origin is x=1.
const TREE_DEPTH1_CHEVRON_COL: u16 = 4;
/// Label column past the chevron (same as the tree-hscroll setup click).
const TREE_LABEL_COL: u16 = 8;
/// Right pane on the default 140-col layout (tree fraction 0.4).
const RIGHT_PANE_COL: u16 = 90;
/// Live `last_click` window is 400ms. Wait past it so the next pair is a
/// fresh double-click, not a continuation of a setup click.
const DOUBLE_CLICK_EXPIRE_MS: u64 = 500;

/// `h`/`l` fold the No-updates group. `l` on a dirty file must not open it.
#[test]
fn pty_fold_h_l_toggles_no_updates_group() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| tree_has(screen, "No updates") && !tree_has(screen, "lib"),
        "lib stays under the folded No-updates group",
        WAIT,
    );

    tui.key('l');
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| !tree_has(screen, "lib"),
        "`l` on the dirty file must not expand No updates",
        WAIT,
    );

    tui.shift_letter('G');
    tui.wait_ms(SETTLE_MS);
    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib"),
        "l on No updates reveals lib",
        WAIT,
    );
    tui.key('h');
    tui.wait_pred(
        |screen| tree_has(screen, "No updates") && !tree_has(screen, "lib"),
        "h folds No updates and hides lib",
        WAIT,
    );
}

/// ASCII collapsed chevron (`>`) on the left-tree row that contains `name`.
fn tree_dir_collapsed(screen: &str, name: &str) -> bool {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(name))
        .is_some_and(|line| line.contains('>'))
}

/// `z` on the repo row hides its dirty file. Not Space (reviewed).
#[test]
fn pty_z_folds_focused_repo() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_pred(
        |screen| tree_has(screen, "README.md"),
        "README is visible before fold",
        WAIT,
    );
    tui.key('k');
    tui.wait_ms(SETTLE_MS);
    tui.key('z');
    tui.wait_pred(
        |screen| !tree_has(screen, "README.md"),
        "z on app hides README.md",
        WAIT,
    );
}

/// `zz` subtree-folds the focused repo. Nested leaf stays hidden after `l`.
///
/// First `z` toggles only this row. Second `z` within 400ms is
/// `toggleSubtree`. A missing chord, a late second `z`, or a no-op leaves
/// `zz-leaf.rs` visible once `l` opens the repo.
#[test]
fn pty_zz_toggles_subtree_not_only_row() {
    let (_root, workspace) = daily_workspace();
    fs::create_dir_all(workspace.join("app").join("src")).unwrap();
    fs::write(
        workspace.join("app").join("src").join("zz-leaf.rs"),
        "fn zz_leaf() {}\n",
    )
    .unwrap();

    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "zz-leaf.rs") && tree_has(screen, "README.md"),
        "nested leaf is visible before zz",
        WAIT,
    );

    tui.search("zz-leaf");
    tui.wait_contains("/zz-leaf", WAIT);
    tui.key('k');
    tui.wait_ms(SETTLE_MS);
    tui.key('k');
    tui.wait_ms(SETTLE_MS);

    tui.key('z');
    tui.wait_pred(
        |screen| !tree_has(screen, "zz-leaf.rs") && !tree_has(screen, "README.md"),
        "first z hides the repo children",
        WAIT,
    );
    tui.wait_contains("z…", WAIT);
    // Chord expiry does not redraw by itself. Wait past 400ms so the next
    // `z` is FoldToggle, not FoldToggleSubtree.
    tui.wait_ms(500);

    tui.zz();
    tui.wait_pred(
        |screen| !tree_has(screen, "zz-leaf.rs") && !tree_has(screen, "README.md"),
        "zz subtree-folds the repo (a missing chord would show the leaf)",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| {
            tree_has(screen, "README.md")
                && tree_has(screen, "src")
                && !tree_has(screen, "zz-leaf.rs")
                && tree_dir_collapsed(screen, "src")
        },
        "l opens the repo with src still folded (a second FoldToggle would show zz-leaf.rs)",
        WAIT,
    );
}

/// Armed `/` then `n` / `N` steps pane matches. `n` unfolds the next hit.
///
/// Three `main` matches so `N` cannot pass as wrap-`n`. Shift+`N` is CSI-u.
/// A no-op leaves `lib` folded, or stays on `lib` after `N`.
#[test]
fn pty_n_and_n_pane_next_prev() {
    let (_root, workspace) = daily_workspace();
    // Third clean `main` checkout so next and prev from `lib` diverge.
    seed_repo(&workspace, "tools", "main", false);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| tree_has(screen, "No updates") && !tree_has(screen, "lib"),
        "lib stays under the folded No-updates group",
        WAIT,
    );

    tui.search("main");
    tui.wait_contains("/main", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("/main")
                && !screen.contains("n next")
                && !tree_has(screen, "lib")
                && screen.contains("workspace › app")
                && screen.contains("Uncommitted changes")
        },
        "first /main match is dirty app; lib stays folded; n next hint is gone",
        GIT_WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |screen| {
            tree_has(screen, "lib")
                && screen.contains("workspace › lib")
                && screen.contains("Working tree clean")
                && !screen.contains("workspace › app")
                && !screen.contains("Uncommitted changes")
        },
        "n moves to lib and loads its graph (a no-op leaves lib folded)",
        GIT_WAIT,
    );

    tui.shift_letter('N');
    tui.wait_pred(
        |screen| {
            screen.contains("/main")
                && screen.contains("workspace › app")
                && screen.contains("Uncommitted changes")
                && !screen.contains("workspace › lib")
                && !screen.contains("workspace › tools")
        },
        "N returns to app (a no-op stays on lib; wrap-n would land on tools)",
        GIT_WAIT,
    );
}

/// CSI PageDown then PageUp jumps the tree by a viewport, not to the ends.
///
/// Launch focuses the first file (`README.md`). A file-focused breadcrumb
/// stays `workspace` (the repo crumb is omitted while the right pane is a
/// diff). A short PTY plus extra dirty files makes one page smaller than
/// the list. PageDown must scroll `README.md` off, load a later file's
/// diff body, and stay off the last rows. A no-op keeps the README diff.
/// `G` would show `page-29` / No updates.
#[test]
fn pty_pgup_pgdn_pages_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_page_files(&workspace);

    let mut tui = PtySession::open_size(&workspace, harness::COLS, 12);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "README.md")
                && !tree_has(screen, "page-29")
                && !page_file_body_visible(screen)
                && screen.contains("UNSTAGED")
        },
        "launch focuses README; last page files stay below the fold",
        GIT_WAIT,
    );

    tui.page_down();
    tui.wait_pred(
        |screen| {
            page_file_body_visible(screen)
                && screen.contains("NEW")
                && !tree_has(screen, "README.md")
                && !tree_has(screen, "page-29")
                && !tree_has(screen, "No updates")
        },
        "PageDown pages the tree (a no-op keeps README; G would show page-29)",
        GIT_WAIT,
    );

    tui.page_up();
    tui.wait_pred(
        |screen| {
            tree_has(screen, "README.md")
                && screen.contains("UNSTAGED")
                && !page_file_body_visible(screen)
                && !tree_has(screen, "page-29")
        },
        "PageUp returns to README (a no-op keeps the paged file)",
        GIT_WAIT,
    );
}

fn seed_tree_page_files(workspace: &Path) {
    let app = workspace.join("app");
    for i in 0..30 {
        fs::write(
            app.join(format!("page-{i:02}.txt")),
            format!("page-{i:02}-body\n"),
        )
        .unwrap();
    }
}

fn page_file_body_visible(screen: &str) -> bool {
    (0..30).any(|i| screen.contains(&format!("page-{i:02}-body")))
}

/// `G` is the last visible row; `gg` returns to the workspace root.
#[test]
fn pty_gg_and_g_jump_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);

    tui.shift_letter('G');
    tui.wait_ms(SETTLE_MS);
    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib"),
        "G then l opens No updates (G actually moved)",
        WAIT,
    );

    tui.gg();
    tui.wait_ms(SETTLE_MS);
    tui.key('h');
    tui.wait_pred(
        |screen| !tree_has(screen, "app") && !tree_has(screen, "lib"),
        "gg then h folds the workspace root (not only No updates)",
        WAIT,
    );
}

/// Left-tree cursor bar (`▌`) on the row that contains `needle`.
fn tree_cursor_on(screen: &str, needle: &str) -> bool {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(needle))
        .is_some_and(|line| line.contains('\u{258C}'))
}

/// Home / End jump the workspace tree. CSI `1~` / `4~` with event types.
///
/// Launch starts on README.md. End must land on No updates (last row).
/// Home must return to the workspace root. Breadcrumb stays the workspace
/// label on file, group, and root, so the claim is the cursor bar plus
/// fold. A no-op leaves the bar on README.md.
#[test]
fn pty_home_and_end_jump_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md") && !tree_cursor_on(screen, "No updates"),
        "launch cursor is the dirty file, not No updates",
        WAIT,
    );

    tui.end();
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "No updates") && !tree_cursor_on(screen, "README.md"),
        "End paints the cursor on No updates (a no-op stays on README)",
        WAIT,
    );
    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib"),
        "End then l opens No updates (End actually moved)",
        WAIT,
    );

    tui.home();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "README.md")
        },
        "Home paints the cursor on the workspace root (a no-op stays on No updates)",
        WAIT,
    );
    tui.key('h');
    tui.wait_pred(
        |screen| !tree_has(screen, "app") && !tree_has(screen, "lib"),
        "Home then h folds the workspace root (not only No updates)",
        WAIT,
    );
}

/// Left-click a repo row selects it and loads the graph. Setup clicks in
/// the hscroll test are not this claim.
#[test]
fn pty_click_selects_tree_row() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_contains("README.md", WAIT);
    let row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, row);
    tui.wait_contains_any(&["Working tree", "WIP on graph"], GIT_WAIT);
    tui.wait_absent("UNSTAGED", WAIT);
}

/// Click the fold chevron, not the label.
#[test]
fn pty_click_chevron_toggles_fold() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| tree_has(screen, "No updates") && !tree_has(screen, "lib"),
        "lib hidden before chevron click",
        WAIT,
    );
    let row = tree_row_containing(&tui.screen(), "No updates")
        .unwrap_or_else(|| panic!("No updates row:\n{}", tui.screen()));
    tui.sgr_click(TREE_DEPTH1_CHEVRON_COL, row);
    tui.wait_pred(
        |screen| tree_has(screen, "lib"),
        "chevron click expands No updates",
        WAIT,
    );
}

/// Click the right pane to focus it. Breadcrumb brackets the last segment.
#[test]
fn pty_click_right_pane_focuses() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| !screen.contains("[workspace]"),
        "left focus does not bracket the workspace crumb",
        WAIT,
    );
    tui.sgr_click(RIGHT_PANE_COL, 6);
    tui.wait_contains("[workspace]", WAIT);
}

/// 0-based screen row whose full line contains `needle` (right pane included).
fn screen_row_containing(screen: &str, needle: &str) -> Option<u16> {
    screen
        .lines()
        .enumerate()
        .find_map(|(i, line)| line.contains(needle).then_some(i as u16))
}

/// Two xterm SGR left press+release reports on the same cell.
///
/// The live loop does not decode a double-click button. It treats two
/// left Downs at the same cell within 400ms as Enter (`nav_enter`).
fn sgr_double_click(tui: &mut PtySession, col: u16, row: u16) {
    tui.sgr_click(col, row);
    tui.sgr_click(col, row);
}

/// Double-click is Enter on the hit row (help: `Enter dblclick`).
///
/// Docs: left Enter focuses right on the same stack; right Enter drills
/// graph → commit files → commit diff; a chevron double-click still folds
/// and must not Enter. A single click only selects. Keyboard Enter on the
/// graph stash is the drill oracle the mouse pair must match.
#[test]
fn pty_double_click_enters_on_hit_row() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| tree_has(screen, "No updates") && !tree_has(screen, "lib"),
        "lib stays under the folded No-updates group",
        WAIT,
    );

    let nu_row = tree_row_containing(&tui.screen(), "No updates")
        .unwrap_or_else(|| panic!("No updates row:\n{}", tui.screen()));
    sgr_double_click(&mut tui, TREE_DEPTH1_CHEVRON_COL, nu_row);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "lib")
                && tree_has(screen, "No updates")
                && !screen.contains("[workspace]")
                && !screen.contains("┌ files")
        },
        "chevron double-click folds once (a second toggle hides lib; Enter would focus right)",
        WAIT,
    );

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let merger_row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, merger_row);
    tui.wait_contains("WIP on graph", GIT_WAIT);
    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("┌ graph")
                && screen.contains("WIP on graph")
                && !screen.contains("[merger]")
                && !screen.contains("┌ files")
        },
        "single-click merger selects the graph only (left focus)",
        WAIT,
    );

    sgr_double_click(&mut tui, TREE_LABEL_COL, merger_row);
    tui.wait_pred(
        |screen| {
            screen.contains("[merger]")
                && screen.contains("WIP on graph")
                && screen.contains("┌ graph")
                && !screen.contains("┌ files")
                && !screen.contains("wip.txt")
        },
        "tree double-click is Enter on the hit repo: focus right, do not drill",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| screen.contains("WIP on graph") && !screen.contains("[merger]"),
        "Esc unfocuses without popping the graph",
        WAIT,
    );

    tui.tab();
    tui.wait_contains("Working tree", WAIT);
    tui.key('j');
    tui.wait_ms(SETTLE_MS);
    tui.enter();
    tui.wait_pred(
        |screen| {
            screen.contains("┌ files")
                && screen.contains("wip.txt")
                && screen.contains("[stash@{0}]")
        },
        "keyboard Enter on the stash row drills to commit files (oracle)",
        GIT_WAIT,
    );
    tui.esc();
    tui.esc();
    tui.wait_pred(
        |screen| {
            screen.contains("WIP on graph")
                && !screen.contains("┌ files")
                && !screen.contains("[stash@{0}]")
        },
        "Esc Esc returns to the graph after the keyboard oracle",
        WAIT,
    );

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let graph_row = screen_row_containing(&tui.screen(), "WIP on graph")
        .unwrap_or_else(|| panic!("graph WIP row:\n{}", tui.screen()));
    tui.sgr_click(RIGHT_PANE_COL, graph_row);
    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("WIP on graph")
                && !screen.contains("┌ files")
                && !screen.contains("wip.txt")
                && !screen.contains("[stash@{0}]")
        },
        "single-click the stash row selects it (no files drill)",
        WAIT,
    );

    sgr_double_click(&mut tui, RIGHT_PANE_COL, graph_row);
    tui.wait_pred(
        |screen| {
            screen.contains("┌ files")
                && screen.contains("wip.txt")
                && screen.contains("[stash@{0}]")
                && screen.contains("workspace › merger")
                && !screen.contains("[merger]")
        },
        "graph double-click matches keyboard Enter: drill to that stash's files",
        GIT_WAIT,
    );

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    let file_row = screen_row_containing(&tui.screen(), "wip.txt")
        .unwrap_or_else(|| panic!("wip.txt row:\n{}", tui.screen()));
    tui.sgr_click(RIGHT_PANE_COL, file_row);
    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("┌ files")
                && screen.contains("wip.txt")
                && !screen.contains("┌ diff")
                && !screen.contains("@@")
                && !screen.contains("+stash me")
        },
        "single-click the commit-file row stays on the files list",
        WAIT,
    );

    sgr_double_click(&mut tui, RIGHT_PANE_COL, file_row);
    tui.wait_pred(
        |screen| {
            screen.contains("┌ diff")
                && screen.contains("[wip.txt]")
                && screen.contains("@@")
                && screen.contains("+stash me")
        },
        "files double-click is Enter: open that file's commit diff",
        GIT_WAIT,
    );

    tui.wait_ms(DOUBLE_CLICK_EXPIRE_MS);
    sgr_double_click(&mut tui, RIGHT_PANE_COL, file_row);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("┌ diff")
                && screen.contains("[wip.txt]")
                && screen.contains("@@")
                && screen.contains("+stash me")
                && !screen.contains("┌ graph")
        },
        "double-click at the diff leaf is a no-op (still that diff)",
        WAIT,
    );
}

/// Graph `c` creates a ref at the focused commit (not a commit overlay).
#[test]
fn pty_graph_c_creates_branch_at_commit() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("/focusbox", WAIT);
    tui.tab();
    tui.wait_contains("keep-leaf-commit", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('/');
    tui.keys("keep-leaf-commit");
    tui.enter();
    tui.wait_contains("/keep-leaf-commit", WAIT);
    tui.wait_contains("create branch", WAIT);
    tui.wait_ms(SETTLE_MS);

    tui.key('c');
    tui.wait_contains("Create branch", WAIT);
    tui.wait_contains("at ", WAIT);
    tui.keys("e2e-at-commit");
    tui.enter();
    tui.wait_contains("created e2e-at-commit at", GIT_WAIT);
    tui.wait_absent("Create branch", WAIT);
}

/// `c` on a dirty file is a no-op. It must not open a commit overlay.
#[test]
fn pty_c_on_tree_file_is_not_commit() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    tui.key('c');
    tui.wait_ms(400);
    let screen = tui.screen();
    harness::assert_absent(&screen, "Create branch");
    harness::assert_absent(&screen, "commit message");
    assert_contains(&screen, "UNSTAGED");
}

/// `r` reloads the focused repo while watch is off.
#[test]
fn pty_r_refreshes_new_dirty_file() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("/README", WAIT);
    tui.wait_pred(
        |screen| !tree_has(screen, "r-live.txt"),
        "new file is absent before r",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    fs::write(workspace.join("app").join("r-live.txt"), "refresh me\n").unwrap();
    tui.key('r');
    tui.wait_contains("r-live.txt", GIT_WAIT);
}

/// `.` shows ignored `notes`, then hides it again.
#[test]
fn pty_dot_toggles_ignored_repos() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.wait_pred(
        |screen| !tree_has(screen, "notes"),
        "notes hidden until shown",
        WAIT,
    );
    tui.key('.');
    tui.wait_pred(
        |screen| tree_has(screen, "notes"),
        "dot shows ignored notes",
        WAIT,
    );
    tui.key('.');
    tui.wait_pred(
        |screen| !tree_has(screen, "notes"),
        "second dot hides notes",
        WAIT,
    );
}

/// `q` leaves immediately (not the Ctrl+C chord).
#[test]
fn pty_q_quits_immediately() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("app", WAIT);
    tui.key('q');
    tui.wait_exit(WAIT);
}

/// Help `/` is highlight-only. Enter must not arm pane search.
#[test]
fn pty_help_enter_does_not_arm_pane_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.key('?');
    tui.wait_contains("MOVE", WAIT);
    tui.key('/');
    tui.keys("quit");
    tui.wait_contains("Esc clears search", WAIT);
    tui.enter();
    tui.wait_ms(SETTLE_MS);
    tui.wait_contains("MOVE", WAIT);
    tui.wait_contains("HELP  /quit", WAIT);
    tui.esc();
    tui.wait_contains("MOVE", WAIT);
    tui.wait_contains("/ search help", WAIT);
    tui.esc();
    tui.wait_absent("MOVE", WAIT);
    let screen = tui.screen();
    harness::assert_absent(&screen, "/quit");
}

/// `x` opens the revert confirm; `n` cancels.
#[test]
fn pty_revert_confirm_n_cancels() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('x');
    tui.wait_contains("Revert ", WAIT);
    tui.wait_contains("tracked", WAIT);
    tui.key('n');
    tui.wait_absent("Revert ", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
}

/// `W` on a linked worktree asks, then removes.
#[test]
fn pty_worktree_w_remove_confirm() {
    let (_root, workspace) = worktree_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("linked-open");
    tui.wait_contains("/linked-open", WAIT);
    tui.wait_contains("feature/linked-open", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.shift_letter('W');
    tui.wait_contains("Remove worktree", WAIT);
    tui.key('y');
    tui.wait_contains("removed worktree", GIT_WAIT);
    tui.wait_pred(
        |screen| !tree_has(screen, "feature/linked-open"),
        "linked worktree row gone after remove",
        WAIT,
    );
}

/// Trackpad hscroll over the left pane pans a long file-diff.
#[test]
fn pty_left_pane_sgr_hscroll_pans_long_diff() {
    let (_root, workspace) = daily_workspace();
    seed_long_diff_file(&workspace, "unique-diffline.rs", DIFF_HSCROLL_TAIL);
    let mut tui = PtySession::open_size(&workspace, 80, 24);
    tui.search("unique-diffline");
    tui.wait_contains("/unique-diffline", WAIT);
    tui.wait_contains("unique-diffline.rs", WAIT);
    tui.wait_pred(
        |screen| screen.contains("nnnn") && !screen.contains(DIFF_HSCROLL_TAIL),
        "long diff tail is clipped before pan",
        WAIT,
    );
    let row = tree_row_containing(&tui.screen(), "unique-diffline")
        .unwrap_or_else(|| panic!("long diff file row:\n{}", tui.screen()));
    for _ in 0..80 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, row);
    }
    tui.wait_contains(DIFF_HSCROLL_TAIL, WAIT);
}

/// CSI-u Repeat of `j` keeps moving. A single press must not reach the end.
#[test]
fn pty_key_repeat_j_reaches_no_updates() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.gg();
    tui.wait_ms(SETTLE_MS);
    tui.letter_press('j');
    tui.wait_ms(80);
    for _ in 0..10 {
        tui.letter_repeat('j');
        tui.wait_ms(50);
    }
    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib"),
        "Repeat j must land on No updates so l reveals lib",
        WAIT,
    );
}

/// Picker `C` creates (and checks out) a branch at HEAD.
#[test]
fn pty_branch_picker_shift_c_creates() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("/focusbox", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('b');
    tui.wait_contains("Branch ", WAIT);
    tui.shift_letter('C');
    tui.wait_contains("Create branch", WAIT);
    tui.keys("e2e-from-picker");
    tui.enter();
    tui.wait_contains("created e2e-from-picker", GIT_WAIT);
}

/// `t` flips the tree/flat pill. `i` flips inline/split on a file diff.
#[test]
fn pty_t_and_i_toggle_view_modes() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains(" tree", WAIT);
    tui.key('t');
    tui.wait_contains("Flat paths", WAIT);
    tui.key('t');
    tui.wait_contains("Directory tree", WAIT);

    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    let was_split = tui.screen().contains("split");
    tui.key('i');
    if was_split {
        tui.wait_contains("inline", WAIT);
    } else {
        tui.wait_contains("split", WAIT);
    }
}

/// Painted chrome for one built-in theme. Surfaces / pills / headings are
/// unique across the cycle so a no-op or a skipped id cannot pass.
struct ThemeChrome {
    id: &'static str,
    toast: &'static str,
    surface: (u8, u8, u8),
    pill: (u8, u8, u8),
    heading: (u8, u8, u8),
}

/// Docs + help `T` cycle. Wraps after Catppuccin Mocha.
const THEME_CYCLE: [ThemeChrome; 5] = [
    ThemeChrome {
        id: "tokyo-night",
        toast: "theme: Tokyo Night",
        surface: (0x1a, 0x1b, 0x26),
        pill: (0x3d, 0x59, 0xa1),
        heading: (0x7d, 0xcf, 0xff),
    },
    ThemeChrome {
        id: "monokai",
        toast: "theme: Monokai",
        surface: (0x27, 0x28, 0x22),
        pill: (0x49, 0x48, 0x3e),
        heading: (0x66, 0xd9, 0xef),
    },
    ThemeChrome {
        id: "dracula",
        toast: "theme: Dracula",
        surface: (0x28, 0x2a, 0x36),
        pill: (0x44, 0x47, 0x5a),
        heading: (0x8b, 0xe9, 0xfd),
    },
    ThemeChrome {
        id: "gruvbox-dark",
        toast: "theme: Gruvbox Dark",
        surface: (0x28, 0x28, 0x28),
        pill: (0x50, 0x49, 0x45),
        heading: (0x83, 0xa5, 0x98),
    },
    ThemeChrome {
        id: "catppuccin-mocha",
        toast: "theme: Catppuccin Mocha",
        surface: (0x1e, 0x1e, 0x2e),
        pill: (0x45, 0x47, 0x5a),
        heading: (0x89, 0xdc, 0xeb),
    },
];

/// Tokyo Night graph lane 0 (`DEFAULT_LANE_COLORS[0]` / dir). Absent from
/// every other built-in palette, so a stuck Tokyo gutter after `T` fails.
const TOKYO_GRAPH_LANE0: (u8, u8, u8) = (0x7a, 0xa2, 0xf7);

fn theme_is_tree_not_flat(screen: &str) -> bool {
    screen.contains(" tree") && !screen.contains("Flat paths")
}

fn wait_theme_chrome(tui: &PtySession, theme: &ThemeChrome, expect_toast: bool) {
    tui.wait_pred(
        |screen| {
            theme_is_tree_not_flat(screen)
                && (!expect_toast || screen.contains(theme.toast))
                && THEME_CYCLE
                    .iter()
                    .filter(|other| other.toast != theme.toast)
                    .all(|other| !screen.contains(other.toast))
        },
        &format!(
            "{} chrome: tree pill, not Flat paths{}",
            theme.id,
            if expect_toast {
                format!(", toast `{}`", theme.toast)
            } else {
                String::new()
            }
        ),
        WAIT,
    );
    tui.wait_has_rgb(theme.surface.0, theme.surface.1, theme.surface.2, WAIT);
    tui.wait_has_rgb(theme.pill.0, theme.pill.1, theme.pill.2, WAIT);
    tui.wait_has_rgb(theme.heading.0, theme.heading.1, theme.heading.2, WAIT);
    for other in THEME_CYCLE.iter().filter(|other| other.id != theme.id) {
        assert!(
            !tui.has_rgb(other.surface.0, other.surface.1, other.surface.2),
            "{} surface must not remain after {}:\n{}",
            other.id,
            theme.id,
            tui.screen()
        );
        assert!(
            !tui.has_rgb(other.pill.0, other.pill.1, other.pill.2),
            "{} mode pill must not remain after {}:\n{}",
            other.id,
            theme.id,
            tui.screen()
        );
        assert!(
            !tui.has_rgb(other.heading.0, other.heading.1, other.heading.2),
            "{} heading must not remain after {}:\n{}",
            other.id,
            theme.id,
            tui.screen()
        );
    }
}

fn wait_graph_lanes_match_theme(tui: &PtySession, theme: &ThemeChrome) {
    if theme.id == "tokyo-night" {
        tui.wait_has_rgb(
            TOKYO_GRAPH_LANE0.0,
            TOKYO_GRAPH_LANE0.1,
            TOKYO_GRAPH_LANE0.2,
            WAIT,
        );
        return;
    }
    tui.wait_pred(
        |_| {
            !tui.has_rgb(
                TOKYO_GRAPH_LANE0.0,
                TOKYO_GRAPH_LANE0.1,
                TOKYO_GRAPH_LANE0.2,
            )
        },
        &format!(
            "graph lanes follow {} (Tokyo lane 0 {} gone)",
            theme.id, "#7aa2f7"
        ),
        WAIT,
    );
}

fn assert_no_theme_store(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let name = entry.unwrap().file_name();
        let s = name.to_string_lossy();
        assert!(
            !s.to_ascii_lowercase().contains("theme"),
            "`T` is session-only; must not write {}:\n{}",
            dir.join(&*s).display(),
            s
        );
    }
}

/// CSI-u Shift+T cycles the documented colour theme. Raw `'T'` / `'t'` are
/// different paths (`t` is tree/flat).
///
/// Help lists `T` cycle theme next to `t` flat/tree. Launch seed
/// `WS_STATUS_THEME` paints that id. Each Shift+T advances
/// Tokyo Night → Monokai → Dracula → Gruvbox Dark → Catppuccin Mocha →
/// Tokyo Night. Toast, surface, mode pill, heading, and graph lane 0 must
/// all match that id. A no-op, a skipped id, or lowercase `t` cannot pass.
/// There is no theme file.
#[test]
fn pty_shift_t_csi_u_cycles_theme() {
    let (_root, workspace) = daily_workspace();
    let config_home = workspace.join(".e2e-config");
    fs::create_dir_all(&config_home).unwrap();
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_THEME", "tokyo-night"),
            ("XDG_CONFIG_HOME", config_home.to_str().unwrap()),
        ],
    );
    tui.wait_contains(" tree", WAIT);
    tui.wait_contains("README.md", WAIT);

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("cycle theme")
                && screen.contains("flat / tree")
                && screen.contains("VIEW")
        },
        "help VIEW lists T cycle theme and t flat/tree as distinct rows",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("cycle theme") && theme_is_tree_not_flat(screen),
        "Esc closes help so Shift+T is not swallowed",
        WAIT,
    );

    tui.search("merger");
    tui.wait_contains("/merger", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("workspace › merger")
                && (screen.contains("Working tree") || screen.contains("Uncommitted"))
        },
        "merger graph is focused so lane colours are on screen",
        GIT_WAIT,
    );

    wait_theme_chrome(&tui, &THEME_CYCLE[0], false);
    wait_graph_lanes_match_theme(&tui, &THEME_CYCLE[0]);

    for step in 1..=THEME_CYCLE.len() {
        let theme = &THEME_CYCLE[step % THEME_CYCLE.len()];
        let before = tui.color_fingerprint();
        tui.shift_letter('T');
        wait_theme_chrome(&tui, theme, true);
        wait_graph_lanes_match_theme(&tui, theme);
        assert_ne!(
            tui.color_fingerprint(),
            before,
            "Shift+T step {step} ({}) must repaint cells; a toast-only no-op fails:\n{}",
            theme.id,
            tui.screen()
        );
    }

    assert_no_theme_store(&workspace.join(".e2e-state"));
    assert_no_theme_store(&config_home);

    let (_root2, seeded) = daily_workspace();
    let mut seeded_tui = PtySession::open_with_env(&seeded, &[("WS_STATUS_THEME", "dracula")]);
    seeded_tui.wait_contains(" tree", WAIT);
    wait_theme_chrome(&seeded_tui, &THEME_CYCLE[2], false);
    seeded_tui.shift_letter('T');
    wait_theme_chrome(&seeded_tui, &THEME_CYCLE[3], true);
}

/// `d` switches a clean non-default checkout.
#[test]
fn pty_d_switches_to_default_branch() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("focusbox");
    tui.wait_contains("/focusbox", WAIT);
    tui.wait_contains("feature/keep", WAIT);
    tui.wait_ms(SETTLE_MS);
    tui.key('d');
    tui.wait_pred(
        |screen| op_finished(screen, "Switched") || screen.contains("Switched 1 repo"),
        "Switched 1 repo without failure",
        GIT_WAIT,
    );
}

/// Extra dirty files so Ctrl-d ±5 cannot clamp to the last row (that is `G`).
fn seed_tree_half_page_files(workspace: &Path) {
    let app = workspace.join("app");
    for i in 0..10 {
        fs::write(
            app.join(format!("jump-{i:02}.txt")),
            format!("jump-{i:02}-body\n"),
        )
        .unwrap();
    }
}

/// CSI-u Ctrl-d then Ctrl-u jumps the tree ±5 rows. Not a viewport page.
///
/// Launch focuses `README.md`. Files sort as README then `jump-00`…
/// `jump-09`, so +5 lands on `jump-04.txt`. The cursor bar and the right
/// pane body must both move. A no-op stays on README. `G` would land on
/// No updates. File-to-file focus keeps the workspace breadcrumb.
#[test]
fn pty_ctrl_u_d_jumps_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_half_page_files(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !screen.contains("jump-04-body")
                && screen.contains("UNSTAGED")
        },
        "launch cursor is README; jump-04 is not focused",
        GIT_WAIT,
    );

    tui.ctrl_letter('d');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "jump-04.txt")
                && screen.contains("jump-04-body")
                && screen.contains("NEW")
                && !tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "jump-09.txt")
        },
        "Ctrl-d moves +5 to jump-04 (a no-op stays on README; G would hit No updates)",
        GIT_WAIT,
    );

    tui.ctrl_letter('u');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !screen.contains("jump-04-body")
        },
        "Ctrl-u returns to README (a no-op keeps jump-04)",
        GIT_WAIT,
    );
}

/// `Ctrl-o` paints the full-file marker on the focused diff.
#[test]
fn pty_ctrl_o_full_file_context() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.search("README");
    tui.wait_contains("UNSTAGED", WAIT);
    harness::assert_absent(&tui.screen(), " · full");
    tui.ctrl('o');
    tui.wait_contains(" · full", WAIT);
}

/// `e` opens the focused dirty file in `$EDITOR`. A stub editor writes a
/// marker and exits 0 so the suite cannot hang in vim. A no-op `e` never
/// creates the marker or remounts with `edited README.md`.
#[test]
fn pty_e_opens_focused_file_in_editor() {
    let (_root, workspace) = daily_workspace();
    let shim_dir = workspace.join(".e2e-editor-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let stub = shim_dir.join("stub-editor");
    let marker = shim_dir.join("opened");
    fs::write(
        &stub,
        "#!/bin/sh\n\
         marker=\"${WS_STATUS_E2E_EDITOR_MARKER:?}\"\n\
         file=\"\"\n\
         for a in \"$@\"; do\n\
           file=\"$a\"\n\
         done\n\
         printf '%s\\n' \"$@\" > \"$marker\"\n\
         if [ -n \"$file\" ]; then\n\
           printf '\\ne2e-editor-marker\\n' >> \"$file\"\n\
         fi\n\
         exit 0\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    let editor = stub.display().to_string();
    let marker_s = marker.display().to_string();
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("EDITOR", editor.as_str()),
            ("WS_STATUS_E2E_EDITOR_MARKER", marker_s.as_str()),
        ],
    );
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md"),
        "launch cursor is the dirty file before e",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.key('e');
    tui.wait_pred(
        |screen| screen.contains("edited README.md") && tree_has(screen, "README.md"),
        "TUI remounts with edited README.md after the stub editor exits (a no-op never paints that)",
        WAIT,
    );
    let marker_body = fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        marker_body.contains("README.md"),
        "stub EDITOR must receive the focused file (a no-op never writes the marker):\n{marker_body}"
    );
    let readme = fs::read_to_string(workspace.join("app").join("README.md")).unwrap();
    assert!(
        readme.contains("e2e-editor-marker"),
        "stub EDITOR must append to the opened file:\n{readme}"
    );
}

/// Startup GitHub Release prompt on a TTY. `n` continues into the TUI.
#[test]
fn pty_update_prompt_n_opens_tui() {
    let (_root, workspace) = daily_workspace();
    let shim_dir = workspace.join(".e2e-curl-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let shim = shim_dir.join("curl");
    fs::write(
        &shim,
        "#!/bin/sh\n\
         for a in \"$@\"; do\n\
           case \"$a\" in\n\
             *releases/latest*)\n\
               printf '%s\\n' '{\"tag_name\":\"v99.0.0\"}'\n\
               exit 0\n\
               ;;\n\
           esac\n\
         done\n\
         exit 1\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&shim).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&shim, perms).unwrap();
    let path = format!(
        "{}:{}",
        shim_dir.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut tui = PtySession::open_pending(&workspace, &[("PATH", &path)], 0);
    tui.wait_contains(UPDATE_PROMPT, WAIT);
    tui.send_bytes(b"n\n");
    tui.wait_ready();
    tui.wait_contains("app", WAIT);
    tui.wait_contains("README.md", WAIT);
}

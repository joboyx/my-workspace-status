//! Additional real-TTY operator paths not claimed by `main.rs`.
//!
//! Fold (`h`/`l`, `z`, `zz` subtree), click-to-select, `gg`/`G`, Home/End,
//! `n`/`N` pane search, PgUp/PgDn, graph `c` create-branch, `r`, ignored
//! repos, plus other session keys the help overlay lists that a person
//! actually types.
//! Same PTY harness: bytes in, painted screen out.

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
                && screen.contains("UNSTAGED")
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

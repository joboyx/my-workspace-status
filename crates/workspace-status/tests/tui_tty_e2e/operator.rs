//! Additional real-TTY operator paths not claimed by `main.rs`.
//!
//! Fold (`h`/`l`, `z`, `zz` subtree), click-to-select, double-click Enter,
//! `m` mouse toggle, `gg`/`G`, Home/End, `n`/`N` pane search, PgUp/PgDn,
//! Ctrl-u/d, graph `c` create-branch, `r`, ignored repos, `e` editor,
//! CSI-u `T` theme, plus other session keys the help overlay lists that a
//! person actually types. Same PTY harness: bytes in, painted screen out.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use super::{
    assert_contains, op_finished, tree_has, tree_row_containing, GIT_WAIT, SETTLE_MS, WAIT,
};
use crate::common::hscroll::DIFF_HSCROLL_TAIL;
use crate::harness::{
    self, left_tree, tree_is_panned_to_tail, PtySession, SGR_WHEEL_DOWN, SGR_WHEEL_RIGHT,
};
use crate::seed::{
    daily_workspace, focus_workspace, seed_long_diff_file, seed_long_path_file, seed_repo,
    worktree_workspace,
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

/// ASCII expanded chevron (`v`) on the left-tree row that contains `name`.
fn tree_dir_expanded(screen: &str, name: &str) -> bool {
    left_tree(screen)
        .lines()
        .find(|line| line.contains(name))
        .is_some_and(|line| line.contains('v') && !line.contains('>'))
}

fn app_children_hidden(screen: &str) -> bool {
    tree_has(screen, "app")
        && tree_dir_collapsed(screen, "app")
        && !tree_has(screen, "zz-leaf.rs")
        && !tree_has(screen, "README.md")
        && !tree_has(screen, "src")
}

fn app_open_src_folded(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_has(screen, "src")
        && tree_dir_collapsed(screen, "src")
        && tree_dir_expanded(screen, "app")
        && !tree_has(screen, "zz-leaf.rs")
}

fn app_subtree_expanded(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && tree_has(screen, "src")
        && tree_has(screen, "zz-leaf.rs")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "src")
}

/// `zz` subtree-folds the focused repo. Nested leaf stays hidden after `l`.
///
/// First `z` toggles this row immediately and paints `z…`. Second `z`
/// within 400ms is `toggleSubtree` (no extra row toggle): descendants
/// follow the focused row. A no-op, a row-only fold, or a chord that
/// unfolds the parent again cannot pass. `z` / fold is a different key.
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
        |screen| app_subtree_expanded(screen) && tree_cursor_on(screen, "zz-leaf.rs"),
        "nested leaf is visible and focused at launch",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && screen.contains("toggle fold")
                && screen.contains("toggle subtree")
                && screen.contains("zz")
        },
        "help MOVE lists zz toggle subtree distinct from z toggle fold",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| !screen.contains("MOVE") && app_subtree_expanded(screen),
        "Esc closes help so z is not swallowed",
        WAIT,
    );

    tui.key('k');
    tui.wait_ms(SETTLE_MS);
    tui.key('k');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "src")
                && app_subtree_expanded(screen)
                && screen.contains("? help")
        },
        "k k from the leaf focuses the expanded app repo",
        WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        |screen| {
            app_children_hidden(screen)
                && tree_cursor_on(screen, "app")
                && screen.contains("z…")
                && !screen.contains("? help")
        },
        "first z folds only this row and arms the 400ms zz window",
        WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        |screen| {
            app_children_hidden(screen)
                && tree_cursor_on(screen, "app")
                && screen.contains("? help")
                && !screen.contains("z…")
        },
        "second z within 400ms keeps the repo folded (a subtree unfold is a no-op)",
        WAIT,
    );

    tui.tab();
    tui.wait_contains("workspace › [app]", WAIT);
    tui.zz();
    tui.wait_ms(SETTLE_MS);
    tui.tab();
    tui.wait_pred(
        |screen| {
            app_children_hidden(screen)
                && tree_cursor_on(screen, "app")
                && screen.contains("workspace › app")
                && !screen.contains("[app]")
        },
        "zz on the graph is a no-op; the hidden workspace tree stays folded",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| app_open_src_folded(screen) && tree_cursor_on(screen, "app"),
        "l opens the repo with src still folded (a row-only fold would show zz-leaf.rs)",
        WAIT,
    );

    tui.key('z');
    tui.wait_pred(
        |screen| app_children_hidden(screen) && screen.contains("z…"),
        "late-z setup: first z folds the repo again",
        WAIT,
    );
    tui.wait_ms(500);
    tui.key('z');
    tui.wait_pred(
        |screen| app_open_src_folded(screen) && tree_cursor_on(screen, "app"),
        "a late second z is a new row toggle, not subtree (src stays folded)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "src") && tree_dir_collapsed(screen, "src"),
        "j from app lands on the folded src dir",
        WAIT,
    );
    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "zz-leaf.rs") && tree_cursor_on(screen, "src"),
        "l on src reveals zz-leaf.rs (src was folded, not missing)",
        WAIT,
    );
    tui.key('h');
    tui.wait_pred(
        |screen| app_open_src_folded(screen) && tree_cursor_on(screen, "src"),
        "h folds src again before the expand chord",
        WAIT,
    );
    tui.key('k');
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "app") && app_open_src_folded(screen),
        "k returns to app with src still folded",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        |screen| app_children_hidden(screen) && tree_cursor_on(screen, "app"),
        "h folds the repo so zz can expand the subtree",
        WAIT,
    );
    tui.key('z');
    tui.wait_pred(
        |screen| {
            app_open_src_folded(screen) && tree_cursor_on(screen, "app") && screen.contains("z…")
        },
        "first z of expand zz opens the repo; src stays folded",
        WAIT,
    );
    tui.key('z');
    tui.wait_pred(
        |screen| app_subtree_expanded(screen) && tree_cursor_on(screen, "app"),
        "second z opens foldable descendants (zz-leaf.rs visible without l on src)",
        WAIT,
    );
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

/// Armed `/main` chip, idle hints, left focus. Tab paints `drill` / `[app]`.
fn armed_tree_search_left(screen: &str) -> bool {
    screen.contains("/main")
        && screen.contains("? help")
        && screen.contains("focus right")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && !screen.contains("drill")
        && !screen.contains("[app]")
        && !screen.contains("[lib]")
        && !screen.contains("[tools]")
        && !screen.contains("[workspace]")
}

/// Cursor, breadcrumb, and graph subject for one `/main` hit.
///
/// `seed {name}` is unique per checkout. `Working tree clean` is not
/// (`lib` and `tools` are both clean). A no-op, a skipped hit, or Tab
/// (`[name]` / `drill`) cannot pass.
fn search_hit_on(screen: &str, name: &str, graph_subject: &str) -> bool {
    let crumb = format!("workspace › {name}");
    let right_crumb = format!("workspace › [{name}]");
    armed_tree_search_left(screen)
        && tree_cursor_on(screen, name)
        && screen.contains(&crumb)
        && !screen.contains(&right_crumb)
        && screen.contains(graph_subject)
}

/// Help `n N`, then armed `/` `n` / CSI-u `N` next / prev on that pane.
///
/// Docs + MOVE: next / prev match after Enter. Tab is other pane. While
/// typing, `n` appends (`mainn`) and must not next. Three `main` checkouts
/// so wrap-`n` cannot pass as `N`. Cursor bar, breadcrumb, and `seed {name}`
/// must all move. Stay armed and left (`/main`, `focus right`, no `[…]`).
#[test]
fn pty_n_and_n_pane_next_prev() {
    let (_root, workspace) = daily_workspace();
    // Third clean `main` checkout so next and prev from `lib` diverge.
    seed_repo(&workspace, "tools", "main", false);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "No updates")
                && !tree_has(screen, "lib")
                && !tree_has(screen, "tools")
                && tree_cursor_on(screen, "README.md")
        },
        "lib and tools stay under the folded No-updates group",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && screen.contains("n   N")
                && screen.contains("next / prev match")
                && screen.contains("search focused pane")
                && screen.contains("Tab")
                && screen.contains("other pane")
        },
        "help MOVE lists n/N next/prev after Enter; Tab is other pane",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("next / prev match")
                && tree_has(screen, "README.md")
                && screen.contains("focus right")
        },
        "Esc closes help so n/N are pane search, not help keys",
        WAIT,
    );

    tui.key('/');
    tui.keys("main");
    tui.wait_pred(
        |screen| {
            screen.contains("SEARCH")
                && screen.contains("Enter arms query")
                && screen.contains("n/N after Enter")
                && !screen.contains("/main")
                && tree_cursor_on(screen, "app")
                && !tree_has(screen, "lib")
                && screen.contains("workspace › app")
                && !screen.contains("[app]")
        },
        "typing /main jumps to app; n/N are not live until Enter",
        GIT_WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |screen| {
            screen.contains("SEARCH")
                && screen.contains("mainn")
                && screen.contains("Enter arms query")
                && tree_cursor_on(screen, "app")
                && !tree_has(screen, "lib")
                && !screen.contains("seed lib")
                && !screen.contains("[app]")
                && !screen.contains("drill")
        },
        "n while typing appends; it must not next or switch panes",
        WAIT,
    );
    tui.send_bytes(b"\x7f");
    tui.wait_pred(
        |screen| screen.contains("SEARCH") && !screen.contains("mainn"),
        "Backspace drops the extra n so Enter can arm /main",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "app", "seed app")
                && screen.contains("Uncommitted changes")
                && !tree_has(screen, "lib")
                && !tree_cursor_on(screen, "lib")
                && !tree_cursor_on(screen, "tools")
        },
        "Enter arms /main on dirty app; lib stays folded",
        GIT_WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "lib", "seed lib")
                && screen.contains("Working tree clean")
                && !tree_cursor_on(screen, "app")
                && !tree_cursor_on(screen, "tools")
                && !screen.contains("seed app")
                && !screen.contains("seed tools")
                && !screen.contains("workspace › app")
                && !screen.contains("workspace › tools")
        },
        "n jumps to lib (a no-op stays on app; skip lands on tools; Tab is [lib])",
        GIT_WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "tools", "seed tools")
                && !tree_cursor_on(screen, "lib")
                && !tree_cursor_on(screen, "app")
                && !screen.contains("seed lib")
                && !screen.contains("workspace › lib")
        },
        "second n jumps to tools (a no-op stays on lib; wrap-n would return to app)",
        GIT_WAIT,
    );

    tui.shift_letter('N');
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "lib", "seed lib")
                && !tree_cursor_on(screen, "tools")
                && !tree_cursor_on(screen, "app")
                && !screen.contains("seed tools")
                && !screen.contains("workspace › tools")
                && !screen.contains("workspace › app")
        },
        "CSI-u N returns to lib (a no-op stays on tools; wrap-n would land on app)",
        GIT_WAIT,
    );

    tui.shift_letter('N');
    tui.wait_pred(
        |screen| {
            search_hit_on(screen, "app", "seed app")
                && screen.contains("Uncommitted changes")
                && !tree_cursor_on(screen, "lib")
                && !tree_cursor_on(screen, "tools")
                && !screen.contains("seed lib")
                && !screen.contains("workspace › lib")
                && !screen.contains("workspace › tools")
        },
        "second CSI-u N returns to app (a no-op stays on lib; wrap-n would land on tools)",
        GIT_WAIT,
    );
}

/// CSI PageDown then PageUp pages the workspace tree by one viewport.
///
/// Help VIEW lists `PgUp PgDn` as "page focused pane", distinct from
/// `Ctrl-u Ctrl-d` ±5. This suite sends xterm CSI `ESC [6~` / `ESC [5~`.
/// Launch focuses `README.md`. Files sort as README then `page-00`…
/// `page-29`. The default PTY paints 28 tree rows, so one page is 27
/// (`visible − 1` overlap) and lands on `page-26.txt`. Cursor bar and
/// the right-pane file body must both move. A no-op stays on README.
/// `j` would land on `page-00`. Ctrl-d would land on `page-04`. `G` /
/// End would land on No updates. Home would land on the workspace root.
#[test]
fn pty_pgup_pgdn_pages_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_page_files(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", WAIT);

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("VIEW")
                && screen.contains("PgUp   PgDn")
                && screen.contains("page focused pane")
                && screen.contains("Ctrl-u   Ctrl-d")
                && screen.contains("page focused ±5")
        },
        "help VIEW lists PgUp/PgDn as a viewport page, distinct from Ctrl-u/d ±5",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("page focused pane")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
        },
        "Esc closes help so PageDown is not swallowed",
        WAIT,
    );

    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "page-00.txt")
                && !tree_cursor_on(screen, "page-04.txt")
                && !tree_cursor_on(screen, "page-26.txt")
                && !tree_cursor_on(screen, "workspace")
                && !tree_has(screen, "page-29")
                && !screen.contains("page-26-body")
                && screen.contains("UNSTAGED")
        },
        "launch cursor is README; page-26 is not focused",
        GIT_WAIT,
    );

    tui.page_down();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "page-26.txt")
                && screen.contains("page-26-body")
                && screen.contains("NEW")
                && !tree_cursor_on(screen, "README.md")
                && !tree_has(screen, "README.md")
                && !tree_cursor_on(screen, "page-00.txt")
                && !tree_cursor_on(screen, "page-04.txt")
                && !tree_cursor_on(screen, "page-29.txt")
                && !tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "workspace")
        },
        "PageDown pages +27 to page-26 (a no-op stays on README; j would hit page-00; Ctrl-d would hit page-04; G would hit No updates)",
        GIT_WAIT,
    );

    tui.page_up();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && !tree_cursor_on(screen, "page-26.txt")
                && !screen.contains("page-26-body")
                && !tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "No updates")
        },
        "PageUp returns to README (a no-op keeps page-26; Home would land on workspace)",
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

/// Cursor bar on a 0-based screen row. Used after tree hscroll clips labels.
fn tree_cursor_bar_on_row(screen: &str, row: u16) -> bool {
    screen
        .lines()
        .nth(row as usize)
        .is_some_and(|line| line.contains('\u{258C}'))
}

fn help_lists_home_end_top_bottom(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains("Home")
            && line.contains("End")
            && line.contains("top / bottom")
            && !line.contains("page")
            && !line.contains("focused pane")
    })
}

fn help_lists_e_open_in_editor(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("GIT")
        && compact.contains("open in editor")
        && compact.contains("Ctrl-o")
        && compact.contains("full-file")
}

/// Help MOVE lists Home/End as list top/bottom. CSI `1~` / `4~` jump the tree.
///
/// Documented result: first / last **tree** row, not pane chrome. `gg` / `G`
/// are the same edges via letters. PageDown is one viewport. Extra dirty
/// files keep the last row off-screen at launch, so a no-op, PageDown, or
/// a jump onto `page-29` / merger cannot pass. Cursor bar, right pane, and
/// fold must all move.
#[test]
fn pty_home_and_end_jump_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_page_files(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && tree_has(screen, "workspace")
                && !tree_cursor_on(screen, "workspace")
                && !tree_has(screen, "No updates")
                && !tree_has(screen, "page-29")
                && !page_file_body_visible(screen)
                && screen.contains("? help")
                && screen.contains("focus right")
                && !screen.contains("[workspace]")
        },
        "launch cursor is README; last tree rows stay below the fold",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("MOVE")
                && help_lists_home_end_top_bottom(screen)
                && screen.contains("gg   G")
                && screen.contains("top / bottom of focused")
                && screen.contains("PgUp   PgDn")
                && screen.contains("page focused pane")
        },
        "help MOVE lists Home/End as top/bottom, not page and not gg/G chrome",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("Home")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
                && screen.contains("UNSTAGED")
        },
        "Esc closes help so Home/End are tree jumps, not help keys",
        WAIT,
    );

    tui.end();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "page-29")
                && !tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "workspace")
                && !tree_has(screen, "README.md")
                && !tree_has(screen, "workspace")
                && tree_has(screen, "page-29")
                && screen.contains("focus a repo for the graph")
                && !screen.contains("UNSTAGED")
                && !page_file_body_visible(screen)
                && !screen.contains("WIP on graph")
                && screen.contains("? help")
                && screen.contains("focus right")
                && !screen.contains("fetch")
                && !screen.contains("[workspace]")
                && !screen.contains("drill")
        },
        "End jumps to the last tree row (a no-op stays on README; PgDn stays mid-list; merger would load its graph)",
        GIT_WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib") && tree_cursor_on(screen, "No updates"),
        "End then l opens No updates (End actually selected that row)",
        WAIT,
    );

    tui.home();
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "No updates")
                && !tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "page-29")
                && tree_has(screen, "workspace")
                && !tree_has(screen, "No updates")
                && !tree_has(screen, "page-29")
                && screen.contains("focus a repo for the graph")
                && screen.contains("? help")
                && screen.contains("focus right")
                && screen.contains("fetch")
                && !screen.contains("[workspace]")
                && !screen.contains("UNSTAGED")
                && !page_file_body_visible(screen)
        },
        "Home jumps to the first tree row (a no-op stays on No updates; PgUp from the end lands on a page file)",
        GIT_WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        |screen| {
            !tree_has(screen, "app")
                && !tree_has(screen, "lib")
                && !tree_has(screen, "README.md")
                && tree_cursor_on(screen, "workspace")
        },
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

/// Tree `m` toggles mouse reporting. Not graph merge.
///
/// Docs / keymap / help: mouse is on by default. Tree `m` (raw byte)
/// flips capture and paints `Mouse off` / `Mouse on`. Off ignores click,
/// drag, and wheel. On accepts them. A focused graph commit would confirm
/// merge instead.
///
/// Default-on click must select `merger`. After `m`, click, vertical
/// wheel (`Cb` 65), and trackpad hscroll (`Cb` 67) are ignored. After
/// the second `m`, click selects README, hscroll pans the clipped tree
/// row, and vertical wheel moves the tree cursor. Toast-only, click-only,
/// or a no-op is red.
#[test]
fn pty_m_toggles_mouse_capture() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("README.md")
                && !screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && !screen.contains("fast-forward if possible")
        },
        "launch paints the tree; mouse status and merge confirm are absent",
        GIT_WAIT,
    );
    let _ = tui.wait_clipped_long_path_row(WAIT);

    // The untracked long path is the first file. Click README so the
    // default-on mouse path both selects and loads a short diff (hscroll
    // over the tree then pans the tree, not a long file-diff).
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_row);
    tui.wait_pred(
        |screen| tree_cursor_on(screen, "README.md") && screen.contains("UNSTAGED"),
        "default mouse-on SGR click selects README (a default-off or dead mouse never loads that pane)",
        GIT_WAIT,
    );

    let merger_row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, merger_row);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("UNSTAGED")
                && (screen.contains("Working tree") || screen.contains("WIP on graph"))
        },
        "default mouse-on SGR click selects merger (a default-off or dead mouse never loads that pane)",
        GIT_WAIT,
    );

    tui.key('m');
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && tree_cursor_on(screen, "merger")
                && !screen.contains("fast-forward if possible")
                && !screen.contains("UNSTAGED")
        },
        "tree `m` paints Mouse off and does not open merge confirm",
        WAIT,
    );

    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row while mouse off:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_row);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("UNSTAGED")
                && (screen.contains("Working tree") || screen.contains("WIP on graph"))
        },
        "SGR click is ignored while Mouse off (toast-only would select README)",
        WAIT,
    );

    let merger_row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row while mouse off:\n{}", tui.screen()));
    tui.sgr_mouse(SGR_WHEEL_DOWN, TREE_LABEL_COL, merger_row);
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && tree_cursor_on(screen, "merger")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("UNSTAGED")
        },
        "vertical wheel is ignored while Mouse off (ungated wheel would leave merger)",
        WAIT,
    );

    let hscroll_row = tui.wait_clipped_long_path_row(WAIT);
    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, hscroll_row);
    }
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse off")
                && harness::clipped_long_path_row(screen).is_some()
                && tree_cursor_on(screen, "merger")
        },
        "SGR hscroll is ignored while Mouse off (ungated wheel would pan the tree)",
        WAIT,
    );

    tui.key('m');
    tui.wait_pred(
        |screen| {
            screen.contains("Mouse on")
                && !screen.contains("Mouse off")
                && tree_cursor_on(screen, "merger")
                && !screen.contains("UNSTAGED")
        },
        "second tree `m` paints Mouse on",
        WAIT,
    );
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row after Mouse on:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_row);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "merger")
                && screen.contains("UNSTAGED")
        },
        "SGR click selects README after Mouse on (status-only would leave merger focused)",
        GIT_WAIT,
    );

    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row before hscroll on:\n{}", tui.screen()));
    let hscroll_row = tui.wait_clipped_long_path_row(WAIT);
    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, hscroll_row);
    }
    tui.wait_pred(
        |screen| tree_is_panned_to_tail(screen) && tree_cursor_bar_on_row(screen, readme_row),
        "SGR hscroll pans the tree after Mouse on and does not steal the README cursor",
        WAIT,
    );

    tui.sgr_mouse(SGR_WHEEL_DOWN, TREE_LABEL_COL, readme_row);
    tui.wait_pred(
        |screen| !tree_cursor_bar_on_row(screen, readme_row),
        "vertical wheel moves the tree cursor after Mouse on (gate-only would stay on README)",
        GIT_WAIT,
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

/// VIEW lists Ctrl-u/d as ±5. Same line as `page focused ±5`, not PgUp/PgDn.
fn help_lists_ctrl_u_d_half_page(screen: &str) -> bool {
    screen.lines().any(|line| {
        line.contains("Ctrl-u")
            && line.contains("Ctrl-d")
            && line.contains("page focused ±5")
            && !line.contains("PgUp")
            && !line.contains("page focused pane")
    })
}

/// Cursor bar, file-diff body, and left focus for one half-page landing.
///
/// `j` is `jump-00`. A 12-row PageDown is `jump-06`. End / `G` / a fitting
/// PageDown is No updates. Tab paints `[workspace]` / `drill`.
fn half_page_on(screen: &str, name: &str, body: &str) -> bool {
    tree_cursor_on(screen, name)
        && screen.contains(body)
        && screen.contains("NEW")
        && screen.contains("focus right")
        && !screen.contains("[workspace]")
        && !screen.contains("drill")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "workspace")
        && !screen.contains("WIP on graph")
        && !screen.contains("UNSTAGED")
}

/// Help VIEW lists Ctrl-u/d as ±5. CSI-u Control jumps the tree half a page.
///
/// Documented result: ±5 rows on the focused list, not `j`/`k` (±1), not
/// PageDown (viewport), not Home/End. Launch focuses README.md. Files sort
/// as README then `jump-00`…`jump-09`, so +5 lands on `jump-04.txt` and a
/// second +5 on `jump-09.txt` (not merger). Cursor bar and the right-pane
/// body must both move. Stay left. A no-op stays on README.
#[test]
fn pty_ctrl_u_d_jumps_workspace_tree() {
    let (_root, workspace) = daily_workspace();
    seed_tree_half_page_files(&workspace);

    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && tree_has(screen, "jump-00.txt")
                && tree_has(screen, "jump-04.txt")
                && tree_has(screen, "jump-09.txt")
                && tree_has(screen, "No updates")
                && !tree_cursor_on(screen, "jump-00.txt")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !tree_cursor_on(screen, "jump-06.txt")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !tree_cursor_on(screen, "No updates")
                && !screen.contains("jump-04-body")
                && screen.contains("? help")
                && screen.contains("focus right")
                && !screen.contains("[workspace]")
        },
        "launch cursor is README; half-page and end rows are not focused",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| {
            screen.contains("VIEW")
                && help_lists_ctrl_u_d_half_page(screen)
                && screen.contains("Ctrl-u   Ctrl-d")
                && screen.contains("PgUp   PgDn")
                && screen.contains("page focused pane")
                && screen.contains("j   k")
                && screen.contains("Home   End")
                && screen.contains("top / bottom")
        },
        "help VIEW lists Ctrl-u/d as ±5, not PgUp/PgDn and not j/Home/End",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("page focused ±5")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
                && screen.contains("UNSTAGED")
                && screen.contains("focus right")
        },
        "Esc closes help so Ctrl-u/d are tree jumps, not help keys",
        WAIT,
    );

    tui.ctrl_letter('d');
    tui.wait_pred(
        |screen| {
            half_page_on(screen, "jump-04.txt", "jump-04-body")
                && !tree_cursor_on(screen, "jump-00.txt")
                && !tree_cursor_on(screen, "jump-03.txt")
                && !tree_cursor_on(screen, "jump-05.txt")
                && !tree_cursor_on(screen, "jump-06.txt")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !screen.contains("jump-00-body")
                && !screen.contains("jump-06-body")
                && !screen.contains("jump-09-body")
        },
        "CSI-u Ctrl-d moves +5 to jump-04 (j is jump-00; 12-row PgDn is jump-06; End/PgDn on this tree is No updates)",
        GIT_WAIT,
    );

    tui.ctrl_letter('d');
    tui.wait_pred(
        |screen| {
            half_page_on(screen, "jump-09.txt", "jump-09-body")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !tree_cursor_on(screen, "jump-08.txt")
                && !screen.contains("jump-04-body")
        },
        "second CSI-u Ctrl-d moves +5 to jump-09 (a no-op stays on jump-04; End is No updates; merger is +6)",
        GIT_WAIT,
    );

    tui.ctrl_letter('u');
    tui.wait_pred(
        |screen| {
            half_page_on(screen, "jump-04.txt", "jump-04-body")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !tree_cursor_on(screen, "README.md")
                && !screen.contains("jump-09-body")
        },
        "CSI-u Ctrl-u returns +5 to jump-04 (a no-op stays on jump-09; Home/two Ctrl-u would hit README)",
        GIT_WAIT,
    );

    tui.ctrl_letter('u');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && screen.contains("focus right")
                && !screen.contains("[workspace]")
                && !screen.contains("drill")
                && !tree_cursor_on(screen, "jump-04.txt")
                && !tree_cursor_on(screen, "jump-00.txt")
                && !tree_cursor_on(screen, "jump-09.txt")
                && !tree_cursor_on(screen, "No updates")
                && !screen.contains("jump-04-body")
                && !screen.contains("NEW")
        },
        "second CSI-u Ctrl-u returns to README (a no-op keeps jump-04; j would stay on jump-00)",
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

/// `e` hands the focused workspace file to `$EDITOR`. Help lists that as
/// open in editor. Ctrl-o is full-file and must not pass as this key.
///
/// Launch starts on README.md. The claim uses a second dirty file so an
/// editor that always opens the first path cannot pass. A TTY stub paints
/// chrome, holds the TTY, then exits 0 (a live vim session would hang
/// `cargo test`). After return, the TUI remounts on the same file. A no-op,
/// a toast with no spawn, or the wrong path fails.
#[test]
fn pty_e_opens_focused_file_in_editor() {
    let (_root, workspace) = daily_workspace();
    fs::write(
        workspace.join("app").join("edit-target.txt"),
        "unique-edit-target-body\n",
    )
    .unwrap();
    let shim_dir = workspace.join(".e2e-editor-shim");
    fs::create_dir_all(&shim_dir).unwrap();
    let stub = shim_dir.join("stub-editor");
    let marker = shim_dir.join("opened");
    let hold = shim_dir.join("hold");
    fs::write(&hold, "1\n").unwrap();
    fs::write(
        &stub,
        "#!/bin/sh\n\
         marker=\"${WS_STATUS_E2E_EDITOR_MARKER:?}\"\n\
         hold=\"${WS_STATUS_E2E_EDITOR_HOLD:?}\"\n\
         file=\"\"\n\
         for a in \"$@\"; do\n\
           file=\"$a\"\n\
         done\n\
         printf '%s\\n' \"$@\" > \"$marker\"\n\
         printf '\\n===== STUB-EDITOR-CHROME =====\\nopening %s\\n===== STUB-EDITOR-CHROME =====\\n' \"$file\"\n\
         if [ -n \"$file\" ]; then\n\
           printf '\\ne2e-editor-marker\\n' >> \"$file\"\n\
         fi\n\
         i=0\n\
         while [ -f \"$hold\" ] && [ \"$i\" -lt 300 ]; do\n\
           sleep 0.1\n\
           i=$((i + 1))\n\
         done\n\
         exit 0\n",
    )
    .unwrap();
    let mut perms = fs::metadata(&stub).unwrap().permissions();
    perms.set_mode(0o755);
    fs::set_permissions(&stub, perms).unwrap();
    let editor = stub.display().to_string();
    let marker_s = marker.display().to_string();
    let hold_s = hold.display().to_string();
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("EDITOR", editor.as_str()),
            ("WS_STATUS_E2E_EDITOR_MARKER", marker_s.as_str()),
            ("WS_STATUS_E2E_EDITOR_HOLD", hold_s.as_str()),
        ],
    );
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && tree_has(screen, "edit-target.txt")
                && screen.contains("UNSTAGED")
                && !tree_cursor_on(screen, "edit-target.txt")
                && !screen.contains("unique-edit-target-body")
                && !screen.contains(" · full")
                && screen.contains("? help")
        },
        "launch cursor is README, not the unique dirty file",
        GIT_WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        |screen| help_lists_e_open_in_editor(screen) && screen.contains("MOVE"),
        "help GIT lists e as open in editor; Ctrl-o stays full-file",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("open in editor")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
        },
        "Esc closes help so e is the editor key, not a help query",
        WAIT,
    );

    tui.search("edit-target");
    tui.wait_pred(
        |screen| {
            screen.contains("/edit-target")
                && tree_cursor_on(screen, "edit-target.txt")
                && !tree_cursor_on(screen, "README.md")
                && screen.contains("unique-edit-target-body")
                && screen.contains("NEW")
                && !screen.contains("UNSTAGED")
                && !screen.contains(" · full")
        },
        "search focuses the unique file (e on README would open the wrong path)",
        GIT_WAIT,
    );

    tui.key('e');
    tui.wait_pred(
        |screen| {
            screen.contains("STUB-EDITOR-CHROME")
                && screen.contains("edit-target.txt")
                && !screen.contains("edited edit-target.txt")
                && !screen.contains(" · full")
        },
        "e must paint editor chrome for the focused path (a no-op stays idle; Ctrl-o paints full-file)",
        WAIT,
    );
    let marker_body = fs::read_to_string(&marker).unwrap_or_default();
    assert!(
        marker_body.contains("edit-target.txt") && !marker_body.contains("README.md"),
        "stub EDITOR must receive the focused file, not README.md:\n{marker_body}"
    );
    fs::remove_file(&hold).unwrap();

    tui.wait_pred(
        |screen| {
            screen.contains("edited edit-target.txt")
                && !screen.contains("edited README.md")
                && !screen.contains("STUB-EDITOR-CHROME")
                && tree_cursor_on(screen, "edit-target.txt")
                && !tree_cursor_on(screen, "README.md")
                && tree_has(screen, "README.md")
                && screen.contains("unique-edit-target-body")
                && screen.contains("e2e-editor-marker")
                && screen.contains("/edit-target")
                && screen.contains("? help")
                && !screen.contains(" · full")
                && !screen.contains("[workspace]")
        },
        "TUI remounts on the same focused file after the editor exits (a toast-only no-op never writes the marker line)",
        GIT_WAIT,
    );
    let readme = fs::read_to_string(workspace.join("app").join("README.md")).unwrap();
    let target = fs::read_to_string(workspace.join("app").join("edit-target.txt")).unwrap();
    assert!(
        !readme.contains("e2e-editor-marker"),
        "README.md must stay closed:\n{readme}"
    );
    assert!(
        target.contains("e2e-editor-marker") && target.contains("unique-edit-target-body"),
        "stub EDITOR must append to the focused file:\n{target}"
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

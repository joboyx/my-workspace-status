//! Additional real-TTY operator paths not claimed by `main.rs`.
//!
//! Fold (`h`/`l`, `z`, `zz` subtree), click-to-select, double-click Enter,
//! `m` mouse toggle, default-on tree SGR hscroll, `gg`/`G`, Home/End,
//! `/` pane search, `n`/`N` next/prev, PgUp/PgDn, Ctrl-u/d, graph `c`
//! create-branch, graph `m` merge into HEAD, graph stash `p` pop, `r`, live
//! watch without `r`, ignored repos, `e` editor, CSI-u `T` theme, first
//! Ctrl+C quit prompt, `q` quit, Space reviewed (`*` ASCII), `s`/`u` stage
//! and unstage, `f` fetch remotes, `p` pull behind, CSI-u Shift+P push,
//! CSI-u Shift+S stash create then graph `a` apply and CSI-u Shift+D drop,
//! streamed collect, plus other session keys the help overlay lists that
//! a person actually types. Same PTY harness: bytes in, painted screen out.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use super::{
    assert_contains, op_finished, tree_has, tree_line_containing, GIT_WAIT, SETTLE_MS, WAIT,
};
use crate::common::hscroll::{DIFF_HSCROLL_TAIL, TREE_HSCROLL_TAIL};
use crate::harness::{
    self, assert_tree_clipped_long_path, left_tree, status_has_tree_hscroll_tail,
    tree_cursor_bar_on_row, tree_is_panned_to_tail, tree_row_containing, PtySession,
    SGR_WHEEL_DOWN, SGR_WHEEL_RIGHT, SGR_WHEEL_RIGHT_MOTION,
};
use crate::seed::{
    ahead_workspace, behind_workspace, daily_workspace, focus_workspace, seed_long_diff_file,
    seed_long_path_file, seed_repo, stream_workspace, unfetched_behind_workspace,
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
///
/// Docs + MOVE: tree-focused `h` / `l` close / open fold. Help `z` is a
/// separate toggle. Live PTY after first paint (cursor already on dirty
/// README) left the group folded. `G` then `l` opened it (`v`, `lib`
/// visible). A second `l` stayed open. `h` folded it (`>`, `lib` hidden).
/// A second `h` stayed folded. Not `z` toggle (`z…`). Not parent-repo
/// fold. Not Enter drill. Not chevron click.
///
/// After first paint the cursor is already on the dirty file. Do not `/`
/// search. A no-op on the group, pan, `z` toggle, file-`l` that opens the
/// group, or `h` that folds `app` / workspace cannot pass.
#[test]
fn pty_fold_h_l_toggles_no_updates_group() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        file_hl_leaves_group_folded,
        "first paint: cursor on dirty README, No updates folded, no lib",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        file_hl_leaves_group_folded,
        "`l` on the dirty file must not expand No updates (file, cursor, diff stay)",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        file_hl_leaves_group_folded,
        "`h` on the dirty file must not fold app or open No updates",
        WAIT,
    );

    tui.shift_letter('G');
    tui.wait_pred(
        group_hl_folded,
        "G focuses folded No updates (a no-op stays on README; l has not opened yet)",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        group_hl_open,
        "l on No updates opens the group (v, lib visible, cursor stays)",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        group_hl_open,
        "second l stays open (z toggle would hide lib)",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        group_hl_folded,
        "h folds No updates (>, lib hidden; app and README stay)",
        WAIT,
    );

    tui.key('h');
    tui.wait_pred(
        group_hl_folded,
        "second h stays folded (z toggle would reveal lib)",
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

/// Folded No-updates group: collapsed chevron, count 1, `lib` hidden.
fn no_updates_group_folded(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, "No updates") else {
        return false;
    };
    tree_dir_collapsed(screen, "No updates")
        && !tree_dir_expanded(screen, "No updates")
        && line.contains('>')
        && line.contains('1')
        && !tree_has(screen, "lib")
}

/// Open No-updates group: expanded chevron, `lib` on the tree.
fn no_updates_group_open(screen: &str) -> bool {
    let Some(line) = tree_line_containing(screen, "No updates") else {
        return false;
    };
    let Some(lib) = tree_line_containing(screen, "lib") else {
        return false;
    };
    tree_dir_expanded(screen, "No updates")
        && !tree_dir_collapsed(screen, "No updates")
        && line.contains('v')
        && line.contains('1')
        && lib.contains("@ lib")
        && lib.contains("& main")
}

fn fold_hl_no_wrong_chrome(screen: &str) -> bool {
    no_wrong_overlays(screen)
        && !screen.contains("z…")
        && !screen.contains("[workspace]")
        && crumb_row(screen).trim() == "workspace"
        && status_row(screen).contains(" tree")
        && status_row(screen).contains(" split")
        && status_row(screen).contains("focus right")
}

/// Tree-focused dirty file: `h`/`l` must not open No updates or fold `app`.
fn file_hl_leaves_group_folded(screen: &str) -> bool {
    idle_dirty_readme_unstaged(screen)
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "merger")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && fold_hl_no_wrong_chrome(screen)
        && !screen.contains("focus a repo for the graph")
}

/// Group focused and folded. `app` / README stay. Right pane is not a file diff.
fn group_hl_folded(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "lib")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && fold_hl_no_wrong_chrome(screen)
        && status_row(screen).contains("other pane")
}

/// Group focused and open. Cursor stays on the group. Not `z` toggle.
fn group_hl_open(screen: &str) -> bool {
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "lib")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_open(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && fold_hl_no_wrong_chrome(screen)
        && status_row(screen).contains("other pane")
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

/// Space on a dirty file paints the ASCII reviewed mark (`*`).
///
/// Docs: Space reviewed (`*` ASCII). Help / keymap: `space` / "mark dirty
/// file reviewed (eye)". Configuration: trailing eye, then Space again
/// unmarks while contents are unchanged. Live PTY after first paint did
/// that toggle. Not `z` fold (`pty_z_folds_focused_repo`). Not `s`/`u`
/// stage. Not graph-focus overlay Space (`[x]`).
///
/// After first paint the cursor is already on the dirty README (not a
/// repo row). Do not `/` search. A no-op, a cursor-only move, a fold that
/// hides the file, a stage, or a `*` that never lands on that row is red.
#[test]
fn pty_space_marks_dirty_file_reviewed() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        super::idle_dirty_readme_unreviewed,
        "first paint: cursor on dirty README, no reviewed mark",
        WAIT,
    );

    tui.key(' ');
    tui.wait_pred(
        super::documented_space_reviewed,
        "Space paints ASCII `*` on the focused README row; file stays; not staged",
        WAIT,
    );

    tui.key(' ');
    tui.wait_pred(
        super::idle_dirty_readme_unreviewed,
        "second Space clears the reviewed mark (documented toggle)",
        WAIT,
    );
}

/// Cells after `README.md` on the left-tree file row (trailing chrome).
fn after_readme_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "README.md")?;
    let at = line.find("README.md")?;
    Some(line[at + "README.md".len()..].to_string())
}

/// Last status row (mode pills + hint chips).
fn status_row(screen: &str) -> &str {
    screen_line_from_end(screen, 0)
}

/// Breadcrumb row (path left, toast right).
fn crumb_row(screen: &str) -> &str {
    screen_line_from_end(screen, 1)
}

/// Trailing `M ` badge, not staged `S `, not reviewed `*`.
fn readme_unstaged_badge(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| after.contains("M ") && !after.contains('S') && !after.contains('*'))
}

/// Trailing staged `S `, not `M ` / `MS`, not reviewed `*`.
fn readme_staged_badge(screen: &str) -> bool {
    after_readme_name(screen)
        .is_some_and(|after| after.contains("S ") && !after.contains('M') && !after.contains('*'))
}

fn has_stage_hint(screen: &str) -> bool {
    let status = status_row(screen);
    status.contains("stage") && !status.contains("unstage")
}

fn has_unstage_hint(screen: &str) -> bool {
    status_row(screen).contains("unstage")
}

fn pane_unstaged_readme(screen: &str) -> bool {
    screen.contains("UNSTAGED") && screen.contains("app/README.md") && screen.contains("+dirty")
}

fn pane_staged_readme(screen: &str) -> bool {
    screen.contains("STAGED")
        && !screen.contains("UNSTAGED")
        && screen.contains("app/README.md")
        && screen.contains("+dirty")
}

fn no_wrong_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("[x]")
        && !screen.contains("nothing to stage")
        && !screen.contains("nothing to unstage")
}

/// First paint: dirty README focused, unstaged. Not a repo row.
fn idle_dirty_readme_unstaged(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && has_stage_hint(screen)
        && !has_unstage_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace"
        && no_wrong_overlays(screen)
}

/// `s` staged the focused dirty file. File stays. Not Space. Not stash.
fn documented_s_staged(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_staged_badge(screen)
        && pane_staged_readme(screen)
        && has_unstage_hint(screen)
        && !has_stage_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb.contains("staged README.md")
        && !crumb.contains("unstaged")
        && no_wrong_overlays(screen)
}

/// `u` restored unstaged. Same file. Not a no-op after stage.
fn documented_u_unstaged(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && has_stage_hint(screen)
        && !has_unstage_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).contains("unstaged README.md")
        && no_wrong_overlays(screen)
}

/// `s` stages the focused dirty file; `u` unstages.
///
/// Docs: Help GIT `s` = stage scope, `u` = unstage scope. Configuration:
/// file / dir / repo. Live PTY after first paint staged `app/README.md`
/// (`git add`): tree badge `M ` → `S `, pane UNSTAGED → STAGED, status
/// `s stage` → `u unstage`, breadcrumb `staged README.md`. `u` reversed
/// (`git restore --staged`). Not Space reviewed (`*`). Not Shift+S stash
/// overlay. Not a toast-only tick.
///
/// After first paint the cursor is already on the dirty README. Do not
/// `/` search. A no-op, wrong file, stage-only with no unstage, paint
/// flicker, toast-only, Space `*`, or stash overlay is red.
#[test]
fn pty_stage_and_unstage_dirty_file() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unstaged,
        "first paint: cursor on dirty README, unstaged, stage hint, no unstage",
        WAIT,
    );

    tui.key('s');
    tui.wait_pred(
        documented_s_staged,
        "s stages focused README: tree S, pane STAGED, unstage hint, staged toast",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_s_staged,
        "staged paint holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key('u');
    tui.wait_pred(
        documented_u_unstaged,
        "u unstages the same README: tree M, pane UNSTAGED, stage hint",
        GIT_WAIT,
    );
}

fn no_stash_wrong_ops(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("popped")
        && !crumb_row(screen).contains("staged")
}

fn app_stash_on_graph(screen: &str) -> bool {
    screen.contains("WIP on main")
        && screen.contains("stash@{0}")
        && screen.contains("seed app")
        && !screen.contains("WIP on graph")
}

fn has_graph_stash_hints(screen: &str) -> bool {
    let status = status_row(screen);
    status.contains("apply stash") && status.contains("drop stash") && status.contains("pop stash")
}

/// CSI-u Shift+S opened the create-only overlay on the dirty README.
fn stash_create_overlay_open(screen: &str) -> bool {
    tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && pane_unstaged_readme(screen)
        && screen.contains("Stash app")
        && screen.contains("s create")
        && screen.contains("Esc cancel")
        && !screen.contains("a apply")
        && !screen.contains("p pop")
        && !screen.contains("d drop")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on main")
}

/// Overlay `s` created a path-scoped stash. README left the tree.
fn documented_stash_created(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_has(screen, "README.md")
        && !tree_has(screen, "app")
        && tree_has(screen, "No updates")
        && tree_has(screen, "0 changed")
        && crumb.contains("Stashed 1 file")
        && !screen.contains("Stash app")
        && !screen.contains("s create")
        && !screen.contains("UNSTAGED")
        && !screen.contains("WIP on main")
        && !crumb.contains("staged")
        && !crumb.contains("applied")
        && !crumb.contains("popped")
        && !crumb.contains("dropped")
        && no_stash_wrong_ops(screen)
}

/// `l` then `j`: app is focused under No updates. App graph shows the stash.
fn app_focused_stash_visible(screen: &str) -> bool {
    tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "README.md")
        && !tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "lib")
        && tree_has(screen, "No updates")
        && app_stash_on_graph(screen)
        && screen.contains("working tree clean")
        && crumb_row(screen).contains("workspace › app")
        && !crumb_row(screen).contains("[app]")
        && tree_pane_focused(screen)
        && no_stash_wrong_ops(screen)
}

/// Tab focused the app graph on the working-tree row. Stash is the next row.
fn app_graph_working_tree_focused(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && graph_cursor_on(screen, "working tree clean")
        && !graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && crumb_row(screen).contains("[app]")
        && no_stash_wrong_ops(screen)
}

/// `j` landed on the app stash row. Graph `a` / `D` hints. Not merger.
fn app_graph_stash_row_focused(screen: &str) -> bool {
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "app")
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && has_graph_stash_hints(screen)
        && crumb_row(screen).contains("[app]")
        && !screen.contains("Drop stash@{0}?")
        && no_stash_wrong_ops(screen)
}

/// Graph `a` applied. README is dirty again. Stash stays (not pop).
fn documented_stash_applied(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "1 changed")
        && readme_unstaged_badge(screen)
        && screen.contains("uncommitted changes")
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && has_graph_stash_hints(screen)
        && crumb.contains("applied stash@{0}")
        && !crumb.contains("popped")
        && !crumb.contains("dropped")
        && !crumb.contains("Stashed")
        && !screen.contains("Drop stash@{0}?")
        && no_stash_wrong_ops(screen)
}

/// CSI-u Shift+D opened drop confirm. Stash and dirty README stay until `y`.
fn stash_drop_confirm_open(screen: &str) -> bool {
    graph_pane_focused(screen)
        && screen.contains("Drop stash@{0}?")
        && tree_has(screen, "README.md")
        && readme_unstaged_badge(screen)
        && graph_cursor_on(screen, "WIP on main")
        && app_stash_on_graph(screen)
        && screen.contains("uncommitted changes")
        && !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("WIP on graph")
        && !screen.contains("popped")
}

/// Confirm `y` dropped the stash. Dirty README stays. Not pop.
fn documented_stash_dropped(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "1 changed")
        && readme_unstaged_badge(screen)
        && screen.contains("uncommitted changes")
        && graph_cursor_on(screen, "seed app")
        && !screen.contains("WIP on main")
        && !screen.contains("Drop stash@{0}?")
        && !status.contains("apply stash")
        && !status.contains("drop stash")
        && crumb.contains("dropped stash@{0}")
        && !crumb.contains("popped")
        && !crumb.contains("applied")
        && no_stash_wrong_ops(screen)
}

/// CSI-u Shift+S creates a stash, graph `a` applies it, CSI-u Shift+D
/// drops it.
///
/// Docs: Help GIT `S` = stash menu, `a p D` = focused stash apply/pop/drop.
/// Configuration: tree dirty file is create-only (`s`). Graph stash row
/// `a` applies and keeps the entry. `D` asks `y`/`n`. Live PTY after
/// first paint (cursor already on dirty README) did that create with
/// CSI-u Shift+S then overlay `s`: toast `Stashed 1 file`, README left
/// the tree, overlay closed. `l`/`j`/Tab/`j` focused app's `stash@{0}`.
/// Graph `a` restored README (`applied stash@{0}`) and kept the stash.
/// CSI-u Shift+D then `y` dropped it (`dropped stash@{0}`); README stayed
/// dirty. Not overlay-open only. Not `s` stage (`S `). Not Space `*`.
/// Not graph `p` pop. Not merger `WIP on graph`. Not a raw `S`/`D` byte.
///
/// After first paint the cursor is already on the dirty README. Do not
/// `/` search. A no-op, toast-only, stage, pop, overlay-open-only, or
/// the merger stash is red.
#[test]
fn pty_stash_create_apply_and_drop() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_dirty_readme_unstaged,
        "first paint: cursor on dirty README, unstaged, no stash overlay",
        WAIT,
    );

    tui.shift_letter('S');
    tui.wait_pred(
        stash_create_overlay_open,
        "CSI-u Shift+S opens create-only stash overlay on dirty README",
        WAIT,
    );

    tui.key('s');
    tui.wait_pred(
        documented_stash_created,
        "overlay s stashes README: Stashed 1 file, file leaves tree, overlay closes",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_stash_created,
        "stashed paint holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "No updates")
                && tree_has(screen, "app")
                && tree_has(screen, "lib")
                && !tree_has(screen, "README.md")
        },
        "l unfolds No updates and shows app (still hidden README)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        app_focused_stash_visible,
        "j focuses app under No updates; app graph shows stash@{0}",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        app_graph_working_tree_focused,
        "Tab focuses app graph on working tree; stash stays the next row",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        app_graph_stash_row_focused,
        "j focuses app stash@{0}; a apply / D drop hints; not merger",
        WAIT,
    );

    tui.key('a');
    tui.wait_pred(
        documented_stash_applied,
        "graph a applies: README returns dirty, stash@{0} stays, applied toast",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_stash_applied,
        "applied paint holds (not pop, not a flicker)",
        WAIT,
    );

    tui.shift_letter('D');
    tui.wait_pred(
        stash_drop_confirm_open,
        "CSI-u Shift+D opens Drop stash@{0}? confirm; stash and README stay",
        WAIT,
    );
    tui.key('y');
    tui.wait_pred(
        documented_stash_dropped,
        "y drops stash@{0}; README stays dirty; graph stash row is gone",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_stash_dropped,
        "dropped paint holds (not pop, not a flicker)",
        WAIT,
    );
}

/// Cells after `syncbox` on the left-tree repo row (branch + sync mark).
fn after_syncbox_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "syncbox")?;
    let at = line.find("syncbox")?;
    Some(line[at + "syncbox".len()..].to_string())
}

/// Trailing ASCII ahead-by-1 (`^1`) on the syncbox tree row.
fn syncbox_row_ahead(screen: &str) -> bool {
    after_syncbox_name(screen).is_some_and(|after| after.contains("^1") && after.contains("& main"))
}

/// Trailing clean `.` on the syncbox row after it lands in No updates.
fn syncbox_row_current(screen: &str) -> bool {
    after_syncbox_name(screen).is_some_and(|after| {
        after.contains("& main")
            && after.contains('.')
            && !after.contains("^1")
            && !after.contains("v1")
    })
}

/// Trailing ASCII behind-by-1 (`v1`) on the syncbox tree row.
///
/// A `v1` on the graph header or a full-screen substring must not pass.
fn syncbox_row_behind(screen: &str) -> bool {
    after_syncbox_name(screen).is_some_and(|after| after.contains("v1") && after.contains("& main"))
}

fn has_push_hint(screen: &str) -> bool {
    status_row(screen).contains("push")
}

fn has_fetch_hint(screen: &str) -> bool {
    status_row(screen).contains("fetch")
}

fn has_pull_hint(screen: &str) -> bool {
    status_row(screen).contains("pull")
}

fn no_wrong_push_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("nothing to push")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("no visible repos for that op")
        && !screen.contains("Stash ")
        && !screen.contains("pop stash")
}

fn no_wrong_fetch_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("no visible repos for that op")
        && !screen.contains("nothing behind to pull")
}

fn crumb_pushed_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Pushed 1 repo")
        && !crumb.contains("failed")
        && !crumb.contains("Fetched")
        && !crumb.contains("Pulled")
}

fn crumb_fetched_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Fetched 1 repo")
        && !crumb.contains("failed")
        && !crumb.contains("Pulled")
        && !crumb.contains("Pushed")
}

fn graph_subject_line(screen: &str, subject: &str) -> Option<String> {
    screen
        .lines()
        .find(|line| line.contains(subject))
        .map(str::to_string)
}

fn graph_subject_meta_line(screen: &str, subject: &str) -> Option<String> {
    let mut lines = screen.lines();
    lines.find(|line| line.contains(subject))?;
    lines.next().map(str::to_string)
}

/// HEAD is the local ahead tip. Origin still sits on the seed.
fn ahead_tip_is_head_not_origin(screen: &str) -> bool {
    graph_subject_line(screen, "ahead-tip-commit")
        .is_some_and(|line| line.contains("@  ahead-tip-commit"))
        && graph_subject_meta_line(screen, "ahead-tip-commit").is_some_and(|line| {
            line.contains("[+main]")
                && !line.contains("[+=main]")
                && !line.contains("[origin/main]")
        })
}

/// HEAD and origin/main fold into `[+=main]` on the pushed tip.
fn ahead_tip_is_head_and_origin(screen: &str) -> bool {
    graph_subject_line(screen, "ahead-tip-commit")
        .is_some_and(|line| line.contains("@  ahead-tip-commit"))
        && graph_subject_meta_line(screen, "ahead-tip-commit")
            .is_some_and(|line| line.contains("[+=main]") && !line.contains("[origin/main]"))
}

fn seed_is_origin_not_head(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("*  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox")
            .is_some_and(|line| line.contains("[origin/main]") && !line.contains("[+main]"))
}

fn seed_is_ancestor_no_origin_chip(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("*  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox").is_some_and(|line| {
            !line.contains("[origin/main]")
                && !line.contains("[+main]")
                && !line.contains("[+=main]")
        })
}

/// First paint: cursor on ahead syncbox. Graph HEAD is the local tip.
fn idle_ahead_syncbox(screen: &str) -> bool {
    let status = status_row(screen);
    let Some(top) = screen.lines().next() else {
        return false;
    };
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "1 ahead")
        && !tree_has(screen, "all current")
        && !tree_has(screen, "No updates")
        && syncbox_row_ahead(screen)
        && top.contains(" tree ")
        && top.contains(" graph")
        && screen.contains("main ^1")
        && screen.contains("Working tree clean")
        && ahead_tip_is_head_not_origin(screen)
        && seed_is_origin_not_head(screen)
        && !screen.contains("Pushed")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && has_push_hint(screen)
        && has_fetch_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace › syncbox"
        && no_wrong_push_overlays(screen)
}

/// CSI-u Shift+P pushed the focused ahead checkout. Origin matches HEAD.
fn documented_shift_p_pushed(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "all current")
        && tree_has(screen, "No updates")
        && !tree_has(screen, "1 ahead")
        && !tree_has(screen, "^1")
        && syncbox_row_current(screen)
        && !screen.contains("main ^1")
        && screen.contains("Working tree clean")
        && ahead_tip_is_head_and_origin(screen)
        && seed_is_ancestor_no_origin_chip(screen)
        && crumb_pushed_one(screen)
        && crumb_row(screen).contains("workspace › syncbox")
        && !crumb_row(screen).contains("[syncbox]")
        && has_fetch_hint(screen)
        && !has_push_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && no_wrong_push_overlays(screen)
}

/// CSI-u Shift+P pushes an ahead checkout against a local bare origin.
///
/// Docs: Help GIT `P` = push ahead/diverged/new. Configuration: repo /
/// checkout only (not workspace). Live PTY after first paint (cursor
/// already on `syncbox`) did that push: tree drops `^1` / `1 ahead`
/// (`all current`, No updates), graph HEAD stays `ahead-tip-commit`
/// (`@`, `[+=main]`), seed loses `[origin/main]`, push hint goes,
/// `Pushed 1 repo`. Not `f` fetch. Not `p` pull. Not a raw `P` byte.
/// Not a toast-only tick.
///
/// After first paint the cursor is already on the ahead repo. Do not
/// `/` search. A no-op, fetch, pull, toast-only, still-ahead, or the
/// wrong repo is red.
#[test]
fn pty_shift_p_csi_u_pushes_ahead() {
    let (_root, workspace) = ahead_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("syncbox", WAIT);
    tui.wait_contains("ahead-tip-commit", GIT_WAIT);
    tui.wait_pred(
        idle_ahead_syncbox,
        "first paint: cursor on ahead syncbox, main ^1, HEAD is local tip, push hint",
        WAIT,
    );

    tui.shift_letter('P');
    tui.wait_pred(
        documented_shift_p_pushed,
        "Shift+P pushes: Pushed 1 repo, HEAD ahead-tip-commit [+=main], no ^1, push hint gone",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_shift_p_pushed,
        "pushed paint holds (not a flicker or toast-only tick)",
        WAIT,
    );
}

/// First paint: workspace focused, syncbox hidden under folded No updates.
/// Looks in-sync. Fetch hint. No pull. No origin tip.
fn idle_unfetched_workspace(screen: &str) -> bool {
    let status = status_row(screen);
    let no_updates = tree_line_containing(screen, "No updates");
    let Some(top) = screen.lines().next() else {
        return false;
    };
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "syncbox")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "all current")
        && !tree_has(screen, "behind")
        && !tree_has(screen, "syncbox")
        && no_updates.is_some_and(|line| line.contains('>') && line.contains('1'))
        && top.contains(" tree ")
        && top.contains(" graph")
        && screen.contains("focus a repo for the graph")
        && !screen.contains("origin-tip-commit")
        && !screen.contains("Fetched")
        && !screen.contains("Pulled")
        && !screen.contains("Pushed")
        && has_fetch_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace"
        && no_wrong_fetch_overlays(screen)
}

/// Workspace `f` fetched remotes. Tree shows behind. Pull hint. Not a pull.
fn documented_workspace_fetch_behind(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "syncbox")
        && tree_has(screen, "1 behind")
        && !tree_has(screen, "all current")
        && !tree_has(screen, "No updates")
        && syncbox_row_behind(screen)
        && screen.contains("focus a repo for the graph")
        && !screen.contains("origin-tip-commit")
        && crumb_fetched_one(screen)
        && has_fetch_hint(screen)
        && has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Pulled")
        && !screen.contains("Pushed")
        && no_wrong_fetch_overlays(screen)
}

/// `j` onto syncbox after fetch: graph shows the origin tip. HEAD stays seed.
fn documented_fetch_graph_behind(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "1 behind")
        && syncbox_row_behind(screen)
        && screen.contains("origin-tip-commit")
        && screen.contains("seed syncbox")
        && screen.contains("[origin/main]")
        && screen.contains("Working tree clean")
        && screen.contains("main v1")
        && !screen.contains("focus a repo for the graph")
        && crumb_row(screen).contains("workspace › syncbox")
        && has_fetch_hint(screen)
        && has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Pulled")
        && !screen.contains("Pushed")
        && no_wrong_fetch_overlays(screen)
}

/// `f` fetches remotes against a local bare origin. Must mark behind.
///
/// Docs: Help GIT `f` = fetch remotes. Configuration: `git fetch --quiet`
/// for the focused checkout, or primary checkouts on the workspace row.
/// Live PTY after first paint (workspace cursor, folded No updates) did
/// that fetch: tree `v1` / `1 behind`, pull hint, `Fetched 1 repo`. `j`
/// onto syncbox paints `origin-tip-commit` on the graph. HEAD stays
/// `seed syncbox`. Not `p` pull. Not Shift+P push. Not a toast-only tick.
///
/// After first paint the cursor is already on the workspace row. Do not
/// `/` search. A no-op, pull, push, toast-only, missing behind mark, or
/// the wrong repo is red.
#[test]
fn pty_fetch_local_remote_marks_behind() {
    let (_root, workspace) = unfetched_behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("No updates", WAIT);
    tui.wait_pred(
        idle_unfetched_workspace,
        "first paint: workspace cursor, folded No updates, in-sync, fetch hint, no pull",
        WAIT,
    );

    tui.key('f');
    tui.wait_pred(
        documented_workspace_fetch_behind,
        "f fetches remotes: Fetched 1 repo, syncbox v1, 1 behind, pull hint, not pulled",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_workspace_fetch_behind,
        "fetched behind paint holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        documented_fetch_graph_behind,
        "j onto syncbox: graph origin-tip-commit, HEAD stays seed, still v1",
        GIT_WAIT,
    );
}

fn no_wrong_pull_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("no visible repos for that op")
        && !screen.contains("Stash ")
        && !screen.contains("pop stash")
}

fn crumb_pulled_one(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    crumb.contains("Pulled 1 repo")
        && !crumb.contains("failed")
        && !crumb.contains("Fetched")
        && !crumb.contains("Pushed")
}

/// Origin tip is a remote-only commit. HEAD is still the seed.
fn origin_tip_is_remote_not_head(screen: &str) -> bool {
    graph_subject_line(screen, "origin-tip-commit")
        .is_some_and(|line| line.contains("*  origin-tip-commit"))
        && graph_subject_meta_line(screen, "origin-tip-commit")
            .is_some_and(|line| line.contains("[origin/main]") && !line.contains("[+=main]"))
}

/// HEAD moved to the origin tip. Local and origin/main fold into `[+=main]`.
fn origin_tip_is_head(screen: &str) -> bool {
    graph_subject_line(screen, "origin-tip-commit")
        .is_some_and(|line| line.contains("@  origin-tip-commit"))
        && graph_subject_meta_line(screen, "origin-tip-commit")
            .is_some_and(|line| line.contains("[+=main]") && !line.contains("[origin/main]"))
}

fn seed_is_head(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("@  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox")
            .is_some_and(|line| line.contains("[+main]") && !line.contains("[+=main]"))
}

fn seed_is_ancestor_not_head(screen: &str) -> bool {
    graph_subject_line(screen, "seed syncbox").is_some_and(|line| line.contains("*  seed syncbox"))
        && graph_subject_meta_line(screen, "seed syncbox")
            .is_some_and(|line| !line.contains("[+main]") && !line.contains("[+=main]"))
}

/// First paint: cursor on behind syncbox. Graph HEAD is still the seed.
fn idle_behind_syncbox(screen: &str) -> bool {
    let status = status_row(screen);
    let Some(top) = screen.lines().next() else {
        return false;
    };
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "# workspace")
        && tree_has(screen, "1 behind")
        && !tree_has(screen, "all current")
        && !tree_has(screen, "No updates")
        && syncbox_row_behind(screen)
        && top.contains(" tree ")
        && top.contains(" graph")
        && screen.contains("main v1")
        && screen.contains("Working tree clean")
        && origin_tip_is_remote_not_head(screen)
        && seed_is_head(screen)
        && !screen.contains("Pulled")
        && !screen.contains("Fetched")
        && !screen.contains("Pushed")
        && has_fetch_hint(screen)
        && has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && crumb_row(screen).trim() == "workspace › syncbox"
        && no_wrong_pull_overlays(screen)
}

/// `p` pulled the focused behind checkout. HEAD is the origin tip.
fn documented_p_pulled(screen: &str) -> bool {
    let status = status_row(screen);
    tree_cursor_on(screen, "syncbox")
        && !tree_cursor_on(screen, "workspace")
        && tree_has(screen, "all current")
        && tree_has(screen, "No updates")
        && !tree_has(screen, "1 behind")
        && !tree_has(screen, "v1")
        && syncbox_row_current(screen)
        && !screen.contains("main v1")
        && screen.contains("Working tree clean")
        && origin_tip_is_head(screen)
        && seed_is_ancestor_not_head(screen)
        && crumb_pulled_one(screen)
        && crumb_row(screen).contains("workspace › syncbox")
        && !crumb_row(screen).contains("[syncbox]")
        && has_fetch_hint(screen)
        && !has_pull_hint(screen)
        && status.contains(" tree")
        && status.contains(" split")
        && !screen.contains("Fetched")
        && !screen.contains("Pushed")
        && no_wrong_pull_overlays(screen)
}

/// `p` pulls a behind checkout against a local bare origin.
///
/// Docs: Help GIT `p` = pull behind. Configuration: focused checkout →
/// that path. Live PTY after first paint (cursor already on `syncbox`)
/// did that pull: tree drops `v1` / `1 behind` (`all current`, No
/// updates), graph HEAD is `origin-tip-commit` (`@`, `[+=main]`), seed
/// stays an ancestor, pull hint goes, `Pulled 1 repo`. Not `f` fetch.
/// Not Shift+P push. Not a toast-only tick. Not graph stash pop.
///
/// After first paint the cursor is already on the behind repo. Do not
/// `/` search. A no-op, fetch-only, push, toast-only, still-behind, or
/// the wrong repo is red.
#[test]
fn pty_pull_behind_local_remote() {
    let (_root, workspace) = behind_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("syncbox", WAIT);
    tui.wait_contains("origin-tip-commit", GIT_WAIT);
    tui.wait_pred(
        idle_behind_syncbox,
        "first paint: cursor on behind syncbox, main v1, HEAD is seed, pull hint",
        WAIT,
    );

    tui.key('p');
    tui.wait_pred(
        documented_p_pulled,
        "p pulls: Pulled 1 repo, HEAD origin-tip-commit [+=main], no v1, pull hint gone",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_p_pulled,
        "pulled paint holds (not a flicker or toast-only tick)",
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

/// MOVE lists `/` as pane search, not help-local search.
fn help_lists_slash_search_focused_pane(screen: &str) -> bool {
    let compact = screen.split_whitespace().collect::<Vec<_>>().join(" ");
    compact.contains("MOVE")
        && compact.contains("/ search focused pane")
        && compact.contains("(Enter")
        && compact.contains("arms)")
        && compact.contains("/ search help")
}

/// Empty `/` prompt on the tree. Help overlay and an armed chip are other keys.
fn pane_search_prompt_on_tree(screen: &str) -> bool {
    screen.contains("SEARCH")
        && screen.contains("▏")
        && screen.contains("Enter arms query")
        && screen.contains("n/N after Enter")
        && !screen.contains("MOVE")
        && !screen.contains("HELP  /")
        && !screen.contains("/ search help")
        && !screen.contains("/merger")
        && !screen.contains("no match")
        && !screen.contains("drill")
        && !screen.contains("[workspace]")
        && !screen.contains("[merger]")
}

/// Typing `/merger` on the tree. Help `/` paints `HELP  /merger`.
fn typing_merger_tree_hit(screen: &str) -> bool {
    pane_search_prompt_on_tree(screen)
        && screen.contains("merger▏")
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "No updates")
        && !tree_cursor_on(screen, "workspace")
        && screen.contains("workspace › merger")
        && !screen.contains("[merger]")
        && screen.contains("WIP on graph")
        && screen.contains("Working tree clean")
        && !screen.contains("+dirty")
        && !screen.contains("UNSTAGED")
        && !screen.contains("app/README.md")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "No updates")
        && !tree_has(screen, "lib")
        && !screen.contains("notes")
}

/// Enter arms `/merger` on the tree. SEARCH is gone. Stay left.
fn armed_merger_search_left(screen: &str) -> bool {
    screen.contains("/merger")
        && screen.contains("? help")
        && screen.contains("focus right")
        && !screen.contains("SEARCH")
        && !screen.contains("Enter arms query")
        && !screen.contains("MOVE")
        && !screen.contains("HELP  /")
        && !screen.contains("drill")
        && !screen.contains("[merger]")
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && screen.contains("workspace › merger")
        && screen.contains("WIP on graph")
        && !screen.contains("+dirty")
        && !screen.contains("UNSTAGED")
        && tree_has(screen, "README.md")
        && !tree_has(screen, "lib")
}

/// Help `/`, then live `/` + query on the focused tree.
///
/// Docs + MOVE: search the focused pane by substring (rows stay visible).
/// `/` paints SEARCH on that pane. Typing jumps the cursor. Enter arms
/// `/query`. Help `/` is a different overlay (`HELP  /…`). A no-op, help
/// search, the launch README row, Tab (`[merger]` / `drill`), a filter
/// that hides README, or a paint-changed-only assert cannot pass.
#[test]
fn pty_slash_pane_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        |screen| {
            tree_has(screen, "README.md")
                && tree_has(screen, "merger")
                && tree_has(screen, "No updates")
                && !tree_has(screen, "lib")
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "merger")
                && screen.contains("+dirty")
                && screen.contains("UNSTAGED")
                && !screen.contains("WIP on graph")
                && !screen.contains("SEARCH")
                && !screen.contains("workspace › merger")
        },
        "launch focuses README; merger is visible; graph subject is not loaded",
        WAIT,
    );

    tui.key('?');
    tui.wait_pred(
        help_lists_slash_search_focused_pane,
        "help MOVE lists / search focused pane (Enter arms)",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("/ search help")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("focus right")
        },
        "Esc closes help so / is pane search, not help search",
        WAIT,
    );

    tui.key('/');
    tui.wait_pred(
        |screen| {
            pane_search_prompt_on_tree(screen)
                && tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "merger")
                && !screen.contains("workspace › merger")
                && screen.contains("+dirty")
        },
        "/ arms SEARCH on the tree; empty query must not jump or open help",
        WAIT,
    );

    tui.keys("merger");
    tui.wait_pred(
        typing_merger_tree_hit,
        "/merger jumps to merger (a no-op stays on README; help search is HELP  /merger)",
        GIT_WAIT,
    );

    tui.enter();
    tui.wait_pred(
        armed_merger_search_left,
        "Enter arms /merger on the tree; SEARCH closes; stay left on merger",
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

/// Help MOVE lists `gg G` as top/bottom of the focused pane. CSI-u `G`
/// jumps the tree. Two `g` bytes return to the root.
///
/// Docs + MOVE: `gg` (second `g` within ~400ms) is the start of the
/// focused list. Lone `g` expires with no move. `G` is the end. Live PTY
/// after first paint (cursor on dirty README) left last rows below the
/// fold. CSI-u `G` (`CSI 103 ; 2 : 1 u` press, `: 3` release) landed on
/// folded No updates. Two `g` bytes landed on `# workspace`. A raw `G`
/// byte is a different path. PageDown is one viewport. Extra dirty files
/// keep the last row off-screen at launch, so a no-op, PageDown, or a
/// jump onto `page-29` / merger cannot pass. Cursor bar, right pane, and
/// fold must all move.
#[test]
fn pty_gg_and_g_jump_workspace_tree() {
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
                && help_lists_gg_g_top_bottom(screen)
                && screen.contains("top / bottom of focused")
        },
        "help MOVE lists gg G as top/bottom of focused pane",
        WAIT,
    );
    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("? help")
                && screen.contains("UNSTAGED")
        },
        "Esc closes help so gg/G are tree jumps, not help keys",
        WAIT,
    );

    tui.key('g');
    tui.wait_ms(500);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && !tree_cursor_on(screen, "workspace")
                && !tree_cursor_on(screen, "No updates")
                && screen.contains("UNSTAGED")
                && tree_has(screen, "workspace")
                && !tree_has(screen, "No updates")
        },
        "lone g expires with no move (gg would land on workspace; G would land on No updates)",
        WAIT,
    );

    tui.shift_letter('G');
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
        "CSI-u G jumps to the last tree row (a no-op stays on README; PgDn stays mid-list; merger would load its graph)",
        GIT_WAIT,
    );

    tui.key('l');
    tui.wait_pred(
        |screen| tree_has(screen, "lib") && tree_cursor_on(screen, "No updates"),
        "G then l opens No updates (G actually selected that row)",
        WAIT,
    );

    tui.gg();
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
        "gg jumps to the first tree row (a no-op stays on No updates; PgUp from the end lands on a page file)",
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

/// Default mouse-on: clipped tree row, README short diff, cursor stays.
fn tree_sgr_hscroll_clipped_readme_focus(screen: &str, readme_row: u16) -> bool {
    harness::clipped_long_path_row(screen).is_some()
        && tree_cursor_bar_on_row(screen, readme_row)
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !screen.contains("Mouse on")
        && !status_has_tree_hscroll_tail(screen)
}

/// Documented tree pan: `TAIL99` on the tree row, prefix gone, README cursor.
fn documented_tree_sgr_hscroll_panned(screen: &str, readme_row: u16) -> bool {
    tree_is_panned_to_tail(screen)
        && tree_row_containing(screen, TREE_HSCROLL_TAIL).is_some()
        && tree_cursor_bar_on_row(screen, readme_row)
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && screen.contains("app/README.md")
        && !screen.contains("SEARCH")
        && !screen.contains("Mouse off")
        && !status_has_tree_hscroll_tail(screen)
}

fn help_lists_gg_g_top_bottom(screen: &str) -> bool {
    screen.contains("gg   G") && screen.contains("top / bottom of focused")
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

/// Left-click a tree row selects it and loads that row's right pane.
///
/// Docs: SGR press+release. Must change the right pane. Setup clicks in
/// the hscroll test and `m` mouse-toggle clicks are not this claim.
/// Chevron click and right-pane click are separate leftovers.
///
/// Live PTY after first paint (cursor already on dirty README, file-diff
/// on the right): SGR press+release on the merger *label* (not the
/// chevron) moved the cursor to merger and replaced UNSTAGED / `+dirty`
/// with that repo's graph (`WIP on graph`, working tree clean). Stay
/// left. Not Enter. Not fold.
///
/// A no-op, cursor-only, chevron fold, right-pane click (`[workspace]`),
/// or Enter drill (`[merger]`) cannot pass.
#[test]
fn pty_click_selects_tree_row() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        super::documented_launch_first_paint,
        "first paint: README file-diff (click has not run)",
        WAIT,
    );

    let row = tree_row_containing(&tui.screen(), "merger")
        .unwrap_or_else(|| panic!("merger row:\n{}", tui.screen()));
    assert_ne!(
        TREE_LABEL_COL, TREE_DEPTH1_CHEVRON_COL,
        "label click must not hit the depth-1 chevron"
    );
    tui.sgr_click(TREE_LABEL_COL, row);
    tui.wait_pred(
        click_selects_merger_row,
        "SGR press+release on merger label selects that row and loads its graph",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        click_selects_merger_row,
        "selected merger graph holds (not a flicker, cursor-only, or toast-only)",
        WAIT,
    );
}

/// Clicked merger label: cursor + graph pane, stay left, no fold, no Enter.
fn click_selects_merger_row(screen: &str) -> bool {
    super::merger_graph_left_unfocused(screen)
        && tree_cursor_on(screen, "merger")
        && !tree_cursor_on(screen, "README.md")
        && !tree_cursor_on(screen, "app")
        && !tree_cursor_on(screen, "workspace")
        && !tree_cursor_on(screen, "No updates")
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && tree_has(screen, "merger")
        && tree_dir_expanded(screen, "app")
        && tree_dir_expanded(screen, "workspace")
        && no_updates_group_folded(screen)
        && tree_pane_focused(screen)
        && !screen.contains("[workspace]")
        && !screen.contains("[merger]")
        && !screen.contains("┌ files")
        && !screen.contains("wip.txt")
        && !screen.contains("UNSTAGED")
        && !screen.contains("+dirty")
        && !screen.contains("app/README.md")
        && (screen.contains("Working tree") || screen.contains("working tree clean"))
        && screen.contains("WIP on graph")
        && crumb_row(screen).contains("workspace › merger")
        && status_row(screen).contains("focus right")
        && status_row(screen).contains(" tree")
        && status_row(screen).contains(" split")
        && no_wrong_overlays(screen)
        && no_mouse_toggle_toast(screen)
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

fn pane_top(screen: &str) -> &str {
    screen.lines().next().unwrap_or("")
}

/// Left tree focused, right graph unfocused. Not files / not a file diff.
fn tree_pane_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" tree ")
        && top.contains(" graph")
        && !top.contains(" graph ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

/// Left tree unfocused, right graph focused. Not files / not a file diff.
fn graph_pane_focused(screen: &str) -> bool {
    let top = pane_top(screen);
    top.contains(" graph ")
        && top.contains(" tree")
        && !top.contains(" tree ")
        && !top.contains(" files")
        && !top.contains(" diff")
}

fn graph_cursor_on(screen: &str, needle: &str) -> bool {
    screen.lines().any(|line| {
        let right = right_of_split(line);
        right.contains('\u{258C}') && right.contains(needle)
    })
}

fn no_mouse_toggle_toast(screen: &str) -> bool {
    !screen.contains("Mouse off") && !screen.contains("Mouse on")
}

fn no_merge_confirm(screen: &str) -> bool {
    !screen.contains("fast-forward if possible")
        && !screen.contains("Merge main into")
        && !screen.contains("otherwise a merge commit")
}

fn no_wrong_merge_overlays(screen: &str) -> bool {
    !screen.contains("MOVE")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("Stash ")
        && !screen.contains("┌ files")
        && no_mouse_toggle_toast(screen)
}

fn focusbox_diverged_graph_body(screen: &str) -> bool {
    screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("noise-leaf-commit")
        && screen.contains("focus-root-commit")
        && screen.contains("[+feature/keep]")
        && screen.contains("[main]")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
}

/// First paint: focusbox on the tree. Graph `m` has not run.
fn idle_focusbox_before_graph_merge(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    tree_cursor_on(screen, "focusbox")
        && tree_pane_focused(screen)
        && tree_has(screen, "feature/keep")
        && crumb.contains("workspace › focusbox")
        && !crumb.contains("[focusbox]")
        && !crumb.contains("Merged")
        && status.contains("focus right")
        && !status.contains("drill")
        && !screen.contains("Merge branch")
        && no_merge_confirm(screen)
        && no_wrong_merge_overlays(screen)
}

/// Tab focused the graph. HEAD is still `keep-leaf-commit`. Merge is idle.
fn graph_focused_diverged_before_merge(screen: &str) -> bool {
    let status = status_row(screen);
    let crumb = crumb_row(screen);
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "focusbox")
        && focusbox_diverged_graph_body(screen)
        && crumb.contains("workspace › [focusbox]")
        && !crumb.contains("Merged")
        && !crumb.contains("Fast-forwarded")
        && status.contains("drill")
        && status.contains("Esc")
        && status.contains("back")
        && !screen.contains("Merge branch")
        && no_merge_confirm(screen)
        && no_wrong_merge_overlays(screen)
}

/// Graph cursor on `main-leaf-commit`. Hint `m` is merge. Overlay closed.
fn main_leaf_ready_to_merge(screen: &str) -> bool {
    let status = status_row(screen);
    graph_focused_diverged_before_merge(screen)
        && graph_cursor_on(screen, "main-leaf-commit")
        && !graph_cursor_on(screen, "keep-leaf-commit")
        && !graph_cursor_on(screen, "working tree")
        && status.contains("checkout")
        && status.contains("create branch")
        && status.contains("merge")
        && screen.contains("/main-leaf-commit")
}

/// `PendingConfirm::MergeIntoHead` boxed overlay. Not mouse. Not a write yet.
fn merge_into_head_confirm(screen: &str) -> bool {
    graph_pane_focused(screen)
        && screen.contains("Merge main into feature/keep?")
        && screen.contains("fast-forward if possible, otherwise a merge commit")
        && screen.contains("merge")
        && screen.contains("cancel")
        && screen.contains("main-leaf-commit")
        && screen.contains("keep-leaf-commit")
        && !screen.contains("Merged")
        && !screen.contains("Fast-forwarded")
        && !screen.contains("Already up to date")
        && !screen.contains("Merge branch")
        && no_mouse_toggle_toast(screen)
        && !screen.contains("MOVE")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("┌ files")
}

/// `y` created a merge commit into HEAD. Fast-forward / no-op cannot pass.
fn documented_graph_merge_commit(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && screen.contains("Merge branch 'main' into feature/keep")
        && screen.contains("keep-leaf-commit")
        && screen.contains("main-leaf-commit")
        && screen.contains("[+feature/keep]")
        && (screen.contains("working tree clean") || screen.contains("Working tree clean"))
        && tree_has(screen, "feature/keep")
        && crumb.contains("Merged main")
        && !crumb.contains("failed")
        && !crumb.contains("Fast-forwarded")
        && !crumb.contains("Already up to date")
        && !screen.contains("Merge main into feature/keep?")
        && no_merge_confirm(screen)
        && no_mouse_toggle_toast(screen)
        && !screen.contains("MOVE")
        && !screen.contains("Create branch")
        && !screen.contains("Focus branches")
        && !screen.contains("┌ files")
        && status.contains("drill")
        && status.contains(" tree")
        && status.contains(" split")
}

/// Graph `m` merges the focused commit into HEAD.
///
/// Docs: Help GIT `m` = graph merge into HEAD. Keymap: graph-focused `m`
/// is `Action::GraphMerge`. Confirm is `PendingConfirm::MergeIntoHead`.
/// Yes runs `merge_into_head` (fast-forward when possible, otherwise a
/// merge commit). Tree `m` is `ToggleMouse` (`pty_m_toggles_mouse_capture`).
///
/// After first paint the cursor is already on `focusbox`. Tab focuses the
/// graph. `/` lands on diverged `main-leaf-commit` (HEAD is
/// `keep-leaf-commit`, so this cannot fast-forward or be already up to
/// date). `m` then `y` must paint the merge-commit subject and `Merged
/// main`. A no-op, mouse toggle, overlay-only, toast-only, fast-forward,
/// or already-up-to-date is red.
#[test]
fn pty_graph_merge_creates_commit() {
    let (_root, workspace) = focus_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("focusbox", WAIT);
    tui.wait_pred(
        idle_focusbox_before_graph_merge,
        "first paint: focusbox on the tree, no merge confirm, not mouse toggle",
        WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_focused_diverged_before_merge,
        "Tab focuses the graph: keep and main tips, HEAD still keep, merge idle",
        GIT_WAIT,
    );

    tui.search("main-leaf-commit");
    tui.wait_pred(
        main_leaf_ready_to_merge,
        "graph cursor on main-leaf-commit; m merge hint; overlay closed",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);

    tui.key('m');
    tui.wait_pred(
        merge_into_head_confirm,
        "graph m opens Merge main into feature/keep confirm (not mouse, not a write)",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        merge_into_head_confirm,
        "merge confirm holds (not a flicker or toast-only tick)",
        WAIT,
    );

    tui.key('y');
    tui.wait_pred(
        documented_graph_merge_commit,
        "y creates merge commit into HEAD: Merge branch 'main' into feature/keep, Merged main",
        GIT_WAIT,
    );
}

fn no_wrong_stash_pop_overlays(screen: &str) -> bool {
    !screen.contains("SEARCH")
        && !screen.contains("MOVE")
        && !screen.contains("Stash ")
        && !screen.contains("Drop stash@{")
        && !screen.contains("nothing behind to pull")
        && !screen.contains("no visible repos for that op")
        && no_mouse_toggle_toast(screen)
}

fn after_wip_name(screen: &str) -> Option<String> {
    let line = tree_line_containing(screen, "wip.txt")?;
    let at = line.find("wip.txt")?;
    Some(line[at + "wip.txt".len()..].to_string())
}

/// Restored `wip.txt` on the merger tree. Badge `A` is the staged add.
fn merger_wip_added(screen: &str) -> bool {
    tree_has(screen, "wip.txt")
        && tree_cursor_on(screen, "merger")
        && after_wip_name(screen).is_some_and(|after| after.contains('A'))
}

fn graph_stash_still_listed(screen: &str) -> bool {
    let right = right_pane(screen);
    right.contains("WIP on graph") || right.contains("stash@{0}")
}

fn no_pull_or_other_stash_write(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    !crumb.contains("Pulled")
        && !crumb.contains("applied")
        && !crumb.contains("dropped")
        && !crumb.contains("Stashed")
        && !crumb.contains("failed")
}

/// Tab focused the merger graph. HEAD is clean. Stash is listed. Pop idle.
fn graph_focused_merger_stash_listed(screen: &str) -> bool {
    super::merger_graph_drilled_right(screen)
        && graph_pane_focused(screen)
        && graph_stash_still_listed(screen)
        && !tree_has(screen, "wip.txt")
        && !crumb_row(screen).contains("popped")
        && no_pull_or_other_stash_write(screen)
        && no_wrong_stash_pop_overlays(screen)
}

/// Tab lands on the uncommitted row. Stash is the next `j`.
fn graph_focused_merger_before_stash_pop(screen: &str) -> bool {
    graph_focused_merger_stash_listed(screen)
        && graph_cursor_on(screen, "working tree")
        && !graph_cursor_on(screen, "WIP on graph")
}

/// Graph cursor on `stash@{0}` (`WIP on graph`). Hint `p` is pop stash.
fn stash_row_ready_to_pop(screen: &str) -> bool {
    let status = status_row(screen);
    graph_focused_merger_stash_listed(screen)
        && graph_cursor_on(screen, "WIP on graph")
        && !graph_cursor_on(screen, "working tree")
        && status.contains("apply stash")
        && status.contains("pop stash")
        && status.contains("drop stash")
        && !status.contains("pull")
}

/// Graph `p` popped `stash@{0}`: apply + drop. Apply-only / drop-only fail.
fn documented_graph_stash_pop(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    graph_pane_focused(screen)
        && tree_cursor_on(screen, "merger")
        && merger_wip_added(screen)
        && !graph_stash_still_listed(screen)
        && screen.contains("uncommitted changes")
        && !screen.contains("working tree clean")
        && crumb.contains("popped stash@{0}")
        && no_pull_or_other_stash_write(screen)
        && !status.contains("pop stash")
        && !status.contains("apply stash")
        && !status.contains("drop stash")
        && status.contains("drill")
        && status.contains(" tree")
        && status.contains(" split")
        && no_wrong_stash_pop_overlays(screen)
}

/// Graph `p` pops the focused stash (apply + drop).
///
/// Docs: Help GIT `a p D` = focused stash apply/pop/drop. Keymap: graph
/// stash row `p` is `Action::GraphStashPop` (`git stash pop` of that
/// `stash@{n}`). Workspace / tree `p` is `Action::Pull`
/// (`pty_pull_behind_local_remote`). Overlay `S` then `a` / `D` is
/// `pty_stash_create_apply_and_drop`. Pop runs immediately. Drop still
/// confirms.
///
/// After first paint, `j` lands on merger. Tab focuses the graph. `j`
/// selects `stash@{0}` (`WIP on graph`). `p` must restore `wip.txt` on
/// the merger tree, drop that stash from the graph, and toast `popped
/// stash@{0}`. A no-op, workspace pull, apply-only (stash stays),
/// drop-only (no `wip.txt`), overlay, or toast-only is red.
#[test]
fn pty_stash_graph_pop() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        super::documented_launch_first_paint,
        "first paint: README file diff (graph stash pop has not run)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        super::merger_graph_left_unfocused,
        "j lands on merger and loads its graph (left focus, stash still listed)",
        GIT_WAIT,
    );

    tui.tab();
    tui.wait_pred(
        graph_focused_merger_before_stash_pop,
        "Tab focuses the merger graph: working tree clean, stash@{0} listed, pop idle",
        GIT_WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        stash_row_ready_to_pop,
        "j selects stash@{0}: graph cursor on WIP on graph; p pop stash hint",
        WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        stash_row_ready_to_pop,
        "stash row holds (not a flicker); overlay closed; not pull",
        WAIT,
    );

    tui.key('p');
    tui.wait_pred(
        documented_graph_stash_pop,
        "graph p pops stash@{0}: wip.txt A on merger, stash gone, popped toast",
        GIT_WAIT,
    );
    tui.wait_ms(SETTLE_MS);
    tui.wait_pred(
        documented_graph_stash_pop,
        "popped paint holds (not a flicker, toast-only tick, or apply-only)",
        WAIT,
    );
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

/// Idle first paint with live watch on: tree + file chrome, no `r` toast.
///
/// Help `r` is `refresh now`. Watch is the poll (`WS_STATUS_WATCH_MS`).
fn idle_tui_ready_for_watch(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && screen.contains(" tree")
        && screen.contains("? help")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && !screen.contains("MOVE")
        && !screen.contains("SEARCH")
        && !refresh_now_toast(screen)
}

/// `r` reload toast. Watch apply must not paint this.
fn refresh_now_toast(screen: &str) -> bool {
    screen.contains("refreshed app") || screen.contains("refreshed workspace")
}

/// New dirty path on the tree with the untracked `A` badge, not chrome-only.
fn tree_shows_watch_dirty_path(screen: &str, name: &str) -> bool {
    tree_line_containing(screen, name).is_some_and(|line| line.contains("A "))
}

/// Live watch paints a new dirty path while nav keys arrive (no `r`).
///
/// Docs: watch apply while keys arrive (no `r`). Help/keymap: `r` is
/// `refresh now` (`pty_r_refreshes_new_dirty_file`, watch off). `watch.rs`:
/// dirty paths sit in `checkout_watch_identity`, so an edit is a real poll
/// move. A no-op, a toast/status tick without the path, a frozen tree until
/// `r`, or a path that only appears after `r` cannot pass.
#[test]
fn pty_watch_applies_while_keys_arrive() {
    let (_root, workspace) = daily_workspace();
    let marker = format!(
        "watch-live-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", "500")]);
    tui.wait_pred(
        idle_tui_ready_for_watch,
        "first paint: tree + file chrome, watch on, no r toast",
        WAIT,
    );
    tui.wait_pred(
        |screen| !tree_has(screen, &marker),
        "new dirty path is absent before the disk write",
        WAIT,
    );

    fs::write(workspace.join("app").join(&marker), "live-watch\n").unwrap();

    let mut down = true;
    tui.wait_pred_while(
        |screen| tree_shows_watch_dirty_path(screen, &marker) && !refresh_now_toast(screen),
        "watch paints the new dirty path on the tree while j/k arrive (no r)",
        GIT_WAIT,
        |session| {
            session.key(if down { 'j' } else { 'k' });
            down = !down;
        },
    );

    let screen = tui.screen();
    let left = left_tree(&screen);
    assert!(
        tree_shows_watch_dirty_path(&screen, &marker),
        "new dirty path must paint on the tree (toast/status-only fails); screen:\n{screen}"
    );
    assert!(
        tree_has(&screen, "README.md") && left.contains("2 changed"),
        "watch dirty-path identity must keep README and bump the workspace change count; screen:\n{screen}"
    );
    harness::assert_absent(&screen, "refreshed app");
    harness::assert_absent(&screen, "refreshed workspace");
    harness::assert_absent(&screen, "MOVE");
    assert_contains(&screen, "? help");
}

/// `r` reloads the focused repo while watch is off.
///
/// `PtySession` defaults to `WS_STATUS_WATCH_MS=0`. Live watch without `r`
/// is `pty_watch_applies_while_keys_arrive`.
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

/// Pinned chrome copy after the first Ctrl+C (`tui/ctrl_c_exit.rs`).
const CTRL_C_EXIT_PROMPT: &str = "Press Ctrl+C again to exit";

fn screen_line_from_end(screen: &str, from_end: usize) -> &str {
    let lines: Vec<&str> = screen.lines().collect();
    lines
        .get(lines.len().saturating_sub(from_end + 1))
        .copied()
        .unwrap_or("")
}

/// Idle daily seed: tree + status pills, breadcrumb on the penultimate row.
///
/// No quit prompt yet. Status is last. A help overlay cannot pass.
fn idle_tree_before_ctrl_c(screen: &str) -> bool {
    let status = screen_line_from_end(screen, 0);
    let crumb = screen_line_from_end(screen, 1);
    tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && status.contains(" tree")
        && status.contains("? help")
        && status.contains("focus right")
        && crumb.trim() == "workspace"
        && !crumb.contains("Ctrl+C")
        && !status.contains(CTRL_C_EXIT_PROMPT)
        && !screen.contains(CTRL_C_EXIT_PROMPT)
        && !screen.contains("MOVE")
}

/// First Ctrl+C pins the quit prompt between breadcrumb and status pills.
///
/// Fail if the copy is only a breadcrumb toast, if status pills vanish, or
/// if the tree is gone. The process-alive check sits on the caller.
fn first_ctrl_c_pinned_prompt(screen: &str) -> bool {
    let status = screen_line_from_end(screen, 0);
    let prompt = screen_line_from_end(screen, 1);
    let crumb = screen_line_from_end(screen, 2);
    prompt.trim() == CTRL_C_EXIT_PROMPT
        && crumb.trim() == "workspace"
        && !crumb.contains("Ctrl+C")
        && status.contains(" tree")
        && status.contains("? help")
        && status.contains("focus right")
        && !status.contains(CTRL_C_EXIT_PROMPT)
        && tree_has(screen, "README.md")
        && tree_has(screen, "app")
        && screen.contains("UNSTAGED")
        && !screen.contains("MOVE")
}

/// First Ctrl+C keeps the process and pins the quit prompt.
///
/// Docs + VIEW: `Ctrl-C Ctrl-C` / `quit (press twice)`. First press is not
/// `q` and not the second Ctrl+C. Help overlay lists the row
/// (`pty_help_overlay`). This claim is idle-tree chrome after one press.
///
/// Encoding: CSI-u Control+c (`CSI 99 ; 5 : 1 u` press, `: 3` release).
/// The live loop requested `REPORT_ALL_KEYS_AS_ESCAPE_CODES` plus event
/// types. C0 `\x03` (`PtySession::ctrl`) is a different path. A live PTY
/// hunt after first paint painted the same pinned row for both encodings.
///
/// Documented result: process stays. Pinned chrome row between the
/// breadcrumb and the status pills shows `Press Ctrl+C again to exit`.
/// Tree and pills stay. Fail if the process exits, if the copy is missing
/// or only a breadcrumb toast, if the status line is replaced, or if
/// nothing happens. Teardown sends `q` (second Ctrl+C is not claimed).
#[test]
fn pty_ctrl_c_prompts_before_quit() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("UNSTAGED", GIT_WAIT);
    tui.wait_pred(
        idle_tree_before_ctrl_c,
        "first paint: tree + status pills, no quit prompt",
        WAIT,
    );
    tui.assert_running("before first Ctrl+C");

    tui.ctrl_letter('c');
    tui.wait_pred(
        first_ctrl_c_pinned_prompt,
        "first CSI-u Ctrl+C pins Press Ctrl+C again to exit between breadcrumb and status",
        WAIT,
    );
    tui.assert_running("after first Ctrl+C (must not quit)");

    tui.key('q');
    tui.wait_exit(WAIT);
}

/// Idle first paint: tree + file chrome, no Ctrl+C quit prompt.
///
/// `q` is help `quit`, not `quit (press twice)`. The status chip may
/// truncate (`…`) before `q` / `quit`, so this checks mounted TUI chrome
/// rather than the truncated hint.
fn idle_tui_ready_for_q(screen: &str) -> bool {
    tree_has(screen, "README.md")
        && screen.contains(" tree")
        && screen.contains("? help")
        && screen.contains("UNSTAGED")
        && screen.contains("+dirty")
        && !screen.contains("Press Ctrl+C again to exit")
        && !screen.contains("MOVE")
}

/// `q` quits immediately (help `q` quit, not the Ctrl+C chord).
///
/// Docs: process exits. Help/keymap: `q` / "quit" — not "press twice"
/// (`Ctrl-C Ctrl-C`). A no-op, a Ctrl+C arm, a twice-to-quit prompt, a
/// crash, or a still-alive process with painted chrome cannot pass.
/// `wait_exit` alone is not enough: the Ctrl+C prompt must never paint.
#[test]
fn pty_q_quits_immediately() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open(&workspace);
    tui.wait_pred(
        idle_tui_ready_for_q,
        "first paint: tree + file chrome, no Ctrl+C quit prompt",
        WAIT,
    );

    tui.key('q');
    tui.wait_exit_without("Press Ctrl+C again to exit", WAIT);
}

/// Unique graph subject for `fast` (`seed_repo` commit message).
const FAST_GRAPH_SUBJECT: &str = "seed fast";
/// Unique graph subject for `slow`. A focused-fast pane must not show this.
const SLOW_GRAPH_SUBJECT: &str = "seed slow";
/// Unique worktree body written into `fast` during streamed collect.
const STREAMED_MARKER_BODY: &str = "streamed-collect-body";

/// Right-pane cells, excluding top/bottom chrome (same rows as [`left_tree`]).
fn right_pane(screen: &str) -> String {
    let lines: Vec<&str> = screen.lines().collect();
    let end = lines.len().saturating_sub(2);
    let start = usize::from(end > 1);
    lines[start..end]
        .iter()
        .map(|line| right_of_split(line))
        .collect::<Vec<_>>()
        .join("\n")
}

fn right_of_split(line: &str) -> String {
    for sep in ["││", "┐┌", "┘└"] {
        if let Some(idx) = line.find(sep) {
            return line[idx + sep.len()..].to_string();
        }
    }
    String::new()
}

/// `WORKSPACE_STATUS_GIT` wrapper that blocks `git status` in `slow`.
struct SlowGitStatusBlock {
    shim: PathBuf,
    arm: PathBuf,
    wait: PathBuf,
    release: PathBuf,
}

impl SlowGitStatusBlock {
    fn install(workspace: &Path) -> Self {
        let shim_dir = workspace.join(".e2e-git-shim");
        fs::create_dir_all(&shim_dir).unwrap();
        let shim = shim_dir.join("git");
        let arm = shim_dir.join("arm");
        let wait = shim_dir.join("wait");
        let release = shim_dir.join("release");
        let real_git = std::env::var("WS_E2E_REAL_GIT").unwrap_or_else(|_| {
            if Path::new("/usr/bin/git").is_file() {
                "/usr/bin/git".into()
            } else {
                "git".into()
            }
        });
        let slow = workspace.join("slow");
        fs::write(
            &shim,
            format!(
                "#!/bin/sh\n\
                 real=\"{real_git}\"\n\
                 arm=\"{arm}\"\n\
                 waitf=\"{wait}\"\n\
                 rel=\"{release}\"\n\
                 slow=\"{slow}\"\n\
                 is_status=0\n\
                 for a in \"$@\"; do\n\
                   case \"$a\" in\n\
                     status) is_status=1; break ;;\n\
                   esac\n\
                 done\n\
                 if [ \"$is_status\" = 1 ] && [ -f \"$arm\" ]; then\n\
                   case \"$PWD\" in\n\
                     \"$slow\"|\"$slow\"/*)\n\
                       : > \"$waitf\"\n\
                       while [ ! -f \"$rel\" ]; do\n\
                         sleep 0.05\n\
                       done\n\
                       ;;\n\
                   esac\n\
                 fi\n\
                 exec \"$real\" \"$@\"\n",
                real_git = real_git,
                arm = arm.display(),
                wait = wait.display(),
                release = release.display(),
                slow = slow.display(),
            ),
        )
        .unwrap();
        let mut perms = fs::metadata(&shim).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&shim, perms).unwrap();
        Self {
            shim,
            arm,
            wait,
            release,
        }
    }

    fn arm(&self) {
        fs::write(&self.arm, "1\n").unwrap();
    }

    fn release(&self) {
        fs::write(&self.release, "1\n").unwrap();
    }

    fn still_blocked(&self) -> bool {
        self.wait.exists() && !self.release.exists()
    }
}

/// Focused `fast` after first paint / click: graph pane, no marker yet.
///
/// Chrome/status rows are excluded. `slow`'s graph subject cannot pass.
fn focused_fast_clean_pane(screen: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    tree_has(screen, "fast")
        && tree_has(screen, "slow")
        && tree_cursor_on(screen, "fast")
        && right.contains("Working tree clean")
        && right.contains(FAST_GRAPH_SUBJECT)
        && !right.contains("Uncommitted changes")
        && !right.contains(SLOW_GRAPH_SUBJECT)
        && !right.contains("# slow")
        && !left.contains("streamed-e2e-")
        && !right.trim().is_empty()
}

/// Focused `fast` tree row + right pane after the marker lands.
///
/// Fail if only chrome ticks, if the pane stays clean/blank, or if `slow`
/// is the painted body. Marker must be on the tree; pane must show the
/// dirty graph or the new file.
fn focused_fast_updated_before_slow(screen: &str, marker: &str) -> bool {
    let left = left_tree(screen);
    let right = right_pane(screen);
    let pane_body = right.contains("Uncommitted changes")
        && (right.contains(FAST_GRAPH_SUBJECT)
            || right.contains(marker)
            || right.contains(STREAMED_MARKER_BODY));
    let pane_diff = right.contains(marker) || right.contains(STREAMED_MARKER_BODY);
    tree_has(screen, "fast")
        && tree_has(screen, "slow")
        && left.contains(marker)
        && !right.trim().is_empty()
        && (pane_body || pane_diff)
        && !right.contains("Working tree clean")
        && !right.contains(SLOW_GRAPH_SUBJECT)
        && !right.contains("# slow")
}

/// Streamed watch collect paints the focused checkout before a blocked peer.
///
/// Docs (`tui-tty-e2e.md` Streamed collect, `tui-rust.md`, `app.rs`): each
/// checkout result applies as it finishes. Unfinished paths stay on the
/// previous generation. The focused checkout is queued first; its pane
/// reloads as soon as that identity changes (`focused_repo_needs_pane`).
/// A slow `git status` must not hold the focused tree or pane. `r` and
/// watch-while-keys are out of scope.
///
/// Setup: `stream_workspace` (`fast` clean, `slow` dirty). Watch on via
/// [`PtySession::open_with_env`]. A `WORKSPACE_STATUS_GIT` wrapper blocks
/// `git status` in `slow` after ARM. First paint, click `fast`, write a
/// unique untracked file, arm, then wait until slow is blocked. The
/// focused tree row and right pane must update while the wrapper still
/// holds. Fail if nothing happens, if only chrome/status ticks, if the
/// focused body stays blank, if `slow` is the pane, or if the update
/// waits for slow to unblock.
#[test]
fn pty_streamed_collect_updates_focused_repo_before_slow() {
    let (_root, workspace) = stream_workspace();
    let marker = format!(
        "streamed-e2e-{}.txt",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let block = SlowGitStatusBlock::install(&workspace);
    let mut tui = PtySession::open_with_env(
        &workspace,
        &[
            ("WS_STATUS_WATCH_MS", "3000"),
            (
                "WORKSPACE_STATUS_GIT",
                block.shim.to_str().expect("utf-8 shim path"),
            ),
        ],
    );
    tui.wait_pred(
        |screen| tree_has(screen, "fast") && tree_has(screen, "slow"),
        "first paint: fast and slow tree rows",
        WAIT,
    );
    let fast_row = tree_row_containing(&tui.screen(), "fast")
        .unwrap_or_else(|| panic!("fast tree row after first paint; screen:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, fast_row);
    tui.wait_pred(
        focused_fast_clean_pane,
        "click fast: tree cursor + Working tree clean pane (seed fast), no marker",
        GIT_WAIT,
    );

    fs::write(workspace.join("fast").join(&marker), STREAMED_MARKER_BODY).unwrap();
    block.arm();

    let start = Instant::now();
    while !block.wait.exists() {
        if start.elapsed() >= WAIT {
            panic!(
                "timeout waiting for slow git status to block; screen:\n{}",
                tui.screen()
            );
        }
        thread::sleep(Duration::from_millis(25));
    }
    assert!(
        block.still_blocked(),
        "slow repo must still be blocked when waiting for fast; screen:\n{}",
        tui.screen()
    );

    let marker_ref = marker.as_str();
    tui.wait_pred(
        |screen| block.still_blocked() && focused_fast_updated_before_slow(screen, marker_ref),
        "focused fast tree + pane update while slow git status is still blocked",
        Duration::from_secs(8),
    );
    assert!(
        block.still_blocked(),
        "fast tree/pane must update before slow git status is released; screen:\n{}",
        tui.screen()
    );
    block.release();
}

/// Tokyo Night `pills.filter.bg` (`#bb9af7`). Help `/` highlight uses this.
const HELP_SEARCH_FILTER_BG: (u8, u8, u8) = (0xbb, 0x9a, 0xf7);

/// Pane `/` typing chrome. Distinct from help `search focused pane (Enter arms)`.
fn pane_search_prompt(screen: &str) -> bool {
    screen.contains("SEARCH")
        && screen.contains("Enter arms query")
        && screen.contains("n/N after Enter")
}

fn help_overlay_open(screen: &str) -> bool {
    screen.contains("MOVE")
        && screen.contains("GIT")
        && screen.contains("VIEW")
        && screen.contains("stage scope")
        && screen.contains("search focused pane")
        && screen.contains("press twice")
        && screen.contains("never quit")
        && screen.contains("next / prev match")
}

fn help_searching(screen: &str, query: &str) -> bool {
    help_overlay_open(screen)
        && screen.contains(&format!("HELP  /{query}"))
        && screen.contains("Esc clears search")
        && !screen.contains("/ search help")
        && !pane_search_prompt(screen)
}

fn help_quit_rows_highlighted(tui: &PtySession) -> bool {
    let (r, g, b) = HELP_SEARCH_FILTER_BG;
    tui.needle_has_bg("press twice", r, g, b)
        && tui.needle_has_bg("never quit", r, g, b)
        && tui.needle_lacks_bg("stage scope", r, g, b)
}

fn help_quit_rows_unhighlighted(tui: &PtySession) -> bool {
    let (r, g, b) = HELP_SEARCH_FILTER_BG;
    tui.needle_lacks_bg("press twice", r, g, b)
        && tui.needle_lacks_bg("never quit", r, g, b)
        && tui.needle_lacks_bg("stage scope", r, g, b)
}

/// Help `/` highlights matching overlay rows. Enter must not arm pane search.
///
/// Docs: while `?` help is open, `/` is overlay-local (highlight only; rows
/// stay visible; no Enter-arm; no `n`/`N` next/prev). A no-op `/`, an Enter
/// that opens pane SEARCH, or a close that leaves `/{query}` armed cannot
/// pass. Glyphs-only screen delta is not enough: matching `quit` rows must
/// use the filter background, and non-matching rows must stay unhighlighted.
#[test]
fn pty_help_enter_does_not_arm_pane_search() {
    let (_root, workspace) = daily_workspace();
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_THEME", "tokyo-night")]);
    tui.wait_contains("README.md", WAIT);
    tui.wait_contains("+dirty", GIT_WAIT);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("focus right")
                && !pane_search_prompt(screen)
                && !screen.contains("MOVE")
        },
        "launch focuses README with no SEARCH prompt",
        GIT_WAIT,
    );

    tui.key('?');
    // After the overlay row-budget fix, help can cover the whole tree pane.
    // Assert overlay + Enter-arm here; README cursor only after help closes.
    tui.wait_pred(
        |screen| {
            help_overlay_open(screen)
                && screen.contains("/ search help")
                && !pane_search_prompt(screen)
                && !screen.contains("HELP  /")
        },
        "help overlay lists MOVE/GIT/VIEW and idle `/ search help`",
        WAIT,
    );

    tui.key('/');
    tui.wait_pred(
        |screen| {
            help_searching(screen, "")
                && screen.contains("HELP  /▏")
                && !screen.contains("HELP  /quit")
        },
        "help `/` opens overlay search (a no-op keeps `/ search help`; pane `/` paints SEARCH)",
        WAIT,
    );

    tui.keys("quit");
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_searching(&screen, "quit")
                && screen.contains("HELP  /quit▏")
                && !screen.contains("HELP  /quitn")
                && help_quit_rows_highlighted(&tui)
        },
        "typing quit highlights matching help rows; non-matching rows stay visible",
        WAIT,
    );

    tui.enter();
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_searching(&screen, "quit")
                && screen.contains("HELP  /quit▏")
                && !screen.contains("HELP  /quitn")
                && help_quit_rows_highlighted(&tui)
                && !screen.contains("[README")
                && !pane_search_prompt(&screen)
        },
        "Enter keeps help highlight only (pane SEARCH / n/N / drill cannot pass)",
        WAIT,
    );

    tui.key('n');
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_searching(&screen, "quitn")
                && screen.contains("HELP  /quitn▏")
                && help_quit_rows_unhighlighted(&tui)
                && !pane_search_prompt(&screen)
        },
        "n after Enter appends to help search (armed n/N would leave /quit and may move the cursor)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |_| {
            let screen = tui.screen();
            help_overlay_open(&screen)
                && screen.contains("/ search help")
                && !screen.contains("HELP  /")
                && !screen.contains("Esc clears search")
                && help_quit_rows_unhighlighted(&tui)
                && !pane_search_prompt(&screen)
        },
        "Esc clears help search; help stays (pane `/` would keep SEARCH or /quitn)",
        WAIT,
    );

    tui.esc();
    tui.wait_pred(
        |screen| {
            !screen.contains("MOVE")
                && !screen.contains("HELP  /")
                && !screen.contains("/quit")
                && !pane_search_prompt(screen)
                && tree_has(screen, "README.md")
                && tree_cursor_on(screen, "README.md")
                && screen.contains("+dirty")
                && screen.contains("? help")
                && screen.contains("focus right")
        },
        "second Esc closes help with pane search still unarmed",
        WAIT,
    );
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

/// Default mouse-on trackpad hscroll pans a clipped tree row.
///
/// Docs / keymap: write xterm SGR wheel right (`CSI < 67`) into the live
/// `event::read` loop. Motion-bit `CSI < 99` is dropped by crossterm 0.28
/// and must not pan. Shared oracle (`common::hscroll`): clipped `very-long`
/// prefix on the **tree row**, then `TAIL99` after pan, prefix gone. A
/// search chip that already contains `TAIL99` does not count. Do not `/`
/// search the tail first. Wait for a clipped tree row on the same frame.
/// Default mouse-on: this is not `pty_m_toggles_mouse_capture` and not
/// file-diff SGR pan. A no-op, a motion-bit-only pan, or a pan of only the
/// right pane / file-diff is red.
#[test]
fn pty_tree_sgr_hscroll_pans_clipped_path() {
    let (_root, workspace) = daily_workspace();
    seed_long_path_file(&workspace);
    let mut tui = PtySession::open(&workspace);
    tui.wait_contains("README.md", WAIT);
    tui.wait_pred(
        |screen| {
            screen.contains("README.md")
                && !screen.contains("Mouse off")
                && !screen.contains("Mouse on")
                && !screen.contains("SEARCH")
        },
        "launch paints the tree; mouse toast and SEARCH are absent",
        GIT_WAIT,
    );
    let _ = tui.wait_clipped_long_path_row(WAIT);

    // Short README diff so hscroll over the tree pans the tree, not a
    // long file-diff. Click is setup, not the click-to-select claim.
    let readme_hit = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row at launch:\n{}", tui.screen()));
    tui.sgr_click(TREE_LABEL_COL, readme_hit);
    tui.wait_pred(
        |screen| {
            tree_cursor_on(screen, "README.md")
                && screen.contains("UNSTAGED")
                && screen.contains("+dirty")
                && screen.contains("app/README.md")
                && !screen.contains("SEARCH")
                && !screen.contains("Mouse off")
                && !screen.contains("Mouse on")
        },
        "default mouse-on click loads a short README diff (not the long path)",
        GIT_WAIT,
    );
    let readme_row = tree_row_containing(&tui.screen(), "README.md")
        .unwrap_or_else(|| panic!("README row before hscroll:\n{}", tui.screen()));
    let row = tui.wait_clipped_long_path_row(WAIT);
    assert_tree_clipped_long_path(&tui.screen());

    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT_MOTION, 6, row);
    }
    tui.wait_ms(SETTLE_MS);
    assert!(
        tree_sgr_hscroll_clipped_readme_focus(&tui.screen(), readme_row),
        "motion-bit CSI < 99 must not pan (crossterm 0.28 drops it):\n{}",
        tui.screen()
    );

    for _ in 0..40 {
        tui.sgr_mouse(SGR_WHEEL_RIGHT, 6, row);
    }
    tui.wait_pred(
        |screen| documented_tree_sgr_hscroll_panned(screen, readme_row),
        "tree row shows TAIL99, drops very-long, keeps README cursor and short diff",
        WAIT,
    );
    crate::common::hscroll::assert_panned_to_tail(&left_tree(&tui.screen()));
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

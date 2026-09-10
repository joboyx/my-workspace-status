use std::fs;
use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::harness::PtySession;
use crate::seed::{compare_ahead_workspace, daily_workspace, git};
use crate::support::{
    crumb_row, graph_cursor_on, merger_graph_drilled_right, merger_graph_left_unfocused,
    panes_files_focused, panes_files_focused_diff_unfocused, panes_files_unfocused_diff_focused,
    status_row, title_has_diff, title_has_files, title_has_graph, tree_has, tree_line_containing,
    GIT_WAIT, WAIT,
};

/// Live poll. `watch_interval_ms` clamps any positive value to 500ms.
const WATCH_MS: &str = "500";

/// After a watch apply paint. The timer fired ~collect-ms earlier. The
/// next 500ms tick then falls in about (250, 480)ms. 160ms puts that
/// fire inside the 400ms `g` window.
const AFTER_APPLY_MS: u64 = 160;

/// Fail-closed wait after arming `g` for the next apply paint. Must stay
/// under 400ms (`DOUBLE_TAP_MS`). `WAIT` / `GIT_WAIT` here expires the
/// chord and can paint ToggleTreeMode for a correct `gt`.
const CHORD_APPLY_WAIT: Duration = Duration::from_millis(360);

/// Same gap as other compare palette humans. `keys("vs default")` can
/// drop the `l` in `default`.
const PALETTE_NAV_LETTER_GAP_MS: u64 = 50;

/// CSI-u press+release (`CSI code ; 1 : 1 u` / `: 3 u`).
fn csi_u_letter(tui: &mut PtySession, letter: char) {
    let codepoint = u32::from(letter.to_ascii_lowercase());
    tui.csi_u(codepoint, 1, 1);
    tui.csi_u(codepoint, 1, 3);
}

/// CSI-u Enter (`CSI 13 ; 1 : 1 u` press, `: 3` release).
fn csi_u_enter(tui: &mut PtySession) {
    tui.csi_u(13, 1, 1);
    tui.csi_u(13, 1, 3);
}

fn open_default_and_main(tui: &mut PtySession) {
    tui.search("app");
    tui.ctrl_letter('k');
    tui.keys("vs default");
    tui.wait_contains("Diff vs default", WAIT);
    tui.enter();
    tui.wait_contains("app · vs origin/main", GIT_WAIT);
    tui.ctrl_letter('k');
    tui.keys("vs branch");
    tui.enter();
    tui.wait_contains("Compare", WAIT);
    tui.keys("main");
    tui.enter();
    tui.wait_contains("app · vs main", GIT_WAIT);
}

fn on_workspace(screen: &str) -> bool {
    screen.contains("# workspace") && !screen.contains("COMMITTED") && !screen.contains("...HEAD")
}

fn on_compare_origin_main(screen: &str) -> bool {
    screen.contains("COMMITTED")
        && screen.contains("origin/main...HEAD")
        && !screen.contains("# workspace")
}

fn on_compare_local_main(screen: &str) -> bool {
    screen.contains("COMMITTED")
        && screen.contains("main...HEAD")
        && !screen.contains("origin/main...HEAD")
        && !screen.contains("# workspace")
}

fn on_compare_tab(screen: &str) -> bool {
    on_compare_origin_main(screen) || on_compare_local_main(screen)
}

/// ToggleTreeMode toast / pill, or CycleTheme toast.
fn tree_or_theme_fired(screen: &str) -> bool {
    let crumb = crumb_row(screen);
    let status = status_row(screen);
    crumb.contains("Flat paths")
        || status.contains(" flat")
        || crumb.contains("theme:")
        || screen.contains("theme: ")
}

/// `r` reload toast. Watch apply must not paint this.
fn refresh_now_toast(screen: &str) -> bool {
    screen.contains("refreshed app") || screen.contains("refreshed workspace")
}

/// New dirty path on the tree with the untracked `A` badge, not chrome-only.
fn tree_shows_watch_dirty_path(screen: &str, name: &str) -> bool {
    tree_has(screen, name)
        && tree_line_containing(screen, name).is_some_and(|line| line.contains("A "))
}

fn unique_dirty_name(tag: &str) -> String {
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Short enough to keep the name and `A` badge on the tree row.
    format!("{tag}{}.txt", n % 1_000_000)
}

fn write_repo_file(repo: &Path, name: &str) {
    fs::write(repo.join(name), format!("{name}\n")).unwrap();
}

fn write_app_dirty(workspace: &Path, name: &str) {
    write_repo_file(&workspace.join("app"), name);
}

fn commit_repo_file(repo: &Path, name: &str, message: &str) {
    git(repo, &["add", name]);
    git(repo, &["commit", "-q", "-m", message]);
}

/// Compare list shows `name` after a HEAD move. Dirty paths stay hidden.
fn compare_shows_committed_path(screen: &str, name: &str) -> bool {
    on_compare_tab(screen) && screen.contains(name) && !refresh_now_toast(screen)
}

/// Sync to a live watch apply on the Workspace tree (visible paint).
///
/// The paint is the apply. The timer fire was ~collect-ms earlier. Next
/// timer is ~500ms after that fire. `GPending` is not armed here.
fn phase_lock_watch(tui: &mut PtySession, workspace: &Path, tag: &str) {
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "phase-lock starts on the Workspace tree (not a compare tab)",
        WAIT,
    );
    let marker = unique_dirty_name(tag);
    write_app_dirty(workspace, &marker);
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && tree_shows_watch_dirty_path(screen, &marker)
                && !refresh_now_toast(screen)
        },
        "watch paints the new dirty path on the workspace tree (no r toast)",
        GIT_WAIT,
    );
}

/// Sync to a live watch apply on a compare tab (new committed path).
///
/// Compare paint is committed-only. A dirty path cannot prove apply.
fn phase_lock_compare_head(tui: &mut PtySession, repo: &Path, tag: &str) {
    tui.wait_pred(
        |screen| on_compare_tab(screen) && !tree_or_theme_fired(screen),
        "phase-lock starts on a compare tab",
        WAIT,
    );
    let marker = unique_dirty_name(tag);
    write_repo_file(repo, &marker);
    commit_repo_file(repo, &marker, "watch phase-lock");
    tui.wait_pred(
        |screen| compare_shows_committed_path(screen, &marker),
        "watch reloads the compare list after the new commit (no r toast)",
        GIT_WAIT,
    );
}

/// Arm `g`, write a unique dirty path, wait until the tree paints it,
/// then send the completing key. Timeout fails the test. A sleep with
/// no apply paint cannot pass.
fn claimed_g_after_tree_apply(tui: &mut PtySession, workspace: &Path, letter: char, tag: &str) {
    tui.wait_ms(AFTER_APPLY_MS);
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "claimed chord starts on the Workspace tree",
        WAIT,
    );
    let marker = unique_dirty_name(tag);
    csi_u_letter(tui, 'g');
    write_app_dirty(workspace, &marker);
    tui.wait_pred(
        |screen| {
            on_workspace(screen)
                && tree_shows_watch_dirty_path(screen, &marker)
                && !refresh_now_toast(screen)
        },
        "watch paints the mid-chord dirty path before the completing key (no apply must not pass)",
        CHORD_APPLY_WAIT,
    );
    csi_u_letter(tui, letter);
}

/// Arm `g`, commit a unique path, wait until the compare list paints it,
/// then send `1`. Timeout is red. A stay on the compare tab is a no-op.
fn claimed_g1_after_compare_head_apply(tui: &mut PtySession, repo: &Path, tag: &str) {
    tui.wait_ms(AFTER_APPLY_MS);
    tui.wait_pred(
        |screen| on_compare_tab(screen) && !tree_or_theme_fired(screen),
        "claimed g1 starts on a compare tab (Workspace g1 is a stay)",
        WAIT,
    );
    let marker = unique_dirty_name(tag);
    let before = tui.screen();
    assert!(
        !before.contains(&marker),
        "mid-chord commit path must be absent before g:\n{before}"
    );
    csi_u_letter(tui, 'g');
    write_repo_file(repo, &marker);
    commit_repo_file(repo, &marker, "watch g1");
    tui.wait_pred(
        |screen| compare_shows_committed_path(screen, &marker),
        "watch reloads the compare list after the mid-chord commit (no apply must not pass)",
        CHORD_APPLY_WAIT,
    );
    csi_u_letter(tui, '1');
}

fn palette_open(screen: &str) -> bool {
    screen.contains("Enter run")
}

fn type_palette_filter(tui: &mut PtySession, query: &str) {
    for c in query.chars() {
        tui.key(c);
        if matches!(c, 'h' | 'H' | 'j' | 'J' | 'k' | 'K' | 'l' | 'L') {
            tui.wait_ms(PALETTE_NAV_LETTER_GAP_MS);
        }
    }
}

fn open_vs_default(tui: &mut PtySession) {
    tui.ctrl_letter('k');
    tui.wait_pred(
        |screen| palette_open(screen) && screen.contains("Ctrl-k"),
        "Ctrl-k opens the command palette (a no-op leaves idle chrome without Enter run)",
        WAIT,
    );
    type_palette_filter(tui, "vs default");
    tui.wait_pred(
        |screen| {
            palette_open(screen)
                && screen.contains("Diff vs default")
                && screen.contains("vs default")
        },
        "palette filter `vs default` shows `Diff vs default` (Enter before the filter lands would run the first catalog row; a dropped nav letter would keep a truncated query)",
        WAIT,
    );
    tui.enter();
}

fn merge_commit_selected(screen: &str) -> bool {
    graph_cursor_on(screen, "merge")
        && !graph_cursor_on(screen, "WIP on graph")
        && !graph_cursor_on(screen, "working tree")
}

fn names_a_commit_path(screen: &str) -> bool {
    screen.contains("left.txt") || screen.contains("right.txt") || screen.contains("README.md")
}

fn merge_commit_files_right(screen: &str) -> bool {
    panes_files_focused(screen)
        && title_has_files(screen)
        && title_has_graph(screen)
        && !title_has_diff(screen)
        && names_a_commit_path(screen)
        && !screen.contains("wip.txt")
}

/// Compare opened from a Files drill: files left, DiffPane right, no graph.
fn compare_from_files_drill(screen: &str) -> bool {
    let files_plus_diff =
        panes_files_focused_diff_unfocused(screen) || panes_files_unfocused_diff_focused(screen);
    files_plus_diff
        && title_has_diff(screen)
        && title_has_files(screen)
        && !title_has_graph(screen)
        && !panes_files_focused(screen)
        && (screen.contains("COMMITTED") || screen.contains("No committed changes"))
        && screen.contains("merger · vs")
}

fn parked_files_drill_returned(screen: &str) -> bool {
    panes_files_focused(screen) || (title_has_graph(screen) && title_has_files(screen))
}

/// `gt` / `g1` / `g2` stay tab actions when a live watch apply lands
/// between the chord halves.
///
/// `gt` and `g2` start on Workspace. The mid-chord oracle is a new dirty
/// path with the tree `A` badge. A stay on Workspace is a failed no-op.
/// `g1` from Workspace is Action::None, so that chord starts on a
/// compare tab. The mid-chord oracle is a new committed path (compare
/// paint hides dirty files). Watch ticks do not clear the armed `g`.
/// A `GIT_WAIT` between `g` and the completing key expires the chord.
/// Sleep without the apply paint cannot pass.
#[test]
fn pty_compare_gt_survives_watch() {
    let (_root, workspace) = compare_ahead_workspace();
    let app = workspace.join("app");
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", WATCH_MS)]);
    tui.wait_contains("app", WAIT);
    open_default_and_main(&mut tui);
    tui.wait_pred(
        on_compare_local_main,
        "setup: active compare is vs local main (not only a strip label)",
        GIT_WAIT,
    );

    csi_u_letter(&mut tui, 'g');
    csi_u_letter(&mut tui, 't');
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "setup: gt wraps to Workspace so the watch apply can paint the tree",
        WAIT,
    );

    phase_lock_watch(&mut tui, &workspace, "lgt");
    claimed_g_after_tree_apply(&mut tui, &workspace, 't', "gt");
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "gt after a live watch apply is NextTab (vs origin/main), not ToggleTreeMode or a Workspace stay",
        WAIT,
    );

    phase_lock_compare_head(&mut tui, &app, "lg1");
    claimed_g1_after_compare_head_apply(&mut tui, &app, "g1");
    tui.wait_pred(
        |screen| on_workspace(screen) && !tree_or_theme_fired(screen),
        "g1 after a live watch apply jumps to Workspace (a stay on the compare tab is a no-op)",
        WAIT,
    );

    phase_lock_watch(&mut tui, &workspace, "lg2");
    claimed_g_after_tree_apply(&mut tui, &workspace, '2', "g2");
    tui.wait_pred(
        |screen| on_compare_origin_main(screen) && !tree_or_theme_fired(screen),
        "g2 after a live watch apply activates vs origin/main (a Workspace stay is a no-op)",
        WAIT,
    );
}

/// Diff vs default from a graph Files drill paints DiffPane. Watch keeps it.
///
/// Parked commit-files must not return on the right. Compare paint is
/// committed-only. The watch apply oracle is a new committed path on
/// the compare list. Sleep without that paint cannot pass.
#[test]
fn pty_compare_from_commit_files_drill_survives_watch() {
    let (_root, workspace) = daily_workspace();
    let merger = workspace.join("merger");
    let mut tui = PtySession::open_with_env(&workspace, &[("WS_STATUS_WATCH_MS", WATCH_MS)]);
    tui.wait_contains("README.md", WAIT);

    tui.search("merger");
    tui.wait_pred(
        merger_graph_left_unfocused,
        "search lands on merger and loads its graph (left focus, not yet Enter)",
        GIT_WAIT,
    );

    csi_u_enter(&mut tui);
    tui.wait_pred(
        merger_graph_drilled_right,
        "CSI-u Enter on merger focuses that graph (file-diff / files drill / no-op cannot pass)",
        WAIT,
    );

    tui.key('j');
    tui.wait_pred(
        |screen| {
            graph_cursor_on(screen, "WIP on graph") && !graph_cursor_on(screen, "working tree")
        },
        "first j selects stash@{0} (skip this row)",
        WAIT,
    );
    tui.key('j');
    tui.wait_pred(
        merge_commit_selected,
        "second j selects the merge commit (not stash / uncommitted)",
        WAIT,
    );

    csi_u_enter(&mut tui);
    tui.wait_pred(
        merge_commit_files_right,
        "CSI-u Enter on the merge commit opens its file list (graph left, files right, right focused)",
        GIT_WAIT,
    );

    open_vs_default(&mut tui);
    tui.wait_pred(
        compare_from_files_drill,
        "Diff vs default from the Files drill paints files left and DiffPane right (not graph+files)",
        GIT_WAIT,
    );

    let marker = unique_dirty_name("fd");
    let before = tui.screen();
    assert!(
        !before.contains(&marker),
        "watch commit path must be absent before the disk commit:\n{before}"
    );
    write_repo_file(&merger, &marker);
    commit_repo_file(&merger, &marker, "watch files-drill");
    tui.wait_pred(
        |screen| {
            compare_from_files_drill(screen)
                && screen.contains(&marker)
                && !refresh_now_toast(screen)
                && !parked_files_drill_returned(screen)
        },
        "watch reloads the compare list with the new committed path and keeps DiffPane (no apply must not pass)",
        GIT_WAIT,
    );
}

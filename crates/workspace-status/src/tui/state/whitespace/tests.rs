use std::path::PathBuf;

use super::super::super::action::Action;
use super::super::super::app::CompareRangeLoad;
use super::super::super::diff::DiffContent;
use super::super::super::drill::{CommitFile, CommitFileSource};
use super::super::super::status::StatusKind;
use super::super::super::ui_state_store::load_ui_state;
use super::super::FocusPane;
use super::*;
use crate::snapshot::RepoSnapshot;
use crate::testutil::{app_state, repo_snapshot, unique_dir, unstaged_change};
use crate::tui::tree::NodeKind;

/// One changed line, as a Workspace, drill, or Explorer diff.
const BODY: &str = "@@ -1,3 +1,3 @@\n a\n-b\n+c\n d\n";

/// `app` on a born HEAD with a default tip (a compare tab opens), with
/// an unstaged edit of `README.md`, and the UI-state store in a fresh
/// temp dir. `app/README.md` exists on disk under `cwd`, so the Explorer
/// live-reload stamp reads a real file.
fn state() -> (AppState, PathBuf) {
    let dir = unique_dir("ws-whitespace-toggle");
    std::fs::create_dir_all(dir.join("app")).unwrap();
    std::fs::write(dir.join("app/README.md"), "a\nc\nd\n").unwrap();
    let mut app = app_state(&[RepoSnapshot {
        head: "abc".into(),
        default_tip_ref: Some("main".into()),
        has_unstaged: true,
        changes: vec![unstaged_change("README.md")],
        ..repo_snapshot("app")
    }]);
    app.cwd = dir.clone();
    app.ui_store_path = dir.join("state").join("ui-state.json");
    (app, dir)
}

fn readme(dash_w: bool) -> DiffContent {
    let mut content = DiffContent::from_unified(BODY);
    content.ignore_whitespace = dash_w;
    content
}

/// Depth 0: the README.md row focused and its diff shown.
fn show_workspace_diff(app: &mut AppState, dash_w: bool) {
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File && row.label.contains("README.md"))
        .expect("file row");
    app.set_diff("app".into(), "README.md".into(), readme(dash_w));
    assert!(app.right_is_diff());
}

fn compare_source() -> CommitFileSource {
    CommitFileSource::Compare {
        base_ref: "main".into(),
        head_ref: "HEAD".into(),
        base_tip: "bbb".into(),
        merge_base: "aaa".into(),
        head: "ccc".into(),
    }
}

fn readme_file() -> CommitFile {
    CommitFile {
        status: "M".into(),
        path: "README.md".into(),
        old_path: None,
        stat: None,
    }
}

/// Open a compare tab with README.md loaded (`dash_w` marks the load).
/// Returns the tab id.
fn open_compare_diff(app: &mut AppState, dash_w: bool) -> u64 {
    app.tabs
        .open_or_focus("app".into(), "main".into(), "HEAD".into());
    let tab = app.tabs.active_compare().unwrap();
    let (tab_id, gen) = (tab.id, tab.generation);
    let load = CompareRangeLoad {
        source: compare_source(),
        files: vec![readme_file()],
        patch_keys: [("README.md".to_string(), "key:README.md".to_string())].into(),
        head: Some("ccc".into()),
        base_tip: "bbb".into(),
        commit: None,
    };
    let _ = app.apply_compare_range(tab_id, gen, Ok(load));
    app.apply_compare_diff(
        tab_id,
        gen,
        &compare_source(),
        "README.md",
        Ok(readme(dash_w)),
    );
    app.focus = FocusPane::Right;
    tab_id
}

fn assert_compare_reload(effect: Effect, want_tab: u64) {
    match effect {
        Effect::LoadCompareDiff { tab_id, path, .. } => {
            assert_eq!(tab_id, want_tab);
            assert_eq!(path, "README.md");
        }
        other => panic!("{other:?}"),
    }
}

fn assert_explorer_reload(effect: Effect) {
    match effect {
        Effect::LoadExplorerPreview { path, change, .. } => {
            assert_eq!(path, "README.md");
            assert!(change.is_some(), "a diff load");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn w_flips_the_setting_and_saves_it_at_once() {
    let (mut app, dir) = state();
    assert!(!app.ignore_whitespace);
    // Nothing shown (the repo row): only the flag flips and saves.
    app.cursor = 0;
    assert!(!app.right_is_diff());
    assert_eq!(app.dispatch(Action::ToggleIgnoreWhitespace), Effect::None);
    assert!(app.ignore_whitespace);
    assert_eq!(app.status, WHITESPACE_HIDDEN);
    assert_eq!(app.status.kind(), StatusKind::Info);
    assert_eq!(
        load_ui_state(&app.ui_store_path).ignore_whitespace,
        Some(true)
    );
    assert!(std::fs::read_to_string(&app.ui_store_path)
        .unwrap()
        .contains("\"ignoreWhitespace\": true"));
    assert_eq!(app.pending_hunk_anchor, None, "no diff, no anchor");

    assert_eq!(app.dispatch(Action::ToggleIgnoreWhitespace), Effect::None);
    assert!(!app.ignore_whitespace);
    assert_eq!(app.status, WHITESPACE_SHOWN);
    assert_eq!(
        load_ui_state(&app.ui_store_path).ignore_whitespace,
        Some(false)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// `:` + `whitespace` + Enter runs the same toggle as `w`.
#[test]
fn the_palette_row_flips_the_setting_like_w() {
    use super::super::super::action::QuickOpenEntry;
    let (mut app, dir) = state();
    for want in [true, false] {
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in "whitespace".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let selected = app
            .command_palette()
            .and_then(|palette| palette.selected())
            .map(|command| command.title);
        assert_eq!(selected, Some("Ignore / show whitespace"));
        app.dispatch(Action::QuickOpenSubmit);
        assert_eq!(app.ignore_whitespace, want);
        assert_eq!(
            load_ui_state(&app.ui_store_path).ignore_whitespace,
            Some(want)
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// Open Quick Open in commands mode, type `query`, and return the title
/// the cursor lands on.
fn palette_landing(app: &mut AppState, query: &str) -> Option<String> {
    use super::super::super::action::QuickOpenEntry;
    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
    for c in query.chars() {
        app.dispatch(Action::QuickOpenChar(c));
    }
    app.command_palette()
        .and_then(|palette| palette.selected())
        .map(|command| command.title.to_string())
}

/// On the PRs tab the whitespace row is disabled, and Close tab (its
/// `Ctrl-w` chip matches `-w`) is not: `:-w` still lands on the exact
/// alias, so Enter says why and closes nothing.
#[test]
fn palette_dash_w_on_the_prs_tab_refuses_and_never_closes_the_tab() {
    let (mut app, dir) = state();
    app.dispatch(Action::OpenPrsTab);
    assert!(app.is_prs_tab());
    let tabs = app.tabs.len();
    assert_eq!(
        palette_landing(&mut app, "-w").as_deref(),
        Some("Ignore / show whitespace")
    );
    assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
    assert_eq!(app.tabs.len(), tabs, "Close tab did not run");
    assert!(app.is_prs_tab());
    assert!(!app.ignore_whitespace);
    assert_eq!(app.status.kind(), StatusKind::Warn, "{}", app.status);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Inside a highlight both `-w` rows are out of scope: Enter refuses and
/// the compare tab stays open.
#[test]
fn palette_dash_w_in_a_highlight_never_closes_the_tab() {
    let (mut app, dir) = state();
    open_compare_diff(&mut app, false);
    app.dispatch(Action::DiffVisualStart);
    assert!(app.diff_visual_anchor.is_some());
    let tabs = app.tabs.len();
    palette_landing(&mut app, "-w");
    app.dispatch(Action::QuickOpenSubmit);
    assert_eq!(app.tabs.len(), tabs, "Close tab did not run");
    assert!(app.is_compare_tab());
    assert!(!app.ignore_whitespace);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_failed_save_keeps_the_toggle_and_says_so() {
    let (mut app, dir) = state();
    let blocker = dir.join("not-a-dir");
    std::fs::write(&blocker, "x").unwrap();
    app.ui_store_path = blocker.join("ui-state.json");
    app.dispatch(Action::ToggleIgnoreWhitespace);
    assert!(app.ignore_whitespace, "the session still flips");
    assert_eq!(app.status.kind(), StatusKind::Error);
    assert!(
        app.status.starts_with("whitespace save failed"),
        "{}",
        app.status
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn w_on_the_workspace_diff_reloads_the_right_pane_near_the_cursor() {
    let (mut app, dir) = state();
    show_workspace_diff(&mut app, false);
    app.focus = FocusPane::Right;
    let change = app
        .current_diff_rows()
        .iter()
        .position(|row| row_text(row) == "c")
        .expect("change row");
    app.diff_cursor = change;
    app.dispatch(Action::DiffVisualStart);
    assert!(app.diff_visual_anchor.is_some());
    assert_eq!(
        app.dispatch(Action::ToggleIgnoreWhitespace),
        Effect::LoadRightPane
    );
    assert!(app.diff_visual_anchor.is_none(), "the range is dropped");
    assert!(app.pending_hunk_anchor.is_some(), "the cursor text is kept");
    // The reload lands with `-w`: the cursor stays on its text.
    app.set_diff("app".into(), "README.md".into(), readme(true));
    assert_eq!(row_text(&app.current_diff_rows()[app.diff_cursor]), "c");
    assert_eq!(app.pending_hunk_anchor, None, "anchor consumed");
    let _ = std::fs::remove_dir_all(&dir);
}

fn row_text(row: &super::super::super::diff::DiffRow) -> String {
    super::super::super::diff::row_search_text(row)
}

#[test]
fn w_on_a_drill_diff_reloads_the_commit_diff() {
    let (mut app, dir) = state();
    app.focus = FocusPane::Right;
    let source = CommitFileSource::Commit {
        commit_id: "abc".into(),
    };
    app.open_commit_diff(
        "app".into(),
        source.clone(),
        vec![readme_file()],
        0,
        "README.md".into(),
        readme(false),
    );
    assert_eq!(
        app.dispatch(Action::ToggleIgnoreWhitespace),
        Effect::LoadCommitDiff {
            repo: "app".into(),
            source,
            path: "README.md".into(),
        }
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn w_on_a_compare_tab_reloads_its_open_file() {
    let (mut app, dir) = state();
    let tab_id = open_compare_diff(&mut app, false);
    assert_compare_reload(app.dispatch(Action::ToggleIgnoreWhitespace), tab_id);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn w_on_an_explorer_diff_requeues_its_preview_and_a_clean_file_stays() {
    let (mut app, dir) = state();
    app.open_loaded_inner_diff("app", "README.md", readme(false));
    assert!(app.is_explorer_tab());
    assert_explorer_reload(app.dispatch(Action::ToggleIgnoreWhitespace));
    assert!(app.ignore_whitespace);
    assert_eq!(app.status, WHITESPACE_HIDDEN, "the reload is silent");
    assert_eq!(
        app.pending_hunk_anchor, None,
        "Explorer keeps its own cursor"
    );

    // A clean file view does not depend on the setting: no reload.
    std::fs::write(dir.join("app/notes.txt"), "x\n").unwrap();
    app.open_loaded_inner_file("app", "notes.txt", &["x"]);
    assert_eq!(app.dispatch(Action::ToggleIgnoreWhitespace), Effect::None);
    assert!(!app.ignore_whitespace);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A tab parked while `w` flipped holds content loaded with the other
/// value; it reloads when it becomes active, and a tab whose content
/// already matches does not.
#[test]
fn a_parked_tab_reloads_its_diff_when_it_becomes_active() {
    let (mut app, dir) = state();
    show_workspace_diff(&mut app, false);
    let tab_id = open_compare_diff(&mut app, false);

    // `w` on the compare tab; the Workspace diff is parked without `-w`.
    assert_compare_reload(app.dispatch(Action::ToggleIgnoreWhitespace), tab_id);
    assert_eq!(app.activate_tab(0), Effect::LoadRightPane);
    // Its `-w` load lands; the compare tab still holds the old content.
    app.set_diff("app".into(), "README.md".into(), readme(true));
    assert_compare_reload(app.activate_tab(1), tab_id);

    // Content that matches the setting does not reload.
    let gen = app.tabs.active_compare().unwrap().generation;
    app.apply_compare_diff(
        tab_id,
        gen,
        &compare_source(),
        "README.md",
        Ok(readme(true)),
    );
    assert_eq!(app.activate_tab(0), Effect::None);
    assert_eq!(app.activate_tab(1), Effect::None);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_parked_explorer_diff_reloads_when_its_tab_becomes_active() {
    let (mut app, dir) = state();
    app.open_loaded_inner_diff("app", "README.md", readme(false));
    let explorer = app.tabs.active;
    assert_eq!(app.activate_tab(0), Effect::None);
    // Nothing shown on the Workspace tab: only the flag flips.
    app.cursor = 0;
    assert_eq!(app.dispatch(Action::ToggleIgnoreWhitespace), Effect::None);
    assert_explorer_reload(app.activate_tab(explorer));
    let _ = std::fs::remove_dir_all(&dir);
}

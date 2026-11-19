use std::path::PathBuf;

use super::*;
use crate::settings::Settings;
use crate::snapshot::build_workspace_snapshot;
use crate::testutil::{app_state, focus_workspace_repos, unique_dir};
use crate::tui::ops::FocusScope;
use crate::tui::status::StatusKind;

/// App state over the focus demo workspace.
fn state() -> AppState {
    app_state(&focus_workspace_repos())
}

/// Put the cursor on row `id`.
fn put_cursor(app: &mut AppState, id: &str) {
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.id == id)
        .unwrap_or_else(|| panic!("missing row {id}"));
}

fn cursor_id(app: &AppState) -> &str {
    app.focused_row().map_or("", |row| row.id.as_str())
}

fn row_ids(app: &AppState) -> Vec<&str> {
    app.rows.iter().map(|row| row.id.as_str()).collect()
}

/// The query saved for the test state's workspace root.
fn saved(app: &AppState) -> Option<String> {
    load_tree_focus(&app.ui_store_path, &workspace_store_id(&app.cwd))
}

#[test]
fn set_focus_narrows_the_tree_saves_and_folds_the_outside_group() {
    let mut app = state();
    put_cursor(&mut app, "repo:services/api");
    let effect = app.set_focus_query("ABC-123");
    assert!(matches!(effect, Effect::None), "same row: no reload");
    assert_eq!(app.focus_query().map(TreeFocus::query), Some("ABC-123"));
    assert_eq!(
        row_ids(&app),
        [
            "workspace",
            "repo:app",
            "checkout:app/.worktrees/abc-123",
            "repo:services/api",
            OUTSIDE_FOCUS_GROUP_ID,
        ],
        "the outside group starts folded"
    );
    assert_eq!(cursor_id(&app), "repo:services/api", "the cursor row stays");
    assert_eq!(app.status, "focus: ABC-123");
    assert_eq!(saved(&app).as_deref(), Some("ABC-123"));
    let chip = app.tree.chrome.focus_chip.clone().expect("chip");
    assert_eq!((chip.focused, chip.total), (2, 4));
}

#[test]
fn cursor_moves_to_the_nearest_visible_ancestor() {
    let mut app = state();
    app.folds.remove("group:no-updates");
    app.rebuild_rows();
    put_cursor(&mut app, "repo:lib");
    let effect = app.set_focus_query("ABC-123");
    assert_eq!(cursor_id(&app), OUTSIDE_FOCUS_GROUP_ID, "folded group");
    assert!(matches!(effect, Effect::LoadRightPane));

    let mut app = state();
    put_cursor(&mut app, "checkout:app/.worktrees/login");
    app.set_focus_query("ABC-123");
    assert_eq!(
        cursor_id(&app),
        "repo:app",
        "a checkout the focus leaves out lands on its family row"
    );
}

#[test]
fn clear_focus_restores_the_tree_and_drops_the_saved_query() {
    let mut app = state();
    let plain = row_ids(&app).join(" ");
    app.set_focus_query("ABC-123");
    put_cursor(&mut app, "repo:services/api");
    app.clear_focus_query();
    assert_eq!(app.focus_query(), None);
    assert_eq!(row_ids(&app).join(" "), plain);
    assert!(!app.folds.contains(OUTSIDE_FOCUS_GROUP_ID));
    assert_eq!(cursor_id(&app), "repo:services/api");
    assert_eq!(app.status, "focus cleared");
    assert_eq!(saved(&app), None);
    assert_eq!(app.tree.chrome.focus_chip, None);
}

#[test]
fn a_query_with_no_terms_clears() {
    let mut app = state();
    app.set_focus_query("lib");
    app.set_focus_query(" , ");
    assert_eq!(app.focus_query(), None);
    assert_eq!(saved(&app), None);
}

#[test]
fn an_open_outside_group_stays_open_while_the_focus_changes() {
    let mut app = state();
    app.set_focus_query("ABC-123");
    app.folds.remove(OUTSIDE_FOCUS_GROUP_ID);
    app.set_focus_query("abc-123, notes");
    assert!(row_ids(&app).contains(&"repo:lib"), "still open");
    app.clear_focus_query();
    app.set_focus_query("lib");
    assert!(
        app.folds.contains(OUTSIDE_FOCUS_GROUP_ID),
        "a new outside group starts folded"
    );
}

#[test]
fn preview_shows_the_draft_without_a_save_and_cancel_restores() {
    let mut app = state();
    put_cursor(&mut app, "repo:services/api");
    let before = row_ids(&app).join(" ");
    app.preview_focus_query("lib");
    assert_eq!(app.focus_query(), None, "the applied focus stays");
    assert_eq!(app.shown_focus_query().map(TreeFocus::query), Some("lib"));
    assert_eq!(saved(&app), None, "a preview never saves");
    assert_eq!(cursor_id(&app), OUTSIDE_FOCUS_GROUP_ID);
    assert_eq!(
        app.tree
            .chrome
            .focus_chip
            .as_ref()
            .map(|c| c.query.as_str()),
        Some("lib")
    );
    // Each keystroke rebuilds from the row the preview started on.
    app.preview_focus_query("li");
    app.preview_focus_query("api");
    assert_eq!(cursor_id(&app), "repo:services/api");
    app.preview_focus_query("");
    assert_eq!(app.shown_focus_query(), None, "an empty draft shows all");
    assert_eq!(app.tree.chrome.focus_chip, None);

    app.preview_focus_query("lib");
    app.cancel_focus_preview();
    assert_eq!(app.shown_focus_query(), None);
    assert_eq!(row_ids(&app).join(" "), before);
    assert_eq!(cursor_id(&app), "repo:services/api");
}

#[test]
fn cancel_restores_the_focus_applied_before_the_preview() {
    let mut app = state();
    app.set_focus_query("ABC-123");
    let focused = row_ids(&app).join(" ");
    app.preview_focus_query("lib");
    assert_ne!(row_ids(&app).join(" "), focused);
    app.cancel_focus_preview();
    assert_eq!(row_ids(&app).join(" "), focused);
    assert_eq!(
        app.shown_focus_query().map(TreeFocus::query),
        Some("ABC-123")
    );
    assert!(
        matches!(app.cancel_focus_preview(), Effect::None),
        "no preview"
    );
}

#[test]
fn applying_a_preview_saves_the_draft_and_ends_the_preview() {
    let mut app = state();
    put_cursor(&mut app, "repo:services/api");
    app.preview_focus_query("lib");
    app.set_focus_query("lib");
    assert_eq!(app.focus_query().map(TreeFocus::query), Some("lib"));
    assert!(app.focus_preview.is_none());
    assert_eq!(saved(&app).as_deref(), Some("lib"));
    assert_eq!(
        cursor_id(&app),
        OUTSIDE_FOCUS_GROUP_ID,
        "the preview's start row"
    );
}

#[test]
fn a_failed_save_keeps_the_focus_and_shows_the_error() {
    let mut app = state();
    let dir = unique_dir("ws-focus-save");
    std::fs::create_dir_all(&dir).unwrap();
    // A directory where the store file should be: the save fails.
    app.ui_store_path = dir.clone();
    app.set_focus_query("ABC-123");
    assert_eq!(app.focus_query().map(TreeFocus::query), Some("ABC-123"));
    assert_eq!(app.status.kind(), StatusKind::Error);
    assert!(
        app.status.starts_with("focus save failed: "),
        "{}",
        &*app.status
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn focus_split_reads_the_applied_focus() {
    let mut app = state();
    assert_eq!(app.focus_split(), None);
    app.set_focus_query("ABC-123");
    let split = app.focus_split().expect("split");
    assert_eq!(split.counts.focused, 2);
    assert_eq!(split.outside, ["lib", "notes"]);
    app.preview_focus_query("lib");
    assert_eq!(
        app.focus_split().expect("split").counts.focused,
        2,
        "a preview does not change what the actions cover"
    );
}

/// Launch over a UI-state file that saved `query` for `/tmp`.
fn launch_with_saved(prefix: &str, query: Option<&str>) -> (AppState, PathBuf) {
    let dir = unique_dir(prefix);
    let ui_store = dir.join("ui-state.json");
    let cwd = PathBuf::from("/tmp");
    if let Some(query) = query {
        save_tree_focus(&ui_store, &workspace_store_id(&cwd), Some(query)).unwrap();
    }
    let (viewed, comments, ui) = (
        dir.join("viewed.json").display().to_string(),
        dir.join("comments.json").display().to_string(),
        ui_store.display().to_string(),
    );
    let settings = Settings::resolve(&Default::default(), |key| match key {
        "WS_STATUS_VIEWED_STORE" => Some(viewed.clone()),
        "WS_STATUS_COMMENT_STORE" => Some(comments.clone()),
        "WS_STATUS_UI_STORE" => Some(ui.clone()),
        _ => None,
    });
    let snapshot = build_workspace_snapshot(&focus_workspace_repos(), &[], false, &[]);
    (AppState::launch(cwd, snapshot, &settings), dir)
}

#[test]
fn launch_restores_the_saved_focus_with_one_notice() {
    let (mut app, dir) = launch_with_saved("ws-focus-launch", Some("ABC-123"));
    assert_eq!(app.focus_query().map(TreeFocus::query), Some("ABC-123"));
    assert_eq!(app.status, "focus restored: ABC-123");
    assert_eq!(app.status.kind(), StatusKind::Info);
    assert!(app.folds.contains(OUTSIDE_FOCUS_GROUP_ID));
    assert_eq!(
        app.tree.children.last().map(|c| c.id.as_str()),
        Some(OUTSIDE_FOCUS_GROUP_ID)
    );
    // The notice is a one-time info line: a rebuild does not repeat it.
    app.status.clear();
    app.rebuild_rows();
    assert_eq!(app.status, "");
    assert_eq!(app.focus_query().map(TreeFocus::query), Some("ABC-123"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn launch_with_no_saved_focus_says_nothing() {
    let (app, dir) = launch_with_saved("ws-focus-launch-none", None);
    assert_eq!(app.focus_query(), None);
    assert!(!app.status.contains("focus"), "{}", &*app.status);
    assert!(!app.folds.contains(OUTSIDE_FOCUS_GROUP_ID));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_watch_snapshot_keeps_the_focus() {
    let mut app = state();
    app.set_focus_query("ABC-123");
    let snapshot = build_workspace_snapshot(&focus_workspace_repos(), &[], false, &[]);
    app.apply_watch_snapshot(snapshot);
    assert_eq!(
        app.tree.children.last().map(|c| c.id.as_str()),
        Some(OUTSIDE_FOCUS_GROUP_ID)
    );
    assert!(app.folds.contains(OUTSIDE_FOCUS_GROUP_ID));
}

/// Paint `app` on a `cols` × 30 frame; the buffer.
fn paint(app: &mut AppState, cols: u16) -> ratatui::buffer::Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, 30)).unwrap();
    terminal
        .draw(|frame| crate::tui::render::draw(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// Text of the cells of `rect` (one row).
fn cells(buf: &ratatui::buffer::Buffer, rect: ratatui::layout::Rect) -> String {
    (rect.x..rect.right())
        .map(|x| buf[(x, rect.y)].symbol().to_string())
        .collect()
}

/// Row `y` of `buf` as text, and the first column of `needle` in it.
fn find(buf: &ratatui::buffer::Buffer, needle: &str) -> (u16, u16) {
    let area = buf.area;
    for y in 0..area.height {
        let line: Vec<String> = (0..area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        let text: String = line.concat();
        if let Some(at) = text.find(needle) {
            // Byte offset to column: every cell before it is one column.
            let col = text[..at].chars().count();
            return (y, u16::try_from(col).unwrap());
        }
    }
    panic!("no {needle:?} on screen");
}

#[test]
fn paint_records_the_chip_cells_and_highlights_matches() {
    let mut app = state();
    app.set_focus_query("ABC-123");
    let buf = paint(&mut app, 160);
    let hit = app.layout.focus_chip_hit.expect("the chip painted");
    assert_eq!(cells(&buf, hit), "(o) ABC-123 · 2/4 repos");
    assert_eq!(hit.y, find(&buf, "(o) ABC-123").0, "on the workspace row");

    let pill = app.theme.pills().filter;
    let (y, x) = find(&buf, "feature/ABC-123-invoice-export");
    let matched = &buf[(x + 8, y)];
    assert_eq!(matched.symbol(), "A");
    assert_eq!((matched.fg, matched.bg), (pill.fg, pill.bg), "match pill");
    assert!(matched.modifier.contains(ratatui::style::Modifier::BOLD));
    assert_ne!(buf[(x, y)].bg, pill.bg, "the rest of the branch is plain");

    let palette = app.theme.palette();
    let (y, x) = find(&buf, "2 outside focus · 1 dirty");
    assert_eq!(buf[(x, y)].fg, palette.muted, "the outside row is dimmed");

    app.clear_focus_query();
    paint(&mut app, 160);
    assert_eq!(app.layout.focus_chip_hit, None, "no focus, no chip");
}

/// At every frame width the summary shows only beside the whole chip, and
/// some width shows the chip with the summary gone.
#[test]
fn the_chip_outlasts_the_summary_on_a_narrow_tree() {
    let mut app = state();
    app.set_focus_query("ABC-123");
    let mut chip_alone = false;
    for cols in 46..=200 {
        let buf = paint(&mut app, cols);
        // The workspace row: ASCII workspace glyph `#`, then the cwd name.
        let (y, _) = find(&buf, "# tmp");
        let row: String = (0..buf.area.width)
            .map(|x| buf[(x, y)].symbol().to_string())
            .collect();
        let shows_summary = row.contains("changed");
        match app.layout.focus_chip_hit {
            Some(hit) => {
                assert_eq!(cells(&buf, hit), "(o) ABC-123 · 2/4 repos", "{cols}");
                chip_alone |= !shows_summary;
            }
            None => assert!(!shows_summary, "{cols}: summary without chip: {row:?}"),
        }
    }
    assert!(
        chip_alone,
        "some width keeps the chip and drops the summary"
    );
}

/// The focus demo workspace with every checkout behind its upstream and
/// off its default branch, so pull and the default switch have targets.
fn behind_state() -> AppState {
    use crate::snapshot::SyncStatus;
    let mut repos = focus_workspace_repos();
    for repo in &mut repos {
        repo.sync_status = SyncStatus::Behind;
        if repo.branch == "main" {
            repo.branch = format!("topic/{}", repo.repo.replace('/', "-"));
        }
    }
    app_state(&repos)
}

#[test]
fn workspace_fetch_under_a_focus_targets_focused_primaries() {
    let mut app = state();
    app.set_focus_query("ABC-123");
    put_cursor(&mut app, "workspace");
    let effect = app.dispatch(crate::tui::action::Action::Fetch);
    assert_eq!(
        effect,
        Effect::Fetch {
            repos: vec!["app".into(), "services/api".into()],
        },
        "app fetches for its linked abc-123; lib and notes are outside"
    );
    assert_eq!(app.status, "Fetching 0/2 focused…");
    assert_eq!(
        app.op_focus_scope(),
        Some(FocusScope { outside: 2 }),
        "the status text names 2 outside families"
    );
}

#[test]
fn workspace_ops_with_no_focus_keep_todays_targets_and_text() {
    let mut app = state();
    put_cursor(&mut app, "workspace");
    let effect = app.dispatch(crate::tui::action::Action::Fetch);
    assert_eq!(
        effect,
        Effect::Fetch {
            repos: vec![
                "app".into(),
                "lib".into(),
                "notes".into(),
                "services/api".into(),
            ],
        }
    );
    assert_eq!(app.status, "Fetching 0/4…");
    assert_eq!(app.op_focus_scope(), None);
}

#[test]
fn workspace_pull_under_a_feature_focus_skips_a_linked_only_family() {
    let mut app = behind_state();
    app.set_focus_query("ABC-123");
    put_cursor(&mut app, "workspace");
    let effect = app.dispatch(crate::tui::action::Action::Pull);
    assert_eq!(
        effect,
        Effect::Pull {
            repos: vec!["services/api".into()],
        },
        "app is kept only through a linked worktree: pull skips it"
    );
    assert_eq!(app.status, "Pulling 0/1 focused…");
}

#[test]
fn workspace_default_switch_under_a_focus_asks_for_the_kept_primaries() {
    let mut app = behind_state();
    app.set_focus_query("app, lib");
    put_cursor(&mut app, "workspace");
    app.dispatch(crate::tui::action::Action::DefaultBranch);
    match &app.confirm {
        Some(super::super::PendingConfirm::SwitchToDefault { repos }) => {
            assert_eq!(repos, &["app", "lib"]);
        }
        other => panic!("expected the switch confirm, got {other:?}"),
    }
}

#[test]
fn a_focus_with_no_target_for_the_op_says_so() {
    let mut app = behind_state();
    // Only the linked `login` checkout is kept: no primary to pull.
    app.set_focus_query("login");
    put_cursor(&mut app, "workspace");
    let effect = app.dispatch(crate::tui::action::Action::Pull);
    assert_eq!(effect, Effect::None);
    assert_eq!(app.status, "no focused repos for that op");
    assert_eq!(app.status.kind(), StatusKind::Warn);
}

#[test]
fn a_row_op_under_a_focus_keeps_todays_targets_and_text() {
    let mut app = state();
    app.set_focus_query("ABC-123");
    app.folds.remove(OUTSIDE_FOCUS_GROUP_ID);
    app.rebuild_rows();
    put_cursor(&mut app, "repo:lib");
    let effect = app.dispatch(crate::tui::action::Action::Fetch);
    assert_eq!(
        effect,
        Effect::Fetch {
            repos: vec!["lib".into()],
        },
        "an outside repo row still fetches itself"
    );
    assert_eq!(app.status, "Fetching 0/1…");
    assert_eq!(app.op_focus_scope(), None);
}

#[test]
fn quick_open_workspace_scope_lists_only_kept_checkouts() {
    use crate::tui::quick_open::QuickOpenScope;
    let mut app = state();
    let checkouts = |app: &AppState| -> Vec<String> {
        app.quick_open_roots(&QuickOpenScope::Workspace)
            .into_iter()
            .map(|root| root.checkout)
            .collect()
    };
    assert_eq!(
        checkouts(&app),
        ["app", "lib", "notes", "services/api"],
        "no focus: every visible primary"
    );
    app.set_focus_query("ABC-123");
    assert_eq!(
        checkouts(&app),
        ["app/.worktrees/abc-123", "services/api"],
        "app's primary is not kept: its kept linked checkout lists its files; \
         outside families do not"
    );
    app.set_focus_query("app");
    assert_eq!(
        checkouts(&app),
        ["app"],
        "a repo focus keeps the whole family but lists only its primary"
    );
}

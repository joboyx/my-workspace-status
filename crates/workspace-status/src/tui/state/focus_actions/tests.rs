use super::*;
use crate::snapshot::{build_workspace_snapshot, RepoSnapshot};
use crate::testutil::{
    app_state, focus_state, focus_workspace_repos, linked_checkout, on_branch, paint, put_cursor,
    repo, row_ids, saved_focus,
};
use crate::tui::command_palette::command_for;
use crate::tui::keys::InputMode;
use crate::tui::status::StatusKind;
use crate::tui::tabs::SWITCH_TO_WORKSPACE_TAB;
use crate::tui::tree::OUTSIDE_FOCUS_GROUP_ID;
use crate::tui::tree_focus::TreeFocus;

fn applied(app: &AppState) -> Option<&str> {
    app.focus_query().map(TreeFocus::query)
}

/// The palette row reason of `action` now.
fn palette_reason(app: &AppState, action: Action) -> Option<String> {
    let command = command_for(&action).expect("palette row");
    app.palette_disabled_reason(command)
}

#[test]
fn star_focuses_the_row_feature_and_again_clears_it() {
    let mut app = focus_state();
    put_cursor(&mut app, "checkout:app/.worktrees/abc-123");
    app.dispatch(Action::FocusThisFeature);
    assert_eq!(applied(&app), Some("ABC-123"));
    assert_eq!(saved_focus(&app).as_deref(), Some("ABC-123"));
    assert_eq!(app.status, "focus: ABC-123");
    // `services/api` is on `feature/abc-123-api`: the same token in
    // another case, so `*` there clears the focus.
    put_cursor(&mut app, "repo:services/api");
    app.dispatch(Action::FocusThisFeature);
    assert_eq!(applied(&app), None, "the toggle ignores case");
    assert_eq!(saved_focus(&app), None);
    assert_eq!(app.status, "focus cleared");
}

#[test]
fn star_replaces_a_different_focus_and_ignores_outer_spaces() {
    let mut app = focus_state();
    app.set_focus_query("lib");
    app.folds.remove(OUTSIDE_FOCUS_GROUP_ID);
    app.rebuild_rows();
    put_cursor(&mut app, "repo:services/api");
    app.dispatch(Action::FocusThisFeature);
    assert_eq!(applied(&app), Some("abc-123"), "a new token replaces lib");

    app.set_focus_query("  ABC-123 ");
    put_cursor(&mut app, "repo:services/api");
    app.dispatch(Action::FocusThisFeature);
    assert_eq!(
        applied(&app),
        None,
        "trimmed, case-insensitive match clears"
    );
}

#[test]
fn star_reads_the_checkout_of_a_file_row_and_the_primary_of_a_family() {
    let web = RepoSnapshot {
        branch: "bugfix/web-42-y".to_string(),
        ..repo("web", true)
    };
    let mut app = app_state(&[web]);
    let file = app
        .rows
        .iter()
        .position(|row| row.kind == NodeKind::File)
        .expect("a file row of the dirty repo");
    app.cursor = file;
    app.dispatch(Action::FocusThisFeature);
    assert_eq!(applied(&app), Some("web-42"));

    let mut app = app_state(&[
        on_branch("app", "feature/XY-9-a"),
        linked_checkout("app/.worktrees/b", "app", "feature/b"),
    ]);
    put_cursor(&mut app, "repo:app");
    assert!(
        app.rows[app.cursor].chrome.is_family,
        "family container row"
    );
    app.dispatch(Action::FocusThisFeature);
    assert_eq!(applied(&app), Some("XY-9"), "the primary checkout's branch");
}

#[test]
fn feature_preset_names_why_it_cannot_run() {
    let mut app = app_state(&[
        on_branch("detached", "HEAD"),
        on_branch("lost", "(unknown)"),
        repo("main-repo", false),
        on_branch("feat", "feature/auth-refresh"),
        on_branch("bare", "feature/"),
    ]);
    let reason = |app: &AppState| palette_reason(app, Action::FocusThisFeature);
    put_cursor(&mut app, "workspace");
    assert_eq!(reason(&app).as_deref(), Some(FOCUS_A_REPO_OR_CHECKOUT));
    put_cursor(&mut app, "repo:detached");
    assert_eq!(reason(&app).as_deref(), Some(NO_FEATURE_ON_DETACHED_HEAD));
    put_cursor(&mut app, "repo:bare");
    assert_eq!(
        reason(&app).as_deref(),
        Some(NO_FEATURE_IN_BRANCH_NAME),
        "a bare prefix is not the default branch"
    );
    put_cursor(&mut app, "repo:lost");
    assert_eq!(reason(&app).as_deref(), Some(NO_FEATURE_ON_UNKNOWN_HEAD));
    put_cursor(&mut app, "repo:feat");
    assert_eq!(reason(&app), None);
    // `main-repo` is clean, in the folded No updates group.
    put_cursor(&mut app, "group:no-updates");
    assert_eq!(reason(&app).as_deref(), Some(FOCUS_A_REPO_OR_CHECKOUT));
    app.folds.remove("group:no-updates");
    app.rebuild_rows();
    put_cursor(&mut app, "repo:main-repo");
    assert_eq!(reason(&app).as_deref(), Some(NO_FEATURE_ON_DEFAULT_BRANCH));

    // The key says the same on the status line and changes nothing.
    app.dispatch(Action::FocusThisFeature);
    assert_eq!(app.status, NO_FEATURE_ON_DEFAULT_BRANCH);
    assert_eq!(app.status.kind(), StatusKind::Warn);
    assert_eq!(applied(&app), None);
}

#[test]
fn repo_preset_focuses_the_family_primary_path() {
    let mut app = focus_state();
    put_cursor(&mut app, "workspace");
    assert_eq!(
        palette_reason(&app, Action::FocusThisRepo).as_deref(),
        Some(FOCUS_A_REPO_OR_CHECKOUT)
    );
    app.dispatch(Action::FocusThisRepo);
    assert_eq!(app.status, FOCUS_A_REPO_OR_CHECKOUT);

    put_cursor(&mut app, "checkout:app/.worktrees/login");
    assert_eq!(palette_reason(&app, Action::FocusThisRepo), None);
    app.dispatch(Action::FocusThisRepo);
    assert_eq!(applied(&app), Some("app"), "a repo focus keeps the family");
    assert!(row_ids(&app).contains(&"checkout:app/.worktrees/login"));
}

#[test]
fn clear_row_needs_a_focus() {
    let mut app = focus_state();
    assert_eq!(
        palette_reason(&app, Action::FocusClear).as_deref(),
        Some(NO_FOCUS_SET)
    );
    app.dispatch(Action::FocusClear);
    assert_eq!(app.status, NO_FOCUS_SET);
    app.set_focus_query("lib");
    assert_eq!(palette_reason(&app, Action::FocusClear), None);
    assert_eq!(palette_reason(&app, Action::FocusPromptOpen), None);
    app.dispatch(Action::FocusClear);
    assert_eq!(applied(&app), None);
}

#[test]
fn prompt_previews_each_edit_and_esc_restores_the_applied_focus() {
    let mut app = focus_state();
    app.set_focus_query("lib");
    let applied_rows = row_ids(&app).join(" ");
    app.dispatch(Action::FocusPromptOpen);
    assert_eq!(app.input_mode(), InputMode::FocusPrompt);
    assert_eq!(
        app.focus_prompt.as_ref().map(|p| p.query.as_str()),
        Some("lib"),
        "prefilled with the applied query"
    );
    for c in ", ABC-123".chars() {
        app.dispatch(Action::FocusPromptChar(c));
    }
    assert_eq!(
        app.shown_focus_query().map(TreeFocus::query),
        Some("lib, ABC-123")
    );
    assert!(row_ids(&app).contains(&"repo:services/api"), "live preview");
    assert_eq!(applied(&app), Some("lib"), "the applied focus stays");
    assert_eq!(
        saved_focus(&app).as_deref(),
        Some("lib"),
        "a preview never saves"
    );
    app.dispatch(Action::FocusPromptBackspace);
    assert_eq!(
        app.focus_prompt.as_ref().map(|p| p.query.as_str()),
        Some("lib, ABC-12")
    );

    app.dispatch(Action::FocusPromptCancel);
    assert_eq!(app.focus_prompt, None);
    assert_eq!(
        app.input_mode(),
        InputMode::Normal {
            search_active: false
        }
    );
    assert_eq!(applied(&app), Some("lib"));
    assert_eq!(row_ids(&app).join(" "), applied_rows, "the tree is back");
    assert_eq!(app.status, FOCUS_UNCHANGED);
}

#[test]
fn prompt_enter_applies_and_an_empty_draft_clears() {
    let mut app = focus_state();
    app.dispatch(Action::FocusPromptOpen);
    assert_eq!(
        app.focus_prompt.as_ref().map(|p| p.query.as_str()),
        Some("")
    );
    for c in "lib".chars() {
        app.dispatch(Action::FocusPromptChar(c));
    }
    app.dispatch(Action::FocusPromptSubmit);
    assert_eq!(app.focus_prompt, None);
    assert_eq!(applied(&app), Some("lib"));
    assert_eq!(saved_focus(&app).as_deref(), Some("lib"));
    assert_eq!(app.status, "focus: lib");

    app.dispatch(Action::FocusPromptOpen);
    for _ in 0..3 {
        app.dispatch(Action::FocusPromptBackspace);
    }
    app.dispatch(Action::FocusPromptSubmit);
    assert_eq!(applied(&app), None, "an empty draft clears");
    assert_eq!(saved_focus(&app), None);
    assert_eq!(app.status, "focus cleared");
}

#[test]
fn focus_rows_say_switch_to_workspace_on_other_tabs() {
    let mut app = focus_state();
    app.open_prs_tab();
    for action in [
        Action::FocusThisFeature,
        Action::FocusThisRepo,
        Action::FocusPromptOpen,
        Action::FocusClear,
    ] {
        assert_eq!(
            palette_reason(&app, action.clone()).as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB),
            "{action:?}"
        );
        app.dispatch(action);
        assert_eq!(app.status, SWITCH_TO_WORKSPACE_TAB);
    }
    assert_eq!(app.focus_prompt, None);
}

/// A watch snapshot that brings repos outside the focus puts them in a
/// folded outside group, as a focus change does.
#[test]
fn an_outside_group_that_a_watch_snapshot_brings_starts_folded() {
    let mut repos = focus_workspace_repos();
    repos.retain(|repo| repo.repo.starts_with("app") || repo.repo == "services/api");
    let mut app = app_state(&repos);
    app.set_focus_query("ABC-123");
    assert!(
        !row_ids(&app).contains(&OUTSIDE_FOCUS_GROUP_ID),
        "all in focus"
    );
    app.apply_watch_snapshot(build_workspace_snapshot(
        &focus_workspace_repos(),
        &[],
        false,
        &[],
    ));
    assert!(row_ids(&app).contains(&OUTSIDE_FOCUS_GROUP_ID));
    assert!(app.folds.contains(OUTSIDE_FOCUS_GROUP_ID), "starts folded");
    assert!(!row_ids(&app).contains(&"repo:lib"));
}

/// The screen as one string per row.
fn screen(buf: &ratatui::buffer::Buffer) -> Vec<String> {
    (0..buf.area.height)
        .map(|y| {
            (0..buf.area.width)
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        })
        .collect()
}

/// The prompt paints its title, the draft, the preview count, and the
/// key hint; the tree behind it keeps its colours (no dim backdrop).
#[test]
fn prompt_paints_the_draft_and_the_preview_count_over_a_bright_tree() {
    let mut app = focus_state();
    let plain = paint(&mut app, 160);
    app.dispatch(Action::FocusPromptOpen);
    let rows = screen(&paint(&mut app, 160));
    let has = |needle: &str| rows.iter().any(|row| row.contains(needle));
    assert!(has("Focus tree"), "{rows:#?}");
    assert!(has("query: …"), "an empty draft shows the ellipsis");
    assert!(has("no focus · every repo shows"));
    assert!(has("Enter apply · Esc restore · empty clears"));

    for c in "lib".chars() {
        app.dispatch(Action::FocusPromptChar(c));
    }
    let buf = paint(&mut app, 160);
    let rows = screen(&buf);
    let has = |needle: &str| rows.iter().any(|row| row.contains(needle));
    assert!(has("query: lib"), "{rows:#?}");
    assert!(has("1/4 repos in focus"), "{rows:#?}");
    // The workspace row (top of the tree, outside the box) keeps the
    // colour it has with no dialog open.
    let (y, x) = (0..buf.area.height)
        .find_map(|y| {
            let row = &rows[usize::from(y)];
            row.find("# tmp")
                .map(|at| (y, u16::try_from(row[..at].chars().count()).unwrap()))
        })
        .expect("workspace row");
    assert_eq!(buf[(x + 2, y)].fg, plain[(x + 2, y)].fg, "not dimmed");
}

/// A click on the chip of the workspace row clears the focus.
#[test]
fn a_click_on_the_focus_chip_clears_the_focus() {
    let mut app = focus_state();
    app.set_focus_query("ABC-123");
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(160, 30)).unwrap();
    terminal
        .draw(|frame| crate::tui::render::draw(frame, &mut app))
        .unwrap();
    let hit = app.layout.focus_chip_hit.expect("the chip painted");
    app.dispatch(Action::Click {
        col: hit.x + 1,
        row: hit.y,
    });
    assert_eq!(applied(&app), None);
    assert_eq!(saved_focus(&app), None);
    assert_eq!(app.status, "focus cleared");
}

/// The chip hit-test takes only the chip cells: a click left of it on
/// the workspace row selects that row, and the focus stays.
#[test]
fn a_click_left_of_the_focus_chip_selects_the_workspace_row() {
    let mut app = focus_state();
    app.set_focus_query("ABC-123");
    put_cursor(&mut app, "repo:services/api");
    paint(&mut app, 160);
    let hit = app.layout.focus_chip_hit.expect("the chip painted");
    assert!(hit.x > 0, "{hit:?}");
    app.dispatch(Action::Click {
        col: hit.x - 1,
        row: hit.y,
    });
    assert_eq!(applied(&app), Some("ABC-123"), "the focus stays");
    assert_eq!(
        app.focused_row().map(|row| row.id.as_str()),
        Some("workspace")
    );
}

/// With a focus set, a click on the workspace PR total still opens the
/// PRs tab: the chip hit-test does not take it.
#[test]
fn a_click_on_the_pr_total_beside_the_chip_still_opens_the_prs_tab() {
    let mut app = crate::tui::state::prs_fixtures::workspace(false);
    app.set_focus_query("ABC-123");
    paint(&mut app, 200);
    let chip = app.layout.focus_chip_hit.expect("the chip painted");
    let total = app.layout.pr_total_hit.expect("the PR total painted");
    assert!(!chip.intersects(total), "{chip:?} {total:?}");
    app.dispatch(Action::Click {
        col: total.x + 1,
        row: total.y,
    });
    assert!(app.is_prs_tab());
    assert_eq!(applied(&app), Some("ABC-123"), "the focus stays");
}

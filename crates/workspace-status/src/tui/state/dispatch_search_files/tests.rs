use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

use super::super::super::action::QuickOpenEntry;
use super::super::super::effect::ExplorerPreviewBody;
use super::super::super::keys::event_to_action;
use super::super::super::quick_open::QuickOpenScope;
use super::super::super::search::SearchPane;
use super::super::super::search_files::{search_row_shows_status, SearchStatus};
use super::super::super::status::StatusMessage;
use super::super::super::tree::NodeKind;
use super::super::FocusPane;
use super::*;
use crate::file_index::{FileEntry, FileRead, IndexRoot};
use crate::file_search::{SearchChunk, SearchHit};
use crate::snapshot::{CheckoutKind, RepoSnapshot};
use crate::testutil::{self, app_state};

fn repo(name: &str, kind: CheckoutKind, primary: Option<&str>) -> RepoSnapshot {
    RepoSnapshot {
        checkout_kind: kind,
        primary_repo: primary.map(str::to_string),
        ..testutil::repo(name, true)
    }
}

/// `app` (primary) with linked `app/.worktrees/feat`, plus primary `lib`.
fn family_state() -> AppState {
    app_state(&[
        repo("app", CheckoutKind::Primary, None),
        repo("app/.worktrees/feat", CheckoutKind::Linked, Some("app")),
        repo("lib", CheckoutKind::Primary, None),
    ])
}

fn focus_row(app: &mut AppState, kind: NodeKind, repo: Option<&str>) {
    app.cursor = app
        .rows
        .iter()
        .position(|row| row.kind == kind && row.repo.as_deref() == repo)
        .unwrap_or_else(|| panic!("no {kind:?} row for {repo:?}"));
}

fn index_of(paths: &[&str]) -> FileIndex {
    FileIndex {
        roots: vec![IndexRoot {
            checkout: "app".into(),
            prefix: String::new(),
        }],
        entries: paths
            .iter()
            .map(|path| FileEntry {
                root: 0,
                display: (*path).to_string(),
                rel_start: 0,
            })
            .collect(),
        truncated: false,
        errors: Vec::new(),
    }
}

fn hit(entry: usize, line: u32) -> SearchHit {
    SearchHit {
        entry,
        line,
        text: format!("line {line}"),
        ranges: vec![(0, 4)],
    }
}

fn chunk(hits: Vec<SearchHit>, next_entry: Option<usize>) -> SearchChunk {
    SearchChunk {
        hits,
        next_entry,
        skipped: 0,
        capped: false,
    }
}

fn dialog(app: &AppState) -> &SearchFilesState {
    app.search_files.as_ref().expect("search dialog open")
}

/// Ctrl-f; returns the index load's generation and roots.
fn open(app: &mut AppState) -> (u64, Vec<IndexRoot>) {
    match app.dispatch(Action::ToggleSearchFiles) {
        Effect::LoadSearchIndex { gen, roots } => (gen, roots),
        other => panic!("expected an index load, got {other:?}"),
    }
}

fn type_text(app: &mut AppState, text: &str) -> Effect {
    let mut last = Effect::None;
    for c in text.chars() {
        last = app.dispatch(Action::SearchFilesChar(c));
    }
    last
}

/// Close the open dialog with Ctrl-f, which parks it as it is, typed
/// query included (Esc would clear a typed query first).
fn park(app: &mut AppState) {
    assert!(app.search_files.is_some(), "a dialog to park");
    assert_eq!(app.dispatch(Action::ToggleSearchFiles), Effect::None);
}

/// Fire the armed debounce as the live loop would once it is due.
fn fire(app: &mut AppState) -> Effect {
    let due = dialog(app).due.expect("debounce armed");
    app.fire_search_files_due(due).expect("due search fires")
}

/// First-chunk generation of `effect`.
fn search_gen(effect: Option<Effect>) -> u64 {
    match effect {
        Some(Effect::SearchFilesChunk {
            gen,
            start: 0,
            room,
            ..
        }) => {
            assert_eq!(room, MAX_SEARCH_HITS);
            gen
        }
        other => panic!("expected a first search chunk, got {other:?}"),
    }
}

/// Open on the `app` checkout, land a two-file index, and search
/// `query`. Returns the search generation.
fn open_and_search(app: &mut AppState, query: &str) -> u64 {
    focus_row(app, NodeKind::Checkout, Some("app"));
    let (gen, _) = open(app);
    assert_eq!(
        app.apply_search_index(gen, index_of(&["README.md", "src/main.rs"])),
        Some(Effect::None),
        "empty query: no search"
    );
    assert_eq!(type_text(app, query), Effect::None, "typing debounces");
    search_gen(Some(fire(app)))
}

#[test]
fn search_scope_follows_focus() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Workspace, None);
    let (_, roots) = open(&mut app);
    assert_eq!(dialog(&app).scope(), QuickOpenScope::Workspace);
    assert_eq!(dialog(&app).title(), "Search · all repos");
    assert_eq!(
        roots,
        vec![
            IndexRoot {
                checkout: "app".into(),
                prefix: "app/".into(),
            },
            IndexRoot {
                checkout: "lib".into(),
                prefix: "lib/".into(),
            },
        ],
        "primary checkouts only"
    );
    app.dispatch(Action::SearchFilesEscape);

    let opened = |app: &mut AppState| {
        let (_, roots) = open(app);
        app.dispatch(Action::ToggleSearchFiles);
        let parked = app.search_files_parked.as_ref().expect("parked");
        (parked.scope(), parked.title(), roots)
    };
    let single = |path: &str| {
        vec![IndexRoot {
            checkout: path.into(),
            prefix: String::new(),
        }]
    };

    focus_row(&mut app, NodeKind::Repo, Some("app"));
    assert_eq!(
        opened(&mut app),
        (
            QuickOpenScope::Checkout("app".into()),
            "Search · app".to_string(),
            single("app")
        ),
        "a family's repo row is its primary"
    );

    let linked = "app/.worktrees/feat";
    let feat = (
        QuickOpenScope::Checkout(linked.into()),
        "Search · feat".to_string(),
        single(linked),
    );
    focus_row(&mut app, NodeKind::Checkout, Some(linked));
    assert_eq!(opened(&mut app), feat, "linked worktree row");
    focus_row(&mut app, NodeKind::File, Some(linked));
    assert_eq!(opened(&mut app), feat, "a file under the linked worktree");

    focus_row(&mut app, NodeKind::Workspace, None);
    app.open_compare_tab(linked.into(), "main".into(), "HEAD".into());
    assert!(app.tabs.active_compare().is_some());
    assert_eq!(
        opened(&mut app),
        feat,
        "a compare tab scopes to its checkout"
    );

    app.open_loaded_inner_file("lib", "src/lib.rs", &["fn lib() {}"]);
    assert!(app.is_explorer_tab());
    assert_eq!(
        opened(&mut app),
        (
            QuickOpenScope::Checkout("lib".into()),
            "Search · lib".to_string(),
            single("lib")
        ),
        "an Explorer tab scopes to its checkout"
    );
}

/// Ctrl-f closes at once and parks the typed query; Esc clears a typed
/// query first and closes (and parks) only an empty one.
#[test]
fn ctrl_f_toggles_and_esc_clears_then_parks() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Checkout, Some("app"));
    open(&mut app);
    assert!(matches!(
        app.input_mode(),
        crate::tui::InputMode::SearchFiles { results: false }
    ));
    assert_eq!(dialog(&app).index, FileIndexState::Loading);
    type_text(&mut app, "main");
    assert_eq!(app.dispatch(Action::ToggleSearchFiles), Effect::None);
    assert!(app.search_files.is_none(), "Ctrl-f again closes");
    assert_eq!(
        app.search_files_parked.as_ref().map(|d| d.query.as_str()),
        Some("main"),
        "Ctrl-f parks the typed query"
    );

    open(&mut app);
    assert_eq!(dialog(&app).query, "main", "the parked query comes back");
    assert_eq!(app.dispatch(Action::SearchFilesEscape), Effect::None);
    let cleared = dialog(&app);
    assert_eq!(cleared.query, "", "Esc clears the typed query");
    assert_eq!(cleared.zone, SearchZone::Query);
    assert_eq!(cleared.due, None, "no search left due");
    assert!(app.search_files_parked.is_none(), "nothing parked yet");

    assert_eq!(app.dispatch(Action::SearchFilesEscape), Effect::None);
    assert!(app.search_files.is_none(), "Esc on an empty query closes");
    assert_eq!(
        app.search_files_parked.as_ref().map(|d| d.query.as_str()),
        Some(""),
        "Esc parks"
    );
}

/// Esc from the results zone clears the query, its hits, and the
/// running search, and puts the keys back on the query zone.
#[test]
fn esc_from_the_results_zone_clears_the_query_and_its_hits() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "main");
    assert!(app
        .apply_search_chunk(gen, chunk(vec![hit(1, 1)], Some(1)))
        .is_some());
    app.dispatch(Action::SearchFilesMove(1));
    assert_eq!(dialog(&app).zone, SearchZone::Results);
    assert!(dialog(&app).searching, "the next chunk is still running");

    assert_eq!(app.dispatch(Action::SearchFilesEscape), Effect::None);
    let cleared = dialog(&app);
    assert_eq!(cleared.query, "");
    assert_eq!(cleared.zone, SearchZone::Query);
    assert!(cleared.hits.is_empty(), "{:?}", cleared.hits);
    assert!(!cleared.searching, "the running search stops");
    assert_eq!(
        app.apply_search_chunk(gen, chunk(vec![hit(0, 2)], None)),
        None,
        "a chunk of the cleared search is dropped"
    );
    assert!(dialog(&app).hits.is_empty());
    assert!(app.search_files_parked.is_none(), "the dialog stays open");

    app.dispatch(Action::SearchFilesEscape);
    assert!(app.search_files.is_none(), "the second Esc closes");
}

#[test]
fn open_clears_a_leftover_status_but_keeps_the_quit_prompt() {
    let mut app = family_state();
    app.status = StatusMessage::info("Fetched 2 repos");
    open(&mut app);
    assert!(app.status.is_empty(), "{}", app.status);
    app.dispatch(Action::SearchFilesEscape);
    app.status = StatusMessage::info("Press Ctrl-c again to exit");
    open(&mut app);
    assert_eq!(app.status, "Press Ctrl-c again to exit");
}

#[test]
fn tab_widens_and_returns_and_next_open_follows_focus() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "main");
    app.apply_search_chunk(gen, chunk(vec![hit(1, 1)], None));
    assert_eq!(dialog(&app).hits.len(), 1);

    let effect = app.dispatch(Action::SearchFilesToggleScope);
    assert!(dialog(&app).widened);
    assert_eq!(dialog(&app).title(), "Search · all repos");
    assert!(dialog(&app).hits.is_empty(), "no hits from the old scope");
    let gen = match effect {
        Effect::LoadSearchIndex { gen, roots } => {
            assert_eq!(roots.len(), 2, "every primary: {roots:?}");
            gen
        }
        other => panic!("expected an index load, got {other:?}"),
    };
    let follow = app.apply_search_index(gen, index_of(&["app/README.md"]));
    let gen = search_gen(follow);
    assert_eq!(dialog(&app).searched, "main");

    let effect = app.dispatch(Action::SearchFilesToggleScope);
    assert!(!dialog(&app).widened);
    assert_eq!(dialog(&app).title(), "Search · app");
    assert!(
        matches!(effect, Effect::LoadSearchIndex { .. }),
        "{effect:?}"
    );
    assert_eq!(app.search_files_live_gen(), 0, "widened search stopped");
    assert_eq!(
        app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], None)),
        None
    );

    // Park widened: the next Ctrl-f follows focus, not the widening.
    app.dispatch(Action::SearchFilesToggleScope);
    park(&mut app);
    let (_, roots) = open(&mut app);
    assert!(!dialog(&app).widened);
    assert_eq!(dialog(&app).scope(), QuickOpenScope::Checkout("app".into()));
    assert_eq!(roots.len(), 1);
    assert_eq!(dialog(&app).query, "main", "query text carries");
}

#[test]
fn tab_on_the_workspace_scope_changes_nothing() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Workspace, None);
    open(&mut app);
    assert_eq!(app.dispatch(Action::SearchFilesToggleScope), Effect::None);
    assert_eq!(dialog(&app).scope(), QuickOpenScope::Workspace);
    assert!(!dialog(&app).widened, "nothing flipped");
}

#[test]
fn same_scope_ctrl_f_restores_the_dialog_as_it_was() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "line");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1), hit(0, 2), hit(1, 3)], None));
    app.dispatch(Action::SearchFilesMove(2));
    app.search_files.as_mut().unwrap().scroll = 1;
    let before = dialog(&app).clone();
    assert_eq!(before.zone, SearchZone::Results);
    park(&mut app);

    assert_eq!(app.dispatch(Action::ToggleSearchFiles), Effect::None);
    let restored = dialog(&app);
    assert_eq!(restored.query, "line");
    assert_eq!(restored.cursor, 2);
    assert_eq!(restored.scroll, 1);
    assert_eq!(restored.hits, before.hits);
    assert_eq!(restored.index, before.index, "parked index kept");
    assert_eq!(restored.zone, SearchZone::Query, "opens in the query zone");
}

#[test]
fn restore_reruns_only_unfinished_work() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "line");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], Some(1)));
    park(&mut app);
    assert_eq!(app.search_files_live_gen(), 0, "closing stops the search");
    assert_eq!(app.apply_search_chunk(gen, chunk(Vec::new(), None)), None);

    let new = search_gen(Some(app.dispatch(Action::ToggleSearchFiles)));
    assert_ne!(new, gen);
    assert_eq!(dialog(&app).hits.len(), 1, "old hits until the first chunk");
    app.apply_search_chunk(new, chunk(vec![hit(0, 1), hit(1, 2)], None));
    assert_eq!(dialog(&app).hits.len(), 2);

    // A debounce still armed at close searches on restore.
    type_text(&mut app, "s");
    park(&mut app);
    let effect = app.dispatch(Action::ToggleSearchFiles);
    assert!(
        matches!(effect, Effect::SearchFilesChunk { ref query, .. } if query == "lines"),
        "{effect:?}"
    );
    assert_eq!(dialog(&app).due, None);

    // An index still loading at close loads again.
    park(&mut app);
    app.search_files_parked.as_mut().unwrap().index = FileIndexState::Loading;
    let effect = app.dispatch(Action::ToggleSearchFiles);
    assert!(
        matches!(effect, Effect::LoadSearchIndex { .. }),
        "{effect:?}"
    );
}

#[test]
fn different_scope_ctrl_f_keeps_the_query_and_searches_again() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "line");
    app.dispatch(Action::SearchFilesToggleOption(
        SearchFilesOption::WholeWord,
    ));
    let gen2 = dialog(&app).search_gen;
    assert_ne!(gen, gen2);
    app.apply_search_chunk(gen2, chunk(vec![hit(0, 1)], None));
    park(&mut app);

    focus_row(&mut app, NodeKind::Repo, Some("lib"));
    let (gen, roots) = open(&mut app);
    assert_eq!(roots[0].checkout, "lib");
    assert_eq!(dialog(&app).query, "line");
    assert!(dialog(&app).options.whole_word, "options carry");
    assert!(dialog(&app).hits.is_empty(), "no hits from app");
    let follow = app.apply_search_index(gen, index_of(&["lib.rs"]));
    match follow {
        Some(Effect::SearchFilesChunk {
            query,
            options,
            start: 0,
            ..
        }) => {
            assert_eq!(query, "line");
            assert!(options.whole_word);
        }
        other => panic!("expected a search, got {other:?}"),
    }
}

#[test]
fn stale_index_and_chunk_generations_are_dropped() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Checkout, Some("app"));
    let (old, _) = open(&mut app);
    app.dispatch(Action::SearchFilesToggleScope);
    assert_eq!(app.apply_search_index(old, index_of(&["a.rs"])), None);
    assert_eq!(dialog(&app).index, FileIndexState::Loading);
    app.dispatch(Action::SearchFilesEscape);
    let (gen, _) = open(&mut app);
    app.dispatch(Action::SearchFilesEscape);
    assert_eq!(
        app.apply_search_index(gen, index_of(&["a.rs"])),
        None,
        "a result after close opens nothing"
    );
    assert!(app.search_files.is_none());

    let mut app = family_state();
    let first = open_and_search(&mut app, "a");
    type_text(&mut app, "b");
    let second = search_gen(Some(fire(&mut app)));
    assert_eq!(app.search_files_live_gen(), second);
    assert_eq!(
        app.apply_search_chunk(first, chunk(vec![hit(0, 1)], None)),
        None
    );
    assert!(dialog(&app).hits.is_empty(), "old generation ignored");
    assert_eq!(
        app.apply_search_chunk(second, chunk(vec![hit(1, 4)], None)),
        Some(Effect::None)
    );
    assert_eq!(dialog(&app).hits, vec![hit(1, 4)]);
    assert!(!dialog(&app).searching);
    assert_eq!(
        app.apply_search_chunk(second, chunk(vec![hit(0, 9)], None)),
        None,
        "a finished search takes no more chunks"
    );
}

#[test]
fn chunks_continue_until_the_list_ends_or_the_cap() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "line");
    // Old hits stay until the first chunk replaces them.
    app.search_files.as_mut().unwrap().hits = vec![hit(0, 99)];
    let follow = app.apply_search_chunk(gen, chunk(vec![hit(0, 1), hit(0, 2)], Some(1)));
    match follow {
        Some(Effect::SearchFilesChunk {
            gen: next,
            query,
            start,
            room,
            ..
        }) => {
            assert_eq!((next, query.as_str(), start), (gen, "line", 1));
            assert_eq!(room, MAX_SEARCH_HITS - 2);
        }
        other => panic!("expected the next chunk, got {other:?}"),
    }
    assert_eq!(dialog(&app).hits, vec![hit(0, 1), hit(0, 2)]);
    assert!(dialog(&app).searching);

    // An edit waiting on the debounce does not change the running search.
    type_text(&mut app, "x");
    let mut last = chunk(vec![hit(1, 5)], Some(2));
    last.skipped = 2;
    match app.apply_search_chunk(gen, last) {
        Some(Effect::SearchFilesChunk { query, start, .. }) => {
            assert_eq!((query.as_str(), start), ("line", 2));
        }
        other => panic!("expected the next chunk, got {other:?}"),
    }
    let mut capped = chunk(vec![hit(1, 6)], Some(3));
    capped.capped = true;
    assert_eq!(app.apply_search_chunk(gen, capped), Some(Effect::None));
    let state = dialog(&app);
    assert_eq!(state.hits.len(), 4, "chunks append");
    assert_eq!(state.skipped, 2);
    assert!(state.capped);
    assert!(!state.searching);
    assert!(state.status().contains(&SearchStatus::Capped));
}

#[test]
fn debounce_arms_on_edit_and_fires_once_due() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Checkout, Some("app"));
    let (gen, _) = open(&mut app);
    app.apply_search_index(gen, index_of(&["README.md"]));
    assert_eq!(app.search_files_due_ms(Instant::now()), None);

    assert_eq!(type_text(&mut app, "ab"), Effect::None);
    let first = dialog(&app).due.expect("armed");
    let wait = app.search_files_due_ms(Instant::now()).expect("armed");
    assert!(wait <= 150, "{wait}");
    assert_eq!(
        app.fire_search_files_due(first - Duration::from_millis(1)),
        None,
        "not due yet"
    );

    std::thread::sleep(Duration::from_millis(2));
    assert_eq!(type_text(&mut app, "c"), Effect::None);
    let second = dialog(&app).due.expect("armed");
    assert!(second > first, "a second edit pushes the deadline");
    assert_eq!(app.fire_search_files_due(first), None);
    assert_eq!(app.search_files_due_ms(second), Some(0));
    match app.fire_search_files_due(second) {
        Some(Effect::SearchFilesChunk { query, .. }) => assert_eq!(query, "abc"),
        other => panic!("expected the search, got {other:?}"),
    }
    assert_eq!(dialog(&app).due, None);
    assert_eq!(app.search_files_due_ms(Instant::now()), None);
}

#[test]
fn option_toggle_searches_at_once() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "Line");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], None));
    type_text(&mut app, "s");
    assert!(dialog(&app).due.is_some());
    let effect = app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Case));
    match effect {
        Effect::SearchFilesChunk { query, options, .. } => {
            assert_eq!(query, "Lines");
            assert!(options.case_sensitive);
        }
        other => panic!("expected a search, got {other:?}"),
    }
    assert_eq!(dialog(&app).due, None, "the toggle took the pending edit");
    assert_eq!(dialog(&app).hits.len(), 1, "old hits until the first chunk");
}

#[test]
fn invalid_regex_keeps_old_hits_and_searches_nothing() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "a(");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], None));
    assert_eq!(
        app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex)),
        Effect::None,
        "`a(` is not a regex"
    );
    let state = dialog(&app);
    assert!(state.options.regex);
    assert!(state.error.is_some());
    assert_eq!(state.hits, vec![hit(0, 1)]);
    assert!(matches!(state.status()[0], SearchStatus::InvalidRegex(_)));

    assert_eq!(type_text(&mut app, "b"), Effect::None);
    assert_eq!(dialog(&app).due, None, "no debounce for an invalid query");
    assert_eq!(dialog(&app).hits, vec![hit(0, 1)]);

    let effect = app.dispatch(Action::SearchFilesChar(')'));
    assert_eq!(effect, Effect::None);
    assert_eq!(dialog(&app).error, None, "`a(b)` compiles");
    assert!(matches!(fire(&mut app), Effect::SearchFilesChunk { .. }));
}

#[test]
fn empty_query_clears_hits_and_stops_the_search() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "a");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], Some(1)));
    assert_eq!(app.dispatch(Action::SearchFilesBackspace), Effect::None);
    let state = dialog(&app);
    assert!(state.hits.is_empty());
    assert_eq!(state.due, None);
    assert!(!state.searching);
    assert_eq!(app.search_files_live_gen(), 0);
    assert_eq!(
        app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex)),
        Effect::None,
        "an empty query never searches"
    );
}

#[test]
fn failed_index_reports_the_root_errors() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Checkout, Some("app"));
    let (gen, _) = open(&mut app);
    type_text(&mut app, "x");
    let mut index = index_of(&[]);
    index.errors = vec!["app: not a git repository".into()];
    assert_eq!(app.apply_search_index(gen, index), Some(Effect::None));
    assert_eq!(
        dialog(&app).index,
        FileIndexState::Failed("app: not a git repository".into())
    );
    assert_eq!(
        dialog(&app).status(),
        vec![SearchStatus::IndexFailed(
            "app: not a git repository".into()
        )]
    );
}

#[test]
fn typing_while_indexing_searches_when_the_index_lands() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Checkout, Some("app"));
    let (gen, _) = open(&mut app);
    type_text(&mut app, "x");
    assert_eq!(fire(&mut app), Effect::None, "no index yet");
    let follow = app.apply_search_index(gen, index_of(&["a.rs"]));
    search_gen(follow);
}

fn text(lines: &[&str]) -> FileRead {
    FileRead::Text {
        lines: lines.iter().map(|line| (*line).to_string()).collect(),
        max_cols: lines.iter().map(|line| line.len()).max().unwrap_or(0),
    }
}

const MAIN_RS: [&str; 6] = [
    "fn main() {",
    "  foo();",
    "}",
    "fn foo() {",
    "  bar();",
    "} // foo",
];

/// Map `code` through the keymap of the current input mode, then
/// dispatch it.
fn press(app: &mut AppState, code: KeyCode) -> Effect {
    let event = Event::Key(KeyEvent::new(code, KeyModifiers::NONE));
    let action = event_to_action(&event, app.input_mode(), false, false);
    app.dispatch(action)
}

fn file_cursor(app: &AppState) -> usize {
    app.tabs.active_file_view().expect("a file view").cursor
}

/// `(tab_id, gen, checkout, rel)` of the inner file load in `effect` (a
/// batch with the Explorer folder listings).
fn inner_load(effect: Effect) -> (u64, u64, String, String) {
    let effects = match effect {
        Effect::Batch(effects) => effects,
        other => vec![other],
    };
    effects
        .into_iter()
        .find_map(|effect| match effect {
            Effect::LoadExplorerPreview {
                tab_id,
                gen,
                repo,
                path,
                ..
            } => Some((tab_id, gen, repo, path)),
            _ => None,
        })
        .expect("an inner file load")
}

/// Land `lines` as the clean body of inner load `gen` of `tab_id`.
fn land_body(app: &mut AppState, tab_id: u64, gen: u64, lines: &[&str]) -> bool {
    app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::File(text(lines)))
}

/// Inner file tabs of the active Explorer tab.
fn inner_count(app: &AppState) -> usize {
    app.tabs.active_explorer().map_or(0, |tab| tab.files.len())
}

#[test]
fn enter_opens_the_hit_line_with_the_search_armed() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "foo");
    app.apply_search_chunk(gen, chunk(vec![hit(1, 2), hit(1, 4)], None));
    app.dispatch(Action::SearchFilesMove(1));
    let (tab_id, gen, repo, path) = inner_load(app.dispatch(Action::SearchFilesSubmit));
    assert_eq!((repo.as_str(), path.as_str()), ("app", "src/main.rs"));
    assert!(app.search_files.is_none(), "Enter closes the dialog");
    assert_eq!(
        app.search_files_parked.as_ref().map(|d| d.cursor),
        Some(1),
        "Enter parks it for the next Ctrl-f"
    );
    assert!(app.is_explorer_tab());
    assert_eq!(app.tabs.labels(0), ["Workspace", "Explorer · app"]);
    assert!(land_body(&mut app, tab_id, gen, &MAIN_RS));
    assert_eq!(file_cursor(&app), 3, "line 4 is cursor 3");
    assert!(app.search_active);
    assert_eq!(app.search_query, "foo");
    assert_eq!(app.search_target, SearchPane::File);
    assert_eq!(app.search_hit, Some(3));
    assert_eq!(app.focus, FocusPane::Right, "the file view takes focus");

    app.dispatch(Action::SearchNext);
    assert_eq!(file_cursor(&app), 5, "`n` steps to the next match");
    app.dispatch(Action::SearchPrev);
    assert_eq!(file_cursor(&app), 3);

    // A hit past the loaded length clamps like any cursor.
    let gen = {
        app.dispatch(Action::ToggleSearchFiles);
        type_text(&mut app, "x");
        search_gen(Some(fire(&mut app)))
    };
    app.apply_search_chunk(gen, chunk(vec![hit(1, 40)], None));
    app.dispatch(Action::SearchFilesSubmit);
    assert_eq!(file_cursor(&app), 5);
    assert_eq!(app.search_hit, None, "past the end is no hit");
}

#[test]
fn regex_enter_opens_the_line_without_arming_the_search() {
    let mut app = family_state();
    open_and_search(&mut app, "fo+");
    let gen = search_gen(Some(
        app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex)),
    ));
    app.apply_search_chunk(gen, chunk(vec![hit(1, 4)], None));
    let (tab_id, gen, ..) = inner_load(app.dispatch(Action::SearchFilesSubmit));
    land_body(&mut app, tab_id, gen, &MAIN_RS);
    assert_eq!(file_cursor(&app), 3);
    assert!(!app.search_active, "in-file search is plain text only");
    assert_eq!(app.search_hit, None);
    assert_eq!(app.focus, FocusPane::Right, "the file view takes focus");
}

#[test]
fn enter_on_an_open_tab_moves_its_cursor() {
    let mut app = family_state();
    app.open_loaded_inner_file("app", "src/main.rs", &MAIN_RS);
    assert_eq!(file_cursor(&app), 0);
    app.dispatch(Action::FocusLeft);
    let tabs = app.tabs.len();

    // On an Explorer tab the dialog scopes to its checkout, `app`.
    let (gen, roots) = open(&mut app);
    assert_eq!(roots[0].checkout, "app");
    app.apply_search_index(gen, index_of(&["README.md", "src/main.rs"]));
    type_text(&mut app, "bar");
    let gen = search_gen(Some(fire(&mut app)));
    app.apply_search_chunk(gen, chunk(vec![hit(1, 5)], None));
    assert_eq!(
        app.dispatch(Action::SearchFilesSubmit),
        Effect::None,
        "the open inner tab shows without a reload"
    );
    assert_eq!(app.tabs.len(), tabs, "the open tab is reused");
    assert_eq!(inner_count(&app), 1, "and its inner file tab");
    assert_eq!(file_cursor(&app), 4);
    assert_eq!(app.search_query, "bar");
    assert_eq!(app.search_hit, Some(4));
    assert_eq!(app.focus, FocusPane::Right, "the tree hands focus over");
}

#[test]
fn e_edits_the_hit_line_and_parks_the_dialog() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "foo");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1), hit(1, 4)], None));
    app.dispatch(Action::SearchFilesMove(1));
    assert_eq!(
        app.dispatch(Action::SearchFilesEdit),
        Effect::EditFile {
            repo: "app".into(),
            path: "src/main.rs".into(),
            line: Some(4),
        }
    );
    assert!(app.search_files.is_none(), "`e` closes the dialog");
    assert!(app.search_files_parked.is_some(), "`e` parks it");
    assert_eq!(app.status, "opening src/main.rs…");
    assert!(app.tabs.is_workspace(), "no Explorer tab for `e`");
}

#[test]
fn enter_or_e_without_a_hit_warns_and_stays_open() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "zzz");
    app.apply_search_chunk(gen, chunk(Vec::new(), None));
    for action in [Action::SearchFilesSubmit, Action::SearchFilesEdit] {
        assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
        assert!(app.search_files.is_some(), "{action:?} keeps the dialog");
        assert_eq!(app.status, NO_SEARCH_MATCHES);
    }
    type_text(&mut app, "z");
    assert!(app.status.is_empty(), "an edit drops the miss warning");
}

#[test]
fn zone_keys_move_edit_and_return_to_the_query() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "a");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1), hit(1, 2)], None));

    press(&mut app, KeyCode::Down);
    assert_eq!(dialog(&app).zone, SearchZone::Results);
    assert_eq!(dialog(&app).cursor, 1);
    assert!(matches!(
        app.input_mode(),
        crate::tui::InputMode::SearchFiles { results: true }
    ));
    press(&mut app, KeyCode::Char('k'));
    assert_eq!(dialog(&app).cursor, 0, "`k` moves in the results zone");
    press(&mut app, KeyCode::Char('x'));
    assert_eq!(dialog(&app).zone, SearchZone::Query);
    assert_eq!(dialog(&app).query, "ax", "`x` goes back and types");
    press(&mut app, KeyCode::Char('e'));
    assert_eq!(dialog(&app).query, "axe", "`e` types in the query zone");

    press(&mut app, KeyCode::Down);
    assert_eq!(dialog(&app).zone, SearchZone::Results);
    press(&mut app, KeyCode::Backspace);
    assert_eq!(dialog(&app).zone, SearchZone::Query);
    assert_eq!(dialog(&app).query, "ax", "Backspace goes back and deletes");

    press(&mut app, KeyCode::Down);
    match press(&mut app, KeyCode::Char('e')) {
        Effect::EditFile { path, line, .. } => {
            assert_eq!((path.as_str(), line), ("src/main.rs", Some(2)));
        }
        other => panic!("`e` in the results zone edits, got {other:?}"),
    }
}

#[test]
fn quick_open_hash_hands_off_to_search_files() {
    let mut app = family_state();
    focus_row(&mut app, NodeKind::Checkout, Some("app"));
    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files));
    assert!(app.quick_open.is_some());
    match press(&mut app, KeyCode::Char('#')) {
        Effect::LoadSearchIndex { roots, .. } => assert_eq!(roots[0].checkout, "app"),
        other => panic!("expected the dialog's index load, got {other:?}"),
    }
    assert!(app.quick_open.is_none(), "Quick Open closes");
    assert_eq!(dialog(&app).query, "", "`#` alone opens with no query");
    assert_eq!(dialog(&app).scope(), QuickOpenScope::Checkout("app".into()));
    for c in "foo".chars() {
        press(&mut app, KeyCode::Char(c));
    }
    assert_eq!(dialog(&app).query, "foo", "the rest types into the dialog");

    // Text after `#` is the query, leading spaces trimmed.
    park(&mut app);
    assert!(app.search_files.is_none());
    focus_row(&mut app, NodeKind::Repo, Some("lib"));
    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files));
    app.quick_open.as_mut().unwrap().query = "#  ba".into();
    let effect = app.dispatch(Action::QuickOpenChar('r'));
    assert!(
        matches!(effect, Effect::LoadSearchIndex { .. }),
        "{effect:?}"
    );
    assert_eq!(dialog(&app).query, "bar");
    let gen = dialog(&app).index_gen;
    let follow = app.apply_search_index(gen, index_of(&["lib.rs"]));
    let gen = match follow {
        Some(Effect::SearchFilesChunk { query, gen, .. }) => {
            assert_eq!(query, "bar");
            gen
        }
        other => panic!("searches as soon as the index lands, got {other:?}"),
    };

    // Enter on a hit opens it like a Ctrl-f search: the file view
    // takes focus.
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], None));
    let (_, _, _, path) = inner_load(app.dispatch(Action::SearchFilesSubmit));
    assert_eq!(path, "lib.rs");
    assert!(app.is_explorer_tab());
    assert_eq!(app.focus, FocusPane::Right);
}

#[test]
fn hash_on_a_parked_same_scope_dialog_searches_its_query_at_once() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "line");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], None));
    park(&mut app);

    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files));
    app.quick_open.as_mut().unwrap().query = "#fo".into();
    match app.dispatch(Action::QuickOpenChar('o')) {
        Effect::SearchFilesChunk {
            query, start: 0, ..
        } => assert_eq!(query, "foo"),
        other => panic!("expected a search with no debounce, got {other:?}"),
    }
    assert_eq!(dialog(&app).due, None);
    assert!(
        matches!(dialog(&app).index, FileIndexState::Ready(_)),
        "parked index kept"
    );

    // `#` alone empties the query and its hits.
    park(&mut app);
    app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files));
    assert_eq!(app.dispatch(Action::QuickOpenChar('#')), Effect::None);
    assert_eq!(dialog(&app).query, "");
    assert!(dialog(&app).hits.is_empty());
    assert_eq!(app.search_files_live_gen(), 0);
}

#[test]
fn continuations_keep_the_searched_options() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "a(");
    // A chip flipped without a new search must not reach the worker.
    app.search_files.as_mut().unwrap().options.regex = true;
    match app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], Some(1))) {
        Some(Effect::SearchFilesChunk { query, options, .. }) => {
            assert_eq!(query, "a(");
            assert!(!options.regex, "continues as the literal search it began");
        }
        other => panic!("expected the next chunk, got {other:?}"),
    }
}

#[test]
fn an_option_that_breaks_the_query_stops_the_running_search() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "a(");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], Some(1)));
    assert_eq!(app.search_files_live_gen(), gen);
    assert_eq!(
        app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex)),
        Effect::None
    );
    assert_eq!(app.search_files_live_gen(), 0, "the literal search stops");
    assert_eq!(
        app.apply_search_chunk(gen, chunk(vec![hit(1, 2)], None)),
        None,
        "its in-flight chunk is dropped"
    );
    let state = dialog(&app);
    assert_eq!(state.hits, vec![hit(0, 1)], "the list stays");
    assert!(matches!(state.status()[0], SearchStatus::InvalidRegex(_)));
}

#[test]
fn enter_in_the_stale_window_arms_the_query_the_list_shows() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "foo");
    app.apply_search_chunk(gen, chunk(vec![hit(1, 4)], None));
    type_text(&mut app, "x");
    search_gen(Some(fire(&mut app)));
    let state = dialog(&app);
    assert!(state.stale && state.searching, "`foox` has no hit in yet");
    assert_eq!(state.hits_query, "foo");
    let (tab_id, gen, ..) = inner_load(app.dispatch(Action::SearchFilesSubmit));
    land_body(&mut app, tab_id, gen, &MAIN_RS);
    assert_eq!(app.search_query, "foo", "not the newer `foox`");
    assert_eq!(app.search_hit, Some(3));
}

#[test]
fn enter_or_e_while_searching_with_no_hit_stays_quiet() {
    let mut app = family_state();
    open_and_search(&mut app, "foo");
    for action in [Action::SearchFilesSubmit, Action::SearchFilesEdit] {
        assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
        assert!(app.search_files.is_some(), "{action:?} keeps the dialog");
        assert!(app.status.is_empty(), "{action:?}: no early miss");
        assert_eq!(dialog(&app).status(), vec![SearchStatus::Searching]);
    }
    type_text(&mut app, "x");
    assert!(dialog(&app).due.is_some());
    app.dispatch(Action::SearchFilesSubmit);
    assert!(app.status.is_empty(), "a due search counts as running");
}

#[test]
fn regex_enter_on_an_open_tab_clears_its_older_search() {
    let mut app = family_state();
    app.open_loaded_inner_file("app", "src/main.rs", &MAIN_RS);
    app.search_active = true;
    app.search_query = "main".into();
    app.search_target = SearchPane::File;
    app.search_hit = Some(0);

    let (gen, _) = open(&mut app);
    app.apply_search_index(gen, index_of(&["README.md", "src/main.rs"]));
    type_text(&mut app, "ba+r");
    fire(&mut app);
    let gen = search_gen(Some(
        app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex)),
    ));
    app.apply_search_chunk(gen, chunk(vec![hit(1, 5)], None));
    app.dispatch(Action::SearchFilesSubmit);
    assert_eq!(inner_count(&app), 1, "the open inner tab is reused");
    assert_eq!(file_cursor(&app), 4);
    assert!(!app.search_active, "the `main` search is gone");
    assert_eq!(app.search_hit, None);
    assert!(app.search_query.is_empty());
}

#[test]
fn page_moves_by_the_painted_results_height() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "a");
    let hits = (1..=40).map(|line| hit(1, line)).collect();
    app.apply_search_chunk(gen, chunk(hits, None));
    app.layout.search_files_rows = 8;
    app.dispatch(Action::SearchFilesPage(1));
    assert_eq!(dialog(&app).zone, SearchZone::Results);
    assert_eq!(dialog(&app).cursor, 7, "one row less than the height");
    app.dispatch(Action::SearchFilesPage(-1));
    assert_eq!(dialog(&app).cursor, 0);
    app.layout.search_files_rows = 0;
    app.dispatch(Action::SearchFilesPage(1));
    assert_eq!(dialog(&app).cursor, 1, "before any paint: one hit");
}

#[test]
fn preview_reads_the_highlighted_hits_file_once_per_file() {
    let mut app = family_state();
    app.layout.term_cols = SEARCH_PREVIEW_MIN_COLS;
    let gen = open_and_search(&mut app, "a");
    assert_eq!(app.search_preview_request(), None, "no hit yet");
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1), hit(1, 2), hit(1, 3)], None));
    let (first, checkout, rel) = app.search_preview_request().expect("a read");
    assert_eq!((checkout.as_str(), rel.as_str()), ("app", "README.md"));
    assert_eq!(app.search_preview_request(), None, "already reading it");

    app.dispatch(Action::SearchFilesMove(1));
    let (second, _, rel) = app.search_preview_request().expect("new file");
    assert_eq!(rel, "src/main.rs");
    assert!(
        !app.apply_search_preview(first, text(&["readme"])),
        "the README read is stale"
    );
    assert!(app.apply_search_preview(second, text(&MAIN_RS)));
    assert!(!app.apply_search_preview(second, text(&MAIN_RS)), "once");
    app.dispatch(Action::SearchFilesMove(1));
    assert_eq!(app.search_preview_request(), None, "same file, no read");
    let preview = dialog(&app).preview.as_ref().expect("preview");
    assert_eq!(preview.body, Some(text(&MAIN_RS)));
}

#[test]
fn preview_waits_for_a_wide_terminal() {
    let mut app = family_state();
    app.layout.term_cols = SEARCH_PREVIEW_MIN_COLS - 1;
    let gen = open_and_search(&mut app, "a");
    app.apply_search_chunk(gen, chunk(vec![hit(1, 2)], None));
    assert_eq!(app.search_preview_request(), None, "no preview pane");
    assert_eq!(dialog(&app).preview, None);
    app.dispatch(Action::Resize {
        cols: SEARCH_PREVIEW_MIN_COLS,
        rows: 30,
    });
    assert!(app.search_preview_request().is_some(), "now it paints");
}

#[test]
fn a_preview_read_in_flight_at_close_is_asked_again_on_restore() {
    let mut app = family_state();
    app.layout.term_cols = SEARCH_PREVIEW_MIN_COLS;
    let gen = open_and_search(&mut app, "a");
    app.apply_search_chunk(gen, chunk(vec![hit(1, 2)], None));
    let (read, _, _) = app.search_preview_request().expect("a read");
    park(&mut app);
    assert!(!app.apply_search_preview(read, text(&MAIN_RS)), "closed");
    assert_eq!(app.dispatch(Action::ToggleSearchFiles), Effect::None);
    let (again, _, rel) = app.search_preview_request().expect("asked again");
    assert_ne!(again, read);
    assert_eq!(rel, "src/main.rs");
    assert!(app.apply_search_preview(again, text(&MAIN_RS)));

    // A finished preview is parked with the dialog and kept.
    park(&mut app);
    app.dispatch(Action::ToggleSearchFiles);
    assert_eq!(app.search_preview_request(), None);
    assert!(dialog(&app).preview.as_ref().unwrap().body.is_some());
}

#[test]
fn enter_or_e_while_the_index_loads_stays_quiet() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "line");
    app.apply_search_chunk(gen, chunk(Vec::new(), None));
    park(&mut app);

    // A fresh open in another scope carries the query; nothing is due
    // or running until the index lands, but a search will run then.
    focus_row(&mut app, NodeKind::Repo, Some("lib"));
    let (gen, _) = open(&mut app);
    assert_eq!(dialog(&app).query, "line");
    assert!(!dialog(&app).pending());
    for action in [Action::SearchFilesSubmit, Action::SearchFilesEdit] {
        assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
        assert!(app.status.is_empty(), "{action:?} while indexing");
    }

    let gen = search_gen(app.apply_search_index(gen, index_of(&["lib.rs"])));
    app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], None));
    assert!(!search_row_shows_status(&app.status));
    assert_eq!(dialog(&app).hits.len(), 1);
}

#[test]
fn hits_landing_drop_an_earlier_miss_warning() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "line");
    app.status = StatusMessage::warn(NO_SEARCH_MATCHES);
    app.apply_search_chunk(gen, chunk(Vec::new(), Some(1)));
    assert_eq!(app.status, NO_SEARCH_MATCHES, "no hits yet: it stays");
    app.apply_search_chunk(gen, chunk(vec![hit(1, 3)], None));
    assert!(app.status.is_empty(), "hits replace the miss");
}

#[test]
fn option_and_scope_changes_clear_the_miss_warning() {
    let mut app = family_state();
    let gen = open_and_search(&mut app, "foo(");
    app.apply_search_chunk(gen, chunk(Vec::new(), None));
    app.dispatch(Action::SearchFilesSubmit);
    assert_eq!(app.status, NO_SEARCH_MATCHES);
    app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex));
    assert!(app.status.is_empty(), "a regex toggle drops the old miss");
    assert!(matches!(
        dialog(&app).status()[0],
        SearchStatus::InvalidRegex(_)
    ));

    app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex));
    let gen = dialog(&app).search_gen;
    app.apply_search_chunk(gen, chunk(Vec::new(), None));
    app.dispatch(Action::SearchFilesEdit);
    assert_eq!(app.status, NO_SEARCH_MATCHES);
    app.dispatch(Action::SearchFilesToggleScope);
    assert!(app.status.is_empty(), "Tab drops the old miss");
    assert!(dialog(&app).widened);

    // The quit prompt is not a miss warning: it stays.
    app.status = crate::tui::ctrl_c_exit::CTRL_C_EXIT_PROMPT.into();
    app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Case));
    assert_eq!(app.status, crate::tui::ctrl_c_exit::CTRL_C_EXIT_PROMPT);
}

//! Search-in-files dialog: open / close / restore, query edits with the
//! debounce, option and scope toggles, and job results.
//!
//! Index loads and search chunks leave here as [`Effect::LoadSearchIndex`] /
//! [`Effect::SearchFilesChunk`] and come back through
//! [`AppState::apply_search_index`] / [`AppState::apply_search_chunk`],
//! which drop any result whose generation is no longer the dialog's
//! latest. An accepted chunk that did not finish the file list schedules
//! the next chunk. A query edit only arms the debounce
//! ([`AppState::search_files_due_ms`]); the live loop fires it through
//! [`AppState::fire_search_files_due`].

use std::sync::Arc;
use std::time::Instant;

use crate::file_index::FileIndex;
use crate::file_search::MAX_SEARCH_HITS;

use super::super::action::{Action, Effect, SearchFilesOption};
use super::super::ctrl_c_exit::is_ctrl_c_exit_prompt;
use super::super::quick_open::FileIndexState;
use super::super::search_files::{query_error, SearchFilesState, SearchZone, SEARCH_DEBOUNCE};
use super::AppState;

impl AppState {
    fn next_search_files_gen(&mut self) -> u64 {
        self.search_files_gen += 1;
        self.search_files_gen
    }

    /// Route one search-in-files [`Action`].
    pub(crate) fn dispatch_search_files(&mut self, action: Action) -> Effect {
        match action {
            Action::ToggleSearchFiles => self.toggle_search_files(),
            Action::SearchFilesChar(c) => {
                let Some(mut query) = self.search_files.as_ref().map(|d| d.query.clone()) else {
                    return Effect::None;
                };
                query.push(c);
                self.search_files_edit(query)
            }
            Action::SearchFilesBackspace => {
                let Some(mut query) = self.search_files.as_ref().map(|d| d.query.clone()) else {
                    return Effect::None;
                };
                query.pop();
                self.search_files_edit(query)
            }
            Action::SearchFilesMove(delta) => {
                if let Some(dialog) = self.search_files.as_mut() {
                    dialog.zone = SearchZone::Results;
                    dialog.move_cursor(delta);
                }
                Effect::None
            }
            Action::SearchFilesToggleScope => self.toggle_search_scope(),
            Action::SearchFilesToggleOption(option) => {
                let Some(dialog) = self.search_files.as_mut() else {
                    return Effect::None;
                };
                let options = &mut dialog.options;
                match option {
                    SearchFilesOption::Case => options.case_sensitive = !options.case_sensitive,
                    SearchFilesOption::WholeWord => options.whole_word = !options.whole_word,
                    SearchFilesOption::Regex => options.regex = !options.regex,
                }
                self.request_search()
            }
            Action::SearchFilesCancel => {
                self.close_search_files();
                Effect::None
            }
            _ => Effect::None,
        }
    }

    /// Ctrl-f: close the open dialog, else open it for the focused scope.
    ///
    /// A parked dialog whose results belong to that same scope comes back
    /// as it was: query, options, hits, highlighted row, and scroll. Only
    /// work it had not finished runs again (an index still loading, or a
    /// search still due or running). Any other open starts fresh in the
    /// focused scope with the parked query and options, loads the index,
    /// and searches once it lands, so no hit from another scope shows.
    fn toggle_search_files(&mut self) -> Effect {
        if self.search_files.is_some() {
            self.close_search_files();
            return Effect::None;
        }
        self.cancel_mouse_drag();
        self.help_open = false;
        self.clear_help_search();
        // A leftover toast would sit in the status row and never expire
        // while the dialog is open. The quit prompt stays: it shows inline.
        if !is_ctrl_c_exit_prompt(&self.status) {
            self.status.clear();
        }
        let focused = self.quick_open_scope();
        match self.search_files_parked.take() {
            Some(mut parked) if parked.focused == focused && parked.scope() == focused => {
                parked.zone = SearchZone::Query;
                let unfinished = parked.pending();
                parked.searching = false;
                parked.due = None;
                let index_ready = matches!(parked.index, FileIndexState::Ready(_));
                self.search_files = Some(parked);
                if !index_ready {
                    self.request_search_index()
                } else if unfinished {
                    self.request_search()
                } else {
                    Effect::None
                }
            }
            parked => {
                let (query, options) = parked
                    .map(|parked| (parked.query, parked.options))
                    .unwrap_or_default();
                self.search_files = Some(SearchFilesState::new(focused, query, options));
                self.request_search_index()
            }
        }
    }

    /// Close the dialog and park it for the restore rule. Its running
    /// search stops: the live generation goes to 0 with no dialog open.
    fn close_search_files(&mut self) {
        if let Some(dialog) = self.search_files.take() {
            self.search_files_parked = Some(dialog);
        }
    }

    /// Tab: widen to all repos, or back to the focused scope. A scope
    /// change drops the hits and reloads the index; the search follows it.
    fn toggle_search_scope(&mut self) -> Effect {
        let Some(dialog) = self.search_files.as_mut() else {
            return Effect::None;
        };
        let before = dialog.scope();
        dialog.widened = !dialog.widened;
        if dialog.scope() == before {
            return Effect::None;
        }
        dialog.clear_results();
        dialog.searching = false;
        self.request_search_index()
    }

    /// Request the index of the dialog's scope (fresh `git ls-files`).
    fn request_search_index(&mut self) -> Effect {
        let Some(scope) = self.search_files.as_ref().map(SearchFilesState::scope) else {
            return Effect::None;
        };
        let roots = self.quick_open_roots(&scope);
        let gen = self.next_search_files_gen();
        let Some(dialog) = self.search_files.as_mut() else {
            return Effect::None;
        };
        dialog.index = FileIndexState::Loading;
        dialog.index_gen = gen;
        Effect::LoadSearchIndex { gen, roots }
    }

    /// Replace the query and arm the debounce. No search leaves here.
    ///
    /// An empty query drops the hits and stops the running search. A query
    /// that does not compile shows its error, keeps the previous list, and
    /// arms nothing.
    fn search_files_edit(&mut self, query: String) -> Effect {
        let Some(dialog) = self.search_files.as_mut() else {
            return Effect::None;
        };
        dialog.zone = SearchZone::Query;
        dialog.query = query;
        dialog.error = query_error(&dialog.query, dialog.options);
        dialog.due = None;
        if dialog.query.is_empty() {
            dialog.searching = false;
            dialog.clear_results();
        } else if dialog.error.is_none() {
            dialog.due = Some(Instant::now() + SEARCH_DEBOUNCE);
        }
        Effect::None
    }

    /// Search the current query now: the first chunk of a new generation.
    ///
    /// Clears the debounce. An empty query drops the hits instead; a query
    /// that does not compile only sets its error. With the index not ready
    /// yet nothing leaves: [`Self::apply_search_index`] searches once it
    /// lands. The hits on screen stay until the first chunk replaces them.
    fn request_search(&mut self) -> Effect {
        let Some(dialog) = self.search_files.as_mut() else {
            return Effect::None;
        };
        dialog.due = None;
        dialog.error = query_error(&dialog.query, dialog.options);
        if dialog.query.is_empty() {
            dialog.searching = false;
            dialog.clear_results();
            return Effect::None;
        }
        if dialog.error.is_some() {
            return Effect::None;
        }
        let FileIndexState::Ready(index) = &dialog.index else {
            return Effect::None;
        };
        let index = Arc::clone(index);
        let gen = self.next_search_files_gen();
        let Some(dialog) = self.search_files.as_mut() else {
            return Effect::None;
        };
        dialog.search_gen = gen;
        dialog.searching = true;
        dialog.stale = true;
        dialog.searched = dialog.query.clone();
        Effect::SearchFilesChunk {
            gen,
            index,
            query: dialog.searched.clone(),
            options: dialog.options,
            start: 0,
            room: MAX_SEARCH_HITS,
        }
    }

    /// Milliseconds until the debounced search is due (rounded up), or
    /// `None` when none is armed.
    pub(crate) fn search_files_due_ms(&self, now: Instant) -> Option<u64> {
        let due = self.search_files.as_ref()?.due?;
        let wait = due.saturating_duration_since(now);
        Some(wait.as_micros().div_ceil(1000) as u64)
    }

    /// Start the debounced search when its deadline is at or before `now`.
    ///
    /// `Some(effect)` when it fired (the dialog changed and needs a paint);
    /// `None` when nothing is due yet.
    pub(crate) fn fire_search_files_due(&mut self, now: Instant) -> Option<Effect> {
        let due = self.search_files.as_ref()?.due?;
        if due > now {
            return None;
        }
        Some(self.request_search())
    }

    /// Generation of the search that may still run, or 0 when none may.
    ///
    /// The interpreter publishes it to running search chunks, which stop
    /// between files once it moved off their own generation.
    pub(crate) fn search_files_live_gen(&self) -> u64 {
        self.search_files
            .as_ref()
            .filter(|dialog| dialog.searching)
            .map_or(0, |dialog| dialog.search_gen)
    }

    /// Accept a finished index load for generation `gen`.
    ///
    /// Dropped unless the dialog is open and `gen` is its latest index
    /// request. An index with no entries and root errors becomes
    /// [`FileIndexState::Failed`]. The current query is then searched: the
    /// returned effect is that first [`Effect::SearchFilesChunk`], if any.
    ///
    /// `None` when the result was dropped; `Some(follow-up)` when accepted.
    pub(crate) fn apply_search_index(&mut self, gen: u64, index: FileIndex) -> Option<Effect> {
        let dialog = self
            .search_files
            .as_mut()
            .filter(|dialog| dialog.index_gen == gen)?;
        dialog.index = if index.entries.is_empty() && !index.errors.is_empty() {
            FileIndexState::Failed(index.errors.join("; "))
        } else {
            FileIndexState::Ready(Arc::new(index))
        };
        Some(self.request_search())
    }

    /// Accept one search chunk for generation `gen`.
    ///
    /// Dropped unless the dialog is open, searching, and `gen` is its
    /// latest search. The first chunk of a generation replaces the hits on
    /// screen; later ones append. The follow-up is the next chunk while the
    /// file list is not done and the cap was not hit, else
    /// [`Effect::None`] and the search is over.
    ///
    /// `None` when the result was dropped; `Some(follow-up)` when accepted.
    pub(crate) fn apply_search_chunk(
        &mut self,
        gen: u64,
        chunk: crate::file_search::SearchChunk,
    ) -> Option<Effect> {
        let dialog = self
            .search_files
            .as_mut()
            .filter(|dialog| dialog.searching && dialog.search_gen == gen)?;
        if dialog.stale {
            dialog.clear_results();
        }
        dialog.hits.extend(chunk.hits);
        dialog.skipped += chunk.skipped;
        dialog.capped = chunk.capped;
        let next = chunk.next_entry.filter(|_| !chunk.capped);
        let (Some(start), FileIndexState::Ready(index)) = (next, &dialog.index) else {
            dialog.searching = false;
            return Some(Effect::None);
        };
        Some(Effect::SearchFilesChunk {
            gen,
            index: Arc::clone(index),
            query: dialog.searched.clone(),
            options: dialog.options,
            start,
            room: MAX_SEARCH_HITS.saturating_sub(dialog.hits.len()),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use super::super::super::quick_open::QuickOpenScope;
    use super::super::super::search_files::SearchStatus;
    use super::super::super::status::StatusMessage;
    use super::super::super::tree::NodeKind;
    use super::*;
    use crate::file_index::{FileEntry, IndexRoot};
    use crate::file_search::{SearchChunk, SearchHit};
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };

    fn repo(name: &str, kind: CheckoutKind, primary: Option<&str>) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: false,
            changes: vec![FileChange {
                path: "README.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }],
            checkout_kind: kind,
            primary_repo: primary.map(str::to_string),
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    /// `app` (primary) with linked `app/.worktrees/feat`, plus primary `lib`.
    fn family_state() -> AppState {
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", CheckoutKind::Primary, None),
                repo("app/.worktrees/feat", CheckoutKind::Linked, Some("app")),
                repo("lib", CheckoutKind::Primary, None),
            ],
            &[],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
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
        app.dispatch(Action::SearchFilesCancel);

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

        app.open_file_tab("lib".into(), "src/lib.rs".into());
        assert!(app.is_file_tab());
        assert_eq!(
            opened(&mut app),
            (
                QuickOpenScope::Checkout("lib".into()),
                "Search · lib".to_string(),
                single("lib")
            ),
            "a file tab scopes to its checkout"
        );
    }

    #[test]
    fn ctrl_f_toggles_and_esc_parks() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        open(&mut app);
        assert!(matches!(
            app.input_mode(),
            crate::tui::InputMode::SearchFiles { results: false }
        ));
        assert_eq!(dialog(&app).index, FileIndexState::Loading);
        assert_eq!(app.dispatch(Action::ToggleSearchFiles), Effect::None);
        assert!(app.search_files.is_none(), "Ctrl-f again closes");
        assert!(app.search_files_parked.is_some());
        open(&mut app);
        assert_eq!(app.dispatch(Action::SearchFilesCancel), Effect::None);
        assert!(app.search_files.is_none(), "Esc closes");
        assert!(app.search_files_parked.is_some(), "Esc parks");
    }

    #[test]
    fn open_clears_a_leftover_status_but_keeps_the_quit_prompt() {
        let mut app = family_state();
        app.status = StatusMessage::info("Fetched 2 repos");
        open(&mut app);
        assert!(app.status.is_empty(), "{}", app.status);
        app.dispatch(Action::SearchFilesCancel);
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
        app.dispatch(Action::SearchFilesCancel);
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
        app.dispatch(Action::SearchFilesCancel);

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
        app.dispatch(Action::SearchFilesCancel);
        assert_eq!(app.search_files_live_gen(), 0, "closing stops the search");
        assert_eq!(app.apply_search_chunk(gen, chunk(Vec::new(), None)), None);

        let new = search_gen(Some(app.dispatch(Action::ToggleSearchFiles)));
        assert_ne!(new, gen);
        assert_eq!(dialog(&app).hits.len(), 1, "old hits until the first chunk");
        app.apply_search_chunk(new, chunk(vec![hit(0, 1), hit(1, 2)], None));
        assert_eq!(dialog(&app).hits.len(), 2);

        // A debounce still armed at close searches on restore.
        type_text(&mut app, "s");
        app.dispatch(Action::SearchFilesCancel);
        let effect = app.dispatch(Action::ToggleSearchFiles);
        assert!(
            matches!(effect, Effect::SearchFilesChunk { ref query, .. } if query == "lines"),
            "{effect:?}"
        );
        assert_eq!(dialog(&app).due, None);

        // An index still loading at close loads again.
        app.dispatch(Action::SearchFilesCancel);
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
        app.dispatch(Action::SearchFilesCancel);

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
        app.dispatch(Action::SearchFilesCancel);
        let (gen, _) = open(&mut app);
        app.dispatch(Action::SearchFilesCancel);
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
    fn move_enters_results_and_typing_returns_to_the_query() {
        let mut app = family_state();
        let gen = open_and_search(&mut app, "a");
        app.apply_search_chunk(gen, chunk(vec![hit(0, 1), hit(0, 2)], None));
        app.dispatch(Action::SearchFilesMove(1));
        assert_eq!(dialog(&app).zone, SearchZone::Results);
        assert_eq!(dialog(&app).cursor, 1);
        assert!(matches!(
            app.input_mode(),
            crate::tui::InputMode::SearchFiles { results: true }
        ));
        app.dispatch(Action::SearchFilesChar('b'));
        assert_eq!(dialog(&app).zone, SearchZone::Query);
        assert_eq!(dialog(&app).query, "ab");
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
}

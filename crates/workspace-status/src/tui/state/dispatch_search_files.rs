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
//! [`AppState::fire_search_files_due`]. Enter and `e` park the dialog and
//! open the highlighted hit in a file tab or in `$EDITOR`.

use std::sync::Arc;
use std::time::Instant;

use crate::file_index::{FileIndex, FileRead};
use crate::file_search::MAX_SEARCH_HITS;

use super::super::action::{Action, Effect, SearchFilesOption};
use super::super::ctrl_c_exit::is_ctrl_c_exit_prompt;
use super::super::quick_open::FileIndexState;
use super::super::search_files::{
    query_error, SearchFilesState, SearchPreview, SearchZone, NO_SEARCH_MATCHES, SEARCH_DEBOUNCE,
    SEARCH_PREVIEW_MIN_COLS,
};
use super::super::status::StatusMessage;
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
            Action::SearchFilesPage(pages) => {
                // One row less than the painted height, so the edge hit
                // stays in view; file headers make a page move a little
                // further than the rows it shows.
                let page = i32::from(self.layout.search_files_rows.saturating_sub(1).max(1));
                if let Some(dialog) = self.search_files.as_mut() {
                    dialog.zone = SearchZone::Results;
                    dialog.move_cursor(pages.saturating_mul(page));
                }
                Effect::None
            }
            Action::SearchFilesToggleScope => self.toggle_search_scope(),
            Action::SearchFilesToggleOption(option) => {
                if self.search_files.is_none() {
                    return Effect::None;
                }
                self.clear_search_miss_warning();
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
            Action::SearchFilesSubmit => self.submit_search_files(),
            Action::SearchFilesEdit => self.edit_search_files_hit(),
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
        self.open_search_files(None)
    }

    /// Quick Open `#`: open the dialog the way Ctrl-f does, with `query` as
    /// its query. A non-empty query searches at once (once the index is
    /// ready), with no debounce.
    pub(super) fn open_search_files_with(&mut self, query: String) -> Effect {
        self.open_search_files(Some(query))
    }

    /// Open the dialog for the focused scope (see
    /// [`Self::toggle_search_files`]). `query` replaces the query text; a
    /// same-scope restore whose query it changes searches again.
    fn open_search_files(&mut self, query: Option<String>) -> Effect {
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
                let mut unfinished = parked.pending();
                if let Some(query) = query.filter(|query| *query != parked.query) {
                    parked.error = query_error(&query, parked.options);
                    parked.query = query;
                    unfinished = true;
                }
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
                let (parked_query, options) = parked
                    .map(|parked| (parked.query, parked.options))
                    .unwrap_or_default();
                let query = query.unwrap_or(parked_query);
                self.search_files = Some(SearchFilesState::new(focused, query, options));
                self.request_search_index()
            }
        }
    }

    /// Close the dialog and park it for the restore rule. Its running
    /// search stops: the live generation goes to 0 with no dialog open.
    fn close_search_files(&mut self) {
        if let Some(mut dialog) = self.search_files.take() {
            // A preview read still running lands after the close and is
            // dropped; forget it so a restore asks again.
            if dialog
                .preview
                .as_ref()
                .is_some_and(|preview| preview.body.is_none())
            {
                dialog.preview = None;
            }
            self.search_files_parked = Some(dialog);
        }
    }

    /// Tab: widen to all repos, or back to the focused scope. A scope
    /// change drops the hits and reloads the index; the search follows it.
    fn toggle_search_scope(&mut self) -> Effect {
        if self.search_files.is_none() {
            return Effect::None;
        }
        self.clear_search_miss_warning();
        let Some(dialog) = self.search_files.as_mut() else {
            return Effect::None;
        };
        let widened = !dialog.widened;
        if dialog.scope_with(widened) == dialog.scope() {
            return Effect::None;
        }
        dialog.widened = widened;
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
        if self.search_files.is_none() {
            return Effect::None;
        }
        self.clear_search_miss_warning();
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
    /// that does not compile sets its error and stops the running search
    /// (its list stays). With the index not ready yet nothing leaves:
    /// [`Self::apply_search_index`] searches once it lands. The hits on
    /// screen stay until the first chunk replaces them.
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
            dialog.searching = false;
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
        dialog.searched_options = dialog.options;
        Effect::SearchFilesChunk {
            gen,
            index,
            query: dialog.searched.clone(),
            options: dialog.searched_options,
            start: 0,
            room: MAX_SEARCH_HITS,
        }
    }

    /// Checkout, checkout-relative path, and 1-based line of the
    /// highlighted hit, or `None` with no hit (or no ready index to name
    /// its file).
    fn search_files_target(&self) -> Option<(String, String, u32)> {
        let dialog = self.search_files.as_ref()?;
        let FileIndexState::Ready(index) = &dialog.index else {
            return None;
        };
        let hit = dialog.hits.get(dialog.cursor)?;
        let entry = index.entries.get(hit.entry)?;
        Some((
            index.checkout(entry).to_string(),
            entry.rel().to_string(),
            hit.line,
        ))
    }

    /// Enter: park the dialog, then open or focus the highlighted hit's
    /// file tab with its cursor on the hit line.
    ///
    /// A literal search also arms the in-file search on the searched text,
    /// so `n` / `N` step its matches; a regex search does not (the in-file
    /// search matches plain text only). The armed text is the query the
    /// shown hits came from. With no hit the dialog stays and the status
    /// warns `no matches` (see [`Self::warn_no_search_hit`]).
    fn submit_search_files(&mut self) -> Effect {
        let Some((checkout, rel, line)) = self.search_files_target() else {
            self.warn_no_search_hit();
            return Effect::None;
        };
        // Arm what the list shows: during a new search's first chunk the
        // hits still come from the previous query.
        let search = self
            .search_files
            .as_ref()
            .filter(|dialog| !dialog.hits_options.regex)
            .map(|dialog| dialog.hits_query.clone());
        self.close_search_files();
        let line = usize::try_from(line.saturating_sub(1)).unwrap_or(usize::MAX);
        self.open_file_tab_at_line(checkout, rel, line, search)
    }

    /// `e`: park the dialog and open the highlighted hit's file in the
    /// editor at the hit line. With no hit the dialog stays and the status
    /// warns `no matches` (see [`Self::warn_no_search_hit`]).
    fn edit_search_files_hit(&mut self) -> Effect {
        let Some((repo, path, line)) = self.search_files_target() else {
            self.warn_no_search_hit();
            return Effect::None;
        };
        self.close_search_files();
        self.status = StatusMessage::progress(format!("opening {path}…"));
        Effect::EditFile {
            repo,
            path,
            line: Some(line),
        }
    }

    /// Drop the `no matches` warning of an earlier Enter or `e`: it was
    /// about the query, options, and scope before this change, and while
    /// it shows it hides the dialog's own status row.
    fn clear_search_miss_warning(&mut self) {
        if self.status == NO_SEARCH_MATCHES {
            self.status.clear();
        }
    }

    /// Enter or `e` found no hit: warn `no matches`, unless a search is
    /// still due or running (its `searching…` status stays instead).
    fn warn_no_search_hit(&mut self) {
        if self
            .search_files
            .as_ref()
            .is_some_and(SearchFilesState::pending)
        {
            return;
        }
        self.status = StatusMessage::warn(NO_SEARCH_MATCHES);
    }

    /// Ask for the highlighted hit's file for the preview pane.
    ///
    /// `Some((gen, checkout, rel))` when the dialog is open, the terminal is
    /// at least [`SEARCH_PREVIEW_MIN_COLS`] wide (the preview paints), and
    /// the highlighted hit's file is not the one the preview already holds
    /// or reads; the preview then waits on that read. `None` otherwise. The
    /// interpreter asks after every change and keeps only the latest read.
    pub(crate) fn search_preview_request(&mut self) -> Option<(u64, String, String)> {
        if self.layout.term_cols < SEARCH_PREVIEW_MIN_COLS {
            return None;
        }
        let (checkout, rel, _) = self.search_files_target()?;
        let dialog = self.search_files.as_ref()?;
        if dialog
            .preview
            .as_ref()
            .is_some_and(|preview| preview.is_for(&checkout, &rel))
        {
            return None;
        }
        let gen = self.next_search_files_gen();
        let dialog = self.search_files.as_mut()?;
        dialog.preview = Some(SearchPreview {
            checkout: checkout.clone(),
            rel: rel.clone(),
            gen,
            body: None,
        });
        Some((gen, checkout, rel))
    }

    /// Accept the preview read for generation `gen`. False (dropped)
    /// unless the dialog is open and still waits on that read.
    pub(crate) fn apply_search_preview(&mut self, gen: u64, body: FileRead) -> bool {
        let Some(preview) = self
            .search_files
            .as_mut()
            .and_then(|dialog| dialog.preview.as_mut())
            .filter(|preview| preview.gen == gen && preview.body.is_none())
        else {
            return false;
        };
        preview.body = Some(body);
        true
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
            dialog.hits_query = dialog.searched.clone();
            dialog.hits_options = dialog.searched_options;
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
            options: dialog.searched_options,
            start,
            room: MAX_SEARCH_HITS.saturating_sub(dialog.hits.len()),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::time::Duration;

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::super::super::action::QuickOpenEntry;
    use super::super::super::keys::event_to_action;
    use super::super::super::quick_open::QuickOpenScope;
    use super::super::super::search::SearchPane;
    use super::super::super::search_files::SearchStatus;
    use super::super::super::status::StatusMessage;
    use super::super::super::tree::NodeKind;
    use super::*;
    use crate::file_index::{FileEntry, FileRead, IndexRoot};
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
        app.tabs.active_file().expect("file tab").cursor
    }

    #[test]
    fn enter_opens_the_hit_line_with_the_search_armed() {
        let mut app = family_state();
        let gen = open_and_search(&mut app, "foo");
        app.apply_search_chunk(gen, chunk(vec![hit(1, 2), hit(1, 4)], None));
        app.dispatch(Action::SearchFilesMove(1));
        let Effect::LoadFileTab {
            tab_id,
            gen,
            repo,
            path,
        } = app.dispatch(Action::SearchFilesSubmit)
        else {
            panic!("expected a new file tab load");
        };
        assert_eq!((repo.as_str(), path.as_str()), ("app", "src/main.rs"));
        assert!(app.search_files.is_none(), "Enter closes the dialog");
        assert_eq!(
            app.search_files_parked.as_ref().map(|d| d.cursor),
            Some(1),
            "Enter parks it for the next Ctrl-f"
        );
        assert!(app.is_file_tab());
        assert!(app.apply_file_tab(tab_id, gen, text(&MAIN_RS)));
        assert_eq!(file_cursor(&app), 3, "line 4 is cursor 3");
        assert!(app.search_active);
        assert_eq!(app.search_query, "foo");
        assert_eq!(app.search_target, SearchPane::File);
        assert_eq!(app.search_hit, Some(3));

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
        app.apply_search_chunk(gen, chunk(vec![hit(0, 40)], None));
        let Effect::LoadFileTab { tab_id, gen, .. } = app.dispatch(Action::SearchFilesSubmit)
        else {
            panic!("expected a new file tab load");
        };
        app.apply_file_tab(tab_id, gen, text(&["a", "b"]));
        assert_eq!(file_cursor(&app), 1);
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
        let Effect::LoadFileTab { tab_id, gen, .. } = app.dispatch(Action::SearchFilesSubmit)
        else {
            panic!("expected a new file tab load");
        };
        app.apply_file_tab(tab_id, gen, text(&MAIN_RS));
        assert_eq!(file_cursor(&app), 3);
        assert!(!app.search_active, "in-file search is plain text only");
        assert_eq!(app.search_hit, None);
    }

    #[test]
    fn enter_on_an_open_tab_moves_its_cursor() {
        let mut app = family_state();
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "src/main.rs".into())
        else {
            panic!("expected a load");
        };
        app.apply_file_tab(tab_id, gen, text(&MAIN_RS));
        assert_eq!(file_cursor(&app), 0);
        let tabs = app.tabs.len();

        // On a file tab the dialog scopes to its checkout, `app`.
        let (gen, roots) = open(&mut app);
        assert_eq!(roots[0].checkout, "app");
        app.apply_search_index(gen, index_of(&["README.md", "src/main.rs"]));
        type_text(&mut app, "bar");
        let gen = search_gen(Some(fire(&mut app)));
        app.apply_search_chunk(gen, chunk(vec![hit(1, 5)], None));
        app.dispatch(Action::SearchFilesSubmit);
        assert_eq!(app.tabs.len(), tabs, "the open tab is reused");
        assert_eq!(app.tabs.active_file().map(|tab| tab.id), Some(tab_id));
        assert_eq!(file_cursor(&app), 4);
        assert_eq!(app.search_query, "bar");
        assert_eq!(app.search_hit, Some(4));
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
        assert!(!app.is_file_tab(), "no file tab for `e`");
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
        app.dispatch(Action::SearchFilesCancel);
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
        match follow {
            Some(Effect::SearchFilesChunk { query, .. }) => assert_eq!(query, "bar"),
            other => panic!("searches as soon as the index lands, got {other:?}"),
        }
    }

    #[test]
    fn hash_on_a_parked_same_scope_dialog_searches_its_query_at_once() {
        let mut app = family_state();
        let gen = open_and_search(&mut app, "line");
        app.apply_search_chunk(gen, chunk(vec![hit(0, 1)], None));
        app.dispatch(Action::SearchFilesCancel);

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
        app.dispatch(Action::SearchFilesCancel);
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
        let Effect::LoadFileTab { tab_id, gen, .. } = app.dispatch(Action::SearchFilesSubmit)
        else {
            panic!("expected a new file tab load");
        };
        app.apply_file_tab(tab_id, gen, text(&MAIN_RS));
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
        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "src/main.rs".into())
        else {
            panic!("expected a load");
        };
        app.apply_file_tab(tab_id, gen, text(&MAIN_RS));
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
        assert_eq!(app.tabs.active_file().map(|tab| tab.id), Some(tab_id));
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
        app.dispatch(Action::SearchFilesCancel);
        assert!(!app.apply_search_preview(read, text(&MAIN_RS)), "closed");
        assert_eq!(app.dispatch(Action::ToggleSearchFiles), Effect::None);
        let (again, _, rel) = app.search_preview_request().expect("asked again");
        assert_ne!(again, read);
        assert_eq!(rel, "src/main.rs");
        assert!(app.apply_search_preview(again, text(&MAIN_RS)));

        // A finished preview is parked with the dialog and kept.
        app.dispatch(Action::SearchFilesCancel);
        app.dispatch(Action::ToggleSearchFiles);
        assert_eq!(app.search_preview_request(), None);
        assert!(dialog(&app).preview.as_ref().unwrap().body.is_some());
    }

    #[test]
    fn option_and_scope_changes_clear_the_miss_warning() {
        let mut app = family_state();
        let gen = open_and_search(&mut app, "foo(");
        app.apply_search_chunk(gen, chunk(Vec::new(), None));
        app.dispatch(Action::SearchFilesSubmit);
        assert_eq!(app.status, NO_SEARCH_MATCHES);
        app.dispatch(Action::SearchFilesToggleOption(SearchFilesOption::Regex));
        assert!(app.status.is_empty(), "Alt-r drops the old miss");
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
}

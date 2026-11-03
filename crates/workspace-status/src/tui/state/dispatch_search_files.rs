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
//! open the highlighted hit in its checkout's Explorer tab or in `$EDITOR`.

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
            Action::SearchFilesEscape => self.escape_search_files(),
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

    /// Esc: clear a typed query the way Backspace edits it (hits, due
    /// search, and error go too) and keep the dialog open on the query
    /// zone. On an empty query, close and park it
    /// ([`Self::close_search_files`]).
    fn escape_search_files(&mut self) -> Effect {
        let Some(dialog) = self.search_files.as_ref() else {
            return Effect::None;
        };
        if dialog.query.is_empty() {
            self.close_search_files();
            return Effect::None;
        }
        self.search_files_edit(String::new())
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

    /// Enter: park the dialog, then open the highlighted hit's file in its
    /// checkout's Explorer tab ([`Self::open_file_in_explorer`]) with the
    /// file pane focused and the cursor on the hit line (the diff row of
    /// that line for a changed file).
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
        self.open_file_in_explorer(checkout, rel, Some(line), search)
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
    /// still due or running, or will run once the loading index lands
    /// (the dialog's own `searching…` / `indexing…` status stays instead).
    fn warn_no_search_hit(&mut self) {
        if self.search_files.as_ref().is_some_and(|dialog| {
            dialog.pending()
                || (matches!(
                    dialog.index,
                    FileIndexState::NotLoaded | FileIndexState::Loading
                ) && !dialog.query.is_empty()
                    && dialog.error.is_none())
        }) {
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
    /// screen; later ones append. Hits drop an earlier `no matches`
    /// warning (see [`Self::warn_no_search_hit`]). The follow-up is the next chunk while the
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
        if !chunk.hits.is_empty() && self.status == NO_SEARCH_MATCHES {
            self.status.clear();
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
mod tests;

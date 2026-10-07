//! Search-in-files dialog state (Ctrl-f).
//!
//! One query line searches the text of every file in the dialog's scope:
//! the files Quick Open lists for the same [`QuickOpenScope`]. The file
//! index and the search run on the blocking pool. The search comes back in
//! chunks ([`crate::file_search::search_chunk`]); this state only holds the
//! accepted hits and the generations that let apply drop stale results.
//! Typing waits [`SEARCH_DEBOUNCE`] before it searches; option toggles, Tab,
//! and opening search at once.

use std::time::{Duration, Instant};

use crate::file_index::FileRead;
use crate::file_search::{build_matcher, SearchHit, SearchOptions, MAX_SEARCH_HITS};

use super::ctrl_c_exit::is_ctrl_c_exit_prompt;
use super::quick_open::{FileIndexState, QuickOpenScope};
use super::tabs::checkout_leaf;

/// Quiet time after the last query edit before the search starts.
pub const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

/// Warning when Enter or `e` finds no highlighted hit to open.
pub const NO_SEARCH_MATCHES: &str = "no matches";

/// Wall time one search chunk may spend on the blocking pool before it
/// returns its hits and the next chunk is scheduled.
pub const SEARCH_CHUNK_BUDGET: Duration = Duration::from_millis(30);

/// Narrowest terminal, in columns, that paints the preview pane and reads
/// its file. Below it the results take the dialog's full width.
pub const SEARCH_PREVIEW_MIN_COLS: u16 = 100;

/// File body the preview pane shows, keyed by checkout and path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchPreview {
    /// Snapshot `repo` path of the file's checkout.
    pub checkout: String,
    /// Checkout-relative path of the file.
    pub rel: String,
    /// Generation of the read; a result for another generation is dropped.
    pub gen: u64,
    /// The read, or `None` while it runs.
    pub body: Option<FileRead>,
}

impl SearchPreview {
    /// True when this preview is for `rel` in `checkout`.
    pub fn is_for(&self, checkout: &str, rel: &str) -> bool {
        self.checkout == checkout && self.rel == rel
    }
}

/// One painted row of the results list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchRow {
    /// File header: index into the file index's entries.
    Header(usize),
    /// Hit line: index into [`SearchFilesState::hits`].
    Hit(usize),
}

/// Which part of the dialog the keys drive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SearchZone {
    /// Printable keys type into the query (the zone on open).
    Query,
    /// `j` / `k` move the highlighted hit; other printable keys go back to
    /// the query and type there.
    Results,
}

/// One part of the dialog's status row. Several can show at once, in the
/// order [`SearchFilesState::status`] returns them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SearchStatus {
    /// The query does not compile (regex mode). The previous list stays.
    InvalidRegex(String),
    /// The scope's file index is loading.
    Indexing,
    /// Every root of the scope failed to list; the text joins the errors.
    IndexFailed(String),
    /// A search is due or running and no hit of it has landed yet.
    Searching,
    /// The finished search found nothing; carries the scope label.
    NoMatches(String),
    /// The search stopped at [`MAX_SEARCH_HITS`].
    Capped,
    /// Files skipped as binary or too large.
    Skipped(usize),
}

impl SearchStatus {
    /// Text the status row paints for this part.
    pub fn text(&self) -> String {
        match self {
            Self::InvalidRegex(err) => format!("invalid regex: {err}"),
            Self::Indexing => "indexing…".to_string(),
            Self::IndexFailed(err) => err.clone(),
            Self::Searching => "searching…".to_string(),
            Self::NoMatches(scope) => format!("no matches in {scope}"),
            Self::Capped => {
                format!(
                    "{MAX_SEARCH_HITS}+ hits · showing first {MAX_SEARCH_HITS} · narrow the query"
                )
            }
            Self::Skipped(count) => format!("{count} skipped (binary / >2 MiB)"),
        }
    }

    /// True for a part painted as an error (red).
    pub fn is_error(&self) -> bool {
        matches!(self, Self::InvalidRegex(_) | Self::IndexFailed(_))
    }

    /// True for a part painted dim.
    pub fn is_dim(&self) -> bool {
        matches!(self, Self::Skipped(_))
    }
}

/// Whether the dialog's status row paints the app `status` instead of
/// [`SearchFilesState::status`]: only the dialog's own [`NO_SEARCH_MATCHES`]
/// warning and the Ctrl-c quit prompt (shown inline) qualify.
pub fn search_row_shows_status(status: &str) -> bool {
    status == NO_SEARCH_MATCHES || is_ctrl_c_exit_prompt(status)
}

/// Short name of `scope`: `all repos`, or the checkout's leaf.
pub fn scope_label(scope: &QuickOpenScope) -> String {
    match scope {
        QuickOpenScope::Workspace => "all repos".to_string(),
        QuickOpenScope::Checkout(path) => checkout_leaf(path),
    }
}

/// Why `query` cannot search under `options`, or `None` when it can (an
/// empty query never searches, so it has no error).
pub(crate) fn query_error(query: &str, options: SearchOptions) -> Option<String> {
    if query.is_empty() {
        return None;
    }
    build_matcher(query, options).err()
}

/// Search-in-files dialog, open or parked for the next Ctrl-f.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchFilesState {
    /// Query line as typed.
    pub query: String,
    /// Case, whole-word, and regex chips.
    pub options: SearchOptions,
    /// Scope focus had when the dialog opened.
    pub focused: QuickOpenScope,
    /// Tab widened the dialog to all repos. Never carried into a new open.
    pub widened: bool,
    /// Zone the keys drive.
    pub zone: SearchZone,
    /// File index of [`Self::scope`].
    pub index: FileIndexState,
    /// Generation of the latest requested index load.
    pub index_gen: u64,
    /// Generation of the latest requested search.
    pub search_gen: u64,
    /// Query the running (or last) search used. A chunk continuation
    /// searches this text, not a newer edit still waiting for the debounce.
    pub searched: String,
    /// Options the running (or last) search used. A chunk continuation
    /// searches with these, not with a chip flipped since.
    pub searched_options: SearchOptions,
    /// Query the hits on screen came from. Set when the first chunk of a
    /// search lands, so Enter arms the in-file search on what the list
    /// shows, not on a newer search whose hits are not in yet.
    pub hits_query: String,
    /// Options the hits on screen came from (see [`Self::hits_query`]).
    pub hits_options: SearchOptions,
    /// A search of [`Self::search_gen`] is queued or running.
    pub searching: bool,
    /// [`Self::hits`] came from an earlier search. The first chunk of
    /// [`Self::search_gen`] replaces them.
    pub stale: bool,
    /// Accepted hits, in index order then line order.
    pub hits: Vec<SearchHit>,
    /// Files the search skipped as binary or too large.
    pub skipped: usize,
    /// The search stopped at [`MAX_SEARCH_HITS`].
    pub capped: bool,
    /// Compile error of the query, painted as `invalid regex: <error>`.
    pub error: Option<String>,
    /// Highlight index into [`Self::hits`].
    pub cursor: usize,
    /// First painted result row (file headers count as rows). Paint keeps
    /// the highlighted row in view; a restore keeps it as it was.
    pub scroll: usize,
    /// When the debounced search of the last query edit starts.
    pub due: Option<Instant>,
    /// File body of the highlighted hit's file for the preview pane.
    pub preview: Option<SearchPreview>,
}

impl SearchFilesState {
    /// Fresh dialog for `focused` with `query` and `options` (carried over
    /// from the parked dialog, if any). No index yet.
    pub fn new(focused: QuickOpenScope, query: String, options: SearchOptions) -> Self {
        Self {
            error: query_error(&query, options),
            query,
            options,
            focused,
            widened: false,
            zone: SearchZone::Query,
            index: FileIndexState::NotLoaded,
            index_gen: 0,
            search_gen: 0,
            searched: String::new(),
            searched_options: SearchOptions::default(),
            hits_query: String::new(),
            hits_options: SearchOptions::default(),
            searching: false,
            stale: false,
            hits: Vec::new(),
            skipped: 0,
            capped: false,
            cursor: 0,
            scroll: 0,
            due: None,
            preview: None,
        }
    }

    /// Checkouts searched: all repos when widened, else the focused scope.
    pub fn scope(&self) -> QuickOpenScope {
        self.scope_with(self.widened)
    }

    /// Checkouts searched with `widened` in place of [`Self::widened`].
    pub fn scope_with(&self, widened: bool) -> QuickOpenScope {
        if widened {
            QuickOpenScope::Workspace
        } else {
            self.focused.clone()
        }
    }

    /// Dialog title: `Search · all repos` or `Search · <checkout leaf>`.
    pub fn title(&self) -> String {
        format!("Search · {}", scope_label(&self.scope()))
    }

    /// True while a search is due (debounce) or running.
    pub fn pending(&self) -> bool {
        self.searching || self.due.is_some()
    }

    /// Move the highlight by `delta`, clamped to the hits.
    pub fn move_cursor(&mut self, delta: i32) {
        let len = self.hits.len();
        if len == 0 {
            self.cursor = 0;
            return;
        }
        let next = (self.cursor as i64).saturating_add(i64::from(delta));
        self.cursor = next.clamp(0, len as i64 - 1) as usize;
    }

    /// Highlighted hit, if any.
    pub fn selected_hit(&self) -> Option<&SearchHit> {
        self.hits.get(self.cursor)
    }

    /// Results list rows: a header before the first hit of each file, then
    /// one row per hit.
    pub fn result_rows(&self) -> Vec<SearchRow> {
        let mut rows = Vec::with_capacity(self.hits.len() + 1);
        let mut last_entry = None;
        for (idx, hit) in self.hits.iter().enumerate() {
            if last_entry != Some(hit.entry) {
                rows.push(SearchRow::Header(hit.entry));
                last_entry = Some(hit.entry);
            }
            rows.push(SearchRow::Hit(idx));
        }
        rows
    }

    /// Move [`Self::scroll`] so the highlighted hit shows in `height`
    /// painted rows of `rows` ([`Self::result_rows`]), with its file header
    /// when that fits. Keeps the scroll while the hit already shows and
    /// never leaves blank rows under the last result.
    pub fn scroll_to_cursor(&mut self, rows: &[SearchRow], height: usize) {
        let height = height.max(1);
        if let Some(at) = rows
            .iter()
            .position(|row| *row == SearchRow::Hit(self.cursor))
        {
            let top = match at.checked_sub(1).map(|above| rows[above]) {
                Some(SearchRow::Header(_)) => at - 1,
                _ => at,
            };
            if top < self.scroll {
                self.scroll = top;
            }
            if at >= self.scroll + height {
                self.scroll = at + 1 - height;
            }
        }
        self.scroll = self.scroll.min(rows.len().saturating_sub(height));
    }

    /// Drop the hits and what was counted with them.
    pub(crate) fn clear_results(&mut self) {
        self.hits.clear();
        self.skipped = 0;
        self.capped = false;
        self.stale = false;
        self.cursor = 0;
        self.scroll = 0;
    }

    /// Footer hints for the zone the keys drive, ` · `-separated.
    ///
    /// Query zone: `↑↓ move · Enter open · Tab all repos · Alt-c/w/r · Esc
    /// close`. The results zone moves on `j/k` and adds `e edit`. Once
    /// widened, Tab names the checkout it goes back to; with focus on the
    /// workspace Tab changes nothing and has no hint.
    pub fn footer_hints(&self) -> String {
        let results = self.zone == SearchZone::Results;
        let mut hints = vec![if results { "j/k move" } else { "↑↓ move" }.to_string()];
        hints.push("Enter open".into());
        if results {
            hints.push("e edit".into());
        }
        match (&self.focused, self.widened) {
            (QuickOpenScope::Workspace, _) => {}
            (_, false) => hints.push("Tab all repos".into()),
            (focused, true) => hints.push(format!("Tab {}", scope_label(focused))),
        }
        hints.push("Alt-c/w/r".into());
        hints.push("Esc close".into());
        hints.join(" · ")
    }

    /// Status row parts, in paint order.
    ///
    /// `invalid regex: …` first; then `indexing…` or the index error until
    /// the index is ready; then `searching…` while a search is due or
    /// running with none of its hits in yet, else `no matches in <scope>`
    /// once a search came back empty; then the cap and the skipped count.
    pub fn status(&self) -> Vec<SearchStatus> {
        let mut parts = Vec::new();
        if let Some(err) = &self.error {
            parts.push(SearchStatus::InvalidRegex(err.clone()));
        }
        match &self.index {
            FileIndexState::NotLoaded | FileIndexState::Loading => {
                parts.push(SearchStatus::Indexing);
            }
            FileIndexState::Failed(err) => parts.push(SearchStatus::IndexFailed(err.clone())),
            FileIndexState::Ready(_) => {
                if self.pending() && (self.stale || self.hits.is_empty()) {
                    parts.push(SearchStatus::Searching);
                } else if !self.pending()
                    && self.error.is_none()
                    && !self.query.is_empty()
                    && self.hits.is_empty()
                {
                    parts.push(SearchStatus::NoMatches(scope_label(&self.scope())));
                }
            }
        }
        if self.capped {
            parts.push(SearchStatus::Capped);
        }
        if self.skipped > 0 {
            parts.push(SearchStatus::Skipped(self.skipped));
        }
        parts
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::file_index::FileIndex;

    fn hit(entry: usize, line: u32) -> SearchHit {
        SearchHit {
            entry,
            line,
            text: String::new(),
            ranges: Vec::new(),
        }
    }

    fn ready() -> FileIndexState {
        FileIndexState::Ready(Arc::new(FileIndex::default()))
    }

    fn texts(state: &SearchFilesState) -> Vec<String> {
        state.status().iter().map(SearchStatus::text).collect()
    }

    #[test]
    fn titles_name_all_repos_or_the_checkout_leaf() {
        let mut state = SearchFilesState::new(
            QuickOpenScope::Checkout("app/.worktrees/feat".into()),
            String::new(),
            SearchOptions::default(),
        );
        assert_eq!(state.title(), "Search · feat");
        state.widened = true;
        assert_eq!(state.scope(), QuickOpenScope::Workspace);
        assert_eq!(state.title(), "Search · all repos");
        let workspace = SearchFilesState::new(
            QuickOpenScope::Workspace,
            String::new(),
            SearchOptions::default(),
        );
        assert_eq!(workspace.title(), "Search · all repos");
    }

    #[test]
    fn new_validates_a_carried_query() {
        let regex = SearchOptions {
            regex: true,
            ..SearchOptions::default()
        };
        let state = SearchFilesState::new(QuickOpenScope::Workspace, "(".into(), regex);
        assert!(state.error.is_some());
        let literal = SearchFilesState::new(
            QuickOpenScope::Workspace,
            "(".into(),
            SearchOptions::default(),
        );
        assert_eq!(literal.error, None);
        assert_eq!(query_error("", regex), None, "empty never searches");
    }

    #[test]
    fn cursor_clamps_to_hits() {
        let mut state = SearchFilesState::new(
            QuickOpenScope::Workspace,
            String::new(),
            SearchOptions::default(),
        );
        state.move_cursor(3);
        assert_eq!(state.cursor, 0);
        state.hits = vec![hit(0, 1), hit(0, 2), hit(1, 7)];
        state.move_cursor(10);
        assert_eq!(state.cursor, 2);
        state.move_cursor(i32::MIN);
        assert_eq!(state.cursor, 0);
    }

    #[test]
    fn status_follows_index_search_and_counts() {
        let mut state = SearchFilesState::new(
            QuickOpenScope::Checkout("apps/web".into()),
            "needle".into(),
            SearchOptions::default(),
        );
        assert_eq!(texts(&state), vec!["indexing…"]);
        state.index = FileIndexState::Failed("web: not a git repository".into());
        assert_eq!(texts(&state), vec!["web: not a git repository"]);
        assert!(state.status()[0].is_error());

        state.index = ready();
        state.due = Some(Instant::now());
        assert_eq!(texts(&state), vec!["searching…"], "debounce pending");
        state.due = None;
        state.searching = true;
        assert_eq!(texts(&state), vec!["searching…"]);
        state.hits = vec![hit(0, 1)];
        assert!(texts(&state).is_empty(), "hits in: no searching…");
        state.stale = true;
        assert_eq!(texts(&state), vec!["searching…"], "old hits only");

        state.stale = false;
        state.searching = false;
        state.hits.clear();
        assert_eq!(texts(&state), vec!["no matches in web"]);
        state.widened = true;
        assert_eq!(texts(&state), vec!["no matches in all repos"]);

        state.hits = vec![hit(0, 1)];
        state.capped = true;
        state.skipped = 3;
        let parts = state.status();
        assert_eq!(
            texts(&state),
            vec![
                "5000+ hits · showing first 5000 · narrow the query",
                "3 skipped (binary / >2 MiB)",
            ]
        );
        assert!(parts[1].is_dim() && !parts[1].is_error());

        state.error = Some("unclosed group".into());
        let parts = state.status();
        assert_eq!(parts[0].text(), "invalid regex: unclosed group");
        assert!(parts[0].is_error());
        state.hits.clear();
        assert!(
            !texts(&state).iter().any(|t| t.starts_with("no matches")),
            "an invalid query is not a miss"
        );
    }

    #[test]
    fn result_rows_put_a_header_before_each_file() {
        let mut state = SearchFilesState::new(
            QuickOpenScope::Workspace,
            "x".into(),
            SearchOptions::default(),
        );
        assert!(state.result_rows().is_empty());
        state.hits = vec![hit(0, 1), hit(0, 5), hit(3, 2)];
        assert_eq!(
            state.result_rows(),
            vec![
                SearchRow::Header(0),
                SearchRow::Hit(0),
                SearchRow::Hit(1),
                SearchRow::Header(3),
                SearchRow::Hit(2),
            ]
        );
    }

    #[test]
    fn scroll_keeps_the_highlighted_hit_and_its_header_in_view() {
        let mut state = SearchFilesState::new(
            QuickOpenScope::Workspace,
            "x".into(),
            SearchOptions::default(),
        );
        // Files of 3 hits each: rows H h h h H h h h …
        state.hits = (0..12).map(|i| hit(i / 3, i as u32 + 1)).collect();
        let rows = state.result_rows();
        assert_eq!(rows.len(), 16);

        state.cursor = 2;
        state.scroll_to_cursor(&rows, 5);
        assert_eq!(state.scroll, 0, "already in view");

        state.cursor = 7; // row 10: the last hit of the third file
        state.scroll_to_cursor(&rows, 5);
        assert_eq!(state.scroll, 6, "the hit is the bottom row");

        state.cursor = 3; // row 5, right under its header (row 4)
        state.scroll_to_cursor(&rows, 5);
        assert_eq!(state.scroll, 4, "scrolling up shows the header too");

        state.cursor = 11;
        state.scroll_to_cursor(&rows, 40);
        assert_eq!(state.scroll, 0, "everything fits: no scroll");

        state.scroll = 14;
        state.cursor = 0;
        state.hits.truncate(1);
        let rows = state.result_rows();
        state.scroll_to_cursor(&rows, 5);
        assert_eq!(state.scroll, 0, "a shorter list clamps the scroll");

        state.hits = (0..12).map(|i| hit(i / 3, i as u32 + 1)).collect();
        let rows = state.result_rows();
        state.scroll = 0;
        state.cursor = 3;
        state.scroll_to_cursor(&rows, 1);
        assert_eq!(state.scroll, 5, "one row: the hit wins over its header");
    }

    #[test]
    fn footer_hints_follow_the_zone_and_scope() {
        let mut state = SearchFilesState::new(
            QuickOpenScope::Checkout("apps/web".into()),
            String::new(),
            SearchOptions::default(),
        );
        assert_eq!(
            state.footer_hints(),
            "↑↓ move · Enter open · Tab all repos · Alt-c/w/r · Esc close"
        );
        state.zone = SearchZone::Results;
        assert_eq!(
            state.footer_hints(),
            "j/k move · Enter open · e edit · Tab all repos · Alt-c/w/r · Esc close"
        );
        state.widened = true;
        assert!(state.footer_hints().contains("Tab web ·"));
        state.focused = QuickOpenScope::Workspace;
        state.widened = false;
        assert!(
            !state.footer_hints().contains("Tab"),
            "Tab has nothing to do"
        );
    }

    #[test]
    fn status_row_shows_only_the_dialogs_own_status() {
        assert!(search_row_shows_status(NO_SEARCH_MATCHES));
        assert!(search_row_shows_status("Press Ctrl-c again to exit"));
        assert!(!search_row_shows_status("Fetched 2 repos"));
        assert!(!search_row_shows_status(""));
    }

    #[test]
    fn empty_query_is_never_a_miss() {
        let mut state = SearchFilesState::new(
            QuickOpenScope::Workspace,
            String::new(),
            SearchOptions::default(),
        );
        state.index = ready();
        assert!(state.status().is_empty());
    }
}

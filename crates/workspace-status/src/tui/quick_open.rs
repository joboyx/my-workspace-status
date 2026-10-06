//! Quick Open overlay state: Ctrl-p / `F` go to a file, `:` / `>` run a command.
//!
//! One query line drives two modes. A query that starts with `>` is the
//! commands mode ([`CommandPaletteState`], substring filter on the rest);
//! any other query fuzzy-ranks the files of the overlay's
//! [`QuickOpenScope`]. The file index and every score run on the blocking
//! pool; this state only holds their latest accepted results and the
//! generations that let apply drop stale ones.

use std::sync::Arc;

use crate::file_index::{FileHit, FileIndex, MAX_INDEX_ENTRIES};

use super::action::QuickOpenEntry;
use super::command_palette::CommandPaletteState;
use super::ctrl_c_exit::is_ctrl_c_exit_prompt;
use super::tabs::checkout_leaf;

/// Files-mode warning when Enter has no hit to act on.
pub const NO_FILE_MATCHES: &str = "no file matches";

/// Whether the files-mode status row paints `status` instead of
/// [`QuickOpenState::file_status_text`].
///
/// Only text Quick Open set itself ([`NO_FILE_MATCHES`]) and the Ctrl-c quit
/// prompt (Quick Open shows it inline) qualify. Any other status is a
/// leftover from outside the overlay and would hide the file count.
pub fn files_row_shows_status(status: &str) -> bool {
    status == NO_FILE_MATCHES || is_ctrl_c_exit_prompt(status)
}

/// Which list the Quick Open query drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuickOpenMode {
    /// Fuzzy file list of the scope.
    Files,
    /// Named commands (query starts with `>`).
    Commands,
}

/// Mode for `query`: [`QuickOpenMode::Commands`] when it starts with `>`.
pub fn quick_open_mode(query: &str) -> QuickOpenMode {
    if query.starts_with('>') {
        QuickOpenMode::Commands
    } else {
        QuickOpenMode::Files
    }
}

/// Commands-mode filter: the query after the `>`, leading spaces trimmed.
///
/// A query without the `>` is returned with leading spaces trimmed.
pub fn commands_filter(query: &str) -> &str {
    query.strip_prefix('>').unwrap_or(query).trim_start()
}

/// Checkouts the files mode lists.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum QuickOpenScope {
    /// Every primary checkout of the visible workspace snapshot.
    Workspace,
    /// One checkout (snapshot `repo` path).
    Checkout(String),
}

impl QuickOpenScope {
    /// Dialog title in files mode: `Go to file · all repos` or
    /// `Go to file · <checkout leaf>`.
    pub fn title(&self) -> String {
        match self {
            Self::Workspace => "Go to file · all repos".to_string(),
            Self::Checkout(path) => format!("Go to file · {}", checkout_leaf(path)),
        }
    }
}

/// Load state of the scope's file index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileIndexState {
    /// No index requested yet (the overlay opened in commands mode).
    NotLoaded,
    /// A `LoadFileIndex` job is queued or running.
    Loading,
    /// Listed files, shared with score jobs.
    Ready(Arc<FileIndex>),
    /// Every root failed to list; the text joins the per-root errors.
    Failed(String),
}

/// Open Quick Open overlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QuickOpenState {
    /// Raw query line, including a leading `>` in commands mode.
    pub query: String,
    /// Checkouts the files mode lists, fixed when the overlay opens.
    pub scope: QuickOpenScope,
    /// Commands-mode list (filter follows [`commands_filter`]).
    pub commands: CommandPaletteState,
    /// File index for [`Self::scope`].
    pub index: FileIndexState,
    /// Generation of the latest requested index load.
    pub index_gen: u64,
    /// Generation of the latest requested score.
    pub score_gen: u64,
    /// True while the latest requested score has not come back.
    pub score_pending: bool,
    /// Ranked hits of the latest accepted score, best first.
    pub hits: Vec<FileHit>,
    /// Highlight index into [`Self::hits`].
    pub file_cursor: usize,
    /// Files-mode Enter came while the index or the latest score was still
    /// pending: submit once that score is accepted. Any edit or highlight
    /// move clears it.
    pub submit_on_ready: bool,
}

impl QuickOpenState {
    /// Fresh overlay: empty query for [`QuickOpenEntry::Files`], `">"` for
    /// [`QuickOpenEntry::Commands`]. No index yet.
    pub fn new(entry: QuickOpenEntry, scope: QuickOpenScope) -> Self {
        let query = match entry {
            QuickOpenEntry::Files => String::new(),
            QuickOpenEntry::Commands => ">".to_string(),
        };
        Self {
            query,
            scope,
            commands: CommandPaletteState::new(),
            index: FileIndexState::NotLoaded,
            index_gen: 0,
            score_gen: 0,
            score_pending: false,
            hits: Vec::new(),
            file_cursor: 0,
            submit_on_ready: false,
        }
    }

    /// Mode the current query selects.
    pub fn mode(&self) -> QuickOpenMode {
        quick_open_mode(&self.query)
    }

    /// Highlighted file hit, if any.
    pub fn selected_hit(&self) -> Option<&FileHit> {
        self.hits.get(self.file_cursor)
    }

    /// Move the file highlight by `delta`, clamped to the hits.
    pub fn move_file_cursor(&mut self, delta: i32) {
        let len = self.hits.len();
        if len == 0 {
            self.file_cursor = 0;
            return;
        }
        let next = self.file_cursor as i32 + delta;
        self.file_cursor = next.clamp(0, len as i32 - 1) as usize;
    }

    /// Files-mode status row text.
    ///
    /// `indexing…` until the index is ready, then the file count
    /// (`first N files` when the listing hit the cap), or `no file matches` once a
    /// non-blank query's score came back empty. A failed index shows its
    /// error text.
    pub fn file_status_text(&self) -> String {
        match &self.index {
            FileIndexState::NotLoaded | FileIndexState::Loading => "indexing…".to_string(),
            FileIndexState::Failed(err) => err.clone(),
            FileIndexState::Ready(index) => {
                if !self.query.trim().is_empty() && !self.score_pending && self.hits.is_empty() {
                    NO_FILE_MATCHES.to_string()
                } else if index.truncated {
                    format!("first {MAX_INDEX_ENTRIES} files")
                } else if index.entries.len() == 1 {
                    "1 file".to_string()
                } else {
                    format!("{} files", index.entries.len())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_index::{FileEntry, IndexRoot};

    fn ready(paths: &[&str], truncated: bool) -> FileIndexState {
        FileIndexState::Ready(Arc::new(FileIndex {
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
            truncated,
            errors: Vec::new(),
        }))
    }

    fn hit(entry: usize) -> FileHit {
        FileHit {
            entry,
            score: 1,
            indices: Vec::new(),
        }
    }

    #[test]
    fn gt_prefix_selects_commands_and_filter_trims() {
        assert_eq!(quick_open_mode(""), QuickOpenMode::Files);
        assert_eq!(quick_open_mode("src/>"), QuickOpenMode::Files);
        assert_eq!(quick_open_mode(">"), QuickOpenMode::Commands);
        assert_eq!(commands_filter(">  pull "), "pull ");
        assert_eq!(commands_filter(">"), "");
    }

    #[test]
    fn scope_titles_name_all_repos_or_the_checkout_leaf() {
        assert_eq!(QuickOpenScope::Workspace.title(), "Go to file · all repos");
        assert_eq!(
            QuickOpenScope::Checkout("apps/web".into()).title(),
            "Go to file · web"
        );
    }

    #[test]
    fn new_pretypes_gt_for_commands_entry() {
        let files = QuickOpenState::new(QuickOpenEntry::Files, QuickOpenScope::Workspace);
        assert_eq!(files.query, "");
        assert_eq!(files.mode(), QuickOpenMode::Files);
        let commands = QuickOpenState::new(QuickOpenEntry::Commands, QuickOpenScope::Workspace);
        assert_eq!(commands.query, ">");
        assert_eq!(commands.mode(), QuickOpenMode::Commands);
    }

    #[test]
    fn file_cursor_clamps_to_hits() {
        let mut state = QuickOpenState::new(QuickOpenEntry::Files, QuickOpenScope::Workspace);
        state.move_file_cursor(3);
        assert_eq!(state.file_cursor, 0);
        state.hits = vec![hit(0), hit(1), hit(2)];
        state.move_file_cursor(10);
        assert_eq!(state.selected_hit().map(|h| h.entry), Some(2));
        state.move_file_cursor(-10);
        assert_eq!(state.selected_hit().map(|h| h.entry), Some(0));
    }

    #[test]
    fn files_row_shows_only_quick_open_status() {
        assert!(files_row_shows_status(NO_FILE_MATCHES));
        assert!(files_row_shows_status("Press Ctrl-c again to exit"));
        assert!(!files_row_shows_status("Fetched 2 repos"));
        assert!(!files_row_shows_status(""));
    }

    #[test]
    fn file_status_text_follows_index_and_score() {
        let mut state = QuickOpenState::new(QuickOpenEntry::Files, QuickOpenScope::Workspace);
        assert_eq!(state.file_status_text(), "indexing…");
        state.index = FileIndexState::Loading;
        assert_eq!(state.file_status_text(), "indexing…");
        state.index = ready(&["a.rs", "b.rs"], false);
        assert_eq!(state.file_status_text(), "2 files");
        state.query = "zzz".into();
        state.score_pending = true;
        assert_eq!(state.file_status_text(), "2 files");
        state.score_pending = false;
        assert_eq!(state.file_status_text(), "no file matches");
        state.hits = vec![hit(0)];
        assert_eq!(state.file_status_text(), "2 files");
        state.index = ready(&["a.rs"], false);
        assert_eq!(state.file_status_text(), "1 file");
        state.index = ready(&["a.rs"], true);
        assert_eq!(
            state.file_status_text(),
            format!("first {MAX_INDEX_ENTRIES} files")
        );
        state.index = FileIndexState::Failed("app: not a git repository".into());
        assert_eq!(state.file_status_text(), "app: not a git repository");
    }
}

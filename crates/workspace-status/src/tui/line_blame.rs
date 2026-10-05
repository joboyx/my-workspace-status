//! Current-line blame: session state, cache keys, and annotation text.
//!
//! [`crate::git::blame_line`] reads who last changed a line; this module
//! keeps those answers per [`BlameKey`] and turns one into the short
//! end-of-line text the diff and file panes paint:
//! `{author}, {age} · {sha7} · {subject}`.

use std::collections::HashMap;

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::action::Action;
use crate::git::{BlameRev, LineBlame};

/// Narrowest annotation worth painting. Below this the pane paints none.
const MIN_ANNOTATION_COLS: usize = 12;

/// Cached answers kept before the cache starts over.
const CACHE_CAP: usize = 512;

/// Annotation for an added line in the UNSTAGED section (no git call).
pub const UNCOMMITTED_TEXT: &str = "You · uncommitted";

/// Annotation for an added line in the STAGED section (no git call).
pub const STAGED_TEXT: &str = "You · staged";

/// Blame action refusal: the annotation is off.
pub const BLAME_IS_OFF: &str = "line blame is off (B)";

/// Blame action refusal: no file-tab or file-diff line has focus.
pub const FOCUS_A_DIFF_OR_FILE_LINE: &str = "focus a diff or file line";

/// Blame action refusal: git has not answered for the focused line yet.
pub const BLAME_STILL_LOADING: &str = "line blame still loading";

/// Blame action refusal: the focused line is staged or in the working
/// tree only.
pub const LINE_NOT_COMMITTED: &str = "line is not committed yet";

/// Blame action refusal: git has no blame for the focused row (untracked
/// file, binary, hunk header, failed run).
pub const NO_BLAME_FOR_LINE: &str = "no blame for this line";

/// One row of the `A` blame-actions menu: the key that picks it, its
/// label, and the blame action it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlameMenuRow {
    /// Key that picks the row inside the menu.
    pub key: char,
    /// Label painted after the key chip (the palette title without
    /// `Blame: `).
    pub label: &'static str,
    /// Blame action the row dispatches.
    pub action: Action,
}

/// Rows of the `A` blame-actions menu, in paint order. Enter runs the
/// first row.
pub const BLAME_MENU_ROWS: [BlameMenuRow; 4] = [
    BlameMenuRow {
        key: 'c',
        label: "open commit changes",
        action: Action::BlameCommitVsParent,
    },
    BlameMenuRow {
        key: 'p',
        label: "open previous line change",
        action: Action::BlamePreviousChange,
    },
    BlameMenuRow {
        key: 'w',
        label: "diff commit to working tree",
        action: Action::BlameCommitVsWorktree,
    },
    BlameMenuRow {
        key: 'g',
        label: "show commit in graph",
        action: Action::BlameRevealGraph,
    },
];

/// Older graph pages a reveal may load before it gives up.
pub const GRAPH_REVEAL_MAX_PAGES: u8 = 10;

/// Pending "show commit in graph": select [`Self::sha`] once the graph of
/// [`Self::repo`] holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphReveal {
    /// Checkout whose graph must show the commit.
    pub repo: String,
    /// Full commit id to select.
    pub sha: String,
    /// Older pages loaded so far for this reveal.
    pub pages: u8,
    /// A graph of [`Self::repo`] loaded, or was already shown with no
    /// load pending, after the reveal started. Until then the graph on
    /// screen may be stale, so the reveal neither widens nor gives up.
    pub seen_load: bool,
}

/// First seven characters of a commit id.
pub fn short_sha(sha: &str) -> String {
    sha.chars().take(7).collect()
}

/// One `git blame -L n,n` question: checkout, revision, path, line, and
/// the content epoch it was asked under.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BlameKey {
    /// Checkout path (same string as snapshot `repo`).
    pub repo: String,
    /// Revision blamed: the working tree, the index, or a commit.
    pub rev: BlameRev,
    /// Path relative to [`Self::repo`] in that revision.
    pub path: String,
    /// 1-based line in that revision's file.
    pub line: u32,
    /// Content version the line number belongs to: the diff fingerprint
    /// for worktree and stash diffs, the load generation for a file tab,
    /// `0` for a fixed commit. A reload with new text asks again.
    pub epoch: u64,
}

/// Which side of a split diff row carries the annotation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlameSide {
    /// The old (left) cell: a deleted line.
    Old,
    /// The new (right) cell: an added or context line.
    New,
}

/// What the focused line shows at the end of its row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LineAnnotation {
    /// Blame is on its way. Nothing is painted.
    Loading,
    /// Text to paint (before [`fit_annotation`]).
    Text(String),
}

/// Session line-blame state: the `B` toggle and the answer cache.
///
/// The cache holds `None` for a line git has no blame for (untracked
/// file, no `HEAD`, failed run), so the pane does not ask again.
#[derive(Clone, Debug)]
pub struct LineBlameState {
    /// Annotation on. `viewDefaults.lineBlame` sets the launch value and
    /// `B` flips it for this session.
    pub enabled: bool,
    cache: HashMap<BlameKey, Option<LineBlame>>,
}

impl Default for LineBlameState {
    fn default() -> Self {
        Self {
            enabled: true,
            cache: HashMap::new(),
        }
    }
}

impl LineBlameState {
    /// Turn the annotation on or off. Off drops every cached answer.
    pub fn set_enabled(&mut self, on: bool) {
        self.enabled = on;
        if !on {
            self.cache.clear();
        }
    }

    /// Cached answer for `key`: `None` when not asked yet, `Some(None)`
    /// when git had no blame for it.
    pub fn cached(&self, key: &BlameKey) -> Option<Option<&LineBlame>> {
        self.cache.get(key).map(Option::as_ref)
    }

    /// Store the answer for `key`. A full cache starts over first.
    pub fn insert(&mut self, key: BlameKey, blame: Option<LineBlame>) {
        if self.cache.len() >= CACHE_CAP && !self.cache.contains_key(&key) {
            self.cache.clear();
        }
        self.cache.insert(key, blame);
    }

    /// Cached answers held now.
    #[cfg(test)]
    pub fn cached_len(&self) -> usize {
        self.cache.len()
    }
}

/// Short age of `then_unix` at `now_unix`: `just now`, `5m ago`, `3h ago`,
/// `3d ago`, `2mo ago` (30 days and up), `1y ago` (365 days and up).
///
/// A time in the future reads `just now`.
pub fn relative_age(now_unix: i64, then_unix: i64) -> String {
    let secs = now_unix.saturating_sub(then_unix);
    let days = secs / 86_400;
    if secs < 60 {
        "just now".into()
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if days < 1 {
        format!("{}h ago", secs / 3_600)
    } else if days < 30 {
        format!("{days}d ago")
    } else if days < 365 {
        format!("{}mo ago", days / 30)
    } else {
        format!("{}y ago", days / 365)
    }
}

/// Annotation for one blamed line at `now_unix`.
///
/// `{author}, {age} · {sha7} · {summary}`, or `You · uncommitted` when
/// the line is not committed yet.
pub fn annotation_text(blame: &LineBlame, now_unix: i64) -> String {
    if blame.uncommitted {
        return UNCOMMITTED_TEXT.into();
    }
    let sha7 = short_sha(&blame.sha);
    format!(
        "{}, {} · {sha7} · {}",
        blame.author,
        relative_age(now_unix, blame.author_time),
        blame.summary
    )
}

/// `text` cut to `max_cols` display columns, ending in `…` when cut.
///
/// The subject is last, so a cut shortens it first. `None` when fewer
/// than 12 columns are free.
pub fn fit_annotation(text: &str, max_cols: usize) -> Option<String> {
    if max_cols < MIN_ANNOTATION_COLS {
        return None;
    }
    if text.width() <= max_cols {
        return Some(text.to_string());
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > max_cols - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn committed(author_time: i64, summary: &str) -> LineBlame {
        LineBlame {
            sha: "0123456789abcdef0123456789abcdef01234567".into(),
            author: "Ada".into(),
            author_time,
            summary: summary.into(),
            orig_line: 3,
            filename: "src/lib.rs".into(),
            previous: None,
            boundary: false,
            uncommitted: false,
        }
    }

    #[test]
    fn relative_age_steps_through_each_unit() {
        let ago = |secs: i64| relative_age(NOW, NOW - secs);
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(-3_600), "just now", "future time");
        assert_eq!(ago(5 * 60), "5m ago");
        assert_eq!(ago(3 * 3_600), "3h ago");
        assert_eq!(ago(3 * 86_400), "3d ago");
        assert_eq!(ago(29 * 86_400), "29d ago");
        assert_eq!(ago(30 * 86_400), "1mo ago");
        assert_eq!(ago(64 * 86_400), "2mo ago");
        assert_eq!(ago(365 * 86_400), "1y ago");
        assert_eq!(ago(800 * 86_400), "2y ago");
    }

    #[test]
    fn annotation_text_joins_author_age_sha_and_subject() {
        let blame = committed(NOW - 2 * 86_400, "fix the parser");
        assert_eq!(
            annotation_text(&blame, NOW),
            "Ada, 2d ago · 0123456 · fix the parser"
        );
        let uncommitted = LineBlame {
            sha: "0".repeat(40),
            uncommitted: true,
            ..blame
        };
        assert_eq!(annotation_text(&uncommitted, NOW), "You · uncommitted");
    }

    #[test]
    fn fit_annotation_cuts_by_display_width() {
        let text = "Ada, 2d ago · 0123456 · fix the parser";
        assert_eq!(fit_annotation(text, 80).as_deref(), Some(text));
        assert_eq!(
            fit_annotation(text, 30).as_deref(),
            Some("Ada, 2d ago · 0123456 · fix t…")
        );
        // Wide glyphs count two columns; the cut never splits one.
        let wide = "Ada, 2d ago · 0123456 · 修正する";
        let cut = fit_annotation(wide, 28).expect("fits");
        assert_eq!(cut, "Ada, 2d ago · 0123456 · 修…");
        assert!(cut.width() <= 28);
    }

    fn key(line: u32) -> BlameKey {
        BlameKey {
            repo: "app".into(),
            rev: BlameRev::Worktree,
            path: "src/lib.rs".into(),
            line,
            epoch: 0,
        }
    }

    #[test]
    fn cache_holds_answers_until_off_or_full() {
        let mut state = LineBlameState::default();
        assert!(state.enabled, "on by default");
        assert_eq!(state.cached(&key(1)), None);
        state.insert(key(1), Some(committed(NOW, "one")));
        state.insert(key(2), None);
        assert_eq!(
            state.cached(&key(1)).flatten().map(|b| b.summary.as_str()),
            Some("one")
        );
        assert_eq!(state.cached(&key(2)), Some(None), "no blame is cached too");
        state.set_enabled(false);
        assert_eq!(state.cached_len(), 0, "off drops the cache");
        state.set_enabled(true);
        for line in 1..=CACHE_CAP as u32 {
            state.insert(key(line), None);
        }
        assert_eq!(state.cached_len(), CACHE_CAP);
        state.insert(key(1), None);
        assert_eq!(state.cached_len(), CACHE_CAP, "a known key does not reset");
        state.insert(key(9_999), None);
        assert_eq!(state.cached_len(), 1, "a full cache starts over");
    }

    #[test]
    fn fit_annotation_is_none_below_twelve_columns() {
        assert_eq!(fit_annotation("You · uncommitted", 11), None);
        assert_eq!(
            fit_annotation("You · uncommitted", 12).as_deref(),
            Some("You · uncom…")
        );
    }
}

//! GraphPane chrome: header / list / 2-line selection footer / loading older.
//!
//! Matches Ink `graphChromeBudget` and `graphSelectionDetailLines`.

use crate::format::{format_commit_ref_chips, format_relative_date, short_id};
use crate::glyphs::GlyphSet;
use crate::model::{GraphModel, GraphRow};

/// Status / pane copy while the next log page loads.
pub const LOADING_OLDER: &str = "loading older…";

/// How many header / footer / list rows GraphPane should reserve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphChromeBudget {
    /// Paint the sync header.
    pub header: bool,
    /// Paint the 2-line selection footer.
    pub footer: bool,
    /// Rows left for the commit list (at least 1).
    pub list_height: u16,
    /// Extra row for [`LOADING_OLDER`].
    pub older: bool,
}

/// Footer first, then header, matching Ink `graphChromeBudget`.
pub fn graph_chrome_budget(
    height: u16,
    loading_older: bool,
    want_header: bool,
) -> GraphChromeBudget {
    let older = loading_older;
    let mut avail = height.saturating_sub(u16::from(older)).max(1);
    let footer = avail >= 3;
    if footer {
        avail = avail.saturating_sub(2);
    }
    let header = want_header && avail >= 2;
    if header {
        avail = avail.saturating_sub(1);
    }
    GraphChromeBudget {
        header,
        footer,
        list_height: avail.max(1),
        older,
    }
}

/// Two selection-detail lines (subject + meta). Truncated to `width`.
pub fn selection_detail_lines(
    model: &GraphModel,
    row: Option<&GraphRow>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
) -> [String; 2] {
    let width = width.max(1);
    let Some(row) = row else {
        return [trunc("no selection", width), String::new()];
    };
    match row {
        GraphRow::Uncommitted { has_changes } => {
            let line = if *has_changes {
                "Uncommitted changes"
            } else {
                "Working tree clean"
            };
            [trunc(line, width), trunc("worktree · not a commit", width)]
        }
        GraphRow::Stash(stash) => {
            let meta = stash.stash_ref.clone();
            [trunc(&stash.subject, width), trunc(&meta, width)]
        }
        GraphRow::Worktree(wt) => {
            let meta = wt.branch.clone().unwrap_or_default();
            [trunc(&wt.path, width), trunc(&meta, width)]
        }
        GraphRow::Commit {
            commit, is_head, ..
        } => {
            let chips = format_commit_ref_chips(
                &commit.refs,
                *is_head,
                model.sync.as_ref().map(|s| s.branch.as_str()),
                glyphs,
            );
            let hash = short_id(&commit.id);
            let mut meta_parts: Vec<String> = Vec::new();
            if chips.is_empty() {
                meta_parts.push("(no refs)".into());
            } else {
                meta_parts.push(chips);
            }
            meta_parts.push(hash.to_string());
            if !commit.author_name.is_empty() {
                meta_parts.push(commit.author_name.clone());
            }
            if commit.author_date_unix > 0 {
                meta_parts.push(format_relative_date(commit.author_date_unix, now_unix));
            }
            [
                trunc(&commit.subject, width),
                trunc(&meta_parts.join(" · "), width),
            ]
        }
    }
}

fn trunc(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".into();
    }
    let keep: String = text.chars().take(max - 1).collect();
    format!("{keep}…")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::UNICODE;
    use crate::model::Commit;

    #[test]
    fn budget_prefers_footer_over_header() {
        let chrome = graph_chrome_budget(3, false, true);
        assert!(chrome.footer);
        assert!(!chrome.header);
        assert_eq!(chrome.list_height, 1);
    }

    #[test]
    fn budget_keeps_header_when_tall() {
        let chrome = graph_chrome_budget(16, false, true);
        assert!(chrome.header);
        assert!(chrome.footer);
        assert_eq!(chrome.list_height, 13);
    }

    #[test]
    fn budget_reserves_loading_older() {
        let chrome = graph_chrome_budget(16, true, true);
        assert!(chrome.older);
        assert_eq!(chrome.list_height, 12);
    }

    #[test]
    fn footer_uncommitted_clean_and_dirty() {
        let model = GraphModel {
            uncommitted: Some(true),
            ..GraphModel::default()
        };
        let dirty = GraphRow::Uncommitted { has_changes: true };
        let [a, b] = selection_detail_lines(&model, Some(&dirty), &UNICODE, 40, 0);
        assert_eq!(a, "Uncommitted changes");
        assert_eq!(b, "worktree · not a commit");
        let clean = GraphRow::Uncommitted { has_changes: false };
        let [c, _] = selection_detail_lines(&model, Some(&clean), &UNICODE, 40, 0);
        assert_eq!(c, "Working tree clean");
    }

    #[test]
    fn footer_commit_subject_and_meta() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "add footer".into(),
            parents: Vec::new(),
            refs: vec!["main".into()],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            head_id: Some(commit.id.clone()),
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: true,
            worktrees: Vec::new(),
        };
        let [subject, meta] =
            selection_detail_lines(&model, Some(&row), &UNICODE, 80, 1_700_000_000);
        assert_eq!(subject, "add footer");
        assert!(meta.contains("abcdefg"), "{meta}");
        assert!(meta.contains("Ada"), "{meta}");
        assert!(meta.contains("2m") || meta.contains("just now"), "{meta}");
    }
}

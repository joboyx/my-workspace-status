//! GraphPane chrome: header / list / 2-line selection footer / loading older.
//!
//! Matches Ink `graphChromeBudget` and `graphSelectionDetailLines`.

use crate::format::{format_commit_ref_chips, format_commit_ref_chips_with, format_relative_date, short_id};
use crate::glyphs::GlyphSet;
use crate::model::{GraphModel, GraphRow};

/// Status / pane copy while the next log page loads.
pub const LOADING_OLDER: &str = "loading older…";

/// Ink `graphSelectionDetailLines` when no row is focused.
pub const FOOTER_NO_SELECTION: &str = "no selection";

/// Ink uncommitted footer meta.
pub const FOOTER_WORKTREE_NOT_A_COMMIT: &str = "worktree · not a commit";

/// Ink spacer footer meta (`kind: 'spacer'`).
pub const FOOTER_CONNECTOR_NOT_SELECTABLE: &str = "connector · not selectable";

/// Ink commit footer when the commit has no ref chips.
pub const FOOTER_NO_REFS: &str = "(no refs)";

/// Ink spacer footer subject.
pub const FOOTER_SPACER_SUBJECT: &str = "…";

/// What GraphPane's 2-line selection footer describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphFooterSelection<'a> {
    /// No focused list row.
    None,
    /// A [`GraphRow`] from [`GraphModel::visible_rows`].
    Row(&'a GraphRow),
    /// Spacer under a commit or stash (not in `visible_rows`).
    Spacer,
}

impl<'a> From<Option<&'a GraphRow>> for GraphFooterSelection<'a> {
    fn from(row: Option<&'a GraphRow>) -> Self {
        match row {
            Some(row) => Self::Row(row),
            None => Self::None,
        }
    }
}

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
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
) -> [String; 2] {
    let width = width.max(1);
    match selection {
        GraphFooterSelection::None => [trunc(FOOTER_NO_SELECTION, width), String::new()],
        GraphFooterSelection::Spacer => [
            trunc(FOOTER_SPACER_SUBJECT, width),
            trunc(FOOTER_CONNECTOR_NOT_SELECTABLE, width),
        ],
        GraphFooterSelection::Row(GraphRow::Uncommitted { has_changes }) => {
            let line = if *has_changes {
                "Uncommitted changes"
            } else {
                "Working tree clean"
            };
            let meta = head_commit_ref_line(model, glyphs)
                .unwrap_or_else(|| FOOTER_WORKTREE_NOT_A_COMMIT.to_string());
            [trunc(line, width), trunc(&meta, width)]
        }
        GraphFooterSelection::Row(GraphRow::Stash(stash)) => {
            // Ink `graphSelectionDetailLines`: `[ref, hash.slice(0,7), date].join(' · ')`.
            let meta = join_meta([
                stash.stash_ref.clone(),
                short_id(&stash.id).to_string(),
                format_relative_date(stash.author_date_unix, now_unix),
            ]);
            [trunc(&stash.subject, width), trunc(&meta, width)]
        }
        GraphFooterSelection::Row(GraphRow::Worktree(wt)) => {
            let meta = wt.branch.clone().unwrap_or_default();
            [trunc(&wt.path, width), trunc(&meta, width)]
        }
        GraphFooterSelection::Row(GraphRow::Commit {
            commit, is_head, ..
        }) => {
            let chips = format_commit_ref_chips_with(
                &commit.refs,
                *is_head,
                model.sync.as_ref().map(|s| s.branch.as_str()),
                glyphs,
                model.default_branch_override.as_deref(),
            );
            let hash = short_id(&commit.id);
            let mut meta_parts: Vec<String> = Vec::new();
            if chips.is_empty() {
                meta_parts.push(FOOTER_NO_REFS.into());
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

fn head_commit_ref_line(model: &GraphModel, glyphs: &GlyphSet) -> Option<String> {
    let id = model.head_id.as_deref()?;
    let commit = model.commits.iter().find(|c| c.id == id)?;
    let chips = format_commit_ref_chips_with(
        &commit.refs,
        true,
        model.sync.as_ref().map(|s| s.branch.as_str()),
        glyphs,
        model.default_branch_override.as_deref(),
    );
    if chips.is_empty() {
        None
    } else {
        Some(chips)
    }
}

fn join_meta(parts: impl IntoIterator<Item = String>) -> String {
    parts
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" · ")
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
    use crate::model::{Commit, GraphRef, Stash};

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
        let [a, b] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&dirty), &UNICODE, 40, 0);
        assert_eq!(a, "Uncommitted changes");
        assert_eq!(b, FOOTER_WORKTREE_NOT_A_COMMIT);
        let clean = GraphRow::Uncommitted { has_changes: false };
        let [c, d] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&clean), &UNICODE, 40, 0);
        assert_eq!(c, "Working tree clean");
        assert_eq!(d, FOOTER_WORKTREE_NOT_A_COMMIT);
    }

    #[test]
    fn footer_uncommitted_lists_head_commit_refs() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "tip".into(),
            parents: Vec::new(),
            refs: vec![GraphRef::local("main"), GraphRef::tag("v1")],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000,
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            head_id: Some(commit.id.clone()),
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Uncommitted { has_changes: false };
        let [subject, meta] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&row), &UNICODE, 80, 0);
        assert_eq!(subject, "Working tree clean");
        assert!(meta.contains("main"), "{meta}");
        assert!(meta.contains("v1"), "{meta}");
        assert_ne!(meta, FOOTER_WORKTREE_NOT_A_COMMIT);
    }

    #[test]
    fn footer_no_selection_connector_and_no_refs() {
        let model = GraphModel::default();
        let [none, empty] =
            selection_detail_lines(&model, GraphFooterSelection::None, &UNICODE, 40, 0);
        assert_eq!(none, FOOTER_NO_SELECTION);
        assert_eq!(empty, "");
        let [dots, connector] =
            selection_detail_lines(&model, GraphFooterSelection::Spacer, &UNICODE, 40, 0);
        assert_eq!(dots, FOOTER_SPACER_SUBJECT);
        assert_eq!(connector, FOOTER_CONNECTOR_NOT_SELECTABLE);
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "untagged".into(),
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let [_, meta] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&row), &UNICODE, 80, 0);
        assert!(
            meta.starts_with(FOOTER_NO_REFS),
            "commit with no chips uses Ink (no refs): {meta}"
        );
    }

    #[test]
    fn footer_stash_ref_hash_date_without_author() {
        let stash = Stash {
            id: "abcdefghhhh".into(),
            stash_ref: "stash@{0}".into(),
            subject: "WIP on main".into(),
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            parent_id: None,
        };
        let model = GraphModel {
            stashes: vec![stash.clone()],
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let [subject, meta] = selection_detail_lines(
            &model,
            GraphFooterSelection::Row(&GraphRow::Stash(stash)),
            &UNICODE,
            80,
            1_700_000_000,
        );
        assert_eq!(subject, "WIP on main");
        assert_eq!(meta, "stash@{0} · abcdefg · 2m");
        assert!(!meta.contains("Ada"), "{meta}");
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
        let [subject, meta] = selection_detail_lines(
            &model,
            GraphFooterSelection::Row(&row),
            &UNICODE,
            80,
            1_700_000_000,
        );
        assert_eq!(subject, "add footer");
        assert!(meta.contains("abcdefg"), "{meta}");
        assert!(meta.contains("Ada"), "{meta}");
        assert!(meta.contains("2m") || meta.contains("just now"), "{meta}");
    }
}

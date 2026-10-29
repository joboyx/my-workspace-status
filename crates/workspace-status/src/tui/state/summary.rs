//! Folder summary rows, scroll, and pan: the right pane beside a focused
//! commit-file folder (a depth-2 drill or a compare tab).
//!
//! The summary paints the folder's files as flat commit-file rows through
//! [`AppState::commit_file_row_paint_segments_in`], so its devicons,
//! status badges, comment marks, and reviewed eye are the file list's. It
//! has no cursor: the wheel, its vertical bar, and the right-pane move
//! keys move its viewport, and the pan keys and the horizontal wheel pan
//! its own columns ([`SummaryView`]), never the file diff it hides.

use std::collections::HashSet;

use super::super::commit_files::{flatten_commit_files, CommitFileRow, FolderSummary};
use super::super::drill::CommitFileSource;
use super::super::icons::glyph;
use super::super::search::{apply_pan, list_row_pan_max};
use super::super::tree::{NodeSegments, SegRole, TextSeg};
use super::super::watch::commit_file_identity;
use super::popover::CommitFileCommentScope;
use super::AppState;
use crate::helpers::visible_width;

/// Viewport top and pan of the folder summary, and the folder they belong
/// to.
///
/// Reads give 0 for any other folder ([`AppState::summary_scroll`]), and
/// the next write or paint starts that folder at the top-left: another
/// folder's summary starts fresh, and a watch reload of the same folder
/// keeps its place (the paint clamps it to the reloaded rows).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct SummaryView {
    /// Identity of the folder ([`AppState::summary_key`]).
    key: String,
    /// First painted file row.
    scroll: usize,
    /// Columns panned off the left edge.
    col_offset: u16,
}

impl AppState {
    /// True while a folder summary covers the right pane
    /// ([`Self::folder_summary`]).
    pub(crate) fn summary_shown(&self) -> bool {
        self.folder_summary_dir().is_some()
    }

    /// Identity of the folder the summary shows: the list's checkout, its
    /// source, and the folder path. A compare tab counts by its refs, not
    /// their resolved commits, so a watch reload that moves its head keeps
    /// the folder. `None` while no summary shows or its source has not
    /// loaded.
    fn summary_key(&self) -> Option<String> {
        let dir_id = format!("dir:{}", self.folder_summary_dir()?);
        let (repo, source) = self.commit_drill_source()?;
        Some(match source {
            CommitFileSource::Compare {
                base_ref, head_ref, ..
            } => format!("{repo}#compare:{base_ref}...{head_ref}#{dir_id}"),
            _ => commit_file_identity(repo, source, &dir_id),
        })
    }

    /// The stored view when it belongs to the folder the summary shows.
    fn current_summary_view(&self) -> Option<&SummaryView> {
        let view = self.summary_view.as_ref()?;
        (Some(&view.key) == self.summary_key().as_ref()).then_some(view)
    }

    /// The view of the folder the summary shows, started at the top-left
    /// when it was set on another folder. `None` with no summary key.
    fn summary_view_mut(&mut self) -> Option<&mut SummaryView> {
        let key = self.summary_key()?;
        if self
            .summary_view
            .as_ref()
            .is_none_or(|view| view.key != key)
        {
            self.summary_view = Some(SummaryView {
                key,
                ..SummaryView::default()
            });
        }
        self.summary_view.as_mut()
    }

    /// First file row the folder summary paints: 0 for a folder other
    /// than the one it was scrolled on.
    pub(crate) fn summary_scroll(&self) -> usize {
        self.current_summary_view().map_or(0, |view| view.scroll)
    }

    /// Columns the folder summary is panned: 0 for a folder other than the
    /// one it was panned on.
    pub(crate) fn summary_col_offset(&self) -> u16 {
        self.current_summary_view()
            .map_or(0, |view| view.col_offset)
    }

    /// Pan the folder summary to `offset` (already clamped to its pan max).
    pub(crate) fn set_summary_col_offset(&mut self, offset: u16) {
        if let Some(view) = self.summary_view_mut() {
            view.col_offset = offset;
        }
    }

    /// Record the top and pan the folder summary painted with (both
    /// clamped to its rows), for the folder it shows.
    pub(crate) fn keep_summary_view(&mut self, scroll: usize, col_offset: u16) {
        if let Some(view) = self.summary_view_mut() {
            view.scroll = scroll;
            view.col_offset = col_offset;
        }
    }

    /// The folder summary's file rows: one flat commit-file row per file
    /// of `summary` (full path label, the file list's row ids).
    pub(crate) fn summary_rows(&self, summary: &FolderSummary) -> Vec<CommitFileRow> {
        flatten_commit_files(&summary.files, false, &HashSet::new(), self.ascii)
    }

    /// Every segment summary row `row` paints: the file list's segments in
    /// the list's comment `scope` ([`Self::commit_file_row_paint_segments_in`]),
    /// with the file's line counts (`+added −deleted`) before its marks and
    /// badge. A file with no counts paints the list's segments only.
    pub(crate) fn summary_row_paint_segments_in(
        &self,
        row: &CommitFileRow,
        scope: Option<CommitFileCommentScope<'_>>,
    ) -> NodeSegments {
        let mut segs = self.commit_file_row_paint_segments_in(row, scope);
        let Some(stat) = row.file.as_ref().and_then(|file| file.stat) else {
            return segs;
        };
        let minus = glyph(self.ascii, "−", "-");
        let count = |text: String, role: SegRole| TextSeg {
            text,
            role,
            hex: None,
            bold: false,
            dim: false,
            icon: None,
        };
        let mut trailing = vec![
            count(format!("+{}", stat.added), SegRole::Added),
            count(format!(" {minus}{}  ", stat.deleted), SegRole::Deleted),
        ];
        trailing.append(&mut segs.trailing);
        segs.trailing = trailing;
        segs
    }

    /// Pan max of the folder summary in a pane `pane_width` columns wide
    /// whose vertical scrollbar shows (`vbar`) or not: its longest painted
    /// row against the width left of the bar, as the commit-file list
    /// clamps ([`Self::commit_file_max_col_in`]). 0 with no summary.
    pub(crate) fn summary_max_col_in(&self, pane_width: u16, vbar: bool) -> usize {
        let Some(summary) = self.folder_summary() else {
            return 0;
        };
        let width = pane_width.saturating_sub(u16::from(vbar)).max(1) as usize;
        let scope = self.commit_file_comment_scope();
        self.summary_rows(&summary)
            .iter()
            .map(|row| {
                let segs = self.summary_row_paint_segments_in(row, scope);
                let label: usize = segs.segments.iter().map(|s| visible_width(&s.text)).sum();
                let trailing: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
                list_row_pan_max(label, row.depth, trailing, width)
            })
            .max()
            .unwrap_or(0)
    }

    /// File rows of the folder summary; 0 with none.
    fn summary_row_count(&self) -> usize {
        self.folder_summary()
            .map_or(0, |summary| summary.files.len())
    }

    /// Largest summary top: its last row on the last painted row.
    fn summary_scroll_max(&self) -> usize {
        let height = usize::from(self.layout.summary_list_height.max(1));
        self.summary_row_count().saturating_sub(height)
    }

    /// Rows a PageUp / PageDown over the folder summary moves: one short
    /// of the painted rows, as on a file diff.
    pub(crate) fn summary_page_rows(&self) -> usize {
        usize::from(self.layout.summary_list_height.saturating_sub(1).max(1))
    }

    /// Scroll the folder summary so its painted rows start at `top`,
    /// clamped to its last full page.
    pub(crate) fn scroll_summary_to(&mut self, top: usize) {
        let top = top.min(self.summary_scroll_max());
        if let Some(view) = self.summary_view_mut() {
            view.scroll = top;
        }
    }

    /// Scroll the folder summary `delta` rows (a wheel step or a move key).
    pub(crate) fn scroll_summary_by(&mut self, delta: i32) {
        let top = (self.summary_scroll() as i64 + i64::from(delta)).max(0);
        self.scroll_summary_to(usize::try_from(top).unwrap_or(usize::MAX));
    }

    /// Pan the folder summary `delta` columns, clamped to its longest
    /// painted row beside its vertical bar when the rows overflow.
    pub(crate) fn pan_summary(&mut self, delta: i32) {
        let vbar = self.summary_row_count() > usize::from(self.layout.summary_list_height);
        let max = self.summary_max_col_in(self.layout.diff_pane_width, vbar);
        let next = apply_pan(self.summary_col_offset(), delta, max);
        self.set_summary_col_offset(next);
    }
}

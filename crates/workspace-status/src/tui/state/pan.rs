//! Horizontal pan for tree, graph, commit-file, and file-diff panes.

use ratatui::style::Color;
use workspace_status_graph::{graph_col_max, graph_hscroll_visible};

use super::super::action::{Action, Effect};
use super::super::diff::{cell_code_width, diff_row_content_width, gutter_width, DiffRow};
use super::super::gates::ListFocusTarget;
use super::super::icons::comment_mark_cols;
use super::super::search::{apply_pan, list_row_pan_max, max_col_offset};
use super::super::split::diff_paint_width;
use super::super::tree::pr_badge_mark;
use super::{AppState, FocusPane};
use crate::helpers::visible_width;

impl AppState {
    pub(crate) fn pan_focused(&mut self, delta: i32) {
        if self.focus == FocusPane::Left {
            self.pan_left_pane(delta);
        } else {
            self.pan_right_pane(delta);
        }
    }

    fn pan_left_pane(&mut self, delta: i32) {
        // A compare tab's left pane is its own file list; the Workspace
        // `drill` is only parked there.
        if self.is_compare_tab() || self.drill.is_diff() {
            let max = self.commit_file_max_col(self.layout.tree_width);
            self.left_col_offset = apply_pan(self.left_col_offset, delta, max);
        } else if self.drill.is_files() {
            let max = self.graph_max_col(self.layout.tree_width);
            self.left_col_offset = apply_pan(self.left_col_offset, delta, max);
        } else {
            let max = self.tree_max_col();
            self.left_col_offset = apply_pan(self.left_col_offset, delta, max);
        }
    }

    fn pan_right_pane(&mut self, delta: i32) {
        // A folder summary pans its own rows, not the file diff it hides.
        if self.summary_shown() {
            self.pan_summary(delta);
        } else if self.right_is_diff() || self.drill.is_diff() {
            self.pan_diff_content(delta);
        } else if self.drill.is_files() {
            let max = self.commit_file_max_col(self.layout.diff_pane_width);
            self.right_col_offset = apply_pan(self.right_col_offset, delta, max);
        } else {
            let max = self.graph_max_col(self.layout.diff_pane_width);
            self.right_col_offset = apply_pan(self.right_col_offset, delta, max);
        }
    }

    pub(crate) fn graph_col_offset(&self) -> u16 {
        if self.drill.is_files() {
            self.left_col_offset
        } else {
            self.right_col_offset
        }
    }

    pub(crate) fn set_graph_col_offset(&mut self, offset: u16) {
        if self.drill.is_files() {
            self.left_col_offset = offset;
        } else {
            self.right_col_offset = offset;
        }
    }

    /// Mouse hscroll: pan the pane under the pointer without moving the
    /// focused row. The workspace tree matches the right pane — scroll does
    /// not steal the cursor. Graph / diff under the pointer pan even when
    /// the other pane holds keyboard focus. When a file diff has long lines,
    /// trackpad hscroll over the left pane pans that diff (the line the
    /// operator is reading) instead of a short tree label. Click still
    /// selects.
    pub(crate) fn mouse_pan(&mut self, col: u16, delta: i32) {
        if col >= self.layout.right_x {
            self.pan_right_pane(delta);
        } else {
            // One row build decides and clamps the diff pan.
            let max = self.shown_diff_pan_max();
            if max > 0 {
                self.diff_col_offset = apply_pan(self.diff_col_offset, delta, max);
            } else {
                self.pan_left_pane(delta);
            }
        }
    }

    /// Pan max of the Workspace tree: its longest row against the pane
    /// width, one column short while the vertical scrollbar shows (the
    /// rows overflow [`Self::tree_list_rows`]), so the pan still reaches
    /// the end of that row beside the bar.
    fn tree_max_col(&self) -> usize {
        let vbar = self.painted_tree_rows().len() > usize::from(self.tree_list_rows());
        self.tree_max_col_in(self.layout.tree_width, vbar)
    }

    /// [`Self::tree_max_col`] for a pane `pane_width` columns wide whose
    /// vertical scrollbar shows (`vbar`) or not: what the paint clamps its
    /// horizontal bar to.
    pub(crate) fn tree_max_col_in(&self, pane_width: u16, vbar: bool) -> usize {
        let width = pane_width.saturating_sub(u16::from(vbar)).max(1) as usize;
        self.rows
            .iter()
            .map(|row| {
                // The painted marks and PR badge widen the trailing run.
                let segs = self.tree_row_paint_segments_in(row, width);
                let label: usize = segs.segments.iter().map(|s| visible_width(&s.text)).sum();
                let trailing: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
                list_row_pan_max(label, row.depth, trailing, width)
            })
            .max()
            .unwrap_or(0)
    }

    /// Pan max of the shown commit-file or compare list: its rows sized by
    /// the segments [`Self::commit_file_row_paint_segments_in`] paints, in
    /// the list's own comment scope ([`Self::commit_file_comment_scope`]),
    /// so a compare tab never reads the parked Workspace drill. One column
    /// short while the vertical scrollbar shows (the rows overflow the
    /// painted list height), so the pan still reaches the end of the
    /// longest row beside the bar.
    fn commit_file_max_col(&self, pane_width: u16) -> usize {
        let vbar =
            self.painted_commit_file_rows().len() > usize::from(self.layout.files_list_height);
        self.commit_file_max_col_in(pane_width, vbar)
    }

    /// [`Self::commit_file_max_col`] for a list `pane_width` columns wide
    /// whose vertical scrollbar shows (`vbar`) or not: what the paint
    /// clamps its horizontal bar to.
    pub(crate) fn commit_file_max_col_in(&self, pane_width: u16, vbar: bool) -> usize {
        let width = pane_width.saturating_sub(u16::from(vbar)).max(1) as usize;
        let scope = self.commit_file_comment_scope();
        self.commit_file_rows()
            .iter()
            .map(|row| {
                let segs = self.commit_file_row_paint_segments_in(row, scope);
                let label: usize = segs.segments.iter().map(|s| visible_width(&s.text)).sum();
                let trailing: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
                list_row_pan_max(label, row.depth, trailing, width)
            })
            .max()
            .unwrap_or(0)
    }

    fn graph_max_col(&self, pane_width: u16) -> usize {
        let Some(model) = self.graph.as_ref() else {
            return 0;
        };
        // Same badges the graph paints: a badged row has less label room.
        let badges: Vec<(usize, &str, Color)> = self
            .graph_pr_badges()
            .into_iter()
            .map(|(index, _, pr)| (index, pr_badge_mark(self.ascii, pr).0, Color::Reset))
            .collect();
        graph_col_max(
            model,
            self.ascii,
            pane_width,
            self.graph_vscroll_shown(),
            &badges,
        )
    }

    /// Max `diff_col_offset` for the painted file diff (0 if it fits).
    pub(crate) fn diff_pan_max(&self) -> usize {
        if self.diff_wrap {
            return 0;
        }
        self.diff_pan_max_for(&self.current_diff_rows())
    }

    /// [`Self::diff_pan_max`] over `rows` the caller already built, so a
    /// frame does not rebuild every diff row to size the pan. `rows` must be
    /// this state's [`Self::current_diff_rows`] for the current layout;
    /// other rows give a pan max for a diff that is not painted.
    pub(crate) fn diff_pan_max_for(&self, rows: &[DiffRow]) -> usize {
        if self.diff_wrap {
            return 0;
        }
        let gutter = gutter_width(rows).saturating_add(comment_mark_cols(self.ascii));
        let pane_w = diff_paint_width(self.layout.diff_pane_width) as usize;
        let content_w = diff_row_content_width(pane_w);
        max_col_offset(&diff_line_lens(rows), cell_code_width(content_w, gutter))
    }

    /// Whether the file diff paints its horizontal scrollbar: only after
    /// the view leaves the left edge (like the graph bar), so an unpanned
    /// diff keeps the body row the bar would take.
    pub(crate) fn diff_hscroll_shown(&self) -> bool {
        !self.diff_wrap && graph_hscroll_visible(self.diff_col_offset)
    }

    /// Pan max of the file diff the right pane shows; 0 when it shows none.
    fn shown_diff_pan_max(&self) -> usize {
        if self.right_is_diff() || self.drill.is_diff() {
            self.diff_pan_max()
        } else {
            0
        }
    }

    /// Pan the right-pane diff by `delta` columns, clamped to its pan max.
    pub(super) fn pan_diff_content(&mut self, delta: i32) {
        let max = self.diff_pan_max();
        self.diff_col_offset = apply_pan(self.diff_col_offset, delta, max);
    }

    /// Apply `PanDiff` and mouse wheel, including trackpad hscroll.
    pub(crate) fn dispatch_hscroll(&mut self, action: Action) -> Effect {
        match action {
            Action::PanDiff(delta) => {
                self.pan_focused(delta);
                Effect::None
            }
            Action::ScrollWheel {
                col,
                row,
                delta,
                horizontal,
            } => {
                if !self.mouse_enabled {
                    return Effect::None;
                }
                if horizontal {
                    self.mouse_pan(col, delta);
                    return Effect::None;
                }
                // Over a commit-message footer the wheel scrolls the
                // message: the list cursor, focus, and right pane stay.
                if self.scroll_graph_footer_msg(col, row, delta)
                    || self.scroll_commit_detail_msg(col, row, delta)
                {
                    return Effect::None;
                }
                if col >= self.layout.right_x {
                    self.focus = FocusPane::Right;
                } else {
                    self.focus = FocusPane::Left;
                }
                match self.list_focus_target() {
                    ListFocusTarget::Tree => {
                        self.move_cursor(delta);
                        Effect::LoadRightPane
                    }
                    ListFocusTarget::Graph => {
                        self.move_graph_cursor(delta);
                        self.follow_graph_files()
                    }
                    ListFocusTarget::CommitFiles => self.move_file_cursor(delta),
                    // Over a folder summary the wheel scrolls its viewport:
                    // it has no cursor.
                    ListFocusTarget::None if self.summary_shown() => {
                        self.scroll_summary_by(delta);
                        Effect::None
                    }
                    ListFocusTarget::None => {
                        self.move_diff_cursor(delta);
                        Effect::None
                    }
                }
            }
            _ => Effect::None,
        }
    }
}

/// Code-cell character counts of every line row (both sides in split).
fn diff_line_lens(rows: &[DiffRow]) -> Vec<usize> {
    let mut lens = Vec::new();
    for row in rows {
        if let DiffRow::Line { left, right } = row {
            lens.push(left.text.chars().count());
            if let Some(right) = right {
                lens.push(right.text.chars().count());
            }
        }
    }
    lens
}

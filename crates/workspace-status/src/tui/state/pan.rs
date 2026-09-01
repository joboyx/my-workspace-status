//! Horizontal pan for tree, graph, commit-file, and file-diff panes.

use workspace_status_graph::graph_col_max;

use super::super::action::{Action, Effect};
use super::super::comments::tree_row_has_comment;
use super::super::diff::{cell_code_width, diff_row_content_width, gutter_width, DiffRow};
use super::super::icons::comment_mark_cols;
use super::super::search::{apply_pan, list_row_pan_max, max_col_offset};
use super::super::tree::{row_segments, NodeKind};
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
        if self.drill.is_diff() {
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
        if self.right_is_diff() || self.drill.is_diff() {
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
        } else if self.diff_can_pan() {
            self.pan_diff_content(delta);
        } else {
            self.pan_left_pane(delta);
        }
    }

    fn tree_max_col(&self) -> usize {
        let width = self.layout.tree_width.max(1) as usize;
        self.rows
            .iter()
            .map(|row| {
                let viewed = row.kind == NodeKind::File && self.reviewed.contains(&row.id);
                let commented = tree_row_has_comment(&self.comment_store, row);
                let segs = row_segments(row, self.ascii, viewed, commented);
                let label: usize = segs.segments.iter().map(|s| visible_width(&s.text)).sum();
                let trailing: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
                list_row_pan_max(label, row.depth, trailing, width)
            })
            .max()
            .unwrap_or(0)
    }

    fn commit_file_max_col(&self, pane_width: u16) -> usize {
        let width = pane_width.max(1) as usize;
        self.commit_file_rows()
            .iter()
            .map(|row| {
                let label: usize = row.segments.iter().map(|s| visible_width(&s.text)).sum();
                let trailing: usize = row
                    .trailing_segs
                    .iter()
                    .map(|s| visible_width(&s.text))
                    .sum();
                list_row_pan_max(label, row.depth, trailing, width)
            })
            .max()
            .unwrap_or(0)
    }

    fn graph_max_col(&self, pane_width: u16) -> usize {
        let Some(model) = self.graph.as_ref() else {
            return 0;
        };
        graph_col_max(model, self.ascii, pane_width, self.graph_scroll > 0)
    }

    fn diff_line_lens(&self) -> Vec<usize> {
        let mut lens = Vec::new();
        for row in self.current_diff_rows() {
            if let DiffRow::Line { left, right } = row {
                lens.push(left.text.chars().count());
                if let Some(right) = right {
                    lens.push(right.text.chars().count());
                }
            }
        }
        lens
    }

    /// Max `diff_col_offset` for the painted file diff (0 if it fits).
    pub(crate) fn diff_pan_max(&self) -> usize {
        let rows = self.current_diff_rows();
        let gutter = gutter_width(&rows).saturating_add(comment_mark_cols(self.ascii));
        let v_cols = u16::from(self.diff_scroll > 0);
        let pane_w = self.layout.diff_pane_width.saturating_sub(v_cols).max(1) as usize;
        let content_w = diff_row_content_width(pane_w);
        max_col_offset(&self.diff_line_lens(), cell_code_width(content_w, gutter))
    }

    fn diff_can_pan(&self) -> bool {
        (self.right_is_diff() || self.drill.is_diff()) && self.diff_pan_max() > 0
    }

    fn pan_diff_content(&mut self, delta: i32) {
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
                row: _,
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
                if col >= self.layout.right_x {
                    self.focus = FocusPane::Right;
                    if self.drill.is_files() {
                        self.move_file_cursor(delta)
                    } else if self.right_is_diff() || self.drill.is_diff() {
                        self.move_diff_cursor(delta);
                        Effect::None
                    } else {
                        self.move_graph_cursor(delta);
                        self.follow_graph_files()
                    }
                } else if self.drill.is_diff() {
                    self.focus = FocusPane::Left;
                    self.move_file_cursor(delta)
                } else if self.drill.is_files() {
                    self.focus = FocusPane::Left;
                    self.move_graph_cursor(delta);
                    self.follow_graph_files()
                } else {
                    self.focus = FocusPane::Left;
                    self.move_cursor(delta);
                    Effect::LoadRightPane
                }
            }
            _ => Effect::None,
        }
    }
}

//! Compose painted display lines from [`GraphModel`].
//!
//! [`GraphModel`] stays the shared source. This module adds the multi-lane
//! gutter (and spacer rows) on top of `visible_rows`.

use std::collections::{HashMap, HashSet};

use crate::format::format_label;
use crate::glyphs::GlyphSet;
use crate::layout::{layout_commits, LaidOutCommit};
use crate::model::{GraphModel, GraphRow};
use crate::stash::{
    apply_stash_join_cells, build_stash_rail_context, stash_leaf_rail_cells, stash_rail_cells,
    stem_down_rail_cells, StashRailContext,
};
use crate::topology::{
    apply_head_node_glyph, blank_gutter, cells_text, pad_to_width, slice_cells_around_lane,
    GraphCell,
};

/// One painted display line (gutter + label).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaintedLine {
    /// Gutter cells (may be empty).
    pub gutter: Vec<GraphCell>,
    /// Text after the gutter (no leading node for commit / stash).
    pub label: String,
    /// Index into [`GraphModel::visible_rows`] for this content line.
    /// Spacer rails are `None`.
    pub row_index: Option<usize>,
}

impl PaintedLine {
    /// Full line as painted (`gutter` + gap + `label`).
    pub fn text(&self) -> String {
        if self.gutter.is_empty() {
            return self.label.clone();
        }
        format!("{} {}", cells_text(&self.gutter), self.label)
    }
}

fn next_commit<'a>(
    rows: &'a [GraphRow],
    start: usize,
    laid_by_id: &'a HashMap<String, LaidOutCommit>,
) -> Option<&'a LaidOutCommit> {
    for row in rows.iter().skip(start) {
        if let GraphRow::Commit { commit, .. } = row {
            return laid_by_id.get(&commit.id);
        }
    }
    None
}

fn prev_commit<'a>(
    rows: &'a [GraphRow],
    end: usize,
    laid_by_id: &'a HashMap<String, LaidOutCommit>,
) -> Option<&'a LaidOutCommit> {
    for row in rows.iter().take(end).rev() {
        if let GraphRow::Commit { commit, .. } = row {
            return laid_by_id.get(&commit.id);
        }
    }
    None
}

/// Paint the model into display lines (sync header is applied by the widget).
pub fn paint_model(
    model: &GraphModel,
    glyphs: &GlyphSet,
    gutter_width: Option<usize>,
) -> Vec<PaintedLine> {
    let laid = layout_commits(&model.commits, glyphs);
    let laid_by_id: HashMap<String, LaidOutCommit> = laid
        .into_iter()
        .map(|row| (row.commit.id.clone(), row))
        .collect();
    let rows = model.visible_rows();

    let topo_width = laid_by_id
        .values()
        .map(|r| r.cells.len())
        .max()
        .unwrap_or(0);
    let width = match gutter_width {
        Some(cap) if cap > 0 => cap.min(topo_width.max(4)),
        _ => topo_width.max(if laid_by_id.is_empty() { 0 } else { 2 }),
    };
    let max_lane = if width > 0 {
        Some((width.saturating_sub(1)) / crate::glyphs::CELL_W)
    } else {
        None
    };

    let mut stash_ctx: HashMap<String, StashRailContext> = HashMap::new();
    let mut stash_joins: HashMap<String, Vec<usize>> = HashMap::new();
    let mut reserved_by_key: HashMap<String, HashSet<usize>> = HashMap::new();

    for (i, row) in rows.iter().enumerate() {
        let GraphRow::Stash(stash) = row else {
            continue;
        };
        let parent = stash
            .parent_id
            .as_ref()
            .and_then(|id| laid_by_id.get(id));
        let prev = prev_commit(&rows, i, &laid_by_id);
        let next = next_commit(&rows, i + 1, &laid_by_id);
        let mut tip_above_parent = false;
        if let Some(parent) = parent {
            for later in rows.iter().skip(i + 1) {
                if let GraphRow::Commit { commit, .. } = later {
                    if commit.id == parent.commit.id {
                        tip_above_parent = true;
                        break;
                    }
                }
            }
        }
        let key = parent
            .map(|p| p.commit.id.clone())
            .unwrap_or_else(|| "__orphan__".into());
        let reserved = reserved_by_key.entry(key.clone()).or_default();
        let ctx = build_stash_rail_context(
            parent,
            prev,
            next,
            tip_above_parent,
            reserved,
            max_lane,
        );
        reserved.insert(ctx.leaf_lane);
        if let Some(parent) = parent {
            if tip_above_parent {
                stash_joins
                    .entry(parent.commit.id.clone())
                    .or_default()
                    .push(ctx.leaf_lane);
            }
        }
        stash_ctx.insert(stash.stash_ref.clone(), ctx);
    }

    let mut paint_width = width;
    for ctx in stash_ctx.values() {
        paint_width = paint_width.max(ctx.leaf_lane * crate::glyphs::CELL_W + 1);
        if let Some(parent) = &ctx.parent {
            paint_width = paint_width.max(parent.cells.len());
        }
    }
    if let Some(cap) = gutter_width {
        if cap > 0 {
            paint_width = paint_width.min(cap);
        }
    }

    let mut out = Vec::new();
    for (i, row) in rows.iter().enumerate() {
        match row {
            GraphRow::Uncommitted | GraphRow::Worktree(_) => {
                out.push(PaintedLine {
                    gutter: blank_gutter(paint_width),
                    label: format_label(row, glyphs),
                    row_index: Some(i),
                });
            }
            GraphRow::Stash(stash) => {
                let ctx = stash_ctx.get(&stash.stash_ref);
                let gutter = if let Some(ctx) = ctx {
                    stash_leaf_rail_cells(paint_width, ctx, glyphs, true, false)
                } else {
                    blank_gutter(paint_width)
                };
                out.push(PaintedLine {
                    gutter,
                    label: format_label(row, glyphs),
                    row_index: Some(i),
                });
                if let Some(ctx) = ctx {
                    out.push(PaintedLine {
                        gutter: stash_leaf_rail_cells(paint_width, ctx, glyphs, false, true),
                        label: String::new(),
                        row_index: None,
                    });
                }
            }
            GraphRow::Commit {
                commit,
                is_head,
                ..
            } => {
                let Some(laid) = laid_by_id.get(&commit.id) else {
                    out.push(PaintedLine {
                        gutter: blank_gutter(paint_width),
                        label: format_label(row, glyphs),
                        row_index: Some(i),
                    });
                    continue;
                };
                let mut cells = laid.cells.clone();
                if let Some(joins) = stash_joins.get(&commit.id) {
                    cells = apply_stash_join_cells(
                        &LaidOutCommit {
                            cells: cells.clone(),
                            ..laid.clone()
                        },
                        joins,
                        glyphs,
                    );
                }
                cells = apply_head_node_glyph(&cells, *is_head, glyphs);
                let join_cols: Vec<usize> = stash_joins
                    .get(&commit.id)
                    .map(|lanes| lanes.iter().map(|l| l * crate::glyphs::CELL_W).collect())
                    .unwrap_or_default();
                if cells.len() > paint_width {
                    cells = slice_cells_around_lane(&cells, paint_width, laid.lane, &join_cols);
                } else {
                    pad_to_width(&mut cells, paint_width);
                }
                out.push(PaintedLine {
                    gutter: cells,
                    label: format_label(row, glyphs),
                    row_index: Some(i),
                });
                let next = next_commit(&rows, i + 1, &laid_by_id);
                let spacer = if let Some(next) = next {
                    stash_rail_cells(paint_width, Some(laid), Some(next), glyphs)
                } else {
                    stem_down_rail_cells(paint_width, laid, glyphs)
                };
                out.push(PaintedLine {
                    gutter: spacer,
                    label: String::new(),
                    row_index: None,
                });
            }
        }
    }
    out
}

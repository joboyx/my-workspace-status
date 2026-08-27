//! Pane-relative graph gutter budget.
//!
//! Topology layout stays full-width; paint clips the gutter so subject/refs
//! keep leftover horizontal space on wide panes. Every row in a paint uses
//! the same left-aligned window so vertical rails stay column-aligned when
//! labels clip.

use crate::topology::{pad_to_width, CellRole, GraphCell};

/// Gutter may use at most this fraction of the graph list inner width.
pub const GUTTER_MAX_FRACTION: f64 = 0.3;

/// Always leave at least this many columns for refs+subject (after the
/// gutter/subject gap). Meta columns still drop via existing row logic.
pub const MIN_SUBJECT_FLOOR: usize = 24;

/// Hybrid max gutter columns for a graph list of `pane_width` columns
/// (segment budget; cursor bar is already excluded by the caller).
pub fn graph_gutter_cap(pane_width: usize) -> usize {
    let w = pane_width.max(1);
    let by_fraction = ((w as f64) * GUTTER_MAX_FRACTION).floor() as usize;
    let by_fraction = by_fraction.max(1);
    // gap (1) + subject floor; meta competes later via drop order
    let by_floor = w.saturating_sub(1).saturating_sub(MIN_SUBJECT_FLOOR).max(1);
    by_fraction.min(by_floor).max(1)
}

/// Shared gutter width for a loaded window: topology need, capped by pane.
pub fn resolve_graph_width(topology_width: usize, pane_width: usize) -> usize {
    if topology_width == 0 {
        return 0;
    }
    topology_width.min(graph_gutter_cap(pane_width))
}

/// One shared clip for every row in a paint: keep `[0, budget)`, pad right.
///
/// Per-row windows around a node shift rails when the cap is tighter than
/// topology (narrow pane, clipped subjects). This slice is the same for
/// commits, spacers, and stash leaves.
pub fn clip_gutter_shared(cells: &[GraphCell], budget: usize) -> Vec<GraphCell> {
    if budget == 0 {
        return Vec::new();
    }
    let mut out = cells.to_vec();
    pad_to_width(&mut out, budget);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::topology::cells_text;

    fn cell(ch: &str, role: CellRole) -> GraphCell {
        GraphCell {
            ch: ch.into(),
            color_lane: Some(0),
            role,
        }
    }

    #[test]
    fn cap_is_tighter_of_fraction_and_subject_floor() {
        assert_eq!(graph_gutter_cap(100), 30);
        assert_eq!(graph_gutter_cap(40), 12);
        assert_eq!(graph_gutter_cap(30), 30 - 1 - MIN_SUBJECT_FLOOR);
    }

    #[test]
    fn resolve_never_exceeds_topology_or_hybrid_cap() {
        assert_eq!(resolve_graph_width(4, 200), 4);
        assert_eq!(resolve_graph_width(80, 100), graph_gutter_cap(100));
        assert_eq!(resolve_graph_width(0, 100), 0);
    }

    #[test]
    fn shared_clip_keeps_left_spine_and_pads() {
        let wide = vec![
            cell("│", CellRole::Pipe),
            cell(" ", CellRole::Blank),
            cell("●", CellRole::Node),
        ];
        let clipped = clip_gutter_shared(&wide, 2);
        assert_eq!(cells_text(&clipped), "│ ");
        assert_eq!(clipped[0].role, CellRole::Pipe);
        assert_ne!(clipped[0].role, CellRole::Node);

        let short = vec![cell("●", CellRole::Node)];
        let padded = clip_gutter_shared(&short, 3);
        assert_eq!(padded.len(), 3);
        assert_eq!(padded[0].role, CellRole::Node);
        assert_eq!(padded[2].role, CellRole::Blank);

        assert!(clip_gutter_shared(&wide, 0).is_empty());
    }
}

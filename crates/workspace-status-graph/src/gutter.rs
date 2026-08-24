//! Pane-relative graph gutter budget.
//!
//! Topology layout stays full-width; paint clips the gutter so subject/refs
//! keep leftover horizontal space on wide panes.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}

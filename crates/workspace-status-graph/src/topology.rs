//! Internal directional connection model for graph gutter painting.
//!
//! Cells accumulate `up` / `down` / `left` / `right` before a glyph is chosen.
//! Public [`GraphCell`] stays `{ ch, color_lane, role }`.

use crate::glyphs::{GlyphSet, CELL_W};

/// Display role of one gutter column.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellRole {
    /// Commit, HEAD, or stash node.
    Node,
    /// Rail / junction.
    Pipe,
    /// Empty column.
    Blank,
}

/// One terminal column in the graph gutter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphCell {
    /// One terminal column.
    pub ch: String,
    /// Lane colour role index, or `None` for blank.
    pub color_lane: Option<usize>,
    /// Node, pipe, or blank.
    pub role: CellRole,
}

impl GraphCell {
    /// Empty gutter cell.
    pub fn blank() -> Self {
        Self {
            ch: " ".to_string(),
            color_lane: None,
            role: CellRole::Blank,
        }
    }
}

/// Join `cells` into the gutter string used by tests and paint.
pub fn cells_text(cells: &[GraphCell]) -> String {
    cells.iter().map(|c| c.ch.as_str()).collect()
}

/// Grow `cells` to at least `width`. Does not truncate.
pub fn ensure_width(cells: &mut Vec<GraphCell>, width: usize) {
    while cells.len() < width {
        cells.push(GraphCell::blank());
    }
}

/// Grow or truncate `cells` to exactly `width`.
pub fn pad_to_width(cells: &mut Vec<GraphCell>, width: usize) {
    ensure_width(cells, width);
    if cells.len() > width {
        cells.truncate(width);
    }
}

/// Focus column for clipping — prefer the painted node, else `lane * CELL_W`.
pub fn gutter_focus_col(cells: &[GraphCell], lane: usize) -> usize {
    if let Some(i) = cells.iter().position(|c| c.role == CellRole::Node) {
        return i;
    }
    if cells.is_empty() {
        return 0;
    }
    (lane * crate::glyphs::CELL_W).min(cells.len() - 1)
}

/// Window of `budget` cells anchored so the node stays visible.
///
/// `extra_cols` (stash join / leaf columns) are included when they fit
/// beside the focus. If the span exceeds the budget, the node stays.
pub fn slice_cells_around_lane(
    cells: &[GraphCell],
    budget: usize,
    lane: usize,
    extra_cols: &[usize],
) -> Vec<GraphCell> {
    if budget == 0 {
        return Vec::new();
    }
    if cells.len() <= budget {
        return cells.to_vec();
    }
    let focus = gutter_focus_col(cells, lane);
    let mut wanted = vec![focus];
    for c in extra_cols {
        if *c < cells.len() {
            wanted.push(*c);
        }
    }
    let min_w = *wanted.iter().min().unwrap();
    let max_w = *wanted.iter().max().unwrap();
    let span = max_w - min_w + 1;
    let start = if span <= budget {
        let slack = budget - span;
        let raw = min_w.saturating_sub(slack / 2);
        raw.min(cells.len() - budget)
    } else {
        let extras: Vec<usize> = wanted.into_iter().filter(|c| *c != focus).collect();
        let extra_bias = if extras.is_empty() {
            focus as f64
        } else {
            extras.iter().sum::<usize>() as f64 / extras.len() as f64
        };
        if extra_bias >= focus as f64 {
            focus.min(cells.len() - budget)
        } else {
            focus.saturating_sub(budget - 1).min(cells.len() - budget)
        }
    };
    cells[start..start + budget].to_vec()
}

/// Per-cell topology before glyph resolution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopoCell {
    pub up: bool,
    pub down: bool,
    pub left: bool,
    pub right: bool,
    pub color_lane: Option<usize>,
    pub role: CellRole,
}

/// Empty topology cell.
pub fn empty_topo() -> TopoCell {
    TopoCell {
        up: false,
        down: false,
        left: false,
        right: false,
        color_lane: None,
        role: CellRole::Blank,
    }
}

/// Merge connection flags into a cell.
///
/// Colour priority: node wins; a through-rail keeps its lane when a
/// horizontal is layered on; else the incoming `color_lane` wins for
/// new pipe content.
pub fn connect(
    cell: &mut TopoCell,
    up: bool,
    down: bool,
    left: bool,
    right: bool,
    color_lane: Option<usize>,
    role: CellRole,
) {
    if cell.role == CellRole::Node && role != CellRole::Node {
        return;
    }
    let had_vertical = cell.up || cell.down;
    if up {
        cell.up = true;
    }
    if down {
        cell.down = true;
    }
    if left {
        cell.left = true;
    }
    if right {
        cell.right = true;
    }

    if role == CellRole::Node {
        cell.role = CellRole::Node;
        cell.color_lane = color_lane;
        return;
    }

    if cell.role == CellRole::Blank {
        cell.role = role;
    }

    let adding_horizontal = left || right;
    let adding_vertical = up || down;
    if had_vertical && adding_horizontal && !adding_vertical {
        // keep existing color_lane
    } else if color_lane.is_some() && (!had_vertical || adding_vertical || cell.color_lane.is_none())
    {
        cell.color_lane = color_lane;
    }
}

/// Resolve Unicode/ASCII glyph from final connectivity.
pub fn glyph_from_topo(cell: &TopoCell, g: &GlyphSet) -> String {
    if cell.role == CellRole::Node {
        return g.commit.to_string();
    }
    if cell.role == CellRole::Blank {
        return " ".to_string();
    }

    let TopoCell {
        up,
        down,
        left,
        right,
        ..
    } = *cell;
    if up && down && left && right {
        return g.cross.to_string();
    }
    if left && up && down && !right {
        return g.tee_left.to_string();
    }
    if right && up && down && !left {
        return g.tee_right.to_string();
    }
    if left && right && down && !up {
        return g.tee_down.to_string();
    }
    if left && right && up && !down {
        return g.tee_up.to_string();
    }
    if left && down && !right && !up {
        return g.corner_down_right.to_string();
    }
    if right && down && !left && !up {
        return g.corner_down_left.to_string();
    }
    if left && up && !right && !down {
        return g.corner_up_right.to_string();
    }
    if right && up && !left && !down {
        return g.corner_up_left.to_string();
    }
    if up && down && !left && !right {
        return g.vertical.to_string();
    }
    if left && right && !up && !down {
        return g.horizontal.to_string();
    }
    if left || right {
        return g.horizontal.to_string();
    }
    if up || down {
        return g.vertical.to_string();
    }
    " ".to_string()
}

/// Materialise a topology row into display cells.
pub fn topo_to_cells(row: &[TopoCell], g: &GlyphSet) -> Vec<GraphCell> {
    row.iter()
        .map(|cell| GraphCell {
            ch: glyph_from_topo(cell, g),
            color_lane: cell.color_lane,
            role: cell.role,
        })
        .collect()
}

/// Grow a topology row to at least `width` cells.
pub fn ensure_topo_width(row: &mut Vec<TopoCell>, width: usize) {
    while row.len() < width {
        row.push(empty_topo());
    }
}

/// Add a vertical through-rail on a lane column.
pub fn add_vertical(row: &mut Vec<TopoCell>, lane: usize, color_lane: usize) {
    let col = lane * CELL_W;
    ensure_topo_width(row, col + 1);
    connect(
        &mut row[col],
        true,
        true,
        false,
        false,
        Some(color_lane),
        CellRole::Pipe,
    );
}

/// Fill the horizontal bridge between two lane columns (exclusive of endpoints).
pub fn add_horizontal_bridge(
    row: &mut Vec<TopoCell>,
    from_lane: usize,
    to_lane: usize,
    color_lane: usize,
) {
    let lo = from_lane.min(to_lane);
    let hi = from_lane.max(to_lane);
    let start = lo * CELL_W;
    let end = hi * CELL_W;
    ensure_topo_width(row, end + 1);
    for col in (start + 1)..end {
        connect(
            &mut row[col],
            false,
            false,
            true,
            true,
            Some(color_lane),
            CellRole::Pipe,
        );
    }
}

/// Whether a topology cell has an upward / downward stem before node clear.
pub fn topo_stem_dirs(row: &[TopoCell], col: usize, commit_lane: usize) -> (bool, bool) {
    let node_col = commit_lane * CELL_W;
    if col == node_col {
        return (true, true);
    }
    match row.get(col) {
        Some(cell) => (cell.up, cell.down),
        None => (false, false),
    }
}

/// Open a new secondary lane: corner at `to_lane` (down + toward commit).
pub fn add_open_corner(
    row: &mut Vec<TopoCell>,
    commit_lane: usize,
    to_lane: usize,
    color_lane: usize,
) {
    let col = to_lane * CELL_W;
    ensure_topo_width(row, col + 1);
    if to_lane > commit_lane {
        connect(
            &mut row[col],
            false,
            true,
            true,
            false,
            Some(color_lane),
            CellRole::Pipe,
        );
    } else {
        connect(
            &mut row[col],
            false,
            true,
            false,
            true,
            Some(color_lane),
            CellRole::Pipe,
        );
    }
    add_horizontal_bridge(row, commit_lane, to_lane, color_lane);
}

/// Close an incoming waiter into the commit: up-corner at `from_lane`.
pub fn add_join_corner(
    row: &mut Vec<TopoCell>,
    commit_lane: usize,
    from_lane: usize,
    color_lane: usize,
) {
    let col = from_lane * CELL_W;
    ensure_topo_width(row, col + 1);
    if from_lane > commit_lane {
        connect(
            &mut row[col],
            true,
            false,
            true,
            false,
            Some(color_lane),
            CellRole::Pipe,
        );
    } else {
        connect(
            &mut row[col],
            true,
            false,
            false,
            true,
            Some(color_lane),
            CellRole::Pipe,
        );
    }
    add_horizontal_bridge(row, commit_lane, from_lane, color_lane);
}

/// Link the commit horizontally into an already-live secondary parent rail.
pub fn add_link_tee(
    row: &mut Vec<TopoCell>,
    commit_lane: usize,
    target_lane: usize,
    color_lane: usize,
) {
    let col = target_lane * CELL_W;
    ensure_topo_width(row, col + 1);
    if target_lane > commit_lane {
        connect(
            &mut row[col],
            false,
            false,
            true,
            false,
            Some(color_lane),
            CellRole::Pipe,
        );
    } else {
        connect(
            &mut row[col],
            false,
            false,
            false,
            true,
            Some(color_lane),
            CellRole::Pipe,
        );
    }
    add_horizontal_bridge(row, commit_lane, target_lane, color_lane);
}

/// Place the commit/merge node on its lane.
pub fn add_node(row: &mut Vec<TopoCell>, commit_lane: usize) {
    let col = commit_lane * CELL_W;
    ensure_topo_width(row, col + 1);
    let cell = &mut row[col];
    cell.role = CellRole::Node;
    cell.color_lane = Some(commit_lane);
    cell.up = false;
    cell.down = false;
    cell.left = false;
    cell.right = false;
}

/// Replace the node glyph with the HEAD mark when `is_head` is true.
pub fn apply_head_node_glyph(cells: &[GraphCell], is_head: bool, g: &GlyphSet) -> Vec<GraphCell> {
    if !is_head {
        return cells.to_vec();
    }
    cells
        .iter()
        .map(|c| {
            if c.role == CellRole::Node {
                GraphCell {
                    ch: g.head_commit.to_string(),
                    color_lane: c.color_lane,
                    role: CellRole::Node,
                }
            } else {
                c.clone()
            }
        })
        .collect()
}

/// Blank gutter of `width` cells.
pub fn blank_gutter(width: usize) -> Vec<GraphCell> {
    (0..width).map(|_| GraphCell::blank()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::UNICODE;

    #[test]
    fn glyph_map_matches_topology_doc() {
        let mut cell = empty_topo();
        cell.role = CellRole::Pipe;
        cell.up = true;
        cell.down = true;
        cell.left = true;
        cell.right = true;
        assert_eq!(glyph_from_topo(&cell, &UNICODE), "┼");
        cell.right = false;
        assert_eq!(glyph_from_topo(&cell, &UNICODE), "┤");
        cell.left = false;
        cell.right = true;
        assert_eq!(glyph_from_topo(&cell, &UNICODE), "├");
    }
}

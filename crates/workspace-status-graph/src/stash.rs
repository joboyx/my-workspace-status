//! Stash leaf paint and commit↔commit densify.
//!
//! Stash visual grammar (normative, see `docs/git-graph-topology.md`):
//! a stash is a one-node side-branch tip (`◇` instead of `●`). The tip
//! sits on a free leaf lane, coloured by `stash^1`. It is never a fake
//! DAG lane. Densify stays on commit spacers only.

use std::collections::HashSet;

use crate::glyphs::{GlyphSet, CELL_W};
use crate::layout::{GraphStemRef, LaidOutCommit};
use crate::topology::{
    add_horizontal_bridge, add_join_corner, add_vertical, blank_gutter, connect, empty_topo,
    ensure_topo_width, ensure_width, slice_cells_around_lane, topo_to_cells, CellRole, GraphCell,
    TopoCell,
};

/// Paint context for a stash as a 1-node side leaf tip.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StashRailContext {
    /// Laid-out `stash^1`, or `None` when outside the loaded window.
    pub parent: Option<LaidOutCommit>,
    /// Live commit-DAG stems at this chrono gap.
    pub live_rails: Vec<GraphStemRef>,
    /// Free leaf lane for `◇` — not in the live set / not on `parent.lane`.
    pub leaf_lane: usize,
    /// True when the stash tip sits above `stash^1` so the join can close.
    pub tip_above_parent: bool,
    /// Newer sibling tips already parked above this row on the same parent.
    pub sibling_spur_lanes: Vec<usize>,
}

/// Live stem refs at a stash chrono gap.
///
/// Prefer `next.stem_up` (post-densify arrival columns); fall back to
/// `prev.stem_down` at the window tail.
pub fn stash_live_rails_at_gap(
    prev: Option<&LaidOutCommit>,
    next: Option<&LaidOutCommit>,
) -> Vec<GraphStemRef> {
    let refs = next
        .map(|r| r.stem_up.as_slice())
        .or_else(|| prev.map(|r| r.stem_down.as_slice()))
        .unwrap_or(&[]);
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for r in refs {
        if seen.contains(&r.col) {
            continue;
        }
        seen.insert(r.col);
        out.push(r.clone());
    }
    out
}

/// Lane indices occupied by live stem columns.
pub fn live_lanes_from_rails(live_rails: &[GraphStemRef]) -> HashSet<usize> {
    live_rails
        .iter()
        .map(|r| r.col / CELL_W)
        .collect()
}

/// Allocate a free leaf lane for a stash tip.
///
/// Lowest free lane that is not live, not the parent, and not reserved
/// by a sibling tip. When `max_lane` is set, search that range first so
/// the diamond stays inside the clipped gutter. Does not steal a live
/// DAG column.
pub fn allocate_stash_leaf_lane(
    live_lanes: &HashSet<usize>,
    parent_lane: Option<usize>,
    reserved: &HashSet<usize>,
    max_lane: Option<usize>,
) -> usize {
    let mut blocked = live_lanes.clone();
    if let Some(lane) = parent_lane {
        blocked.insert(lane);
    }
    for lane in reserved {
        blocked.insert(*lane);
    }
    if let Some(max) = max_lane {
        for lane in 0..=max {
            if !blocked.contains(&lane) {
                return lane;
            }
        }
    }
    let mut lane = 0;
    loop {
        if !blocked.contains(&lane) {
            return lane;
        }
        lane += 1;
    }
}

/// Build [`StashRailContext`] for one stash at its chrono gap.
pub fn build_stash_rail_context(
    parent: Option<&LaidOutCommit>,
    prev: Option<&LaidOutCommit>,
    next: Option<&LaidOutCommit>,
    tip_above_parent: bool,
    reserved: &HashSet<usize>,
    max_lane: Option<usize>,
) -> StashRailContext {
    let live_rails = stash_live_rails_at_gap(prev, next);
    let mut live_lanes = live_lanes_from_rails(&live_rails);
    if let Some(parent) = parent {
        live_lanes.insert(parent.lane);
    }
    let leaf_lane = allocate_stash_leaf_lane(
        &live_lanes,
        parent.map(|p| p.lane),
        reserved,
        max_lane,
    );
    let sibling_spur_lanes = if parent.is_some() && tip_above_parent {
        reserved
            .iter()
            .copied()
            .filter(|lane| *lane != leaf_lane)
            .collect()
    } else {
        Vec::new()
    };
    StashRailContext {
        parent: parent.cloned(),
        live_rails,
        leaf_lane,
        tip_above_parent: parent.is_some() && tip_above_parent,
        sibling_spur_lanes,
    }
}

/// Pair previous stem-down refs to next stem-up refs by rail identity.
///
/// Prefer same-column matches when duplicate ids exist (sibling waiters).
pub fn match_stem_refs(
    down: &[GraphStemRef],
    up: &[GraphStemRef],
) -> Vec<(GraphStemRef, GraphStemRef)> {
    let mut used = HashSet::new();
    let mut pairs = Vec::new();
    for from in down {
        let same_col = up.iter().enumerate().find(|(i, to)| {
            !used.contains(i) && to.id == from.id && to.col == from.col
        });
        let hit = same_col.or_else(|| {
            up.iter()
                .enumerate()
                .find(|(i, to)| !used.contains(i) && to.id == from.id)
        });
        if let Some((i, to)) = hit {
            used.insert(i);
            pairs.push((from.clone(), to.clone()));
        }
    }
    pairs
}

/// Paint one densify remapped rail on a commit spacer.
fn paint_stem_transition(topo: &mut Vec<TopoCell>, from_col: usize, to_col: usize, color_lane: usize) {
    let from_lane = from_col / CELL_W;
    let to_lane = to_col / CELL_W;
    ensure_topo_width(topo, from_col.max(to_col) + 1);

    if from_col == to_col {
        add_vertical(topo, from_lane, color_lane);
        return;
    }

    if from_col > to_col {
        connect(
            &mut topo[from_col],
            true,
            false,
            true,
            false,
            Some(color_lane),
            CellRole::Pipe,
        );
        connect(
            &mut topo[to_col],
            false,
            true,
            false,
            true,
            Some(color_lane),
            CellRole::Pipe,
        );
    } else {
        connect(
            &mut topo[from_col],
            true,
            false,
            false,
            true,
            Some(color_lane),
            CellRole::Pipe,
        );
        connect(
            &mut topo[to_col],
            false,
            true,
            true,
            false,
            Some(color_lane),
            CellRole::Pipe,
        );
    }
    add_horizontal_bridge(topo, from_lane, to_lane, color_lane);
}

fn clip_or_pad(
    cells: Vec<GraphCell>,
    budget: usize,
    anchor_lane: usize,
    extra_cols: &[usize],
) -> Vec<GraphCell> {
    if cells.len() > budget {
        slice_cells_around_lane(&cells, budget, anchor_lane, extra_cols)
    } else if cells.len() < budget {
        let mut cells = cells;
        cells.extend(blank_gutter(budget - cells.len()));
        cells
    } else {
        cells
    }
}

/// Densify gutter that preserves live rails between two commits.
///
/// Historical name (`stash_rail_*`) — commit↔commit spacer densify only.
/// Stash leaf tips use [`stash_leaf_rail_cells`] on `stash^1`'s lane colour.
pub fn stash_rail_cells(
    display_width: usize,
    prev: Option<&LaidOutCommit>,
    next: Option<&LaidOutCommit>,
    glyphs: &GlyphSet,
) -> Vec<GraphCell> {
    if display_width == 0 {
        return Vec::new();
    }
    let (Some(prev), Some(next)) = (prev, next) else {
        return blank_gutter(display_width);
    };
    let mut topology_width = prev.cells.len().max(next.cells.len()).max(display_width);
    for r in prev.stem_down.iter().chain(next.stem_up.iter()) {
        topology_width = topology_width.max(r.col + 1);
    }
    let mut topo = vec![empty_topo(); topology_width];
    for (from, to) in match_stem_refs(&prev.stem_down, &next.stem_up) {
        paint_stem_transition(&mut topo, from.col, to.col, from.color_lane);
    }
    let mut cells = topo_to_cells(&topo, glyphs);
    ensure_width(&mut cells, topology_width);
    clip_or_pad(cells, display_width, prev.lane, &[])
}

/// Through-rails only from `prev.stem_down` (no densify to a next commit).
pub fn stem_down_rail_cells(
    display_width: usize,
    prev: &LaidOutCommit,
    glyphs: &GlyphSet,
) -> Vec<GraphCell> {
    if display_width == 0 {
        return Vec::new();
    }
    let mut topology_width = prev.cells.len().max(display_width);
    for r in &prev.stem_down {
        topology_width = topology_width.max(r.col + 1);
    }
    let mut topo = vec![empty_topo(); topology_width];
    for r in &prev.stem_down {
        paint_stem_transition(&mut topo, r.col, r.col, r.color_lane);
    }
    let mut cells = topo_to_cells(&topo, glyphs);
    ensure_width(&mut cells, topology_width);
    clip_or_pad(cells, display_width, prev.lane, &[])
}

fn merge_join_overlay_cell(base: &GraphCell, over: &GraphCell, g: &GlyphSet) -> GraphCell {
    if base.role == CellRole::Node {
        return base.clone();
    }
    if over.role == CellRole::Blank {
        return base.clone();
    }
    if base.role == CellRole::Blank {
        return over.clone();
    }

    let base_vertical = [
        g.vertical,
        g.cross,
        g.tee_left,
        g.tee_right,
        g.tee_up,
        g.tee_down,
    ]
    .contains(&base.ch.as_str());
    let over_horizontal = [
        g.horizontal,
        g.tee_up,
        g.tee_down,
        g.cross,
        g.corner_up_right,
        g.corner_up_left,
        g.corner_down_right,
        g.corner_down_left,
    ]
    .contains(&over.ch.as_str());
    if base_vertical && over_horizontal {
        return GraphCell {
            ch: g.cross.to_string(),
            color_lane: base.color_lane.or(over.color_lane),
            role: CellRole::Pipe,
        };
    }
    if base.role == CellRole::Pipe && over.role == CellRole::Pipe {
        return GraphCell {
            ch: over.ch.clone(),
            color_lane: over.color_lane.or(base.color_lane),
            role: CellRole::Pipe,
        };
    }
    base.clone()
}

/// Overlay close-elbow joins for stash leaf tips onto a `stash^1` commit row.
pub fn apply_stash_join_cells(
    parent: &LaidOutCommit,
    leaf_lanes: &[usize],
    glyphs: &GlyphSet,
) -> Vec<GraphCell> {
    let unique: Vec<usize> = {
        let mut seen = HashSet::new();
        leaf_lanes
            .iter()
            .copied()
            .filter(|l| *l != parent.lane && seen.insert(*l))
            .collect()
    };
    if unique.is_empty() {
        return parent.cells.clone();
    }

    let mut width = parent.cells.len();
    for lane in &unique {
        width = width.max(lane * CELL_W + 1).max(parent.lane * CELL_W + 1);
    }

    let mut out = parent.cells.clone();
    ensure_width(&mut out, width);

    let mut topo = vec![empty_topo(); width];
    for leaf_lane in unique {
        add_join_corner(&mut topo, parent.lane, leaf_lane, parent.lane);
    }
    let overlay = topo_to_cells(&topo, glyphs);
    for (i, over) in overlay.iter().enumerate() {
        if over.role == CellRole::Blank {
            continue;
        }
        out[i] = merge_join_overlay_cell(&out[i], over, glyphs);
    }
    out
}

/// Leaf-tip gutter for a stash: 1-node side tip on a free lane.
///
/// Tip row (`node`): through-rails + `◇` on `leaf_lane` (no mid-rail
/// tee, no `down` on the tip column). Spacer (`spur_rail`): live rails
/// plus a short spur toward the `stash^1` join. Does not densify
/// commit↔commit rails.
pub fn stash_leaf_rail_cells(
    display_width: usize,
    ctx: &StashRailContext,
    glyphs: &GlyphSet,
    node: bool,
    spur_rail: bool,
) -> Vec<GraphCell> {
    if display_width == 0 {
        return Vec::new();
    }
    let leaf_lane = ctx.leaf_lane;
    let tip_color = ctx.parent.as_ref().map(|p| p.lane).unwrap_or(leaf_lane);
    let mut topology_width = display_width.max(leaf_lane * CELL_W + 1);
    for r in &ctx.live_rails {
        topology_width = topology_width.max(r.col + 1);
    }
    if let Some(parent) = &ctx.parent {
        topology_width = topology_width.max(parent.cells.len());
    }

    let mut topo = vec![empty_topo(); topology_width];
    for r in &ctx.live_rails {
        add_vertical(&mut topo, r.col / CELL_W, r.color_lane);
    }
    for lane in &ctx.sibling_spur_lanes {
        if *lane == leaf_lane {
            continue;
        }
        add_vertical(&mut topo, *lane, tip_color);
    }

    let leaf_col = leaf_lane * CELL_W;
    let paint_spur = !node && spur_rail && ctx.parent.is_some() && ctx.tip_above_parent;
    if paint_spur {
        ensure_topo_width(&mut topo, leaf_col + 1);
        connect(
            &mut topo[leaf_col],
            true,
            true,
            false,
            false,
            Some(tip_color),
            CellRole::Pipe,
        );
    }

    let mut cells = topo_to_cells(&topo, glyphs);
    ensure_width(&mut cells, topology_width);
    if node {
        ensure_width(&mut cells, leaf_col + 1);
        cells[leaf_col] = GraphCell {
            ch: glyphs.stash.to_string(),
            color_lane: Some(tip_color),
            role: CellRole::Node,
        };
    }

    let anchor = if node || paint_spur {
        leaf_lane
    } else {
        ctx.parent.as_ref().map(|p| p.lane).unwrap_or(leaf_lane)
    };
    let extra_cols: Vec<usize> = ctx
        .live_rails
        .iter()
        .map(|r| r.col)
        .chain(ctx.sibling_spur_lanes.iter().map(|lane| lane * CELL_W))
        .chain(ctx.parent.as_ref().map(|p| p.lane * CELL_W))
        .chain(std::iter::once(leaf_lane * CELL_W))
        .collect();
    clip_or_pad(cells, display_width, anchor, &extra_cols)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::UNICODE;
    use crate::layout::layout_commits;
    use crate::model::Commit;
    use crate::topology::cells_text;

    fn c(id: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.into(),
            subject: id.into(),
            parents: parents.iter().map(|p| (*p).to_string()).collect(),
            refs: Vec::new(),
            author_name: String::new(),
            author_date_unix: 0,
        }
    }

    #[test]
    fn stash_leaf_sits_on_free_lane_coloured_by_parent() {
        let laid = layout_commits(&[c("head", &["old"]), c("old", &[])], &UNICODE);
        let parent = &laid[0];
        assert_eq!(parent.lane, 0);
        let ctx = build_stash_rail_context(
            Some(parent),
            None,
            Some(parent),
            true,
            &HashSet::new(),
            None,
        );
        assert_ne!(ctx.leaf_lane, parent.lane, "◇ must not sit on stash^1 lane");
        assert_eq!(ctx.leaf_lane, 1);

        let tip = stash_leaf_rail_cells(4, &ctx, &UNICODE, true, false);
        let tip_text = cells_text(&tip);
        assert!(tip_text.contains('◇'), "{tip_text:?}");
        assert_eq!(tip[ctx.leaf_lane * CELL_W].ch, "◇");
        assert_eq!(tip[ctx.leaf_lane * CELL_W].color_lane, Some(parent.lane));
        assert_eq!(tip[ctx.leaf_lane * CELL_W].role, CellRole::Node);
        // No down stem under the tip — the node replaced any pipe.
        assert!(!tip_text.contains("◇│") && tip[ctx.leaf_lane * CELL_W].role == CellRole::Node);

        let spur = stash_leaf_rail_cells(4, &ctx, &UNICODE, false, true);
        let spur_text = cells_text(&spur);
        assert!(
            spur[ctx.leaf_lane * CELL_W].ch == UNICODE.vertical,
            "spacer must carry a short spur, got {spur_text:?}"
        );
        assert_eq!(spur[ctx.leaf_lane * CELL_W].color_lane, Some(parent.lane));
    }

    #[test]
    fn stash_does_not_steal_live_merge_lane() {
        let laid = layout_commits(
            &[
                c("merge", &["main", "side"]),
                c("main", &["base"]),
                c("side", &["base"]),
                c("base", &[]),
            ],
            &UNICODE,
        );
        let parent = laid.iter().find(|r| r.commit.id == "merge").unwrap();
        let ctx = build_stash_rail_context(
            Some(parent),
            None,
            Some(parent),
            true,
            &HashSet::new(),
            None,
        );
        let live = live_lanes_from_rails(&ctx.live_rails);
        assert!(
            !live.contains(&ctx.leaf_lane),
            "leaf {} collided with live {:?}",
            ctx.leaf_lane,
            live
        );
        assert_ne!(ctx.leaf_lane, parent.lane);
    }

    #[test]
    fn parent_row_gains_join_elbow_for_stash() {
        let laid = layout_commits(&[c("head", &["old"]), c("old", &[])], &UNICODE);
        let parent = &laid[0];
        let cells = apply_stash_join_cells(parent, &[1], &UNICODE);
        let text = cells_text(&cells);
        assert!(
            text.contains('╯'),
            "join should close on stash^1, got {text:?}"
        );
        assert!(cells.iter().any(|c| c.role == CellRole::Node));
    }
}

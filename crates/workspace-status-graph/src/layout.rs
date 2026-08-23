//! Assign lanes and paint a lazygit-style coloured cell matrix.
//!
//! Lane assignment matches the Ink walk (`src/tui/graph/layout.ts`). Paint
//! builds an internal directional connection model then resolves glyphs.
//!
//! Duplicate first-parent waiters are intentional: sibling tips that share
//! a parent keep distinct lanes until the parent row joins them. After each
//! join, active lanes densify left so history returns to lane 0.

use crate::glyphs::{GlyphSet, CELL_W};
use crate::model::Commit;
use crate::topology::{
    add_join_corner, add_link_tee, add_node, add_open_corner, add_vertical, cells_text,
    ensure_topo_width, ensure_width, pad_to_width, topo_stem_dirs, topo_to_cells, GraphCell,
    TopoCell,
};

/// One live stem endpoint identified by the commit id the rail waits on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphStemRef {
    /// Gutter column (`lane * CELL_W`).
    pub col: usize,
    /// Waiter / commit identity for this stem.
    pub id: String,
    /// Lane colour role for the rail.
    pub color_lane: usize,
}

/// Commit after lane assignment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaidOutCommit {
    /// Source commit.
    pub commit: Commit,
    /// Assigned lane (0-based).
    pub lane: usize,
    /// Lane count used on this row.
    pub lane_count: usize,
    /// Padded to the window's max gutter width.
    pub cells: Vec<GraphCell>,
    /// `cells` joined — tests / debug.
    pub edges: String,
    /// Upward stems into this row, keyed by rail identity.
    pub stem_up: Vec<GraphStemRef>,
    /// Downward stems leaving this row, keyed by rail identity.
    pub stem_down: Vec<GraphStemRef>,
}

struct ParentPlan {
    next: Vec<Option<String>>,
    opened: Vec<usize>,
    linked: Vec<usize>,
}

/// Plan next active vector + secondary opens + links to already-live parents.
///
/// Always keep a first-parent waiter on `commit_lane` (duplicate ids OK).
fn plan_parents(
    active: &[Option<String>],
    commit_lane: usize,
    parents: &[String],
) -> ParentPlan {
    let mut next = active.to_vec();
    while next.len() <= commit_lane {
        next.push(None);
    }
    next[commit_lane] = None;

    let mut opened = Vec::new();
    let mut linked = Vec::new();
    if parents.is_empty() {
        return ParentPlan {
            next,
            opened,
            linked,
        };
    }

    next[commit_lane] = Some(parents[0].clone());

    for parent in parents.iter().skip(1) {
        let existing = next.iter().position(|id| id.as_deref() == Some(parent.as_str()));
        if let Some(existing) = existing {
            if existing != commit_lane {
                linked.push(existing);
            }
            continue;
        }
        let mut pl = None;
        for i in (commit_lane + 1)..next.len() {
            if next[i].is_none() {
                pl = Some(i);
                break;
            }
        }
        let pl = if let Some(pl) = pl {
            next[pl] = Some(parent.clone());
            pl
        } else {
            while next.len() <= commit_lane {
                next.push(None);
            }
            let pl = next.len();
            next.push(Some(parent.clone()));
            pl
        };
        opened.push(pl);
    }

    ParentPlan {
        next,
        opened,
        linked,
    }
}

fn trim_trailing_nulls(active: &mut Vec<Option<String>>) {
    while active.last().is_some_and(|id| id.is_none()) {
        active.pop();
    }
}

/// Densify active lanes left — remove holes left by closes.
fn compact_active(active: &mut Vec<Option<String>>) {
    let live: Vec<Option<String>> = active.iter().filter(|id| id.is_some()).cloned().collect();
    *active = live;
}

fn lane_stem_id(
    lane: usize,
    commit_id: &str,
    active: &[Option<String>],
    next: &[Option<String>],
    join_from: &[usize],
    opened: &[usize],
) -> Option<String> {
    if join_from.contains(&lane) {
        return Some(commit_id.to_string());
    }
    if opened.contains(&lane) {
        return next.get(lane).and_then(|id| id.clone());
    }
    active
        .get(lane)
        .and_then(|id| id.clone())
        .or_else(|| next.get(lane).and_then(|id| id.clone()))
}

fn collect_stem_refs(
    topo: &[TopoCell],
    commit: &Commit,
    commit_lane: usize,
    active: &[Option<String>],
    next: &[Option<String>],
    join_from: &[usize],
    opened: &[usize],
) -> (Vec<GraphStemRef>, Vec<GraphStemRef>) {
    let mut stem_up = Vec::new();
    let mut stem_down = Vec::new();
    let node_col = commit_lane * CELL_W;
    let lane_count = topo.len().div_ceil(CELL_W);

    for lane in 0..lane_count {
        let col = lane * CELL_W;
        let (up, down) = topo_stem_dirs(topo, col, commit_lane);
        if col == node_col {
            if up {
                stem_up.push(GraphStemRef {
                    col,
                    id: commit.id.clone(),
                    color_lane: commit_lane,
                });
            }
            if down {
                if let Some(parent) = commit.parents.first() {
                    stem_down.push(GraphStemRef {
                        col,
                        id: parent.clone(),
                        color_lane: commit_lane,
                    });
                }
            }
            continue;
        }
        let Some(id) = lane_stem_id(lane, &commit.id, active, next, join_from, opened) else {
            continue;
        };
        let color_lane = topo.get(col).and_then(|c| c.color_lane).unwrap_or(lane);
        if up {
            stem_up.push(GraphStemRef {
                col,
                id: id.clone(),
                color_lane,
            });
        }
        if down {
            stem_down.push(GraphStemRef {
                col,
                id,
                color_lane,
            });
        }
    }

    (stem_up, stem_down)
}

fn paint_cells(
    g: &GlyphSet,
    commit: &Commit,
    commit_lane: usize,
    active: &[Option<String>],
    next: &[Option<String>],
    vertical_lanes: &[usize],
    opened: &[usize],
    linked: &[usize],
    join_from: &[usize],
    col_count: usize,
) -> (Vec<GraphCell>, Vec<GraphStemRef>, Vec<GraphStemRef>) {
    let mut topo = Vec::new();
    ensure_topo_width(&mut topo, col_count);

    for &lane in vertical_lanes {
        if lane == commit_lane {
            continue;
        }
        add_vertical(&mut topo, lane, lane);
    }

    for &from in join_from {
        if from == commit_lane {
            continue;
        }
        add_join_corner(&mut topo, commit_lane, from, commit_lane);
    }

    for &to in linked {
        if to == commit_lane {
            continue;
        }
        add_link_tee(&mut topo, commit_lane, to, to);
    }

    for &to in opened {
        if to == commit_lane {
            continue;
        }
        add_open_corner(&mut topo, commit_lane, to, to);
    }

    let (stem_up, stem_down) = collect_stem_refs(
        &topo, commit, commit_lane, active, next, join_from, opened,
    );
    add_node(&mut topo, commit_lane);

    let mut cells = topo_to_cells(&topo, g);
    let node_spacer = commit_lane * CELL_W + 1;
    if cells.get(node_spacer).map(|c| c.role) != Some(crate::topology::CellRole::Pipe) {
        ensure_width(&mut cells, node_spacer + 1);
        if cells.get(node_spacer).map(|c| c.role) != Some(crate::topology::CellRole::Pipe) {
            cells[node_spacer] = GraphCell::blank();
        }
    }

    (cells, stem_up, stem_down)
}

/// Pad every row's cells to `width` (blank on the right).
pub fn pad_graph_cells(rows: &mut [LaidOutCommit], width: usize) {
    for row in rows {
        pad_to_width(&mut row.cells, width);
        row.edges = cells_text(&row.cells);
    }
}

/// Assign lanes and Unicode/ASCII edge cells for a newest-first commit list.
pub fn layout_commits(commits: &[Commit], glyphs: &GlyphSet) -> Vec<LaidOutCommit> {
    let mut active: Vec<Option<String>> = Vec::new();
    let mut out: Vec<LaidOutCommit> = Vec::new();

    for commit in commits {
        let lane = if let Some(i) = active
            .iter()
            .position(|id| id.as_deref() == Some(commit.id.as_str()))
        {
            i
        } else if let Some(i) = active.iter().position(|id| id.is_none()) {
            active[i] = Some(commit.id.clone());
            i
        } else {
            let i = active.len();
            active.push(Some(commit.id.clone()));
            i
        };

        let incoming: Vec<usize> = active
            .iter()
            .enumerate()
            .filter(|(i, id)| *i != lane && id.as_deref() == Some(commit.id.as_str()))
            .map(|(i, _)| i)
            .collect();

        let ParentPlan {
            mut next,
            opened,
            linked,
        } = plan_parents(&active, lane, &commit.parents);
        let join_from = incoming.clone();
        for &i in &incoming {
            if i != lane && i < next.len() {
                next[i] = None;
            }
        }

        let vertical_lanes: Vec<usize> = active
            .iter()
            .enumerate()
            .filter(|(i, id)| id.is_some() && *i != lane && !join_from.contains(i))
            .map(|(i, _)| i)
            .collect();

        let mut highest = lane;
        for (i, id) in next.iter().enumerate() {
            if id.is_some() {
                highest = highest.max(i);
            }
        }
        for &i in opened.iter().chain(linked.iter()).chain(join_from.iter()) {
            highest = highest.max(i);
        }
        for (i, id) in active.iter().enumerate() {
            if id.is_some() {
                highest = highest.max(i);
            }
        }
        let lane_count = highest.max(0) + 1;
        let col_count = lane_count * CELL_W;

        let (cells, stem_up, stem_down) = paint_cells(
            glyphs,
            commit,
            lane,
            &active,
            &next,
            &vertical_lanes,
            &opened,
            &linked,
            &join_from,
            col_count,
        );

        out.push(LaidOutCommit {
            commit: commit.clone(),
            lane,
            lane_count,
            edges: cells_text(&cells),
            cells,
            stem_up,
            stem_down,
        });

        active = next;
        trim_trailing_nulls(&mut active);
        compact_active(&mut active);
    }

    let max_width = out.iter().map(|r| r.cells.len()).max().unwrap_or(0);
    pad_graph_cells(&mut out, max_width);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::{ASCII, UNICODE};

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

    fn edges(rows: &[LaidOutCommit]) -> Vec<(String, usize, String)> {
        rows.iter()
            .map(|r| (r.commit.id.clone(), r.lane, r.edges.trim_end().to_string()))
            .collect()
    }

    #[test]
    fn linear_history_stays_on_one_lane() {
        let rows = layout_commits(&[c("a", &["b"]), c("b", &["d"]), c("d", &[])], &UNICODE);
        assert_eq!(rows.len(), 3);
        for r in &rows {
            assert_eq!(r.lane, 0);
            assert_eq!(r.lane_count, 1);
            assert!(r.edges.contains('●'));
        }
        assert_eq!(
            rows.iter().map(|r| r.edges.trim_end()).collect::<Vec<_>>(),
            ["●", "●", "●"]
        );
    }

    #[test]
    fn merge_diamond_opens_second_lane_then_collapses() {
        let rows = layout_commits(
            &[
                c("m999", &["a111", "b222"]),
                c("a111", &["r000"]),
                c("b222", &["r000"]),
                c("r000", &[]),
            ],
            &UNICODE,
        );
        assert_eq!(
            edges(&rows),
            vec![
                ("m999".into(), 0, "●─╮".into()),
                ("a111".into(), 0, "● │".into()),
                ("b222".into(), 1, "│ ●".into()),
                ("r000".into(), 0, "●─╯".into()),
            ]
        );
        assert_eq!(rows[3].lane, 0);
    }

    #[test]
    fn two_parent_join_keeps_sibling_tip_lanes() {
        let rows = layout_commits(
            &[
                c("mainTip", &["base"]),
                c("tipA", &["base"]),
                c("tipB", &["base"]),
                c("base", &[]),
            ],
            &UNICODE,
        );
        let by_id = |id: &str| rows.iter().find(|r| r.commit.id == id).unwrap();
        assert_eq!(by_id("mainTip").lane, 0);
        assert_eq!(by_id("tipA").lane, 1);
        assert_eq!(by_id("tipB").lane, 2);
        assert_eq!(by_id("base").lane, 0);
        let join = &by_id("base").edges;
        assert!(
            join.contains('╯') || join.contains('┴'),
            "base should show join, got {join:?}"
        );
    }

    #[test]
    fn compact_left_after_join() {
        let rows = layout_commits(
            &[
                c("merge", &["mainKeep", "sideTip"]),
                c("sideTip", &["mainKeep"]),
                c("mainKeep", &["older"]),
                c("older", &[]),
            ],
            &UNICODE,
        );
        assert_eq!(
            edges(&rows),
            vec![
                ("merge".into(), 0, "●─╮".into()),
                ("sideTip".into(), 1, "│ ●".into()),
                ("mainKeep".into(), 0, "●─╯".into()),
                ("older".into(), 0, "●".into()),
            ]
        );
    }

    #[test]
    fn live_rail_link_uses_tee_not_open_corner() {
        let rows = layout_commits(
            &[
                c("featTip", &["featBase"]),
                c("merge", &["main", "featBase"]),
                c("main", &["featBase"]),
                c("featBase", &[]),
            ],
            &UNICODE,
        );
        let merge = rows.iter().find(|r| r.commit.id == "merge").unwrap();
        assert!(
            merge.edges.contains('├') || merge.edges.contains('┤'),
            "expected tee, got {:?}",
            merge.edges
        );
        assert!(
            !merge.edges.contains('╮'),
            "open-corner on a live rail: {:?}",
            merge.edges
        );
    }

    #[test]
    fn horizontal_across_through_rail_is_cross() {
        let rows = layout_commits(
            &[
                c("after", &["merge"]),
                c("sideLive", &["sideBase"]),
                c("merge", &["main", "feat"]),
                c("main", &["root"]),
                c("feat", &["root"]),
                c("sideBase", &["root"]),
                c("root", &[]),
            ],
            &UNICODE,
        );
        let merge = rows.iter().find(|r| r.commit.id == "merge").unwrap();
        assert!(
            merge.edges.contains('┼'),
            "expected cross, got {:?}",
            merge.edges
        );
    }

    #[test]
    fn ascii_junctions_share_topology() {
        let commits = [
            c("after", &["merge"]),
            c("sideLive", &["sideBase"]),
            c("merge", &["main", "feat"]),
            c("main", &["root"]),
            c("feat", &["root"]),
            c("sideBase", &["root"]),
            c("root", &[]),
        ];
        let uni = layout_commits(&commits, &UNICODE);
        let asc = layout_commits(&commits, &ASCII);
        let uni_merge = uni.iter().find(|r| r.commit.id == "merge").unwrap();
        let asc_merge = asc.iter().find(|r| r.commit.id == "merge").unwrap();
        assert!(uni_merge.edges.contains('┼'), "{}", uni_merge.edges);
        assert!(
            asc_merge.edges.contains('+'),
            "ASCII should map the same cross to +, got {:?}",
            asc_merge.edges
        );
        assert!(asc_merge.edges.contains('*'), "{}", asc_merge.edges);
        assert_eq!(uni_merge.lane, asc_merge.lane);
    }
}

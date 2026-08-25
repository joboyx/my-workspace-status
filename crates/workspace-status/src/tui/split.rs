//! Session-only pane and in-diff split math.
//!
//! Pane split, hit-testing, and side-by-side column math.
//! Fractions are not written to disk; the next launch resets to the defaults.

/// Default tree/right split.
pub const TREE_WIDTH_FRACTION: f64 = 0.4;

/// Default in-diff split.
pub const DIFF_SPLIT_FRACTION: f64 = 0.5;

/// Minimum outer width for either tree/right pane.
pub const MIN_PANE_COLS: u16 = 20;

/// Right-pane horizontal padding (one column each side).
pub const DIFF_PAD_X: u16 = 2;

/// Below this right-pane content width, side-by-side falls back to inline.
pub const NARROW_SXS: u16 = 100;

/// Minimum width of either side-by-side column when the pane is wide enough.
pub const MIN_DIFF_COL: u16 = 16;

/// Columns after the tree border before right-pane content starts.
pub const DIFF_CONTENT_PAD: u16 = 2;

/// Preferred diff layout. Narrow panes still paint inline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffMode {
    #[default]
    SideBySide,
    Inline,
}

/// Which drag handle a mouse cell hits, if any.
///
/// Graph scrollbar hits reuse this enum so click / drag / release stay on
/// one mouse stack with the pane and in-diff splitters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitHit {
    Pane,
    DiffSplit,
    /// Painted graph scrollbar thumb (`█`).
    GraphThumb,
    /// Graph scrollbar track (not the thumb).
    GraphTrack,
    Other,
}

/// Active left-button drag, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitDrag {
    #[default]
    None,
    Pane,
    Diff,
    /// Graph list scrollbar. Row delta maps onto `graph_scroll`.
    GraphScrollbar {
        /// Mouse row at mouse-down (or after a track jump).
        origin_row: u16,
        /// `graph_scroll` at that origin.
        origin_scroll: u16,
    },
}

/// Frozen pane widths from terminal columns + session fraction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaneWidths {
    pub tree_width: u16,
    pub tree_inner_width: u16,
    pub diff_width: u16,
}

/// Left / right column widths for one side-by-side row (`left + RULE + right`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SideBySideWidths {
    pub left_width: u16,
    pub right_width: u16,
}

/// Geometry used by splitter hit-test. Coordinates are 0-based (crossterm).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SplitLayout {
    pub term_cols: u16,
    pub term_rows: u16,
    pub pane_height: u16,
    /// Outer tree box width.
    pub tree_width: u16,
    /// Right-pane content width (inner).
    pub diff_pane_width: u16,
    /// 0-based first content column of the right pane.
    pub diff_content_x: u16,
    /// 0-based RULE column when a split is painted.
    pub diff_split_rule_x: Option<u16>,
    /// 0-based graph scrollbar column when a graph list is painted.
    pub graph_scrollbar_x: Option<u16>,
    /// 0-based first row of the graph scrollbar track (list, not header/footer).
    pub graph_scrollbar_y: u16,
    /// Graph list height (scrollbar track).
    pub graph_scrollbar_height: u16,
    /// Painted graph line count (`paint_model` length).
    pub graph_content_len: usize,
    /// Current graph list skip (`graph_scroll`).
    pub graph_scroll: u16,
}

/// One paired side-by-side row from a unified diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitRow {
    pub left: String,
    pub right: String,
}

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else {
        fallback
    }
}

/// Clamp an outer tree width so both panes stay ≥ [`MIN_PANE_COLS`].
pub fn clamp_tree_width(term_cols: u16, tree_width: u16) -> u16 {
    let cols = term_cols.max(1);
    let min_tree = MIN_PANE_COLS;
    let max_tree = min_tree.max(cols.saturating_sub(MIN_PANE_COLS + DIFF_PAD_X));
    tree_width.clamp(min_tree, max_tree)
}

/// Convert an outer tree width into a fraction of `term_cols` (after clamp).
pub fn tree_fraction_from_width(term_cols: u16, tree_width: u16) -> f64 {
    let cols = term_cols.max(1);
    f64::from(clamp_tree_width(cols, tree_width)) / f64::from(cols)
}

/// Map a 0-based mouse column onto a tree-width fraction.
pub fn tree_fraction_from_col(term_cols: u16, col: u16) -> f64 {
    tree_fraction_from_width(term_cols, col.saturating_add(1))
}

/// Clamp a tree-width fraction so both panes stay ≥ [`MIN_PANE_COLS`].
pub fn clamp_tree_fraction(term_cols: u16, fraction: f64) -> f64 {
    let cols = term_cols.max(1);
    let raw = finite_or(fraction, TREE_WIDTH_FRACTION);
    let desired = (f64::from(cols) * raw).floor().max(0.0) as u16;
    tree_fraction_from_width(cols, desired)
}

/// Outer tree width, inner content width, and right-pane content width.
pub fn pane_widths(term_cols: u16, fraction: f64) -> PaneWidths {
    let cols = term_cols.max(1);
    let raw = finite_or(fraction, TREE_WIDTH_FRACTION);
    let desired = (f64::from(cols) * raw).floor().max(0.0) as u16;
    let tree_width = clamp_tree_width(cols, desired);
    let tree_inner_width = tree_width.saturating_sub(3).max(8);
    let diff_width = cols
        .saturating_sub(tree_width)
        .saturating_sub(DIFF_PAD_X)
        .max(MIN_PANE_COLS);
    PaneWidths {
        tree_width,
        tree_inner_width,
        diff_width,
    }
}

/// True when the right pane is painting a side-by-side split.
pub fn is_side_by_side_split(mode: DiffMode, pane_width: u16) -> bool {
    mode == DiffMode::SideBySide && pane_width >= NARROW_SXS
}

/// Preferred mode after the narrow-pane fallback.
pub fn effective_diff_mode(mode: DiffMode, pane_width: u16) -> DiffMode {
    if is_side_by_side_split(mode, pane_width) {
        DiffMode::SideBySide
    } else {
        DiffMode::Inline
    }
}

/// 1-based terminal column of the first right-pane content cell.
pub fn diff_content_origin_x(tree_width: u16) -> u16 {
    tree_width.saturating_add(DIFF_CONTENT_PAD)
}

/// 1-based terminal column of the in-diff vertical RULE.
pub fn diff_split_rule_x(tree_width: u16, left_width: u16) -> u16 {
    diff_content_origin_x(tree_width).saturating_add(left_width)
}

/// Clamp a left-column width so both sides stay ≥ [`MIN_DIFF_COL`] when possible.
pub fn clamp_diff_left_width(pane_width: u16, left_width: i32) -> u16 {
    let inner = (i32::from(pane_width) - 1).max(1);
    let min_col = i32::from(MIN_DIFF_COL).min(inner / 2).max(1);
    let max_col = min_col.max(inner - min_col);
    let raw = if left_width == i32::MIN {
        (inner as f64 * DIFF_SPLIT_FRACTION).round() as i32
    } else {
        left_width
    };
    raw.clamp(min_col, max_col) as u16
}

/// Convert a left-column width into a fraction of the inner (pane − RULE) width.
pub fn diff_split_fraction_from_left_width(pane_width: u16, left_width: i32) -> f64 {
    let inner = (i32::from(pane_width) - 1).max(1) as f64;
    f64::from(clamp_diff_left_width(pane_width, left_width)) / inner
}

/// Map a 0-based mouse column onto an in-diff split fraction.
pub fn diff_split_fraction_from_col(tree_width: u16, pane_width: u16, col: u16) -> f64 {
    let origin = i32::from(diff_content_origin_x(tree_width));
    let x_1based = i32::from(col) + 1;
    diff_split_fraction_from_left_width(pane_width, x_1based - origin)
}

/// Left / right column widths for one side-by-side diff row.
pub fn side_by_side_column_widths(pane_width: u16, fraction: f64) -> SideBySideWidths {
    let inner = (i32::from(pane_width) - 1).max(1);
    let raw = finite_or(fraction, DIFF_SPLIT_FRACTION);
    let desired = (inner as f64 * raw).floor() as i32;
    let left_width = clamp_diff_left_width(pane_width, desired);
    let right_width = (inner as u16).saturating_sub(left_width);
    SideBySideWidths {
        left_width,
        right_width,
    }
}

/// True when `x` (1-based) is on/near `center` (`center ± 1`), inside the terminal.
pub fn is_divider_column(x: u16, center: u16, term_cols: u16) -> bool {
    if x < 1 || x > term_cols {
        return false;
    }
    for delta in [0i32, -1, 1] {
        let col = i32::from(center) + delta;
        if col >= 1 && col <= i32::from(term_cols) && col == i32::from(x) {
            return true;
        }
    }
    false
}

/// Max `graph_scroll` so the last painted lines can sit in the viewport.
pub fn graph_scroll_max(content_len: usize, list_height: u16) -> usize {
    content_len.saturating_sub(list_height.max(1) as usize)
}

/// Map a 0-based mouse row onto `graph_scroll` (track-fraction jump).
pub fn graph_scroll_from_row(layout: SplitLayout, row: u16) -> u16 {
    graph_scroll_from_delta(layout, layout.graph_scrollbar_y, 0, row)
}

/// Map a drag (`origin_row` / `origin_scroll` plus current `row`) onto `graph_scroll`.
pub fn graph_scroll_from_delta(
    layout: SplitLayout,
    origin_row: u16,
    origin_scroll: u16,
    row: u16,
) -> u16 {
    let max = graph_scroll_max(layout.graph_content_len, layout.graph_scrollbar_height) as i32;
    if max == 0 {
        return 0;
    }
    let denom = i32::from(layout.graph_scrollbar_height.saturating_sub(1).max(1));
    let delta = i32::from(row).saturating_sub(i32::from(origin_row));
    (i32::from(origin_scroll) + delta * max / denom).clamp(0, max) as u16
}

fn hit_graph_scrollbar(layout: SplitLayout, col: u16, row: u16) -> Option<SplitHit> {
    let x = layout.graph_scrollbar_x?;
    if col != x || layout.graph_scrollbar_height == 0 {
        return None;
    }
    if row < layout.graph_scrollbar_y {
        return None;
    }
    let rel = row.saturating_sub(layout.graph_scrollbar_y);
    if rel >= layout.graph_scrollbar_height {
        return None;
    }
    if let Some((thumb_off, thumb_len)) = workspace_status_graph::graph_scrollbar_thumb(
        layout.graph_content_len,
        layout.graph_scroll,
        layout.graph_scrollbar_height,
    ) {
        if rel >= thumb_off && rel < thumb_off.saturating_add(thumb_len) {
            return Some(SplitHit::GraphThumb);
        }
    }
    Some(SplitHit::GraphTrack)
}

/// Map a 0-based mouse cell onto a drag handle.
///
/// Graph scrollbar (exact column, list track) wins over the 3-column pane
/// divider band so a left-pane graph thumb stays draggable. Then pane, then
/// in-diff RULE.
pub fn hit_split(layout: SplitLayout, col: u16, row: u16) -> SplitHit {
    let x = col.saturating_add(1);
    let y = row.saturating_add(1);
    if x < 1 || y < 1 || x > layout.term_cols || y > layout.term_rows {
        return SplitHit::Other;
    }
    if y > layout.pane_height {
        return SplitHit::Other;
    }
    if let Some(hit) = hit_graph_scrollbar(layout, col, row) {
        return hit;
    }
    if is_divider_column(x, layout.tree_width, layout.term_cols) {
        return SplitHit::Pane;
    }
    if let Some(rule_x) = layout.diff_split_rule_x {
        let rule_1based = rule_x.saturating_add(1);
        if rule_1based >= 1 && is_divider_column(x, rule_1based, layout.term_cols) {
            return SplitHit::DiffSplit;
        }
    }
    SplitHit::Other
}

#[allow(dead_code)]
fn unified_kind(line: &str) -> &'static str {
    if line.starts_with("+++")
        || line.starts_with("---")
        || line.starts_with("diff ")
        || line.starts_with("index ")
        || line.starts_with("new file")
        || line.starts_with("deleted file")
        || line.starts_with("@@")
        || line == "staged"
        || line == "unstaged"
        || line.starts_with("untracked")
        || line == "(no diff)"
        || line.starts_with("no diff")
    {
        "meta"
    } else if line.starts_with('+') {
        "add"
    } else if line.starts_with('-') {
        "del"
    } else {
        "ctx"
    }
}

/// Zip unified-diff lines into side-by-side pairs.
#[allow(dead_code)]
pub fn pair_unified_lines(lines: &[String]) -> Vec<SplitRow> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let kind = unified_kind(&lines[i]);
        if kind == "meta" {
            out.push(SplitRow {
                left: lines[i].clone(),
                right: String::new(),
            });
            i += 1;
            continue;
        }
        if kind == "ctx" {
            out.push(SplitRow {
                left: lines[i].clone(),
                right: lines[i].clone(),
            });
            i += 1;
            continue;
        }
        let mut dels = Vec::new();
        let mut adds = Vec::new();
        while i < lines.len() && unified_kind(&lines[i]) == "del" {
            dels.push(lines[i].clone());
            i += 1;
        }
        while i < lines.len() && unified_kind(&lines[i]) == "add" {
            adds.push(lines[i].clone());
            i += 1;
        }
        if dels.is_empty() && adds.is_empty() {
            i += 1;
            continue;
        }
        let n = dels.len().max(adds.len());
        for j in 0..n {
            out.push(SplitRow {
                left: dels.get(j).cloned().unwrap_or_default(),
                right: adds.get(j).cloned().unwrap_or_default(),
            });
        }
    }
    out
}

/// Visible columns of `text`, padded or truncated to `width`.
#[allow(dead_code)]
pub fn pad_trunc(text: &str, width: u16) -> String {
    let width = width as usize;
    let chars: Vec<char> = text.chars().collect();
    if chars.len() >= width {
        chars.into_iter().take(width).collect()
    } else {
        let mut out: String = chars.into_iter().collect();
        out.push_str(&" ".repeat(width - out.chars().count()));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pane_widths_default_fraction_depends_only_on_cols() {
        let a = pane_widths(120, TREE_WIDTH_FRACTION);
        let b = pane_widths(120, TREE_WIDTH_FRACTION);
        assert_eq!(a, b);
        assert_eq!(a.tree_width, 48);
        assert_eq!(a.tree_inner_width, 45);
        assert_eq!(a.diff_width, 70);
    }

    #[test]
    fn clamp_keeps_both_panes_above_min() {
        let cols = 100;
        let too_wide = pane_widths(cols, clamp_tree_fraction(cols, 0.95));
        let too_narrow = pane_widths(cols, clamp_tree_fraction(cols, 0.05));
        assert!(too_wide.tree_width >= MIN_PANE_COLS);
        assert!(too_wide.diff_width >= MIN_PANE_COLS);
        assert!(too_narrow.tree_width >= MIN_PANE_COLS);
        assert!(too_narrow.diff_width >= MIN_PANE_COLS);
        assert!(too_wide.tree_width <= cols - MIN_PANE_COLS - DIFF_PAD_X);
    }

    #[test]
    fn tree_fraction_from_width_round_trips() {
        let cols = 120;
        let fraction = tree_fraction_from_width(cols, 55);
        assert_eq!(pane_widths(cols, fraction).tree_width, 55);
    }

    #[test]
    fn tree_fraction_from_col_uses_one_based_width() {
        let cols = 120;
        let fraction = tree_fraction_from_col(cols, 54);
        assert_eq!(pane_widths(cols, fraction).tree_width, 55);
    }

    #[test]
    fn side_by_side_default_is_even_split() {
        for width in [100u16, 117, 118, 141] {
            let w = side_by_side_column_widths(width, DIFF_SPLIT_FRACTION);
            assert_eq!(w.left_width, (width - 1) / 2, "left width={width}");
            assert_eq!(w.left_width + 1 + w.right_width, width, "sum width={width}");
        }
    }

    #[test]
    fn side_by_side_honours_fraction_and_clamps() {
        let wide = side_by_side_column_widths(120, 0.7);
        let narrow = side_by_side_column_widths(120, 0.3);
        assert!(wide.left_width > narrow.left_width);
        let too_wide = side_by_side_column_widths(120, 0.95);
        let too_narrow = side_by_side_column_widths(120, 0.05);
        assert!(too_wide.left_width >= MIN_DIFF_COL);
        assert!(too_wide.right_width >= MIN_DIFF_COL);
        assert!(too_narrow.left_width >= MIN_DIFF_COL);
        assert!(too_narrow.right_width >= MIN_DIFF_COL);
        assert!(too_wide.left_width > 0 && too_wide.right_width > 0);
        assert!(too_narrow.left_width > 0 && too_narrow.right_width > 0);
    }

    #[test]
    fn diff_split_fraction_round_trips() {
        let pane = 120;
        let fraction = diff_split_fraction_from_left_width(pane, 70);
        assert_eq!(side_by_side_column_widths(pane, fraction).left_width, 70);
    }

    #[test]
    fn is_side_by_side_split_needs_width() {
        assert!(is_side_by_side_split(DiffMode::SideBySide, NARROW_SXS));
        assert!(!is_side_by_side_split(DiffMode::SideBySide, NARROW_SXS - 1));
        assert!(!is_side_by_side_split(DiffMode::Inline, 200));
        assert_eq!(
            effective_diff_mode(DiffMode::SideBySide, NARROW_SXS - 1),
            DiffMode::Inline
        );
        assert_eq!(
            effective_diff_mode(DiffMode::SideBySide, NARROW_SXS),
            DiffMode::SideBySide
        );
    }

    fn wide_layout(rule: Option<u16>) -> SplitLayout {
        SplitLayout {
            term_cols: 160,
            term_rows: 24,
            pane_height: 22,
            tree_width: 48,
            diff_pane_width: 110,
            diff_content_x: 50,
            diff_split_rule_x: rule,
            graph_scrollbar_x: None,
            graph_scrollbar_y: 0,
            graph_scrollbar_height: 0,
            graph_content_len: 0,
            graph_scroll: 0,
        }
    }

    fn graph_sb_layout() -> SplitLayout {
        SplitLayout {
            term_cols: 160,
            term_rows: 24,
            pane_height: 22,
            tree_width: 48,
            diff_pane_width: 110,
            diff_content_x: 50,
            diff_split_rule_x: None,
            graph_scrollbar_x: Some(158),
            graph_scrollbar_y: 2,
            graph_scrollbar_height: 10,
            graph_content_len: 40,
            graph_scroll: 0,
        }
    }

    #[test]
    fn hit_test_pane_splitter_vs_in_diff() {
        let left = side_by_side_column_widths(110, 0.5).left_width;
        let rule_0 = 50 + left;
        let layout = wide_layout(Some(rule_0));
        // 0-based col 47 is 1-based 48 = treeWidth (pane divider).
        assert_eq!(hit_split(layout, 47, 5), SplitHit::Pane);
        assert_eq!(hit_split(layout, 46, 5), SplitHit::Pane);
        assert_eq!(hit_split(layout, 48, 5), SplitHit::Pane);
        assert_eq!(hit_split(layout, rule_0, 5), SplitHit::DiffSplit);
        assert_eq!(
            hit_split(layout, rule_0.saturating_sub(1), 5),
            SplitHit::DiffSplit
        );
        assert_eq!(hit_split(layout, rule_0 + 1, 5), SplitHit::DiffSplit);
        assert_eq!(hit_split(layout, 10, 5), SplitHit::Other);
        assert_eq!(hit_split(layout, 100, 5), SplitHit::Other);
        assert_eq!(hit_split(layout, 47, 23), SplitHit::Other);
    }

    #[test]
    fn hit_test_skips_in_diff_when_rule_absent() {
        let layout = wide_layout(None);
        assert_eq!(hit_split(layout, 47, 5), SplitHit::Pane);
        assert_eq!(hit_split(layout, 100, 5), SplitHit::Other);
    }

    #[test]
    fn pane_divider_wins_over_in_diff_when_bands_overlap() {
        let layout = SplitLayout {
            term_cols: 80,
            term_rows: 20,
            pane_height: 18,
            tree_width: 48,
            diff_pane_width: 110,
            diff_content_x: 50,
            diff_split_rule_x: Some(47),
            graph_scrollbar_x: None,
            graph_scrollbar_y: 0,
            graph_scrollbar_height: 0,
            graph_content_len: 0,
            graph_scroll: 0,
        };
        assert_eq!(hit_split(layout, 47, 4), SplitHit::Pane);
    }

    #[test]
    fn hit_test_graph_scrollbar_thumb_and_track() {
        let layout = graph_sb_layout();
        let thumb = workspace_status_graph::graph_scrollbar_thumb(40, 0, 10).expect("thumb");
        let thumb_row = layout.graph_scrollbar_y + thumb.0;
        assert_eq!(hit_split(layout, 158, thumb_row), SplitHit::GraphThumb);
        let track_row = layout
            .graph_scrollbar_y
            .saturating_add(layout.graph_scrollbar_height.saturating_sub(1));
        if track_row != thumb_row {
            assert_eq!(hit_split(layout, 158, track_row), SplitHit::GraphTrack);
        }
        assert_eq!(hit_split(layout, 157, thumb_row), SplitHit::Other);
        assert_eq!(hit_split(layout, 158, 0), SplitHit::Other);
    }

    #[test]
    fn graph_scrollbar_column_wins_over_pane_divider_band() {
        let layout = SplitLayout {
            term_cols: 80,
            term_rows: 20,
            pane_height: 18,
            tree_width: 48,
            diff_pane_width: 30,
            diff_content_x: 50,
            diff_split_rule_x: None,
            // Left-pane graph: last inner column overlaps the ±1 pane band.
            graph_scrollbar_x: Some(46),
            graph_scrollbar_y: 1,
            graph_scrollbar_height: 10,
            graph_content_len: 40,
            graph_scroll: 0,
        };
        assert!(
            matches!(
                hit_split(layout, 46, 4),
                SplitHit::GraphThumb | SplitHit::GraphTrack
            ),
            "exact scrollbar column must win over the pane divider band"
        );
        assert_eq!(hit_split(layout, 47, 4), SplitHit::Pane);
    }

    #[test]
    fn graph_scroll_from_row_jumps_toward_track_and_drag_delta() {
        let layout = graph_sb_layout();
        let jumped = graph_scroll_from_row(layout, layout.graph_scrollbar_y + 9);
        assert!(jumped > 0, "bottom of track should scroll down: {jumped}");
        assert_eq!(
            jumped as usize,
            graph_scroll_max(layout.graph_content_len, layout.graph_scrollbar_height)
        );
        let dragged = graph_scroll_from_delta(
            layout,
            layout.graph_scrollbar_y,
            0,
            layout.graph_scrollbar_y + 5,
        );
        assert!(
            dragged > 0 && dragged < jumped,
            "mid drag {dragged} jump {jumped}"
        );
        assert_eq!(
            graph_scroll_from_delta(layout, 4, 3, 4),
            3,
            "zero delta keeps origin scroll"
        );
    }

    #[test]
    fn pair_unified_lines_zips_del_add_runs() {
        let lines = [
            "@@ -1,2 +1,2 @@".into(),
            "-old".into(),
            "+new".into(),
            " context".into(),
        ];
        let rows = pair_unified_lines(&lines);
        assert_eq!(rows[0].left, "@@ -1,2 +1,2 @@");
        assert_eq!(rows[0].right, "");
        assert_eq!(rows[1].left, "-old");
        assert_eq!(rows[1].right, "+new");
        assert_eq!(rows[2].left, " context");
        assert_eq!(rows[2].right, " context");
    }
}

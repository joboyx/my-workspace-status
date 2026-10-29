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

/// Below this painted diff width ([`diff_paint_width`]), side-by-side falls back to inline.
pub const NARROW_SXS: u16 = 100;

/// Right-pane columns a file diff keeps for its vertical scrollbar when it
/// picks split or inline, whether or not the bar shows.
pub const DIFF_VSCROLL_COLS: u16 = 1;

/// Narrowest terminal the panes paint in. The help overlay keeps every
/// description at least `HELP_MIN_DESC_WIDTH` wide from 46 columns, and
/// both panes need `2 × MIN_PANE_COLS + DIFF_PAD_X` (42). Narrower, the
/// TUI paints a resize notice instead.
pub const MIN_TERM_COLS: u16 = 46;

/// Shortest terminal the panes paint in: the tab strip, breadcrumb, status
/// row, and pane borders leave a graph list with its selection footer.
pub const MIN_TERM_ROWS: u16 = 12;

/// Tree / right split step for `<` / `>` (fraction of the terminal width).
pub const TREE_FRACTION_STEP: f64 = 0.05;

/// Minimum width of either side-by-side column when the pane is wide enough.
pub const MIN_DIFF_COL: u16 = 16;

/// Preferred diff layout. Narrow panes still paint inline.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DiffMode {
    #[default]
    SideBySide,
    Inline,
}

/// Which drag handle a mouse cell hits, if any.
///
/// Graph and file-diff scrollbar hits reuse this enum so click / drag /
/// release stay on one mouse stack with the pane and in-diff splitters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SplitHit {
    Pane,
    DiffSplit,
    /// Top rule row of an expanded commit-message footer (graph or depth-2
    /// commit files).
    CommitMsg,
    /// Painted graph scrollbar thumb (`█`).
    GraphThumb,
    /// Graph scrollbar track (not the thumb).
    GraphTrack,
    /// Painted graph horizontal scrollbar thumb.
    GraphHThumb,
    /// Graph horizontal scrollbar track (not the thumb).
    GraphHTrack,
    /// Painted file-diff horizontal scrollbar thumb.
    DiffHThumb,
    /// File-diff horizontal scrollbar track (not the thumb).
    DiffHTrack,
    Other,
}

/// A pane edge the mouse drags to resize, for the hover highlight.
///
/// Scrollbars are not resize edges: they scroll or pan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResizeEdge {
    /// The tree / right pane divider ([`SplitHit::Pane`], [`SplitDrag::Pane`]).
    Pane,
    /// The side-by-side diff RULE ([`SplitHit::DiffSplit`], [`SplitDrag::Diff`]).
    DiffSplit,
    /// The top rule of an expanded commit-message footer (graph or depth-2)
    /// ([`SplitHit::CommitMsg`], [`SplitDrag::CommitMsg`]). It moves up and
    /// down, where the other edges move left and right.
    CommitMsg,
}

/// Active left-button drag, if any.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SplitDrag {
    #[default]
    None,
    Pane,
    Diff,
    /// Commit-message footer top rule. Row delta maps onto the footer
    /// message rows: up adds rows, down removes them.
    CommitMsg {
        /// Mouse row at mouse-down.
        origin_row: u16,
        /// Commit-message rows at mouse-down.
        origin_lines: usize,
    },
    /// Graph list scrollbar. Row delta maps onto `graph_scroll`.
    GraphScrollbar {
        /// Mouse row at mouse-down (or after a track jump).
        origin_row: u16,
        /// `graph_scroll` at that origin.
        origin_scroll: u16,
    },
    /// Graph horizontal scrollbar. Column delta maps onto `col_offset`.
    GraphHScrollbar {
        /// Mouse column at mouse-down (or after a track jump).
        origin_col: u16,
        /// Horizontal pan offset at that origin.
        origin_offset: u16,
    },
    /// File-diff horizontal scrollbar. Column delta maps onto `diff_col_offset`.
    DiffHScrollbar {
        /// Mouse column at mouse-down (or after a track jump).
        origin_col: u16,
        /// Horizontal pan offset at that origin.
        origin_offset: u16,
    },
}

impl SplitHit {
    /// The resize edge this hit lands on, or `None` for a scrollbar or
    /// [`SplitHit::Other`].
    pub fn resize_edge(self) -> Option<ResizeEdge> {
        match self {
            Self::Pane => Some(ResizeEdge::Pane),
            Self::DiffSplit => Some(ResizeEdge::DiffSplit),
            Self::CommitMsg => Some(ResizeEdge::CommitMsg),
            Self::GraphThumb
            | Self::GraphTrack
            | Self::GraphHThumb
            | Self::GraphHTrack
            | Self::DiffHThumb
            | Self::DiffHTrack
            | Self::Other => None,
        }
    }
}

impl SplitDrag {
    /// The resize edge this drag moves, or `None` for a scrollbar drag or
    /// no drag.
    pub fn resize_edge(self) -> Option<ResizeEdge> {
        match self {
            Self::Pane => Some(ResizeEdge::Pane),
            Self::Diff => Some(ResizeEdge::DiffSplit),
            Self::CommitMsg { .. } => Some(ResizeEdge::CommitMsg),
            Self::None
            | Self::GraphScrollbar { .. }
            | Self::GraphHScrollbar { .. }
            | Self::DiffHScrollbar { .. } => None,
        }
    }
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
    /// Outer left-pane width (its border columns included when boxed).
    pub tree_width: u16,
    /// Right-pane content width (inner).
    pub diff_pane_width: u16,
    /// 0-based first content column of the right pane, as painted.
    pub diff_content_x: u16,
    /// Panes paint flat (no border glyphs). The pane divider band is then
    /// the two columns at the pane boundary (the left pane's right pad cell
    /// and the right pane's left pad cell), and the pane's last row is
    /// content (a horizontal scrollbar can sit there).
    pub flat_panes: bool,
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
    /// 0-based graph horizontal scrollbar row when the bar is painted.
    pub graph_hscrollbar_y: Option<u16>,
    /// 0-based first column of the graph horizontal scrollbar track.
    pub graph_hscrollbar_x: u16,
    /// Graph horizontal scrollbar track width.
    pub graph_hscrollbar_width: u16,
    /// Max horizontal pan for the painted graph.
    pub graph_col_max: u16,
    /// Current graph horizontal pan (`col_offset`).
    pub graph_col_offset: u16,
    /// 0-based file-diff horizontal scrollbar row when the bar is painted.
    pub diff_hscrollbar_y: Option<u16>,
    /// 0-based first column of the file-diff horizontal scrollbar track.
    pub diff_hscrollbar_x: u16,
    /// File-diff horizontal scrollbar track width.
    pub diff_hscrollbar_width: u16,
    /// Max horizontal pan for the painted file diff.
    pub diff_col_max: u16,
    /// Current file-diff horizontal pan (`diff_col_offset`).
    pub diff_col_offset: u16,
    /// 0-based row of the expanded commit-message footer's top rule (graph
    /// or depth-2 commit files) when it is painted ([`SplitHit::CommitMsg`]).
    pub commit_msg_rule_y: Option<u16>,
    /// 0-based first column of that rule.
    pub commit_msg_rule_x: u16,
    /// Rule width (the pane width).
    pub commit_msg_rule_width: u16,
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
    tree_fraction_from_width(cols, fraction_cols(cols, fraction))
}

/// Whole columns for `fraction` of `cols`. The epsilon keeps a width that
/// went through [`tree_fraction_from_width`] on the same column (`w / c * c`
/// can land a hair under `w` in floating point).
fn fraction_cols(cols: u16, fraction: f64) -> u16 {
    let raw = finite_or(fraction, TREE_WIDTH_FRACTION);
    (f64::from(cols) * raw + 1e-9).floor().max(0.0) as u16
}

/// Outer tree width, inner content width, and right-pane content width.
pub fn pane_widths(term_cols: u16, fraction: f64) -> PaneWidths {
    let cols = term_cols.max(1);
    let tree_width = clamp_tree_width(cols, fraction_cols(cols, fraction));
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

/// Widest row a file diff paints in a `pane_width`-column right pane: the
/// pane less [`DIFF_VSCROLL_COLS`]. The split / inline choice reads this.
pub fn diff_paint_width(pane_width: u16) -> u16 {
    pane_width.saturating_sub(DIFF_VSCROLL_COLS).max(1)
}

/// Layout a file diff in a `pane_width`-column right pane uses.
///
/// The one split decision. Row build, the header label, the status pill,
/// and paint all read it, so rows built split are never painted inline.
pub fn diff_pane_mode(mode: DiffMode, pane_width: u16) -> DiffMode {
    effective_diff_mode(mode, diff_paint_width(pane_width))
}

/// Tree fraction after `steps` keyboard steps of [`TREE_FRACTION_STEP`].
///
/// Moves the outer tree width by whole columns from its current width, so
/// a step never rounds back to the same column, then clamps like a drag.
pub fn step_tree_fraction(term_cols: u16, fraction: f64, steps: i32) -> f64 {
    let cols = term_cols.max(1);
    let current = pane_widths(cols, fraction).tree_width;
    let step = (f64::from(cols) * TREE_FRACTION_STEP).round().max(1.0) as i32;
    let target = (i32::from(current) + steps * step).clamp(0, i32::from(u16::MAX)) as u16;
    tree_fraction_from_width(cols, target)
}

/// Columns a file-diff row paints before its text: the cursor bar (`▌` /
/// blank).
pub const DIFF_CURSOR_BAR_COLS: u16 = 1;

/// Width a side-by-side row splits (`left + RULE + right`) in a
/// `pane_width`-column right pane: [`diff_paint_width`] less the
/// [`DIFF_CURSOR_BAR_COLS`] cursor bar.
///
/// The row paint, the recorded RULE column ([`diff_split_rule_x`]), and a
/// RULE drag ([`diff_split_fraction_from_col`]) all size the columns from
/// this width, so the recorded column is the painted one at every fraction.
pub fn diff_split_row_width(pane_width: u16) -> u16 {
    diff_paint_width(pane_width)
        .saturating_sub(DIFF_CURSOR_BAR_COLS)
        .max(1)
}

/// 0-based terminal column of the in-diff vertical RULE as painted.
///
/// `content_x` is the 0-based first content column of the right pane as
/// painted (`LayoutHit::diff_content_x`), so the result is right for boxed
/// and flat panes alike. The RULE follows the cursor bar and the left
/// column of a [`diff_split_row_width`]-wide row.
pub fn diff_split_rule_x(content_x: u16, pane_width: u16, fraction: f64) -> u16 {
    let left = side_by_side_column_widths(diff_split_row_width(pane_width), fraction).left_width;
    content_x
        .saturating_add(DIFF_CURSOR_BAR_COLS)
        .saturating_add(left)
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
///
/// `content_x` is the 0-based first content column of the right pane as
/// painted and `pane_width` its width; the inverse of
/// [`diff_split_rule_x`], so the RULE lands under the pointer.
pub fn diff_split_fraction_from_col(content_x: u16, pane_width: u16, col: u16) -> f64 {
    let left = i32::from(col) - i32::from(content_x) - i32::from(DIFF_CURSOR_BAR_COLS);
    diff_split_fraction_from_left_width(diff_split_row_width(pane_width), left)
}

/// Left / right column widths for one side-by-side diff row.
pub fn side_by_side_column_widths(pane_width: u16, fraction: f64) -> SideBySideWidths {
    let inner = (i32::from(pane_width) - 1).max(1);
    let raw = finite_or(fraction, DIFF_SPLIT_FRACTION);
    // The epsilon keeps a width that went through
    // [`diff_split_fraction_from_left_width`] on the same column, as in
    // `fraction_cols`: a press on the RULE must not move it.
    let desired = (inner as f64 * raw + 1e-9).floor() as i32;
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

/// True when `x` (1-based) is on the pane divider grab band.
///
/// Boxed panes: the 3-column band around the left pane's right border
/// ([`is_divider_column`] on `tree_width`). Flat panes: the two columns at
/// the pane boundary, the left pane's last column and the right pane's
/// first (the pad cells beside the boundary, so the band covers no row
/// text).
fn is_pane_divider(layout: SplitLayout, x: u16) -> bool {
    if !layout.flat_panes {
        return is_divider_column(x, layout.tree_width, layout.term_cols);
    }
    let left_last = layout.tree_width;
    let right_first = left_last.saturating_add(1);
    x >= 1 && x <= layout.term_cols && (x == left_last || x == right_first)
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

/// Map a 0-based mouse column onto `col_offset` (track-fraction jump).
pub fn graph_col_from_col(layout: SplitLayout, col: u16) -> u16 {
    graph_col_from_delta(layout, layout.graph_hscrollbar_x, 0, col)
}

/// Map a drag (`origin_col` / `origin_offset` plus current `col`) onto pan.
pub fn graph_col_from_delta(
    layout: SplitLayout,
    origin_col: u16,
    origin_offset: u16,
    col: u16,
) -> u16 {
    let max = i32::from(layout.graph_col_max);
    if max == 0 {
        return 0;
    }
    let denom = i32::from(layout.graph_hscrollbar_width.saturating_sub(1).max(1));
    let delta = i32::from(col).saturating_sub(i32::from(origin_col));
    (i32::from(origin_offset) + delta * max / denom).clamp(0, max) as u16
}

/// Map a 0-based mouse column onto `diff_col_offset` (track-fraction jump).
pub fn diff_col_from_col(layout: SplitLayout, col: u16) -> u16 {
    diff_col_from_delta(layout, layout.diff_hscrollbar_x, 0, col)
}

/// Map a drag (`origin_col` / `origin_offset` plus current `col`) onto pan.
pub fn diff_col_from_delta(
    layout: SplitLayout,
    origin_col: u16,
    origin_offset: u16,
    col: u16,
) -> u16 {
    let max = i32::from(layout.diff_col_max);
    if max == 0 {
        return 0;
    }
    let denom = i32::from(layout.diff_hscrollbar_width.saturating_sub(1).max(1));
    let delta = i32::from(col).saturating_sub(i32::from(origin_col));
    (i32::from(origin_offset) + delta * max / denom).clamp(0, max) as u16
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

fn hit_graph_hscrollbar(layout: SplitLayout, col: u16, row: u16) -> Option<SplitHit> {
    let y = layout.graph_hscrollbar_y?;
    if row != y || layout.graph_hscrollbar_width == 0 {
        return None;
    }
    if col < layout.graph_hscrollbar_x {
        return None;
    }
    let rel = col.saturating_sub(layout.graph_hscrollbar_x);
    if rel >= layout.graph_hscrollbar_width {
        return None;
    }
    if let Some((thumb_off, thumb_len)) = workspace_status_graph::graph_scrollbar_thumb(
        (layout.graph_col_max as usize).saturating_add(1),
        layout.graph_col_offset,
        layout.graph_hscrollbar_width,
    ) {
        if rel >= thumb_off && rel < thumb_off.saturating_add(thumb_len) {
            return Some(SplitHit::GraphHThumb);
        }
    }
    Some(SplitHit::GraphHTrack)
}

fn hit_diff_hscrollbar(layout: SplitLayout, col: u16, row: u16) -> Option<SplitHit> {
    let y = layout.diff_hscrollbar_y?;
    if row != y || layout.diff_hscrollbar_width == 0 || layout.diff_col_max == 0 {
        return None;
    }
    if col < layout.diff_hscrollbar_x {
        return None;
    }
    let rel = col.saturating_sub(layout.diff_hscrollbar_x);
    if rel >= layout.diff_hscrollbar_width {
        return None;
    }
    let (thumb_off, thumb_len) = workspace_status_graph::graph_scrollbar_thumb(
        (layout.diff_col_max as usize).saturating_add(1),
        layout.diff_col_offset,
        layout.diff_hscrollbar_width,
    )?;
    if rel >= thumb_off && rel < thumb_off.saturating_add(thumb_len) {
        return Some(SplitHit::DiffHThumb);
    }
    Some(SplitHit::DiffHTrack)
}

/// True when 0-based (`col`, `row`) is on the painted commit-message
/// footer rule.
fn is_commit_msg_rule(layout: SplitLayout, col: u16, row: u16) -> bool {
    let Some(y) = layout.commit_msg_rule_y else {
        return false;
    };
    let x = layout.commit_msg_rule_x;
    row == y && col >= x && col < x.saturating_add(layout.commit_msg_rule_width)
}

/// Map a 0-based mouse cell onto a drag handle.
///
/// Graph scrollbar (exact column, list track) wins over the pane divider
/// band (3 columns boxed, 2 flat) so a left-pane graph thumb stays
/// draggable. Horizontal graph track is next, then the file-diff horizontal
/// bar, then pane, then the commit-message footer rule, then in-diff RULE.
/// Boxed panes stop above the pane area's last row (the bottom border);
/// flat panes include it.
pub fn hit_split(layout: SplitLayout, col: u16, row: u16) -> SplitHit {
    let x = col.saturating_add(1);
    let y = row.saturating_add(1);
    if x < 1 || y < 1 || x > layout.term_cols || y > layout.term_rows {
        return SplitHit::Other;
    }
    // `pane_height` rows start under the tab strip, so the last pane row is
    // 1-based `pane_height + 1`; boxed, that row is the bottom border.
    let last_row = if layout.flat_panes {
        layout.pane_height.saturating_add(1)
    } else {
        layout.pane_height
    };
    if y > last_row {
        return SplitHit::Other;
    }
    if let Some(hit) = hit_graph_scrollbar(layout, col, row) {
        return hit;
    }
    if let Some(hit) = hit_graph_hscrollbar(layout, col, row) {
        return hit;
    }
    if let Some(hit) = hit_diff_hscrollbar(layout, col, row) {
        return hit;
    }
    if is_pane_divider(layout, x) {
        return SplitHit::Pane;
    }
    if is_commit_msg_rule(layout, col, row) {
        return SplitHit::CommitMsg;
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

    #[test]
    fn diff_pane_mode_reserves_the_scrollbar_column() {
        // A 100-column pane paints 99 columns once the bar is reserved.
        assert_eq!(diff_paint_width(NARROW_SXS), NARROW_SXS - 1);
        assert_eq!(
            diff_pane_mode(DiffMode::SideBySide, NARROW_SXS),
            DiffMode::Inline
        );
        assert_eq!(
            diff_pane_mode(DiffMode::SideBySide, NARROW_SXS + DIFF_VSCROLL_COLS),
            DiffMode::SideBySide
        );
        assert_eq!(diff_pane_mode(DiffMode::Inline, 200), DiffMode::Inline);
    }

    #[test]
    fn step_tree_fraction_moves_five_percent_and_clamps() {
        let cols = 140;
        let start = pane_widths(cols, TREE_WIDTH_FRACTION).tree_width;
        assert_eq!(start, 56);
        let wider = step_tree_fraction(cols, TREE_WIDTH_FRACTION, 1);
        assert_eq!(pane_widths(cols, wider).tree_width, 63);
        let narrower = step_tree_fraction(cols, TREE_WIDTH_FRACTION, -1);
        assert_eq!(pane_widths(cols, narrower).tree_width, 49);
        // Every column count round-trips, so a step never stalls.
        for cols in [80u16, 99, 140, 157, 200] {
            let mut fraction = TREE_WIDTH_FRACTION;
            for _ in 0..40 {
                fraction = step_tree_fraction(cols, fraction, 1);
            }
            let widths = pane_widths(cols, fraction);
            assert_eq!(widths.tree_width, cols - MIN_PANE_COLS - DIFF_PAD_X);
            assert!(widths.diff_width >= MIN_PANE_COLS);
            for _ in 0..40 {
                fraction = step_tree_fraction(cols, fraction, -1);
            }
            assert_eq!(pane_widths(cols, fraction).tree_width, MIN_PANE_COLS);
        }
    }

    #[test]
    fn tree_width_round_trips_for_every_column() {
        for cols in 1u16..400 {
            for width in MIN_PANE_COLS..cols.saturating_sub(MIN_PANE_COLS + DIFF_PAD_X) {
                let fraction = tree_fraction_from_width(cols, width);
                assert_eq!(
                    pane_widths(cols, fraction).tree_width,
                    width,
                    "{cols} {width}"
                );
            }
        }
    }

    fn wide_layout(rule: Option<u16>) -> SplitLayout {
        SplitLayout {
            term_cols: 160,
            term_rows: 24,
            pane_height: 22,
            tree_width: 48,
            diff_pane_width: 110,
            diff_content_x: 50,
            flat_panes: false,
            diff_split_rule_x: rule,
            graph_scrollbar_x: None,
            graph_scrollbar_y: 0,
            graph_scrollbar_height: 0,
            graph_content_len: 0,
            graph_scroll: 0,
            graph_hscrollbar_y: None,
            graph_hscrollbar_x: 0,
            graph_hscrollbar_width: 0,
            graph_col_max: 0,
            graph_col_offset: 0,
            diff_hscrollbar_y: None,
            diff_hscrollbar_x: 0,
            diff_hscrollbar_width: 0,
            diff_col_max: 0,
            diff_col_offset: 0,
            commit_msg_rule_y: None,
            commit_msg_rule_x: 0,
            commit_msg_rule_width: 0,
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
            flat_panes: false,
            diff_split_rule_x: None,
            graph_scrollbar_x: Some(158),
            graph_scrollbar_y: 2,
            graph_scrollbar_height: 10,
            graph_content_len: 40,
            graph_scroll: 0,
            graph_hscrollbar_y: None,
            graph_hscrollbar_x: 0,
            graph_hscrollbar_width: 0,
            graph_col_max: 0,
            graph_col_offset: 0,
            diff_hscrollbar_y: None,
            diff_hscrollbar_x: 0,
            diff_hscrollbar_width: 0,
            diff_col_max: 0,
            diff_col_offset: 0,
            commit_msg_rule_y: None,
            commit_msg_rule_x: 0,
            commit_msg_rule_width: 0,
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
            flat_panes: false,
            diff_split_rule_x: Some(47),
            graph_scrollbar_x: None,
            graph_scrollbar_y: 0,
            graph_scrollbar_height: 0,
            graph_content_len: 0,
            graph_scroll: 0,
            graph_hscrollbar_y: None,
            graph_hscrollbar_x: 0,
            graph_hscrollbar_width: 0,
            graph_col_max: 0,
            graph_col_offset: 0,
            diff_hscrollbar_y: None,
            diff_hscrollbar_x: 0,
            diff_hscrollbar_width: 0,
            diff_col_max: 0,
            diff_col_offset: 0,
            commit_msg_rule_y: None,
            commit_msg_rule_x: 0,
            commit_msg_rule_width: 0,
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
            flat_panes: false,
            diff_split_rule_x: None,
            // Left-pane graph: last inner column overlaps the ±1 pane band.
            graph_scrollbar_x: Some(46),
            graph_scrollbar_y: 1,
            graph_scrollbar_height: 10,
            graph_content_len: 40,
            graph_scroll: 0,
            graph_hscrollbar_y: None,
            graph_hscrollbar_x: 0,
            graph_hscrollbar_width: 0,
            graph_col_max: 0,
            graph_col_offset: 0,
            diff_hscrollbar_y: None,
            diff_hscrollbar_x: 0,
            diff_hscrollbar_width: 0,
            diff_col_max: 0,
            diff_col_offset: 0,
            commit_msg_rule_y: None,
            commit_msg_rule_x: 0,
            commit_msg_rule_width: 0,
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

    fn graph_hsb_layout() -> SplitLayout {
        let mut layout = graph_sb_layout();
        layout.graph_scrollbar_x = None;
        layout.graph_hscrollbar_y = Some(12);
        layout.graph_hscrollbar_x = 50;
        layout.graph_hscrollbar_width = 20;
        layout.graph_col_max = 40;
        layout.graph_col_offset = 0;
        layout
    }

    #[test]
    fn hit_test_graph_horizontal_scrollbar_thumb_and_track() {
        let layout = graph_hsb_layout();
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 0, 20).expect("thumb");
        let thumb_col = layout.graph_hscrollbar_x + thumb.0;
        assert_eq!(hit_split(layout, thumb_col, 12), SplitHit::GraphHThumb);
        let track_col = layout
            .graph_hscrollbar_x
            .saturating_add(layout.graph_hscrollbar_width.saturating_sub(1));
        if track_col != thumb_col {
            assert_eq!(hit_split(layout, track_col, 12), SplitHit::GraphHTrack);
        }
        assert_eq!(hit_split(layout, thumb_col, 11), SplitHit::Other);
        assert_eq!(
            hit_split(layout, layout.graph_hscrollbar_x.saturating_sub(1), 12),
            SplitHit::Other
        );
    }

    #[test]
    fn vertical_graph_scrollbar_column_wins_over_horizontal_track() {
        let mut layout = graph_hsb_layout();
        layout.graph_scrollbar_x = Some(69);
        layout.graph_hscrollbar_x = 50;
        layout.graph_hscrollbar_width = 20;
        layout.graph_hscrollbar_y = Some(4);
        layout.graph_scrollbar_y = 2;
        layout.graph_scrollbar_height = 10;
        assert!(
            matches!(
                hit_split(layout, 69, 4),
                SplitHit::GraphThumb | SplitHit::GraphTrack
            ),
            "vertical column must win the corner cell"
        );
    }

    #[test]
    fn graph_col_from_col_jumps_toward_track_and_drag_delta() {
        let layout = graph_hsb_layout();
        let jumped = graph_col_from_col(
            layout,
            layout
                .graph_hscrollbar_x
                .saturating_add(layout.graph_hscrollbar_width.saturating_sub(1)),
        );
        assert!(jumped > 0, "right of track should pan right: {jumped}");
        assert_eq!(jumped, layout.graph_col_max);
        let dragged = graph_col_from_delta(
            layout,
            layout.graph_hscrollbar_x,
            0,
            layout.graph_hscrollbar_x + 10,
        );
        assert!(
            dragged > 0 && dragged < jumped,
            "mid drag {dragged} jump {jumped}"
        );
        assert_eq!(
            graph_col_from_delta(layout, 4, 3, 4),
            3,
            "zero delta keeps origin offset"
        );
    }

    fn diff_hsb_layout() -> SplitLayout {
        let mut layout = wide_layout(None);
        layout.diff_hscrollbar_y = Some(12);
        layout.diff_hscrollbar_x = 50;
        layout.diff_hscrollbar_width = 20;
        layout.diff_col_max = 40;
        layout.diff_col_offset = 0;
        layout
    }

    #[test]
    fn hit_test_diff_horizontal_scrollbar_thumb_and_track() {
        let layout = diff_hsb_layout();
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 0, 20).expect("thumb");
        let thumb_col = layout.diff_hscrollbar_x + thumb.0;
        assert_eq!(hit_split(layout, thumb_col, 12), SplitHit::DiffHThumb);
        let track_col = layout
            .diff_hscrollbar_x
            .saturating_add(layout.diff_hscrollbar_width.saturating_sub(1));
        if track_col != thumb_col {
            assert_eq!(hit_split(layout, track_col, 12), SplitHit::DiffHTrack);
        }
        assert_eq!(hit_split(layout, thumb_col, 11), SplitHit::Other);
        assert_eq!(
            hit_split(layout, layout.diff_hscrollbar_x.saturating_sub(1), 12),
            SplitHit::Other
        );
    }

    #[test]
    fn hit_test_diff_horizontal_scrollbar_absent_when_max_is_zero() {
        let mut layout = diff_hsb_layout();
        layout.diff_col_max = 0;
        layout.diff_col_offset = 8;
        assert_eq!(
            hit_split(layout, layout.diff_hscrollbar_x, 12),
            SplitHit::Other,
            "no max must not classify the row as a track jump to origin"
        );
        layout.diff_col_max = 40;
        layout.diff_hscrollbar_y = None;
        assert_eq!(
            hit_split(layout, layout.diff_hscrollbar_x, 12),
            SplitHit::Other
        );
    }

    #[test]
    fn hit_test_diff_horizontal_thumb_at_nonzero_offset_is_not_left_track() {
        let mut layout = diff_hsb_layout();
        layout.diff_col_offset = 12;
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 12, 20).expect("thumb");
        assert!(
            thumb.0 > 0,
            "origin-hidden offset must move the thumb off the left edge"
        );
        let thumb_col = layout.diff_hscrollbar_x + thumb.0;
        assert_eq!(hit_split(layout, thumb_col, 12), SplitHit::DiffHThumb);
        assert_eq!(
            hit_split(layout, layout.diff_hscrollbar_x, 12),
            SplitHit::DiffHTrack,
            "left-edge track must stay a jump when the thumb has left origin"
        );
    }

    #[test]
    fn graph_horizontal_track_wins_over_diff_horizontal_track() {
        let mut layout = diff_hsb_layout();
        layout.graph_hscrollbar_y = Some(12);
        layout.graph_hscrollbar_x = 50;
        layout.graph_hscrollbar_width = 20;
        layout.graph_col_max = 40;
        layout.graph_col_offset = 0;
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 0, 20).expect("thumb");
        let thumb_col = 50 + thumb.0;
        assert_eq!(
            hit_split(layout, thumb_col, 12),
            SplitHit::GraphHThumb,
            "graph h-bar must win the overlapping cell"
        );
    }

    #[test]
    fn diff_horizontal_track_wins_over_pane_divider_band() {
        let mut layout = wide_layout(None);
        layout.diff_hscrollbar_y = Some(5);
        layout.diff_hscrollbar_x = 46;
        layout.diff_hscrollbar_width = 4;
        layout.diff_col_max = 10;
        assert!(
            matches!(
                hit_split(layout, 47, 5),
                SplitHit::DiffHThumb | SplitHit::DiffHTrack
            ),
            "file-diff h-bar must win over the pane divider band"
        );
        assert_eq!(hit_split(layout, 47, 4), SplitHit::Pane);
    }

    /// [`wide_layout`] as a flat render records it: the left pane is 48
    /// columns and the right pane's content starts on its first column.
    fn flat_wide_layout(rule: Option<u16>) -> SplitLayout {
        SplitLayout {
            diff_content_x: 48,
            flat_panes: true,
            ..wide_layout(rule)
        }
    }

    #[test]
    fn diff_split_rule_round_trips_through_the_fraction_boxed_and_flat() {
        // Boxed: content after the right pane's left border (48 + 1).
        // Flat: content on the right pane's first column (48).
        for content_x in [49u16, 48] {
            for pane in [101u16, 110, 157] {
                let row = diff_split_row_width(pane);
                assert_eq!(row, pane - DIFF_VSCROLL_COLS - DIFF_CURSOR_BAR_COLS);
                for step in 1..20 {
                    let fraction = f64::from(step) * 0.05;
                    let left = side_by_side_column_widths(row, fraction).left_width;
                    let rule = diff_split_rule_x(content_x, pane, fraction);
                    assert_eq!(
                        rule,
                        content_x + DIFF_CURSOR_BAR_COLS + left,
                        "x={content_x} pane={pane} fraction={fraction}"
                    );
                    // A press on the rule keeps the split.
                    let back = diff_split_fraction_from_col(content_x, pane, rule);
                    assert_eq!(
                        diff_split_rule_x(content_x, pane, back),
                        rule,
                        "x={content_x} pane={pane} fraction={fraction}"
                    );
                }
                let left = side_by_side_column_widths(row, DIFF_SPLIT_FRACTION).left_width;
                let rule = diff_split_rule_x(content_x, pane, DIFF_SPLIT_FRACTION);
                // A drag 7 columns right of the rule asks for 7 more left
                // columns, and the rule lands under the pointer.
                let moved = diff_split_fraction_from_col(content_x, pane, rule + 7);
                assert_eq!(
                    moved,
                    diff_split_fraction_from_left_width(row, i32::from(left) + 7),
                    "x={content_x} pane={pane}"
                );
                assert_eq!(diff_split_rule_x(content_x, pane, moved), rule + 7);
            }
        }
    }

    #[test]
    fn only_the_pane_divider_diff_rule_and_commit_msg_rule_are_resize_edges() {
        assert_eq!(SplitHit::Pane.resize_edge(), Some(ResizeEdge::Pane));
        assert_eq!(
            SplitHit::DiffSplit.resize_edge(),
            Some(ResizeEdge::DiffSplit)
        );
        assert_eq!(
            SplitHit::CommitMsg.resize_edge(),
            Some(ResizeEdge::CommitMsg)
        );
        for hit in [
            SplitHit::GraphThumb,
            SplitHit::GraphTrack,
            SplitHit::GraphHThumb,
            SplitHit::GraphHTrack,
            SplitHit::DiffHThumb,
            SplitHit::DiffHTrack,
            SplitHit::Other,
        ] {
            assert_eq!(hit.resize_edge(), None, "{hit:?}");
        }
        assert_eq!(SplitDrag::Pane.resize_edge(), Some(ResizeEdge::Pane));
        assert_eq!(SplitDrag::Diff.resize_edge(), Some(ResizeEdge::DiffSplit));
        assert_eq!(
            SplitDrag::CommitMsg {
                origin_row: 0,
                origin_lines: 4,
            }
            .resize_edge(),
            Some(ResizeEdge::CommitMsg)
        );
        for drag in [
            SplitDrag::None,
            SplitDrag::GraphScrollbar {
                origin_row: 0,
                origin_scroll: 0,
            },
            SplitDrag::GraphHScrollbar {
                origin_col: 0,
                origin_offset: 0,
            },
            SplitDrag::DiffHScrollbar {
                origin_col: 0,
                origin_offset: 0,
            },
        ] {
            assert_eq!(drag.resize_edge(), None, "{drag:?}");
        }
    }

    /// The painted commit-message footer rule is a resize edge across its
    /// width on its one row; the pane divider band wins where they meet
    /// (a left-pane graph), and no recorded rule means no edge.
    #[test]
    fn commit_msg_rule_row_is_the_commit_msg_edge() {
        for flat in [false, true] {
            let base = if flat {
                flat_wide_layout(None)
            } else {
                wide_layout(None)
            };
            assert_eq!(hit_split(base, 10, 15), SplitHit::Other, "no rule");
            let left = SplitLayout {
                commit_msg_rule_y: Some(15),
                commit_msg_rule_x: 1,
                commit_msg_rule_width: 46,
                ..base
            };
            for col in [1, 10, 45] {
                assert_eq!(
                    hit_split(left, col, 15),
                    SplitHit::CommitMsg,
                    "{flat} {col}"
                );
            }
            assert_eq!(
                hit_split(left, 0, 15),
                SplitHit::Other,
                "{flat}: left of it"
            );
            for row in [14, 16] {
                assert_eq!(
                    hit_split(left, 10, row),
                    SplitHit::Other,
                    "{flat} row {row}"
                );
            }
            assert_eq!(
                hit_split(left, 47, 15),
                SplitHit::Pane,
                "{flat}: the band wins"
            );
            let right = SplitLayout {
                commit_msg_rule_y: Some(15),
                commit_msg_rule_x: 50,
                commit_msg_rule_width: 109,
                ..base
            };
            for col in [50, 100, 158] {
                assert_eq!(
                    hit_split(right, col, 15),
                    SplitHit::CommitMsg,
                    "{flat} {col}"
                );
            }
            assert_eq!(
                hit_split(right, 159, 15),
                SplitHit::Other,
                "{flat}: past it"
            );
        }
    }

    #[test]
    fn pane_divider_band_is_three_columns_boxed_and_two_flat() {
        let boxed = wide_layout(None);
        // Last left content column, left pane's right border, right pane's
        // left border.
        for col in [46, 47, 48] {
            assert_eq!(hit_split(boxed, col, 5), SplitHit::Pane, "boxed {col}");
        }
        for col in [45, 49] {
            assert_eq!(hit_split(boxed, col, 5), SplitHit::Other, "boxed {col}");
        }
        let flat = flat_wide_layout(None);
        // Left pane's last column, right pane's first column.
        for col in [47, 48] {
            assert_eq!(hit_split(flat, col, 5), SplitHit::Pane, "flat {col}");
        }
        for col in [46, 49] {
            assert_eq!(hit_split(flat, col, 5), SplitHit::Other, "flat {col}");
        }
    }

    #[test]
    fn flat_rule_band_stays_rule_plus_minus_one() {
        let rule = diff_split_rule_x(48, 110, DIFF_SPLIT_FRACTION);
        let layout = flat_wide_layout(Some(rule));
        for col in [rule - 1, rule, rule + 1] {
            assert_eq!(hit_split(layout, col, 5), SplitHit::DiffSplit, "{col}");
        }
        assert_eq!(hit_split(layout, rule + 2, 5), SplitHit::Other);
    }

    #[test]
    fn last_pane_row_is_hit_only_when_flat() {
        // pane_height 22 under the tab strip: 0-based rows 1..=22, the last
        // one the bottom border when boxed and a content row when flat.
        let last = 22;
        let with_hbar = |mut layout: SplitLayout| {
            layout.diff_hscrollbar_y = Some(last);
            layout.diff_hscrollbar_x = 50;
            layout.diff_hscrollbar_width = 20;
            layout.diff_col_max = 40;
            layout
        };
        let flat = with_hbar(flat_wide_layout(None));
        assert_eq!(hit_split(flat, 50, last), SplitHit::DiffHThumb);
        assert_eq!(hit_split(flat, 69, last), SplitHit::DiffHTrack);
        assert_eq!(hit_split(flat, 47, last), SplitHit::Pane);
        assert_eq!(hit_split(flat, 50, last + 1), SplitHit::Other);
        let boxed = with_hbar(wide_layout(None));
        assert_eq!(hit_split(boxed, 50, last), SplitHit::Other);
        assert_eq!(hit_split(boxed, 47, last), SplitHit::Other);
        assert_eq!(hit_split(boxed, 47, last - 1), SplitHit::Pane);
    }

    #[test]
    fn diff_col_from_col_jumps_toward_track_and_drag_delta() {
        let layout = diff_hsb_layout();
        let jumped = diff_col_from_col(
            layout,
            layout
                .diff_hscrollbar_x
                .saturating_add(layout.diff_hscrollbar_width.saturating_sub(1)),
        );
        assert!(jumped > 0, "right of track should pan right: {jumped}");
        assert_eq!(jumped, layout.diff_col_max);
        let dragged = diff_col_from_delta(
            layout,
            layout.diff_hscrollbar_x,
            0,
            layout.diff_hscrollbar_x + 10,
        );
        assert!(
            dragged > 0 && dragged < jumped,
            "mid drag {dragged} jump {jumped}"
        );
        assert_eq!(
            diff_col_from_delta(layout, 4, 3, 4),
            3,
            "zero delta keeps origin offset"
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

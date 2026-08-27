//! Ratatui widget for the workspace-status git graph.
//!
//! The crate owns the graph *model* and a renderable [`GraphWidget`].
//! It does not run a terminal app.
//!
//! Interactive and headless callers share [`GraphModel::visible_rows`] and
//! the format helpers. Display differs: the widget paints a multi-lane
//! gutter from the same model. Hidden ignored worktrees stay out of the
//! visible row list unless [`GraphModel::show_ignored`] is true.

mod action;
mod chrome;
mod format;
mod glyphs;
mod gutter;
mod lane_colors;
mod layout;
mod model;
mod paint;
mod stash;
mod topology;
mod widget;

pub use action::{Action, Effect};
pub use chrome::{
    graph_chrome_budget, selection_detail_lines, selection_detail_parts, GraphChromeBudget,
    GraphFooterSelection, FOOTER_CONNECTOR_NOT_SELECTABLE, FOOTER_NO_REFS, FOOTER_NO_SELECTION,
    FOOTER_SPACER_SUBJECT, FOOTER_WORKTREE_NOT_A_COMMIT, LOADING_OLDER,
};
pub use format::{
    assemble_commit_spacer, assemble_stash_spacer, format_commit_ref_chips,
    format_commit_ref_chips_with, format_commit_spacer, format_commit_subject, format_label,
    format_local_timestamp, format_relative_date, format_row, format_stash_spacer, format_sync,
    format_utc_timestamp, meta_column_widths, meta_column_widths_with_stashes, meta_columns_text,
    overflow_chip_text, pick_meta_columns, short_id, CommitSpacerOpts, LabelKind, LabelPart,
    MetaCols, StashSpacerOpts, RELATIVE_DATE_LIMIT_SECS,
};
pub use glyphs::{GlyphSet, ASCII, CELL_W, UNICODE};
pub use gutter::{
    clip_gutter_shared, graph_gutter_cap, resolve_graph_width, GUTTER_MAX_FRACTION,
    MIN_SUBJECT_FLOOR,
};
pub use lane_colors::{
    cells_to_spans, default_lane_colors, hex_color, lane_fg, DEFAULT_LANE_COLORS,
};
pub use layout::{layout_commits, GraphStemRef, LaidOutCommit};
pub use model::{
    Commit, GraphModel, GraphRef, GraphRow, RefKind, Stash, SyncState, SyncStatus, Worktree,
    DEFAULT_GRAPH_WINDOW,
};
pub use paint::{paint_model, paint_model_with, PaintOpts, PaintedLine};
pub use topology::{cells_text, CellRole, GraphCell};
pub use widget::{
    graph_col_max, graph_hscroll_visible, graph_scrollbar_thumb, graph_vscroll_visible,
    GraphLabelPalette, GraphWidget,
};

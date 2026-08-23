//! Ratatui widget for the workspace-status git graph.
//!
//! The crate owns the graph *model* and a renderable [`GraphWidget`].
//! It does not run a terminal app. The TypeScript Ink TUI still ships
//! in this repository for features the Rust TUI has not ported yet.
//!
//! Interactive and headless callers share [`GraphModel::visible_rows`] and
//! the format helpers. Display differs: the widget paints a multi-lane
//! gutter from the same model. Hidden ignored worktrees stay out of the
//! visible row list unless [`GraphModel::show_ignored`] is true.

mod action;
mod chrome;
mod format;
mod glyphs;
mod lane_colors;
mod layout;
mod model;
mod paint;
mod stash;
mod topology;
mod widget;

pub use action::{Action, Effect};
pub use chrome::{graph_chrome_budget, selection_detail_lines, GraphChromeBudget, LOADING_OLDER};
pub use format::{
    format_commit_ref_chips, format_commit_spacer, format_commit_subject, format_label,
    format_relative_date, format_row, format_stash_spacer, format_sync, meta_column_widths,
    meta_column_widths_with_stashes, meta_columns_text, pick_meta_columns, short_id,
    CommitSpacerOpts, MetaCols, StashSpacerOpts,
};
pub use glyphs::{GlyphSet, ASCII, CELL_W, UNICODE};
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
pub use widget::GraphWidget;

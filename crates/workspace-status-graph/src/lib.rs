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
mod format;
mod glyphs;
mod layout;
mod model;
mod paint;
mod stash;
mod topology;
mod widget;

pub use action::{Action, Effect};
pub use format::{format_label, format_row, format_sync};
pub use glyphs::{GlyphSet, ASCII, CELL_W, UNICODE};
pub use layout::{layout_commits, GraphStemRef, LaidOutCommit};
pub use model::{Commit, GraphModel, GraphRow, Stash, SyncState, SyncStatus, Worktree};
pub use paint::{paint_model, PaintedLine};
pub use topology::{cells_text, CellRole, GraphCell};
pub use widget::GraphWidget;

//! Ratatui widget for the workspace-status git graph.
//!
//! The crate owns the graph *model* and a renderable [`GraphWidget`].
//! It does not run a terminal app. The TypeScript Ink TUI still owns
//! interactive paint.
//!
//! Interactive and headless callers share [`GraphModel::visible_rows`] and
//! the format helpers. Display differs. Hidden ignored worktrees stay out
//! of the visible row list unless [`GraphModel::show_ignored`] is true.

mod action;
mod format;
mod glyphs;
mod model;
mod widget;

pub use action::{Action, Effect};
pub use format::{format_row, format_sync};
pub use glyphs::{GlyphSet, ASCII, UNICODE};
pub use model::{Commit, GraphModel, GraphRow, Stash, SyncState, SyncStatus, Worktree};
pub use widget::GraphWidget;

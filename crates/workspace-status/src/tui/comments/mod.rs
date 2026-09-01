//! Local TUI comments and markdown export.
//!
//! Persist under `$XDG_STATE_HOME/my-workspace-status/comments.json`.
//! `WS_STATUS_COMMENT_STORE` overrides that path. Comments never write into
//! user git repos. `y` copies the focused tree / graph / commit-file / diff
//! row and descendants under that row (after GC).

mod export;
mod overlay;
mod store;
mod target;

pub use export::{copy_to_clipboard, export_markdown};
pub use overlay::{comment_key_label, CommentExport, CommentPrompt};
pub use store::{
    comment_store_path, comment_store_path_from_env, load_comment_store, put_comment,
    repo_identity, save_comment_store, CommentKey, CommentStore, COMMENT_STORE_VERSION,
};
pub use target::{
    collect_live_set, comments_in_focus_scope, commit_file_row_has_comment, diff_line_has_comment,
    gc_comments, graph_row_has_comment, resolve_comment_target, sole_non_default_branch,
    tree_row_has_comment, viewport_line_number, viewport_line_range, CommentExportList,
    CommentLiveSet,
};

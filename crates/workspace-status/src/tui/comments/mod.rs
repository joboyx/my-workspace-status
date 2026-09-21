//! Local TUI comments, markdown export, and entity-reference copy.
//!
//! Persist under `$XDG_STATE_HOME/my-workspace-status/comments.json`.
//! `WS_STATUS_COMMENT_STORE` overrides that path. Records are namespaced
//! by workspace identity inside that file. Comments never write into
//! user git repos. `y` copies the focused tree / graph / commit-file / diff
//! row and descendants under that row (after GC). Resolved comments stay in
//! that copy and carry `[resolved]` in the markdown. `'` copies a pasteable
//! entity reference for the focused row. It does not write the comment store.

mod export;
mod overlay;
mod reference;
mod store;
mod target;

pub use export::{copy_to_clipboard, export_markdown};
pub use overlay::{
    comment_key_label, comment_overlay_footer_save, CommentExport, CommentPrompt,
    COMMENT_OVERLAY_FOOTER_EDIT,
};
pub use reference::{format_entity_reference, DiffSide, EntityRef};
#[cfg(not(test))]
pub use store::comment_store_path;
#[cfg(test)]
pub use store::put_comment;
pub use store::{
    load_comment_store, put_comment_entry, save_comment_store, CommentKey, CommentStore,
};
pub use target::{
    collect_live_set, comments_in_focus_scope, commit_file_row_comments_resolved,
    commit_file_row_has_comment, covering_line_comment, diff_focus_side, diff_line_comment_state,
    gc_comments, graph_row_comments_resolved, graph_row_has_comment, resolve_comment_target,
    resolve_entity_reference, tree_row_comments_resolved, tree_row_has_comment,
    viewport_line_number, viewport_line_range, CommentExportList,
};

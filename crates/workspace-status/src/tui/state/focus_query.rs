//! Focus query state: narrow the left-pane tree to one feature or repo.
//!
//! The applied query ([`AppState::focus_query`]) persists per workspace
//! root in the UI-state store; the focus prompt's draft previews the tree
//! live ([`AppState::preview_focus_query`]) without a save. Both rebuild the
//! tree through [`AppState::visible_tree`], which applies the positional
//! filter and hidden repos first and the focus last (R11).
//!
//! "Focus query" names the filter; "tree focus" in this crate means
//! keyboard focus on the tree pane ([`super::FocusPane`]).

use super::super::action::Effect;
use super::super::status::StatusMessage;
use super::super::tree::{find_node, flatten_with, TreeNode, OUTSIDE_FOCUS_GROUP_ID};
use super::super::tree_focus::{split_by_focus, FocusSplit, TreeFocus};
use super::super::ui_state_store::{load_tree_focus, save_tree_focus};
use super::super::viewed::workspace_store_id;
use super::{
    focus_ancestor_ids, initial_cursor, persist_failed_status, tree_signatures, visible_snapshot,
    AppState,
};

/// The focus prompt's live preview.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FocusPreview {
    /// The draft query the tree shows.
    draft: TreeFocus,
    /// The tree row the cursor was on when the preview started. Each
    /// preview rebuild, the apply, and the cancel put the cursor back on
    /// it, or on its nearest visible ancestor.
    anchor: Option<RowAnchor>,
}

/// A tree row to put the cursor back on after a rebuild.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RowAnchor {
    /// Row id.
    id: String,
    /// The row's family primary repo path, if any: the family container
    /// row is the ancestor of a checkout the focus leaves out.
    primary_repo: Option<String>,
}

/// The focus split that the workspace-row actions read.
// The caller lands with the workspace-row actions (Task 4) of MYWS-099;
// `expect` fails the build once it calls it, so this attribute cannot
// outlive it.
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the caller lands in MYWS-099 Task 4")
)]
impl AppState {
    /// The visible snapshot split by the applied focus query: focused
    /// families with their kept checkouts, outside families, and the
    /// counts. `None` when no focus is set. Workspace-row actions read it.
    pub(crate) fn focus_split(&self) -> Option<FocusSplit> {
        let focus = self.focus_query.as_ref()?;
        let visible = visible_snapshot(&self.snapshot, self.show_hidden);
        Some(split_by_focus(focus, &visible))
    }
}

/// The focus API that the focus key, palette rows, prompt and chip click
/// call.
impl AppState {
    /// The applied focus query, or `None` when the tree shows every repo.
    /// A preview draft does not change it.
    pub(crate) fn focus_query(&self) -> Option<&TreeFocus> {
        self.focus_query.as_ref()
    }

    /// Apply `query` as the focus query and save it for this workspace
    /// root. A query with no terms clears the focus ([`Self::clear_focus_query`]).
    ///
    /// Ends a running preview. The cursor stays on its row when that row is
    /// still visible, else it moves to the row's nearest visible ancestor
    /// (the row the preview started on, while one runs). Status: `focus:
    /// <query>`, or the save error.
    pub(crate) fn set_focus_query(&mut self, query: &str) -> Effect {
        let focus = TreeFocus::parse(query);
        if focus.is_empty() {
            return self.clear_focus_query();
        }
        let anchor = self.take_preview_anchor();
        self.focus_query = Some(focus);
        let effect = self.rebuild_for_focus(anchor);
        self.status = StatusMessage::info(format!("focus: {}", query.trim()));
        self.save_focus_query();
        effect
    }

    /// Clear the focus query and its saved entry: the tree shows every repo
    /// again. Ends a running preview; the cursor rule is
    /// [`Self::set_focus_query`]'s. Status: `focus cleared`, or the save
    /// error.
    pub(crate) fn clear_focus_query(&mut self) -> Effect {
        let anchor = self.take_preview_anchor();
        self.focus_query = None;
        let effect = self.rebuild_for_focus(anchor);
        self.status = StatusMessage::info("focus cleared");
        self.save_focus_query();
        effect
    }

    /// Show the tree under the prompt draft `draft`, with no save and no
    /// change to the applied query. The first call starts the preview and
    /// notes the cursor row; each call puts the cursor back on that row or
    /// its nearest visible ancestor. A draft with no terms shows every
    /// repo. [`Self::set_focus_query`] applies a draft (Enter);
    /// [`Self::cancel_focus_preview`] drops it (Esc).
    pub(crate) fn preview_focus_query(&mut self, draft: &str) -> Effect {
        let anchor = match self.focus_preview.take() {
            Some(preview) => preview.anchor,
            None => self.cursor_anchor(),
        };
        self.focus_preview = Some(FocusPreview {
            draft: TreeFocus::parse(draft),
            anchor: anchor.clone(),
        });
        self.rebuild_for_focus(anchor)
    }

    /// End the preview and show the tree of the focus that was applied
    /// before it, with the cursor back on the row it started on (or its
    /// nearest visible ancestor). No save. No-op with no preview.
    pub(crate) fn cancel_focus_preview(&mut self) -> Effect {
        if self.focus_preview.is_none() {
            return Effect::None;
        }
        let anchor = self.take_preview_anchor();
        self.rebuild_for_focus(anchor)
    }

    /// End a running preview and return the row it started on; with no
    /// preview, the cursor row now.
    fn take_preview_anchor(&mut self) -> Option<RowAnchor> {
        match self.focus_preview.take() {
            Some(preview) => preview.anchor,
            None => self.cursor_anchor(),
        }
    }

    /// The cursor row as an anchor.
    fn cursor_anchor(&self) -> Option<RowAnchor> {
        self.focused_row().map(|row| RowAnchor {
            id: row.id.clone(),
            primary_repo: row.primary_repo.clone(),
        })
    }

    /// Save the applied query (or its removal) for this workspace root.
    /// A failed save keeps the focus and shows the error.
    fn save_focus_query(&mut self) {
        let workspace_id = workspace_store_id(&self.cwd);
        let query = self.focus_query.as_ref().map(TreeFocus::query);
        if let Err(err) = save_tree_focus(&self.ui_store_path, &workspace_id, query) {
            self.status = StatusMessage::error(persist_failed_status("focus", err));
        }
    }
}

impl AppState {
    /// The focus query the tree shows now: the prompt draft while a
    /// preview runs, else the applied query. Paint highlights its matches.
    pub(crate) fn shown_focus_query(&self) -> Option<&TreeFocus> {
        match &self.focus_preview {
            Some(preview) => Some(&preview.draft).filter(|draft| !draft.is_empty()),
            None => self.focus_query.as_ref(),
        }
    }

    /// Launch: restore the focus query saved for this workspace root and
    /// say so once (`focus restored: <query>`). A missing or bad entry
    /// leaves the tree unfocused and says nothing.
    pub(super) fn restore_focus_query(&mut self) {
        let workspace_id = workspace_store_id(&self.cwd);
        let Some(query) = load_tree_focus(&self.ui_store_path, &workspace_id) else {
            return;
        };
        self.focus_query = Some(TreeFocus::parse(&query));
        self.rebuild_for_focus(None);
        self.cursor = initial_cursor(&self.rows);
        self.status
            .offer_info(format!("focus restored: {}", query.trim()));
    }

    /// Rebuild the tree under the shown focus and put the cursor on
    /// `anchor` or its nearest visible ancestor.
    ///
    /// The outside-focus group starts folded each time it appears; while
    /// it stays, its fold state stays. Returns
    /// [`Effect::LoadRightPane`] when the cursor row changed.
    fn rebuild_for_focus(&mut self, anchor: Option<RowAnchor>) -> Effect {
        let before = self.focused_row().map(|row| row.id.clone());
        let had_outside = self.has_outside_focus_group();
        self.tree = self.visible_tree();
        self.sync_outside_focus_fold(had_outside);
        self.rows = flatten_with(&self.tree, &self.folds, self.ascii);
        self.cursor = anchor
            .and_then(|anchor| self.nearest_visible_row(&anchor))
            .unwrap_or_else(|| self.cursor.min(self.rows.len().saturating_sub(1)));
        self.signatures = tree_signatures(&self.tree, &self.file_facts);
        self.tree_ghosts.clear();
        let after = self.focused_row().map(|row| row.id.clone());
        if before == after {
            Effect::None
        } else {
            Effect::LoadRightPane
        }
    }

    /// True when the tree has the outside-focus group now.
    pub(super) fn has_outside_focus_group(&self) -> bool {
        has_outside_group(&self.tree)
    }

    /// After a tree rebuild: fold the outside-focus group when it has just
    /// appeared (`had_outside` is false), and drop its fold state when it
    /// is gone. While it stays, its fold state stays. Every rebuild that
    /// can make it appear calls this (a focus change, a watch snapshot, a
    /// branch switch), so it always starts folded.
    pub(super) fn sync_outside_focus_fold(&mut self, had_outside: bool) {
        if !has_outside_group(&self.tree) {
            self.folds.remove(OUTSIDE_FOCUS_GROUP_ID);
        } else if !had_outside {
            self.folds.insert(OUTSIDE_FOCUS_GROUP_ID.to_string());
        }
    }

    /// Row index of `anchor`, or of its nearest ancestor that is a visible
    /// row. A row the new tree has walks up its tree path (a folded group
    /// hides it: the group row). A row the new tree lacks (a checkout the
    /// focus leaves out, and its files) tries its id ancestors, then its
    /// family container.
    fn nearest_visible_row(&self, anchor: &RowAnchor) -> Option<usize> {
        let mut candidates = vec![anchor.id.clone()];
        candidates.extend(focus_ancestor_ids(&anchor.id));
        if let Some(primary) = &anchor.primary_repo {
            candidates.push(format!("repo:{primary}"));
        }
        candidates.iter().find_map(|id| {
            let path = node_id_path(&self.tree, id)?;
            path.iter()
                .rev()
                .find_map(|id| self.rows.iter().position(|row| &row.id == id))
        })
    }
}

/// True when `tree` has the outside-focus group.
fn has_outside_group(tree: &TreeNode) -> bool {
    find_node(tree, OUTSIDE_FOCUS_GROUP_ID).is_some()
}

/// Ids from the root to the node `id`, both ends included; `None` when the
/// tree has no such node.
fn node_id_path(node: &TreeNode, id: &str) -> Option<Vec<String>> {
    if node.id == id {
        return Some(vec![node.id.clone()]);
    }
    node.children.iter().find_map(|child| {
        let mut path = node_id_path(child, id)?;
        path.insert(0, node.id.clone());
        Some(path)
    })
}

#[cfg(test)]
mod tests;

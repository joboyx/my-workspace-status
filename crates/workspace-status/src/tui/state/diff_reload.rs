//! Reload of the diff the right pane shows, for a view setting that changes
//! how every load reads (Ctrl-o full-file context, `w` whitespace).
//!
//! [`AppState::anchored_shown_diff_reload`] picks the load for the surface
//! on screen ([`AppState::shown_diff_reload`]) and keeps the cursor text as
//! the anchor the landing diff moves to.

use super::super::action::Effect;
use super::super::diff::anchor_row_text;
use super::super::drill::DrillView;
use super::AppState;

impl AppState {
    /// [`Self::shown_diff_reload`] with the cursor text kept as the anchor
    /// the landing diff moves to, or [`Effect::None`] with no diff shown.
    /// No load, no anchor, so a stale anchor cannot move a later diff.
    pub(super) fn anchored_shown_diff_reload(&mut self) -> Effect {
        if !self.right_is_diff() {
            return Effect::None;
        }
        let effect = self.shown_diff_reload();
        if effect != Effect::None {
            let rows = self.current_diff_rows();
            self.pending_hunk_anchor = Some(anchor_row_text(
                &rows,
                self.diff_cursor,
                self.diff_body_height(),
            ));
        }
        effect
    }

    /// The load that refreshes the diff on the Workspace or compare tab:
    /// the compare tab's open file, the drill diff, or the depth-0 right
    /// pane. [`Effect::None`] for a commit-file list or a compare tab with
    /// no file loaded. The caller checks that the right pane shows a diff.
    pub(super) fn shown_diff_reload(&self) -> Effect {
        if let Some(tab) = self.tabs.active_compare() {
            let (Some(source), Some(path)) = (tab.source.clone(), tab.path.clone()) else {
                return Effect::None;
            };
            return Effect::LoadCompareDiff {
                tab_id: tab.id,
                repo: tab.checkout_path.clone(),
                source,
                path,
                old_path: self.focused_compare_old_path(),
            };
        }
        match &self.drill {
            DrillView::Diff {
                repo, source, path, ..
            } => Effect::LoadCommitDiff {
                repo: repo.clone(),
                source: source.clone(),
                path: path.clone(),
            },
            DrillView::Graph => Effect::LoadRightPane,
            DrillView::Files { .. } => Effect::None,
        }
    }
}

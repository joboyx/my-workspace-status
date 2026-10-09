//! Explorer tabs: open / reveal, keys, refusals, and pool loads.
//!
//! An Explorer tab shows one checkout as a lazy file tree
//! ([`super::super::explorer::ExplorerTree`]) beside a preview of the
//! focused file: the worktree diff of a changed file, or the read-only body
//! of a clean one. Folder listings leave as [`Effect::LoadExplorerDir`] and
//! land through [`AppState::apply_explorer_dir`]; previews leave as
//! [`Effect::LoadExplorerPreview`] and land through
//! [`AppState::apply_explorer_preview`]. Both drop a result for a closed tab
//! or an older request. Status letters come from the live snapshot at paint
//! time. Git writes refuse with [`EXPLORER_TAB_READ_ONLY`].

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use ratatui::layout::Rect;

use crate::file_index::FileRead;
use crate::snapshot::FileChange;

use super::super::action::{Action, Effect};
use super::super::command_palette::{CommandScope, PaletteCommand};
use super::super::effect::ExplorerPreviewBody;
use super::super::explorer::{ExplorerEntry, ExplorerRow, ExplorerStatus};
use super::super::gates::ListFocusTarget;
use super::super::selection::TextSelection;
use super::super::split::SplitDrag;
use super::super::status::StatusMessage;
use super::super::tabs::{
    ExplorerPreview, ExplorerTab, OpenFile, EXPLORER_HAS_NO_SEARCH, EXPLORER_TAB_READ_ONLY,
    ONLY_WORKSPACE_TAB_OPEN, SWITCH_TO_WORKSPACE_TAB,
};
use super::super::tree::{dir_path_from_id, NodeKind};
use super::{row_hit_rect, single_or_batch, AppState, FocusPane, NO_SEARCH_ARMED};

/// `e` refusal on an Explorer folder row (the folder-summary copy).
const FOCUS_A_FILE_TO_EDIT: &str = "focus a file to edit";

/// Where `-` puts the Explorer cursor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExplorerTarget {
    /// A checkout row: the tree root, cursor unchanged.
    Root,
    /// A folder row: that folder, expanded, cursor on it.
    Dir(String),
    /// A file row: its parent folders expanded, cursor on it.
    File(String),
}

impl AppState {
    /// True when an Explorer tab is active.
    pub fn is_explorer_tab(&self) -> bool {
        self.tabs.active_explorer().is_some()
    }

    /// Git status of `checkout` from the live snapshot, indexed for the
    /// Explorer paint. Empty when the checkout is not in the snapshot.
    ///
    /// Memoized per checkout and a fingerprint of its snapshot changes, so
    /// the several asks of one frame (and every frame until the next
    /// status reload) build the index once. The fingerprint hashes the
    /// changes in place; it allocates nothing.
    pub(crate) fn explorer_status(&self, checkout: &str) -> Rc<ExplorerStatus> {
        let changes = self
            .snapshot
            .repos
            .iter()
            .find(|repo| repo.repo == checkout)
            .map_or(&[][..], |repo| repo.changes.as_slice());
        let fingerprint = changes_fingerprint(changes);
        if let Some((memo_checkout, memo_print, status)) =
            self.explorer_status_memo.borrow().as_ref()
        {
            if memo_checkout == checkout && *memo_print == fingerprint {
                return Rc::clone(status);
            }
        }
        let status = Rc::new(ExplorerStatus::from_changes(changes));
        *self.explorer_status_memo.borrow_mut() =
            Some((checkout.to_string(), fingerprint, Rc::clone(&status)));
        status
    }

    /// Painted rows of the active Explorer tab and the cursor row index.
    pub(crate) fn explorer_rows(&self) -> Option<(Vec<ExplorerRow>, Option<usize>)> {
        let tab = self.tabs.active_explorer()?;
        let rows = tab.tree.rows(&self.explorer_status(&tab.checkout));
        let cursor = tab.tree.cursor_index(&rows);
        Some((rows, cursor))
    }

    /// `-`: parent folder on an Explorer tab; on the Workspace tab with the
    /// tree focused, open or focus the focused row's checkout Explorer and
    /// reveal the row; anywhere else (another pane, where the commit
    /// message shows, a row with no checkout, a compare or file tab),
    /// shrink the commit message ([`Action::ResizeCommitMsg`]).
    pub(super) fn explorer_reveal(&mut self) -> Effect {
        if self.is_explorer_tab() {
            return self.explorer_to_parent();
        }
        if self.tabs.is_workspace() && self.list_focus_target() == ListFocusTarget::Tree {
            if let Some((checkout, target)) = self.explorer_reveal_target() {
                return self.open_explorer_tab(checkout, target);
            }
        }
        self.dispatch(Action::ResizeCommitMsg(-1))
    }

    /// Checkout and target of the focused Workspace tree row, or `None` for
    /// a row with no checkout (workspace, group). The checkout is
    /// [`Self::focused_checkout_path`], as Quick Open scopes.
    fn explorer_reveal_target(&self) -> Option<(String, ExplorerTarget)> {
        let checkout = self.focused_checkout_path()?;
        let row = self.focused_row()?;
        let target = match row.kind {
            NodeKind::Workspace | NodeKind::Group => return None,
            NodeKind::Repo | NodeKind::Checkout | NodeKind::Section => ExplorerTarget::Root,
            NodeKind::Dir => dir_path_from_id(&row.id, &checkout)
                .map_or(ExplorerTarget::Root, ExplorerTarget::Dir),
            NodeKind::File => row.file.as_ref().map_or(ExplorerTarget::Root, |file| {
                ExplorerTarget::File(file.path.trim_end_matches('/').to_string())
            }),
        };
        Some((checkout, target))
    }

    /// Open or focus the Explorer tab of `checkout` and reveal `target`.
    ///
    /// Parks the active tab's session first. Returns the folder listings
    /// still missing (root first) and the preview load of the cursor row.
    pub(crate) fn open_explorer_tab(&mut self, checkout: String, target: ExplorerTarget) -> Effect {
        let before = self.tabs.active;
        self.park_active_session();
        let opened = self.tabs.open_or_focus_explorer(checkout);
        if self.tabs.active != before {
            self.clear_tab_transients();
        }
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return Effect::None;
        };
        let mut dirs: Vec<String> = tab.tree.expand("").into_iter().collect();
        let (rel, is_dir) = match &target {
            ExplorerTarget::Root => ("", true),
            ExplorerTarget::Dir(rel) => (rel.as_str(), true),
            ExplorerTarget::File(rel) => (rel.as_str(), false),
        };
        // A reused tab may hold listings older than the target: list those
        // folders again. The cursor stays on the target and lands on it
        // when the new listing arrives.
        let stale = tab.tree.stale_on_path(rel);
        if !rel.is_empty() {
            dirs.extend(tab.tree.reveal(rel, is_dir));
        }
        for dir in stale {
            if !dirs.contains(&dir) {
                dirs.push(dir);
            }
        }
        if target != ExplorerTarget::Root || opened != OpenFile::Focused {
            tab.focus_preview = false;
        }
        let mut effects = explorer_dir_loads(tab, dirs);
        let tab_id = tab.id;
        self.apply_active_session();
        push_effect(&mut effects, self.explorer_preview_effect(tab_id));
        single_or_batch(effects)
    }

    /// Why `action` refuses on the active Explorer tab, or `None` (also off
    /// an Explorer tab).
    ///
    /// The file tab's list ([`Self::file_tab_refusal`]) plus the blame
    /// actions: git writes, comments, highlight, diff-only keys, compare
    /// opens, and a confirm need the Workspace tab. [`Self::dispatch`] puts
    /// the reason on the status line; `palette_disabled_reason` shows the
    /// same copy on the row.
    pub(crate) fn explorer_tab_refusal(&self, action: &Action) -> Option<String> {
        if !self.is_explorer_tab() {
            return None;
        }
        matches!(
            action,
            Action::Stage
                | Action::Unstage
                | Action::Revert
                | Action::StashMenu
                | Action::StashMenuEnter
                | Action::Fetch
                | Action::Pull
                | Action::Push
                | Action::DefaultBranch
                | Action::Branch
                | Action::BranchSubmit
                | Action::CreateBranchSubmit
                | Action::RemoveWorktree
                | Action::GraphStashApply
                | Action::GraphStashPop
                | Action::GraphStashDrop
                | Action::GraphCheckout
                | Action::GraphCreateBranch
                | Action::GraphMerge
                | Action::GraphFocusBranches
                | Action::GraphFocusClear
                | Action::GraphFocusSubmit
                | Action::ToggleReviewed
                | Action::CommentStart
                | Action::ExportComments
                | Action::DiffVisualStart
                | Action::ExternalDiff
                | Action::ToggleFullContext
                | Action::CompareVsDefault
                | Action::CompareVsBranch
                | Action::CompareVsCommit
                | Action::CompareCommitVsParent
                | Action::BlameCommitVsParent
                | Action::BlamePreviousChange
                | Action::BlameCommitVsWorktree
                | Action::BlameRevealGraph
                | Action::BlameMenu
                | Action::ConfirmYes
                | Action::ConfirmYesClean
        )
        .then(|| EXPLORER_TAB_READ_ONLY.to_string())
    }

    /// Palette row reason on an Explorer tab after
    /// [`Self::explorer_tab_refusal`]: Workspace view rows say they need
    /// the Workspace tab; search rows say there is no search.
    pub(crate) fn explorer_tab_palette_reason(&self, command: &PaletteCommand) -> Option<String> {
        if command.scope == CommandScope::Highlight {
            return Some("highlight diff lines first (V)".into());
        }
        match command.action {
            Action::SearchStart => Some(EXPLORER_HAS_NO_SEARCH.into()),
            Action::SearchNext | Action::SearchPrev => Some(NO_SEARCH_ARMED.into()),
            Action::NextTab | Action::PreviousTab => {
                (self.tabs.len() <= 1).then(|| ONLY_WORKSPACE_TAB_OPEN.into())
            }
            Action::FoldToggleSubtree
            | Action::ToggleTreeMode
            | Action::ToggleShowIgnored
            | Action::ResizeTree(_)
            | Action::ToggleCommitMsgExpand
            | Action::ResizeCommitMsg(_) => Some(SWITCH_TO_WORKSPACE_TAB.into()),
            _ => None,
        }
    }

    /// Keys the active Explorer tab handles itself.
    ///
    /// `None` lets [`Self::dispatch`] fall through to the shared router
    /// (quit, help, theme, mouse toggle, tabs, Quick Open, search in files,
    /// wrap, inline / split, copy reference, the tab row, drag / release,
    /// resize, ticks).
    pub(super) fn dispatch_explorer_tab(&mut self, action: &Action) -> Option<Effect> {
        let preview = self.focus == FocusPane::Right;
        let effect = match *action {
            Action::Move(delta) if preview => self.explorer_preview_move(i64::from(delta)),
            Action::Move(delta) => self.explorer_move(delta as isize),
            Action::MoveToStart if preview => self.explorer_preview_edge(false),
            Action::MoveToStart => self.explorer_move(isize::MIN / 2),
            Action::MoveToEnd if preview => self.explorer_preview_edge(true),
            Action::MoveToEnd => self.explorer_move(isize::MAX / 2),
            Action::PageMove(pages) if preview => {
                let page = match self.tabs.active_explorer().map(|tab| &tab.preview) {
                    Some(ExplorerPreview::File(_)) => self.layout.file_view_height,
                    _ => self.diff_body_height() as u16,
                };
                let page = i64::from(page.saturating_sub(1).max(1));
                self.explorer_preview_move(i64::from(pages) * page)
            }
            Action::PageMove(pages) => {
                let page = self.layout.explorer_tree.height.saturating_sub(1).max(1);
                self.explorer_move(pages as isize * page as isize)
            }
            Action::PanDiff(delta) if preview => {
                self.explorer_preview_pan(delta);
                Effect::None
            }
            Action::ScrollWheel {
                col,
                row,
                delta,
                horizontal,
            } => self.explorer_wheel(col, row, delta, horizontal),
            Action::Click { col, row } | Action::CtrlClick { col, row }
                if row != self.layout.tab_y =>
            {
                if self.mouse_enabled {
                    self.explorer_click(col, row)
                } else {
                    Effect::None
                }
            }
            Action::NavEsc | Action::FoldClose if preview => {
                self.focus = FocusPane::Left;
                Effect::None
            }
            Action::BackClick if preview && self.mouse_enabled => {
                self.focus = FocusPane::Left;
                Effect::None
            }
            Action::FoldClose => self.explorer_close_or_parent(),
            Action::FoldOpen if !preview => self.explorer_fold(FoldKind::Open),
            Action::FoldToggle if !preview => self.explorer_fold(FoldKind::Toggle),
            Action::NavEnter if !preview => self.explorer_enter(),
            Action::FocusRight => {
                self.explorer_focus_preview();
                Effect::None
            }
            Action::FocusLeft => {
                self.focus = FocusPane::Left;
                Effect::None
            }
            Action::Refresh => self.refresh_explorer(),
            Action::Edit => self.edit_explorer(),
            Action::SearchStart if !self.help_open => {
                self.status = StatusMessage::warn(EXPLORER_HAS_NO_SEARCH);
                Effect::None
            }
            Action::PanDiff(_)
            | Action::NavEnter
            | Action::NavEsc
            | Action::BackClick
            | Action::FoldOpen
            | Action::FoldToggle
            | Action::FoldToggleSubtree
            | Action::ToggleTreeMode
            | Action::ToggleShowIgnored
            | Action::ToggleCommitMsgExpand
            | Action::ResizeCommitMsg(_)
            | Action::ResizeTree(_) => Effect::None,
            _ => return None,
        };
        Some(effect)
    }

    /// Move the tree cursor `delta` rows and preview the new row.
    fn explorer_move(&mut self, delta: isize) -> Effect {
        let Some((rows, _)) = self.explorer_rows() else {
            return Effect::None;
        };
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return Effect::None;
        };
        if rows.is_empty() {
            return Effect::None;
        }
        tab.tree.move_cursor(&rows, delta);
        let tab_id = tab.id;
        self.explorer_preview_effect(tab_id)
    }

    /// The tree cursor row of the active Explorer tab.
    fn explorer_cursor_row(&self) -> Option<ExplorerRow> {
        let (rows, cursor) = self.explorer_rows()?;
        rows.into_iter().nth(cursor?)
    }

    /// `l`, `z`: open (or toggle) the focused folder. A file row does
    /// nothing.
    fn explorer_fold(&mut self, kind: FoldKind) -> Effect {
        let Some(row) = self.explorer_cursor_row().filter(|row| row.is_dir) else {
            return Effect::None;
        };
        self.explorer_fold_dir(&row.rel, kind)
    }

    fn explorer_fold_dir(&mut self, rel: &str, kind: FoldKind) -> Effect {
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return Effect::None;
        };
        tab.tree.set_cursor_rel(rel);
        let load = match kind {
            FoldKind::Open => tab.tree.expand(rel),
            FoldKind::Toggle => tab.tree.toggle(rel),
        };
        let effects = explorer_dir_loads(tab, load.into_iter().collect());
        single_or_batch(effects)
    }

    /// `h` in the tree: close an open folder; on a file or a closed folder,
    /// jump to the parent folder (a root-level row stays).
    fn explorer_close_or_parent(&mut self) -> Effect {
        let Some(row) = self.explorer_cursor_row() else {
            return Effect::None;
        };
        if row.is_dir && row.expanded {
            if let Some(tab) = self.tabs.active_explorer_mut() {
                tab.tree.collapse(&row.rel);
            }
            return Effect::None;
        }
        self.explorer_to_parent()
    }

    /// `-` on an Explorer tab: cursor to the parent folder row. The
    /// preview pane gives focus back to the tree.
    fn explorer_to_parent(&mut self) -> Effect {
        let Some(row) = self.explorer_cursor_row() else {
            return Effect::None;
        };
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return Effect::None;
        };
        // The stored cursor may name a row that is gone; start from the
        // painted one.
        tab.tree.set_cursor_rel(&row.rel);
        if !tab.tree.cursor_to_parent() {
            return Effect::None;
        }
        let tab_id = tab.id;
        self.focus = FocusPane::Left;
        self.explorer_preview_effect(tab_id)
    }

    /// Enter in the tree: a folder opens or closes, a file moves focus to
    /// the preview.
    fn explorer_enter(&mut self) -> Effect {
        let Some(row) = self.explorer_cursor_row() else {
            return Effect::None;
        };
        if row.is_dir {
            return self.explorer_fold_dir(&row.rel, FoldKind::Toggle);
        }
        self.explorer_focus_preview();
        Effect::None
    }

    /// Focus the preview pane when it shows a file.
    fn explorer_focus_preview(&mut self) {
        let has_file = self
            .tabs
            .active_explorer()
            .is_some_and(|tab| tab.preview.rel().is_some());
        if has_file {
            self.focus = FocusPane::Right;
        }
    }

    /// Move the preview's line cursor (file) or diff cursor by `delta`.
    fn explorer_preview_move(&mut self, delta: i64) -> Effect {
        match self.tabs.active_explorer().map(|tab| &tab.preview) {
            Some(ExplorerPreview::File(_)) => self.step_file_cursor(delta),
            Some(ExplorerPreview::Diff { .. }) => {
                let delta = delta.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32;
                self.move_diff_cursor(delta);
            }
            _ => {}
        }
        Effect::None
    }

    /// `gg` / `G` in the preview.
    fn explorer_preview_edge(&mut self, end: bool) -> Effect {
        match self.tabs.active_explorer().map(|tab| &tab.preview) {
            Some(ExplorerPreview::File(_)) => {
                self.set_file_cursor(if end { usize::MAX } else { 0 });
            }
            Some(ExplorerPreview::Diff { .. }) => {
                let n = self.current_diff_rows().len();
                self.diff_cursor = if end { n.saturating_sub(1) } else { 0 };
                self.sync_diff_scroll();
            }
            _ => {}
        }
        Effect::None
    }

    /// Pan the preview's code columns.
    fn explorer_preview_pan(&mut self, delta: i32) {
        match self.tabs.active_explorer().map(|tab| &tab.preview) {
            Some(ExplorerPreview::File(_)) => self.pan_file_tab(delta),
            Some(ExplorerPreview::Diff { .. }) => self.pan_diff_content(delta),
            _ => {}
        }
    }

    /// Wheel over the tree moves the tree cursor; over the preview it moves
    /// the preview cursor (or pans, horizontally). Never the hidden
    /// Workspace tree.
    fn explorer_wheel(&mut self, col: u16, row: u16, delta: i32, horizontal: bool) -> Effect {
        if !self.mouse_enabled {
            return Effect::None;
        }
        let at = ratatui::layout::Position::new(col, row);
        if row_hit_rect(
            self.layout.explorer_preview_rows,
            self.layout.explorer_preview,
        )
        .contains(at)
        {
            if horizontal {
                self.explorer_preview_pan(delta);
                return Effect::None;
            }
            return self.explorer_preview_move(i64::from(delta));
        }
        if horizontal
            || !row_hit_rect(self.layout.explorer_tree_rows, self.layout.explorer_tree).contains(at)
        {
            return Effect::None;
        }
        self.explorer_move(delta as isize)
    }

    /// Press in a pane: a tree row takes the cursor (a folder also opens or
    /// closes); the preview takes focus and, for a file body, the line. A
    /// flat pad cell counts as its row; a text selection arms only in the
    /// inner area.
    fn explorer_click(&mut self, col: u16, row: u16) -> Effect {
        self.drag = SplitDrag::None;
        self.pad_press = None;
        let at = ratatui::layout::Position::new(col, row);
        let tree = self.layout.explorer_tree;
        if row_hit_rect(self.layout.explorer_tree_rows, tree).contains(at) {
            self.text_selection = TextSelection::arm(tree, col, row);
            self.focus = FocusPane::Left;
            let Some((rows, _)) = self.explorer_rows() else {
                return Effect::None;
            };
            let Some(tab) = self.tabs.active_explorer() else {
                return Effect::None;
            };
            let index = tab.tree_scroll + usize::from(row - tree.y);
            let Some(hit) = rows.get(index).filter(|hit| !hit.placeholder) else {
                return Effect::None;
            };
            if hit.is_dir {
                return self.explorer_fold_dir(&hit.rel.clone(), FoldKind::Toggle);
            }
            let rel = hit.rel.clone();
            let Some(tab) = self.tabs.active_explorer_mut() else {
                return Effect::None;
            };
            tab.tree.set_cursor_rel(&rel);
            let tab_id = tab.id;
            return self.explorer_preview_effect(tab_id);
        }
        let preview = self.layout.explorer_preview;
        if row_hit_rect(self.layout.explorer_preview_rows, preview).contains(at) {
            self.explorer_focus_preview();
            if matches!(
                self.tabs.active_explorer().map(|tab| &tab.preview),
                Some(ExplorerPreview::File(_))
            ) {
                self.click_file_tab(col, row);
            } else {
                let pane: Rect = self.layout.explorer_preview;
                self.text_selection = TextSelection::arm(pane, col, row);
            }
        }
        Effect::None
    }

    /// `r`: re-list the visible open folders and reload the preview.
    fn refresh_explorer(&mut self) -> Effect {
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return Effect::None;
        };
        let dirs = tab.tree.invalidate();
        let mut effects = explorer_dir_loads(tab, dirs);
        tab.preview = ExplorerPreview::None;
        let tab_id = tab.id;
        push_effect(&mut effects, self.explorer_preview_effect(tab_id));
        single_or_batch(effects)
    }

    /// `e`: open the focused file in the editor, at the preview's line when
    /// the preview shows the file's text.
    fn edit_explorer(&mut self) -> Effect {
        let Some(row) = self.explorer_cursor_row().filter(|row| !row.is_dir) else {
            self.status = StatusMessage::warn(FOCUS_A_FILE_TO_EDIT);
            return Effect::None;
        };
        let Some(tab) = self.tabs.active_explorer() else {
            return Effect::None;
        };
        let line = match &tab.preview {
            ExplorerPreview::File(file)
                if file.rel == row.rel
                    && matches!(file.body.as_deref(), Some(FileRead::Text { .. })) =>
            {
                Some(u32::try_from(file.cursor + 1).unwrap_or(u32::MAX))
            }
            _ => None,
        };
        let repo = tab.checkout.clone();
        self.status = StatusMessage::progress(format!("opening {}…", row.rel));
        Effect::EditFile {
            repo,
            path: row.rel,
            line,
        }
    }

    /// Reload the active Explorer preview when the editor just saved its
    /// file.
    pub(super) fn reload_explorer_after_edit(&mut self, repo: &str, path: &str) -> Option<Effect> {
        let tab = self.tabs.active_explorer_mut()?;
        if tab.checkout != repo || tab.preview.rel() != Some(path) {
            return None;
        }
        tab.preview = ExplorerPreview::None;
        let tab_id = tab.id;
        Some(self.explorer_preview_effect(tab_id))
    }

    /// Bring tab `tab_id`'s preview in line with its cursor row: a new file
    /// row starts a load ([`Effect::LoadExplorerPreview`]), a folder or no
    /// row clears it, the same file keeps it.
    fn explorer_preview_effect(&mut self, tab_id: u64) -> Effect {
        let Some(tab) = self.tabs.get_explorer_id(tab_id) else {
            return Effect::None;
        };
        let checkout = tab.checkout.clone();
        let rows = tab.tree.rows(&self.explorer_status(&checkout));
        let file = tab
            .tree
            .cursor_index(&rows)
            .map(|index| &rows[index])
            .filter(|row| !row.is_dir)
            .map(|row| row.rel.clone());
        let same = file.is_some() && tab.preview.rel() == file.as_deref();
        let active = self
            .tabs
            .active_explorer()
            .is_some_and(|tab| tab.id == tab_id);
        let Some(rel) = file else {
            let Some(tab) = self.tabs.get_explorer_id_mut(tab_id) else {
                return Effect::None;
            };
            if !matches!(tab.preview, ExplorerPreview::None) {
                tab.next_preview(ExplorerPreview::None);
            }
            tab.focus_preview = false;
            if active {
                self.focus = FocusPane::Left;
            }
            return Effect::None;
        };
        if same {
            return Effect::None;
        }
        let change = self
            .snapshot
            .repos
            .iter()
            .find(|repo| repo.repo == checkout)
            .and_then(|repo| {
                repo.changes
                    .iter()
                    .find(|change| change.path.trim_end_matches('/') == rel)
            })
            .cloned();
        let context = change
            .as_ref()
            .and_then(|_| self.workspace_diff_context(&checkout, &rel));
        let Some(tab) = self.tabs.get_explorer_id_mut(tab_id) else {
            return Effect::None;
        };
        let gen = tab.preview_gen.saturating_add(1);
        let preview = if change.is_some() {
            ExplorerPreview::Diff {
                rel: rel.clone(),
                content: Default::default(),
                loading: true,
            }
        } else {
            tab.file_preview(&rel, gen)
        };
        tab.next_preview(preview);
        tab.diff_cursor = 0;
        tab.diff_scroll = 0;
        tab.diff_col_offset = 0;
        if active {
            self.diff_cursor = 0;
            self.diff_scroll = 0;
            self.diff_col_offset = 0;
        }
        Effect::LoadExplorerPreview {
            tab_id,
            gen,
            repo: checkout,
            path: rel,
            change,
            context,
        }
    }

    /// After a status load of `checkout`: list again each open, visible
    /// folder of its Explorer tabs whose cached listing lacks a path the
    /// status now reports (a new untracked file or folder), so the row
    /// shows without `r`. Deleted paths are skipped (they already merge in
    /// as rows). The cursor keeps its rel.
    ///
    /// No churn: a folder with a listing in flight is not asked twice, and
    /// a status path that sent a re-list does not send another while it
    /// stays in the status (`ExplorerTab::status_relisted`). Only the path
    /// that first finds a folder stale is marked, so thousands of new files
    /// in one folder mark one path; the others find the folder in flight,
    /// and after the listing lands they are in it. Linear in the status:
    /// set lookups only, and nothing at all without an Explorer tab of
    /// `checkout`.
    pub(crate) fn explorer_status_relist(&mut self, checkout: &str) -> Option<Effect> {
        if !self
            .tabs
            .explorer_tabs()
            .any(|tab| tab.checkout == checkout)
        {
            return None;
        }
        let changes: HashSet<&str> = self
            .snapshot
            .repos
            .iter()
            .find(|repo| repo.repo == checkout)?
            .changes
            .iter()
            .filter(|change| {
                change.unstaged_status.as_deref() != Some("D")
                    && change.staged_status.as_deref() != Some("D")
            })
            .map(|change| change.path.trim_end_matches('/'))
            .filter(|path| !path.is_empty())
            .collect();
        let ids: Vec<u64> = self
            .tabs
            .explorer_tabs()
            .filter(|tab| tab.checkout == checkout)
            .map(|tab| tab.id)
            .collect();
        let mut effects = Vec::new();
        for id in ids {
            let Some(tab) = self.tabs.get_explorer_id_mut(id) else {
                continue;
            };
            tab.status_relisted
                .retain(|path| changes.contains(path.as_str()));
            let mut dirs: Vec<String> = Vec::new();
            for path in &changes {
                if tab.status_relisted.contains(*path) {
                    continue;
                }
                let mut asks = false;
                for dir in tab.tree.stale_on_path(path) {
                    if tab.tree.is_open_and_visible(&dir)
                        && !tab.dir_reqs.contains_key(&dir)
                        && !dirs.contains(&dir)
                    {
                        dirs.push(dir);
                        asks = true;
                    }
                }
                if asks {
                    tab.status_relisted.insert((*path).to_string());
                }
            }
            effects.extend(explorer_dir_loads(tab, dirs));
        }
        (!effects.is_empty()).then(|| single_or_batch(effects))
    }

    /// Land folder `rel_dir`'s listing for request `req` of tab `tab_id`.
    ///
    /// `None` (dropped) when the tab closed or a newer listing of that
    /// folder was requested. A read error lists the folder empty and, on
    /// the active tab, says why. Otherwise returns the preview load the
    /// new rows call for ([`Effect::None`] when the preview stands).
    pub(crate) fn apply_explorer_dir(
        &mut self,
        tab_id: u64,
        req: u64,
        rel_dir: &str,
        entries: Result<Vec<ExplorerEntry>, String>,
    ) -> Option<Effect> {
        let active = self
            .tabs
            .active_explorer()
            .is_some_and(|tab| tab.id == tab_id);
        let tab = self.tabs.get_explorer_id_mut(tab_id)?;
        if tab.dir_reqs.get(rel_dir) != Some(&req) {
            return None;
        }
        tab.dir_reqs.remove(rel_dir);
        let entries = match entries {
            Ok(entries) => entries,
            Err(err) => {
                if active {
                    self.status = StatusMessage::error(err);
                }
                Vec::new()
            }
        };
        let tab = self.tabs.get_explorer_id_mut(tab_id)?;
        tab.tree.apply_listing(rel_dir, entries);
        Some(self.explorer_preview_effect(tab_id))
    }

    /// Land a preview load of tab `tab_id` at generation `gen`.
    ///
    /// Dropped (false) when the tab closed, a newer preview started, or the
    /// body kind does not match the preview (a diff for a file body).
    pub(crate) fn apply_explorer_preview(
        &mut self,
        tab_id: u64,
        gen: u64,
        body: ExplorerPreviewBody,
    ) -> bool {
        let Some(tab) = self
            .tabs
            .get_explorer_id_mut(tab_id)
            .filter(|tab| tab.preview_gen == gen)
        else {
            return false;
        };
        match (&mut tab.preview, body) {
            (
                ExplorerPreview::Diff {
                    content, loading, ..
                },
                ExplorerPreviewBody::Diff(diff),
            ) => {
                *content = diff;
                *loading = false;
            }
            (ExplorerPreview::File(file), ExplorerPreviewBody::File(read)) => {
                file.body = Some(Arc::new(read));
                file.body_generation = Some(gen);
                let len = file.lines().len();
                file.cursor = file.cursor.min(len.saturating_sub(1));
                file.scroll = file.scroll.min(file.cursor);
            }
            _ => return false,
        }
        true
    }

    /// True while the active Explorer tab's diff preview is loading.
    pub(crate) fn explorer_diff_loading(&self) -> bool {
        matches!(
            self.tabs.active_explorer().map(|tab| &tab.preview),
            Some(ExplorerPreview::Diff { loading: true, .. })
        )
    }

    /// The active Explorer tab's diff preview: checkout, path, and diff.
    pub(crate) fn explorer_diff(&self) -> Option<(&str, &str, &super::super::diff::DiffContent)> {
        let tab = self.tabs.active_explorer()?;
        match &tab.preview {
            ExplorerPreview::Diff { rel, content, .. } => {
                Some((tab.checkout.as_str(), rel.as_str(), content))
            }
            _ => None,
        }
    }

    /// Park the active Explorer tab's pane focus and diff view. Returns
    /// false off an Explorer tab.
    pub(super) fn park_explorer_session(&mut self) -> bool {
        let focus_preview = self.focus == FocusPane::Right;
        let (cursor, scroll, col) = (self.diff_cursor, self.diff_scroll, self.diff_col_offset);
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return false;
        };
        tab.focus_preview = focus_preview;
        tab.diff_cursor = cursor;
        tab.diff_scroll = scroll;
        tab.diff_col_offset = col;
        true
    }

    /// Restore the active Explorer tab's pane focus and diff view. The
    /// Explorer has no `/` search, so none stays armed. Returns false off
    /// an Explorer tab.
    pub(super) fn apply_explorer_session(&mut self) -> bool {
        let Some(tab) = self.tabs.active_explorer() else {
            return false;
        };
        self.focus = if tab.focus_preview {
            FocusPane::Right
        } else {
            FocusPane::Left
        };
        self.diff_cursor = tab.diff_cursor;
        self.diff_scroll = tab.diff_scroll;
        self.diff_col_offset = tab.diff_col_offset;
        self.search_mode = false;
        self.search_active = false;
        self.search_query.clear();
        self.search_hit = None;
        true
    }
}

/// What `l` / `z` / Enter does to a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FoldKind {
    /// Expand (`l`).
    Open,
    /// Expand or collapse (`z`, Enter, click).
    Toggle,
}

/// Hash of every field of `changes` that [`ExplorerStatus::from_changes`]
/// reads.
fn changes_fingerprint(changes: &[FileChange]) -> u64 {
    let mut hasher = DefaultHasher::new();
    changes.len().hash(&mut hasher);
    for change in changes {
        change.path.hash(&mut hasher);
        change.staged_status.hash(&mut hasher);
        change.unstaged_status.hash(&mut hasher);
        change.untracked.hash(&mut hasher);
    }
    hasher.finish()
}

/// Push `effect` unless it is [`Effect::None`].
fn push_effect(effects: &mut Vec<Effect>, effect: Effect) {
    if effect != Effect::None {
        effects.push(effect);
    }
}

/// One [`Effect::LoadExplorerDir`] per folder of `dirs`, each with a new
/// request id on `tab`.
fn explorer_dir_loads(tab: &mut ExplorerTab, dirs: Vec<String>) -> Vec<Effect> {
    dirs.into_iter()
        .map(|rel_dir| {
            let parent_ignored = tab.tree.is_ignored_dir(&rel_dir);
            let req = tab.next_dir_req(&rel_dir);
            Effect::LoadExplorerDir {
                tab_id: tab.id,
                req,
                repo: tab.checkout.clone(),
                rel_dir,
                parent_ignored,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::super::command_palette::PALETTE_COMMANDS;
    use super::super::super::quick_open::QuickOpenScope;
    use super::super::super::tabs::WORKSPACE_TAB_CANNOT_CLOSE;
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::status::StatusKind;

    fn change(path: &str, unstaged: Option<&str>, untracked: bool) -> FileChange {
        FileChange {
            path: path.into(),
            staged_status: None,
            unstaged_status: unstaged.map(str::to_string),
            untracked,
            old_path: None,
        }
    }

    fn repo(
        name: &str,
        kind: CheckoutKind,
        primary: Option<&str>,
        changes: Vec<FileChange>,
    ) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: "abc".into(),
            has_unstaged: !changes.is_empty(),
            has_staged: false,
            has_untracked: changes.iter().any(|change| change.untracked),
            changes,
            checkout_kind: kind,
            primary_repo: primary.map(str::to_string),
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: Some("main".into()),
            local_branches: Vec::new(),
        }
    }

    /// `app` (with a linked worktree) and `lib`, each with changes.
    fn state() -> AppState {
        let snapshot = build_workspace_snapshot(
            &[
                repo(
                    "app",
                    CheckoutKind::Primary,
                    None,
                    vec![
                        change("src/tui/app.rs", Some("M"), false),
                        change("new.txt", None, true),
                    ],
                ),
                repo(
                    "app/.worktrees/feat",
                    CheckoutKind::Linked,
                    Some("app"),
                    vec![change("README.md", Some("M"), false)],
                ),
                repo(
                    "lib",
                    CheckoutKind::Primary,
                    None,
                    vec![change("lib.rs", Some("M"), false)],
                ),
            ],
            &[],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn focus(app: &mut AppState, kind: NodeKind, repo: Option<&str>, file: Option<&str>) {
        app.cursor = app
            .rows
            .iter()
            .position(|row| {
                row.kind == kind
                    && row.repo.as_deref() == repo
                    && file.is_none_or(|path| {
                        row.file.as_ref().is_some_and(|change| change.path == path)
                            || row.id.ends_with(&format!(":{path}"))
                    })
            })
            .unwrap_or_else(|| panic!("no {kind:?} row for {repo:?} {file:?}"));
    }

    fn flat(effect: Effect) -> Vec<Effect> {
        match effect {
            Effect::Batch(effects) => effects,
            Effect::None => Vec::new(),
            other => vec![other],
        }
    }

    fn dir_loads(effects: &[Effect]) -> Vec<String> {
        effects
            .iter()
            .filter_map(|effect| match effect {
                Effect::LoadExplorerDir { rel_dir, .. } => Some(rel_dir.clone()),
                _ => None,
            })
            .collect()
    }

    fn entry(name: &str, is_dir: bool, ignored: bool) -> ExplorerEntry {
        ExplorerEntry {
            name: name.into(),
            is_dir,
            ignored,
        }
    }

    /// The `app` checkout on disk, per folder.
    fn app_listing(rel_dir: &str) -> Vec<ExplorerEntry> {
        match rel_dir {
            "" => vec![
                entry("src", true, false),
                entry("target", true, true),
                entry("new.txt", false, false),
                entry("README.md", false, false),
            ],
            "src" => vec![entry("tui", true, false), entry("lib.rs", false, false)],
            "src/tui" => vec![entry("app.rs", false, false)],
            _ => Vec::new(),
        }
    }

    /// Land every folder listing of `effect` (and of the listings it
    /// leads to). Returns the non-listing follow-ups, in order.
    fn land(app: &mut AppState, effect: Effect) -> Vec<Effect> {
        let mut out = Vec::new();
        let mut queue = flat(effect);
        while !queue.is_empty() {
            let effect = queue.remove(0);
            match effect {
                Effect::LoadExplorerDir {
                    tab_id,
                    req,
                    rel_dir,
                    ..
                } => {
                    let follow = app
                        .apply_explorer_dir(tab_id, req, &rel_dir, Ok(app_listing(&rel_dir)))
                        .expect("fresh listing");
                    queue.extend(flat(follow));
                }
                other => out.push(other),
            }
        }
        out
    }

    fn tab(app: &AppState) -> &ExplorerTab {
        app.tabs.active_explorer().expect("explorer tab")
    }

    fn cursor_rel(app: &AppState) -> String {
        let (rows, cursor) = app.explorer_rows().expect("explorer tab");
        rows[cursor.expect("cursor row")].rel.clone()
    }

    /// Open `app` at its root with every listing landed.
    fn open_root(app: &mut AppState) {
        focus(app, NodeKind::Repo, Some("app"), None);
        let effect = app.dispatch(Action::ExplorerReveal);
        land(app, effect);
        assert!(app.is_explorer_tab());
    }

    #[test]
    fn one_explorer_tab_per_checkout() {
        let mut app = state();
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        let first = app.dispatch(Action::ExplorerReveal);
        assert_eq!(dir_loads(&flat(first)), [""], "a new tab lists its root");
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(tab(&app).checkout, "app");
        assert_eq!(app.tabs.labels()[1], "Explorer · app");

        app.dispatch(Action::JumpToTab(1));
        focus(
            &mut app,
            NodeKind::File,
            Some("app"),
            Some("src/tui/app.rs"),
        );
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(app.tabs.len(), 2, "a file under the primary reuses it");
        assert_eq!(app.tabs.active, 1);

        app.dispatch(Action::JumpToTab(1));
        focus(
            &mut app,
            NodeKind::Checkout,
            Some("app/.worktrees/feat"),
            None,
        );
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(app.tabs.len(), 3, "a linked worktree has its own tab");
        assert_eq!(tab(&app).checkout, "app/.worktrees/feat");
        assert_eq!(app.tabs.labels()[2], "Explorer · feat");

        app.dispatch(Action::JumpToTab(1));
        focus(&mut app, NodeKind::Repo, Some("lib"), None);
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(app.tabs.len(), 4);
        app.dispatch(Action::JumpToTab(1));
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(app.tabs.len(), 4, "the same row twice is one tab");
        assert_eq!(app.tabs.active, 3);
    }

    #[test]
    fn minus_on_a_nested_file_reveals_it_root_first() {
        let mut app = state();
        focus(
            &mut app,
            NodeKind::File,
            Some("app"),
            Some("src/tui/app.rs"),
        );
        let effects = flat(app.dispatch(Action::ExplorerReveal));
        assert_eq!(dir_loads(&effects), ["", "src", "src/tui"]);
        assert!(app.is_explorer_tab());
        let follow = land(&mut app, Effect::Batch(effects));
        assert!(tab(&app).tree.is_expanded("src"));
        assert!(tab(&app).tree.is_expanded("src/tui"));
        assert_eq!(cursor_rel(&app), "src/tui/app.rs");
        let preview = follow
            .into_iter()
            .find(|effect| !matches!(effect, Effect::None))
            .expect("a preview load");
        match preview {
            Effect::LoadExplorerPreview {
                repo, path, change, ..
            } => {
                assert_eq!((repo.as_str(), path.as_str()), ("app", "src/tui/app.rs"));
                assert!(change.is_some(), "a changed file previews its diff");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.explorer_diff_loading());
        assert_eq!(app.focus, FocusPane::Left, "the tree keeps focus");
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout("app".into()),
            "Quick Open and search in files scope to the checkout"
        );

        // A folder row (here the compact `src/tui`) opens that folder with
        // the cursor on it.
        app.dispatch(Action::JumpToTab(1));
        app.cursor = app
            .rows
            .iter()
            .position(|row| {
                row.kind == NodeKind::Dir
                    && dir_path_from_id(&row.id, "app").as_deref() == Some("src/tui")
            })
            .expect("src/tui dir row");
        assert_eq!(app.dispatch(Action::ExplorerReveal), Effect::None);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(cursor_rel(&app), "src/tui");
        assert!(tab(&app).tree.is_expanded("src/tui"));
    }

    #[test]
    fn minus_off_a_checkout_shrinks_the_commit_message() {
        let mut app = state();
        let lines = app.commit_msg_lines;
        focus(&mut app, NodeKind::Workspace, None, None);
        assert_eq!(app.dispatch(Action::ExplorerReveal), Effect::None);
        assert_eq!(app.commit_msg_lines, lines - 1);
        assert!(app.tabs.is_workspace(), "no Explorer tab");
        assert_eq!(app.tabs.len(), 1);

        // With the right pane focused `-` sizes the commit message there.
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        app.focus = FocusPane::Right;
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(app.commit_msg_lines, lines - 2);
        assert_eq!(app.tabs.len(), 1, "no Explorer tab from the graph pane");
        app.focus = FocusPane::Left;
        let lines = lines - 1;

        // A compare tab keeps today's `-`.
        app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(app.commit_msg_lines, lines - 2);
        assert!(app.is_compare_tab());
        assert_eq!(app.tabs.len(), 2);

        // A file tab ignores it, as it ignores the commit-message keys.
        app.open_file_tab("app".into(), "README.md".into());
        assert_eq!(app.dispatch(Action::ExplorerReveal), Effect::None);
        assert_eq!(app.commit_msg_lines, lines - 2);
        assert!(app.is_file_tab());
        assert_eq!(app.tabs.len(), 3);
    }

    #[test]
    fn minus_on_the_explorer_jumps_to_the_parent_folder() {
        let mut app = state();
        focus(
            &mut app,
            NodeKind::File,
            Some("app"),
            Some("src/tui/app.rs"),
        );
        let effect = app.dispatch(Action::ExplorerReveal);
        land(&mut app, effect);
        let lines = app.commit_msg_lines;
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(cursor_rel(&app), "src/tui");
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(cursor_rel(&app), "src");
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(cursor_rel(&app), "src", "a root-level row stays");
        assert_eq!(app.commit_msg_lines, lines, "no commit-message resize");
        assert_eq!(app.tabs.len(), 2, "no second Explorer tab");
    }

    #[test]
    fn workspace_never_closes_and_close_activates_the_left_tab() {
        let mut app = state();
        app.dispatch(Action::CloseTab);
        assert_eq!(&*app.status, WORKSPACE_TAB_CANNOT_CLOSE);
        assert_eq!(app.status.kind(), StatusKind::Warn);

        app.open_file_tab("app".into(), "README.md".into());
        app.dispatch(Action::JumpToTab(1));
        open_root(&mut app);
        assert_eq!(app.tabs.active, 2);
        app.dispatch(Action::CloseTab);
        assert_eq!(app.tabs.len(), 2);
        assert!(app.is_file_tab(), "the left neighbour is active");
        assert!(app.tabs.find_explorer("app").is_none());
    }

    #[test]
    fn git_writes_refuse_with_the_explorer_copy() {
        let mut app = state();
        open_root(&mut app);
        for action in [
            Action::Stage,
            Action::Unstage,
            Action::Revert,
            Action::Push,
            Action::Fetch,
            Action::StashMenu,
            Action::BlameMenu,
            Action::ExternalDiff,
            Action::CommentStart,
            Action::ToggleReviewed,
            Action::GraphCheckout,
        ] {
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(&*app.status, EXPLORER_TAB_READ_ONLY, "{action:?}");
            assert_eq!(app.status.kind(), StatusKind::Warn, "{action:?}");
        }
        let stage = PALETTE_COMMANDS
            .iter()
            .find(|command| command.action == Action::Stage)
            .expect("stage row");
        assert_eq!(
            app.palette_disabled_reason(stage).as_deref(),
            Some(EXPLORER_TAB_READ_ONLY)
        );
        app.dispatch(Action::SearchStart);
        assert_eq!(&*app.status, EXPLORER_HAS_NO_SEARCH);
        assert!(!app.search_mode);
    }

    #[test]
    fn fold_keys_and_enter_move_between_tree_and_preview() {
        let mut app = state();
        open_root(&mut app);
        // Root: src/, target/, new.txt, README.md (folders first).
        assert_eq!(cursor_rel(&app), "src");
        let open = app.dispatch(Action::FoldOpen);
        assert_eq!(dir_loads(&flat(open.clone())), ["src"]);
        land(&mut app, open);
        assert!(tab(&app).tree.is_expanded("src"));
        app.dispatch(Action::FoldClose);
        assert!(
            !tab(&app).tree.is_expanded("src"),
            "h closes an open folder"
        );
        assert_eq!(
            app.dispatch(Action::NavEnter),
            Effect::None,
            "listing cached"
        );
        assert!(tab(&app).tree.is_expanded("src"), "Enter opens it again");

        app.dispatch(Action::Move(1));
        assert_eq!(cursor_rel(&app), "src/tui");
        let preview = app.dispatch(Action::Move(1));
        assert_eq!(cursor_rel(&app), "src/lib.rs");
        let Effect::LoadExplorerPreview {
            tab_id,
            gen,
            change: None,
            ..
        } = preview
        else {
            panic!("a clean file reads its body: {preview:?}");
        };
        // A read for a newer generation never started; the current one lands.
        let body = || ExplorerPreviewBody::File(FileRead::Binary);
        assert!(!app.apply_explorer_preview(tab_id, gen + 1, body()));
        assert!(app.apply_explorer_preview(tab_id, gen, body()));
        assert!(!app.apply_explorer_preview(tab_id + 9, gen, body()));
        app.dispatch(Action::NavEnter);
        assert_eq!(app.focus, FocusPane::Right, "Enter on a file focuses it");
        app.dispatch(Action::NavEsc);
        assert_eq!(app.focus, FocusPane::Left, "Esc goes back to the tree");
        app.dispatch(Action::NavEnter);
        app.dispatch(Action::FoldClose);
        assert_eq!(app.focus, FocusPane::Left, "h in the preview too");
        assert_eq!(cursor_rel(&app), "src/lib.rs", "without moving the tree");
        app.dispatch(Action::FoldClose);
        assert_eq!(cursor_rel(&app), "src", "h on a file jumps to its folder");
        assert!(
            !app.apply_explorer_preview(tab_id, gen, body()),
            "a folder row dropped the file preview: its read is stale"
        );
    }

    #[test]
    fn stale_folder_listing_is_dropped() {
        let mut app = state();
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        let Effect::LoadExplorerDir { tab_id, req, .. } = app.dispatch(Action::ExplorerReveal)
        else {
            panic!("a root listing");
        };
        let root =
            |app: &mut AppState, req| app.apply_explorer_dir(tab_id, req, "", Ok(app_listing("")));
        assert!(root(&mut app, req + 1).is_none(), "unknown request");
        assert!(root(&mut app, req).is_some());
        assert!(root(&mut app, req).is_none(), "already landed");

        let reload = flat(app.dispatch(Action::Refresh));
        assert_eq!(dir_loads(&reload), [""], "r re-lists the root");
        assert!(root(&mut app, req).is_none(), "the pre-r listing is stale");
        land(&mut app, Effect::Batch(reload));
        let (rows, _) = app.explorer_rows().unwrap();
        let target = rows.iter().find(|row| row.rel == "target").unwrap();
        assert!(target.ignored, "ignored entries stay listed");
    }

    #[test]
    fn minus_relists_a_folder_whose_listing_lacks_the_file() {
        let mut app = state();
        open_root(&mut app);
        let app_repo = app
            .snapshot
            .repos
            .iter()
            .position(|repo| repo.repo == "app")
            .unwrap();
        app.snapshot.repos[app_repo]
            .changes
            .push(change("fresh.txt", None, true));
        app.rebuild_rows();
        app.dispatch(Action::JumpToTab(1));
        focus(&mut app, NodeKind::File, Some("app"), Some("fresh.txt"));
        let effects = flat(app.dispatch(Action::ExplorerReveal));
        assert_eq!(app.tabs.len(), 2, "the tab is reused");
        assert_eq!(dir_loads(&effects), [""], "the stale root lists again");
        assert_ne!(cursor_rel(&app), "fresh.txt", "not a row yet");
        let Some(Effect::LoadExplorerDir { tab_id, req, .. }) = effects.first().cloned() else {
            panic!("{effects:?}");
        };
        let mut listing = app_listing("");
        listing.push(entry("fresh.txt", false, false));
        let follow = app
            .apply_explorer_dir(tab_id, req, "", Ok(listing))
            .expect("fresh listing");
        assert_eq!(cursor_rel(&app), "fresh.txt", "the cursor lands on it");
        assert!(
            matches!(follow, Effect::LoadExplorerPreview { ref path, .. } if path == "fresh.txt"),
            "{follow:?}"
        );
    }

    #[test]
    fn results_for_a_closed_tab_are_dropped() {
        let mut app = state();
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        let Effect::LoadExplorerDir { tab_id, req, .. } = app.dispatch(Action::ExplorerReveal)
        else {
            panic!("a root listing");
        };
        app.dispatch(Action::CloseTab);
        assert!(app.tabs.is_workspace());
        assert!(
            app.apply_explorer_dir(tab_id, req, "", Ok(app_listing("")))
                .is_none(),
            "a listing for a closed tab"
        );

        open_root(&mut app);
        let preview = app.dispatch(Action::Move(3));
        let Effect::LoadExplorerPreview { tab_id, gen, .. } = preview else {
            panic!("{preview:?}");
        };
        app.dispatch(Action::CloseTab);
        assert!(app.tabs.get_explorer_id(tab_id).is_none());
        assert!(!app.apply_explorer_preview(
            tab_id,
            gen,
            ExplorerPreviewBody::File(FileRead::Binary)
        ));
    }

    #[test]
    fn two_explorer_tabs_loading_at_once_each_get_their_own_result() {
        let mut app = state();
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        let Effect::LoadExplorerDir {
            tab_id: app_tab,
            req: app_req,
            ..
        } = app.dispatch(Action::ExplorerReveal)
        else {
            panic!("app root listing");
        };
        app.dispatch(Action::JumpToTab(1));
        focus(&mut app, NodeKind::Repo, Some("lib"), None);
        let Effect::LoadExplorerDir {
            tab_id: lib_tab,
            req: lib_req,
            ..
        } = app.dispatch(Action::ExplorerReveal)
        else {
            panic!("lib root listing");
        };
        assert_ne!(app_tab, lib_tab);
        let lib_listing = vec![entry("lib.rs", false, false)];
        // Land them out of order. The lib tab's cursor lands on lib.rs (a
        // changed file), so its listing asks for that diff.
        let lib_follow = app
            .apply_explorer_dir(lib_tab, lib_req, "", Ok(lib_listing))
            .expect("lib listing");
        let Effect::LoadExplorerPreview {
            tab_id,
            gen: lib_gen,
            change: Some(_),
            ..
        } = lib_follow
        else {
            panic!("lib.rs diff: {lib_follow:?}");
        };
        assert_eq!(tab_id, lib_tab);
        assert!(app
            .apply_explorer_dir(app_tab, app_req, "", Ok(app_listing("")))
            .is_some());
        let names = |app: &AppState, id: u64| -> Vec<String> {
            let tab = app.tabs.get_explorer_id(id).unwrap();
            tab.tree
                .rows(&ExplorerStatus::default())
                .into_iter()
                .map(|row| row.rel)
                .collect()
        };
        assert_eq!(names(&app, lib_tab), ["lib.rs"]);
        assert_eq!(
            names(&app, app_tab),
            ["src", "target", "new.txt", "README.md"]
        );

        // The app tab previews its clean README.md.
        app.dispatch(Action::JumpToTab(2));
        let Effect::LoadExplorerPreview {
            tab_id,
            gen: app_gen,
            change: None,
            ..
        } = app.dispatch(Action::MoveToEnd)
        else {
            panic!("README.md preview");
        };
        assert_eq!(tab_id, app_tab);
        let readme = ExplorerPreviewBody::File(FileRead::Text {
            lines: vec!["app readme".into()],
            max_cols: 10,
        });
        let diff = super::super::super::diff::DiffContent::from_unified("+lib\n");
        assert!(app.apply_explorer_preview(app_tab, app_gen, readme));
        assert!(app.apply_explorer_preview(lib_tab, lib_gen, ExplorerPreviewBody::Diff(diff)));
        match &app.tabs.get_explorer_id(app_tab).unwrap().preview {
            ExplorerPreview::File(file) => assert_eq!(file.lines(), ["app readme"]),
            other => panic!("{other:?}"),
        }
        match &app.tabs.get_explorer_id(lib_tab).unwrap().preview {
            ExplorerPreview::Diff {
                rel,
                content,
                loading,
            } => {
                assert_eq!(rel, "lib.rs");
                assert_eq!(content.unstaged, "+lib\n");
                assert!(!loading);
            }
            other => panic!("{other:?}"),
        }
    }

    /// A status load that reports a new untracked file under an open,
    /// listed folder lists that folder again once; the row then shows
    /// `??`. An unchanged status sends nothing, in flight or after.
    #[test]
    fn a_status_path_missing_from_an_open_listing_relists_its_folder_once() {
        let mut app = state();
        app.snapshot.repos[0]
            .changes
            .push(change("brand-new.txt", None, true));
        assert_eq!(
            app.explorer_status_relist(&app.snapshot.repos[0].repo.clone()),
            None,
            "no Explorer tab: nothing to send"
        );
        app.snapshot.repos[0]
            .changes
            .retain(|change| change.path != "brand-new.txt");
        open_root(&mut app);
        let open = app.dispatch(Action::FoldOpen);
        land(&mut app, open);
        assert!(tab(&app).tree.is_expanded("src"));
        let status_load = |app: &mut AppState, changes: Vec<FileChange>| {
            let snap = repo("app", CheckoutKind::Primary, None, changes);
            crate::tui::app::apply_one_repo_snapshot(app, "app", Some(snap));
            app.explorer_status_relist("app")
        };
        let base = vec![
            change("src/tui/app.rs", Some("M"), false),
            change("new.txt", None, true),
        ];
        assert_eq!(status_load(&mut app, base.clone()), None, "nothing new");

        let mut with_new = base.clone();
        with_new.push(change("src/fresh.rs", None, true));
        with_new.push(change("src/other.rs", None, true));
        let Some(Effect::LoadExplorerDir {
            tab_id,
            req,
            rel_dir,
            ..
        }) = status_load(&mut app, with_new.clone())
        else {
            panic!("a re-list of src");
        };
        assert_eq!(rel_dir, "src", "two new files, one re-list");
        assert_eq!(tab(&app).status_relisted.len(), 1, "one path marked");
        assert_eq!(
            status_load(&mut app, with_new.clone()),
            None,
            "in flight: not asked twice"
        );
        let mut listing = app_listing("src");
        listing.push(entry("fresh.rs", false, false));
        listing.push(entry("other.rs", false, false));
        app.apply_explorer_dir(tab_id, req, "src", Ok(listing))
            .expect("fresh listing");
        assert_eq!(status_load(&mut app, with_new), None, "landed: no churn");
        let (rows, _) = app.explorer_rows().unwrap();
        assert!(rows.iter().any(|row| row.rel == "src/fresh.rs"));
        let status = app.explorer_status("app");
        assert!(status.is_untracked("src/fresh.rs"), "paints ??");
        assert_eq!(cursor_rel(&app), "src", "the cursor keeps its row");

        // A new untracked folder at the root re-lists the root; a deleted
        // path never re-lists.
        let mut more = base;
        more.push(change("newdir/", None, true));
        more.push(change("gone.txt", Some("D"), false));
        let relist = flat(status_load(&mut app, more).expect("a root re-list"));
        assert_eq!(dir_loads(&relist), [""]);
    }
}

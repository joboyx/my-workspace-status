//! Explorer tabs: open / reveal, keys, refusals, live reload, and pool
//! loads.
//!
//! An Explorer tab shows one checkout as a lazy file tree
//! ([`super::super::explorer::ExplorerTree`]) beside inner file tabs
//! ([`super::super::tabs::InnerFile`]): the whole-file worktree diff of a
//! changed file, or the read-only body of a clean one. Every file open
//! lands here ([`AppState::open_file_in_explorer`]); there is no top-level
//! file tab. Inner tabs follow VS Code preview rules:
//! a tree move onto a file replaces the one preview tab, Enter (or an
//! explicit open: Quick Open, a search-in-files hit) pins it,
//! `[` / `]` switch tabs, Ctrl-w closes one. Folder listings leave as
//! [`Effect::LoadExplorerDir`] and land through
//! [`AppState::apply_explorer_dir`]; file loads leave as
//! [`Effect::LoadExplorerPreview`] and land through
//! [`AppState::apply_explorer_preview`]. Both drop a result for a closed tab
//! or an older request. Status letters come from the live snapshot at paint
//! time. Git writes refuse with [`EXPLORER_TAB_READ_ONLY`].

use std::collections::hash_map::DefaultHasher;
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use ratatui::layout::Rect;

use crate::file_index::FileRead;
use crate::snapshot::FileChange;

use super::super::action::{Action, Effect, ExternalDiffKind};
use super::super::command_palette::{CommandScope, PaletteCommand};
use super::super::comments::{covering_line_comment, resolve_comment_target, CommentKey};
use super::super::diff::{row_for_new_line, row_line_ref};
use super::super::effect::ExplorerPreviewBody;
use super::super::event_pump::overlay_blocks_background_ticks;
use super::super::explorer::{ExplorerEntry, ExplorerRow, ExplorerStatus};
use super::super::gates::{ListFocusTarget, FOCUS_A_FILE_TO_MARK_REVIEWED};
use super::super::search::{explorer_row_match_indices, step_from_anchor, SearchPane};
use super::super::selection::TextSelection;
use super::super::split::SplitDrag;
use super::super::status::StatusMessage;
use super::super::tabs::{
    ExplorerPreview, ExplorerTab, OpenFile, ParkedSearch, EXPLORER_DIFFS_SHOW_WHOLE_FILE,
    EXPLORER_FILE_TABS_ONLY, EXPLORER_TAB_READ_ONLY, FILE_DELETED, FOCUS_A_TREE_ROW_TO_EXPLORE,
    NO_EXPLORER_FILE_TABS, ONLY_WORKSPACE_TAB_OPEN, SWITCH_TO_WORKSPACE_TAB, TAB_TO_THE_FILE_PANE,
};
use super::super::tree::{dir_path_from_id, NodeKind};
use super::super::watch::{file_disk_token, GONE_DISK_TOKEN};
use super::{
    single_or_batch, AppState, FocusPane, NO_HIGHLIGHT_TARGET, NO_SEARCH_ARMED, Z_FOLDS_TREE_ROWS,
};

/// `e` refusal on an Explorer folder row (the folder-summary copy).
const FOCUS_A_FILE_TO_EDIT: &str = "focus a file to edit";
/// `E` refusal with no changed inner file (the Workspace copy).
const FOCUS_A_FILE_TO_DIFF: &str = "focus a file to diff";

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
    /// tree focused on a row in a checkout, open or focus that checkout's
    /// Explorer and reveal the row. Anywhere else (another pane, a
    /// workspace or group row, a compare tab) it refuses with the
    /// [`Self::explorer_open_target`] copy on the status line.
    pub(super) fn explorer_reveal(&mut self) -> Effect {
        if self.is_explorer_tab() {
            return self.explorer_to_parent();
        }
        match self.explorer_open_target() {
            Ok((checkout, target)) => self.open_explorer_tab(checkout, target),
            Err(reason) => {
                self.status = StatusMessage::warn(reason);
                Effect::None
            }
        }
    }

    /// Checkout and target Open in Explorer (`-` off an Explorer tab)
    /// opens, or why it refuses.
    ///
    /// The one Open in Explorer gate: [`Self::explorer_reveal`] puts the
    /// reason on the status line and `palette_disabled_reason` paints it on
    /// the palette row, so the key and the row give the same copy. Any tab
    /// but Workspace (an Explorer tab included, where `-` is the parent
    /// folder) refuses with [`SWITCH_TO_WORKSPACE_TAB`]; on the Workspace
    /// tab a pane other than the tree, or a workspace or group row, refuses
    /// with [`FOCUS_A_TREE_ROW_TO_EXPLORE`].
    pub(crate) fn explorer_open_target(&self) -> Result<(String, ExplorerTarget), &'static str> {
        if !self.tabs.is_workspace() {
            return Err(SWITCH_TO_WORKSPACE_TAB);
        }
        if self.list_focus_target() != ListFocusTarget::Tree {
            return Err(FOCUS_A_TREE_ROW_TO_EXPLORE);
        }
        self.explorer_reveal_target()
            .ok_or(FOCUS_A_TREE_ROW_TO_EXPLORE)
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
    /// still missing (root first) and the preview load of the cursor row
    /// (a tree move: a file previews, it is not pinned), else the live
    /// reload of the active inner file tab when its file moved.
    pub(crate) fn open_explorer_tab(&mut self, checkout: String, target: ExplorerTarget) -> Effect {
        let Some((mut effects, tab_id)) = self.explorer_open_and_reveal(checkout, &target) else {
            return Effect::None;
        };
        let preview = self.explorer_sync_preview(tab_id);
        let reload = self.explorer_reload_if_shown(&preview);
        push_effect(&mut effects, preview);
        push_effect(&mut effects, reload);
        single_or_batch(effects)
    }

    /// Open `rel` of `checkout` in that checkout's Explorer tab (opened or
    /// reused) as a pinned inner file tab, the explicit open of Quick Open
    /// and search in files.
    ///
    /// The tree reveals and selects `rel`; that is cosmetic, as the
    /// listings may land after the file load. The tree keeps focus. An
    /// inner tab of `rel` that is already open is activated and pinned (no
    /// reload unless its file moved). With `line`, the cursor lands on that
    /// 0-based line once the file shows ([`Self::explorer_line_target`]);
    /// `search` then arms the in-file search on it. Returns the folder
    /// listings still missing and the file load.
    pub(crate) fn open_file_in_explorer(
        &mut self,
        checkout: String,
        rel: String,
        line: Option<usize>,
        search: Option<String>,
    ) -> Effect {
        let target = ExplorerTarget::File(rel.clone());
        let Some((mut effects, tab_id)) = self.explorer_open_and_reveal(checkout, &target) else {
            return Effect::None;
        };
        push_effect(&mut effects, self.explorer_show_file(tab_id, &rel, true));
        self.focus = FocusPane::Left;
        if let Some(line) = line {
            self.explorer_line_target(tab_id, line, search);
        }
        single_or_batch(effects)
    }

    /// [`Self::active_explorer_live_reload`] when `load` is
    /// [`Effect::None`]: a reused Explorer tab whose cursor row showed no
    /// file (a folder) still checks the inner tab it shows.
    fn explorer_reload_if_shown(&mut self, load: &Effect) -> Effect {
        if *load == Effect::None {
            self.active_explorer_live_reload()
        } else {
            Effect::None
        }
    }

    /// Open or focus the Explorer tab of `checkout` and reveal `target` in
    /// its tree, without touching the inner file tabs. Returns the folder
    /// listings still missing (root first) and the tab id.
    fn explorer_open_and_reveal(
        &mut self,
        checkout: String,
        target: &ExplorerTarget,
    ) -> Option<(Vec<Effect>, u64)> {
        let before = self.tabs.active;
        self.park_active_session();
        let opened = self.tabs.open_or_focus_explorer(checkout);
        if self.tabs.active != before {
            self.clear_tab_transients();
        }
        let tab = self.tabs.active_explorer_mut()?;
        let mut dirs: Vec<String> = tab.tree.expand("").into_iter().collect();
        let (rel, is_dir) = match target {
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
        if *target != ExplorerTarget::Root || opened != OpenFile::Focused {
            tab.focus_preview = false;
        }
        let effects = explorer_dir_loads(tab, dirs);
        let tab_id = tab.id;
        self.apply_active_session();
        Some((effects, tab_id))
    }

    /// Why `action` refuses on the active Explorer tab, or `None` (also off
    /// an Explorer tab).
    ///
    /// Git writes, diff compare opens, and a confirm need the Workspace
    /// tab ([`EXPLORER_TAB_READ_ONLY`]): a late checkout / merge result can
    /// open a confirm while an Explorer tab is active, and `y` there must
    /// not write (n / Esc still close it). The Workspace tree view keys
    /// (`t`, `.`) and the commit-message rows (`M`, Shorter / Taller) say
    /// [`SWITCH_TO_WORKSPACE_TAB`]. Ctrl-o says
    /// [`EXPLORER_DIFFS_SHOW_WHOLE_FILE`] and never changes the context.
    /// The line keys `;` and `V` need the file pane focused
    /// ([`TAB_TO_THE_FILE_PANE`]). The file keys (`space`, `E`, `y`, `'`,
    /// the blame actions) run on the active inner file. [`Self::dispatch`]
    /// puts the reason on the status line; `palette_disabled_reason` shows
    /// the same copy on the row.
    pub(crate) fn explorer_tab_refusal(&self, action: &Action) -> Option<String> {
        if !self.is_explorer_tab() {
            return None;
        }
        match action {
            Action::ToggleTreeMode
            | Action::ToggleShowIgnored
            | Action::ToggleCommitMsgExpand
            | Action::ResizeCommitMsg(_) => return Some(SWITCH_TO_WORKSPACE_TAB.into()),
            Action::ToggleFullContext => return Some(EXPLORER_DIFFS_SHOW_WHOLE_FILE.into()),
            Action::CommentStart | Action::DiffVisualStart
                if self.list_focus_target() != ListFocusTarget::None =>
            {
                return Some(TAB_TO_THE_FILE_PANE.into());
            }
            _ => {}
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
                | Action::CompareVsDefault
                | Action::CompareVsBranch
                | Action::CompareVsCommit
                | Action::CompareCommitVsParent
                | Action::ConfirmYes
                | Action::ConfirmYesClean
        )
        .then(|| EXPLORER_TAB_READ_ONLY.to_string())
    }

    /// Palette row reason on an Explorer tab after
    /// [`Self::explorer_tab_refusal`] and the blame gate
    /// ([`Self::blame_refusal`]): the row's highlight scope, then the
    /// row's own check, with the copy its key gives. `n` / `N` need an
    /// armed search, the subtree fold needs the tree focused, the tab moves
    /// need another tab; `space` and `E` need a changed inner file; `;`,
    /// `V`, and `'` need a target.
    pub(crate) fn explorer_tab_palette_reason(&self, command: &PaletteCommand) -> Option<String> {
        let highlighted = self.diff_visual_anchor.is_some();
        match (command.scope, highlighted) {
            (CommandScope::NoHighlight, true) => return Some("exit highlight first (Esc)".into()),
            (CommandScope::Highlight, false) => {
                return Some("highlight diff lines first (V)".into())
            }
            _ => {}
        }
        match command.action {
            Action::SearchNext | Action::SearchPrev => {
                (!self.search_is_armed()).then(|| NO_SEARCH_ARMED.into())
            }
            Action::NextTab | Action::PreviousTab => {
                (self.tabs.len() <= 1).then(|| ONLY_WORKSPACE_TAB_OPEN.into())
            }
            Action::FoldToggleSubtree => {
                (self.focus == FocusPane::Right).then(|| Z_FOLDS_TREE_ROWS.into())
            }
            Action::ToggleReviewed => self
                .explorer_active_change()
                .is_none()
                .then(|| FOCUS_A_FILE_TO_MARK_REVIEWED.into()),
            Action::ExternalDiff => self
                .explorer_active_change()
                .is_none()
                .then(|| FOCUS_A_FILE_TO_DIFF.into()),
            Action::CommentStart => self
                .current_comment_target()
                .is_none()
                .then(|| "no comment target".into()),
            Action::DiffVisualStart => {
                let empty = match self.tabs.active_file_view() {
                    Some(file) => file.lines().is_empty(),
                    None => self.current_diff_rows().is_empty(),
                };
                empty.then(|| NO_HIGHLIGHT_TARGET.into())
            }
            Action::CopyEntityReference => self
                .current_entity_reference()
                .is_none()
                .then(|| "no copy target".into()),
            _ => None,
        }
    }

    /// Why `[` / `]` ([`Action::PreviousFileTab`] /
    /// [`Action::NextFileTab`]) cannot run: off an Explorer tab, or on one
    /// with no inner file tab. `None` when they run.
    pub(crate) fn file_tab_cycle_reason(&self) -> Option<String> {
        match self.tabs.active_explorer() {
            None => Some(EXPLORER_FILE_TABS_ONLY.into()),
            Some(tab) if tab.files.is_empty() => Some(NO_EXPLORER_FILE_TABS.into()),
            Some(_) => None,
        }
    }

    /// Keys the active Explorer tab handles itself.
    ///
    /// `None` lets [`Self::dispatch`] fall through to the shared router
    /// (quit, help, theme, mouse toggle, tabs, Quick Open, search in files,
    /// wrap, inline / split, copy reference, the tab row, drag / release,
    /// resize, ticks). Ctrl-w closes the active inner file tab first and
    /// falls through to close the Explorer tab once none is left.
    pub(super) fn dispatch_explorer_tab(&mut self, action: &Action) -> Option<Effect> {
        let preview = self.focus == FocusPane::Right;
        let effect = match *action {
            // Esc (or a right click) on an armed search clears it first,
            // through the shared Esc path.
            Action::NavEsc | Action::BackClick if self.search_active => return None,
            // A right click in a `V` highlight leaves it, as on the Workspace.
            Action::BackClick if self.diff_visual_anchor.is_some() => return None,
            Action::Move(delta) if preview => {
                // Only Ctrl-d / Ctrl-u map to a `Move` of more than one
                // row (Page keys and the wheel come in other actions).
                if delta.unsigned_abs() > 1 {
                    self.recenter_file_view();
                }
                self.explorer_preview_move(i64::from(delta))
            }
            Action::Move(delta) => self.explorer_move(delta as isize),
            Action::MoveToStart if preview => self.explorer_preview_edge(false),
            Action::MoveToStart => self.explorer_move(isize::MIN / 2),
            Action::MoveToEnd if preview => self.explorer_preview_edge(true),
            Action::MoveToEnd => self.explorer_move(isize::MAX / 2),
            Action::PageMove(pages) if preview => {
                let page = match self.explorer_view() {
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
            Action::FoldToggleSubtree if !preview => self.explorer_fold_subtree(),
            Action::FoldToggleSubtree => {
                self.status = StatusMessage::warn(Z_FOLDS_TREE_ROWS);
                Effect::None
            }
            Action::NavEnter if !preview => self.explorer_enter(),
            Action::FocusRight => {
                self.explorer_focus_preview();
                Effect::None
            }
            Action::FocusLeft => {
                self.clear_diff_visual();
                self.focus = FocusPane::Left;
                Effect::None
            }
            // Highlight has no GPending mode: its second `g` lands here.
            Action::ArmGChord
                if preview
                    && self.diff_visual_anchor.is_some()
                    && self.chord_pending(self.g_pending_at) =>
            {
                self.g_pending_at = None;
                self.explorer_preview_edge(false)
            }
            Action::Refresh => self.refresh_explorer(),
            Action::Edit => self.edit_explorer(),
            Action::ToggleReviewed => self.explorer_toggle_reviewed(),
            Action::ExternalDiff => self.explorer_external_diff(),
            Action::PreviousFileTab => self.explorer_cycle_inner(-1),
            Action::NextFileTab => self.explorer_cycle_inner(1),
            Action::CloseTab => return self.explorer_close_inner(),
            Action::PanDiff(_)
            | Action::NavEnter
            | Action::NavEsc
            | Action::BackClick
            | Action::FoldOpen
            | Action::FoldToggle => Effect::None,
            _ => return None,
        };
        Some(effect)
    }

    /// Checkout and snapshot change of the active Explorer tab's active
    /// inner file; `None` for a clean file or with no inner tab.
    fn explorer_active_change(&self) -> Option<(String, FileChange)> {
        let tab = self.tabs.active_explorer()?;
        let rel = &tab.active_inner()?.rel;
        let change = self.explorer_change(&tab.checkout, rel)?;
        Some((tab.checkout.clone(), change))
    }

    /// `space`: toggle the reviewed mark of the active inner file (the
    /// Workspace tree's mark of the same file). A clean file, or no inner
    /// tab, says [`FOCUS_A_FILE_TO_MARK_REVIEWED`]. The Explorer paints no
    /// mark, so the status line says which way it went.
    fn explorer_toggle_reviewed(&mut self) -> Effect {
        let Some((checkout, change)) = self.explorer_active_change() else {
            self.status = StatusMessage::warn(FOCUS_A_FILE_TO_MARK_REVIEWED);
            return Effect::None;
        };
        let before = self.status.clone();
        let Some(marked) = self.toggle_viewed_for(&checkout, &change.path) else {
            return Effect::None;
        };
        // A failed save already put its error on the status line.
        if self.status == before {
            self.status = StatusMessage::info(if marked {
                "marked reviewed"
            } else {
                "reviewed mark cleared"
            });
        }
        Effect::None
    }

    /// `E`: the external diff tool on the active inner file's worktree
    /// change. A clean file, or no inner tab, says
    /// [`FOCUS_A_FILE_TO_DIFF`].
    fn explorer_external_diff(&mut self) -> Effect {
        let Some((repo, _)) = self.explorer_active_change() else {
            self.status = StatusMessage::warn(FOCUS_A_FILE_TO_DIFF);
            return Effect::None;
        };
        let Some(path) = self
            .tabs
            .active_explorer()
            .and_then(ExplorerTab::active_inner)
            .map(|inner| inner.rel.clone())
        else {
            return Effect::None;
        };
        self.status = StatusMessage::progress(format!("opening diff {path}…"));
        Effect::ExternalDiff {
            repo,
            path,
            kind: ExternalDiffKind::Worktree,
        }
    }

    /// Comment target of an Explorer tab when it is not a diff line:
    /// `Some(None)` with the tree focused (no target; the Workspace row is
    /// parked) or a File view with no line, `Some(Some(key))` for the File
    /// view's cursor line (or the `V` range) as a working-tree line
    /// comment. `None` for a focused inner diff, which keys its line the
    /// Workspace way.
    pub(super) fn explorer_file_comment_target(&self) -> Option<Option<CommentKey>> {
        let tab = self.tabs.active_explorer()?;
        if self.list_focus_target() != ListFocusTarget::None {
            return Some(None);
        }
        let file = self.tabs.active_file_view()?;
        let Some((lo, hi)) = self
            .file_visual_range()
            .or_else(|| (file.cursor < file.lines().len()).then_some((file.cursor, file.cursor)))
        else {
            return Some(None);
        };
        let line = |n: usize| u32::try_from(n + 1).ok();
        let key = resolve_comment_target(
            &self.snapshot,
            None,
            None,
            None,
            true,
            Some(&tab.checkout),
            Some(&file.rel),
            None,
            line(lo),
            line(hi),
        );
        let visual = self.diff_visual_anchor.is_some();
        Some(key.map(|key| {
            if visual {
                key
            } else {
                covering_line_comment(&self.comment_store, &key).unwrap_or(key)
            }
        }))
    }

    /// Inclusive 0-based line range of a `V` highlight on the active
    /// Explorer File view (anchor to cursor, clamped to the loaded lines),
    /// or `None` without one.
    pub(crate) fn file_visual_range(&self) -> Option<(usize, usize)> {
        let anchor = self.diff_visual_anchor?;
        let file = self.tabs.active_file_view()?;
        let last = file.lines().len().checked_sub(1)?;
        let (anchor, cursor) = (anchor.min(last), file.cursor.min(last));
        Some((anchor.min(cursor), anchor.max(cursor)))
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
        self.explorer_sync_preview(tab_id)
    }

    /// The file the active Explorer tab names (copy reference): with the
    /// right pane focused, the active inner file; with the tree focused,
    /// the tree's file row, else (a folder row, or a cursor row not listed
    /// yet) the active inner file. `None` with neither.
    pub(super) fn explorer_named_file(&self) -> Option<String> {
        let tab = self.tabs.active_explorer()?;
        let inner = || tab.active_inner().map(|inner| inner.rel.clone());
        if self.focus == FocusPane::Right {
            return inner();
        }
        self.explorer_resolved_cursor_row(tab.id)
            .filter(|row| !row.is_dir)
            .map(|row| row.rel)
            .or_else(inner)
    }

    /// The tree cursor row of the active Explorer tab.
    fn explorer_cursor_row(&self) -> Option<ExplorerRow> {
        let (rows, cursor) = self.explorer_rows()?;
        rows.into_iter().nth(cursor?)
    }

    /// `l`, `z`: open (or toggle) the focused folder. A file row does
    /// nothing. `z` on a folder arms `zz`, as on the Workspace tree.
    fn explorer_fold(&mut self, kind: FoldKind) -> Effect {
        let Some(row) = self.explorer_cursor_row().filter(|row| row.is_dir) else {
            return Effect::None;
        };
        if kind == FoldKind::Toggle {
            self.z_pending_at = Some(Instant::now());
        }
        self.explorer_fold_dir(&row.rel, kind)
    }

    /// `zz` (the second `z`): bring the focused folder's subtree in line
    /// with the folder, as the Workspace tree does. A closed folder closes
    /// every open folder under it; an open one opens its cached subfolders
    /// and lists the next level ([`ExplorerTree::expand_subtree`]). A file
    /// row does nothing.
    ///
    /// [`ExplorerTree::expand_subtree`]: super::super::explorer::ExplorerTree::expand_subtree
    fn explorer_fold_subtree(&mut self) -> Effect {
        let Some(row) = self.explorer_cursor_row().filter(|row| row.is_dir) else {
            return Effect::None;
        };
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return Effect::None;
        };
        tab.tree.set_cursor_rel(&row.rel);
        if !tab.tree.is_expanded(&row.rel) {
            tab.tree.collapse_subtree(&row.rel);
            return Effect::None;
        }
        let loads = tab.tree.expand_subtree(&row.rel);
        single_or_batch(explorer_dir_loads(tab, loads))
    }

    /// Pane `/` binds to on an Explorer tab: the tree when it has focus,
    /// else the active inner file's body or diff.
    pub(super) fn explorer_search_pane(&self) -> SearchPane {
        if self.focus == FocusPane::Left {
            return SearchPane::ExplorerTree;
        }
        match self.explorer_view() {
            Some(ExplorerPreview::File(_)) => SearchPane::File,
            Some(ExplorerPreview::Diff { .. }) => SearchPane::Diff,
            None => SearchPane::ExplorerTree,
        }
    }

    /// Tree cursor rel `/` returns to on Esc: the stored cursor, else the
    /// painted cursor row.
    pub(super) fn explorer_search_anchor_rel(&self) -> Option<String> {
        let tab = self.tabs.active_explorer()?;
        tab.tree
            .cursor_rel()
            .map(str::to_string)
            .or_else(|| self.explorer_cursor_row().map(|row| row.rel))
    }

    /// Explorer tree search step from the cursor row over the listed rows.
    /// Returns true when it wrapped. The caller previews the landed row
    /// ([`Self::explorer_tree_search_preview`]).
    pub(super) fn apply_explorer_tree_search(&mut self, dir: i32) -> bool {
        let landing = self.explorer_rows().and_then(|(rows, cursor)| {
            let hits = explorer_row_match_indices(&rows, &self.search_query);
            step_from_anchor(&hits, cursor, dir).map(|landing| (rows, landing))
        });
        let Some((rows, landing)) = landing else {
            self.set_search_status(false);
            return false;
        };
        if let Some(tab) = self.tabs.active_explorer_mut() {
            tab.tree.set_cursor_rel(&rows[landing.target].rel);
        }
        self.set_search_status(true);
        landing.wrapped
    }

    /// Preview the tree cursor row after a search move, as a tree move does
    /// (VS Code preview rules; nothing is pinned).
    pub(super) fn explorer_tree_search_preview(&mut self) -> Effect {
        match self.active_explorer_id() {
            Some(tab_id) => self.explorer_sync_preview(tab_id),
            None => Effect::None,
        }
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
        self.explorer_sync_preview(tab_id)
    }

    /// Enter in the tree: a folder opens or closes; a file pins its inner
    /// tab (creating it when the row is not one yet). The tree keeps
    /// focus.
    fn explorer_enter(&mut self) -> Effect {
        let Some(row) = self.explorer_cursor_row() else {
            return Effect::None;
        };
        if row.is_dir {
            return self.explorer_fold_dir(&row.rel, FoldKind::Toggle);
        }
        let Some(tab_id) = self.tabs.active_explorer().map(|tab| tab.id) else {
            return Effect::None;
        };
        self.explorer_show_file(tab_id, &row.rel, true)
    }

    /// Focus the right pane when an inner file tab shows.
    fn explorer_focus_preview(&mut self) {
        if self.explorer_view().is_some() {
            self.focus = FocusPane::Right;
        }
    }

    /// View of the active Explorer tab's active inner file tab.
    fn explorer_view(&self) -> Option<&ExplorerPreview> {
        Some(&self.tabs.active_explorer()?.active_inner()?.view)
    }

    /// Move the preview's line cursor (file) or diff cursor by `delta`.
    fn explorer_preview_move(&mut self, delta: i64) -> Effect {
        match self.explorer_view() {
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
        match self.explorer_view() {
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
        match self.explorer_view() {
            Some(ExplorerPreview::File(_)) => self.pan_file_tab(delta),
            Some(ExplorerPreview::Diff { .. }) => self.pan_diff_content(delta),
            None => {}
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
        if self.layout.explorer_preview_rows.contains(at) {
            if horizontal {
                self.explorer_preview_pan(delta);
                return Effect::None;
            }
            return self.explorer_preview_move(i64::from(delta));
        }
        if horizontal || !self.layout.explorer_tree_rows.contains(at) {
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
        if self.layout.explorer_tree_rows.contains(at) {
            self.text_selection = TextSelection::arm(tree, col, row);
            self.clear_diff_visual();
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
            return self.explorer_sync_preview(tab_id);
        }
        if self.layout.explorer_preview_rows.contains(at) {
            self.explorer_focus_preview();
            if matches!(self.explorer_view(), Some(ExplorerPreview::File(_))) {
                self.click_file_tab(col, row);
            } else {
                let pane: Rect = self.layout.explorer_preview;
                self.text_selection = TextSelection::arm(pane, col, row);
            }
        }
        Effect::None
    }

    /// `r`: re-list the visible open folders and reload the active inner
    /// file tab.
    fn refresh_explorer(&mut self) -> Effect {
        let Some(tab) = self.tabs.active_explorer_mut() else {
            return Effect::None;
        };
        let dirs = tab.tree.invalidate();
        let mut effects = explorer_dir_loads(tab, dirs);
        let tab_id = tab.id;
        push_effect(&mut effects, self.explorer_reload_active(tab_id));
        single_or_batch(effects)
    }

    /// `e`: open a file in the editor.
    ///
    /// With the right pane focused: the active inner file, at the file
    /// view's cursor line (a text body) or at the new-file line of the diff
    /// cursor row (a row with one). With the tree focused: the tree's file
    /// row, at the file view's cursor line only when that row is the
    /// active inner file showing text. A folder row (or a cursor row not
    /// listed yet) refuses with the folder-summary copy.
    fn edit_explorer(&mut self) -> Effect {
        let Some(tab) = self.tabs.active_explorer() else {
            return Effect::None;
        };
        let right = self.focus == FocusPane::Right;
        let rel = if right {
            tab.active_inner().map(|inner| inner.rel.clone())
        } else {
            self.explorer_resolved_cursor_row(tab.id)
                .filter(|row| !row.is_dir)
                .map(|row| row.rel)
        };
        let Some(rel) = rel else {
            self.status = StatusMessage::warn(FOCUS_A_FILE_TO_EDIT);
            return Effect::None;
        };
        let to_line = |n: usize| u32::try_from(n + 1).unwrap_or(u32::MAX);
        let line = match tab.active_inner().filter(|inner| inner.rel == rel) {
            Some(inner) => match &inner.view {
                ExplorerPreview::File(file)
                    if matches!(file.body.as_deref(), Some(FileRead::Text { .. })) =>
                {
                    Some(to_line(file.cursor))
                }
                ExplorerPreview::Diff {
                    content,
                    loading: false,
                    ..
                } if right => row_line_ref(content, self.diff_layout(), self.diff_cursor)
                    .and_then(|line| line.new_no),
                _ => None,
            },
            None => None,
        };
        let repo = tab.checkout.clone();
        self.status = StatusMessage::progress(format!("opening {rel}…"));
        Effect::EditFile {
            repo,
            path: rel,
            line,
        }
    }

    /// Reload the active Explorer tab's active inner file tab when the
    /// editor returns from its file (`repo` / `path`); `None` for any other
    /// file, and `None` when the file did not move on disk.
    ///
    /// This is the live-reload check ([`Self::explorer_live_reload`]): a
    /// save reloads quietly and keeps the painted content, the file cursor,
    /// and the diff cursor (clamped when the diff got shorter), so the view
    /// stays on the line the editor opened. An exit without a save moves no
    /// stamp and reloads nothing. The check sees a save through the disk
    /// token (`size:mtimeMs`), so a save that keeps both the size and the
    /// millisecond mtime of the last load is missed, as on the watch poll;
    /// `r` reloads by hand. The status load after the edit still catches a
    /// clean / changed flip.
    ///
    /// Only the active inner tab of the active Explorer tab is checked
    /// here. An inactive inner tab of that path (or one in an Explorer tab
    /// that is not active) is left to the same check when it shows again
    /// (`[` / `]`, a tree move, Ctrl-w, a tab switch) and after each status
    /// load.
    pub(crate) fn reload_explorer_after_edit(&mut self, repo: &str, path: &str) -> Option<Effect> {
        let tab = self.tabs.active_explorer()?;
        if tab.checkout != repo || tab.active_inner()?.rel != path {
            return None;
        }
        self.explorer_live_reload(repo)
    }

    /// Load the active inner file tab of `tab_id` again at a new
    /// generation (a diff when the snapshot now lists the file as changed,
    /// else its body). A file view keeps its line cursor; a diff view
    /// starts at the top.
    fn explorer_reload_active(&mut self, tab_id: u64) -> Effect {
        let Some(tab) = self.tabs.get_explorer_id(tab_id) else {
            return Effect::None;
        };
        let Some(rel) = tab.active_inner().map(|inner| inner.rel.clone()) else {
            return Effect::None;
        };
        let checkout = tab.checkout.clone();
        let change = self.explorer_change(&checkout, &rel);
        let stamp = self.explorer_stamp(&checkout, &rel, change.as_ref()).1;
        let active = self.active_explorer_id() == Some(tab_id);
        let Some(tab) = self.tabs.get_explorer_id_mut(tab_id) else {
            return Effect::None;
        };
        let gen = tab.next_gen();
        let mut view = tab.loading_view(&rel, change.is_some(), gen);
        let Some(inner) = tab.active_inner_mut() else {
            return Effect::None;
        };
        inner.stamp = Some(stamp);
        if let (ExplorerPreview::File(new), ExplorerPreview::File(old)) = (&mut view, &inner.view) {
            new.cursor = old.cursor;
            new.scroll = old.scroll;
            new.col_offset = old.col_offset;
        }
        inner.view = view;
        inner.gen = gen;
        inner.diff_cursor = 0;
        inner.diff_scroll = 0;
        inner.diff_col_offset = 0;
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
        }
    }

    /// Id of the active Explorer tab.
    fn active_explorer_id(&self) -> Option<u64> {
        self.tabs.active_explorer().map(|tab| tab.id)
    }

    /// Snapshot change of `rel` in `checkout`, `None` for a clean file.
    fn explorer_change(&self, checkout: &str, rel: &str) -> Option<FileChange> {
        self.snapshot
            .repos
            .iter()
            .find(|repo| repo.repo == checkout)?
            .changes
            .iter()
            .find(|change| change.path.trim_end_matches('/') == rel)
            .cloned()
    }

    /// Bring tab `tab_id`'s inner file tabs in line with its tree cursor
    /// (VS Code preview rules, through [`Self::explorer_show_file`]): a
    /// file row shows as the preview tab, or as its own tab when it already
    /// is one. A folder row, no row, or a row that only stands in for a
    /// cursor path not listed yet leaves the inner tabs as they are.
    fn explorer_sync_preview(&mut self, tab_id: u64) -> Effect {
        let Some(row) = self.explorer_resolved_cursor_row(tab_id) else {
            return Effect::None;
        };
        if row.is_dir {
            return Effect::None;
        }
        self.explorer_show_file(tab_id, &row.rel, false)
    }

    /// The tree cursor row of tab `tab_id` when it is the stored cursor
    /// path (or, with no stored cursor yet, the first row). `None` while
    /// the cursor path is not listed yet: `cursor_index` then stands in an
    /// ancestor or the first row, which must never preview.
    fn explorer_resolved_cursor_row(&self, tab_id: u64) -> Option<ExplorerRow> {
        let tab = self.tabs.get_explorer_id(tab_id)?;
        let rows = tab.tree.rows(&self.explorer_status(&tab.checkout));
        let index = tab.tree.cursor_index(&rows)?;
        let row = rows.into_iter().nth(index)?;
        tab.tree
            .cursor_rel()
            .is_none_or(|cursor| cursor == row.rel)
            .then_some(row)
    }

    /// Make `rel` the active inner file tab of `tab_id`.
    ///
    /// An existing inner tab of `rel` is shown as it is; with `pin` it is
    /// pinned. It loads again only when it was hidden and its file moved
    /// ([`Self::explorer_check_shown`]). Otherwise a new tab loads:
    /// unpinned, it replaces the preview tab in place, or is inserted right
    /// after the active tab when there is no preview; pinned, it is
    /// inserted right after the active tab. Returns that load
    /// ([`Effect::LoadExplorerPreview`]).
    fn explorer_show_file(&mut self, tab_id: u64, rel: &str, pin: bool) -> Effect {
        let Some(checkout) = self
            .tabs
            .get_explorer_id(tab_id)
            .map(|tab| tab.checkout.clone())
        else {
            return Effect::None;
        };
        let change = self.explorer_change(&checkout, rel);
        let before = self.explorer_park_inner(tab_id);
        let known = self
            .tabs
            .get_explorer_id(tab_id)
            .is_some_and(|tab| tab.find_inner(rel).is_some());
        // Taken before the read: a change between the stat and the read
        // costs at most one extra reload.
        let stamp = (!known).then(|| self.explorer_stamp(&checkout, rel, change.as_ref()).1);
        let Some(tab) = self.tabs.get_explorer_id_mut(tab_id) else {
            return Effect::None;
        };
        let mut load = Effect::None;
        let index = match tab.find_inner(rel) {
            Some(index) => index,
            None => {
                let mut inner = tab.new_inner(rel, change.is_some(), pin);
                inner.stamp = stamp;
                load = Effect::LoadExplorerPreview {
                    tab_id,
                    gen: inner.gen,
                    repo: checkout,
                    path: rel.to_string(),
                    change,
                };
                match tab.preview_index().filter(|_| !pin) {
                    Some(index) => {
                        tab.files[index] = inner;
                        index
                    }
                    None => {
                        let at = tab.active_file.map_or(tab.files.len(), |active| active + 1);
                        tab.files.insert(at, inner);
                        at
                    }
                }
            }
        };
        if pin {
            tab.files[index].pinned = true;
        }
        tab.active_file = Some(index);
        self.explorer_load_inner(tab_id, before);
        if load == Effect::None {
            return self.explorer_check_shown(tab_id, before);
        }
        load
    }

    /// The live-reload check ([`Self::explorer_live_reload`]) of the inner
    /// tab that now shows on tab `tab_id`, when that tab is the active
    /// Explorer tab and the active inner tab changed (`before` is the
    /// generation of the one shown before): an inner tab that was hidden
    /// may have missed disk edits. [`Effect::None`] otherwise.
    fn explorer_check_shown(&mut self, tab_id: u64, before: Option<u64>) -> Effect {
        let shown = self
            .tabs
            .active_explorer()
            .filter(|tab| tab.id == tab_id)
            .and_then(ExplorerTab::active_inner)
            .map(|inner| inner.gen);
        if shown.is_none() || shown == before {
            return Effect::None;
        }
        self.active_explorer_live_reload()
    }

    /// `[` / `]`: activate the inner file tab `delta` away (wrapping) and
    /// select its file in the tree. Which tab is the preview stays. The
    /// tab that now shows gets the live-reload check.
    fn explorer_cycle_inner(&mut self, delta: isize) -> Effect {
        if let Some(reason) = self.file_tab_cycle_reason() {
            self.status = StatusMessage::warn(reason);
            return Effect::None;
        }
        let Some(tab) = self.tabs.active_explorer() else {
            return Effect::None;
        };
        let tab_id = tab.id;
        let len = tab.files.len() as isize;
        let current = tab.active_file.unwrap_or(0) as isize;
        let next = (current + delta).rem_euclid(len) as usize;
        let before = self.explorer_park_inner(tab_id);
        if let Some(tab) = self.tabs.get_explorer_id_mut(tab_id) {
            tab.active_file = Some(next);
        }
        self.explorer_load_inner(tab_id, before);
        self.explorer_reveal_and_check(tab_id, before)
    }

    /// Ctrl-w on an Explorer tab: close the active inner file tab and
    /// activate its left neighbour (else the first tab), selected in the
    /// tree, with the live-reload check. `None` with no inner tab, so the
    /// shared router closes the Explorer tab.
    fn explorer_close_inner(&mut self) -> Option<Effect> {
        let tab = self.tabs.active_explorer_mut()?;
        let index = tab.active_file?;
        let tab_id = tab.id;
        let before = Some(tab.files.remove(index).gen);
        tab.active_file = (!tab.files.is_empty()).then(|| index.saturating_sub(1));
        self.explorer_load_inner(tab_id, before);
        Some(self.explorer_reveal_and_check(tab_id, before))
    }

    /// [`Self::explorer_reveal_active`] plus the live-reload check of the
    /// inner tab that now shows ([`Self::explorer_check_shown`]).
    fn explorer_reveal_and_check(&mut self, tab_id: u64, before: Option<u64>) -> Effect {
        let mut effects = Vec::new();
        push_effect(&mut effects, self.explorer_reveal_active(tab_id));
        push_effect(&mut effects, self.explorer_check_shown(tab_id, before));
        single_or_batch(effects)
    }

    /// Reveal and select the active inner tab's file in tab `tab_id`'s
    /// tree. Returns the folder listings still missing.
    fn explorer_reveal_active(&mut self, tab_id: u64) -> Effect {
        let Some(tab) = self.tabs.get_explorer_id_mut(tab_id) else {
            return Effect::None;
        };
        let Some(rel) = tab.active_inner().map(|inner| inner.rel.clone()) else {
            return Effect::None;
        };
        let dirs = tab.tree.reveal(&rel, false);
        tab.tree.set_cursor_rel(&rel);
        single_or_batch(explorer_dir_loads(tab, dirs))
    }

    /// Before the active inner tab of `tab_id` changes: on the active
    /// Explorer tab, park the live diff view in it. Returns its load
    /// generation (for [`Self::explorer_load_inner`]).
    fn explorer_park_inner(&mut self, tab_id: u64) -> Option<u64> {
        let live = (self.diff_cursor, self.diff_scroll, self.diff_col_offset);
        let active = self.active_explorer_id() == Some(tab_id);
        let inner = self.tabs.get_explorer_id_mut(tab_id)?.active_inner_mut()?;
        if active {
            (inner.diff_cursor, inner.diff_scroll, inner.diff_col_offset) = live;
        }
        Some(inner.gen)
    }

    /// After the active inner tab of `tab_id` changed (`before` is the
    /// generation [`Self::explorer_park_inner`] returned): on the active
    /// Explorer tab, load the new inner tab's parked diff view; another
    /// inner tab also drops the highlight and the armed search, and with
    /// none left the tree takes focus.
    fn explorer_load_inner(&mut self, tab_id: u64, before: Option<u64>) {
        let Some(tab) = self.tabs.active_explorer().filter(|tab| tab.id == tab_id) else {
            return;
        };
        let inner = tab.active_inner();
        let after = inner.map(|inner| inner.gen);
        (self.diff_cursor, self.diff_scroll, self.diff_col_offset) = inner
            .map_or((0, 0, 0), |inner| {
                (inner.diff_cursor, inner.diff_scroll, inner.diff_col_offset)
            });
        if after.is_none() {
            self.focus = FocusPane::Left;
        }
        if before != after {
            self.clear_diff_visual();
            self.clear_explorer_search();
        }
    }

    /// Drop a typed or armed `/` search of the inner file (its body or
    /// diff). A tree search stays: it is bound to the tree, not to the file
    /// it previews.
    fn clear_explorer_search(&mut self) {
        if self.search_target == SearchPane::ExplorerTree {
            return;
        }
        self.search_mode = false;
        self.search_active = false;
        self.search_query.clear();
        self.search_hit = None;
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
    /// the active tab, says why. Otherwise returns the preview load of the
    /// cursor file when this listing is what made the cursor row resolve
    /// ([`Effect::None`] in every other case).
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
        // Sync only when this listing is what lets the cursor row resolve
        // (a reveal or a new tab waiting on it). A listing that lands while
        // the cursor row already shows (a status re-list, `r`, a folder
        // opened earlier) leaves the inner tabs alone, so an inner tab closed
        // with Ctrl-w does not come back on its own.
        let resolved_before = self.explorer_resolved_cursor_row(tab_id).is_some();
        let tab = self.tabs.get_explorer_id_mut(tab_id)?;
        tab.tree.apply_listing(rel_dir, entries);
        if resolved_before {
            return Some(Effect::None);
        }
        Some(self.explorer_sync_preview(tab_id))
    }

    /// Land a file load of tab `tab_id` at generation `gen` in the inner
    /// tab that waits for it.
    ///
    /// Dropped (false) when the tab closed, no inner tab has that
    /// generation (closed, replaced, or reloaded since), or the body kind
    /// does not match the view (a diff for a file body). A failed read
    /// while the view still paints a body (a silent live reload) keeps that
    /// body; if the file is gone, the next poll reports it deleted. A file
    /// view keeps its cursor and scroll line numbers, clamped to the new
    /// line count. A pending line target lands
    /// ([`Self::explorer_apply_pending_line`]).
    pub(crate) fn apply_explorer_preview(
        &mut self,
        tab_id: u64,
        gen: u64,
        body: ExplorerPreviewBody,
    ) -> bool {
        let active = self
            .tabs
            .active_explorer()
            .filter(|tab| tab.id == tab_id)
            .and_then(ExplorerTab::active_inner)
            .is_some_and(|inner| inner.gen == gen);
        let Some(inner) = self
            .tabs
            .get_explorer_id_mut(tab_id)
            .and_then(|tab| tab.inner_by_gen_mut(gen))
        else {
            return false;
        };
        match (&mut inner.view, body) {
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
                if matches!(read, FileRead::Failed(_)) && file.body.is_some() {
                    return false;
                }
                file.body = Some(Arc::new(read));
                file.body_generation = Some(gen);
                let len = file.lines().len();
                file.cursor = file.cursor.min(len.saturating_sub(1));
                file.scroll = file.scroll.min(file.cursor);
                if active && self.search_hit.is_some_and(|hit| hit >= len) {
                    self.search_hit = None;
                }
            }
            _ => return false,
        }
        // A live reload keeps the diff cursor row; a shorter diff clamps it.
        if active && self.diff_cursor > 0 && self.explorer_diff().is_some() {
            let rows = self.current_diff_rows().len();
            if self.diff_cursor >= rows {
                self.sync_diff_scroll();
            }
        }
        self.explorer_apply_pending_line(tab_id, gen);
        true
    }

    /// Point the active inner file tab of `tab_id` at 0-based `line` (a
    /// search-in-files hit).
    ///
    /// With `search`, the in-file search is armed on that text for the
    /// pane the view paints (the file body, or the diff of a changed file),
    /// as if `/` and Enter had landed on `line`, so `n` / `N` step its
    /// matches from there. Without it any armed search is cleared. The
    /// cursor moves now when the view has loaded, else when the load lands
    /// ([`InnerFile::pending_line`](super::super::tabs::InnerFile::pending_line)).
    fn explorer_line_target(&mut self, tab_id: u64, line: usize, search: Option<String>) {
        let Some(inner) = self
            .tabs
            .get_explorer_id_mut(tab_id)
            .and_then(ExplorerTab::active_inner_mut)
        else {
            return;
        };
        inner.pending_line = Some(line);
        let gen = inner.gen;
        let pane = match inner.view {
            ExplorerPreview::File(_) => SearchPane::File,
            ExplorerPreview::Diff { .. } => SearchPane::Diff,
        };
        self.search_mode = false;
        self.search_origin = None;
        self.search_hit = None;
        match search {
            Some(query) => {
                self.search_active = true;
                self.search_query = query;
                self.search_target = pane;
            }
            None => {
                self.search_active = false;
                self.search_query.clear();
            }
        }
        self.explorer_apply_pending_line(tab_id, gen);
    }

    /// Move the inner tab of `tab_id` at load `gen` to its pending line
    /// once its view has loaded.
    ///
    /// A file body clamps the line to its length. A diff takes the row
    /// whose new line number is `line + 1`, unstaged section first (the
    /// top when no row shows it). On the active inner tab of the active
    /// Explorer tab the live diff cursor moves, and an armed search on that
    /// pane takes the row as its current hit (none for a line the view
    /// does not have); another inner tab keeps the row in its parked
    /// cursor.
    fn explorer_apply_pending_line(&mut self, tab_id: u64, gen: u64) {
        let active = self
            .tabs
            .active_explorer()
            .filter(|tab| tab.id == tab_id)
            .and_then(ExplorerTab::active_inner)
            .is_some_and(|inner| inner.gen == gen);
        let mode = self.diff_layout();
        let Some(inner) = self
            .tabs
            .get_explorer_id_mut(tab_id)
            .and_then(|tab| tab.inner_by_gen_mut(gen))
        else {
            return;
        };
        let Some(line) = inner.pending_line else {
            return;
        };
        let (pane, hit) = match &mut inner.view {
            ExplorerPreview::File(file) => {
                if file.body.is_none() {
                    return;
                }
                let len = file.lines().len();
                file.cursor = line.min(len.saturating_sub(1));
                // A line past the end clamps; it is no search hit.
                (SearchPane::File, (line < len).then_some(file.cursor))
            }
            ExplorerPreview::Diff {
                content, loading, ..
            } => {
                if *loading {
                    return;
                }
                let new_line = u32::try_from(line.saturating_add(1)).unwrap_or(u32::MAX);
                let row = row_for_new_line(content, mode, new_line);
                if !active {
                    inner.diff_cursor = row.unwrap_or(0);
                    inner.diff_scroll = u16::try_from(inner.diff_cursor).unwrap_or(u16::MAX);
                }
                (SearchPane::Diff, row)
            }
        };
        inner.pending_line = None;
        if !active {
            return;
        }
        if pane == SearchPane::Diff {
            self.diff_cursor = hit.unwrap_or(0);
            self.sync_diff_scroll();
        }
        if self.search_active && self.search_target == pane {
            self.search_hit = hit;
        }
    }

    /// Live-reload token of `rel` in `checkout`: its disk token (one
    /// `stat`) and the stamp (that token plus its snapshot `change`, so a
    /// stage, commit, or clean / changed flip moves it too).
    fn explorer_stamp(
        &self,
        checkout: &str,
        rel: &str,
        change: Option<&FileChange>,
    ) -> (String, String) {
        let disk = file_disk_token(&self.cwd, checkout, rel);
        let stamp = format!("{disk}|{change:?}");
        (disk, stamp)
    }

    /// Silent reload of the active inner file tab of the active Explorer
    /// tab of `checkout` when its stamp ([`Self::explorer_stamp`]) moved
    /// since the last load.
    ///
    /// Runs after each checkout status load (the watch poll), when an
    /// Explorer tab becomes active, and when an open inner tab shows again.
    /// Background tabs are not checked. The same kind of view keeps its
    /// painted content until the new load lands, with no status; a file
    /// that turned changed (or clean) swaps to a loading view of the other
    /// kind. A deleted file keeps the last content and warns
    /// [`FILE_DELETED`]. While an overlay holds input nothing is checked or
    /// recorded, so the next poll catches the change. One `stat`; no git.
    pub(crate) fn explorer_live_reload(&mut self, checkout: &str) -> Option<Effect> {
        if overlay_blocks_background_ticks(self.input_mode()) {
            return None;
        }
        let tab = self.tabs.active_explorer()?;
        if tab.checkout != checkout {
            return None;
        }
        let (tab_id, rel) = (tab.id, tab.active_inner()?.rel.clone());
        let change = self.explorer_change(checkout, &rel);
        let (disk, stamp) = self.explorer_stamp(checkout, &rel, change.as_ref());
        let tab = self.tabs.active_explorer_mut()?;
        let inner = tab.active_inner_mut()?;
        if inner.stamp.as_deref() == Some(stamp.as_str()) {
            return None;
        }
        inner.stamp = Some(stamp);
        if disk == GONE_DISK_TOKEN {
            let has_content = match &inner.view {
                ExplorerPreview::File(file) => file
                    .body
                    .as_deref()
                    .is_some_and(|body| !matches!(body, FileRead::Failed(_))),
                ExplorerPreview::Diff { loading, .. } => !loading,
            };
            if has_content {
                self.status = StatusMessage::warn(FILE_DELETED);
            }
            return None;
        }
        let changed = change.is_some();
        let same_kind = matches!(
            (&inner.view, changed),
            (ExplorerPreview::Diff { .. }, true) | (ExplorerPreview::File(_), false)
        );
        let gen = tab.next_gen();
        let flipped = (!same_kind).then(|| tab.loading_view(&rel, changed, gen));
        let inner = tab.active_inner_mut()?;
        inner.gen = gen;
        match flipped {
            Some(view) => {
                inner.view = view;
                (inner.diff_cursor, inner.diff_scroll, inner.diff_col_offset) = (0, 0, 0);
                (self.diff_cursor, self.diff_scroll, self.diff_col_offset) = (0, 0, 0);
                self.clear_diff_visual();
                self.clear_explorer_search();
            }
            None => {
                if let ExplorerPreview::File(file) = &mut inner.view {
                    file.generation = gen;
                }
            }
        }
        Some(Effect::LoadExplorerPreview {
            tab_id,
            gen,
            repo: checkout.to_string(),
            path: rel,
            change,
        })
    }

    /// [`Self::explorer_live_reload`] for the active Explorer tab's
    /// checkout, or [`Effect::None`] off an Explorer tab.
    pub(super) fn active_explorer_live_reload(&mut self) -> Effect {
        let Some(checkout) = self.tabs.active_explorer().map(|tab| tab.checkout.clone()) else {
            return Effect::None;
        };
        self.explorer_live_reload(&checkout).unwrap_or(Effect::None)
    }

    /// True while the active Explorer tab's active inner diff is loading.
    pub(crate) fn explorer_diff_loading(&self) -> bool {
        matches!(
            self.explorer_view(),
            Some(ExplorerPreview::Diff { loading: true, .. })
        )
    }

    /// The active Explorer tab's active inner diff: checkout, path, and
    /// diff.
    pub(crate) fn explorer_diff(&self) -> Option<(&str, &str, &super::super::diff::DiffContent)> {
        let tab = self.tabs.active_explorer()?;
        match &tab.active_inner()?.view {
            ExplorerPreview::Diff { rel, content, .. } => {
                Some((tab.checkout.as_str(), rel.as_str(), content))
            }
            ExplorerPreview::File(_) => None,
        }
    }

    /// Park the active Explorer tab's pane focus and `/` search, and its
    /// diff view in the active inner tab. Returns false off an Explorer
    /// tab.
    pub(super) fn park_explorer_session(&mut self) -> bool {
        let Some(tab_id) = self.active_explorer_id() else {
            return false;
        };
        let focus_preview = self.focus == FocusPane::Right;
        let search = ParkedSearch {
            mode: self.search_mode,
            active: self.search_active,
            query: self.search_query.clone(),
            target: self.search_target,
            hit: self.search_hit,
        };
        self.explorer_park_inner(tab_id);
        if let Some(tab) = self.tabs.active_explorer_mut() {
            tab.focus_preview = focus_preview;
            tab.search = search;
        }
        true
    }

    /// Restore the active Explorer tab's pane focus, its `/` search, and
    /// its active inner tab's diff view. Returns false off an Explorer tab.
    pub(super) fn apply_explorer_session(&mut self) -> bool {
        let Some(tab) = self.tabs.active_explorer() else {
            return false;
        };
        let inner = tab.active_inner();
        self.focus = if tab.focus_preview && inner.is_some() {
            FocusPane::Right
        } else {
            FocusPane::Left
        };
        (self.diff_cursor, self.diff_scroll, self.diff_col_offset) = inner
            .map_or((0, 0, 0), |inner| {
                (inner.diff_cursor, inner.diff_scroll, inner.diff_col_offset)
            });
        let search = tab.search.clone();
        self.search_mode = search.mode;
        self.search_active = search.active;
        self.search_query = search.query;
        self.search_target = search.target;
        self.search_hit = search.hit;
        self.search_origin = None;
        true
    }
}

#[cfg(test)]
impl AppState {
    /// Open `rel` of `checkout` the way Quick Open does (a pinned inner file
    /// tab of the checkout's Explorer tab) and land `lines` as its clean
    /// body. The folder listings are left in flight. Returns the Explorer
    /// tab id and the load generation.
    pub(crate) fn open_loaded_inner_file(
        &mut self,
        checkout: &str,
        rel: &str,
        lines: &[&str],
    ) -> (u64, u64) {
        let effect = self.open_file_in_explorer(checkout.into(), rel.into(), None, None);
        let effects = match effect {
            Effect::Batch(effects) => effects,
            other => vec![other],
        };
        let (tab_id, gen) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadExplorerPreview {
                    tab_id,
                    gen,
                    change: None,
                    ..
                } => Some((*tab_id, *gen)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("a clean file load of {rel}: {effects:?}"));
        let body = FileRead::Text {
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
            max_cols: lines.iter().map(|line| line.len()).max().unwrap_or(0),
        };
        assert!(self.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::File(body)));
        (tab_id, gen)
    }

    /// Open changed `rel` of `checkout` the way Quick Open does and land
    /// `diff` as its whole-file diff. Returns the Explorer tab id and the
    /// load generation.
    pub(crate) fn open_loaded_inner_diff(
        &mut self,
        checkout: &str,
        rel: &str,
        diff: super::super::diff::DiffContent,
    ) -> (u64, u64) {
        let effect = self.open_file_in_explorer(checkout.into(), rel.into(), None, None);
        let effects = match effect {
            Effect::Batch(effects) => effects,
            other => vec![other],
        };
        let (tab_id, gen) = effects
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadExplorerPreview {
                    tab_id,
                    gen,
                    change: Some(_),
                    ..
                } => Some((*tab_id, *gen)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("a diff load of {rel}: {effects:?}"));
        assert!(self.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        (tab_id, gen)
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

    use super::super::super::action::QuickOpenEntry;
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

    /// The Open in Explorer palette row.
    fn open_in_explorer_row() -> &'static PaletteCommand {
        PALETTE_COMMANDS
            .iter()
            .find(|command| command.action == Action::ExplorerReveal)
            .expect("Open in Explorer row")
    }

    /// Run palette row `title` from Quick Open commands mode.
    fn run_palette_row(app: &mut AppState, title: &str) -> Effect {
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in title.chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let palette = app.command_palette_mut().expect("palette open");
        palette.cursor = palette
            .visible()
            .iter()
            .position(|command| command.title == title)
            .unwrap_or_else(|| panic!("palette row {title}"));
        app.dispatch(Action::QuickOpenSubmit)
    }

    /// Off the Explorer-open rows `-` and the palette row refuse with the
    /// same copy: no Explorer tab, no commit-message resize.
    fn assert_minus_refuses(app: &mut AppState, want: &str, what: &str) {
        let lines = app.commit_msg_lines;
        let tabs = app.tabs.len();
        let active = app.tabs.active;
        assert_eq!(
            app.palette_disabled_reason(open_in_explorer_row())
                .as_deref(),
            Some(want),
            "palette: {what}"
        );
        app.status.clear();
        assert_eq!(app.dispatch(Action::ExplorerReveal), Effect::None, "{what}");
        assert_eq!(&*app.status, want, "status: {what}");
        assert_eq!(app.status.kind(), StatusKind::Warn, "{what}");
        assert_eq!(app.commit_msg_lines, lines, "no resize: {what}");
        assert_eq!(app.tabs.len(), tabs, "no new tab: {what}");
        assert_eq!(app.tabs.active, active, "same tab: {what}");
    }

    #[test]
    fn minus_off_a_checkout_tree_row_refuses_like_the_palette_row() {
        let mut app = state();
        focus(&mut app, NodeKind::Workspace, None, None);
        assert_minus_refuses(&mut app, FOCUS_A_TREE_ROW_TO_EXPLORE, "workspace row");

        // The graph pane is not the tree.
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        app.focus = FocusPane::Right;
        assert_eq!(app.list_focus_target(), ListFocusTarget::Graph);
        assert_minus_refuses(&mut app, FOCUS_A_TREE_ROW_TO_EXPLORE, "graph pane");
        app.focus = FocusPane::Left;

        app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
        assert!(app.is_compare_tab());
        assert_minus_refuses(&mut app, SWITCH_TO_WORKSPACE_TAB, "compare tab");
    }

    /// On an Explorer tab `-` is the parent folder, a different action, so
    /// the palette row is off there and never dispatches.
    #[test]
    fn open_in_explorer_row_is_off_on_an_explorer_tab() {
        let mut app = state();
        open_root(&mut app);
        assert_eq!(
            app.palette_disabled_reason(open_in_explorer_row())
                .as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB)
        );
        let lines = app.commit_msg_lines;
        assert_eq!(run_palette_row(&mut app, "Open in Explorer"), Effect::None);
        assert_eq!(&*app.status, SWITCH_TO_WORKSPACE_TAB);
        assert!(app.quick_open.is_some(), "a disabled row keeps the palette");
        assert_eq!(app.commit_msg_lines, lines);
        assert_eq!(app.tabs.len(), 2);
    }

    #[test]
    fn open_in_explorer_row_opens_the_checkout_from_a_tree_row() {
        let mut app = state();
        focus(
            &mut app,
            NodeKind::Checkout,
            Some("app/.worktrees/feat"),
            None,
        );
        assert_eq!(app.palette_disabled_reason(open_in_explorer_row()), None);
        let effect = run_palette_row(&mut app, "Open in Explorer");
        assert!(app.quick_open.is_none(), "the palette closes");
        assert_eq!(dir_loads(&flat(effect)), [""], "a new tab lists its root");
        assert!(app.is_explorer_tab());
        assert_eq!(tab(&app).checkout, "app/.worktrees/feat");
        assert_eq!(app.tabs.len(), 2);
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

        app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
        app.dispatch(Action::JumpToTab(1));
        open_root(&mut app);
        assert_eq!(app.tabs.active, 2);
        app.dispatch(Action::CloseTab);
        assert_eq!(app.tabs.len(), 2);
        assert!(app.is_compare_tab(), "the left neighbour is active");
        assert!(app.tabs.find_explorer("app").is_none());
    }

    /// Palette reason of the first row that runs `action`.
    fn palette_reason(app: &AppState, action: Action) -> Option<String> {
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.action == action)
            .unwrap_or_else(|| panic!("palette row for {action:?}"));
        app.palette_disabled_reason(command)
    }

    /// `action` refuses with `reason` on its key and on its palette row
    /// (when it has one), as a warning.
    fn assert_refused(app: &mut AppState, action: Action, reason: &str) {
        if PALETTE_COMMANDS
            .iter()
            .any(|command| command.action == action)
        {
            assert_eq!(
                palette_reason(app, action.clone()).as_deref(),
                Some(reason),
                "{action:?}"
            );
        }
        app.status.clear();
        assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
        assert_eq!(&*app.status, reason, "{action:?}");
        assert_eq!(app.status.kind(), StatusKind::Warn, "{action:?}");
    }

    #[test]
    fn t_dot_m_msg_rows_and_git_writes_still_refuse() {
        let mut app = state();
        open_root(&mut app);
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        for focus in [FocusPane::Left, FocusPane::Right] {
            app.focus = focus;
            for action in [
                Action::Stage,
                Action::Unstage,
                Action::Revert,
                Action::Push,
                Action::Pull,
                Action::Fetch,
                Action::DefaultBranch,
                Action::Branch,
                Action::RemoveWorktree,
                Action::StashMenu,
                Action::GraphCheckout,
                Action::GraphMerge,
                Action::CompareVsDefault,
                Action::CompareVsBranch,
                Action::CompareCommitVsParent,
            ] {
                assert_refused(&mut app, action, EXPLORER_TAB_READ_ONLY);
            }
            for action in [
                Action::ToggleTreeMode,
                Action::ToggleShowIgnored,
                Action::ToggleCommitMsgExpand,
                Action::ResizeCommitMsg(-1),
                Action::ResizeCommitMsg(1),
            ] {
                assert_refused(&mut app, action, SWITCH_TO_WORKSPACE_TAB);
            }
        }
        assert_eq!(app.tabs.len(), 2, "no compare tab opened");
    }

    #[test]
    fn ctrl_o_refuses_and_never_toggles_full_context() {
        let mut app = state();
        app.full_context.insert("file:keep".into());
        let before = app.full_context.clone();
        app.open_loaded_inner_diff(
            "app",
            "src/tui/app.rs",
            super::super::super::diff::DiffContent::from_unified("@@ -1 +1 @@\n-a\n+b\n"),
        );
        for focus in [FocusPane::Left, FocusPane::Right] {
            app.focus = focus;
            assert_refused(
                &mut app,
                Action::ToggleFullContext,
                EXPLORER_DIFFS_SHOW_WHOLE_FILE,
            );
            assert_eq!(app.full_context, before, "{focus:?}");
        }
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        assert_refused(
            &mut app,
            Action::ToggleFullContext,
            EXPLORER_DIFFS_SHOW_WHOLE_FILE,
        );
        assert_eq!(app.full_context, before);
    }

    /// `state()` with the Workspace tree on `lib`'s changed `lib.rs`, then
    /// `app`'s changed `src/tui/app.rs` open in its Explorer (inline
    /// rows: 0 label, 1 hunk, 2 ` a` (1,1), 3 `-b` (2,-), 4 `+c` (-,2)).
    fn changed_inner_state() -> AppState {
        let mut app = state();
        app.diff_mode = super::super::super::split::DiffMode::Inline;
        focus(&mut app, NodeKind::File, Some("lib"), Some("lib.rs"));
        app.open_loaded_inner_diff(
            "app",
            "src/tui/app.rs",
            super::super::super::diff::DiffContent::from_unified("@@ -1,2 +1,2 @@\n a\n-b\n+c\n"),
        );
        app
    }

    #[test]
    fn space_marks_changed_inner_file_reviewed() {
        use super::super::super::viewed::viewed_identity;
        let mut app = changed_inner_state();
        let identity = viewed_identity("app", "src/tui/app.rs");
        for (focus, marked, status) in [
            (FocusPane::Left, true, "marked reviewed"),
            (FocusPane::Right, false, "reviewed mark cleared"),
        ] {
            app.focus = focus;
            assert_eq!(palette_reason(&app, Action::ToggleReviewed), None);
            assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
            assert_eq!(
                app.viewed_store.contains_key(&identity),
                marked,
                "{focus:?}"
            );
            assert_eq!(&*app.status, status);
            assert!(
                !app.viewed_store
                    .contains_key(&viewed_identity("lib", "lib.rs")),
                "never the parked Workspace row"
            );
        }
        // A clean file has no change to mark.
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        assert_refused(
            &mut app,
            Action::ToggleReviewed,
            FOCUS_A_FILE_TO_MARK_REVIEWED,
        );
        assert!(app.viewed_store.is_empty());
    }

    #[test]
    fn e_diffs_inner_file() {
        let mut app = changed_inner_state();
        for focus in [FocusPane::Left, FocusPane::Right] {
            app.focus = focus;
            assert_eq!(palette_reason(&app, Action::ExternalDiff), None);
            assert_eq!(
                app.dispatch(Action::ExternalDiff),
                Effect::ExternalDiff {
                    repo: "app".into(),
                    path: "src/tui/app.rs".into(),
                    kind: ExternalDiffKind::Worktree,
                },
                "{focus:?}"
            );
        }
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        assert_refused(&mut app, Action::ExternalDiff, FOCUS_A_FILE_TO_DIFF);
        // No inner file at all.
        let mut app = state();
        open_root(&mut app);
        assert!(tab(&app).files.is_empty());
        assert_refused(&mut app, Action::ExternalDiff, FOCUS_A_FILE_TO_DIFF);
        assert_refused(
            &mut app,
            Action::ToggleReviewed,
            FOCUS_A_FILE_TO_MARK_REVIEWED,
        );
    }

    #[test]
    fn comment_and_export_target_inner_file_not_workspace_row() {
        use super::super::super::comments::{
            format_entity_reference, put_comment_entry, EntityRef,
        };
        use super::super::super::keys::InputMode;
        let readme = |line, end_line| CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line,
            end_line,
        };
        let prompt_key = |app: &AppState| app.comment.as_ref().map(|prompt| prompt.key.clone());
        let mut app = state();
        focus(&mut app, NodeKind::File, Some("lib"), Some("lib.rs"));
        app.open_loaded_inner_file("app", "README.md", &["a", "b", "c"]);

        // The tree has focus: `;` and `V` say to Tab over.
        for action in [Action::CommentStart, Action::DiffVisualStart] {
            assert_refused(&mut app, action, TAB_TO_THE_FILE_PANE);
        }
        assert_eq!(prompt_key(&app), None);
        assert_eq!(app.diff_visual_anchor, None);

        // `;` on the clean file: a working-tree line comment.
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::Move(1));
        assert_eq!(palette_reason(&app, Action::CommentStart), None);
        app.dispatch(Action::CommentStart);
        assert_eq!(prompt_key(&app), Some(readme(2, 2)));
        app.dispatch(Action::CommentCancel);

        // `V` `j` `;`: the highlighted lines.
        assert_eq!(palette_reason(&app, Action::DiffVisualStart), None);
        app.dispatch(Action::DiffVisualStart);
        assert_eq!(app.input_mode(), InputMode::DiffVisual);
        app.dispatch(Action::Move(1));
        assert_eq!(app.file_visual_range(), Some((1, 2)));
        // `'` in the highlight copies its span, in the diff range format.
        let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
            panic!("expected a copy");
        };
        assert_eq!(
            text,
            "kind: diff\nrepo: app\npath: README.md\nlines: 2-3\nsource: worktree\nside: new\n"
        );
        assert_eq!(app.file_visual_range(), Some((1, 2)), "the copy keeps it");
        app.dispatch(Action::CommentStart);
        assert_eq!(prompt_key(&app), Some(readme(2, 3)));
        assert_eq!(
            app.diff_visual_anchor, None,
            "the comment ends the highlight"
        );
        app.dispatch(Action::CommentCancel);

        // `y` exports the inner file's comments only, from either pane.
        app.comment_store =
            put_comment_entry(&app.comment_store, readme(2, 3), "readme note", false);
        let lib = CommentKey::WorktreeLine {
            repo: "lib".into(),
            branch: "main".into(),
            path: "lib.rs".into(),
            line: 1,
            end_line: 1,
        };
        app.comment_store = put_comment_entry(&app.comment_store, lib, "lib note", false);
        for focus in [FocusPane::Right, FocusPane::Left] {
            app.focus = focus;
            let Effect::CopyClipboard { text, .. } = app.dispatch(Action::ExportComments) else {
                panic!("expected an export");
            };
            assert!(text.contains("readme note"), "{text}");
            assert!(!text.contains("lib note"), "{text}");
            app.dispatch(Action::ExportCommentsCancel);
        }

        // `'` on the file pane names the inner file.
        app.focus = FocusPane::Right;
        let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
            panic!("expected a copy");
        };
        assert_eq!(
            text,
            format_entity_reference(&EntityRef::file("app", None, "README.md"))
        );

        // A changed inner file: `;` and `'` key the diff line.
        let mut app = changed_inner_state();
        assert_refused(&mut app, Action::CommentStart, TAB_TO_THE_FILE_PANE);
        app.dispatch(Action::FocusRight);
        app.diff_cursor = 4;
        app.dispatch(Action::CommentStart);
        assert_eq!(
            prompt_key(&app),
            Some(CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "src/tui/app.rs".into(),
                line: 2,
                end_line: 2,
            })
        );
        app.dispatch(Action::CommentCancel);
        let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
            panic!("expected a copy");
        };
        assert!(text.starts_with("kind: diff\n"), "{text}");
        assert!(text.contains("path: src/tui/app.rs\n"), "{text}");
        assert!(text.contains("lines: 2\n"), "{text}");
        // `V` highlights diff rows, as on the Workspace.
        app.diff_cursor = 2;
        app.dispatch(Action::DiffVisualStart);
        assert_eq!(app.diff_visual_anchor, Some(2));
        app.dispatch(Action::Move(2));
        app.dispatch(Action::CommentStart);
        assert_eq!(
            prompt_key(&app).map(|key| match key {
                CommentKey::WorktreeLine { line, end_line, .. } => (line, end_line),
                other => panic!("{other:?}"),
            }),
            Some((1, 2))
        );
        // The tree names the tree row, not the Workspace row.
        app.dispatch(Action::CommentCancel);
        app.focus = FocusPane::Left;
        let Effect::CopyClipboard { text, .. } = app.dispatch(Action::CopyEntityReference) else {
            panic!("expected a copy");
        };
        assert!(text.contains("path: src/tui/app.rs\n"), "{text}");
        assert!(text.starts_with("kind: file\n"), "{text}");
    }

    /// Open `app` at its root, then open `src` (listing landed).
    /// Rows: src/, src/tui/, src/lib.rs, target/, new.txt, README.md.
    fn open_src(app: &mut AppState) {
        open_root(app);
        let open = app.dispatch(Action::FoldOpen);
        land(app, open);
        assert_eq!(cursor_rel(app), "src");
    }

    /// Type `query` into an open `/` prompt; returns the last key's effect.
    fn type_query(app: &mut AppState, query: &str) -> Effect {
        query
            .chars()
            .map(|c| app.dispatch(Action::SearchChar(c)))
            .last()
            .unwrap_or(Effect::None)
    }

    /// The path of a preview load.
    fn preview_path(effect: &Effect) -> Option<&str> {
        match effect {
            Effect::LoadExplorerPreview { path, .. } => Some(path.as_str()),
            _ => None,
        }
    }

    #[test]
    fn slash_searches_explorer_rows_and_n_steps_with_preview() {
        let mut app = state();
        open_src(&mut app);
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        assert!(app.search_mode);
        assert_eq!(app.search_target, SearchPane::ExplorerTree);
        assert_eq!(app.search_target.title(), "tree");

        // Typing steps from the `/` origin (src): the first match after it.
        let typed = type_query(&mut app, ".");
        assert_eq!(cursor_rel(&app), "src/lib.rs");
        assert_eq!(preview_path(&typed), Some("src/lib.rs"), "a hit previews");
        assert_eq!(app.search_match_position(), Some((Some(1), 3)));
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_is_armed());

        let next = app.dispatch(Action::SearchNext);
        assert_eq!(cursor_rel(&app), "new.txt");
        assert_eq!(preview_path(&next), Some("new.txt"));
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor_rel(&app), "README.md");
        assert_eq!(app.search_match_position(), Some((Some(3), 3)));
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor_rel(&app), "src/lib.rs", "n wraps to the top");
        assert_eq!(&*app.status, super::super::SEARCH_WRAPPED_TO_TOP);
        app.dispatch(Action::SearchPrev);
        assert_eq!(cursor_rel(&app), "README.md", "N wraps to the bottom");

        // VS Code preview rules: one reusable preview tab, nothing pinned.
        let tab = tab(&app);
        assert_eq!(tab.files.len(), 1);
        assert_eq!(
            tab.active_inner().map(|inner| inner.rel.as_str()),
            Some("README.md")
        );
        assert!(!tab.files[0].pinned);
        assert_eq!(app.focus, FocusPane::Left, "the tree keeps focus");

        // A query with no listed match warns and leaves the cursor.
        app.dispatch(Action::SearchStart);
        type_query(&mut app, "app.rs");
        assert_eq!(cursor_rel(&app), "README.md", "src/tui is not listed yet");
        assert_eq!(&*app.status, super::super::SEARCH_NO_MATCH);
    }

    #[test]
    fn esc_restores_explorer_tree_cursor() {
        let mut app = state();
        open_src(&mut app);
        app.dispatch(Action::Move(2));
        assert_eq!(cursor_rel(&app), "src/lib.rs");
        app.dispatch(Action::SearchStart);
        type_query(&mut app, "readme");
        assert_eq!(cursor_rel(&app), "README.md");
        let esc = app.dispatch(Action::SearchCancel);
        assert_eq!(cursor_rel(&app), "src/lib.rs", "Esc puts the cursor back");
        assert!(!app.search_mode && !app.search_active);
        // The search hit replaced the preview tab; the origin row previews again.
        assert_eq!(preview_path(&esc), Some("src/lib.rs"));
        let tab = tab(&app);
        assert_eq!(
            tab.active_inner().map(|inner| inner.rel.as_str()),
            Some("src/lib.rs")
        );
        assert_eq!(tab.files.len(), 1);
    }

    #[test]
    fn esc_clears_an_armed_explorer_search() {
        let mut app = state();
        app.mouse_enabled = true;
        open_src(&mut app);
        app.dispatch(Action::SearchStart);
        type_query(&mut app, ".");
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_is_armed());
        assert_eq!(cursor_rel(&app), "src/lib.rs");
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(!app.search_is_armed());
        assert_eq!(&*app.status, "search cleared");
        assert_eq!(app.search_match_position(), None, "no match highlight");
        assert_eq!(cursor_rel(&app), "src/lib.rs", "the cursor stays");
        assert_eq!(app.focus, FocusPane::Left);

        // A right click clears it the same way.
        app.dispatch(Action::SearchStart);
        type_query(&mut app, "readme");
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_is_armed());
        app.dispatch(Action::BackClick);
        assert!(!app.search_is_armed());
        assert_eq!(cursor_rel(&app), "README.md");

        // An armed file search: the first Esc clears it, the next one
        // gives focus back to the tree.
        app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta"]);
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::SearchStart);
        type_query(&mut app, "beta");
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.search_target, SearchPane::File);
        assert!(app.search_is_armed());
        app.dispatch(Action::NavEsc);
        assert!(!app.search_is_armed());
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(app.tabs.active_file_view().map(|file| file.cursor), Some(1));
        app.dispatch(Action::NavEsc);
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn a_tree_search_step_that_stays_put_opens_no_preview_tab() {
        let mut app = state();
        open_root(&mut app);
        app.dispatch(Action::Move(3));
        assert_eq!(cursor_rel(&app), "README.md");
        app.dispatch(Action::CloseTab);
        assert!(tab(&app).files.is_empty(), "Ctrl-w closed the preview");

        app.dispatch(Action::SearchStart);
        assert_eq!(type_query(&mut app, "qqq"), Effect::None);
        assert_eq!(app.dispatch(Action::SearchSubmit), Effect::None);
        assert!(app.search_is_armed());
        assert_eq!(app.dispatch(Action::SearchNext), Effect::None);
        assert_eq!(app.dispatch(Action::SearchPrev), Effect::None);
        assert_eq!(&*app.status, super::super::SEARCH_NO_MATCH);
        assert_eq!(cursor_rel(&app), "README.md");
        assert!(tab(&app).files.is_empty(), "no match opens no preview tab");

        // A step that moves the cursor still previews.
        app.dispatch(Action::SearchStart);
        type_query(&mut app, "new");
        assert_eq!(cursor_rel(&app), "new.txt");
        assert_eq!(tab(&app).files.len(), 1);
    }

    #[test]
    fn slash_binds_the_inner_file_pane_when_the_right_pane_has_focus() {
        let mut app = state();
        app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta", "alphabet"]);
        app.dispatch(Action::FocusRight);
        assert_eq!(app.focus, FocusPane::Right);
        app.dispatch(Action::SearchStart);
        assert_eq!(app.search_target, SearchPane::File);
        type_query(&mut app, "alpha");
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.tabs.active_file_view().map(|file| file.cursor), Some(2));

        app.dispatch(Action::FocusLeft);
        app.dispatch(Action::SearchStart);
        assert_eq!(app.search_target, SearchPane::ExplorerTree);
        app.dispatch(Action::SearchCancel);

        // A changed file shows its diff: `/` binds the diff.
        let effect = app.open_file_in_explorer("app".into(), "new.txt".into(), None, None);
        let gen = flat(effect)
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadExplorerPreview {
                    gen,
                    change: Some(_),
                    ..
                } => Some(*gen),
                _ => None,
            })
            .expect("a diff load");
        let tab_id = tab(&app).id;
        let diff = super::super::super::diff::DiffContent::from_unified("@@ -0,0 +1 @@\n+new\n");
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::SearchStart);
        assert_eq!(app.search_target, SearchPane::Diff);
    }

    #[test]
    fn explorer_tree_search_is_parked_per_tab_and_survives_previews() {
        let mut app = state();
        open_src(&mut app);
        app.dispatch(Action::SearchStart);
        type_query(&mut app, ".");
        app.dispatch(Action::SearchSubmit);
        // Previews change the active inner tab; the tree search stays.
        app.dispatch(Action::SearchNext);
        app.dispatch(Action::Move(1));
        assert!(app.search_is_armed());
        assert_eq!(app.search_target, SearchPane::ExplorerTree);

        // Workspace has its own search; the Explorer's comes back.
        app.dispatch(Action::JumpToTab(1));
        assert!(app.tabs.is_workspace());
        assert!(!app.search_active, "the Workspace search was never armed");
        app.dispatch(Action::JumpToTab(2));
        assert!(app.is_explorer_tab());
        assert!(app.search_is_armed());
        assert_eq!(app.search_query, ".");
        assert_eq!(app.search_target, SearchPane::ExplorerTree);
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor_rel(&app), "src/lib.rs", "n wraps from README.md");

        // An in-file search still drops when the active inner file changes.
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::SearchStart);
        type_query(&mut app, "x");
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.search_target, SearchPane::File);
        assert!(app.search_active);
        app.dispatch(Action::FocusLeft);
        app.dispatch(Action::Move(2));
        assert_eq!(cursor_rel(&app), "new.txt");
        assert!(!app.search_active, "the file search cleared");
    }

    #[test]
    fn zz_collapses_and_expands_subtree() {
        let mut app = state();
        open_root(&mut app);
        assert_eq!(cursor_rel(&app), "src");
        // First z opens src (listing landed); the second opens one level more.
        let first = app.dispatch(Action::FoldToggle);
        assert!(app.z_pending_at.is_some(), "z on a folder arms zz");
        land(&mut app, first);
        assert!(tab(&app).tree.is_expanded("src"));
        let deeper = app.dispatch(Action::FoldToggleSubtree);
        assert_eq!(dir_loads(&flat(deeper.clone())), ["src/tui"]);
        land(&mut app, deeper);
        assert!(tab(&app).tree.is_expanded("src/tui"));
        let rows = app.explorer_rows().expect("rows").0;
        assert!(rows.iter().any(|row| row.rel == "src/tui/app.rs"));

        // zz on the open folder: z closes it, the second z closes the subtree.
        app.dispatch(Action::FoldToggle);
        assert!(!tab(&app).tree.is_expanded("src"));
        assert_eq!(app.dispatch(Action::FoldToggleSubtree), Effect::None);
        assert!(!tab(&app).tree.is_expanded("src/tui"));
        assert_eq!(cursor_rel(&app), "src");

        // zz again: every cached folder opens at once, nothing to list.
        app.dispatch(Action::FoldToggle);
        assert_eq!(app.dispatch(Action::FoldToggleSubtree), Effect::None);
        assert!(tab(&app).tree.is_expanded("src/tui"));

        // On the ignored `target/`, zz lists nothing below it.
        app.dispatch(Action::FoldToggle);
        app.dispatch(Action::Move(1));
        assert_eq!(cursor_rel(&app), "target");
        let open = app.dispatch(Action::FoldToggle);
        land(&mut app, open);
        assert_eq!(app.dispatch(Action::FoldToggleSubtree), Effect::None);

        // A file row arms no zz; the right pane says z folds tree rows.
        app.dispatch(Action::Move(1));
        app.dispatch(Action::FoldToggle);
        assert!(app.z_pending_at.is_none());
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::FoldToggleSubtree);
        assert_eq!(&*app.status, super::super::Z_FOLDS_TREE_ROWS);
    }

    #[test]
    fn angle_brackets_resize_shared_tree_fraction() {
        use super::super::super::split::pane_widths;
        let mut app = state();
        app.layout.term_cols = 140;
        let width = |app: &AppState| pane_widths(140, app.tree_fraction).tree_width;
        open_root(&mut app);
        assert_eq!(width(&app), 56);
        assert_eq!(app.dispatch(Action::ResizeTree(1)), Effect::None);
        assert_eq!(width(&app), 63);
        app.dispatch(Action::ResizeTree(-1));
        app.dispatch(Action::ResizeTree(-1));
        assert_eq!(width(&app), 49);
        app.dispatch(Action::JumpToTab(1));
        assert_eq!(width(&app), 49, "one split for every tab");
    }

    #[test]
    fn explorer_palette_enables_search_fold_and_resize_rows() {
        let mut app = state();
        open_root(&mut app);
        let reason = |app: &AppState, action: Action| {
            let command = PALETTE_COMMANDS
                .iter()
                .find(|command| command.action == action)
                .unwrap_or_else(|| panic!("palette row for {action:?}"));
            app.palette_disabled_reason(command)
        };
        for action in [
            Action::SearchStart,
            Action::FoldToggleSubtree,
            Action::ResizeTree(-1),
            Action::ResizeTree(1),
        ] {
            assert_eq!(reason(&app, action.clone()), None, "{action:?}");
        }
        assert_eq!(
            reason(&app, Action::SearchNext).as_deref(),
            Some(NO_SEARCH_ARMED)
        );
        // `t`, `.`, `M`, `+`: the row and the key give the same copy.
        for action in [
            Action::ToggleTreeMode,
            Action::ToggleShowIgnored,
            Action::ToggleCommitMsgExpand,
            Action::ResizeCommitMsg(1),
        ] {
            assert_eq!(
                reason(&app, action.clone()).as_deref(),
                Some(SWITCH_TO_WORKSPACE_TAB),
                "{action:?}"
            );
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(&*app.status, SWITCH_TO_WORKSPACE_TAB, "{action:?}");
            assert_eq!(app.status.kind(), StatusKind::Warn, "{action:?}");
        }
        // `-` stays the parent folder.
        let open = app.dispatch(Action::FoldOpen);
        land(&mut app, open);
        app.dispatch(Action::Move(1));
        assert_eq!(cursor_rel(&app), "src/tui");
        app.dispatch(Action::ExplorerReveal);
        assert_eq!(cursor_rel(&app), "src");
    }

    #[test]
    fn fold_keys_and_tab_move_between_tree_and_preview() {
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
        app.dispatch(Action::FocusRight);
        assert_eq!(app.focus, FocusPane::Right, "Tab focuses the file");
        app.dispatch(Action::NavEsc);
        assert_eq!(app.focus, FocusPane::Left, "Esc goes back to the tree");
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::FoldClose);
        assert_eq!(app.focus, FocusPane::Left, "h in the preview too");
        assert_eq!(cursor_rel(&app), "src/lib.rs", "without moving the tree");
        app.dispatch(Action::FoldClose);
        assert_eq!(cursor_rel(&app), "src", "h on a file jumps to its folder");
        assert!(
            app.apply_explorer_preview(tab_id, gen, body()),
            "a folder row keeps the file tab: its read still lands"
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
        assert!(
            app.tabs.get_explorer_id(tab_id).is_some(),
            "the first Ctrl-w closes the file tab"
        );
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
        let view = |id: u64| {
            &app.tabs
                .get_explorer_id(id)
                .unwrap()
                .active_inner()
                .unwrap()
                .view
        };
        match view(app_tab) {
            ExplorerPreview::File(file) => assert_eq!(file.lines(), ["app readme"]),
            other => panic!("{other:?}"),
        }
        match view(lib_tab) {
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

    /// `(rel, pinned)` of each inner file tab, in paint order.
    fn inners(app: &AppState) -> Vec<(String, bool)> {
        tab(app)
            .files
            .iter()
            .map(|file| (file.rel.clone(), file.pinned))
            .collect()
    }

    fn active_rel(app: &AppState) -> Option<String> {
        tab(app).active_inner().map(|file| file.rel.clone())
    }

    fn pinned(rel: &str) -> (String, bool) {
        (rel.into(), true)
    }

    fn preview(rel: &str) -> (String, bool) {
        (rel.into(), false)
    }

    /// `(tab_id, gen, path, changed)` of an inner tab load.
    fn load_of(effect: &Effect) -> (u64, u64, String, bool) {
        match effect {
            Effect::LoadExplorerPreview {
                tab_id,
                gen,
                path,
                change,
                ..
            } => (*tab_id, *gen, path.clone(), change.is_some()),
            other => panic!("an inner tab load, got {other:?}"),
        }
    }

    /// Root rows: src/, target/, new.txt (untracked), README.md (clean).
    #[test]
    fn tree_move_replaces_the_one_preview_tab() {
        let mut app = state();
        open_root(&mut app);
        assert!(inners(&app).is_empty(), "a folder row opens no file tab");
        let (_, _, path, changed) = load_of(&app.dispatch(Action::Move(2)));
        assert_eq!((path.as_str(), changed), ("new.txt", true));
        assert_eq!(inners(&app), [preview("new.txt")]);
        let (_, _, path, changed) = load_of(&app.dispatch(Action::Move(1)));
        assert_eq!((path.as_str(), changed), ("README.md", false));
        assert_eq!(inners(&app), [preview("README.md")], "replaced in place");
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
        assert_eq!(app.tabs.len(), 2, "no top-level file tab");
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn stale_gen_of_a_replaced_preview_is_dropped() {
        let mut app = state();
        open_root(&mut app);
        let (tab_id, old, ..) = load_of(&app.dispatch(Action::Move(2)));
        let (_, new, ..) = load_of(&app.dispatch(Action::Move(1)));
        assert!(new > old);
        let diff = super::super::super::diff::DiffContent::from_unified("+new\n");
        assert!(
            !app.apply_explorer_preview(tab_id, old, ExplorerPreviewBody::Diff(diff)),
            "the replaced preview's load is stale"
        );
        let body = ExplorerPreviewBody::File(FileRead::Text {
            lines: vec!["readme".into()],
            max_cols: 6,
        });
        assert!(app.apply_explorer_preview(tab_id, new, body));
        assert_eq!(
            app.tabs
                .active_file_view()
                .map(|file| file.lines().to_vec()),
            Some(vec!["readme".to_string()])
        );
    }

    #[test]
    fn enter_pins_and_focus_stays_on_tree() {
        let mut app = state();
        open_root(&mut app);
        app.dispatch(Action::Move(2));
        assert_eq!(
            app.dispatch(Action::NavEnter),
            Effect::None,
            "the previewed file is already loading"
        );
        assert_eq!(inners(&app), [pinned("new.txt")]);
        assert_eq!(app.focus, FocusPane::Left, "Enter keeps the tree focused");
        let (_, _, path, _) = load_of(&app.dispatch(Action::Move(1)));
        assert_eq!(path, "README.md");
        assert_eq!(
            inners(&app),
            [pinned("new.txt"), preview("README.md")],
            "a pinned tab is kept; the new preview opens after it"
        );
        app.dispatch(Action::NavEnter);
        assert_eq!(inners(&app), [pinned("new.txt"), pinned("README.md")]);
        assert_eq!(app.focus, FocusPane::Left);
        app.dispatch(Action::FocusRight);
        assert_eq!(app.focus, FocusPane::Right, "Tab still focuses the file");
    }

    #[test]
    fn move_onto_a_pinned_file_shows_it_without_a_duplicate_preview() {
        let mut app = state();
        open_root(&mut app);
        app.dispatch(Action::Move(2));
        app.dispatch(Action::NavEnter);
        load_of(&app.dispatch(Action::Move(1)));
        assert_eq!(
            app.dispatch(Action::Move(-1)),
            Effect::None,
            "new.txt is a tab: no reload"
        );
        assert_eq!(inners(&app), [pinned("new.txt"), preview("README.md")]);
        assert_eq!(active_rel(&app).as_deref(), Some("new.txt"));
        assert_eq!(
            app.dispatch(Action::Move(1)),
            Effect::None,
            "the preview tab of README.md shows again"
        );
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
        assert_eq!(inners(&app).len(), 2);
    }

    #[test]
    fn folder_row_keeps_the_preview() {
        let mut app = state();
        open_root(&mut app);
        let (tab_id, gen, ..) = load_of(&app.dispatch(Action::Move(3)));
        assert_eq!(app.dispatch(Action::MoveToStart), Effect::None);
        assert_eq!(cursor_rel(&app), "src");
        assert_eq!(inners(&app), [preview("README.md")]);
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
        assert!(app.apply_explorer_preview(
            tab_id,
            gen,
            ExplorerPreviewBody::File(FileRead::Binary)
        ));
        let open = app.dispatch(Action::FoldOpen);
        land(&mut app, open);
        assert_eq!(app.dispatch(Action::Move(1)), Effect::None, "src/tui");
        assert_eq!(inners(&app), [preview("README.md")]);
        assert!(
            app.tabs.active_file_view().is_some(),
            "README.md still paints"
        );
    }

    #[test]
    fn brackets_switch_inner_tabs_and_select_the_tree_row() {
        let mut app = state();
        app.dispatch(Action::NextFileTab);
        assert_eq!(&*app.status, EXPLORER_FILE_TABS_ONLY, "off an Explorer tab");
        open_root(&mut app);
        app.dispatch(Action::NextFileTab);
        assert_eq!(&*app.status, NO_EXPLORER_FILE_TABS);
        let row = |action: Action| {
            PALETTE_COMMANDS
                .iter()
                .find(|command| command.action == action)
                .expect("palette row")
        };
        assert_eq!(
            app.palette_disabled_reason(row(Action::NextFileTab))
                .as_deref(),
            Some(NO_EXPLORER_FILE_TABS)
        );

        // new.txt pinned, then src/lib.rs pinned, then README.md previewed.
        app.dispatch(Action::Move(2));
        app.dispatch(Action::NavEnter);
        app.dispatch(Action::MoveToStart);
        let open = app.dispatch(Action::FoldOpen);
        land(&mut app, open);
        let (_, _, path, _) = load_of(&app.dispatch(Action::Move(2)));
        assert_eq!(path, "src/lib.rs");
        app.dispatch(Action::NavEnter);
        load_of(&app.dispatch(Action::MoveToEnd));
        assert_eq!(
            inners(&app),
            [
                pinned("new.txt"),
                pinned("src/lib.rs"),
                preview("README.md")
            ]
        );
        assert_eq!(
            app.palette_disabled_reason(row(Action::PreviousFileTab)),
            None
        );

        // Close src/ so `[` has to open it again to select src/lib.rs.
        app.dispatch(Action::MoveToStart);
        app.dispatch(Action::FoldClose);
        assert!(!tab(&app).tree.is_expanded("src"));
        app.dispatch(Action::NextFileTab);
        assert_eq!(active_rel(&app).as_deref(), Some("new.txt"), "] wraps");
        assert_eq!(cursor_rel(&app), "new.txt");
        app.dispatch(Action::NextFileTab);
        assert_eq!(active_rel(&app).as_deref(), Some("src/lib.rs"));
        assert!(tab(&app).tree.is_expanded("src"), "revealed");
        assert_eq!(cursor_rel(&app), "src/lib.rs");
        app.dispatch(Action::PreviousFileTab);
        app.dispatch(Action::PreviousFileTab);
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"), "[ wraps");
        assert_eq!(cursor_rel(&app), "README.md");
        assert_eq!(tab(&app).preview_index(), Some(2), "the preview stays");
        assert_eq!(app.focus, FocusPane::Left);

        app.dispatch(Action::JumpToTab(1));
        assert_eq!(
            app.palette_disabled_reason(row(Action::NextFileTab))
                .as_deref(),
            Some(EXPLORER_FILE_TABS_ONLY)
        );
    }

    #[test]
    fn ctrl_w_closes_inner_tabs_then_the_explorer_tab() {
        let mut app = state();
        open_root(&mut app);
        app.dispatch(Action::Move(2));
        app.dispatch(Action::NavEnter);
        load_of(&app.dispatch(Action::Move(1)));
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::MoveToStart);
        assert_eq!(inners(&app), [pinned("new.txt"), preview("README.md")]);
        app.dispatch(Action::CloseTab);
        assert_eq!(inners(&app), [pinned("new.txt")], "the active tab closes");
        assert_eq!(
            active_rel(&app).as_deref(),
            Some("new.txt"),
            "left neighbour"
        );
        assert_eq!(cursor_rel(&app), "new.txt", "selected in the tree");
        assert_eq!(app.tabs.len(), 2);
        app.dispatch(Action::CloseTab);
        assert!(inners(&app).is_empty());
        assert_eq!(app.focus, FocusPane::Left, "nothing left to focus");
        assert!(app.is_explorer_tab(), "the Explorer tab stays");
        app.dispatch(Action::CloseTab);
        assert!(app.tabs.is_workspace(), "then Ctrl-w closes it");
        assert_eq!(app.tabs.len(), 1);
    }

    #[test]
    fn ctrl_w_on_the_first_inner_tab_activates_the_next_one() {
        let mut app = state();
        open_root(&mut app);
        app.dispatch(Action::Move(2));
        app.dispatch(Action::NavEnter);
        app.dispatch(Action::Move(1));
        app.dispatch(Action::NavEnter);
        app.dispatch(Action::PreviousFileTab);
        assert_eq!(active_rel(&app).as_deref(), Some("new.txt"));
        app.dispatch(Action::CloseTab);
        assert_eq!(inners(&app), [pinned("README.md")]);
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
        assert_eq!(cursor_rel(&app), "README.md");
    }

    #[test]
    fn changed_file_preview_shows_the_full_context_marker() {
        let mut app = state();
        open_root(&mut app);
        let (tab_id, gen, _, changed) = load_of(&app.dispatch(Action::Move(2)));
        assert!(changed, "new.txt is untracked: a diff");
        let diff = super::super::super::diff::DiffContent::from_unified("@@ -0,0 +1 @@\n+new\n");
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        assert!(app.explorer_diff().is_some());
        assert_eq!(
            app.diff_context_lines(),
            Some(crate::git::FULL_DIFF_CONTEXT_LINES)
        );
        assert!(
            app.full_context_active(),
            "the diff header says full context"
        );
    }

    #[test]
    fn clean_file_preview_is_plain_body() {
        let mut app = state();
        open_root(&mut app);
        let (tab_id, gen, path, changed) = load_of(&app.dispatch(Action::Move(3)));
        assert_eq!((path.as_str(), changed), ("README.md", false));
        let body = ExplorerPreviewBody::File(FileRead::Text {
            lines: vec!["# app".into(), "text".into()],
            max_cols: 5,
        });
        assert!(app.apply_explorer_preview(tab_id, gen, body));
        assert!(app.explorer_diff().is_none());
        assert!(!app.full_context_active());
        let file = app.tabs.active_file_view().expect("a file body");
        assert_eq!(file.lines(), ["# app", "text"]);
        assert_eq!(file.generation, gen);
    }

    /// After Ctrl-w empties the inner tabs, no listing brings a file tab
    /// back: not one in flight since before the close, not `r`, not a
    /// status re-list.
    #[test]
    fn listing_after_ctrl_w_empties_the_inner_tabs_previews_nothing() {
        let mut app = state();
        open_root(&mut app);
        // src/ opens, its listing still in flight.
        let in_flight = app.dispatch(Action::FoldOpen);
        assert_eq!(dir_loads(&flat(in_flight.clone())), ["src"]);
        // Root rows now: src, (loading), target, new.txt, README.md.
        load_of(&app.dispatch(Action::MoveToEnd));
        assert_eq!(cursor_rel(&app), "README.md");
        app.dispatch(Action::CloseTab);
        assert!(inners(&app).is_empty());
        assert!(app.is_explorer_tab());
        let no_preview = |effects: &[Effect]| {
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadExplorerPreview { .. }))
        };

        let follow = land(&mut app, in_flight);
        assert!(no_preview(&follow), "in-flight listing: {follow:?}");
        assert!(inners(&app).is_empty());

        let refresh = app.dispatch(Action::Refresh);
        let follow = land(&mut app, refresh);
        assert!(no_preview(&follow), "`r`: {follow:?}");
        assert!(inners(&app).is_empty());

        let mut changes = app.snapshot.repos[0].changes.clone();
        changes.push(change("src/fresh.rs", None, true));
        let snap = repo("app", CheckoutKind::Primary, None, changes);
        crate::tui::app::apply_one_repo_snapshot(&mut app, "app", Some(snap));
        let relist = app.explorer_status_relist("app").expect("a re-list of src");
        let follow = land(&mut app, relist);
        assert!(no_preview(&follow), "status re-list: {follow:?}");
        assert!(inners(&app).is_empty());
        assert_eq!(cursor_rel(&app), "README.md");

        app.dispatch(Action::CloseTab);
        assert!(
            app.tabs.is_workspace(),
            "Ctrl-w now closes the Explorer tab"
        );
    }

    /// G3: a reveal whose cursor path is not listed yet resolves the
    /// cursor to a stand-in row. When that row is a file, nothing previews
    /// and the inner tabs stay until the real listing lands; then the
    /// revealed file previews.
    #[test]
    fn reveal_onto_a_fallback_file_row_previews_only_the_revealed_file() {
        let mut app = state();
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        let Effect::LoadExplorerDir { tab_id, req, .. } = app.dispatch(Action::ExplorerReveal)
        else {
            panic!("a root listing");
        };
        // A root of files only: the first row is a file.
        let follow = app
            .apply_explorer_dir(tab_id, req, "", Ok(vec![entry("README.md", false, false)]))
            .expect("fresh listing");
        assert_eq!(
            load_of(&follow).2,
            "README.md",
            "no stored cursor: first row"
        );
        assert_eq!(inners(&app), [preview("README.md")]);

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
        assert_eq!(dir_loads(&effects), [""], "the stale root lists again");
        assert!(
            !effects
                .iter()
                .any(|effect| matches!(effect, Effect::LoadExplorerPreview { .. })),
            "the stand-in row README.md must not preview: {effects:?}"
        );
        let (rows, cursor) = app.explorer_rows().unwrap();
        assert_eq!(rows[cursor.unwrap()].rel, "README.md", "the stand-in row");
        assert_eq!(inners(&app), [preview("README.md")], "unchanged");
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"));

        let Some(Effect::LoadExplorerDir { req, .. }) = effects.first().cloned() else {
            panic!("{effects:?}");
        };
        let listing = vec![
            entry("fresh.txt", false, false),
            entry("README.md", false, false),
        ];
        let follow = app
            .apply_explorer_dir(tab_id, req, "", Ok(listing))
            .expect("fresh listing");
        assert_eq!(load_of(&follow).2, "fresh.txt");
        assert_eq!(inners(&app), [preview("fresh.txt")]);
    }

    /// Quick Open (files) over `paths` of `checkout`, with `hits` (entry
    /// indices) scored.
    fn quick_open_with(app: &mut AppState, checkout: &str, paths: &[&str], hits: &[usize]) {
        use super::super::super::action::QuickOpenEntry;
        use super::super::super::quick_open::{FileIndexState, QuickOpenState};
        use crate::file_index::{FileEntry, FileHit, FileIndex, IndexRoot};
        let mut quick = QuickOpenState::new(QuickOpenEntry::Files, app.quick_open_scope());
        quick.index = FileIndexState::Ready(Arc::new(FileIndex {
            roots: vec![IndexRoot {
                checkout: checkout.into(),
                prefix: String::new(),
            }],
            entries: paths
                .iter()
                .map(|path| FileEntry {
                    root: 0,
                    display: (*path).to_string(),
                    rel_start: 0,
                })
                .collect(),
            truncated: false,
            errors: Vec::new(),
        }));
        quick.hits = hits
            .iter()
            .map(|&entry| FileHit {
                entry,
                score: 1,
                indices: Vec::new(),
            })
            .collect();
        app.quick_open = Some(quick);
    }

    fn has_file_load(effect: &Effect) -> bool {
        flat(effect.clone())
            .iter()
            .any(|effect| matches!(effect, Effect::LoadExplorerPreview { .. }))
    }

    /// The inner file load of `effect`: `(tab_id, gen)`.
    fn file_load(effect: &Effect) -> (u64, u64) {
        flat(effect.clone())
            .iter()
            .find_map(|effect| match effect {
                Effect::LoadExplorerPreview { tab_id, gen, .. } => Some((*tab_id, *gen)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("an inner file load: {effect:?}"))
    }

    fn text(lines: &[&str]) -> ExplorerPreviewBody {
        ExplorerPreviewBody::File(FileRead::Text {
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
            max_cols: lines.iter().map(|line| line.len()).max().unwrap_or(0),
        })
    }

    #[test]
    fn quick_open_from_workspace_pins_in_explorer() {
        let mut app = state();
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        app.focus = FocusPane::Right;
        quick_open_with(&mut app, "app", &["README.md", "src/lib.rs"], &[1, 0]);
        let effect = app.dispatch(Action::QuickOpenSubmit);
        assert!(app.quick_open.is_none(), "Enter closes the overlay");
        assert!(app.is_explorer_tab());
        assert_eq!(tab(&app).checkout, "app");
        assert_eq!(
            dir_loads(&flat(effect.clone())),
            ["", "src"],
            "the tree reveals the file"
        );
        assert!(has_file_load(&effect), "{effect:?}");
        assert_eq!(
            inners(&app),
            [pinned("src/lib.rs")],
            "pinned, not a preview"
        );
        assert_eq!(app.focus, FocusPane::Left, "the tree keeps focus");
        assert_eq!(tab(&app).tree.cursor_rel(), Some("src/lib.rs"));
        // Once the listings land the tree row of the file is selected, and
        // the open inner tab is not loaded twice.
        let follow = land(&mut app, effect);
        let loads = |follow: &[Effect]| {
            follow
                .iter()
                .filter(|effect| matches!(effect, Effect::LoadExplorerPreview { .. }))
                .count()
        };
        assert_eq!(loads(&follow), 1, "only the open's own load: {follow:?}");
        assert_eq!(cursor_rel(&app), "src/lib.rs");
        assert_eq!(inners(&app), [pinned("src/lib.rs")]);
    }

    #[test]
    fn quick_open_from_compare_tab_routes_to_checkout_explorer() {
        let mut app = state();
        app.open_compare_tab("lib".into(), "main".into(), "HEAD".into());
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout("lib".into())
        );
        quick_open_with(&mut app, "lib", &["lib.rs", "README.md"], &[1]);
        let effect = app.dispatch(Action::QuickOpenSubmit);
        assert!(has_file_load(&effect), "{effect:?}");
        assert_eq!(
            app.tabs.labels(),
            ["Workspace", "lib ↔ main", "Explorer · lib"]
        );
        assert_eq!(tab(&app).checkout, "lib");
        assert_eq!(inners(&app), [pinned("README.md")]);
        assert_eq!(app.focus, FocusPane::Left);
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout("lib".into()),
            "Quick Open on the Explorer tab keeps to its checkout"
        );
    }

    #[test]
    fn reopen_same_file_reuses_inner_tab() {
        let mut app = state();
        let first = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
        let (tab_id, gen) = file_load(&first);
        assert!(app.apply_explorer_preview(tab_id, gen, text(&["# app"])));
        app.dispatch(Action::JumpToTab(1));
        let again = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
        assert!(!has_file_load(&again), "already open and loaded: {again:?}");
        assert_eq!(app.tabs.len(), 2, "one Explorer tab");
        assert_eq!(inners(&app), [pinned("README.md")], "one inner tab");

        // A tree preview of the file is promoted, not duplicated.
        let mut app = state();
        open_root(&mut app);
        load_of(&app.dispatch(Action::MoveToEnd));
        assert_eq!(inners(&app), [preview("README.md")]);
        app.dispatch(Action::JumpToTab(1));
        let open = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
        assert!(!has_file_load(&open), "{open:?}");
        assert_eq!(inners(&app), [pinned("README.md")]);
    }

    #[test]
    fn search_hit_lands_on_line_in_clean_file_and_arms_n() {
        let mut app = state();
        let effect = app.open_file_in_explorer(
            "app".into(),
            "README.md".into(),
            Some(3),
            Some("foo".into()),
        );
        let (tab_id, gen) = file_load(&effect);
        // Armed before the load lands, and still armed after the folder
        // listings land first.
        let follow = land(&mut app, effect);
        assert_eq!(
            follow,
            [Effect::LoadExplorerPreview {
                tab_id,
                gen,
                repo: "app".into(),
                path: "README.md".into(),
                change: None,
            }],
            "only the open's own load"
        );
        assert!(app.search_is_armed());
        assert_eq!(app.search_target, SearchPane::File);
        assert_eq!(app.search_query, "foo");
        assert_eq!(tab(&app).active_inner().unwrap().pending_line, Some(3));

        let lines = ["foo", "x", "foo", "foo bar", "y", "foo"];
        assert!(app.apply_explorer_preview(tab_id, gen, text(&lines)));
        let cursor = |app: &AppState| app.tabs.active_file_view().unwrap().cursor;
        assert_eq!(cursor(&app), 3);
        assert_eq!(app.search_hit, Some(3));
        assert_eq!(tab(&app).active_inner().unwrap().pending_line, None);
        assert_eq!(app.focus, FocusPane::Left, "the tree keeps focus");
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 5, "`n` steps from the hit line");
        app.dispatch(Action::SearchPrev);
        assert_eq!(cursor(&app), 3);
        let next = PALETTE_COMMANDS
            .iter()
            .find(|command| command.action == Action::SearchNext)
            .expect("next match row");
        assert_eq!(app.palette_disabled_reason(next), None);
    }

    #[test]
    fn search_hit_lands_on_diff_row_in_changed_file() {
        let mut app = state();
        // `src/tui/app.rs` is modified in `app`: it opens as a diff.
        let effect = app.open_file_in_explorer(
            "app".into(),
            "src/tui/app.rs".into(),
            Some(1),
            Some("two".into()),
        );
        let (tab_id, gen) = file_load(&effect);
        assert!(app.explorer_diff_loading());
        assert_eq!(app.search_target, SearchPane::Diff);
        assert!(app.search_is_armed());
        // Rows: STAGED 0, hunk 1, `one` 2, `-two` 3; UNSTAGED 4, hunk 5,
        // `one` 6, `+two` 7, `three two` 8.
        let diff = super::super::super::diff::DiffContent {
            staged: "@@ -1,2 +1 @@\n one\n-two\n".into(),
            unstaged: "@@ -1 +1,3 @@\n one\n+two\n+three two\n".into(),
            ..Default::default()
        };
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        assert_eq!(app.diff_cursor, 7, "line 2 of the file, unstaged first");
        assert_eq!(app.search_hit, Some(7));
        app.dispatch(Action::SearchNext);
        assert_eq!(app.diff_cursor, 8, "`n` steps from the hit row");
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.diff_cursor, 7);

        // Without a query the cursor still lands; nothing is armed.
        let mut app = state();
        let effect = app.open_file_in_explorer("app".into(), "new.txt".into(), Some(0), None);
        let (tab_id, gen) = file_load(&effect);
        let diff = super::super::super::diff::DiffContent::from_unified("@@ -0,0 +1 @@\n+new\n");
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        assert_eq!(app.diff_cursor, 2);
        assert!(!app.search_is_armed());
    }

    #[test]
    fn no_session_tab_is_a_file_tab() {
        let mut app = state();
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        quick_open_with(&mut app, "app", &["README.md"], &[0]);
        app.dispatch(Action::QuickOpenSubmit);
        app.dispatch(Action::JumpToTab(1));
        focus(&mut app, NodeKind::Repo, Some("app"), None);
        quick_open_with(&mut app, "app", &["src/lib.rs"], &[0]);
        app.dispatch(Action::QuickOpenSubmit);
        app.open_file_in_explorer("app".into(), "new.txt".into(), Some(0), Some("x".into()));
        assert_eq!(app.tabs.labels(), ["Workspace", "Explorer · app"]);
        assert_eq!(
            inners(&app),
            [pinned("README.md"), pinned("src/lib.rs"), pinned("new.txt")]
        );
    }

    #[test]
    fn late_checkout_confirm_cannot_write_from_an_explorer_tab() {
        let mut app = state();
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        crate::tui::app::apply_checkout_compute(
            &mut app,
            "app".into(),
            crate::tui::app::CheckoutCompute::Confirm {
                local_branch: "main".into(),
                remote_ref: "origin/main".into(),
                ahead_behind: Some((0, 1)),
            },
        );
        assert!(app.confirm.is_some(), "the late result opens its confirm");
        for action in [Action::ConfirmYes, Action::ConfirmYesClean] {
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(&*app.status, EXPLORER_TAB_READ_ONLY, "{action:?}");
            assert!(app.confirm.is_some(), "{action:?} does not consume it");
        }
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none(), "n still closes it");
    }

    #[test]
    fn closing_the_explorer_tab_restores_the_workspace_session() {
        let mut app = state();
        app.focus = FocusPane::Right;
        app.search_active = true;
        app.search_query = "readme".into();
        app.search_target = SearchPane::Tree;
        app.diff_cursor = 7;
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        assert!(!app.search_active, "the Explorer tab has no search");
        app.dispatch(Action::CloseTab);
        app.dispatch(Action::CloseTab);
        assert!(app.tabs.is_workspace());
        assert_eq!(app.focus, FocusPane::Right);
        assert!(app.search_active);
        assert_eq!(app.search_query, "readme");
        assert_eq!(app.search_target, SearchPane::Tree);
        assert_eq!(app.diff_cursor, 7);
    }

    /// A state whose cwd is a fresh temp dir with `app/notes.txt`.
    fn disk_state(lines: &[&str]) -> (AppState, PathBuf) {
        let cwd = crate::testutil::unique_dir("ws-explorer-live");
        std::fs::create_dir_all(cwd.join("app")).expect("app dir");
        write_notes(&cwd, lines);
        let mut app = state();
        app.cwd = cwd.clone();
        (app, cwd)
    }

    /// Write `lines` to `app/notes.txt`. Callers change the line count (so
    /// the size) on every rewrite, so the disk token moves whatever the
    /// mtime granularity.
    fn write_notes(cwd: &std::path::Path, lines: &[&str]) {
        std::fs::write(cwd.join("app/notes.txt"), lines.join("\n")).expect("write file");
    }

    fn file_lines(app: &AppState) -> Vec<String> {
        app.tabs
            .active_file_view()
            .expect("a file view")
            .lines()
            .to_vec()
    }

    fn set_changes(app: &mut AppState, changes: Vec<FileChange>) {
        let snap = repo("app", CheckoutKind::Primary, None, changes);
        crate::tui::app::apply_one_repo_snapshot(app, "app", Some(snap));
    }

    #[test]
    fn explorer_live_reload_on_disk_and_status_change() {
        let (mut app, cwd) = disk_state(&["alpha", "beta"]);
        let (tab_id, gen) = app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta"]);
        assert!(
            tab(&app).active_inner().unwrap().stamp.is_some(),
            "open stamps"
        );
        for _ in 0..3 {
            assert_eq!(app.explorer_live_reload("app"), None, "unchanged");
        }
        assert_eq!(app.explorer_live_reload("lib"), None, "another checkout");
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::MoveToEnd);

        // A disk edit reloads quietly: the old body stays painted.
        write_notes(&cwd, &["beta", "alpha", "beta"]);
        let reload = app.explorer_live_reload("app").expect("a reload");
        let (_, disk_gen) = file_load(&reload);
        assert!(disk_gen > gen);
        assert_eq!(file_lines(&app), ["alpha", "beta"], "old body stays");
        assert!(app.status.is_empty(), "silent: {}", &*app.status);
        assert_eq!(app.explorer_live_reload("app"), None, "stamp recorded");
        assert!(app.apply_explorer_preview(tab_id, disk_gen, text(&["beta", "alpha", "beta"])));
        assert_eq!(file_lines(&app), ["beta", "alpha", "beta"]);
        assert_eq!(
            app.tabs.active_file_view().unwrap().cursor,
            1,
            "the cursor keeps its line"
        );

        // A failed silent read keeps the body.
        write_notes(&cwd, &["one"]);
        let (_, failed) = file_load(&app.explorer_live_reload("app").expect("a reload"));
        assert!(!app.apply_explorer_preview(
            tab_id,
            failed,
            ExplorerPreviewBody::File(FileRead::Failed("gone".into()))
        ));
        assert_eq!(file_lines(&app), ["beta", "alpha", "beta"]);

        // While an overlay holds input nothing is checked or recorded.
        write_notes(&cwd, &["one", "two"]);
        let stamp = tab(&app).active_inner().unwrap().stamp.clone();
        quick_open_with(&mut app, "app", &["notes.txt"], &[0]);
        assert_eq!(app.explorer_live_reload("app"), None);
        assert_eq!(tab(&app).active_inner().unwrap().stamp, stamp);
        app.dispatch(Action::QuickOpenCancel);

        // A status change that makes the file changed swaps in a diff.
        let mut changes = app.snapshot.repos[0].changes.clone();
        changes.push(change("notes.txt", Some("M"), false));
        set_changes(&mut app, changes);
        let reload = app.explorer_live_reload("app").expect("a reload");
        match &reload {
            Effect::LoadExplorerPreview { change, .. } => assert!(change.is_some()),
            other => panic!("{other:?}"),
        }
        assert!(app.explorer_diff_loading(), "a loading diff view");
        let (_, diff_gen) = file_load(&reload);
        let diff =
            super::super::super::diff::DiffContent::from_unified("@@ -1 +1,2 @@\n one\n+two\n");
        assert!(app.apply_explorer_preview(tab_id, diff_gen, ExplorerPreviewBody::Diff(diff)));
        assert!(app.explorer_diff().is_some());

        // Background tabs wait; activating the tab checks once.
        app.dispatch(Action::JumpToTab(1));
        write_notes(&cwd, &["one", "two", "three"]);
        assert_eq!(app.explorer_live_reload("app"), None, "not active");
        let back = app.dispatch(Action::JumpToTab(2));
        assert!(has_file_load(&back), "{back:?}");
        assert_eq!(app.dispatch(Action::JumpToTab(1)), Effect::None);
        assert_eq!(app.dispatch(Action::JumpToTab(2)), Effect::None, "once");
        let (_, gen) = file_load(&back);
        let diff = super::super::super::diff::DiffContent::from_unified(
            "@@ -1 +1,3 @@\n one\n+two\n+three\n",
        );
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));

        // A deleted file keeps the last content and says so.
        std::fs::remove_file(cwd.join("app/notes.txt")).expect("delete");
        assert_eq!(app.explorer_live_reload("app"), None, "nothing to read");
        assert_eq!(&*app.status, FILE_DELETED);
        assert_eq!(app.status.kind(), StatusKind::Warn);
        assert!(app.explorer_diff().is_some(), "content kept");
        assert_eq!(app.explorer_live_reload("app"), None, "gone recorded");
        let _ = std::fs::remove_dir_all(&cwd);
    }

    fn edit_of(effect: Effect) -> (String, Option<u32>) {
        match effect {
            Effect::EditFile { repo, path, line } => {
                assert_eq!(repo, "app");
                (path, line)
            }
            other => panic!("an editor launch, got {other:?}"),
        }
    }

    #[test]
    fn e_opens_the_active_inner_file_at_its_line() {
        // Focus right on a clean file: its cursor line.
        let mut app = state();
        app.open_loaded_inner_file("app", "README.md", &["a", "b", "c", "d"]);
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::Move(2));
        assert_eq!(
            edit_of(app.dispatch(Action::Edit)),
            ("README.md".into(), Some(3))
        );
        assert_eq!(&*app.status, "opening README.md…");

        // A binary body opens with no line.
        let effect = app.open_file_in_explorer("app".into(), "logo.png".into(), None, None);
        let (tab_id, gen) = file_load(&effect);
        assert!(app.apply_explorer_preview(
            tab_id,
            gen,
            ExplorerPreviewBody::File(FileRead::Binary)
        ));
        app.dispatch(Action::FocusRight);
        assert_eq!(
            edit_of(app.dispatch(Action::Edit)),
            ("logo.png".into(), None)
        );

        // A diff: the new-file line of the cursor row; a hunk header has none.
        let effect = app.open_file_in_explorer("app".into(), "src/tui/app.rs".into(), None, None);
        let (tab_id, gen) = file_load(&effect);
        let diff = super::super::super::diff::DiffContent::from_unified(
            "@@ -1,2 +1,2 @@\n one\n-old\n+new\n",
        );
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        app.dispatch(Action::FocusRight);
        // UNSTAGED 0, hunk 1, `one` 2, `-old` 3, `+new` 4.
        app.diff_cursor = 4;
        assert_eq!(
            edit_of(app.dispatch(Action::Edit)),
            ("src/tui/app.rs".into(), Some(2))
        );
        app.diff_cursor = 1;
        assert_eq!(
            edit_of(app.dispatch(Action::Edit)),
            ("src/tui/app.rs".into(), None)
        );
    }

    #[test]
    fn e_with_the_tree_focused_edits_the_tree_row() {
        let mut app = state();
        let open = app.open_file_in_explorer("app".into(), "README.md".into(), None, None);
        let (tab_id, gen) = file_load(&open);
        land(&mut app, open);
        assert!(app.apply_explorer_preview(tab_id, gen, text(&["a", "b", "c"])));
        app.dispatch(Action::FocusRight);
        app.dispatch(Action::Move(2));
        app.dispatch(Action::FocusLeft);
        assert_eq!(
            edit_of(app.dispatch(Action::Edit)),
            ("README.md".into(), Some(3)),
            "the tree row is the active inner file: its line"
        );
        // Another file row: that file, with no line.
        app.tabs
            .active_explorer_mut()
            .unwrap()
            .tree
            .set_cursor_rel("new.txt");
        assert_eq!(
            edit_of(app.dispatch(Action::Edit)),
            ("new.txt".into(), None)
        );
        // A folder row refuses.
        app.tabs
            .active_explorer_mut()
            .unwrap()
            .tree
            .set_cursor_rel("src");
        assert_eq!(app.dispatch(Action::Edit), Effect::None);
        assert_eq!(&*app.status, FOCUS_A_FILE_TO_EDIT);
    }

    #[test]
    fn editor_return_reloads_only_the_active_inner_file() {
        let (mut app, cwd) = disk_state(&["alpha", "beta"]);
        assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());
        let (tab_id, gen) = app.open_loaded_inner_file("app", "notes.txt", &["alpha", "beta"]);
        app.tabs.active_file_view_mut().expect("a file view").cursor = 1;

        // An exit without a save moves no stamp: nothing reloads.
        assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());

        write_notes(&cwd, &["alpha", "beta", "gamma"]);
        assert!(app.reload_explorer_after_edit("app", "other.txt").is_none());
        assert!(app.reload_explorer_after_edit("lib", "notes.txt").is_none());

        // A save reloads quietly: the old body stays painted and the line
        // cursor stays.
        let reload = app
            .reload_explorer_after_edit("app", "notes.txt")
            .expect("the active inner file reloads");
        let (_, saved) = file_load(&reload);
        assert!(saved > gen, "a new generation");
        assert_eq!(file_lines(&app), ["alpha", "beta"], "old body stays");
        assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());
        assert!(app.apply_explorer_preview(tab_id, saved, text(&["alpha", "beta", "gamma"])));
        assert_eq!(file_lines(&app), ["alpha", "beta", "gamma"]);
        assert_eq!(app.tabs.active_file_view().unwrap().cursor, 1);

        // An inner tab that is not active is left to the live-reload check
        // that runs when it shows again.
        std::fs::write(cwd.join("app/other.txt"), "o1").expect("write other");
        app.open_loaded_inner_file("app", "other.txt", &["o1"]);
        write_notes(&cwd, &["alpha"]);
        assert!(app.reload_explorer_after_edit("app", "notes.txt").is_none());
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn editor_return_keeps_the_inner_diff_cursor() {
        use super::super::super::diff::DiffContent;
        let (mut app, cwd) = disk_state(&["a", "c", "d"]);
        app.diff_mode = super::super::super::split::DiffMode::Inline;
        let mut changes = app.snapshot.repos[0].changes.clone();
        changes.push(change("notes.txt", Some("M"), false));
        set_changes(&mut app, changes);
        // Inline rows: 0 label, 1 hunk, 2 ` a`, 3 `-b`, 4 `+c`, 5 `+d`.
        let (tab_id, _) = app.open_loaded_inner_diff(
            "app",
            "notes.txt",
            DiffContent::from_unified("@@ -1,2 +1,3 @@\n a\n-b\n+c\n+d\n"),
        );
        app.diff_cursor = 4;

        // `e` on the diff row, a save, then the editor returns.
        write_notes(&cwd, &["a", "c", "d", "e"]);
        let reload = app
            .reload_explorer_after_edit("app", "notes.txt")
            .expect("the saved file reloads");
        match &reload {
            Effect::LoadExplorerPreview { change, .. } => assert!(change.is_some()),
            other => panic!("{other:?}"),
        }
        assert!(!app.explorer_diff_loading(), "the old diff stays painted");
        assert_eq!(app.diff_cursor, 4, "the cursor row stays while it loads");
        assert!(app.status.is_empty(), "silent: {}", &*app.status);

        let (_, gen) = file_load(&reload);
        let diff = DiffContent::from_unified("@@ -1,2 +1,4 @@\n a\n-b\n+c\n+d\n+e\n");
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        assert!(app.explorer_diff().is_some());
        assert_eq!(app.diff_cursor, 4, "the cursor row stays on the new diff");
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn showing_an_open_inner_tab_runs_the_live_reload_check() {
        let (mut app, cwd) = disk_state(&["n1"]);
        std::fs::write(cwd.join("app/other.txt"), "o1").expect("write other");
        app.open_loaded_inner_file("app", "notes.txt", &["n1"]);
        app.open_loaded_inner_file("app", "other.txt", &["o1"]);
        assert_eq!(inners(&app), [pinned("notes.txt"), pinned("other.txt")]);

        // `]` onto notes.txt after a disk edit: a silent reload.
        write_notes(&cwd, &["n1", "n2"]);
        assert_eq!(
            app.explorer_live_reload("app"),
            None,
            "other.txt is unchanged"
        );
        let shown = app.dispatch(Action::NextFileTab);
        assert_eq!(active_rel(&app).as_deref(), Some("notes.txt"));
        assert!(has_file_load(&shown), "{shown:?}");
        assert_eq!(file_lines(&app), ["n1"], "the old body stays painted");
        assert!(
            !has_file_load(&app.dispatch(Action::PreviousFileTab)),
            "other.txt did not move"
        );

        // Ctrl-w onto notes.txt after another edit.
        write_notes(&cwd, &["n1", "n2", "n3"]);
        let shown = app.dispatch(Action::CloseTab);
        assert_eq!(active_rel(&app).as_deref(), Some("notes.txt"));
        assert!(has_file_load(&shown), "{shown:?}");

        // A Quick Open of the open file checks it too.
        app.open_loaded_inner_file("app", "other.txt", &["o1"]);
        write_notes(&cwd, &["n1", "n2", "n3", "n4"]);
        let open = app.open_file_in_explorer("app".into(), "notes.txt".into(), None, None);
        assert!(has_file_load(&open), "{open:?}");
        assert_eq!(inners(&app).len(), 2, "no duplicate");
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn breadcrumb_and_copy_reference_name_the_active_inner_file() {
        use crate::tui::chrome::breadcrumb_segments;
        let mut app = state();
        open_root(&mut app);
        let (tab_id, gen, ..) = load_of(&app.dispatch(Action::MoveToEnd));
        assert!(app.apply_explorer_preview(tab_id, gen, text(&["# app"])));
        app.dispatch(Action::MoveToStart);
        assert_eq!(cursor_rel(&app), "src", "the tree cursor sits on a folder");
        app.tabs.active_explorer_mut().unwrap().painted_rel = Some("src".into());
        let copied = |app: &mut AppState| match app.dispatch(Action::CopyEntityReference) {
            Effect::CopyClipboard { text, .. } => text,
            other => panic!("a copy, got {other:?}"),
        };

        // Tree focused: the breadcrumb names the folder row; a copy names
        // the open file (a folder has no file reference).
        assert_eq!(breadcrumb_segments(&app)[1..], ["app", "src"]);
        let text = copied(&mut app);
        assert!(text.contains("README.md"), "{text}");

        // File pane focused: both name the active inner file.
        app.dispatch(Action::FocusRight);
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(breadcrumb_segments(&app)[1..], ["app", "README.md"]);
        let text = copied(&mut app);
        assert!(
            text.contains("README.md") && !text.contains("src"),
            "{text}"
        );

        // Tree focused on a file row: that row.
        app.dispatch(Action::FocusLeft);
        load_of(&app.dispatch(Action::Move(2)));
        assert_eq!(cursor_rel(&app), "new.txt");
        let text = copied(&mut app);
        assert!(text.contains("new.txt"), "{text}");
    }

    /// Go to line from the Explorer tree jumps in the preview: a changed
    /// file's diff by its new-side line, a clean file's body by line. A
    /// folder row or a body still loading has no lines.
    #[test]
    fn go_to_line_jumps_in_the_preview_from_the_tree() {
        use super::super::go_to_line::NO_LINES_TO_GO_TO;
        use crate::tui::diff::{DiffContent, DiffRow};
        let mut app = state();
        open_root(&mut app);
        assert_eq!(cursor_rel(&app), "src");
        assert_eq!(app.go_to_line_refusal().as_deref(), Some(NO_LINES_TO_GO_TO));
        assert_eq!(app.dispatch(Action::GoToLine(2)), Effect::None);
        assert_eq!(app.status, NO_LINES_TO_GO_TO);
        assert_eq!(app.focus, FocusPane::Left, "a refusal moves nothing");

        // new.txt is untracked: its preview is a diff.
        let Effect::LoadExplorerPreview {
            tab_id,
            gen,
            change: Some(_),
            ..
        } = app.dispatch(Action::Move(2))
        else {
            panic!("new.txt diff preview");
        };
        let diff = DiffContent::from_unified("@@ -0,0 +1,3 @@\n+one\n+two\n+three\n");
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        assert_eq!(app.dispatch(Action::GoToLine(2)), Effect::None);
        assert_eq!(app.focus, FocusPane::Right, "the jump focuses the preview");
        let row_text = |app: &AppState| match &app.current_diff_rows()[app.diff_cursor] {
            DiffRow::Line { left, .. } => left.text.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(row_text(&app), "two");
        app.dispatch(Action::GoToLine(40));
        assert_eq!(row_text(&app), "three", "clamped to the last line");

        // README.md is clean: its preview is the file body.
        app.dispatch(Action::FocusLeft);
        let Effect::LoadExplorerPreview {
            tab_id,
            gen,
            change: None,
            ..
        } = app.dispatch(Action::MoveToEnd)
        else {
            panic!("README.md body preview");
        };
        assert_eq!(
            app.go_to_line_refusal().as_deref(),
            Some(NO_LINES_TO_GO_TO),
            "still loading"
        );
        let lines: Vec<String> = (1..=30).map(|n| format!("line {n}")).collect();
        let body = ExplorerPreviewBody::File(FileRead::Text { lines, max_cols: 7 });
        assert!(app.apply_explorer_preview(tab_id, gen, body));
        assert_eq!(app.go_to_line_refusal(), None);
        app.dispatch(Action::GoToLine(12));
        assert_eq!(app.focus, FocusPane::Right);
        let cursor = |app: &AppState| app.tabs.active_file_view().expect("body").cursor;
        assert_eq!(cursor(&app), 11);
        app.dispatch(Action::GoToLine(0));
        assert_eq!(cursor(&app), 0, "below 1 is line 1");
        assert_eq!(cursor_rel(&app), "README.md", "the tree did not move");
    }

    /// A pinned inner file keeps showing while the tree sits on a folder
    /// row, so go to line still lands in it; closing the last inner file
    /// leaves no lines to go to.
    #[test]
    fn go_to_line_lands_in_the_active_inner_file_from_a_folder_row() {
        use super::super::go_to_line::NO_LINES_TO_GO_TO;
        let mut app = state();
        open_root(&mut app);
        let lines: Vec<String> = (1..=20).map(|n| format!("line {n}")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        app.open_loaded_inner_file("app", "README.md", &lines);
        app.dispatch(Action::FocusLeft);
        app.dispatch(Action::MoveToStart);
        assert_eq!(cursor_rel(&app), "src", "a folder row");
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
        assert_eq!(app.go_to_line_refusal(), None);
        assert_eq!(app.dispatch(Action::GoToLine(7)), Effect::None);
        assert_eq!(app.focus, FocusPane::Right, "the jump focuses the file");
        assert_eq!(app.tabs.active_file_view().map(|file| file.cursor), Some(6));

        app.dispatch(Action::CloseTab);
        assert!(tab(&app).files.is_empty(), "no inner file left");
        assert_eq!(app.go_to_line_refusal().as_deref(), Some(NO_LINES_TO_GO_TO));
    }

    /// A `Theme: …` row previews over an Explorer inner file, Esc puts the
    /// open-time theme back, and Enter applies it with the file still open.
    #[test]
    fn explorer_inner_file_previews_and_applies_a_palette_theme_row() {
        use crate::tui::theme::ThemeId;
        let mut app = state();
        open_root(&mut app);
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        app.focus = FocusPane::Right;
        let before = app.theme;
        assert_ne!(before, ThemeId::Nord);
        let type_nord = |app: &mut AppState| {
            app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
            for c in "nord".chars() {
                app.dispatch(Action::QuickOpenChar(c));
            }
        };
        type_nord(&mut app);
        assert_eq!(app.theme, ThemeId::Nord, "focus previews the row");
        app.dispatch(Action::QuickOpenCancel);
        assert_eq!(app.theme, before, "Esc restores the open-time theme");

        type_nord(&mut app);
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert_eq!(app.theme, ThemeId::Nord);
        assert_eq!(app.status, "theme: Nord");
        assert_eq!(active_rel(&app).as_deref(), Some("README.md"));
        assert_eq!(app.focus, FocusPane::Right);
    }

    /// The Search in files, Go to file, and `Theme: …` palette rows run on
    /// an Explorer tab (with or without its file pane focused); Open in
    /// Explorer is off.
    #[test]
    fn explorer_palette_rows_from_main_follow_the_explorer_gates() {
        let mut app = state();
        open_root(&mut app);
        app.open_loaded_inner_file("app", "README.md", &["# app"]);
        let reason = |app: &AppState, title: &str| {
            let command = PALETTE_COMMANDS
                .iter()
                .find(|command| command.title == title)
                .unwrap_or_else(|| panic!("palette row {title}"));
            app.palette_disabled_reason(command)
        };
        for focus in [FocusPane::Left, FocusPane::Right] {
            app.focus = focus;
            assert_eq!(reason(&app, "Search in files"), None, "{focus:?}");
            assert_eq!(reason(&app, "Go to file"), None, "{focus:?}");
            assert_eq!(reason(&app, "Theme: Nord"), None, "{focus:?}");
            assert_eq!(
                reason(&app, "Open in Explorer").as_deref(),
                Some(SWITCH_TO_WORKSPACE_TAB),
                "{focus:?}"
            );
            for title in ["Shorter commit message", "Taller commit message"] {
                assert_eq!(
                    reason(&app, title).as_deref(),
                    Some(SWITCH_TO_WORKSPACE_TAB),
                    "{title} {focus:?}"
                );
            }
        }
    }
}

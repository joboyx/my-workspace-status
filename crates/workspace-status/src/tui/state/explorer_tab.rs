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
//! explicit open: Quick Open, a search-in-files hit) pins it, an explicit
//! open also focuses the file pane, `[` / `]` switch tabs, Ctrl-w closes
//! one. Folder listings leave as [`Effect::LoadExplorerDir`] and land
//! through [`AppState::apply_explorer_dir`]; file loads leave as
//! [`Effect::LoadExplorerPreview`] and land through
//! [`AppState::apply_explorer_preview`]. Both drop a result for a closed tab
//! or an older request. Status letters come from the live snapshot at paint
//! time. Git writes refuse with [`EXPLORER_TAB_READ_ONLY`].

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use ratatui::layout::Rect;
use ratatui::style::Color;

use crate::file_index::FileRead;
use crate::helpers::visible_width;
use crate::snapshot::FileChange;

use super::super::action::{Action, Effect, ExternalDiffKind};
use super::super::command_palette::{CommandScope, PaletteCommand};
use super::super::comments::{
    checkout_file_comment_states, covering_line_comment, resolve_comment_target, CommentKey,
};
use super::super::diff::{row_for_new_line, row_line_ref};
use super::super::effect::ExplorerPreviewBody;
use super::super::event_pump::overlay_blocks_background_ticks;
use super::super::explorer::{ExplorerEntry, ExplorerRow, ExplorerStatus};
use super::super::gates::{ListFocusTarget, FOCUS_A_FILE_TO_MARK_REVIEWED};
use super::super::icons::IconKind;
use super::super::popover::IconTarget;
use super::super::search::{
    apply_pan, explorer_row_match_indices, list_row_pan_max, step_from_anchor, SearchPane,
};
use super::super::selection::TextSelection;
use super::super::split::{list_hbar_rows, SplitDrag};
use super::super::status::StatusMessage;
use super::super::tabs::{
    ExplorerPreview, ExplorerTab, OpenFile, ParkedDiff, EXPLORER_DIFFS_SHOW_WHOLE_FILE,
    EXPLORER_FILE_TABS_ONLY, EXPLORER_TAB_READ_ONLY, FILE_DELETED, FOCUS_A_TREE_ROW_TO_EXPLORE,
    NO_EXPLORER_FILE_TABS, ONLY_WORKSPACE_TAB_OPEN, SWITCH_TO_WORKSPACE_TAB, TAB_TO_THE_FILE_PANE,
};
use super::super::tree::{
    cursor_for_viewport_top, dir_name_segments, dir_path_from_id, file_change_segments,
    file_row_id, file_type_seg, list_viewport_start, segment_icons, with_comment_mark,
    with_viewed_mark, NodeKind, NodeSegments, SegRole, TextSeg, UNSTAGED_ROW_SUFFIX,
};
use super::super::viewed::normalize_viewed_path;
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
    /// listings may land after the file load. The file pane takes focus
    /// (as Tab would), so the move keys and `/` act on the file; the focus
    /// is parked with the tab like any pane focus. An inner tab of `rel`
    /// that is already open is activated and pinned (no reload unless its
    /// file moved). With `line`, the cursor lands on that 0-based line
    /// once the file shows ([`Self::explorer_line_target`]); `search` then
    /// arms the in-file search on it. Returns the folder listings still
    /// missing and the file load.
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
        self.explorer_focus_preview();
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
            | Action::ToggleShowHidden
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
    /// `V`, `'`, and `gw` need a target.
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
            Action::ViewRemotely => self.view_remotely_refusal(),
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
            Action::PanDiff(delta) => {
                self.pan_explorer_tree(delta);
                Effect::None
            }
            Action::ScrollWheel {
                col,
                row,
                delta,
                horizontal,
            } => self.explorer_wheel(col, row, delta, horizontal),
            // A click goes to an open popover first, and a click on an icon
            // pins its popover, as on the Workspace. No Explorer icon is a PR
            // badge, so Ctrl+click is a plain click.
            Action::Click { col, row } | Action::CtrlClick { col, row }
                if row != self.layout.tab_y =>
            {
                if self.mouse_enabled {
                    self.click_with(col, row, Self::explorer_click)
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
            Action::NavEnter
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
    /// tab, says [`FOCUS_A_FILE_TO_MARK_REVIEWED`]. The file's tree row
    /// paints the eye ([`Self::explorer_row_paint_segments`]); the inner
    /// file can be another row than the cursor's, so the status line also
    /// says which way it went.
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

    /// Scroll the Explorer tree so its rows start at `top`: the tree
    /// cursor moves with the viewport ([`cursor_for_viewport_top`] over the
    /// tree rows, the painted tree height tall), as [`Self::explorer_move`]
    /// moves it for the wheel, and a moved cursor previews its row.
    pub(super) fn scroll_explorer_tree_to(&mut self, top: usize) -> Effect {
        let Some((rows, cursor)) = self.explorer_rows() else {
            return Effect::None;
        };
        let n = rows.len();
        let cursor = cursor.unwrap_or(0);
        let height = usize::from(self.layout.explorer_tree.height.max(1));
        let target = cursor_for_viewport_top(n, cursor, top, |c| list_viewport_start(n, c, height));
        if n == 0 || target == cursor {
            return Effect::None;
        }
        self.explorer_move(target as isize - cursor as isize)
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
            self.z_pending_at = Some(self.input_clock());
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

    /// Wheel over the tree moves the tree cursor; horizontally it pans the
    /// shown file view when that view can pan, else the tree (the
    /// Workspace tree's rule). Over the preview it moves the preview
    /// cursor (or pans, horizontally). Never the hidden Workspace tree.
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
        if !self.layout.explorer_tree_rows.contains(at) {
            return Effect::None;
        }
        if horizontal {
            // As over the Workspace tree (`mouse_pan`): a shown file view
            // that can pan takes the pan, else the tree pans.
            if self.explorer_preview_pan_max() > 0 {
                self.explorer_preview_pan(delta);
            } else {
                self.pan_explorer_tree(delta);
            }
            return Effect::None;
        }
        self.explorer_move(delta as isize)
    }

    /// Pan max of the active inner file the Explorer shows: a changed
    /// file's diff ([`Self::diff_pan_max`]) or a clean file's body
    /// (`FileTab::pan_max`); 0 with wrap on, no inner file, or a body that
    /// fits.
    fn explorer_preview_pan_max(&self) -> usize {
        match self.explorer_view() {
            Some(ExplorerPreview::Diff { .. }) => self.diff_pan_max(),
            Some(ExplorerPreview::File(file)) if !self.diff_wrap => {
                file.pan_max(self.layout.file_view_width, self.ascii)
            }
            _ => 0,
        }
    }

    /// Plain left press on an Explorer tab, as [`Self::click_at`] on the
    /// Workspace: the drag handles first ([`Self::press_split_hit`]: the
    /// pane divider, the in-diff RULE of a side-by-side diff, and every
    /// painted scrollbar — the tree's, a changed file's diff bars, a clean
    /// file's bars), which pick no row and arm no text selection; then the
    /// row under the press ([`Self::explorer_click_row`]).
    fn explorer_click(&mut self, col: u16, row: u16) -> Effect {
        self.text_selection = None;
        self.drag = SplitDrag::None;
        self.pad_press = None;
        if let Some(effect) = self.press_split_hit(col, row) {
            return effect;
        }
        self.explorer_click_row(col, row)
    }

    /// The row under (`col`, `row`) after [`Self::explorer_click`] found no
    /// drag handle there (or a flat divider press released in place). A
    /// tree row takes the cursor (a folder also opens or closes, except on
    /// a press on its folder glyph, which only selects it for the icon's
    /// popover); a second
    /// press on the same cell within 400 ms ([`Self::note_row_press`]) is
    /// Enter on that row ([`Self::explorer_enter`]), except on a folder,
    /// whose first press already toggled it (as on a Workspace tree
    /// chevron). The preview takes focus and the clicked line: a clean
    /// file's line ([`Self::click_file_tab`]) or a changed file's diff row
    /// ([`Self::click_diff`], as on the Workspace diff). A flat pad cell
    /// counts as its row; a text selection arms only in the inner area,
    /// and a drag from the press selects text.
    pub(super) fn explorer_click_row(&mut self, col: u16, row: u16) -> Effect {
        let is_double = self.note_row_press(col, row);
        let at = ratatui::layout::Position::new(col, row);
        let tree = self.layout.explorer_tree;
        if self.layout.explorer_tree_rows.contains(at) {
            self.text_selection = TextSelection::arm(tree, col, row);
            self.clear_diff_visual();
            self.focus = FocusPane::Left;
            // Below the painted rows: the horizontal bar row of a panned
            // tree.
            if row >= tree.y.saturating_add(self.explorer_tree_list_rows()) {
                return Effect::None;
            }
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
                // A press on the folder glyph selects the row for its
                // popover pin (`click_with`) and leaves the fold as is, as
                // a Workspace tree row's glyph does.
                if self.icon_hit_at(col, row).is_some() {
                    let rel = hit.rel.clone();
                    if let Some(tab) = self.tabs.active_explorer_mut() {
                        tab.tree.set_cursor_rel(&rel);
                    }
                    return Effect::None;
                }
                if is_double {
                    return Effect::None;
                }
                return self.explorer_fold_dir(&hit.rel.clone(), FoldKind::Toggle);
            }
            let rel = hit.rel.clone();
            let Some(tab) = self.tabs.active_explorer_mut() else {
                return Effect::None;
            };
            tab.tree.set_cursor_rel(&rel);
            let tab_id = tab.id;
            let preview = self.explorer_sync_preview(tab_id);
            if is_double {
                return super::popover::then(preview, self.explorer_enter());
            }
            return preview;
        }
        if self.layout.explorer_preview_rows.contains(at) {
            self.explorer_focus_preview();
            match self.explorer_view() {
                Some(ExplorerPreview::File(_)) => self.click_file_tab(col, row),
                view => {
                    let diff = view.is_some();
                    let pane: Rect = self.layout.explorer_preview;
                    self.text_selection = TextSelection::arm(pane, col, row);
                    if diff {
                        self.click_diff(row);
                    }
                }
            }
        }
        Effect::None
    }

    /// Rows the Explorer tree paints: the tree rows less the horizontal
    /// scrollbar row it keeps while panned ([`list_hbar_rows`]), as
    /// [`Self::tree_list_rows`] counts the Workspace tree.
    fn explorer_tree_list_rows(&self) -> u16 {
        let height = self.layout.explorer_tree.height;
        height.saturating_sub(list_hbar_rows(height, self.explorer_tree_col_offset()))
    }

    /// Columns the active Explorer tab's tree is panned; 0 off an Explorer
    /// tab.
    pub(super) fn explorer_tree_col_offset(&self) -> u16 {
        self.tabs
            .active_explorer()
            .map_or(0, |tab| tab.tree_col_offset)
    }

    /// Pan the active Explorer tab's tree to `offset` (already clamped to
    /// its pan max).
    pub(super) fn set_explorer_tree_col_offset(&mut self, offset: u16) {
        if let Some(tab) = self.tabs.active_explorer_mut() {
            tab.tree_col_offset = offset;
        }
    }

    /// What every row of the Explorer tree of `checkout` reads for its marks
    /// and flash, built once per paint (or pan clamp, `gh`, or popover
    /// section) instead of once per row: one pass over the comment store
    /// and, while a flash decays, one walk of the Workspace tree.
    pub(crate) fn explorer_row_marks(&self, checkout: &str) -> ExplorerRowMarks {
        ExplorerRowMarks {
            checkout: checkout.to_string(),
            comments: checkout_file_comment_states(&self.comment_store, &self.snapshot, checkout),
            folder_flashes: self.explorer_folder_flashes(checkout),
        }
    }

    /// Every segment Explorer tree row `row` paints: its label and status
    /// mark ([`explorer_row_segments`]), then on a file row the comment
    /// mark and the reviewed eye, in the order and with the glyphs a
    /// Workspace tree file row adds them
    /// ([`super::super::tree::painted_row_segments`]). The comment mark
    /// uses the Workspace file row's match (`marks`, from
    /// [`checkout_file_comment_states`]); the eye is the Workspace tree's
    /// stored reviewed mark of the same file ([`file_row_id`] in
    /// [`AppState::reviewed`]), which Space toggles on either tab. Paint,
    /// the pan clamp, `gh`, and the popover sections read it.
    pub(crate) fn explorer_row_paint_segments(
        &self,
        row: &ExplorerRow,
        status: &ExplorerStatus,
        marks: &ExplorerRowMarks,
    ) -> NodeSegments {
        let segs = explorer_row_segments(row, status, self.ascii);
        if row.is_dir || row.placeholder {
            return segs;
        }
        let comment = marks
            .comments
            .get(normalize_viewed_path(&row.rel).as_str())
            .copied();
        let viewed = status.change(&row.rel).is_some()
            && self
                .reviewed
                .contains(&file_row_id(&marks.checkout, &row.rel));
        NodeSegments {
            segments: segs.segments,
            trailing: with_viewed_mark(
                with_comment_mark(
                    segs.trailing,
                    self.ascii,
                    comment.is_some(),
                    comment == Some(true),
                ),
                self.ascii,
                viewed,
            ),
        }
    }

    /// Fade colour of Explorer tree row `row` while a watch reload flash of
    /// its path decays: the flash of the Workspace tree row that shows the
    /// same path, which [`AppState::apply_watch_snapshot`] stamps when that
    /// status reload adds, changes, or removes it. A file reads its own row
    /// ([`file_row_id`], or the Changes-side twin); a folder reads `marks`
    /// ([`Self::explorer_folder_flashes`]). Map lookups: no git, no I/O.
    pub(crate) fn explorer_row_flash_color(
        &self,
        row: &ExplorerRow,
        marks: &ExplorerRowMarks,
    ) -> Option<Color> {
        if row.placeholder {
            return None;
        }
        if row.is_dir {
            return marks.folder_flashes.get(&row.rel).copied();
        }
        let id = file_row_id(&marks.checkout, &row.rel);
        self.flash_color(&id)
            .or_else(|| self.flash_color(&format!("{id}{UNSTAGED_ROW_SUFFIX}")))
    }

    /// Flash colour of each Explorer folder of `checkout` that a decaying
    /// Workspace tree flash covers, keyed by folder path. Empty with no
    /// flash.
    ///
    /// The Workspace tree collapses a chain of single-child folders into
    /// one row (`dir:<checkout>:a/b`), so a folder may have no row of its
    /// own there: a folder row's flash covers its own path and each parent
    /// folder up to the next folder row of the same section (Staged, or
    /// Changes with the `#unstaged` suffix). The folder rows known are the
    /// live tree's and the ones a reload just removed (each removal is
    /// still a flash stamp while it decays), so a removed chain covers the
    /// same folders it covered before the reload. A wholly untracked folder
    /// is a file row with a trailing `/` (`file:<checkout>:<rel>/`) and
    /// flashes its folder.
    fn explorer_folder_flashes(&self, checkout: &str) -> HashMap<String, Color> {
        let mut out = HashMap::new();
        if self.flashes.is_empty() {
            return out;
        }
        let dir_prefix = format!("dir:{checkout}:");
        let file_prefix = file_row_id(checkout, "");
        let mut known: HashSet<String> = HashSet::new();
        let mut stack = vec![&self.tree];
        while let Some(node) = stack.pop() {
            if node.kind == NodeKind::Dir {
                if let Some(rest) = node.id.strip_prefix(&dir_prefix) {
                    known.insert(rest.to_string());
                }
            }
            stack.extend(&node.children);
        }
        known.extend(
            self.flashes
                .keys()
                .filter_map(|id| id.strip_prefix(&dir_prefix))
                .map(str::to_string),
        );
        for id in self.flashes.keys() {
            let Some(color) = self.flash_color(id) else {
                continue;
            };
            if let Some(rest) = id.strip_prefix(&dir_prefix) {
                let (path, suffix) = match rest.strip_suffix(UNSTAGED_ROW_SUFFIX) {
                    Some(path) => (path, UNSTAGED_ROW_SUFFIX),
                    None => (rest, ""),
                };
                let mut rel = path;
                loop {
                    out.entry(rel.to_string()).or_insert(color);
                    let Some((parent, _)) = rel.rsplit_once('/') else {
                        break;
                    };
                    if known.contains(&format!("{parent}{suffix}")) {
                        break;
                    }
                    rel = parent;
                }
            } else if let Some(folder) = id
                .strip_prefix(&file_prefix)
                .and_then(|rest| rest.strip_suffix('/'))
            {
                out.entry(folder.to_string()).or_insert(color);
            }
        }
        out
    }

    /// Checkout, row, and status of the active Explorer tab's listed tree
    /// row at `rel` (a popover section reads its icon from it). `None`
    /// off an Explorer tab or once the row is gone.
    pub(crate) fn explorer_row_at(
        &self,
        rel: &str,
    ) -> Option<(String, ExplorerRow, Rc<ExplorerStatus>)> {
        let tab = self.tabs.active_explorer()?;
        let status = self.explorer_status(&tab.checkout);
        let row = tab
            .tree
            .rows(&status)
            .into_iter()
            .find(|row| row.rel == rel && !row.placeholder)?;
        Some((tab.checkout.clone(), row, status))
    }

    /// Icons of the Explorer tree's cursor row in paint order, clipped
    /// ones included: what `gh` pins with the tree focused.
    pub(crate) fn explorer_cursor_row_icons(&self) -> Vec<(IconKind, IconTarget)> {
        let (Some(tab), Some(row)) = (self.tabs.active_explorer(), self.explorer_cursor_row())
        else {
            return Vec::new();
        };
        let status = self.explorer_status(&tab.checkout);
        let marks = self.explorer_row_marks(&tab.checkout);
        segment_icons(&self.explorer_row_paint_segments(&row, &status, &marks))
            .into_iter()
            .map(|kind| (kind, IconTarget::ExplorerRow(row.rel.clone())))
            .collect()
    }

    /// Rel of the Explorer tree's cursor row; `None` off an Explorer tab
    /// or with no row to focus. A popover owner and an Explorer row
    /// target compare against it.
    pub(crate) fn explorer_cursor_rel(&self) -> Option<String> {
        self.explorer_cursor_row().map(|row| row.rel)
    }

    /// Pan max of the active Explorer tree in a pane `pane_width` columns
    /// wide whose vertical scrollbar shows (`vbar`) or not: its longest
    /// painted row ([`Self::explorer_row_paint_segments`]) against the width left of
    /// the bar, as the Workspace tree clamps ([`Self::tree_max_col_in`]).
    /// A loading placeholder row is left out: it paints a short spinner
    /// copy only until its folder lists. 0 off an Explorer tab.
    pub(crate) fn explorer_tree_max_col_in(&self, pane_width: u16, vbar: bool) -> usize {
        let Some(tab) = self.tabs.active_explorer() else {
            return 0;
        };
        let width = pane_width.saturating_sub(u16::from(vbar)).max(1) as usize;
        let status = self.explorer_status(&tab.checkout);
        let marks = self.explorer_row_marks(&tab.checkout);
        tab.tree
            .rows(&status)
            .iter()
            .filter(|row| !row.placeholder)
            .map(|row| {
                let segs = self.explorer_row_paint_segments(row, &status, &marks);
                let label: usize = segs.segments.iter().map(|s| visible_width(&s.text)).sum();
                let trailing: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
                list_row_pan_max(label, row.depth, trailing, width)
            })
            .max()
            .unwrap_or(0)
    }

    /// Shift-←/→ with the tree focused, or the horizontal wheel over it:
    /// pan the Explorer tree `delta` columns, clamped to its longest row
    /// beside its vertical bar when the rows overflow, as
    /// [`Self::pan_left_pane`] pans the Workspace tree.
    fn pan_explorer_tree(&mut self, delta: i32) {
        let Some((rows, _)) = self.explorer_rows() else {
            return;
        };
        let vbar = rows.len() > usize::from(self.explorer_tree_list_rows());
        let max = self.explorer_tree_max_col_in(self.layout.explorer_tree.width, vbar);
        let next = apply_pan(self.explorer_tree_col_offset(), delta, max);
        self.set_explorer_tree_col_offset(next);
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
        inner.diff = ParkedDiff::default();
        if active {
            self.set_live_diff(ParkedDiff::default());
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
    pub(super) fn explorer_resolved_cursor_row(&self, tab_id: u64) -> Option<ExplorerRow> {
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
        let live = self.live_diff();
        let active = self.active_explorer_id() == Some(tab_id);
        let inner = self.tabs.get_explorer_id_mut(tab_id)?.active_inner_mut()?;
        if active {
            inner.diff = live;
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
        let diff = inner.map_or_else(ParkedDiff::default, |inner| inner.diff);
        self.set_live_diff(diff);
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
                    inner.diff.cursor = row.unwrap_or(0);
                    inner.diff.scroll = u16::try_from(inner.diff.cursor).unwrap_or(u16::MAX);
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
                inner.diff = ParkedDiff::default();
                self.set_live_diff(ParkedDiff::default());
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
}

#[cfg(test)]
impl AppState {
    /// Open `rel` of `checkout` the way Quick Open does (a pinned inner file
    /// tab of the checkout's Explorer tab, the file pane focused) and land
    /// `lines` as its clean body. The folder listings are left in flight.
    /// Returns the Explorer tab id and the load generation.
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

/// Per-paint lookups of the Explorer tree of one checkout
/// ([`AppState::explorer_row_marks`]): every row probes them instead of
/// scanning the comment store or the Workspace tree itself.
pub(crate) struct ExplorerRowMarks {
    /// The tab's checkout (snapshot `repo`).
    checkout: String,
    /// Commented file path → every comment on it resolved
    /// ([`checkout_file_comment_states`]).
    comments: HashMap<String, bool>,
    /// Folder path → its flash colour
    /// ([`AppState::explorer_folder_flashes`]).
    folder_flashes: HashMap<String, Color>,
}

/// What `l` / `z` / Enter does to a folder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FoldKind {
    /// Expand (`l`).
    Open,
    /// Expand or collapse (`z`, Enter, click).
    Toggle,
}

/// Explorer badge of an untracked file (the Workspace tree says `A`).
const UNTRACKED_BADGE: &str = "??";

/// Label and status mark of one listed Explorer row, built from the
/// Workspace tree's row pieces (the paint draws a loading placeholder row
/// itself).
///
/// A folder is the Workspace folder row ([`dir_name_segments`]: folder
/// glyph and name), with a `●` in the modified colour when it has changes
/// under it. A changed file is the Workspace file row
/// ([`file_change_segments`]: devicon, name in its status colour, and the
/// bold status letter), except that an untracked file's letter is `??`. A
/// clean file is the devicon and its name. Ignored entries keep their
/// glyph and name, muted and dim, with no mark. The comment mark and the
/// reviewed eye come on top ([`AppState::explorer_row_paint_segments`]).
fn explorer_row_segments(row: &ExplorerRow, status: &ExplorerStatus, ascii: bool) -> NodeSegments {
    let plain = |text: &str, role: SegRole| TextSeg {
        text: text.to_string(),
        role,
        hex: None,
        bold: false,
        dim: false,
        icon: None,
    };
    let segs = if row.is_dir {
        let mut segs = dir_name_segments(&row.name, ascii);
        if status.dir_dirty(&row.rel) {
            let dot = if ascii { "*" } else { "●" };
            segs.trailing = vec![plain(dot, SegRole::Modified)];
        }
        segs
    } else if let Some(change) = status.change(&row.rel) {
        let mut segs = file_change_segments(change, true, ascii);
        if change.untracked {
            if let Some(badge) = segs.trailing.first_mut() {
                badge.text = UNTRACKED_BADGE.to_string();
            }
        }
        segs
    } else {
        NodeSegments {
            segments: vec![
                file_type_seg(&row.rel, ascii),
                plain(&row.name, SegRole::File),
            ],
            trailing: Vec::new(),
        }
    };
    if !row.ignored {
        return segs;
    }
    let muted = |seg: TextSeg| TextSeg {
        role: SegRole::Muted,
        hex: None,
        bold: false,
        dim: true,
        ..seg
    };
    NodeSegments {
        segments: segs.segments.into_iter().map(muted).collect(),
        trailing: Vec::new(),
    }
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
        // The paint reads the rename source (`old → new`).
        change.old_path.hash(&mut hasher);
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
mod tests;

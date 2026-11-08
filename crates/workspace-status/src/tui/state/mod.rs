//! TUI state and Action dispatch.

mod dispatch;
mod dispatch_drill;
mod dispatch_keymap;
mod dispatch_quick_open;
mod dispatch_search_files;
mod dispatch_write;
mod doctor;
mod explorer_tab;
mod file_view;
mod go_to_line;
mod line_blame;
mod outage;
mod pan;
mod popover;
mod prs_tab;
mod pull_request;
mod remote_view;
mod scroll_glide;
mod summary;
mod tab_park;

#[cfg(test)]
pub(crate) use prs_tab::fixtures as prs_fixtures;
pub(crate) use prs_tab::{prs_row_flash_id, PrsLine, PrsView, PRS_DETAIL_MIN_COLS};
pub(crate) use pull_request::PrDetailState;
use tab_park::TabView;

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use workspace_status_graph::{
    graph_chrome_budget_for_meta, graph_footer_request, graph_vscroll_visible, paint_model,
    painted_line_count, painted_line_of_row, selection_footer, GraphChromeBudget,
    GraphFooterSelection, GraphModel, GraphRow, LabelKind, LabelPart, PaintedLine, SelectionFooter,
    ASCII, COMMIT_MSG_LINES_DEFAULT, COMMIT_MSG_LINES_MAX, COMMIT_MSG_LINES_MIN, FOOTER_RULE_ROWS,
    UNICODE,
};

use crate::config::ViewDefaults;
use crate::settings::Settings;
use crate::snapshot::{
    carry_status_failed_local_branches, CheckoutKind, FileChange, SyncStatus,
    WorkspaceRepoSnapshot, WorkspaceSnapshot,
};

#[cfg(test)]
use super::action::Action;
use super::action::{Effect, ExternalDiffKind, SearchFilesOption};
use super::activity::{activity_targets, ACTIVITY_DAYS};
use super::anim::{next_tick_ms, AnimNeeds, RemoteActivity, ScrollGlide};
use super::branches::{
    branch_name_error, can_open_branch_picker, checkoutable_branch_names, merge_rev_for_commit,
    BranchPickerState, CreateBranchState, DIRTY_WORKTREE_STATUS,
};
use super::chrome::{
    diverged_pull_status, rebase_pull_status, rewritten_pull_status, status_uses_status_text,
    STATUS_NO_COMMENTS,
};
use super::comments::{
    collect_live_set, comment_key_label, comments_in_focus_scope, covering_line_comment,
    diff_focus_side, diff_line_comment_state, export_markdown, format_entity_reference,
    gc_comments, load_comment_store, put_comment_entry, resolve_comment_target,
    resolve_entity_reference, viewport_line_number, viewport_line_range, CommentEntry,
    CommentExport, CommentExportList, CommentKey, CommentPrompt, CommentStore, DiffSide, EntityRef,
};
use super::commit_card::CommitCardCache;
use super::commit_files::{
    ancestor_dir_ids, collect_foldable_subtree_ids as collect_commit_subtree_ids,
    commit_file_cursor_index, flatten_commit_files, folder_summary, CommitFileRow, FolderSummary,
};
use super::ctrl_c_exit::{
    handle_ctrl_c, is_ctrl_c_exit_prompt, CTRL_C_EXIT_PROMPT, QUIT_WHILE_BUSY_PROMPT,
};
use super::diff::{
    anchor_row_text, build_diff_rows, build_partial_patch, diff_pane_header_rows,
    diff_row_content_width, diff_wrap_row_heights, find_anchor_row, gutter_width, row_search_text,
    wrap_viewport_start, DiffContent, DiffRow, PartialPatchKind,
};
use super::drill::{
    source_from_graph_row, stash_ref_from_graph_row, CommitFile, CommitFileSource, DrillView,
};
use super::explorer::ExplorerStatus;
use super::fetch::{background_fetch_targets, OTHER_INSTANCE_FETCH_FAILED};
use super::fetch_claim::load_background_fetch_record;
use super::file_facts::{FileFactsCache, RepoFileFacts};
use super::gates::{
    ListFocusTarget, FOCUS_A_FILE_DIFF, FOCUS_A_FILE_TO_MARK_REVIEWED,
    REVIEWED_MARKS_ARE_FOR_TREE_FILES,
};
use super::graph_focus::GraphFocusPickerState;
use super::hyperlink::LinkHit;
use super::icons::{comment_mark_cols, IconKind};
use super::keys::{expire_stale_g_chord_echo, GChordEchoState, InputMode, DOUBLE_TAP_MS};
use super::line_blame::{GraphReveal, LineBlameState};
use super::ops::{
    collect_write_files, format_running_op, op_is_kind_noop, op_kind_noop_reason, op_targets,
    push_targets, refresh_target, snapshot_pushable, Op, RevertScope, RunningOp, ScopedFile,
};
use super::outage::{OutageSource, Outages};
use super::popover::{IconTarget, PopoverState};
use super::quick_open::QuickOpenState;
use super::search::{
    collect_graph_match_indices, collect_match_ids, commit_file_row_match_indices,
    explorer_row_match_indices, focus_commit_file_search, focus_diff_search, focus_graph_search,
    focus_tree_search, match_diff_line_indices, SearchPane,
};
use super::search_files::{SearchFilesState, SearchZone};
use super::selection::TextSelection;
use super::split::{
    clamp_tree_fraction, diff_paint_width, diff_pane_mode, diff_split_fraction_from_col,
    graph_scroll_max, hcol_from_col, hcol_from_delta, hit_split, list_hbar_rows, pane_widths,
    step_tree_fraction, tree_fraction_from_col, vscroll_from_delta, vscroll_from_row, DiffMode,
    HScrollbar, HScrollbarLayout, ResizeEdge, SplitDrag, SplitHit, SplitLayout, VScrollbar,
    VScrollbarLayout, DIFF_SPLIT_FRACTION, NARROW_SXS, TREE_WIDTH_FRACTION,
};
use super::stash::{
    checkout_path, resolve_stash_menu_key, row_is_hidden, stash_dirty_for_row,
    stash_ops_for_context, StashMenuKeyResult, StashOp, StashOpId, StashOpsContext,
};
use super::status::{StatusKind, StatusMessage};
use super::store_save::{
    StoreKind, StoreSaveJob, StoreSaved, StoreSync, UnsavedReason, UnsavedStore,
};
use super::tabs::{
    compare_file_dirty, CommitPickerState, ComparePickerKind, ComparePickerState, OpenCompare,
    TabStrip, WorktreeFile, CANNOT_REVERT_COMMITTED_DIFF, CANNOT_REVERT_WORKTREE_COMPARE,
    COMPARE_FOCUS_A_FILE, COMPARE_HEAD_MOVED, COMPARE_HEAD_REF, COMPARE_STILL_LOADING,
    FOCUS_A_COMMIT_TO_DIFF, ROOT_COMMIT_HAS_NO_PARENT, SWITCH_TO_WORKSPACE_TAB,
    WORKSPACE_TAB_CANNOT_CLOSE,
};
use super::theme::{BackgroundMode, ThemeId};
use super::toast::ToastStack;
use super::tree::{
    build_tree, collect_foldable_subtree_ids, cursor_for_viewport_top, default_folds, flatten_with,
    list_viewport_start, visible_for_tree, visible_window, workspace_label_from_cwd, NodeKind,
    TreeNode, VisibleRow,
};
use super::ui_state_store::{save_activity_sparkline, save_commit_msg_lines, save_theme};
use super::undercurl::UndercurlHit;
use super::viewed::{
    load_viewed_store, reconcile_viewed, toggle_viewed, viewed_identity, viewed_row_ids,
    workspace_store_id, ViewedStore,
};
use super::watch::{
    capture_removal_ghosts, checkout_flash_ids, commit_file_identity, commit_file_signatures,
    flash_strength, flashable_row_kinds, graph_flash_decision, graph_flash_meta, graph_row_id,
    graph_row_identity, graph_row_signatures, is_new_row_set, merge_ghost_rows, prune_flashes,
    prune_ghosts, tree_signatures, FlashKind, FlashStamp, GhostRow, GraphFlashDecision,
    GraphFlashMeta,
};
use crate::git::{PushPlan, FULL_DIFF_CONTEXT_LINES};

/// Status for a key drained while a git write runs and the job has no name.
pub(crate) const BUSY_WRITE_RUNNING: &str = "busy: a git write is running";

/// The diff of a right pane that shows none (an Explorer folder row).
static EMPTY_DIFF: DiffContent = DiffContent {
    staged: String::new(),
    unstaged: String::new(),
    is_new: false,
    is_committed: false,
    vs_worktree: false,
    committed_label: false,
    error: None,
};

/// `n` / `N` with no armed search.
pub(crate) const NO_SEARCH_ARMED: &str = "no search — press / first";

/// A forward search step (`/` typing, Enter, `n`) ran past the last match
/// and started over at the top.
pub(crate) const SEARCH_WRAPPED_TO_TOP: &str = "search wrapped to top";

/// A backward search step (`N`) ran past the first match and started over
/// at the bottom.
pub(crate) const SEARCH_WRAPPED_TO_BOTTOM: &str = "search wrapped to bottom";

/// The `/` query matches nothing in the bound pane.
pub(crate) const SEARCH_NO_MATCH: &str = "no match";

/// Message line of the depth-2 commit-files footer when the drilled commit,
/// stash, or working tree has no row in the loaded graph (a watch reload
/// after an amend, say); the meta row under it names the source.
pub(crate) const COMMIT_FOOTER_NOT_IN_GRAPH: &str = "not in the loaded graph";

/// `z` where nothing folds (graph list, file diff).
pub(crate) const Z_FOLDS_TREE_ROWS: &str = "z folds tree rows";

/// Unmapped key during `V` highlight.
pub(crate) const ESC_EXITS_HIGHLIGHT: &str = "Esc exits highlight";

/// `V` on a pane with no line to highlight.
pub(crate) const NO_HIGHLIGHT_TARGET: &str = "no highlight target";

/// Which pane has keyboard focus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusPane {
    Left,
    Right,
}

/// One painted icon, for hover peek, click pin, and Ctrl+click.
///
/// Paint records one span per icon it paints (a tagged tree segment, a PR
/// badge on a tree or graph row), rebuilt every frame. The span is the
/// glyph columns only, clipped to what is on screen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconHit {
    /// 0-based screen row of the icon.
    pub y: u16,
    /// 0-based first screen column of the icon.
    pub x: u16,
    /// Painted width in display columns.
    pub width: u16,
    /// Catalog kind of the icon.
    pub kind: IconKind,
    /// What the icon describes.
    pub target: IconTarget,
}

impl IconHit {
    /// True when screen cell (`col`, `row`) is inside this icon.
    pub fn contains(&self, col: u16, row: u16) -> bool {
        row == self.y && col >= self.x && col < self.x.saturating_add(self.width)
    }

    /// Screen cells of the icon.
    pub fn rect(&self) -> Rect {
        Rect::new(self.x, self.y, self.width, 1)
    }
}

/// The plain left press of a tab kind (no popover, no icon): what a click
/// on an icon runs to select its row before it pins.
pub(super) type PlainClick = fn(&mut AppState, u16, u16) -> Effect;

/// A painted PR badge: the PR view of an [`IconHit`].
///
/// A tree repo / checkout row and a graph worktree row each record one
/// span. Ctrl+click inside it opens the PR of [`Self::repo`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrBadgeHit {
    /// 0-based screen row of the badge.
    pub y: u16,
    /// 0-based first screen column of the badge.
    pub x: u16,
    /// Painted width in display columns.
    pub width: u16,
    /// Checkout path (snapshot `repo`) of the row the badge is on.
    pub repo: PathBuf,
}

impl PrBadgeHit {
    /// True when screen cell (`col`, `row`) is inside this badge.
    pub fn contains(&self, col: u16, row: u16) -> bool {
        row == self.y && col >= self.x && col < self.x.saturating_add(self.width)
    }
}

/// Last painted layout, used for mouse hit testing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutHit {
    pub tree_x: u16,
    pub tree_y: u16,
    pub tree_width: u16,
    pub tree_height: u16,
    pub right_x: u16,
    pub list_offset: usize,
    pub term_cols: u16,
    /// Terminal height in rows at the last paint (the whole frame).
    pub term_rows: u16,
    pub pane_height: u16,
    pub outer_tree_width: u16,
    pub diff_pane_width: u16,
    /// Right-pane content height: the rows under the pane's title (the top
    /// border or the flat title row), less the bottom border when boxed.
    pub diff_pane_height: u16,
    /// 0-based first right-pane content column, as painted (after the left
    /// border when boxed, after the one-cell left pad when flat). Row
    /// clicks pick the pane by [`Self::right_x`], so a pad cell selects its
    /// row too.
    pub diff_content_x: u16,
    pub diff_split_rule_x: Option<u16>,
    pub right_y: u16,
    pub files_list_y: u16,
    /// Painted commit-file list height in rows. Screen rows from
    /// `files_list_y + files_list_height` down (a panned list's horizontal
    /// scrollbar row, the commit-message footer) are not file rows.
    pub files_list_height: u16,
    pub files_list_offset: usize,
    /// 0-based graph scrollbar column when a graph list is painted.
    pub graph_scrollbar_x: Option<u16>,
    /// 0-based first row of the graph scrollbar track.
    pub graph_scrollbar_y: u16,
    /// Graph list height (scrollbar track).
    pub graph_scrollbar_height: u16,
    /// Painted graph line count.
    pub graph_content_len: usize,
    /// Graph list top the last frame painted: `graph_scroll`, or the step
    /// of a live Ctrl-d / Ctrl-u glide. `None` when no graph list painted.
    /// Graph row clicks and the scrollbar thumb read it, as the tree reads
    /// [`Self::list_offset`].
    pub graph_painted_top: Option<u16>,
    /// 0-based graph horizontal scrollbar row when the bar is painted.
    pub graph_hscrollbar_y: Option<u16>,
    /// 0-based first column of the graph horizontal scrollbar track.
    pub graph_hscrollbar_x: u16,
    /// Graph horizontal scrollbar track width.
    pub graph_hscrollbar_width: u16,
    /// Max horizontal pan for the painted graph.
    pub graph_col_max: u16,
    /// 0-based first row of the graph selection footer (its rule row) when
    /// its expanded message is taller than the footer (wheel there scrolls
    /// the message).
    pub graph_footer_y: Option<u16>,
    /// 0-based first column of that footer.
    pub graph_footer_x: u16,
    /// Footer width.
    pub graph_footer_width: u16,
    /// Footer height, the rule row on top and the meta rows included.
    pub graph_footer_height: u16,
    /// Message rows of that footer: its body less the wrapped meta rows
    /// pinned at the bottom.
    pub graph_footer_message_rows: u16,
    /// Max message scroll for the painted footer.
    pub graph_footer_scroll_max: usize,
    /// Graph pane width at the last graph paint.
    ///
    /// `0` before the first paint. [`AppState::graph_chrome`] wraps the
    /// selected row's meta line at this width, so the list height between
    /// frames matches the painted footer.
    pub graph_width: u16,
    /// Graph list height at the last graph paint.
    ///
    /// `0` before the first paint. Not cleared between frames: the next
    /// graph paint compares its list height with it
    /// ([`AppState::keep_graph_cursor_in_view`]).
    pub graph_list_height: u16,
    /// 0-based first row of the depth-2 commit-files footer (its rule row)
    /// when its expanded message is taller than its message rows (wheel
    /// there scrolls the message, not the file list).
    pub commit_detail_footer_y: Option<u16>,
    /// 0-based first column of that footer.
    pub commit_detail_footer_x: u16,
    /// Footer width.
    pub commit_detail_footer_width: u16,
    /// Footer height, the rule row on top included.
    pub commit_detail_footer_height: u16,
    /// Message rows of that footer: its body less the wrapped meta rows
    /// pinned at the bottom.
    pub commit_detail_footer_message_rows: u16,
    /// Max message scroll for the painted commit-files footer.
    pub commit_detail_footer_scroll_max: usize,
    /// 0-based row of the commit-message footer's top rule (graph selection
    /// footer or depth-2 commit-files footer) when the footer is expanded
    /// and painted: the [`ResizeEdge::CommitMsg`] edge.
    pub commit_msg_rule_y: Option<u16>,
    /// 0-based first column of that rule (the pane's first column).
    pub commit_msg_rule_x: u16,
    /// Rule width (the pane width).
    pub commit_msg_rule_width: u16,
    /// Most message rows the painted pane shows in its expanded footer
    /// ([`workspace_status_graph::commit_msg_lines_fit`], the same for a
    /// graph pane and a commit-files pane). A drag of the rule clamps here.
    pub commit_msg_lines_fit: usize,
    /// File-diff vertical scrollbar from the last paint: its column (`None`
    /// when no file diff paints one), the body rows it spans (not the path
    /// header), the diff row count, the painted top, and the largest top.
    /// `AppState::split_layout` hit-tests with the live `diff_scroll` as
    /// `scroll` instead, as a bar press or drag moves it before the next
    /// paint.
    pub diff_vbar: VScrollbarLayout,
    /// 0-based diff horizontal scrollbar row when the bar is painted.
    pub diff_hscrollbar_y: Option<u16>,
    /// 0-based first column of the diff horizontal scrollbar track.
    pub diff_hscrollbar_x: u16,
    /// Diff horizontal scrollbar track width.
    pub diff_hscrollbar_width: u16,
    /// Max horizontal pan for the painted file diff.
    pub diff_col_max: u16,
    /// Workspace tree vertical scrollbar from the last paint: its column
    /// (`None` while the rows fit), the pane rows it spans, the painted
    /// row count, the painted top, and the largest top.
    pub tree_vbar: VScrollbarLayout,
    /// Commit-file or compare list vertical scrollbar from the last paint
    /// (the list rows, not the commit footer under them).
    pub files_vbar: VScrollbarLayout,
    /// Explorer tree vertical scrollbar from the last paint.
    pub explorer_tree_vbar: VScrollbarLayout,
    /// Workspace tree horizontal scrollbar from the last paint: its row
    /// (`None` until the tree pans off the left edge), track, pan clamp
    /// (`AppState::tree_max_col`), and the painted pan (`split_layout`
    /// hit-tests with the live pan, as a press or drag moves it first).
    pub tree_hbar: HScrollbarLayout,
    /// Commit-file or compare list horizontal scrollbar from the last
    /// paint, with the list's pan clamp (`AppState::commit_file_max_col`);
    /// `split_layout` hit-tests with the live pan.
    pub files_hbar: HScrollbarLayout,
    /// Explorer tree horizontal scrollbar from the last paint, with the
    /// tree's pan clamp (`AppState::explorer_tree_max_col_in`);
    /// `split_layout` hit-tests with the live pan.
    pub explorer_tree_hbar: HScrollbarLayout,
    /// Explorer clean-file view vertical scrollbar from the last paint:
    /// its column (`None` while the lines fit), the body rows it spans,
    /// the line count, the painted top line, and the top that shows the
    /// last line on the bottom row.
    pub file_view_vbar: VScrollbarLayout,
    /// Explorer clean-file view horizontal scrollbar from the last paint,
    /// with the file's pan clamp ([`super::tabs::FileTab::pan_max`]);
    /// `split_layout` hit-tests with the live pan.
    pub file_view_hbar: HScrollbarLayout,
    /// Folder summary vertical scrollbar from the last paint: the rows
    /// under its header, the file row count, and the painted top.
    pub summary_vbar: VScrollbarLayout,
    /// Folder summary horizontal scrollbar from the last paint, with the
    /// summary's pan clamp (`AppState::summary_max_col_in`);
    /// `split_layout` hit-tests with the live pan.
    pub summary_hbar: HScrollbarLayout,
    /// PRs tab list vertical scrollbar from the last paint: its column
    /// (`None` while the lines fit), the scrolled lines it spans (not the
    /// pinned error and loading lines), the line count, and the painted
    /// top.
    pub prs_list_vbar: VScrollbarLayout,
    /// PRs tab detail vertical scrollbar from the last paint: its column
    /// (`None` while the lines fit), the detail rows, the line count, the
    /// painted top, and the largest top (0 while the lines fit), where the
    /// detail's wheel and keys clamp.
    pub prs_detail_vbar: VScrollbarLayout,
    /// File rows the folder summary painted last (the rows under its
    /// header, less a horizontal bar row): its page and scroll limit.
    pub summary_list_height: u16,
    /// Tab-strip row (always painted).
    pub tab_y: u16,
    /// Hit boxes `(x, width, tab_index)` for the painted strip.
    ///
    /// An overflow marker (`‹N` / `N›`) maps to its nearest hidden tab.
    pub tab_hits: Vec<(u16, u16, usize)>,
    /// Close `[✗]` hit boxes for compare and Explorer tabs. Workspace never has one.
    pub tab_close_hits: Vec<(u16, u16, usize)>,
    /// Icon spans painted on tree and graph rows, rebuilt every paint.
    ///
    /// Empty when no row shows an icon. [`Self::pr_badge_hits`] reads the
    /// PR badges from it.
    pub icon_hits: Vec<IconHit>,
    /// Comment-mark targets of the right pane's line viewer cursor line, as
    /// this frame's line viewer paint read them (the file diff's cursor
    /// row from the rows it built, or the clean-file view's cursor line),
    /// rebuilt every paint. A popover asks here whether its line-mark
    /// section is on the cursor line, so its paint builds no diff rows.
    pub cursor_line_marks: Vec<IconTarget>,
    /// Hyperlink spans of the frame, rebuilt every paint: the OSC 8 overlay
    /// prints exactly these.
    ///
    /// Each painted PR badge ref (from `pr_badge_links` in `render.rs`), then
    /// the popover URL line. A dialog or the popover drops the links under
    /// its box before it records its own.
    pub links: Vec<LinkHit>,
    /// Curly-underline spans of the frame, rebuilt every paint: the code
    /// columns of each painted row of a line with an open comment, in the
    /// diff pane and the file view. A dialog, the popover, or a toast drops
    /// the spans under its box, and link cells are cut out at the end of
    /// the paint. The post-draw writer prints exactly these.
    pub undercurls: Vec<UndercurlHit>,
    /// The popover painted last frame, if any.
    pub popover: Option<PopoverHit>,
    /// First tab the strip painted. Kept across paints so the window only
    /// scrolls when the active tab would leave it. `0` when every tab fits.
    pub tab_scroll: usize,
    /// Max help body scroll the last paint allowed. `0` when help is closed
    /// or every body row fits.
    pub help_scroll_max: usize,
    /// 0-based first column of the painted file view body (gutter included).
    pub file_view_x: u16,
    /// 0-based first row of the painted file view body.
    pub file_view_y: u16,
    /// File view body width, gutter and vertical scrollbar column included.
    pub file_view_width: u16,
    /// File view line rows: the body height less the horizontal scrollbar
    /// row a panned view keeps.
    pub file_view_height: u16,
    /// The file body's rows across the full pane width: the flat pad cells
    /// beside [`Self::file_view_x`]..[`Self::file_view_width`] included
    /// (the body itself when boxed). A press here picks the row.
    pub file_view_rows: Rect,
    /// 0-based file line painted on each body row from
    /// [`Self::file_view_y`] (a wrapped line repeats). Empty when no file
    /// view paints.
    pub file_view_row_lines: Vec<usize>,
    /// Inner area of the painted Explorer tree pane (empty off an Explorer
    /// tab).
    pub explorer_tree: Rect,
    /// Inner area of the painted Explorer preview pane (empty off an
    /// Explorer tab).
    pub explorer_preview: Rect,
    /// [`Self::explorer_tree`]'s rows across the full pane width, flat pad
    /// cells included (the inner area when boxed). Clicks and the wheel
    /// hit rows here; text selection keeps to the inner area.
    pub explorer_tree_rows: Rect,
    /// [`Self::explorer_preview`]'s rows across the full pane width, as
    /// [`Self::explorer_tree_rows`].
    pub explorer_preview_rows: Rect,
    /// Inner area of the painted PRs tab list pane (empty off the PRs tab
    /// and while the narrow layout shows the detail).
    pub prs_list: Rect,
    /// The rows of the PRs tab list lines (below the error and loading
    /// lines) across the full pane width, flat pad cells included. A press
    /// or the wheel here hits a line; the first row is line
    /// [`super::tabs::PrsTab::scroll`].
    pub prs_list_rows: Rect,
    /// Inner area of the painted PRs tab detail pane (empty when it does
    /// not paint).
    pub prs_detail: Rect,
    /// [`Self::prs_detail`]'s rows across the full pane width.
    pub prs_detail_rows: Rect,
    /// Cells of the workspace PR total on the workspace row (`8 PRs · 2
    /// need you`) when the tree painted it; a click there opens the PRs
    /// tab.
    pub pr_total_hit: Option<Rect>,
    /// Results rows the search-in-files dialog painted last; PgUp / PgDn
    /// move by it.
    pub search_files_rows: u16,
    /// The last draw painted a time-based frame (a spinner or a fading toast).
    /// [`AppState::next_anim_tick_ms`] keeps redrawing while it holds.
    pub animating: bool,
    /// The last draw painted a scroll glide step; redraws at the draw floor
    /// while it holds.
    pub gliding: bool,
    /// One redraw wanted this many ms after the last draw, with no frame
    /// ticks before it: a toast at full strength wakes the loop when its
    /// fade starts (under `NO_COLOR`, when it expires).
    pub wake_ms: Option<u64>,
    /// Row of the search-in-files dialog's top border, where its option
    /// chips paint.
    pub search_option_y: u16,
    /// Option chip hit boxes `(x, width, option)` on
    /// [`Self::search_option_y`], each chip with its padding spaces.
    /// Rebuilt every paint; empty when the dialog did not paint its chips.
    pub search_option_hits: Vec<(u16, u16, SearchFilesOption)>,
}

impl Default for LayoutHit {
    fn default() -> Self {
        Self {
            tree_x: 0,
            tree_y: 1,
            tree_width: 40,
            tree_height: 20,
            right_x: 40,
            list_offset: 0,
            term_cols: 120,
            term_rows: 24,
            pane_height: 22,
            outer_tree_width: 48,
            diff_pane_width: 70,
            diff_pane_height: 20,
            diff_content_x: 50,
            diff_split_rule_x: None,
            right_y: 1,
            files_list_y: 3,
            files_list_height: 20,
            files_list_offset: 0,
            graph_scrollbar_x: None,
            graph_scrollbar_y: 0,
            graph_scrollbar_height: 0,
            graph_content_len: 0,
            graph_painted_top: None,
            graph_hscrollbar_y: None,
            graph_hscrollbar_x: 0,
            graph_hscrollbar_width: 0,
            graph_col_max: 0,
            graph_footer_y: None,
            graph_footer_x: 0,
            graph_footer_width: 0,
            graph_footer_height: 0,
            graph_footer_message_rows: 0,
            graph_footer_scroll_max: 0,
            graph_width: 0,
            graph_list_height: 0,
            commit_detail_footer_y: None,
            commit_detail_footer_x: 0,
            commit_detail_footer_width: 0,
            commit_detail_footer_height: 0,
            commit_detail_footer_message_rows: 0,
            commit_detail_footer_scroll_max: 0,
            commit_msg_rule_y: None,
            commit_msg_rule_x: 0,
            commit_msg_rule_width: 0,
            commit_msg_lines_fit: COMMIT_MSG_LINES_MAX,
            diff_vbar: VScrollbarLayout::default(),
            diff_hscrollbar_y: None,
            diff_hscrollbar_x: 0,
            diff_hscrollbar_width: 0,
            diff_col_max: 0,
            tree_vbar: VScrollbarLayout::default(),
            files_vbar: VScrollbarLayout::default(),
            explorer_tree_vbar: VScrollbarLayout::default(),
            tree_hbar: HScrollbarLayout::default(),
            files_hbar: HScrollbarLayout::default(),
            explorer_tree_hbar: HScrollbarLayout::default(),
            file_view_vbar: VScrollbarLayout::default(),
            file_view_hbar: HScrollbarLayout::default(),
            summary_vbar: VScrollbarLayout::default(),
            summary_hbar: HScrollbarLayout::default(),
            prs_list_vbar: VScrollbarLayout::default(),
            prs_detail_vbar: VScrollbarLayout::default(),
            summary_list_height: 0,
            tab_y: 0,
            tab_hits: Vec::new(),
            tab_close_hits: Vec::new(),
            icon_hits: Vec::new(),
            links: Vec::new(),
            undercurls: Vec::new(),
            popover: None,
            tab_scroll: 0,
            help_scroll_max: 0,
            file_view_x: 0,
            file_view_y: 0,
            file_view_width: 0,
            file_view_height: 0,
            file_view_rows: Rect::default(),
            file_view_row_lines: Vec::new(),
            explorer_tree: Rect::default(),
            explorer_preview: Rect::default(),
            explorer_tree_rows: Rect::default(),
            explorer_preview_rows: Rect::default(),
            prs_list: Rect::default(),
            prs_list_rows: Rect::default(),
            prs_detail: Rect::default(),
            prs_detail_rows: Rect::default(),
            pr_total_hit: None,
            search_files_rows: 0,
            animating: false,
            gliding: false,
            wake_ms: None,
            search_option_y: 0,
            search_option_hits: Vec::new(),
            cursor_line_marks: Vec::new(),
        }
    }
}

impl LayoutHit {
    /// The PR badges among [`Self::icon_hits`], in paint order.
    pub fn pr_badge_hits(&self) -> Vec<PrBadgeHit> {
        self.icon_hits
            .iter()
            .filter_map(|hit| match &hit.target {
                IconTarget::PullRequest(repo) => Some(PrBadgeHit {
                    y: hit.y,
                    x: hit.x,
                    width: hit.width,
                    repo: repo.clone(),
                }),
                IconTarget::TreeRow(_)
                | IconTarget::CommitFileRow(_)
                | IconTarget::ExplorerRow(_)
                | IconTarget::GraphRow(_)
                | IconTarget::GraphWorktree(_)
                | IconTarget::GraphChip { .. }
                | IconTarget::GraphSync
                | IconTarget::GraphMoreLines(_)
                | IconTarget::CommitDetailMoreLines(_)
                | IconTarget::Commit { .. }
                | IconTarget::LineComment { .. } => None,
            })
            .collect()
    }
}

/// Where the last frame painted the popover, for clicks and drags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverHit {
    /// Box, border included.
    pub rect: Rect,
    /// Text area inside the border and padding. A drag selects in here.
    pub inner: Rect,
    /// `(screen row, line index)` of each painted field or action line.
    /// The index is into [`super::popover::flat_lines`].
    pub lines: Vec<(u16, usize)>,
}

/// The value of the box in `hits` (`(x, width, value)` on row `y`) that
/// covers cell (`col`, `row`).
fn hit_row_box<T: Copy>(hits: &[(u16, u16, T)], y: u16, col: u16, row: u16) -> Option<T> {
    if row != y {
        return None;
    }
    hits.iter()
        .find(|(x, width, _)| col >= *x && col < x.saturating_add(*width))
        .map(|(_, _, value)| *value)
}

/// Whether cell `(col, row)` is on a recorded footer box: `top` is its
/// first row (`None` when no scrolling footer painted this frame), then
/// `x`, `width`, and `height`. The wheel hit test of every
/// commit-message footer.
fn on_footer_box(
    (col, row): (u16, u16),
    top: Option<u16>,
    x: u16,
    width: u16,
    height: u16,
) -> bool {
    top.is_some_and(|top| {
        (top..top.saturating_add(height)).contains(&row)
            && (x..x.saturating_add(width)).contains(&col)
    })
}

/// Cursor, scroll, and folds saved when `/` opens. Every query change and
/// Enter step forward from it, so typing never walks further per key. Esc
/// while typing restores the bound pane from it, so an incremental search
/// that moved the cursor leaves no trace.
#[derive(Clone, Debug)]
struct SearchOrigin {
    tree_row: Option<String>,
    folds: HashSet<String>,
    graph_cursor: usize,
    graph_scroll: u16,
    commit_file_cursor: usize,
    commit_file_folds: HashSet<String>,
    diff_cursor: usize,
    diff_scroll: u16,
    file_cursor: usize,
    file_scroll: usize,
    /// Cursor rel of the active Explorer tab's tree.
    explorer_cursor: Option<String>,
}

/// Confirm overlay before a destructive file write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PendingConfirm {
    Revert {
        targets: Vec<RevertTarget>,
        /// Focused-row path shown in the overlay.
        label: String,
    },
    /// DiffVisual `x`: discard the highlighted lines of one file.
    ///
    /// The patch is built before the confirm opens, so a refusal is
    /// immediate and `y` applies exactly what the overlay named.
    RevertRange {
        repo: String,
        path: String,
        patch: String,
    },
    StashDrop {
        repo: String,
        stash_ref: String,
    },
    CheckoutOutOfSync {
        repo: String,
        branch: String,
        remote_ref: String,
        /// Commits only on the local branch, then only on `remote_ref`, when git could count them.
        ahead_behind: Option<(usize, usize)>,
    },
    RemoveWorktree {
        primary: String,
        path: String,
        force: bool,
        branch: String,
        merged_into_default: Option<bool>,
        /// Changed files in the worktree (snapshot count) that `--force` deletes.
        changed: usize,
    },
    /// `d` on a scope that resolves to more than one repo off its default branch.
    SwitchToDefault {
        repos: Vec<String>,
    },
    MergeIntoHead {
        repo: String,
        rev: String,
        label: String,
        into: String,
    },
    /// `P` on a checkout with commits to push: `y` runs the push.
    ///
    /// Branch, remote, and `publish` come from [`crate::git::push_plan`],
    /// read off the loop thread before the box opens.
    Push {
        /// Checkout path (snapshot `repo`).
        repo: String,
        /// Branch the push sends.
        branch: String,
        /// Remote the push goes to.
        remote: String,
        /// True when the push creates the remote branch (`git push -u`).
        publish: bool,
        /// Commits ahead of, then behind, the upstream from the snapshot
        /// sync note; `None` with no upstream or no parsable count.
        ahead_behind: Option<(u32, u32)>,
    },
    /// `p` on a diverged checkout whose upstream was force-pushed and holds
    /// nothing of yours: `y` moves the branch to the upstream with a guarded
    /// `git reset --keep` ([`crate::git::reset_keep_to_upstream`]).
    ResetToUpstream {
        /// Checkout path (snapshot `repo`).
        repo: String,
        /// Local branch the reset moves.
        branch: String,
        /// Upstream ref the branch moves to, for example `origin/main`.
        upstream: String,
        /// Branch tip the snapshot classified. The reset refuses when the
        /// branch has moved since.
        tip: String,
        /// Commits the upstream dropped that the reset leaves behind.
        dropped: usize,
    },
    /// Compare-tab highlight `x`: put the merge-base lines back for the
    /// highlighted rows of the committed compare diff (worktree only).
    ///
    /// The patch is built before the confirm opens from loaded content.
    CompareRevertRange {
        target: CompareRevertTarget,
        patch: String,
    },
    /// Compare-tab `x` on a file: restore it in the worktree from the
    /// merge base. Status `A` deletes it; `R` also restores `old_path`.
    CompareRevertFile {
        target: CompareRevertTarget,
    },
}

impl PendingConfirm {
    /// The key that accepts this confirm: `Y` when only `Y` is offered
    /// (several untracked files), otherwise `y`.
    pub fn accept_key(&self) -> char {
        match self {
            Self::Revert { targets, .. }
                if revert_scope(targets).key_deletes_untracked(false).is_none() =>
            {
                'Y'
            }
            _ => 'y',
        }
    }

    /// True for the confirms a compare tab opens (and may accept).
    pub fn is_compare_owned(&self) -> bool {
        matches!(
            self,
            Self::CompareRevertRange { .. } | Self::CompareRevertFile { .. }
        )
    }
}

/// Ahead / behind counts for the push confirm, from the snapshot sync note
/// (`ahead by N commits`, `diverged (ahead A, behind B)`).
///
/// `None` with no upstream or a note in another shape.
fn push_ahead_behind(sync: SyncStatus, note: &str) -> Option<(u32, u32)> {
    match sync {
        SyncStatus::Ahead => {
            let n = note.strip_prefix("ahead by ")?.strip_suffix(" commits")?;
            Some((n.parse().ok()?, 0))
        }
        SyncStatus::Diverged => {
            let inner = note.strip_prefix("diverged (ahead ")?.strip_suffix(')')?;
            let (ahead, behind) = inner.split_once(", behind ")?;
            Some((ahead.parse().ok()?, behind.parse().ok()?))
        }
        _ => None,
    }
}

/// One compare-tab file that `x` may revert to the merge base.
///
/// Built only after [`AppState::compare_revert_target`] proves the compare
/// head is the checked-out working tree for the file. The write worker
/// checks `head` and cleanliness again right before it writes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompareRevertTarget {
    /// Checkout path (snapshot `repo`).
    pub repo: String,
    /// File path on the head side.
    pub path: String,
    /// Merge-base path of a rename.
    pub old_path: Option<String>,
    /// One-letter `name-status` of `path` in the compare range.
    pub status: String,
    /// Compare base ref, for confirm copy.
    pub base_ref: String,
    /// Revert target commit (the compare merge base, not the base tip).
    pub merge_base: String,
    /// Compare head. The checkout's HEAD must still be this commit.
    pub head: String,
}

/// One path in a pending revert confirm.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevertTarget {
    pub repo: String,
    pub path: String,
    pub untracked: bool,
    pub old_path: Option<String>,
}

/// Checkout of a compare-owned confirm.
fn compare_repo(pending: &PendingConfirm) -> Option<&str> {
    match pending {
        PendingConfirm::CompareRevertRange { target, .. }
        | PendingConfirm::CompareRevertFile { target } => Some(target.repo.as_str()),
        _ => None,
    }
}

/// Tracked / untracked mix of a whole-file revert confirm's targets.
pub(crate) fn revert_scope(targets: &[RevertTarget]) -> RevertScope {
    let untracked = targets.iter().filter(|t| t.untracked).count();
    RevertScope::of(targets.len() - untracked, untracked)
}

/// File-diff that owns `diff_cursor`, `diff_scroll`, and `diff_col_offset`.
///
/// A new identity resets those fields to the origin. A same-view reload
/// (watch, `Ctrl-o` after the hunk anchor) keeps them.
#[derive(Clone, Debug, PartialEq, Eq)]
enum DiffViewId {
    Workspace {
        repo: String,
        path: String,
    },
    Commit {
        repo: String,
        source: CommitFileSource,
        path: String,
    },
}

/// Cached [`AppState::diff_search_hits`] result.
#[derive(Clone, Debug)]
struct DiffSearchMemo {
    /// Content fingerprint, painted layout, case-folded query.
    key: (u64, DiffMode, String),
    hits: Vec<usize>,
}

/// Memo for [`AppState::cursor_comment_lines`]: content fingerprint,
/// painted layout, diff cursor, and `V` anchor, then the line span.
type CommentLinesMemo = Option<((u64, DiffMode, usize, Option<usize>), Option<(u32, u32)>)>;

/// Source line span a comment on diff rows `rows` targets: the `V` range
/// from `anchor` to `cursor` ([`viewport_line_range`]), else the first
/// numbered row from `cursor` ([`viewport_line_number`]) as a one-line
/// span.
fn comment_line_span(rows: &[DiffRow], cursor: usize, anchor: Option<usize>) -> Option<(u32, u32)> {
    match anchor {
        Some(anchor) => viewport_line_range(rows, anchor, cursor),
        None => viewport_line_number(rows, cursor as u16).map(|line| (line, line)),
    }
}

/// Memo for [`AppState::file_search_hits`].
#[derive(Clone, Debug)]
struct FileSearchMemo {
    /// Tab id, generation of the painted body
    /// ([`super::tabs::FileTab::body_generation`]), case-folded query.
    key: (u64, Option<u64>, String),
    /// Shared with every caller of the frame, so a hit does not copy.
    hits: std::rc::Rc<[usize]>,
}

/// Interactive session state. Dispatch runs no git: that work is the
/// returned [`Effect`].
///
/// Status results and the viewed / comment stores do no file I/O here
/// either: the facts of dirty worktree files come in with each status
/// result ([`Self::set_file_facts`]), and a store change only asks for a
/// save that runs on a worker ([`Self::begin_store_save`]). Small file I/O
/// left on the loop thread: the UI-state file writes (theme, sparkline,
/// commit-message rows), the background-fetch record read on a watch
/// tick, and the one `stat` of the open Explorer file.
#[derive(Clone, Debug)]
pub struct AppState {
    pub cwd: PathBuf,
    pub snapshot: WorkspaceSnapshot,
    pub show_hidden: bool,
    /// Workspace directory tree vs flat paths. Session-only. Default true.
    pub tree_mode: bool,
    /// Commit-file directory tree vs flat paths. Independent of `tree_mode`.
    pub commit_tree_mode: bool,
    /// Folded commit-file dir ids for the current drill list.
    pub commit_file_folds: HashSet<String>,
    pub tree: TreeNode,
    pub folds: HashSet<String>,
    pub rows: Vec<VisibleRow>,
    pub cursor: usize,
    pub help_open: bool,
    pub help_search_query: Option<String>,
    /// Help body rows scrolled off the top when the columns are taller
    /// than the help dialog. Clamped to [`LayoutHit::help_scroll_max`].
    pub help_scroll: usize,
    pub focus: FocusPane,
    /// Breadcrumb trailing slot / overlay status row ([`StatusMessage`]).
    pub status: StatusMessage,
    /// One-shot acks painted bottom-right. Filled from an ok `status` by
    /// [`Self::route_ack_to_toast`] at the top of each draw.
    pub toasts: ToastStack,
    /// Background job sources that fail now ([`Self::note_outage`]). The
    /// tab strip paints a failure mark while one lasts.
    pub outages: Outages,
    pub graph: Option<GraphModel>,
    pub graph_identity: Option<(String, String)>,
    pub graph_scroll: u16,
    pub graph_cursor: usize,
    /// True while the next `git log` window is fetching (`loading older…`).
    pub graph_loading_older: bool,
    /// True while listing commit files.
    pub commit_files_loading: bool,
    pub drill: DrillView,
    pub diff_content: DiffContent,
    pub diff_scroll: u16,
    /// Focused file-diff row (section, hunk, or line).
    pub diff_cursor: usize,
    pub diff_repo: Option<String>,
    pub diff_path: Option<String>,
    /// Painted file-diff identity. Viewport reset follows this, not paint.
    diff_view: Option<DiffViewId>,
    pub reviewed: HashSet<String>,
    pub viewed_store: ViewedStore,
    /// Save state of [`Self::viewed_store`]: the bucket as last loaded or
    /// saved (a save writes only the delta from it, so marks another `ws`
    /// process saved survive) and the save asked for or running.
    viewed_sync: StoreSync<String, String>,
    pub viewed_path: PathBuf,
    pub comment_store: CommentStore,
    /// Save state of [`Self::comment_store`]. Same rules as
    /// [`Self::viewed_sync`].
    comment_sync: StoreSync<CommentKey, CommentEntry>,
    /// Disk facts (`size:mtimeMs`, viewed fingerprint) of each dirty file,
    /// read at launch and then by the status worker of each checkout.
    file_facts: FileFactsCache,
    /// Checkout path → repo identity seen in this session's snapshots
    /// (`CommentLiveSet::checkout_owners`): comment GC drops a gone
    /// worktree's comment only when its owner repo is known.
    comment_checkout_owners: HashMap<String, String>,
    pub comment_path: PathBuf,
    /// UI-state store path (see [`crate::tui::ui_state_store`]). The
    /// palette `Theme: …` rows and Toggle activity sparkline write it;
    /// [`crate::settings::Settings::resolve`] loads it at launch, not
    /// `AppState`.
    pub ui_store_path: PathBuf,
    /// Shared background-fetch store ([`crate::tui::fetch_claim`]). The
    /// scheduled fetch claims it and each watch tick reads its record; set
    /// by [`Self::open_background_fetch_store`].
    pub(crate) fetch_store_path: PathBuf,
    /// Newest background-fetch generation this instance has applied or
    /// written. A record with this generation is not applied again.
    fetch_applied_generation: u64,
    pub comment: Option<CommentPrompt>,
    pub comment_export: Option<CommentExport>,
    /// Visual-line anchor on a focused file diff (`V`). Cursor is the other end.
    pub diff_visual_anchor: Option<usize>,
    /// Painted rows captured when visual-line highlight started.
    ///
    /// Highlight drops when this list no longer matches
    /// [`Self::current_diff_rows`] (watch reload, split/inline, resize).
    diff_visual_rows: Option<Vec<DiffRow>>,
    pub layout: LayoutHit,
    pub ascii: bool,
    /// `NO_COLOR` is set: the new visuals (spinner, toasts, …) paint with no
    /// colour of their own. The rest of the TUI keeps its palette.
    pub no_color: bool,
    /// The terminal gets the curly underline on commented lines
    /// ([`crate::tui::undercurl::undercurl_enabled`], resolved at launch).
    /// Off, those lines keep the plain underline the frame paints.
    pub undercurl: bool,
    /// Spinner clock: every spinner frame counts from this instant.
    pub anim_epoch: Instant,
    /// Ctrl-d / Ctrl-u glide the viewport (`smoothScroll`, default on).
    /// Off, the viewport jumps as the cursor moves.
    pub smooth_scroll: bool,
    /// Paint the tree commit-activity sparkline (the UI-state store value,
    /// default off).
    pub activity_sparkline: bool,
    /// The live Ctrl-d / Ctrl-u viewport glide, if any
    /// ([`Self::arm_scroll_glide`], [`Self::glide_start`]).
    pub(crate) scroll_glide: Option<ScrollGlide>,
    /// Clock of the frame being drawn, set at the top of each draw. Glide
    /// painters read it so one frame uses one instant.
    pub(crate) frame_now: Instant,
    /// Remote git work in flight, published by the interpreter for paint.
    pub remote_activity: RemoteActivity,
    /// Day commit counts (last [`ACTIVITY_DAYS`] days, today last) per
    /// activity key, from the low-priority [`Effect::LoadActivity`] workers
    /// (one job per repo). Empty until those land; the tree paints no
    /// sparkline before.
    pub activity: HashMap<String, [u32; ACTIVITY_DAYS]>,
    pub search_mode: bool,
    pub search_active: bool,
    pub search_query: String,
    /// Pane bound when `/` started. `n`/`N` stay on this pane.
    pub search_target: SearchPane,
    /// Current matching diff line, when the bound pane is a diff.
    pub search_hit: Option<usize>,
    /// Where the bound pane stood when `/` opened. Cleared on Enter / Esc.
    search_origin: Option<SearchOrigin>,
    pub diff_col_offset: u16,
    /// Horizontal pan of the left list (workspace tree or drill graph/files).
    pub left_col_offset: u16,
    /// Horizontal pan of the right list (graph or commit-files).
    pub right_col_offset: u16,
    /// Viewport top and pan of the folder summary, for the folder they
    /// were set on ([`summary::SummaryView`]).
    summary_view: Option<summary::SummaryView>,
    /// File identities currently shown with unlimited `-U` context.
    pub full_context: HashSet<String>,
    /// Search text of the hunk/change row to keep in view after `-U` reload.
    pending_hunk_anchor: Option<String>,
    pub confirm: Option<PendingConfirm>,
    pub stash_menu: Option<Vec<StashOp>>,
    pub stash_repo: Option<String>,
    /// The `A` blame-actions menu is open. Its header reads the focused
    /// line's annotation live, so it holds no copy of the blame.
    pub blame_menu: bool,
    /// The Doctor setup checkup overlay (palette `Doctor: setup checkup`).
    pub doctor: Option<doctor::DoctorOverlay>,
    /// Last Doctor probe request id; only the newest answer lands.
    doctor_gen: u64,
    pub branch_picker: Option<BranchPickerState>,
    pub graph_focus_picker: Option<GraphFocusPickerState>,
    /// Per-repo local branch names whose ancestors the graph shows. `None` = `--all`.
    pub graph_branch_focus: Option<(String, Vec<String>)>,
    pub create_branch: Option<CreateBranchState>,
    /// Open Quick Open overlay (`:` / `>` commands, Ctrl-p / `F` files).
    pub quick_open: Option<QuickOpenState>,
    /// Session counter for Quick Open index and score generations.
    quick_open_gen: u64,
    /// Theme active when Quick Open opened, while it is open. Focusing a
    /// palette `Theme: …` row previews that theme in [`Self::theme`]; any
    /// close but Enter on a theme row restores this one.
    theme_before_preview: Option<ThemeId>,
    /// Open search-in-files dialog (Ctrl-f).
    pub search_files: Option<SearchFilesState>,
    /// Last closed search-in-files dialog, restored by the next Ctrl-f in
    /// the same scope.
    pub search_files_parked: Option<SearchFilesState>,
    /// Session counter for search-in-files index and search generations.
    search_files_gen: u64,
    /// Permanent Workspace plus session compare and Explorer tabs.
    pub tabs: TabStrip,
    pub compare_picker: Option<ComparePickerState>,
    /// Checkout waiting for [`Effect::PrepareComparePicker`]. Cleared on open or abandon.
    pub(crate) compare_picker_pending: Option<String>,
    /// Workspace view parked while a session tab is active.
    workspace_park: Option<TabView>,
    pub flashes: HashMap<String, FlashStamp>,
    pub signatures: BTreeMap<String, String>,
    pub graph_signatures: BTreeMap<String, String>,
    graph_flash_meta: Option<GraphFlashMeta>,
    commit_file_signatures: BTreeMap<String, String>,
    tree_ghosts: Vec<GhostRow<VisibleRow>>,
    commit_file_ghosts: Vec<GhostRow<CommitFileRow>>,
    pub tree_fraction: f64,
    pub diff_split_fraction: f64,
    pub diff_mode: DiffMode,
    /// Soft-wrap file-diff code. On by default; `viewDefaults.wrap` sets the
    /// launch value and `\` flips it for this session (no XDG store).
    pub diff_wrap: bool,
    /// Expand commit / stash messages in the graph footer and commit-files
    /// header. On by default; `M` collapses for this session (no XDG store).
    /// Graph list rows stay one line.
    pub commit_msg_expand: bool,
    /// Message rows of the expanded commit-message footer (graph pane and
    /// commit-files pane). The footer keeps this height whatever the
    /// selected message is. Launch value: the UI-state store value a drag
    /// saved (`saved_commit_msg_lines`), else
    /// `viewDefaults.commitMessageLines`, else
    /// [`COMMIT_MSG_LINES_DEFAULT`]. A drag of the graph footer's top rule
    /// changes it and saves it to the UI-state store on release; the
    /// Shorter / Taller commit message palette rows change it for this
    /// session only (no save). Always in
    /// [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`].
    pub commit_msg_lines: usize,
    /// Commit-message rows the UI-state store held at launch (`None` when
    /// none or a bad value). They beat `viewDefaults.commitMessageLines` in
    /// [`Self::apply_view_defaults`].
    saved_commit_msg_lines: Option<usize>,
    /// Scroll of an expanded message taller than its footer, keyed by the
    /// graph row identity of the commit or stash it belongs to. The graph
    /// selection footer and the depth-2 commit-files footer share it, so
    /// the same commit reads the same offset on both; any other commit
    /// reads `0`.
    commit_msg_scroll: Option<(String, usize)>,
    pub drag: SplitDrag,
    /// Flat panes: a press on a pad cell at the pane join (the divider grab
    /// band). It arms [`SplitDrag::Pane`]; a release before the pointer
    /// leaves its column makes it a row click at that cell instead.
    pad_press: Option<(u16, u16)>,
    /// Mouse text selection armed by a press in a pane body.
    pub text_selection: Option<TextSelection>,
    /// Last painted frame before the selection highlight. Release copies from it.
    pub(crate) painted_frame: Buffer,
    /// Last frame was below [`super::split::MIN_TERM_COLS`] ×
    /// [`super::split::MIN_TERM_ROWS`] and painted only the resize notice.
    /// Mouse events are dropped then, since no pane was painted to hit.
    pub(crate) too_small: bool,
    pub theme: ThemeId,
    /// Who paints pane and chrome backgrounds. Launch default
    /// [`BackgroundMode::Paint`]; `viewDefaults.background` sets it.
    pub background: BackgroundMode,
    pub mouse_enabled: bool,
    /// Last pointer cell `(col, row)` from any-event motion. `None` when
    /// unknown or mouse capture is off. Paint derives the tab `[✗]` and
    /// search option chip hover from it, so a stale layout never keeps a
    /// stale highlight.
    pub pointer: Option<(u16, u16)>,
    /// Open icon popover: a hover peek or a pinned popover.
    pub popover: Option<PopoverState>,
    /// Hover dwell and leave grace of the icon peek.
    pub(crate) peek: popover::PeekTimers,
    /// Pinned popover line a left press landed on. Release runs it unless
    /// the press turned into a drag.
    popover_press: Option<popover::PopoverLineRef>,
    pub(crate) z_pending_at: Option<Instant>,
    pub(crate) g_pending_at: Option<Instant>,
    /// Typeless CSI-u release-as-press for `g`-chord keys.
    pub(crate) g_chord_echo: GChordEchoState,
    pub(crate) ctrl_c_armed_until: Option<Instant>,
    /// Ctrl-C arm whose prompt the loop timer cleared. Kept so a press that
    /// arrived before it ended, but was handled after, still quits.
    ctrl_c_closed_until: Option<Instant>,
    last_click: Option<(u16, u16, Instant)>,
    /// When the input event being handled arrived (its stdin read). Chord
    /// and double-click windows measure from it. `None` outside input
    /// handling, where they use the wall clock ([`Self::input_clock`]).
    input_at: Option<Instant>,
    /// PR answers for branch-row badges, per checkout and (remote, branch).
    pr_cache: pull_request::PrCache,
    /// PRs tab group titles from the title command, and its fill state.
    pr_titles: prs_tab::PrTitles,
    /// Diff search hits for one (content, layout, query), so the status
    /// pill and the diff paint do not rebuild every diff row each frame.
    diff_search_memo: RefCell<Option<DiffSearchMemo>>,
    /// File view search hits for one (tab id, load generation, case-folded
    /// query), so the status pill and the paint do not rescan the file.
    file_search_memo: RefCell<Option<FileSearchMemo>>,
    /// Last Explorer status index: checkout, fingerprint of its snapshot
    /// changes, and the index. Paint asks several times a frame; a status
    /// reload that changes the checkout's changes changes the fingerprint.
    explorer_status_memo: RefCell<Option<(String, u64, std::rc::Rc<ExplorerStatus>)>>,
    /// Current-line blame toggle and answer cache.
    pub line_blame: LineBlameState,
    /// Commit popover cards looked up off the graph, per (checkout, full
    /// commit id). Graph commits never land here.
    pub commit_cards: CommitCardCache,
    /// Focused diff row's source line, so the blame hook does not rebuild
    /// the diff rows after every schedule and apply.
    line_blame_row_memo: RefCell<line_blame::RowLineMemo>,
    /// Source line span a diff comment targets
    /// ([`Self::cursor_comment_lines`]), so the Comment gate a line-comment
    /// popover asks on every frame does not rebuild the diff rows.
    comment_lines_memo: RefCell<CommentLinesMemo>,
    /// Pending blame "show commit in graph": selected once the graph
    /// holds the commit. Dropped when the graph repo changes.
    pub graph_reveal: Option<GraphReveal>,
    /// Latest blame "previous line change" request; an older result is
    /// dropped.
    blame_previous_gen: u64,
    /// [`Self::current_diff_rows`] calls, so tests can bound the row
    /// builds a frame or keypress costs on a large diff.
    #[cfg(test)]
    pub(crate) diff_row_builds: std::cell::Cell<usize>,
}

/// Wall clock now, in Unix seconds (0 before the epoch).
pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl AppState {
    /// TUI launch state: glyphs, theme, `NO_COLOR`, and the viewed /
    /// comment / UI-state / background-fetch store paths and the activity
    /// sparkline come from the resolved `settings`.
    pub fn launch(cwd: PathBuf, snapshot: WorkspaceSnapshot, settings: &Settings) -> Self {
        let mut state = Self::build(
            cwd,
            snapshot,
            settings.ascii,
            settings.viewed_store.clone(),
            settings.comment_store.clone(),
            settings.ui_store.clone(),
            settings.theme,
        );
        state.no_color = settings.no_color;
        state.undercurl = settings.undercurl;
        state.smooth_scroll = settings.smooth_scroll;
        state.set_pr_poll_mode(settings.pr_poll);
        state.pr_titles.command = settings.pr_title_command.clone();
        state.activity_sparkline = settings.activity_sparkline;
        state.saved_commit_msg_lines = settings.commit_msg_lines;
        if let Some(lines) = settings.commit_msg_lines {
            state.commit_msg_lines = lines;
        }
        state.open_background_fetch_store(settings.fetch_store.clone());
        // A checkout whose `git status` failed in the launch snapshot
        // starts the live-refresh outage at once.
        state.sync_live_refresh_outage(Instant::now());
        state
    }

    /// Test state with a unique temp viewed store, Tokyo Night, and
    /// [`BackgroundMode::Terminal`] (see [`Self::with_viewed_path`]).
    #[cfg(test)]
    pub fn new(cwd: PathBuf, snapshot: WorkspaceSnapshot, ascii: bool) -> Self {
        Self::with_viewed_path(cwd, snapshot, ascii, default_viewed_path())
    }

    /// Test state with `viewed_path`; the comment, UI-state, and
    /// background-fetch stores sit next to it.
    ///
    /// Pins Tokyo Night and [`BackgroundMode::Terminal`], the v0.1.244 look
    /// (theme, boxed panes, no fills), so render tests keep proving that mode
    /// is unchanged. Tests of the painted look or the shipped default theme
    /// set [`BackgroundMode::Paint`] / the theme themselves; the shipped
    /// launch default is pinned by `launch_with_no_env_and_no_config_paints_background`.
    #[cfg(test)]
    pub(crate) fn with_viewed_path(
        cwd: PathBuf,
        snapshot: WorkspaceSnapshot,
        ascii: bool,
        viewed_path: PathBuf,
    ) -> Self {
        let comment_path = viewed_path.with_file_name("comments.json");
        let ui_store_path = viewed_path.with_file_name("ui-state.json");
        let mut state = Self::build(
            cwd,
            snapshot,
            ascii,
            viewed_path,
            comment_path,
            ui_store_path,
            ThemeId::TokyoNight,
        );
        state.background = BackgroundMode::Terminal;
        let fetch_store = state.viewed_path.with_file_name("background-fetch.json");
        state.open_background_fetch_store(fetch_store);
        state
    }

    fn build(
        cwd: PathBuf,
        snapshot: WorkspaceSnapshot,
        ascii: bool,
        viewed_path: PathBuf,
        comment_path: PathBuf,
        ui_store_path: PathBuf,
        theme: ThemeId,
    ) -> Self {
        let show_hidden = snapshot.show_hidden;
        let tree_mode = true;
        let commit_tree_mode = true;
        let visible = visible_snapshot(&snapshot, show_hidden);
        let tree = build_tree(&visible, tree_mode, &workspace_label_from_cwd(&cwd));
        let folds = default_folds(&tree);
        let rows = flatten_with(&tree, &folds, ascii);
        let cursor = initial_cursor(&rows);
        // Launch reads the dirty files once, before the loop starts; status
        // results bring them after that.
        let file_facts = FileFactsCache::from_disk(&cwd, &snapshot);
        let signatures = tree_signatures(&tree, &file_facts);
        let workspace_id = workspace_store_id(&cwd);
        let viewed_store = load_viewed_store(&viewed_path, &workspace_id);
        let comment_store = load_comment_store(&comment_path, &workspace_id);
        let mut state = Self {
            cwd,
            snapshot,
            show_hidden,
            tree_mode,
            commit_tree_mode,
            commit_file_folds: HashSet::new(),
            tree,
            folds,
            rows,
            cursor,
            help_open: false,
            help_search_query: None,
            help_scroll: 0,
            focus: FocusPane::Left,
            status: StatusMessage::default(),
            toasts: ToastStack::default(),
            outages: Outages::default(),
            graph: None,
            graph_identity: None,
            graph_scroll: 0,
            graph_cursor: 0,
            graph_loading_older: false,
            commit_files_loading: false,
            drill: DrillView::Graph,
            diff_content: DiffContent::default(),
            diff_scroll: 0,
            diff_cursor: 0,
            diff_repo: None,
            diff_path: None,
            diff_view: None,
            reviewed: HashSet::new(),
            viewed_sync: StoreSync::new(&viewed_store),
            viewed_store,
            viewed_path,
            comment_sync: StoreSync::new(&comment_store),
            comment_store,
            file_facts,
            comment_checkout_owners: HashMap::new(),
            comment_path,
            ui_store_path,
            fetch_store_path: PathBuf::new(),
            fetch_applied_generation: 0,
            comment: None,
            comment_export: None,
            diff_visual_anchor: None,
            diff_visual_rows: None,
            layout: LayoutHit::default(),
            ascii,
            no_color: false,
            undercurl: false,
            anim_epoch: Instant::now(),
            smooth_scroll: true,
            activity_sparkline: false,
            scroll_glide: None,
            frame_now: Instant::now(),
            remote_activity: RemoteActivity::default(),
            activity: HashMap::new(),
            search_mode: false,
            search_active: false,
            search_query: String::new(),
            search_target: SearchPane::Tree,
            search_hit: None,
            search_origin: None,
            diff_col_offset: 0,
            left_col_offset: 0,
            right_col_offset: 0,
            summary_view: None,
            full_context: HashSet::new(),
            pending_hunk_anchor: None,
            confirm: None,
            stash_menu: None,
            stash_repo: None,
            blame_menu: false,
            doctor: None,
            doctor_gen: 0,
            branch_picker: None,
            graph_focus_picker: None,
            graph_branch_focus: None,
            create_branch: None,
            quick_open: None,
            quick_open_gen: 0,
            theme_before_preview: None,
            search_files: None,
            search_files_parked: None,
            search_files_gen: 0,
            tabs: TabStrip::default(),
            compare_picker: None,
            compare_picker_pending: None,
            workspace_park: None,
            flashes: HashMap::new(),
            signatures,
            graph_signatures: BTreeMap::new(),
            graph_flash_meta: None,
            commit_file_signatures: BTreeMap::new(),
            tree_ghosts: Vec::new(),
            commit_file_ghosts: Vec::new(),
            tree_fraction: TREE_WIDTH_FRACTION,
            diff_split_fraction: DIFF_SPLIT_FRACTION,
            diff_mode: DiffMode::SideBySide,
            diff_wrap: true,
            commit_msg_expand: true,
            commit_msg_lines: COMMIT_MSG_LINES_DEFAULT,
            saved_commit_msg_lines: None,
            commit_msg_scroll: None,
            drag: SplitDrag::None,
            pad_press: None,
            text_selection: None,
            painted_frame: Buffer::default(),
            too_small: false,
            theme,
            background: BackgroundMode::default(),
            mouse_enabled: true,
            pointer: None,
            popover: None,
            peek: popover::PeekTimers::default(),
            popover_press: None,
            z_pending_at: None,
            g_pending_at: None,
            g_chord_echo: GChordEchoState::default(),
            ctrl_c_armed_until: None,
            ctrl_c_closed_until: None,
            last_click: None,
            input_at: None,
            pr_cache: pull_request::PrCache::default(),
            pr_titles: prs_tab::PrTitles::default(),
            diff_search_memo: RefCell::new(None),
            file_search_memo: RefCell::new(None),
            explorer_status_memo: RefCell::new(None),
            line_blame: LineBlameState::default(),
            commit_cards: CommitCardCache::default(),
            line_blame_row_memo: RefCell::new(None),
            comment_lines_memo: RefCell::new(None),
            graph_reveal: None,
            blame_previous_gen: 0,
            #[cfg(test)]
            diff_row_builds: std::cell::Cell::new(0),
        };
        state.reconcile_viewed_store();
        state.reconcile_comment_store();
        state
    }

    /// Apply the workspace config `viewDefaults` once at launch.
    ///
    /// Sets each key that is present and leaves omitted keys at the in-app
    /// defaults from [`Self::new`]. A `tree` change rebuilds the workspace
    /// tree, folds, rows, cursor, and signatures like the `t` toggle, with
    /// no status message. `commitTree` also sets the file list mode that
    /// each new compare tab opens in.
    /// `commitMessageLines` applies only when the UI-state store held no
    /// valid saved value at launch: saved > config > default.
    pub fn apply_view_defaults(&mut self, defaults: &ViewDefaults) {
        if let Some(tree) = defaults.tree {
            if tree != self.tree_mode {
                self.tree_mode = tree;
                self.tree = self.visible_tree();
                self.folds = default_folds(&self.tree);
                self.rows = flatten_with(&self.tree, &self.folds, self.ascii);
                self.cursor = initial_cursor(&self.rows);
                self.signatures = tree_signatures(&self.tree, &self.file_facts);
                self.tree_ghosts.clear();
            }
        }
        if let Some(commit_tree) = defaults.commit_tree {
            self.commit_tree_mode = commit_tree;
            self.tabs.commit_tree_default = commit_tree;
        }
        if let Some(split) = defaults.diff_split {
            self.diff_mode = if split {
                DiffMode::SideBySide
            } else {
                DiffMode::Inline
            };
        }
        if let Some(wrap) = defaults.wrap {
            self.diff_wrap = wrap;
        }
        if let Some(expand) = defaults.commit_message_expand {
            self.commit_msg_expand = expand;
        }
        // A saved drag value beats the config baseline.
        if let Some(lines) = self
            .saved_commit_msg_lines
            .or(defaults.commit_message_lines)
        {
            self.commit_msg_lines = lines.clamp(COMMIT_MSG_LINES_MIN, COMMIT_MSG_LINES_MAX);
        }
        if let Some(on) = defaults.line_blame {
            self.line_blame.set_enabled(on);
        }
        if let Some(background) = defaults.background {
            self.background = background;
        }
    }

    pub fn input_mode(&self) -> InputMode {
        if self.confirm.is_some() {
            InputMode::Confirm
        } else if self.stash_menu.is_some() {
            InputMode::StashMenu
        } else if self.blame_menu {
            InputMode::BlameMenu
        } else if self.comment.is_some() {
            InputMode::Comment
        } else if self.comment_export.is_some() {
            InputMode::CommentExport
        } else if self.create_branch.is_some() {
            InputMode::CreateBranch
        } else if self.branch_picker.is_some() {
            InputMode::BranchPicker
        } else if self.compare_picker.is_some() {
            InputMode::ComparePicker
        } else if self.graph_focus_picker.is_some() {
            InputMode::GraphFocusPicker
        } else if self.quick_open.is_some() {
            InputMode::QuickOpen
        } else if let Some(dialog) = &self.search_files {
            InputMode::SearchFiles {
                results: dialog.zone == SearchZone::Results,
            }
        } else if self.help_open {
            if self.help_search_query.is_some() {
                InputMode::HelpSearch
            } else {
                InputMode::Help
            }
        } else if self.doctor.is_some() {
            InputMode::Doctor
        } else if self.popover_pinned() {
            if self.chord_pending(self.g_pending_at) {
                InputMode::PopoverGPending
            } else {
                InputMode::Popover
            }
        } else if self.search_mode {
            InputMode::SearchPrompt
        } else if self.diff_visual_anchor.is_some()
            && self.list_focus_target() == ListFocusTarget::None
        {
            InputMode::DiffVisual
        } else if self.chord_pending(self.z_pending_at) {
            InputMode::ZPending {
                search_active: self.search_active,
            }
        } else if self.chord_pending(self.g_pending_at) {
            InputMode::GPending {
                search_active: self.search_active,
            }
        } else {
            InputMode::Normal {
                search_active: self.search_active,
            }
        }
    }

    fn chord_pending(&self, at: Option<Instant>) -> bool {
        at.is_some_and(|t| {
            self.input_clock().saturating_duration_since(t) <= Duration::from_millis(DOUBLE_TAP_MS)
        })
    }

    /// Start handling an input event whose bytes arrived at `read_at`.
    ///
    /// Until [`Self::end_input`], chord and double-click windows measure
    /// from `read_at`, so two keys from one burst keep their meaning when
    /// the loop handles them late.
    pub(crate) fn begin_input(&mut self, read_at: Instant) {
        self.input_at = Some(read_at);
    }

    /// Stop measuring from the last input event. Paint and timers use the
    /// wall clock again.
    pub(crate) fn end_input(&mut self) {
        self.input_at = None;
    }

    /// Arrival instant of the input event being handled, else now.
    fn input_clock(&self) -> Instant {
        self.input_at.unwrap_or_else(Instant::now)
    }

    /// Drop a stale arming-`g` echo after [`DOUBLE_TAP_MS`].
    ///
    /// Call this before the `g`-chord echo filter so a late `g` is a new
    /// tap. An echo of `t` / `T` / `1`–`9` after a consumed chord stays.
    pub(crate) fn expire_stale_g_chord(&mut self) {
        let now = self.input_clock();
        expire_stale_g_chord_echo(&mut self.g_chord_echo, &mut self.g_pending_at, now);
    }

    pub fn focused_row(&self) -> Option<&VisibleRow> {
        self.rows.get(self.cursor)
    }

    /// Checkout path for the focused tree row, if any.
    pub fn focused_checkout_path(&self) -> Option<String> {
        self.focused_row().and_then(|row| row.repo.clone())
    }

    pub fn right_is_diff(&self) -> bool {
        if self.is_compare_tab() {
            return true;
        }
        match &self.drill {
            DrillView::Diff { .. } => true,
            DrillView::Files { .. } => false,
            DrillView::Graph => matches!(self.focused_row().map(|r| r.kind), Some(NodeKind::File)),
        }
    }

    pub fn graph_stash_focused(&self) -> bool {
        !self.is_explorer_tab()
            && !self.is_prs_tab()
            && self.graph_pane_focused()
            && self.focused_graph_stash_ref().is_some()
    }

    pub fn graph_commit_focused(&self) -> bool {
        !self.is_explorer_tab()
            && !self.is_prs_tab()
            && self.graph_pane_focused()
            && matches!(self.focused_graph_row(), Some(GraphRow::Commit { .. }))
    }

    pub fn in_commit_drill(&self) -> bool {
        self.is_compare_tab() || self.drill.is_files() || self.drill.is_diff()
    }

    /// True when a compare tab is active.
    pub fn is_compare_tab(&self) -> bool {
        self.tabs.active_compare().is_some()
    }

    /// ViewStack depth: 0 workspace or compare, 1 commit files, 2 commit diff.
    fn nav_depth(&self) -> u8 {
        if self.is_compare_tab() {
            return 0;
        }
        match self.drill {
            DrillView::Graph => 0,
            DrillView::Files { .. } => 1,
            DrillView::Diff { .. } => 2,
        }
    }

    /// True when unshifted `h` / `l` fold the focused row: the workspace
    /// tree, or a folder row of a focused commit-file list. File rows and
    /// other panes pan.
    pub(crate) fn hl_folds(&self) -> bool {
        // Tree: open / close a folder. Preview: `h` goes back to the tree.
        if self.is_explorer_tab() {
            return true;
        }
        match self.list_focus_target() {
            ListFocusTarget::Tree => true,
            ListFocusTarget::CommitFiles => self
                .focused_commit_file_row()
                .is_some_and(|row| row.foldable),
            _ => false,
        }
    }

    /// Which list (or diff) the focused pane is driving.
    ///
    /// Depth 0 left is the workspace tree; depth 1 left is the graph; depth 2
    /// left is the commit-file list. Right at depth 2 (and depth 0 file diffs)
    /// is `None` so `j`/`k` move the focused file-diff row. The viewport keeps
    /// that row near the vertical middle. An Explorer tab is `None` while
    /// the right pane drives its active inner file, else `Tree` (its own
    /// tree, never the parked Workspace one).
    pub(crate) fn list_focus_target(&self) -> ListFocusTarget {
        // The PRs tab list is none of the Workspace lists.
        if self.is_prs_tab() {
            return ListFocusTarget::None;
        }
        if let Some(tab) = self.tabs.active_explorer() {
            return if self.focus == FocusPane::Right && tab.active_inner().is_some() {
                ListFocusTarget::None
            } else {
                ListFocusTarget::Tree
            };
        }
        if self.is_compare_tab() {
            return if self.focus == FocusPane::Left {
                ListFocusTarget::CommitFiles
            } else {
                ListFocusTarget::None
            };
        }
        match &self.drill {
            DrillView::Graph => {
                if self.focus == FocusPane::Left {
                    ListFocusTarget::Tree
                } else if self.right_is_diff() {
                    ListFocusTarget::None
                } else {
                    ListFocusTarget::Graph
                }
            }
            DrillView::Files { .. } => {
                if self.focus == FocusPane::Left {
                    ListFocusTarget::Graph
                } else {
                    ListFocusTarget::CommitFiles
                }
            }
            DrillView::Diff { .. } => {
                if self.focus == FocusPane::Left {
                    ListFocusTarget::CommitFiles
                } else {
                    ListFocusTarget::None
                }
            }
        }
    }

    pub(crate) fn commit_files_list_focused(&self) -> bool {
        self.list_focus_target() == ListFocusTarget::CommitFiles
    }

    pub fn graph_pane_focused(&self) -> bool {
        self.list_focus_target() == ListFocusTarget::Graph
    }

    /// Commit-file list for the active compare tab or graph drill.
    pub(crate) fn commit_drill_files(&self) -> Option<&[CommitFile]> {
        if let Some(tab) = self.tabs.active_compare() {
            return Some(tab.files.as_slice());
        }
        match &self.drill {
            DrillView::Files { files, .. } | DrillView::Diff { files, .. } => Some(files),
            DrillView::Graph => None,
        }
    }

    /// Checkout and source behind the commit-file list, if one is shown.
    ///
    /// A compare tab answers from its own checkout and range, never the
    /// parked Workspace drill. `None` while the compare range is loading.
    pub(crate) fn commit_drill_source(&self) -> Option<(&str, &CommitFileSource)> {
        if let Some(tab) = self.tabs.active_compare() {
            return Some((tab.checkout_path.as_str(), tab.source.as_ref()?));
        }
        match &self.drill {
            DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. } => {
                Some((repo.as_str(), source))
            }
            DrillView::Graph => None,
        }
    }

    /// Checkout, file path, and source of the diff in the right pane.
    ///
    /// A compare tab answers from its own checkout, open file, and range,
    /// never the parked Workspace diff; `None` until both are loaded. A
    /// Workspace tree file diff has source `None` (the worktree).
    pub(crate) fn open_diff_target(&self) -> Option<(&str, &str, Option<&CommitFileSource>)> {
        if self.is_explorer_tab() {
            return self
                .explorer_diff()
                .map(|(checkout, rel, _)| (checkout, rel, None));
        }
        if let Some(tab) = self.tabs.active_compare() {
            let source = tab.source.as_ref()?;
            return Some((
                tab.checkout_path.as_str(),
                tab.path.as_deref()?,
                Some(source),
            ));
        }
        match &self.drill {
            DrillView::Diff {
                repo, path, source, ..
            } => Some((repo.as_str(), path.as_str(), Some(source))),
            _ => Some((self.diff_repo.as_deref()?, self.diff_path.as_deref()?, None)),
        }
    }

    /// Checkout, file path, and source of the line viewer in the right
    /// pane: the active Explorer clean-file view (the worktree, source
    /// `None`), else [`Self::open_diff_target`]. The line comments its
    /// gutter marks are keyed to this scope.
    pub(crate) fn line_viewer_target(&self) -> Option<(&str, &str, Option<&CommitFileSource>)> {
        if let (Some(tab), Some(file)) = (self.tabs.active_explorer(), self.tabs.active_file_view())
        {
            return Some((tab.checkout.as_str(), file.rel.as_str(), None));
        }
        self.open_diff_target()
    }

    /// Comment state of diff `line` in the right pane: `None` when no line
    /// comment covers it, `Some(true)` when every covering one is resolved.
    ///
    /// Only comments keyed to [`Self::open_diff_target`] count, so a compare
    /// diff never paints Workspace worktree comments.
    pub(crate) fn diff_line_comment(&self, line: u32) -> Option<bool> {
        let (repo, path, source) = self.open_diff_target()?;
        let snap = self.snapshot.repos.iter().find(|r| r.repo == repo);
        diff_line_comment_state(
            &self.comment_store,
            repo,
            snap.and_then(|r| r.primary_repo.as_deref()),
            snap.map(|r| r.branch.as_str()),
            path,
            source,
            line,
        )
    }

    /// Comment state of 1-based `line` of the active Explorer File view,
    /// as [`Self::diff_line_comment`] reads a diff line: the working-tree
    /// line comments of that checkout's branch and file.
    pub(crate) fn file_line_comment(&self, line: u32) -> Option<bool> {
        let tab = self.tabs.active_explorer()?;
        let file = self.tabs.active_file_view()?;
        let snap = self.snapshot.repos.iter().find(|r| r.repo == tab.checkout);
        diff_line_comment_state(
            &self.comment_store,
            &tab.checkout,
            snap.and_then(|r| r.primary_repo.as_deref()),
            snap.map(|r| r.branch.as_str()),
            &file.rel,
            None,
            line,
        )
    }

    fn files_list_origin_x(&self) -> u16 {
        if self.is_compare_tab() || self.drill.is_diff() {
            self.layout.tree_x
        } else {
            self.layout.diff_content_x
        }
    }

    fn set_commit_file_cursor(&mut self, idx: usize) {
        if let Some(tab) = self.tabs.active_compare_mut() {
            tab.file_cursor = idx;
            return;
        }
        match &mut self.drill {
            DrillView::Files { cursor, .. } => *cursor = idx,
            DrillView::Diff { file_cursor, .. } => *file_cursor = idx,
            DrillView::Graph => {}
        }
    }

    /// Graph-row source for the right pane at depth ≥ 1.
    pub(crate) fn follow_commit_source(&self) -> Option<(String, CommitFileSource)> {
        let repo = self.focused_graph_repo()?;
        let source = source_from_graph_row(&self.focused_graph_row()?)?;
        Some((repo, source))
    }

    /// Load commit files for the focused graph row (depth 1 left).
    ///
    /// Depth 0 right is the graph itself — no follow. Same source is a no-op.
    fn follow_graph_files(&self) -> Effect {
        let DrillView::Files { repo, source, .. } = &self.drill else {
            return Effect::None;
        };
        match self.follow_commit_source() {
            Some((next_repo, next_source)) if next_repo == *repo && next_source == *source => {
                Effect::None
            }
            Some(_) => Effect::LoadRightPane,
            None => Effect::None,
        }
    }

    /// Load the focused file's commit diff through `load_right`.
    ///
    /// A folder row loads nothing: the right pane shows its
    /// [`Self::folder_summary`] and keeps the last diff loaded behind it.
    /// The already-shown path loads nothing either, so a file → folder →
    /// same file round trip repaints the kept diff.
    fn maybe_load_focused_commit_diff(&self) -> Effect {
        if let Some(tab) = self.tabs.active_compare() {
            let Some(row) = self.focused_commit_file_row() else {
                return Effect::None;
            };
            if !row.is_file() {
                return Effect::None;
            }
            if tab.path.as_deref() == Some(row.path.as_str()) {
                return Effect::None;
            }
            let Some(source) = tab.source.clone() else {
                return Effect::None;
            };
            return Effect::LoadCompareDiff {
                tab_id: tab.id,
                repo: tab.checkout_path.clone(),
                source,
                path: row.path.clone(),
                old_path: row.file.as_ref().and_then(|file| file.old_path.clone()),
            };
        }
        let DrillView::Diff { path, .. } = &self.drill else {
            return Effect::None;
        };
        let Some(row) = self.focused_commit_file_row() else {
            return Effect::None;
        };
        if !row.is_file() || row.path == *path {
            return Effect::None;
        }
        Effect::LoadRightPane
    }

    /// Header / footer / list split for the graph pane.
    ///
    /// The pane width is [`LayoutHit::graph_width`] of the last paint.
    pub fn graph_chrome(&self) -> GraphChromeBudget {
        self.graph_chrome_in(self.layout.tree_height.max(1), self.layout.graph_width)
    }

    /// Header / footer / list split for a graph pane of `height` rows and
    /// `width` columns.
    ///
    /// The footer request is fixed ([`graph_footer_request`], same call as
    /// the widget); the selected row's meta line, wrapped at `width`, adds
    /// its extra rows on top ([`graph_chrome_budget_for_meta`]). So the
    /// split follows the expand state, the rows, the pane size, and the
    /// selected row's meta row count, never the selected message's length.
    /// `width` 0 (no paint yet) counts one meta row.
    pub(crate) fn graph_chrome_in(&self, height: u16, width: u16) -> GraphChromeBudget {
        let meta_rows = if width == 0 {
            1
        } else {
            // The meta line does not depend on the expand state, so the
            // collapsed footer gives it without wrapping the whole message.
            let width = usize::from(width);
            self.graph_selection_footer_in(width, false)
                .meta_rows(width)
        };
        graph_chrome_budget_for_meta(
            height,
            self.graph.as_ref().is_some_and(|g| g.sync.is_some()),
            graph_footer_request(self.commit_msg_expand, self.commit_msg_lines),
            u16::try_from(meta_rows).unwrap_or(u16::MAX),
        )
    }

    /// Graph selection-footer content of the cursor row at `width`, in the
    /// current expand state: the same [`selection_footer`] the widget
    /// paints. No graph gives the empty footer.
    pub(crate) fn graph_selection_footer(&self, width: usize) -> SelectionFooter {
        self.graph_selection_footer_in(width, self.commit_msg_expand)
    }

    /// [`Self::graph_selection_footer`] in the `expand` state. Clones only
    /// the cursor row.
    fn graph_selection_footer_in(&self, width: usize, expand: bool) -> SelectionFooter {
        let Some(model) = self.graph.as_ref() else {
            return SelectionFooter::default();
        };
        let selected = model.visible_row_at(self.graph_cursor);
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        selection_footer(
            model,
            GraphFooterSelection::from(selected.as_ref()),
            glyphs,
            width.max(1),
            unix_now(),
            expand,
        )
    }

    /// Message lines of the graph selection footer below the shown ones:
    /// the count its `more lines below` hint paints. `0` when the footer is
    /// not painted or every line shows.
    pub(crate) fn graph_footer_lines_below(&self) -> usize {
        let layout = &self.layout;
        if layout.graph_footer_y.is_none() {
            return 0;
        }
        // Message rows: the footer less its rule and the pinned meta rows.
        let rows = usize::from(layout.graph_footer_message_rows);
        let messages = self
            .graph_selection_footer(usize::from(layout.graph_footer_width))
            .message
            .len();
        if rows == 0 || messages <= rows {
            return 0;
        }
        let offset = self
            .graph_footer_msg_scroll()
            .min(layout.graph_footer_scroll_max);
        messages.saturating_sub(offset + rows)
    }

    fn graph_selected_row_identity(&self) -> Option<String> {
        let repo = self.graph_focus_repo()?;
        let rows = self.graph.as_ref()?.visible_rows();
        rows.get(self.graph_cursor)
            .map(|row| graph_row_identity(&repo, row))
    }

    /// First painted footer message line for the selected graph row.
    pub(crate) fn graph_footer_msg_scroll(&self) -> usize {
        match (&self.commit_msg_scroll, self.graph_selected_row_identity()) {
            (Some((id, offset)), Some(current)) if *id == current => *offset,
            _ => 0,
        }
    }

    /// Wheel over an overflowing graph footer scrolls its message.
    ///
    /// Returns false when `(col, row)` is not on that footer, so the wheel
    /// moves the list as usual.
    pub(crate) fn scroll_graph_footer_msg(&mut self, col: u16, row: u16, delta: i32) -> bool {
        let layout = &self.layout;
        let on_footer = on_footer_box(
            (col, row),
            layout.graph_footer_y,
            layout.graph_footer_x,
            layout.graph_footer_width,
            layout.graph_footer_height,
        );
        if !on_footer {
            return false;
        }
        let Some(id) = self.graph_selected_row_identity() else {
            return false;
        };
        let max = self.layout.graph_footer_scroll_max;
        self.step_commit_msg_scroll(id, self.graph_footer_msg_scroll(), max, delta);
        true
    }

    /// Store `current + delta`, clamped to `0..=max`, as the message scroll
    /// of identity `id`.
    fn step_commit_msg_scroll(&mut self, id: String, current: usize, max: usize, delta: i32) {
        let current = current.min(max) as i64;
        let next = (current + i64::from(delta)).clamp(0, max as i64);
        self.commit_msg_scroll = Some((id, next as usize));
    }

    /// Graph row identity of the commit or stash the commit-files footer
    /// shows (a depth-2 drill or a Diff commit vs parent tab,
    /// [`Self::commit_footer_source`]): the same key
    /// [`Self::graph_footer_msg_scroll`] uses for that row, so one commit
    /// keeps one offset on every footer. `None` for a range compare tab and
    /// for sources with no message (worktree, compare, commit vs worktree).
    pub(crate) fn commit_detail_msg_identity(&self) -> Option<String> {
        let (repo, source) = self.commit_footer_source()?;
        match source.as_ref() {
            CommitFileSource::Commit { commit_id } => Some(format!("{repo}#commit:{commit_id}")),
            CommitFileSource::Stash { stash_ref } => Some(format!("{repo}#stash:{stash_ref}")),
            CommitFileSource::Worktree
            | CommitFileSource::Compare { .. }
            | CommitFileSource::CommitVsWorktree { .. } => None,
        }
    }

    /// First painted message line of the depth-2 commit-files footer.
    ///
    /// Shares [`Self::commit_msg_scroll`] with the graph selection footer:
    /// another commit reads `0`.
    pub(crate) fn commit_detail_msg_scroll(&self) -> usize {
        self.commit_msg_scroll_of(self.commit_detail_msg_identity().as_deref())
    }

    /// Stored message scroll of the footer showing graph row identity
    /// `identity` ([`Self::commit_msg_scroll`]); `0` for another identity
    /// or none.
    pub(crate) fn commit_msg_scroll_of(&self, identity: Option<&str>) -> usize {
        match (&self.commit_msg_scroll, identity) {
            (Some((id, offset)), Some(current)) if id == current => *offset,
            _ => 0,
        }
    }

    /// Wheel over an overflowing depth-2 commit-files footer scrolls its
    /// message.
    ///
    /// Returns false when `(col, row)` is not on that footer, so the wheel
    /// moves the file list as usual. The file cursor, focus, and diff stay.
    pub(crate) fn scroll_commit_detail_msg(&mut self, col: u16, row: u16, delta: i32) -> bool {
        let layout = &self.layout;
        let on_footer = on_footer_box(
            (col, row),
            layout.commit_detail_footer_y,
            layout.commit_detail_footer_x,
            layout.commit_detail_footer_width,
            layout.commit_detail_footer_height,
        );
        if !on_footer {
            return false;
        }
        let Some(id) = self.commit_detail_msg_identity() else {
            return false;
        };
        let max = self.layout.commit_detail_footer_scroll_max;
        self.step_commit_msg_scroll(id, self.commit_detail_msg_scroll(), max, delta);
        true
    }

    pub fn commit_file_rows(&self) -> Vec<CommitFileRow> {
        let files = if let Some(tab) = self.tabs.active_compare() {
            tab.files.as_slice()
        } else {
            match &self.drill {
                DrillView::Files { files, .. } | DrillView::Diff { files, .. } => files.as_slice(),
                DrillView::Graph => return Vec::new(),
            }
        };
        flatten_commit_files(
            files,
            self.commit_tree_mode,
            &self.commit_file_folds,
            self.ascii,
        )
    }

    pub(crate) fn commit_files_cursor(&self) -> usize {
        if let Some(tab) = self.tabs.active_compare() {
            return tab.file_cursor;
        }
        match &self.drill {
            DrillView::Files { cursor, .. } => *cursor,
            DrillView::Diff { file_cursor, .. } => *file_cursor,
            DrillView::Graph => 0,
        }
    }

    /// Highlighted commit-file row when the files list is open.
    pub(crate) fn focused_commit_file_row(&self) -> Option<CommitFileRow> {
        if self.drill.is_graph() && !self.is_compare_tab() {
            return None;
        }
        let rows = self.commit_file_rows();
        rows.get(self.commit_files_cursor()).cloned()
    }

    /// Kind of the highlighted commit-file row, when the files list is open.
    pub(crate) fn focused_commit_file_kind(
        &self,
    ) -> Option<super::commit_files::CommitFileRowKind> {
        self.focused_commit_file_row().map(|row| row.kind)
    }

    /// Folder summary the right pane shows in place of a file diff.
    ///
    /// `Some` only while a file list sits next to a diff (a compare tab or
    /// a depth-2 drill) and the focused list row is a folder. Derived from
    /// the cursor, never stored: the previous file's diff stays loaded, so
    /// moving back onto that file repaints it with no reload. Built from
    /// the full file list, so files under folded subfolders count.
    pub(crate) fn folder_summary(&self) -> Option<FolderSummary> {
        let dir = self.folder_summary_dir()?;
        Some(folder_summary(self.commit_drill_files()?, &dir))
    }

    /// Folder path of [`Self::folder_summary`], without building the
    /// summary.
    fn folder_summary_dir(&self) -> Option<String> {
        if self.is_explorer_tab() || (!self.is_compare_tab() && !self.drill.is_diff()) {
            return None;
        }
        let row = self.focused_commit_file_row().filter(|row| row.is_dir())?;
        Some(row.path)
    }

    /// True when `row` is a compare-tab file row marked reviewed.
    pub(crate) fn compare_file_reviewed(&self, row: &CommitFileRow) -> bool {
        row.is_file()
            && self
                .tabs
                .active_compare()
                .is_some_and(|tab| tab.is_reviewed(&row.path))
    }

    fn focused_compare_old_path(&self) -> Option<String> {
        let tab = self.tabs.active_compare()?;
        let path = tab.path.as_deref()?;
        tab.files
            .iter()
            .find(|file| file.path == path)
            .and_then(|file| file.old_path.clone())
    }

    pub(crate) fn focused_commit_edit_path(&self) -> Option<(String, String)> {
        if let Some(tab) = self.tabs.active_compare() {
            let _ = tab.source.as_ref()?;
            if self.commit_files_list_focused() {
                let row = self.focused_commit_file_row()?;
                if !row.is_file() {
                    return None;
                }
                return Some((tab.checkout_path.clone(), row.path));
            }
            if self.folder_summary().is_some() {
                return None;
            }
            return tab
                .path
                .as_ref()
                .map(|path| (tab.checkout_path.clone(), path.clone()));
        }
        let repo = match &self.drill {
            DrillView::Files { repo, .. } | DrillView::Diff { repo, .. } => repo.clone(),
            DrillView::Graph => return None,
        };
        if self.commit_files_list_focused() {
            let row = self.focused_commit_file_row()?;
            if !row.is_file() {
                return None;
            }
            return Some((repo, row.path));
        }
        if self.folder_summary().is_some() {
            return None;
        }
        match &self.drill {
            DrillView::Diff { path, .. } => Some((repo, path.clone())),
            _ => None,
        }
    }

    /// LEFT / RIGHT pair for `E`, or `None` for a commit-vs-working-tree
    /// diff: neither kind is a commit against the working tree.
    fn external_diff_kind(&self) -> Option<ExternalDiffKind> {
        if let Some(tab) = self.tabs.active_compare() {
            return match &tab.source {
                Some(CommitFileSource::Compare {
                    merge_base, head, ..
                }) => Some(ExternalDiffKind::Rev {
                    left_rev: merge_base.clone(),
                    right_rev: head.clone(),
                    left_path: self.focused_compare_old_path(),
                }),
                Some(CommitFileSource::CommitVsWorktree { .. }) => None,
                _ => Some(ExternalDiffKind::Worktree),
            };
        }
        let source = match &self.drill {
            DrillView::Files { source, .. } | DrillView::Diff { source, .. } => source,
            DrillView::Graph => return Some(ExternalDiffKind::Worktree),
        };
        Some(match source {
            CommitFileSource::Worktree => ExternalDiffKind::Worktree,
            CommitFileSource::CommitVsWorktree { .. } => return None,
            CommitFileSource::Commit { commit_id } => ExternalDiffKind::Rev {
                left_rev: format!("{commit_id}^"),
                right_rev: commit_id.clone(),
                left_path: None,
            },
            CommitFileSource::Stash { stash_ref } => ExternalDiffKind::Rev {
                left_rev: format!("{stash_ref}^1"),
                right_rev: stash_ref.clone(),
                left_path: None,
            },
            CommitFileSource::Compare {
                merge_base, head, ..
            } => ExternalDiffKind::Rev {
                left_rev: merge_base.clone(),
                right_rev: head.clone(),
                left_path: self.focused_compare_old_path(),
            },
        })
    }

    /// Checkout and source of the commit-files footer: the drilled source
    /// of a Workspace drill, or the head commit of a Diff commit vs parent
    /// tab ([`super::tabs::CompareTab::commit_vs_parent`], a commit source). `None`
    /// for the graph view and for any other compare tab: a range has no
    /// single commit message.
    pub(crate) fn commit_footer_source(&self) -> Option<(&str, Cow<'_, CommitFileSource>)> {
        if let Some(tab) = self.tabs.active_compare() {
            let commit_id = tab.commit_vs_parent()?.to_string();
            return Some((
                tab.checkout_path.as_str(),
                Cow::Owned(CommitFileSource::Commit { commit_id }),
            ));
        }
        match &self.drill {
            DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. } => {
                Some((repo.as_str(), Cow::Borrowed(source)))
            }
            DrillView::Graph => None,
        }
    }

    /// Full id of the one commit the active compare tab shows against its
    /// first parent (Diff commit vs parent), whose footer pins under the
    /// tab's file list. `None` on another tab or a range compare tab.
    pub(crate) fn active_compare_commit(&self) -> Option<&str> {
        self.tabs.active_compare()?.commit_vs_parent()
    }

    /// True on a compare tab of a range (not Diff commit vs parent): it has
    /// no single commit message, so its file list takes the whole left
    /// pane with no commit footer. Paint and layout both ask here.
    pub(crate) fn is_range_compare_tab(&self) -> bool {
        self.is_compare_tab() && self.active_compare_commit().is_none()
    }

    /// Graph row the commit-files footer describes: the commit, stash, or
    /// uncommitted row a depth-2 drill opened, or the commit of a Diff
    /// commit vs parent tab ([`Self::commit_footer_source`]).
    ///
    /// First choice is that row cloned from the loaded graph (only that
    /// row, [`Self::commit_footer_graph`]), so chips match the graph
    /// footer. A Diff commit vs parent tab whose commit has no graph row
    /// (an old commit a blame action opened, another checkout's graph, or
    /// none loaded) builds the row from the commit its range load read
    /// ([`super::tabs::CompareTab::commit`]). `None` for a range compare
    /// tab, a source with no graph row (compare, commit vs worktree), or a
    /// row found nowhere ([`Self::commit_detail_fallback_name`] then names
    /// it). No git call: the draw path reads only loaded data.
    pub(crate) fn commit_detail_graph_row(&self) -> Option<GraphRow> {
        let (repo, source) = self.commit_footer_source()?;
        let graph_row =
            self.commit_footer_graph(repo)
                .and_then(|model| match source.as_ref() {
                    CommitFileSource::Commit { commit_id } => model.commit_row(commit_id),
                    CommitFileSource::Stash { stash_ref } => model.stash_row(stash_ref),
                    CommitFileSource::Worktree => model
                        .uncommitted
                        .map(|has_changes| GraphRow::Uncommitted { has_changes }),
                    CommitFileSource::Compare { .. }
                    | CommitFileSource::CommitVsWorktree { .. } => None,
                });
        if graph_row.is_some() {
            return graph_row;
        }
        let tab = self.tabs.active_compare()?;
        // Only the commit the source names: a stale load never stands in.
        let commit = tab.commit.clone().filter(|commit| {
            matches!(source.as_ref(), CommitFileSource::Commit { commit_id } if *commit_id == commit.id)
        })?;
        let is_head = self
            .checkout_head_and_default(&tab.checkout_path)
            .is_some_and(|(head, _)| head == commit.id);
        Some(GraphRow::Commit {
            commit,
            is_head,
            worktrees: Vec::new(),
        })
    }

    /// The loaded graph when it serves the commit-files footer of checkout
    /// `repo`: always on a Workspace drill (the drill opened from it), and
    /// on a compare tab only while it is that checkout's graph
    /// ([`Self::graph_identity`]).
    fn commit_footer_graph(&self, repo: &str) -> Option<&GraphModel> {
        let model = self.graph.as_ref()?;
        let serves = !self.is_compare_tab()
            || self
                .graph_identity
                .as_ref()
                .is_some_and(|(graph_repo, _)| graph_repo == repo);
        serves.then_some(model)
    }

    /// Graph model the commit-files footer builds its lines against: chip
    /// colours (default branch), the checkout mark (sync branch), and HEAD.
    /// The loaded graph when it serves the footer's checkout
    /// ([`Self::commit_footer_graph`]); otherwise a model with only that
    /// checkout's HEAD, sync state, and default-branch override from the
    /// snapshot, so a commit from the tab's own load colours its chips for
    /// its own checkout.
    pub(crate) fn commit_footer_model(&self) -> Cow<'_, GraphModel> {
        let Some((repo, _)) = self.commit_footer_source() else {
            return self
                .graph
                .as_ref()
                .map_or_else(|| Cow::Owned(GraphModel::default()), Cow::Borrowed);
        };
        if let Some(model) = self.commit_footer_graph(repo) {
            return Cow::Borrowed(model);
        }
        let row = self.snapshot.repos.iter().find(|row| row.repo == repo);
        Cow::Owned(GraphModel {
            head_id: row
                .map(|row| row.head.clone())
                .filter(|head| !head.is_empty()),
            sync: row.map(super::graph_load::sync_from_snapshot),
            default_branch_override: row.and_then(|row| row.default_branch_override.clone()),
            ..GraphModel::default()
        })
    }

    /// Name of the footer's source when [`Self::commit_detail_graph_row`]
    /// finds no row: the short hash, the `stash@{n}`, `uncommitted`, or
    /// the range label ([`CommitFileSource::short_label`]). `None` for a
    /// range compare tab.
    pub(crate) fn commit_detail_fallback_name(&self) -> Option<String> {
        self.commit_footer_source()
            .map(|(_, source)| source.short_label())
    }

    /// Body content of a commit-message footer for graph row `row` at
    /// `width`: [`selection_footer`], what the graph selection footer
    /// paints for that row (message lines, and the whole meta line that
    /// paint wraps onto the bottom rows), in the current expand state.
    ///
    /// With no row, a `fallback` name (the drilled source, for example a
    /// commit a watch reload dropped from the graph) gives a muted
    /// [`COMMIT_FOOTER_NOT_IN_GRAPH`] line and a meta line with that name,
    /// so the footer still names what the file list shows. No row and no
    /// name gives the graph's no-selection footer.
    ///
    /// Every commit-message footer builds its lines here, so the same
    /// commit gives the same rows on every surface. The row is input, not
    /// read from the drill, so a footer for a commit outside a Workspace
    /// drill can call it too.
    pub(crate) fn commit_footer_parts(
        &self,
        row: Option<&GraphRow>,
        fallback: Option<&str>,
        width: usize,
    ) -> SelectionFooter {
        if let (None, Some(name)) = (row, fallback) {
            let meta = |text: &str| {
                vec![LabelPart {
                    text: text.to_string(),
                    kind: LabelKind::Meta,
                    target: None,
                }]
            };
            return SelectionFooter {
                message: vec![meta(COMMIT_FOOTER_NOT_IN_GRAPH)],
                meta: meta(name),
            };
        }
        let model = self.commit_footer_model();
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        selection_footer(
            &model,
            GraphFooterSelection::from(row),
            glyphs,
            width.max(1),
            unix_now(),
            self.commit_msg_expand,
        )
    }

    /// Rows the commit-files footer body asks for, before the pane-size cap.
    ///
    /// The graph selection footer's request ([`graph_footer_request`]) less
    /// its rule row: expanded is [`Self::commit_msg_lines`] message rows
    /// plus one meta row, collapsed is the subject and one meta row. Fixed
    /// for the expand state and the rows, never sized by the selected
    /// message: the footer content ([`Self::commit_footer_parts`]) holds
    /// only the real message lines, so a short message or a source with no
    /// message leaves blank rows and a long one scrolls. A meta line that
    /// wraps adds its extra rows on top
    /// ([`Self::commit_detail_footer_meta_rows`]).
    pub(crate) fn commit_detail_footer_request(&self) -> usize {
        usize::from(
            graph_footer_request(self.commit_msg_expand, self.commit_msg_lines)
                .saturating_sub(FOOTER_RULE_ROWS),
        )
    }

    /// Rows the depth-2 commit-files footer's meta line wraps to at
    /// `width` (at least 1): [`SelectionFooter::meta_rows`] of
    /// [`Self::commit_footer_parts`] for the drilled row. The layout pass
    /// and the paint both size the footer with it, so the file list rows
    /// and the click mapping agree.
    pub(crate) fn commit_detail_footer_meta_rows(&self, width: usize) -> usize {
        let fallback = self.commit_detail_fallback_name();
        self.commit_footer_parts(
            self.commit_detail_graph_row().as_ref(),
            fallback.as_deref(),
            width,
        )
        .meta_rows(width)
    }

    /// Text of the depth-2 commit-files footer lines at `width`.
    ///
    /// The footer sits at the bottom of the left commit-files pane beside a
    /// file diff, under the file list. The depth-1 files pane has no footer
    /// (the graph pane beside it already shows one). The lines are the
    /// graph selection footer's for the drilled row
    /// ([`Self::commit_detail_graph_row`], [`Self::commit_footer_parts`]):
    /// collapsed, the subject and the meta rows; expanded, the whole
    /// subject plus body wrapped one column short of `width` (the message
    /// scrollbar column) with no line cap, then the meta rows (the meta
    /// line wrapped at `width`). Paint pins the meta rows at the footer's
    /// bottom under the message rows, like the graph footer. No blank
    /// padding: the footer keeps its fixed
    /// [`Self::commit_detail_footer_request`] height in paint, where unused
    /// message rows stay blank and a message taller than the rows scrolls
    /// ([`Self::commit_detail_msg_scroll`]). Tests read the footer text
    /// here.
    #[cfg(test)]
    pub(crate) fn commit_detail_footer_lines(&self, width: usize) -> Vec<String> {
        let fallback = self.commit_detail_fallback_name();
        self.commit_footer_parts(
            self.commit_detail_graph_row().as_ref(),
            fallback.as_deref(),
            width,
        )
        .lines(width.max(1))
        .iter()
        .map(|parts| parts.iter().map(|part| part.text.as_str()).collect())
        .collect()
    }

    /// Message lines of the depth-2 commit-files footer below the shown
    /// ones: the count its more-below hint paints. `0` when the footer did
    /// not record an overflowing message this frame or every line shows.
    pub(crate) fn commit_detail_footer_lines_below(&self) -> usize {
        let layout = &self.layout;
        if layout.commit_detail_footer_y.is_none() {
            return 0;
        }
        // Message rows: the footer less its rule and the pinned meta rows.
        let rows = usize::from(layout.commit_detail_footer_message_rows);
        let fallback = self.commit_detail_fallback_name();
        let messages = self
            .commit_footer_parts(
                self.commit_detail_graph_row().as_ref(),
                fallback.as_deref(),
                usize::from(layout.commit_detail_footer_width),
            )
            .message
            .len();
        if rows == 0 || messages <= rows {
            return 0;
        }
        let offset = self
            .commit_detail_msg_scroll()
            .min(layout.commit_detail_footer_scroll_max);
        messages.saturating_sub(offset + rows)
    }

    fn restore_commit_file_cursor(&mut self, path: Option<&str>) {
        let rows = self.commit_file_rows();
        self.set_commit_file_cursor(commit_file_cursor_index(&rows, path));
    }

    /// Same-source commit-file reload: keep `path`, else clamp to
    /// `min(old_index, n-1)`.
    fn restore_commit_file_cursor_retain(&mut self, path: &str, previous: usize) {
        let rows = self.commit_file_rows();
        if rows.is_empty() {
            self.set_commit_file_cursor(0);
            return;
        }
        if let Some(idx) = rows
            .iter()
            .position(|row| row.is_file() && row.path == path)
            .or_else(|| rows.iter().position(|row| row.path == path))
        {
            self.set_commit_file_cursor(idx);
            return;
        }
        self.set_commit_file_cursor(previous.min(rows.len() - 1));
    }

    fn visible_tree(&self) -> TreeNode {
        let visible = visible_snapshot(&self.snapshot, self.show_hidden);
        build_tree(
            &visible,
            self.tree_mode,
            &workspace_label_from_cwd(&self.cwd),
        )
    }

    pub fn rebuild_rows(&mut self) {
        let focus_id = self.focused_row().map(|r| r.id.clone());
        self.tree = self.visible_tree();
        self.rows = flatten_with(&self.tree, &self.folds, self.ascii);
        self.restore_focus_after_tree_rebuild(focus_id);
    }

    fn toggle_tree_mode(&mut self) -> Effect {
        if self.in_commit_drill() {
            return self.toggle_commit_tree_mode();
        }
        let previous_id = self.focused_row().map(|row| row.id.clone());
        self.tree_mode = !self.tree_mode;
        self.tree = self.visible_tree();
        self.folds = default_folds(&self.tree);
        self.rows = flatten_with(&self.tree, &self.folds, self.ascii);
        self.restore_focus_after_tree_rebuild(previous_id);
        self.signatures = tree_signatures(&self.tree, &self.file_facts);
        self.tree_ghosts.clear();
        self.status = if self.tree_mode {
            "Directory tree".into()
        } else {
            "Flat paths".into()
        };
        Effect::LoadRightPane
    }

    fn toggle_commit_tree_mode(&mut self) -> Effect {
        let path = self.focused_commit_file_row().map(|row| row.path);
        self.commit_tree_mode = !self.commit_tree_mode;
        self.commit_file_folds.clear();
        if self.is_compare_tab() || !self.drill.is_graph() {
            self.restore_commit_file_cursor(path.as_deref());
        }
        self.status = if self.commit_tree_mode {
            "Directory tree".into()
        } else {
            "Flat paths".into()
        };
        self.reseed_commit_file_signatures();
        // Tree → flat from a folder lands on a file: load its diff.
        self.maybe_load_focused_commit_diff()
    }

    fn restore_focus_after_tree_rebuild(&mut self, previous_id: Option<String>) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        if let Some(id) = previous_id {
            if let Some(idx) = self.rows.iter().position(|row| row.id == id) {
                self.cursor = idx;
                return;
            }
            for ancestor in focus_ancestor_ids(&id) {
                if let Some(idx) = self.rows.iter().position(|row| row.id == ancestor) {
                    self.cursor = idx;
                    return;
                }
            }
        }
        self.cursor = self.cursor.min(self.rows.len() - 1);
    }

    /// Replace the snapshot. File disk tokens and viewed fingerprints come
    /// from [`Self::set_file_facts`]; this reads no file.
    pub fn apply_snapshot(&mut self, snapshot: WorkspaceSnapshot) {
        let snapshot = carry_status_failed_local_branches(&self.snapshot, snapshot);
        self.snapshot = snapshot;
        self.snapshot.show_hidden = self.show_hidden;
        self.rebuild_rows();
        self.signatures = tree_signatures(&self.tree, &self.file_facts);
        self.reconcile_viewed_store();
        self.reconcile_comment_store();
        self.sync_live_refresh_outage(Instant::now());
    }

    /// Apply a watch poll. Keeps fold / focus / scroll. Flashes only rows
    /// whose identity actually changed.
    pub fn apply_watch_snapshot(&mut self, snapshot: WorkspaceSnapshot) -> Vec<String> {
        let focus_id = self.focused_row().map(|r| r.id.clone());
        let folds = self.folds.clone();
        let graph_scroll = self.graph_scroll;
        let diff_scroll = self.diff_scroll;
        let diff_cursor = self.diff_cursor;
        let before = self.signatures.clone();
        let old_rows = self.rows.clone();
        self.apply_snapshot(snapshot);
        self.folds = folds;
        self.rebuild_rows();
        self.restore_focus_after_tree_rebuild(focus_id);
        self.graph_scroll = graph_scroll;
        self.diff_scroll = diff_scroll;
        self.diff_cursor = diff_cursor;
        let now = Instant::now();
        self.prune_flash_state(now);
        if is_new_row_set(&before, &self.signatures) {
            self.tree_ghosts.clear();
            return Vec::new();
        }
        self.tree_ghosts.extend(capture_removal_ghosts(
            &old_rows,
            |row| row.id.as_str(),
            |row| row.id.clone(),
            &before,
            &self.signatures,
            now,
        ));
        let kinds = flashable_row_kinds(&before, &self.signatures, true);
        self.stamp_flashes(kinds.iter().cloned(), now);
        kinds.into_iter().map(|(id, _)| id).collect()
    }

    /// The Ctrl-C arm a press at `now` (its arrival) answers.
    ///
    /// The loop timer closes an arm on the wall clock, but a press that
    /// arrived inside the window and was handled late still answers it:
    /// [`handle_ctrl_c`] compares the press's arrival with the window end.
    fn take_ctrl_c_arm(&mut self) -> Option<Instant> {
        let armed = self.ctrl_c_armed_until.or(self.ctrl_c_closed_until);
        self.ctrl_c_closed_until = None;
        armed
    }

    fn ctrl_c(&mut self, now: Instant) -> Effect {
        let armed = self.take_ctrl_c_arm();
        let result = handle_ctrl_c(armed, now);
        self.ctrl_c_armed_until = result.armed_until;
        if result.quit {
            return Effect::Quit;
        }
        if result.prompt {
            self.status = StatusMessage::warn(CTRL_C_EXIT_PROMPT);
        }
        Effect::None
    }

    /// Disarm an expired Ctrl-C window. Clears the prompt only when it is still showing.
    pub fn expire_ctrl_c_prompt(&mut self, now: Instant) -> bool {
        let Some(until) = self.ctrl_c_armed_until else {
            return false;
        };
        if now < until {
            return false;
        }
        self.ctrl_c_closed_until = self.ctrl_c_armed_until.take();
        if is_ctrl_c_exit_prompt(&self.status) {
            self.status.clear();
            true
        } else {
            false
        }
    }

    /// Milliseconds left on the Ctrl-C arm, if one is active.
    pub fn ctrl_c_remaining_ms(&self, now: Instant) -> Option<u64> {
        let until = self.ctrl_c_armed_until?;
        Some(until.saturating_duration_since(now).as_millis() as u64)
    }

    /// `q` while an exclusive write or default-branch switch runs.
    ///
    /// Same chord as Ctrl-C: the first press arms the window and shows
    /// [`QUIT_WHILE_BUSY_PROMPT`]; a second `q` or Ctrl-C inside it quits.
    pub fn quit_while_busy(&mut self, now: Instant) -> Effect {
        let armed = self.take_ctrl_c_arm();
        let result = handle_ctrl_c(armed, now);
        self.ctrl_c_armed_until = result.armed_until;
        if result.quit {
            return Effect::Quit;
        }
        self.status = StatusMessage::warn(QUIT_WHILE_BUSY_PROMPT);
        Effect::None
    }

    /// Put `busy: <op> running` on the status slot for a key that a running
    /// git write drained. `op` is `None` when the running job has no name.
    pub fn note_busy_drop(&mut self, op: Option<&str>) {
        let text = match op {
            Some(op) => format!("busy: {op} running"),
            None => BUSY_WRITE_RUNNING.to_string(),
        };
        self.status = StatusMessage::warn(text);
    }

    /// Milliseconds until an info / ok status clears. Starts its clock the
    /// first time it is visible. `None` while an overlay paints `status`
    /// as its own text, or when the message does not expire. While `?`
    /// help is open the clock restarts, so the message gets its full time
    /// once help closes.
    pub fn status_expiry_ms(&mut self, now: Instant) -> Option<u64> {
        if self.help_open {
            self.status.restart_clock();
            return None;
        }
        if status_uses_status_text(self) {
            return None;
        }
        self.status.expiry_ms(now)
    }

    /// Clear an expired info / ok status. True when the slot changed.
    /// Never while `?` help is open.
    pub fn expire_status(&mut self, now: Instant) -> bool {
        if self.help_open || status_uses_status_text(self) {
            return false;
        }
        self.status.expire(now)
    }

    /// Move a one-shot ack ([`StatusKind::Ok`]) from the status slot to a
    /// toast pushed at `now`, and drop toasts whose time ran out.
    ///
    /// Info, progress, warn, and error stay in the slot. An ok message also
    /// stays while an overlay paints `status` as its own row
    /// (`status_uses_status_text`). `draw` calls this first, so dispatch
    /// and state tests still see the ack in `status`.
    pub fn route_ack_to_toast(&mut self, now: Instant) {
        self.toasts.prune(now);
        if self.status.kind() != StatusKind::Ok || status_uses_status_text(self) {
            return;
        }
        let text = self.status.trim();
        if !text.is_empty() {
            self.toasts.push(text, now);
        }
        self.status.clear();
    }

    fn prune_flash_state(&mut self, now: Instant) {
        prune_flashes(&mut self.flashes, now);
        prune_ghosts(&mut self.tree_ghosts, now);
        prune_ghosts(&mut self.commit_file_ghosts, now);
    }

    /// Drop expired flashes and ghosts before paint / the next tick.
    pub fn prune_expired_flashes(&mut self) {
        self.prune_flash_state(Instant::now());
    }

    /// True when a flash or ghost is still decaying.
    pub fn has_active_flashes(&self) -> bool {
        !self.flashes.is_empty()
            || !self.tree_ghosts.is_empty()
            || !self.commit_file_ghosts.is_empty()
    }

    /// Milliseconds between timer redraws while something animates: a
    /// decaying flash, a spinner or fading toast frame, or a scroll glide painted
    /// last frame, or the one wake a holding toast asked for. `None` when
    /// idle, so no timer redraw runs.
    pub fn next_anim_tick_ms(&self) -> Option<u64> {
        next_tick_ms(AnimNeeds {
            flashes: self.has_active_flashes(),
            frames: self.layout.animating,
            glide: self.layout.gliding,
            wake_ms: self.layout.wake_ms,
        })
    }

    /// Fade colour for a row id, if it is still flashing.
    pub fn flash_color(&self, id: &str) -> Option<Color> {
        let stamp = self.flashes.get(id)?;
        let strength = flash_strength(Instant::now().saturating_duration_since(stamp.at));
        self.theme.palette().flash_bg_for(stamp.kind, strength)
    }

    fn stamp_flashes(
        &mut self,
        stamps: impl IntoIterator<Item = (String, FlashKind)>,
        now: Instant,
    ) {
        for (id, kind) in stamps {
            self.flashes.insert(id, FlashStamp { at: now, kind });
        }
    }

    /// Flash checkout chrome after fetch / pull / push / default-branch.
    pub fn stamp_checkout_flashes(&mut self, repos: &[String]) {
        let now = Instant::now();
        self.prune_flash_state(now);
        for repo in repos {
            self.stamp_flashes(
                checkout_flash_ids(repo)
                    .into_iter()
                    .map(|id| (id, FlashKind::Update)),
                now,
            );
        }
    }

    /// Tree rows including removal ghosts for the flash window.
    pub fn painted_tree_rows(&self) -> Vec<VisibleRow> {
        merge_ghost_rows(&self.rows, &self.tree_ghosts, |row| row.id.as_str())
    }

    /// Commit-file rows including removal ghosts for the flash window.
    pub fn painted_commit_file_rows(&self) -> Vec<CommitFileRow> {
        merge_ghost_rows(&self.commit_file_rows(), &self.commit_file_ghosts, |row| {
            row.id.as_str()
        })
    }

    /// Visible-row indexes plus fade colours for the graph widget.
    pub fn graph_flash_rows(&self) -> Vec<(usize, Color)> {
        let Some(model) = self.graph.as_ref() else {
            return Vec::new();
        };
        let Some((repo, _)) = self.graph_identity.as_ref() else {
            return Vec::new();
        };
        model
            .visible_rows()
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let id = graph_row_identity(repo, row);
                self.flash_color(&id).map(|color| (i, color))
            })
            .collect()
    }

    /// Flash colour for a commit-file row in the current drill.
    pub fn commit_file_flash_color(&self, row_id: &str) -> Option<Color> {
        let (repo, source) = match &self.drill {
            DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. } => {
                (repo, source)
            }
            DrillView::Graph => return None,
        };
        self.flash_color(&commit_file_identity(repo, source, row_id))
    }

    fn unfolded_commit_file_rows(&self, files: &[CommitFile]) -> Vec<CommitFileRow> {
        flatten_commit_files(files, self.commit_tree_mode, &HashSet::new(), self.ascii)
    }

    fn reseed_commit_file_signatures(&mut self) {
        self.commit_file_ghosts.clear();
        let (repo, source, files) = match &self.drill {
            DrillView::Files {
                repo,
                source,
                files,
                ..
            }
            | DrillView::Diff {
                repo,
                source,
                files,
                ..
            } => (repo.clone(), source.clone(), files.clone()),
            DrillView::Graph => {
                self.commit_file_signatures.clear();
                return;
            }
        };
        let unfolded = self.unfolded_commit_file_rows(&files);
        self.commit_file_signatures = commit_file_signatures(&repo, &source, &unfolded);
    }

    /// Enter on a palette `Theme: …` row: apply `id` in this session and
    /// save it as the last-used theme. It ends any palette preview, so no
    /// later close puts the old theme back.
    fn set_theme(&mut self, id: ThemeId) -> Effect {
        self.theme_before_preview = None;
        self.theme = id;
        self.status = format!("theme: {}", self.theme.label()).into();
        if let Err(err) = save_theme(&self.ui_store_path, self.theme) {
            self.status = StatusMessage::error(persist_failed_status("theme", err));
        }
        Effect::None
    }

    /// Palette Toggle activity sparkline: flip the tree bars in this session
    /// and save the new value to the UI-state store at once. A failed save
    /// keeps the flip. Turning it on asks for the counts of every repo, so
    /// the bars appear without a reload.
    fn toggle_activity_sparkline(&mut self) -> Effect {
        let on = !self.activity_sparkline;
        self.activity_sparkline = on;
        self.status = if on {
            "Activity sparkline on".into()
        } else {
            "Activity sparkline off".into()
        };
        if let Err(err) = save_activity_sparkline(&self.ui_store_path, on) {
            self.status = StatusMessage::error(persist_failed_status("sparkline", err));
        }
        if on {
            Effect::LoadActivity {
                checkouts: activity_targets(&self.snapshot, None),
            }
        } else {
            Effect::None
        }
    }

    fn op_effect(&mut self, op: Op) -> Effect {
        if self
            .focused_row()
            .is_some_and(|row| op_is_kind_noop(row.kind, op))
        {
            self.status = StatusMessage::warn(op_kind_noop_reason(op));
            return Effect::None;
        }
        let targets = op_targets(&self.snapshot, self.focused_row(), self.show_hidden, op);
        if targets.is_empty() {
            self.status = StatusMessage::warn("no visible repos for that op");
            return Effect::None;
        }
        match op {
            Op::Fetch => {
                self.status =
                    StatusMessage::progress(format_running_op(RunningOp::Fetch, 0, targets.len()));
                Effect::Fetch { repos: targets }
            }
            Op::Pull => {
                let with_sync = |sync: crate::snapshot::SyncStatus| -> Vec<String> {
                    targets
                        .iter()
                        .filter(|repo| {
                            self.snapshot
                                .repos
                                .iter()
                                .any(|r| r.repo == **repo && r.sync_status == sync)
                        })
                        .cloned()
                        .collect()
                };
                let behind = with_sync(crate::snapshot::SyncStatus::Behind);
                let diverged = with_sync(crate::snapshot::SyncStatus::Diverged);
                if behind.is_empty() {
                    self.pull_nothing_behind(&diverged)
                } else {
                    self.status = StatusMessage::progress(format_running_op(
                        RunningOp::Pull,
                        0,
                        behind.len(),
                    ));
                    Effect::Pull { repos: behind }
                }
            }
            Op::DefaultBranch => {
                let repos = targets
                    .into_iter()
                    .filter(|repo| {
                        self.snapshot.repos.iter().any(|r| {
                            r.repo == *repo
                                && !crate::helpers::is_default_branch(
                                    &r.branch,
                                    r.default_branch_override.as_deref(),
                                )
                        })
                    })
                    .collect::<Vec<_>>();
                if repos.is_empty() {
                    self.status = StatusMessage::warn("no non-default branches to switch");
                    Effect::None
                } else if repos.len() > 1 {
                    // A workspace-wide switch asks first; one repo runs now.
                    self.cancel_mouse_drag();
                    self.confirm = Some(PendingConfirm::SwitchToDefault { repos });
                    Effect::None
                } else {
                    self.status = StatusMessage::progress(format_running_op(
                        RunningOp::DefaultBranch,
                        0,
                        repos.len(),
                    ));
                    Effect::DefaultBranch { repos }
                }
            }
        }
    }

    /// `p` with nothing behind among the targets; `diverged` are the
    /// diverged targets in row order.
    ///
    /// Exactly one target whose upstream was force-pushed with nothing of
    /// yours opens the reset confirm. Several of them warn to focus one.
    /// Otherwise the first one with your commits on top names the rebase.
    /// Otherwise the plain [`diverged_pull_status`]. Only the confirm's `y`
    /// writes.
    fn pull_nothing_behind(&mut self, diverged: &[String]) -> Effect {
        let rewrites: Vec<&WorkspaceRepoSnapshot> = diverged
            .iter()
            .filter_map(|repo| {
                self.snapshot
                    .repos
                    .iter()
                    .find(|r| r.repo == *repo && r.upstream_rewrite.is_some())
            })
            .collect();
        let (nothing_yours, yours): (Vec<&WorkspaceRepoSnapshot>, Vec<&WorkspaceRepoSnapshot>) =
            rewrites
                .into_iter()
                .partition(|r| r.upstream_rewrite.as_ref().is_some_and(|rw| rw.yours == 0));
        match (nothing_yours.as_slice(), yours.first()) {
            ([one], _) => {
                let Some(rewrite) = one.upstream_rewrite.as_ref() else {
                    return Effect::None;
                };
                let pending = PendingConfirm::ResetToUpstream {
                    repo: one.repo.clone(),
                    branch: one.branch.clone(),
                    upstream: rewrite.upstream.clone(),
                    tip: rewrite.tip.clone(),
                    dropped: rewrite.dropped,
                };
                self.help_open = false;
                self.cancel_mouse_drag();
                self.confirm = Some(pending);
            }
            ([first, rest @ ..], _) => {
                self.status = StatusMessage::warn(rewritten_pull_status(&first.repo, rest.len()));
            }
            ([], Some(first)) => {
                if let Some(rewrite) = first.upstream_rewrite.as_ref() {
                    self.status = StatusMessage::warn(rebase_pull_status(
                        &first.repo,
                        &first.branch,
                        rewrite,
                    ));
                }
            }
            ([], None) => self.status = StatusMessage::warn(diverged_pull_status(diverged)),
        }
        Effect::None
    }

    fn refresh_effect(&self) -> Effect {
        let base = match refresh_target(self.focused_row()) {
            None => Effect::ReloadSnapshot,
            Some(repo) => Effect::ReloadRepo { repo },
        };
        if let Some(tab) = self.tabs.active_compare() {
            let compare = Effect::LoadCompareRange {
                tab_id: tab.id,
                repo: tab.checkout_path.clone(),
                base_ref: tab.base_ref.clone(),
                head_ref: tab.head_ref.clone(),
                force: true,
            };
            return Effect::Batch(vec![base, compare]);
        }
        base
    }

    fn move_cursor(&mut self, delta: i32) {
        if self.rows.is_empty() {
            self.cursor = 0;
            return;
        }
        let next = self.cursor as i32 + delta;
        self.cursor = next.clamp(0, self.rows.len() as i32 - 1) as usize;
    }

    fn clear_help_search(&mut self) {
        self.help_search_query = None;
    }

    /// Apply the focused row's current fold state to foldable descendants.
    ///
    /// The first `z` already toggled this row and armed the 400ms window.
    /// The second `z` must not toggle it again. Descendants follow the
    /// focused row: a folded parent folds the subtree; an open parent
    /// opens it.
    fn fold_subtree(&mut self) {
        if self.commit_files_list_focused() {
            let Some(row) = self.focused_commit_file_row() else {
                return;
            };
            let id = row.id.clone();
            let path = row.path.clone();
            let Some(files) = self.commit_drill_files().map(|files| files.to_vec()) else {
                return;
            };
            let ids = collect_commit_subtree_ids(&files, self.commit_tree_mode, &id);
            if ids.is_empty() {
                return;
            }
            let fold_descendants = self.commit_file_folds.contains(&id);
            for sid in ids {
                if sid == id {
                    continue;
                }
                if fold_descendants {
                    self.commit_file_folds.insert(sid);
                } else {
                    self.commit_file_folds.remove(&sid);
                }
            }
            self.restore_commit_file_cursor(Some(&path));
            return;
        }
        if self.list_focus_target() != ListFocusTarget::Tree {
            return;
        }
        let Some(row) = self.focused_row().cloned() else {
            return;
        };
        let ids = collect_foldable_subtree_ids(&self.tree, &row.id);
        if ids.is_empty() {
            return;
        }
        let fold_descendants = self.folds.contains(&row.id);
        for sid in ids {
            if sid == row.id {
                continue;
            }
            if fold_descendants {
                self.folds.insert(sid);
            } else {
                self.folds.remove(&sid);
            }
        }
        self.rebuild_rows();
    }

    fn fold_op(&mut self, op: FoldOp) {
        if self.commit_files_list_focused() {
            self.fold_commit_file(op);
            return;
        }
        if self.list_focus_target() != ListFocusTarget::Tree {
            return;
        }
        let Some(row) = self.focused_row().cloned() else {
            return;
        };
        if !row.foldable {
            return;
        }
        match op {
            FoldOp::Toggle => {
                if !self.folds.remove(&row.id) {
                    self.folds.insert(row.id);
                }
            }
            FoldOp::Close => {
                self.folds.insert(row.id);
            }
            FoldOp::Open => {
                self.folds.remove(&row.id);
            }
        }
        self.rebuild_rows();
    }

    fn fold_commit_file(&mut self, op: FoldOp) {
        let Some(row) = self.focused_commit_file_row() else {
            return;
        };
        if !row.foldable {
            return;
        }
        match op {
            FoldOp::Toggle => {
                if !self.commit_file_folds.remove(&row.id) {
                    self.commit_file_folds.insert(row.id);
                }
            }
            FoldOp::Close => {
                self.commit_file_folds.insert(row.id);
            }
            FoldOp::Open => {
                self.commit_file_folds.remove(&row.id);
            }
        }
        self.restore_commit_file_cursor(Some(&row.path));
    }

    /// Rows the diff pane path header takes (same count the paint uses).
    fn diff_header_rows(&self) -> u16 {
        diff_pane_header_rows(
            &self.diff_header_path(),
            self.layout.diff_pane_width,
            self.layout.diff_pane_height,
        )
    }

    fn diff_body_height(&self) -> usize {
        let h_bar = u16::from(self.diff_hscroll_shown());
        self.layout
            .diff_pane_height
            .saturating_sub(self.diff_header_rows() + h_bar)
            .max(1) as usize
    }

    fn split_layout(&self) -> SplitLayout {
        SplitLayout {
            term_cols: self.layout.term_cols.max(1),
            term_rows: self.layout.pane_height.saturating_add(2).max(1),
            pane_height: self.layout.pane_height.max(1),
            tree_width: self.layout.outer_tree_width.max(1),
            diff_pane_width: self.layout.diff_pane_width,
            diff_content_x: self.layout.diff_content_x,
            flat_panes: self.background == BackgroundMode::Paint,
            diff_split_rule_x: self.layout.diff_split_rule_x,
            graph_vbar: VScrollbarLayout {
                x: self.layout.graph_scrollbar_x,
                y: self.layout.graph_scrollbar_y,
                height: self.layout.graph_scrollbar_height,
                content_len: self.layout.graph_content_len,
                scroll: self.graph_painted_top(),
                scroll_max: graph_scroll_max(
                    self.layout.graph_content_len,
                    self.layout.graph_scrollbar_height,
                )
                .min(usize::from(u16::MAX)) as u16,
            },
            diff_vbar: VScrollbarLayout {
                scroll: self.diff_scroll,
                ..self.layout.diff_vbar
            },
            tree_vbar: self.layout.tree_vbar,
            files_vbar: self.layout.files_vbar,
            explorer_tree_vbar: self.layout.explorer_tree_vbar,
            file_view_vbar: self.layout.file_view_vbar,
            graph_hbar: HScrollbarLayout {
                y: self.layout.graph_hscrollbar_y,
                x: self.layout.graph_hscrollbar_x,
                width: self.layout.graph_hscrollbar_width,
                col_max: self.layout.graph_col_max,
                col_offset: self.hbar_offset(HScrollbar::Graph),
            },
            diff_hbar: HScrollbarLayout {
                y: self.layout.diff_hscrollbar_y,
                x: self.layout.diff_hscrollbar_x,
                width: self.layout.diff_hscrollbar_width,
                col_max: self.layout.diff_col_max,
                col_offset: self.hbar_offset(HScrollbar::Diff),
            },
            tree_hbar: HScrollbarLayout {
                col_offset: self.hbar_offset(HScrollbar::Tree),
                ..self.layout.tree_hbar
            },
            files_hbar: HScrollbarLayout {
                col_offset: self.hbar_offset(HScrollbar::CommitFiles),
                ..self.layout.files_hbar
            },
            explorer_tree_hbar: HScrollbarLayout {
                col_offset: self.hbar_offset(HScrollbar::ExplorerTree),
                ..self.layout.explorer_tree_hbar
            },
            file_view_hbar: HScrollbarLayout {
                col_offset: self.hbar_offset(HScrollbar::FileView),
                ..self.layout.file_view_hbar
            },
            // The summary viewport is its own state (no cursor): a press or
            // drag moves it before the next paint, as on the file diff.
            summary_vbar: VScrollbarLayout {
                scroll: self.summary_scroll().min(usize::from(u16::MAX)) as u16,
                ..self.layout.summary_vbar
            },
            summary_hbar: HScrollbarLayout {
                col_offset: self.hbar_offset(HScrollbar::Summary),
                ..self.layout.summary_hbar
            },
            prs_list_vbar: self.layout.prs_list_vbar,
            prs_detail_vbar: self.layout.prs_detail_vbar,
            commit_msg_rule_y: self.layout.commit_msg_rule_y,
            commit_msg_rule_x: self.layout.commit_msg_rule_x,
            commit_msg_rule_width: self.layout.commit_msg_rule_width,
        }
    }

    /// Store the split of a divider press or drag at `col`. A column that
    /// paints the tree at its current width (a press on the divider, or a
    /// drag the clamp holds at a narrow terminal) keeps the stored fraction,
    /// so it can come back when the terminal grows.
    fn apply_tree_fraction_from_col(&mut self, col: u16) {
        let cols = self.layout.term_cols.max(1);
        let fraction = clamp_tree_fraction(cols, tree_fraction_from_col(cols, col));
        if pane_widths(cols, fraction).tree_width
            != pane_widths(cols, self.tree_fraction).tree_width
        {
            self.tree_fraction = fraction;
        }
    }

    fn apply_diff_fraction_from_col(&mut self, col: u16) {
        self.diff_split_fraction = diff_split_fraction_from_col(
            self.layout.diff_content_x,
            self.layout.diff_pane_width.max(1),
            col,
        );
    }

    /// Left press at (`col`, `row`).
    ///
    /// An open popover takes the press first ([`Self::popover_click`]). A
    /// press on a painted icon selects its row like any click, then pins
    /// the icon's popover; it never counts toward a double-click. Anything
    /// else is [`Self::click_at`].
    fn click(&mut self, col: u16, row: u16) -> Effect {
        self.click_with(col, row, Self::click_at)
    }

    /// [`Self::click`] with `plain` as the plain click: [`Self::click_at`]
    /// on a Workspace or compare tab, `explorer_click` on an Explorer tab.
    pub(super) fn click_with(&mut self, col: u16, row: u16, plain: PlainClick) -> Effect {
        if let Some(effect) = self.popover_click(col, row, plain) {
            return effect;
        }
        let Some(hit) = self.icon_hit_at(col, row).cloned() else {
            return plain(self, col, row);
        };
        // Cleared before the select too, so an earlier press on this cell
        // (a Ctrl+click that found no PR) never makes this click a drill.
        self.last_click = None;
        let selected = plain(self, col, row);
        self.last_click = None;
        self.text_selection = None;
        let details = self.pin_icon(&hit);
        popover::then(selected, details)
    }

    /// True when (`col`, `row`) is on a painted line-gutter comment mark.
    pub(super) fn on_line_comment_mark(&self, col: u16, row: u16) -> bool {
        self.icon_hit_at(col, row)
            .is_some_and(|hit| matches!(hit.target, IconTarget::LineComment { .. }))
    }

    /// The drag handle under (`col`, `row`) ([`hit_split`]). The in-diff
    /// RULE's 3-column band covers the comment mark of the right cell next
    /// to it: the mark wins, so a press there picks its row and pins
    /// instead of moving the RULE. An Explorer tab has no commit-message
    /// footer, so a recorded commit-message rule is never an edge there.
    fn split_hit_at(&self, col: u16, row: u16) -> SplitHit {
        match hit_split(self.split_layout(), col, row) {
            SplitHit::DiffSplit if self.on_line_comment_mark(col, row) => SplitHit::Other,
            SplitHit::CommitMsg if self.is_explorer_tab() => SplitHit::Other,
            hit => hit,
        }
    }

    /// Plain left press: tabs, the PRs tab panes, the workspace PR total,
    /// dividers, scrollbars, then the row under it.
    fn click_at(&mut self, col: u16, row: u16) -> Effect {
        self.text_selection = None;
        self.pad_press = None;
        if let Some(index) = self.hit_tab_close(col, row) {
            return self.close_tab_at(index);
        }
        if let Some(index) = self.hit_tab(col, row) {
            return self.activate_tab(index);
        }
        // The PRs tab has no divider: its panes and their scrollbars take
        // the press.
        if self.is_prs_tab() {
            return self.prs_click(col, row);
        }
        if self
            .layout
            .pr_total_hit
            .is_some_and(|hit| hit.contains(ratatui::layout::Position::new(col, row)))
        {
            self.last_click = None;
            return self.open_prs_tab();
        }
        if let Some(effect) = self.press_split_hit(col, row) {
            return effect;
        }
        self.arm_text_selection(col, row);
        self.click_row(col, row)
    }

    /// Press on the drag handle under (`col`, `row`) ([`Self::split_hit_at`]),
    /// on every tab: the pane divider, the in-diff RULE, the commit-message
    /// rule, or a scrollbar arms its drag. `None` off every handle, for the
    /// caller's row click.
    fn press_split_hit(&mut self, col: u16, row: u16) -> Option<Effect> {
        match self.split_hit_at(col, row) {
            SplitHit::Pane if self.background == BackgroundMode::Paint => {
                // A flat divider press sits on a row's pad cell: resize only
                // once the pointer moves, else the release clicks the row
                // (`last_click` stays for its double-click test).
                self.drag = SplitDrag::Pane;
                self.pad_press = Some((col, row));
            }
            SplitHit::Pane => {
                self.drag = SplitDrag::Pane;
                self.apply_tree_fraction_from_col(col);
                self.last_click = None;
            }
            SplitHit::DiffSplit => {
                self.drag = SplitDrag::Diff;
                self.apply_diff_fraction_from_col(col);
                self.last_click = None;
            }
            SplitHit::CommitMsg => {
                // Rows change only once the pointer moves off this row.
                self.drag = SplitDrag::CommitMsg {
                    origin_row: row,
                    origin_lines: self.commit_msg_lines,
                };
                self.last_click = None;
            }
            SplitHit::VThumb(bar) => return Some(self.press_vscrollbar(bar, true, row)),
            SplitHit::VTrack(bar) => return Some(self.press_vscrollbar(bar, false, row)),
            SplitHit::HThumb(bar) => self.press_hscrollbar(bar, true, col),
            SplitHit::HTrack(bar) => self.press_hscrollbar(bar, false, col),
            SplitHit::Other => return None,
        }
        Some(Effect::None)
    }

    /// Note a row press at (`col`, `row`) and say whether it is a
    /// double-click: a second press on the same cell within 400 ms, timed
    /// by when the presses arrived ([`Self::input_clock`]).
    fn note_row_press(&mut self, col: u16, row: u16) -> bool {
        let now = self.input_clock();
        let is_double = self.last_click.as_ref().is_some_and(|(c, r, at)| {
            *c == col
                && *r == row
                && now.saturating_duration_since(*at) <= Duration::from_millis(400)
        });
        self.last_click = Some((col, row, now));
        is_double
    }

    /// The row under (`col`, `row`) after [`Self::click_at`] found no tab,
    /// divider, or scrollbar there: focus its pane and select the row; a
    /// second press on the same cell within 400 ms is a double-click
    /// ([`Self::note_row_press`]).
    fn click_row(&mut self, col: u16, row: u16) -> Effect {
        let is_double = self.note_row_press(col, row);
        if col >= self.layout.right_x {
            self.focus = FocusPane::Right;
            if self.is_compare_tab() {
                self.click_diff(row);
                return Effect::None;
            }
            if self.drill.is_files() {
                self.clear_diff_visual();
                return self.click_commit_files(col, row, is_double);
            }
            if !self.right_is_diff() && self.graph.is_some() {
                self.clear_diff_visual();
                return self.click_graph(row, is_double, true);
            }
            if self.right_is_diff() || self.drill.is_diff() {
                self.click_diff(row);
                if is_double {
                    return self.nav_enter();
                }
                return Effect::None;
            }
            if is_double {
                return self.nav_enter();
            }
            return Effect::None;
        }
        self.focus = FocusPane::Left;
        self.clear_diff_visual();
        if self.is_compare_tab() || self.drill.is_diff() {
            return self.click_commit_files(col, row, is_double);
        }
        if self.drill.is_files() {
            return self.click_graph(row, is_double, false);
        }
        // Below the painted rows: the horizontal bar row of a panned tree.
        let rows_end = self.layout.tree_y.saturating_add(self.tree_list_rows());
        if row < self.layout.tree_y || row >= rows_end {
            return Effect::None;
        }
        let idx = self.layout.list_offset + (row - self.layout.tree_y) as usize;
        let painted = self.painted_tree_rows();
        let Some(tree_row) = painted.get(idx).cloned() else {
            return Effect::None;
        };
        let Some(live) = self.rows.iter().position(|r| r.id == tree_row.id) else {
            return Effect::None;
        };
        self.cursor = live;
        if tree_row.foldable && self.is_tree_chevron(col, tree_row.depth) {
            // Two Downs are a double-click. Skip the second toggle so the
            // fold from the first click remains. Do not Enter.
            if is_double {
                return Effect::None;
            }
            self.fold_op(FoldOp::Toggle);
            return Effect::LoadRightPane;
        }
        if is_double {
            return self.nav_enter();
        }
        Effect::LoadRightPane
    }

    /// Select the file-diff row under `row` and keep it near the middle.
    fn click_diff(&mut self, row: u16) {
        let body_y = self.layout.right_y.saturating_add(self.diff_header_rows());
        if row < body_y {
            return;
        }
        if self.layout.diff_hscrollbar_y == Some(row) {
            return;
        }
        let n = self.current_diff_rows().len();
        if n == 0 {
            return;
        }
        let visual_y = (row - body_y) as usize;
        let idx = if self.diff_wrap {
            let Some(idx) = self.diff_logical_index_at_visual(visual_y) else {
                return;
            };
            idx
        } else {
            self.diff_scroll as usize + visual_y
        };
        if idx >= n {
            return;
        }
        self.diff_cursor = idx;
        self.sync_diff_scroll();
    }

    fn diff_wrap_heights(&self) -> Vec<usize> {
        self.diff_wrap_heights_for(&self.current_diff_rows())
    }

    /// [`Self::diff_wrap_heights`] over rows the caller already built.
    fn diff_wrap_heights_for(&self, rows: &[DiffRow]) -> Vec<usize> {
        let pane_w = diff_paint_width(self.layout.diff_pane_width);
        let content_w = diff_row_content_width(pane_w as usize) as u16;
        let gutter = gutter_width(rows).saturating_add(comment_mark_cols(self.ascii));
        let split = self.diff_layout() == DiffMode::SideBySide;
        diff_wrap_row_heights(rows, content_w, gutter, split, self.diff_split_fraction)
    }

    fn diff_logical_index_at_visual(&self, visual_y: usize) -> Option<usize> {
        let heights = self.diff_wrap_heights();
        let start = self.diff_scroll as usize;
        let mut y = 0usize;
        for (i, h) in heights.iter().copied().enumerate().skip(start) {
            let h = h.max(1);
            if visual_y < y.saturating_add(h) {
                return Some(i);
            }
            y = y.saturating_add(h);
        }
        None
    }

    fn is_tree_chevron(&self, col: u16, depth: usize) -> bool {
        col == self.layout.tree_x + 1 + (depth as u16) * 2
    }

    fn is_files_chevron(&self, col: u16, depth: usize) -> bool {
        col == self.files_list_origin_x() + 1 + (depth as u16) * 2
    }

    fn click_graph(&mut self, row: u16, is_double: bool, right: bool) -> Effect {
        let y = if right {
            self.layout.right_y
        } else {
            self.layout.tree_y
        };
        if row < y {
            return Effect::None;
        }
        let Some(model) = self.graph.as_ref() else {
            return Effect::None;
        };
        let chrome = self.graph_chrome();
        let mut offset = (row - y) as usize;
        if chrome.header {
            if offset == 0 {
                return Effect::None;
            }
            offset -= 1;
        }
        if offset >= chrome.list_height as usize {
            return Effect::None;
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        // Hit-test the rows on screen: mid-glide they sit above or below
        // the jump top in `graph_scroll`.
        let top = usize::from(self.graph_painted_top());
        let Some(line) = painted.get(top + offset) else {
            return Effect::None;
        };
        if let Some(idx) = line.row_index {
            self.graph_cursor = idx;
            self.sync_graph_scroll();
            if is_double {
                return self.nav_enter();
            }
            return self.follow_graph_files();
        }
        Effect::None
    }

    fn click_commit_files(&mut self, col: u16, row: u16, is_double: bool) -> Effect {
        let list_end = self
            .layout
            .files_list_y
            .saturating_add(self.layout.files_list_height);
        if row < self.layout.files_list_y || row >= list_end {
            return Effect::None;
        }
        let idx = self.layout.files_list_offset + (row - self.layout.files_list_y) as usize;
        let painted = self.painted_commit_file_rows();
        let Some(file_row) = painted.get(idx).cloned() else {
            return Effect::None;
        };
        let live_rows = self.commit_file_rows();
        let Some(live) = live_rows.iter().position(|r| r.id == file_row.id) else {
            return Effect::None;
        };
        self.set_commit_file_cursor(live);
        if file_row.foldable && self.is_files_chevron(col, file_row.depth) {
            if is_double {
                return Effect::None;
            }
            self.fold_commit_file(FoldOp::Toggle);
            return Effect::None;
        }
        let load = self.maybe_load_focused_commit_diff();
        if is_double {
            return match self.nav_enter() {
                Effect::None => load,
                other => other,
            };
        }
        load
    }

    fn drag_split(&mut self, col: u16, row: u16) -> Effect {
        // Drag events carry the pointer, but only buttonless motion stores
        // it: keep it current under a resize drag, so after the release the
        // edge stays lit only while the pointer is still on it.
        if self.drag.resize_edge().is_some() {
            self.pointer = Some((col, row));
        }
        match self.drag {
            // A flat pad press stays a row click while the pointer keeps to
            // its column (a vertical move too): the release clicks the
            // pressed row.
            SplitDrag::Pane if self.pad_press.is_some_and(|(c, _)| c == col) => {}
            SplitDrag::Pane => {
                if self.pad_press.take().is_some() {
                    // Now a resize, as a boxed divider press: never half of
                    // a double-click.
                    self.last_click = None;
                }
                self.apply_tree_fraction_from_col(col);
            }
            SplitDrag::Diff => self.apply_diff_fraction_from_col(col),
            SplitDrag::CommitMsg {
                origin_row,
                origin_lines,
            } => self.drag_commit_msg(origin_row, origin_lines, row),
            SplitDrag::VScrollbar {
                bar,
                origin_row,
                origin_scroll,
            } => {
                let geometry = self.split_layout().vbar(bar);
                let top = vscroll_from_delta(geometry, origin_row, origin_scroll, row);
                return self.scroll_vbar_to(bar, top);
            }
            SplitDrag::HScrollbar {
                bar,
                origin_col,
                origin_offset,
            } => {
                let geometry = self.split_layout().hbar(bar);
                let offset = hcol_from_delta(geometry, origin_col, origin_offset, col);
                self.scroll_hbar_to(bar, offset);
            }
            SplitDrag::None => {
                if let Some(selection) = self.text_selection.as_mut() {
                    selection.extend_to(col, row);
                }
            }
        }
        Effect::None
    }

    /// Arm a text selection when the press lands in the tree or right pane body.
    fn arm_text_selection(&mut self, col: u16, row: u16) {
        let layout = &self.layout;
        let left = Rect::new(
            layout.tree_x,
            layout.tree_y,
            layout.tree_width,
            layout.tree_height,
        );
        let right = Rect::new(
            layout.diff_content_x,
            layout.right_y,
            layout.diff_pane_width,
            layout.diff_pane_height,
        );
        self.text_selection =
            TextSelection::arm(left, col, row).or_else(|| TextSelection::arm(right, col, row));
    }

    /// End a text selection on mouse release.
    ///
    /// A drag that selected visible text copies it and announces the copy.
    /// A plain click or a blank selection copies nothing.
    pub(crate) fn finish_text_selection(&mut self) -> Effect {
        let Some(selection) = self.text_selection.take() else {
            return Effect::None;
        };
        if !selection.is_active() {
            return Effect::None;
        }
        let text = selection.text(&self.painted_frame);
        if text.trim().is_empty() {
            return Effect::None;
        }
        Effect::CopyClipboard {
            text,
            announce: true,
        }
    }

    /// Drop any split / scrollbar drag and any text selection.
    ///
    /// A cancelled commit-message rule drag keeps the rows it reached for
    /// this session (as a cancelled pane or RULE drag keeps its split) and
    /// saves nothing: only a release saves.
    pub(crate) fn cancel_mouse_drag(&mut self) {
        self.drag = SplitDrag::None;
        self.pad_press = None;
        self.text_selection = None;
    }

    /// Mouse release: ends any drag; a flat divider press still pending
    /// ([`Self::pad_press`], with the pane drag it armed) clicks the row
    /// under it. A commit-message rule drag saves its rows
    /// ([`Self::finish_commit_msg_drag`]). Then [`Self::release_mouse`].
    fn release_press(&mut self) -> Effect {
        let press = self
            .pad_press
            .take()
            .filter(|_| matches!(self.drag, SplitDrag::Pane));
        if let SplitDrag::CommitMsg { origin_lines, .. } = self.drag {
            self.finish_commit_msg_drag(origin_lines);
        }
        self.drag = SplitDrag::None;
        let Some((col, row)) = press else {
            return self.release_mouse();
        };
        let clicked = if self.is_explorer_tab() {
            self.explorer_click_row(col, row)
        } else {
            self.click_row(col, row)
        };
        popover::then(clicked, self.release_mouse())
    }

    /// A commit-message rule drag at `row`: [`commit_msg_lines_from_drag`]
    /// against this frame's pane fit, then keep the focused graph row in
    /// view as [`Self::resize_commit_msg`] does. Session only until release.
    fn drag_commit_msg(&mut self, origin_row: u16, origin_lines: usize, row: u16) {
        let next = commit_msg_lines_from_drag(
            origin_lines,
            origin_row,
            row,
            self.layout.commit_msg_lines_fit,
        );
        if next == self.commit_msg_lines {
            return;
        }
        self.commit_msg_lines = next;
        self.status = format!("msg lines {next}").into();
        // The graph list height changed: keep the focused row in view.
        self.sync_graph_scroll();
    }

    /// Release of a commit-message rule drag: save the rows to the UI-state
    /// store when they differ from `origin_lines` (the rows at the press).
    /// The config file is never written. A failed save keeps the rows for
    /// this session and names the store in the status row, like the theme
    /// and sparkline saves.
    fn finish_commit_msg_drag(&mut self, origin_lines: usize) {
        if self.commit_msg_lines == origin_lines {
            return;
        }
        if let Err(err) = save_commit_msg_lines(&self.ui_store_path, self.commit_msg_lines) {
            self.status = StatusMessage::error(persist_failed_status("commit message lines", err));
        }
    }

    /// Press on vertical scrollbar `bar` at `row`: the bar's pane takes
    /// focus; a press on the thumb grabs it where it is painted, a track
    /// press jumps the viewport toward `row` ([`vscroll_from_row`]). Either
    /// arms [`SplitDrag::VScrollbar`] from that top, and neither picks a row
    /// or counts toward a double-click. Returns what the move asks for: a
    /// list that follows its cursor loads the row the cursor moved to, as
    /// a wheel step does ([`Self::scroll_vbar_to`]).
    fn press_vscrollbar(&mut self, bar: VScrollbar, on_thumb: bool, row: u16) -> Effect {
        let geometry = self.split_layout().vbar(bar);
        // A thumb press keeps the painted top: it ends any glide without
        // jumping to the glide's target.
        let top = if on_thumb {
            geometry.scroll
        } else {
            vscroll_from_row(geometry, row)
        };
        match bar {
            VScrollbar::Graph => self.focus_graph_pane(),
            VScrollbar::Diff | VScrollbar::FileView | VScrollbar::Summary => {
                self.focus = FocusPane::Right;
            }
            VScrollbar::Tree | VScrollbar::ExplorerTree => self.focus = FocusPane::Left,
            VScrollbar::CommitFiles => self.focus = self.commit_files_pane(),
            // The PRs tab keeps the list focused ([`PrsTab::parked_view`]);
            // its narrow toggle, not the focus, picks the pane it shows.
            VScrollbar::PrsList | VScrollbar::PrsDetail => self.focus = FocusPane::Left,
        }
        let effect = self.scroll_vbar_to(bar, top);
        self.drag = SplitDrag::VScrollbar {
            bar,
            origin_row: row,
            origin_scroll: top,
        };
        self.last_click = None;
        effect
    }

    /// Scroll the viewport of vertical scrollbar `bar` to `top`, and return
    /// what the move asks for.
    ///
    /// The graph bar moves `graph_scroll` only (the graph wheel moves the
    /// cursor, but the graph viewport does not follow it). Every other
    /// viewport follows its cursor, so the cursor moves to the row nearest
    /// it whose viewport starts at `top` ([`cursor_for_viewport_top`]), and
    /// a moved cursor asks for what the wheel asks for: the tree loads the
    /// right pane, a commit-file row its diff, an Explorer row its preview.
    /// The clean-file view scrolls at the edge of its line cursor, so its
    /// cursor moves to the nearest edge line of the new viewport. The
    /// folder summary and the PRs detail have no cursor: their viewport
    /// moves alone. The PRs list cursor moves and asks for nothing, as its
    /// wheel.
    fn scroll_vbar_to(&mut self, bar: VScrollbar, top: u16) -> Effect {
        let top = usize::from(top);
        match bar {
            VScrollbar::Graph => {
                self.graph_scroll = top as u16;
                Effect::None
            }
            VScrollbar::Summary => {
                self.scroll_summary_to(top);
                Effect::None
            }
            VScrollbar::Diff => {
                self.scroll_diff_to(top);
                Effect::None
            }
            VScrollbar::Tree => self.scroll_tree_to(top),
            VScrollbar::CommitFiles => self.scroll_commit_files_to(top),
            VScrollbar::ExplorerTree => self.scroll_explorer_tree_to(top),
            VScrollbar::FileView => {
                self.scroll_file_view_to(top);
                Effect::None
            }
            VScrollbar::PrsList => {
                self.scroll_prs_list_to(top);
                Effect::None
            }
            VScrollbar::PrsDetail => {
                self.scroll_prs_detail_to(top);
                Effect::None
            }
        }
    }

    /// Scroll the file diff so its viewport starts at row `top`.
    ///
    /// The diff viewport follows `diff_cursor` (middle-biased, as
    /// [`Self::sync_diff_scroll`] places it), so the cursor moves to the row
    /// nearest it whose viewport starts at `top` ([`cursor_for_viewport_top`]),
    /// as a wheel step moves the cursor.
    fn scroll_diff_to(&mut self, top: usize) {
        let rows = self.current_diff_rows();
        let n = rows.len();
        if n == 0 {
            return;
        }
        let height = self.diff_body_height();
        self.diff_cursor = if self.diff_wrap {
            let heights = self.diff_wrap_heights_for(&rows);
            cursor_for_viewport_top(n, self.diff_cursor, top, |cursor| {
                wrap_viewport_start(&heights, cursor, height)
            })
        } else {
            cursor_for_viewport_top(n, self.diff_cursor, top, |cursor| {
                list_viewport_start(n, cursor, height)
            })
        };
        self.sync_diff_scroll_for(&rows);
    }

    /// Scroll the Workspace tree so its painted rows start at `top`: the
    /// tree cursor moves with the viewport ([`cursor_for_viewport_top`] over
    /// the painted rows, [`Self::tree_list_rows`] tall), and a moved cursor
    /// loads the right pane, as the wheel does.
    fn scroll_tree_to(&mut self, top: usize) -> Effect {
        let painted = self.painted_tree_rows();
        let focus = self.rows.get(self.cursor).map(|row| row.id.as_str());
        let cursor = focus
            .and_then(|id| painted.iter().position(|row| row.id == id))
            .unwrap_or(0);
        let height = usize::from(self.tree_list_rows());
        let n = painted.len();
        let target = cursor_for_viewport_top(n, cursor, top, |c| list_viewport_start(n, c, height));
        let live = nearest_live_row(&painted, target, |row| {
            self.rows.iter().position(|live| live.id == row.id)
        });
        match live {
            Some(live) if live != self.cursor => {
                self.cursor = live;
                Effect::LoadRightPane
            }
            _ => Effect::None,
        }
    }

    /// Scroll the commit-file or compare list so its painted rows start at
    /// `top`: the file cursor moves with the viewport, and a moved cursor
    /// loads its file's diff, as the wheel does.
    fn scroll_commit_files_to(&mut self, top: usize) -> Effect {
        let painted = self.painted_commit_file_rows();
        let rows = self.commit_file_rows();
        let current = self.commit_files_cursor();
        let focus = rows.get(current).map(|row| row.id.as_str());
        let cursor = focus
            .and_then(|id| painted.iter().position(|row| row.id == id))
            .unwrap_or(0);
        let height = usize::from(self.layout.files_list_height.max(1));
        let n = painted.len();
        let target = cursor_for_viewport_top(n, cursor, top, |c| list_viewport_start(n, c, height));
        let live = nearest_live_row(&painted, target, |row| {
            rows.iter().position(|live| live.id == row.id)
        });
        match live {
            Some(live) if live != current => {
                self.set_commit_file_cursor(live);
                self.maybe_load_focused_commit_diff()
            }
            _ => Effect::None,
        }
    }

    /// Rows the Workspace tree paints: the pane rows less the horizontal
    /// scrollbar row it keeps while panned ([`list_hbar_rows`]).
    fn tree_list_rows(&self) -> u16 {
        let height = self.layout.tree_height;
        height.saturating_sub(list_hbar_rows(height, self.left_col_offset))
    }

    /// The pane that shows the commit-file or compare list: the left pane
    /// beside a file diff (depth 2, a compare tab), else the right pane
    /// (depth 1).
    fn commit_files_pane(&self) -> FocusPane {
        if self.is_compare_tab() || self.drill.is_diff() {
            FocusPane::Left
        } else {
            FocusPane::Right
        }
    }

    /// Press on horizontal scrollbar `bar` at `col`: the bar's pane takes
    /// focus; a press on the thumb grabs it where it is painted, a track
    /// press jumps the pan toward `col` ([`hcol_from_col`]). Either arms
    /// [`SplitDrag::HScrollbar`] from that offset, and neither picks a row,
    /// moves a cursor, or counts toward a double-click.
    fn press_hscrollbar(&mut self, bar: HScrollbar, on_thumb: bool, col: u16) {
        let geometry = self.split_layout().hbar(bar);
        let offset = if on_thumb {
            geometry.col_offset
        } else {
            hcol_from_col(geometry, col)
        };
        match bar {
            HScrollbar::Graph => self.focus_graph_pane(),
            HScrollbar::Diff | HScrollbar::FileView | HScrollbar::Summary => {
                self.focus = FocusPane::Right;
            }
            HScrollbar::Tree | HScrollbar::ExplorerTree => self.focus = FocusPane::Left,
            HScrollbar::CommitFiles => self.focus = self.commit_files_pane(),
        }
        self.scroll_hbar_to(bar, offset);
        self.drag = SplitDrag::HScrollbar {
            bar,
            origin_col: col,
            origin_offset: offset,
        };
        self.last_click = None;
    }

    /// The pan offset horizontal scrollbar `bar` moves: the offset
    /// Shift-←/→ and the horizontal wheel pan on its pane.
    fn hbar_offset(&self, bar: HScrollbar) -> u16 {
        match bar {
            HScrollbar::Graph => self.graph_col_offset(),
            HScrollbar::Diff => self.diff_col_offset,
            HScrollbar::Tree => self.left_col_offset,
            HScrollbar::CommitFiles if self.commit_files_pane() == FocusPane::Left => {
                self.left_col_offset
            }
            HScrollbar::CommitFiles => self.right_col_offset,
            HScrollbar::ExplorerTree => self.explorer_tree_col_offset(),
            HScrollbar::FileView => self
                .tabs
                .active_file_view()
                .map_or(0, |file| file.col_offset),
            HScrollbar::Summary => self.summary_col_offset(),
        }
    }

    /// Pan horizontal scrollbar `bar`'s pane to `offset` (already clamped
    /// to the bar's `col_max`, the pane's pan clamp). The paint also
    /// drops a pan the clamp no longer allows here (the pane widened).
    pub(crate) fn scroll_hbar_to(&mut self, bar: HScrollbar, offset: u16) {
        match bar {
            HScrollbar::Graph => self.set_graph_col_offset(offset),
            HScrollbar::Diff => self.diff_col_offset = offset,
            HScrollbar::Tree => self.left_col_offset = offset,
            HScrollbar::CommitFiles if self.commit_files_pane() == FocusPane::Left => {
                self.left_col_offset = offset;
            }
            HScrollbar::CommitFiles => self.right_col_offset = offset,
            HScrollbar::ExplorerTree => self.set_explorer_tree_col_offset(offset),
            HScrollbar::FileView => {
                if let Some(file) = self.tabs.active_file_view_mut() {
                    file.col_offset = offset;
                }
            }
            HScrollbar::Summary => self.set_summary_col_offset(offset),
        }
    }

    fn focus_graph_pane(&mut self) {
        self.focus = if self.drill.is_files() {
            FocusPane::Left
        } else {
            FocusPane::Right
        };
    }

    fn toggle_diff_mode(&mut self) -> Effect {
        self.diff_mode = match self.diff_mode {
            DiffMode::SideBySide => DiffMode::Inline,
            DiffMode::Inline => DiffMode::SideBySide,
        };
        self.status = self.diff_mode_status().into();
        self.drop_stale_diff_visual();
        Effect::None
    }

    /// Status after `i`: the preferred mode, or what split needs when the
    /// right pane is too narrow to paint it.
    fn diff_mode_status(&self) -> String {
        match (self.diff_mode, self.diff_layout()) {
            (DiffMode::SideBySide, DiffMode::Inline) => format!(
                "split needs a ≥{NARROW_SXS}-col diff pane (now {}) — widen it with < or drag",
                diff_paint_width(self.layout.diff_pane_width)
            ),
            (DiffMode::SideBySide, DiffMode::SideBySide) => "Diff: split".into(),
            (DiffMode::Inline, _) => "Diff: inline".into(),
        }
    }

    /// Move the tree / right split by `steps` of 5% (`<` / `>`). Session-only, like a drag.
    /// A step the clamp refuses keeps the stored fraction.
    fn step_tree_width(&mut self, steps: i32) -> Effect {
        let cols = self.layout.term_cols.max(1);
        let before = pane_widths(cols, self.tree_fraction).tree_width;
        let stepped = step_tree_fraction(cols, self.tree_fraction, steps);
        if pane_widths(cols, stepped).tree_width == before {
            let edge = if steps < 0 { "narrowest" } else { "widest" };
            self.status = StatusMessage::warn(format!("tree pane is at its {edge}"));
        } else {
            self.tree_fraction = stepped;
        }
        Effect::None
    }

    fn toggle_diff_wrap(&mut self) -> Effect {
        self.diff_wrap = !self.diff_wrap;
        if self.diff_wrap {
            self.diff_col_offset = 0;
            self.status = "wrap on".into();
        } else {
            self.status = "wrap off".into();
        }
        Effect::None
    }

    fn toggle_commit_msg_expand(&mut self) -> Effect {
        self.commit_msg_expand = !self.commit_msg_expand;
        self.status = if self.commit_msg_expand {
            "msg on".into()
        } else {
            "msg off".into()
        };
        // The graph list height changed: keep the focused row in view.
        self.sync_graph_scroll();
        Effect::None
    }

    /// Shorter / Taller commit message (palette only): change the expanded
    /// commit-message footer by `delta` rows for this session, clamped to
    /// [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`]. Unlike a drag of
    /// the footer rule, it saves nothing.
    fn resize_commit_msg(&mut self, delta: i32) -> Effect {
        let next = (self.commit_msg_lines as i64).saturating_add(i64::from(delta));
        self.commit_msg_lines =
            next.clamp(COMMIT_MSG_LINES_MIN as i64, COMMIT_MSG_LINES_MAX as i64) as usize;
        self.status = format!("msg lines {}", self.commit_msg_lines).into();
        // The graph list height changed: keep the focused row in view.
        self.sync_graph_scroll();
        Effect::None
    }

    /// Replace the graph model for `repo` at `head`.
    ///
    /// Same-checkout reloads keep the focused row by
    /// [`super::watch::graph_row_id`] (commit sha, stash ref, worktree
    /// path, or uncommitted). A silent HEAD move that only prepends
    /// commits therefore does not jump to the top. If that row is gone,
    /// the cursor clamps to `min(old_index, n-1)` instead of forcing
    /// row 0. Switching checkout (or the first load) resets cursor and
    /// scroll to the top.
    pub fn set_graph(&mut self, model: GraphModel, repo: String, head: String) {
        let after = graph_row_signatures(&model, &repo);
        let next_meta = graph_flash_meta(&model, &repo);
        let focused = self.focused_graph_repo();
        let decision = graph_flash_decision(
            focused.as_deref(),
            &repo,
            &self.graph_signatures,
            &after,
            self.graph_flash_meta.as_ref(),
            &next_meta,
        );
        let now = Instant::now();
        self.prune_flash_state(now);
        match decision {
            GraphFlashDecision::Stale => {}
            GraphFlashDecision::Seed => {
                self.graph_signatures = after;
                self.graph_flash_meta = Some(next_meta);
            }
            GraphFlashDecision::Apply { include_adds } => {
                let kinds = flashable_row_kinds(&self.graph_signatures, &after, include_adds);
                self.stamp_flashes(kinds, now);
                self.graph_signatures = after;
                self.graph_flash_meta = Some(next_meta);
            }
        }
        let previous_repo = self.graph_identity.as_ref().map(|(repo, _)| repo.clone());
        let previous_cursor = self.graph_cursor;
        let previous_row_id = self.focused_graph_row().as_ref().map(graph_row_id);
        let repo_changed = previous_repo.as_deref() != Some(repo.as_str());
        if repo_changed {
            self.left_col_offset = 0;
            self.right_col_offset = 0;
            if !matches!(self.drill, DrillView::Files { .. } | DrillView::Diff { .. }) {
                self.drill = DrillView::Graph;
            }
        }
        self.graph_identity = Some((repo.clone(), head));
        self.graph = Some(model);
        self.restore_graph_focus_after_load(
            previous_repo.as_deref(),
            &repo,
            previous_row_id.as_deref(),
            previous_cursor,
        );
        self.retry_graph_reveal(&repo, true);
        if self.drill.is_graph() {
            self.diff_content = DiffContent::default();
            self.diff_repo = None;
            self.diff_path = None;
        }
    }

    /// Restore graph list focus after [`Self::set_graph`] replaces the model.
    ///
    /// Policy: same checkout → find the previous
    /// [`super::watch::graph_row_id`]; if it is missing, clamp to
    /// `min(old_index, n-1)` (nearest remaining neighbor, never a forced
    /// jump to 0). Different checkout or an empty list → 0. Scroll
    /// recenters on the retained row the same way as `j`/`k`.
    fn restore_graph_focus_after_load(
        &mut self,
        previous_repo: Option<&str>,
        repo: &str,
        previous_row_id: Option<&str>,
        previous_cursor: usize,
    ) {
        let rows = self
            .graph
            .as_ref()
            .map(|graph| graph.visible_rows())
            .unwrap_or_default();
        let n = rows.len();
        if n == 0 || previous_repo != Some(repo) {
            self.graph_cursor = 0;
            self.graph_scroll = 0;
            return;
        }
        if let Some(id) = previous_row_id {
            if let Some(idx) = rows.iter().position(|row| graph_row_id(row) == id) {
                self.graph_cursor = idx;
                self.sync_graph_scroll();
                return;
            }
        }
        self.graph_cursor = previous_cursor.min(n - 1);
        self.sync_graph_scroll();
    }

    /// Store workspace-file diff content for the right pane.
    pub fn set_diff(&mut self, repo: String, path: String, content: DiffContent) {
        if self.is_compare_tab() {
            return;
        }
        self.adopt_diff_view(DiffViewId::Workspace {
            repo: repo.clone(),
            path: path.clone(),
        });
        self.diff_repo = Some(repo);
        self.diff_path = Some(path);
        self.diff_content = content;
        let n = self.current_diff_rows().len();
        if n == 0 {
            self.diff_cursor = 0;
        } else {
            self.diff_cursor = self.diff_cursor.min(n - 1);
        }
        self.apply_pending_hunk_anchor();
        self.sync_diff_scroll();
        self.drop_stale_diff_visual();
        if self.drill.is_graph() {
            self.graph = None;
        }
    }

    pub fn clear_right(&mut self) {
        if self.is_compare_tab() {
            return;
        }
        self.graph = None;
        self.graph_identity = None;
        self.graph_cursor = 0;
        self.graph_loading_older = false;
        self.commit_files_loading = false;
        self.drill = DrillView::Graph;
        self.diff_content = DiffContent::default();
        self.diff_repo = None;
        self.diff_path = None;
        self.diff_view = None;
        self.reset_diff_viewport();
        self.graph_signatures.clear();
        self.graph_flash_meta = None;
        self.commit_file_signatures.clear();
        self.commit_file_ghosts.clear();
    }

    /// Open the commit-files drill before git returns.
    ///
    /// Paint uses `loading files…` while `commit_files_loading` is true and
    /// the list is empty.
    pub fn begin_commit_files(&mut self, repo: String, source: CommitFileSource) {
        if self.is_compare_tab() {
            return;
        }
        self.commit_file_folds.clear();
        self.commit_files_loading = true;
        self.right_col_offset = 0;
        self.drill = DrillView::Files {
            repo,
            source,
            files: Vec::new(),
            cursor: 0,
        };
        self.focus = FocusPane::Right;
    }

    /// Fill the commit-file list. Enter from the graph focuses the right pane;
    /// a follow/reload that is already at files/diff keeps the current focus.
    pub fn open_commit_files(
        &mut self,
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
    ) {
        if self.is_compare_tab() {
            return;
        }
        let same_source = match &self.drill {
            DrillView::Files {
                repo: r, source: s, ..
            }
            | DrillView::Diff {
                repo: r, source: s, ..
            } => r == &repo && s == &source,
            DrillView::Graph => false,
        };
        let keep_path = if same_source {
            self.focused_commit_file_row().map(|row| row.path.clone())
        } else {
            None
        };
        // `begin_commit_files` already set the source, so a first load is
        // a new source or the pending `loading files…` list.
        let first_load = !same_source || self.commit_files_loading;
        let previous_cursor = self.commit_files_cursor();
        let before = self.commit_file_signatures.clone();
        let old_rows = if same_source {
            self.commit_file_rows()
        } else {
            Vec::new()
        };
        if !same_source {
            self.commit_file_folds.clear();
            self.right_col_offset = 0;
        }
        self.commit_files_loading = false;
        if first_load {
            self.status.offer_info(source.files_status(files.len()));
        }
        let cursor = DrillView::files_cursor(&files, 0);
        let retain_focus = !self.drill.is_graph();
        self.drill = DrillView::Files {
            repo: repo.clone(),
            source: source.clone(),
            files,
            cursor,
        };
        if !retain_focus {
            self.focus = FocusPane::Right;
        }
        if let Some(path) = keep_path.as_deref() {
            self.restore_commit_file_cursor_retain(path, previous_cursor);
        }
        let unfolded = match &self.drill {
            DrillView::Files { files, .. } => self.unfolded_commit_file_rows(files),
            _ => Vec::new(),
        };
        let after = commit_file_signatures(&repo, &source, &unfolded);
        let now = Instant::now();
        self.prune_flash_state(now);
        if !same_source || is_new_row_set(&before, &after) {
            self.commit_file_signatures = after;
            self.commit_file_ghosts.clear();
        } else {
            self.commit_file_ghosts.extend(capture_removal_ghosts(
                &old_rows,
                |row| row.id.as_str(),
                |row| commit_file_identity(&repo, &source, &row.id),
                &before,
                &after,
                now,
            ));
            self.stamp_flashes(flashable_row_kinds(&before, &after, true), now);
            self.commit_file_signatures = after;
        }
    }

    pub fn open_commit_diff(
        &mut self,
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
        file_cursor: usize,
        path: String,
        content: DiffContent,
    ) {
        if self.is_compare_tab() {
            return;
        }
        let entering = !self.drill.is_diff();
        self.adopt_diff_view(DiffViewId::Commit {
            repo: repo.clone(),
            source: source.clone(),
            path: path.clone(),
        });
        if entering {
            self.left_col_offset = 0;
        }
        self.drill = DrillView::Diff {
            repo,
            source,
            files,
            file_cursor,
            path,
            content,
        };
        let n = self.current_diff_rows().len();
        if n == 0 {
            self.diff_cursor = 0;
        } else {
            self.diff_cursor = self.diff_cursor.min(n - 1);
        }
        self.apply_pending_hunk_anchor();
        self.sync_diff_scroll();
        self.drop_stale_diff_visual();
    }

    fn reset_diff_viewport(&mut self) {
        self.diff_scroll = 0;
        self.diff_cursor = 0;
        self.diff_col_offset = 0;
        self.clear_diff_visual();
    }

    fn adopt_diff_view(&mut self, id: DiffViewId) {
        if self.diff_view.as_ref() != Some(&id) {
            self.reset_diff_viewport();
            self.diff_view = Some(id);
        }
    }

    pub fn focused_file(&self) -> Option<(String, FileChange)> {
        let row = self.focused_row()?;
        Some((row.repo.clone()?, row.file.clone()?))
    }

    pub fn focused_graph_repo(&self) -> Option<String> {
        let row = self.focused_row()?;
        match row.kind {
            NodeKind::Repo | NodeKind::Checkout | NodeKind::Dir | NodeKind::Section => {
                row.repo.clone()
            }
            NodeKind::File => None,
            NodeKind::Workspace | NodeKind::Group => None,
        }
    }

    fn current_search_pane(&self) -> SearchPane {
        if self.is_explorer_tab() {
            return self.explorer_search_pane();
        }
        match self.list_focus_target() {
            ListFocusTarget::Tree => SearchPane::Tree,
            ListFocusTarget::Graph => SearchPane::Graph,
            ListFocusTarget::CommitFiles => SearchPane::CommitFiles,
            ListFocusTarget::None => SearchPane::Diff,
        }
    }

    pub(crate) fn current_diff_content(&self) -> &DiffContent {
        if let Some(tab) = self.tabs.active_compare() {
            return &tab.content;
        }
        if self.is_explorer_tab() {
            return self
                .explorer_diff()
                .map_or(&EMPTY_DIFF, |(_, _, content)| content);
        }
        match &self.drill {
            DrillView::Diff { content, .. } => content,
            _ => &self.diff_content,
        }
    }

    /// Path shown in the diff pane header (`repo/path`).
    pub fn diff_header_path(&self) -> String {
        if self.is_explorer_tab() {
            return self
                .explorer_diff()
                .map(|(checkout, rel, _)| format!("{checkout}/{rel}"))
                .unwrap_or_default();
        }
        if let Some(tab) = self.tabs.active_compare() {
            return match tab.path.as_deref() {
                Some(path) if !path.is_empty() => {
                    format!("{}/{}  {}", tab.checkout_path, path, tab.range_header())
                }
                _ => tab.range_header(),
            };
        }
        match &self.drill {
            DrillView::Diff { repo, path, .. } => format!("{repo}/{path}"),
            _ => match (self.diff_repo.as_deref(), self.diff_path.as_deref()) {
                (Some(repo), Some(path)) => format!("{repo}/{path}"),
                (_, Some(path)) => path.to_string(),
                _ => String::new(),
            },
        }
    }

    /// Numbered rows for the current layout (inline vs split).
    ///
    /// Empty while a [`Self::folder_summary`] covers the right pane, so
    /// right-pane moves, wheel, search, and `V` never touch the hidden diff.
    pub fn current_diff_rows(&self) -> Vec<DiffRow> {
        #[cfg(test)]
        self.diff_row_builds.set(self.diff_row_builds.get() + 1);
        if self.folder_summary().is_some() {
            return Vec::new();
        }
        build_diff_rows(self.current_diff_content(), self.diff_layout())
    }

    /// Layout the file diff paints in: [`diff_pane_mode`] of the preferred
    /// mode and the last painted right-pane width. Row build, header,
    /// status pill, and paint all read this one decision.
    pub fn diff_layout(&self) -> DiffMode {
        diff_pane_mode(self.diff_mode, self.layout.diff_pane_width)
    }

    fn set_search_status(&mut self, hit: bool) {
        if self.search_query.trim().is_empty() {
            self.status.clear();
        } else if hit {
            // Armed query is a `/query` chip on the idle bar, not `n next N prev`.
            self.status.clear();
        } else {
            self.status = StatusMessage::warn(SEARCH_NO_MATCH);
        }
    }

    /// Step the bound pane to a match, vim `/` style.
    ///
    /// `dir` `0` (each typed key, Enter) first puts the pane back at the
    /// [`SearchOrigin`] and steps forward from there, so the result only
    /// depends on the query. `1` / `-1` (`n` / `N`) step from the pane's
    /// cursor. A step that wraps says which way, after the match status.
    fn apply_search(&mut self, dir: i32) -> Effect {
        let explorer_before = self.explorer_search_anchor_rel();
        if dir == 0 {
            if let Some(origin) = self.search_origin.clone() {
                self.put_pane_at_search_origin(origin);
            }
        }
        let wrapped = match self.search_target {
            SearchPane::Tree => self.apply_tree_search(dir),
            SearchPane::Graph => self.apply_graph_search(dir),
            SearchPane::CommitFiles => self.apply_commit_file_search(dir),
            SearchPane::Diff => self.apply_diff_search(dir),
            SearchPane::File => self.apply_file_search(dir),
            SearchPane::ExplorerTree => self.apply_explorer_tree_search(dir),
        };
        if wrapped {
            self.status = StatusMessage::info(if dir < 0 {
                SEARCH_WRAPPED_TO_BOTTOM
            } else {
                SEARCH_WRAPPED_TO_TOP
            });
        }
        self.search_pane_effect(explorer_before)
    }

    /// Diff row indices whose search text contains `query` (case-folded),
    /// cached by content fingerprint, layout, and query.
    ///
    /// No hits while a [`Self::folder_summary`] hides the diff. That check
    /// runs before the cache, which keys on the hidden diff's content.
    pub(crate) fn diff_search_hits(&self, query: &str) -> Vec<usize> {
        let query = query.trim().to_lowercase();
        if query.is_empty() || self.folder_summary().is_some() {
            return Vec::new();
        }
        let key = (
            self.current_diff_content().syntax_fingerprint(),
            self.diff_layout(),
            query,
        );
        if let Some(memo) = self.diff_search_memo.borrow().as_ref() {
            if memo.key == key {
                return memo.hits.clone();
            }
        }
        let texts: Vec<String> = self
            .current_diff_rows()
            .iter()
            .map(row_search_text)
            .collect();
        let hits = match_diff_line_indices(&texts, &key.2);
        *self.diff_search_memo.borrow_mut() = Some(DiffSearchMemo {
            key,
            hits: hits.clone(),
        });
        hits
    }

    /// 1-based match position of the bound pane's cursor (`None` when the
    /// cursor is off a match) and the match count, for the `/q 3/7` pill.
    ///
    /// `None` with no query. Hits follow each pane's search order; the
    /// current match is the tree row, graph cursor, commit-file row, or the
    /// diff search hit (the row the last step landed on).
    pub fn search_match_position(&self) -> Option<(Option<usize>, usize)> {
        let query = self.search_query.trim();
        if query.is_empty() {
            return None;
        }
        let (current, hits): (Option<usize>, Vec<usize>) = match self.search_target {
            SearchPane::Tree => {
                let ids = collect_match_ids(&self.tree, query);
                let current = self.focused_row().map(|row| row.id.as_str());
                let pos = current.and_then(|id| ids.iter().position(|hit| hit == id));
                return Some((pos.map(|p| p + 1), ids.len()));
            }
            SearchPane::Graph => {
                let rows = self.graph.as_ref()?.visible_rows();
                (
                    Some(self.graph_cursor),
                    collect_graph_match_indices(&rows, query),
                )
            }
            SearchPane::CommitFiles => {
                let (rows, hits) = self.commit_file_search_hits()?;
                (self.commit_file_search_anchor(&rows), hits)
            }
            SearchPane::Diff => (self.search_hit, self.diff_search_hits(query)),
            SearchPane::File => {
                let hits = self.file_search_hits(query);
                let pos = self
                    .tabs
                    .active_file_view()
                    .and_then(|tab| hits.binary_search(&tab.cursor).ok());
                return Some((pos.map(|p| p + 1), hits.len()));
            }
            SearchPane::ExplorerTree => {
                let (rows, cursor) = self.explorer_rows()?;
                (cursor, explorer_row_match_indices(&rows, query))
            }
        };
        let pos = current.and_then(|cur| hits.iter().position(|hit| *hit == cur));
        Some((pos.map(|p| p + 1), hits.len()))
    }

    /// Save the bound pane's cursor, scroll, and folds as `/` opens.
    fn save_search_origin(&mut self) {
        self.search_origin = Some(SearchOrigin {
            tree_row: self.focused_row().map(|row| row.id.clone()),
            folds: self.folds.clone(),
            graph_cursor: self.graph_cursor,
            graph_scroll: self.graph_scroll,
            commit_file_cursor: self.commit_files_cursor(),
            commit_file_folds: self.commit_file_folds.clone(),
            diff_cursor: self.diff_cursor,
            diff_scroll: self.diff_scroll,
            file_cursor: self.tabs.active_file_view().map_or(0, |tab| tab.cursor),
            file_scroll: self.tabs.active_file_view().map_or(0, |tab| tab.scroll),
            explorer_cursor: self.explorer_search_anchor_rel(),
        });
    }

    /// Put the bound pane back where `/` found it (Esc while typing).
    pub(crate) fn restore_search_origin(&mut self) -> Effect {
        let Some(origin) = self.search_origin.take() else {
            return Effect::None;
        };
        let explorer_before = self.explorer_search_anchor_rel();
        self.put_pane_at_search_origin(origin);
        self.search_pane_effect(explorer_before)
    }

    /// Cursor, scroll, and folds of the bound pane back to `origin`.
    fn put_pane_at_search_origin(&mut self, origin: SearchOrigin) {
        match self.search_target {
            SearchPane::Tree => {
                self.folds = origin.folds;
                self.rebuild_rows();
                if let Some(idx) = origin
                    .tree_row
                    .and_then(|id| self.rows.iter().position(|row| row.id == id))
                {
                    self.cursor = idx;
                }
            }
            SearchPane::Graph => {
                self.graph_cursor = origin.graph_cursor;
                self.graph_scroll = origin.graph_scroll;
            }
            SearchPane::CommitFiles => {
                self.commit_file_folds = origin.commit_file_folds;
                self.set_commit_file_cursor(origin.commit_file_cursor);
            }
            SearchPane::Diff => {
                self.diff_cursor = origin.diff_cursor;
                self.diff_scroll = origin.diff_scroll;
            }
            SearchPane::File => {
                if let Some(tab) = self.tabs.active_file_view_mut() {
                    tab.cursor = origin.file_cursor;
                    tab.scroll = origin.file_scroll;
                }
            }
            SearchPane::ExplorerTree => {
                if let (Some(tab), Some(rel)) =
                    (self.tabs.active_explorer_mut(), origin.explorer_cursor)
                {
                    tab.tree.set_cursor_rel(&rel);
                }
            }
        }
    }

    /// Load that follows the bound pane's cursor after a search move. An
    /// Explorer tree move previews its row, as `j` / `k` do; a step that
    /// left the tree cursor on `explorer_before` (no match, or the only
    /// match) previews nothing, so it cannot open a preview tab.
    fn search_pane_effect(&mut self, explorer_before: Option<String>) -> Effect {
        match self.search_target {
            SearchPane::Tree => Effect::LoadRightPane,
            SearchPane::Graph => self.follow_graph_files(),
            SearchPane::CommitFiles => self.maybe_load_focused_commit_diff(),
            SearchPane::ExplorerTree => {
                if self.explorer_search_anchor_rel() == explorer_before {
                    Effect::None
                } else {
                    self.explorer_tree_search_preview()
                }
            }
            SearchPane::Diff | SearchPane::File => Effect::None,
        }
    }

    /// Tree search step. Returns true when it wrapped.
    fn apply_tree_search(&mut self, dir: i32) -> bool {
        let current = self.focused_row().map(|r| r.id.clone());
        let (folds, landing) = focus_tree_search(
            &self.tree,
            &self.folds,
            &self.search_query,
            current.as_deref(),
            dir,
        );
        self.folds = folds;
        self.rebuild_rows();
        let Some(landing) = landing else {
            self.set_search_status(false);
            return false;
        };
        if let Some(idx) = self.rows.iter().position(|r| r.id == landing.target) {
            self.cursor = idx;
        }
        self.set_search_status(true);
        landing.wrapped
    }

    /// Graph search step from the graph cursor. Returns true when it wrapped.
    fn apply_graph_search(&mut self, dir: i32) -> bool {
        let Some(model) = self.graph.as_ref() else {
            self.set_search_status(false);
            return false;
        };
        let rows = model.visible_rows();
        let Some(landing) = focus_graph_search(&rows, &self.search_query, self.graph_cursor, dir)
        else {
            self.set_search_status(false);
            return false;
        };
        self.graph_cursor = landing.target;
        self.sync_graph_scroll();
        self.set_search_status(true);
        landing.wrapped
    }

    /// Commit-file list flattened with no folds (the search order), and
    /// the positions in it of the rows the search highlights, steps to,
    /// and counts. `None` with no commit-file list.
    fn commit_file_search_hits(&self) -> Option<(Vec<CommitFileRow>, Vec<usize>)> {
        let rows = self.unfolded_commit_file_rows(self.commit_drill_files()?);
        let hits = commit_file_row_match_indices(&rows, &self.search_query);
        Some((rows, hits))
    }

    /// Ids of the commit-file rows the bound search highlights: the same
    /// rows `n` / `N` step to and the pill counts. Empty unless `/` is
    /// bound to the commit-file list with a query.
    pub(crate) fn commit_file_search_match_ids(&self) -> HashSet<String> {
        if self.search_target != SearchPane::CommitFiles || self.search_query.trim().is_empty() {
            return HashSet::new();
        }
        let Some((rows, hits)) = self.commit_file_search_hits() else {
            return HashSet::new();
        };
        hits.into_iter().map(|i| rows[i].id.clone()).collect()
    }

    /// Position of the focused commit-file row (file or folder) in `rows`.
    fn commit_file_search_anchor(&self, rows: &[CommitFileRow]) -> Option<usize> {
        let focused = self.focused_commit_file_row()?;
        rows.iter().position(|row| row.id == focused.id)
    }

    /// Commit-file search step from the focused row. Returns true when it
    /// wrapped. A landing (file or folder) unfolds the folders above it and
    /// puts the cursor on that row by id, so a folder and a file with the
    /// same path stay apart; a landed folder keeps its own fold.
    fn apply_commit_file_search(&mut self, dir: i32) -> bool {
        let Some(files) = self.commit_drill_files() else {
            self.set_search_status(false);
            return false;
        };
        let rows = self.unfolded_commit_file_rows(files);
        let anchor = self.commit_file_search_anchor(&rows);
        let Some(landing) = focus_commit_file_search(&rows, &self.search_query, anchor, dir) else {
            self.set_search_status(false);
            return false;
        };
        let target = &rows[landing.target];
        for id in ancestor_dir_ids(&target.path) {
            self.commit_file_folds.remove(&id);
        }
        if let Some(idx) = self
            .commit_file_rows()
            .iter()
            .position(|row| row.id == target.id)
        {
            self.set_commit_file_cursor(idx);
        }
        self.set_search_status(true);
        landing.wrapped
    }

    /// Diff search step from the diff cursor. Returns true when it wrapped.
    fn apply_diff_search(&mut self, dir: i32) -> bool {
        let rows = self.current_diff_rows();
        let texts: Vec<String> = rows.iter().map(row_search_text).collect();
        let Some(landing) = focus_diff_search(&texts, &self.search_query, self.diff_cursor, dir)
        else {
            self.search_hit = None;
            self.set_search_status(false);
            return false;
        };
        self.search_hit = Some(landing.target);
        self.diff_cursor = landing.target;
        self.sync_diff_scroll_for(&rows);
        self.set_search_status(true);
        landing.wrapped
    }

    fn file_context_id(repo: &str, path: &str) -> String {
        format!("file:{repo}:{path}")
    }

    fn commit_context_id(repo: &str, path: &str) -> String {
        format!("commit:{repo}:{path}")
    }

    fn displayed_diff_id(&self) -> Option<String> {
        if self.is_explorer_tab() {
            return self
                .explorer_diff()
                .map(|(checkout, rel, _)| Self::file_context_id(checkout, rel));
        }
        if let Some(tab) = self.tabs.active_compare() {
            return tab
                .path
                .as_ref()
                .map(|path| Self::commit_context_id(&tab.checkout_path, path));
        }
        match &self.drill {
            DrillView::Diff { repo, path, .. } => Some(Self::commit_context_id(repo, path)),
            DrillView::Graph => {
                let (repo, file) = self.focused_file()?;
                Some(Self::file_context_id(&repo, &file.path))
            }
            DrillView::Files { .. } => None,
        }
    }

    /// `Some(n)` means `-Un` for this identity. `None` is git's default.
    pub fn diff_context_for(&self, id: &str) -> Option<u32> {
        if self.full_context.contains(id) {
            Some(FULL_DIFF_CONTEXT_LINES)
        } else {
            None
        }
    }

    /// Workspace dirty-file context, independent of pane focus.
    pub fn workspace_diff_context(&self, repo: &str, path: &str) -> Option<u32> {
        self.diff_context_for(&Self::file_context_id(repo, path))
    }

    /// Commit-file (or stash-file) context, independent of pane focus.
    pub fn commit_diff_context(&self, repo: &str, path: &str) -> Option<u32> {
        self.diff_context_for(&Self::commit_context_id(repo, path))
    }

    /// Context for the file currently shown in the right pane.
    /// Survives tree focus so a reload does not drop unlimited `-U`. An
    /// Explorer diff always loads the whole file.
    pub fn diff_context_lines(&self) -> Option<u32> {
        if self.is_explorer_tab() {
            return self.explorer_diff().map(|_| FULL_DIFF_CONTEXT_LINES);
        }
        let id = self.displayed_diff_id()?;
        self.diff_context_for(&id)
    }

    pub fn full_context_active(&self) -> bool {
        self.diff_context_lines().is_some()
    }

    fn toggle_full_context(&mut self) -> Effect {
        if !self.right_is_diff() {
            return Effect::None;
        }
        let Some(id) = self.displayed_diff_id() else {
            return Effect::None;
        };
        let rows = self.current_diff_rows();
        self.pending_hunk_anchor = Some(anchor_row_text(
            &rows,
            self.diff_cursor,
            self.diff_body_height(),
        ));
        if !self.full_context.remove(&id) {
            self.full_context.insert(id);
        }
        if let Some(tab) = self.tabs.active_compare() {
            let Some(source) = tab.source.clone() else {
                return Effect::None;
            };
            let Some(path) = tab.path.clone() else {
                return Effect::None;
            };
            let old_path = self.focused_compare_old_path();
            return Effect::LoadCompareDiff {
                tab_id: tab.id,
                repo: tab.checkout_path.clone(),
                source,
                path,
                old_path,
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

    fn apply_pending_hunk_anchor(&mut self) {
        let Some(needle) = self.pending_hunk_anchor.take() else {
            return;
        };
        let rows = self.current_diff_rows();
        let idx = find_anchor_row(&rows, &needle);
        self.diff_cursor = idx;
        self.sync_diff_scroll();
    }

    fn focused_file_if_shown(&self) -> Option<(String, FileChange)> {
        let row = self.focused_row()?;
        if row.kind != NodeKind::File {
            return None;
        }
        if row.hidden && !self.show_hidden {
            return None;
        }
        Some((row.repo.clone()?, row.file.clone()?))
    }

    fn stage_effect(&mut self) -> Effect {
        if self.diff_visual_anchor.is_some() {
            self.diff_visual_write_effect(FileWrite::Stage)
        } else {
            self.file_write_effect(FileWrite::Stage)
        }
    }

    fn unstage_effect(&mut self) -> Effect {
        if self.diff_visual_anchor.is_some() {
            self.diff_visual_write_effect(FileWrite::Unstage)
        } else {
            self.file_write_effect(FileWrite::Unstage)
        }
    }

    fn diff_visual_write_effect(&mut self, write: FileWrite) -> Effect {
        let Some(anchor) = self.diff_visual_anchor else {
            return self.file_write_effect(write);
        };
        let kind = match write {
            FileWrite::Stage => PartialPatchKind::Stage,
            FileWrite::Unstage => PartialPatchKind::Unstage,
        };
        let (repo, path, patch) = match self.visual_patch(anchor, kind) {
            Ok(built) => built,
            Err(err) => {
                self.status = StatusMessage::warn(err);
                return Effect::None;
            }
        };
        let reverse = matches!(write, FileWrite::Unstage);
        let verb = if reverse { "unstaging" } else { "staging" };
        self.status = StatusMessage::progress(format!("{verb} range {path}…"));
        self.clear_diff_visual();
        Effect::ApplyCachedPatch {
            repo,
            path,
            patch,
            reverse,
        }
    }

    /// Repo, path, and patch for the DiffVisual range, or the refusal text.
    ///
    /// On a compare tab only Revert builds a patch: it becomes
    /// [`PartialPatchKind::RevertCommitted`] on the loaded compare diff, and
    /// only when [`Self::compare_revert_target`] allows the write.
    fn visual_patch(
        &self,
        anchor: usize,
        kind: PartialPatchKind,
    ) -> Result<(String, String, String), String> {
        let (repo, path, kind) = if self.is_compare_tab() {
            if kind != PartialPatchKind::Revert {
                return Err(self.visual_write_refuse_status(kind));
            }
            let target = self.compare_revert_target()?;
            (target.repo, target.path, PartialPatchKind::RevertCommitted)
        } else {
            let Some((repo, path)) = self.visual_write_target() else {
                return Err(self.visual_write_refuse_status(kind));
            };
            (repo, path, kind)
        };
        let patch = build_partial_patch(
            self.current_diff_content(),
            self.diff_layout(),
            anchor,
            self.diff_cursor,
            kind,
            &path,
        )?;
        Ok((repo, path, patch))
    }

    /// Repo and path for a Workspace DiffVisual cached patch, or `None`
    /// to refuse. A compare tab answers from
    /// [`Self::compare_revert_target`] instead (see [`Self::visual_patch`]).
    ///
    /// Every `DrillView::Diff` returns `None` (commit, stash, and worktree
    /// drills). That is the only write stop for those views:
    /// their content leaves `is_committed: false` (`DiffContent::from_commit_lines`
    /// only relabels the section `COMMITTED`). Workspace
    /// file diffs (`set_diff`) keep `DrillView::Graph` and still emit
    /// `Effect::ApplyCachedPatch`.
    fn visual_write_target(&self) -> Option<(String, String)> {
        if self.is_compare_tab() {
            return None;
        }
        match &self.drill {
            DrillView::Diff { .. } => None,
            _ => Some((self.diff_repo.clone()?, self.diff_path.clone()?)),
        }
    }

    fn visual_write_refuse_status(&self, kind: PartialPatchKind) -> String {
        let verb = match kind {
            PartialPatchKind::Stage => "stage",
            PartialPatchKind::Unstage => "unstage",
            PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => "revert",
        };
        let target = match &self.drill {
            DrillView::Diff {
                source: CommitFileSource::Stash { .. },
                ..
            } => "a stash diff",
            DrillView::Diff {
                source: CommitFileSource::Worktree,
                ..
            } => "an uncommitted drill diff",
            _ => "a committed diff",
        };
        format!("cannot {verb} {target}")
    }

    fn file_write_effect(&mut self, write: FileWrite) -> Effect {
        let scoped = collect_write_files(&self.snapshot, self.focused_row(), self.show_hidden);
        let selected: Vec<ScopedFile> = scoped
            .into_iter()
            .filter(|file| match write {
                FileWrite::Stage => is_stageable(&file.change),
                FileWrite::Unstage => is_unstageable(&file.change),
            })
            .collect();
        if selected.is_empty() {
            let kind = self.focused_row().map(|row| row.kind);
            self.status = StatusMessage::warn(empty_write_status(kind, write));
            return Effect::None;
        }
        let groups = group_write_paths(&selected);
        let total: usize = groups.iter().map(|(_, paths)| paths.len()).sum();
        let verb = match write {
            FileWrite::Stage => "staging",
            FileWrite::Unstage => "unstaging",
        };
        self.status = StatusMessage::progress(if total == 1 {
            format!("{verb} {}…", groups[0].1[0])
        } else {
            format!("{verb} {total} files…")
        });
        let effects: Vec<Effect> = groups
            .into_iter()
            .map(|(repo, paths)| match write {
                FileWrite::Stage => Effect::Stage { repo, paths },
                FileWrite::Unstage => Effect::Unstage { repo, paths },
            })
            .collect();
        single_or_batch(effects)
    }

    /// The compare-tab file that `x` would revert, or why it may not.
    ///
    /// A commit-vs-working-tree tab refuses first (`cannot revert a
    /// working-tree compare`), then a tab with a pinned head (`cannot
    /// revert a committed diff`).
    /// With a `V` highlight, or with the diff focused, the target is the
    /// open diff, and only once its content is loaded for the current
    /// range and path, and never while a folder summary hides it. On the
    /// file list it is the focused file row.
    /// [`Self::compare_head_is_worktree`] must then hold for the file.
    pub(crate) fn compare_revert_target(&self) -> Result<CompareRevertTarget, String> {
        let Some(tab) = self.tabs.active_compare() else {
            return Err(SWITCH_TO_WORKSPACE_TAB.into());
        };
        // The new side already is the working tree: nothing to restore.
        if tab.worktree_file.is_some() {
            return Err(CANNOT_REVERT_WORKTREE_COMPARE.into());
        }
        // A pinned head is a commit in history, never the working tree.
        if tab.is_pinned() {
            return Err(CANNOT_REVERT_COMMITTED_DIFF.into());
        }
        if tab.loading {
            return Err(COMPARE_STILL_LOADING.into());
        }
        let Some(CommitFileSource::Compare {
            base_ref,
            merge_base,
            head,
            ..
        }) = tab.source.as_ref()
        else {
            return Err(COMPARE_STILL_LOADING.into());
        };
        let path = if self.diff_visual_anchor.is_some() || !self.commit_files_list_focused() {
            if self.folder_summary().is_some() {
                return Err(FOCUS_A_FILE_DIFF.into());
            }
            tab.loaded_diff_path()
                .ok_or(COMPARE_STILL_LOADING)?
                .to_string()
        } else {
            match self.focused_commit_file_row() {
                Some(row) if row.is_file() => row.path,
                _ => return Err(COMPARE_FOCUS_A_FILE.into()),
            }
        };
        let Some(file) = tab.files.iter().find(|file| file.path == path) else {
            return Err(COMPARE_STILL_LOADING.into());
        };
        let target = CompareRevertTarget {
            repo: tab.checkout_path.clone(),
            path,
            old_path: file.old_path.clone(),
            status: file.status.clone(),
            base_ref: base_ref.clone(),
            merge_base: merge_base.clone(),
            head: head.clone(),
        };
        self.compare_head_is_worktree(&target)?;
        Ok(target)
    }

    /// `Ok` when the compare head is the checked-out working tree for
    /// `target`: the checkout's HEAD is still `target.head`, and neither
    /// `path` nor a rename's `old_path` has a change against HEAD.
    ///
    /// Reads the snapshot, which can be a watch tick old. The write worker
    /// checks both again with git right before it writes.
    fn compare_head_is_worktree(&self, target: &CompareRevertTarget) -> Result<(), String> {
        let Some(snap) = self
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == target.repo)
        else {
            return Err(COMPARE_HEAD_MOVED.into());
        };
        if snap.head != target.head {
            return Err(COMPARE_HEAD_MOVED.into());
        }
        let paths = [Some(target.path.as_str()), target.old_path.as_deref()];
        let touches = |path: &str| paths.contains(&Some(path));
        let dirty = snap.changes.iter().find(|change| {
            touches(&change.path) || change.old_path.as_deref().is_some_and(touches)
        });
        match dirty {
            Some(change) => Err(compare_file_dirty(&change.path)),
            None => Ok(()),
        }
    }

    /// Why highlighted `x` would refuse, or `None` when it would open the
    /// range confirm.
    ///
    /// The same checks, in the same order, as the palette row "Revert
    /// highlighted lines": the compare gate, then the range patch. The
    /// VISUAL hint row uses it so it never offers an `x` that refuses.
    pub(crate) fn highlight_revert_refusal(&self) -> Option<String> {
        let Some(anchor) = self.diff_visual_anchor else {
            return Some("highlight diff lines first (V)".into());
        };
        self.compare_refusal(&super::action::Action::Revert)
            .or_else(|| self.visual_patch(anchor, PartialPatchKind::Revert).err())
    }

    /// Why `x` with no highlight would refuse on a compare tab, or `None`
    /// when it would open the whole-file confirm.
    ///
    /// The same checks, in the same order, as [`Self::begin_compare_revert`]
    /// without a highlight: the compare gate, then the change kind. The idle
    /// hint row uses it so it never offers an `x` that refuses.
    pub(crate) fn compare_file_revert_refusal(&self) -> Option<String> {
        match self.compare_revert_target() {
            Ok(target) => compare_file_revert_kind_refusal(&target.status),
            Err(err) => Some(err),
        }
    }

    /// Compare-tab `x`: open the range or whole-file revert confirm.
    ///
    /// A refusal sets the status and keeps any highlight. Opening the
    /// confirm clears the highlight.
    fn begin_compare_revert(&mut self) -> Effect {
        let target = match self.compare_revert_target() {
            Ok(target) => target,
            Err(err) => {
                self.status = StatusMessage::warn(err);
                return Effect::None;
            }
        };
        if let Some(anchor) = self.diff_visual_anchor {
            match self.visual_patch(anchor, PartialPatchKind::Revert) {
                Ok((_, _, patch)) => {
                    self.clear_diff_visual();
                    self.confirm = Some(PendingConfirm::CompareRevertRange { target, patch });
                }
                Err(err) => self.status = StatusMessage::warn(err),
            }
            return Effect::None;
        }
        if let Some(err) = compare_file_revert_kind_refusal(&target.status) {
            self.status = StatusMessage::warn(err);
            return Effect::None;
        }
        self.confirm = Some(PendingConfirm::CompareRevertFile { target });
        Effect::None
    }

    fn begin_revert(&mut self) -> Effect {
        if self.is_compare_tab() {
            return self.begin_compare_revert();
        }
        if let Some(anchor) = self.diff_visual_anchor {
            return self.begin_revert_range(anchor);
        }
        let scoped = collect_write_files(&self.snapshot, self.focused_row(), self.show_hidden);
        let selected: Vec<ScopedFile> = scoped
            .into_iter()
            .filter(|file| is_revertible(&file.change))
            .collect();
        if selected.is_empty() {
            let staged_only = self.focused_file_if_shown().is_some_and(|(_, change)| {
                change.staged_status.is_some()
                    && change.unstaged_status.is_none()
                    && !change.untracked
            });
            self.status = StatusMessage::warn(if staged_only {
                "nothing to discard (staged only)"
            } else if matches!(
                self.focused_row().map(|row| row.kind),
                Some(
                    NodeKind::File
                        | NodeKind::Dir
                        | NodeKind::Repo
                        | NodeKind::Checkout
                        | NodeKind::Section
                )
            ) {
                "nothing to discard"
            } else {
                "focus a file, dir, checkout, or repo to revert"
            });
            return Effect::None;
        }
        let targets: Vec<RevertTarget> = selected
            .iter()
            .map(|file| RevertTarget {
                repo: file.repo.clone(),
                path: file.change.path.clone(),
                untracked: file.change.untracked,
                old_path: file.change.old_path.clone(),
            })
            .collect();
        let label = self
            .focused_row()
            .map(|row| {
                if row.chrome.path.is_empty() {
                    row.label.clone()
                } else {
                    row.chrome.path.clone()
                }
            })
            .unwrap_or_else(|| targets.first().map(|t| t.path.clone()).unwrap_or_default());
        self.confirm = Some(PendingConfirm::Revert { targets, label });
        Effect::None
    }

    /// DiffVisual `x`: build the reverse patch, then open its confirm.
    ///
    /// A refusal keeps the highlight and sets the status. Opening the
    /// confirm clears the highlight.
    fn begin_revert_range(&mut self, anchor: usize) -> Effect {
        match self.visual_patch(anchor, PartialPatchKind::Revert) {
            Ok((repo, path, patch)) => {
                self.clear_diff_visual();
                self.confirm = Some(PendingConfirm::RevertRange { repo, path, patch });
            }
            Err(err) => self.status = StatusMessage::warn(err),
        }
        Effect::None
    }

    fn confirm_yes(&mut self, clean: bool) -> Effect {
        match self.confirm.take() {
            Some(PendingConfirm::Revert { targets, label }) => {
                if targets.is_empty() {
                    return Effect::None;
                }
                // A key the box does not show keeps the confirm open.
                let Some(delete_untracked) = revert_scope(&targets).key_deletes_untracked(clean)
                else {
                    self.confirm = Some(PendingConfirm::Revert { targets, label });
                    return Effect::None;
                };
                let groups = group_revert_targets(&targets, delete_untracked);
                let tracked_n: usize = groups.iter().map(|(_, tracked, _)| tracked.len()).sum();
                let untracked_n: usize =
                    groups.iter().map(|(_, _, untracked)| untracked.len()).sum();
                self.status = StatusMessage::progress(if tracked_n + untracked_n == 1 {
                    if untracked_n == 1 {
                        format!("deleting {}…", groups[0].2[0])
                    } else {
                        format!("reverting {}…", groups[0].1[0])
                    }
                } else if tracked_n == 0 {
                    format!("deleting {untracked_n} untracked…")
                } else {
                    format!("reverting {tracked_n} tracked, {untracked_n} untracked…")
                });
                let effects: Vec<Effect> = groups
                    .into_iter()
                    .map(|(repo, tracked, untracked)| Effect::Revert {
                        repo,
                        tracked,
                        untracked,
                    })
                    .collect();
                single_or_batch(effects)
            }
            // The range confirm offers only y / n: `Y` keeps it open.
            Some(pending @ PendingConfirm::RevertRange { .. }) if clean => {
                self.confirm = Some(pending);
                Effect::None
            }
            Some(PendingConfirm::RevertRange { repo, path, patch }) => {
                self.status = StatusMessage::progress(format!("reverting range {path}…"));
                Effect::RevertPatch { repo, path, patch }
            }
            Some(PendingConfirm::StashDrop { repo, stash_ref }) => {
                self.status = StatusMessage::progress(format!("dropping {stash_ref}…"));
                Effect::StashDrop { repo, stash_ref }
            }
            Some(PendingConfirm::CheckoutOutOfSync {
                repo,
                branch,
                remote_ref,
                ..
            }) => {
                self.status = StatusMessage::progress(format!(
                    "checking out {branch}, then fast-forwarding to {remote_ref}…"
                ));
                Effect::CheckoutBranch {
                    repo,
                    selected_name: branch,
                    fast_forward_ref: Some(remote_ref),
                }
            }
            Some(PendingConfirm::RemoveWorktree {
                primary,
                path,
                force,
                ..
            }) => {
                self.status = StatusMessage::progress(format!("removing worktree {path}…"));
                Effect::RemoveWorktree {
                    primary,
                    path,
                    force,
                }
            }
            Some(PendingConfirm::MergeIntoHead {
                repo, rev, label, ..
            }) => {
                self.status = StatusMessage::progress(format!("merging {label}…"));
                Effect::MergeIntoHead { repo, rev, label }
            }
            Some(PendingConfirm::SwitchToDefault { repos }) => {
                self.status = StatusMessage::progress(format_running_op(
                    RunningOp::DefaultBranch,
                    0,
                    repos.len(),
                ));
                Effect::DefaultBranch { repos }
            }
            // The push confirm offers only y / n: `Y` keeps it open.
            Some(pending @ PendingConfirm::Push { .. }) if clean => {
                self.confirm = Some(pending);
                Effect::None
            }
            Some(PendingConfirm::Push { repo, .. }) => {
                self.status = StatusMessage::progress(format_running_op(RunningOp::Push, 0, 1));
                Effect::Push { repos: vec![repo] }
            }
            // The reset confirm offers only y / n: `Y` keeps it open.
            Some(pending @ PendingConfirm::ResetToUpstream { .. }) if clean => {
                self.confirm = Some(pending);
                Effect::None
            }
            Some(PendingConfirm::ResetToUpstream {
                repo,
                branch,
                upstream,
                tip,
                ..
            }) => {
                self.status = StatusMessage::progress(format!("moving {branch} to {upstream}…"));
                Effect::ResetToUpstream {
                    repo,
                    branch,
                    upstream,
                    tip,
                }
            }
            // Compare confirms offer only y / n: `Y` keeps them open.
            Some(pending) if pending.is_compare_owned() && clean => {
                self.confirm = Some(pending);
                Effect::None
            }
            // A compare confirm runs only on the compare tab of its checkout.
            Some(pending)
                if pending.is_compare_owned()
                    && self.tabs.active_compare().is_none_or(|tab| {
                        Some(tab.checkout_path.as_str()) != compare_repo(&pending)
                    }) =>
            {
                self.status = "revert cancelled".into();
                Effect::None
            }
            Some(PendingConfirm::CompareRevertRange { target, patch }) => {
                self.status = StatusMessage::progress(format!(
                    "reverting range {} to merge base…",
                    target.path
                ));
                Effect::CompareRevertPatch {
                    repo: target.repo,
                    path: target.path,
                    head: target.head,
                    patch,
                }
            }
            Some(PendingConfirm::CompareRevertFile { target }) => {
                self.status =
                    StatusMessage::progress(format!("reverting {} to merge base…", target.path));
                Effect::CompareRevertFile {
                    repo: target.repo,
                    path: target.path,
                    old_path: target.old_path,
                    deletes: target.status == "A",
                    merge_base: target.merge_base,
                    head: target.head,
                }
            }
            None => Effect::None,
        }
    }

    fn toggle_reviewed(&mut self) -> Effect {
        if self.is_compare_tab() {
            return self.toggle_compare_reviewed();
        }
        if self.nav_depth() >= 1 {
            self.status = StatusMessage::warn(REVIEWED_MARKS_ARE_FOR_TREE_FILES);
            return Effect::None;
        }
        let Some(row) = self
            .focused_row()
            .filter(|row| row.kind == NodeKind::File)
            .cloned()
        else {
            self.status = StatusMessage::warn(FOCUS_A_FILE_TO_MARK_REVIEWED);
            return Effect::None;
        };
        let Some(repo) = row.repo.as_deref() else {
            return Effect::None;
        };
        let Some(file) = row.file.as_ref() else {
            return Effect::None;
        };
        self.toggle_viewed_for(repo, &file.path);
        Effect::None
    }

    /// Toggle the persisted reviewed mark of changed file `path` in
    /// checkout `repo` and ask for a store save. Returns the new mark
    /// (`true` when now reviewed), or `None` when the snapshot lists no
    /// change for it or no status result read its facts yet (nothing
    /// changes).
    ///
    /// The mark takes the fingerprint the last status result read for the
    /// file ([`Self::set_file_facts`]), not a new read of the worktree.
    pub(super) fn toggle_viewed_for(&mut self, repo: &str, path: &str) -> Option<bool> {
        self.snapshot
            .repos
            .iter()
            .find(|r| r.repo == repo)
            .and_then(|r| r.changes.iter().find(|c| c.path == path))?;
        let fingerprint = self.file_facts.get(repo, path)?.fingerprint.clone();
        let identity = viewed_identity(repo, path);
        self.viewed_store = toggle_viewed(&self.viewed_store, &identity, &fingerprint);
        let marked = self.viewed_store.contains_key(&identity);
        self.viewed_sync.want(None);
        self.refresh_reviewed();
        Some(marked)
    }

    /// Recompute [`Self::reviewed`] from the viewed store and the file
    /// facts. No file I/O.
    fn refresh_reviewed(&mut self) {
        self.reviewed = if self.viewed_store.is_empty() {
            HashSet::new()
        } else {
            let current = self.file_facts.fingerprints(&self.snapshot);
            viewed_row_ids(&self.snapshot, &self.viewed_store, &current)
        };
    }

    /// Replace the disk facts of checkout `repo`, read by its status
    /// worker; `None` drops them (the checkout left the snapshot). Call it
    /// before [`Self::apply_watch_snapshot`] applies that status.
    pub(crate) fn set_file_facts(&mut self, repo: &str, facts: Option<RepoFileFacts>) {
        self.file_facts.set(repo, facts);
    }

    /// Read the disk facts of every dirty file of the snapshot on this
    /// thread, as the status workers would. Tests that change a worktree
    /// file and then apply a snapshot use it.
    #[cfg(test)]
    pub(crate) fn read_file_facts_from_disk(&mut self) {
        self.file_facts = FileFactsCache::from_disk(&self.cwd, &self.snapshot);
    }

    /// A save of store `kind` is asked for and none runs.
    pub(crate) fn store_save_ready(&self, kind: StoreKind) -> bool {
        match kind {
            StoreKind::Viewed => self.viewed_sync.ready(),
            StoreKind::Comments => self.comment_sync.ready(),
        }
    }

    /// A save of some store runs on a worker.
    pub(crate) fn store_save_in_flight(&self) -> bool {
        self.viewed_sync.in_flight() || self.comment_sync.in_flight()
    }

    /// A save of some store is asked for or runs.
    pub(crate) fn store_saves_pending(&self) -> bool {
        self.viewed_sync.pending() || self.comment_sync.pending()
    }

    /// Ask again for the save of store `kind` whose last save failed, when
    /// none is asked for or runs. Quit calls it once per store so changes
    /// a failed save kept still get one more try. Returns true when it
    /// asked.
    pub(crate) fn retry_failed_store_save(&mut self, kind: StoreKind) -> bool {
        match kind {
            StoreKind::Viewed => self.viewed_sync.retry_failed(),
            StoreKind::Comments => self.comment_sync.retry_failed(),
        }
    }

    /// The stores whose changes are not on disk at quit: a save still
    /// asked for or running when `timed_out` (the grace that ran out) is
    /// set, else a last save that failed.
    pub(crate) fn unsaved_stores(&self, timed_out: Option<Duration>) -> Vec<UnsavedStore> {
        StoreKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let (pending, failed) = match kind {
                    StoreKind::Viewed => (self.viewed_sync.pending(), self.viewed_sync.failed()),
                    StoreKind::Comments => {
                        (self.comment_sync.pending(), self.comment_sync.failed())
                    }
                };
                let reason = match (timed_out, failed) {
                    (Some(grace), _) if pending => UnsavedReason::TimedOut(grace),
                    (_, Some(err)) => UnsavedReason::Failed(err.to_string()),
                    _ => return None,
                };
                Some(UnsavedStore { kind, reason })
            })
            .collect()
    }

    /// Start the save of store `kind` asked for: the job that writes this
    /// session's changes since the last load or save. `None` unless
    /// [`Self::store_save_ready`]. The caller runs the job off the loop
    /// thread and lands it with [`Self::land_store_save`].
    pub(crate) fn begin_store_save(&mut self, kind: StoreKind) -> Option<StoreSaveJob> {
        let cwd = self.cwd.clone();
        match kind {
            StoreKind::Viewed => {
                let (baseline, store) = self.viewed_sync.begin(&self.viewed_store)?;
                Some(StoreSaveJob::Viewed {
                    baseline,
                    store,
                    path: self.viewed_path.clone(),
                    cwd,
                })
            }
            StoreKind::Comments => {
                let (baseline, store) = self.comment_sync.begin(&self.comment_store)?;
                Some(StoreSaveJob::Comments {
                    baseline,
                    store,
                    path: self.comment_path.clone(),
                    cwd,
                })
            }
        }
    }

    /// Land a store save. On success the merged bucket (with entries other
    /// `ws` processes saved) plus the changes made since the save started
    /// becomes the in-memory store, the merged bucket the new baseline, and
    /// an ack the save carried shows. On error the status names the
    /// failure, and the baseline and the changes stay, so the next save
    /// sends them again: the next change asks for one, and quit retries
    /// once ([`Self::retry_failed_store_save`]).
    pub(crate) fn land_store_save(&mut self, saved: StoreSaved) {
        let kind = saved.kind();
        let landed = match saved {
            StoreSaved::Viewed(result) => {
                let landed = self.viewed_sync.land(&mut self.viewed_store, result);
                self.refresh_reviewed();
                landed
            }
            StoreSaved::Comments(result) => self.comment_sync.land(&mut self.comment_store, result),
        };
        match landed {
            Ok(Some(ack)) => self.status = StatusMessage::ok(ack),
            Ok(None) => {}
            Err(err) => {
                self.status = StatusMessage::error(persist_failed_status(kind.label(), err));
            }
        }
    }

    /// Run every store save asked for on this thread, as the workers
    /// would. Tests that drive [`Self::dispatch`] without the interpreter
    /// use it.
    #[cfg(test)]
    pub(crate) fn flush_store_saves(&mut self) {
        for kind in StoreKind::ALL {
            while let Some(job) = self.begin_store_save(kind) {
                self.land_store_save(job.run());
            }
        }
    }

    /// Space on a compare tab: session-only mark on the focused committed file.
    fn toggle_compare_reviewed(&mut self) -> Effect {
        let Some((_, path)) = self.focused_commit_edit_path() else {
            self.status = StatusMessage::warn(FOCUS_A_FILE_TO_MARK_REVIEWED);
            return Effect::None;
        };
        if let Some(tab) = self.tabs.active_compare_mut() {
            tab.toggle_reviewed(&path);
        }
        Effect::None
    }

    /// Drop viewed marks whose file is clean or whose fingerprint (from the
    /// file facts) moved, and ask for a save when one dropped. A dirty file
    /// with no facts keeps its mark: its fingerprint is unknown, not
    /// changed. No file I/O.
    fn reconcile_viewed_store(&mut self) {
        if !self.viewed_store.is_empty() {
            let mut current = self.file_facts.fingerprints(&self.snapshot);
            for identity in self.file_facts.unknown_identities(&self.snapshot) {
                if let Some(marked) = self.viewed_store.get(&identity) {
                    current.insert(identity, marked.clone());
                }
            }
            let next = reconcile_viewed(&self.viewed_store, &current);
            if next != self.viewed_store {
                self.viewed_store = next;
                self.viewed_sync.want(None);
            }
        }
        self.refresh_reviewed();
    }

    fn reconcile_comment_store(&mut self) {
        let live = collect_live_set(
            &self.snapshot,
            &self.comment_store,
            &self.comment_checkout_owners,
        );
        let next = gc_comments(&self.comment_store, &live, &self.cwd);
        self.comment_checkout_owners = live.checkout_owners;
        if next != self.comment_store {
            self.comment_store = next;
            self.comment_sync.want(None);
        }
    }

    /// Source line span a comment on the file diff targets
    /// ([`comment_line_span`] of the diff cursor and `V` anchor over
    /// [`Self::current_diff_rows`]), memoised by content fingerprint,
    /// layout, cursor, and anchor. The Comment gate runs on every frame a
    /// line-comment popover paints; the diff pane primes the memo from the
    /// rows it built ([`Self::prime_cursor_comment_lines`]), so that frame
    /// builds the rows once. `None` under a folder summary, like the rows.
    fn cursor_comment_lines(&self) -> Option<(u32, u32)> {
        if self.folder_summary().is_some() {
            return None;
        }
        let key = self.comment_lines_key();
        if let Some((hit_key, hit)) = self.comment_lines_memo.borrow().as_ref() {
            if *hit_key == key {
                return *hit;
            }
        }
        let hit = comment_line_span(&self.current_diff_rows(), key.2, key.3);
        *self.comment_lines_memo.borrow_mut() = Some((key, hit));
        hit
    }

    /// Fill the [`Self::cursor_comment_lines`] memo from `rows`, this
    /// frame's [`Self::current_diff_rows`], so a gate asked later in the
    /// frame reads it instead of building the rows again.
    pub(crate) fn prime_cursor_comment_lines(&self, rows: &[DiffRow]) {
        if self.folder_summary().is_some() {
            return;
        }
        let key = self.comment_lines_key();
        let hit = comment_line_span(rows, key.2, key.3);
        *self.comment_lines_memo.borrow_mut() = Some((key, hit));
    }

    /// Key of the [`Self::cursor_comment_lines`] memo.
    fn comment_lines_key(&self) -> (u64, DiffMode, usize, Option<usize>) {
        (
            self.current_diff_content().syntax_fingerprint(),
            self.diff_layout(),
            self.diff_cursor,
            self.diff_visual_anchor,
        )
    }

    fn current_comment_target(&self) -> Option<CommentKey> {
        if self.tabs.active_explorer().is_some() {
            if let Some(key) = self.explorer_file_comment_target() {
                return key;
            }
        }
        match self.list_focus_target() {
            ListFocusTarget::None => {
                let (repo, path, source) = self.open_diff_target()?;
                let visual = self.diff_visual_anchor.is_some();
                let (line, end_line) = self.cursor_comment_lines()?;
                let key = resolve_comment_target(
                    &self.snapshot,
                    None,
                    None,
                    None,
                    true,
                    Some(repo),
                    Some(path),
                    source,
                    Some(line),
                    Some(end_line),
                )?;
                if visual {
                    Some(key)
                } else {
                    Some(covering_line_comment(&self.comment_store, &key).unwrap_or(key))
                }
            }
            ListFocusTarget::Graph => {
                let repo = self.focused_graph_repo();
                let row = self.focused_graph_row();
                resolve_comment_target(
                    &self.snapshot,
                    None,
                    row.as_ref(),
                    repo.as_deref(),
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                )
            }
            ListFocusTarget::Tree => resolve_comment_target(
                &self.snapshot,
                self.focused_row(),
                None,
                None,
                false,
                None,
                None,
                None,
                None,
                None,
            ),
            ListFocusTarget::CommitFiles => None,
        }
    }

    fn current_entity_reference(&self) -> Option<EntityRef> {
        // A focused Explorer diff names its line, as on the Workspace.
        let explorer_diff_line =
            self.list_focus_target() == ListFocusTarget::None && self.explorer_diff().is_some();
        if let Some(tab) = self.tabs.active_explorer().filter(|_| !explorer_diff_line) {
            // A `V` range on a clean file copies its span, as a diff range
            // does: the working-tree file is the new side.
            if let (Some((lo, hi)), Some(file)) =
                (self.file_visual_range(), self.tabs.active_file_view())
            {
                let line = |n: usize| u32::try_from(n + 1).ok();
                return resolve_entity_reference(
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
                    DiffSide::New,
                    None,
                    None,
                    None,
                );
            }
            let primary = self
                .snapshot
                .repos
                .iter()
                .find(|repo| repo.repo == tab.checkout)
                .and_then(|repo| repo.primary_repo.as_deref());
            let rel = self.explorer_named_file()?;
            return Some(EntityRef::file(&tab.checkout, primary, rel));
        }
        match self.list_focus_target() {
            ListFocusTarget::None => {
                let (repo, path, source) = self.open_diff_target()?;
                let rows = self.current_diff_rows();
                let (start, end) = if let Some(anchor) = self.diff_visual_anchor {
                    (anchor, self.diff_cursor)
                } else {
                    (self.diff_cursor, self.diff_cursor)
                };
                let (line, end_line) = if self.diff_visual_anchor.is_some() {
                    viewport_line_range(&rows, start, end)?
                } else {
                    let line = viewport_line_number(&rows, self.diff_cursor as u16)?;
                    (line, line)
                };
                resolve_entity_reference(
                    &self.snapshot,
                    None,
                    None,
                    None,
                    true,
                    Some(repo),
                    Some(path),
                    source,
                    Some(line),
                    Some(end_line),
                    diff_focus_side(&rows, start, end),
                    None,
                    None,
                    None,
                )
            }
            ListFocusTarget::Graph => {
                let repo = self.focused_graph_repo();
                let row = self.focused_graph_row();
                resolve_entity_reference(
                    &self.snapshot,
                    None,
                    row.as_ref(),
                    repo.as_deref(),
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    DiffSide::Unified,
                    None,
                    None,
                    None,
                )
            }
            ListFocusTarget::Tree => {
                let cwd = self.cwd.to_string_lossy();
                resolve_entity_reference(
                    &self.snapshot,
                    self.focused_row(),
                    None,
                    None,
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    DiffSide::Unified,
                    None,
                    None,
                    Some(cwd.as_ref()),
                )
            }
            ListFocusTarget::CommitFiles => {
                let compare_base = self.tabs.active_compare().map(|tab| tab.base_ref.clone());
                let repo = if let Some(tab) = self.tabs.active_compare() {
                    Some(tab.checkout_path.as_str())
                } else {
                    match &self.drill {
                        DrillView::Files { repo, .. } | DrillView::Diff { repo, .. } => {
                            Some(repo.as_str())
                        }
                        DrillView::Graph => None,
                    }
                };
                let row = self.focused_commit_file_row();
                resolve_entity_reference(
                    &self.snapshot,
                    None,
                    None,
                    None,
                    false,
                    None,
                    None,
                    None,
                    None,
                    None,
                    DiffSide::Unified,
                    row.as_ref(),
                    repo,
                    None,
                )
                .map(|entity| entity.with_compare_base(compare_base.as_deref()))
            }
        }
    }

    fn copy_entity_reference(&mut self) -> Effect {
        let Some(entity) = self.current_entity_reference() else {
            self.status = StatusMessage::warn("no copy target");
            return Effect::None;
        };
        let text = format_entity_reference(&entity);
        Effect::CopyClipboard {
            text,
            announce: true,
        }
    }

    pub(crate) fn clear_diff_visual(&mut self) {
        self.diff_visual_anchor = None;
        self.diff_visual_rows = None;
    }

    /// Drop visual-line highlight when the painted row list changed.
    pub(crate) fn drop_stale_diff_visual(&mut self) {
        if self.diff_visual_anchor.is_none() {
            return;
        }
        let current = self.current_diff_rows();
        match self.diff_visual_rows.as_ref() {
            Some(snapshot) if snapshot == &current => {}
            _ => self.clear_diff_visual(),
        }
    }

    /// True when visual-line highlight includes painted diff row `idx`.
    pub fn diff_visual_contains(&self, idx: usize) -> bool {
        let Some(anchor) = self.diff_visual_anchor else {
            return false;
        };
        let lo = anchor.min(self.diff_cursor);
        let hi = anchor.max(self.diff_cursor);
        idx >= lo && idx <= hi
    }

    fn begin_diff_visual(&mut self) -> Effect {
        if self.list_focus_target() != ListFocusTarget::None {
            return Effect::None;
        }
        // A clean Explorer file anchors on a file line index, a diff on
        // its row.
        let anchor = match self.tabs.active_file_view() {
            Some(file) => (!file.lines().is_empty()).then_some(file.cursor),
            None => (!self.current_diff_rows().is_empty()).then_some(self.diff_cursor),
        };
        let Some(anchor) = anchor else {
            self.status = StatusMessage::warn(NO_HIGHLIGHT_TARGET);
            return Effect::None;
        };
        self.cancel_mouse_drag();
        self.help_open = false;
        self.comment_export = None;
        self.diff_visual_anchor = Some(anchor);
        self.diff_visual_rows = Some(self.current_diff_rows());
        self.status.clear();
        Effect::None
    }

    fn cancel_diff_visual(&mut self) -> Effect {
        if self.diff_visual_anchor.is_some() {
            self.clear_diff_visual();
            self.status.clear();
        }
        Effect::None
    }

    fn begin_comment(&mut self) -> Effect {
        self.cancel_mouse_drag();
        self.help_open = false;
        self.comment_export = None;
        let Some(key) = self.current_comment_target() else {
            self.status = StatusMessage::warn("no comment target");
            return Effect::None;
        };
        self.clear_diff_visual();
        let entry = self.comment_store.get(&key).cloned().unwrap_or_default();
        let label = comment_key_label(&key);
        self.comment =
            Some(CommentPrompt::new(key, entry.body, label).with_resolved(entry.resolved));
        self.status.clear();
        Effect::None
    }

    fn submit_comment(&mut self) -> Effect {
        let Some(prompt) = self.comment.take() else {
            return Effect::None;
        };
        let body = prompt.body();
        let empty = body.trim().is_empty();
        self.comment_store =
            put_comment_entry(&self.comment_store, prompt.key, &body, prompt.resolved);
        // The save runs on a worker; its landing shows the ack or the error.
        self.comment_sync.want(Some(if empty {
            "comment deleted"
        } else {
            "comment saved"
        }));
        Effect::None
    }

    fn export_comments(&mut self) -> Effect {
        self.cancel_mouse_drag();
        self.help_open = false;
        self.comment = None;
        self.reconcile_comment_store();
        let scoped = self.scoped_comment_store();
        if scoped.is_empty() {
            self.status = StatusMessage::info(STATUS_NO_COMMENTS);
            return Effect::None;
        }
        let markdown = export_markdown(&scoped);
        self.comment_export = Some(CommentExport {
            markdown: markdown.clone(),
            copied: None,
        });
        self.status.clear();
        Effect::CopyClipboard {
            text: markdown,
            announce: true,
        }
    }

    fn scoped_comment_store(&self) -> CommentStore {
        // An Explorer tab exports its active inner file, never the parked
        // Workspace row.
        if let Some(tab) = self.tabs.active_explorer() {
            let Some(inner) = tab.active_inner() else {
                return CommentStore::new();
            };
            let list = CommentExportList::Diff {
                repo: &tab.checkout,
                path: &inner.rel,
                source: None,
            };
            return comments_in_focus_scope(&self.comment_store, &self.snapshot, &self.tree, list);
        }
        let graph_repo = self.focused_graph_repo();
        let graph_row = self.focused_graph_row();
        let commit_file = self.focused_commit_file_row();
        let list = match self.list_focus_target() {
            ListFocusTarget::Tree => CommentExportList::Tree {
                row: self.focused_row(),
            },
            ListFocusTarget::Graph => CommentExportList::Graph {
                repo: graph_repo.as_deref(),
                row: graph_row.as_ref(),
            },
            ListFocusTarget::CommitFiles => {
                match (self.commit_drill_source(), commit_file.as_ref()) {
                    (Some((repo, source)), Some(row)) => CommentExportList::CommitFiles {
                        repo,
                        source,
                        path: row.path.as_str(),
                        is_dir: row.is_dir(),
                    },
                    // A compare tab never falls back to the parked Workspace row.
                    _ if self.is_compare_tab() => return CommentStore::new(),
                    _ => CommentExportList::Tree {
                        row: self.focused_row(),
                    },
                }
            }
            ListFocusTarget::None => match self.open_diff_target() {
                Some((repo, path, source)) => CommentExportList::Diff { repo, path, source },
                // A compare tab never falls back to the parked Workspace row.
                None if self.is_compare_tab() => return CommentStore::new(),
                None => CommentExportList::Tree {
                    row: self.focused_row(),
                },
            },
        };
        comments_in_focus_scope(&self.comment_store, &self.snapshot, &self.tree, list)
    }

    fn refuse_remove_worktree(&mut self) -> Effect {
        self.status = StatusMessage::warn("Focus a linked worktree to remove");
        Effect::None
    }

    fn begin_remove_worktree(&mut self) -> Effect {
        let Some(row) = self.focused_row() else {
            return self.refuse_remove_worktree();
        };
        if row_is_hidden(row, self.show_hidden) {
            return Effect::None;
        }
        if !matches!(row.kind, NodeKind::Checkout | NodeKind::Repo) {
            return self.refuse_remove_worktree();
        }
        let Some(repo_path) = row.repo.as_deref() else {
            return self.refuse_remove_worktree();
        };
        let Some(snap) = self.snapshot.repos.iter().find(|r| r.repo == repo_path) else {
            return self.refuse_remove_worktree();
        };
        if snap.checkout_kind != CheckoutKind::Linked {
            return self.refuse_remove_worktree();
        }
        let Some(primary) = snap.primary_repo.clone() else {
            return self.refuse_remove_worktree();
        };
        let force = snap.has_unstaged || snap.has_staged || snap.has_untracked;
        let path = snap.repo.clone();
        let branch = snap.branch.clone();
        let merged_into_default = snap.merged_into_default;
        let changed = snap.changes.len();
        self.confirm = Some(PendingConfirm::RemoveWorktree {
            primary,
            path,
            force,
            branch,
            merged_into_default,
            changed,
        });
        Effect::None
    }

    fn fetch_tick_effect(&mut self) -> Effect {
        let targets = background_fetch_targets(&self.snapshot, self.show_hidden);
        if targets.is_empty() {
            return Effect::None;
        }
        // The background tick never writes the status slot: a failure
        // shows as the scheduled-fetch outage.
        Effect::Fetch { repos: targets }
    }

    /// Use the shared background-fetch store at `path` and start from its
    /// current generation for this workspace, so an outcome recorded before
    /// launch never paints.
    pub(crate) fn open_background_fetch_store(&mut self, path: PathBuf) {
        self.fetch_applied_generation =
            load_background_fetch_record(&path, &workspace_store_id(&self.cwd))
                .map_or(0, |record| record.generation);
        self.fetch_store_path = path;
    }

    /// Apply, at `now`, a background-fetch outcome that another instance
    /// recorded since this one last applied or wrote one. Returns true
    /// when it did.
    ///
    /// The record has no repo or reason, so a failure starts (or updates)
    /// the scheduled-fetch outage with [`OTHER_INSTANCE_FETCH_FAILED`], and
    /// a success ends it ([`Self::note_background_fetch_outcome`]). The
    /// status slot is left alone. The generation already applied changes
    /// nothing. Any other generation applies, a lower one too: the store
    /// was deleted or reset.
    pub(crate) fn apply_background_fetch_record(&mut self, now: Instant) -> bool {
        let Some(record) =
            load_background_fetch_record(&self.fetch_store_path, &workspace_store_id(&self.cwd))
        else {
            return false;
        };
        if record.generation == self.fetch_applied_generation {
            return false;
        }
        let failure = record
            .failed
            .then(|| OTHER_INSTANCE_FETCH_FAILED.to_string());
        self.note_background_fetch_outcome(Some(record.generation), failure, now);
        true
    }

    /// Note, at `now`, how a scheduled background fetch ended.
    ///
    /// `generation`: the shared record this instance wrote or applied for
    /// it, now marked applied here (`None` when no record was written).
    /// `failure`: the outage toast text when a repo failed. It starts the
    /// [`OutageSource::ScheduledFetch`] outage, or updates its text while
    /// it goes on; `None` ends the outage. The status slot is left alone.
    pub(crate) fn note_background_fetch_outcome(
        &mut self,
        generation: Option<u64>,
        failure: Option<String>,
        now: Instant,
    ) {
        if let Some(generation) = generation {
            self.fetch_applied_generation = generation;
        }
        match failure {
            Some(text) => self.note_outage(OutageSource::ScheduledFetch, text, now),
            None => self.note_recovered(&OutageSource::ScheduledFetch),
        }
    }

    fn focused_checkout_if_shown(&self) -> Option<String> {
        let row = self.focused_row()?;
        if row_is_hidden(row, self.show_hidden) {
            return None;
        }
        checkout_path(row)
    }

    fn push_effect(&mut self) -> Effect {
        // `push_targets` holds at most the focused checkout. The confirm
        // opens once git names the branch and remote (`open_push_confirm`).
        let targets = push_targets(&self.snapshot, self.focused_row(), self.show_hidden);
        let Some(repo) = targets.into_iter().next() else {
            let on_checkout = self
                .focused_row()
                .is_some_and(|row| matches!(row.kind, NodeKind::Repo | NodeKind::Checkout));
            self.status = StatusMessage::warn(if on_checkout {
                "nothing to push"
            } else {
                "focus a repo or checkout to push"
            });
            return Effect::None;
        };
        Effect::PreparePush { repo }
    }

    /// Open the push confirm after git read the [`PushPlan`] for `repo`.
    ///
    /// `Err` (detached HEAD) puts the reason on the status line. A checkout
    /// the snapshot no longer shows as pushable says `nothing to push`.
    /// A confirm already open stays as is. Otherwise the box names the branch, the remote, and the commit
    /// count from the snapshot; `y` returns [`Effect::Push`].
    pub fn open_push_confirm(&mut self, repo: String, plan: Result<PushPlan, String>) {
        // Another confirm opened while git read the plan: keep that one.
        if self.confirm.is_some() {
            return;
        }
        let plan = match plan {
            Ok(plan) => plan,
            Err(reason) => {
                self.status = StatusMessage::error(reason);
                return;
            }
        };
        let Some(snap) = self
            .snapshot
            .repos
            .iter()
            .find(|r| r.repo == repo && snapshot_pushable(r))
        else {
            self.status = StatusMessage::warn("nothing to push");
            return;
        };
        let ahead_behind = push_ahead_behind(snap.sync_status, &snap.sync_note);
        self.help_open = false;
        self.cancel_mouse_drag();
        self.confirm = Some(PendingConfirm::Push {
            repo,
            branch: plan.branch,
            remote: plan.remote,
            publish: plan.publish,
            ahead_behind,
        });
    }

    fn begin_stash_menu(&mut self) -> Effect {
        if self.nav_depth() >= 2 {
            self.status = StatusMessage::warn(super::gates::STASH_NEEDS_TREE_OR_GRAPH);
            return Effect::None;
        }
        let Some(repo) = self.focused_checkout_if_shown() else {
            self.status = StatusMessage::warn("focus a visible repo to stash");
            return Effect::None;
        };
        self.help_open = false;
        Effect::PrepareStashMenu { repo }
    }

    /// Fill the stash overlay after git lists the latest stash.
    pub fn open_stash_menu(&mut self, repo: String, latest_stash_ref: Option<String>) {
        let focused_stash = self.focused_graph_stash_ref();
        let row = self.focused_row();
        let (dirty, dirty_paths) = if focused_stash.is_some() {
            let dirty = row
                .and_then(|r| r.repo.as_deref())
                .and_then(|repo| self.snapshot.repos.iter().find(|r| r.repo == repo))
                .map(|snap| snap.has_unstaged || snap.has_staged || snap.has_untracked)
                .unwrap_or(false);
            (dirty, None)
        } else {
            match row {
                Some(row) => stash_dirty_for_row(&self.snapshot, row),
                None => (false, None),
            }
        };
        // A tree row offers the latest stash only when its whole checkout
        // is clean, so apply / pop cannot land on top of local changes.
        let checkout_clean = !self
            .snapshot
            .repos
            .iter()
            .any(|r| r.repo == repo && (r.has_unstaged || r.has_staged || r.has_untracked));
        let latest_for_ops = if self.graph_pane_focused() || checkout_clean {
            latest_stash_ref
        } else {
            None
        };
        let ops = stash_ops_for_context(&StashOpsContext {
            dirty,
            dirty_paths,
            focused_stash_ref: focused_stash,
            latest_stash_ref: latest_for_ops,
        });
        if ops.is_empty() {
            self.stash_menu = None;
            self.stash_repo = None;
            self.status = StatusMessage::warn("nothing to stash");
            return;
        }
        // The box lists each op with its key; a chip summary would repeat it.
        self.status.clear();
        self.stash_repo = Some(repo);
        self.stash_menu = Some(ops);
    }

    fn stash_menu_key(&mut self, input: Option<char>, enter: bool, escape: bool) -> Effect {
        let Some(ops) = self.stash_menu.clone() else {
            return Effect::None;
        };
        match resolve_stash_menu_key(input, enter, escape, &ops) {
            StashMenuKeyResult::Cancel => {
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = "stash cancelled".into();
                Effect::None
            }
            StashMenuKeyResult::Ignore => Effect::None,
            StashMenuKeyResult::Run(op) => self.run_stash_op(op),
        }
    }

    fn run_stash_op(&mut self, op: StashOp) -> Effect {
        let Some(repo) = self.stash_repo.clone() else {
            return Effect::None;
        };
        match op.id {
            StashOpId::Create => {
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = "stash".into();
                Effect::StashCreate {
                    repo,
                    paths: op.paths.unwrap_or_default(),
                }
            }
            StashOpId::Apply => {
                let stash_ref = op.stash_ref.unwrap_or_else(|| "stash@{0}".into());
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = StatusMessage::progress(format!("applying {stash_ref}…"));
                Effect::StashApply { repo, stash_ref }
            }
            StashOpId::Pop => {
                let stash_ref = op.stash_ref.unwrap_or_else(|| "stash@{0}".into());
                self.stash_menu = None;
                self.stash_repo = None;
                self.status = StatusMessage::progress(format!("popping {stash_ref}…"));
                Effect::StashPop { repo, stash_ref }
            }
            StashOpId::Drop => {
                let stash_ref = op.stash_ref.unwrap_or_else(|| "stash@{0}".into());
                self.stash_menu = None;
                self.confirm = Some(PendingConfirm::StashDrop {
                    repo,
                    stash_ref: stash_ref.clone(),
                });
                Effect::None
            }
        }
    }

    fn begin_branch_picker(&mut self) -> Effect {
        let Some(row) = self.focused_row() else {
            self.status = StatusMessage::warn("focus a repo to pick a branch");
            return Effect::None;
        };
        if row_is_hidden(row, self.show_hidden) {
            self.status = StatusMessage::warn("focus a visible repo to pick a branch");
            return Effect::None;
        }
        if !can_open_branch_picker(&self.snapshot, row) {
            self.status = StatusMessage::warn("focus a checkout to pick a branch");
            return Effect::None;
        }
        let Some(repo) = checkout_path(row) else {
            self.status = StatusMessage::warn("focus a checkout to pick a branch");
            return Effect::None;
        };
        self.help_open = false;
        Effect::PrepareBranchPicker { repo }
    }

    /// Fill the branch picker after git lists local branches.
    pub fn open_branch_picker(&mut self, repo: String, branches: Vec<crate::git::LocalBranch>) {
        if branches.is_empty() {
            self.status = StatusMessage::warn("no local branches");
            self.branch_picker = None;
            return;
        }
        let default = self
            .snapshot
            .repos
            .iter()
            .find(|r| r.repo == repo)
            .and_then(|r| r.default_branch_override.clone());
        let sorted = super::branches::sort_branches_for_picker(branches, default.as_deref());
        self.branch_picker = Some(BranchPickerState::checkout(repo, sorted));
        self.status.clear();
    }

    /// Local branch names whose ancestors the current graph should show.
    pub(crate) fn graph_focus_revs(&self) -> Vec<String> {
        let owned = self.focused_graph_repo();
        let repo = self
            .graph_identity
            .as_ref()
            .map(|(repo, _)| repo.as_str())
            .or(owned.as_deref());
        let Some(repo) = repo else {
            return Vec::new();
        };
        match &self.graph_branch_focus {
            Some((focus_repo, names)) if focus_repo == repo && !names.is_empty() => names.clone(),
            _ => Vec::new(),
        }
    }

    fn graph_focus_repo(&self) -> Option<String> {
        self.graph_identity
            .as_ref()
            .map(|(repo, _)| repo.clone())
            .or_else(|| self.focused_graph_repo())
    }

    /// Checkout whose graph `o` should load.
    ///
    /// Graph list: the loaded graph (identity, else the tree row that owns
    /// it). Tree: the highlighted repo or worktree, not a stale identity
    /// from a previous row.
    pub(crate) fn graph_focus_picker_repo(&self) -> Option<String> {
        if self.graph_pane_focused() {
            return self.graph_focus_repo();
        }
        if self.list_focus_target() != ListFocusTarget::Tree {
            return None;
        }
        match self.focused_row().map(|row| row.kind) {
            Some(NodeKind::Repo | NodeKind::Checkout) => self.focused_graph_repo(),
            _ => None,
        }
    }

    /// True when `/` armed a non-empty query, so `n` / `N` step matches.
    pub(crate) fn search_is_armed(&self) -> bool {
        self.search_active && !self.search_query.trim().is_empty()
    }

    /// True when the current row has an applied graph branch focus.
    pub(crate) fn graph_focus_is_active(&self) -> bool {
        let Some(repo) = self.graph_focus_picker_repo() else {
            return false;
        };
        self.graph_branch_focus
            .as_ref()
            .is_some_and(|(focus_repo, names)| !names.is_empty() && focus_repo == &repo)
    }

    fn begin_graph_focus_picker(&mut self) -> Effect {
        let Some(repo) = self.graph_focus_picker_repo() else {
            return Effect::None;
        };
        self.help_open = false;
        Effect::PrepareGraphFocusPicker { repo }
    }

    /// Fill the graph focus overlay after git lists local branches.
    pub fn open_graph_focus_picker(
        &mut self,
        repo: String,
        branches: Vec<crate::git::LocalBranch>,
    ) {
        if branches.is_empty() {
            self.status = StatusMessage::warn("no local branches");
            self.graph_focus_picker = None;
            return;
        }
        let default = self
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .and_then(|row| row.default_branch_override.clone());
        let sorted = super::branches::sort_branches_for_picker(branches, default.as_deref());
        let preselected = match &self.graph_branch_focus {
            Some((focus_repo, names)) if focus_repo == &repo => names.as_slice(),
            _ => &[],
        };
        self.graph_focus_picker = Some(GraphFocusPickerState::new(repo, sorted, preselected));
        self.status.clear();
    }

    fn submit_graph_focus_picker(&mut self) -> Effect {
        let Some(picker) = self.graph_focus_picker.as_ref() else {
            return Effect::None;
        };
        let repo = picker.repo.clone();
        let Some(names) = picker.apply_names() else {
            self.status = StatusMessage::warn("no matching branches");
            return Effect::None;
        };
        self.graph_focus_picker = None;
        if names.is_empty() {
            return self.clear_graph_branch_focus();
        }
        self.apply_graph_branch_focus(repo, names)
    }

    fn clear_graph_branch_focus(&mut self) -> Effect {
        self.graph_focus_picker = None;
        if self.graph_branch_focus.is_none() {
            if self.graph_pane_focused() {
                self.status = "full graph".into();
            }
            return Effect::None;
        }
        self.apply_graph_branch_focus(self.graph_focus_repo().unwrap_or_default(), Vec::new())
    }

    fn apply_graph_branch_focus(&mut self, repo: String, names: Vec<String>) -> Effect {
        self.graph_identity = None;
        self.graph_cursor = 0;
        self.graph_scroll = 0;
        if names.is_empty() {
            self.graph_branch_focus = None;
            self.status = "full graph".into();
        } else {
            let label = names.join(", ");
            self.graph_branch_focus = Some((repo, names));
            self.status = format!("graph focus: {label}").into();
        }
        Effect::LoadRightPane
    }

    fn submit_branch_picker(&mut self) -> Effect {
        let Some(picker) = self.branch_picker.as_ref() else {
            return Effect::None;
        };
        let repo = picker.repo.clone();
        if let Some(selected) = picker.selected().cloned() {
            if selected.current {
                self.branch_picker = None;
                self.status = format!("Already on {}", selected.name).into();
                return Effect::None;
            }
            return self.checkout_or_confirm(repo, selected.name);
        }
        if picker.on_create_row() {
            let name = picker.create_name().unwrap_or_default().to_string();
            let commit_id = picker.commit_id.clone();
            self.branch_picker = None;
            // The graph picker creates at its commit and does not check out.
            return match commit_id {
                Some(commit_id) => {
                    let short = commit_id.get(..7).unwrap_or(&commit_id);
                    self.status = StatusMessage::progress(format!("creating {name} at {short}…"));
                    Effect::CreateBranchAt {
                        repo,
                        name,
                        commit_id,
                    }
                }
                None => {
                    self.status = StatusMessage::progress(format!("creating {name}…"));
                    Effect::CreateBranch { repo, name }
                }
            };
        }
        // No branch matches and no create row: say why (a bad or existing
        // name hides the create row).
        let query = picker.filter.trim();
        self.status = StatusMessage::warn(match picker.create_refusal() {
            _ if query.is_empty() => "no matching branches".to_string(),
            Some(reason) if picker.creates => format!("no branch matches {query} · {reason}"),
            _ => format!("no branch matches {query}"),
        });
        Effect::None
    }

    /// Emit a checkout effect. Origin out-of-sync confirm is decided later
    /// (`plan_graph_checkout`) from the selected name, not from local vs origin
    /// of every checkout.
    pub fn checkout_or_confirm(&mut self, repo: String, selected_name: String) -> Effect {
        self.status = StatusMessage::progress(format!("checking out {selected_name}…"));
        Effect::CheckoutBranch {
            repo,
            selected_name,
            fast_forward_ref: None,
        }
    }

    /// Confirm checkout when a local exists and is out of sync with the selected `origin/*`.
    ///
    /// `ahead_behind` is local-only / remote-only commit counts for the box copy.
    pub fn confirm_checkout_if_out_of_sync(
        &mut self,
        repo: String,
        branch: String,
        remote_ref: Option<String>,
        ahead_behind: Option<(usize, usize)>,
    ) -> Effect {
        if let Some(remote_ref) = remote_ref {
            self.confirm = Some(PendingConfirm::CheckoutOutOfSync {
                repo,
                branch: branch.clone(),
                remote_ref: remote_ref.clone(),
                ahead_behind,
            });
            Effect::None
        } else {
            self.checkout_or_confirm(repo, branch)
        }
    }

    fn submit_create_branch(&mut self) -> Effect {
        let Some(create) = self.create_branch.take() else {
            return Effect::None;
        };
        // Git ref rules are checked here, so a bad name never reaches git.
        if let Some(reason) = branch_name_error(&create.name) {
            self.status = StatusMessage::warn(reason);
            self.create_branch = Some(create);
            return Effect::None;
        }
        self.status = StatusMessage::progress(format!("creating {}…", create.name.trim()));
        Effect::CreateBranchAt {
            repo: create.repo,
            name: create.name.trim().to_string(),
            commit_id: create.commit_id,
        }
    }

    fn begin_graph_checkout(&mut self) -> Effect {
        if !self.graph_commit_focused() {
            return Effect::None;
        }
        if self.hidden_focus() {
            self.status = StatusMessage::warn("focus a visible repo to checkout");
            return Effect::None;
        }
        let Some(repo) = self.focused_graph_repo() else {
            return Effect::None;
        };
        let Some(GraphRow::Commit { commit, .. }) = self.focused_graph_row() else {
            return Effect::None;
        };
        let names = checkoutable_branch_names(&commit.refs);
        if names.is_empty() {
            return Effect::None;
        }
        if self.graph_repo_is_dirty(&repo) {
            self.status = StatusMessage::warn(DIRTY_WORKTREE_STATUS);
            return Effect::None;
        }
        self.help_open = false;
        if names.len() == 1 {
            return self.checkout_or_confirm(repo, names[0].clone());
        }
        // Every local branch, not only those on this commit, so the create
        // row never offers a name that exists elsewhere. Snapshot data: no
        // git on the loop thread.
        let taken = self
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .map(|row| row.local_branches.clone())
            .unwrap_or_default();
        self.branch_picker = Some(BranchPickerState::from_names(
            repo,
            names,
            Some(commit.id.clone()),
            taken,
        ));
        self.status.clear();
        Effect::None
    }

    fn begin_graph_create_branch(&mut self) -> Effect {
        if !self.graph_commit_focused() {
            return Effect::None;
        }
        if self.hidden_focus() {
            self.status = StatusMessage::warn("focus a visible repo to create a branch");
            return Effect::None;
        }
        let Some(repo) = self.focused_graph_repo() else {
            return Effect::None;
        };
        let Some(GraphRow::Commit { commit, .. }) = self.focused_graph_row() else {
            return Effect::None;
        };
        let commit_id = commit.id.clone();
        self.help_open = false;
        self.create_branch = Some(CreateBranchState {
            repo,
            name: String::new(),
            commit_id,
        });
        self.status.clear();
        Effect::None
    }

    fn begin_graph_merge(&mut self) -> Effect {
        if !self.graph_commit_focused() {
            return Effect::None;
        }
        if self.hidden_focus() {
            self.status = StatusMessage::warn("focus a visible repo to merge");
            return Effect::None;
        }
        let Some(repo) = self.focused_graph_repo() else {
            return Effect::None;
        };
        let Some(GraphRow::Commit { commit, .. }) = self.focused_graph_row() else {
            return Effect::None;
        };
        if self.graph_repo_is_dirty(&repo) {
            self.status = StatusMessage::warn(DIRTY_WORKTREE_STATUS);
            return Effect::None;
        }
        let (rev, label) = merge_rev_for_commit(&commit.id, &commit.refs);
        let into = self.graph_head_label(&repo);
        self.help_open = false;
        self.confirm = Some(PendingConfirm::MergeIntoHead {
            repo,
            rev,
            label,
            into,
        });
        Effect::None
    }

    fn graph_head_label(&self, repo: &str) -> String {
        self.snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .map(|row| row.branch.clone())
            .filter(|branch| !branch.is_empty())
            .unwrap_or_else(|| "HEAD".into())
    }

    fn graph_repo_is_dirty(&self, repo: &str) -> bool {
        self.snapshot
            .repos
            .iter()
            .find(|row| row.repo == repo)
            .is_some_and(|row| row.has_unstaged || row.has_staged)
    }

    /// True when the focused tree row belongs to a hidden repo that `.` hides.
    pub(crate) fn hidden_focus(&self) -> bool {
        self.focused_row()
            .is_some_and(|row| row_is_hidden(row, self.show_hidden))
    }

    pub fn focused_graph_row(&self) -> Option<GraphRow> {
        self.graph.as_ref()?.visible_row_at(self.graph_cursor)
    }

    pub fn focused_graph_stash_ref(&self) -> Option<String> {
        if !self.graph_pane_focused() {
            return None;
        }
        stash_ref_from_graph_row(&self.focused_graph_row()?)
    }

    fn move_graph_cursor(&mut self, delta: i32) {
        let n = self
            .graph
            .as_ref()
            .map(|g| g.visible_rows().len())
            .unwrap_or(0);
        if n == 0 {
            self.graph_cursor = 0;
            return;
        }
        let next = self.graph_cursor as i32 + delta;
        self.graph_cursor = next.clamp(0, n as i32 - 1) as usize;
        self.sync_graph_scroll();
    }

    /// Page in painted-list space, then
    /// snap onto a selectable row. Click stays on `visible_rows`.
    fn page_graph(&mut self, pages: i32) -> Effect {
        let Some(model) = self.graph.as_ref() else {
            return Effect::None;
        };
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        if painted.is_empty() {
            return Effect::None;
        }
        let list_h = self.graph_chrome().list_height.max(1) as usize;
        let page = list_h.saturating_sub(1).max(1) as i32;
        let current = painted
            .iter()
            .position(|line| line.row_index == Some(self.graph_cursor) && line.selectable)
            .unwrap_or(0);
        let last = painted.len() as i32 - 1;
        let target = (current as i32 + pages * page).clamp(0, last) as usize;
        let snapped = nearest_selectable_painted_index(&painted, target);
        if let Some(idx) = painted[snapped].row_index {
            self.graph_cursor = idx;
        }
        self.sync_graph_scroll();
        self.follow_graph_files()
    }

    /// Record the new terminal width. The stored `tree_fraction` stays as it
    /// is: [`pane_widths`] clamps it to whole columns at draw time. Rounding
    /// it here would shrink the split a little on each `Resize` event of a
    /// terminal drag, and a split the narrowest terminal clamped would not
    /// come back when the terminal grows.
    fn apply_terminal_size(&mut self, cols: u16) {
        self.layout.term_cols = cols.max(1);
    }

    /// Graph list top on screen: the last frame's painted top
    /// ([`LayoutHit::graph_painted_top`]), which trails
    /// [`Self::graph_scroll`] during a Ctrl-d / Ctrl-u glide. Falls back to
    /// `graph_scroll` before any graph list paints.
    pub(crate) fn graph_painted_top(&self) -> u16 {
        self.layout.graph_painted_top.unwrap_or(self.graph_scroll)
    }

    /// Whether the graph list paints its vertical scrollbar: the painted
    /// lines overflow the list, or it has left the top. Mirrors the widget
    /// from the line count the last frame recorded, so a pan key does not
    /// repaint the whole model to count it.
    pub(crate) fn graph_vscroll_shown(&self) -> bool {
        self.graph.is_some()
            && graph_vscroll_visible(
                self.layout.graph_content_len,
                self.graph_chrome().list_height,
                self.graph_scroll,
            )
    }

    pub(crate) fn sync_graph_scroll(&mut self) {
        let Some((idx, line_count)) = self.graph_cursor_painted() else {
            return;
        };
        let list_h = self.graph_chrome().list_height.max(1) as usize;
        let (start, _) = visible_window(line_count, idx, list_h);
        self.graph_scroll = start as u16;
    }

    /// Keep the graph cursor row in view when a paint gives the list
    /// `list_height` rows and the last paint gave it `painted_height`.
    ///
    /// The list height can change with no key press: the selected row's
    /// meta line wraps onto more rows in a narrower pane (a narrower
    /// terminal, Enter into the depth-1 left pane) or when its relative
    /// date grows. When the old height showed the cursor row and the new
    /// one hides it, the viewport moves as [`Self::sync_graph_scroll`]
    /// moves it. A cursor that the old height did not show (a scrollbar
    /// drag moves only the viewport) keeps its viewport. It finds the
    /// cursor line without a paint, so the frame still paints the model
    /// once.
    pub(crate) fn keep_graph_cursor_in_view(&mut self, painted_height: u16, list_height: u16) {
        if list_height >= painted_height || list_height == 0 {
            return;
        }
        let Some(model) = self.graph.as_ref() else {
            return;
        };
        let Some(idx) = painted_line_of_row(model, self.graph_cursor) else {
            return;
        };
        let line_count = painted_line_count(model);
        let top = usize::from(self.graph_scroll);
        let shows = |height: u16| idx >= top && idx < top + usize::from(height);
        if shows(painted_height) && !shows(list_height) {
            let (start, _) = visible_window(line_count, idx, usize::from(list_height));
            self.graph_scroll = start as u16;
        }
    }

    /// Painted graph line of the cursor row, the line [`Self::graph_scroll`]
    /// counts in. `None` with no graph or no selectable cursor line.
    pub(crate) fn graph_cursor_line(&self) -> Option<usize> {
        self.graph_cursor_painted().map(|(line, _)| line)
    }

    /// The cursor row's painted line and the painted line count.
    fn graph_cursor_painted(&self) -> Option<(usize, usize)> {
        let model = self.graph.as_ref()?;
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        let idx = painted
            .iter()
            .position(|line| line.row_index == Some(self.graph_cursor) && line.selectable)?;
        Some((idx, painted.len()))
    }

    fn sync_diff_scroll(&mut self) {
        let rows = self.current_diff_rows();
        self.sync_diff_scroll_for(&rows);
    }

    /// [`Self::sync_diff_scroll`] over rows the caller already built, so a
    /// cursor move on a large diff builds them once.
    fn sync_diff_scroll_for(&mut self, rows: &[DiffRow]) {
        let n = rows.len();
        if n == 0 {
            self.diff_cursor = 0;
            self.diff_scroll = 0;
            self.clear_diff_visual();
            return;
        }
        self.diff_cursor = self.diff_cursor.min(n - 1);
        if self.diff_wrap {
            let heights = self.diff_wrap_heights_for(rows);
            self.diff_scroll =
                wrap_viewport_start(&heights, self.diff_cursor, self.diff_body_height()) as u16;
            return;
        }
        let (start, _) = visible_window(n, self.diff_cursor, self.diff_body_height());
        self.diff_scroll = start as u16;
    }

    fn move_diff_cursor(&mut self, delta: i32) {
        let rows = self.current_diff_rows();
        let n = rows.len() as i32;
        if n > 0 {
            self.diff_cursor = (self.diff_cursor as i32 + delta).clamp(0, n - 1) as usize;
        }
        self.sync_diff_scroll_for(&rows);
    }

    fn page_step(&self) -> i32 {
        self.layout.tree_height.max(1).saturating_sub(1).max(1) as i32
    }

    fn move_file_cursor(&mut self, delta: i32) -> Effect {
        let n = self.commit_file_rows().len();
        if n == 0 {
            self.set_commit_file_cursor(0);
            return Effect::None;
        }
        let next = (self.commit_files_cursor() as i32 + delta).clamp(0, n as i32 - 1) as usize;
        self.set_commit_file_cursor(next);
        self.maybe_load_focused_commit_diff()
    }

    fn move_focused(&mut self, delta: i32) -> Effect {
        match self.list_focus_target() {
            ListFocusTarget::CommitFiles => self.move_file_cursor(delta),
            ListFocusTarget::Graph => {
                self.move_graph_cursor(delta);
                self.follow_graph_files()
            }
            // A folder summary has no cursor: the keys scroll it.
            ListFocusTarget::None if self.summary_shown() => {
                self.scroll_summary_by(delta);
                Effect::None
            }
            ListFocusTarget::None => {
                self.move_diff_cursor(delta);
                Effect::None
            }
            ListFocusTarget::Tree => {
                self.move_cursor(delta);
                self.drill = DrillView::Graph;
                Effect::LoadRightPane
            }
        }
    }

    fn move_focused_edge(&mut self, end: bool) -> Effect {
        match self.list_focus_target() {
            ListFocusTarget::CommitFiles => {
                let rows = self.commit_file_rows();
                let idx = if end {
                    rows.len().saturating_sub(1)
                } else {
                    commit_file_cursor_index(&rows, None)
                };
                self.set_commit_file_cursor(idx);
                self.maybe_load_focused_commit_diff()
            }
            ListFocusTarget::Graph => {
                let n = self
                    .graph
                    .as_ref()
                    .map(|g| g.visible_rows().len())
                    .unwrap_or(0);
                self.graph_cursor = if end { n.saturating_sub(1) } else { 0 };
                self.sync_graph_scroll();
                self.follow_graph_files()
            }
            ListFocusTarget::None if self.summary_shown() => {
                self.scroll_summary_to(if end { usize::MAX } else { 0 });
                Effect::None
            }
            ListFocusTarget::None => {
                let n = self.current_diff_rows().len();
                self.diff_cursor = if end { n.saturating_sub(1) } else { 0 };
                self.sync_diff_scroll();
                Effect::None
            }
            ListFocusTarget::Tree => {
                if end {
                    if !self.rows.is_empty() {
                        self.cursor = self.rows.len() - 1;
                    }
                } else {
                    self.cursor = 0;
                }
                self.drill = DrillView::Graph;
                Effect::LoadRightPane
            }
        }
    }

    /// True when Enter on the right pane opens the next depth: a graph
    /// commit or stash opens its files, a commit-file row opens its diff.
    ///
    /// Same checks as [`Self::nav_enter`] without the status copy. The
    /// `Enter drill` hint shows only when this holds.
    pub(crate) fn nav_enter_drills(&self) -> bool {
        if self.is_compare_tab() || self.focus != FocusPane::Right || self.hidden_focus() {
            return false;
        }
        match &self.drill {
            DrillView::Graph => {
                self.focused_graph_repo().is_some()
                    && self
                        .focused_graph_row()
                        .is_some_and(|row| source_from_graph_row(&row).is_some())
            }
            DrillView::Files { cursor, .. } => self
                .commit_file_rows()
                .get(*cursor)
                .is_some_and(|row| !row.is_dir()),
            DrillView::Diff { .. } => false,
        }
    }

    /// Title of the left pane: `tree`, `graph` (commit-files drill), or
    /// `files` (commit diff drill and compare tabs).
    pub fn left_pane_title(&self) -> &'static str {
        if self.drill.is_diff() || self.is_compare_tab() {
            "files"
        } else if self.drill.is_files() {
            "graph"
        } else {
            "tree"
        }
    }

    fn nav_enter(&mut self) -> Effect {
        if self.is_compare_tab() {
            if self.focus == FocusPane::Left {
                self.focus = FocusPane::Right;
            }
            return Effect::None;
        }
        if self.hidden_focus() {
            self.status = StatusMessage::warn("hidden stay out of drill");
            return Effect::None;
        }
        if self.focus == FocusPane::Left {
            self.focus = FocusPane::Right;
            return Effect::None;
        }
        match &self.drill {
            DrillView::Graph => {
                let Some(repo) = self.focused_graph_repo() else {
                    self.status = StatusMessage::warn("focus a repo commit to drill");
                    return Effect::None;
                };
                let Some(row) = self.focused_graph_row() else {
                    self.status = StatusMessage::warn("focus a graph commit to drill");
                    return Effect::None;
                };
                let Some(source) = source_from_graph_row(&row) else {
                    self.status = StatusMessage::warn("focus a graph commit to drill");
                    return Effect::None;
                };
                Effect::LoadCommitFiles { repo, source }
            }
            DrillView::Files {
                repo,
                source,
                cursor,
                ..
            } => {
                let rows = self.commit_file_rows();
                let Some(row) = rows.get(*cursor) else {
                    self.status = StatusMessage::warn("no files in this commit");
                    return Effect::None;
                };
                if row.is_dir() {
                    self.fold_commit_file(FoldOp::Toggle);
                    return Effect::None;
                }
                Effect::LoadCommitDiff {
                    repo: repo.clone(),
                    source: source.clone(),
                    path: row.path.clone(),
                }
            }
            DrillView::Diff { .. } => Effect::None,
        }
    }

    fn nav_esc(&mut self) -> Effect {
        if self.is_compare_tab() {
            self.focus = FocusPane::Left;
            return Effect::None;
        }
        if self.compare_picker_pending.take().is_some() {
            return Effect::None;
        }
        if self.focus == FocusPane::Right {
            self.focus = FocusPane::Left;
            return Effect::None;
        }
        match &self.drill {
            DrillView::Diff {
                repo,
                source,
                files,
                file_cursor,
                ..
            } => {
                let repo = repo.clone();
                let source = source.clone();
                let files = files.clone();
                let cursor = *file_cursor;
                self.drill = DrillView::Files {
                    repo,
                    source,
                    files,
                    cursor,
                };
                Effect::DropCommitDiff
            }
            DrillView::Files { .. } => {
                self.drill = DrillView::Graph;
                Effect::LoadRightPane
            }
            DrillView::Graph => Effect::None,
        }
    }

    fn graph_stash_op(&mut self, id: StashOpId) -> Effect {
        if self.hidden_focus() {
            return Effect::None;
        }
        let Some(stash_ref) = self.focused_graph_stash_ref() else {
            self.status = StatusMessage::warn("focus a graph stash row");
            return Effect::None;
        };
        let Some(repo) = self.focused_graph_repo() else {
            self.status = StatusMessage::warn("focus a visible repo to stash");
            return Effect::None;
        };
        match id {
            StashOpId::Apply => {
                self.status = StatusMessage::progress(format!("applying {stash_ref}…"));
                Effect::StashApply { repo, stash_ref }
            }
            StashOpId::Pop => {
                self.status = StatusMessage::progress(format!("popping {stash_ref}…"));
                Effect::StashPop { repo, stash_ref }
            }
            StashOpId::Drop => {
                self.confirm = Some(PendingConfirm::StashDrop {
                    repo,
                    stash_ref: stash_ref.clone(),
                });
                Effect::None
            }
            StashOpId::Create => Effect::None,
        }
    }

    fn hit_tab(&self, col: u16, row: u16) -> Option<usize> {
        hit_row_box(&self.layout.tab_hits, self.layout.tab_y, col, row)
    }

    fn hit_tab_close(&self, col: u16, row: u16) -> Option<usize> {
        hit_row_box(&self.layout.tab_close_hits, self.layout.tab_y, col, row)
    }

    /// Tab index whose `[✗]` sits under [`Self::pane_pointer`] in the last
    /// paint (none while the search-in-files dialog is open).
    pub fn hovered_tab_close(&self) -> Option<usize> {
        let (col, row) = self.pane_pointer()?;
        self.hit_tab_close(col, row)
    }

    /// [`Self::pointer`] for hover on the tab strip and the panes: `None`
    /// while the search-in-files dialog is open.
    ///
    /// The dialog keeps the pointer for its option chips. It covers the
    /// panes, so a tab `[✗]` or an icon under the pointer must not hover
    /// or arm the icon peek then.
    pub(crate) fn pane_pointer(&self) -> Option<(u16, u16)> {
        self.pointer.filter(|_| self.search_files.is_none())
    }

    /// Resize edge to highlight: the edge an active drag moves, else the
    /// edge under [`Self::pane_pointer`] in the last paint.
    ///
    /// `None` while mouse capture is off, below the minimum terminal
    /// size, while an overlay takes the mouse (dialog, prompt, help,
    /// palette, pinned popover), with the pointer inside a hover popover,
    /// and on a painted tab label (a press there activates or closes the
    /// tab). A press there never reaches the edges, so they do not hover.
    /// Scrollbar hits are not edges.
    pub fn hovered_resize_edge(&self) -> Option<ResizeEdge> {
        if !self.resize_edges_take_mouse() {
            return None;
        }
        if let Some(edge) = self.drag.resize_edge() {
            return Some(edge);
        }
        let (col, row) = self.pane_pointer()?;
        // The same order as `click_at`: a tab takes the press first.
        if self.pointer_in_popover()
            || self.hit_tab_close(col, row).is_some()
            || self.hit_tab(col, row).is_some()
        {
            return None;
        }
        self.split_hit_at(col, row).resize_edge()
    }

    /// True when a left press can land on a resize edge: mouse on, both
    /// panes painted (any tab kind), and an input mode whose clicks reach
    /// [`Self::press_split_hit`] (through [`Self::click_at`], or
    /// `explorer_click` on an Explorer tab).
    fn resize_edges_take_mouse(&self) -> bool {
        self.mouse_enabled
            && !self.too_small
            && !self.is_prs_tab()
            && matches!(
                self.input_mode(),
                InputMode::Normal { .. }
                    | InputMode::ZPending { .. }
                    | InputMode::GPending { .. }
                    | InputMode::DiffVisual
            )
    }

    /// Search-in-files option chip painted at (`col`, `row`) in the last
    /// paint, while the dialog is open.
    pub fn search_option_at(&self, col: u16, row: u16) -> Option<SearchFilesOption> {
        self.search_files.as_ref()?;
        hit_row_box(
            &self.layout.search_option_hits,
            self.layout.search_option_y,
            col,
            row,
        )
    }

    /// Search-in-files option chip under [`Self::pointer`] in the last
    /// paint, while the dialog is open.
    pub fn hovered_search_option(&self) -> Option<SearchFilesOption> {
        let (col, row) = self.pointer?;
        self.search_option_at(col, row)
    }

    /// Store the pointer from any-event motion.
    ///
    /// Returns `true` when the hovered tab `[✗]`, icon, search option chip,
    /// or resize edge ([`Self::hovered_resize_edge`]) changed, so the live
    /// loop redraws only then. Arms or drops the icon peek dwell and leave
    /// grace ([`Self::peek_remaining_ms`]).
    /// Ignored while mouse capture is off. Chords, status, and effects are
    /// untouched.
    pub fn set_pointer(&mut self, pointer: Option<(u16, u16)>) -> bool {
        self.set_pointer_at(pointer, Instant::now())
    }

    /// [`Self::set_pointer`] at time `now`.
    pub(crate) fn set_pointer_at(&mut self, pointer: Option<(u16, u16)>, now: Instant) -> bool {
        let pointer = pointer.filter(|_| self.mouse_enabled);
        let before = self.hovered_tab_close();
        let icon_before = self.hovered_icon().cloned();
        let chip_before = self.hovered_search_option();
        let edge_before = self.hovered_resize_edge();
        self.pointer = pointer;
        let icon = self.hovered_icon().cloned();
        self.track_peek(icon.as_ref(), now);
        self.hovered_tab_close() != before
            || icon != icon_before
            || self.hovered_search_option() != chip_before
            || self.hovered_resize_edge() != edge_before
    }

    fn activate_tab(&mut self, index: usize) -> Effect {
        if index >= self.tabs.len() || index == self.tabs.active {
            return Effect::None;
        }
        self.abandon_compare_picker();
        self.clear_tab_transients();
        self.park_active_session();
        self.tabs.active = index;
        self.apply_active_session();
        // A working-tree tab or an Explorer inner file tab may have missed
        // watch reloads while inactive. Only one tab is active, so at most
        // one check reloads. The PRs tab asks for the details it lists.
        match self.active_worktree_compare_reload() {
            Effect::None if self.is_prs_tab() => self.request_pr_details(),
            Effect::None => self.active_explorer_live_reload(),
            reload => reload,
        }
    }

    /// Drop the visual-line highlight and any pending confirm.
    ///
    /// Both belong to the tab that opened them. A tab switch or close must
    /// not carry a range or a `y` confirm over to another tab's diff.
    fn clear_tab_transients(&mut self) {
        self.clear_diff_visual();
        self.confirm = None;
    }

    fn activate_relative_tab(&mut self, delta: i32) -> Effect {
        let len = self.tabs.len() as i32;
        if len <= 1 {
            self.status = StatusMessage::warn(super::tabs::ONLY_WORKSPACE_TAB_OPEN);
            return Effect::None;
        }
        let next = (self.tabs.active as i32 + delta).rem_euclid(len) as usize;
        self.activate_tab(next)
    }

    fn jump_to_tab(&mut self, n: u8) -> Effect {
        if n == 0 {
            return Effect::None;
        }
        self.activate_tab(n as usize - 1)
    }

    /// Concrete checkout for Diff vs * commands.
    pub fn compare_target_checkout(&self) -> Option<String> {
        if let Some(tab) = self.tabs.active_compare() {
            return Some(tab.checkout_path.clone());
        }
        match &self.drill {
            DrillView::Files { repo, .. } | DrillView::Diff { repo, .. } => {
                return Some(repo.clone());
            }
            DrillView::Graph => {}
        }
        if self.graph_pane_focused() {
            if let Some(repo) = self.focused_graph_repo() {
                return Some(repo);
            }
        }
        let row = self.focused_row()?;
        match row.kind {
            NodeKind::Workspace | NodeKind::Group => None,
            NodeKind::Repo => {
                let repo = row.repo.as_deref()?;
                if super::branches::is_family_container(&self.snapshot, repo) {
                    None
                } else {
                    Some(repo.to_string())
                }
            }
            NodeKind::Checkout | NodeKind::Section | NodeKind::Dir | NodeKind::File => {
                row.repo.clone()
            }
        }
    }

    /// Snapshot HEAD sha (empty when unborn) and default tip ref of `checkout`.
    fn checkout_head_and_default(&self, checkout: &str) -> Option<(&str, Option<&str>)> {
        self.snapshot
            .repos
            .iter()
            .find(|row| row.repo == checkout)
            .map(|row| (row.head.as_str(), row.default_tip_ref.as_deref()))
    }

    fn open_compare_tab(&mut self, checkout: String, base_ref: String, head_ref: String) -> Effect {
        self.open_compare_tab_for(checkout, base_ref, head_ref, None)
    }

    /// Open or focus the commit-vs-working-tree tab of commit `base` (a
    /// full id) and `file` in `checkout`. A new tab records the file's
    /// [`Self::worktree_compare_stamp`] and loads.
    pub(super) fn open_worktree_compare_tab(
        &mut self,
        checkout: String,
        base: String,
        file: WorktreeFile,
    ) -> Effect {
        self.open_compare_tab_for(checkout, base, COMPARE_HEAD_REF.into(), Some(file))
    }

    fn open_compare_tab_for(
        &mut self,
        checkout: String,
        base_ref: String,
        head_ref: String,
        worktree_file: Option<WorktreeFile>,
    ) -> Effect {
        let before = self.tabs.active;
        self.park_active_session();
        let stamp = worktree_file
            .as_ref()
            .and_then(|file| self.worktree_compare_stamp(&checkout, file));
        let opened = match worktree_file {
            Some(file) => {
                self.tabs
                    .open_or_focus_worktree(checkout.clone(), base_ref.clone(), file)
            }
            None => self
                .tabs
                .open_or_focus(checkout.clone(), base_ref.clone(), head_ref.clone()),
        };
        if let (OpenCompare::Created(_), Some(tab)) = (opened, self.tabs.active_compare_mut()) {
            tab.worktree_stamp = stamp;
        }
        if self.tabs.active != before {
            self.clear_tab_transients();
        }
        match opened {
            OpenCompare::Focused => {
                self.apply_active_session();
                self.active_worktree_compare_reload()
            }
            OpenCompare::Created(tab_id) => {
                self.apply_active_session();
                self.focus = FocusPane::Left;
                Effect::LoadCompareRange {
                    tab_id,
                    repo: checkout,
                    base_ref,
                    head_ref,
                    force: true,
                }
            }
        }
    }

    /// Diff vs default. [`Self::compare_refusal`] has already refused a
    /// missing checkout, an unborn HEAD, and a missing default tip.
    pub(crate) fn compare_vs_default(&mut self) -> Effect {
        let Some(checkout) = self.compare_target_checkout() else {
            return Effect::None;
        };
        let Some(base_ref) = self
            .checkout_head_and_default(&checkout)
            .and_then(|(_, default_tip)| default_tip)
            .map(str::to_string)
        else {
            return Effect::None;
        };
        self.open_compare_tab(checkout, base_ref, COMPARE_HEAD_REF.into())
    }

    /// Diff vs branch / Diff vs commit: prepare the compare picker that
    /// lists `kind`. [`Self::compare_refusal`] has already refused a missing
    /// checkout and an unborn HEAD.
    pub(crate) fn prepare_compare_picker(&mut self, kind: ComparePickerKind) -> Effect {
        let Some(checkout) = self.compare_target_checkout() else {
            return Effect::None;
        };
        self.compare_picker_pending = Some(checkout.clone());
        Effect::PrepareComparePicker {
            repo: checkout,
            kind,
        }
    }

    /// Checkout and full commit id for Diff commit vs parent, or why it
    /// refuses.
    ///
    /// The graph pane must focus a commit row (the same predicate as
    /// [`Self::graph_commit_focused`]) that has a parent. The checkout is the
    /// repo that owns the loaded graph, the one graph `b` / `m` act on.
    fn commit_vs_parent_target(&self) -> Result<(String, String), &'static str> {
        if !self.graph_commit_focused() {
            return Err(FOCUS_A_COMMIT_TO_DIFF);
        }
        let Some(GraphRow::Commit { commit, .. }) = self.focused_graph_row() else {
            return Err(FOCUS_A_COMMIT_TO_DIFF);
        };
        if commit.parents.is_empty() {
            return Err(ROOT_COMMIT_HAS_NO_PARENT);
        }
        let repo = self.focused_graph_repo().ok_or(FOCUS_A_COMMIT_TO_DIFF)?;
        Ok((repo, commit.id))
    }

    /// Diff commit vs parent: open or focus `<sha>^...<sha>` with the head
    /// pinned to `<sha>`. `^` is the first parent, so a merge shows its
    /// changes against the first parent. Never checks anything out.
    /// [`Self::compare_refusal`] has already refused a non-commit focus and
    /// a root commit.
    pub(crate) fn compare_commit_vs_parent(&mut self) -> Effect {
        let Ok((repo, commit_id)) = self.commit_vs_parent_target() else {
            return Effect::None;
        };
        self.open_compare_tab(repo, format!("{commit_id}^"), commit_id)
    }

    /// Close the active compare or Explorer tab (Workspace refuses).
    pub(crate) fn close_active_tab(&mut self) -> Effect {
        self.close_tab_at(self.tabs.active)
    }

    fn close_tab_at(&mut self, index: usize) -> Effect {
        if index == 0 {
            self.status = StatusMessage::warn(WORKSPACE_TAB_CANNOT_CLOSE);
            return Effect::None;
        }
        let closing_active = index == self.tabs.active;
        if closing_active {
            self.park_active_session();
            self.abandon_compare_picker();
        }
        if !self.tabs.close_at(index) {
            return Effect::None;
        }
        self.clear_tab_transients();
        if closing_active {
            self.apply_active_session();
            // An Explorer inner file tab that becomes active may have
            // missed watch reloads.
            return self.active_explorer_live_reload();
        }
        Effect::None
    }

    pub(crate) fn open_compare_picker(
        &mut self,
        repo: String,
        branches: Vec<crate::git::LocalBranch>,
    ) {
        self.compare_picker_pending = None;
        self.compare_picker = Some(ComparePickerState::Branch(BranchPickerState::new(
            repo, branches,
        )));
        self.status.clear();
    }

    pub(crate) fn open_compare_commit_picker(
        &mut self,
        repo: String,
        commits: Vec<crate::git::AncestorCommit>,
    ) {
        self.compare_picker_pending = None;
        self.compare_picker = Some(ComparePickerState::Commit(CommitPickerState::new(
            repo, commits,
        )));
        self.status.clear();
    }

    pub(crate) fn abandon_compare_picker(&mut self) {
        self.compare_picker_pending = None;
        self.compare_picker = None;
    }

    pub(crate) fn submit_compare_picker(&mut self) -> Effect {
        let Some(picker) = self.compare_picker.as_ref() else {
            return Effect::None;
        };
        let Some(base_ref) = picker.selected_base() else {
            self.status = StatusMessage::warn(super::tabs::compare_picker_empty(picker));
            return Effect::None;
        };
        let repo = picker.repo().to_string();
        self.compare_picker = None;
        self.open_compare_tab(repo, base_ref, COMPARE_HEAD_REF.into())
    }

    /// Path of the folder row under compare tab `tab_id`'s file cursor, if
    /// the cursor is on a folder. An inactive tab reads its parked folds and
    /// tree mode.
    fn compare_tab_focused_dir(&self, tab_id: u64) -> Option<String> {
        let tab = self.tabs.get_id(tab_id)?;
        let active = self
            .tabs
            .active_compare()
            .is_some_and(|active| active.id == tab_id);
        let (tree_mode, folds) = if active {
            (self.commit_tree_mode, &self.commit_file_folds)
        } else {
            (tab.tree_mode, &tab.folds)
        };
        let rows = flatten_commit_files(&tab.files, tree_mode, folds, self.ascii);
        rows.into_iter()
            .nth(tab.file_cursor)
            .filter(|row| row.is_dir())
            .map(|row| row.path)
    }

    /// Store a commit card lookup for commit `sha` of checkout `repo`.
    ///
    /// Returns true when the open popover (peek or pinned) shows that
    /// commit, so the loop paints the card.
    pub(crate) fn apply_commit_card(
        &mut self,
        repo: String,
        sha: String,
        result: Result<workspace_status_graph::Commit, String>,
    ) -> bool {
        let shown = self.popover.as_ref().is_some_and(|popover| {
            popover.targets.iter().any(|(_, target)| {
                matches!(target, IconTarget::Commit { repo: r, sha: s } if *r == repo && *s == sha)
            })
        });
        self.commit_cards.insert(repo, sha, result);
        shown
    }

    /// Apply a compare range load. Cursor is a flattened row, not a `files` index.
    ///
    /// A focused folder row stays focused, so its summary shows for the new
    /// range. Otherwise a still-present path keeps that file. A gone folder
    /// or path clamps to `min(old_index, n-1)`. A first load (no path yet)
    /// selects the first file.
    pub(crate) fn apply_compare_range(
        &mut self,
        tab_id: u64,
        gen: u64,
        result: Result<super::app::CompareRangeLoad, String>,
    ) -> Option<Effect> {
        let focused_dir = self.compare_tab_focused_dir(tab_id);
        let (source, keep, checkout, previous_cursor, had_path) = {
            let tab = self.tabs.get_id_mut(tab_id)?;
            if tab.generation != gen {
                return None;
            }
            tab.loading = false;
            let load = match result {
                Err(err) => {
                    tab.error = Some(err);
                    tab.commit = None;
                    tab.files.clear();
                    tab.patch_keys.clear();
                    tab.source = None;
                    tab.path = None;
                    tab.content = DiffContent::default();
                    tab.content_for = None;
                    return None;
                }
                Ok(load) => load,
            };
            tab.error = None;
            tab.source = Some(load.source.clone());
            tab.commit = load.commit;
            tab.last_head = load.head;
            tab.last_base_tip = Some(load.base_tip);
            let previous_cursor = tab.file_cursor;
            let had_path = tab.path.is_some();
            let keep = tab
                .path
                .clone()
                .filter(|path| load.files.iter().any(|file| &file.path == path));
            tab.files = load.files;
            tab.patch_keys = load.patch_keys;
            (
                load.source,
                keep,
                tab.checkout_path.clone(),
                previous_cursor,
                had_path,
            )
        };
        let active = self
            .tabs
            .active_compare()
            .is_some_and(|tab| tab.id == tab_id);
        let mut folds = if active {
            self.commit_file_folds.clone()
        } else {
            self.tabs.get_id(tab_id)?.folds.clone()
        };
        // A focused folder wins over the open file's path.
        let keep_path = focused_dir.as_deref().or(keep.as_deref());
        if let Some(path) = keep_path {
            for id in ancestor_dir_ids(path) {
                folds.remove(&id);
            }
        }
        let files = self.tabs.get_id(tab_id)?.files.clone();
        let tree_mode = if active {
            self.commit_tree_mode
        } else {
            self.tabs.get_id(tab_id)?.tree_mode
        };
        let rows = flatten_commit_files(&files, tree_mode, &folds, self.ascii);
        let cursor = if let Some(dir) = focused_dir.as_deref() {
            // A gone folder clamps like a gone file; a clamp onto another
            // folder shows that folder's summary.
            rows.iter()
                .position(|row| row.is_dir() && row.path == dir)
                .unwrap_or_else(|| previous_cursor.min(rows.len().saturating_sub(1)))
        } else if keep.is_some() {
            commit_file_cursor_index(&rows, keep.as_deref())
        } else if had_path && !rows.is_empty() {
            previous_cursor.min(rows.len() - 1)
        } else {
            commit_file_cursor_index(&rows, None)
        };
        let selected = rows.get(cursor).and_then(|row| {
            row.is_file().then(|| {
                (
                    row.path.clone(),
                    row.file.as_ref().and_then(|file| file.old_path.clone()),
                )
            })
        });
        if self
            .tabs
            .active_compare()
            .is_some_and(|tab| tab.id == tab_id)
        {
            self.commit_file_folds = folds.clone();
        }
        let tab = self.tabs.get_id_mut(tab_id)?;
        tab.folds = folds;
        tab.file_cursor = cursor;
        match selected {
            Some((path, old_path)) => {
                tab.path = Some(path.clone());
                Some(Effect::LoadCompareDiff {
                    tab_id,
                    repo: checkout,
                    source,
                    path,
                    old_path,
                })
            }
            None => {
                tab.path = None;
                tab.content = DiffContent::default();
                tab.content_for = None;
                None
            }
        }
    }

    pub(crate) fn apply_compare_diff(
        &mut self,
        tab_id: u64,
        gen: u64,
        source: &CommitFileSource,
        path: &str,
        content: Result<DiffContent, String>,
    ) {
        let checkout = {
            let Some(tab) = self.tabs.get_id_mut(tab_id) else {
                return;
            };
            if tab.generation != gen {
                return;
            }
            if tab.source.as_ref() != Some(source) {
                return;
            }
            match content {
                Ok(content) => {
                    tab.path = Some(path.to_string());
                    tab.content = content;
                    tab.content_for = Some((source.clone(), path.to_string()));
                    Some(tab.checkout_path.clone())
                }
                Err(err) => {
                    tab.error = Some(err);
                    tab.content = DiffContent::default();
                    tab.content_for = None;
                    None
                }
            }
        };
        let is_active = self
            .tabs
            .active_compare()
            .is_some_and(|live| live.id == tab_id);
        if !is_active {
            return;
        }
        let Some(checkout) = checkout else {
            // A failed load must not carry a Ctrl-O anchor into the next diff.
            self.pending_hunk_anchor = None;
            return;
        };
        self.adopt_diff_view(DiffViewId::Commit {
            repo: checkout,
            source: source.clone(),
            path: path.to_string(),
        });
        // Same viewport sync as `set_diff` / `open_commit_diff`: a reload
        // (Ctrl-O, watch refresh) keeps the cursor in range and on its hunk.
        let n = self.current_diff_rows().len();
        self.diff_cursor = self.diff_cursor.min(n.saturating_sub(1));
        self.apply_pending_hunk_anchor();
        self.sync_diff_scroll();
        self.drop_stale_diff_visual();
    }

    pub(crate) fn apply_compare_probe(
        &mut self,
        tab_id: u64,
        changed: bool,
        error: Option<String>,
    ) -> Option<Effect> {
        let tab = self.tabs.get_id_mut(tab_id)?;
        if let Some(err) = error {
            tab.error = Some(err);
            return None;
        }
        if !changed {
            return None;
        }
        let repo = tab.checkout_path.clone();
        let base_ref = tab.base_ref.clone();
        let head_ref = tab.head_ref.clone();
        Some(Effect::LoadCompareRange {
            tab_id,
            repo,
            base_ref,
            head_ref,
            force: true,
        })
    }

    pub(crate) fn begin_compare_range(&mut self, tab_id: u64, force: bool) -> Option<u64> {
        let tab = self.tabs.get_id_mut(tab_id)?;
        if force {
            Some(tab.bump_generation())
        } else {
            Some(tab.generation)
        }
    }

    /// HEAD probes for loaded compare tabs. A commit-vs-working-tree tab
    /// is never probed: [`Self::worktree_compare_reload`] watches it.
    pub(crate) fn compare_probe_effects(&self) -> Vec<Effect> {
        self.tabs
            .compare_tabs()
            .filter(|tab| !tab.loading && tab.worktree_file.is_none())
            .map(|tab| Effect::ProbeCompareTab {
                tab_id: tab.id,
                repo: tab.checkout_path.clone(),
                base_ref: tab.base_ref.clone(),
                head_ref: tab.head_ref.clone(),
                last_head: tab.last_head.clone(),
                last_base_tip: tab.last_base_tip.clone(),
            })
            .collect()
    }

    /// What a commit-vs-working-tree tab of `file` in `checkout` watches:
    /// the checkout's HEAD, plus the workspace-snapshot change row and the
    /// tree row signature (status, size, mtime) of the file's path and old
    /// path. Reads loaded state only. `None` when the checkout is not in
    /// the snapshot.
    fn worktree_compare_stamp(&self, checkout: &str, file: &WorktreeFile) -> Option<String> {
        let snap = self
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == checkout)?;
        let mut stamp = snap.head.clone();
        for path in [Some(file.path.as_str()), file.old_path.as_deref()]
            .into_iter()
            .flatten()
        {
            let change = snap
                .changes
                .iter()
                .find(|change| change.path == path || change.old_path.as_deref() == Some(path));
            stamp.push_str(&format!("|{change:?}"));
            for id in [
                format!("file:{checkout}:{path}"),
                format!("file:{checkout}:{path}#unstaged"),
            ] {
                stamp.push('|');
                stamp.push_str(self.signatures.get(&id).map_or("", String::as_str));
            }
        }
        Some(stamp)
    }

    /// Reload the active commit-vs-working-tree tab of `checkout` when its
    /// [`Self::worktree_compare_stamp`] moved since the last load. Runs
    /// after each checkout status load, and when such a tab becomes active
    /// again ([`Self::active_worktree_compare_reload`]).
    ///
    /// Reloads the file list, so the row's status follows the disk; the
    /// range apply keeps the open path and reloads its diff in place (the
    /// viewport stays). No git runs here.
    pub(crate) fn worktree_compare_reload(&mut self, checkout: &str) -> Option<Effect> {
        let tab = self.tabs.active_compare()?;
        let file = tab.worktree_file.as_ref()?;
        if tab.loading || tab.checkout_path != checkout {
            return None;
        }
        let stamp = self.worktree_compare_stamp(checkout, file);
        let tab = self.tabs.active_compare_mut()?;
        if tab.worktree_stamp == stamp {
            return None;
        }
        tab.worktree_stamp = stamp;
        Some(Effect::LoadCompareRange {
            tab_id: tab.id,
            repo: tab.checkout_path.clone(),
            base_ref: tab.base_ref.clone(),
            head_ref: tab.head_ref.clone(),
            force: true,
        })
    }

    /// [`Self::worktree_compare_reload`] for the active tab's checkout, or
    /// [`Effect::None`].
    fn active_worktree_compare_reload(&mut self) -> Effect {
        let Some(checkout) = self
            .tabs
            .active_compare()
            .map(|tab| tab.checkout_path.clone())
        else {
            return Effect::None;
        };
        self.worktree_compare_reload(&checkout)
            .unwrap_or(Effect::None)
    }
}

enum FileWrite {
    Stage,
    Unstage,
}

fn empty_write_status(kind: Option<NodeKind>, write: FileWrite) -> String {
    let verb = match write {
        FileWrite::Stage => "stage",
        FileWrite::Unstage => "unstage",
    };
    match kind {
        Some(
            NodeKind::File
            | NodeKind::Dir
            | NodeKind::Repo
            | NodeKind::Checkout
            | NodeKind::Section,
        ) => {
            format!("nothing to {verb}")
        }
        _ => format!("focus a file, dir, checkout, or repo to {verb}"),
    }
}

fn is_stageable(change: &FileChange) -> bool {
    change.unstaged_status.is_some() || change.untracked
}

fn is_unstageable(change: &FileChange) -> bool {
    change.staged_status.is_some()
}

/// Why a compare whole-file `x` refuses a `name-status` kind, or `None` for
/// the kinds it restores (modified, added, deleted, renamed).
fn compare_file_revert_kind_refusal(status: &str) -> Option<String> {
    (!matches!(status, "M" | "A" | "D" | "R")).then(|| format!("cannot revert a {status} change"))
}

fn is_revertible(change: &FileChange) -> bool {
    change.unstaged_status.is_some() || change.untracked
}

fn op_paths(change: &FileChange) -> Vec<String> {
    match &change.old_path {
        Some(old) if old != &change.path => vec![old.clone(), change.path.clone()],
        _ => vec![change.path.clone()],
    }
}

fn group_write_paths(files: &[ScopedFile]) -> Vec<(String, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for file in files {
        let paths = match groups.iter().position(|(repo, _)| repo == &file.repo) {
            Some(idx) => &mut groups[idx].1,
            None => {
                groups.push((file.repo.clone(), Vec::new()));
                &mut groups.last_mut().unwrap().1
            }
        };
        for path in op_paths(&file.change) {
            if !paths.contains(&path) {
                paths.push(path);
            }
        }
    }
    groups
}

fn group_revert_targets(
    targets: &[RevertTarget],
    delete_untracked: bool,
) -> Vec<(String, Vec<String>, Vec<String>)> {
    let mut groups: Vec<(String, Vec<String>, Vec<String>)> = Vec::new();
    for target in targets {
        let (tracked, untracked) = match groups.iter().position(|(repo, _, _)| repo == &target.repo)
        {
            Some(idx) => {
                let group = &mut groups[idx];
                (&mut group.1, &mut group.2)
            }
            None => {
                groups.push((target.repo.clone(), Vec::new(), Vec::new()));
                let group = groups.last_mut().unwrap();
                (&mut group.1, &mut group.2)
            }
        };
        let paths = match &target.old_path {
            Some(old) if old != &target.path => vec![old.clone(), target.path.clone()],
            _ => vec![target.path.clone()],
        };
        if target.untracked {
            if delete_untracked {
                untracked.extend(paths);
            }
        } else {
            tracked.extend(paths);
        }
    }
    groups.retain(|(_, tracked, untracked)| !tracked.is_empty() || !untracked.is_empty());
    groups
}

/// Live-row index of the painted row nearest `target` that is a live row
/// (`live_index` gives `Some`), not a removal ghost the paint merged in.
/// `None` when no painted row is live.
fn nearest_live_row<T>(
    painted: &[T],
    target: usize,
    live_index: impl Fn(&T) -> Option<usize>,
) -> Option<usize> {
    (0..painted.len()).find_map(|step| {
        let below = painted
            .get(target.saturating_add(step))
            .and_then(&live_index);
        let above = target
            .checked_sub(step)
            .and_then(|index| painted.get(index))
            .and_then(&live_index);
        below.or(above)
    })
}

/// Snap a painted-list index onto a selectable line (prefer forward, then back).
/// Nearest selectable graph index at or after `start`.
fn nearest_selectable_painted_index(painted: &[PaintedLine], from: usize) -> usize {
    if painted.is_empty() {
        return 0;
    }
    let from = from.min(painted.len() - 1);
    if painted[from].selectable {
        return from;
    }
    for (i, row) in painted.iter().enumerate().skip(from + 1) {
        if row.selectable {
            return i;
        }
    }
    for i in (0..from).rev() {
        if painted[i].selectable {
            return i;
        }
    }
    0
}

fn single_or_batch(effects: Vec<Effect>) -> Effect {
    match effects.len() {
        0 => Effect::None,
        1 => effects.into_iter().next().expect("one effect"),
        _ => Effect::Batch(effects),
    }
}

fn initial_cursor(rows: &[VisibleRow]) -> usize {
    rows.iter()
        .position(|r| r.kind == NodeKind::File)
        .or_else(|| {
            rows.iter()
                .position(|r| r.kind == NodeKind::Repo || r.kind == NodeKind::Checkout)
        })
        .unwrap_or(0)
}

enum FoldOp {
    Toggle,
    Close,
    Open,
}

/// Viewed-store path in a unique temp dir per test state, so the sibling
/// comment and UI-state stores are unique too.
#[cfg(test)]
fn default_viewed_path() -> PathBuf {
    crate::testutil::unique_dir("ws-state-test").join("viewed.json")
}

/// Commit-message rows for a drag of the footer rule from `origin_row`
/// (pressed with `origin_lines` rows) to `row`, in a pane that shows at
/// most `fit` rows ([`workspace_status_graph::commit_msg_lines_fit`]).
///
/// Up adds one row per row moved and down removes one, from
/// `min(origin_lines, fit)` so the first row moved shows. The result is
/// clamped to [`COMMIT_MSG_LINES_MIN`]..=`min(fit, COMMIT_MSG_LINES_MAX)`,
/// so the list keeps the rows the footer budget gives it. Back on
/// `origin_row` it is `origin_lines`, so a drag that ends where it began
/// changes nothing (even rows a smaller pane cannot show).
fn commit_msg_lines_from_drag(origin_lines: usize, origin_row: u16, row: u16, fit: usize) -> usize {
    if row == origin_row {
        return origin_lines;
    }
    let most = fit.clamp(COMMIT_MSG_LINES_MIN, COMMIT_MSG_LINES_MAX);
    let base = origin_lines.min(most) as i64;
    let next = base + i64::from(origin_row) - i64::from(row);
    next.clamp(COMMIT_MSG_LINES_MIN as i64, most as i64) as usize
}

/// Idle / breadcrumb toast when a persist write returns [`std::io::Error`].
fn persist_failed_status(kind: &str, err: impl std::fmt::Display) -> String {
    format!("{kind} save failed: {err}")
}

/// Ancestor ids to try when a tree-mode row disappears in flat mode.
fn focus_ancestor_ids(id: &str) -> Vec<String> {
    let Some((kind, rest)) = id.split_once(':') else {
        return Vec::new();
    };
    if rest.is_empty() {
        return Vec::new();
    }
    if kind == "checkout" {
        return vec![format!("repo:{rest}")];
    }
    if kind != "file" && kind != "dir" {
        return Vec::new();
    }
    let Some((repo, path)) = rest.split_once(':') else {
        return Vec::new();
    };
    let segments: Vec<&str> = path.split('/').filter(|part| !part.is_empty()).collect();
    let parent_segs = if segments.is_empty() {
        &[][..]
    } else {
        &segments[..segments.len() - 1]
    };
    let mut ids = Vec::new();
    for len in (1..=parent_segs.len()).rev() {
        ids.push(format!("dir:{repo}:{}", parent_segs[..len].join("/")));
    }
    ids.push(format!("repo:{repo}"));
    ids.push(format!("checkout:{repo}"));
    ids
}

fn visible_snapshot(snapshot: &WorkspaceSnapshot, show_hidden: bool) -> WorkspaceSnapshot {
    let mut copy = snapshot.clone();
    copy.show_hidden = show_hidden;
    visible_for_tree(&copy)
}

#[cfg(test)]
mod tests;

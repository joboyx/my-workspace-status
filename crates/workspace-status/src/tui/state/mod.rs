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
mod pan;
mod popover;
mod pull_request;
mod remote_view;
mod scroll_glide;

pub(crate) use pull_request::PrDetailState;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use workspace_status_graph::{
    format_commit_message, format_relative_date, graph_chrome_budget_for, graph_footer_request,
    graph_vscroll_visible, paint_model, selection_footer_parts, wrap_commit_message,
    GraphChromeBudget, GraphFooterSelection, GraphModel, GraphRow, PaintedLine, ASCII,
    COMMIT_MSG_LINES_DEFAULT, COMMIT_MSG_LINES_MAX, COMMIT_MSG_LINES_MIN, FOOTER_RULE_ROWS,
    UNICODE,
};

use crate::config::ViewDefaults;
use crate::settings::Settings;
use crate::snapshot::{
    carry_status_failed_local_branches, CheckoutKind, FileChange, SyncStatus, WorkspaceSnapshot,
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
use super::chrome::{diverged_pull_status, status_uses_status_text, STATUS_NO_COMMENTS};
use super::comments::{
    collect_live_set, comment_key_label, comments_in_focus_scope, covering_line_comment,
    diff_focus_side, diff_line_comment_state, export_markdown, format_entity_reference,
    gc_comments, load_comment_store, put_comment_entry, resolve_comment_target,
    resolve_entity_reference, save_comment_store, viewport_line_number, viewport_line_range,
    CommentExport, CommentExportList, CommentKey, CommentPrompt, CommentStore, DiffSide, EntityRef,
};
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
use super::fetch::background_fetch_targets;
use super::fetch_claim::load_background_fetch_record;
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
    clamp_tree_fraction, diff_col_from_col, diff_col_from_delta, diff_paint_width, diff_pane_mode,
    diff_split_fraction_from_col, graph_col_from_col, graph_col_from_delta,
    graph_scroll_from_delta, graph_scroll_from_row, hit_split, pane_widths, step_tree_fraction,
    tree_fraction_from_col, DiffMode, ResizeEdge, SplitDrag, SplitHit, SplitLayout,
    DIFF_SPLIT_FRACTION, NARROW_SXS, TREE_WIDTH_FRACTION,
};
use super::stash::{
    checkout_path, resolve_stash_menu_key, row_is_hidden, stash_dirty_for_row,
    stash_ops_for_context, StashMenuKeyResult, StashOp, StashOpId, StashOpsContext,
};
use super::status::{StatusKind, StatusMessage};
use super::tabs::{
    compare_file_dirty, compare_range_label, CommitPickerState, ComparePickerKind,
    ComparePickerState, OpenCompare, TabStrip, WorktreeFile, CANNOT_REVERT_COMMITTED_DIFF,
    CANNOT_REVERT_WORKTREE_COMPARE, COMPARE_FOCUS_A_FILE, COMPARE_HEAD_MOVED, COMPARE_HEAD_REF,
    COMPARE_STILL_LOADING, FOCUS_A_COMMIT_TO_DIFF, ROOT_COMMIT_HAS_NO_PARENT,
    SWITCH_TO_WORKSPACE_TAB, WORKSPACE_TAB_CANNOT_CLOSE,
};
use super::theme::{BackgroundMode, ThemeId};
use super::toast::ToastStack;
use super::tree::{
    build_tree, collect_foldable_subtree_ids, default_folds, flatten_with, visible_for_tree,
    visible_window, workspace_label_from_cwd, NodeKind, TreeNode, VisibleRow,
};
use super::ui_state_store::{save_activity_sparkline, save_commit_msg_lines, save_theme};
use super::undercurl::UndercurlHit;
use super::viewed::{
    collect_current_fingerprints, fingerprint_file_change, load_viewed_store, reconcile_viewed,
    save_viewed_store, toggle_viewed, viewed_identity, viewed_row_ids, workspace_store_id,
    ViewedStore,
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
    /// `files_list_y + files_list_height` down (such as the commit-message
    /// footer) are not file rows.
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
    /// Footer height, the rule row on top and the meta row included.
    pub graph_footer_height: u16,
    /// Max message scroll for the painted footer.
    pub graph_footer_scroll_max: usize,
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
    /// Max message scroll for the painted commit-files footer.
    pub commit_detail_footer_scroll_max: usize,
    /// 0-based row of the graph commit-message footer's top rule when the
    /// footer is expanded and painted: the [`ResizeEdge::CommitMsg`] edge.
    pub commit_msg_rule_y: Option<u16>,
    /// 0-based first column of that rule (the graph pane's first column).
    pub commit_msg_rule_x: u16,
    /// Rule width (the graph pane width).
    pub commit_msg_rule_width: u16,
    /// Most message rows the painted graph pane shows in its expanded
    /// footer ([`workspace_status_graph::commit_msg_lines_fit`]). A drag of
    /// the rule clamps here.
    pub commit_msg_lines_fit: usize,
    /// 0-based diff scrollbar column when a file diff is painted.
    pub diff_scrollbar_x: Option<u16>,
    /// 0-based first row of the diff scrollbar track (body, not header).
    pub diff_scrollbar_y: u16,
    /// Diff body height (vertical scrollbar track).
    pub diff_scrollbar_height: u16,
    /// 0-based diff horizontal scrollbar row when the bar is painted.
    pub diff_hscrollbar_y: Option<u16>,
    /// 0-based first column of the diff horizontal scrollbar track.
    pub diff_hscrollbar_x: u16,
    /// Diff horizontal scrollbar track width.
    pub diff_hscrollbar_width: u16,
    /// Max horizontal pan for the painted file diff.
    pub diff_col_max: u16,
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
    /// Hyperlink spans of the frame, rebuilt every paint: the OSC 8 overlay
    /// prints exactly these.
    ///
    /// Each painted PR badge glyph (from [`AppState::pr_badge_links`]), then
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
    /// File view body width, gutter included.
    pub file_view_width: u16,
    /// File view body height in rows.
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
            graph_footer_scroll_max: 0,
            commit_detail_footer_y: None,
            commit_detail_footer_x: 0,
            commit_detail_footer_width: 0,
            commit_detail_footer_height: 0,
            commit_detail_footer_scroll_max: 0,
            commit_msg_rule_y: None,
            commit_msg_rule_x: 0,
            commit_msg_rule_width: 0,
            commit_msg_lines_fit: COMMIT_MSG_LINES_MAX,
            diff_scrollbar_x: None,
            diff_scrollbar_y: 0,
            diff_scrollbar_height: 0,
            diff_hscrollbar_y: None,
            diff_hscrollbar_x: 0,
            diff_hscrollbar_width: 0,
            diff_col_max: 0,
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
            search_files_rows: 0,
            animating: false,
            gliding: false,
            wake_ms: None,
            search_option_y: 0,
            search_option_hits: Vec::new(),
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
                | IconTarget::GraphRow(_)
                | IconTarget::GraphWorktree(_)
                | IconTarget::GraphChip { .. }
                | IconTarget::GraphSync
                | IconTarget::GraphMoreLines(_)
                | IconTarget::CommitDetailMoreLines(_) => None,
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

/// Workspace chrome parked while a compare or Explorer tab is active.
#[derive(Clone, Debug)]
struct WorkspacePark {
    focus: FocusPane,
    search_mode: bool,
    search_active: bool,
    search_query: String,
    search_target: SearchPane,
    search_hit: Option<usize>,
    left_col_offset: u16,
    right_col_offset: u16,
    diff_col_offset: u16,
    diff_cursor: usize,
    diff_scroll: u16,
    commit_file_folds: HashSet<String>,
    commit_tree_mode: bool,
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

/// Memo for [`AppState::file_search_hits`].
#[derive(Clone, Debug)]
struct FileSearchMemo {
    /// Tab id, generation of the painted body
    /// ([`super::tabs::FileTab::body_generation`]), case-folded query.
    key: (u64, Option<u64>, String),
    /// Shared with every caller of the frame, so a hit does not copy.
    hits: std::rc::Rc<[usize]>,
}

/// Interactive session state. Dispatch is pure besides the returned [`Effect`].
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
    pub viewed_path: PathBuf,
    pub comment_store: CommentStore,
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
    /// The status this instance showed for the latest background fetch
    /// failure. A later success clears the status only while it still
    /// equals this stamp (text and kind).
    fetch_failure_stamp: Option<StatusMessage>,
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
    workspace_park: Option<WorkspacePark>,
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
    last_click: Option<(u16, u16, Instant)>,
    /// PR answers for branch-row badges, per checkout and (remote, branch).
    pr_cache: pull_request::PrCache,
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
    /// Focused diff row's source line, so the blame hook does not rebuild
    /// the diff rows after every schedule and apply.
    line_blame_row_memo: RefCell<line_blame::RowLineMemo>,
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

fn unix_now() -> i64 {
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
        state.activity_sparkline = settings.activity_sparkline;
        state.saved_commit_msg_lines = settings.commit_msg_lines;
        if let Some(lines) = settings.commit_msg_lines {
            state.commit_msg_lines = lines;
        }
        state.open_background_fetch_store(settings.fetch_store.clone());
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
        let signatures = tree_signatures(&tree, &cwd);
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
            viewed_store,
            viewed_path,
            comment_store,
            comment_path,
            ui_store_path,
            fetch_store_path: PathBuf::new(),
            fetch_applied_generation: 0,
            fetch_failure_stamp: None,
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
            last_click: None,
            pr_cache: pull_request::PrCache::default(),
            diff_search_memo: RefCell::new(None),
            file_search_memo: RefCell::new(None),
            explorer_status_memo: RefCell::new(None),
            line_blame: LineBlameState::default(),
            line_blame_row_memo: RefCell::new(None),
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
                self.signatures = tree_signatures(&self.tree, &self.cwd);
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
            InputMode::Popover
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
        at.is_some_and(|t| t.elapsed() <= Duration::from_millis(DOUBLE_TAP_MS))
    }

    /// Drop a stale arming-`g` echo after [`DOUBLE_TAP_MS`].
    ///
    /// Call this before the `g`-chord echo filter so a late `g` is a new
    /// tap. An echo of `t` / `T` / `1`–`9` after a consumed chord stays.
    pub(crate) fn expire_stale_g_chord(&mut self) {
        expire_stale_g_chord_echo(&mut self.g_chord_echo, &mut self.g_pending_at);
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
            && self.graph_pane_focused()
            && self.focused_graph_stash_ref().is_some()
    }

    pub fn graph_commit_focused(&self) -> bool {
        !self.is_explorer_tab()
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
    pub fn graph_chrome(&self) -> GraphChromeBudget {
        self.graph_chrome_in(self.layout.tree_height.max(1))
    }

    /// Header / footer / list split for a graph pane of `height` rows.
    ///
    /// The footer request is fixed ([`graph_footer_request`], same call as
    /// the widget), so the split does not depend on the selected message or
    /// the pane width.
    pub(crate) fn graph_chrome_in(&self, height: u16) -> GraphChromeBudget {
        graph_chrome_budget_for(
            height,
            self.graph.as_ref().is_some_and(|g| g.sync.is_some()),
            graph_footer_request(self.commit_msg_expand, self.commit_msg_lines),
        )
    }

    /// Painted selection-footer lines at `width`, before the footer budget.
    pub(crate) fn graph_footer_line_count(&self, width: usize) -> usize {
        if !self.commit_msg_expand {
            return 2;
        }
        let Some(model) = self.graph.as_ref() else {
            return 2;
        };
        let rows = model.visible_rows();
        let selected = rows.get(self.graph_cursor);
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        selection_footer_parts(
            model,
            GraphFooterSelection::from(selected),
            glyphs,
            width.max(1),
            unix_now(),
            true,
        )
        .len()
    }

    /// Message lines of the graph selection footer below the shown ones:
    /// the count its `more lines below` hint paints. `0` when the footer is
    /// not painted or every line shows.
    pub(crate) fn graph_footer_lines_below(&self) -> usize {
        let layout = &self.layout;
        if layout.graph_footer_y.is_none() {
            return 0;
        }
        // Message rows: the footer less its rule and the pinned meta row.
        let rows = usize::from(layout.graph_footer_height)
            .saturating_sub(usize::from(FOOTER_RULE_ROWS))
            .saturating_sub(1);
        let messages = self
            .graph_footer_line_count(usize::from(layout.graph_footer_width))
            .saturating_sub(1);
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

    /// Graph row identity of the commit or stash the depth-2 commit-files
    /// footer shows: the same key [`Self::graph_footer_msg_scroll`] uses
    /// for that row. `None` for a compare tab and for sources with no
    /// message (worktree, compare, commit vs worktree).
    pub(crate) fn commit_detail_msg_identity(&self) -> Option<String> {
        if self.tabs.active_compare().is_some() {
            return None;
        }
        let (repo, source) = match &self.drill {
            DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. } => {
                (repo, source)
            }
            DrillView::Graph => return None,
        };
        match source {
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
        match (&self.commit_msg_scroll, self.commit_detail_msg_identity()) {
            (Some((id, offset)), Some(current)) if *id == current => *offset,
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
        if self.is_explorer_tab() || (!self.is_compare_tab() && !self.drill.is_diff()) {
            return None;
        }
        let row = self.focused_commit_file_row().filter(|row| row.is_dir())?;
        Some(folder_summary(self.commit_drill_files()?, &row.path))
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

    pub(crate) fn commit_detail_meta(&self) -> (String, Option<String>) {
        if let Some(tab) = self.tabs.active_compare() {
            return (
                super::tabs::checkout_leaf(&tab.checkout_path),
                Some(tab.range_header()),
            );
        }
        let (repo, source) = match &self.drill {
            DrillView::Files { repo, source, .. } | DrillView::Diff { repo, source, .. } => {
                (repo.as_str(), source)
            }
            DrillView::Graph => return (String::new(), None),
        };
        let title = repo.rsplit('/').next().unwrap_or(repo).to_string();
        let subtitle = match source {
            CommitFileSource::Worktree => Some("Uncommitted changes".into()),
            CommitFileSource::Stash { stash_ref } => {
                let subject = self.graph.as_ref().and_then(|model| {
                    model
                        .stashes
                        .iter()
                        .find(|stash| stash.stash_ref == *stash_ref)
                        .map(|stash| stash.subject.clone())
                });
                Some(match subject {
                    Some(subject) if !subject.is_empty() => format!("{stash_ref} · {subject}"),
                    _ => stash_ref.clone(),
                })
            }
            CommitFileSource::Compare {
                base_ref, head_ref, ..
            } => Some(compare_range_label(base_ref, head_ref)),
            CommitFileSource::CommitVsWorktree { .. } => Some(source.short_label()),
            CommitFileSource::Commit { commit_id } => {
                let short = if commit_id.len() >= 7 {
                    &commit_id[..7]
                } else {
                    commit_id.as_str()
                };
                let commit = self
                    .graph
                    .as_ref()
                    .and_then(|model| model.commits.iter().find(|commit| commit.id == *commit_id));
                let refs = commit
                    .map(|commit| {
                        commit
                            .refs
                            .iter()
                            .map(|r| r.name.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    })
                    .unwrap_or_default();
                let subject = commit.map(|c| c.subject.as_str()).unwrap_or("");
                let mut bits = vec![short.to_string()];
                if !refs.is_empty() {
                    bits.push(refs);
                }
                if !subject.is_empty() {
                    bits.push(subject.to_string());
                }
                if let Some(commit) = commit {
                    if !commit.author_name.is_empty() {
                        bits.push(commit.author_name.clone());
                    }
                    if commit.author_date_unix != 0 {
                        bits.push(format_relative_date(commit.author_date_unix, unix_now()));
                    }
                }
                Some(bits.join(" · "))
            }
        };
        (title, subtitle)
    }

    /// Rows the commit-files footer asks for, before the pane-size cap.
    ///
    /// Fixed for the expand state and [`Self::commit_msg_lines`], never sized
    /// by the selected message: expanded is title + meta + N message rows,
    /// collapsed is title + subtitle. This is the painted height (before
    /// the cap), not a line count: [`Self::commit_detail_footer_lines`]
    /// returns the title and meta, then only the real message lines, so a
    /// short message or a source with no message gives fewer lines (the
    /// unused rows paint blank) and a long one gives more (they scroll).
    pub(crate) fn commit_detail_footer_request(&self) -> usize {
        if self.commit_msg_expand {
            2 + self.commit_msg_lines
        } else {
            2
        }
    }

    /// Title plus subtitle / wrapped message for the commit-files footer.
    ///
    /// The footer sits at the bottom of the left commit-files pane beside a
    /// file diff, under the file list. The depth-1 files pane has no footer
    /// (the graph pane beside it already shows one). Collapsed is the title
    /// and the dense one-line subtitle from [`Self::commit_detail_meta`].
    /// Expanded is the title, a meta line (sha / refs / author), then the
    /// whole subject plus body, wrapped one column short of `width` (the
    /// message scrollbar column) with no line cap, like
    /// [`selection_footer_parts`]. Without a message (collapsed, a compare
    /// tab, or a worktree / compare / commit-vs-worktree source) the
    /// subtitle takes the meta row and nothing follows.
    ///
    /// Always the title and the meta / subtitle line, then only the real
    /// message lines: no blank padding. The footer keeps its fixed
    /// [`Self::commit_detail_footer_request`] height in paint, where unused
    /// message rows stay blank and a message taller than the rows scrolls
    /// ([`Self::commit_detail_msg_scroll`]).
    pub(crate) fn commit_detail_footer_lines(&self, width: usize) -> Vec<String> {
        let (title, subtitle) = self.commit_detail_meta();
        let subtitle = subtitle.unwrap_or_default();
        let mut footer = vec![title];
        if !self.commit_msg_expand {
            footer.push(subtitle);
        } else {
            match self.expanded_commit_message() {
                Some((meta, message)) => {
                    footer.push(meta);
                    footer.extend(wrap_commit_message(
                        &message,
                        width.saturating_sub(1).max(1),
                        usize::MAX,
                    ));
                }
                None => footer.push(subtitle),
            }
        }
        footer
    }

    /// Message lines of the depth-2 commit-files footer below the shown
    /// ones: the count its more-below hint paints. `0` when the footer did
    /// not record an overflowing message this frame or every line shows.
    pub(crate) fn commit_detail_footer_lines_below(&self) -> usize {
        let layout = &self.layout;
        if layout.commit_detail_footer_y.is_none() {
            return 0;
        }
        // Message rows: the footer less its rule, title, and meta rows.
        let rows = usize::from(layout.commit_detail_footer_height)
            .saturating_sub(usize::from(FOOTER_RULE_ROWS))
            .saturating_sub(2);
        let messages = self
            .commit_detail_footer_lines(usize::from(layout.commit_detail_footer_width))
            .len()
            .saturating_sub(2);
        if rows == 0 || messages <= rows {
            return 0;
        }
        let offset = self
            .commit_detail_msg_scroll()
            .min(layout.commit_detail_footer_scroll_max);
        messages.saturating_sub(offset + rows)
    }

    fn expanded_commit_message(&self) -> Option<(String, String)> {
        if self.tabs.active_compare().is_some() {
            return None;
        }
        let source = match &self.drill {
            DrillView::Files { source, .. } | DrillView::Diff { source, .. } => source,
            DrillView::Graph => return None,
        };
        match source {
            CommitFileSource::Commit { commit_id } => {
                let commit = self
                    .graph
                    .as_ref()?
                    .commits
                    .iter()
                    .find(|commit| commit.id == *commit_id)?;
                let short = if commit_id.len() >= 7 {
                    &commit_id[..7]
                } else {
                    commit_id.as_str()
                };
                let refs = commit
                    .refs
                    .iter()
                    .map(|r| r.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut bits = vec![short.to_string()];
                if !refs.is_empty() {
                    bits.push(refs);
                }
                if !commit.author_name.is_empty() {
                    bits.push(commit.author_name.clone());
                }
                if commit.author_date_unix != 0 {
                    bits.push(format_relative_date(commit.author_date_unix, unix_now()));
                }
                Some((
                    bits.join(" · "),
                    format_commit_message(&commit.subject, &commit.body),
                ))
            }
            CommitFileSource::Stash { stash_ref } => {
                let stash = self
                    .graph
                    .as_ref()?
                    .stashes
                    .iter()
                    .find(|stash| stash.stash_ref == *stash_ref)?;
                Some((
                    stash.stash_ref.clone(),
                    format_commit_message(&stash.subject, &stash.body),
                ))
            }
            CommitFileSource::Worktree
            | CommitFileSource::Compare { .. }
            | CommitFileSource::CommitVsWorktree { .. } => None,
        }
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
        self.signatures = tree_signatures(&self.tree, &self.cwd);
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

    pub fn apply_snapshot(&mut self, snapshot: WorkspaceSnapshot) {
        let snapshot = carry_status_failed_local_branches(&self.snapshot, snapshot);
        self.snapshot = snapshot;
        self.snapshot.show_hidden = self.show_hidden;
        self.rebuild_rows();
        self.signatures = tree_signatures(&self.tree, &self.cwd);
        self.reconcile_viewed_store();
        self.reconcile_comment_store();
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

    fn ctrl_c(&mut self, now: Instant) -> Effect {
        let result = handle_ctrl_c(self.ctrl_c_armed_until, now);
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
        self.ctrl_c_armed_until = None;
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
        let result = handle_ctrl_c(self.ctrl_c_armed_until, now);
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
                    self.status = StatusMessage::warn(diverged_pull_status(&diverged));
                    Effect::None
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
            graph_scrollbar_x: self.layout.graph_scrollbar_x,
            graph_scrollbar_y: self.layout.graph_scrollbar_y,
            graph_scrollbar_height: self.layout.graph_scrollbar_height,
            graph_content_len: self.layout.graph_content_len,
            graph_scroll: self.graph_painted_top(),
            graph_hscrollbar_y: self.layout.graph_hscrollbar_y,
            graph_hscrollbar_x: self.layout.graph_hscrollbar_x,
            graph_hscrollbar_width: self.layout.graph_hscrollbar_width,
            graph_col_max: self.layout.graph_col_max,
            graph_col_offset: if self.drill.is_files() {
                self.left_col_offset
            } else {
                self.right_col_offset
            },
            diff_hscrollbar_y: self.layout.diff_hscrollbar_y,
            diff_hscrollbar_x: self.layout.diff_hscrollbar_x,
            diff_hscrollbar_width: self.layout.diff_hscrollbar_width,
            diff_col_max: self.layout.diff_col_max,
            diff_col_offset: self.diff_col_offset,
            commit_msg_rule_y: self.layout.commit_msg_rule_y,
            commit_msg_rule_x: self.layout.commit_msg_rule_x,
            commit_msg_rule_width: self.layout.commit_msg_rule_width,
        }
    }

    fn apply_tree_fraction_from_col(&mut self, col: u16) {
        let cols = self.layout.term_cols.max(1);
        self.tree_fraction = clamp_tree_fraction(cols, tree_fraction_from_col(cols, col));
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
        if let Some(effect) = self.popover_click(col, row) {
            return effect;
        }
        let Some(hit) = self.icon_hit_at(col, row).cloned() else {
            return self.click_at(col, row);
        };
        // Cleared before the select too, so an earlier press on this cell
        // (a Ctrl+click that found no PR) never makes this click a drill.
        self.last_click = None;
        let selected = self.click_at(col, row);
        self.last_click = None;
        self.text_selection = None;
        let details = self.pin_icon(&hit);
        popover::then(selected, details)
    }

    /// Plain left press: tabs, dividers, scrollbars, then the row under it.
    fn click_at(&mut self, col: u16, row: u16) -> Effect {
        self.text_selection = None;
        self.pad_press = None;
        if let Some(index) = self.hit_tab_close(col, row) {
            return self.close_tab_at(index);
        }
        if let Some(index) = self.hit_tab(col, row) {
            return self.activate_tab(index);
        }
        match hit_split(self.split_layout(), col, row) {
            SplitHit::Pane if self.background == BackgroundMode::Paint => {
                // A flat divider press sits on a row's pad cell: resize only
                // once the pointer moves, else the release clicks the row
                // (`last_click` stays for its double-click test).
                self.drag = SplitDrag::Pane;
                self.pad_press = Some((col, row));
                return Effect::None;
            }
            SplitHit::Pane => {
                self.drag = SplitDrag::Pane;
                self.apply_tree_fraction_from_col(col);
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::DiffSplit => {
                self.drag = SplitDrag::Diff;
                self.apply_diff_fraction_from_col(col);
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::CommitMsg => {
                // Rows change only once the pointer moves off this row.
                self.drag = SplitDrag::CommitMsg {
                    origin_row: row,
                    origin_lines: self.commit_msg_lines,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphThumb => {
                self.focus_graph_pane();
                // The press ended any glide: keep the rows on screen, so a
                // grab mid-glide does not jump to the glide's target.
                self.graph_scroll = self.graph_painted_top();
                self.drag = SplitDrag::GraphScrollbar {
                    origin_row: row,
                    origin_scroll: self.graph_scroll,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphTrack => {
                self.focus_graph_pane();
                let jumped = graph_scroll_from_row(self.split_layout(), row);
                self.graph_scroll = jumped;
                self.drag = SplitDrag::GraphScrollbar {
                    origin_row: row,
                    origin_scroll: jumped,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphHThumb => {
                self.focus_graph_pane();
                self.drag = SplitDrag::GraphHScrollbar {
                    origin_col: col,
                    origin_offset: self.graph_col_offset(),
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::GraphHTrack => {
                self.focus_graph_pane();
                let jumped = graph_col_from_col(self.split_layout(), col);
                self.set_graph_col_offset(jumped);
                self.drag = SplitDrag::GraphHScrollbar {
                    origin_col: col,
                    origin_offset: jumped,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::DiffHThumb => {
                self.focus = FocusPane::Right;
                self.drag = SplitDrag::DiffHScrollbar {
                    origin_col: col,
                    origin_offset: self.diff_col_offset,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::DiffHTrack => {
                self.focus = FocusPane::Right;
                let jumped = diff_col_from_col(self.split_layout(), col);
                self.diff_col_offset = jumped;
                self.drag = SplitDrag::DiffHScrollbar {
                    origin_col: col,
                    origin_offset: jumped,
                };
                self.last_click = None;
                return Effect::None;
            }
            SplitHit::Other => self.arm_text_selection(col, row),
        }
        self.click_row(col, row)
    }

    /// The row under (`col`, `row`) after [`Self::click_at`] found no tab,
    /// divider, or scrollbar there: focus its pane and select the row; a
    /// second press on the same cell within 400 ms is a double-click.
    fn click_row(&mut self, col: u16, row: u16) -> Effect {
        let now = Instant::now();
        let is_double = self.last_click.as_ref().is_some_and(|(c, r, at)| {
            *c == col && *r == row && at.elapsed() <= Duration::from_millis(400)
        });
        self.last_click = Some((col, row, now));
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
        if row < self.layout.tree_y {
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
            SplitDrag::GraphScrollbar {
                origin_row,
                origin_scroll,
            } => {
                self.graph_scroll =
                    graph_scroll_from_delta(self.split_layout(), origin_row, origin_scroll, row);
            }
            SplitDrag::GraphHScrollbar {
                origin_col,
                origin_offset,
            } => {
                let next =
                    graph_col_from_delta(self.split_layout(), origin_col, origin_offset, col);
                self.set_graph_col_offset(next);
            }
            SplitDrag::DiffHScrollbar {
                origin_col,
                origin_offset,
            } => {
                self.diff_col_offset =
                    diff_col_from_delta(self.split_layout(), origin_col, origin_offset, col);
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
        let clicked = self.click_row(col, row);
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
    fn step_tree_width(&mut self, steps: i32) -> Effect {
        let cols = self.layout.term_cols.max(1);
        let before = pane_widths(cols, self.tree_fraction).tree_width;
        self.tree_fraction = step_tree_fraction(cols, self.tree_fraction, steps);
        if pane_widths(cols, self.tree_fraction).tree_width == before {
            let edge = if steps < 0 { "narrowest" } else { "widest" };
            self.status = StatusMessage::warn(format!("tree pane is at its {edge}"));
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
                let rows = self.commit_file_search_rows()?;
                let current = self.commit_file_search_anchor(&rows);
                (current, commit_file_row_match_indices(&rows, query))
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

    /// Commit-file list flattened with no folds: the search order.
    fn commit_file_search_rows(&self) -> Option<Vec<CommitFileRow>> {
        Some(self.unfolded_commit_file_rows(self.commit_drill_files()?))
    }

    /// Position of the focused commit-file row (file or folder) in `rows`.
    fn commit_file_search_anchor(&self, rows: &[CommitFileRow]) -> Option<usize> {
        let focused = self.focused_commit_file_row()?;
        rows.iter().position(|row| row.id == focused.id)
    }

    /// Commit-file search step from the focused row. Returns true when it
    /// wrapped.
    fn apply_commit_file_search(&mut self, dir: i32) -> bool {
        let Some(rows) = self.commit_file_search_rows() else {
            self.set_search_status(false);
            return false;
        };
        let anchor = self.commit_file_search_anchor(&rows);
        let Some(landing) = focus_commit_file_search(&rows, &self.search_query, anchor, dir) else {
            self.set_search_status(false);
            return false;
        };
        let path = rows[landing.target].path.clone();
        for id in ancestor_dir_ids(&path) {
            self.commit_file_folds.remove(&id);
        }
        self.restore_commit_file_cursor(Some(&path));
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
    /// `DiffContent::from_unified` leaves `is_committed: false`. Workspace
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
    /// checkout `repo` and save the store. Returns the new mark (`true`
    /// when now reviewed), or `None` when the snapshot lists no change for
    /// it (nothing is written).
    pub(super) fn toggle_viewed_for(&mut self, repo: &str, path: &str) -> Option<bool> {
        let full = self
            .snapshot
            .repos
            .iter()
            .find(|r| r.repo == repo)
            .and_then(|r| r.changes.iter().find(|c| c.path == path))?;
        let identity = viewed_identity(repo, path);
        let fingerprint = fingerprint_file_change(&self.cwd, repo, full);
        self.viewed_store = toggle_viewed(&self.viewed_store, &identity, &fingerprint);
        if let Err(err) = save_viewed_store(
            &self.viewed_store,
            &self.viewed_path,
            &workspace_store_id(&self.cwd),
        ) {
            self.status = StatusMessage::error(persist_failed_status("viewed", err));
        }
        self.reviewed = viewed_row_ids(&self.snapshot, &self.viewed_store, &self.cwd);
        Some(self.viewed_store.contains_key(&identity))
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

    fn reconcile_viewed_store(&mut self) {
        let current = collect_current_fingerprints(&self.snapshot, &self.cwd);
        let next = reconcile_viewed(&self.viewed_store, &current);
        if next != self.viewed_store {
            self.viewed_store = next;
            if let Err(err) = save_viewed_store(
                &self.viewed_store,
                &self.viewed_path,
                &workspace_store_id(&self.cwd),
            ) {
                self.status = StatusMessage::error(persist_failed_status("viewed", err));
            }
        }
        self.reviewed = viewed_row_ids(&self.snapshot, &self.viewed_store, &self.cwd);
    }

    fn reconcile_comment_store(&mut self) {
        let live = collect_live_set(&self.snapshot, &self.comment_store);
        let next = gc_comments(&self.comment_store, &live);
        if next != self.comment_store {
            self.comment_store = next;
            if let Err(err) = save_comment_store(
                &self.comment_store,
                &self.comment_path,
                &workspace_store_id(&self.cwd),
            ) {
                self.status = StatusMessage::error(persist_failed_status("comment", err));
            }
        }
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
                let rows = self.current_diff_rows();
                let visual = self.diff_visual_anchor.is_some();
                let (line, end_line) = if let Some(anchor) = self.diff_visual_anchor {
                    viewport_line_range(&rows, anchor, self.diff_cursor)?
                } else {
                    let line = viewport_line_number(&rows, self.diff_cursor as u16)?;
                    (line, line)
                };
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
        match save_comment_store(
            &self.comment_store,
            &self.comment_path,
            &workspace_store_id(&self.cwd),
        ) {
            Ok(()) => {
                self.status = StatusMessage::ok(if empty {
                    "comment deleted"
                } else {
                    "comment saved"
                });
            }
            Err(err) => {
                self.status = StatusMessage::error(persist_failed_status("comment", err));
            }
        }
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
        // The background tick paints nothing unless a repo fails.
        Effect::Fetch { repos: targets }
    }

    /// Use the shared background-fetch store at `path` and start from its
    /// current generation for this workspace, so an outcome recorded before
    /// launch never paints.
    pub(crate) fn open_background_fetch_store(&mut self, path: PathBuf) {
        self.fetch_applied_generation =
            load_background_fetch_record(&path, &workspace_store_id(&self.cwd))
                .map_or(0, |record| record.generation);
        self.fetch_failure_stamp = None;
        self.fetch_store_path = path;
    }

    /// Apply a background-fetch outcome that another instance recorded
    /// since this one last applied or wrote one. Returns true when it did.
    ///
    /// A failure shows the error `fetch failed`. A success clears the
    /// status only while it still shows this instance's last failure stamp.
    /// No progress or summary line. The generation already applied changes
    /// nothing, so a newer local status is never stamped over. Any other
    /// generation applies, a lower one too: the store was deleted or reset.
    pub(crate) fn apply_background_fetch_record(&mut self) -> bool {
        let Some(record) =
            load_background_fetch_record(&self.fetch_store_path, &workspace_store_id(&self.cwd))
        else {
            return false;
        };
        if record.generation == self.fetch_applied_generation {
            return false;
        }
        if record.failed {
            self.status = StatusMessage::error("fetch failed");
        }
        self.note_background_fetch_outcome(record.generation, record.failed);
        true
    }

    /// Mark background-fetch `generation` as applied here.
    ///
    /// `failed`: the current status (the one just painted for that fetch)
    /// becomes the failure stamp. Ok: the status clears if it still equals
    /// the stamp (text and kind), and the stamp is forgotten.
    pub(crate) fn note_background_fetch_outcome(&mut self, generation: u64, failed: bool) {
        self.fetch_applied_generation = generation;
        if failed {
            self.fetch_failure_stamp = Some(self.status.clone());
        } else if self
            .fetch_failure_stamp
            .take()
            .is_some_and(|stamp| stamp == self.status)
        {
            self.status.clear();
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

    fn apply_terminal_size(&mut self, cols: u16) {
        self.layout.term_cols = cols.max(1);
        self.tree_fraction = clamp_tree_fraction(self.layout.term_cols, self.tree_fraction);
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
    /// `None` while mouse capture is off, on an Explorer tab (it has no
    /// pane, RULE, or commit-message drag), below the minimum terminal
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
        hit_split(self.split_layout(), col, row).resize_edge()
    }

    /// True when a left press can land on a resize edge: mouse on, a
    /// Workspace or compare tab with both panes painted, and an input mode
    /// whose clicks reach [`Self::click_at`].
    fn resize_edges_take_mouse(&self) -> bool {
        self.mouse_enabled
            && !self.too_small
            && !self.is_explorer_tab()
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

    fn park_active_session(&mut self) {
        // An Explorer tab owns its pane focus and diff view.
        if self.park_explorer_session() {
            return;
        }
        if let Some(tab) = self.tabs.active_compare_mut() {
            tab.focus_right = self.focus == FocusPane::Right;
            tab.search_mode = self.search_mode;
            tab.search_active = self.search_active;
            tab.search_query = self.search_query.clone();
            tab.search_hit = self.search_hit;
            tab.left_col_offset = self.left_col_offset;
            tab.diff_col_offset = self.diff_col_offset;
            tab.diff_cursor = self.diff_cursor;
            tab.diff_scroll = self.diff_scroll;
            tab.folds = self.commit_file_folds.clone();
            tab.tree_mode = self.commit_tree_mode;
            return;
        }
        self.workspace_park = Some(WorkspacePark {
            focus: self.focus,
            search_mode: self.search_mode,
            search_active: self.search_active,
            search_query: self.search_query.clone(),
            search_target: self.search_target,
            search_hit: self.search_hit,
            left_col_offset: self.left_col_offset,
            right_col_offset: self.right_col_offset,
            diff_col_offset: self.diff_col_offset,
            diff_cursor: self.diff_cursor,
            diff_scroll: self.diff_scroll,
            commit_file_folds: self.commit_file_folds.clone(),
            commit_tree_mode: self.commit_tree_mode,
        });
    }

    fn apply_active_session(&mut self) {
        if self.apply_explorer_session() {
            return;
        }
        if let Some(tab) = self.tabs.active_compare() {
            self.focus = if tab.focus_right {
                FocusPane::Right
            } else {
                FocusPane::Left
            };
            self.search_mode = tab.search_mode;
            self.search_active = tab.search_active;
            self.search_query = tab.search_query.clone();
            self.search_target = if tab.search_active || tab.search_mode {
                if tab.focus_right {
                    SearchPane::Diff
                } else {
                    SearchPane::CommitFiles
                }
            } else {
                SearchPane::CommitFiles
            };
            self.search_hit = tab.search_hit;
            self.left_col_offset = tab.left_col_offset;
            self.diff_col_offset = tab.diff_col_offset;
            self.diff_cursor = tab.diff_cursor;
            self.diff_scroll = tab.diff_scroll;
            self.commit_file_folds = tab.folds.clone();
            self.commit_tree_mode = tab.tree_mode;
            return;
        }
        if let Some(park) = self.workspace_park.take() {
            self.focus = park.focus;
            self.search_mode = park.search_mode;
            self.search_active = park.search_active;
            self.search_query = park.search_query;
            self.search_target = park.search_target;
            self.search_hit = park.search_hit;
            self.left_col_offset = park.left_col_offset;
            self.right_col_offset = park.right_col_offset;
            self.diff_col_offset = park.diff_col_offset;
            self.diff_cursor = park.diff_cursor;
            self.diff_scroll = park.diff_scroll;
            self.commit_file_folds = park.commit_file_folds;
            self.commit_tree_mode = park.commit_tree_mode;
        }
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
        // one check reloads.
        match self.active_worktree_compare_reload() {
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
mod tests {
    use super::super::command_palette::PALETTE_COMMANDS;
    use super::super::comments::put_comment;
    use super::super::gates::ListFocusTarget;
    use super::super::keys::InputMode;
    use super::super::theme::{theme_id_from, ThemeId};
    use super::super::tree::list_viewport_start;
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::testutil::init_repo;
    use crate::tui::split::{
        diff_split_fraction_from_left_width, diff_split_row_width, diff_split_rule_x, pane_widths,
        side_by_side_column_widths, DIFF_SPLIT_FRACTION, MIN_PANE_COLS,
    };
    use crate::tui::status::StatusKind;
    use crate::tui::ui_state_store::load_ui_state;
    use crate::tui::watch::watch_interval_ms;
    use workspace_status_graph::{Commit, GraphModel, GraphRef, Stash};

    /// Palette gate for the catalog row titled `title`.
    fn palette_reason(app: &AppState, title: &str) -> Option<String> {
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == title)
            .unwrap_or_else(|| panic!("no palette row {title}"));
        app.palette_disabled_reason(command)
    }

    fn repo(name: &str, dirty: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: dirty,
            has_staged: false,
            has_untracked: false,
            changes: if dirty {
                vec![FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                }]
            } else {
                vec![]
            },
            checkout_kind: crate::snapshot::CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    fn state() -> AppState {
        let snapshot = build_workspace_snapshot(
            &[repo("app", true), repo("notes", true), repo("lib", false)],
            &["notes".into()],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    #[test]
    fn ignore_toggle_shows_and_hides_notes() {
        let mut app = state();
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
        let effect = app.dispatch(Action::ToggleShowHidden);
        assert_eq!(effect, Effect::LoadRightPane);
        assert!(app.show_hidden);
        assert!(app.rows.iter().any(|r| r.label.contains("notes")));
        app.dispatch(Action::ToggleShowHidden);
        assert!(!app.show_hidden);
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
    }

    fn hidden_primary_family_hidden(app: &AppState) -> bool {
        app.rows.iter().all(|r| r.id != "repo:app")
            && app.rows.iter().all(|r| r.id != "checkout:app")
            && app.rows.iter().all(|r| r.id != "repo:app/.worktrees/feat")
            && app
                .rows
                .iter()
                .all(|r| r.id != "checkout:app/.worktrees/feat")
            && app
                .rows
                .iter()
                .all(|r| !r.label.contains("feature/linked-open"))
    }

    #[test]
    fn dot_toggle_hides_and_shows_linked_child_of_hidden_primary() {
        let mut linked = repo("app/.worktrees/feat", true);
        linked.checkout_kind = CheckoutKind::Linked;
        linked.primary_repo = Some("app".into());
        linked.branch = "feature/linked-open".into();
        let snapshot = build_workspace_snapshot(
            &[repo("app", true), linked, repo("lib", true)],
            &["app".into()],
            false,
            &[],
        );
        assert!(
            snapshot.repos.iter().any(|r| r.repo == "app" && r.hidden),
            "primary stays tagged hidden"
        );
        assert!(
            snapshot
                .repos
                .iter()
                .any(|r| r.repo == "app/.worktrees/feat" && !r.hidden),
            "linked child must not be retagged hidden"
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        assert!(
            app.rows.iter().any(|r| r.id == "repo:lib"),
            "visible lib must still paint"
        );
        assert!(
            hidden_primary_family_hidden(&app),
            "cold start must hide hidden primary and its linked child, got {:?}",
            app.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
        );

        let effect = app.dispatch(Action::ToggleShowHidden);
        assert_eq!(effect, Effect::LoadRightPane);
        assert!(app.show_hidden);
        assert!(app.rows.iter().any(|r| r.id == "repo:app"));
        assert!(app
            .rows
            .iter()
            .any(|r| r.id == "checkout:app/.worktrees/feat"));
        assert!(
            app.rows.iter().all(|r| r.id != "repo:app/.worktrees/feat"),
            "shown family must nest the linked checkout"
        );
        assert!(app
            .rows
            .iter()
            .any(|r| r.label.contains("feature/linked-open")));

        app.dispatch(Action::ToggleShowHidden);
        assert!(!app.show_hidden);
        assert!(
            hidden_primary_family_hidden(&app),
            "second toggle must hide both again, got {:?}",
            app.rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn hidden_skipped_on_workspace_fetch() {
        let mut app = state();
        app.cursor = 0;
        let effect = app.dispatch(Action::Fetch);
        match effect {
            Effect::Fetch { repos } => {
                assert_eq!(repos, vec!["app", "lib"]);
                assert!(!repos.iter().any(|r| r == "notes"));
            }
            other => panic!("expected fetch, got {other:?}"),
        }
        assert_eq!(app.status, "Fetching 0/2…");
    }

    #[test]
    fn workspace_pull_and_default_arm_running_op_progress() {
        let mut app_snap = repo("app", false);
        app_snap.sync_status = SyncStatus::Behind;
        app_snap.branch = "feature/x".into();
        let mut lib_snap = repo("lib", false);
        lib_snap.sync_status = SyncStatus::Behind;
        lib_snap.branch = "feature/y".into();
        let snapshot = build_workspace_snapshot(&[app_snap, lib_snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        match app.dispatch(Action::Pull) {
            Effect::Pull { repos } => assert_eq!(repos, vec!["app", "lib"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "Pulling 0/2…");
        // Two repos: `d` asks first and writes nothing until `y`.
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert_eq!(
            app.confirm,
            Some(PendingConfirm::SwitchToDefault {
                repos: vec!["app".into(), "lib".into()],
            })
        );
        match app.dispatch(Action::ConfirmYes) {
            Effect::DefaultBranch { repos } => assert_eq!(repos, vec!["app", "lib"]),
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "Switching 0/2…");
    }

    #[test]
    fn multi_repo_default_switch_confirms_and_n_cancels() {
        let mut app_snap = repo("app", false);
        app_snap.branch = "feature/x".into();
        let mut lib_snap = repo("lib", false);
        lib_snap.branch = "feature/y".into();
        let snapshot = build_workspace_snapshot(&[app_snap, lib_snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert!(matches!(
            app.confirm,
            Some(PendingConfirm::SwitchToDefault { ref repos }) if repos.len() == 2
        ));
        // Enter is not a yes: the box stays and the status names the key.
        assert_eq!(app.dispatch(Action::ConfirmEnter), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.status, "press y to confirm · n or Esc to cancel");
        assert_eq!(app.status.kind(), StatusKind::Info);
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "switch cancelled");

        // One repo in scope still switches at once.
        focus_repo(&mut app, "app");
        match app.dispatch(Action::DefaultBranch) {
            Effect::DefaultBranch { repos } => assert_eq!(repos, vec!["app"]),
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
    }

    #[test]
    fn confirm_enter_names_shift_y_when_only_y_upper_accepts() {
        let mut app = state();
        let target = |path: &str| RevertTarget {
            repo: "app".into(),
            path: path.into(),
            untracked: true,
            old_path: None,
        };
        app.confirm = Some(PendingConfirm::Revert {
            targets: vec![target("a.txt"), target("b.txt")],
            label: "app".into(),
        });
        assert_eq!(app.dispatch(Action::ConfirmEnter), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.status, "press Y to confirm · n or Esc to cancel");
        // No confirm open: Enter in that mode cannot reach here, but stays inert.
        app.confirm = None;
        app.status.clear();
        assert_eq!(app.dispatch(Action::ConfirmEnter), Effect::None);
        assert!(app.status.is_empty());
    }

    #[test]
    fn shown_hidden_is_in_workspace_fetch() {
        let mut app = state();
        app.dispatch(Action::ToggleShowHidden);
        app.cursor = 0;
        let effect = app.dispatch(Action::Fetch);
        match effect {
            Effect::Fetch { repos } => {
                assert!(repos.contains(&"app".to_string()));
                assert!(repos.contains(&"notes".to_string()));
            }
            other => panic!("expected fetch, got {other:?}"),
        }
    }

    #[test]
    fn space_toggles_reviewed_on_file_only() {
        let mut app = state();
        let file_idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file row");
        app.cursor = file_idx;
        let id = app.rows[file_idx].id.clone();
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(app.reviewed.contains(&id));
        app.cursor = 0;
        app.dispatch(Action::ToggleReviewed);
        assert!(app.reviewed.contains(&id));
    }

    fn sample_commit_files() -> Vec<CommitFile> {
        vec![CommitFile {
            status: "M".into(),
            path: "README.md".into(),
            old_path: None,
            stat: None,
        }]
    }

    fn sample_commit_source() -> CommitFileSource {
        CommitFileSource::Commit {
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        }
    }

    fn two_commit_files() -> Vec<CommitFile> {
        vec![
            CommitFile {
                status: "M".into(),
                path: "one.rs".into(),
                old_path: None,
                stat: None,
            },
            CommitFile {
                status: "M".into(),
                path: "two.rs".into(),
                old_path: None,
                stat: None,
            },
        ]
    }

    fn tall_panning_diff() -> DiffContent {
        let mut body = format!("@@ -0,0 +1,41 @@\n+{}\n", "x".repeat(40));
        for i in 0..40 {
            body.push_str(&format!("+line {i}\n"));
        }
        DiffContent::from_unified(body)
    }

    fn pan_and_scroll_focused_diff(app: &mut AppState) {
        app.diff_wrap = false;
        app.focus = FocusPane::Right;
        // Wide enough for a one-row `app/<path>` header, narrow enough to pan.
        app.layout.diff_pane_width = 16;
        app.layout.diff_pane_height = 8;
        let steps = (app.diff_body_height() / 2 + 8) as i32;
        app.dispatch(Action::Move(steps));
        for _ in 0..8 {
            app.dispatch(Action::PanDiff(1));
        }
        assert!(
            app.diff_cursor > 0 && app.diff_scroll > 0 && app.diff_col_offset > 0,
            "need a stale viewport, cursor={} scroll={} pan={}",
            app.diff_cursor,
            app.diff_scroll,
            app.diff_col_offset
        );
    }

    #[test]
    fn tree_writes_say_why_when_depth_at_least_one_and_right_focused() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let cursor = app.cursor;
        let id = app.rows[cursor].id.clone();
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
        );
        assert_eq!(app.focus, FocusPane::Right);
        assert!(app.in_commit_drill());
        for (action, why) in [
            (Action::Stage, "Esc back to the tree to stage"),
            (Action::Unstage, "Esc back to the tree to unstage"),
            (Action::Revert, "Esc back to the tree to revert"),
            (Action::Fetch, "Esc back to the tree to fetch"),
            (Action::Pull, "Esc back to the tree to pull"),
            (Action::Push, "Esc back to the tree to push"),
            (
                Action::DefaultBranch,
                "Esc back to the tree to switch to the default branch",
            ),
            (Action::Branch, "Esc back to the tree to pick a branch"),
            (
                Action::RemoveWorktree,
                "Esc back to the tree to remove a worktree",
            ),
            (Action::StashMenu, "focus the left pane (Tab) to stash"),
        ] {
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(
                app.cursor, cursor,
                "{action:?} must not move the tree cursor"
            );
            assert!(app.confirm.is_none(), "{action:?}");
            assert_eq!(app.status, why, "{action:?} must say why");
            assert_eq!(app.status.kind(), StatusKind::Warn, "{action:?}");
        }
        match app.dispatch(Action::Edit) {
            Effect::EditFile { path, .. } => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(!app.reviewed.contains(&id));
        assert_eq!(app.status, REVIEWED_MARKS_ARE_FOR_TREE_FILES);
        assert_eq!(
            palette_reason(&app, "Mark reviewed").as_deref(),
            Some(REVIEWED_MARKS_ARE_FOR_TREE_FILES)
        );
    }

    #[test]
    fn pull_and_default_on_file_or_dir_say_why_fetch_stays() {
        let mut snap = tree_repo();
        snap.branch = "feature/x".into();
        snap.sync_status = SyncStatus::Behind;
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_id(&mut app, "file:app:README.md");
        assert_eq!(app.dispatch(Action::Pull), Effect::None);
        assert_eq!(app.status, "focus a repo or checkout to pull");
        assert_eq!(app.status.kind(), StatusKind::Warn);
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert_eq!(app.status, "focus a repo or checkout to switch to default");
        match app.dispatch(Action::Fetch) {
            Effect::Fetch { repos } => assert_eq!(repos, vec!["app"]),
            other => panic!("{other:?}"),
        }
        focus_id(&mut app, "dir:app:src");
        assert_eq!(app.dispatch(Action::Pull), Effect::None);
        assert_eq!(app.status, "focus a repo or checkout to pull");
        assert_eq!(app.status.kind(), StatusKind::Warn);
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert_eq!(app.status, "focus a repo or checkout to switch to default");
        match app.dispatch(Action::Fetch) {
            Effect::Fetch { repos } => assert_eq!(repos, vec!["app"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn space_toggles_reviewed_only_at_depth_zero_workspace_file() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        app.focus = FocusPane::Right;
        assert!(app.right_is_diff());
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(app.reviewed.contains(&id));
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
        );
        app.dispatch(Action::ToggleReviewed);
        assert!(app.reviewed.contains(&id), "depth 1 must not unmark");
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["+line".into()]),
        );
        app.dispatch(Action::ToggleReviewed);
        assert!(app.reviewed.contains(&id), "depth 2 must not unmark");
    }

    #[test]
    fn ctrl_o_fires_from_left_when_right_is_already_a_diff() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        app.focus = FocusPane::Left;
        assert!(app.right_is_diff());
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert!(app.full_context_active());

        let mut commit = state();
        focus_repo(&mut commit, "app");
        commit.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        commit.focus = FocusPane::Left;
        match commit.dispatch(Action::ToggleFullContext) {
            Effect::LoadCommitDiff { path, .. } => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
        assert!(commit.full_context_active());
    }

    #[test]
    fn remove_worktree_refuses_with_status_when_not_linked() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        assert!(app.confirm.is_none());
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        let file_idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file");
        app.cursor = file_idx;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        assert!(app.confirm.is_none());
        focus_checkout(&mut app, "app/.worktrees/feat");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        let want_changed = app
            .snapshot
            .repos
            .iter()
            .find(|r| r.repo == "app/.worktrees/feat")
            .map(|r| r.changes.len())
            .unwrap();
        assert!(matches!(
            app.confirm,
            Some(PendingConfirm::RemoveWorktree { changed, .. }) if changed == want_changed
        ));
        assert!(app.confirm.is_some());
        app.dispatch(Action::ConfirmNo);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn workspace_default_branch_skips_default_and_hidden() {
        let mut app = state();
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert!(app.status.contains("no non-default"));
    }

    #[test]
    fn default_branch_on_focused_default_repo_is_noop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
        assert!(app.status.contains("no non-default"));
        let snapshot = build_workspace_snapshot(
            &[RepoSnapshot {
                repo: "app".into(),
                branch: "feature/x".into(),
                sync_status: SyncStatus::NoUpstream,
                sync_note: String::new(),
                head: String::new(),
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
                merged_into_default: None,
                default_branch_override: None,
                default_tip_ref: None,
                local_branches: Vec::new(),
            }],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        match app.dispatch(Action::DefaultBranch) {
            Effect::DefaultBranch { repos } => assert_eq!(repos, vec!["app"]),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "Switching 0/1…");
    }

    fn two_file_repo() -> RepoSnapshot {
        RepoSnapshot {
            repo: "app".into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: true,
            changes: vec![
                FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                },
                FileChange {
                    path: "new.txt".into(),
                    staged_status: None,
                    unstaged_status: None,
                    untracked: true,
                    old_path: None,
                },
            ],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    #[test]
    fn bulk_stage_on_repo_and_file_rows() {
        let snapshot = build_workspace_snapshot(
            &[two_file_repo(), repo("notes", true)],
            &["notes".into()],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        focus_repo(&mut app, "app");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, mut paths } => {
                assert_eq!(repo, "app");
                paths.sort();
                assert_eq!(paths, vec!["README.md", "new.txt"]);
            }
            other => panic!("{other:?}"),
        }
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Stage) {
            Effect::Stage { paths, .. } => assert_eq!(paths, vec!["README.md"]),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn bulk_stage_on_checkout_not_family_container() {
        let mut linked = two_file_repo();
        linked.repo = ".worktrees/app/feat".into();
        linked.checkout_kind = CheckoutKind::Linked;
        linked.primary_repo = Some("app".into());
        linked.has_untracked = false;
        linked.changes = vec![FileChange {
            path: "wt.md".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        let snapshot = build_workspace_snapshot(&[two_file_repo(), linked], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        let idx = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Checkout && row.repo.as_deref() == Some("app"))
            .expect("primary checkout");
        app.cursor = idx;
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, mut paths } => {
                assert_eq!(repo, "app");
                paths.sort();
                assert_eq!(paths, vec!["README.md", "new.txt"]);
            }
            other => panic!("{other:?}"),
        }
        let wt = app
            .rows
            .iter()
            .position(|row| {
                row.kind == NodeKind::Checkout && row.repo.as_deref() == Some(".worktrees/app/feat")
            })
            .expect("linked checkout");
        app.cursor = wt;
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, ".worktrees/app/feat");
                assert_eq!(paths, vec!["wt.md"]);
            }
            other => panic!("{other:?}"),
        }
    }

    fn mixed_dirty(name: &str) -> RepoSnapshot {
        let mut snap = two_file_repo();
        snap.repo = name.into();
        snap
    }

    #[test]
    fn workspace_stage_unstage_revert_are_noop() {
        let snapshot =
            build_workspace_snapshot(&[mixed_dirty("app"), mixed_dirty("api")], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(
            app.focused_row().map(|row| row.kind),
            Some(NodeKind::Workspace)
        );
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        assert_eq!(app.dispatch(Action::Unstage), Effect::None);
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn no_updates_group_fetch_pull_default_are_noop() {
        let mut app = state();
        let idx = app
            .rows
            .iter()
            .position(|row| row.id == "group:no-updates")
            .expect("no-updates group");
        app.cursor = idx;
        assert_eq!(app.dispatch(Action::Fetch), Effect::None);
        assert_eq!(app.dispatch(Action::Pull), Effect::None);
        assert_eq!(app.dispatch(Action::DefaultBranch), Effect::None);
    }

    #[test]
    fn refresh_on_checkout_names_that_repo() {
        let mut app = state();
        focus_repo(&mut app, "app");
        match app.dispatch(Action::Refresh) {
            Effect::ReloadRepo { repo } => assert_eq!(repo, "app"),
            other => panic!("{other:?}"),
        }
        app.cursor = 0;
        assert_eq!(
            app.focused_row().map(|row| row.kind),
            Some(NodeKind::Workspace)
        );
        assert_eq!(app.dispatch(Action::Refresh), Effect::ReloadSnapshot);
        let idx = app
            .rows
            .iter()
            .position(|row| row.id == "group:no-updates")
            .expect("no-updates group");
        app.cursor = idx;
        assert_eq!(app.dispatch(Action::Refresh), Effect::ReloadSnapshot);
    }

    #[test]
    fn refresh_on_linked_checkout_names_that_path() {
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", true),
                RepoSnapshot {
                    repo: ".worktrees/app/feat".into(),
                    branch: "feature/x".into(),
                    sync_status: SyncStatus::NoUpstream,
                    sync_note: String::new(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: None,
                    default_branch_override: None,
                    default_tip_ref: None,
                    local_branches: Vec::new(),
                },
            ],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let idx = app
            .rows
            .iter()
            .position(|row| {
                row.kind == NodeKind::Checkout && row.repo.as_deref() == Some(".worktrees/app/feat")
            })
            .expect("linked checkout");
        app.cursor = idx;
        match app.dispatch(Action::Refresh) {
            Effect::ReloadRepo { repo } => assert_eq!(repo, ".worktrees/app/feat"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn revert_y_keeps_untracked_on_mixed_scope_y_clean_deletes() {
        let snapshot = build_workspace_snapshot(&[two_file_repo()], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert_eq!(tracked, vec!["README.md"]);
                assert!(untracked.is_empty());
            }
            other => panic!("{other:?}"),
        }
        focus_repo(&mut app, "app");
        app.dispatch(Action::Revert);
        match app.dispatch(Action::ConfirmYesClean) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert_eq!(tracked, vec!["README.md"]);
                assert_eq!(untracked, vec!["new.txt"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn revert_key_not_offered_keeps_confirm_open() {
        // Tracked-only scope: `Y` is not offered.
        let mut app = state();
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        let status = app.status.clone();
        assert_eq!(app.dispatch(Action::ConfirmYesClean), Effect::None);
        assert!(app.confirm.is_some(), "`Y` must keep the confirm open");
        assert_eq!(app.status, status);
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert_eq!(tracked, vec!["README.md"]);
                assert!(untracked.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());

        let untracked_confirm = |paths: &[&str]| PendingConfirm::Revert {
            targets: paths
                .iter()
                .map(|path| RevertTarget {
                    repo: "app".into(),
                    path: (*path).into(),
                    untracked: true,
                    old_path: None,
                })
                .collect(),
            label: paths[0].into(),
        };

        // One untracked file: `y` deletes it, `Y` is not offered.
        app.confirm = Some(untracked_confirm(&["new.txt"]));
        assert_eq!(app.dispatch(Action::ConfirmYesClean), Effect::None);
        assert!(app.confirm.is_some());
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert!(tracked.is_empty());
                assert_eq!(untracked, vec!["new.txt"]);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "deleting new.txt…");
        assert_eq!(app.status.kind(), StatusKind::Progress);

        // Many untracked files: `y` is not offered, `Y` deletes them all.
        app.confirm = Some(untracked_confirm(&["a.txt", "b.txt"]));
        assert_eq!(app.dispatch(Action::ConfirmYes), Effect::None);
        assert!(app.confirm.is_some(), "`y` must keep the confirm open");
        match app.dispatch(Action::ConfirmYesClean) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert!(tracked.is_empty());
                assert_eq!(untracked, vec!["a.txt", "b.txt"]);
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "deleting 2 untracked…");
    }

    #[test]
    fn compare_picker_says_when_the_filter_matches_nothing_and_on_esc() {
        let mut app = state();
        let branch = |name: &str| crate::git::LocalBranch {
            name: name.into(),
            current: false,
            authordate: 0,
        };
        app.open_compare_picker("app".into(), Vec::new());
        assert_eq!(app.dispatch(Action::ComparePickerSubmit), Effect::None);
        assert_eq!(app.status, "No branches to compare");

        app.open_compare_picker("app".into(), vec![branch("develop")]);
        app.dispatch(Action::ComparePickerChar('z'));
        app.dispatch(Action::ComparePickerChar('q'));
        assert_eq!(app.dispatch(Action::ComparePickerSubmit), Effect::None);
        assert_eq!(app.status, "no branch matches zq");
        assert_eq!(app.status.kind(), StatusKind::Warn);

        assert_eq!(app.dispatch(Action::ComparePickerCancel), Effect::None);
        assert!(app.compare_picker.is_none());
        assert_eq!(app.status, "compare cancelled");
        assert_eq!(app.status.kind(), StatusKind::Info);
    }

    #[test]
    fn compare_commit_picker_filters_and_opens_a_live_tab_at_the_picked_commit() {
        const OLD: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        const NEW: &str = "ccc2222ddddddddddddddddddddddddddddddddd";
        let mut app = state();
        let commit = |id: &str, subject: &str| crate::git::AncestorCommit {
            id: id.into(),
            subject: subject.into(),
        };
        app.open_compare_commit_picker("app".into(), Vec::new());
        assert_eq!(app.dispatch(Action::ComparePickerSubmit), Effect::None);
        assert_eq!(app.status, "No commits to compare");

        app.open_compare_commit_picker(
            "app".into(),
            vec![commit(NEW, "Add parser"), commit(OLD, "Initial import")],
        );
        let labels = |app: &AppState| {
            app.compare_picker
                .as_ref()
                .unwrap()
                .window_labels(0, usize::MAX)
        };
        assert_eq!(
            labels(&app),
            vec!["ccc2222  Add parser", "aaa1111  Initial import"]
        );
        for c in "AAA1".chars() {
            app.dispatch(Action::ComparePickerChar(c));
        }
        assert_eq!(labels(&app), vec!["aaa1111  Initial import"]);
        for _ in 0..4 {
            app.dispatch(Action::ComparePickerBackspace);
        }
        for c in "pARSER".chars() {
            app.dispatch(Action::ComparePickerChar(c));
        }
        assert_eq!(labels(&app), vec!["ccc2222  Add parser"]);
        app.dispatch(Action::ComparePickerChar('z'));
        assert_eq!(app.dispatch(Action::ComparePickerSubmit), Effect::None);
        assert_eq!(app.status, "no commit matches pARSERz");
        assert_eq!(app.status.kind(), StatusKind::Warn);

        for _ in 0.."pARSER".len() {
            app.dispatch(Action::ComparePickerBackspace);
        }
        app.dispatch(Action::ComparePickerMove(1));
        match app.dispatch(Action::ComparePickerSubmit) {
            Effect::LoadCompareRange {
                repo,
                base_ref,
                head_ref,
                ..
            } => {
                assert_eq!(repo, "app");
                assert_eq!(base_ref, OLD);
                assert_eq!(head_ref, COMPARE_HEAD_REF);
            }
            other => panic!("{other:?}"),
        }
        assert!(app.compare_picker.is_none());
        let tab = app.tabs.active_compare_mut().unwrap();
        assert_eq!(tab.label(), "app ↔ aaa1111");
        assert!(!tab.is_pinned());

        app.dispatch(Action::JumpToTab(1));
        app.open_compare_commit_picker("app".into(), vec![commit(OLD, "Initial import")]);
        assert_eq!(app.dispatch(Action::ComparePickerCancel), Effect::None);
        assert!(app.compare_picker.is_none());
        assert_eq!(app.status, "compare cancelled");
        assert_eq!(app.status.kind(), StatusKind::Info);
    }

    #[test]
    fn branch_picker_typing_keeps_the_filter_off_the_status_rows() {
        let mut app = state();
        app.branch_picker = Some(crate::tui::branches::BranchPickerState::new(
            "app".into(),
            vec![crate::git::LocalBranch {
                name: "topic".into(),
                current: false,
                authordate: 0,
            }],
        ));
        app.dispatch(Action::BranchChar('t'));
        assert_eq!(
            app.branch_picker.as_ref().map(|p| p.filter.as_str()),
            Some("t")
        );
        assert!(
            app.status.is_empty(),
            "title paints the filter: {}",
            app.status
        );
        app.dispatch(Action::BranchBackspace);
        assert!(app.status.is_empty());
    }

    #[test]
    fn pull_on_a_diverged_repo_says_why_not_nothing_behind() {
        let mut diverged = repo("app", false);
        diverged.sync_status = SyncStatus::Diverged;
        diverged.sync_note = "ahead 1, behind 1".into();
        let snapshot = build_workspace_snapshot(&[diverged], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Pull), Effect::None);
        assert_eq!(app.status, "app has diverged — pull it from a terminal");
        assert_eq!(app.status.kind(), StatusKind::Warn);
    }

    #[test]
    fn workspace_push_asks_for_a_checkout() {
        let mut ahead = repo("app", false);
        ahead.sync_status = SyncStatus::Ahead;
        ahead.sync_note = "ahead 1".into();
        let snapshot = build_workspace_snapshot(&[ahead], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(app.focused_row().map(|r| r.kind), Some(NodeKind::Workspace));
        assert_eq!(app.dispatch(Action::Push), Effect::None);
        assert_eq!(app.status, "focus a repo or checkout to push");
        assert_eq!(app.status.kind(), StatusKind::Warn);
    }

    #[test]
    fn push_in_sync_is_noop_ahead_allowed() {
        let mut in_sync = repo("app", false);
        in_sync.sync_status = SyncStatus::UpToDate;
        let mut ahead = repo("lib", false);
        ahead.sync_status = SyncStatus::Ahead;
        ahead.sync_note = "ahead 1".into();
        let snapshot = build_workspace_snapshot(&[in_sync, ahead], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.folds.remove("group:no-updates");
        app.rebuild_rows();
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Push), Effect::None);
        assert!(app.status.contains("nothing to push"));
        focus_repo(&mut app, "lib");
        assert_eq!(confirm_and_push(&mut app, Action::Push, "lib"), vec!["lib"]);
    }

    fn push_plan_ok(branch: &str, publish: bool) -> Result<PushPlan, String> {
        Ok(PushPlan {
            branch: branch.into(),
            remote: "origin".into(),
            publish,
        })
    }

    /// `action` asks git for the push plan of `repo` (no push yet); the
    /// plan opens the confirm; `y` returns the push and its targets.
    fn confirm_and_push(app: &mut AppState, action: Action, repo: &str) -> Vec<String> {
        assert_eq!(
            app.dispatch(action),
            Effect::PreparePush { repo: repo.into() }
        );
        assert!(app.confirm.is_none(), "the confirm waits for the plan");
        app.open_push_confirm(repo.into(), push_plan_ok("main", false));
        assert!(
            matches!(app.confirm, Some(PendingConfirm::Push { .. })),
            "{:?}",
            app.confirm
        );
        let repos = match app.dispatch(Action::ConfirmYes) {
            Effect::Push { repos } => repos,
            other => panic!("{other:?}"),
        };
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "Pushing 0/1…");
        repos
    }

    /// One checkout `app` on `main`, ahead of its upstream, focused.
    fn ahead_app(sync: SyncStatus, note: &str) -> AppState {
        let mut ahead = repo("app", false);
        ahead.sync_status = sync;
        ahead.sync_note = note.into();
        let snapshot = build_workspace_snapshot(&[ahead], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_repo(&mut app, "app");
        app
    }

    #[test]
    fn push_opens_a_confirm_and_only_y_pushes() {
        let mut app = ahead_app(SyncStatus::Ahead, "ahead by 3 commits");
        assert_eq!(
            app.dispatch(Action::Push),
            Effect::PreparePush { repo: "app".into() }
        );
        app.help_open = true;
        app.open_push_confirm("app".into(), push_plan_ok("main", false));
        assert!(!app.help_open, "the confirm closes help");
        assert_eq!(
            app.confirm,
            Some(PendingConfirm::Push {
                repo: "app".into(),
                branch: "main".into(),
                remote: "origin".into(),
                publish: false,
                ahead_behind: Some((3, 0)),
            })
        );
        assert_eq!(app.confirm.as_ref().map(|c| c.accept_key()), Some('y'));
        assert!(!app.confirm.as_ref().unwrap().is_compare_owned());

        // Enter does not accept: it names the key.
        assert_eq!(app.dispatch(Action::ConfirmEnter), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.status, "press y to confirm · n or Esc to cancel");
        // `Y` is not offered: the box stays open.
        assert_eq!(app.dispatch(Action::ConfirmYesClean), Effect::None);
        assert!(app.confirm.is_some());
        // `y` pushes the focused checkout.
        assert_eq!(
            app.dispatch(Action::ConfirmYes),
            Effect::Push {
                repos: vec!["app".into()]
            }
        );
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "Pushing 0/1…");
    }

    #[test]
    fn push_confirm_n_cancels() {
        let mut app = ahead_app(SyncStatus::Ahead, "ahead by 1 commits");
        app.open_push_confirm("app".into(), push_plan_ok("main", false));
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "push cancelled");
        // A cancel is not a one-shot ack: it stays on the status line and
        // never moves to a toast.
        assert_ne!(app.status.kind(), StatusKind::Ok);
        app.route_ack_to_toast(std::time::Instant::now());
        assert_eq!(app.status, "push cancelled");
    }

    #[test]
    fn push_with_nothing_to_push_opens_no_confirm() {
        let mut app = ahead_app(SyncStatus::Ahead, "ahead by 1 commits");
        app.cursor = 0;
        assert_eq!(app.focused_row().map(|r| r.kind), Some(NodeKind::Workspace));
        assert_eq!(app.dispatch(Action::Push), Effect::None);
        assert_eq!(app.status, "focus a repo or checkout to push");
        assert!(app.confirm.is_none());

        let mut in_sync = repo("app", false);
        in_sync.sync_status = SyncStatus::UpToDate;
        let snapshot = build_workspace_snapshot(&[in_sync], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.folds.remove("group:no-updates");
        app.rebuild_rows();
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::Push), Effect::None);
        assert_eq!(app.status, "nothing to push");
        assert!(app.confirm.is_none());
    }

    #[test]
    fn push_confirm_refuses_detached_head_and_a_stale_snapshot() {
        let mut app = ahead_app(SyncStatus::Ahead, "ahead by 1 commits");
        app.open_push_confirm("app".into(), Err("detached HEAD cannot push".into()));
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "detached HEAD cannot push");
        assert_eq!(app.status.kind(), StatusKind::Error);

        // A reload between `P` and the plan found nothing left to push.
        app.snapshot.repos[0].sync_status = SyncStatus::UpToDate;
        app.open_push_confirm("app".into(), push_plan_ok("main", false));
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "nothing to push");
        assert_eq!(app.status.kind(), StatusKind::Warn);
    }

    #[test]
    fn push_plan_does_not_replace_an_open_confirm() {
        let mut app = ahead_app(SyncStatus::Ahead, "ahead by 1 commits");
        let drop = PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        };
        app.confirm = Some(drop.clone());
        app.open_push_confirm("app".into(), push_plan_ok("main", false));
        assert_eq!(app.confirm, Some(drop));
    }

    #[test]
    fn push_confirm_counts_come_from_the_sync_note() {
        assert_eq!(
            push_ahead_behind(SyncStatus::Ahead, "ahead by 3 commits"),
            Some((3, 0))
        );
        assert_eq!(
            push_ahead_behind(SyncStatus::Diverged, "diverged (ahead 2, behind 5)"),
            Some((2, 5))
        );
        assert_eq!(push_ahead_behind(SyncStatus::NoUpstream, ""), None);
        assert_eq!(push_ahead_behind(SyncStatus::Ahead, "ahead 1"), None);
        assert_eq!(
            push_ahead_behind(SyncStatus::Diverged, "diverged (ahead x, behind 1)"),
            None
        );
    }

    #[test]
    fn palette_push_goes_through_the_confirm() {
        let mut app = ahead_app(SyncStatus::Ahead, "ahead by 1 commits");
        palette_select(&mut app, "Push");
        assert_eq!(
            confirm_and_push(&mut app, Action::QuickOpenSubmit, "app"),
            vec!["app"]
        );
    }

    #[test]
    fn help_toggle_does_not_quit() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::ToggleHelp), Effect::None);
        assert!(app.help_open);
        assert_eq!(app.dispatch(Action::Quit), Effect::Quit);
    }

    #[test]
    fn ctrl_c_prompts_then_quits_within_the_window() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        assert_eq!(app.status, CTRL_C_EXIT_PROMPT);
        assert!(app.ctrl_c_armed_until.is_some());
        assert_eq!(app.dispatch(Action::CtrlC), Effect::Quit);
    }

    #[test]
    fn ctrl_c_expired_arm_is_a_fresh_prompt_not_quit() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        let until = app.ctrl_c_armed_until.expect("armed");
        assert!(!app.expire_ctrl_c_prompt(until - Duration::from_millis(1)));
        assert_eq!(app.status, CTRL_C_EXIT_PROMPT);
        assert!(app.expire_ctrl_c_prompt(until));
        assert!(app.status.is_empty());
        assert!(app.ctrl_c_armed_until.is_none());
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        assert_eq!(app.status, CTRL_C_EXIT_PROMPT);
    }

    #[test]
    fn other_keys_do_not_disarm_ctrl_c() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::CtrlC), Effect::None);
        let _ = app.dispatch(Action::Move(1));
        assert_eq!(app.dispatch(Action::CtrlC), Effect::Quit);
    }

    #[test]
    fn q_still_quits_immediately() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::Quit), Effect::Quit);
    }

    #[test]
    fn q_while_busy_arms_the_press_again_prompt() {
        let mut app = state();
        let t0 = Instant::now();
        assert_eq!(app.quit_while_busy(t0), Effect::None);
        assert_eq!(app.status, QUIT_WHILE_BUSY_PROMPT);
        assert_eq!(app.status.kind(), StatusKind::Warn);
        assert_eq!(
            app.quit_while_busy(t0 + Duration::from_millis(100)),
            Effect::Quit
        );

        // `q` then Ctrl-C inside the window also quits.
        let mut app = state();
        assert_eq!(app.quit_while_busy(Instant::now()), Effect::None);
        assert_eq!(app.dispatch(Action::CtrlC), Effect::Quit);

        // An expired arm is a fresh prompt, and the prompt clears with it.
        let mut app = state();
        assert_eq!(app.quit_while_busy(t0), Effect::None);
        let until = app.ctrl_c_armed_until.expect("armed");
        assert!(app.expire_ctrl_c_prompt(until));
        assert!(app.status.is_empty());
        assert_eq!(app.quit_while_busy(until), Effect::None);
    }

    #[test]
    fn busy_drop_names_the_running_op() {
        let mut app = state();
        app.note_busy_drop(Some("revert"));
        assert_eq!(app.status, "busy: revert running");
        assert_eq!(app.status.kind(), StatusKind::Warn);
        app.note_busy_drop(None);
        assert_eq!(app.status, BUSY_WRITE_RUNNING);
    }

    #[test]
    fn info_status_expires_but_not_under_an_overlay() {
        let mut app = state();
        let t0 = Instant::now();
        let ttl = Duration::from_millis(crate::tui::status::STATUS_TTL_MS);
        assert_eq!(
            app.dispatch(Action::ToggleShowHidden),
            Effect::LoadRightPane
        );
        assert_eq!(app.status.kind(), StatusKind::Info);
        assert_eq!(
            app.status_expiry_ms(t0),
            Some(crate::tui::status::STATUS_TTL_MS)
        );
        assert!(!app.expire_status(t0 + ttl - Duration::from_millis(1)));
        assert!(app.expire_status(t0 + ttl));
        assert!(app.status.is_empty());

        // Search typing echoes into `status`: no expiry while it is open.
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        app.status = "/".into();
        assert_eq!(app.status_expiry_ms(t0), None);
        assert!(!app.expire_status(t0 + ttl * 10));
        assert_eq!(app.status, "/");

        // Warnings stay until replaced.
        let mut app = state();
        app.status = StatusMessage::warn("nothing to push");
        assert_eq!(app.status_expiry_ms(t0), None);
        assert!(!app.expire_status(t0 + ttl * 10));
    }

    #[test]
    fn help_holds_the_status_expiry_clock() {
        let mut app = state();
        let t0 = Instant::now();
        let ttl = Duration::from_millis(crate::tui::status::STATUS_TTL_MS);
        app.status = StatusMessage::ok("Fetched 1 repo");
        assert_eq!(
            app.status_expiry_ms(t0),
            Some(crate::tui::status::STATUS_TTL_MS)
        );
        app.help_open = true;
        let late = t0 + ttl * 10;
        assert_eq!(app.status_expiry_ms(late), None);
        assert!(!app.expire_status(late));
        assert_eq!(app.status, "Fetched 1 repo");
        app.help_open = false;
        assert_eq!(
            app.status_expiry_ms(late),
            Some(crate::tui::status::STATUS_TTL_MS),
            "the clock restarts once help closes"
        );
        assert!(app.expire_status(late + ttl));
    }

    /// Draw `app` once into a 100x30 test terminal.
    fn draw_once(app: &mut AppState) {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
        terminal
            .draw(|frame| crate::tui::render::draw(frame, app))
            .unwrap();
    }

    fn toast_texts(app: &AppState) -> Vec<String> {
        app.toasts
            .visible(Instant::now())
            .map(|(toast, _)| toast.text.clone())
            .collect()
    }

    #[test]
    fn ok_status_moves_to_a_toast_on_draw() {
        let mut app = state();
        app.status = StatusMessage::ok("Fetched 3 repos");
        assert!(app.toasts.is_empty());
        draw_once(&mut app);
        assert!(app.status.is_empty(), "the ack left the status slot");
        assert_eq!(toast_texts(&app), ["Fetched 3 repos"]);
        // The next draw does not push it twice.
        draw_once(&mut app);
        assert_eq!(toast_texts(&app), ["Fetched 3 repos"]);
    }

    #[test]
    fn error_warn_info_progress_stay_in_status_line() {
        for status in [
            StatusMessage::error("fetch failed"),
            StatusMessage::warn("nothing to push"),
            StatusMessage::info("showing hidden repos"),
            StatusMessage::progress("Fetching 1/3…"),
        ] {
            let mut app = state();
            let kind = status.kind();
            let text = status.to_string();
            app.status = status;
            draw_once(&mut app);
            assert_eq!(app.status, text.as_str(), "{kind:?} stays in the slot");
            assert_eq!(app.status.kind(), kind);
            assert!(app.toasts.is_empty(), "{kind:?} makes no toast");
        }
    }

    #[test]
    fn overlay_status_row_keeps_its_ok_text() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        assert!(status_uses_status_text(&app));
        app.status = StatusMessage::ok("Checked out main");
        draw_once(&mut app);
        assert_eq!(app.status, "Checked out main", "the overlay row keeps it");
        assert!(app.toasts.is_empty());
    }

    #[test]
    fn n_without_an_armed_search_says_so() {
        let mut app = state();
        let cursor = app.cursor;
        assert_eq!(app.dispatch(Action::SearchNext), Effect::None);
        assert_eq!(app.status, NO_SEARCH_ARMED);
        assert_eq!(app.status.kind(), StatusKind::Warn);
        app.status.clear();
        assert_eq!(app.dispatch(Action::SearchPrev), Effect::None);
        assert_eq!(app.status, NO_SEARCH_ARMED);
        assert_eq!(app.cursor, cursor);
    }

    #[test]
    fn z_on_a_file_diff_says_tree_rows_and_arms_nothing() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        assert_eq!(app.dispatch(Action::FoldToggle), Effect::None);
        assert_eq!(app.status, Z_FOLDS_TREE_ROWS);
        assert!(app.z_pending_at.is_none(), "no fake z… pending");
        assert!(!matches!(app.input_mode(), InputMode::ZPending { .. }));
    }

    #[test]
    fn space_on_a_non_file_row_says_focus_a_file() {
        let mut app = state();
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert_eq!(app.status, FOCUS_A_FILE_TO_MARK_REVIEWED);
        assert_eq!(app.status.kind(), StatusKind::Warn);
    }

    fn focus_file(app: &mut AppState, needle: &str) {
        let idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File && r.label.contains(needle))
            .expect("file row");
        app.cursor = idx;
    }

    #[test]
    fn search_n_n_unfolds_parents() {
        let mut app = state();
        app.folds.insert("repo:app".into());
        app.rebuild_rows();
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        app.dispatch(Action::SearchChar('R'));
        app.dispatch(Action::SearchChar('E'));
        app.dispatch(Action::SearchChar('A'));
        app.dispatch(Action::SearchChar('D'));
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_active);
        assert!(!app.search_mode);
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
        assert_eq!(
            app.focused_row().map(|r| r.id.as_str()),
            Some("file:app:README.md")
        );
        let first = app.focused_row().map(|r| r.id.clone());
        app.dispatch(Action::SearchNext);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.focused_row().map(|r| r.id.clone()), first);
    }

    #[test]
    fn hidden_not_in_search_or_stage() {
        let mut app = state();
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        for c in "notes".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert!(app.status.contains("no match"));
        app.dispatch(Action::SearchCancel);
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
        focus_repo(&mut app, "app");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                assert_eq!(paths, vec!["README.md"]);
            }
            other => panic!("{other:?}"),
        }
        app.dispatch(Action::ToggleShowHidden);
        assert!(app.rows.iter().any(|r| r.label.contains("notes")));
        focus_file(&mut app, "README.md");
        // notes file is also README? notes has README.md in helper
        let notes = app.rows.iter().position(|r| r.id == "file:notes:README.md");
        assert!(notes.is_some());
        app.cursor = notes.unwrap();
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, .. } => assert_eq!(repo, "notes"),
            other => panic!("expected stage notes, got {other:?}"),
        }
    }

    #[test]
    fn stage_unstage_revert_on_file_only() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                assert_eq!(paths, vec!["README.md"]);
            }
            other => panic!("{other:?}"),
        }
        // synthetic staged-only
        if let Some(row) = app.rows.iter_mut().find(|r| r.kind == NodeKind::File) {
            if let Some(file) = row.file.as_mut() {
                file.staged_status = Some("M".into());
                file.unstaged_status = None;
            }
        }
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Unstage) {
            Effect::Unstage { paths, .. } => assert_eq!(paths, vec!["README.md"]),
            other => panic!("{other:?}"),
        }
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.status.contains("staged only"));
    }

    #[test]
    fn revert_confirm_cancels_and_applies() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("cancelled"));
        focus_file(&mut app, "README.md");
        app.dispatch(Action::Revert);
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                repo,
                tracked,
                untracked,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(tracked, vec!["README.md"]);
                assert!(untracked.is_empty());
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
    }

    #[test]
    fn resize_does_not_dismiss_revert_confirm() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(
            app.dispatch(Action::Resize {
                cols: 100,
                rows: 24
            }),
            Effect::None
        );
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn watch_zero_is_disable_and_refresh_keeps_focus() {
        assert_eq!(watch_interval_ms(Some("0"), None), 0);
        assert!(watch_interval_ms(Some("2000"), None) >= 500);
        let mut app = state();
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        let folds = app.folds.clone();
        app.graph_scroll = 3;
        app.diff_scroll = 4;
        let snapshot = app.snapshot.clone();
        let changed = app.apply_watch_snapshot(snapshot);
        assert!(changed.is_empty());
        assert_eq!(app.focused_row().unwrap().id, id);
        assert_eq!(app.folds, folds);
        assert_eq!(app.graph_scroll, 3);
        assert_eq!(app.diff_scroll, 4);
        assert_eq!(app.dispatch(Action::WatchTick), Effect::WatchRefresh);
    }

    #[test]
    fn watch_refresh_keeps_tree_focus_when_new_file_appears() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        let mut snapshot = app.snapshot.clone();
        snapshot.repos[0].changes.insert(
            0,
            FileChange {
                path: "aaa-new.rs".into(),
                staged_status: None,
                unstaged_status: Some("A".into()),
                untracked: true,
                old_path: None,
            },
        );
        let _ = app.apply_watch_snapshot(snapshot);
        assert_eq!(app.focused_row().unwrap().id, id);
        assert!(
            app.rows.iter().any(|row| row.label.contains("aaa-new.rs")),
            "new file must paint so this is a real insert-above, not a no-op"
        );
        assert!(!app.focused_row().unwrap().id.contains("aaa-new"));
    }

    #[test]
    fn watch_refresh_walks_to_repo_when_focused_file_is_gone() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let mut snapshot = app.snapshot.clone();
        snapshot.repos[0].changes = vec![FileChange {
            path: "src/lib.rs".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        snapshot.repos[0].has_unstaged = true;
        snapshot.repos[0].has_untracked = false;
        let _ = app.apply_watch_snapshot(snapshot);
        let focused = app.focused_row().expect("tree row");
        assert_eq!(
            focused.id, "repo:app",
            "gone file should land on the repo ancestor, not jump to row 0: {focused:?}"
        );
        assert!(
            app.cursor > 0,
            "workspace root is row 0; ancestor restore must not force the top"
        );
    }

    fn mini_graph(ids: &[&str]) -> GraphModel {
        GraphModel {
            uncommitted: Some(false),
            commits: ids
                .iter()
                .map(|id| Commit {
                    id: (*id).into(),
                    subject: format!("s-{id}"),
                    ..Commit::default()
                })
                .collect(),
            window: ids.len(),
            skip: 0,
            limit: 300,
            ..GraphModel::default()
        }
    }

    #[test]
    fn watch_add_update_remove_flashes_same_ids_only() {
        let mut app = state();
        app.flashes.clear();
        let file_id = app
            .rows
            .iter()
            .find(|row| row.label.contains("README.md"))
            .map(|row| row.id.clone())
            .expect("readme row");
        let mut snapshot = app.snapshot.clone();
        snapshot.repos[0].changes.push(FileChange {
            path: "new.rs".into(),
            staged_status: None,
            unstaged_status: Some("A".into()),
            untracked: true,
            old_path: None,
        });
        let added = app.apply_watch_snapshot(snapshot.clone());
        assert!(
            added.iter().any(|id| id.contains("new.rs")),
            "added file should flash: {added:?}"
        );
        assert!(
            !added.contains(&file_id),
            "unchanged readme should not flash: {added:?}"
        );
        let new_id = added
            .iter()
            .find(|id| id.contains("new.rs"))
            .cloned()
            .expect("new.rs id");
        assert_eq!(
            app.flashes.get(&new_id).map(|stamp| stamp.kind),
            Some(FlashKind::Add)
        );

        snapshot.repos[0]
            .changes
            .iter_mut()
            .find(|change| change.path == "README.md")
            .expect("readme change")
            .unstaged_status = Some("A".into());
        let updated = app.apply_watch_snapshot(snapshot.clone());
        assert!(
            updated.iter().any(|id| id.contains("README.md")),
            "updated readme should flash: {updated:?}"
        );
        assert_eq!(
            app.flashes.get(&file_id).map(|stamp| stamp.kind),
            Some(FlashKind::Update)
        );

        snapshot.repos[0].changes.retain(|c| c.path != "README.md");
        let removed = app.apply_watch_snapshot(snapshot);
        assert!(
            removed.iter().any(|id| id.contains("README.md")),
            "removed readme should flash: {removed:?}"
        );
        assert_eq!(
            app.flashes.get(&file_id).map(|stamp| stamp.kind),
            Some(FlashKind::Remove)
        );
        assert!(
            app.tree_ghosts.iter().any(|g| g.id.contains("README.md")),
            "removed readme should stay as a ghost"
        );

        app.stamp_checkout_flashes(&["app".into()]);
        assert_eq!(
            app.flashes.get("repo:app").map(|stamp| stamp.kind),
            Some(FlashKind::Update)
        );
        assert_eq!(
            app.flashes.get("checkout:app").map(|stamp| stamp.kind),
            Some(FlashKind::Update)
        );
    }

    #[test]
    fn graph_repo_switch_seeds_without_flashing() {
        let mut app = state();
        app.flashes.clear();
        app.set_graph(mini_graph(&["aaa"]), "app".into(), "aaa".into());
        assert!(
            app.flashes.is_empty(),
            "first graph paint seeds: {:?}",
            app.flashes.keys().collect::<Vec<_>>()
        );
        app.set_graph(mini_graph(&["aaa"]), "lib".into(), "aaa".into());
        assert!(
            app.flashes.is_empty(),
            "repo switch must not flash: {:?}",
            app.flashes.keys().collect::<Vec<_>>()
        );
        let mut updated = mini_graph(&["aaa"]);
        updated.commits[0].subject = "changed".into();
        app.set_graph(updated, "lib".into(), "aaa".into());
        assert!(
            app.flashes.keys().any(|id| id.contains("commit:aaa")),
            "same-repo subject change should flash: {:?}",
            app.flashes.keys().collect::<Vec<_>>()
        );
    }

    fn graph_focus_id(app: &AppState) -> String {
        graph_row_id(&app.focused_graph_row().expect("graph row"))
    }

    fn graph_row_index(app: &AppState, id: &str) -> usize {
        app.graph
            .as_ref()
            .expect("graph")
            .visible_rows()
            .iter()
            .position(|row| graph_row_id(row) == id)
            .unwrap_or_else(|| panic!("missing graph row {id}"))
    }

    #[test]
    fn set_graph_same_repo_new_head_keeps_focused_commit() {
        let mut app = state();
        app.layout.tree_height = 24;
        app.set_graph(mini_graph(&["aaa", "bbb"]), "app".into(), "aaa".into());
        let bbb = graph_row_index(&app, "commit:bbb");
        assert!(bbb > 0, "bbb must not be the top row");
        app.graph_cursor = bbb;
        app.set_graph(
            mini_graph(&["ccc", "aaa", "bbb"]),
            "app".into(),
            "ccc".into(),
        );
        assert_eq!(graph_focus_id(&app), "commit:bbb");
        assert_eq!(app.graph_cursor, graph_row_index(&app, "commit:bbb"));
        assert_ne!(
            app.graph_cursor, 0,
            "new HEAD must not jump focus to the top row"
        );
        assert_ne!(graph_focus_id(&app), "commit:ccc");
    }

    #[test]
    fn set_graph_same_repo_missing_row_clamps_to_neighbor() {
        let mut app = state();
        app.layout.tree_height = 24;
        app.set_graph(
            mini_graph(&["aaa", "bbb", "ccc"]),
            "app".into(),
            "aaa".into(),
        );
        let bbb = graph_row_index(&app, "commit:bbb");
        app.graph_cursor = bbb;
        app.set_graph(mini_graph(&["aaa", "ccc"]), "app".into(), "aaa".into());
        let n = app.graph.as_ref().unwrap().visible_rows().len();
        assert_eq!(
            app.graph_cursor,
            bbb.min(n - 1),
            "gone row must clamp to min(old, n-1), not jump to 0"
        );
        assert_ne!(app.graph_cursor, 0);
        assert_eq!(graph_focus_id(&app), "commit:ccc");
    }

    #[test]
    fn set_graph_repo_switch_resets_to_top() {
        let mut app = state();
        app.layout.tree_height = 24;
        app.set_graph(mini_graph(&["aaa", "bbb"]), "app".into(), "aaa".into());
        app.graph_cursor = graph_row_index(&app, "commit:bbb");
        app.graph_scroll = 2;
        app.set_graph(mini_graph(&["aaa", "bbb"]), "lib".into(), "aaa".into());
        assert_eq!(app.graph_cursor, 0);
        assert_eq!(app.graph_scroll, 0);
    }

    #[test]
    fn apply_watch_snapshot_does_not_reset_graph_cursor() {
        let mut app = state();
        app.layout.tree_height = 24;
        app.set_graph(mini_graph(&["aaa", "bbb"]), "app".into(), "aaa".into());
        let bbb = graph_row_index(&app, "commit:bbb");
        app.graph_cursor = bbb;
        let mut snapshot = app.snapshot.clone();
        snapshot.repos[0].changes.insert(
            0,
            FileChange {
                path: "aaa-new.rs".into(),
                staged_status: None,
                unstaged_status: Some("A".into()),
                untracked: true,
                old_path: None,
            },
        );
        let _ = app.apply_watch_snapshot(snapshot);
        assert_eq!(graph_focus_id(&app), "commit:bbb");
        assert_eq!(app.graph_cursor, bbb);
        assert_ne!(app.graph_cursor, 0);
    }

    #[test]
    fn edit_keeps_fold_focus_scroll() {
        let mut app = state();
        app.folds.insert("group:no-updates".into());
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        let folds = app.folds.clone();
        app.diff_scroll = 7;
        match app.dispatch(Action::Edit) {
            Effect::EditFile { repo, path, .. } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "README.md");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.focused_row().unwrap().id, id);
        assert_eq!(app.folds, folds);
        assert_eq!(app.diff_scroll, 7);
        assert_eq!(app.status, "opening README.md…");
        assert_eq!(app.status.kind(), StatusKind::Progress);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Edit), Effect::None);
    }

    #[test]
    fn external_diff_same_focus_as_edit() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::ExternalDiff) {
            Effect::ExternalDiff { repo, path, kind } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "README.md");
                assert_eq!(kind, ExternalDiffKind::Worktree);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            app.dispatch(Action::Edit),
            Effect::EditFile {
                repo: "app".into(),
                path: "README.md".into(),
                line: None,
            }
        );
        assert_eq!(app.status, "opening README.md…");
        assert_eq!(app.status.kind(), StatusKind::Progress);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::ExternalDiff), Effect::None);
        assert_eq!(app.status, "focus a file to diff");
    }

    fn compare_source() -> CommitFileSource {
        CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: "bbb".into(),
            merge_base: "aaa".into(),
            head: "ccc".into(),
        }
    }

    fn compare_range_load(paths: &[&str]) -> crate::tui::app::CompareRangeLoad {
        crate::tui::app::CompareRangeLoad {
            source: compare_source(),
            files: paths
                .iter()
                .map(|path| CommitFile {
                    status: "M".into(),
                    path: (*path).into(),
                    old_path: None,
                    stat: None,
                })
                .collect(),
            patch_keys: paths
                .iter()
                .map(|path| ((*path).to_string(), format!("key:{path}")))
                .collect(),
            head: Some("ccc".into()),
            base_tip: "bbb".into(),
        }
    }

    #[test]
    fn compare_tab_edit_does_not_open_workspace_dirty_file() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        assert_eq!(app.dispatch(Action::Edit), Effect::None);
        assert_eq!(app.status, "focus a file to edit");
        assert_eq!(app.dispatch(Action::ExternalDiff), Effect::None);
        assert_eq!(app.status, "focus a file to diff");
        assert_eq!(
            palette_reason(&app, "Open in editor").as_deref(),
            Some("focus a file to edit")
        );
    }

    #[test]
    fn compare_tab_edit_skips_dir_row() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        {
            let tab = app.tabs.active_compare_mut().unwrap();
            tab.source = Some(compare_source());
            tab.files = vec![CommitFile {
                status: "A".into(),
                path: "src/view.rs".into(),
                old_path: None,
                stat: None,
            }];
            tab.file_cursor = 0;
        }
        let row = app.focused_commit_file_row().expect("compare row");
        assert!(!row.is_file(), "cursor 0 is the dir: {row:?}");
        assert_eq!(app.dispatch(Action::Edit), Effect::None);
        assert_eq!(app.status, "focus a file to edit");
        assert_eq!(app.dispatch(Action::ExternalDiff), Effect::None);
    }

    /// Files for the folder-summary tests: `README.md`, then `src/` with
    /// `a.rs` (+3 −1), `b.rs` (+2 −0), and `deep/c.rs` (binary, no counts).
    ///
    /// Tree rows: `src`, `src/deep`, `src/deep/c.rs`, `src/a.rs`,
    /// `src/b.rs`, `README.md`. Flat rows start at `README.md`.
    fn summary_files() -> Vec<CommitFile> {
        [
            ("README.md", Some((1, 1))),
            ("src/a.rs", Some((3, 1))),
            ("src/b.rs", Some((2, 0))),
            ("src/deep/c.rs", None),
        ]
        .into_iter()
        .map(|(path, stat)| CommitFile {
            status: "M".into(),
            path: path.into(),
            old_path: None,
            stat: stat.map(|(added, deleted)| crate::git::LineStat { added, deleted }),
        })
        .collect()
    }

    fn summary_row(app: &AppState, path: &str) -> usize {
        app.commit_file_rows()
            .iter()
            .position(|row| row.path == path)
            .unwrap_or_else(|| panic!("row {path}"))
    }

    /// Move the focused file list onto the row for `path`.
    fn summary_move_to(app: &mut AppState, path: &str) -> Effect {
        let delta = summary_row(app, path) as i32 - app.commit_files_cursor() as i32;
        app.dispatch(Action::Move(delta))
    }

    fn summary_paths(summary: &FolderSummary) -> Vec<&str> {
        summary
            .files
            .iter()
            .map(|file| file.path.as_str())
            .collect()
    }

    /// Depth-2 commit drill on [`summary_files`] with the `src/a.rs` diff
    /// open and the file list focused on it.
    fn summary_drill_app() -> AppState {
        let mut app = state();
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            summary_files(),
            0,
            "src/a.rs".into(),
            DiffContent::from_unified("@@ -1 +1,2 @@\n keep\n+line\n"),
        );
        let idx = summary_row(&app, "src/a.rs");
        app.set_commit_file_cursor(idx);
        app.focus = FocusPane::Left;
        app
    }

    /// Compare tab `app ↔ main` on [`summary_files`] with the `src/a.rs`
    /// diff loaded and the file list focused on it. The snapshot's `app`
    /// HEAD is the compare head and its worktree is clean, so `x` may
    /// revert `src/a.rs`.
    fn summary_compare_app() -> AppState {
        let mut app = state();
        for row in app
            .snapshot
            .repos
            .iter_mut()
            .filter(|row| row.repo == "app")
        {
            row.head = "ccc".into();
            row.changes.clear();
        }
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let mut load = compare_range_load(&[]);
        load.files = summary_files();
        let _ = app.apply_compare_range(tab_id, gen, Ok(load));
        let _ = summary_move_to(&mut app, "src/a.rs");
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "src/a.rs",
            Ok(DiffContent::from_compare_lines(
                [
                    "diff --git a/src/a.rs b/src/a.rs",
                    "--- a/src/a.rs",
                    "+++ b/src/a.rs",
                    "@@ -1,2 +1,2 @@",
                    " keep",
                    "-old",
                    "+new",
                ]
                .iter()
                .map(|line| line.to_string())
                .collect(),
            )),
        );
        app
    }

    #[test]
    fn drill_folder_focus_shows_summary_and_keeps_the_diff() {
        let mut app = summary_drill_app();
        assert_eq!(app.folder_summary(), None, "file row shows its diff");
        assert!(!app.current_diff_rows().is_empty());

        assert_eq!(summary_move_to(&mut app, "src"), Effect::None);
        let summary = app.folder_summary().expect("folder summary");
        assert_eq!(summary.dir, "src");
        assert_eq!(
            summary_paths(&summary),
            ["src/a.rs", "src/b.rs", "src/deep/c.rs"]
        );
        assert_eq!((summary.added, summary.deleted), (5, 1));
        assert!(app.current_diff_rows().is_empty(), "no hidden diff rows");
        assert!(
            matches!(&app.drill, DrillView::Diff { path, .. } if path == "src/a.rs"),
            "the diff stays loaded behind the summary"
        );

        // Same file: the kept diff repaints with no reload.
        assert_eq!(summary_move_to(&mut app, "src/a.rs"), Effect::None);
        assert_eq!(app.folder_summary(), None);
        assert!(!app.current_diff_rows().is_empty());

        // Folder again: the summary, not the file diff.
        assert_eq!(summary_move_to(&mut app, "src/deep"), Effect::None);
        let summary = app.folder_summary().expect("nested folder summary");
        assert_eq!(summary_paths(&summary), ["src/deep/c.rs"]);
        assert_eq!((summary.added, summary.deleted), (0, 0));
        assert!(app.current_diff_rows().is_empty());

        // Another file loads its diff.
        assert_eq!(summary_move_to(&mut app, "src/b.rs"), Effect::LoadRightPane);
    }

    #[test]
    fn compare_folder_focus_shows_summary_and_keeps_the_diff() {
        let mut app = summary_compare_app();
        assert_eq!(app.folder_summary(), None);
        assert!(!app.current_diff_rows().is_empty());

        assert_eq!(summary_move_to(&mut app, "src"), Effect::None);
        let summary = app.folder_summary().expect("folder summary");
        assert_eq!(
            summary_paths(&summary),
            ["src/a.rs", "src/b.rs", "src/deep/c.rs"]
        );
        assert!(app.current_diff_rows().is_empty());
        assert_eq!(
            app.tabs.active_compare().unwrap().path.as_deref(),
            Some("src/a.rs"),
            "the diff stays loaded behind the summary"
        );

        assert_eq!(summary_move_to(&mut app, "src/a.rs"), Effect::None);
        assert_eq!(app.folder_summary(), None);
        assert!(!app.current_diff_rows().is_empty());

        assert_eq!(summary_move_to(&mut app, "src"), Effect::None);
        assert!(app.folder_summary().is_some(), "folder again: the summary");
        assert!(app.current_diff_rows().is_empty());

        match summary_move_to(&mut app, "src/b.rs") {
            Effect::LoadCompareDiff { path, .. } => assert_eq!(path, "src/b.rs"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn folded_folder_summary_lists_every_descendant() {
        for mut app in [summary_drill_app(), summary_compare_app()] {
            let _ = summary_move_to(&mut app, "src");
            assert_eq!(app.dispatch(Action::FoldClose), Effect::None);
            assert!(!app
                .commit_file_rows()
                .iter()
                .any(|row| row.path == "src/deep/c.rs"));
            let summary = app.folder_summary().expect("folded folder summary");
            assert_eq!(
                summary_paths(&summary),
                ["src/a.rs", "src/b.rs", "src/deep/c.rs"]
            );
        }
    }

    #[test]
    fn folder_summary_needs_a_diff_beside_the_list() {
        let mut app = summary_drill_app();
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            summary_files(),
        );
        app.set_commit_file_cursor(0);
        assert!(!app.focused_commit_file_row().unwrap().is_file());
        assert_eq!(app.folder_summary(), None, "depth 1 has no diff pane");
    }

    /// Dispatch `action` and read the palette row `title`; both must refuse
    /// with `reason` and run nothing.
    fn assert_summary_refuses(app: &mut AppState, action: Action, title: &str, reason: &str) {
        assert_eq!(
            palette_reason(app, title).as_deref(),
            Some(reason),
            "palette {title}"
        );
        app.status.clear();
        assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
        assert_eq!(app.status, reason, "{action:?}");
        assert_eq!(app.confirm, None, "{action:?}");
    }

    #[test]
    fn drill_folder_summary_refuses_file_diff_keys() {
        let mut app = summary_drill_app();
        let _ = summary_move_to(&mut app, "src");
        for focus in [FocusPane::Left, FocusPane::Right] {
            app.focus = focus;
            assert_summary_refuses(
                &mut app,
                Action::Edit,
                "Open in editor",
                "focus a file to edit",
            );
            assert_summary_refuses(
                &mut app,
                Action::ExternalDiff,
                "Open in diff tool",
                "focus a file to diff",
            );
            assert_summary_refuses(
                &mut app,
                Action::ToggleFullContext,
                "Full-file context",
                FOCUS_A_FILE_DIFF,
            );
            assert_summary_refuses(
                &mut app,
                Action::DiffVisualStart,
                "Highlight diff lines",
                FOCUS_A_FILE_DIFF,
            );
            assert!(app.full_context.is_empty(), "{focus:?}");
            assert_eq!(app.diff_visual_anchor, None, "{focus:?}");
        }

        // The right pane's own keys refuse while it shows the summary.
        app.focus = FocusPane::Right;
        assert_summary_refuses(&mut app, Action::CommentStart, "Comment", FOCUS_A_FILE_DIFF);
        assert_eq!(app.comment, None);
        assert_summary_refuses(
            &mut app,
            Action::CopyEntityReference,
            "Copy entity reference",
            FOCUS_A_FILE_DIFF,
        );
        assert_summary_refuses(
            &mut app,
            Action::ExportComments,
            "Copy comments",
            FOCUS_A_FILE_DIFF,
        );

        // A highlight left on the hidden diff never writes it.
        app.diff_visual_anchor = Some(0);
        assert_summary_refuses(
            &mut app,
            Action::Stage,
            "Stage highlighted lines",
            FOCUS_A_FILE_DIFF,
        );
        assert_summary_refuses(
            &mut app,
            Action::Revert,
            "Revert highlighted lines",
            FOCUS_A_FILE_DIFF,
        );

        // Back on a file the keys run again.
        app.diff_visual_anchor = None;
        app.focus = FocusPane::Left;
        let _ = summary_move_to(&mut app, "src/a.rs");
        assert_eq!(palette_reason(&app, "Open in editor"), None);
        assert!(matches!(
            app.dispatch(Action::Edit),
            Effect::EditFile { ref path, .. } if path == "src/a.rs"
        ));
    }

    #[test]
    fn compare_folder_summary_refuses_file_diff_keys() {
        let mut app = summary_compare_app();
        app.focus = FocusPane::Right;
        assert!(app.compare_revert_target().is_ok(), "src/a.rs may revert");
        assert_eq!(palette_reason(&app, "Mark reviewed"), None);
        app.focus = FocusPane::Left;
        let _ = summary_move_to(&mut app, "src");

        for focus in [FocusPane::Left, FocusPane::Right] {
            app.focus = focus;
            assert_summary_refuses(
                &mut app,
                Action::Edit,
                "Open in editor",
                "focus a file to edit",
            );
            assert_summary_refuses(
                &mut app,
                Action::ExternalDiff,
                "Open in diff tool",
                "focus a file to diff",
            );
            assert_summary_refuses(
                &mut app,
                Action::ToggleReviewed,
                "Mark reviewed",
                FOCUS_A_FILE_TO_MARK_REVIEWED,
            );
            assert_summary_refuses(
                &mut app,
                Action::ToggleFullContext,
                "Full-file context",
                FOCUS_A_FILE_DIFF,
            );
            assert_summary_refuses(
                &mut app,
                Action::DiffVisualStart,
                "Highlight diff lines",
                FOCUS_A_FILE_DIFF,
            );
            assert!(app.tabs.active_compare().unwrap().reviewed.is_empty());
        }

        // Right pane: `x` would revert the hidden diff's file.
        assert_summary_refuses(&mut app, Action::Revert, "Revert", FOCUS_A_FILE_DIFF);
        assert_summary_refuses(&mut app, Action::CommentStart, "Comment", FOCUS_A_FILE_DIFF);
        assert_summary_refuses(
            &mut app,
            Action::CopyEntityReference,
            "Copy entity reference",
            FOCUS_A_FILE_DIFF,
        );
        assert_summary_refuses(
            &mut app,
            Action::ExportComments,
            "Copy comments",
            FOCUS_A_FILE_DIFF,
        );
        // The file list keeps its own `x` copy.
        app.focus = FocusPane::Left;
        assert_summary_refuses(&mut app, Action::Revert, "Revert", COMPARE_FOCUS_A_FILE);

        // V-highlight revert of the hidden diff.
        app.focus = FocusPane::Right;
        app.diff_visual_anchor = Some(1);
        assert_eq!(
            app.highlight_revert_refusal().as_deref(),
            Some(FOCUS_A_FILE_DIFF)
        );
        assert_summary_refuses(
            &mut app,
            Action::Revert,
            "Revert highlighted lines",
            FOCUS_A_FILE_DIFF,
        );
    }

    #[test]
    fn compare_folder_summary_hides_file_diff_hints() {
        let mut app = summary_compare_app();
        app.focus = FocusPane::Right;
        let keys = |app: &AppState| -> Vec<String> {
            super::super::chrome::action_hint_segments(app)
                .into_iter()
                .map(|segment| segment.key)
                .collect()
        };
        assert!(keys(&app).contains(&"e".into()), "{:?}", keys(&app));
        assert!(keys(&app).contains(&"x".into()), "{:?}", keys(&app));
        app.focus = FocusPane::Left;
        let _ = summary_move_to(&mut app, "src");
        app.focus = FocusPane::Right;
        assert_eq!(keys(&app), Vec::<String>::new());
    }

    /// A search typed over a folder summary must not cache empty hits for
    /// the diff kept behind it, nor count that diff's hits.
    #[test]
    fn diff_search_over_a_folder_summary_does_not_reach_the_kept_diff() {
        let mut app = summary_compare_app();
        app.focus = FocusPane::Right;
        assert_eq!(app.diff_search_hits("new").len(), 1, "file diff hit");

        app.focus = FocusPane::Left;
        let _ = summary_move_to(&mut app, "src");
        app.focus = FocusPane::Right;
        assert!(app.diff_search_hits("new").is_empty(), "summary: no hits");
        app.dispatch(Action::SearchStart);
        for c in "new".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.search_match_position(), Some((None, 0)));

        // Same file again (no reload, same content): fresh hits, not the
        // empty list from the summary.
        app.focus = FocusPane::Left;
        assert_eq!(summary_move_to(&mut app, "src/a.rs"), Effect::None);
        app.focus = FocusPane::Right;
        assert_eq!(app.diff_search_hits("new").len(), 1);
        assert_eq!(app.search_match_position().map(|(_, n)| n), Some(1));
    }

    #[test]
    fn tree_to_flat_from_a_folder_loads_the_landed_file() {
        let mut app = summary_drill_app();
        let _ = summary_move_to(&mut app, "src");
        assert_eq!(app.dispatch(Action::ToggleTreeMode), Effect::LoadRightPane);
        assert_eq!(app.focused_commit_file_row().unwrap().path, "README.md");

        let mut app = summary_compare_app();
        let _ = summary_move_to(&mut app, "src");
        match app.dispatch(Action::ToggleTreeMode) {
            Effect::LoadCompareDiff { path, .. } => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn compare_range_reload_keeps_a_focused_folder() {
        let mut app = summary_compare_app();
        let _ = summary_move_to(&mut app, "src");
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        // A new folder sorts above `src`: the cursor follows the folder.
        let mut load = compare_range_load(&["app/new.rs"]);
        load.files.extend(summary_files());
        assert_eq!(app.apply_compare_range(tab_id, gen, Ok(load)), None);
        let row = app.focused_commit_file_row().expect("kept row");
        assert!(row.is_dir(), "{row:?}");
        assert_eq!(row.path, "src");
        assert_ne!(app.commit_files_cursor(), 0);
        assert_eq!(app.folder_summary().unwrap().dir, "src");
        let tab = app.tabs.active_compare().unwrap();
        assert_eq!(tab.path, None, "the old range's diff is dropped");
    }

    #[test]
    fn compare_range_reload_clamps_a_gone_folder() {
        let mut app = summary_compare_app();
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        // A gone folder clamps; landing on another folder shows its summary.
        let _ = summary_move_to(&mut app, "src/deep");
        let previous = app.commit_files_cursor();
        let load = compare_range_load(&["README.md", "src/a.rs", "lib/sub/x.rs", "lib/y.rs"]);
        assert_eq!(app.apply_compare_range(tab_id, gen, Ok(load)), None);
        assert_eq!(app.commit_files_cursor(), previous);
        let summary = app.folder_summary().expect("clamped onto a folder");
        assert_eq!(summary.dir, "lib/sub");
        assert_eq!(summary_paths(&summary), ["lib/sub/x.rs"]);
    }

    fn compare_tab_with_view_rs(app: &mut AppState) {
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        let tab = app.tabs.active_compare_mut().unwrap();
        tab.source = Some(compare_source());
        tab.files = vec![CommitFile {
            status: "A".into(),
            path: "src/view.rs".into(),
            old_path: None,
            stat: None,
        }];
        tab.patch_keys = [("src/view.rs".to_string(), "key:src/view.rs".to_string())].into();
    }

    #[test]
    fn compare_space_marks_focused_file_not_workspace() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let readme = app.focused_row().unwrap().id.clone();
        app.reviewed.insert(readme.clone());
        app.viewed_store
            .insert("app\0README.md".into(), "fingerprint".into());
        let workspace_store = app.viewed_store.clone();
        compare_tab_with_view_rs(&mut app);
        app.tabs.active_compare_mut().unwrap().file_cursor = 1;
        assert!(app.focused_commit_file_row().unwrap().is_file());
        assert_eq!(palette_reason(&app, "Mark reviewed"), None);

        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert_ne!(app.status, super::super::tabs::SWITCH_TO_WORKSPACE_TAB);
        assert!(app
            .tabs
            .active_compare()
            .unwrap()
            .is_reviewed("src/view.rs"));
        let row = app.focused_commit_file_row().unwrap();
        assert!(app.compare_file_reviewed(&row));
        assert!(app.reviewed.contains(&readme), "workspace mark untouched");
        assert_eq!(app.viewed_store, workspace_store, "no viewed store write");

        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(!app
            .tabs
            .active_compare()
            .unwrap()
            .is_reviewed("src/view.rs"));
    }

    #[test]
    fn compare_space_on_dir_row_names_file_focus() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        compare_tab_with_view_rs(&mut app);
        app.tabs.active_compare_mut().unwrap().file_cursor = 0;
        assert!(!app.focused_commit_file_row().unwrap().is_file());
        assert_eq!(
            palette_reason(&app, "Mark reviewed").as_deref(),
            Some(FOCUS_A_FILE_TO_MARK_REVIEWED)
        );
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert_eq!(app.status, FOCUS_A_FILE_TO_MARK_REVIEWED);
        assert!(app.tabs.active_compare().unwrap().reviewed.is_empty());
    }

    #[test]
    fn compare_space_on_right_diff_marks_open_file() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        compare_tab_with_view_rs(&mut app);
        {
            // The open diff is the focused file row (row 0 is `src`, whose
            // summary would cover the diff).
            let tab = app.tabs.active_compare_mut().unwrap();
            tab.path = Some("src/view.rs".into());
            tab.file_cursor = 1;
        }
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(app
            .tabs
            .active_compare()
            .unwrap()
            .is_reviewed("src/view.rs"));
    }

    #[test]
    fn apply_compare_range_selects_first_file_not_dir() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let follow = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["src/view.rs", "README.md"])),
        );
        let row = app.focused_commit_file_row().expect("compare row");
        assert!(
            row.is_file(),
            "new compare load must land on a file: {row:?}"
        );
        assert_eq!(row.path, "src/view.rs");
        match follow {
            Some(Effect::LoadCompareDiff { path, .. }) => assert_eq!(path, "src/view.rs"),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            app.dispatch(Action::Edit),
            Effect::EditFile {
                repo: "app".into(),
                path: "src/view.rs".into(),
                line: None,
            }
        );
    }

    #[test]
    fn compare_move_to_start_selects_first_file_not_dir() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["src/auth.ts", "src/session.ts"])),
        );
        let first = app.focused_commit_file_row().expect("first file");
        assert!(
            first.is_file(),
            "new compare load must land on a file: {first:?}"
        );
        assert_eq!(first.path, "src/auth.ts");
        let rows = app.commit_file_rows();
        assert!(
            !rows[0].is_file(),
            "row 0 is the parent directory: {:?}",
            rows[0]
        );
        assert_eq!(rows[0].path, "src");

        app.dispatch(Action::Move(1));
        let later = app.focused_commit_file_row().expect("later file");
        assert_eq!(later.path, "src/session.ts");

        app.dispatch(Action::MoveToStart);
        let row = app.focused_commit_file_row().expect("gg row");
        assert!(
            row.is_file(),
            "MoveToStart must land on a file, not a directory: {row:?}"
        );
        assert_eq!(row.path, "src/auth.ts");
        assert_ne!(
            app.tabs.active_compare().unwrap().file_cursor,
            0,
            "MoveToStart must not use raw row 0 when that row is a directory"
        );
    }

    #[test]
    fn apply_compare_range_keeps_path_when_new_file_prepended() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["src/a.rs", "src/b.rs"])),
        );
        app.tabs.active_compare_mut().unwrap().path = Some("src/b.rs".into());
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["src/z.rs", "src/a.rs", "src/b.rs"])),
        );
        let row = app.focused_commit_file_row().expect("kept row");
        assert_eq!(row.path, "src/b.rs");
        assert!(row.is_file());
        assert_ne!(
            app.tabs.active_compare().unwrap().file_cursor,
            0,
            "prepended file must not jump the compare cursor to the top"
        );
    }

    #[test]
    fn apply_compare_range_gone_path_clamps_to_neighbor() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["a.rs", "b.rs", "c.rs"])),
        );
        {
            let tab = app.tabs.active_compare_mut().unwrap();
            tab.path = Some("b.rs".into());
            tab.file_cursor = 1;
        }
        let _ = app.apply_compare_range(tab_id, gen, Ok(compare_range_load(&["a.rs", "c.rs"])));
        let row = app.focused_commit_file_row().expect("clamped row");
        assert_eq!(row.path, "c.rs");
        assert_ne!(
            app.tabs.active_compare().unwrap().file_cursor,
            0,
            "gone compare path must clamp, not jump to the first file"
        );
    }

    #[test]
    fn apply_compare_range_keeps_path_on_flattened_row() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["src/a.rs", "src/b.rs"])),
        );
        app.tabs.active_compare_mut().unwrap().path = Some("src/b.rs".into());
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["src/a.rs", "src/b.rs"])),
        );
        let row = app.focused_commit_file_row().expect("kept row");
        assert_eq!(row.path, "src/b.rs");
        assert!(row.is_file());
        let files_idx = app
            .tabs
            .active_compare()
            .unwrap()
            .files
            .iter()
            .position(|file| file.path == "src/b.rs")
            .unwrap();
        assert_ne!(
            app.tabs.active_compare().unwrap().file_cursor,
            files_idx,
            "cursor must be the flattened row, not the files index"
        );
    }

    #[test]
    fn compare_t_restores_file_cursor() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(tab_id, gen, Ok(compare_range_load(&["src/view.rs"])));
        assert!(app.focused_commit_file_row().unwrap().is_file());
        assert!(app.commit_tree_mode);
        assert_eq!(app.dispatch(Action::ToggleTreeMode), Effect::None);
        assert!(!app.commit_tree_mode);
        let row = app.focused_commit_file_row().expect("flat row");
        assert!(row.is_file());
        assert_eq!(row.path, "src/view.rs");
        app.dispatch(Action::ToggleTreeMode);
        let row = app.focused_commit_file_row().expect("tree row");
        assert!(row.is_file());
        assert_eq!(row.path, "src/view.rs");
    }

    #[test]
    fn new_compare_tab_opens_in_launch_commit_tree_default() {
        fn compare_ready(app: &mut AppState) {
            let repo = app
                .snapshot
                .repos
                .iter_mut()
                .find(|row| row.repo == "app")
                .expect("app");
            repo.head = "ccc".into();
            repo.default_tip_ref = Some("origin/main".into());
        }

        let mut app = state();
        compare_ready(&mut app);
        assert!(matches!(
            app.dispatch(Action::CompareVsDefault),
            Effect::LoadCompareRange { .. }
        ));
        assert!(app.is_compare_tab());
        assert!(app.commit_tree_mode, "no config: compare tab opens as tree");

        let mut app = state();
        compare_ready(&mut app);
        app.apply_view_defaults(&ViewDefaults {
            commit_tree: Some(false),
            ..ViewDefaults::default()
        });
        assert!(matches!(
            app.dispatch(Action::CompareVsDefault),
            Effect::LoadCompareRange { .. }
        ));
        assert!(app.is_compare_tab());
        assert!(
            !app.commit_tree_mode,
            "commitTree flat: compare tab is flat"
        );
        assert!(!app.tabs.active_compare().unwrap().tree_mode);

        app.dispatch(Action::ToggleTreeMode);
        assert!(app.commit_tree_mode, "t stays tab-local");
        app.open_compare_tab("app".into(), "develop".into(), COMPARE_HEAD_REF.into());
        assert_eq!(app.tabs.active, 2);
        assert!(
            !app.commit_tree_mode,
            "next tab opens in the launch default, not the toggled value"
        );
        app.dispatch(Action::JumpToTab(2));
        assert!(app.commit_tree_mode, "first tab keeps its toggle");
    }

    #[test]
    fn workspace_commit_file_folds_survive_compare_tab() {
        let mut app = state();
        let repo = app
            .snapshot
            .repos
            .iter_mut()
            .find(|row| row.repo == "app")
            .expect("app");
        repo.head = "ccc".into();
        repo.default_tip_ref = Some("origin/main".into());
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "src/a.rs".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "M".into(),
                    path: "src/b.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        assert!(app.drill.is_files());
        assert!(app.commit_tree_mode);
        let dir = app
            .commit_file_rows()
            .iter()
            .position(|row| row.id == "dir:src")
            .expect("dir");
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = dir;
        }
        app.dispatch(Action::FoldToggle);
        assert!(app.commit_file_folds.contains("dir:src"));
        assert!(app
            .commit_file_rows()
            .iter()
            .all(|row| row.id != "file:src/a.rs"));

        match app.dispatch(Action::CompareVsDefault) {
            Effect::LoadCompareRange { .. } => {}
            other => panic!("{other:?}"),
        }
        assert!(app.is_compare_tab());
        assert!(app.commit_file_folds.is_empty());
        assert!(app.commit_tree_mode);
        assert_eq!(app.dispatch(Action::ToggleTreeMode), Effect::None);
        assert!(!app.commit_tree_mode);

        app.dispatch(Action::JumpToTab(1));
        assert!(!app.is_compare_tab());
        assert!(app.drill.is_files());
        assert!(app.commit_tree_mode, "workspace trie restored");
        assert!(
            app.commit_file_folds.contains("dir:src"),
            "workspace folds restored"
        );
        assert!(
            app.commit_file_rows()
                .iter()
                .all(|row| row.id != "file:src/a.rs"),
            "folded dir still hides files"
        );
    }

    #[test]
    fn palette_compare_refusals_match_key_press() {
        use super::super::tabs::{
            CANNOT_STAGE_COMPARE, CANNOT_UNSTAGE_COMPARE, SWITCH_TO_WORKSPACE_TAB,
        };
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        for (title, reason) in [
            ("Stage", CANNOT_STAGE_COMPARE),
            ("Unstage", CANNOT_UNSTAGE_COMPARE),
            ("Revert", COMPARE_STILL_LOADING),
            ("Fetch remotes", SWITCH_TO_WORKSPACE_TAB),
            ("Stash menu", SWITCH_TO_WORKSPACE_TAB),
            ("Branch picker", SWITCH_TO_WORKSPACE_TAB),
            ("Graph focus branches", SWITCH_TO_WORKSPACE_TAB),
            ("Clear graph focus", SWITCH_TO_WORKSPACE_TAB),
        ] {
            assert_eq!(
                palette_reason(&app, title).as_deref(),
                Some(reason),
                "{title}"
            );
        }
        for (action, reason) in [
            (Action::Stage, CANNOT_STAGE_COMPARE),
            (Action::Unstage, CANNOT_UNSTAGE_COMPARE),
            (Action::Revert, COMPARE_STILL_LOADING),
            (Action::Fetch, SWITCH_TO_WORKSPACE_TAB),
            (Action::StashMenu, SWITCH_TO_WORKSPACE_TAB),
        ] {
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(app.status, reason, "{action:?}");
        }
        assert_eq!(
            palette_reason(&app, "Copy entity reference").as_deref(),
            Some("no copy target")
        );
        assert_eq!(
            palette_reason(&app, "Comment").as_deref(),
            Some("no comment target"),
            "Comment is open on compare; this tab has no loaded file yet"
        );
        assert_eq!(palette_reason(&app, "Copy comments"), None);

        // Blame actions run on a compare tab; with the file list focused
        // they refuse with the blame gate's copy, key and palette alike.
        app.focus = FocusPane::Left;
        for (title, action) in [
            ("Blame: open commit changes", Action::BlameCommitVsParent),
            (
                "Blame: open previous line change",
                Action::BlamePreviousChange,
            ),
            ("Blame: show commit in graph", Action::BlameRevealGraph),
        ] {
            assert_eq!(
                palette_reason(&app, title).as_deref(),
                Some(super::super::line_blame::FOCUS_A_DIFF_OR_FILE_LINE),
                "{title}"
            );
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(
                app.status,
                super::super::line_blame::FOCUS_A_DIFF_OR_FILE_LINE,
                "{action:?}"
            );
        }
    }

    #[test]
    fn compare_refusal_blocks_git_writes_and_allows_highlight() {
        use super::super::tabs::{
            CANNOT_STAGE_COMPARE, CANNOT_UNSTAGE_COMPARE, SWITCH_TO_WORKSPACE_TAB,
        };
        let switch = [
            Action::Fetch,
            Action::Pull,
            Action::Push,
            Action::DefaultBranch,
            Action::Branch,
            Action::BranchSubmit,
            Action::CreateBranchSubmit,
            Action::RemoveWorktree,
            Action::GraphCheckout,
            Action::GraphCreateBranch,
            Action::GraphMerge,
            Action::GraphStashApply,
            Action::GraphStashPop,
            Action::GraphStashDrop,
            Action::GraphFocusBranches,
            Action::GraphFocusClear,
            Action::GraphFocusSubmit,
            Action::StashMenu,
            Action::StashMenuEnter,
            // No compare-owned confirm is open.
            Action::ConfirmYes,
            Action::ConfirmYesClean,
        ];
        let allowed = [
            Action::CommentStart,
            Action::CommentSubmit,
            Action::CommentToggleResolved,
            Action::ExportComments,
            Action::DiffVisualStart,
            Action::DiffVisualCancel,
            Action::CopyEntityReference,
            Action::ToggleReviewed,
            Action::ToggleFullContext,
            Action::Edit,
            Action::ExternalDiff,
            Action::Move(1),
            Action::ConfirmNo,
            Action::CloseTab,
        ];

        let mut app = state();
        for action in switch.iter().chain(&allowed) {
            assert_eq!(app.compare_refusal(action), None, "workspace: {action:?}");
        }
        assert_eq!(app.compare_refusal(&Action::Stage), None);

        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        for action in &switch {
            assert_eq!(
                app.compare_refusal(action).as_deref(),
                Some(SWITCH_TO_WORKSPACE_TAB),
                "{action:?}"
            );
        }
        assert_eq!(
            app.compare_refusal(&Action::Stage).as_deref(),
            Some(CANNOT_STAGE_COMPARE)
        );
        assert_eq!(
            app.compare_refusal(&Action::Unstage).as_deref(),
            Some(CANNOT_UNSTAGE_COMPARE)
        );
        for action in &allowed {
            assert_eq!(app.compare_refusal(action), None, "compare: {action:?}");
        }
        // Revert answers from the head-is-worktree condition.
        assert_eq!(
            app.compare_refusal(&Action::Revert).as_deref(),
            Some(COMPARE_STILL_LOADING)
        );
        // A Workspace confirm never runs on a compare tab; a compare one does.
        app.confirm = Some(PendingConfirm::RevertRange {
            repo: "app".into(),
            path: "README.md".into(),
            patch: "p".into(),
        });
        for action in [Action::ConfirmYes, Action::ConfirmYesClean] {
            assert_eq!(
                app.compare_refusal(&action).as_deref(),
                Some(SWITCH_TO_WORKSPACE_TAB),
                "{action:?}"
            );
        }
        app.confirm = Some(PendingConfirm::CompareRevertFile {
            target: compare_revert_target_fixture("README.md", "M"),
        });
        for action in [Action::ConfirmYes, Action::ConfirmYesClean] {
            assert_eq!(app.compare_refusal(&action), None, "{action:?}");
        }
    }

    /// Compare tab on `README.md` with two added lines (new 2 and 3), diff focused.
    fn compare_tab_with_added_lines(app: &mut AppState) {
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(tab_id, gen, Ok(compare_range_load(&["README.md"])));
        let lines = ["@@ -1,2 +1,4 @@", " # app", "+new", "+more", " keep"];
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "README.md",
            Ok(DiffContent::from_compare_lines(
                lines.iter().map(|line| line.to_string()).collect(),
            )),
        );
        app.focus = FocusPane::Right;
        let (row, _) = numbered_diff_rows(app)
            .into_iter()
            .find(|(_, line)| *line == 2)
            .expect("new line 2");
        app.diff_cursor = row;
    }

    /// Compare diff of `regions.txt` with two separated hunks (ALPHA, OMEGA).
    /// `full` adds the unchanged lines between them, as Ctrl-O's `-U` does.
    fn compare_regions_content(full: bool) -> DiffContent {
        let mut body = String::from("@@ -1,4 +1,4 @@\n head-0\n-ALPHA-OLD\n+ALPHA-NEW\n tail-0\n");
        if full {
            for i in 0..20 {
                body.push_str(&format!(" mid-{i:02}\n"));
            }
            body.push_str(" head-1\n-OMEGA-OLD\n+OMEGA-NEW\n tail-1\n");
        } else {
            body.push_str("@@ -25,3 +25,3 @@\n head-1\n-OMEGA-OLD\n+OMEGA-NEW\n tail-1\n");
        }
        DiffContent::from_unified(body)
    }

    /// Open a compare tab on `regions.txt` with the right pane focused.
    fn compare_tab_on_regions(app: &mut AppState) -> (u64, u64) {
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(tab_id, gen, Ok(compare_range_load(&["regions.txt"])));
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "regions.txt",
            Ok(compare_regions_content(false)),
        );
        app.focus = FocusPane::Right;
        app.layout.diff_pane_height = 8;
        (tab_id, gen)
    }

    fn cursor_row_text(app: &AppState) -> String {
        row_search_text(&app.current_diff_rows()[app.diff_cursor])
    }

    #[test]
    fn compare_ctrl_o_reload_keeps_cursor_on_its_hunk() {
        let mut app = state();
        let (tab_id, gen) = compare_tab_on_regions(&mut app);
        let omega = app
            .current_diff_rows()
            .iter()
            .position(|row| row_search_text(row).contains("OMEGA-NEW"))
            .expect("second hunk change row");
        app.diff_cursor = omega;
        let hunk_only_cursor = app.diff_cursor;
        for full in [true, false] {
            match app.dispatch(Action::ToggleFullContext) {
                Effect::LoadCompareDiff {
                    tab_id: id, path, ..
                } => {
                    assert_eq!(id, tab_id);
                    assert_eq!(path, "regions.txt");
                }
                other => panic!("{other:?}"),
            }
            assert_eq!(app.full_context_active(), full);
            app.apply_compare_diff(
                tab_id,
                gen,
                &compare_source(),
                "regions.txt",
                Ok(compare_regions_content(full)),
            );
            assert!(
                cursor_row_text(&app).contains("OMEGA-NEW"),
                "full={full}: cursor left the hunk: {}",
                cursor_row_text(&app)
            );
            assert_eq!(app.pending_hunk_anchor, None, "anchor consumed");
            let start = app.diff_scroll as usize;
            assert!(
                (start..start + 8).contains(&app.diff_cursor),
                "full={full}: cursor {} out of view at scroll {start}",
                app.diff_cursor
            );
            if full {
                assert!(
                    app.diff_cursor > hunk_only_cursor,
                    "full file moves the row down"
                );
            } else {
                assert_eq!(app.diff_cursor, hunk_only_cursor);
            }
        }
    }

    #[test]
    fn compare_diff_reload_clamps_cursor_and_drops_stale_state() {
        let mut app = state();
        let (tab_id, gen) = compare_tab_on_regions(&mut app);
        // A shorter reload (watch refresh) clamps a cursor past the end.
        app.diff_cursor = 999;
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "regions.txt",
            Ok(compare_regions_content(false)),
        );
        assert_eq!(app.diff_cursor, app.current_diff_rows().len() - 1);
        // A highlight whose rows changed is dropped.
        app.diff_cursor = 1;
        app.dispatch(Action::DiffVisualStart);
        assert!(app.diff_visual_anchor.is_some());
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "regions.txt",
            Ok(compare_regions_content(true)),
        );
        assert_eq!(app.diff_visual_anchor, None, "stale highlight dropped");
        // A failed Ctrl-O load does not leak its anchor into the next diff.
        assert!(matches!(
            app.dispatch(Action::ToggleFullContext),
            Effect::LoadCompareDiff { .. }
        ));
        assert!(app.pending_hunk_anchor.is_some());
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "regions.txt",
            Err("boom".into()),
        );
        assert_eq!(app.pending_hunk_anchor, None);
    }

    #[test]
    fn compare_v_highlights_extends_and_cancels() {
        let mut app = state();
        compare_tab_with_added_lines(&mut app);
        let anchor = app.diff_cursor;
        assert_eq!(palette_reason(&app, "Highlight diff lines"), None);
        assert_eq!(app.dispatch(Action::DiffVisualStart), Effect::None);
        assert_eq!(app.diff_visual_anchor, Some(anchor));
        assert!(app.status.is_empty(), "{}", app.status);
        app.dispatch(Action::Move(1));
        assert!(app.diff_visual_contains(anchor));
        assert!(app.diff_visual_contains(anchor + 1));
        app.dispatch(Action::DiffVisualCancel);
        assert_eq!(app.diff_visual_anchor, None);
        assert!(!app.diff_visual_contains(anchor));
    }

    #[test]
    fn compare_highlight_apostrophe_copies_range() {
        let mut app = state();
        compare_tab_with_added_lines(&mut app);
        app.dispatch(Action::DiffVisualStart);
        app.dispatch(Action::Move(1));
        let effect = app.dispatch(Action::CopyEntityReference);
        let text = assert_copy_clipboard(effect, "diff", "path: README.md", true);
        assert!(text.contains("lines: 2-3"), "{text}");
        assert!(text.contains("source: compare main...ccc"), "{text}");
    }

    #[test]
    fn compare_highlight_palette_uses_compare_reasons() {
        use super::super::tabs::{CANNOT_STAGE_COMPARE, CANNOT_UNSTAGE_COMPARE};
        let mut app = state();
        compare_tab_with_added_lines(&mut app);
        app.dispatch(Action::DiffVisualStart);
        app.dispatch(Action::Move(1));
        assert_eq!(
            palette_reason(&app, "Stage highlighted lines").as_deref(),
            Some(CANNOT_STAGE_COMPARE)
        );
        assert_eq!(
            palette_reason(&app, "Unstage highlighted lines").as_deref(),
            Some(CANNOT_UNSTAGE_COMPARE)
        );
        // The fixture snapshot's HEAD is not the compare head.
        assert_eq!(
            palette_reason(&app, "Revert highlighted lines").as_deref(),
            Some(COMPARE_HEAD_MOVED)
        );
        assert_eq!(palette_reason(&app, "Exit highlight"), None);
        assert_eq!(palette_reason(&app, "Copy entity reference"), None);

        let anchor = app.diff_visual_anchor;
        for (action, reason) in [
            (Action::Stage, CANNOT_STAGE_COMPARE),
            (Action::Unstage, CANNOT_UNSTAGE_COMPARE),
        ] {
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(app.status, reason, "{action:?}");
            assert_eq!(app.diff_visual_anchor, anchor, "{action:?} keeps the range");
        }
    }

    fn compare_revert_target_fixture(path: &str, status: &str) -> CompareRevertTarget {
        CompareRevertTarget {
            repo: "app".into(),
            path: path.into(),
            old_path: None,
            status: status.into(),
            base_ref: "main".into(),
            merge_base: "aaa".into(),
            head: "ccc".into(),
        }
    }

    /// Committed compare diff of `regions.txt`: ALPHA (hunk 1) and OMEGA
    /// (hunk 2) change between the merge base and the head.
    const COMPARE_REGIONS_DIFF: &[&str] = &[
        "diff --git a/regions.txt b/regions.txt",
        "index 1111111..2222222 100644",
        "--- a/regions.txt",
        "+++ b/regions.txt",
        "@@ -1,4 +1,4 @@",
        " keep-a",
        " keep-b",
        " keep-c",
        "-ALPHA-OLD",
        "+ALPHA-NEW",
        "@@ -12,3 +12,3 @@",
        " pad-5",
        "-OMEGA-OLD",
        "+OMEGA-NEW",
    ];

    /// Compare tab `app ↔ main` (merge base `aaa`, head `ccc`) listing
    /// `regions.txt` (M), `summary.txt` (A) and `new.txt` (R from
    /// `old.txt`), with the regions.txt diff loaded and focused. The
    /// snapshot's `app` HEAD is `ccc` and its worktree is clean.
    fn compare_regions_app() -> AppState {
        let mut app = state();
        for row in app
            .snapshot
            .repos
            .iter_mut()
            .filter(|row| row.repo == "app")
        {
            row.head = "ccc".into();
            row.changes.clear();
        }
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let mut load = compare_range_load(&["regions.txt", "summary.txt", "new.txt"]);
        load.files[1].status = "A".into();
        load.files[2].status = "R".into();
        load.files[2].old_path = Some("old.txt".into());
        let _ = app.apply_compare_range(tab_id, gen, Ok(load));
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "regions.txt",
            Ok(DiffContent::from_compare_lines(
                COMPARE_REGIONS_DIFF.iter().map(|l| l.to_string()).collect(),
            )),
        );
        app.focus = FocusPane::Right;
        app
    }

    fn diff_row_of(app: &AppState, text: &str) -> usize {
        app.current_diff_rows()
            .iter()
            .position(|row| {
                matches!(row, DiffRow::Line { left, right }
                    if left.text == text || right.as_ref().is_some_and(|r| r.text == text))
            })
            .unwrap_or_else(|| panic!("row {text}"))
    }

    fn set_app_change(app: &mut AppState, path: &str, old_path: Option<&str>) {
        let row = app
            .snapshot
            .repos
            .iter_mut()
            .find(|row| row.repo == "app")
            .unwrap();
        row.changes = vec![FileChange {
            path: path.into(),
            staged_status: Some("M".into()),
            unstaged_status: None,
            untracked: false,
            old_path: old_path.map(str::to_string),
        }];
    }

    #[test]
    fn compare_revert_target_requires_loaded_head_and_clean_file() {
        let mut app = compare_regions_app();
        let target = app.compare_revert_target().expect("clean and at HEAD");
        assert_eq!(target.path, "regions.txt");
        assert_eq!(target.merge_base, "aaa");
        assert_eq!(target.head, "ccc");
        assert_eq!(app.compare_refusal(&Action::Revert), None);
        assert_eq!(palette_reason(&app, "Revert"), None);

        // Range still loading.
        app.tabs.active_compare_mut().unwrap().loading = true;
        assert_eq!(
            app.compare_revert_target().unwrap_err(),
            COMPARE_STILL_LOADING
        );
        app.tabs.active_compare_mut().unwrap().loading = false;

        // A diff load for another path is queued: content is still regions.txt.
        app.tabs.active_compare_mut().unwrap().path = Some("summary.txt".into());
        assert_eq!(
            app.compare_revert_target().unwrap_err(),
            COMPARE_STILL_LOADING
        );
        assert_eq!(
            palette_reason(&app, "Revert").as_deref(),
            Some(COMPARE_STILL_LOADING)
        );
        app.tabs.active_compare_mut().unwrap().path = Some("regions.txt".into());

        // HEAD moved and the tab has not reloaded yet.
        app.snapshot.repos.iter_mut().for_each(|row| {
            if row.repo == "app" {
                row.head = "ddd".into();
            }
        });
        assert_eq!(app.compare_revert_target().unwrap_err(), COMPARE_HEAD_MOVED);
        app.snapshot.repos.iter_mut().for_each(|row| {
            if row.repo == "app" {
                row.head = "ccc".into();
            }
        });

        // Staged change to the file.
        set_app_change(&mut app, "regions.txt", None);
        assert_eq!(
            app.compare_revert_target().unwrap_err(),
            "regions.txt has uncommitted changes"
        );
        assert_eq!(
            palette_reason(&app, "Revert").as_deref(),
            Some("regions.txt has uncommitted changes")
        );
        app.status.clear();
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert_eq!(app.status, "regions.txt has uncommitted changes");
        assert_eq!(app.confirm, None);

        // A rename's merge-base path counts too (left file row).
        set_app_change(&mut app, "old.txt", None);
        assert!(app.compare_revert_target().is_ok(), "regions.txt is clean");
        app.focus = FocusPane::Left;
        app.tabs.active_compare_mut().unwrap().file_cursor = app
            .commit_file_rows()
            .iter()
            .position(|row| row.path == "new.txt")
            .unwrap();
        assert_eq!(
            app.compare_revert_target().unwrap_err(),
            "old.txt has uncommitted changes"
        );

        // Workspace tab: the compare gate never refuses.
        app.dispatch(Action::JumpToTab(1));
        assert_eq!(app.compare_refusal(&Action::Revert), None);
    }

    #[test]
    fn compare_x_highlight_reverts_range_to_merge_base_after_y() {
        let mut app = compare_regions_app();
        app.diff_cursor = 1; // first @@ row
        app.dispatch(Action::DiffVisualStart);
        let alpha = diff_row_of(&app, "ALPHA-NEW");
        while app.diff_cursor < alpha {
            app.dispatch(Action::Move(1));
        }
        assert_eq!(palette_reason(&app, "Revert highlighted lines"), None);
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert_eq!(app.diff_visual_anchor, None, "the confirm clears VISUAL");
        let Some(PendingConfirm::CompareRevertRange { target, patch }) = app.confirm.clone() else {
            panic!("range confirm: {:?} / {}", app.confirm, app.status);
        };
        assert_eq!(target.path, "regions.txt");
        assert!(patch.contains("-ALPHA-OLD\n+ALPHA-NEW\n"), "{patch}");
        assert!(!patch.contains("OMEGA"), "{patch}");

        // `Y` is not offered: the confirm stays open and nothing runs.
        assert_eq!(app.dispatch(Action::ConfirmYesClean), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(
            app.dispatch(Action::ConfirmYes),
            Effect::CompareRevertPatch {
                repo: "app".into(),
                path: "regions.txt".into(),
                head: "ccc".into(),
                patch,
            }
        );
        assert_eq!(app.confirm, None);
    }

    #[test]
    fn compare_x_highlight_keeps_new_file_refusal() {
        let mut app = compare_regions_app();
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        app.tabs.active_compare_mut().unwrap().path = Some("summary.txt".into());
        let lines = [
            "diff --git a/summary.txt b/summary.txt",
            "new file mode 100644",
            "index 0000000..3333333",
            "--- /dev/null",
            "+++ b/summary.txt",
            "@@ -0,0 +1 @@",
            "+summary",
        ];
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "summary.txt",
            Ok(DiffContent::from_compare_lines(
                lines.iter().map(|l| l.to_string()).collect(),
            )),
        );
        app.diff_cursor = 1;
        app.dispatch(Action::DiffVisualStart);
        app.dispatch(Action::Move(1));
        assert_eq!(
            palette_reason(&app, "Revert highlighted lines").as_deref(),
            Some("cannot revert lines of a new file")
        );
        app.dispatch(Action::Revert);
        assert_eq!(app.status, "cannot revert lines of a new file");
        assert_eq!(app.confirm, None);
        assert!(app.diff_visual_anchor.is_some(), "a refusal keeps VISUAL");
    }

    #[test]
    fn compare_x_file_confirms_then_restores_from_merge_base() {
        // Diff focused, no highlight: the open file (M).
        let mut app = compare_regions_app();
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert_eq!(
            app.confirm,
            Some(PendingConfirm::CompareRevertFile {
                target: compare_revert_target_fixture("regions.txt", "M"),
            })
        );
        app.dispatch(Action::ConfirmNo);
        assert_eq!(app.confirm, None);
        assert_eq!(app.status, "revert cancelled");
        app.dispatch(Action::Revert);
        assert_eq!(
            app.dispatch(Action::ConfirmYes),
            Effect::CompareRevertFile {
                repo: "app".into(),
                path: "regions.txt".into(),
                old_path: None,
                deletes: false,
                merge_base: "aaa".into(),
                head: "ccc".into(),
            }
        );

        // File list: the added file deletes; the rename carries its old path.
        app.focus = FocusPane::Left;
        for (path, old_path, deletes) in [
            ("summary.txt", None, true),
            ("new.txt", Some("old.txt"), false),
        ] {
            app.tabs.active_compare_mut().unwrap().file_cursor = app
                .commit_file_rows()
                .iter()
                .position(|row| row.path == path)
                .unwrap();
            app.dispatch(Action::Revert);
            assert!(
                matches!(app.confirm, Some(PendingConfirm::CompareRevertFile { .. })),
                "{path}: {}",
                app.status
            );
            assert_eq!(
                app.dispatch(Action::ConfirmYes),
                Effect::CompareRevertFile {
                    repo: "app".into(),
                    path: path.into(),
                    old_path: old_path.map(str::to_string),
                    deletes,
                    merge_base: "aaa".into(),
                    head: "ccc".into(),
                },
                "{path}"
            );
        }
    }

    #[test]
    fn compare_palette_revert_row_checks_the_file_status_like_x() {
        let mut app = compare_regions_app();
        assert_eq!(palette_reason(&app, "Revert"), None);
        assert_eq!(app.compare_file_revert_refusal(), None);

        // A type change: `x`, the hint row, and the palette row all refuse.
        app.tabs.active_compare_mut().unwrap().files[0].status = "T".into();
        let reason = "cannot revert a T change";
        assert_eq!(app.compare_refusal(&Action::Revert), None, "gate passes");
        assert_eq!(app.compare_file_revert_refusal().as_deref(), Some(reason));
        assert_eq!(palette_reason(&app, "Revert").as_deref(), Some(reason));
        let hints: Vec<String> = super::super::chrome::action_hint_segments(&app)
            .into_iter()
            .map(|hint| hint.key)
            .collect();
        assert!(!hints.contains(&"x".to_string()), "{hints:?}");
        app.status.clear();
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert_eq!(app.status, reason);
        assert_eq!(app.confirm, None);
    }

    #[test]
    fn compare_idle_hints_read_the_compare_tab_not_the_parked_tree() {
        let keys = |app: &AppState| -> Vec<(String, String)> {
            super::super::chrome::action_hint_segments(app)
                .into_iter()
                .map(|hint| (hint.key, hint.label))
                .collect()
        };
        let has = |hints: &[(String, String)], key: &str| hints.iter().any(|(k, _)| k == key);
        let mut app = compare_regions_app();

        // Park a dirty Workspace file row: Workspace offers s / u / x there.
        app.dispatch(Action::JumpToTab(1));
        set_app_change(&mut app, "regions.txt", None);
        app.snapshot
            .repos
            .iter_mut()
            .find(|row| row.repo == "app")
            .unwrap()
            .changes[0]
            .unstaged_status = Some("M".into());
        app.rebuild_rows();
        app.focus = FocusPane::Left;
        app.cursor = app
            .rows
            .iter()
            .position(|row| {
                row.kind == NodeKind::File
                    && row.repo.as_deref() == Some("app")
                    && row
                        .file
                        .as_ref()
                        .is_some_and(|f| f.unstaged_status.is_some())
            })
            .expect("app unstaged file row");
        let hints = keys(&app);
        for key in ["s", "x", "f"] {
            assert!(has(&hints, key), "Workspace {key}: {hints:?}");
        }

        // The compare tab ignores that parked row.
        app.dispatch(Action::JumpToTab(2));
        app.focus = FocusPane::Right;
        let hints = keys(&app);
        for key in ["s", "u", "f", "S"] {
            assert!(!has(&hints, key), "compare {key}: {hints:?}");
        }
        // regions.txt is dirty in the worktree: compare `x` refuses.
        assert!(app.compare_file_revert_refusal().is_some());
        assert!(!has(&hints, "x"), "dirty file: {hints:?}");

        // Clean file at the compare head: `x` reverts to the merge base.
        for row in app.snapshot.repos.iter_mut().filter(|r| r.repo == "app") {
            row.changes.clear();
        }
        assert_eq!(app.compare_file_revert_refusal(), None);
        let hints = keys(&app);
        assert!(
            hints.contains(&("x".into(), "revert to merge base".into())),
            "{hints:?}"
        );
        assert!(has(&hints, "e") && has(&hints, "Ctrl-o"), "{hints:?}");

        // File list: the focused compare file decides.
        app.focus = FocusPane::Left;
        let rows = app.commit_file_rows();
        let file = rows.iter().position(|row| row.path == "new.txt").unwrap();
        app.tabs.active_compare_mut().unwrap().file_cursor = file;
        assert!(has(&keys(&app), "x"), "clean rename");
        set_app_change(&mut app, "old.txt", None);
        assert!(!has(&keys(&app), "x"), "rename source is dirty");
        assert!(!has(&keys(&app), "s"));
    }

    #[test]
    fn compare_visual_hints_drop_stage_and_follow_revert() {
        let keys = |app: &AppState| -> Vec<String> {
            super::super::chrome::visual_hint_segments(app)
                .into_iter()
                .map(|hint| hint.key)
                .collect()
        };
        let mut app = compare_regions_app();
        app.diff_cursor = 1;
        app.dispatch(Action::DiffVisualStart);
        let hints = keys(&app);
        assert!(!hints.contains(&"s u".to_string()), "{hints:?}");
        assert!(hints.contains(&"x".to_string()), "{hints:?}");
        set_app_change(&mut app, "regions.txt", None);
        let hints = keys(&app);
        assert!(!hints.contains(&"x".to_string()), "dirty file: {hints:?}");
        assert!(hints.contains(&";".to_string()), "{hints:?}");

        // Clean file at HEAD, but a new file: the range patch refuses, so
        // the hint row must not offer `x` either (same as the palette row).
        let mut app = compare_regions_app();
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        app.tabs.active_compare_mut().unwrap().path = Some("summary.txt".into());
        let lines = [
            "diff --git a/summary.txt b/summary.txt",
            "new file mode 100644",
            "index 0000000..3333333",
            "--- /dev/null",
            "+++ b/summary.txt",
            "@@ -0,0 +1 @@",
            "+summary",
        ];
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "summary.txt",
            Ok(DiffContent::from_compare_lines(
                lines.iter().map(|l| l.to_string()).collect(),
            )),
        );
        app.diff_cursor = 1;
        app.dispatch(Action::DiffVisualStart);
        app.dispatch(Action::Move(1));
        assert_eq!(app.compare_refusal(&Action::Revert), None, "target is ok");
        assert_eq!(
            palette_reason(&app, "Revert highlighted lines").as_deref(),
            Some("cannot revert lines of a new file")
        );
        let hints = keys(&app);
        assert!(!hints.contains(&"x".to_string()), "new file: {hints:?}");
        assert!(!hints.contains(&"s u".to_string()), "{hints:?}");
    }

    #[test]
    fn compare_confirms_never_cross_tabs() {
        let mut app = compare_regions_app();
        // A Workspace confirm left on a compare tab does not run.
        let workspace_confirm = PendingConfirm::RevertRange {
            repo: "app".into(),
            path: "README.md".into(),
            patch: "p".into(),
        };
        app.confirm = Some(workspace_confirm.clone());
        assert_eq!(app.dispatch(Action::ConfirmYes), Effect::None);
        assert_eq!(app.status, SWITCH_TO_WORKSPACE_TAB);
        assert_eq!(app.confirm, Some(workspace_confirm));

        // Switching tabs drops a compare confirm.
        app.confirm = None;
        app.dispatch(Action::Revert);
        assert!(app.confirm.is_some());
        app.dispatch(Action::JumpToTab(1));
        assert_eq!(app.confirm, None);

        // A compare confirm forced onto Workspace is cancelled, not run.
        app.confirm = Some(PendingConfirm::CompareRevertFile {
            target: compare_revert_target_fixture("regions.txt", "M"),
        });
        assert_eq!(app.dispatch(Action::ConfirmYes), Effect::None);
        assert_eq!(app.status, "revert cancelled");
        assert_eq!(app.confirm, None);
    }

    #[test]
    fn tab_switch_and_close_drop_highlight_and_confirm() {
        let stash_drop = || PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        };
        let mut app = state();
        compare_tab_with_added_lines(&mut app);
        app.dispatch(Action::DiffVisualStart);
        app.confirm = Some(stash_drop());
        app.dispatch(Action::JumpToTab(1));
        assert!(!app.is_compare_tab());
        assert_eq!(app.diff_visual_anchor, None);
        assert_eq!(app.diff_visual_rows, None);
        assert_eq!(app.confirm, None);

        app.confirm = Some(stash_drop());
        app.dispatch(Action::JumpToTab(2));
        assert!(app.is_compare_tab());
        assert_eq!(app.confirm, None, "a Workspace confirm must not follow");

        app.focus = FocusPane::Right;
        app.dispatch(Action::DiffVisualStart);
        assert!(app.diff_visual_anchor.is_some());
        app.confirm = Some(stash_drop());
        app.dispatch(Action::CloseTab);
        assert!(!app.is_compare_tab());
        assert_eq!(app.diff_visual_anchor, None);
        assert_eq!(app.confirm, None);

        // Mouse: a tab click and a `[✗]` click take the same path.
        compare_tab_with_added_lines(&mut app);
        app.layout.tab_y = 0;
        app.layout.tab_hits = vec![(0, 12, 0), (13, 20, 1)];
        app.layout.tab_close_hits = vec![(28, 3, 1)];
        app.dispatch(Action::DiffVisualStart);
        app.confirm = Some(stash_drop());
        app.dispatch(Action::Click { col: 2, row: 0 });
        assert!(!app.is_compare_tab());
        assert_eq!(app.diff_visual_anchor, None);
        assert_eq!(app.confirm, None);
        app.dispatch(Action::Click { col: 14, row: 0 });
        assert!(app.is_compare_tab());
        app.focus = FocusPane::Right;
        app.dispatch(Action::DiffVisualStart);
        app.confirm = Some(stash_drop());
        app.dispatch(Action::Click { col: 29, row: 0 });
        assert!(!app.is_compare_tab());
        assert_eq!(app.diff_visual_anchor, None);
        assert_eq!(app.confirm, None);
    }

    #[test]
    fn compare_left_apostrophe_names_base_branch() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(tab_id, gen, Ok(compare_range_load(&["README.md"])));
        app.focus = FocusPane::Left;
        assert_eq!(app.list_focus_target(), ListFocusTarget::CommitFiles);
        let effect = app.dispatch(Action::CopyEntityReference);
        let text = assert_copy_clipboard(effect, "file", "README.md", true);
        assert!(
            text.contains("base: main"),
            "left-pane compare copy must name the branch: {text}"
        );
    }

    #[test]
    fn compare_apostrophe_copies_diff_and_is_not_a_mutation() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(tab_id, gen, Ok(compare_range_load(&["README.md"])));
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "README.md",
            Ok(two_line_readme()),
        );
        app.focus = FocusPane::Right;
        assert_eq!(app.list_focus_target(), ListFocusTarget::None);
        assert_eq!(palette_reason(&app, "Copy entity reference"), None);
        let effect = app.dispatch(Action::CopyEntityReference);
        assert_ne!(app.status, super::super::tabs::SWITCH_TO_WORKSPACE_TAB);
        let text = assert_copy_clipboard(effect, "diff", "README.md", true);
        assert!(text.contains("path: README.md"), "{text}");
        assert!(text.contains("lines:"), "{text}");
        assert!(
            text.contains("source: compare main...ccc"),
            "compare diff copy must name the base branch: {text}"
        );
        assert!(!text.contains("source: worktree"), "{text}");
    }

    /// Commit-line key a compare diff of `README.md` stores (head `ccc`).
    fn compare_line_key(line: u32, end_line: u32) -> CommentKey {
        CommentKey::CommitLine {
            repo: "app".into(),
            sha: "ccc".into(),
            path: "README.md".into(),
            line,
            end_line,
        }
    }

    fn type_comment(app: &mut AppState, body: &str) {
        for c in body.chars() {
            app.dispatch(Action::CommentInput(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            )));
        }
    }

    #[test]
    fn compare_semicolon_keys_line_comment_to_compare_head() {
        let mut app = state();
        // The unit-test store file is shared; start from an empty store.
        app.comment_store = CommentStore::new();
        compare_tab_with_added_lines(&mut app);
        assert_eq!(palette_reason(&app, "Comment"), None);
        app.dispatch(Action::CommentStart);
        let prompt = app.comment.as_ref().expect("compare comment overlay");
        assert_eq!(prompt.key, compare_line_key(2, 2));
        assert!(prompt.label.contains("commit ccc"), "{}", prompt.label);
        type_comment(&mut app, "cmp");
        app.dispatch(Action::CommentSubmit);
        assert_eq!(app.status, "comment saved");
        assert_eq!(
            app.comment_store
                .get(&compare_line_key(2, 2))
                .map(|e| e.body.as_str()),
            Some("cmp")
        );
        assert_eq!(app.diff_line_comment(2), Some(false));
        assert_eq!(app.diff_line_comment(3), None);

        // The file row on the compare list marks it too.
        let (repo, source) = app.commit_drill_source().expect("compare list source");
        assert_eq!(repo, "app");
        assert_eq!(source, &compare_source());
        assert!(crate::tui::comments::commit_file_row_has_comment(
            &app.comment_store,
            repo,
            None,
            source,
            "README.md",
            Some("main"),
        ));

        // Ctrl-R in the overlay flips resolved, and the mark follows.
        app.dispatch(Action::CommentStart);
        assert_eq!(
            app.comment.as_ref().map(|p| p.key.clone()),
            Some(compare_line_key(2, 2))
        );
        app.dispatch(Action::CommentToggleResolved);
        assert!(app.comment.as_ref().is_some_and(|p| p.resolved));
        app.dispatch(Action::CommentSubmit);
        assert!(app
            .comment_store
            .get(&compare_line_key(2, 2))
            .is_some_and(|e| e.resolved));
        assert_eq!(app.diff_line_comment(2), Some(true));
    }

    #[test]
    fn compare_highlight_semicolon_keys_range_to_compare_head() {
        let mut app = state();
        // The unit-test store file is shared; start from an empty store.
        app.comment_store = CommentStore::new();
        compare_tab_with_added_lines(&mut app);
        app.dispatch(Action::DiffVisualStart);
        let (row, _) = numbered_diff_rows(&app)
            .into_iter()
            .find(|(_, line)| *line == 3)
            .expect("new line 3");
        app.diff_cursor = row;
        app.dispatch(Action::CommentStart);
        let prompt = app.comment.as_ref().expect("range overlay");
        assert_eq!(prompt.key, compare_line_key(2, 3));
        assert_eq!(app.diff_visual_anchor, None, "; leaves highlight");
    }

    #[test]
    fn compare_export_copies_compare_scope_only() {
        let mut app = state();
        // The unit-test store file is shared; start from an empty store.
        app.comment_store = CommentStore::new();
        focus_readme_diff(&mut app, two_line_readme());
        let worktree = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 2,
            end_line: 2,
        };
        let other_commit = CommentKey::CommitLine {
            repo: "app".into(),
            sha: "ddd".into(),
            path: "README.md".into(),
            line: 2,
            end_line: 2,
        };
        app.comment_store = put_comment(&app.comment_store, worktree, "workspace-note");
        app.comment_store = put_comment(&app.comment_store, other_commit, "other-commit-note");
        app.comment_store = put_comment(&app.comment_store, compare_line_key(3, 3), "compare-note");
        compare_tab_with_added_lines(&mut app);

        for focus in [FocusPane::Right, FocusPane::Left] {
            app.focus = focus;
            assert_eq!(palette_reason(&app, "Copy comments"), None);
            let text = match app.dispatch(Action::ExportComments) {
                Effect::CopyClipboard { text, .. } => text,
                other => panic!("{focus:?}: expected CopyClipboard, got {other:?}"),
            };
            assert!(text.contains("compare-note"), "{focus:?}: {text}");
            assert!(text.contains("commit `ccc`"), "{focus:?}: {text}");
            assert!(text.contains("`README.md`:3"), "{focus:?}: {text}");
            assert!(!text.contains("workspace-note"), "{focus:?}: {text}");
            assert!(!text.contains("other-commit-note"), "{focus:?}: {text}");
            app.dispatch(Action::ExportCommentsCancel);
        }
    }

    /// Regression: a Workspace worktree comment on the same checkout, path,
    /// and line number must not paint on a compare diff or its file row.
    #[test]
    fn compare_diff_does_not_paint_workspace_worktree_comments() {
        let mut app = state();
        // The unit-test store file is shared; start from an empty store.
        app.comment_store = CommentStore::new();
        focus_readme_diff(&mut app, two_line_readme());
        let worktree = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 2,
            end_line: 2,
        };
        app.comment_store = put_comment(&app.comment_store, worktree, "workspace-note");
        assert_eq!(app.diff_line_comment(2), Some(false), "Workspace paints it");

        compare_tab_with_added_lines(&mut app);
        assert_eq!(app.diff_line_comment(2), None, "compare must not paint it");
        let (repo, source) = app.commit_drill_source().expect("compare list source");
        assert!(!crate::tui::comments::commit_file_row_has_comment(
            &app.comment_store,
            repo,
            None,
            source,
            "README.md",
            Some("main"),
        ));

        app.dispatch(Action::JumpToTab(1));
        assert!(!app.is_compare_tab());
        assert_eq!(
            app.diff_line_comment(2),
            Some(false),
            "Workspace still does"
        );
    }

    #[test]
    fn compare_tab_without_range_has_no_comment_scope() {
        let mut app = state();
        // The unit-test store file is shared; start from an empty store.
        app.comment_store = CommentStore::new();
        focus_readme_diff(&mut app, two_line_readme());
        let worktree = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 2,
            end_line: 2,
        };
        app.comment_store = put_comment(&app.comment_store, worktree, "workspace-note");
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Right;
        assert_eq!(app.open_diff_target(), None);
        assert_eq!(app.commit_drill_source(), None);
        assert_eq!(app.dispatch(Action::CommentStart), Effect::None);
        assert_eq!(app.status, "no comment target");
        assert_eq!(app.dispatch(Action::ExportComments), Effect::None);
        assert_eq!(app.status, STATUS_NO_COMMENTS);
    }

    #[test]
    fn compare_left_wheel_moves_files_when_workspace_graph_is_parked() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.drill = DrillView::Graph;
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(
            tab_id,
            gen,
            Ok(compare_range_load(&["alpha.txt", "beta.txt"])),
        );
        app.focus = FocusPane::Left;
        app.layout.right_x = 48;
        let tree_cursor = app.cursor;
        let file_cursor = app.tabs.active_compare().unwrap().file_cursor;
        match app.dispatch(Action::ScrollWheel {
            col: 10,
            row: 4,
            delta: 1,
            horizontal: false,
        }) {
            Effect::LoadCompareDiff { path, .. } => assert_eq!(path, "beta.txt"),
            other => panic!("{other:?}"),
        }
        assert_eq!(app.cursor, tree_cursor, "parked tree cursor stays");
        assert_eq!(app.drill, DrillView::Graph);
        assert_ne!(
            app.tabs.active_compare().unwrap().file_cursor,
            file_cursor,
            "compare file list must move"
        );
    }

    #[test]
    fn compare_right_wheel_moves_diff_when_workspace_files_are_parked() {
        let mut app = state();
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "src/a.rs".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "M".into(),
                    path: "src/b.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        assert!(app.drill.is_files());
        let parked_file_cursor = match &app.drill {
            DrillView::Files { cursor, .. } => *cursor,
            other => panic!("{other:?}"),
        };
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        let tab_id = app.tabs.active_compare().unwrap().id;
        let gen = app.tabs.active_compare().unwrap().generation;
        let _ = app.apply_compare_range(tab_id, gen, Ok(compare_range_load(&["README.md"])));
        app.apply_compare_diff(
            tab_id,
            gen,
            &compare_source(),
            "README.md",
            Ok(two_line_readme()),
        );
        app.focus = FocusPane::Right;
        app.layout.right_x = 48;
        let file_cursor = app.tabs.active_compare().unwrap().file_cursor;
        let diff_cursor = app.diff_cursor;
        assert_eq!(
            app.dispatch(Action::ScrollWheel {
                col: 80,
                row: 4,
                delta: 1,
                horizontal: false,
            }),
            Effect::None
        );
        assert_eq!(
            app.tabs.active_compare().unwrap().file_cursor,
            file_cursor,
            "compare file cursor stays"
        );
        match &app.drill {
            DrillView::Files { cursor, .. } => assert_eq!(*cursor, parked_file_cursor),
            other => panic!("{other:?}"),
        }
        assert_ne!(app.diff_cursor, diff_cursor, "compare DiffPane must move");
    }

    #[test]
    fn compare_click_close_hits_close_compare_and_workspace_never_closes() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.tabs
            .open_or_focus("app".into(), "develop".into(), "HEAD".into());
        assert_eq!(app.tabs.active, 2);
        app.layout.tab_y = 0;
        app.layout.tab_hits = vec![(0, 12, 0), (13, 20, 1), (34, 22, 2)];
        app.layout.tab_close_hits = vec![(28, 3, 1), (51, 3, 2)];
        assert_eq!(
            app.dispatch(Action::Click { col: 29, row: 0 }),
            Effect::None
        );
        assert_eq!(app.tabs.active, 1);
        assert_eq!(app.tabs.compare_count(), 1);
        app.layout.tab_close_hits = vec![(2, 3, 0)];
        assert_eq!(app.dispatch(Action::Click { col: 3, row: 0 }), Effect::None);
        assert_eq!(app.status, WORKSPACE_TAB_CANNOT_CLOSE);
        assert_eq!(app.tabs.active, 1);
        assert_eq!(app.tabs.compare_count(), 1);
    }

    /// Ctrl-w runs the key → action → dispatch path: it closes the active
    /// compare tab (the left neighbour takes over), and on Workspace it only
    /// warns, never quits or closes.
    #[test]
    fn ctrl_w_closes_active_tab_and_warns_on_workspace() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        fn press_ctrl_w(app: &mut AppState) -> Effect {
            let event = Event::Key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::CONTROL));
            let action = crate::tui::keys::event_to_action(&event, app.input_mode(), false, false);
            assert_eq!(action, Action::CloseTab);
            app.dispatch(action)
        }
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.tabs
            .open_or_focus("app".into(), "develop".into(), "HEAD".into());
        assert_eq!(app.tabs.active, 2);
        assert_eq!(app.tabs.compare_count(), 2);

        assert_eq!(press_ctrl_w(&mut app), Effect::None);
        assert_eq!(app.tabs.active, 1, "left neighbour is active");
        assert_eq!(app.tabs.compare_count(), 1);
        assert_eq!(press_ctrl_w(&mut app), Effect::None);
        assert!(app.tabs.is_workspace());
        assert_eq!(app.tabs.compare_count(), 0);

        let len = app.tabs.len();
        assert_eq!(press_ctrl_w(&mut app), Effect::None);
        assert_eq!(app.status, WORKSPACE_TAB_CANNOT_CLOSE);
        assert_eq!(app.tabs.len(), len, "Workspace never closes");
        assert!(app.tabs.is_workspace());
    }

    #[test]
    fn pointer_move_reports_tab_close_hover_changes_and_keeps_chords() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.layout.tab_y = 0;
        app.layout.tab_close_hits = vec![(28, 3, 1)];
        assert!(!app.set_pointer(Some((5, 0))), "off close: no hover change");
        assert_eq!(app.hovered_tab_close(), None);
        assert!(app.set_pointer(Some((28, 0))), "onto close");
        assert_eq!(app.hovered_tab_close(), Some(1));
        assert!(!app.set_pointer(Some((30, 0))), "still on the same close");
        assert!(app.set_pointer(Some((29, 1))), "row below leaves close");
        assert!(!app.set_pointer(Some((2, 1))), "still off close");
        assert!(app.set_pointer(Some((28, 0))));
        assert!(app.set_pointer(Some((31, 0))), "past close");
        assert!(app.set_pointer(Some((29, 0))));
        assert!(app.set_pointer(None), "pointer cleared");

        let armed = Instant::now();
        app.z_pending_at = Some(armed);
        app.g_pending_at = Some(armed);
        app.status = "keep".into();
        assert_eq!(
            app.dispatch(Action::PointerMove { col: 29, row: 0 }),
            Effect::None
        );
        assert_eq!(app.z_pending_at, Some(armed), "z chord stays armed");
        assert_eq!(app.g_pending_at, Some(armed), "g chord stays armed");
        assert_eq!(app.status, "keep");
        assert_eq!(app.pointer, None, "dispatch does not store the pointer");
    }

    #[test]
    fn pointer_on_search_chips_reports_hover_and_hides_the_tab_close() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.layout.tab_y = 0;
        app.layout.tab_close_hits = vec![(28, 3, 1)];
        app.layout.search_option_y = 2;
        app.layout.search_option_hits = vec![
            (80, 4, SearchFilesOption::Case),
            (85, 4, SearchFilesOption::WholeWord),
            (90, 4, SearchFilesOption::Regex),
        ];
        assert!(
            !app.set_pointer(Some((80, 2))),
            "no dialog: its boxes do not hover"
        );
        assert_eq!(app.hovered_search_option(), None);
        assert_eq!(app.search_option_at(80, 2), None);
        app.set_pointer(Some((0, 5)));

        app.dispatch(Action::ToggleSearchFiles);
        assert!(app.search_files.is_some());
        assert!(
            app.set_pointer(Some((80, 2))),
            "the open dialog hovers the chip under the pointer"
        );
        assert_eq!(app.hovered_search_option(), Some(SearchFilesOption::Case));
        assert!(!app.set_pointer(Some((83, 2))), "same chip, last cell");
        assert!(app.set_pointer(Some((84, 2))), "gap between chips");
        assert_eq!(app.hovered_search_option(), None);
        assert!(app.set_pointer(Some((85, 2))));
        assert_eq!(
            app.hovered_search_option(),
            Some(SearchFilesOption::WholeWord)
        );
        assert!(app.set_pointer(Some((93, 2))), "onto the next chip");
        assert_eq!(app.hovered_search_option(), Some(SearchFilesOption::Regex));
        assert!(app.set_pointer(Some((93, 3))), "row below leaves the chip");
        assert_eq!(app.hovered_search_option(), None);

        assert!(
            !app.set_pointer(Some((29, 0))),
            "the tab strip does not hover under the dialog"
        );
        assert_eq!(app.hovered_tab_close(), None);
        assert_eq!(app.pointer, Some((29, 0)), "the dialog keeps the pointer");

        app.dispatch(Action::SearchFilesCancel);
        assert_eq!(app.hovered_tab_close(), Some(1), "closed: the strip hovers");
        assert_eq!(
            app.search_option_at(80, 2),
            None,
            "a closed dialog's boxes never hit"
        );
    }

    #[test]
    fn toggle_mouse_clears_pointer_and_off_ignores_motion() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.layout.tab_y = 0;
        app.layout.tab_close_hits = vec![(28, 3, 1)];
        assert!(app.set_pointer(Some((29, 0))));
        app.dispatch(Action::ToggleMouse);
        assert!(!app.mouse_enabled);
        assert_eq!(app.pointer, None);
        assert_eq!(app.hovered_tab_close(), None);
        assert!(!app.set_pointer(Some((29, 0))), "mouse off: no hover");
        assert_eq!(app.pointer, None);
        app.dispatch(Action::ToggleMouse);
        assert!(app.set_pointer(Some((29, 0))));
        assert_eq!(app.hovered_tab_close(), Some(1));
    }

    #[test]
    fn compare_tab_branch_submit_does_not_checkout() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_branch_picker(
            "app".into(),
            vec![
                LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                },
                LocalBranch {
                    name: "feature/x".into(),
                    current: false,
                    authordate: 2,
                },
            ],
        );
        app.dispatch(Action::BranchChar('f'));
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        assert!(app.is_compare_tab());
        assert_eq!(app.dispatch(Action::BranchSubmit), Effect::None);
        assert_eq!(app.status, super::super::tabs::SWITCH_TO_WORKSPACE_TAB);
        assert!(app.branch_picker.is_some());
    }

    #[test]
    fn compare_probe_skips_loading_tabs() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.tabs.active_compare_mut().unwrap().loading = true;
        assert!(app.compare_probe_effects().is_empty());
        app.tabs.active_compare_mut().unwrap().loading = false;
        assert_eq!(app.compare_probe_effects().len(), 1);
    }

    #[test]
    fn nav_esc_on_compare_left_keeps_the_tab() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        assert!(app.is_compare_tab());
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert_eq!(app.focus, FocusPane::Left);
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.is_compare_tab());
        assert_eq!(app.tabs.compare_count(), 1);
        assert_eq!(app.tabs.active, 1);
    }

    #[test]
    fn nav_esc_abandons_in_flight_compare_picker() {
        let mut app = state();
        app.compare_picker_pending = Some("app".into());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.compare_picker_pending.is_none());
    }

    #[test]
    fn external_diff_commit_stash_and_worktree_source() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        let commit_id = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: commit_id.into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
        );
        let idx = app
            .commit_file_rows()
            .iter()
            .position(|row| row.path == "src/lib.rs")
            .expect("file");
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = idx;
        }
        match app.dispatch(Action::ExternalDiff) {
            Effect::ExternalDiff { repo, path, kind } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "src/lib.rs");
                assert_eq!(
                    kind,
                    ExternalDiffKind::Rev {
                        left_rev: format!("{commit_id}^"),
                        right_rev: commit_id.into(),
                        left_path: None,
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: commit_id.into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
            idx,
            "src/lib.rs".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        match app.dispatch(Action::ExternalDiff) {
            Effect::ExternalDiff { path, kind, .. } => {
                assert_eq!(path, "src/lib.rs");
                assert_eq!(
                    kind,
                    ExternalDiffKind::Rev {
                        left_rev: format!("{commit_id}^"),
                        right_rev: commit_id.into(),
                        left_path: None,
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Stash {
                stash_ref: "stash@{0}".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
        );
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = idx;
        }
        match app.dispatch(Action::ExternalDiff) {
            Effect::ExternalDiff { path, kind, .. } => {
                assert_eq!(path, "src/lib.rs");
                assert_eq!(
                    kind,
                    ExternalDiffKind::Rev {
                        left_rev: "stash@{0}^1".into(),
                        right_rev: "stash@{0}".into(),
                        left_path: None,
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Worktree,
            vec![CommitFile {
                status: "M".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
        );
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = idx;
        }
        match app.dispatch(Action::ExternalDiff) {
            Effect::ExternalDiff { path, kind, .. } => {
                assert_eq!(path, "src/lib.rs");
                assert_eq!(kind, ExternalDiffKind::Worktree);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn file_writes_on_git_fixture_honor_revert_confirm() {
        use crate::config::WorkspaceStatusConfig;
        use crate::git::{revert_tracked_file, stage_file, unstage_file};
        use crate::parallel::FETCH_CONCURRENCY;
        use crate::tui::collect_full_snapshot;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "ws-tui-ops-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();

        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot =
            collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                stage_file(&repo_dir, &paths[0]).unwrap();
            }
            other => panic!("{other:?}"),
        }
        unstage_file(&repo_dir, "README.md").unwrap();
        focus_file(&mut app, "README.md");
        app.dispatch(Action::Revert);
        assert!(app.confirm.is_some());
        app.dispatch(Action::ConfirmNo);
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# dirty\n"
        );
        app.dispatch(Action::Revert);
        match app.dispatch(Action::ConfirmYes) {
            Effect::Revert {
                tracked, untracked, ..
            } => {
                assert!(untracked.is_empty());
                revert_tracked_file(&repo_dir, &tracked[0]).unwrap();
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# seed\n"
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn focus_repo(app: &mut AppState, name: &str) {
        let idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Repo && r.repo.as_deref() == Some(name))
            .expect("repo row");
        app.cursor = idx;
    }

    #[test]
    fn push_omits_hidden_and_unfocused_worktree() {
        use crate::snapshot::CheckoutKind;
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", true),
                RepoSnapshot {
                    repo: ".worktrees/app/feat".into(),
                    branch: "feature/x".into(),
                    sync_status: SyncStatus::Ahead,
                    sync_note: "ahead 1".into(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: None,
                    default_branch_override: None,
                    default_tip_ref: None,
                    local_branches: Vec::new(),
                },
                RepoSnapshot {
                    repo: "notes".into(),
                    branch: "main".into(),
                    sync_status: SyncStatus::Ahead,
                    sync_note: "ahead 1".into(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Primary,
                    primary_repo: None,
                    merged_into_default: None,
                    default_branch_override: None,
                    default_tip_ref: None,
                    local_branches: Vec::new(),
                },
            ],
            &["notes".into()],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::Push), Effect::None);
        focus_repo(&mut app, "app");
        let repos = confirm_and_push(&mut app, Action::Push, "app");
        assert_eq!(repos, vec!["app"]);
        assert!(!repos
            .iter()
            .any(|r| r.contains("worktrees") || r == "notes"));
        let wt = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some(".worktrees/app/feat"));
        if let Some(idx) = wt {
            app.cursor = idx;
            assert_eq!(
                confirm_and_push(&mut app, Action::Push, ".worktrees/app/feat"),
                vec![".worktrees/app/feat"]
            );
        }
        app.dispatch(Action::ToggleShowHidden);
        let notes = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes"))
            .expect("notes");
        app.cursor = notes;
        assert_eq!(
            confirm_and_push(&mut app, Action::Push, "notes"),
            vec!["notes"]
        );
    }

    #[test]
    fn stash_pop_drop_confirm_cancel_and_apply() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Left;
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("file menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create]
        );
        assert_eq!(app.dispatch(Action::StashMenuCancel), Effect::None);
        assert!(app.stash_menu.is_none());
        assert!(app.status.contains("cancelled"));

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(
            app.dispatch(Action::StashMenuChar('s')),
            Effect::StashCreate {
                repo: "app".into(),
                paths: vec!["README.md".into()],
            }
        );
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('a')), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.dispatch(Action::StashMenuChar('p')), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.dispatch(Action::StashMenuChar('d')), Effect::None);
        assert!(app.confirm.is_none());

        app.folds.remove("group:no-updates");
        app.rebuild_rows();
        focus_repo(&mut app, "lib");
        app.focus = FocusPane::Left;
        app.open_stash_menu("lib".into(), None);
        assert!(app.stash_menu.is_none());
        assert!(app.status.contains("nothing to stash"));
        // A clean checkout with a stash offers apply / pop of the latest.
        app.open_stash_menu("lib".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("clean row menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Apply, StashOpId::Pop]
        );
        assert!(
            app.status.is_empty(),
            "the box lists the keys; no status line repeats them: {:?}",
            app.status
        );
        assert!(ops
            .iter()
            .all(|op| op.stash_ref.as_deref() == Some("stash@{0}")));
        match app.dispatch(Action::StashMenuChar('a')) {
            Effect::StashApply { repo, stash_ref } => {
                assert_eq!(repo, "lib");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }

        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{0}".into(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{0}"))
            .expect("stash");
        app.graph_cursor = idx;
        match app.dispatch(Action::GraphStashApply) {
            Effect::StashApply { repo, stash_ref } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.dispatch(Action::GraphStashDrop), Effect::None);
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.status.contains("drop cancelled"));
        app.dispatch(Action::GraphStashDrop);
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
            other => panic!("{other:?}"),
        }
        match app.dispatch(Action::GraphStashPop) {
            Effect::StashPop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn branch_picker_list_create_checkout() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        assert!(matches!(
            app.dispatch(Action::Branch),
            Effect::PrepareBranchPicker { repo } if repo == "app"
        ));
        app.open_branch_picker(
            "app".into(),
            vec![
                LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                },
                LocalBranch {
                    name: "feature/x".into(),
                    current: false,
                    authordate: 2,
                },
            ],
        );
        assert!(app.branch_picker.is_some());
        app.dispatch(Action::BranchChar('f'));
        assert_eq!(
            app.branch_picker
                .as_ref()
                .unwrap()
                .selected()
                .map(|b| b.name.as_str()),
            Some("feature/x")
        );
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "feature/x");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }

        app.open_branch_picker(
            "app".into(),
            vec![LocalBranch {
                name: "main".into(),
                current: true,
                authordate: 1,
            }],
        );
        // A name git would refuse gets no create row; Enter says why.
        for c in "bad..name".chars() {
            app.dispatch(Action::BranchChar(c));
        }
        assert_eq!(app.branch_picker.as_ref().unwrap().row_count(), 0);
        assert_eq!(app.dispatch(Action::BranchSubmit), Effect::None);
        assert!(app.branch_picker.is_some());
        assert_eq!(
            app.status,
            "no branch matches bad..name · a branch name cannot contain .."
        );
        assert_eq!(app.status.kind(), StatusKind::Warn);
        for _ in "bad..name".chars() {
            app.dispatch(Action::BranchBackspace);
        }
        for c in "feature/new".chars() {
            app.dispatch(Action::BranchChar(c));
        }
        let picker = app.branch_picker.as_ref().unwrap();
        assert!(picker.on_create_row(), "the create row is the only row");
        match app.dispatch(Action::BranchSubmit) {
            Effect::CreateBranch { name, repo } => {
                assert_eq!(repo, "app");
                assert_eq!(name, "feature/new");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.branch_picker.is_none());
        assert_eq!(app.status, "creating feature/new…");

        assert_eq!(
            app.confirm_checkout_if_out_of_sync(
                "app".into(),
                "feature/x".into(),
                Some("origin/feature/x".into()),
                Some((0, 1)),
            ),
            Effect::None
        );
        assert!(app.confirm.is_some());
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        app.confirm_checkout_if_out_of_sync(
            "app".into(),
            "feature/x".into(),
            Some("origin/feature/x".into()),
            None,
        );
        match app.dispatch(Action::ConfirmYes) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "feature/x");
                assert_eq!(fast_forward_ref.as_deref(), Some("origin/feature/x"));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn tree_picker_already_on_closes_with_no_git() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_branch_picker(
            "app".into(),
            vec![LocalBranch {
                name: "main".into(),
                current: true,
                authordate: 1,
            }],
        );
        assert_eq!(app.dispatch(Action::BranchSubmit), Effect::None);
        assert!(app.branch_picker.is_none());
        assert_eq!(app.status, "Already on main");
        assert!(app.confirm.is_none());
    }

    #[test]
    fn tree_picker_local_selection_never_confirms() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_branch_picker(
            "app".into(),
            vec![
                LocalBranch {
                    name: "main".into(),
                    current: true,
                    authordate: 1,
                },
                LocalBranch {
                    name: "feature/x".into(),
                    current: false,
                    authordate: 2,
                },
            ],
        );
        app.dispatch(Action::BranchChar('f'));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "feature/x");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert!(app.branch_picker.is_some());
    }

    fn graph_state(dirty: bool) -> AppState {
        let mut app_repo = repo("app", dirty);
        if !dirty {
            app_repo.branch = "feature/x".into();
        }
        let snapshot = build_workspace_snapshot(
            &[app_repo, repo("notes", true)],
            &["notes".into()],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn install_graph_commit(app: &mut AppState, refs: &[&str]) {
        install_graph_commit_refs(app, refs.iter().map(|s| GraphRef::from(*s)).collect());
    }

    fn install_graph_commit_refs(app: &mut AppState, refs: Vec<GraphRef>) {
        let id = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let model = GraphModel {
            commits: vec![Commit {
                id: id.into(),
                subject: "head".into(),
                refs,
                ..Commit::default()
            }],
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some(id.into()),
            sync: None,
            show_hidden: app.show_hidden,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(model, "app".into(), id.into());
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    #[test]
    fn graph_commit_b_one_local_ref_checkouts() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        match app.dispatch(Action::GraphCheckout) {
            Effect::CheckoutBranch {
                repo,
                selected_name,
                fast_forward_ref,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(selected_name, "main");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_commit_b_several_refs_opens_name_picker() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["origin/z", "topic", "main", "origin/main"]);
        assert_eq!(app.dispatch(Action::GraphCheckout), Effect::None);
        let picker = app.branch_picker.as_ref().expect("picker");
        let names: Vec<&str> = picker.branches.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, vec!["main", "topic", "origin/main", "origin/z"]);
        app.dispatch(Action::BranchMove(2));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                ..
            } => {
                assert_eq!(selected_name, "origin/main");
                assert!(fast_forward_ref.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_checkout_picker_create_row_creates_at_the_commit() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["topic", "main"]);
        assert_eq!(app.dispatch(Action::GraphCheckout), Effect::None);
        // j / k / C type into the filter: they never move or open a prompt.
        for c in "jk/C".chars() {
            app.dispatch(Action::BranchChar(c));
        }
        let picker = app.branch_picker.as_ref().expect("picker");
        assert_eq!(picker.filter, "jk/C");
        assert_eq!(picker.create_name(), Some("jk/C"));
        assert!(picker.on_create_row());
        match app.dispatch(Action::BranchSubmit) {
            Effect::CreateBranchAt {
                repo,
                name,
                commit_id,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(name, "jk/C");
                assert_eq!(commit_id, "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.branch_picker.is_none());
        assert_eq!(app.status, "creating jk/C at aaa1111…");
    }

    #[test]
    fn graph_checkout_picker_exact_name_hides_the_create_row() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["topic", "main"]);
        app.dispatch(Action::GraphCheckout);
        for c in "main".chars() {
            app.dispatch(Action::BranchChar(c));
        }
        let picker = app.branch_picker.as_ref().expect("picker");
        assert_eq!(picker.create_name(), None);
        assert_eq!(picker.row_count(), 1);
        app.dispatch(Action::BranchMove(1));
        assert!(matches!(
            app.dispatch(Action::BranchSubmit),
            Effect::CheckoutBranch { selected_name, .. } if selected_name == "main"
        ));
    }

    #[test]
    fn graph_checkout_picker_hides_create_for_a_local_branch_on_another_commit() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        for repo in app.snapshot.repos.iter_mut().filter(|r| r.repo == "app") {
            repo.local_branches = vec!["main".into(), "topic".into(), "elsewhere".into()];
        }
        install_graph_commit(&mut app, &["topic", "main"]);
        app.dispatch(Action::GraphCheckout);
        for c in "elsewhere".chars() {
            app.dispatch(Action::BranchChar(c));
        }
        let picker = app.branch_picker.as_ref().expect("picker");
        assert_eq!(picker.create_name(), None);
        assert_eq!(picker.row_count(), 0);
        assert_eq!(app.dispatch(Action::BranchSubmit), Effect::None);
        assert!(app.branch_picker.is_some(), "nothing to run keeps it open");
        assert_eq!(
            app.status,
            "no branch matches elsewhere · branch elsewhere already exists"
        );
    }

    #[test]
    fn tree_picker_with_matches_still_offers_create_last() {
        use crate::git::LocalBranch;
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_branch_picker(
            "app".into(),
            vec![LocalBranch {
                name: "feature/x".into(),
                current: false,
                authordate: 1,
            }],
        );
        for c in "feat".chars() {
            app.dispatch(Action::BranchChar(c));
        }
        assert_eq!(app.branch_picker.as_ref().unwrap().row_count(), 2);
        app.dispatch(Action::BranchMove(1));
        assert!(matches!(
            app.dispatch(Action::BranchSubmit),
            Effect::CreateBranch { name, .. } if name == "feat"
        ));
    }

    #[test]
    fn graph_commit_b_dirty_refuses() {
        let mut app = graph_state(true);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        assert_eq!(app.dispatch(Action::GraphCheckout), Effect::None);
        assert!(app.branch_picker.is_none());
        assert!(app.status.contains("commit or stash"));
    }

    #[test]
    fn graph_commit_b_tag_only_is_noop() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit_refs(&mut app, vec![GraphRef::tag("v1.0")]);
        assert_eq!(app.dispatch(Action::GraphCheckout), Effect::None);
        assert!(app.branch_picker.is_none());
    }

    #[test]
    fn graph_commit_c_creates_at_commit_without_checkout() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        assert!(!app.create_branch.as_ref().unwrap().commit_id.is_empty());
        for c in "topic/x".chars() {
            app.dispatch(Action::CreateBranchChar(c));
        }
        match app.dispatch(Action::CreateBranchSubmit) {
            Effect::CreateBranchAt {
                repo,
                name,
                commit_id,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(name, "topic/x");
                assert_eq!(commit_id, "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_commit_m_opens_confirm_yes_merges_no_cancels() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        match &app.confirm {
            Some(PendingConfirm::MergeIntoHead {
                repo,
                rev,
                label,
                into,
            }) => {
                assert_eq!(repo, "app");
                assert_eq!(rev, "topic");
                assert_eq!(label, "topic");
                assert_eq!(into, "feature/x");
            }
            other => panic!("{other:?}"),
        }
        match app.dispatch(Action::ConfirmYes) {
            Effect::MergeIntoHead { repo, rev, label } => {
                assert_eq!(repo, "app");
                assert_eq!(rev, "topic");
                assert_eq!(label, "topic");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());

        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "merge cancelled");
    }

    #[test]
    fn graph_commit_m_dirty_refuses() {
        let mut app = graph_state(true);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("commit or stash"));
    }

    #[test]
    fn graph_commit_m_tag_merges_commit_id() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit_refs(&mut app, vec![GraphRef::tag("v1.0")]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        match &app.confirm {
            Some(PendingConfirm::MergeIntoHead { rev, label, .. }) => {
                assert_eq!(rev, "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
                assert_eq!(label, "aaa1111");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_merge_worktree_only_when_that_row_is_focused() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("commit or stash"));

        focus_checkout(&mut app, "app/.worktrees/feat");
        install_graph_commit(&mut app, &["topic"]);
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        match &app.confirm {
            Some(PendingConfirm::MergeIntoHead { repo, into, .. }) => {
                assert_eq!(repo, "app/.worktrees/feat");
                assert_eq!(into, "feature/x");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_stash_m_is_noop_for_merge() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{0}".into(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(_)))
            .expect("stash");
        app.graph_cursor = idx;
        assert!(!app.graph_commit_focused());
        assert_eq!(app.dispatch(Action::GraphMerge), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn tree_b_still_opens_head_picker() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        assert!(matches!(
            app.dispatch(Action::Branch),
            Effect::PrepareBranchPicker { repo } if repo == "app"
        ));
        install_graph_commit(&mut app, &["main"]);
        app.focus = FocusPane::Left;
        assert!(matches!(
            app.dispatch(Action::Branch),
            Effect::PrepareBranchPicker { repo } if repo == "app"
        ));
    }

    #[test]
    fn tree_c_is_noop_on_repo_file_and_workspace() {
        let mut app = state();
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        assert!(app.create_branch.is_none());
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
    }

    #[test]
    fn tree_o_prepares_graph_focus_on_repo_and_worktree() {
        use crate::tui::graph_focus::GRAPH_FOCUS_NEED_CONTEXT;

        let mut app = state();
        focus_repo(&mut app, "app");
        assert!(matches!(
            app.dispatch(Action::GraphFocusBranches),
            Effect::PrepareGraphFocusPicker { repo } if repo == "app"
        ));
        assert_eq!(palette_reason(&app, "Graph focus branches"), None);
        assert_eq!(
            palette_reason(&app, "Clear graph focus").as_deref(),
            Some("no graph focus to clear")
        );

        app.graph_identity = Some(("lib".into(), "head-lib".into()));
        assert!(
            matches!(
                app.dispatch(Action::GraphFocusBranches),
                Effect::PrepareGraphFocusPicker { repo } if repo == "app"
            ),
            "tree o must use the highlighted repo, not a stale graph identity"
        );

        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::GraphFocusBranches), Effect::None);
        assert_eq!(
            palette_reason(&app, "Graph focus branches").as_deref(),
            Some(GRAPH_FOCUS_NEED_CONTEXT)
        );

        app.cursor = 0;
        assert_eq!(app.dispatch(Action::GraphFocusBranches), Effect::None);
        assert_eq!(
            palette_reason(&app, "Graph focus branches").as_deref(),
            Some(GRAPH_FOCUS_NEED_CONTEXT)
        );

        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_commit(&mut app, &["main"]);
        assert!(matches!(
            app.dispatch(Action::GraphFocusBranches),
            Effect::PrepareGraphFocusPicker { repo } if repo == "app"
        ));
        app.graph_branch_focus = Some(("app".into(), vec!["main".into()]));
        app.graph_identity = None;
        let keys: Vec<String> = crate::tui::chrome::action_hint_segments(&app)
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(
            keys.contains(&"O".into()),
            "graph O hint must follow active focus, not identity: {keys:?}"
        );

        let snapshot = build_workspace_snapshot(
            &[
                repo("app", false),
                RepoSnapshot {
                    repo: "app/.worktrees/feat".into(),
                    branch: "feature/x".into(),
                    sync_status: SyncStatus::NoUpstream,
                    sync_note: String::new(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: None,
                    default_branch_override: None,
                    default_tip_ref: None,
                    local_branches: Vec::new(),
                },
            ],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let idx = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Checkout)
            .expect("linked worktree row");
        app.cursor = idx;
        let expected = app.rows[idx].repo.clone().expect("checkout path");
        assert!(matches!(
            app.dispatch(Action::GraphFocusBranches),
            Effect::PrepareGraphFocusPicker { repo } if repo == expected
        ));
    }

    #[test]
    fn graph_stash_b_stays_tree_and_c_is_noop() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{0}".into(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(_)))
            .expect("stash");
        app.graph_cursor = idx;
        assert!(!app.graph_commit_focused());
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(app.dispatch(Action::Branch), Effect::None);
        assert_eq!(app.dispatch(Action::GraphCreateBranch), Effect::None);
        assert!(app.create_branch.is_none());
    }

    #[test]
    fn stash_and_branch_git_fixture_honor_confirm() {
        use crate::config::WorkspaceStatusConfig;
        use crate::git::{
            checkout_branch, create_branch_checkout, latest_stash_ref, list_local_branches,
            stash_apply, stash_drop, stash_pop, stash_push,
        };
        use crate::parallel::FETCH_CONCURRENCY;
        use crate::tui::collect_full_snapshot;
        use std::fs;
        use std::time::{SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "ws-tui-stash-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();

        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot =
            collect_full_snapshot(&workspace, &config, &[], false, false, FETCH_CONCURRENCY);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        focus_file(&mut app, "README.md");
        app.open_stash_menu("app".into(), latest_stash_ref(&repo_dir));
        match app.dispatch(Action::StashMenuChar('s')) {
            Effect::StashCreate { paths, .. } => stash_push(&repo_dir, &paths).unwrap(),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# seed\n"
        );
        let latest = latest_stash_ref(&repo_dir).expect("stash");
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: latest.clone(),
                subject: "latest".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == latest))
            .expect("stash row");
        app.graph_cursor = idx;
        match app.dispatch(Action::GraphStashApply) {
            Effect::StashApply { stash_ref, .. } => stash_apply(&repo_dir, &stash_ref).unwrap(),
            other => panic!("{other:?}"),
        }
        assert_eq!(
            fs::read_to_string(repo_dir.join("README.md")).unwrap(),
            "# dirty\n"
        );
        app.dispatch(Action::GraphStashDrop);
        app.dispatch(Action::ConfirmNo);
        assert_eq!(
            latest_stash_ref(&repo_dir).as_deref(),
            Some(latest.as_str())
        );
        app.dispatch(Action::GraphStashDrop);
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => stash_drop(&repo_dir, &stash_ref).unwrap(),
            other => panic!("{other:?}"),
        }
        assert!(latest_stash_ref(&repo_dir).is_none());

        fs::write(repo_dir.join("README.md"), "# again\n").unwrap();
        stash_push(&repo_dir, &[]).unwrap();
        let latest = latest_stash_ref(&repo_dir).expect("stash2");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: latest.clone(),
                subject: "again".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == latest))
            .expect("stash2 row");
        app.graph_cursor = idx;
        assert_eq!(
            app.dispatch(Action::GraphStashPop),
            Effect::StashPop {
                repo: "app".into(),
                stash_ref: latest.clone(),
            }
        );
        stash_pop(&repo_dir, &latest).unwrap();
        assert!(latest_stash_ref(&repo_dir).is_none());

        focus_repo(&mut app, "app");
        app.focus = FocusPane::Left;
        match app.dispatch(Action::Branch) {
            Effect::PrepareBranchPicker { repo } => {
                app.open_branch_picker(repo, list_local_branches(&repo_dir));
            }
            other => panic!("{other:?}"),
        }
        assert!(app
            .branch_picker
            .as_ref()
            .unwrap()
            .branches
            .iter()
            .any(|b| b.name == "main"));
        for c in "feature/pick".chars() {
            app.dispatch(Action::BranchChar(c));
        }
        match app.dispatch(Action::BranchSubmit) {
            Effect::CreateBranch { name, .. } => {
                create_branch_checkout(&repo_dir, &name).unwrap();
            }
            other => panic!("{other:?}"),
        }
        assert!(checkout_branch("main", &repo_dir));
        app.open_branch_picker("app".into(), list_local_branches(&repo_dir));
        app.dispatch(Action::BranchChar('p'));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch { selected_name, .. } => {
                assert_eq!(selected_name, "feature/pick");
                assert!(checkout_branch(&selected_name, &repo_dir));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            crate::git::exec_git(&["branch", "--show-current"], &repo_dir),
            "feature/pick"
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn linked_snapshot() -> crate::snapshot::WorkspaceSnapshot {
        build_workspace_snapshot(
            &[
                repo("app", true),
                RepoSnapshot {
                    repo: "app/.worktrees/feat".into(),
                    branch: "feature/x".into(),
                    sync_status: SyncStatus::NoUpstream,
                    sync_note: String::new(),
                    head: String::new(),
                    has_unstaged: false,
                    has_staged: false,
                    has_untracked: false,
                    changes: Vec::new(),
                    checkout_kind: CheckoutKind::Linked,
                    primary_repo: Some("app".into()),
                    merged_into_default: Some(false),
                    default_branch_override: None,
                    default_tip_ref: None,
                    local_branches: Vec::new(),
                },
                repo("notes", true),
            ],
            &["notes".into()],
            false,
            &[],
        )
    }

    fn focus_checkout(app: &mut AppState, name: &str) {
        let idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Checkout && r.repo.as_deref() == Some(name))
            .expect("checkout row");
        app.cursor = idx;
    }

    #[test]
    fn remove_worktree_linked_only_confirm_cancel_and_apply() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        app.cursor = 0;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        assert!(app.confirm.is_none());
        focus_repo(&mut app, "app");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        let file_idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file");
        app.cursor = file_idx;
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert_eq!(app.status, "Focus a linked worktree to remove");
        focus_checkout(&mut app, "app/.worktrees/feat");
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        assert!(matches!(
            app.confirm,
            Some(PendingConfirm::RemoveWorktree { ref path, force, .. })
                if path == "app/.worktrees/feat" && !force
        ));
        app.dispatch(Action::ConfirmNo);
        assert!(app.confirm.is_none());
        assert!(app.status.contains("cancelled"));
        focus_checkout(&mut app, "app/.worktrees/feat");
        app.dispatch(Action::RemoveWorktree);
        match app.dispatch(Action::ConfirmYes) {
            Effect::RemoveWorktree {
                primary,
                path,
                force,
            } => {
                assert_eq!(primary, "app");
                assert_eq!(path, "app/.worktrees/feat");
                assert!(!force);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn remove_worktree_hidden_is_noop() {
        let snapshot = build_workspace_snapshot(
            &[RepoSnapshot {
                repo: "notes/.worktrees/feat".into(),
                branch: "feature/x".into(),
                sync_status: SyncStatus::NoUpstream,
                sync_note: String::new(),
                head: String::new(),
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Linked,
                primary_repo: Some("notes".into()),
                merged_into_default: None,
                default_branch_override: None,
                default_tip_ref: None,
                local_branches: Vec::new(),
            }],
            &["notes/.worktrees/feat".into()],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        assert!(app
            .rows
            .iter()
            .all(|r| r.repo.as_deref() != Some("notes/.worktrees/feat")));
        assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        app.dispatch(Action::ToggleShowHidden);
        if let Some(idx) = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes/.worktrees/feat"))
        {
            app.cursor = idx;
            app.show_hidden = false;
            assert_eq!(app.dispatch(Action::RemoveWorktree), Effect::None);
        }
    }

    /// Record a background fetch outcome in `app`'s store the way another
    /// `ws` instance in the same workspace does: claim, then finish.
    fn record_other_instance_fetch(app: &AppState, failed: bool, now: u64) -> u64 {
        crate::tui::fetch_claim::try_claim_background_fetch(
            &app.fetch_store_path,
            &workspace_store_id(&app.cwd),
            1,
            now,
        )
        .unwrap()
        .expect("the other instance claims")
        .finish(failed)
        .unwrap()
    }

    #[test]
    fn background_fetch_record_paints_fetch_failed_once_per_generation() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        assert!(!app.apply_background_fetch_record(), "no record yet");
        assert_eq!(record_other_instance_fetch(&app, true, 10), 1);
        assert!(app.apply_background_fetch_record());
        assert_eq!(app.status, "fetch failed");
        assert_eq!(app.status.kind(), StatusKind::Error);

        app.status = StatusMessage::error("push failed");
        assert!(
            !app.apply_background_fetch_record(),
            "the same generation is not applied again"
        );
        assert_eq!(app.status, "push failed", "a newer local status stays");
    }

    #[test]
    fn background_fetch_record_success_clears_only_its_own_stamp() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        record_other_instance_fetch(&app, true, 10);
        assert!(app.apply_background_fetch_record());
        assert_eq!(app.status, "fetch failed");
        record_other_instance_fetch(&app, false, 20);
        assert!(app.apply_background_fetch_record());
        assert_eq!(app.status, "", "a later success clears the shared failure");

        record_other_instance_fetch(&app, true, 30);
        assert!(app.apply_background_fetch_record());
        app.status = StatusMessage::error("push failed");
        record_other_instance_fetch(&app, false, 40);
        assert!(app.apply_background_fetch_record());
        assert_eq!(app.status, "push failed", "an unrelated status stays");

        record_other_instance_fetch(&app, true, 50);
        assert!(app.apply_background_fetch_record());
        app.status = StatusMessage::warn("fetch failed");
        record_other_instance_fetch(&app, false, 60);
        assert!(app.apply_background_fetch_record());
        assert_eq!(
            app.status.kind(),
            StatusKind::Warn,
            "same text, other kind: not the stamp"
        );
    }

    #[test]
    fn background_fetch_record_after_a_store_reset_applies() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        app.note_background_fetch_outcome(5, false);
        assert_eq!(record_other_instance_fetch(&app, true, 10), 1, "new store");
        assert!(app.apply_background_fetch_record());
        assert_eq!(app.status, "fetch failed");
        assert_eq!(app.status.kind(), StatusKind::Error);
        app.status = StatusMessage::error("push failed");
        assert!(
            !app.apply_background_fetch_record(),
            "the same generation is still not applied again"
        );
        assert_eq!(app.status, "push failed");
    }

    #[test]
    fn background_fetch_record_from_before_launch_paints_nothing() {
        let first = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        record_other_instance_fetch(&first, true, 10);
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        app.open_background_fetch_store(first.fetch_store_path.clone());
        assert!(!app.apply_background_fetch_record(), "seeded at launch");
        assert_eq!(app.status, "");
        record_other_instance_fetch(&first, true, 20);
        assert!(app.apply_background_fetch_record());
        assert_eq!(app.status, "fetch failed", "a later generation applies");
    }

    #[test]
    fn fetch_tick_includes_linked_worktrees_skips_hidden() {
        let mut app = AppState::new(PathBuf::from("/tmp"), linked_snapshot(), true);
        match app.dispatch(Action::FetchTick) {
            Effect::Fetch { repos } => {
                assert_eq!(repos, vec!["app", "app/.worktrees/feat"]);
                assert!(!repos.iter().any(|r| r == "notes"));
            }
            other => panic!("{other:?}"),
        }
        // The background tick leaves the status slot alone.
        assert_eq!(app.status, "");
        assert_eq!(app.dispatch(Action::WatchTick), Effect::WatchRefresh);
        assert_eq!(watch_interval_ms(Some("0"), None), 0);
        assert_ne!(
            crate::tui::fetch::fetch_interval_ms(Some("0"), None),
            watch_interval_ms(Some("0"), None) + 1
        );
        assert_eq!(crate::tui::fetch::fetch_interval_ms(Some("0"), None), 0);
        assert_eq!(
            watch_interval_ms(None, None),
            crate::tui::watch::DEFAULT_WATCH_MS
        );
        assert_eq!(
            crate::tui::fetch::fetch_interval_ms(None, None),
            crate::tui::fetch::DEFAULT_FETCH_MS
        );
        assert_ne!(
            crate::tui::fetch::DEFAULT_FETCH_MS,
            crate::tui::watch::DEFAULT_WATCH_MS
        );
    }

    fn unwritable_store_path(prefix: &str) -> (PathBuf, PathBuf) {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let blocker = root.join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        (root, blocker.join("store.json"))
    }

    #[test]
    fn submit_comment_names_save_failure_when_unwritable() {
        let mut app = state();
        focus_readme_diff(&mut app, two_line_readme());
        let (root, path) = unwritable_store_path("ws-comment-save-fail");
        app.comment_path = path;
        app.dispatch(Action::CommentStart);
        app.dispatch(Action::CommentInput(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('n'),
            crossterm::event::KeyModifiers::NONE,
        )));
        app.dispatch(Action::CommentSubmit);
        assert!(
            !app.status.contains("comment saved"),
            "failed persist must not claim success: {}",
            app.status
        );
        assert!(
            app.status.contains("comment save failed"),
            "failed persist must name the failure: {}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn toggle_reviewed_names_save_failure_when_unwritable() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        let (root, path) = unwritable_store_path("ws-viewed-save-fail");
        app.viewed_path = path;
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(
            app.reviewed.contains(&id),
            "session mark still flips when disk write fails"
        );
        assert!(
            !app.status.contains("comment saved"),
            "failed viewed persist must not claim comment success: {}",
            app.status
        );
        assert!(
            app.status.contains("viewed save failed"),
            "failed viewed persist must name the failure: {}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reconcile_comment_names_save_failure_when_unwritable() {
        let mut app = state();
        let (root, path) = unwritable_store_path("ws-comment-gc-fail");
        app.comment_path = path;
        app.comment_store = put_comment(
            &app.comment_store,
            CommentKey::Branch {
                repo: "app".into(),
                branch: "gone-branch".into(),
            },
            "stale-note",
        );
        app.apply_snapshot(app.snapshot.clone());
        assert!(
            app.status.contains("comment save failed"),
            "GC persist failure must name the failure: {}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn reconcile_viewed_names_save_failure_when_unwritable() {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "ws-viewed-gc-fail-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("ws");
        let repo_dir = workspace.join("app");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut app = AppState::new(workspace.clone(), snapshot.clone(), true);
        focus_file(&mut app, "README.md");
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        let (blocker_root, path) = unwritable_store_path("ws-viewed-gc-path");
        app.viewed_path = path;
        fs::write(repo_dir.join("README.md"), "# changed\n").unwrap();
        app.apply_snapshot(snapshot);
        assert!(
            app.status.contains("viewed save failed"),
            "reconcile persist failure must name the failure: {}",
            app.status
        );
        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&blocker_root);
    }

    #[test]
    fn reviewed_persists_and_drops_on_fingerprint_change() {
        use crate::tui::viewed::{load_viewed_store, viewed_identity, workspace_store_id};
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "ws-reviewed-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("ws");
        let repo_dir = workspace.join("app");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut app = AppState::new(workspace.clone(), snapshot.clone(), true);
        focus_file(&mut app, "README.md");
        let id = app.focused_row().unwrap().id.clone();
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        assert!(app.reviewed.contains(&id));
        let loaded = load_viewed_store(&app.viewed_path, &workspace_store_id(&app.cwd));
        assert!(loaded.contains_key(&viewed_identity("app", "README.md")));
        fs::write(repo_dir.join("README.md"), "# changed\n").unwrap();
        app.apply_snapshot(snapshot);
        assert!(!app.reviewed.contains(&id));
        assert!(load_viewed_store(&app.viewed_path, &workspace_store_id(&app.cwd)).is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    fn dual_ms_repo(name: &str) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: true,
            has_staged: true,
            has_untracked: false,
            changes: vec![FileChange {
                path: "both.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    fn focus_row_id(app: &mut AppState, id: &str) {
        let idx = app
            .rows
            .iter()
            .position(|r| r.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"));
        app.cursor = idx;
    }

    #[test]
    fn toggle_reviewed_on_dual_ms_rows_survives_reconcile() {
        use std::fs;
        let root = std::env::temp_dir().join(format!(
            "ws-reviewed-dual-ms-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("ws");
        let repo_dir = workspace.join("app");
        fs::create_dir_all(&repo_dir).unwrap();
        fs::write(repo_dir.join("both.rs"), "x\n").unwrap();
        let snapshot = build_workspace_snapshot(&[dual_ms_repo("app")], &[], false, &[]);
        let mut app = AppState::new(workspace, snapshot, true);

        focus_row_id(&mut app, "file:app:both.rs");
        let staged = app.focused_row().unwrap().file.clone().unwrap();
        assert_eq!(staged.staged_status.as_deref(), Some("M"));
        assert_eq!(staged.unstaged_status, None);
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        app.reconcile_viewed_store();
        assert!(
            app.reviewed.contains("file:app:both.rs"),
            "Staged dual id must survive reconcile_viewed_store"
        );
        assert!(
            app.reviewed.contains("file:app:both.rs#unstaged"),
            "Changes dual id must light after Space on Staged + reconcile"
        );

        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        app.reconcile_viewed_store();
        assert!(app.reviewed.is_empty());

        focus_row_id(&mut app, "file:app:both.rs#unstaged");
        let unstaged = app.focused_row().unwrap().file.clone().unwrap();
        assert_eq!(unstaged.staged_status, None);
        assert_eq!(unstaged.unstaged_status.as_deref(), Some("M"));
        assert_eq!(app.dispatch(Action::ToggleReviewed), Effect::None);
        app.reconcile_viewed_store();
        assert!(
            app.reviewed.contains("file:app:both.rs"),
            "Staged dual id must survive Space on #unstaged + reconcile"
        );
        assert!(
            app.reviewed.contains("file:app:both.rs#unstaged"),
            "file:…#unstaged must survive reconcile_viewed_store"
        );
        let _ = fs::remove_dir_all(&root);
    }

    fn graph_commit(id: &str, subject: &str) -> Commit {
        Commit {
            id: id.into(),
            subject: subject.into(),
            ..Commit::default()
        }
    }

    fn install_graph(app: &mut AppState, stashes: Vec<Stash>) {
        let model = GraphModel {
            commits: vec![graph_commit(
                "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                "head",
            )],
            stashes,
            worktrees: Vec::new(),
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            sync: None,
            show_hidden: app.show_hidden,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    #[test]
    fn enter_on_graph_commit_opens_files_then_diff_and_esc_pops() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        match app.dispatch(Action::NavEnter) {
            Effect::LoadCommitFiles { repo, source } => {
                assert_eq!(repo, "app");
                assert_eq!(
                    source,
                    CommitFileSource::Commit {
                        commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()
                    }
                );
            }
            other => panic!("{other:?}"),
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        assert!(app.drill.is_files());
        assert!(app.graph.is_some(), "graph stays in the model during files");
        let rows = app.commit_file_rows();
        assert!(rows.iter().any(|row| row.id == "dir:src"));
        let lib = rows
            .iter()
            .position(|row| row.path == "src/lib.rs")
            .expect("src/lib.rs row");
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = lib;
        }
        match app.dispatch(Action::NavEnter) {
            Effect::LoadCommitDiff { path, .. } => assert_eq!(path, "src/lib.rs"),
            other => panic!("{other:?}"),
        }
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
            0,
            "src/lib.rs".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        assert!(app.drill.is_diff());
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(app.list_focus_target(), ListFocusTarget::None);
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.drill.is_diff(), "right Esc unfocuses without popping");
        assert_eq!(app.focus, FocusPane::Left);
        assert!(app.commit_files_list_focused());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::DropCommitDiff);
        assert!(app.drill.is_files());
        assert_eq!(app.focus, FocusPane::Left);
        assert_eq!(app.list_focus_target(), ListFocusTarget::Graph);
        assert!(app.graph.is_some());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::LoadRightPane);
        assert!(app.drill.is_graph());
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn esc_on_right_at_files_unfocuses_without_popping() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
        );
        assert_eq!(app.focus, FocusPane::Right);
        assert!(app.drill.is_files());
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert!(app.drill.is_files());
        assert_eq!(app.focus, FocusPane::Left);
        assert_eq!(app.list_focus_target(), ListFocusTarget::Graph);
    }

    #[test]
    fn j_k_on_depth_2_left_moves_commit_files_not_graph() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        let files = vec![
            CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            },
            CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            },
        ];
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files.clone(),
        );
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files,
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        assert_eq!(app.dispatch(Action::NavEsc), Effect::None);
        assert_eq!(app.focus, FocusPane::Left);
        assert!(app.drill.is_diff());
        let graph_before = app.graph_cursor;
        let rows = app.commit_file_rows();
        let lib = rows
            .iter()
            .position(|row| row.path == "src/lib.rs")
            .expect("src/lib.rs row");
        assert!(lib > 0, "src/lib.rs should not be the first row: {rows:?}");
        if let DrillView::Diff { file_cursor, .. } = &mut app.drill {
            *file_cursor = lib - 1;
        }
        match app.dispatch(Action::Move(1)) {
            Effect::LoadRightPane => {}
            other => panic!("expected LoadRightPane, got {other:?}"),
        }
        assert_eq!(app.graph_cursor, graph_before);
        assert_eq!(app.commit_files_cursor(), lib);
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn j_k_on_depth_1_left_still_moves_graph() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        if let Some(model) = app.graph.as_mut() {
            model.commits.push(graph_commit(
                "bbb2222ccccccccccccccccccccccccccccccccc",
                "other",
            ));
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
        );
        app.focus = FocusPane::Left;
        let files_before = app.commit_files_cursor();
        let graph_before = app.graph_cursor;
        assert_eq!(app.dispatch(Action::Move(1)), Effect::LoadRightPane);
        assert_eq!(app.commit_files_cursor(), files_before);
        assert_ne!(app.graph_cursor, graph_before);
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn click_on_commit_files_footer_does_not_select_a_hidden_file() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        let files: Vec<CommitFile> = ["a.txt", "b.txt", "c.txt", "d.txt"]
            .into_iter()
            .map(|path| CommitFile {
                status: "M".into(),
                path: path.into(),
                old_path: None,
                stat: None,
            })
            .collect();
        let source = CommitFileSource::Commit {
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        };
        app.open_commit_files("app".into(), source.clone(), files.clone());
        // Depth 2: the file list and its commit footer sit in the left pane.
        app.open_commit_diff(
            "app".into(),
            source,
            files,
            0,
            "a.txt".into(),
            DiffContent::from_lines(vec!["+a".into()]),
        );
        assert!(app.drill.is_diff());
        assert_eq!(app.commit_file_rows().len(), 4);
        app.layout = wide_split_layout();
        // Two list rows scrolled by one (b.txt, c.txt); the footer is under
        // them, where a scrolled-off d.txt would otherwise map.
        app.layout.files_list_y = 2;
        app.layout.files_list_height = 2;
        app.layout.files_list_offset = 1;
        app.set_commit_file_cursor(1);
        app.focus = FocusPane::Left;
        let col = 10;
        assert!(col < app.layout.right_x, "click lands in the left pane");

        let footer_row = app.layout.files_list_y + app.layout.files_list_height;
        assert_eq!(
            app.dispatch(Action::Click {
                col,
                row: footer_row
            }),
            Effect::None
        );
        assert_eq!(app.commit_files_cursor(), 1);
        app.dispatch(Action::Release);

        app.dispatch(Action::Click {
            col,
            row: app.layout.files_list_y + 1,
        });
        assert_eq!(app.commit_files_cursor(), 2, "list rows still map");
    }

    /// The diff search memo answers repeat calls and follows a content or
    /// query change instead of serving stale hits.
    #[test]
    fn diff_search_hits_follow_content_and_query() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec![
                "@@ -1,2 +1,2 @@".into(),
                "-old needle".into(),
                "+new needle".into(),
            ]),
        );
        let first = app.diff_search_hits("Needle");
        assert_eq!(first.len(), 2, "{first:?}");
        assert_eq!(app.diff_search_hits("needle "), first, "memo hit");
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,1 @@".into(), "+needle".into()]),
        );
        assert_eq!(app.diff_search_hits("needle").len(), 1, "new content");
        assert!(app.diff_search_hits("absent").is_empty(), "new query");
        assert!(app.diff_search_hits("  ").is_empty());
    }

    /// The graph pan clamp reads the painted line count the last frame
    /// recorded instead of repainting the model.
    #[test]
    fn graph_vscroll_shown_reads_the_recorded_line_count() {
        let mut app = state();
        focus_repo(&mut app, "app");
        assert!(!app.graph_vscroll_shown(), "no graph");
        install_graph(&mut app, Vec::new());
        let list = usize::from(app.graph_chrome().list_height);
        app.layout.graph_content_len = list;
        assert!(!app.graph_vscroll_shown(), "fits");
        app.layout.graph_content_len = list + 1;
        assert!(app.graph_vscroll_shown(), "overflows at the top");
        app.layout.graph_content_len = 0;
        app.graph_scroll = 1;
        assert!(app.graph_vscroll_shown(), "left the top");
    }

    /// `h` / `l` on a commit-file folder row fold / unfold it like the
    /// workspace tree; on a file row they still pan.
    #[test]
    fn h_l_fold_commit_file_dirs_and_pan_on_files() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        app.focus = FocusPane::Right;
        let key = |c| Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        let at = |app: &mut AppState, id: &str| {
            let idx = app
                .commit_file_rows()
                .iter()
                .position(|row| row.id == id)
                .expect(id);
            if let DrillView::Files { cursor, .. } = &mut app.drill {
                *cursor = idx;
            }
        };
        at(&mut app, "dir:src");
        assert!(app.hl_folds());
        let close = super::super::app::map_event(&app, &key('h'));
        assert_eq!(close, Action::FoldClose);
        app.dispatch(close);
        assert!(app.commit_file_folds.contains("dir:src"));
        let open = super::super::app::map_event(&app, &key('l'));
        assert_eq!(open, Action::FoldOpen);
        app.dispatch(open);
        assert!(!app.commit_file_folds.contains("dir:src"));

        let file = app
            .commit_file_rows()
            .iter()
            .find(|row| row.is_file())
            .map(|row| row.id.clone())
            .expect("file row");
        at(&mut app, &file);
        assert!(!app.hl_folds());
        assert_eq!(
            super::super::app::map_event(&app, &key('h')),
            Action::PanDiff(-1)
        );
    }

    #[test]
    fn fold_on_depth_2_left_folds_commit_files() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        let files = vec![
            CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            },
            CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            },
        ];
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files,
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        app.focus = FocusPane::Left;
        let dir = app
            .commit_file_rows()
            .iter()
            .position(|row| row.id == "dir:src")
            .expect("dir:src");
        if let DrillView::Diff { file_cursor, .. } = &mut app.drill {
            *file_cursor = dir;
        }
        assert!(app
            .commit_file_rows()
            .iter()
            .any(|row| row.path == "src/lib.rs"));
        app.dispatch(Action::FoldToggle);
        assert!(
            !app.commit_file_rows()
                .iter()
                .any(|row| row.path == "src/lib.rs"),
            "folding dir:src should hide src/lib.rs"
        );
    }

    #[test]
    fn hidden_is_not_drilled_unless_shown() {
        let mut app = state();
        assert!(app.rows.iter().all(|r| !r.label.contains("notes")));
        app.cursor = 0;
        app.focus = FocusPane::Right;
        install_graph(&mut app, Vec::new());
        // tree still on workspace / app, not hidden notes
        app.dispatch(Action::ToggleShowHidden);
        let notes = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes"))
            .expect("notes");
        app.cursor = notes;
        app.show_hidden = false;
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::NavEnter), Effect::None);
        assert!(app.status.contains("stay out of drill"));
        app.show_hidden = true;
        app.rebuild_rows();
        let notes = app
            .rows
            .iter()
            .position(|r| r.repo.as_deref() == Some("notes"))
            .expect("notes shown");
        app.cursor = notes;
        install_graph(&mut app, Vec::new());
        match app.dispatch(Action::NavEnter) {
            Effect::LoadCommitFiles { repo, .. } => assert_eq!(repo, "notes"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn graph_stash_row_apply_pop_drop_target_that_ref() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![
                Stash {
                    stash_ref: "stash@{0}".into(),
                    subject: "latest".into(),
                    parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                    ..Stash::default()
                },
                Stash {
                    stash_ref: "stash@{1}".into(),
                    subject: "older".into(),
                    parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                    ..Stash::default()
                },
            ],
        );
        let rows = app.graph.as_ref().unwrap().visible_rows();
        let idx = rows
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{1}"))
            .expect("stash@{1}");
        app.graph_cursor = idx;
        match app.dispatch(Action::GraphStashApply) {
            Effect::StashApply { stash_ref, repo } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{1}");
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.dispatch(Action::GraphStashDrop), Effect::None);
        assert!(app.status.contains("stash@{1}"));
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.status.contains("drop cancelled"));
        app.dispatch(Action::GraphStashDrop);
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{1}"),
            other => panic!("{other:?}"),
        }
        match app.dispatch(Action::GraphStashPop) {
            Effect::StashPop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{1}"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn stash_menu_from_repo_or_file_is_create_only() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Left;
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create]
        );
        assert!(ops.iter().all(|op| op.stash_ref.is_none()));
        app.stash_menu = None;
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                stash_ref: "stash@{1}".into(),
                subject: "older".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(|r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{1}"))
            .expect("stash");
        app.graph_cursor = idx;
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("graph menu");
        assert!(ops.iter().any(|op| op.id == StashOpId::Drop));
        assert!(
            ops.iter()
                .any(|op| op.stash_ref.as_deref() == Some("stash@{1}")),
            "{ops:?}"
        );
        assert!(ops
            .iter()
            .all(|op| { op.stash_ref.is_none() || op.stash_ref.as_deref() == Some("stash@{1}") }));
    }

    fn focus_graph_row(app: &mut AppState, pred: impl Fn(&GraphRow) -> bool) {
        let idx = app
            .graph
            .as_ref()
            .unwrap()
            .visible_rows()
            .iter()
            .position(pred)
            .expect("graph row");
        app.graph_cursor = idx;
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    const PARENT_TITLE: &str = "Diff commit vs parent in new tab";
    const CHILD_SHA: &str = "ccc2222ddddddddddddddddddddddddddddddddd";
    const ROOT_SHA: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    /// `app` graph: uncommitted, a child commit, a stash, and a root commit.
    fn install_parent_graph(app: &mut AppState) {
        let model = GraphModel {
            commits: vec![
                Commit {
                    id: CHILD_SHA.into(),
                    subject: "child".into(),
                    parents: vec![ROOT_SHA.into()],
                    ..Commit::default()
                },
                graph_commit(ROOT_SHA, "root"),
            ],
            stashes: vec![graph_stash("stash@{0}", "wip")],
            head_id: Some(CHILD_SHA.into()),
            show_hidden: app.show_hidden,
            uncommitted: Some(true),
            ..GraphModel::default()
        };
        app.set_graph(model, "app".into(), CHILD_SHA.into());
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    fn focus_graph_commit(app: &mut AppState, id: &str) {
        focus_graph_row(
            app,
            |r| matches!(r, GraphRow::Commit { commit, .. } if commit.id == id),
        );
    }

    /// Palette row and dispatch give `reason` and open no tab.
    fn assert_parent_compare_refused(app: &mut AppState, reason: &str) {
        assert_eq!(
            palette_reason(app, PARENT_TITLE).as_deref(),
            Some(reason),
            "palette"
        );
        let tabs = app.tabs.compare_count();
        app.status.clear();
        assert_eq!(app.dispatch(Action::CompareCommitVsParent), Effect::None);
        assert_eq!(app.status, reason, "dispatch");
        assert_eq!(app.tabs.compare_count(), tabs, "no tab opens");
    }

    #[test]
    fn commit_vs_parent_opens_a_pinned_tab_on_the_graph_repo() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_parent_graph(&mut app);
        focus_graph_commit(&mut app, CHILD_SHA);
        assert_eq!(palette_reason(&app, PARENT_TITLE), None);
        let tab_id = match app.dispatch(Action::CompareCommitVsParent) {
            Effect::LoadCompareRange {
                tab_id,
                repo,
                base_ref,
                head_ref,
                force,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(base_ref, format!("{CHILD_SHA}^"));
                assert_eq!(head_ref, CHILD_SHA);
                assert!(force);
                tab_id
            }
            other => panic!("{other:?}"),
        };
        assert!(app.is_compare_tab());
        let tab = app.tabs.active_compare().unwrap();
        assert_eq!(tab.id, tab_id);
        assert!(tab.is_pinned());
        assert_eq!(tab.label(), "app ↔ ccc2222^");
        assert_eq!(tab.range_header(), "ccc2222^...ccc2222");
        assert_eq!(
            app.commit_detail_meta().1.as_deref(),
            Some("ccc2222^...ccc2222")
        );
        // Probes and reloads keep the pinned head.
        match app.compare_probe_effects().as_slice() {
            [] => {}
            other => panic!("still loading, no probe: {other:?}"),
        }
        app.tabs.active_compare_mut().unwrap().loading = false;
        match app.compare_probe_effects().as_slice() {
            [Effect::ProbeCompareTab { head_ref, .. }] => assert_eq!(head_ref, CHILD_SHA),
            other => panic!("{other:?}"),
        }
        match app.apply_compare_probe(tab_id, true, None) {
            Some(Effect::LoadCompareRange { head_ref, .. }) => assert_eq!(head_ref, CHILD_SHA),
            other => panic!("{other:?}"),
        }

        // On the compare tab no graph commit is focused.
        assert_parent_compare_refused(&mut app, FOCUS_A_COMMIT_TO_DIFF);

        // Back on Workspace the same commit focuses the existing tab.
        app.dispatch(Action::JumpToTab(1));
        focus_graph_commit(&mut app, CHILD_SHA);
        assert_eq!(app.dispatch(Action::CompareCommitVsParent), Effect::None);
        assert_eq!(app.tabs.compare_count(), 1);
        assert_eq!(app.tabs.active, 1);
    }

    #[test]
    fn commit_vs_parent_refuses_off_a_parented_graph_commit() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_parent_graph(&mut app);
        focus_graph_commit(&mut app, ROOT_SHA);
        assert_parent_compare_refused(&mut app, ROOT_COMMIT_HAS_NO_PARENT);
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Uncommitted { .. }));
        assert_parent_compare_refused(&mut app, FOCUS_A_COMMIT_TO_DIFF);
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Stash(_)));
        assert_parent_compare_refused(&mut app, FOCUS_A_COMMIT_TO_DIFF);
        focus_graph_commit(&mut app, CHILD_SHA);
        app.focus = FocusPane::Left;
        assert_parent_compare_refused(&mut app, FOCUS_A_COMMIT_TO_DIFF);
    }

    #[test]
    fn compare_open_refusals_match_on_palette_and_dispatch() {
        use super::super::tabs::{DEFAULT_BRANCH_NOT_FOUND, FOCUS_A_CHECKOUT, HEAD_HAS_NO_COMMIT};
        let mut app = state();
        let check = |app: &mut AppState, title: &str, action: Action, reason: &str| {
            assert_eq!(
                palette_reason(app, title).as_deref(),
                Some(reason),
                "{title}"
            );
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(app.status, reason, "{action:?}");
            assert!(app.tabs.is_workspace());
            assert!(app.compare_picker_pending.is_none());
        };
        app.cursor = 0;
        for (title, action) in [
            ("Diff vs default in new tab", Action::CompareVsDefault),
            ("Diff vs branch in new tab…", Action::CompareVsBranch),
            ("Diff vs commit in new tab…", Action::CompareVsCommit),
        ] {
            check(&mut app, title, action, FOCUS_A_CHECKOUT);
        }
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Left;
        for (title, action) in [
            ("Diff vs default in new tab", Action::CompareVsDefault),
            ("Diff vs branch in new tab…", Action::CompareVsBranch),
            ("Diff vs commit in new tab…", Action::CompareVsCommit),
        ] {
            check(&mut app, title, action, HEAD_HAS_NO_COMMIT);
        }
        for row in app.snapshot.repos.iter_mut().filter(|r| r.repo == "app") {
            row.head = "ccc".into();
        }
        check(
            &mut app,
            "Diff vs default in new tab",
            Action::CompareVsDefault,
            DEFAULT_BRANCH_NOT_FOUND,
        );
        assert_eq!(palette_reason(&app, "Diff vs branch in new tab…"), None);
        assert_eq!(
            app.dispatch(Action::CompareVsBranch),
            Effect::PrepareComparePicker {
                repo: "app".into(),
                kind: ComparePickerKind::Branch,
            }
        );
        assert_eq!(palette_reason(&app, "Diff vs commit in new tab…"), None);
        assert_eq!(
            app.dispatch(Action::CompareVsCommit),
            Effect::PrepareComparePicker {
                repo: "app".into(),
                kind: ComparePickerKind::Commit,
            }
        );
        assert_eq!(app.compare_picker_pending.as_deref(), Some("app"));
    }

    #[test]
    fn pinned_compare_tab_refuses_revert() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_parent_graph(&mut app);
        focus_graph_commit(&mut app, CHILD_SHA);
        app.dispatch(Action::CompareCommitVsParent);
        {
            let tab = app.tabs.active_compare_mut().unwrap();
            tab.loading = false;
            tab.source = Some(CommitFileSource::Compare {
                base_ref: format!("{CHILD_SHA}^"),
                head_ref: CHILD_SHA.into(),
                base_tip: ROOT_SHA.into(),
                merge_base: ROOT_SHA.into(),
                head: CHILD_SHA.into(),
            });
            tab.files = vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }];
        }
        for row in app.snapshot.repos.iter_mut().filter(|r| r.repo == "app") {
            row.head = CHILD_SHA.into();
            row.changes.clear();
        }
        assert_eq!(
            app.compare_revert_target().err().as_deref(),
            Some(CANNOT_REVERT_COMMITTED_DIFF)
        );
        assert_eq!(
            app.compare_file_revert_refusal().as_deref(),
            Some(CANNOT_REVERT_COMMITTED_DIFF)
        );
        assert_eq!(
            palette_reason(&app, "Revert").as_deref(),
            Some(CANNOT_REVERT_COMMITTED_DIFF)
        );
        app.status.clear();
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert_eq!(app.status, CANNOT_REVERT_COMMITTED_DIFF);
        assert!(app.confirm.is_none());
    }

    fn graph_stash(stash_ref: &str, subject: &str) -> Stash {
        Stash {
            stash_ref: stash_ref.into(),
            subject: subject.into(),
            parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            ..Stash::default()
        }
    }

    #[test]
    fn graph_stash_menu_on_commit_offers_latest_apply_pop_not_drop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Commit { .. }));
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("graph commit menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create, StashOpId::Apply, StashOpId::Pop]
        );
        assert!(ops
            .iter()
            .all(|op| { op.stash_ref.is_none() || op.stash_ref.as_deref() == Some("stash@{0}") }));
        assert!(!ops.iter().any(|op| op.id == StashOpId::Drop));

        match app.dispatch(Action::StashMenuChar('p')) {
            Effect::StashPop { repo, stash_ref } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert!(app.stash_menu.is_none());

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('d')), Effect::None);
        assert!(app.confirm.is_none());
    }

    #[test]
    fn graph_stash_menu_dispatch_from_right_focused_graph_is_noop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Commit { .. }));
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(app.dispatch(Action::StashMenu), Effect::None);
        assert_eq!(app.status, "focus the left pane (Tab) to stash");
    }

    #[test]
    fn graph_stash_menu_on_uncommitted_offers_latest_apply_pop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        if let Some(graph) = app.graph.as_mut() {
            graph.uncommitted = Some(true);
        }
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Uncommitted { .. }));
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("uncommitted menu");
        assert_eq!(
            ops.iter().map(|op| op.id).collect::<Vec<_>>(),
            vec![StashOpId::Create, StashOpId::Apply, StashOpId::Pop]
        );
        assert!(!ops.iter().any(|op| op.id == StashOpId::Drop));
        assert!(ops
            .iter()
            .filter(|op| op.id != StashOpId::Create)
            .all(|op| op.stash_ref.as_deref() == Some("stash@{0}")));
    }

    #[test]
    fn stash_menu_pop_runs_immediately_drop_still_confirms() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        focus_graph_row(
            &mut app,
            |r| matches!(r, GraphRow::Stash(s) if s.stash_ref == "stash@{0}"),
        );
        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        let ops = app.stash_menu.clone().expect("stash row menu");
        assert!(ops.iter().any(|op| op.id == StashOpId::Drop));
        match app.dispatch(Action::StashMenuChar('p')) {
            Effect::StashPop { repo, stash_ref } => {
                assert_eq!(repo, "app");
                assert_eq!(stash_ref, "stash@{0}");
            }
            other => panic!("{other:?}"),
        }
        assert!(app.confirm.is_none());
        assert!(app.stash_menu.is_none());

        app.open_stash_menu("app".into(), Some("stash@{0}".into()));
        assert_eq!(app.dispatch(Action::StashMenuChar('d')), Effect::None);
        assert!(app.confirm.is_some());
        match app.dispatch(Action::ConfirmYes) {
            Effect::StashDrop { stash_ref, .. } => assert_eq!(stash_ref, "stash@{0}"),
            other => panic!("{other:?}"),
        }
    }

    fn wide_split_layout() -> super::LayoutHit {
        super::LayoutHit {
            term_cols: 160,
            pane_height: 22,
            outer_tree_width: 48,
            right_x: 48,
            diff_pane_width: 110,
            diff_content_x: 50,
            diff_split_rule_x: Some(diff_split_rule_x(50, 110, DIFF_SPLIT_FRACTION)),
            ..super::LayoutHit::default()
        }
    }

    #[test]
    fn pane_drag_updates_ratio_and_release_ends_drag() {
        let mut app = state();
        app.layout = wide_split_layout();
        let start = pane_widths(160, app.tree_fraction).tree_width;
        assert_eq!(
            app.dispatch(Action::Click { col: 47, row: 5 }),
            Effect::None
        );
        assert_eq!(app.drag, SplitDrag::Pane);
        assert_eq!(app.dispatch(Action::Drag { col: 70, row: 8 }), Effect::None);
        let after = pane_widths(160, app.tree_fraction).tree_width;
        assert!(after > start, "start={start} after={after}");
        assert_eq!(after, 71);
        assert_eq!(app.drag, SplitDrag::Pane);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(pane_widths(160, app.tree_fraction).tree_width, after);
    }

    #[test]
    fn in_diff_drag_updates_ratio_and_release_ends_drag() {
        let mut app = state();
        app.layout = wide_split_layout();
        let rule = app.layout.diff_split_rule_x.expect("rule");
        let start = side_by_side_column_widths(110, app.diff_split_fraction).left_width;
        assert_eq!(
            app.dispatch(Action::Click { col: rule, row: 6 }),
            Effect::None
        );
        assert_eq!(app.drag, SplitDrag::Diff);
        assert_eq!(
            app.dispatch(Action::Drag {
                col: rule + 20,
                row: 6
            }),
            Effect::None
        );
        let after = side_by_side_column_widths(110, app.diff_split_fraction).left_width;
        assert!(after > start, "start={start} after={after}");
        assert_eq!(app.drag, SplitDrag::Diff);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(
            side_by_side_column_widths(110, app.diff_split_fraction).left_width,
            after
        );
    }

    /// [`wide_split_layout`] as each mode's render records it. Both modes
    /// start the right pane's content one column in (48 + 1): after its
    /// left border when boxed, after its left pad cell when flat.
    fn split_layout_for(background: BackgroundMode) -> super::LayoutHit {
        let content_x = match background {
            BackgroundMode::Terminal | BackgroundMode::Paint => 49,
        };
        super::LayoutHit {
            diff_content_x: content_x,
            diff_split_rule_x: Some(diff_split_rule_x(content_x, 110, DIFF_SPLIT_FRACTION)),
            ..wide_split_layout()
        }
    }

    #[test]
    fn in_diff_drag_from_the_painted_rule_keeps_then_moves_the_split() {
        // The rule splits the painted row: the pane less the scrollbar
        // column and the cursor bar.
        let row = diff_split_row_width(110);
        for background in [BackgroundMode::Terminal, BackgroundMode::Paint] {
            let mut app = state();
            app.background = background;
            app.layout = split_layout_for(background);
            let rule = app.layout.diff_split_rule_x.expect("rule");
            let content_x = app.layout.diff_content_x;
            let left =
                i32::from(side_by_side_column_widths(row, app.diff_split_fraction).left_width);
            app.dispatch(Action::Click { col: rule, row: 6 });
            assert_eq!(app.drag, SplitDrag::Diff, "{background:?}");
            assert_eq!(
                app.diff_split_fraction,
                diff_split_fraction_from_left_width(row, left),
                "{background:?}: a press on the painted rule keeps the split"
            );
            assert_eq!(
                diff_split_rule_x(content_x, 110, app.diff_split_fraction),
                rule,
                "{background:?}"
            );
            app.dispatch(Action::Drag {
                col: rule + 6,
                row: 6,
            });
            assert_eq!(
                app.diff_split_fraction,
                diff_split_fraction_from_left_width(row, left + 6),
                "{background:?}"
            );
            assert_eq!(
                diff_split_rule_x(content_x, 110, app.diff_split_fraction),
                rule + 6,
                "{background:?}: the rule follows the pointer"
            );
        }
    }

    #[test]
    fn resize_edge_hover_follows_the_pointer_and_redraws_only_on_change() {
        let mut app = state();
        app.layout = wide_split_layout();
        let rule = app.layout.diff_split_rule_x.expect("rule");
        assert!(!app.set_pointer(Some((10, 5))), "tree body: no edge");
        assert_eq!(app.hovered_resize_edge(), None);
        assert!(app.set_pointer(Some((47, 5))), "onto the pane divider");
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));
        assert!(!app.set_pointer(Some((46, 6))), "same edge: no redraw");
        assert!(!app.set_pointer(Some((48, 7))), "same edge: no redraw");
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));
        assert!(app.set_pointer(Some((rule, 5))), "onto the diff rule");
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::DiffSplit));
        assert!(
            !app.set_pointer(Some((rule + 1, 9))),
            "same edge: no redraw"
        );
        assert!(app.set_pointer(Some((rule + 2, 9))), "off the rule");
        assert_eq!(app.hovered_resize_edge(), None);
        assert!(app.set_pointer(Some((47, 5))));
        assert!(app.set_pointer(None), "pointer gone: edge gone");
        assert_eq!(app.hovered_resize_edge(), None);
        // The status row under the panes is no edge.
        assert!(!app.set_pointer(Some((47, 23))));
        assert_eq!(app.hovered_resize_edge(), None);
    }

    #[test]
    fn scrollbar_hits_are_not_resize_edges() {
        let mut app = state();
        app.layout = super::LayoutHit {
            // A left-pane graph scrollbar on the divider band's first column.
            graph_scrollbar_x: Some(46),
            graph_scrollbar_y: 2,
            graph_scrollbar_height: 10,
            graph_content_len: 40,
            // A diff h-bar track starting on the divider band's last column.
            diff_hscrollbar_y: Some(20),
            diff_hscrollbar_x: 48,
            diff_hscrollbar_width: 100,
            diff_col_max: 40,
            ..wide_split_layout()
        };
        for (col, row) in [(46, 2), (46, 11), (48, 20), (60, 20)] {
            app.set_pointer(Some((col, row)));
            assert_eq!(app.hovered_resize_edge(), None, "({col}, {row})");
        }
        // The same column under the scrollbar track is the divider again.
        app.set_pointer(Some((46, 15)));
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));
    }

    #[test]
    fn resize_edge_hover_clears_under_overlays_mouse_off_and_explorer() {
        use crate::file_search::SearchOptions;
        use crate::tui::quick_open::QuickOpenScope;
        let mut app = state();
        app.layout = wide_split_layout();
        app.set_pointer(Some((47, 5)));
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));

        app.help_open = true;
        assert_eq!(app.hovered_resize_edge(), None, "help");
        app.help_open = false;
        app.search_files = Some(SearchFilesState::new(
            QuickOpenScope::Workspace,
            String::new(),
            SearchOptions::default(),
        ));
        assert_eq!(app.hovered_resize_edge(), None, "search-in-files dialog");
        app.search_files = None;
        app.too_small = true;
        assert_eq!(app.hovered_resize_edge(), None, "resize notice");
        app.too_small = false;
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));

        app.dispatch(Action::ToggleMouse);
        assert!(!app.mouse_enabled);
        assert_eq!(app.hovered_resize_edge(), None, "mouse off");
        assert!(!app.set_pointer(Some((47, 5))), "mouse off ignores motion");
        assert_eq!(app.hovered_resize_edge(), None);
        app.dispatch(Action::ToggleMouse);
        app.set_pointer(Some((47, 5)));
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));

        app.open_explorer_tab("app".into(), super::explorer_tab::ExplorerTarget::Root);
        assert!(app.is_explorer_tab());
        app.layout = wide_split_layout();
        assert_eq!(
            app.hovered_resize_edge(),
            None,
            "Explorer has no pane or rule drag"
        );
    }

    #[test]
    fn resize_edge_stays_lit_while_dragging_and_ends_on_release() {
        let mut app = state();
        app.layout = wide_split_layout();
        let rule = app.layout.diff_split_rule_x.expect("rule");
        app.set_pointer(Some((rule, 6)));
        app.dispatch(Action::Click { col: rule, row: 6 });
        assert_eq!(app.drag, SplitDrag::Diff);
        // Far past the clamp: the rule stops, the pointer leaves it.
        app.dispatch(Action::Drag { col: 159, row: 6 });
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::DiffSplit));
        app.dispatch(Action::Release);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(app.hovered_resize_edge(), None, "release ends it");

        app.set_pointer(Some((47, 6)));
        app.dispatch(Action::Click { col: 47, row: 6 });
        assert_eq!(app.drag, SplitDrag::Pane);
        // Over the diff rule mid-drag: the dragged edge wins.
        app.dispatch(Action::Drag { col: rule, row: 6 });
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));
        // Back onto the divider band, then release: still on the edge.
        app.dispatch(Action::Drag { col: 46, row: 6 });
        app.dispatch(Action::Release);
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));
    }

    /// Row of the commit-message footer rule in [`commit_msg_layout`].
    const MSG_RULE_Y: u16 = 15;

    /// [`wide_split_layout`] with a right-pane graph whose expanded
    /// footer rule sits on [`MSG_RULE_Y`], in a pane `pane_height` rows
    /// tall.
    fn commit_msg_layout(pane_height: u16) -> super::LayoutHit {
        super::LayoutHit {
            tree_height: pane_height,
            diff_split_rule_x: None,
            commit_msg_rule_y: Some(MSG_RULE_Y),
            commit_msg_rule_x: 50,
            commit_msg_rule_width: 109,
            commit_msg_lines_fit: workspace_status_graph::commit_msg_lines_fit(pane_height),
            ..wide_split_layout()
        }
    }

    /// A right-pane graph of 40 commits with its footer rule recorded and
    /// the UI-state store in a fresh temp dir.
    fn commit_msg_drag_state(prefix: &str, pane_height: u16) -> (AppState, PathBuf) {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 40);
        let dir = crate::testutil::unique_dir(prefix);
        app.ui_store_path = dir.join("state").join("ui-state.json");
        app.layout = commit_msg_layout(pane_height);
        (app, dir)
    }

    #[test]
    fn commit_msg_lines_from_drag_moves_one_row_per_row_and_clamps() {
        // Up adds, down removes.
        assert_eq!(commit_msg_lines_from_drag(4, 15, 13, 20), 6);
        assert_eq!(commit_msg_lines_from_drag(4, 15, 17, 20), 2);
        assert_eq!(commit_msg_lines_from_drag(4, 15, 0, 20), 19);
        // Clamped to 1..=20 and to the pane fit.
        assert_eq!(commit_msg_lines_from_drag(4, 15, 40, 20), 1);
        assert_eq!(commit_msg_lines_from_drag(19, 15, 0, 20), 20);
        assert_eq!(commit_msg_lines_from_drag(4, 15, 0, 6), 6);
        assert_eq!(commit_msg_lines_from_drag(4, 15, 0, 0), 1, "fit under 1");
        // More rows than the pane shows: the first row moved shows.
        assert_eq!(commit_msg_lines_from_drag(20, 15, 16, 6), 5);
        assert_eq!(commit_msg_lines_from_drag(20, 15, 14, 6), 6);
        // Back on the press row: the rows at the press, even past the fit.
        assert_eq!(commit_msg_lines_from_drag(20, 15, 15, 6), 20);
        assert_eq!(commit_msg_lines_from_drag(4, 15, 15, 20), 4);
    }

    /// Press on the rule arms the drag and changes nothing; up adds rows,
    /// down removes them, the focused row stays in view, and the release
    /// saves the new rows to the UI-state store once.
    #[test]
    fn commit_msg_rule_drag_resizes_the_footer_and_release_saves_once() {
        let (mut app, dir) = commit_msg_drag_state("ws-msg-drag-save", 40);
        app.dispatch(Action::MoveToEnd);
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT);
        let status = app.status.clone();
        assert_eq!(
            app.dispatch(Action::Click {
                col: 60,
                row: MSG_RULE_Y
            }),
            Effect::None
        );
        assert_eq!(
            app.drag,
            SplitDrag::CommitMsg {
                origin_row: MSG_RULE_Y,
                origin_lines: COMMIT_MSG_LINES_DEFAULT,
            }
        );
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT);
        assert_eq!(app.status, status, "a press alone changes nothing");
        // Along the press row: no change.
        app.dispatch(Action::Drag {
            col: 90,
            row: MSG_RULE_Y,
        });
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT);

        let list_before = app.graph_chrome().list_height;
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y - 3,
        });
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT + 3);
        assert_eq!(app.status, "msg lines 7");
        assert_eq!(app.graph_chrome().list_height + 3, list_before);
        let idx = painted_focus_index(&app);
        let scroll = app.graph_scroll as usize;
        let list_h = app.graph_chrome().list_height as usize;
        assert!(
            idx >= scroll && idx < scroll + list_h,
            "focused painted row {idx} in view at {scroll}..{}",
            scroll + list_h
        );
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y + 1,
        });
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT - 1);
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y - 2,
        });
        assert_eq!(app.commit_msg_lines, 6);
        assert!(!app.ui_store_path.exists(), "nothing saved mid-drag");

        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(app.commit_msg_lines, 6);
        assert_eq!(load_ui_state(&app.ui_store_path).commit_msg_lines, Some(6));
        let saved = std::fs::read(&app.ui_store_path).unwrap();

        // A press and release, and a drag back to its start, write nothing.
        std::fs::remove_file(&app.ui_store_path).unwrap();
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Release);
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y - 4,
        });
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Release);
        assert_eq!(app.commit_msg_lines, 6);
        assert!(!app.ui_store_path.exists(), "no change, no save");
        assert!(saved.ends_with(b"\"commitMessageLines\": 6\n}\n"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The drag stops at 1 and 20 rows, and at the rows a small pane
    /// shows, so the list keeps the rows the footer budget gives it.
    #[test]
    fn commit_msg_rule_drag_clamps_to_the_range_and_the_pane_fit() {
        let (mut app, dir) = commit_msg_drag_state("ws-msg-drag-clamp", 60);
        assert_eq!(app.layout.commit_msg_lines_fit, COMMIT_MSG_LINES_MAX);
        app.commit_msg_lines = 10;
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Drag { col: 60, row: 0 });
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_MAX, "at 20");
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y + 7,
        });
        assert_eq!(app.commit_msg_lines, 3);
        app.dispatch(Action::Drag { col: 60, row: 40 });
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_MIN, "at 1");
        app.dispatch(Action::Release);
        assert_eq!(
            load_ui_state(&app.ui_store_path).commit_msg_lines,
            Some(COMMIT_MSG_LINES_MIN)
        );

        // A 12-row pane shows 5 message rows: the drag stops there.
        app.layout = commit_msg_layout(12);
        app.layout.commit_msg_rule_y = Some(8);
        let fit = workspace_status_graph::commit_msg_lines_fit(12);
        assert_eq!(fit, 5);
        app.dispatch(Action::Click { col: 60, row: 8 });
        app.dispatch(Action::Drag { col: 60, row: 0 });
        assert_eq!(app.commit_msg_lines, fit);
        let chrome = app.graph_chrome();
        assert_eq!(usize::from(chrome.footer_body_height()) - 1, fit);
        assert!(chrome.list_height >= 1);
        app.dispatch(Action::Release);
        assert_eq!(
            load_ui_state(&app.ui_store_path).commit_msg_lines,
            Some(fit)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A key that cancels the mouse drag mid-drag keeps the rows reached
    /// and saves nothing, then or at the later release.
    #[test]
    fn commit_msg_rule_drag_cancel_keeps_the_rows_and_saves_nothing() {
        let (mut app, dir) = commit_msg_drag_state("ws-msg-drag-cancel", 40);
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y - 2,
        });
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT + 2);
        app.dispatch(Action::ToggleHelp);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT + 2);
        app.dispatch(Action::ToggleHelp);
        app.dispatch(Action::Release);
        assert!(!app.ui_store_path.exists(), "cancel saves nothing");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed save keeps the dragged rows and names the store.
    #[test]
    fn commit_msg_rule_drag_names_save_failure_and_keeps_the_rows() {
        let (mut app, dir) = commit_msg_drag_state("ws-msg-drag-fail", 40);
        let (root, path) = unwritable_store_path("ws-msg-drag-save-fail");
        app.ui_store_path = path;
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y - 1,
        });
        app.dispatch(Action::Release);
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT + 1);
        assert_eq!(app.status.kind(), StatusKind::Error);
        assert!(
            app.status.contains("commit message lines save failed"),
            "{}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The commit-message rule hovers like the other edges: redraw only on
    /// change, lit through its drag, and off on release away from it.
    #[test]
    fn commit_msg_rule_hovers_and_stays_lit_while_dragging() {
        let (mut app, dir) = commit_msg_drag_state("ws-msg-hover", 40);
        assert!(!app.set_pointer(Some((60, MSG_RULE_Y - 1))), "list row");
        assert!(app.set_pointer(Some((60, MSG_RULE_Y))), "onto the rule");
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::CommitMsg));
        assert!(!app.set_pointer(Some((158, MSG_RULE_Y))), "same edge");
        assert!(app.set_pointer(Some((47, MSG_RULE_Y))), "the band wins");
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));
        assert!(app.set_pointer(Some((60, MSG_RULE_Y + 1))), "footer body");
        assert_eq!(app.hovered_resize_edge(), None);

        app.set_pointer(Some((60, MSG_RULE_Y)));
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y - 5,
        });
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::CommitMsg));
        app.dispatch(Action::Release);
        assert_eq!(app.hovered_resize_edge(), None, "released off the rule");

        // No recorded rule (collapsed or no graph footer): no edge, and a
        // press there is no drag.
        app.layout.commit_msg_rule_y = None;
        app.set_pointer(Some((60, MSG_RULE_Y)));
        assert_eq!(app.hovered_resize_edge(), None);
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        assert_eq!(app.drag, SplitDrag::None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An Explorer tab refuses the commit-message resize: no edge hovers
    /// and a press on a stale recorded rule does not drag it.
    #[test]
    fn explorer_tab_has_no_commit_msg_edge() {
        let (mut app, dir) = commit_msg_drag_state("ws-msg-explorer", 40);
        app.open_explorer_tab("app".into(), super::explorer_tab::ExplorerTarget::Root);
        assert!(app.is_explorer_tab());
        app.layout = commit_msg_layout(40);
        app.set_pointer(Some((60, MSG_RULE_Y)));
        assert_eq!(app.hovered_resize_edge(), None);
        let lines = app.commit_msg_lines;
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Drag { col: 60, row: 2 });
        app.dispatch(Action::Release);
        assert!(
            !matches!(app.drag, SplitDrag::CommitMsg { .. }),
            "{:?}",
            app.drag
        );
        assert_eq!(app.commit_msg_lines, lines);
        assert!(!app.ui_store_path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Launch rows: a valid saved drag value beats config
    /// `viewDefaults.commitMessageLines`, which beats the default; a bad
    /// saved value falls through to config and keeps the saved theme. A
    /// drag saves to the UI-state file only: the config file keeps its
    /// bytes.
    #[test]
    fn launch_commit_msg_lines_saved_beats_config_beats_default() {
        use crate::config::load_config_files;
        let dir = crate::testutil::unique_dir("ws-launch-msg-lines");
        let config_file = dir.join("config").join("config.json");
        let workspace = dir.join("workspace");
        std::fs::create_dir_all(config_file.parent().unwrap()).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        let config_bytes = b"{\"viewDefaults\": {\"commitMessageLines\": 3}}\n";
        std::fs::write(&config_file, config_bytes).unwrap();
        let config = load_config_files(Some(&config_file), &workspace).unwrap();
        assert_eq!(config.view_defaults.commit_message_lines, Some(3));
        let ui_store = dir.join("state").join("ui-state.json");
        let ui_store_s = ui_store.display().to_string();
        let launch = |view_defaults: &ViewDefaults| {
            let settings = Settings::resolve(&config.runtime, |key| match key {
                "WS_STATUS_UI_STORE" => Some(ui_store_s.clone()),
                _ => None,
            });
            let snapshot = build_workspace_snapshot(&[tree_repo()], &[], false, &[]);
            let mut app = AppState::launch(PathBuf::from("/tmp"), snapshot, &settings);
            app.apply_view_defaults(view_defaults);
            app
        };

        assert_eq!(
            launch(&ViewDefaults::default()).commit_msg_lines,
            COMMIT_MSG_LINES_DEFAULT,
            "nothing saved, no config: default"
        );
        assert_eq!(
            launch(&config.view_defaults).commit_msg_lines,
            3,
            "nothing saved: config"
        );

        let mut app = launch(&config.view_defaults);
        app.layout = commit_msg_layout(40);
        app.dispatch(Action::Click {
            col: 60,
            row: MSG_RULE_Y,
        });
        app.dispatch(Action::Drag {
            col: 60,
            row: MSG_RULE_Y - 5,
        });
        app.dispatch(Action::Release);
        assert_eq!(app.commit_msg_lines, 8);
        assert_eq!(
            std::fs::read(&config_file).unwrap(),
            config_bytes,
            "the config file is never written"
        );
        assert_eq!(
            launch(&config.view_defaults).commit_msg_lines,
            8,
            "saved beats config"
        );
        assert_eq!(launch(&ViewDefaults::default()).commit_msg_lines, 8);

        std::fs::write(
            &ui_store,
            "{\"version\": 1, \"theme\": \"nord\", \"activitySparkline\": true, \"commitMessageLines\": 42}\n",
        )
        .unwrap();
        let bad = launch(&config.view_defaults);
        assert_eq!(bad.commit_msg_lines, 3, "a bad saved value falls to config");
        assert_eq!(bad.theme, ThemeId::Nord, "the rest of the store loads");
        assert!(bad.activity_sparkline);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Hover matches what a press does: a tab label on the divider band
    /// takes the press (it activates the tab), so it does not light the
    /// edge; the strip cells past the labels still do, as a press there
    /// starts a pane drag.
    #[test]
    fn tab_label_on_the_divider_band_does_not_hover_the_edge() {
        let mut app = state();
        app.layout = super::LayoutHit {
            tab_y: 0,
            tab_hits: vec![(40, 10, 0)],
            tab_close_hits: vec![(48, 3, 0)],
            ..wide_split_layout()
        };
        for col in [46, 47] {
            app.set_pointer(Some((col, 0)));
            assert_eq!(app.hovered_resize_edge(), None, "tab label at {col}");
        }
        app.set_pointer(Some((48, 0)));
        assert_eq!(app.hovered_resize_edge(), None, "tab close");
        app.layout.tab_hits = vec![(10, 10, 0)];
        app.layout.tab_close_hits = vec![(20, 3, 0)];
        app.set_pointer(Some((47, 0)));
        assert_eq!(
            app.hovered_resize_edge(),
            Some(ResizeEdge::Pane),
            "bare strip cell on the band"
        );
        app.dispatch(Action::Click { col: 47, row: 0 });
        assert_eq!(app.drag, SplitDrag::Pane, "and a press there drags");
        app.dispatch(Action::Release);

        app.layout.tab_hits = vec![(40, 10, 0)];
        app.dispatch(Action::Click { col: 47, row: 0 });
        assert_eq!(app.drag, SplitDrag::None, "a press on the label is no drag");
    }

    /// Quick Open (and its command palette), a confirm dialog, and a
    /// hover popover under the pointer clear the edge hover.
    #[test]
    fn resize_edge_hover_clears_under_quick_open_confirm_and_a_popover() {
        use super::super::action::QuickOpenEntry;
        let mut app = state();
        app.layout = wide_split_layout();
        app.set_pointer(Some((47, 5)));
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));

        for entry in [QuickOpenEntry::Files, QuickOpenEntry::Commands] {
            app.dispatch(Action::ToggleQuickOpen(entry));
            assert!(app.quick_open.is_some(), "{entry:?}");
            assert_eq!(app.hovered_resize_edge(), None, "{entry:?}");
            app.quick_open = None;
            assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));
        }

        app.confirm = Some(PendingConfirm::SwitchToDefault {
            repos: vec!["app".into()],
        });
        assert_eq!(app.input_mode(), InputMode::Confirm);
        assert_eq!(app.hovered_resize_edge(), None, "confirm dialog");
        app.confirm = None;
        assert_eq!(app.hovered_resize_edge(), Some(ResizeEdge::Pane));

        // A painted hover popover (not pinned) over the band.
        app.layout.popover = Some(PopoverHit {
            rect: Rect::new(40, 3, 20, 6),
            inner: Rect::new(42, 4, 16, 4),
            lines: Vec::new(),
        });
        assert_eq!(
            app.input_mode(),
            InputMode::Normal {
                search_active: false
            }
        );
        assert_eq!(app.hovered_resize_edge(), None, "pointer in the popover");
        app.set_pointer(Some((47, 12)));
        assert_eq!(
            app.hovered_resize_edge(),
            Some(ResizeEdge::Pane),
            "below the popover"
        );
    }

    #[test]
    fn pane_divider_band_follows_the_background_mode() {
        // Boxed: last left content column (46) through the right pane's
        // left border (48). Flat: the boundary columns 47 and 48 (the left
        // pane's right pad cell and the right pane's left pad cell).
        for (background, col, divider) in [
            (BackgroundMode::Terminal, 46, true),
            (BackgroundMode::Terminal, 48, true),
            (BackgroundMode::Paint, 46, false),
            (BackgroundMode::Paint, 47, true),
            (BackgroundMode::Paint, 48, true),
        ] {
            let mut app = state();
            app.background = background;
            app.layout = split_layout_for(background);
            app.dispatch(Action::Click { col, row: 5 });
            assert_eq!(
                app.drag == SplitDrag::Pane,
                divider,
                "{background:?} col {col}: {:?}",
                app.drag
            );
        }
    }

    /// Flat divider band = the pad cells at the pane join. A press there
    /// arms the pane drag; released without leaving its column it is a row
    /// click on that cell (row, focus, double-click), with no resize.
    #[test]
    fn flat_divider_pad_release_without_move_clicks_the_row() {
        let mut app = state();
        app.background = BackgroundMode::Paint;
        app.layout = split_layout_for(BackgroundMode::Paint);
        let fraction = app.tree_fraction;
        let start = app.cursor;
        let target = (0..app.rows.len())
            .find(|&idx| app.layout.list_offset + idx != start)
            .expect("another row");
        let row = app.layout.tree_y + target as u16;
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::Click { col: 47, row }), Effect::None);
        assert_eq!(app.drag, SplitDrag::Pane, "the press arms the divider");
        assert_eq!(app.cursor, start, "no row click on press");
        // A drag event in the same column is not a move.
        app.dispatch(Action::Drag {
            col: 47,
            row: row + 1,
        });
        assert_eq!(app.tree_fraction, fraction);
        assert_eq!(app.dispatch(Action::Release), Effect::LoadRightPane);
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(
            app.cursor,
            app.layout.list_offset + target,
            "left pane's right pad"
        );
        assert_eq!(app.focus, FocusPane::Left);
        assert_eq!(app.tree_fraction, fraction, "a click never resizes");

        // A second press + release on the same cell is a double-click on
        // the row, as on its text.
        app.dispatch(Action::Click { col: 47, row });
        let double = app.dispatch(Action::Release);
        let mut twin = state();
        twin.layout = split_layout_for(BackgroundMode::Paint);
        twin.dispatch(Action::Click { col: 10, row });
        assert_eq!(double, twin.dispatch(Action::Click { col: 10, row }));

        app.cancel_mouse_drag();
        app.focus = FocusPane::Left;
        app.dispatch(Action::Click { col: 48, row });
        assert_eq!(app.drag, SplitDrag::Pane);
        app.dispatch(Action::Release);
        assert_eq!(app.focus, FocusPane::Right, "right pane's left pad");
        assert_eq!(app.tree_fraction, fraction);
    }

    /// A flat divider press that moves resizes as before and clicks no row.
    #[test]
    fn flat_divider_pad_press_and_drag_resizes() {
        for col in [47, 48] {
            let mut app = state();
            app.background = BackgroundMode::Paint;
            app.layout = split_layout_for(BackgroundMode::Paint);
            let start = pane_widths(160, app.tree_fraction).tree_width;
            let cursor = app.cursor;
            let row = app.layout.tree_y + u16::from(cursor == 0);
            app.dispatch(Action::Click { col, row });
            app.dispatch(Action::Drag { col: 70, row });
            let after = pane_widths(160, app.tree_fraction).tree_width;
            assert!(after > start, "col {col}: start={start} after={after}");
            assert_eq!(app.dispatch(Action::Release), Effect::None, "col {col}");
            assert_eq!(app.cursor, cursor, "col {col}: no row click after a drag");
            assert_eq!(app.drag, SplitDrag::None);
            assert!(app.last_click.is_none(), "col {col}: a resize is no click");
        }
    }

    /// A pad press that something else ended (here a right click, which
    /// cancels the drag) never clicks a row on a later release.
    #[test]
    fn flat_divider_pad_press_cancelled_before_release_clicks_nothing() {
        let mut app = state();
        app.background = BackgroundMode::Paint;
        app.layout = split_layout_for(BackgroundMode::Paint);
        let cursor = app.cursor;
        let row = app.layout.tree_y + u16::from(cursor == 0);
        app.focus = FocusPane::Right;
        app.dispatch(Action::Click { col: 47, row });
        assert_eq!(app.drag, SplitDrag::Pane);
        app.dispatch(Action::BackClick);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.cursor, cursor, "no row click");
    }

    #[test]
    fn diff_hscrollbar_on_the_last_pane_row_drags_only_when_flat() {
        for (background, drags) in [
            (BackgroundMode::Paint, true),
            (BackgroundMode::Terminal, false),
        ] {
            let mut app = state();
            app.background = background;
            app.layout = split_layout_for(background);
            // pane_height 22: 0-based row 22 is the pane area's last row.
            app.layout.diff_hscrollbar_y = Some(22);
            app.layout.diff_hscrollbar_x = 60;
            app.layout.diff_hscrollbar_width = 20;
            app.layout.diff_col_max = 40;
            app.dispatch(Action::Click { col: 60, row: 22 });
            assert_eq!(
                matches!(app.drag, SplitDrag::DiffHScrollbar { .. }),
                drags,
                "{background:?}: {:?}",
                app.drag
            );
        }
    }

    #[test]
    fn drag_clamp_keeps_panes_nonzero() {
        let mut app = state();
        app.layout = wide_split_layout();
        app.dispatch(Action::Click { col: 47, row: 4 });
        app.dispatch(Action::Drag { col: 0, row: 4 });
        let w = pane_widths(160, app.tree_fraction);
        assert!(w.tree_width >= 20);
        assert!(w.diff_width >= 20);
        app.dispatch(Action::Release);
        let rule = app.layout.diff_split_rule_x.expect("rule");
        app.dispatch(Action::Click { col: rule, row: 4 });
        app.dispatch(Action::Drag { col: 0, row: 4 });
        let s = side_by_side_column_widths(110, app.diff_split_fraction);
        assert!(s.left_width >= 1);
        assert!(s.right_width >= 1);
    }

    #[test]
    fn click_on_tree_is_not_a_split_drag() {
        let mut app = state();
        app.layout = wide_split_layout();
        assert!(!app.rows.is_empty());
        let effect = app.dispatch(Action::Click {
            col: 10,
            row: app.layout.tree_y,
        });
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(effect, Effect::LoadRightPane);
        assert_eq!(app.cursor, app.layout.list_offset);
    }

    #[test]
    fn pane_divider_drag_does_not_select_or_copy() {
        let mut app = state();
        app.layout = wide_split_layout();
        app.dispatch(Action::Click { col: 47, row: 5 });
        assert_eq!(app.drag, SplitDrag::Pane);
        assert!(app.text_selection.is_none());
        app.dispatch(Action::Drag { col: 70, row: 8 });
        assert!(app.text_selection.is_none());
        assert_eq!(app.dispatch(Action::Release), Effect::None);
    }

    #[test]
    fn tree_press_arms_selection_and_drag_clamps_to_tree_pane() {
        let mut app = state();
        app.layout = wide_split_layout();
        let row = app.layout.tree_y;
        app.dispatch(Action::Click { col: 10, row });
        let armed = app.text_selection.expect("press in tree arms a selection");
        assert!(!armed.is_active());
        app.dispatch(Action::Drag { col: 100, row: 6 });
        let dragged = app.text_selection.expect("drag keeps the selection");
        assert!(dragged.is_active());
        assert_eq!(
            dragged.head,
            (app.layout.tree_x + app.layout.tree_width - 1, 6),
            "drag past the divider clamps to the tree pane"
        );
    }

    #[test]
    fn plain_click_release_copies_nothing() {
        let mut app = state();
        app.layout = wide_split_layout();
        let row = app.layout.tree_y;
        app.dispatch(Action::Click { col: 10, row });
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert!(app.text_selection.is_none());
    }

    #[test]
    fn help_and_mouse_toggle_clear_text_selection() {
        let mut app = state();
        app.layout = wide_split_layout();
        let row = app.layout.tree_y;
        app.dispatch(Action::Click { col: 10, row });
        app.dispatch(Action::Drag { col: 20, row: 4 });
        app.dispatch(Action::ToggleHelp);
        assert!(app.text_selection.is_none());
        app.dispatch(Action::ToggleHelp);
        app.dispatch(Action::Click { col: 10, row });
        app.dispatch(Action::Drag { col: 20, row: 4 });
        app.dispatch(Action::ToggleMouse);
        assert!(app.text_selection.is_none());
        assert!(!app.mouse_enabled);
        app.dispatch(Action::Click { col: 10, row });
        assert!(app.text_selection.is_none(), "mouse off ignores presses");
    }

    #[test]
    fn i_toggles_inline_without_mouse() {
        let mut app = state();
        assert_eq!(app.diff_mode, DiffMode::SideBySide);
        app.dispatch(Action::ToggleDiffMode);
        assert_eq!(app.diff_mode, DiffMode::Inline);
        app.dispatch(Action::ToggleDiffMode);
        assert_eq!(app.diff_mode, DiffMode::SideBySide);
    }

    #[test]
    fn i_says_what_width_split_needs_when_the_pane_is_too_narrow() {
        let mut app = state();
        app.layout.diff_pane_width = 82;
        app.diff_mode = DiffMode::Inline;
        app.dispatch(Action::ToggleDiffMode);
        assert_eq!(app.diff_mode, DiffMode::SideBySide);
        assert_eq!(
            app.status,
            "split needs a ≥100-col diff pane (now 81) — widen it with < or drag"
        );
        app.dispatch(Action::ToggleDiffMode);
        assert_eq!(app.status, "Diff: inline");
        app.layout.diff_pane_width = 120;
        app.dispatch(Action::ToggleDiffMode);
        assert_eq!(app.status, "Diff: split");
    }

    #[test]
    fn angle_brackets_move_the_tree_split_by_five_percent_and_clamp() {
        let mut app = state();
        app.layout.term_cols = 140;
        let width = |app: &AppState| pane_widths(140, app.tree_fraction).tree_width;
        assert_eq!(width(&app), 56);
        assert_eq!(app.dispatch(Action::ResizeTree(1)), Effect::None);
        assert_eq!(width(&app), 63);
        app.dispatch(Action::ResizeTree(-1));
        app.dispatch(Action::ResizeTree(-1));
        assert_eq!(width(&app), 49);
        for _ in 0..20 {
            app.dispatch(Action::ResizeTree(-1));
        }
        assert_eq!(width(&app), MIN_PANE_COLS);
        assert_eq!(app.status, "tree pane is at its narrowest");
        for _ in 0..30 {
            app.dispatch(Action::ResizeTree(1));
        }
        assert_eq!(
            pane_widths(140, app.tree_fraction).diff_width,
            MIN_PANE_COLS
        );
        assert_eq!(app.status, "tree pane is at its widest");
    }

    #[test]
    fn right_click_steps_back_like_esc_and_respects_mouse_off() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        app.dispatch(Action::BackClick);
        assert_eq!(app.focus, FocusPane::Left);
        app.focus = FocusPane::Right;
        app.mouse_enabled = false;
        app.dispatch(Action::BackClick);
        assert_eq!(app.focus, FocusPane::Right, "mouse off ignores it");
        app.mouse_enabled = true;
        app.search_active = true;
        app.search_query = "app".into();
        app.dispatch(Action::BackClick);
        assert!(!app.search_active, "an armed search clears first, like Esc");
        assert_eq!(app.focus, FocusPane::Right);
    }

    fn needle_diff(app: &mut AppState) {
        focus_file(app, "README.md");
        let mut unified = String::from("@@ -1,9 +1,9 @@\n");
        for i in 0..9 {
            if i % 3 == 1 {
                unified.push_str(&format!(" needle {i}\n"));
            } else {
                unified.push_str(&format!(" hay {i}\n"));
            }
        }
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(unified),
        );
        app.focus = FocusPane::Right;
    }

    #[test]
    fn diff_search_counts_matches_and_says_when_it_wraps() {
        let mut app = state();
        needle_diff(&mut app);
        app.dispatch(Action::SearchStart);
        for c in "needle".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.search_match_position(), Some((Some(1), 3)));
        app.dispatch(Action::SearchNext);
        assert_eq!(app.search_match_position(), Some((Some(2), 3)));
        assert!(app.status.is_empty(), "{}", &*app.status);
        app.dispatch(Action::SearchNext);
        assert_eq!(app.search_match_position(), Some((Some(3), 3)));
        assert!(app.status.is_empty(), "{}", &*app.status);
        app.dispatch(Action::SearchNext);
        assert_eq!(app.search_match_position(), Some((Some(1), 3)));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        assert_eq!(app.status.kind(), StatusKind::Info);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.search_match_position(), Some((Some(3), 3)));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
        assert_eq!(app.status.kind(), StatusKind::Info);
    }

    /// Typing `/needle` with the diff cursor in the middle.
    fn type_diff_search_from(app: &mut AppState, cursor: usize, query: &str) {
        app.diff_cursor = cursor;
        type_search(app, query);
    }

    #[test]
    fn diff_enter_lands_on_the_next_match_below_the_cursor() {
        let mut app = state();
        needle_diff(&mut app);
        let hits = app.diff_search_hits("needle");
        assert_eq!(hits.len(), 3);
        // Mid-list non-match: next match below, not the first.
        type_diff_search_from(&mut app, hits[1] - 1, "needle");
        assert_eq!(app.diff_cursor, hits[1]);
        assert_eq!(app.search_hit, Some(hits[1]));
        assert_eq!(app.search_match_position(), Some((Some(2), 3)));
        assert!(app.status.is_empty(), "{}", &*app.status);
        // The origin row's own match is skipped.
        type_diff_search_from(&mut app, hits[1], "needle");
        assert_eq!(app.diff_cursor, hits[2]);
        assert_eq!(app.search_match_position(), Some((Some(3), 3)));
        // Below the last match: Enter wraps to the top and says so.
        type_diff_search_from(&mut app, hits[2] + 1, "needle");
        assert_eq!(app.diff_cursor, hits[0]);
        assert_eq!(app.search_match_position(), Some((Some(1), 3)));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
    }

    #[test]
    fn diff_n_steps_from_the_moved_cursor_not_the_stale_hit() {
        let mut app = state();
        needle_diff(&mut app);
        let hits = app.diff_search_hits("needle");
        type_diff_search_from(&mut app, 0, "needle");
        assert_eq!(app.search_hit, Some(hits[0]));
        // The user moves past the second match: `n` goes on from there.
        app.diff_cursor = hits[1] + 1;
        app.dispatch(Action::SearchNext);
        assert_eq!(app.diff_cursor, hits[2]);
        assert_eq!(app.search_match_position(), Some((Some(3), 3)));
        app.diff_cursor = hits[1] + 1;
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.diff_cursor, hits[1]);
        assert_eq!(app.search_match_position(), Some((Some(2), 3)));
        assert!(app.status.is_empty(), "{}", &*app.status);
    }

    #[test]
    fn diff_typing_recomputes_from_the_origin_and_no_hit_stays_there() {
        let mut app = state();
        needle_diff(&mut app);
        let hits = app.diff_search_hits("needle");
        app.diff_cursor = 0;
        app.dispatch(Action::SearchStart);
        for c in "needle".chars() {
            app.dispatch(Action::SearchChar(c));
            // Each key starts again from the origin: no walk per char.
            assert_eq!(app.diff_cursor, hits[0], "after {c}");
        }
        app.dispatch(Action::SearchChar('X'));
        assert_eq!(app.diff_cursor, 0, "no hit: back at the origin");
        assert_eq!(app.status, SEARCH_NO_MATCH);
        assert_eq!(app.status.kind(), StatusKind::Warn);
        app.dispatch(Action::SearchBackspace);
        assert_eq!(app.diff_cursor, hits[0]);
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.diff_cursor, hits[0], "Enter matches the preview");
        assert!(app.search_origin.is_none(), "Enter drops the origin");
    }

    #[test]
    fn diff_search_with_no_match_says_so_without_a_wrap_notice() {
        let mut app = state();
        needle_diff(&mut app);
        type_diff_search_from(&mut app, 3, "zzz-missing");
        assert_eq!(app.diff_cursor, 3);
        assert_eq!(app.status, SEARCH_NO_MATCH);
        app.dispatch(Action::SearchNext);
        assert_eq!(app.diff_cursor, 3);
        assert_eq!(app.status, SEARCH_NO_MATCH);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.status, SEARCH_NO_MATCH);
    }

    #[test]
    fn compare_tab_diff_search_steps_from_the_diff_cursor() {
        let mut app = summary_compare_app();
        app.focus = FocusPane::Right;
        let hits = app.diff_search_hits("e");
        assert!(hits.len() >= 2, "{hits:?}");
        type_diff_search_from(&mut app, hits[0], "e");
        assert_eq!(app.search_target, SearchPane::Diff);
        assert_eq!(app.diff_cursor, hits[1], "origin match skipped");
        assert_eq!(app.search_match_position(), Some((Some(2), hits.len())));
        app.diff_cursor = *hits.last().unwrap();
        app.dispatch(Action::SearchNext);
        assert_eq!(app.diff_cursor, hits[0]);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        // The tab parks and restores the diff cursor `n` steps from.
        let _ = app.activate_tab(0);
        let _ = app.activate_tab(1);
        assert_eq!(app.diff_cursor, hits[0]);
        app.dispatch(Action::SearchNext);
        assert_eq!(app.diff_cursor, hits[1]);
    }

    #[test]
    fn esc_while_typing_puts_the_diff_cursor_back() {
        let mut app = state();
        needle_diff(&mut app);
        app.diff_cursor = 0;
        app.dispatch(Action::SearchStart);
        for c in "needle".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert_ne!(app.diff_cursor, 0, "typing jumps to the first match");
        app.dispatch(Action::SearchCancel);
        assert_eq!(app.diff_cursor, 0);
        assert!(!app.search_active && !app.search_mode);
    }

    #[test]
    fn esc_while_typing_puts_the_tree_cursor_and_folds_back() {
        let mut app = state();
        focus_repo(&mut app, "app");
        let row = app.focused_row().map(|r| r.id.clone());
        let folds = app.folds.clone();
        app.dispatch(Action::SearchStart);
        for c in "lib".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert_ne!(app.focused_row().map(|r| r.id.clone()), row);
        assert_eq!(app.dispatch(Action::SearchCancel), Effect::LoadRightPane);
        assert_eq!(app.focused_row().map(|r| r.id.clone()), row);
        assert_eq!(app.folds, folds);
    }

    #[test]
    fn esc_up_a_depth_leaves_no_toast() {
        let mut app = graph_state(false);
        install_graph_commit(&mut app, &["main"]);
        app.drill = DrillView::Files {
            repo: "app".into(),
            source: CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files: Vec::new(),
            cursor: 0,
        };
        app.focus = FocusPane::Left;
        app.status.clear();
        assert_eq!(app.dispatch(Action::NavEsc), Effect::LoadRightPane);
        assert!(app.drill.is_graph());
        assert!(app.status.is_empty(), "{}", &*app.status);
    }

    #[test]
    fn backslash_toggles_diff_wrap_and_clears_pan() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.layout.diff_pane_width = 8;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(format!("@@ -0,0 +1,1 @@\n+{}", "x".repeat(40))),
        );
        assert!(app.diff_wrap, "wrap is on at launch");
        app.dispatch(Action::PanDiff(4));
        assert_eq!(app.diff_col_offset, 0, "pan is a no-op while wrap is on");
        app.dispatch(Action::ToggleDiffWrap);
        assert!(!app.diff_wrap);
        assert_eq!(app.status, "wrap off");
        app.dispatch(Action::PanDiff(4));
        assert!(app.diff_col_offset > 0, "clip+pan works after wrap off");
        app.dispatch(Action::ToggleDiffWrap);
        assert!(app.diff_wrap);
        assert_eq!(app.diff_col_offset, 0);
        assert_eq!(app.status, "wrap on");
    }

    #[test]
    fn m_toggles_commit_msg_expand_on_graph_footer_and_commit_detail() {
        // Default on; `M` collapses, a second `M` expands again.
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        let tail = "TAILTOKEN";
        let body = "UNIQUE_DETAIL_BODY";
        if let Some(model) = app.graph.as_mut() {
            model.commits[0].subject = format!("{}{tail}", "n".repeat(40));
            model.commits[0].body = body.into();
            model.commits[0].author_name = "Ada".into();
            model.commits[0].author_date_unix = unix_now() - 60;
        }
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
        assert!(app.commit_msg_expand, "expanded by default");
        app.dispatch(Action::ToggleCommitMsgExpand);
        assert!(!app.commit_msg_expand);
        assert_eq!(app.status, "msg off");
        app.dispatch(Action::ToggleCommitMsgExpand);
        assert!(app.commit_msg_expand);
        assert_eq!(app.status, "msg on");

        let glyphs = &workspace_status_graph::ASCII;
        let model = app.graph.as_ref().expect("graph");
        let row = model
            .visible_rows()
            .into_iter()
            .find(|r| matches!(r, GraphRow::Commit { .. }))
            .expect("commit row");
        let collapsed = workspace_status_graph::selection_footer_lines(
            model,
            workspace_status_graph::GraphFooterSelection::Row(&row),
            glyphs,
            16,
            unix_now(),
            false,
        );
        let collapsed_text = collapsed.join("\n");
        assert!(
            !collapsed_text.contains(body),
            "collapsed footer hides body: {collapsed_text}"
        );
        assert!(
            !collapsed_text.contains(tail),
            "collapsed footer clips subject: {collapsed_text}"
        );
        let expanded = workspace_status_graph::selection_footer_lines(
            model,
            workspace_status_graph::GraphFooterSelection::Row(&row),
            glyphs,
            16,
            unix_now(),
            true,
        );
        let expanded_text = expanded.join("");
        assert!(
            expanded_text.contains(body),
            "expanded footer shows body: {expanded:?}"
        );
        assert!(
            expanded_text.contains(tail),
            "expanded footer shows subject tail: {expanded:?}"
        );

        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
        );
        // 32 columns: subject (2 rows), blank, body fill the N = 4 rows.
        let collapsed_files_footer = {
            app.commit_msg_expand = false;
            app.commit_detail_footer_lines(32)
        };
        assert_eq!(collapsed_files_footer.len(), 2, "title + subtitle");
        let collapsed_join = collapsed_files_footer.join("\n");
        assert!(
            !collapsed_join.contains(body),
            "collapsed files footer hides body: {collapsed_join}"
        );
        app.commit_msg_expand = true;
        let expanded_files_footer = app.commit_detail_footer_lines(32);
        assert_eq!(
            expanded_files_footer.len(),
            2 + 2 + 1 + 1,
            "title + meta + subject (2 rows) + blank + body, no padding"
        );
        let expanded_join = expanded_files_footer.join("");
        assert!(
            expanded_join.contains(body),
            "expanded files footer shows body: {expanded_files_footer:?}"
        );
        assert!(
            expanded_join.contains(tail),
            "expanded files footer shows subject tail: {expanded_files_footer:?}"
        );
        // A long body is never clipped: every line comes back, wrapped one
        // column short of the width (the scrollbar column), for the footer
        // to scroll.
        app.graph.as_mut().expect("graph").commits[0].body = long_commit_body(30);
        let long_footer = app.commit_detail_footer_lines(32);
        assert_eq!(
            long_footer.len(),
            2 + 2 + 1 + 30,
            "title + meta + subject (2 rows) + blank + 30 body lines: {long_footer:?}"
        );
        assert!(
            long_footer.iter().all(|line| !line.contains('…')),
            "no … clip: {long_footer:?}"
        );
        assert!(
            long_footer[2..]
                .iter()
                .all(|line| line.chars().count() <= 31),
            "message wraps at width - 1: {long_footer:?}"
        );
        assert_eq!(long_footer.last().map(String::as_str), Some("body line 29"));
        app.dispatch(Action::ToggleCommitMsgExpand);
        assert!(!app.commit_msg_expand);
        assert_eq!(app.status, "msg off");
    }

    /// A commit body of `n` lines `body line 00`, `body line 01`, ….
    fn long_commit_body(n: usize) -> String {
        (0..n)
            .map(|i| format!("body line {i:02}"))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Depth 2 on the seed commit with a 30-line body: `one.rs` selected and
    /// its diff on the right, drawn once so the footer records its box.
    fn depth_2_long_message_state() -> AppState {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        app.graph.as_mut().expect("graph").commits[0].body = long_commit_body(30);
        app.open_commit_diff(
            "app".into(),
            sample_commit_source(),
            two_commit_files(),
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        draw_once(&mut app);
        app
    }

    fn wheel(col: u16, row: u16, delta: i32) -> Action {
        Action::ScrollWheel {
            col,
            row,
            delta,
            horizontal: false,
        }
    }

    #[test]
    fn wheel_over_depth_2_commit_footer_scrolls_the_message_not_the_file_list() {
        let mut app = depth_2_long_message_state();
        let top = app
            .layout
            .commit_detail_footer_y
            .expect("overflowing footer records its hit box");
        let max = app.layout.commit_detail_footer_scroll_max;
        assert!(max > 0);
        let col = app.layout.commit_detail_footer_x + 2;
        let last_row = top + app.layout.commit_detail_footer_height - 1;
        let focus = app.focus;
        assert_eq!(app.commit_files_cursor(), 0);
        for _ in 0..max + 5 {
            let effect = app.dispatch(wheel(col, last_row, 1));
            assert!(matches!(effect, Effect::None), "no diff load: {effect:?}");
        }
        assert_eq!(app.commit_files_cursor(), 0, "file cursor stays");
        assert_eq!(app.focus, focus, "focus stays");
        assert_eq!(app.commit_detail_msg_scroll(), max, "clamped at the end");
        // The rule row is part of the footer box, like the graph footer.
        app.dispatch(wheel(col, top, -1));
        assert_eq!(app.commit_detail_msg_scroll(), max - 1, "wheel up");
        for _ in 0..max + 5 {
            app.dispatch(wheel(col, last_row, -1));
        }
        assert_eq!(app.commit_detail_msg_scroll(), 0, "clamped at the top");

        // Outside the footer the wheel still moves the file cursor.
        let list_row = app.layout.files_list_y;
        assert!(list_row < top);
        let effect = app.dispatch(wheel(col, list_row, 1));
        assert_eq!(app.commit_files_cursor(), 1, "list wheel moves the cursor");
        assert!(!matches!(effect, Effect::None), "and loads the diff");
        assert_eq!(app.commit_detail_msg_scroll(), 0);
    }

    #[test]
    fn depth_2_commit_footer_scroll_is_keyed_by_the_commit_graph_identity() {
        let mut app = depth_2_long_message_state();
        let top = app.layout.commit_detail_footer_y.expect("footer box");
        let col = app.layout.commit_detail_footer_x + 2;
        for _ in 0..3 {
            app.dispatch(wheel(col, top, 1));
        }
        assert_eq!(app.commit_detail_msg_scroll(), 3);
        // Same key as the graph footer of that commit row.
        let row = app
            .graph
            .as_ref()
            .expect("graph")
            .visible_rows()
            .into_iter()
            .find(|row| matches!(row, GraphRow::Commit { .. }))
            .expect("commit row");
        assert_eq!(
            app.commit_detail_msg_identity(),
            Some(graph_row_identity("app", &row))
        );
        // Another commit starts at the top and records nothing until a wheel.
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "bbb2222cccccccccccccccccccccccccccccccc".into(),
            },
            two_commit_files(),
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        assert_eq!(app.commit_detail_msg_scroll(), 0, "another commit");
        // A stash keys like its graph row too.
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Stash {
                stash_ref: "stash@{0}".into(),
            },
            two_commit_files(),
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        assert_eq!(
            app.commit_detail_msg_identity().as_deref(),
            Some("app#stash:stash@{0}")
        );
        assert_eq!(app.commit_detail_msg_scroll(), 0, "a stash");
    }

    fn arm_graph_scrollbar(app: &mut AppState, content_len: usize) {
        app.layout.term_cols = 160;
        app.layout.pane_height = 22;
        app.layout.outer_tree_width = 48;
        app.layout.right_x = 48;
        app.layout.right_y = 1;
        app.layout.graph_scrollbar_x = Some(158);
        app.layout.graph_scrollbar_y = 2;
        app.layout.graph_scrollbar_height = 10;
        app.layout.graph_content_len = content_len;
        app.graph_scroll = 0;
        app.focus = FocusPane::Right;
    }

    #[test]
    fn graph_scrollbar_thumb_drag_updates_scroll_and_release_ends_drag() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let content_len = paint_model(app.graph.as_ref().unwrap(), glyphs, None).len();
        arm_graph_scrollbar(&mut app, content_len);
        let thumb =
            workspace_status_graph::graph_scrollbar_thumb(content_len, 0, 10).expect("thumb");
        let thumb_row = 2 + thumb.0;
        let start = app.graph_scroll;
        let cursor = app.graph_cursor;
        assert_eq!(
            app.dispatch(Action::Click {
                col: 158,
                row: thumb_row
            }),
            Effect::None
        );
        assert_eq!(
            app.drag,
            SplitDrag::GraphScrollbar {
                origin_row: thumb_row,
                origin_scroll: start
            }
        );
        assert_eq!(app.graph_scroll, start, "thumb grab must not jump");
        assert_eq!(
            app.dispatch(Action::Drag {
                col: 158,
                row: thumb_row + 6
            }),
            Effect::None
        );
        assert!(
            app.graph_scroll > start,
            "thumb drag should scroll, got {}",
            app.graph_scroll
        );
        assert_eq!(
            app.graph_cursor, cursor,
            "scrollbar must not move the graph cursor"
        );
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);

        app.dispatch(Action::Move(1));
        assert_ne!(app.graph_cursor, cursor, "j still moves the graph cursor");
        let after_j = app.graph_cursor;
        app.dispatch(Action::PageMove(1));
        assert_ne!(app.graph_cursor, after_j, "PageDown still pages the graph");
        app.dispatch(Action::PanDiff(1));
        app.dispatch(Action::PanDiff(-1));
    }

    #[test]
    fn graph_scrollbar_track_click_jumps_and_arms_drag() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let content_len = paint_model(app.graph.as_ref().unwrap(), glyphs, None).len();
        arm_graph_scrollbar(&mut app, content_len);
        let thumb =
            workspace_status_graph::graph_scrollbar_thumb(content_len, 0, 10).expect("thumb");
        let track_row = 2 + 9;
        assert!(
            track_row >= 2 + thumb.0 + thumb.1,
            "fixture track row must sit below the thumb"
        );
        let cursor = app.graph_cursor;
        assert_eq!(
            app.dispatch(Action::Click {
                col: 158,
                row: track_row
            }),
            Effect::None
        );
        assert!(
            app.graph_scroll > 0,
            "track click should jump toward the click"
        );
        assert_eq!(
            app.drag,
            SplitDrag::GraphScrollbar {
                origin_row: track_row,
                origin_scroll: app.graph_scroll
            }
        );
        assert_eq!(app.graph_cursor, cursor);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
    }

    #[test]
    fn click_on_graph_body_is_not_a_scrollbar_drag() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let content_len = paint_model(app.graph.as_ref().unwrap(), glyphs, None).len();
        arm_graph_scrollbar(&mut app, content_len);
        let effect = app.dispatch(Action::Click { col: 80, row: 4 });
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(effect, Effect::None);
        assert_eq!(app.focus, FocusPane::Right);
    }

    fn arm_graph_hscrollbar(app: &mut AppState, col_max: u16) {
        app.layout.term_cols = 160;
        app.layout.pane_height = 22;
        app.layout.outer_tree_width = 48;
        app.layout.right_x = 48;
        app.layout.graph_hscrollbar_y = Some(12);
        app.layout.graph_hscrollbar_x = 50;
        app.layout.graph_hscrollbar_width = 20;
        app.layout.graph_col_max = col_max;
        app.right_col_offset = 0;
        app.focus = FocusPane::Right;
    }

    #[test]
    fn graph_horizontal_scrollbar_thumb_drag_updates_offset() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 4);
        arm_graph_hscrollbar(&mut app, 40);
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 0, 20).expect("thumb");
        let thumb_col = 50 + thumb.0;
        let start = app.right_col_offset;
        assert_eq!(
            app.dispatch(Action::Click {
                col: thumb_col,
                row: 12
            }),
            Effect::None
        );
        assert_eq!(
            app.drag,
            SplitDrag::GraphHScrollbar {
                origin_col: thumb_col,
                origin_offset: start
            }
        );
        assert_eq!(app.right_col_offset, start, "thumb grab must not jump");
        assert_eq!(
            app.dispatch(Action::Drag {
                col: thumb_col + 10,
                row: 12
            }),
            Effect::None
        );
        assert!(
            app.right_col_offset > start,
            "thumb drag should pan, got {}",
            app.right_col_offset
        );
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        app.dispatch(Action::PanDiff(-20));
        assert_eq!(app.right_col_offset, 0, "keyboard pan still clamps at 0");
    }

    fn arm_diff_hscrollbar(app: &mut AppState, col_max: u16) {
        app.layout.term_cols = 160;
        app.layout.pane_height = 22;
        app.layout.outer_tree_width = 48;
        app.layout.right_x = 48;
        app.layout.right_y = 1;
        app.layout.diff_pane_width = 110;
        app.layout.diff_hscrollbar_y = Some(12);
        app.layout.diff_hscrollbar_x = 50;
        app.layout.diff_hscrollbar_width = 20;
        app.layout.diff_col_max = col_max;
        app.diff_col_offset = 0;
        app.focus = FocusPane::Left;
    }

    fn long_line_diff() -> DiffContent {
        DiffContent::from_unified(format!("@@ -0,0 +1,1 @@\n+{}", "x".repeat(80)))
    }

    #[test]
    fn diff_horizontal_scrollbar_thumb_drag_updates_offset() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), long_line_diff());
        arm_diff_hscrollbar(&mut app, 40);
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 0, 20).expect("thumb");
        let thumb_col = 50 + thumb.0;
        let start = app.diff_col_offset;
        let cursor = app.diff_cursor;
        assert_eq!(
            app.dispatch(Action::Click {
                col: thumb_col,
                row: 12
            }),
            Effect::None
        );
        assert_eq!(
            app.drag,
            SplitDrag::DiffHScrollbar {
                origin_col: thumb_col,
                origin_offset: start
            }
        );
        assert_eq!(app.diff_col_offset, start, "thumb grab must not jump");
        assert_eq!(
            app.diff_cursor, cursor,
            "scrollbar must not move the cursor"
        );
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(
            app.dispatch(Action::Drag {
                col: thumb_col + 10,
                row: 12
            }),
            Effect::None
        );
        assert!(
            app.diff_col_offset > start,
            "thumb drag should pan, got {}",
            app.diff_col_offset
        );
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
        app.dispatch(Action::PanDiff(-80));
        assert_eq!(app.diff_col_offset, 0, "keyboard pan still clamps at 0");
    }

    #[test]
    fn diff_horizontal_scrollbar_thumb_grab_at_origin_hidden_offset_does_not_jump() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), long_line_diff());
        arm_diff_hscrollbar(&mut app, 40);
        app.diff_col_offset = 12;
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 12, 20).expect("thumb");
        assert!(
            thumb.0 > 0,
            "origin-hidden offset must move the thumb off the left edge"
        );
        let thumb_col = 50 + thumb.0;
        let start = app.diff_col_offset;
        let cursor = app.diff_cursor;
        assert_eq!(
            app.dispatch(Action::Click {
                col: thumb_col,
                row: 12
            }),
            Effect::None
        );
        assert_eq!(
            app.drag,
            SplitDrag::DiffHScrollbar {
                origin_col: thumb_col,
                origin_offset: start
            }
        );
        assert_eq!(
            app.diff_col_offset, start,
            "thumb grab must not jump from a non-zero pan"
        );
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(
            app.dispatch(Action::Drag {
                col: thumb_col + 10,
                row: 12
            }),
            Effect::None
        );
        assert!(
            app.diff_col_offset > start,
            "thumb drag should pan from {start}, got {}",
            app.diff_col_offset
        );
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
    }

    #[test]
    fn diff_horizontal_scrollbar_left_track_click_at_nonzero_offset_jumps_to_origin() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), long_line_diff());
        arm_diff_hscrollbar(&mut app, 40);
        app.diff_col_offset = 12;
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 12, 20).expect("thumb");
        assert!(thumb.0 > 0, "left edge must sit left of the thumb");
        let cursor = app.diff_cursor;
        assert_eq!(
            app.dispatch(Action::Click { col: 50, row: 12 }),
            Effect::None
        );
        assert_eq!(
            app.diff_col_offset, 0,
            "left-edge track click must jump to origin (and hide the bar)"
        );
        assert_eq!(
            app.drag,
            SplitDrag::DiffHScrollbar {
                origin_col: 50,
                origin_offset: 0
            }
        );
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.focus, FocusPane::Right);
    }

    #[test]
    fn diff_horizontal_scrollbar_track_click_jumps_and_arms_drag() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), long_line_diff());
        arm_diff_hscrollbar(&mut app, 40);
        let thumb = workspace_status_graph::graph_scrollbar_thumb(41, 0, 20).expect("thumb");
        let track_col = 50 + 19;
        assert!(
            track_col >= 50 + thumb.0 + thumb.1,
            "fixture track col must sit right of the thumb"
        );
        let cursor = app.diff_cursor;
        assert_eq!(
            app.dispatch(Action::Click {
                col: track_col,
                row: 12
            }),
            Effect::None
        );
        assert!(
            app.diff_col_offset > 0,
            "track click should jump toward the click"
        );
        assert_eq!(
            app.drag,
            SplitDrag::DiffHScrollbar {
                origin_col: track_col,
                origin_offset: app.diff_col_offset
            }
        );
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert_eq!(app.drag, SplitDrag::None);
    }

    /// Repo-relative path that wraps to three header rows at width 20.
    const LONG_DIFF_PATH: &str = "src/deeply/nested/folder/module/file.rs";

    /// File diff of 40 added lines under `path`, in a 20×20 right pane.
    fn tall_diff_in_narrow_pane(path: &str) -> AppState {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let mut lines = vec!["@@ -1,1 +1,40 @@".into()];
        lines.extend((0..40).map(|i| format!("+line {i}")));
        app.set_diff("app".into(), path.into(), DiffContent::from_lines(lines));
        app.layout.term_cols = 160;
        app.layout.pane_height = 22;
        app.layout.outer_tree_width = 48;
        app.layout.right_x = 48;
        app.layout.right_y = 1;
        app.layout.diff_pane_width = 20;
        app.layout.diff_pane_height = 20;
        app
    }

    #[test]
    fn diff_body_height_shrinks_by_the_wrapped_header_rows() {
        let short = tall_diff_in_narrow_pane("README.md");
        assert_eq!(short.diff_header_rows(), 1);
        assert_eq!(short.diff_body_height(), 19);
        let long = tall_diff_in_narrow_pane(LONG_DIFF_PATH);
        assert_eq!(long.diff_header_rows(), 3, "{}", long.diff_header_path());
        assert_eq!(long.diff_body_height(), 17);
    }

    /// The pan max over rows a caller already built matches the old
    /// self-building derivation, the h-bar row waits for a pan, and a held
    /// `j` / PageDown builds the diff rows once per key.
    #[test]
    fn diff_pan_max_reuses_rows_and_a_key_builds_rows_once() {
        use super::super::diff::{cell_code_width, diff_row_content_width, gutter_width};
        use super::super::search::max_col_offset;
        let mut app = tall_diff_in_narrow_pane("README.md");
        app.diff_wrap = false;
        let mut lines = vec!["@@ -1,1 +1,41 @@".into(), format!("+{}", "w".repeat(60))];
        lines.extend((0..40).map(|i| format!("+line {i}")));
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(lines),
        );
        let rows = app.current_diff_rows();
        let lens: Vec<usize> = rows
            .iter()
            .filter_map(|row| match row {
                DiffRow::Line { left, .. } => Some(left.text.chars().count()),
                _ => None,
            })
            .collect();
        let gutter = gutter_width(&rows).saturating_add(comment_mark_cols(app.ascii));
        let content_w = diff_row_content_width(diff_paint_width(20) as usize);
        let expected = max_col_offset(&lens, cell_code_width(content_w, gutter));
        assert!(expected > 0);
        assert_eq!(app.diff_pan_max(), expected);
        assert_eq!(app.diff_pan_max_for(&rows), expected);

        assert!(!app.diff_hscroll_shown(), "no h-bar at pan 0");
        assert_eq!(app.diff_body_height(), 19);
        app.diff_col_offset = 1;
        assert!(app.diff_hscroll_shown());
        assert_eq!(app.diff_body_height(), 18, "the h-bar takes a body row");
        app.diff_wrap = true;
        assert_eq!(app.diff_pan_max(), 0);
        assert!(!app.diff_hscroll_shown(), "wrap never pans");
        app.diff_wrap = false;

        app.focus = FocusPane::Right;
        for wrap in [false, true] {
            app.diff_wrap = wrap;
            for action in [Action::Move(1), Action::PageMove(1)] {
                app.diff_row_builds.set(0);
                app.dispatch(action.clone());
                assert_eq!(app.diff_row_builds.get(), 1, "{action:?} wrap={wrap}");
            }
        }
    }

    #[test]
    fn click_below_a_wrapped_diff_header_selects_the_first_body_row() {
        let mut app = tall_diff_in_narrow_pane(LONG_DIFF_PATH);
        app.focus = FocusPane::Right;
        app.dispatch(Action::Move(30));
        let scroll = app.diff_scroll as usize;
        assert!(scroll > 0, "fixture must scroll the diff");
        let header = app.diff_header_rows();
        assert_eq!(header, 3);

        let cursor = app.diff_cursor;
        let header_row = app.layout.right_y + header - 1;
        app.dispatch(Action::Click {
            col: 100,
            row: header_row,
        });
        assert_eq!(app.diff_cursor, cursor, "a header row selects nothing");

        let body_row = app.layout.right_y + header;
        app.dispatch(Action::Click {
            col: 100,
            row: body_row,
        });
        assert_eq!(app.diff_cursor, scroll, "first body row is `diff_scroll`");
    }

    #[test]
    fn click_on_diff_body_is_not_a_horizontal_scrollbar_drag() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), long_line_diff());
        arm_diff_hscrollbar(&mut app, 40);
        let effect = app.dispatch(Action::Click { col: 80, row: 4 });
        assert_eq!(app.drag, SplitDrag::None);
        assert_eq!(effect, Effect::None);
        assert_eq!(app.focus, FocusPane::Right);
    }

    #[test]
    fn mouse_hscroll_pans_graph_under_cursor_without_focus_steal() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        let model = GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: format!("subject-{}", "y".repeat(80)),
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
                ..Commit::default()
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        app.focus = FocusPane::Left;
        app.drill = DrillView::Graph;
        app.layout.right_x = 48;
        app.layout.diff_pane_width = 20;
        assert_eq!(app.right_col_offset, 0);
        assert_eq!(
            app.dispatch(Action::ScrollWheel {
                col: 80,
                row: 4,
                delta: 5,
                horizontal: true,
            }),
            Effect::None
        );
        assert!(
            app.right_col_offset > 0,
            "hscroll over the graph must pan even when the tree is focused"
        );
        assert_eq!(
            app.focus,
            FocusPane::Left,
            "hscroll must not steal left-pane focus"
        );
        app.mouse_enabled = false;
        let panned = app.right_col_offset;
        app.dispatch(Action::ScrollWheel {
            col: 80,
            row: 4,
            delta: 5,
            horizontal: true,
        });
        assert_eq!(app.right_col_offset, panned, "mouse off ignores hscroll");
    }

    /// State whose UI-state store is `state/ui-state.json` in a fresh temp dir.
    fn state_with_ui_store(prefix: &str) -> (AppState, PathBuf) {
        let dir = crate::testutil::unique_dir(prefix);
        let mut app = state();
        app.ui_store_path = dir.join("state").join("ui-state.json");
        (app, dir)
    }

    /// Open Quick Open in commands mode (`:`) and type `query`.
    fn palette_type(app: &mut AppState, query: &str) {
        use super::super::action::QuickOpenEntry;
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in query.chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
    }

    /// Title of the highlighted palette row.
    fn palette_focus(app: &AppState) -> Option<String> {
        app.command_palette()
            .and_then(|palette| palette.selected())
            .map(|command| command.title.to_string())
    }

    fn backspace(app: &mut AppState, count: usize) {
        for _ in 0..count {
            app.dispatch(Action::QuickOpenBackspace);
        }
    }

    #[test]
    fn set_theme_applies_saves_and_names_the_theme() {
        let (mut app, dir) = state_with_ui_store("ws-theme-set-save");
        app.theme = ThemeId::TokyoNight;
        assert_eq!(app.dispatch(Action::SetTheme(ThemeId::Slate)), Effect::None);
        assert_eq!(app.theme, ThemeId::Slate);
        assert_eq!(app.status, "theme: Slate");
        assert_eq!(app.status.kind(), StatusKind::Info);
        let text = std::fs::read_to_string(&app.ui_store_path).expect("store written");
        assert!(text.contains("\"slate\""), "store names slate: {text}");
        assert_eq!(
            load_ui_state(&app.ui_store_path).theme,
            Some(ThemeId::Slate)
        );
        assert_eq!(
            app.dispatch(Action::SetTheme(ThemeId::GithubDarkDimmed)),
            Effect::None
        );
        assert_eq!(app.status, "theme: GitHub Dark Dimmed");
        assert_eq!(
            load_ui_state(&app.ui_store_path).theme,
            Some(ThemeId::GithubDarkDimmed)
        );
        assert_eq!(
            theme_id_from(Some("gruvbox-dark"), None, None),
            ThemeId::GruvboxDark
        );
        assert_eq!(theme_id_from(Some("nope"), None, None), ThemeId::Slate);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_theme_names_save_failure_when_unwritable() {
        let mut app = state();
        app.theme = ThemeId::TokyoNight;
        let (root, path) = unwritable_store_path("ws-theme-save-fail");
        app.ui_store_path = path;
        assert_eq!(
            app.dispatch(Action::SetTheme(ThemeId::Monokai)),
            Effect::None
        );
        assert_eq!(app.theme, ThemeId::Monokai, "session theme still changes");
        assert_eq!(app.status.kind(), StatusKind::Error);
        assert!(
            app.status.contains("theme save failed"),
            "failed persist must name the theme store: {}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn set_theme_leaves_foreign_store_file_unchanged() {
        let (mut app, dir) = state_with_ui_store("ws-theme-foreign");
        std::fs::create_dir_all(app.ui_store_path.parent().unwrap()).unwrap();
        let foreign = b"{\"editor\": \"vim\"}\n";
        std::fs::write(&app.ui_store_path, foreign).unwrap();
        app.theme = ThemeId::TokyoNight;
        assert_eq!(
            app.dispatch(Action::SetTheme(ThemeId::Monokai)),
            Effect::None
        );
        assert_eq!(app.theme, ThemeId::Monokai, "session theme still changes");
        assert_eq!(std::fs::read(&app.ui_store_path).unwrap(), foreign);
        assert_eq!(app.status.kind(), StatusKind::Error);
        assert!(
            app.status.contains("theme save failed"),
            "foreign store must surface an error: {}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn set_theme_keeps_a_stored_activity_sparkline() {
        let (mut app, dir) = state_with_ui_store("ws-theme-keeps-sparkline");
        std::fs::create_dir_all(app.ui_store_path.parent().unwrap()).unwrap();
        std::fs::write(
            &app.ui_store_path,
            "{\"version\": 1, \"theme\": \"dracula\", \"activitySparkline\": true}\n",
        )
        .unwrap();
        assert_eq!(app.dispatch(Action::SetTheme(ThemeId::Nord)), Effect::None);
        assert_eq!(app.status.kind(), StatusKind::Info);
        assert_eq!(
            std::fs::read_to_string(&app.ui_store_path).unwrap(),
            "{\n  \"version\": 1,\n  \"theme\": \"nord\",\n  \"activitySparkline\": true\n}\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The toggle writes the store at once and keeps the stored theme; a
    /// second toggle writes it back off.
    #[test]
    fn toggle_activity_sparkline_saves_at_once_and_keeps_the_theme() {
        let (mut app, dir) = state_with_ui_store("ws-sparkline-toggle-save");
        std::fs::create_dir_all(app.ui_store_path.parent().unwrap()).unwrap();
        std::fs::write(
            &app.ui_store_path,
            "{\"version\": 1, \"theme\": \"dracula\"}\n",
        )
        .unwrap();
        assert!(!app.activity_sparkline);
        let effect = app.dispatch(Action::ToggleActivitySparkline);
        assert!(app.activity_sparkline);
        assert!(matches!(effect, Effect::LoadActivity { .. }), "{effect:?}");
        assert_eq!(app.status, "Activity sparkline on");
        assert_eq!(app.status.kind(), StatusKind::Info);
        let stored = load_ui_state(&app.ui_store_path);
        assert!(stored.activity_sparkline, "saved on");
        assert_eq!(stored.theme, Some(ThemeId::Dracula), "theme kept");

        assert_eq!(app.dispatch(Action::ToggleActivitySparkline), Effect::None);
        assert!(!app.activity_sparkline);
        assert_eq!(app.status, "Activity sparkline off");
        let stored = load_ui_state(&app.ui_store_path);
        assert!(!stored.activity_sparkline, "saved off");
        assert_eq!(stored.theme, Some(ThemeId::Dracula), "theme kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A failed save names the sparkline store; the session still flips.
    #[test]
    fn toggle_activity_sparkline_names_save_failure_and_still_flips() {
        let mut app = state();
        let (root, path) = unwritable_store_path("ws-sparkline-save-fail");
        app.ui_store_path = path;
        let effect = app.dispatch(Action::ToggleActivitySparkline);
        assert!(app.activity_sparkline, "session still flips");
        assert!(matches!(effect, Effect::LoadActivity { .. }), "{effect:?}");
        assert_eq!(app.status.kind(), StatusKind::Error);
        assert!(
            app.status.contains("sparkline save failed"),
            "failed persist must name the sparkline: {}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn launch_takes_the_ui_store_path_and_its_sparkline() {
        let dir = crate::testutil::unique_dir("ws-launch-ui-store");
        let ui_store = dir.join("ui-state.json");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &ui_store,
            "{\"version\": 1, \"theme\": \"nord\", \"activitySparkline\": true}\n",
        )
        .unwrap();
        let (viewed_s, comments_s, ui_store_s) = (
            dir.join("viewed.json").display().to_string(),
            dir.join("comments.json").display().to_string(),
            ui_store.display().to_string(),
        );
        let settings = Settings::resolve(&Default::default(), |key| match key {
            "WS_STATUS_VIEWED_STORE" => Some(viewed_s.clone()),
            "WS_STATUS_COMMENT_STORE" => Some(comments_s.clone()),
            "WS_STATUS_UI_STORE" => Some(ui_store_s.clone()),
            _ => None,
        });
        let snapshot = build_workspace_snapshot(&[tree_repo()], &[], false, &[]);
        let app = AppState::launch(PathBuf::from("/tmp"), snapshot, &settings);
        assert_eq!(app.ui_store_path, ui_store);
        assert_eq!(app.theme, ThemeId::Nord);
        assert!(app.activity_sparkline, "the stored sparkline loads on");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn palette_moves_preview_theme_rows_and_restore_on_other_rows() {
        let (mut app, dir) = state_with_ui_store("ws-theme-preview-move");
        app.theme = ThemeId::Slate;
        palette_type(&mut app, "");
        assert_eq!(app.theme, ThemeId::Slate, "open lands on a non-theme row");
        let palette = app.command_palette().expect("palette open");
        let tokyo = palette
            .rows()
            .iter()
            .position(|command| command.title == "Theme: Tokyo Night")
            .expect("Tokyo Night row");
        let delta = tokyo as i32 - palette.cursor as i32;
        app.dispatch(Action::QuickOpenMove(delta));
        assert_eq!(palette_focus(&app).as_deref(), Some("Theme: Tokyo Night"));
        assert_eq!(app.theme, ThemeId::TokyoNight);
        app.dispatch(Action::QuickOpenMove(1));
        assert_eq!(app.theme, ThemeId::Monokai);
        app.dispatch(Action::QuickOpenMove(-2));
        assert_eq!(palette_focus(&app).as_deref(), Some("Icon popover"));
        assert_eq!(app.theme, ThemeId::Slate, "a non-theme row restores");
        app.dispatch(Action::QuickOpenMove(1));
        assert_eq!(app.theme, ThemeId::TokyoNight);
        assert!(!app.ui_store_path.exists(), "a preview never saves");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn palette_typing_previews_the_focused_theme_row() {
        let (mut app, dir) = state_with_ui_store("ws-theme-preview-type");
        app.theme = ThemeId::Slate;
        palette_type(&mut app, "nord");
        assert_eq!(palette_focus(&app).as_deref(), Some("Theme: Nord"));
        assert_eq!(app.theme, ThemeId::Nord);
        backspace(&mut app, "nord".len());
        app.dispatch(Action::QuickOpenChar('q'));
        app.dispatch(Action::QuickOpenChar('u'));
        app.dispatch(Action::QuickOpenChar('i'));
        app.dispatch(Action::QuickOpenChar('t'));
        assert_eq!(palette_focus(&app).as_deref(), Some("Quit"));
        assert_eq!(app.theme, ThemeId::Slate, "typing onto Quit restores");
        backspace(&mut app, "quit".len());
        for c in "dracula".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        assert_eq!(app.theme, ThemeId::Dracula);
        // Backspace over `>` leaves commands mode for files mode.
        backspace(&mut app, "dracula".len() + 1);
        assert!(app.quick_open.is_some());
        assert!(app.command_palette().is_none(), "files mode");
        assert_eq!(app.theme, ThemeId::Slate, "files mode restores");
        assert!(!app.ui_store_path.exists(), "a preview never saves");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn palette_esc_and_toggle_close_restore_the_open_time_theme() {
        use super::super::action::QuickOpenEntry;
        let (mut app, dir) = state_with_ui_store("ws-theme-preview-close");
        app.theme = ThemeId::Kanagawa;
        // Several previews in a row; Esc goes back to the theme before the
        // first one, not to an earlier preview.
        palette_type(&mut app, "dracula");
        assert_eq!(app.theme, ThemeId::Dracula);
        backspace(&mut app, "dracula".len());
        for c in "nord".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        assert_eq!(app.theme, ThemeId::Nord);
        app.dispatch(Action::QuickOpenCancel);
        assert!(app.quick_open.is_none());
        assert_eq!(app.theme, ThemeId::Kanagawa, "Esc restores");

        palette_type(&mut app, "everforest");
        assert_eq!(app.theme, ThemeId::Everforest);
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        assert!(app.quick_open.is_none());
        assert_eq!(app.theme, ThemeId::Kanagawa, "toggle close restores");

        // Files entry, then `>` into commands on the first keystroke.
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files));
        for c in ">one".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        assert_eq!(palette_focus(&app).as_deref(), Some("Theme: One Dark"));
        assert_eq!(app.theme, ThemeId::OneDark);
        app.dispatch(Action::QuickOpenCancel);
        assert_eq!(app.theme, ThemeId::Kanagawa, "Esc after a files entry");
        assert!(!app.ui_store_path.exists(), "a close never saves");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn palette_enter_on_a_theme_row_applies_and_saves_it() {
        let (mut app, dir) = state_with_ui_store("ws-theme-preview-enter");
        app.theme = ThemeId::Slate;
        palette_type(&mut app, "nord");
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert_eq!(app.theme, ThemeId::Nord);
        assert_eq!(app.status, "theme: Nord");
        assert_eq!(load_ui_state(&app.ui_store_path).theme, Some(ThemeId::Nord));
        // The applied theme is the new open-time theme.
        palette_type(&mut app, "dracula");
        assert_eq!(app.theme, ThemeId::Dracula);
        app.dispatch(Action::QuickOpenCancel);
        assert_eq!(app.theme, ThemeId::Nord);
        assert_eq!(load_ui_state(&app.ui_store_path).theme, Some(ThemeId::Nord));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn palette_enter_on_a_theme_row_keeps_it_when_the_save_fails() {
        let mut app = state();
        app.theme = ThemeId::Slate;
        let (root, path) = unwritable_store_path("ws-theme-preview-fail");
        app.ui_store_path = path;
        palette_type(&mut app, "rose-pine");
        assert_eq!(palette_focus(&app).as_deref(), Some("Theme: Rosé Pine"));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert_eq!(app.theme, ThemeId::RosePine, "applied in this session");
        assert_eq!(app.status.kind(), StatusKind::Error);
        assert!(
            app.status.contains("theme save failed"),
            "failed persist must name the theme store: {}",
            app.status
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn palette_enter_on_another_row_after_a_preview_restores_and_saves_nothing() {
        let (mut app, dir) = state_with_ui_store("ws-theme-preview-other");
        app.theme = ThemeId::Slate;
        palette_type(&mut app, "nord");
        assert_eq!(app.theme, ThemeId::Nord);
        backspace(&mut app, "nord".len());
        for c in "keymap".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        assert_eq!(palette_focus(&app).as_deref(), Some("Keymap help"));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert!(app.help_open, "Keymap help ran");
        assert_eq!(app.theme, ThemeId::Slate);
        assert!(!app.ui_store_path.exists(), "nothing saved");
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn tree_repo() -> RepoSnapshot {
        RepoSnapshot {
            repo: "app".into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: false,
            changes: vec![
                FileChange {
                    path: "src/lib.rs".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                },
                FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                },
            ],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    fn tree_app() -> AppState {
        let snapshot = build_workspace_snapshot(&[tree_repo()], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn focus_id(app: &mut AppState, id: &str) {
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing {id}"));
    }

    #[test]
    fn tree_mode_default_has_dir_and_basename() {
        let app = tree_app();
        assert!(app.tree_mode);
        let dir = app
            .rows
            .iter()
            .find(|row| row.id == "dir:app:src")
            .expect("dir");
        assert_eq!(dir.kind, NodeKind::Dir);
        assert!(dir.foldable);
        let lib = app
            .rows
            .iter()
            .find(|row| row.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("lib.rs"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert!(app.rows.iter().any(|row| row.id == "file:app:README.md"));
    }

    #[test]
    fn view_defaults_set_launch_modes_and_omitted_keys_keep_in_app_defaults() {
        let mut app = tree_app();
        assert!(app.diff_wrap, "wrap is on by default");
        let rows_before: Vec<String> = app.rows.iter().map(|row| row.id.clone()).collect();
        let cursor_before = app.cursor;

        app.apply_view_defaults(&ViewDefaults::default());
        assert!(app.tree_mode);
        assert!(app.commit_tree_mode);
        assert_eq!(app.diff_mode, DiffMode::SideBySide);
        assert!(app.diff_wrap);
        assert!(app.commit_msg_expand);
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT);
        assert_eq!(
            app.background,
            BackgroundMode::Terminal,
            "omitted background keeps the fixture's terminal pin"
        );
        assert_eq!(
            app.rows
                .iter()
                .map(|row| row.id.clone())
                .collect::<Vec<_>>(),
            rows_before
        );
        assert_eq!(app.cursor, cursor_before);

        app.apply_view_defaults(&ViewDefaults {
            tree: Some(false),
            commit_tree: Some(false),
            diff_split: Some(false),
            wrap: Some(false),
            commit_message_expand: Some(false),
            line_blame: None,
            commit_message_lines: Some(3),
            background: Some(BackgroundMode::Paint),
        });
        assert!(!app.tree_mode);
        assert!(app.rows.iter().all(|row| row.kind != NodeKind::Dir));
        assert!(app.rows.iter().any(|row| row.id == "file:app:src/lib.rs"));
        assert!(app.focused_row().is_some());
        assert!(!app.commit_tree_mode);
        assert_eq!(app.diff_mode, DiffMode::Inline);
        assert!(!app.diff_wrap);
        assert!(!app.commit_msg_expand);
        assert_eq!(app.commit_msg_lines, 3);
        assert_eq!(app.background, BackgroundMode::Paint);
        assert_eq!(app.status, "", "launch defaults post no status");

        app.apply_view_defaults(&ViewDefaults {
            background: Some(BackgroundMode::Terminal),
            ..ViewDefaults::default()
        });
        assert_eq!(app.background, BackgroundMode::Terminal);
    }

    #[test]
    fn launch_with_no_env_and_no_config_paints_background() {
        let viewed = default_viewed_path();
        let comments = viewed.with_extension("comments.json");
        let ui_store = viewed.with_extension("ui-state.json");
        let (viewed_s, comments_s) = (viewed.display().to_string(), comments.display().to_string());
        let ui_store_s = ui_store.display().to_string();
        let settings = Settings::resolve(&Default::default(), |key| match key {
            "WS_STATUS_VIEWED_STORE" => Some(viewed_s.clone()),
            "WS_STATUS_COMMENT_STORE" => Some(comments_s.clone()),
            "WS_STATUS_UI_STORE" => Some(ui_store_s.clone()),
            _ => None,
        });
        let snapshot = build_workspace_snapshot(&[tree_repo()], &[], false, &[]);
        let mut app = AppState::launch(PathBuf::from("/tmp"), snapshot, &settings);
        assert_eq!(app.background, BackgroundMode::Paint);
        assert_eq!(app.theme, ThemeId::Slate, "Slate is the launch default");
        assert!(
            !app.activity_sparkline,
            "the activity sparkline launches off"
        );

        app.apply_view_defaults(&ViewDefaults::default());
        assert_eq!(
            app.background,
            BackgroundMode::Paint,
            "an empty viewDefaults keeps the paint launch default"
        );
    }

    #[test]
    fn t_toggles_flat_full_paths_without_dirs() {
        let mut app = tree_app();
        focus_id(&mut app, "file:app:src/lib.rs");
        assert_eq!(app.dispatch(Action::ToggleTreeMode), Effect::LoadRightPane);
        assert!(!app.tree_mode);
        assert_eq!(app.status, "Flat paths");
        assert!(app.rows.iter().all(|row| row.kind != NodeKind::Dir));
        let lib = app
            .rows
            .iter()
            .find(|row| row.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("lib.rs"));
        assert!(lib.label.contains("src"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert_eq!(
            app.focused_row().map(|row| row.id.as_str()),
            Some("file:app:src/lib.rs")
        );
        app.dispatch(Action::ToggleTreeMode);
        assert!(app.tree_mode);
        assert_eq!(app.status, "Directory tree");
        assert!(app.rows.iter().any(|row| row.id == "dir:app:src"));
        assert_eq!(
            app.focused_row().map(|row| row.id.as_str()),
            Some("file:app:src/lib.rs")
        );
    }

    #[test]
    fn z_on_dir_hides_files() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("dir:app:src"));
        assert!(app.rows.iter().all(|row| row.id != "file:app:src/lib.rs"));
        assert!(app.rows.iter().any(|row| row.id == "file:app:README.md"));
    }

    #[test]
    fn stage_on_dir_skips_checkout_root_sibling() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        match app.dispatch(Action::Stage) {
            Effect::Stage { repo, paths } => {
                assert_eq!(repo, "app");
                assert_eq!(paths, vec!["src/lib.rs"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn t_in_commit_files_toggles_commit_trie_not_workspace() {
        let mut app = tree_app();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        assert!(app.commit_tree_mode);
        assert!(app.tree_mode);
        assert!(app.commit_file_rows().iter().any(|row| row.id == "dir:src"));
        assert_eq!(app.dispatch(Action::ToggleTreeMode), Effect::None);
        assert!(!app.commit_tree_mode);
        assert!(app.tree_mode, "workspace t stays workspace-only");
        assert!(app.rows.iter().any(|row| row.id == "dir:app:src"));
        let rows = app.commit_file_rows();
        assert!(rows.iter().all(|row| !row.is_dir()));
        assert!(rows.iter().any(|row| {
            row.id == "file:src/lib.rs"
                && row.label.contains("lib.rs")
                && row.label.contains("src")
                && !row.label.contains("src/lib.rs")
                && row.trailing.contains('A')
        }));
        app.dispatch(Action::ToggleTreeMode);
        assert!(app.commit_tree_mode);
        assert!(app.commit_file_rows().iter().any(|row| row.id == "dir:src"));
    }

    #[test]
    fn t_from_dir_falls_back_to_checkout() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        app.dispatch(Action::ToggleTreeMode);
        let focused = app.focused_row().expect("row");
        assert!(
            focused.id == "repo:app" || focused.id == "checkout:app",
            "{}",
            focused.id
        );
    }

    fn type_search(app: &mut AppState, query: &str) {
        assert_eq!(app.dispatch(Action::SearchStart), Effect::None);
        for c in query.chars() {
            app.dispatch(Action::SearchChar(c));
        }
        app.dispatch(Action::SearchSubmit);
    }

    fn install_two_graph_commits(app: &mut AppState) {
        let model = GraphModel {
            commits: vec![
                graph_commit("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "alpha unique"),
                graph_commit("ccc3333ddddddddddddddddddddddddddddddddd", "beta unique"),
            ],
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            sync: None,
            show_hidden: app.show_hidden,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    #[test]
    fn graph_search_enter_focuses_subject_and_n_cycles() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_two_graph_commits(&mut app);
        assert_eq!(app.graph_cursor, 0);
        type_search(&mut app, "unique");
        assert!(app.search_active);
        assert_eq!(app.search_target, SearchPane::Graph);
        assert_eq!(app.graph_cursor, 1, "the origin row's own match is skipped");
        app.dispatch(Action::SearchNext);
        assert_eq!(app.graph_cursor, 0);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.graph_cursor, 1);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.graph_cursor, 0);
        assert!(app.status.is_empty(), "{}", &*app.status);
    }

    /// Graph of `subjects`, top to bottom, on the right pane.
    fn install_graph_subjects(app: &mut AppState, subjects: &[&str]) {
        let commits: Vec<Commit> = subjects
            .iter()
            .enumerate()
            .map(|(i, subject)| graph_commit(&format!("{:040x}", i + 1), subject))
            .collect();
        let head = commits[0].id.clone();
        let model = GraphModel {
            commits,
            head_id: Some(head.clone()),
            show_hidden: app.show_hidden,
            ..GraphModel::default()
        };
        app.set_graph(model, "app".into(), head);
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    fn four_graph_rows() -> AppState {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_graph_subjects(
            &mut app,
            &["one unique", "two plain", "three unique", "four unique"],
        );
        app
    }

    #[test]
    fn graph_search_steps_from_the_cursor_and_wraps_both_ways() {
        let mut app = four_graph_rows();
        app.graph_cursor = 1;
        app.dispatch(Action::SearchStart);
        for c in "uni".chars() {
            app.dispatch(Action::SearchChar(c));
            assert_eq!(app.graph_cursor, 2, "preview from the origin after {c}");
        }
        for c in "que".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert_eq!(app.graph_cursor, 2);
        app.dispatch(Action::SearchSubmit);
        assert_eq!(
            app.graph_cursor, 2,
            "Enter keeps the preview, no extra step"
        );
        assert_eq!(app.search_match_position(), Some((Some(2), 3)));
        app.dispatch(Action::SearchNext);
        assert_eq!(app.graph_cursor, 3);
        assert_eq!(app.search_match_position(), Some((Some(3), 3)));
        app.dispatch(Action::SearchNext);
        assert_eq!(app.graph_cursor, 0);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        assert_eq!(app.search_match_position(), Some((Some(1), 3)));
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.graph_cursor, 3);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
        // From a non-match the user moved to: next below, previous above.
        app.graph_cursor = 1;
        app.dispatch(Action::SearchNext);
        assert_eq!(app.graph_cursor, 2);
        assert!(app.status.is_empty(), "{}", &*app.status);
        app.graph_cursor = 1;
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.graph_cursor, 0);
        // Esc on the armed search clears it and leaves the cursor there.
        app.dispatch(Action::NavEsc);
        assert!(!app.search_active);
        assert_eq!(app.graph_cursor, 0);
    }

    #[test]
    fn graph_typing_with_no_hit_leaves_the_cursor_at_the_origin() {
        let mut app = four_graph_rows();
        app.graph_cursor = 1;
        app.dispatch(Action::SearchStart);
        for c in "three".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert_eq!(app.graph_cursor, 2);
        app.dispatch(Action::SearchChar('Z'));
        assert_eq!(app.graph_cursor, 1);
        assert_eq!(app.status, SEARCH_NO_MATCH);
        app.dispatch(Action::SearchCancel);
        assert_eq!(app.graph_cursor, 1);
    }

    #[test]
    fn graph_only_match_on_the_cursor_counts_as_a_wrap() {
        let mut app = four_graph_rows();
        app.graph_cursor = 2;
        type_search(&mut app, "three");
        assert_eq!(app.graph_cursor, 2);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        assert_eq!(app.search_match_position(), Some((Some(1), 1)));
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.graph_cursor, 2);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
    }

    #[test]
    fn graph_search_focuses_author_sha_ref_and_time() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        let now = unix_now();
        let mut hit = graph_commit("aa11bb22cc33dd44ee55ff6677889900aabbccdd", "alpha unique");
        hit.author_name = "Ada SearchAuthor".into();
        hit.author_date_unix = now - 90;
        hit.refs = vec![GraphRef::tag("v9.9.9"), GraphRef::local("topic-search")];
        let miss = graph_commit("ccc3333ddddddddddddddddddddddddddddddddd", "beta unique");
        let model = GraphModel {
            commits: vec![hit, miss],
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some("aa11bb22cc33dd44ee55ff6677889900aabbccdd".into()),
            sync: None,
            show_hidden: app.show_hidden,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(
            model,
            "app".into(),
            "aa11bb22cc33dd44ee55ff6677889900aabbccdd".into(),
        );
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;

        for query in ["SearchAuthor", "aa11bb2", "v9.9.9", "topic-search", "1m"] {
            app.graph_cursor = 1;
            type_search(&mut app, query);
            assert_eq!(
                app.graph_cursor, 0,
                "query {query} should focus the hit row"
            );
        }
    }

    #[test]
    fn commit_files_search_focuses_matching_path() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        type_search(&mut app, "lib");
        let row = app
            .commit_file_rows()
            .get(app.commit_files_cursor())
            .cloned()
            .expect("search row");
        assert_eq!(row.path, "src/lib.rs");
        assert_eq!(app.search_target, SearchPane::CommitFiles);
    }

    #[test]
    fn commit_files_search_walks_the_painted_tree_order() {
        let mut app = summary_drill_app();
        assert!(app.commit_tree_mode);
        let focused = |app: &AppState| app.focused_commit_file_row().map(|row| row.path);
        // Painted: src/, src/deep/, c.rs, a.rs, b.rs, README.md — not the
        // `files` order README, a, b, c.
        assert_eq!(focused(&app).as_deref(), Some("src/a.rs"));
        type_search(&mut app, ".rs");
        assert_eq!(app.search_target, SearchPane::CommitFiles);
        assert_eq!(focused(&app).as_deref(), Some("src/b.rs"));
        assert_eq!(app.search_match_position(), Some((Some(3), 3)));
        app.dispatch(Action::SearchNext);
        assert_eq!(focused(&app).as_deref(), Some("src/deep/c.rs"));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        assert_eq!(app.search_match_position(), Some((Some(1), 3)));
        app.dispatch(Action::SearchNext);
        assert_eq!(focused(&app).as_deref(), Some("src/a.rs"));
        assert_eq!(app.search_match_position(), Some((Some(2), 3)));
        app.dispatch(Action::SearchPrev);
        app.dispatch(Action::SearchPrev);
        assert_eq!(focused(&app).as_deref(), Some("src/b.rs"));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
        // A folder row anchors at its own place in that order.
        let _ = summary_move_to(&mut app, "src");
        app.dispatch(Action::SearchNext);
        assert_eq!(focused(&app).as_deref(), Some("src/deep/c.rs"));
        assert!(app.status.is_empty(), "{}", &*app.status);
        let _ = summary_move_to(&mut app, "src");
        app.dispatch(Action::SearchPrev);
        assert_eq!(focused(&app).as_deref(), Some("src/b.rs"));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
    }

    #[test]
    fn tree_search_steps_from_the_focused_row() {
        let snapshot = build_workspace_snapshot(
            &[repo("app", true), repo("lib", true), repo("web", true)],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let readme = |repo: &str| format!("file:{repo}:README.md");
        let focused = |app: &AppState| app.focused_row().map(|row| row.id.clone());
        let order: Vec<String> = crate::tui::tree::flatten(&app.tree, &HashSet::new())
            .into_iter()
            .map(|row| row.id)
            .collect();
        let at = |id: &str| order.iter().position(|x| x == id).expect(id);
        focus_repo(&mut app, "lib");
        let lib_repo = focused(&app).expect("lib row");
        assert!(at(&readme("app")) < at(&lib_repo) && at(&lib_repo) < at(&readme("lib")));
        type_search(&mut app, "README");
        assert_eq!(focused(&app), Some(readme("lib")), "next below, not first");
        assert_eq!(app.search_match_position(), Some((Some(2), 3)));
        app.dispatch(Action::SearchNext);
        assert_eq!(focused(&app), Some(readme("web")));
        app.dispatch(Action::SearchNext);
        assert_eq!(focused(&app), Some(readme("app")));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        app.dispatch(Action::SearchPrev);
        assert_eq!(focused(&app), Some(readme("web")));
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
        focus_repo(&mut app, "lib");
        app.dispatch(Action::SearchPrev);
        assert_eq!(focused(&app), Some(readme("app")), "previous above");
    }

    #[test]
    fn tree_typing_recomputes_folds_from_the_origin() {
        let mut app = state();
        app.folds.insert("repo:app".into());
        app.rebuild_rows();
        let origin = app.focused_row().map(|row| row.id.clone());
        app.dispatch(Action::SearchStart);
        for c in "README".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
        app.dispatch(Action::SearchChar('Z'));
        assert!(app.folds.contains("repo:app"), "no stray unfold");
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
        assert_eq!(app.focused_row().map(|row| row.id.clone()), origin);
        assert_eq!(app.status, SEARCH_NO_MATCH);
    }

    #[test]
    fn diff_search_sets_hit_and_scrolls() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec![
                "diff --git a/README.md b/README.md".into(),
                "@@ -1,3 +1,4 @@".into(),
                " context".into(),
                "+needle-unique".into(),
                " more".into(),
            ]),
        );
        type_search(&mut app, "needle-unique");
        assert_eq!(app.search_target, SearchPane::Diff);
        assert_eq!(app.search_hit, Some(3));
        assert!(app.diff_scroll <= 3);
    }

    #[test]
    fn tree_search_still_unfolds_parents() {
        let mut app = state();
        app.folds.insert("repo:app".into());
        app.rebuild_rows();
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
        type_search(&mut app, "README");
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
        assert_eq!(
            app.focused_row().map(|r| r.id.as_str()),
            Some("file:app:README.md")
        );
    }

    #[test]
    fn ctrl_o_toggles_unlimited_context_and_restores() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        assert_eq!(app.diff_context_lines(), None);
        assert!(!app.full_context_active());
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.diff_context_lines(), Some(FULL_DIFF_CONTEXT_LINES));
        assert!(app.full_context_active());
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.diff_context_lines(), None);
        assert!(!app.full_context_active());
    }

    #[test]
    fn ctrl_o_reload_keeps_hunk_text_in_view() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.layout.diff_pane_height = 8;
        let hunk_only = DiffContent::from_unified(
            "@@ -46,7 +46,7 @@\n pad-a\n pad-b\n pad-c\n-WIDE-HUNK-BASE\n+WIDE-HUNK-NEEDLE\n pad-d\n pad-e\n pad-f\n",
        );
        app.set_diff("app".into(), "README.md".into(), hunk_only);
        assert_eq!(app.diff_scroll, 0);
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        let mut full = String::from("@@ -1,40 +1,40 @@\n");
        for i in 0..30 {
            full.push_str(&format!(" HEAD-{i:02}\n"));
        }
        full.push_str(
            " pad-a\n pad-b\n pad-c\n-WIDE-HUNK-BASE\n+WIDE-HUNK-NEEDLE\n pad-d\n pad-e\n pad-f\n",
        );
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(full),
        );
        let rows = app.current_diff_rows();
        let start = app.diff_scroll as usize;
        let end = (start + 8).min(rows.len());
        let visible: Vec<String> = rows[start..end].iter().map(row_search_text).collect();
        assert!(
            visible
                .iter()
                .any(|t| t.contains("WIDE-HUNK-NEEDLE") || t.contains("WIDE-HUNK-BASE")),
            "hunk must stay in view, got scroll={start} visible={visible:?}"
        );
        assert!(
            visible.iter().all(|t| !t.contains("HEAD-00")),
            "file start must not replace the hunk, got scroll={start} visible={visible:?}"
        );
        assert!(app.diff_scroll > 0, "full-file must not stay at scroll 0");
    }

    #[test]
    fn ctrl_o_on_tree_or_graph_is_noop() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Left;
        assert_eq!(app.dispatch(Action::ToggleFullContext), Effect::None);
        install_graph(&mut app, Vec::new());
        app.focus = FocusPane::Right;
        assert_eq!(app.dispatch(Action::ToggleFullContext), Effect::None);
    }

    #[test]
    fn open_commit_files_reload_keeps_focused_path() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_commit_files("app".into(), sample_commit_source(), two_commit_files());
        match app.dispatch(Action::Move(1)) {
            Effect::LoadRightPane | Effect::None => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.focused_commit_file_row().unwrap().path, "two.rs");
        let mut files = two_commit_files();
        files.insert(
            0,
            CommitFile {
                status: "A".into(),
                path: "zero.rs".into(),
                old_path: None,
                stat: None,
            },
        );
        app.open_commit_files("app".into(), sample_commit_source(), files);
        assert_eq!(app.focused_commit_file_row().unwrap().path, "two.rs");
        assert_ne!(
            app.commit_files_cursor(),
            0,
            "same-source reload must not jump to the first file"
        );
    }

    #[test]
    fn open_commit_files_gone_path_clamps_to_neighbor() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.open_commit_files(
            "app".into(),
            sample_commit_source(),
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "a.rs".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "M".into(),
                    path: "b.rs".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "M".into(),
                    path: "c.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        match app.dispatch(Action::Move(1)) {
            Effect::LoadRightPane | Effect::None => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.focused_commit_file_row().unwrap().path, "b.rs");
        app.open_commit_files(
            "app".into(),
            sample_commit_source(),
            vec![
                CommitFile {
                    status: "M".into(),
                    path: "a.rs".into(),
                    old_path: None,
                    stat: None,
                },
                CommitFile {
                    status: "M".into(),
                    path: "c.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        assert_eq!(app.focused_commit_file_row().unwrap().path, "c.rs");
        assert_ne!(
            app.commit_files_cursor(),
            0,
            "gone commit-file path must clamp, not jump to the first file"
        );
    }

    #[test]
    fn set_diff_same_path_keeps_viewport() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), tall_panning_diff());
        pan_and_scroll_focused_diff(&mut app);
        let cursor = app.diff_cursor;
        let scroll = app.diff_scroll;
        let pan = app.diff_col_offset;
        app.set_diff("app".into(), "README.md".into(), tall_panning_diff());
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.diff_scroll, scroll);
        assert_eq!(app.diff_col_offset, pan);
    }

    #[test]
    fn set_diff_new_path_resets_viewport() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), tall_panning_diff());
        pan_and_scroll_focused_diff(&mut app);
        app.set_diff("app".into(), "src/lib.rs".into(), tall_panning_diff());
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.diff_scroll, 0);
        assert_eq!(app.diff_col_offset, 0);
    }

    #[test]
    fn open_commit_diff_new_path_resets_viewport() {
        let mut app = state();
        focus_repo(&mut app, "app");
        let files = two_commit_files();
        let source = sample_commit_source();
        app.open_commit_diff(
            "app".into(),
            source.clone(),
            files.clone(),
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        pan_and_scroll_focused_diff(&mut app);
        app.open_commit_diff(
            "app".into(),
            source,
            files,
            1,
            "two.rs".into(),
            tall_panning_diff(),
        );
        assert_eq!(app.diff_cursor, 0, "new commit file must drop the old row");
        assert_eq!(app.diff_scroll, 0, "new commit file must start at the top");
        assert_eq!(
            app.diff_col_offset, 0,
            "new commit file must start at the left"
        );
    }

    #[test]
    fn open_commit_diff_same_path_keeps_viewport() {
        let mut app = state();
        focus_repo(&mut app, "app");
        let files = two_commit_files();
        let source = sample_commit_source();
        app.open_commit_diff(
            "app".into(),
            source.clone(),
            files.clone(),
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        pan_and_scroll_focused_diff(&mut app);
        let cursor = app.diff_cursor;
        let scroll = app.diff_scroll;
        let pan = app.diff_col_offset;
        app.open_commit_diff(
            "app".into(),
            source,
            files,
            0,
            "one.rs".into(),
            tall_panning_diff(),
        );
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.diff_scroll, scroll);
        assert_eq!(app.diff_col_offset, pan);
    }

    #[test]
    fn workspace_then_commit_same_path_resets_viewport() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.set_diff("app".into(), "README.md".into(), tall_panning_diff());
        pan_and_scroll_focused_diff(&mut app);
        app.open_commit_diff(
            "app".into(),
            sample_commit_source(),
            sample_commit_files(),
            0,
            "README.md".into(),
            tall_panning_diff(),
        );
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.diff_scroll, 0);
        assert_eq!(app.diff_col_offset, 0);
    }

    #[test]
    fn focused_diff_l_increases_pan_h_decreases_tree_still_folds() {
        let mut app = state();
        app.diff_wrap = false;
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.layout.diff_pane_width = 8;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(format!("@@ -0,0 +1,1 @@\n+{}", "x".repeat(40))),
        );
        assert_eq!(app.diff_col_offset, 0);
        app.dispatch(Action::PanDiff(1));
        assert!(app.diff_col_offset > 0, "l increases pan offset");
        let after_l = app.diff_col_offset;
        app.dispatch(Action::PanDiff(-1));
        assert!(app.diff_col_offset < after_l, "h decreases pan offset");
        app.dispatch(Action::PanDiff(-1));
        app.dispatch(Action::PanDiff(-1));
        assert_eq!(app.diff_col_offset, 0);

        let mut tree = state();
        focus_repo(&mut tree, "app");
        tree.focus = FocusPane::Left;
        tree.dispatch(Action::FoldClose);
        assert!(tree.folds.contains("repo:app"));
        tree.dispatch(Action::FoldOpen);
        assert!(!tree.folds.contains("repo:app"));
    }

    #[test]
    fn mouse_hscroll_over_left_pane_pans_long_diff() {
        let mut app = state();
        app.diff_wrap = false;
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Left;
        app.layout.right_x = 48;
        app.layout.diff_pane_width = 8;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_unified(format!("@@ -0,0 +1,1 @@\n+{}", "x".repeat(40))),
        );
        assert_eq!(app.diff_col_offset, 0);
        assert_eq!(
            app.dispatch(Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 5,
                horizontal: true,
            }),
            Effect::None
        );
        assert!(
            app.diff_col_offset > 0,
            "hscroll over the left pane must pan a long file diff"
        );
        assert_eq!(
            app.focus,
            FocusPane::Left,
            "hscroll must not steal tree focus"
        );
        assert_eq!(app.left_col_offset, 0, "short tree labels stay unpanned");
    }

    #[test]
    fn list_pan_shifts_tree_and_graph_without_stealing_fold() {
        let mut snap = repo("app", true);
        snap.changes = vec![FileChange {
            path: format!("deep/nested/{}/tail.rs", "x".repeat(40)),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        let mut tree = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_file(&mut tree, "tail.rs");
        tree.focus = FocusPane::Left;
        tree.layout.tree_width = 16;
        assert_eq!(tree.left_col_offset, 0);
        tree.dispatch(Action::PanDiff(4));
        assert!(
            tree.left_col_offset > 0,
            "Shift/pan should reveal a long tree path"
        );
        let panned = tree.left_col_offset;
        tree.dispatch(Action::PanDiff(-20));
        assert_eq!(tree.left_col_offset, 0);
        focus_repo(&mut tree, "app");
        tree.dispatch(Action::FoldClose);
        assert!(tree.folds.contains("repo:app"));
        tree.dispatch(Action::FoldOpen);
        assert!(!tree.folds.contains("repo:app"));
        assert_eq!(tree.left_col_offset, 0);
        tree.dispatch(Action::PanDiff(1));
        assert!(panned > 0);

        let mut graph = state();
        focus_repo(&mut graph, "app");
        let model = GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: format!("subject-{}", "y".repeat(80)),
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
                ..Commit::default()
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: None,
            ..GraphModel::default()
        };
        graph.set_graph(
            model,
            "app".into(),
            "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        );
        graph.focus = FocusPane::Right;
        graph.drill = DrillView::Graph;
        graph.layout.diff_pane_width = 20;
        graph.dispatch(Action::PanDiff(5));
        assert!(
            graph.right_col_offset > 0,
            "h/l on a focused graph should pan a long subject"
        );
    }

    #[test]
    fn tree_mouse_hscroll_pans_without_moving_cursor() {
        let mut snap = repo("app", true);
        snap.changes = vec![FileChange {
            path: format!("deep/nested/{}/tail.rs", "x".repeat(40)),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus_file(&mut app, "tail.rs");
        app.focus = FocusPane::Left;
        app.layout.tree_width = 16;
        app.layout.right_x = 40;
        let cursor = app.cursor;
        let id = app.rows[cursor].id.clone();
        assert_eq!(app.left_col_offset, 0);

        assert_eq!(
            app.dispatch(Action::ScrollWheel {
                col: 8,
                row: 4,
                delta: 4,
                horizontal: true,
            }),
            Effect::None
        );
        assert_eq!(app.cursor, cursor, "hscroll must not move the tree cursor");
        assert_eq!(app.rows[app.cursor].id, id);
        assert_eq!(app.focus, FocusPane::Left);
        assert!(
            app.left_col_offset > 0,
            "tree mouse hscroll should pan a long path"
        );

        app.focus = FocusPane::Right;
        let panned = app.left_col_offset;
        app.dispatch(Action::ScrollWheel {
            col: 8,
            row: 4,
            delta: 2,
            horizontal: true,
        });
        assert_eq!(app.cursor, cursor);
        assert_eq!(
            app.focus,
            FocusPane::Right,
            "hscroll must not steal pane focus"
        );
        assert!(
            app.left_col_offset >= panned,
            "hscroll over the tree pans the tree even when the right pane is focused"
        );

        app.focus = FocusPane::Left;
        let after_pan = app.cursor;
        app.dispatch(Action::ScrollWheel {
            col: 8,
            row: 4,
            delta: -1,
            horizontal: false,
        });
        assert_ne!(
            app.cursor, after_pan,
            "vertical wheel over the tree still moves the cursor"
        );

        let mut fold = AppState::new(
            PathBuf::from("/tmp"),
            build_workspace_snapshot(&[repo("app", true)], &[], false, &[]),
            true,
        );
        focus_repo(&mut fold, "app");
        fold.dispatch(Action::FoldClose);
        assert!(fold.folds.contains("repo:app"));
        fold.dispatch(Action::FoldOpen);
        assert!(!fold.folds.contains("repo:app"));
    }

    #[test]
    fn full_context_survives_tree_focus_and_commit_file_list() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        app.focus = FocusPane::Right;
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        assert_eq!(
            app.dispatch(Action::ToggleFullContext),
            Effect::LoadRightPane
        );
        app.focus = FocusPane::Left;
        assert_eq!(app.diff_context_lines(), Some(FULL_DIFF_CONTEXT_LINES));
        assert_eq!(
            app.workspace_diff_context("app", "README.md"),
            Some(FULL_DIFF_CONTEXT_LINES)
        );
        match app.dispatch(Action::ToggleFullContext) {
            Effect::LoadRightPane => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(app.diff_context_lines(), None);

        let mut commit = state();
        focus_repo(&mut commit, "app");
        commit.focus = FocusPane::Right;
        commit.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
            0,
            "README.md".into(),
            DiffContent::from_lines(vec!["@@ -1,1 +1,2 @@".into(), "+line".into()]),
        );
        match commit.dispatch(Action::ToggleFullContext) {
            Effect::LoadCommitDiff { path, .. } => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
        assert_eq!(commit.diff_context_lines(), Some(FULL_DIFF_CONTEXT_LINES));
        commit.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
        );
        assert_eq!(commit.diff_context_lines(), None);
        assert_eq!(
            commit.commit_diff_context("app", "README.md"),
            Some(FULL_DIFF_CONTEXT_LINES)
        );
    }

    #[test]
    fn help_then_slash_starts_help_search_not_pane_search() {
        let mut app = state();
        let cursor = app.cursor;
        app.dispatch(Action::ToggleHelp);
        assert!(app.help_open);
        assert_eq!(app.input_mode(), InputMode::Help);
        assert_eq!(
            crate::tui::keys::event_to_action(
                &crossterm::event::Event::Key(crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Char('/'),
                    crossterm::event::KeyModifiers::NONE,
                )),
                app.input_mode(),
                false,
                false,
            ),
            Action::SearchStart
        );
        app.dispatch(Action::SearchStart);
        assert!(app.help_search_query.is_some());
        assert!(!app.search_mode);
        assert!(!app.search_active);
        assert_eq!(app.input_mode(), InputMode::HelpSearch);
        app.dispatch(Action::SearchChar('q'));
        app.dispatch(Action::SearchChar('u'));
        app.dispatch(Action::SearchChar('i'));
        assert_eq!(app.help_search_query.as_deref(), Some("qui"));
        assert!(!app.search_mode);
        app.dispatch(Action::SearchChar('n'));
        assert_eq!(app.help_search_query.as_deref(), Some("quin"));
        app.dispatch(Action::SearchSubmit);
        assert_eq!(app.help_search_query.as_deref(), Some("quin"));
        app.dispatch(Action::SearchNext);
        app.dispatch(Action::SearchPrev);
        assert_eq!(app.help_search_query.as_deref(), Some("quin"));
        app.dispatch(Action::SearchCancel);
        assert!(app.help_open);
        assert!(app.help_search_query.is_none());
        assert_eq!(app.cursor, cursor, "help search never moves a pane cursor");
        assert!(!app.search_is_armed());
        assert!(app.search_origin.is_none());
    }

    #[test]
    fn slash_without_help_still_starts_pane_search() {
        let mut app = state();
        assert!(!app.help_open);
        app.dispatch(Action::SearchStart);
        assert!(app.search_mode);
        assert!(app.help_search_query.is_none());
        app.dispatch(Action::SearchChar('R'));
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_active);
        assert!(!app.search_mode);
    }

    #[test]
    fn enter_keeps_graph_and_shows_commit_meta() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(&mut app, Vec::new());
        if let Some(model) = app.graph.as_mut() {
            model.commits[0].author_name = "Ada".into();
            model.commits[0].author_date_unix = unix_now() - 3600;
            model.commits[0].refs = vec![GraphRef::local("main")];
        }
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
        );
        assert!(app.graph.is_some());
        assert!(app.in_commit_drill());
        let (title, subtitle) = app.commit_detail_meta();
        assert_eq!(title, "app");
        let subtitle = subtitle.expect("subtitle");
        assert!(subtitle.contains("aaa1111"), "{subtitle}");
        assert!(subtitle.contains("main"), "{subtitle}");
        assert!(subtitle.contains("head"), "{subtitle}");
        assert!(subtitle.contains("Ada"), "{subtitle}");
        assert!(
            subtitle.contains("1h") || subtitle.contains("59m") || subtitle.contains("just now"),
            "{subtitle}"
        );
        assert!(app
            .commit_file_rows()
            .iter()
            .any(|row| row.path == "src/lib.rs"));
    }

    #[test]
    fn e_on_commit_file_and_diff_emits_edit() {
        let mut app = state();
        focus_repo(&mut app, "app");
        app.focus = FocusPane::Right;
        app.open_commit_files(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
        );
        let idx = app
            .commit_file_rows()
            .iter()
            .position(|row| row.path == "src/lib.rs")
            .expect("file");
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = idx;
        }
        match app.dispatch(Action::Edit) {
            Effect::EditFile { repo, path, .. } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "src/lib.rs");
            }
            other => panic!("{other:?}"),
        }
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![CommitFile {
                status: "A".into(),
                path: "src/lib.rs".into(),
                old_path: None,
                stat: None,
            }],
            idx,
            "src/lib.rs".into(),
            DiffContent::from_lines(vec!["+fn x() {}".into()]),
        );
        match app.dispatch(Action::Edit) {
            Effect::EditFile { repo, path, .. } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "src/lib.rs");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn workspace_t_unchanged_outside_commit_drill() {
        let mut app = tree_app();
        assert!(app.tree_mode);
        assert!(app.commit_tree_mode);
        app.dispatch(Action::ToggleTreeMode);
        assert!(!app.tree_mode);
        assert!(app.commit_tree_mode);
        assert!(app.drill.is_graph());
    }
    #[test]
    fn single_z_folds_only_focused_row() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("dir:app:src"));
        assert!(!app.folds.contains("repo:app"));
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn zz_second_z_is_toggle_subtree() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("repo:app"));
        assert!(!app.folds.contains("dir:app:src"));
        app.dispatch(Action::FoldToggleSubtree);
        assert!(
            app.folds.contains("repo:app"),
            "second z must not toggle the parent a second time"
        );
        assert!(
            app.folds.contains("dir:app:src"),
            "second z folds descendants to match the folded parent"
        );
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
        assert!(app.rows.iter().all(|r| r.id != "file:app:README.md"));
    }

    #[test]
    fn zz_on_a_folded_parent_opens_the_subtree() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        app.dispatch(Action::FoldToggle);
        app.dispatch(Action::FoldToggleSubtree);
        assert!(app.folds.contains("repo:app"));
        assert!(app.folds.contains("dir:app:src"));
        app.dispatch(Action::FoldToggle);
        assert!(!app.folds.contains("repo:app"));
        assert!(app.folds.contains("dir:app:src"));
        app.dispatch(Action::FoldToggleSubtree);
        assert!(!app.folds.contains("repo:app"));
        assert!(
            !app.folds.contains("dir:app:src"),
            "second z opens descendants to match the open parent"
        );
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
        assert!(app.rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn fold_toggle_subtree_does_not_toggle_the_focused_row() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        app.dispatch(Action::FoldToggleSubtree);
        assert!(
            !app.folds.contains("repo:app"),
            "FoldToggleSubtree must not toggle the parent (first z already did)"
        );
        assert!(!app.folds.contains("dir:app:src"));
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
    }

    #[test]
    fn expired_z_is_a_new_single_toggle() {
        let mut app = tree_app();
        focus_id(&mut app, "dir:app:src");
        app.dispatch(Action::FoldToggle);
        assert!(app.folds.contains("dir:app:src"));
        app.z_pending_at = Some(Instant::now() - Duration::from_millis(500));
        app.dispatch(Action::FoldToggle);
        assert!(!app.folds.contains("dir:app:src"));
        assert!(!app.folds.contains("repo:app"));
    }

    #[test]
    fn gg_chord_arms_without_moving_then_moves_to_start() {
        let mut app = tree_app();
        app.cursor = 3.min(app.rows.len().saturating_sub(1));
        let before = app.cursor;
        app.dispatch(Action::ArmGChord);
        assert_eq!(app.cursor, before);
        assert!(app.g_pending_at.is_some());
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.cursor, 0);
        assert!(app.g_pending_at.is_none());
    }

    #[test]
    fn g_pending_survives_ticks_none_and_release() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.dispatch(Action::ArmGChord);
        assert!(app.g_pending_at.is_some());
        assert!(matches!(app.input_mode(), InputMode::GPending { .. }));
        app.dispatch(Action::WatchTick);
        app.dispatch(Action::None);
        app.dispatch(Action::FetchTick);
        app.dispatch(Action::Release);
        assert!(app.g_pending_at.is_some());
        assert!(matches!(app.input_mode(), InputMode::GPending { .. }));
        app.dispatch(Action::ToggleTreeMode);
        assert!(app.g_pending_at.is_none());
    }

    #[test]
    fn gg_g_on_focused_diff_scroll_to_edges() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let mut lines = vec!["@@ -1,1 +1,40 @@".into()];
        lines.extend((0..40).map(|i| format!("+line {i}")));
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(lines),
        );
        app.focus = FocusPane::Right;
        app.layout.diff_pane_height = 8;
        assert_eq!(app.list_focus_target(), ListFocusTarget::None);
        let view_h = app.diff_body_height();
        let past_mid = (view_h / 2 + 2) as i32;
        app.dispatch(Action::Move(past_mid));
        let mid = app.diff_scroll;
        assert!(
            mid > 0,
            "Move past the midpoint should leave the top, scroll={mid} cursor={}",
            app.diff_cursor
        );
        app.dispatch(Action::MoveToEnd);
        assert!(
            app.diff_scroll > mid,
            "MoveToEnd must scroll a focused diff, mid={mid} after={}",
            app.diff_scroll
        );
        assert_eq!(app.focus, FocusPane::Right);
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.diff_scroll, 0);
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.focus, FocusPane::Right);
    }

    #[test]
    fn focused_diff_j_past_midpoint_keeps_focus_near_middle() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        let mut lines = vec!["@@ -1,1 +1,40 @@".into()];
        lines.extend((0..40).map(|i| format!("+line {i}")));
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(lines),
        );
        app.focus = FocusPane::Right;
        app.layout.diff_pane_height = 8;
        let n = app.current_diff_rows().len();
        let view_h = app.diff_body_height();
        assert_eq!(view_h, 7);
        let steps = view_h / 2 + 3;
        app.dispatch(Action::Move(steps as i32));
        assert_eq!(app.diff_cursor, steps);
        assert_eq!(
            app.diff_scroll as usize,
            list_viewport_start(n, app.diff_cursor, view_h)
        );
        let offset = app.diff_cursor - app.diff_scroll as usize;
        assert_eq!(
            offset,
            view_h / 2,
            "focused diff row should sit at the vertical middle, offset={offset} mid={}",
            view_h / 2
        );
        assert_ne!(
            offset,
            view_h.saturating_sub(1),
            "keep-middle must not leave the focused row on the last viewport line"
        );
    }

    #[test]
    fn gg_g_on_left_tree_with_file_diff_shown_moves_tree() {
        let mut app = tree_app();
        focus_file(&mut app, "README.md");
        app.set_diff(
            "app".into(),
            "README.md".into(),
            DiffContent::from_lines(vec!["+line".into(); 20]),
        );
        app.focus = FocusPane::Left;
        app.diff_scroll = 4;
        app.cursor = 2.min(app.rows.len().saturating_sub(1));
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.cursor, 0);
        assert_eq!(
            app.diff_scroll, 4,
            "left gg must not scroll an unfocused file diff"
        );
        assert_eq!(app.focus, FocusPane::Left);
        app.dispatch(Action::MoveToEnd);
        assert_eq!(app.cursor, app.rows.len() - 1);
        assert_eq!(
            app.diff_scroll, 4,
            "left G must not scroll an unfocused file diff"
        );
        assert_eq!(app.focus, FocusPane::Left);
    }

    #[test]
    fn ctrl_d_moves_five_and_pgdn_uses_page_height() {
        let mut app = tree_app();
        assert!(app.rows.len() >= 5, "need a list long enough for +5");
        app.cursor = 0;
        app.layout.tree_height = 2;
        app.dispatch(Action::Move(5));
        assert_eq!(app.cursor, 5.min(app.rows.len() - 1));
        app.cursor = 0;
        app.dispatch(Action::PageMove(1));
        assert_eq!(app.cursor, 1);
        app.cursor = 4;
        app.dispatch(Action::Move(-5));
        assert_eq!(app.cursor, 0);
    }

    fn install_linear_graph(app: &mut AppState, n: usize) {
        let commits: Vec<Commit> = (0..n)
            .map(|i| {
                let id = format!("c{i:02}{}", "a".repeat(36));
                let parents = if i + 1 < n {
                    vec![format!("c{:02}{}", i + 1, "a".repeat(36))]
                } else {
                    Vec::new()
                };
                Commit {
                    id,
                    subject: format!("commit {i}"),
                    parents,
                    ..Commit::default()
                }
            })
            .collect();
        let head = commits[0].id.clone();
        let model = GraphModel {
            commits,
            stashes: Vec::new(),
            worktrees: Vec::new(),
            head_id: Some(head.clone()),
            sync: None,
            show_hidden: app.show_hidden,
            uncommitted: None,
            ..GraphModel::default()
        };
        app.set_graph(model, "app".into(), head);
        app.focus = FocusPane::Right;
        app.drill = DrillView::Graph;
    }

    fn painted_focus_index(app: &AppState) -> usize {
        let model = app.graph.as_ref().expect("graph");
        let glyphs = if app.ascii { &ASCII } else { &UNICODE };
        let painted = paint_model(model, glyphs, None);
        painted
            .iter()
            .position(|line| line.row_index == Some(app.graph_cursor) && line.selectable)
            .expect("focused painted row")
    }

    #[test]
    fn graph_pgdn_pages_painted_lines_and_keeps_focus_in_viewport() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        // Collapsed footer (rule + 2 rows), height 12, no sync header →
        // list_height 9, page = 8 painted lines.
        app.commit_msg_expand = false;
        app.layout.tree_height = 12;
        let list_h = app.graph_chrome().list_height.max(1) as usize;
        assert_eq!(list_h, 9, "list_height from chrome budget");
        let page = list_h.saturating_sub(1).max(1);
        app.graph_cursor = 0;
        app.sync_graph_scroll();
        let before = painted_focus_index(&app);
        let selectable = app.graph.as_ref().unwrap().visible_rows().len();
        app.dispatch(Action::PageMove(1));
        let after = painted_focus_index(&app);
        let scroll = app.graph_scroll as usize;
        assert!(
            after >= scroll && after < scroll + list_h,
            "focused painted row {after} must stay in [{scroll}, {})",
            scroll + list_h
        );
        assert!(
            after >= page.saturating_sub(1) && after <= page + 1,
            "PageDown should move about one painted viewport ({page}), got painted {before} → {after}"
        );
        assert!(
            app.graph_cursor < page.min(selectable),
            "must not apply painted page ({page}) to selectable indices; cursor={}",
            app.graph_cursor
        );

        app.dispatch(Action::MoveToEnd);
        let end = painted_focus_index(&app);
        let end_scroll = app.graph_scroll as usize;
        assert!(
            end >= end_scroll && end < end_scroll + list_h,
            "End should sync scroll: painted {end} in [{end_scroll}, {})",
            end_scroll + list_h
        );
        assert!(end_scroll > 0, "tall list End should scroll");
        app.dispatch(Action::MoveToStart);
        assert_eq!(app.graph_cursor, 0);
        assert_eq!(app.graph_scroll, 0);
    }

    #[test]
    fn graph_pgdn_pages_with_the_expanded_footer_on_a_short_pane() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 20);
        // Expanded N = 4 asks for the rule + 5 rows; an 8-row pane caps the
        // body at half (4) under the rule, so the list keeps 3 rows and a
        // page is 2 painted lines.
        assert!(app.commit_msg_expand);
        app.layout.tree_height = 8;
        let chrome = app.graph_chrome();
        assert_eq!((chrome.footer_height, chrome.list_height), (5, 3));
        let list_h = chrome.list_height as usize;
        let page = list_h - 1;
        app.graph_cursor = 0;
        app.sync_graph_scroll();
        app.dispatch(Action::PageMove(1));
        let after = painted_focus_index(&app);
        let scroll = app.graph_scroll as usize;
        assert!(
            after >= scroll && after < scroll + list_h,
            "focused painted row {after} must stay in [{scroll}, {})",
            scroll + list_h
        );
        assert!(
            after >= page.saturating_sub(1) && after <= page + 1,
            "PageDown moves about one painted viewport ({page}), got {after}"
        );
        app.dispatch(Action::PageMove(1));
        let again = painted_focus_index(&app);
        let scroll = app.graph_scroll as usize;
        assert!(again > after, "a second page moves on");
        assert!(again >= scroll && again < scroll + list_h);
    }

    /// Install a linear graph where commit 2 has a one-line message and
    /// commit 3 a body far longer than any footer.
    fn install_short_and_long_messages(app: &mut AppState) {
        install_linear_graph(app, 20);
        let body = (0..40)
            .map(|i| format!("body line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(model) = app.graph.as_mut() {
            model.commits[3].body = body;
        }
    }

    #[test]
    fn graph_list_height_ignores_loading_older() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        for height in [3, 12, 40] {
            app.layout.tree_height = height;
            for expand in [true, false] {
                app.commit_msg_expand = expand;
                app.graph_loading_older = false;
                let idle = app.graph_chrome();
                app.graph_loading_older = true;
                assert_eq!(app.graph_chrome(), idle, "height {height} expand {expand}");
            }
        }
    }

    #[test]
    fn graph_footer_height_does_not_follow_the_selected_message() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_short_and_long_messages(&mut app);
        app.layout.tree_height = 40;
        for expand in [true, false] {
            app.commit_msg_expand = expand;
            app.graph_cursor = 2;
            app.sync_graph_scroll();
            let short = app.graph_chrome();
            app.dispatch(Action::Move(1));
            assert_eq!(app.graph_cursor, 3, "expand {expand}");
            let long = app.graph_chrome();
            assert_eq!(long.list_height, short.list_height, "expand {expand}");
            assert_eq!(long.footer_height, short.footer_height, "expand {expand}");
            app.dispatch(Action::Move(-1));
            assert_eq!(app.graph_chrome(), short, "expand {expand}");
            // The rule row, then N message rows + meta, or subject + meta.
            let want = if expand {
                app.commit_msg_lines as u16 + 2
            } else {
                3
            };
            assert_eq!(short.footer_height, want, "expand {expand}");
        }
    }

    #[test]
    fn minus_and_plus_resize_the_footer_by_one_row_and_clamp() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_short_and_long_messages(&mut app);
        // Tall enough that the half-pane cap never limits N = 20.
        app.layout.tree_height = 60;
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_DEFAULT);
        let base = app.graph_chrome();
        assert_eq!(base.footer_height as usize, COMMIT_MSG_LINES_DEFAULT + 2);

        assert_eq!(app.dispatch(Action::ResizeCommitMsg(1)), Effect::None);
        assert_eq!(
            app.status,
            format!("msg lines {}", COMMIT_MSG_LINES_DEFAULT + 1)
        );
        let grown = app.graph_chrome();
        assert_eq!(grown.footer_height, base.footer_height + 1);
        assert_eq!(grown.list_height + 1, base.list_height);

        app.dispatch(Action::ResizeCommitMsg(-1));
        app.dispatch(Action::ResizeCommitMsg(-1));
        assert_eq!(
            app.status,
            format!("msg lines {}", COMMIT_MSG_LINES_DEFAULT - 1)
        );
        assert_eq!(app.graph_chrome().footer_height + 1, base.footer_height);

        for _ in 0..30 {
            app.dispatch(Action::ResizeCommitMsg(-1));
        }
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_MIN);
        assert_eq!(app.status, "msg lines 1", "status also at the clamp");
        assert_eq!(app.graph_chrome().footer_height, 3);
        for _ in 0..30 {
            app.dispatch(Action::ResizeCommitMsg(1));
        }
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_MAX);
        assert_eq!(app.status, "msg lines 20", "status also at the clamp");
        assert_eq!(app.graph_chrome().footer_height, 22);

        // Collapsed keeps its rule + 2 rows; N still changes for the next
        // expand.
        app.commit_msg_expand = false;
        app.dispatch(Action::ResizeCommitMsg(-1));
        assert_eq!(app.commit_msg_lines, COMMIT_MSG_LINES_MAX - 1);
        assert_eq!(app.graph_chrome().footer_height, 3);
        assert!(!app.commit_msg_expand, "- / + never toggle expand");
        assert!(
            !app.ui_store_path.exists(),
            "the palette resize is session-only: no UI-state save"
        );
    }

    #[test]
    fn footer_budget_changes_keep_the_focused_graph_row_in_view() {
        let in_view = |app: &AppState| {
            let idx = painted_focus_index(app);
            let scroll = app.graph_scroll as usize;
            let list_h = app.graph_chrome().list_height as usize;
            assert!(
                idx >= scroll && idx < scroll + list_h,
                "focused painted row {idx} outside [{scroll}, {})",
                scroll + list_h
            );
        };
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 40);
        app.layout.tree_height = 30;
        app.commit_msg_lines = COMMIT_MSG_LINES_MIN;
        app.dispatch(Action::MoveToEnd);
        in_view(&app);
        let before = app.graph_chrome().list_height;
        for _ in 0..13 {
            app.dispatch(Action::ResizeCommitMsg(1));
        }
        assert!(app.graph_chrome().list_height < before, "the list shrank");
        in_view(&app);

        // `M` from collapsed to expanded shrinks the list too.
        app.commit_msg_expand = false;
        app.dispatch(Action::MoveToEnd);
        in_view(&app);
        app.dispatch(Action::ToggleCommitMsgExpand);
        assert!(app.commit_msg_expand);
        in_view(&app);
    }

    #[test]
    fn graph_j_past_midpoint_keeps_focus_near_middle() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 40);
        // Collapsed footer (rule + 2 rows) keeps a 9-row list.
        app.commit_msg_expand = false;
        app.layout.tree_height = 12;
        let list_h = app.graph_chrome().list_height.max(1) as usize;
        assert_eq!(list_h, 9, "list_height from chrome budget");
        app.graph_cursor = 0;
        app.sync_graph_scroll();
        let steps = list_h / 2 + 4;
        app.dispatch(Action::Move(steps as i32));
        let idx = painted_focus_index(&app);
        let scroll = app.graph_scroll as usize;
        let offset = idx.saturating_sub(scroll);
        let mid = list_h / 2;
        assert!(
            (offset as i32 - mid as i32).abs() <= 1,
            "focused painted row should sit near the middle: offset={offset} mid={mid} idx={idx} scroll={scroll} list_h={list_h}"
        );
        assert_ne!(
            offset,
            list_h.saturating_sub(1),
            "keep-middle must not leave the focused row on the last viewport line"
        );
    }

    #[test]
    fn graph_scrollbar_drag_does_not_recenter_until_focus_moves() {
        let mut app = graph_state(false);
        focus_repo(&mut app, "app");
        install_linear_graph(&mut app, 40);
        app.layout.tree_height = 12;
        app.dispatch(Action::MoveToEnd);
        let cursor = app.graph_cursor;
        let scrolled = app.graph_scroll;
        assert!(scrolled > 0, "End should leave the top of the graph");
        app.graph_scroll = 0;
        assert_eq!(
            app.graph_cursor, cursor,
            "viewport-only scroll must not move the graph cursor"
        );
        assert_eq!(
            app.graph_scroll, 0,
            "scrollbar drag must keep an independent viewport until the focused row moves"
        );
        app.dispatch(Action::Move(0));
        assert_eq!(app.graph_cursor, cursor);
        assert_eq!(
            app.graph_scroll, scrolled,
            "the next focused-row move recentres the viewport"
        );
    }

    #[test]
    fn double_click_dispatches_enter() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        let idx = app
            .rows
            .iter()
            .position(|r| r.id == "repo:app")
            .expect("repo");
        let row = app.layout.tree_y + idx as u16;
        // click on the label, not the chevron (depth 1 => chevron at x=2)
        let col = 8;
        app.dispatch(Action::Click { col, row });
        assert_eq!(app.cursor, idx);
        assert_eq!(app.focus, FocusPane::Left);
        let effect = app.dispatch(Action::Click { col, row });
        assert_eq!(app.focus, FocusPane::Right);
        assert_eq!(effect, Effect::None);
    }

    #[test]
    fn chevron_click_toggles_fold() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        let idx = app
            .rows
            .iter()
            .position(|r| r.id == "dir:app:src")
            .expect("dir");
        let depth = app.rows[idx].depth;
        let col = app.layout.tree_x + 1 + (depth as u16) * 2;
        let row = app.layout.tree_y + idx as u16;
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
        app.dispatch(Action::Click { col, row });
        assert!(app.folds.contains("dir:app:src"));
        assert_eq!(
            app.cursor,
            app.rows.iter().position(|r| r.id == "dir:app:src").unwrap()
        );
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
    }

    #[test]
    fn double_click_chevron_folds_once_and_does_not_enter() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        let idx = app
            .rows
            .iter()
            .position(|r| r.id == "dir:app:src")
            .expect("dir");
        let depth = app.rows[idx].depth;
        let col = app.layout.tree_x + 1 + (depth as u16) * 2;
        let row = app.layout.tree_y + idx as u16;
        app.dispatch(Action::Click { col, row });
        assert!(app.folds.contains("dir:app:src"));
        assert_eq!(app.focus, FocusPane::Left);
        let effect = app.dispatch(Action::Click { col, row });
        assert!(
            app.folds.contains("dir:app:src"),
            "second Down must not undo the fold"
        );
        assert_eq!(
            app.focus,
            FocusPane::Left,
            "chevron double-click is not Enter"
        );
        assert_eq!(effect, Effect::None);
        assert!(app.rows.iter().all(|r| r.id != "file:app:src/lib.rs"));
    }

    #[test]
    fn m_toggles_mouse_and_ignores_clicks_when_off() {
        let mut app = tree_app();
        app.layout.tree_x = 0;
        app.layout.tree_y = 1;
        app.layout.list_offset = 0;
        app.layout.right_x = 40;
        assert!(app.mouse_enabled);
        app.dispatch(Action::ToggleMouse);
        assert!(!app.mouse_enabled);
        assert!(app.status.contains("off"));
        let start = app.cursor;
        app.dispatch(Action::Click {
            col: 8,
            row: app.layout.tree_y,
        });
        assert_eq!(app.cursor, start);
        app.dispatch(Action::ToggleMouse);
        assert!(app.mouse_enabled);
        assert!(app.status.contains("on"));
    }

    fn two_line_readme() -> DiffContent {
        DiffContent::from_unified("@@ -1 +1,2 @@\n # app\n+dirty\n")
    }

    fn three_line_readme() -> DiffContent {
        DiffContent::from_unified("@@ -1 +1,3 @@\n # app\n+dirty\n+watch-extra-line\n")
    }

    fn numbered_diff_rows(app: &AppState) -> Vec<(usize, u32)> {
        app.current_diff_rows()
            .iter()
            .enumerate()
            .filter_map(|(i, row)| {
                let n = match row {
                    DiffRow::Line { left, right } => {
                        right.as_ref().and_then(|c| c.line_no).or(left.line_no)
                    }
                    _ => None,
                };
                n.map(|line| (i, line))
            })
            .collect()
    }

    fn focus_readme_diff(app: &mut AppState, content: DiffContent) {
        focus_file(app, "README.md");
        app.focus = FocusPane::Right;
        app.set_diff("app".into(), "README.md".into(), content);
    }

    #[test]
    fn highlight_gg_and_g_move_the_head_and_keep_the_anchor() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        let last = app.current_diff_rows().len() - 1;
        app.dispatch(Action::Move(2));
        assert_eq!(app.dispatch(Action::DiffVisualStart), Effect::None);
        let anchor = app.diff_visual_anchor.expect("highlight");
        assert_eq!(app.dispatch(Action::MoveToEnd), Effect::None);
        assert_eq!(app.diff_cursor, last);
        assert_eq!(app.diff_visual_anchor, Some(anchor));
        assert!(app.diff_visual_contains(last));
        // `gg`: the first `g` arms, the second moves to the top.
        assert_eq!(app.dispatch(Action::ArmGChord), Effect::None);
        assert_eq!(app.diff_cursor, last);
        assert_eq!(app.dispatch(Action::ArmGChord), Effect::None);
        assert_eq!(app.diff_cursor, 0);
        assert_eq!(app.diff_visual_anchor, Some(anchor));
        assert!(app.g_pending_at.is_none());
        assert_eq!(app.input_mode(), InputMode::DiffVisual);
    }

    /// Text of the diff row under the cursor (inline layout: the left cell).
    fn diff_cursor_text(app: &AppState) -> String {
        match &app.current_diff_rows()[app.diff_cursor] {
            DiffRow::Line { left, .. } => left.text.clone(),
            other => panic!("cursor off a line row: {other:?}"),
        }
    }

    /// Go to `line`, then the text of the diff row it landed on.
    fn go_to_diff_line(app: &mut AppState, line: usize) -> String {
        assert_eq!(app.dispatch(Action::GoToLine(line)), Effect::None);
        assert!(app.diff_scroll as usize <= app.diff_cursor, "kept in view");
        diff_cursor_text(app)
    }

    /// `two_hunk_readme` new lines: keep-a 1, keep-b 2, ALPHA-NEW 3,
    /// keep-c 4, then keep-x 20, keep-y 21, OMEGA-NEW 22, keep-z 23.
    #[test]
    fn go_to_line_in_the_workspace_diff_takes_the_new_side_or_the_nearest_row() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        assert_eq!(app.go_to_line_refusal(), None);
        assert_eq!(go_to_diff_line(&mut app, 21), "keep-y");
        assert_eq!(
            go_to_diff_line(&mut app, 3),
            "ALPHA-NEW",
            "new line 3, not old line 3"
        );
        assert_eq!(go_to_diff_line(&mut app, 10), "keep-c", "nearest row");
        assert_eq!(go_to_diff_line(&mut app, 15), "keep-x", "nearest row");
        assert_eq!(go_to_diff_line(&mut app, 0), "keep-a", "below 1 is line 1");
        assert_eq!(go_to_diff_line(&mut app, 9999), "keep-z", "the last line");
    }

    #[test]
    fn go_to_line_counts_deleted_rows_by_old_line_without_a_new_side() {
        let mut app = state();
        let deleted = DiffContent::from_unified("@@ -1,3 +0,0 @@\n-gone-a\n-gone-b\n-gone-c\n");
        focus_readme_diff(&mut app, deleted);
        assert_eq!(go_to_diff_line(&mut app, 2), "gone-b");
        assert_eq!(go_to_diff_line(&mut app, 300), "gone-c");
    }

    /// Only a focused file diff with numbered lines is a line view on the
    /// Workspace tab.
    #[test]
    fn go_to_line_refuses_off_a_focused_diff_with_lines() {
        let mut app = state();
        focus_readme_diff(&mut app, binary_readme());
        assert_eq!(
            app.go_to_line_refusal().as_deref(),
            Some(super::go_to_line::NO_LINES_TO_GO_TO)
        );
        focus_readme_diff(&mut app, two_hunk_readme());
        app.focus = FocusPane::Left;
        assert_eq!(app.go_to_line_refusal().as_deref(), Some(FOCUS_A_FILE_DIFF));
        let cursor = app.diff_cursor;
        assert_eq!(app.dispatch(Action::GoToLine(20)), Effect::None);
        assert_eq!(app.status, FOCUS_A_FILE_DIFF);
        assert_eq!(app.diff_cursor, cursor, "nothing moved");
    }

    #[test]
    fn go_to_line_in_a_compare_diff() {
        let mut app = state();
        compare_tab_with_added_lines(&mut app);
        assert_eq!(go_to_diff_line(&mut app, 3), "more");
        assert_eq!(go_to_diff_line(&mut app, 99), "keep");
        app.focus = FocusPane::Left;
        assert_eq!(
            app.go_to_line_refusal().as_deref(),
            Some(FOCUS_A_FILE_DIFF),
            "the compare file list"
        );
    }

    #[test]
    fn go_to_line_in_a_commit_and_a_stash_diff() {
        let stash = CommitFileSource::Stash {
            stash_ref: "stash@{0}".into(),
        };
        for source in [sample_commit_source(), stash] {
            let mut app = state();
            app.focus = FocusPane::Right;
            app.open_commit_diff(
                "app".into(),
                source,
                sample_commit_files(),
                0,
                "README.md".into(),
                two_hunk_readme(),
            );
            assert_eq!(go_to_diff_line(&mut app, 22), "OMEGA-NEW");
            app.focus = FocusPane::Left;
            assert_eq!(
                app.go_to_line_refusal().as_deref(),
                Some(FOCUS_A_FILE_DIFF),
                "the commit file list"
            );
        }
    }

    /// `:` over a `V` range still lists the row; the jump moves the range
    /// end and keeps the anchor, as `G` does.
    #[test]
    fn go_to_line_in_highlight_moves_the_head_and_keeps_the_anchor() {
        use super::super::action::QuickOpenEntry;
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_alpha_new(&mut app);
        let anchor = app.diff_visual_anchor;
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in "22".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(palette.selected_action(), Some(Action::GoToLine(22)));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert_eq!(diff_cursor_text(&app), "OMEGA-NEW");
        assert_eq!(app.diff_visual_anchor, anchor);
        assert!(app.diff_visual_contains(app.diff_cursor));
    }

    #[test]
    fn highlight_unmapped_key_says_esc_exits() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        assert_eq!(app.dispatch(Action::DiffVisualStart), Effect::None);
        assert_eq!(app.dispatch(Action::DiffVisualUnmapped), Effect::None);
        assert_eq!(app.status, ESC_EXITS_HIGHLIGHT);
        assert_eq!(app.status.kind(), StatusKind::Warn);
        assert!(app.diff_visual_anchor.is_some(), "highlight stays");
        assert_eq!(app.dispatch(Action::SetTheme(ThemeId::Nord)), Effect::None);
        assert!(
            app.diff_visual_anchor.is_some(),
            "a theme row keeps the highlight"
        );
        assert_eq!(app.dispatch(Action::Quit), Effect::Quit);
    }

    #[test]
    fn semicolon_without_visual_opens_covering_range() {
        let mut app = state();
        focus_readme_diff(&mut app, two_line_readme());
        let numbered = numbered_diff_rows(&app);
        let line2 = numbered
            .iter()
            .find(|(_, line)| *line == 2)
            .map(|(idx, _)| *idx)
            .expect("line 2");
        let range = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 1,
            end_line: 2,
        };
        app.comment_store = put_comment(&app.comment_store, range.clone(), "range-note");
        app.diff_cursor = line2;
        app.dispatch(Action::CommentStart);
        let prompt = app.comment.take().expect("covering overlay");
        assert_eq!(prompt.key, range);
        assert_eq!(prompt.body(), "range-note");
        assert!(prompt.label.contains("1-2"));
        app.comment = Some(CommentPrompt::new(prompt.key, String::new(), prompt.label));
        app.dispatch(Action::CommentSubmit);
        assert!(!app.comment_store.contains_key(&range));
        assert_eq!(app.status, "comment deleted");
    }

    #[test]
    fn ctrl_r_toggles_resolved_and_enter_persists() {
        let mut app = state();
        focus_readme_diff(&mut app, two_line_readme());
        let numbered = numbered_diff_rows(&app);
        let line2 = numbered
            .iter()
            .find(|(_, line)| *line == 2)
            .map(|(idx, _)| *idx)
            .expect("line 2");
        let key = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 2,
            end_line: 2,
        };
        app.comment_store = CommentStore::new();
        app.diff_cursor = line2;
        app.dispatch(Action::CommentStart);
        app.dispatch(Action::CommentInput(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('n'),
            crossterm::event::KeyModifiers::NONE,
        )));
        app.dispatch(Action::CommentToggleResolved);
        assert!(app.comment.as_ref().is_some_and(|p| p.resolved));
        app.dispatch(Action::CommentSubmit);
        assert!(app.comment_store.get(&key).is_some_and(|e| e.resolved));
        assert_eq!(
            app.comment_store.get(&key).map(|e| e.body.as_str()),
            Some("n")
        );
        let md = crate::tui::comments::export_markdown(&app.comment_store);
        assert!(md.contains("n"));
        assert!(md.contains("[resolved]"), "{md}");
        app.dispatch(Action::CommentStart);
        app.dispatch(Action::CommentToggleResolved);
        assert!(app.comment.as_ref().is_some_and(|p| !p.resolved));
        app.dispatch(Action::CommentSubmit);
        assert!(app.comment_store.get(&key).is_some_and(|e| !e.resolved));
        let md = crate::tui::comments::export_markdown(&app.comment_store);
        assert!(md.contains("n"));
        assert!(!md.contains("[resolved]"), "{md}");
    }

    #[test]
    fn visual_semicolon_keeps_highlight_span() {
        let mut app = state();
        focus_readme_diff(&mut app, two_line_readme());
        let numbered = numbered_diff_rows(&app);
        assert!(numbered.len() >= 2);
        let one_line = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: numbered[1].1,
            end_line: numbered[1].1,
        };
        app.comment_store = put_comment(&app.comment_store, one_line, "one");
        app.diff_cursor = numbered[0].0;
        app.dispatch(Action::DiffVisualStart);
        app.diff_cursor = numbered[1].0;
        app.dispatch(Action::CommentStart);
        let prompt = app.comment.expect("visual overlay");
        match prompt.key {
            CommentKey::WorktreeLine { line, end_line, .. } => {
                assert_eq!(line, numbered[0].1.min(numbered[1].1));
                assert_eq!(end_line, numbered[0].1.max(numbered[1].1));
                assert_ne!(line, end_line);
            }
            other => panic!("{other:?}"),
        }
        assert!(prompt.body().is_empty());
    }

    #[test]
    fn set_diff_same_file_new_rows_drops_visual() {
        let mut app = state();
        focus_readme_diff(&mut app, two_line_readme());
        let numbered = numbered_diff_rows(&app);
        assert!(numbered.len() >= 2);
        app.diff_cursor = numbered[0].0;
        app.dispatch(Action::DiffVisualStart);
        app.diff_cursor = numbered[1].0;
        assert!(app.diff_visual_anchor.is_some());
        app.set_diff("app".into(), "README.md".into(), two_line_readme());
        assert!(
            app.diff_visual_anchor.is_some(),
            "identical rows must keep highlight"
        );
        app.set_diff("app".into(), "README.md".into(), three_line_readme());
        assert!(
            app.diff_visual_anchor.is_none(),
            "new painted rows must drop highlight"
        );
        app.dispatch(Action::CommentStart);
        let prompt = app.comment.expect("single-line overlay");
        match prompt.key {
            CommentKey::WorktreeLine { line, end_line, .. } => {
                assert_eq!(line, end_line, "must not store a stale visual span");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn toggle_diff_mode_and_commit_diff_drop_visual() {
        let mut app = state();
        focus_readme_diff(&mut app, two_line_readme());
        app.layout.diff_pane_width = 110;
        app.dispatch(Action::DiffVisualStart);
        assert!(app.diff_visual_anchor.is_some());
        app.dispatch(Action::ToggleDiffMode);
        assert!(
            app.diff_visual_anchor.is_none(),
            "split/inline row list change must drop highlight"
        );

        focus_readme_diff(&mut app, two_line_readme());
        app.dispatch(Action::DiffVisualStart);
        assert!(app.diff_visual_anchor.is_some());
        app.open_commit_diff(
            "app".into(),
            CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            sample_commit_files(),
            0,
            "README.md".into(),
            three_line_readme(),
        );
        assert!(
            app.diff_visual_anchor.is_none(),
            "commit diff row list change must drop highlight"
        );
    }

    fn two_hunk_readme() -> DiffContent {
        DiffContent::from_unified(
            "\
diff --git a/README.md b/README.md
--- a/README.md
+++ b/README.md
@@ -1,4 +1,4 @@
 keep-a
 keep-b
-ALPHA-OLD
+ALPHA-NEW
 keep-c
@@ -20,4 +20,4 @@
 keep-x
 keep-y
-OMEGA-OLD
+OMEGA-NEW
 keep-z
",
        )
    }

    #[test]
    fn visual_stage_emits_partial_patch_not_whole_file() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        let rows = app.current_diff_rows();
        let first = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Hunk { .. }))
            .expect("hunk");
        let second = rows
            .iter()
            .enumerate()
            .skip(first + 1)
            .find_map(|(i, r)| matches!(r, DiffRow::Hunk { .. }).then_some(i))
            .expect("second");
        app.diff_cursor = first;
        app.dispatch(Action::DiffVisualStart);
        app.diff_cursor = second - 1;
        match app.dispatch(Action::Stage) {
            Effect::ApplyCachedPatch {
                repo,
                path,
                patch,
                reverse,
            } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "README.md");
                assert!(!reverse);
                assert!(
                    patch.contains("ALPHA-NEW") && !patch.contains("OMEGA-NEW"),
                    "{patch}"
                );
            }
            other => panic!("{other:?}"),
        }
        assert!(app.diff_visual_anchor.is_none());
        assert_eq!(app.status, "staging range README.md…");
    }

    #[test]
    fn visual_stage_fails_closed_on_context_only() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        let rows = app.current_diff_rows();
        let ctx = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == "keep-a"))
            .expect("context");
        app.diff_cursor = ctx;
        app.dispatch(Action::DiffVisualStart);
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        assert!(
            app.diff_visual_anchor.is_some(),
            "fail-closed keeps highlight"
        );
        assert!(app.status.contains("nothing to stage"), "{}", app.status);
    }

    fn highlight_alpha_new(app: &mut AppState) {
        let rows = app.current_diff_rows();
        let change = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == "ALPHA-NEW"))
            .expect("add");
        app.diff_cursor = change;
        app.dispatch(Action::DiffVisualStart);
        assert!(app.diff_visual_anchor.is_some());
    }

    fn open_visual_drill_diff(app: &mut AppState, source: CommitFileSource) {
        app.focus = FocusPane::Right;
        app.open_commit_diff(
            "app".into(),
            source,
            sample_commit_files(),
            0,
            "README.md".into(),
            two_hunk_readme(),
        );
        highlight_alpha_new(app);
    }

    fn assert_visual_write_refused(app: &mut AppState, subject: &str) {
        for (action, verb) in [
            (Action::Stage, "stage"),
            (Action::Unstage, "unstage"),
            (Action::Revert, "revert"),
        ] {
            assert_eq!(app.dispatch(action), Effect::None);
            assert!(
                app.diff_visual_anchor.is_some(),
                "fail-closed keeps highlight"
            );
            let want = format!("cannot {verb} {subject}");
            assert_eq!(app.status, want);
            assert!(app.confirm.is_none(), "refusal opens no confirm");
        }
    }

    #[test]
    fn visual_stage_refuses_commit_drill_diff() {
        let mut app = state();
        open_visual_drill_diff(&mut app, sample_commit_source());
        assert_visual_write_refused(&mut app, "a committed diff");
    }

    #[test]
    fn visual_stage_refuses_stash_drill_diff() {
        let mut app = state();
        open_visual_drill_diff(
            &mut app,
            CommitFileSource::Stash {
                stash_ref: "stash@{0}".into(),
            },
        );
        assert_visual_write_refused(&mut app, "a stash diff");
    }

    #[test]
    fn visual_stage_refuses_worktree_drill_diff() {
        let mut app = state();
        open_visual_drill_diff(&mut app, CommitFileSource::Worktree);
        assert_visual_write_refused(&mut app, "an uncommitted drill diff");
    }

    fn binary_readme() -> DiffContent {
        DiffContent::from_unified("Binary files a/README.md and b/README.md differ\n")
    }

    fn mixed_readme() -> DiffContent {
        let patch = two_hunk_readme().unstaged;
        DiffContent {
            staged: patch.clone(),
            unstaged: patch,
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            error: None,
        }
    }

    fn highlight_binary_stub(app: &mut AppState) {
        let rows = app.current_diff_rows();
        let binary = rows
            .iter()
            .position(
                |r| matches!(r, DiffRow::Line { left, .. } if left.text.contains("Binary files")),
            )
            .expect("binary");
        app.diff_cursor = binary;
        app.dispatch(Action::DiffVisualStart);
        assert!(app.diff_visual_anchor.is_some());
    }

    #[test]
    fn visual_stage_fails_closed_on_binary() {
        let mut app = state();
        focus_readme_diff(&mut app, binary_readme());
        highlight_binary_stub(&mut app);
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        assert!(
            app.diff_visual_anchor.is_some(),
            "fail-closed keeps highlight"
        );
        assert!(
            app.status.contains("cannot stage a binary highlight"),
            "{}",
            app.status
        );
    }

    #[test]
    fn visual_unstage_fails_closed_on_binary() {
        let mut app = state();
        focus_readme_diff(
            &mut app,
            DiffContent {
                staged: "Binary files a/README.md and b/README.md differ\n".into(),
                unstaged: String::new(),
                is_new: false,
                is_committed: false,
                vs_worktree: false,
                error: None,
            },
        );
        highlight_binary_stub(&mut app);
        assert_eq!(app.dispatch(Action::Unstage), Effect::None);
        assert!(
            app.diff_visual_anchor.is_some(),
            "fail-closed keeps highlight"
        );
        assert!(
            app.status.contains("cannot unstage a binary highlight"),
            "{}",
            app.status
        );
    }

    #[test]
    fn visual_stage_fails_closed_on_mixed_sections() {
        let mut app = state();
        focus_readme_diff(&mut app, mixed_readme());
        let last = app.current_diff_rows().len().saturating_sub(1);
        app.diff_cursor = 0;
        app.dispatch(Action::DiffVisualStart);
        app.diff_cursor = last;
        assert_eq!(app.dispatch(Action::Stage), Effect::None);
        assert!(
            app.diff_visual_anchor.is_some(),
            "fail-closed keeps highlight"
        );
        assert!(
            app.status.contains("highlight spans staged and unstaged"),
            "{}",
            app.status
        );
        assert_eq!(app.dispatch(Action::Unstage), Effect::None);
        assert!(
            app.status.contains("highlight spans staged and unstaged"),
            "{}",
            app.status
        );
    }

    fn highlight_first_readme_hunk(app: &mut AppState) {
        let rows = app.current_diff_rows();
        let first = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Hunk { .. }))
            .expect("hunk");
        let second = rows
            .iter()
            .enumerate()
            .skip(first + 1)
            .find_map(|(i, r)| matches!(r, DiffRow::Hunk { .. }).then_some(i))
            .expect("second");
        app.diff_cursor = first;
        app.dispatch(Action::DiffVisualStart);
        app.diff_cursor = second - 1;
    }

    #[test]
    fn visual_revert_confirms_then_emits_revert_patch() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        assert_eq!(
            palette_reason(&app, "Revert highlighted lines"),
            None,
            "a buildable range keeps palette revert enabled"
        );
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(
            app.diff_visual_anchor.is_none(),
            "opening the confirm clears highlight"
        );
        let Some(PendingConfirm::RevertRange { repo, path, patch }) = app.confirm.clone() else {
            panic!("{:?}", app.confirm);
        };
        assert_eq!((repo.as_str(), path.as_str()), ("app", "README.md"));
        assert!(
            patch.contains("ALPHA-NEW") && !patch.contains("OMEGA-NEW"),
            "{patch}"
        );
        assert_eq!(
            app.dispatch(Action::ConfirmYes),
            Effect::RevertPatch {
                repo: "app".into(),
                path: "README.md".into(),
                patch,
            }
        );
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "reverting range README.md…");
    }

    #[test]
    fn visual_revert_confirm_ignores_capital_y() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        app.dispatch(Action::Revert);
        let pending = app.confirm.clone();
        assert!(matches!(pending, Some(PendingConfirm::RevertRange { .. })));
        let status = app.status.clone();
        assert_eq!(app.dispatch(Action::ConfirmYesClean), Effect::None);
        assert_eq!(app.confirm, pending, "Y keeps the y / n confirm open");
        assert_eq!(app.status, status);
    }

    #[test]
    fn visual_revert_confirm_no_cancels() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        app.dispatch(Action::Revert);
        assert!(matches!(
            app.confirm,
            Some(PendingConfirm::RevertRange { .. })
        ));
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none());
        assert_eq!(app.status, "revert cancelled");
    }

    #[test]
    fn visual_revert_fails_closed_on_context_only() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        let rows = app.current_diff_rows();
        let ctx = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == "keep-a"))
            .expect("context");
        app.diff_cursor = ctx;
        app.dispatch(Action::DiffVisualStart);
        assert_eq!(
            palette_reason(&app, "Revert highlighted lines").as_deref(),
            Some("nothing to revert in highlight")
        );
        assert_eq!(app.dispatch(Action::Revert), Effect::None);
        assert!(app.confirm.is_none());
        assert!(
            app.diff_visual_anchor.is_some(),
            "fail-closed keeps highlight"
        );
        assert_eq!(app.status, "nothing to revert in highlight");
    }

    /// Open Quick Open in commands mode, filter to `title`, and put the
    /// cursor on that row.
    fn palette_select(app: &mut AppState, title: &str) {
        use super::super::action::QuickOpenEntry;
        if app.quick_open.is_none() {
            app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        }
        for c in title.chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let palette = app.command_palette_mut().expect("palette open");
        let index = palette
            .visible()
            .iter()
            .position(|command| command.title == title)
            .unwrap_or_else(|| panic!("palette row {title}"));
        palette.cursor = index;
    }

    /// Map `key` with no modifier the way the event loop does.
    fn plain_key_action(app: &AppState, key: char) -> Action {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let event = Event::Key(KeyEvent::new(KeyCode::Char(key), KeyModifiers::NONE));
        super::super::app::map_event(app, &event)
    }

    #[test]
    fn palette_opens_over_highlight_and_esc_keeps_the_range() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        let anchor = app.diff_visual_anchor;
        let cursor = app.diff_cursor;
        assert_eq!(app.input_mode(), InputMode::DiffVisual);
        let colon = plain_key_action(&app, ':');
        assert_eq!(
            colon,
            Action::ToggleQuickOpen(super::super::action::QuickOpenEntry::Commands),
            "`:` in highlight opens commands"
        );
        app.dispatch(colon);
        assert_eq!(app.input_mode(), InputMode::QuickOpen);
        assert_eq!(
            app.quick_open.as_ref().map(|quick| quick.query.as_str()),
            Some(">")
        );
        assert_eq!(app.diff_visual_anchor, anchor, "open keeps the anchor");
        app.dispatch(Action::QuickOpenCancel);
        assert!(app.quick_open.is_none());
        assert_eq!(app.diff_visual_anchor, anchor, "Esc keeps the anchor");
        assert_eq!(app.diff_cursor, cursor);
        assert_eq!(app.input_mode(), InputMode::DiffVisual);
    }

    #[test]
    fn opening_a_file_from_highlight_drops_the_range() {
        use crate::file_index::{score_files, FileEntry, FileIndex, IndexRoot, MAX_RESULTS};
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        assert_eq!(app.input_mode(), InputMode::DiffVisual);

        let open = plain_key_action(&app, 'F');
        assert_eq!(
            open,
            Action::ToggleQuickOpen(super::super::action::QuickOpenEntry::Files),
            "`F` in highlight opens files"
        );
        let Effect::LoadFileIndex { gen, .. } = app.dispatch(open) else {
            panic!("expected an index load");
        };
        assert!(
            app.diff_visual_anchor.is_some(),
            "opening files keeps the range while the overlay is up"
        );
        let index = FileIndex {
            roots: vec![IndexRoot {
                checkout: "app".into(),
                prefix: String::new(),
            }],
            entries: vec![FileEntry {
                root: 0,
                display: "src/lib.rs".into(),
                rel_start: 0,
            }],
            truncated: false,
            errors: Vec::new(),
        };
        let Some(Effect::ScoreFiles { gen, index, query }) = app.apply_file_index(gen, index)
        else {
            panic!("expected a score");
        };
        app.apply_file_score(gen, score_files(&index, &query, MAX_RESULTS));

        let effect = app.dispatch(Action::QuickOpenSubmit);
        let Effect::Batch(effects) = &effect else {
            panic!("folder listings and the file load: {effect:?}");
        };
        assert!(
            effects.iter().any(|effect| matches!(effect,
                Effect::LoadExplorerPreview { repo, path, .. }
                    if repo == "app" && path == "src/lib.rs")),
            "{effect:?}"
        );
        assert!(app.is_explorer_tab(), "the Explorer tab is active");
        assert_eq!(app.diff_visual_anchor, None, "the range is dropped");
        assert_ne!(app.input_mode(), InputMode::DiffVisual);

        app.dispatch(Action::JumpToTab(1));
        assert!(app.tabs.is_workspace(), "back on Workspace");
        assert_eq!(
            app.diff_visual_anchor, None,
            "the range does not come back with the Workspace tab"
        );
    }

    /// Open the palette, type `filter`, and return the row under the cursor.
    fn palette_cursor_title(app: &mut AppState, filter: &str) -> &'static str {
        app.dispatch(Action::ToggleQuickOpen(
            super::super::action::QuickOpenEntry::Commands,
        ));
        for c in filter.chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let palette = app.command_palette().expect("palette open");
        palette.selected().expect("a visible row").title
    }

    #[test]
    fn palette_cursor_lands_on_the_first_enabled_row() {
        use super::super::command_palette::CommandGroup;
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        // Whole-file Revert runs from the file row, not the diff pane.
        app.focus = FocusPane::Left;
        let first = palette_cursor_title(&mut app, "");
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == first)
            .expect("catalog row");
        assert_ne!(command.group, CommandGroup::Highlight, "{first}");
        assert_eq!(app.palette_disabled_reason(command), None, "{first}");
        app.quick_open = None;

        assert_eq!(palette_cursor_title(&mut app, "revert"), "Revert");
        // Backspace re-lands the cursor on the wider list too.
        app.dispatch(Action::QuickOpenBackspace);
        let palette = app.command_palette().expect("palette open");
        assert_eq!(palette.selected().map(|c| c.title), Some("Revert"));
        // j / k still reach the disabled HIGHLIGHT row.
        app.dispatch(Action::QuickOpenMove(-1));
        let palette = app.command_palette().expect("palette open");
        assert_eq!(
            palette.selected().map(|c| c.title),
            Some("Revert highlighted lines")
        );
    }

    #[test]
    fn palette_cursor_in_highlight_lands_on_the_range_row() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        assert_eq!(
            palette_cursor_title(&mut app, "revert"),
            "Revert highlighted lines"
        );
    }

    #[test]
    fn palette_whole_file_rows_wait_for_highlight_exit() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        let anchor = app.diff_visual_anchor;
        for title in [
            "Fetch remotes",
            "Stage",
            "Revert",
            "Inline / split",
            "Open PR",
            // Help clears the highlight, and highlight mode has no `?` key.
            "Keymap help",
        ] {
            assert_eq!(
                palette_reason(&app, title).as_deref(),
                Some("exit highlight first (Esc)"),
                "{title}"
            );
        }
        for title in ["Theme: Nord", "Comment"] {
            assert_eq!(palette_reason(&app, title), None, "{title}");
        }
        palette_select(&mut app, "Fetch remotes");
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(
            app.command_palette().is_some(),
            "a disabled row keeps the palette open"
        );
        assert_eq!(app.status, "exit highlight first (Esc)");
        assert_eq!(app.diff_visual_anchor, anchor);
    }

    #[test]
    fn palette_highlight_rows_need_a_highlight() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        for title in [
            "Stage highlighted lines",
            "Unstage highlighted lines",
            "Revert highlighted lines",
            "Exit highlight",
        ] {
            assert_eq!(
                palette_reason(&app, title).as_deref(),
                Some("highlight diff lines first (V)"),
                "{title}"
            );
        }
        assert_eq!(palette_reason(&app, "Keymap help"), None);
        assert_eq!(
            palette_reason(&app, "Stage").as_deref(),
            Some("focus the tree (Tab) to stage")
        );
    }

    #[test]
    fn palette_tab_search_and_fold_rows_say_why_they_are_off() {
        use super::super::tabs::ONLY_WORKSPACE_TAB_OPEN;
        let mut app = state();
        focus_repo(&mut app, "app");
        for title in ["Next tab", "Previous tab"] {
            assert_eq!(
                palette_reason(&app, title).as_deref(),
                Some(ONLY_WORKSPACE_TAB_OPEN),
                "{title}"
            );
        }
        // `gt` gives the same copy as the palette row.
        assert_eq!(app.dispatch(Action::NextTab), Effect::None);
        assert_eq!(app.status, ONLY_WORKSPACE_TAB_OPEN);
        for title in ["Next match", "Previous match"] {
            assert_eq!(
                palette_reason(&app, title).as_deref(),
                Some(NO_SEARCH_ARMED),
                "{title}"
            );
        }
        app.search_active = true;
        app.search_query = "app".into();
        assert_eq!(palette_reason(&app, "Next match"), None);
        assert_eq!(palette_reason(&app, "Fold subtree"), None, "tree row");
        assert_eq!(palette_reason(&app, "Quit"), None);
        assert_eq!(palette_reason(&app, "Other pane"), None);

        install_graph(&mut app, vec![graph_stash("stash@{0}", "latest")]);
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Commit { .. }));
        assert_eq!(
            palette_reason(&app, "Fold subtree").as_deref(),
            Some(Z_FOLDS_TREE_ROWS)
        );
        assert_eq!(
            palette_reason(&app, "Stash menu").as_deref(),
            Some("focus the left pane (Tab) to stash")
        );
        assert_eq!(
            palette_reason(&app, "Apply stash").as_deref(),
            Some(super::super::gates::FOCUS_A_GRAPH_STASH)
        );
    }

    #[test]
    fn palette_other_pane_flips_focus_and_quit_quits() {
        let mut app = state();
        focus_repo(&mut app, "app");
        assert_eq!(app.focus, FocusPane::Left);
        palette_select(&mut app, "Other pane");
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert_eq!(app.focus, FocusPane::Right);
        palette_select(&mut app, "Other pane");
        app.dispatch(Action::QuickOpenSubmit);
        assert_eq!(app.focus, FocusPane::Left, "Tab goes back from the right");
        palette_select(&mut app, "Quit");
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::Quit);
    }

    /// Go to file closes commands mode, then opens Quick Open again in
    /// files mode, as `Ctrl-p` / `F` do.
    #[test]
    fn palette_go_to_file_opens_quick_open_in_files_mode() {
        use super::super::quick_open::QuickOpenMode;
        let mut app = state();
        focus_repo(&mut app, "app");
        palette_select(&mut app, "Go to file");
        assert_eq!(palette_reason(&app, "Go to file"), None);
        let effect = app.dispatch(Action::QuickOpenSubmit);
        assert!(
            matches!(effect, Effect::LoadFileIndex { .. }),
            "files mode loads the index: {effect:?}"
        );
        let quick = app.quick_open.as_ref().expect("Quick Open stays up");
        assert_eq!(quick.mode(), QuickOpenMode::Files);
        assert_eq!(quick.query, "", "no `>`: files mode");
        assert_eq!(app.input_mode(), InputMode::QuickOpen);
    }

    /// Search in files closes the palette and opens the search-in-files
    /// dialog, as `Ctrl-f` does.
    #[test]
    fn palette_search_in_files_opens_the_search_dialog() {
        let mut app = state();
        focus_repo(&mut app, "app");
        palette_select(&mut app, "Search in files");
        assert_eq!(palette_reason(&app, "Search in files"), None);
        let effect = app.dispatch(Action::QuickOpenSubmit);
        assert!(
            matches!(effect, Effect::LoadSearchIndex { .. }),
            "the dialog loads its index: {effect:?}"
        );
        assert!(app.quick_open.is_none(), "the palette closes");
        assert!(app.search_files.is_some(), "the search dialog opens");
        assert!(
            matches!(app.input_mode(), InputMode::SearchFiles { .. }),
            "{:?}",
            app.input_mode()
        );
    }

    #[test]
    fn palette_fold_subtree_folds_an_open_parent_and_its_children_like_zz() {
        let mut app = tree_app();
        focus_id(&mut app, "repo:app");
        assert!(!app.folds.contains("repo:app"));
        palette_select(&mut app, "Fold subtree");
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert!(app.folds.contains("repo:app"), "the focused parent folds");
        assert!(app.folds.contains("dir:app:src"), "its children fold too");
        assert!(
            app.z_pending_at.is_none(),
            "no `z` chord stays armed after the palette run"
        );

        palette_select(&mut app, "Fold subtree");
        app.dispatch(Action::QuickOpenSubmit);
        assert!(!app.folds.contains("repo:app"), "a second run opens it");
        assert!(!app.folds.contains("dir:app:src"));
        assert!(app.rows.iter().any(|r| r.id == "file:app:src/lib.rs"));
    }

    #[test]
    fn palette_alias_finds_a_row_and_runs_it() {
        use super::super::action::QuickOpenEntry;
        let mut app = state();
        focus_repo(&mut app, "app");
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in "exit".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let palette = app.command_palette().expect("palette open");
        assert_eq!(palette.filter, "exit");
        assert_eq!(palette.selected().map(|c| c.title), Some("Quit"));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::Quit);
    }

    #[test]
    fn palette_range_row_refuses_after_a_reload_drops_the_range() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        palette_select(&mut app, "Stage highlighted lines");
        app.set_diff("app".into(), "README.md".into(), two_line_readme());
        assert!(
            app.diff_visual_anchor.is_none(),
            "the reload drops the range"
        );
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(
            app.command_palette().is_some(),
            "a disabled row keeps it open"
        );
        assert_eq!(app.status, "highlight diff lines first (V)");
    }

    #[test]
    fn palette_esc_clears_the_disabled_row_reason() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        palette_select(&mut app, "Fetch remotes");
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert_eq!(app.status, "exit highlight first (Esc)");
        app.dispatch(Action::QuickOpenCancel);
        assert_eq!(app.input_mode(), InputMode::DiffVisual);
        assert_eq!(app.status, "", "the reason does not linger in highlight");

        // A status the palette did not set stays. Open clears a leftover
        // one, so this one lands while the palette is up.
        palette_select(&mut app, "Fetch remotes");
        app.status = "staged range README.md".into();
        app.dispatch(Action::QuickOpenCancel);
        assert_eq!(app.status, "staged range README.md");
    }

    #[test]
    fn palette_exit_highlight_clears_the_range() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        assert_eq!(palette_reason(&app, "Exit highlight"), None);
        palette_select(&mut app, "Exit highlight");
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert!(app.diff_visual_anchor.is_none());
    }

    #[test]
    fn palette_range_rows_use_the_range_patch_gate() {
        let mut app = state();
        focus_readme_diff(&mut app, two_hunk_readme());
        highlight_first_readme_hunk(&mut app);
        assert_eq!(palette_reason(&app, "Stage highlighted lines"), None);
        assert_eq!(
            palette_reason(&app, "Unstage highlighted lines").as_deref(),
            Some("nothing to unstage in highlight")
        );
        palette_select(&mut app, "Stage highlighted lines");
        match app.dispatch(Action::QuickOpenSubmit) {
            Effect::ApplyCachedPatch { patch, reverse, .. } => {
                assert!(!reverse);
                assert!(
                    patch.contains("ALPHA-NEW") && !patch.contains("OMEGA-NEW"),
                    "{patch}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    fn assert_copy_clipboard(effect: Effect, kind: &str, needle: &str, announce: bool) -> String {
        match effect {
            Effect::CopyClipboard {
                text,
                announce: got,
            } => {
                assert_eq!(got, announce, "announce");
                assert!(
                    text.starts_with("kind:"),
                    "copy payload must start with kind:: {text}"
                );
                assert!(
                    text.contains(&format!("kind: {kind}")),
                    "expected kind {kind}: {text}"
                );
                assert!(text.contains(needle), "expected {needle} in {text}");
                text
            }
            other => panic!("expected CopyClipboard, got {other:?}"),
        }
    }

    #[test]
    fn apostrophe_copies_focused_file_without_comment_overlay() {
        let mut app = state();
        let store_before = app.comment_store.clone();
        focus_file(&mut app, "README.md");
        let effect = app.dispatch(Action::CopyEntityReference);
        let text = assert_copy_clipboard(effect, "file", "README.md", true);
        assert!(text.contains("path: README.md"), "{text}");
        assert_ne!(
            &*app.status, "copied",
            "dispatch must not flash copied before the copy"
        );
        assert!(app.comment_export.is_none());
        assert!(app.comment.is_none());
        assert_eq!(app.comment_store, store_before);
        app.dispatch(Action::CommentStart);
        assert!(app.comment.is_none());
        assert_eq!(app.status, "no comment target");
    }

    #[test]
    fn apostrophe_copies_graph_stash() {
        let mut app = state();
        focus_repo(&mut app, "app");
        install_graph(
            &mut app,
            vec![Stash {
                id: "deadbeef1234567890abdeadbeef1234567890ab".into(),
                stash_ref: "stash@{0}".into(),
                subject: "WIP on main: first".into(),
                parent_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
                ..Stash::default()
            }],
        );
        focus_graph_row(&mut app, |r| matches!(r, GraphRow::Stash(_)));
        app.dispatch(Action::CommentStart);
        assert!(app.comment.is_none());
        assert_eq!(app.status, "no comment target");
        let effect = app.dispatch(Action::CopyEntityReference);
        let text = assert_copy_clipboard(effect, "stash", "stash@{0}", true);
        assert!(
            text.contains("deadbeef1234567890abdeadbeef1234567890ab"),
            "{text}"
        );
        assert_ne!(&*app.status, "copied");
        assert!(app.comment_export.is_none());
    }

    #[test]
    fn apostrophe_copies_numbered_diff() {
        let mut app = state();
        focus_readme_diff(&mut app, two_line_readme());
        let numbered = numbered_diff_rows(&app);
        let line2 = numbered
            .iter()
            .find(|(_, line)| *line == 2)
            .map(|(idx, _)| *idx)
            .expect("line 2");
        app.diff_cursor = line2;
        let effect = app.dispatch(Action::CopyEntityReference);
        let text = assert_copy_clipboard(effect, "diff", "README.md", true);
        assert!(text.contains("path: README.md"), "{text}");
        assert!(text.contains("lines: 2"), "{text}");
        assert_ne!(&*app.status, "copied");
        assert!(app.comment_export.is_none());
    }

    #[test]
    fn apostrophe_without_target_sets_status() {
        let mut app = state();
        app.folds.remove("group:no-updates");
        app.rebuild_rows();
        let idx = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Group)
            .expect("group");
        app.cursor = idx;
        assert_eq!(app.dispatch(Action::CopyEntityReference), Effect::None);
        assert_eq!(app.status, "no copy target");
        assert!(app.comment_export.is_none());
    }

    /// A group row has no checkout: `-` and the Open in Explorer row refuse
    /// with the same copy, and `-` does not resize the commit message.
    #[test]
    fn open_in_explorer_refuses_on_a_group_row() {
        use super::super::tabs::FOCUS_A_TREE_ROW_TO_EXPLORE;
        let mut app = state();
        app.folds.remove("group:no-updates");
        app.rebuild_rows();
        app.cursor = app
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Group)
            .expect("group");
        let lines = app.commit_msg_lines;
        assert_eq!(
            palette_reason(&app, "Open in Explorer").as_deref(),
            Some(FOCUS_A_TREE_ROW_TO_EXPLORE)
        );
        assert_eq!(app.dispatch(Action::ExplorerReveal), Effect::None);
        assert_eq!(app.status, FOCUS_A_TREE_ROW_TO_EXPLORE);
        assert_eq!(app.status.kind(), StatusKind::Warn);
        assert_eq!(app.commit_msg_lines, lines);
        assert!(app.tabs.is_workspace());
    }

    #[test]
    fn apostrophe_and_y_both_announce_the_copy() {
        let mut app = state();
        focus_file(&mut app, "README.md");
        match app.dispatch(Action::CopyEntityReference) {
            Effect::CopyClipboard { announce, .. } => assert!(announce),
            other => panic!("expected CopyClipboard, got {other:?}"),
        }
        assert_ne!(
            &*app.status, "copied",
            "entity copy leaves status to the interpreter"
        );
        assert!(app.comment_export.is_none());

        app.comment_store = CommentStore::new();
        assert_eq!(app.dispatch(Action::ExportComments), Effect::None);
        assert_eq!(app.status, STATUS_NO_COMMENTS);
        assert!(app.comment_export.is_none(), "nothing to show or copy");

        let key = CommentKey::Worktree { path: "app".into() };
        app.comment_store = put_comment(&app.comment_store, key, "note");
        focus_repo(&mut app, "app");
        match app.dispatch(Action::ExportComments) {
            Effect::CopyClipboard { announce, text } => {
                assert!(announce);
                assert!(text.contains("note"), "{text}");
            }
            other => panic!("expected CopyClipboard, got {other:?}"),
        }
        assert!(app.status.is_empty(), "the interpreter reports the copy");
        assert_eq!(
            app.comment_export.as_ref().map(|e| e.copied),
            Some(None),
            "header waits for the copy result"
        );
    }
}

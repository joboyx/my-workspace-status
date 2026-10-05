//! Bottom chrome: status pills, hint chips, and breadcrumb.
//!
//! Also the dialog geometry ([`dialog_rect`]): confirms, pickers, and help
//! are boxes centered over the panes, not status-line prompts.

use std::time::Duration;

/// Status text after a clipboard copy worked.
///
/// The comment-export overlay paints the copy result in its own header, so
/// it hides this status ([`export_shows_status`]) instead of repeating it.
pub const STATUS_COPIED: &str = "copied";

/// Status text after a clipboard copy failed. Hidden in the export overlay
/// like [`STATUS_COPIED`].
pub const STATUS_COPY_FAILED: &str = "copy failed";

/// Status text when `y` finds no comments in scope. Nothing is copied.
pub const STATUS_NO_COMMENTS: &str = "no comments here";

/// True when the comment-export overlay paints `status` as its own row.
///
/// The copy result is already the overlay header, so `copied` /
/// `copy failed` stay out. The export paint reads this check.
pub fn export_shows_status(status: &str) -> bool {
    !status.is_empty() && status != STATUS_COPIED && status != STATUS_COPY_FAILED
}

/// Status text `p` sets when the focused checkout has nothing to pull.
///
/// `effect.rs` reads it back to decide whether a second `p` escalates to a
/// fetch, so the wording is load-bearing, not cosmetic.
pub const STATUS_NOTHING_TO_PULL: &str = "nothing behind to pull";

/// Status text `p` sets when nothing is behind but a target has diverged.
///
/// An empty `diverged` gives [`STATUS_NOTHING_TO_PULL`].
/// [`is_idle_pull_status`] reads either copy back.
pub fn diverged_pull_status(diverged: &[String]) -> String {
    match diverged {
        [repo] => format!("{repo} has diverged — pull it from a terminal"),
        [repo, rest @ ..] => format!(
            "{repo} (+{} more) diverged — pull from a terminal",
            rest.len()
        ),
        [] => STATUS_NOTHING_TO_PULL.to_string(),
    }
}

/// True when `status` is what `p` sets when it found nothing to pull:
/// [`STATUS_NOTHING_TO_PULL`] or a [`diverged_pull_status`] line.
///
/// `effect.rs` then queues a pull behind an inflight fetch, so this check
/// and that copy must agree.
pub fn is_idle_pull_status(status: &str) -> bool {
    status == STATUS_NOTHING_TO_PULL
        || status.ends_with(" has diverged — pull it from a terminal")
        || status.ends_with(" diverged — pull from a terminal")
}

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use workspace_status_graph::GraphRow;

use crate::helpers::{is_default_branch, visible_width};
use crate::snapshot::{CheckoutKind, SyncStatus};

use super::branches::{can_open_branch_picker, checkoutable_branch_names};
use super::comments::{COMMENT_OVERLAY_CHROME_ROWS, COMMENT_OVERLAY_MAX_BODY_LINES};
use super::commit_files::CommitFileRowKind;
use super::ctrl_c_exit::is_ctrl_c_exit_prompt;
use super::drill::DrillView;
use super::help::{help_status_lines, HelpTab};
use super::icons::truncate_visible;
use super::keys::DOUBLE_TAP_MS;
use super::ops::{collect_write_files, op_targets, push_targets, Op};
use super::split::DiffMode;
use super::stash::{stash_ops_for_context, StashOpsContext};
use super::state::{
    revert_scope, AppState, FocusPane, SEARCH_NO_MATCH, SEARCH_WRAPPED_TO_BOTTOM,
    SEARCH_WRAPPED_TO_TOP,
};
use super::tabs::checkout_leaf;
use super::theme::{hex_color, Palette, Pill, Pills};
use super::tree::NodeKind;

/// Columns between a hint chip and its label.
pub const HINT_CHIP_GAP: usize = 2;
/// Gap rendered between two hints.
const HINT_SEPARATOR: &str = "  ";
/// Muted marker painted where hints were dropped to fit the terminal width.
pub const HINT_ELLIPSIS: &str = "…";
/// Status-bar copy while `/` search is in typing mode.
pub const SEARCH_TYPING_HINT: &str = "Enter arms query · Esc clears · n/N after Enter";
const BREADCRUMB_SEP: &str = " › ";

/// One rendered action hint: key chip text and description label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HintSegment {
    pub key: String,
    pub label: String,
    pub destructive: bool,
}

/// Row kind that selects the hint list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HintRowKind {
    Workspace,
    Repo,
    Checkout,
    Group,
    Dir,
    File,
    GraphCommit,
    GraphStash,
    GraphUncommitted,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HintActionId {
    Stage,
    Unstage,
    Revert,
    Fetch,
    Pull,
    Push,
    DefaultBranch,
    Branch,
    RemoveWorktree,
    Edit,
    ToggleViewed,
    FullFile,
    GraphCheckout,
    GraphCreateBranch,
    GraphMerge,
    GraphFocus,
    GraphFocusClear,
    StashMenu,
    StashApply,
    StashPop,
    StashDrop,
}

struct HintAction {
    id: HintActionId,
    key: &'static str,
    label: &'static str,
    kinds: &'static [HintRowKind],
    destructive: bool,
    depths: Option<&'static [u8]>,
    focus_left_only: bool,
}

const SCOPED: &[HintRowKind] = &[
    HintRowKind::Repo,
    HintRowKind::Checkout,
    HintRowKind::Dir,
    HintRowKind::File,
];

const HINT_ACTIONS: &[HintAction] = &[
    HintAction {
        id: HintActionId::Stage,
        key: "s",
        label: "stage",
        kinds: SCOPED,
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::Unstage,
        key: "u",
        label: "unstage",
        kinds: SCOPED,
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::Revert,
        key: "x",
        label: "revert",
        kinds: SCOPED,
        destructive: true,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::Fetch,
        key: "f",
        label: "fetch",
        kinds: &[
            HintRowKind::Workspace,
            HintRowKind::Repo,
            HintRowKind::Checkout,
            HintRowKind::Dir,
            HintRowKind::File,
        ],
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::Pull,
        key: "p",
        label: "pull",
        kinds: &[
            HintRowKind::Workspace,
            HintRowKind::Repo,
            HintRowKind::Checkout,
        ],
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::Push,
        key: "P",
        label: "push",
        kinds: &[HintRowKind::Repo, HintRowKind::Checkout],
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::DefaultBranch,
        key: "d",
        label: "default branch",
        kinds: &[
            HintRowKind::Workspace,
            HintRowKind::Repo,
            HintRowKind::Checkout,
        ],
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::Branch,
        key: "b",
        label: "branch",
        kinds: &[HintRowKind::Repo, HintRowKind::Checkout],
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::RemoveWorktree,
        key: "W",
        label: "remove worktree",
        kinds: &[HintRowKind::Checkout, HintRowKind::Repo],
        destructive: true,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::Edit,
        key: "e",
        label: "edit",
        kinds: &[HintRowKind::File],
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::ToggleViewed,
        key: "space",
        label: "reviewed",
        kinds: &[HintRowKind::File],
        destructive: false,
        depths: Some(&[0]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::FullFile,
        key: "Ctrl-o",
        label: "full file",
        kinds: &[HintRowKind::File],
        destructive: false,
        depths: None,
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::GraphCheckout,
        key: "b",
        label: "checkout",
        kinds: &[HintRowKind::GraphCommit],
        destructive: false,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::GraphCreateBranch,
        key: "c",
        label: "create branch",
        kinds: &[HintRowKind::GraphCommit],
        destructive: false,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::GraphMerge,
        key: "m",
        label: "merge",
        kinds: &[HintRowKind::GraphCommit],
        destructive: false,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::GraphFocus,
        key: "o",
        label: "focus branches",
        kinds: &[
            HintRowKind::Repo,
            HintRowKind::Checkout,
            HintRowKind::GraphCommit,
            HintRowKind::GraphStash,
            HintRowKind::GraphUncommitted,
        ],
        destructive: false,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::GraphFocusClear,
        key: "O",
        label: "clear focus",
        kinds: &[
            HintRowKind::Repo,
            HintRowKind::Checkout,
            HintRowKind::GraphCommit,
            HintRowKind::GraphStash,
            HintRowKind::GraphUncommitted,
        ],
        destructive: false,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::StashMenu,
        key: "S",
        label: "stash",
        kinds: &[
            HintRowKind::Repo,
            HintRowKind::Checkout,
            HintRowKind::Dir,
            HintRowKind::File,
            HintRowKind::GraphCommit,
            HintRowKind::GraphStash,
            HintRowKind::GraphUncommitted,
        ],
        destructive: false,
        depths: None,
        focus_left_only: true,
    },
    HintAction {
        id: HintActionId::StashApply,
        key: "a",
        label: "apply stash",
        kinds: &[HintRowKind::GraphStash],
        destructive: false,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::StashPop,
        key: "p",
        label: "pop stash",
        kinds: &[HintRowKind::GraphStash],
        destructive: false,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
    HintAction {
        id: HintActionId::StashDrop,
        key: "D",
        label: "drop stash",
        kinds: &[HintRowKind::GraphStash],
        destructive: true,
        depths: Some(&[0, 1]),
        focus_left_only: false,
    },
];

const TREE_WRITE_BLOCKED: &[HintActionId] = &[
    HintActionId::Stage,
    HintActionId::Unstage,
    HintActionId::Revert,
    HintActionId::Fetch,
    HintActionId::Pull,
    HintActionId::Push,
    HintActionId::DefaultBranch,
    HintActionId::Branch,
    HintActionId::RemoveWorktree,
];

/// Hint actions a compare tab can run. Stage / unstage and the remote,
/// branch, stash, graph, and worktree actions stay on the Workspace tab
/// (`AppState::compare_refusal`), so the hint row never offers them there.
const COMPARE_HINT_ACTIONS: &[HintActionId] = &[
    HintActionId::Revert,
    HintActionId::Edit,
    HintActionId::ToggleViewed,
    HintActionId::FullFile,
];

const GRAPH_HINT_KINDS: &[HintRowKind] = &[
    HintRowKind::GraphCommit,
    HintRowKind::GraphStash,
    HintRowKind::GraphUncommitted,
];

/// Breadcrumb row. Always painted: dialogs sit over the panes, not in
/// place of the bottom chrome.
pub fn breadcrumb_rows(_state: &AppState) -> u16 {
    1
}

/// Pinned Ctrl-C prompt row. Overlay pickers render the copy inline instead.
pub fn ctrl_c_prompt_rows(state: &AppState) -> u16 {
    u16::from(ctrl_c_prompt_pinned(state))
}

/// True when the quit prompt sits on its own chrome row (not a breadcrumb toast).
pub fn ctrl_c_prompt_pinned(state: &AppState) -> bool {
    is_ctrl_c_exit_prompt(&state.status)
        && state.stash_menu.is_none()
        && state.branch_picker.is_none()
        && state.compare_picker.is_none()
        && state.graph_focus_picker.is_none()
        && state.create_branch.is_none()
        && state.comment.is_none()
        && state.comment_export.is_none()
        && state.quick_open.is_none()
}

/// Bold quit-prompt line painted between the breadcrumb and the status / overlay.
pub fn ctrl_c_prompt_line(state: &AppState, width: u16) -> Line<'static> {
    let palette = state.theme.palette();
    Line::from(Span::styled(
        truncate_visible(&state.status, width as usize),
        Style::default()
            .fg(palette.modified)
            .add_modifier(Modifier::BOLD),
    ))
}

/// A boxed dialog painted centered over the panes.
///
/// Variants follow the paint order in [`open_dialog`]: when two overlays
/// are open at once, the first one in this list paints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DialogKind {
    /// `?` help, idle or with its `/` search.
    Help,
    /// y/n confirm for a write.
    Confirm,
    /// Stash operations menu.
    StashMenu,
    /// Graph `c` create-branch prompt.
    CreateBranch,
    /// Comment textarea.
    Comment,
    /// Comment export result.
    CommentExport,
    /// Branch checkout / create picker.
    BranchPicker,
    /// Compare branch or commit picker.
    ComparePicker,
    /// Graph branch focus picker.
    GraphFocusPicker,
    /// Quick Open: `:` files, Ctrl-k / `>` commands.
    QuickOpen,
}

/// The dialog that paints this frame, or `None` when only the panes and
/// the bottom chrome paint.
///
/// Same order as the render if-chain: help, confirm, stash, create
/// branch, comment, export, then the list pickers and the palette.
pub fn open_dialog(state: &AppState) -> Option<DialogKind> {
    if state.help_open {
        Some(DialogKind::Help)
    } else if state.confirm.is_some() {
        Some(DialogKind::Confirm)
    } else if state.stash_menu.is_some() {
        Some(DialogKind::StashMenu)
    } else if state.create_branch.is_some() {
        Some(DialogKind::CreateBranch)
    } else if state.comment.is_some() {
        Some(DialogKind::Comment)
    } else if state.comment_export.is_some() {
        Some(DialogKind::CommentExport)
    } else if state.branch_picker.is_some() {
        Some(DialogKind::BranchPicker)
    } else if state.compare_picker.is_some() {
        Some(DialogKind::ComparePicker)
    } else if state.graph_focus_picker.is_some() {
        Some(DialogKind::GraphFocusPicker)
    } else if state.quick_open.is_some() {
        Some(DialogKind::QuickOpen)
    } else {
        None
    }
}

/// Widest a dialog other than help gets, in columns.
///
/// Help lays its three columns out across the whole pane area instead.
pub const DIALOG_MAX_WIDTH: u16 = 96;

/// Border, title, and footer rows a list dialog paints around its rows.
const LIST_OVERLAY_CHROME_ROWS: u16 = 4;

/// List rows a list dialog shows; more results scroll inside the box.
///
/// The cursor row stays in the window.
pub const LIST_OVERLAY_MAX_ROWS: usize = 12;

/// Fixed height of a list dialog (pickers and Quick Open).
///
/// Border (2), query / title row, [`LIST_OVERLAY_MAX_ROWS`] rows, the
/// reserved status row, and the footer. The result count never changes it.
pub const LIST_DIALOG_ROWS: u16 = LIST_OVERLAY_CHROME_ROWS + LIST_OVERLAY_MAX_ROWS as u16 + 1;

/// Box width of `kind` inside `area` (the pane area).
///
/// Help takes the whole width less a two-column margin each side; the
/// other dialogs also stop at [`DIALOG_MAX_WIDTH`].
pub fn dialog_width(area: Rect, kind: DialogKind) -> u16 {
    let width = area.width.saturating_sub(4);
    match kind {
        DialogKind::Help => width,
        _ => width.min(DIALOG_MAX_WIDTH),
    }
}

/// Which help columns the active tab paints.
pub fn help_tab(state: &AppState) -> HelpTab {
    if state.is_file_tab() {
        HelpTab::File
    } else if state.is_compare_tab() {
        HelpTab::Compare
    } else {
        HelpTab::Workspace
    }
}

/// Box height of `kind` at box `width`, borders included.
///
/// Fixed per kind and never from a result count, so the input row stays
/// put while the list changes. [`dialog_rect`] clamps it to the panes.
pub fn dialog_height(state: &AppState, kind: DialogKind, width: u16) -> u16 {
    match kind {
        // Help lays its columns out at the box width.
        DialogKind::Help => help_status_lines(width, help_tab(state)),
        DialogKind::Confirm => match state.confirm.as_ref() {
            // Title, branch, changed-files, chips; one spare row for a wrapped detail.
            Some(super::state::PendingConfirm::RemoveWorktree { .. }) => 7,
            Some(super::state::PendingConfirm::SwitchToDefault { .. }) => 6,
            Some(
                super::state::PendingConfirm::StashDrop { .. }
                | super::state::PendingConfirm::RevertRange { .. },
            ) => 5,
            // One row per count line: mixed is 7, the others 6.
            Some(super::state::PendingConfirm::Revert { targets, .. }) => {
                5 + revert_scope(targets).count_lines()
            }
            Some(
                super::state::PendingConfirm::CheckoutOutOfSync { .. }
                | super::state::PendingConfirm::MergeIntoHead { .. }
                | super::state::PendingConfirm::CompareRevertRange { .. }
                | super::state::PendingConfirm::CompareRevertFile { .. },
            ) => 7,
            None => 0,
        },
        // Border, title, one row per op, the status row, and the footer.
        DialogKind::StashMenu => {
            let ops = state.stash_menu.as_ref().map_or(0, Vec::len) as u16;
            4u16.saturating_add(ops).saturating_add(1)
        }
        // Border, title, name, the status row (`create {name}`), and the footer.
        DialogKind::CreateBranch => 6,
        // Room for the most body lines; fewer lines leave blank rows.
        DialogKind::Comment => COMMENT_OVERLAY_CHROME_ROWS + COMMENT_OVERLAY_MAX_BODY_LINES as u16,
        DialogKind::CommentExport => 20,
        DialogKind::BranchPicker
        | DialogKind::ComparePicker
        | DialogKind::GraphFocusPicker
        | DialogKind::QuickOpen => LIST_DIALOG_ROWS,
    }
}

/// A `width` × `height` box centered in `pane_area`, clamped to it.
///
/// Dialogs paint over the panes, so the panes never shrink for one.
pub fn dialog_rect(pane_area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(pane_area.width);
    let height = height.min(pane_area.height);
    Rect {
        x: pane_area.x + (pane_area.width - width) / 2,
        y: pane_area.y + (pane_area.height - height) / 2,
        width,
        height,
    }
}

/// Plain-text join of chip key + gap + label (tests / width math).
#[allow(dead_code)]
pub fn format_hint_plain(segment: &HintSegment) -> String {
    if segment.label.is_empty() {
        return segment.key.clone();
    }
    format!(
        "{}{}{}",
        segment.key,
        " ".repeat(HINT_CHIP_GAP),
        segment.label
    )
}

fn hint_segment_columns(segment: &HintSegment) -> usize {
    if segment.label.is_empty() {
        return segment.key.chars().count() + 2;
    }
    segment.key.chars().count() + 2 + HINT_CHIP_GAP + segment.label.chars().count()
}

/// One painted piece of the hint row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HintPiece {
    /// Key chip plus label.
    Hint(HintSegment),
    /// Muted [`HINT_ELLIPSIS`] where hints were dropped.
    More,
}

fn piece_columns(piece: &HintPiece) -> usize {
    match piece {
        HintPiece::Hint(segment) => hint_segment_columns(segment),
        HintPiece::More => HINT_ELLIPSIS.chars().count(),
    }
}

fn pieces_width(pieces: &[HintPiece]) -> usize {
    let cols: usize = pieces.iter().map(piece_columns).sum();
    cols + pieces.len().saturating_sub(1) * HINT_SEPARATOR.len()
}

/// Fit `segments`, then `pinned`, into `available` columns.
///
/// Over-long lists cut rather than wrap. `segments` keep their longest
/// prefix that fits, a muted `…` marks the cut, and `pinned` (`q quit`)
/// paints after it, so the way out never truncates away.
pub fn fit_hint_segments(
    segments: &[HintSegment],
    pinned: &[HintSegment],
    available: usize,
) -> Vec<HintPiece> {
    let hints = |list: &[HintSegment]| -> Vec<HintPiece> {
        list.iter().cloned().map(HintPiece::Hint).collect()
    };
    let all: Vec<HintPiece> = hints(segments).into_iter().chain(hints(pinned)).collect();
    if pieces_width(&all) <= available {
        return all;
    }
    let tail: Vec<HintPiece> = std::iter::once(HintPiece::More)
        .chain(hints(pinned))
        .collect();
    if pieces_width(&tail) > available {
        let pinned = hints(pinned);
        return if pieces_width(&pinned) <= available {
            pinned
        } else {
            Vec::new()
        };
    }
    let mut kept = Vec::new();
    for segment in segments {
        let mut next = kept.clone();
        next.push(HintPiece::Hint(segment.clone()));
        let with_tail: Vec<HintPiece> = next.iter().chain(tail.iter()).cloned().collect();
        if pieces_width(&with_tail) > available {
            break;
        }
        kept = next;
    }
    kept.extend(tail);
    kept
}

/// Enter / Esc hints, each labelled by what the key does now.
///
/// `Enter drill` shows only when Enter opens the next depth
/// ([`AppState::nav_enter_drills`]). Esc reads `clear` while a
/// search is armed, `← <pane>` when it moves focus left, `↑ <depth>` when
/// it pops a depth, and is absent when it does nothing.
pub fn nav_chrome_hint_segments(state: &AppState) -> Vec<HintSegment> {
    let mut out = Vec::new();
    let compare = state.is_compare_tab();
    if state.focus == FocusPane::Left {
        if compare || !state.hidden_ignored_focus() {
            out.push(hint("Enter", "focus right", false));
        }
    } else if state.nav_enter_drills() {
        out.push(hint("Enter", "drill", false));
    }
    if let Some(label) = esc_hint_label(state) {
        out.push(hint("Esc", &label, false));
    }
    out
}

/// What Esc does from here, or `None` when it does nothing.
fn esc_hint_label(state: &AppState) -> Option<String> {
    if state.search_active {
        return Some("clear".into());
    }
    if state.focus == FocusPane::Right {
        return Some(format!("← {}", state.left_pane_title()));
    }
    if state.is_compare_tab() {
        return None;
    }
    match state.drill {
        DrillView::Diff { .. } => Some("↑ files".into()),
        DrillView::Files { .. } => Some("↑ graph".into()),
        DrillView::Graph => None,
    }
}

/// `n N  next / prev` chip while a search is armed. Esc comes from the nav hints.
pub fn search_hint_segments(state: &AppState) -> Vec<HintSegment> {
    if !state.search_is_armed() {
        return Vec::new();
    }
    vec![hint("n N", "next / prev", false)]
}

/// Extra chips. Appended after core hints so they truncate first.
pub fn extra_hint_segments() -> Vec<HintSegment> {
    vec![
        hint(";", "comment", false),
        hint("y", "copy comments", false),
        hint("Tab", "other pane", false),
    ]
}

/// Chips that survive truncation, painted last ([`fit_hint_segments`]).
pub fn pinned_hint_segments() -> Vec<HintSegment> {
    vec![hint("q", "quit", false)]
}

/// Hints while `V` visual-line highlight is on a focused file diff.
///
/// A compare tab has no stage / unstage, and shows `x` only while the
/// highlighted lines may revert to the merge base: the same test as the
/// palette row "Revert highlighted lines" (`highlight_revert_refusal`).
pub fn visual_hint_segments(state: &AppState) -> Vec<HintSegment> {
    // The way out first: a full row cuts the tail, not Esc.
    let mut hints = vec![
        hint("Esc", "cancel highlight", false),
        hint("j k", "extend range", false),
    ];
    if !state.is_compare_tab() {
        hints.push(hint("s u", "stage / unstage", false));
        hints.push(hint("x", "revert", true));
    } else if state.highlight_revert_refusal().is_none() {
        hints.push(hint("x", "revert to merge base", true));
    }
    hints.push(hint(";", "comment range", false));
    hints
}

fn hint(key: &str, label: &str, destructive: bool) -> HintSegment {
    HintSegment {
        key: key.into(),
        label: label.into(),
        destructive,
    }
}

fn is_graph_kind(kind: HintRowKind) -> bool {
    GRAPH_HINT_KINDS.contains(&kind)
}

/// Hints for every action valid on `kind` at the given nav dims.
///
/// A compare tab reads the compare tab, not the parked Workspace tree row:
/// only [`COMPARE_HINT_ACTIONS`], with `x` shown while it may revert the
/// focused compare file to the merge base.
pub fn action_hint_segments(state: &AppState) -> Vec<HintSegment> {
    let kind = hint_row_kind(state);
    let depth = nav_depth(state);
    let focus = state.focus;
    let compare = state.is_compare_tab();
    // Compare `x` also runs from the focused diff, so it is not hidden there.
    let hide_tree_writes = !compare && (depth >= 1 || focus == FocusPane::Right);
    HINT_ACTIONS
        .iter()
        .filter(|action| !compare || COMPARE_HINT_ACTIONS.contains(&action.id))
        .filter(|action| action.kinds.contains(&kind))
        .filter(|action| match action.depths {
            Some(depths) => depths.contains(&depth),
            None => true,
        })
        .filter(|action| !action.focus_left_only || focus == FocusPane::Left)
        .filter(|action| !(hide_tree_writes && TREE_WRITE_BLOCKED.contains(&action.id)))
        .filter(|action| {
            if is_graph_kind(kind) {
                graph_action_visible(state, action)
            } else {
                true
            }
        })
        .filter(|action| scope_action_visible(state, action, kind, depth))
        .map(|action| {
            let label = if action.id == HintActionId::RemoveWorktree {
                remove_worktree_hint_label(state)
            } else if compare && action.id == HintActionId::Revert {
                "revert to merge base".into()
            } else {
                action.label.to_string()
            };
            HintSegment {
                key: action.key.into(),
                label,
                destructive: action.destructive,
            }
        })
        .collect()
}

fn graph_action_visible(state: &AppState, action: &HintAction) -> bool {
    match action.id {
        HintActionId::GraphCheckout => focused_commit_checkoutable(state),
        HintActionId::GraphCreateBranch | HintActionId::GraphMerge => {
            matches!(state.focused_graph_row(), Some(GraphRow::Commit { .. }))
        }
        HintActionId::GraphFocus => true,
        HintActionId::GraphFocusClear => state.graph_focus_is_active(),
        HintActionId::StashApply | HintActionId::StashPop | HintActionId::StashDrop => {
            matches!(state.focused_graph_row(), Some(GraphRow::Stash(_)))
        }
        HintActionId::StashMenu => !graph_stash_ops(state).is_empty(),
        _ => true,
    }
}

fn focused_commit_checkoutable(state: &AppState) -> bool {
    match state.focused_graph_row() {
        Some(GraphRow::Commit { commit, .. }) => {
            !checkoutable_branch_names(&commit.refs).is_empty()
        }
        _ => false,
    }
}

fn graph_stash_ops(state: &AppState) -> Vec<super::stash::StashOp> {
    let dirty = state
        .graph
        .as_ref()
        .is_some_and(|model| model.uncommitted == Some(true));
    let latest = state
        .graph
        .as_ref()
        .and_then(|model| model.stashes.first().map(|stash| stash.stash_ref.clone()));
    let focused = match state.focused_graph_row() {
        Some(GraphRow::Stash(stash)) => Some(stash.stash_ref),
        _ => None,
    };
    stash_ops_for_context(&StashOpsContext {
        dirty,
        dirty_paths: None,
        focused_stash_ref: focused,
        latest_stash_ref: latest,
    })
}

fn scope_action_visible(
    state: &AppState,
    action: &HintAction,
    kind: HintRowKind,
    depth: u8,
) -> bool {
    if is_graph_kind(kind) && action.id != HintActionId::StashMenu {
        return true;
    }
    let focused = hint_tree_row(state);
    match action.id {
        HintActionId::Stage => collect_write_files(&state.snapshot, focused, state.show_ignored)
            .iter()
            .any(|file| file.change.unstaged_status.is_some() || file.change.untracked),
        HintActionId::Unstage => collect_write_files(&state.snapshot, focused, state.show_ignored)
            .iter()
            .any(|file| file.change.staged_status.is_some()),
        HintActionId::Revert if state.is_compare_tab() => {
            state.compare_file_revert_refusal().is_none()
        }
        HintActionId::Revert => collect_write_files(&state.snapshot, focused, state.show_ignored)
            .iter()
            .any(|file| file.change.unstaged_status.is_some() || file.change.untracked),
        HintActionId::Pull => op_targets(&state.snapshot, focused, state.show_ignored, Op::Pull)
            .into_iter()
            .any(|repo| {
                state
                    .snapshot
                    .repos
                    .iter()
                    .any(|row| row.repo == repo && row.sync_status == SyncStatus::Behind)
            }),
        HintActionId::Push => {
            !push_targets(&state.snapshot, focused, state.show_ignored).is_empty()
        }
        HintActionId::DefaultBranch => op_targets(
            &state.snapshot,
            focused,
            state.show_ignored,
            Op::DefaultBranch,
        )
        .into_iter()
        .any(|repo| {
            state.snapshot.repos.iter().any(|row| {
                row.repo == repo
                    && !is_default_branch(&row.branch, row.default_branch_override.as_deref())
            })
        }),
        HintActionId::Branch => {
            focused.is_some_and(|row| can_open_branch_picker(&state.snapshot, row))
        }
        HintActionId::GraphFocusClear => state.graph_focus_is_active(),
        HintActionId::RemoveWorktree => focused.is_some_and(|row| can_remove_worktree(state, row)),
        HintActionId::StashMenu => {
            if depth >= 2 {
                false
            } else if depth >= 1 {
                true
            } else {
                collect_write_files(&state.snapshot, focused, state.show_ignored)
                    .iter()
                    .any(|file| {
                        file.change.staged_status.is_some()
                            || file.change.unstaged_status.is_some()
                            || file.change.untracked
                    })
            }
        }
        HintActionId::ToggleViewed if state.is_compare_tab() => {
            state.focused_commit_edit_path().is_some()
        }
        HintActionId::ToggleViewed => {
            depth == 0
                && focused.is_some_and(|row| {
                    row.kind == NodeKind::File
                        && row.file.as_ref().is_some_and(|change| {
                            change.staged_status.is_some()
                                || change.unstaged_status.is_some()
                                || change.untracked
                        })
                })
        }
        _ => true,
    }
}

fn can_remove_worktree(state: &AppState, row: &super::tree::VisibleRow) -> bool {
    if !matches!(row.kind, NodeKind::Checkout | NodeKind::Repo) {
        return false;
    }
    let Some(path) = row.repo.as_deref() else {
        return false;
    };
    state.snapshot.repos.iter().any(|snap| {
        snap.repo == path
            && snap.checkout_kind == CheckoutKind::Linked
            && snap.primary_repo.is_some()
            && !row.chrome.is_family
    })
}

fn remove_worktree_hint_label(state: &AppState) -> String {
    let Some(row) = hint_tree_row(state) else {
        return "remove worktree".into();
    };
    if row.chrome.checkout_kind != Some(CheckoutKind::Linked) {
        return "remove worktree".into();
    }
    match row.chrome.merged_into_default {
        Some(true) => "remove worktree (merged)".into(),
        Some(false) => "remove worktree (open)".into(),
        None => "remove worktree".into(),
    }
}

fn hint_tree_row(state: &AppState) -> Option<&super::tree::VisibleRow> {
    state.focused_row()
}

/// ViewStack depth analogue: graph 0, commit files 1, commit diff 2.
///
/// A compare tab is always depth 0 even when Workspace `drill` is Files/Diff.
pub fn nav_depth(state: &AppState) -> u8 {
    if state.is_compare_tab() {
        return 0;
    }
    match state.drill {
        DrillView::Graph => 0,
        DrillView::Files { .. } => 1,
        DrillView::Diff { .. } => 2,
    }
}

/// Active row kind for the hint bar.
pub fn hint_row_kind(state: &AppState) -> HintRowKind {
    if state.graph_pane_focused() {
        return match state.focused_graph_row() {
            Some(GraphRow::Commit { .. } | GraphRow::Worktree(_)) => HintRowKind::GraphCommit,
            Some(GraphRow::Stash(_)) => HintRowKind::GraphStash,
            Some(GraphRow::Uncommitted { .. }) => HintRowKind::GraphUncommitted,
            None => HintRowKind::Workspace,
        };
    }
    if state.commit_files_list_focused() {
        return match state.focused_commit_file_kind() {
            Some(CommitFileRowKind::Dir) => HintRowKind::Dir,
            Some(CommitFileRowKind::File) | None => HintRowKind::File,
        };
    }
    if (state.drill.is_diff() || state.right_is_diff()) && state.focus == FocusPane::Right {
        // A folder summary hides the file diff: no file-diff keys.
        return if state.folder_summary().is_some() {
            HintRowKind::Dir
        } else {
            HintRowKind::File
        };
    }
    match state.focused_row().map(|row| row.kind) {
        Some(NodeKind::Workspace) => HintRowKind::Workspace,
        Some(NodeKind::Repo) => HintRowKind::Repo,
        Some(NodeKind::Checkout) => HintRowKind::Checkout,
        Some(NodeKind::Group) => HintRowKind::Group,
        Some(NodeKind::Dir | NodeKind::Section) => HintRowKind::Dir,
        Some(NodeKind::File) => HintRowKind::File,
        None => HintRowKind::Workspace,
    }
}

/// Display segments for the breadcrumb (workspace + drill frames).
///
/// A file tab reads `workspace › <checkout leaf> › <rel>`.
pub fn breadcrumb_segments(state: &AppState) -> Vec<String> {
    let mut out = vec![workspace_label(state)];
    if let Some(tab) = state.tabs.active_file() {
        out.push(checkout_leaf(&tab.checkout));
        out.push(tab.rel.clone());
        return out;
    }
    let mut seen_repo: Option<String> = None;
    let mut seen_commit: Option<String> = None;

    if nav_depth(state) == 0 {
        if let Some(repo) = focused_repo_basename(state) {
            out.push(repo);
        }
        return out;
    }

    if let Some(repo) = drill_repo_basename(state) {
        out.push(repo.clone());
        seen_repo = Some(repo);
    }
    if let Some(commit) = drill_commit_label(state) {
        out.push(commit.clone());
        seen_commit = Some(commit);
    }
    if let DrillView::Diff { repo, path, .. } = &state.drill {
        let repo_base = base_name(repo);
        if seen_repo.as_deref() != Some(repo_base.as_str()) {
            out.push(repo_base);
        }
        let hash = drill_commit_label(state);
        if hash.as_ref() != seen_commit.as_ref() {
            if let Some(hash) = hash {
                out.push(hash);
            }
        }
        out.push(base_name(path));
    }
    out
}

fn workspace_label(state: &AppState) -> String {
    state
        .cwd
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("workspace")
        .to_string()
}

fn base_name(path: &str) -> String {
    path.rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn focused_repo_basename(state: &AppState) -> Option<String> {
    state
        .focused_row()
        .and_then(|row| row.repo.as_deref())
        .map(base_name)
}

fn drill_repo_basename(state: &AppState) -> Option<String> {
    match &state.drill {
        DrillView::Files { repo, .. } | DrillView::Diff { repo, .. } => Some(base_name(repo)),
        DrillView::Graph => None,
    }
}

fn drill_commit_label(state: &AppState) -> Option<String> {
    let source = match &state.drill {
        DrillView::Files { source, .. } | DrillView::Diff { source, .. } => source,
        DrillView::Graph => return None,
    };
    Some(source.short_label())
}

/// Visual breadcrumb (`workspace › [repo]` when the last segment is right-focused).
#[allow(dead_code)]
pub fn format_breadcrumb(segments: &[String], focus: FocusPane) -> String {
    segments
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let last = i + 1 == segments.len();
            let body = if last && focus == FocusPane::Right {
                format!("[{seg}]")
            } else {
                seg.clone()
            };
            if i == 0 {
                body
            } else {
                format!("{BREADCRUMB_SEP}{body}")
            }
        })
        .collect()
}

fn allocate_chrome_row(total_width: usize, op_status_len: usize) -> (usize, usize) {
    let width = total_width;
    if op_status_len == 0 || width == 0 {
        return (width, 0);
    }
    let op_status_max = op_status_len.min(width);
    let breadcrumb_max = width.saturating_sub(op_status_max.saturating_add(1));
    (breadcrumb_max, op_status_max)
}

/// True when an open overlay paints `status` as its own prompt / filter text.
///
/// The breadcrumb slot stays empty then, and the status does not expire.
pub(crate) fn status_uses_status_text(state: &AppState) -> bool {
    state.search_mode
        || state.stash_menu.is_some()
        || state.branch_picker.is_some()
        || state.compare_picker.is_some()
        || state.graph_focus_picker.is_some()
        || state.create_branch.is_some()
        || state.comment.is_some()
        || state.comment_export.is_some()
        || state.quick_open.is_some()
}

fn breadcrumb_op_status(state: &AppState) -> String {
    if status_uses_status_text(state) || is_ctrl_c_exit_prompt(&state.status) {
        return String::new();
    }
    state.status.trim().to_string()
}

/// Breadcrumb row: path on the left, optional toast / running-op status
/// (`Pulling 1/2…`) on the right.
pub fn breadcrumb_line(state: &AppState, width: u16) -> Line<'static> {
    let palette = state.theme.palette();
    let width = width as usize;
    let op = breadcrumb_op_status(state);
    let (crumb_max, op_max) = allocate_chrome_row(width, visible_width(&op));
    let segments = breadcrumb_segments(state);
    // A file tab is one pane: no right-focus mark on the path.
    let focus = if state.is_file_tab() {
        FocusPane::Left
    } else {
        state.focus
    };
    let mut spans = Vec::new();
    let mut used = 0usize;
    for (i, seg) in segments.iter().enumerate() {
        let last = i + 1 == segments.len();
        let sep = if i == 0 { "" } else { BREADCRUMB_SEP };
        let body = if last && focus == FocusPane::Right {
            format!("[{seg}]")
        } else {
            seg.clone()
        };
        let color = if last {
            if focus == FocusPane::Right {
                palette.cursor
            } else {
                palette.heading
            }
        } else {
            palette.muted
        };
        let piece = format!("{sep}{body}");
        let piece_w = visible_width(&piece);
        if used + piece_w > crumb_max {
            let remain = crumb_max.saturating_sub(used);
            if remain > 0 {
                spans.push(Span::styled(
                    truncate_visible(&piece, remain),
                    Style::default().fg(color),
                ));
            }
            used = crumb_max;
            break;
        }
        spans.push(Span::styled(piece, Style::default().fg(color)));
        used += piece_w;
    }
    if op_max > 0 {
        let gap = crumb_max.saturating_sub(used);
        if gap > 0 {
            spans.push(Span::raw(" ".repeat(gap)));
            used += gap;
        }
        if used < width {
            spans.push(Span::raw(" "));
        }
        spans.push(Span::styled(
            truncate_visible(&op, op_max),
            Style::default().fg(state.status.kind().color(palette)),
        ));
    }
    Line::from(spans)
}

/// Status row: mode pills + contextual hint chips, or a replacing prompt.
pub fn status_line(state: &AppState, width: u16) -> Line<'static> {
    let palette = state.theme.palette();
    let pills = state.theme.pills();
    let surface = hex_color(state.theme.theme().surface);
    // Dialogs paint `status` inside the box (stash chips, picker notes, the
    // export result) or have no use for the hints. The comment box has no
    // status row, so its status stays here.
    match open_dialog(state) {
        Some(DialogKind::Comment) => {
            return Line::from(Span::styled(
                truncate_visible(&state.status, width as usize),
                Style::default().fg(state.status.kind().color(palette)),
            ));
        }
        Some(_) => return Line::default(),
        None => {}
    }
    if state.search_mode {
        return search_typing_line(state, palette, pills.filter);
    }
    if state.is_file_tab() {
        return file_tab_status_line(state, palette, pills, surface, width);
    }
    idle_status_line(state, palette, pills, surface, width)
}

/// Hint chips on a file tab, in cut order (the last ones truncate first).
pub fn file_tab_hint_segments() -> Vec<HintSegment> {
    vec![
        hint("e", "edit", false),
        hint("/", "search", false),
        hint("\\", "wrap", false),
        hint("'", "copy ref", false),
        hint("r", "reload", false),
        hint(":", "go to file", false),
    ]
}

/// Idle row on a file tab: `wrap` and armed-search pills, `? help`, the
/// viewer hints, then the pinned `q quit`.
fn file_tab_status_line(
    state: &AppState,
    palette: Palette,
    pills: Pills,
    surface: Color,
    width: u16,
) -> Line<'static> {
    let mut spans = Vec::new();
    if state.diff_wrap {
        spans.push(pill_span("wrap", pills.diff));
    }
    if let Some(label) = search_pill_label(state) {
        spans.push(pill_span(&label, pills.filter));
    }
    spans.push(Span::styled(
        " ? help".to_string(),
        Style::default().fg(palette.file),
    ));
    push_hint_pieces(
        &mut spans,
        &file_tab_hint_segments(),
        palette,
        surface,
        width,
    );
    Line::from(spans)
}

fn search_typing_line(state: &AppState, palette: Palette, filter: Pill) -> Line<'static> {
    let query = state.search_query.clone();
    // The live preview's `no match` / wrap notice replaces the hint, so it
    // shows before Enter (the idle bar that paints `status` is hidden).
    let notice = [
        SEARCH_NO_MATCH,
        SEARCH_WRAPPED_TO_TOP,
        SEARCH_WRAPPED_TO_BOTTOM,
    ]
    .contains(&&*state.status);
    let tail = if notice {
        Span::styled(
            format!("   {}", &*state.status),
            Style::default().fg(state.status.kind().color(palette)),
        )
    } else {
        Span::styled(
            format!("   {SEARCH_TYPING_HINT}"),
            Style::default().fg(palette.muted),
        )
    };
    Line::from(vec![
        pill_span("SEARCH", filter),
        Span::styled(format!(" {query}"), Style::default().fg(palette.repo)),
        Span::styled("▏", Style::default().fg(palette.cursor)),
        tail,
    ])
}

/// Diff pill text: the layout the diff paints in, `split→inline` when
/// split is preferred but the pane is too narrow for it.
pub fn diff_pill_label(preferred: DiffMode, painted: DiffMode) -> &'static str {
    match (preferred, painted) {
        (DiffMode::SideBySide, DiffMode::Inline) => "split→inline",
        (_, DiffMode::Inline) => "inline",
        (_, DiffMode::SideBySide) => "split",
    }
}

/// Armed-search pill: `/query 3/7 · graph` (match position, count, and the
/// pane `n` / `N` step). `-/7` when the cursor sits off a match.
pub fn search_pill_label(state: &AppState) -> Option<String> {
    if !state.search_is_armed() {
        return None;
    }
    let query = state.search_query.trim();
    let pane = state.search_target.title();
    Some(match state.search_match_position() {
        Some((pos, total)) => {
            let pos = pos.map_or_else(|| "-".to_string(), |p| p.to_string());
            format!("/{query} {pos}/{total} · {pane}")
        }
        None => format!("/{query} · {pane}"),
    })
}

fn idle_status_line(
    state: &AppState,
    palette: Palette,
    pills: Pills,
    surface: Color,
    width: u16,
) -> Line<'static> {
    let tree_mode = if state.is_compare_tab() || nav_depth(state) >= 1 {
        state.commit_tree_mode
    } else {
        state.tree_mode
    };
    let mode_label = if tree_mode { "tree" } else { "flat" };
    let message = if z_pending(state) { "z…" } else { "? help" };
    let visual = state.diff_visual_anchor.is_some();

    let mut spans = vec![pill_span(mode_label, pills.mode)];
    // The diff pill is the layout the open diff paints in; no diff, no pill.
    // A folder summary is not a diff.
    if state.right_is_diff() && state.folder_summary().is_none() {
        spans.push(pill_span(
            diff_pill_label(state.diff_mode, state.diff_layout()),
            pills.diff,
        ));
    }
    if let Some(label) = search_pill_label(state) {
        spans.push(pill_span(&label, pills.filter));
    }
    if visual {
        spans.push(pill_span("VISUAL", pills.filter));
    }
    spans.push(Span::styled(
        format!(" {message}"),
        Style::default().fg(palette.file),
    ));
    let hints = if visual {
        visual_hint_segments(state)
    } else {
        // Search chips follow the row's actions: the pill already says a
        // search is armed, so on a full row the actions keep their room.
        let mut hints = nav_chrome_hint_segments(state);
        hints.extend(action_hint_segments(state));
        hints.extend(search_hint_segments(state));
        hints.extend(extra_hint_segments());
        hints
    };
    push_hint_pieces(&mut spans, &hints, palette, surface, width);
    Line::from(spans)
}

/// Fit `hints` plus the pinned `q quit` into the room `spans` leave on a
/// `width`-column row, then append them as chips.
fn push_hint_pieces(
    spans: &mut Vec<Span<'static>>,
    hints: &[HintSegment],
    palette: Palette,
    surface: Color,
    width: u16,
) {
    let used = spans
        .iter()
        .map(|span| visible_width(&span.content))
        .sum::<usize>()
        + HINT_SEPARATOR.len();
    let fitted = fit_hint_segments(
        hints,
        &pinned_hint_segments(),
        (width as usize).saturating_sub(used),
    );
    if fitted.is_empty() {
        return;
    }
    spans.push(Span::raw(HINT_SEPARATOR));
    for (i, piece) in fitted.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(HINT_SEPARATOR));
        }
        let segment = match piece {
            HintPiece::Hint(segment) => segment,
            HintPiece::More => {
                spans.push(Span::styled(
                    HINT_ELLIPSIS,
                    Style::default().fg(palette.muted),
                ));
                continue;
            }
        };
        let chip_bg = if segment.destructive {
            palette.deleted
        } else {
            palette.cursor
        };
        spans.push(Span::styled(
            format!(" {} ", segment.key),
            Style::default()
                .fg(surface)
                .bg(chip_bg)
                .add_modifier(Modifier::BOLD),
        ));
        if !segment.label.is_empty() {
            spans.push(Span::raw(" ".repeat(HINT_CHIP_GAP)));
            let label_fg = if segment.destructive {
                palette.deleted
            } else {
                palette.muted
            };
            spans.push(Span::styled(
                segment.label.clone(),
                Style::default().fg(label_fg),
            ));
        }
    }
}

fn z_pending(state: &AppState) -> bool {
    state
        .z_pending_at
        .is_some_and(|at| at.elapsed() <= Duration::from_millis(DOUBLE_TAP_MS))
}

fn pill_span(label: &str, pill: Pill) -> Span<'static> {
    Span::styled(
        format!(" {label} "),
        Style::default()
            .fg(pill.fg)
            .bg(pill.bg)
            .add_modifier(Modifier::BOLD),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::tui::ctrl_c_exit::CTRL_C_EXIT_PROMPT;
    use crate::tui::state::AppState;
    use crate::tui::status::StatusMessage;
    use std::path::PathBuf;

    fn hint_of(key: &str, label: &str) -> HintSegment {
        hint(key, label, false)
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
                Vec::new()
            },
            checkout_kind: CheckoutKind::Primary,
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
        AppState::new(PathBuf::from("/tmp/workspace"), snapshot, true)
    }

    fn piece_keys(pieces: &[HintPiece]) -> Vec<String> {
        pieces
            .iter()
            .map(|piece| match piece {
                HintPiece::Hint(segment) => segment.key.clone(),
                HintPiece::More => HINT_ELLIPSIS.to_string(),
            })
            .collect()
    }

    #[test]
    fn fit_cuts_extras_first_and_keeps_quit_after_a_muted_ellipsis() {
        let core = vec![
            hint_of("Enter", "focus right"),
            hint_of("s", "stage"),
            hint_of("x", "revert"),
        ];
        let mut all = core.clone();
        all.extend(extra_hint_segments());
        let pinned = pinned_hint_segments();
        let core_pieces: Vec<HintPiece> = core.iter().cloned().map(HintPiece::Hint).collect();
        let tail = [HintPiece::More, HintPiece::Hint(hint_of("q", "quit"))];
        let width = pieces_width(&[core_pieces.as_slice(), &tail].concat());
        let keys = piece_keys(&fit_hint_segments(&all, &pinned, width));
        assert_eq!(keys, vec!["Enter", "s", "x", HINT_ELLIPSIS, "q"]);
        // Everything fits: no ellipsis, quit last.
        let keys = piece_keys(&fit_hint_segments(&all, &pinned, 400));
        assert_eq!(keys.last().map(String::as_str), Some("q"));
        assert!(!keys.contains(&HINT_ELLIPSIS.to_string()), "{keys:?}");
        // Too narrow for anything but quit.
        let keys = piece_keys(&fit_hint_segments(&all, &pinned, 9));
        assert_eq!(keys, vec!["q"]);
        assert!(fit_hint_segments(&all, &pinned, 8).is_empty());
    }

    #[test]
    fn status_row_keeps_help_and_quit_at_140_and_80_cols() {
        let mut app = state();
        let idx = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.cursor = idx;
        for width in [140u16, 80] {
            let line = status_line(&app, width);
            let text = line_plain(&line);
            assert!(text.contains("? help"), "{width}: {text}");
            assert!(text.trim_end().ends_with(" q   quit"), "{width}: {text}");
            assert!(visible_width(&text) <= width as usize, "{width}: {text}");
            // The ellipsis is muted text, never a key chip.
            for span in &line.spans {
                if span.content.contains(HINT_ELLIPSIS) {
                    assert_eq!(span.content.as_ref(), HINT_ELLIPSIS, "{width}");
                    assert_eq!(span.style.bg, None, "{width}");
                    assert_eq!(span.style.fg, Some(app.theme.palette().muted));
                }
            }
        }
        let narrow = line_plain(&status_line(&app, 80));
        assert!(narrow.contains(HINT_ELLIPSIS), "{narrow}");
    }

    fn nav_keys(app: &AppState) -> Vec<(String, String)> {
        nav_chrome_hint_segments(app)
            .into_iter()
            .map(|s| (s.key, s.label))
            .collect()
    }

    fn pair(key: &str, label: &str) -> (String, String) {
        (key.to_string(), label.to_string())
    }

    #[test]
    fn nav_hints_say_what_enter_and_esc_do() {
        let mut app = state();
        let repo = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Repo && row.repo.as_deref() == Some("app"))
            .expect("app repo");
        app.cursor = repo;
        assert_eq!(nav_keys(&app), vec![pair("Enter", "focus right")]);
        // Right-focused graph with no commit loaded: Enter cannot drill.
        app.focus = FocusPane::Right;
        assert_eq!(nav_keys(&app), vec![pair("Esc", "← tree")]);
        // A depth-0 file diff never drills.
        let file = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.cursor = file;
        assert!(app.right_is_diff());
        assert_eq!(nav_keys(&app), vec![pair("Esc", "← tree")]);
        // An armed search: Esc clears it first.
        app.search_active = true;
        app.search_query = "READ".into();
        assert_eq!(nav_keys(&app), vec![pair("Esc", "clear")]);
    }

    #[test]
    fn nav_hints_name_the_depth_esc_pops_to() {
        use crate::tui::drill::{CommitFile, CommitFileSource, DrillView};
        let mut app = state();
        let files = vec![CommitFile {
            status: "M".into(),
            path: "src/a.rs".into(),
            old_path: None,
            stat: None,
        }];
        let source = CommitFileSource::Commit {
            commit_id: "abc1234".into(),
        };
        app.drill = DrillView::Files {
            repo: "app".into(),
            source: source.clone(),
            files: files.clone(),
            cursor: 0,
        };
        app.focus = FocusPane::Left;
        assert_eq!(
            nav_keys(&app),
            vec![pair("Enter", "focus right"), pair("Esc", "↑ graph")]
        );
        app.focus = FocusPane::Right;
        // Cursor 0 is the `src` directory row: Enter folds, it does not drill.
        assert_eq!(nav_keys(&app), vec![pair("Esc", "← graph")]);
        if let DrillView::Files { cursor, .. } = &mut app.drill {
            *cursor = 1;
        }
        assert_eq!(
            nav_keys(&app),
            vec![pair("Enter", "drill"), pair("Esc", "← graph")]
        );
        app.drill = DrillView::Diff {
            repo: "app".into(),
            source,
            files,
            file_cursor: 1,
            path: "src/a.rs".into(),
            content: Default::default(),
        };
        assert_eq!(nav_keys(&app), vec![pair("Esc", "← files")]);
        app.focus = FocusPane::Left;
        assert_eq!(
            nav_keys(&app),
            vec![pair("Enter", "focus right"), pair("Esc", "↑ files")]
        );
    }

    #[test]
    fn compare_right_pane_shows_no_drill_hint() {
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Right;
        assert_eq!(nav_keys(&app), vec![pair("Esc", "← files")]);
        app.focus = FocusPane::Left;
        assert_eq!(nav_keys(&app), vec![pair("Enter", "focus right")]);
    }

    #[test]
    fn format_hint_plain_joins_with_chip_gap() {
        assert_eq!(HINT_CHIP_GAP, 2);
        assert_eq!(format_hint_plain(&hint_of("s", "stage")), "s  stage");
    }

    #[test]
    fn repo_hints_include_graph_focus_file_hints_do_not() {
        let mut app = state();
        let idx = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Repo && row.repo.as_deref() == Some("app"))
            .expect("app repo");
        app.cursor = idx;
        let keys: Vec<String> = action_hint_segments(&app)
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(keys.contains(&"o".into()), "{keys:?}");
        assert!(
            !keys.contains(&"O".into()),
            "clear-focus hint stays off until a focus is on: {keys:?}"
        );

        app.graph_branch_focus = Some(("app".into(), vec!["main".into()]));
        let keys: Vec<String> = action_hint_segments(&app)
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(keys.contains(&"o".into()), "{keys:?}");
        assert!(keys.contains(&"O".into()), "{keys:?}");

        let file = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.cursor = file;
        let keys: Vec<String> = action_hint_segments(&app)
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(!keys.contains(&"o".into()), "{keys:?}");
        assert!(!keys.contains(&"O".into()), "{keys:?}");
    }

    #[test]
    fn compare_space_hint_follows_file_focus() {
        use crate::tui::drill::{CommitFile, CommitFileSource};
        let mut app = state();
        app.tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        app.focus = FocusPane::Left;
        {
            let tab = app.tabs.active_compare_mut().unwrap();
            tab.source = Some(CommitFileSource::Compare {
                base_ref: "main".into(),
                head_ref: "HEAD".into(),
                base_tip: "bbb".into(),
                merge_base: "aaa".into(),
                head: "ccc".into(),
            });
            tab.files = vec![CommitFile {
                status: "M".into(),
                path: "src/a.rs".into(),
                old_path: None,
                stat: None,
            }];
        }
        let keys = |app: &AppState| -> Vec<String> {
            action_hint_segments(app)
                .into_iter()
                .map(|s| s.key)
                .collect()
        };
        app.tabs.active_compare_mut().unwrap().file_cursor = 0;
        assert!(!keys(&app).contains(&"space".into()), "dir row");
        app.tabs.active_compare_mut().unwrap().file_cursor = 1;
        assert!(keys(&app).contains(&"space".into()), "file row");
        app.focus = FocusPane::Right;
        assert!(!keys(&app).contains(&"space".into()), "no open diff");
        app.tabs.active_compare_mut().unwrap().path = Some("src/a.rs".into());
        assert!(keys(&app).contains(&"space".into()), "open diff");
    }

    #[test]
    fn idle_file_hints_include_stage_and_extras_in_the_list() {
        let mut app = state();
        let idx = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.cursor = idx;
        let keys: Vec<String> = action_hint_segments(&app)
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(keys.contains(&"s".into()), "{keys:?}");
        assert!(keys.contains(&"x".into()), "{keys:?}");
        let extras: Vec<String> = extra_hint_segments().into_iter().map(|s| s.key).collect();
        assert_eq!(
            extras,
            vec![";".to_string(), "y".to_string(), "Tab".to_string()]
        );
        assert_eq!(pinned_hint_segments(), vec![hint_of("q", "quit")]);
        let visual: Vec<String> = visual_hint_segments(&app)
            .into_iter()
            .map(|s| s.key)
            .collect();
        assert!(visual.contains(&"s u".into()), "{visual:?}");
        assert!(visual.contains(&"x".into()), "{visual:?}");
        assert!(visual.contains(&"j k".into()), "{visual:?}");
    }

    #[test]
    fn diff_pill_shows_the_painted_layout_only_with_a_diff() {
        assert_eq!(
            diff_pill_label(DiffMode::SideBySide, DiffMode::SideBySide),
            "split"
        );
        assert_eq!(
            diff_pill_label(DiffMode::SideBySide, DiffMode::Inline),
            "split→inline"
        );
        assert_eq!(
            diff_pill_label(DiffMode::Inline, DiffMode::Inline),
            "inline"
        );
        let mut app = state();
        let repo = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Repo && row.repo.as_deref() == Some("app"))
            .expect("app repo");
        app.cursor = repo;
        let text = line_plain(&status_line(&app, 200));
        assert!(
            !text.contains("split") && !text.contains("inline"),
            "{text}"
        );
        let file = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.cursor = file;
        app.layout.diff_pane_width = 80;
        let text = line_plain(&status_line(&app, 200));
        assert!(text.contains(" split→inline "), "{text}");
        app.layout.diff_pane_width = 140;
        let text = line_plain(&status_line(&app, 200));
        assert!(text.contains(" split ") && !text.contains("→"), "{text}");
        app.diff_mode = DiffMode::Inline;
        let text = line_plain(&status_line(&app, 200));
        assert!(text.contains(" inline "), "{text}");
    }

    #[test]
    fn armed_search_shows_position_pane_and_step_chips() {
        let mut app = state();
        app.search_active = true;
        app.search_query = "README".into();
        app.search_target = crate::tui::search::SearchPane::Tree;
        let first = app
            .rows
            .iter()
            .position(|row| row.label.contains("README"))
            .expect("README row");
        app.cursor = first;
        let label = search_pill_label(&app).expect("armed");
        assert_eq!(label, "/README 1/1 · tree");
        app.cursor = 0;
        assert_eq!(
            search_pill_label(&app).as_deref(),
            Some("/README -/1 · tree")
        );
        let keys: Vec<(String, String)> = nav_chrome_hint_segments(&app)
            .into_iter()
            .chain(search_hint_segments(&app))
            .map(|s| (s.key, s.label))
            .collect();
        assert_eq!(
            keys,
            vec![
                pair("Enter", "focus right"),
                pair("Esc", "clear"),
                pair("n N", "next / prev"),
            ]
        );
        let text = line_plain(&status_line(&app, 200));
        assert!(text.contains("/README -/1 · tree"), "{text}");
        app.search_active = false;
        assert!(search_pill_label(&app).is_none());
        assert!(search_hint_segments(&app).is_empty());
    }

    #[test]
    fn breadcrumb_keeps_the_repo_on_a_file_row() {
        let mut app = state();
        let file = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.cursor = file;
        assert!(app.right_is_diff());
        let repo = app.rows[file].repo.clone().expect("repo");
        assert_eq!(
            breadcrumb_segments(&app),
            vec!["workspace".to_string(), repo]
        );
    }

    #[test]
    fn breadcrumb_marks_right_focus() {
        let mut app = state();
        let idx = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Repo && row.repo.as_deref() == Some("app"))
            .expect("app repo");
        app.cursor = idx;
        app.focus = FocusPane::Right;
        let text = format_breadcrumb(&breadcrumb_segments(&app), app.focus);
        assert!(text.contains('›'), "{text}");
        assert!(text.contains("[app]"), "{text}");
    }

    #[test]
    fn typing_line_shows_the_preview_wrap_and_no_match_before_enter() {
        use crate::tui::action::Action;
        let mut app = state();
        let readme = app
            .rows
            .iter()
            .position(|row| row.id == "file:app:README.md")
            .expect("app README");
        // From the last row, the only `README` match is behind the cursor.
        app.cursor = app.rows.len() - 1;
        assert!(readme < app.cursor);
        app.dispatch(Action::SearchStart);
        let text = line_plain(&status_line(&app, 200));
        assert!(text.contains(SEARCH_TYPING_HINT), "{text}");
        for c in "README".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        assert!(app.search_mode);
        let line = status_line(&app, 200);
        let text = line_plain(&line);
        assert!(text.contains(SEARCH_WRAPPED_TO_TOP), "{text}");
        assert!(!text.contains(SEARCH_TYPING_HINT), "{text}");
        let notice = line
            .spans
            .iter()
            .find(|span| span.content.contains(SEARCH_WRAPPED_TO_TOP))
            .expect("notice span");
        assert_eq!(
            notice.style.fg,
            Some(app.status.kind().color(app.theme.palette()))
        );
        app.dispatch(Action::SearchChar('Z'));
        let text = line_plain(&status_line(&app, 200));
        assert!(text.contains(SEARCH_NO_MATCH), "{text}");
        assert!(!text.contains(SEARCH_TYPING_HINT), "{text}");
        app.status = StatusMessage::info("/");
        let text = line_plain(&status_line(&app, 200));
        assert!(text.contains(SEARCH_TYPING_HINT), "{text}");
    }

    #[test]
    fn search_typing_hint_does_not_advertise_n_as_next() {
        assert!(SEARCH_TYPING_HINT.contains("Enter arms query"));
        assert!(SEARCH_TYPING_HINT.contains("n/N after Enter"));
        assert!(!SEARCH_TYPING_HINT.contains("n/N next/prev"));
    }

    #[test]
    fn breadcrumb_stays_while_help_open() {
        let idle = state();
        assert_eq!(breadcrumb_rows(&idle), 1);
        assert_eq!(open_dialog(&idle), None);
        let mut help = state();
        help.help_open = true;
        assert_eq!(breadcrumb_rows(&help), 1);
        assert_eq!(open_dialog(&help), Some(DialogKind::Help));
        assert_eq!(
            dialog_height(&help, DialogKind::Help, 116),
            help_status_lines(116, crate::tui::help::HelpTab::Workspace)
        );
    }

    #[test]
    fn dialog_rect_centers_inside_pane_area() {
        let area = Rect::new(0, 1, 120, 30);
        let width = dialog_width(area, DialogKind::QuickOpen);
        assert_eq!(width, DIALOG_MAX_WIDTH);
        let rect = dialog_rect(area, width, LIST_DIALOG_ROWS);
        assert_eq!(rect.width, 96);
        assert_eq!(rect.x, 12);
        assert_eq!(rect.height, LIST_DIALOG_ROWS);
        assert_eq!(rect.y, 1 + (30 - LIST_DIALOG_ROWS) / 2);
        assert_eq!(dialog_width(area, DialogKind::Help), 116);
    }

    #[test]
    fn dialog_rect_clamps_to_small_pane_area() {
        let area = Rect::new(0, 1, 46, 9);
        let width = dialog_width(area, DialogKind::BranchPicker);
        let rect = dialog_rect(area, width, LIST_DIALOG_ROWS);
        assert_eq!(rect.width, 42);
        assert_eq!(rect.height, 9);
        assert!(rect.x >= area.x && rect.right() <= area.right(), "{rect:?}");
        assert!(
            rect.y >= area.y && rect.bottom() <= area.bottom(),
            "{rect:?}"
        );
    }

    #[test]
    fn dialog_height_is_fixed_per_kind() {
        use crate::git::LocalBranch;
        use crate::tui::branches::{BranchPickerState, CreateBranchState};
        use crate::tui::comments::{CommentKey, CommentPrompt};
        let branches = |n: usize| {
            (0..n)
                .map(|i| LocalBranch {
                    name: format!("topic/{i}"),
                    current: false,
                    authordate: 0,
                })
                .collect::<Vec<_>>()
        };
        let mut app = state();
        app.branch_picker = Some(BranchPickerState::checkout("app".into(), branches(1)));
        assert_eq!(open_dialog(&app), Some(DialogKind::BranchPicker));
        let one = dialog_height(&app, DialogKind::BranchPicker, 96);
        app.branch_picker = Some(BranchPickerState::checkout("app".into(), branches(40)));
        let many = dialog_height(&app, DialogKind::BranchPicker, 96);
        assert_eq!(one, LIST_DIALOG_ROWS);
        assert_eq!(many, LIST_DIALOG_ROWS);
        app.branch_picker = None;

        let mut prompt = CommentPrompt::new(
            CommentKey::Branch {
                repo: "app".into(),
                branch: "feature/x".into(),
            },
            "one".into(),
            "app · branch feature/x".into(),
        );
        app.comment = Some(prompt.clone());
        assert_eq!(dialog_height(&app, DialogKind::Comment, 96), 14);
        prompt.insert_newline();
        prompt.insert_newline();
        assert_eq!(prompt.line_count(), 3);
        app.comment = Some(prompt);
        assert_eq!(dialog_height(&app, DialogKind::Comment, 96), 14);
        app.comment = None;

        app.create_branch = Some(CreateBranchState {
            repo: "app".into(),
            name: String::new(),
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        });
        assert_eq!(dialog_height(&app, DialogKind::CreateBranch, 96), 6);
        app.status = "create topic".into();
        assert_eq!(dialog_height(&app, DialogKind::CreateBranch, 96), 6);
    }

    #[test]
    fn ctrl_c_prompt_is_pinned_not_a_breadcrumb_toast() {
        let mut app = state();
        app.status = CTRL_C_EXIT_PROMPT.into();
        let crumb = line_plain(&breadcrumb_line(&app, 80));
        assert!(
            !crumb.contains("Ctrl-c"),
            "quit prompt must not sit in the breadcrumb toast: {crumb:?}"
        );
        assert_eq!(ctrl_c_prompt_rows(&app), 1);
        assert_eq!(breadcrumb_rows(&app), 1);
        let prompt = line_plain(&ctrl_c_prompt_line(&app, 80));
        assert!(
            prompt.contains("Ctrl-c again"),
            "pinned row should show the quit prompt: {prompt:?}"
        );
        let status = line_plain(&status_line(&app, 80));
        assert!(
            !status.contains("Ctrl-c again"),
            "status line keeps pills/hints: {status:?}"
        );
        app.stash_menu = Some(Vec::new());
        assert_eq!(
            ctrl_c_prompt_rows(&app),
            0,
            "overlay pickers render the copy inline"
        );
    }

    #[test]
    fn overlay_status_paints_in_the_box_not_the_status_row() {
        let mut app = state();
        app.stash_menu = Some(Vec::new());
        app.status = "stash  s stash  a apply".into();
        assert_eq!(line_plain(&status_line(&app, 80)).trim(), "");
        app.stash_menu = None;
        app.comment_export = Some(crate::tui::comments::CommentExport {
            markdown: "# Comments\n".into(),
            copied: Some(true),
        });
        app.status = "busy".into();
        assert_eq!(line_plain(&status_line(&app, 80)).trim(), "");
        assert!(export_shows_status(&app.status));
        assert!(!export_shows_status(STATUS_COPIED));
        assert!(!export_shows_status(STATUS_COPY_FAILED));
    }

    #[test]
    fn confirm_dialog_height_per_variant() {
        let mut app = state();
        let target = |path: &str, untracked: bool| super::super::state::RevertTarget {
            repo: "app".into(),
            path: path.into(),
            untracked,
            old_path: None,
        };
        // Tracked-only scope paints one count line.
        app.confirm = Some(super::super::state::PendingConfirm::Revert {
            targets: vec![target("README.md", false)],
            label: "README.md".into(),
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 6);
        assert_eq!(open_dialog(&app), Some(DialogKind::Confirm));
        assert_eq!(breadcrumb_rows(&app), 1);
        // Mixed scope paints tracked and untracked count lines.
        app.confirm = Some(super::super::state::PendingConfirm::Revert {
            targets: vec![target("README.md", false), target("new.txt", true)],
            label: "app".into(),
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 7);
        app.confirm = Some(super::super::state::PendingConfirm::Revert {
            targets: vec![target("a.txt", true), target("b.txt", true)],
            label: "app".into(),
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 6);
        app.confirm = Some(super::super::state::PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 5);
        app.confirm = Some(super::super::state::PendingConfirm::RevertRange {
            repo: "app".into(),
            path: "README.md".into(),
            patch: String::new(),
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 5);
        app.confirm = Some(super::super::state::PendingConfirm::RemoveWorktree {
            primary: "app".into(),
            path: ".worktrees/topic".into(),
            force: false,
            branch: "topic".into(),
            merged_into_default: Some(true),
            changed: 0,
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 7);
        app.confirm = Some(super::super::state::PendingConfirm::CheckoutOutOfSync {
            repo: "app".into(),
            branch: "main".into(),
            remote_ref: "origin/main".into(),
            ahead_behind: Some((0, 2)),
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 7);
        app.confirm = Some(super::super::state::PendingConfirm::SwitchToDefault {
            repos: vec!["app".into(), "lib".into()],
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 6);
        app.confirm = Some(super::super::state::PendingConfirm::MergeIntoHead {
            repo: "app".into(),
            rev: "topic".into(),
            label: "topic".into(),
            into: "main".into(),
        });
        assert_eq!(dialog_height(&app, DialogKind::Confirm, 96), 7);
    }

    #[test]
    fn create_branch_and_stash_reserve_their_status_row() {
        use crate::tui::branches::CreateBranchState;
        let mut app = state();
        app.create_branch = Some(CreateBranchState {
            repo: "app".into(),
            name: String::new(),
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        });
        assert_eq!(dialog_height(&app, DialogKind::CreateBranch, 96), 6);
        app.status = "create topic".into();
        assert_eq!(dialog_height(&app, DialogKind::CreateBranch, 96), 6);
        app.create_branch = None;
        app.stash_menu = Some(Vec::new());
        app.status.clear();
        assert_eq!(dialog_height(&app, DialogKind::StashMenu, 96), 5);
        app.status = "stash  s stash".into();
        assert_eq!(dialog_height(&app, DialogKind::StashMenu, 96), 5);
    }

    #[test]
    fn comment_dialog_ignores_status_echo() {
        use crate::tui::comments::{CommentExport, CommentKey, CommentPrompt};
        let mut app = state();
        app.comment = Some(CommentPrompt::new(
            CommentKey::Branch {
                repo: "app".into(),
                branch: "feature/x".into(),
            },
            String::new(),
            "app · branch feature/x".into(),
        ));
        assert_eq!(dialog_height(&app, DialogKind::Comment, 96), 14);
        app.status = "body: hello".into();
        assert_eq!(
            dialog_height(&app, DialogKind::Comment, 96),
            14,
            "idle / typing status must not grow the comment dialog"
        );
        assert_eq!(
            line_plain(&status_line(&app, 80)),
            "body: hello",
            "the comment box has no status row; the status line keeps it"
        );
        if let Some(prompt) = app.comment.as_mut() {
            prompt.insert_newline();
        }
        assert_eq!(
            dialog_height(&app, DialogKind::Comment, 96),
            14,
            "a second body line does not grow the box"
        );
        app.comment = None;
        app.status = STATUS_COPIED.into();
        app.comment_export = Some(CommentExport {
            markdown: "# Comments\n\nNo comments.\n".into(),
            copied: Some(true),
        });
        assert_eq!(dialog_height(&app, DialogKind::CommentExport, 96), 20);
    }

    #[test]
    fn quick_open_uses_list_dialog_height() {
        use crate::tui::action::QuickOpenEntry;
        use crate::tui::quick_open::{QuickOpenScope, QuickOpenState};
        let mut app = state();
        app.quick_open = Some(QuickOpenState::new(
            QuickOpenEntry::Commands,
            QuickOpenScope::Workspace,
        ));
        assert_eq!(open_dialog(&app), Some(DialogKind::QuickOpen));
        assert_eq!(LIST_DIALOG_ROWS, 17);
        assert_eq!(
            dialog_height(&app, DialogKind::QuickOpen, 96),
            LIST_DIALOG_ROWS
        );
        assert_eq!(line_plain(&status_line(&app, 80)), "");
        app.status = "Press Ctrl-c again to exit".into();
        assert_eq!(
            ctrl_c_prompt_rows(&app),
            0,
            "Quick Open shows the quit prompt inline"
        );
    }

    fn line_plain(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn file_tab_chrome_names_the_file_and_viewer_keys() {
        let mut app = state();
        app.focus = FocusPane::Right;
        app.open_file_tab("app".into(), "src/main.rs".into());
        assert_eq!(
            breadcrumb_segments(&app),
            ["workspace", "app", "src/main.rs"]
        );
        assert_eq!(
            line_plain(&breadcrumb_line(&app, 80)),
            "workspace › app › src/main.rs",
            "no right-focus mark on a file tab"
        );
        assert_eq!(help_tab(&app), HelpTab::File);
        // A file tab opens wrapped: it shares the diff wrap flag (on by default).
        assert!(app.diff_wrap);
        let row = line_plain(&status_line(&app, 160));
        assert!(
            row.starts_with(" wrap  ? help"),
            "no tree / diff pill: {row}"
        );
        app.diff_wrap = false;
        let row = line_plain(&status_line(&app, 160));
        assert!(row.starts_with(" ? help"), "no tree / diff pill: {row}");
        for chip in [
            "edit",
            "search",
            "wrap",
            "copy ref",
            "reload",
            "go to file",
            "quit",
        ] {
            assert!(row.contains(chip), "{chip}: {row}");
        }
        assert!(!row.contains("stage"), "{row}");
        app.diff_wrap = true;
        let row = line_plain(&status_line(&app, 60));
        assert!(row.starts_with(" wrap "), "{row}");
        assert!(
            row.trim_end().ends_with("quit"),
            "q quit stays pinned: {row}"
        );
    }

    #[test]
    fn running_op_progress_sits_on_breadcrumb_not_status_hints() {
        use super::super::ops::{format_running_op, RunningOp};
        let mut app = state();
        app.status = StatusMessage::progress(format_running_op(RunningOp::Pull, 1, 2));
        let crumb = line_plain(&breadcrumb_line(&app, 80));
        assert!(
            crumb.contains("Pulling 1/2…"),
            "breadcrumb trailing slot should show progress: {crumb:?}"
        );
        assert!(
            crumb.find("workspace").unwrap() < crumb.find("Pulling").unwrap(),
            "progress is trailing: {crumb:?}"
        );
        let status = line_plain(&status_line(&app, 80));
        assert!(
            !status.contains("Pulling"),
            "status line keeps pills/hints: {status:?}"
        );
        assert!(status.contains("? help"), "{status:?}");
    }

    #[test]
    fn breadcrumb_truncates_path_before_running_op() {
        use super::super::ops::{format_running_op, RunningOp};
        let mut app = state();
        app.status = StatusMessage::progress(format_running_op(RunningOp::Fetch, 2, 18));
        let crumb = line_plain(&breadcrumb_line(&app, 20));
        assert!(
            crumb.contains("Fetching") || crumb.contains("2/18"),
            "narrow row still keeps op status: {crumb:?}"
        );
    }

    #[test]
    fn completed_op_summary_names_only_the_first_failure() {
        use super::super::ops::{format_completed_op, OpTally, RepoOpResult, RunningOp};
        let mut app = state();
        let mut tally = OpTally::default();
        for repo in ["notes", "dotfiles", "lib"] {
            tally.note(repo, RepoOpResult::Ok);
        }
        tally.note("app", RepoOpResult::Failed("timeout".into()));
        app.status = StatusMessage::error(format_completed_op(RunningOp::Fetch, &tally));
        let crumb = line_plain(&breadcrumb_line(&app, 80));
        assert!(
            crumb.contains("Fetched 4 repos (1 failed: app — timeout)"),
            "breadcrumb trailing slot should show counts and the failure: {crumb:?}"
        );
        assert!(
            !crumb.contains("notes") && !crumb.contains("dotfiles"),
            "repos that worked are not listed: {crumb:?}"
        );
        let status = line_plain(&status_line(&app, 80));
        assert!(
            !status.contains("Fetched"),
            "status line keeps pills/hints: {status:?}"
        );
    }

    #[test]
    fn breadcrumb_status_color_follows_kind() {
        let mut app = state();
        let palette = app.theme.palette();
        for (status, want) in [
            (StatusMessage::info("showing ignored repos"), palette.muted),
            (StatusMessage::progress("Fetching 1/2…"), palette.muted),
            (StatusMessage::ok("Fetched 2 repos"), palette.added),
            (StatusMessage::warn("nothing to push"), palette.modified),
            (StatusMessage::error("push failed"), palette.deleted),
        ] {
            let text = status.to_string();
            app.status = status;
            let line = breadcrumb_line(&app, 100);
            let span = line
                .spans
                .iter()
                .find(|span| span.content.contains(&text))
                .unwrap_or_else(|| panic!("{text} not painted: {line:?}"));
            assert_eq!(span.style.fg, Some(want), "{text}");
        }
    }
}

//! Bottom chrome: status pills, hint chips, and breadcrumb.
//!
//! Confirm overlays are boxed, not status-line y/n.

use std::time::Duration;

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use workspace_status_graph::GraphRow;

use crate::helpers::{is_default_branch, visible_width};
use crate::snapshot::{CheckoutKind, SyncStatus};

use super::branches::{can_open_branch_picker, checkoutable_branch_names};
use super::commit_files::CommitFileRowKind;
use super::ctrl_c_exit::{is_ctrl_c_exit_prompt, CTRL_C_EXIT_PROMPT};
use super::drill::{CommitFileSource, DrillView};
use super::help::help_status_lines;
use super::icons::truncate_visible;
use super::keys::DOUBLE_TAP_MS;
use super::ops::{collect_write_files, op_targets, push_targets, Op};
use super::split::DiffMode;
use super::stash::{stash_ops_for_context, StashOpsContext};
use super::state::{AppState, FocusPane};
use super::theme::{hex_color, Palette, Pill, Pills};
use super::tree::NodeKind;

/// Columns between a hint chip and its label.
pub const HINT_CHIP_GAP: usize = 2;
/// Gap rendered between two hints.
const HINT_SEPARATOR: &str = "  ";
/// Marker appended when hints were dropped to fit the terminal width.
const HINT_ELLIPSIS: &str = "…";
/// Status-bar copy while `/` search is in typing mode.
pub const SEARCH_TYPING_HINT: &str = "Enter arms query · Esc clears · n/N after Enter";
const EASY_MOTION_HINT: &str = "type label · Esc cancels";
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
        key: "ctrl+o",
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

const GRAPH_HINT_KINDS: &[HintRowKind] = &[
    HintRowKind::GraphCommit,
    HintRowKind::GraphStash,
    HintRowKind::GraphUncommitted,
];

/// Rows reserved below the panes for breadcrumb + status / overlay.
///
/// Help hides the breadcrumb. Confirms,
/// stash, create-branch, and pickers replace the status line with a
/// boxed overlay and shrink the panes by the overlay row budget.
#[allow(dead_code)]
pub fn bottom_chrome_rows(state: &AppState) -> u16 {
    breadcrumb_rows(state)
        .saturating_add(ctrl_c_prompt_rows(state))
        .saturating_add(overlay_status_rows(state))
}

/// Breadcrumb row. Hidden while `?` help is open.
pub fn breadcrumb_rows(state: &AppState) -> u16 {
    u16::from(!state.help_open)
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
        && state.create_branch.is_none()
}

/// Bold quit-prompt line painted between the breadcrumb and the status / overlay.
pub fn ctrl_c_prompt_line(state: &AppState, width: u16) -> Line<'static> {
    let palette = state.theme.palette();
    Line::from(Span::styled(
        truncate_visible(CTRL_C_EXIT_PROMPT, width as usize),
        Style::default()
            .fg(palette.modified)
            .add_modifier(Modifier::BOLD),
    ))
}

/// Status line or replacing overlay rows.
pub fn overlay_status_rows(state: &AppState) -> u16 {
    overlay_status_rows_for(state, state.layout.term_cols.max(1))
}

/// Overlay row budget at `term_cols` so wrap math can follow a resize.
pub fn overlay_status_rows_for(state: &AppState, term_cols: u16) -> u16 {
    if state.help_open {
        return help_status_lines(term_cols);
    }
    if let Some(pending) = state.confirm.as_ref() {
        return match pending {
            super::state::PendingConfirm::RemoveWorktree { .. } => 6,
            super::state::PendingConfirm::StashDrop { .. } => 5,
            super::state::PendingConfirm::Revert { .. }
            | super::state::PendingConfirm::CheckoutOutOfSync { .. }
            | super::state::PendingConfirm::MergeIntoHead { .. } => 7,
        };
    }
    if let Some(ops) = state.stash_menu.as_ref() {
        let extra = u16::from(!state.status.is_empty());
        return 4u16.saturating_add(ops.len() as u16).saturating_add(extra);
    }
    if state.create_branch.is_some() {
        return 5;
    }
    if let Some(picker) = state.branch_picker.as_ref() {
        let n = picker.visible().len().max(1).min(12) as u16;
        let extra = u16::from(!state.status.is_empty());
        return (4u16.saturating_add(n).saturating_add(extra)).min(17);
    }
    1
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

fn hints_width(kept: &[HintSegment]) -> usize {
    let cols: usize = kept.iter().map(hint_segment_columns).sum();
    cols + kept.len().saturating_sub(1) * HINT_SEPARATOR.len()
}

/// Longest prefix of `segments` that fits in `available` columns.
///
/// Over-long lists cut rather than wrap. A `…` chip marks truncation.
pub fn fit_hint_segments(segments: &[HintSegment], available: usize) -> Vec<HintSegment> {
    let mut kept = Vec::new();
    for segment in segments {
        let mut next = kept.clone();
        next.push(segment.clone());
        if hints_width(&next) > available {
            break;
        }
        kept.push(segment.clone());
    }
    if kept.len() == segments.len() {
        return kept;
    }
    let ellipsis = HintSegment {
        key: HINT_ELLIPSIS.into(),
        label: String::new(),
        destructive: false,
    };
    while !kept.is_empty()
        && hints_width(&[kept.as_slice(), &[ellipsis.clone()]].concat()) > available
    {
        kept.pop();
    }
    if kept.is_empty() {
        Vec::new()
    } else {
        kept.push(ellipsis);
        kept
    }
}

/// Enter/Esc chrome hints (not registry actions).
pub fn nav_chrome_hint_segments(depth: u8, focus: FocusPane) -> Vec<HintSegment> {
    let mut out = Vec::new();
    if focus == FocusPane::Left {
        out.push(hint("⏎", "focus right", false));
        if depth > 0 {
            out.push(hint("Esc", "back", false));
        }
    } else {
        if depth < 2 {
            out.push(hint("⏎", "drill", false));
        }
        out.push(hint("Esc", "back", false));
    }
    out
}

/// Extra chips. Appended after core hints so they truncate first.
pub fn extra_hint_segments() -> Vec<HintSegment> {
    vec![hint("Tab", "other pane", false), hint("q", "quit", false)]
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
pub fn action_hint_segments(state: &AppState) -> Vec<HintSegment> {
    let kind = hint_row_kind(state);
    let depth = nav_depth(state);
    let focus = state.focus;
    let hide_tree_writes = depth >= 1 || focus == FocusPane::Right;
    HINT_ACTIONS
        .iter()
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
pub fn nav_depth(state: &AppState) -> u8 {
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
        return HintRowKind::File;
    }
    match state.focused_row().map(|row| row.kind) {
        Some(NodeKind::Workspace) => HintRowKind::Workspace,
        Some(NodeKind::Repo) => HintRowKind::Repo,
        Some(NodeKind::Checkout) => HintRowKind::Checkout,
        Some(NodeKind::Group) => HintRowKind::Group,
        Some(NodeKind::Dir) => HintRowKind::Dir,
        Some(NodeKind::File) => HintRowKind::File,
        None => HintRowKind::Workspace,
    }
}

/// Display segments for the breadcrumb (workspace + drill frames).
pub fn breadcrumb_segments(state: &AppState) -> Vec<String> {
    let mut out = vec![workspace_label(state)];
    let mut seen_repo: Option<String> = None;
    let mut seen_commit: Option<String> = None;

    if nav_depth(state) == 0 {
        if !state.right_is_diff() {
            if let Some(repo) = focused_repo_basename(state) {
                out.push(repo);
            }
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
    Some(match source {
        CommitFileSource::Worktree => "uncommitted".into(),
        CommitFileSource::Stash { stash_ref } => stash_ref.clone(),
        CommitFileSource::Commit { commit_id } => {
            if commit_id.len() > 7 {
                commit_id[..7].to_string()
            } else {
                commit_id.clone()
            }
        }
    })
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

fn status_uses_status_text(state: &AppState) -> bool {
    state.search_mode
        || state.easy_motion.is_some()
        || state.stash_menu.is_some()
        || state.branch_picker.is_some()
        || state.create_branch.is_some()
}

fn breadcrumb_op_status(state: &AppState) -> String {
    if status_uses_status_text(state) || is_ctrl_c_exit_prompt(&state.status) {
        return String::new();
    }
    state.status.trim().to_string()
}

fn is_op_status_error(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("failed") || lower.contains("error")
}

/// Breadcrumb row: path on the left, optional toast / running-op status
/// (`Pulling 1/2…`) on the right.
pub fn breadcrumb_line(state: &AppState, width: u16) -> Line<'static> {
    let palette = state.theme.palette();
    let width = width as usize;
    let op = breadcrumb_op_status(state);
    let (crumb_max, op_max) = allocate_chrome_row(width, visible_width(&op));
    let segments = breadcrumb_segments(state);
    let focus = state.focus;
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
        let op_color = if is_op_status_error(&op) {
            palette.deleted
        } else {
            palette.muted
        };
        spans.push(Span::styled(
            truncate_visible(&op, op_max),
            Style::default().fg(op_color),
        ));
    }
    Line::from(spans)
}

/// Status row: mode pills + contextual hint chips, or a replacing prompt.
pub fn status_line(state: &AppState, width: u16) -> Line<'static> {
    let palette = state.theme.palette();
    let pills = state.theme.pills();
    let surface = hex_color(state.theme.theme().surface);
    if state.stash_menu.is_some() || state.branch_picker.is_some() || state.create_branch.is_some()
    {
        return Line::from(Span::styled(
            truncate_visible(&state.status, width as usize),
            Style::default().fg(palette.muted),
        ));
    }
    if state.search_mode {
        return search_typing_line(state, palette, pills.filter);
    }
    if let Some(motion) = state.easy_motion.as_ref() {
        return easy_motion_line(motion.typed.as_str(), palette, pills.filter);
    }
    idle_status_line(state, palette, pills, surface, width)
}

fn search_typing_line(state: &AppState, palette: Palette, filter: Pill) -> Line<'static> {
    let query = state.search_query.clone();
    Line::from(vec![
        pill_span("SEARCH", filter),
        Span::styled(format!(" {query}"), Style::default().fg(palette.repo)),
        Span::styled("▏", Style::default().fg(palette.cursor)),
        Span::styled(
            format!("   {SEARCH_TYPING_HINT}"),
            Style::default().fg(palette.muted),
        ),
    ])
}

fn easy_motion_line(typed: &str, palette: Palette, filter: Pill) -> Line<'static> {
    let shown = if typed.is_empty() { "…" } else { typed };
    Line::from(vec![
        pill_span("EASY", filter),
        Span::styled(format!(" {shown}"), Style::default().fg(palette.repo)),
        Span::styled(
            format!("   {EASY_MOTION_HINT}"),
            Style::default().fg(palette.muted),
        ),
    ])
}

fn idle_status_line(
    state: &AppState,
    palette: Palette,
    pills: Pills,
    surface: Color,
    width: u16,
) -> Line<'static> {
    let tree_mode = if nav_depth(state) >= 1 {
        state.commit_tree_mode
    } else {
        state.tree_mode
    };
    let mode_label = if tree_mode { "tree" } else { "flat" };
    let diff_label = match state.diff_mode {
        DiffMode::Inline => "inline",
        DiffMode::SideBySide => "split",
    };
    let search_query = if state.search_active {
        state.search_query.trim().to_string()
    } else {
        String::new()
    };
    let message = if z_pending(state) { "z…" } else { "? help" };
    let mut used =
        mode_label.len() + 2 + diff_label.len() + 2 + 1 + message.len() + HINT_SEPARATOR.len();
    if !search_query.is_empty() {
        used += search_query.len() + 3;
    }
    let mut hints = nav_chrome_hint_segments(nav_depth(state), state.focus);
    hints.extend(action_hint_segments(state));
    hints.extend(extra_hint_segments());
    let fitted = fit_hint_segments(&hints, (width as usize).saturating_sub(used));

    let mut spans = vec![
        pill_span(mode_label, pills.mode),
        pill_span(diff_label, pills.diff),
    ];
    if !search_query.is_empty() {
        spans.push(pill_span(&format!("/{search_query}"), pills.filter));
    }
    spans.push(Span::styled(
        format!(" {message}"),
        Style::default().fg(palette.file),
    ));
    if !fitted.is_empty() {
        spans.push(Span::raw(HINT_SEPARATOR));
        for (i, segment) in fitted.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw(HINT_SEPARATOR));
            }
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
    Line::from(spans)
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
    use crate::tui::state::AppState;
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

    #[test]
    fn fit_appends_ellipsis_instead_of_dropping_core_hints_for_extras() {
        let core = vec![
            hint_of("⏎", "focus right"),
            hint_of("s", "stage"),
            hint_of("x", "revert"),
        ];
        let mut all = core.clone();
        all.extend(extra_hint_segments());
        let fitted = fit_hint_segments(&all, hints_width(&core) + 4);
        let keys: Vec<&str> = fitted.iter().map(|s| s.key.as_str()).collect();
        assert!(keys.contains(&"⏎"), "{keys:?}");
        assert!(keys.contains(&"s"), "{keys:?}");
        assert!(keys.contains(&HINT_ELLIPSIS), "{keys:?}");
        assert!(
            !keys.contains(&"q"),
            "extras must truncate before core hints"
        );
    }

    #[test]
    fn nav_chrome_pills_and_hints() {
        assert_eq!(
            nav_chrome_hint_segments(0, FocusPane::Left),
            vec![hint_of("⏎", "focus right")]
        );
        assert_eq!(
            nav_chrome_hint_segments(0, FocusPane::Right),
            vec![hint_of("⏎", "drill"), hint_of("Esc", "back")]
        );
        assert_eq!(
            nav_chrome_hint_segments(2, FocusPane::Right),
            vec![hint_of("Esc", "back")]
        );
        assert_eq!(
            nav_chrome_hint_segments(1, FocusPane::Left),
            vec![hint_of("⏎", "focus right"), hint_of("Esc", "back")]
        );
    }

    #[test]
    fn format_hint_plain_joins_with_chip_gap() {
        assert_eq!(HINT_CHIP_GAP, 2);
        assert_eq!(format_hint_plain(&hint_of("s", "stage")), "s  stage");
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
        assert_eq!(extras, vec!["Tab".to_string(), "q".to_string()]);
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
    fn search_typing_hint_does_not_advertise_n_as_next() {
        assert!(SEARCH_TYPING_HINT.contains("Enter arms query"));
        assert!(SEARCH_TYPING_HINT.contains("n/N after Enter"));
        assert!(!SEARCH_TYPING_HINT.contains("n/N next/prev"));
    }

    #[test]
    fn bottom_chrome_hides_breadcrumb_during_help() {
        let idle = state();
        assert_eq!(breadcrumb_rows(&idle), 1);
        assert_eq!(overlay_status_rows(&idle), 1);
        assert_eq!(bottom_chrome_rows(&idle), 2);
        let mut help = state();
        help.help_open = true;
        assert_eq!(breadcrumb_rows(&help), 0);
        assert!(overlay_status_rows(&help) > 1);
        assert_eq!(bottom_chrome_rows(&help), overlay_status_rows(&help));
    }

    #[test]
    fn ctrl_c_prompt_is_pinned_not_a_breadcrumb_toast() {
        let mut app = state();
        app.status = CTRL_C_EXIT_PROMPT.into();
        let crumb = line_plain(&breadcrumb_line(&app, 80));
        assert!(
            !crumb.contains("Ctrl+C"),
            "quit prompt must not sit in the breadcrumb toast: {crumb:?}"
        );
        assert_eq!(ctrl_c_prompt_rows(&app), 1);
        assert_eq!(bottom_chrome_rows(&app), 3);
        let prompt = line_plain(&ctrl_c_prompt_line(&app, 80));
        assert!(
            prompt.contains("Ctrl+C again"),
            "pinned row should show the quit prompt: {prompt:?}"
        );
        let status = line_plain(&status_line(&app, 80));
        assert!(
            !status.contains("Ctrl+C again"),
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
    fn confirm_overlay_uses_row_budget() {
        let mut app = state();
        app.confirm = Some(super::super::state::PendingConfirm::Revert {
            targets: Vec::new(),
            label: "README.md".into(),
        });
        assert_eq!(overlay_status_rows(&app), 7);
        assert_eq!(breadcrumb_rows(&app), 1);
        app.confirm = Some(super::super::state::PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        });
        assert_eq!(overlay_status_rows(&app), 5);
        app.confirm = Some(super::super::state::PendingConfirm::RemoveWorktree {
            primary: "app".into(),
            path: ".worktrees/topic".into(),
            force: false,
            branch: "topic".into(),
            merged_into_default: Some(true),
        });
        assert_eq!(overlay_status_rows(&app), 6);
        app.confirm = Some(super::super::state::PendingConfirm::CheckoutOutOfSync {
            repo: "app".into(),
            branch: "main".into(),
            remote_ref: "origin/main".into(),
        });
        assert_eq!(overlay_status_rows(&app), 7);
        app.confirm = Some(super::super::state::PendingConfirm::MergeIntoHead {
            repo: "app".into(),
            rev: "topic".into(),
            label: "topic".into(),
            into: "main".into(),
        });
        assert_eq!(overlay_status_rows(&app), 7);
    }

    fn line_plain(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn running_op_progress_sits_on_breadcrumb_not_status_hints() {
        use super::super::ops::{format_running_op, RunningOp};
        let mut app = state();
        app.status = format_running_op(RunningOp::Pull, 1, 2);
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
        app.status = format_running_op(RunningOp::Fetch, 2, 18);
        let crumb = line_plain(&breadcrumb_line(&app, 20));
        assert!(
            crumb.contains("Fetching") || crumb.contains("2/18"),
            "narrow row still keeps op status: {crumb:?}"
        );
    }
}

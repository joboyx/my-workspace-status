//! Icon popovers: a hover peek, a pinned popover, and their content.
//!
//! Paint records one [`super::state::IconHit`] per painted icon: a tagged
//! tree segment; a graph node, row glyph, chip, or comment mark; a PR
//! badge. The pointer resting on a hit for [`PEEK_DWELL_MS`] opens a peek:
//! paint only, never an input mode, so it never takes a key. The pointer
//! leaving both the icon and the peek starts a [`PEEK_GRACE_MS`] grace,
//! then the peek closes.
//!
//! A click on an icon, a click inside a peek, or `gh` pins a popover. A
//! pinned popover is [`super::keys::InputMode::Popover`]: `j` / `k` move
//! between its field and action lines, Enter or a click runs the focused
//! action, `y` copies the focused line, `r` refreshes the PR of a PR
//! section, and a drag selects its text. Esc, a click outside it, or a
//! change of the focused row closes it.
//!
//! [`PopoverState`] keeps only what to describe (icon kind and target per
//! section). Every frame rebuilds the sections from live state
//! ([`popover_sections`]), so a watch tick under a pinned popover shows new
//! counts and a vanished icon drops its section. Builders are pure
//! functions of the [`AppState`] and one target, so a new icon kind adds one
//! builder arm here and nothing in the core.

use std::cell::OnceCell;
use std::path::{Path, PathBuf};
use std::time::Instant;

use ratatui::layout::Rect;
use workspace_status_graph::{
    format_local_timestamp, format_relative_date, short_id, Commit, GraphIconKind, GraphRef,
    GraphRow, PartTarget, RefKind, Stash, SyncStatus, Worktree, ASCII, UNICODE,
};

use crate::git::UpstreamRewrite;
use crate::helpers::{is_default_branch, is_detached_head_branch, DEFAULT_BRANCH_NAMES};
use crate::snapshot::{CheckoutKind, FileChange, WorkspaceRepoSnapshot};

use super::action::Action;
use super::command_palette::{command_for, PaletteCommand};
use super::comments::{checkout_file_comments, tree_row_comments, CommentEntry};
use super::commit_files::CommitFileRow;
use super::gates::ListFocusTarget;
use super::icons::{
    capture_count, file_type_name, spec, status_letter_from_change, upstream_rewrite_text, IconKind,
};
use super::line_blame::relative_age;
use super::pr_status::{CheckBucket, ChecksSummary, PrState};
use super::pull_request::{PrReview, PullRequestDetail, ReviewCount};
use super::state::{AppState, PrDetailState};
use super::tree::{
    file_change_from_name_status, find_node, is_merge_mark_kind, path_under_dir, pr_badge_kind,
    pr_badge_mark, NodeKind, NodeSegments, SegRole, TextSeg, TreeNode, VisibleRow,
};

/// Pointer rest on an icon before its peek opens, in milliseconds.
pub const PEEK_DWELL_MS: u64 = 400;

/// Time a peek stays after the pointer leaves the icon and the peek, in
/// milliseconds. Moving onto the peek inside it keeps the peek open.
pub const PEEK_GRACE_MS: u64 = 250;

/// Widest popover box, border included. A narrower frame caps it at the
/// frame width less 4 columns.
pub const POPOVER_MAX_WIDTH: u16 = 64;

/// Footer of a pinned popover.
pub const POPOVER_FOOTER: &str = "y copy line · Enter run · Esc close";

/// What one painted icon describes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum IconTarget {
    /// An icon on the workspace tree row with this
    /// [`VisibleRow::id`](super::tree::VisibleRow::id).
    TreeRow(String),
    /// The PR badge of this checkout (snapshot `repo`), on a tree row or a
    /// graph worktree row.
    PullRequest(PathBuf),
    /// An icon on the shown commit-file or compare file list row with this
    /// [`CommitFileRow::id`].
    CommitFileRow(String),
    /// An icon on the active Explorer tab's tree row with this
    /// checkout-relative path ([`super::explorer::ExplorerRow::rel`]).
    ExplorerRow(String),
    /// An icon on the graph row with this [`graph_row_id`]: a node, the
    /// uncommitted glyph, or the comment mark.
    GraphRow(String),
    /// The linked-worktree glyph of the graph worktree with this path, on
    /// its worktree row or as a worktree mark on a commit spacer.
    GraphWorktree(String),
    /// A ref chip, a checkout / sync mark run, `[HEAD]`, or `[+N]` painted
    /// for the graph row with [`graph_row_id`] `row`: on its commit spacer
    /// or in the selection footer.
    GraphChip {
        /// [`graph_row_id`] of the row the chip is painted for.
        row: String,
        /// The chip ([`workspace_status_graph::LabelPart::target`]).
        chip: PartTarget,
    },
    /// The graph sync header: HEAD's branch against its upstream.
    GraphSync,
    /// The selection footer's `more lines below` hint of the graph row with
    /// this [`graph_row_id`].
    GraphMoreLines(String),
    /// The depth-2 commit-files footer's `more lines below` hint of the
    /// commit or stash with this identity
    /// ([`AppState::commit_detail_msg_identity`]).
    CommitDetailMoreLines(String),
    /// The comment mark on a line gutter of the right pane's line viewer:
    /// a file diff (Workspace, compare, or Explorer changed file) or the
    /// Explorer clean-file view ([`AppState::line_viewer_target`]).
    LineComment {
        /// Checkout of the viewer's file.
        checkout: String,
        /// Repo-relative path of the viewer's file.
        path: String,
        /// 1-based line number the gutter paints the mark beside.
        line: u32,
    },
}

/// How a popover opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopoverOrigin {
    /// Hover dwell. Paint only; keys pass through.
    Peek,
    /// Click or `gh`. Owns the keys until it closes.
    Pinned,
}

/// The focus a popover opened on. A pinned popover closes when the focus
/// no longer matches (another row, another pane, another tab).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverOwner {
    /// Active tab index.
    pub tab: usize,
    /// Focused list.
    pub list: ListFocusTarget,
    /// Focused tree row id, commit-file row id, [`graph_row_id`], or, on
    /// a focused line viewer, its cursor (`diff:<row>` on a file diff,
    /// `file:<line>` on the clean-file view, both 0-based).
    pub row: String,
}

/// An open popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverState {
    /// Peek or pinned.
    pub origin: PopoverOrigin,
    /// One section per entry, in paint order: icon kind and target.
    pub targets: Vec<(IconKind, IconTarget)>,
    /// Screen cells of the icon the popover hangs from. `None` (from `gh`)
    /// hangs it from the first painted icon of [`Self::targets`], else
    /// from the focused row.
    pub anchor: Option<Rect>,
    /// Focused line, an index into [`flat_lines`] of the sections.
    pub focus_line: usize,
    /// Focus when the popover opened.
    pub owner: PopoverOwner,
}

impl PopoverState {
    /// True for a pinned popover.
    pub fn is_pinned(&self) -> bool {
        self.origin == PopoverOrigin::Pinned
    }
}

/// One line of a popover section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PopoverLine {
    /// Plain text (the catalog meaning). Not focusable.
    Text(String),
    /// Muted status text, such as `loading…`. Not focusable.
    Note(String),
    /// A muted label and a value. `y` copies the value.
    Field {
        /// Short lower-case label; empty for a line that continues the
        /// field above it.
        label: &'static str,
        /// Value text.
        value: String,
        /// Colour of the value: the role the pane paints the same thing
        /// in (a branch name, a sync count, a status side), else
        /// [`SegRole::File`].
        role: SegRole,
        /// True when the value is a URL the painted value text links to
        /// (OSC 8). The link keeps the full value when the text is cut.
        link: bool,
        /// Length in bytes of the head of the value that paints bold: 0
        /// for none, the value's length for all of it.
        bold: usize,
        /// A short mark painted before the value in its own colour (the
        /// `✗` / `…` of a PR check name), while the value keeps
        /// [`Self::Field::role`]. `y` copies the value alone.
        mark: Option<(&'static str, SegRole)>,
    },
    /// A horizontal rule between the fields and the actions. Not
    /// focusable.
    Rule,
    /// An existing command: a title and a key chip. Enter or a click runs
    /// its [`Action`].
    Action {
        /// The palette row: its action and gate.
        command: &'static PaletteCommand,
        /// Painted title: the palette title, or a section's own name for
        /// it (the PR section names Open PR CI by its CI page).
        title: &'static str,
        /// Key chip: the palette row's keys, or a key that runs the action
        /// only inside this popover (the PR section's `y` and `r`, whose
        /// palette rows show no chip). Empty paints no chip.
        keys: &'static str,
    },
}

impl PopoverLine {
    /// The palette row of `action` as a line. `None` when no row runs it.
    pub fn action(action: &Action) -> Option<Self> {
        command_for(action).map(|command| Self::Action {
            command,
            title: command.title,
            keys: command.keys,
        })
    }

    /// The palette row of `action` as a line painted with `title`. `None`
    /// when no row runs it.
    pub fn titled_action(action: &Action, title: &'static str) -> Option<Self> {
        command_for(action).map(|command| Self::Action {
            command,
            title,
            keys: command.keys,
        })
    }

    /// This line with the key chip `keys`, for a key that runs the action
    /// only inside the popover. A line that is not an action stays as it
    /// is.
    pub fn with_keys(self, keys: &'static str) -> Self {
        match self {
            Self::Action { command, title, .. } => Self::Action {
                command,
                title,
                keys,
            },
            line => line,
        }
    }

    /// True for a line `j` / `k` can focus: a field or an action.
    pub fn focusable(&self) -> bool {
        !matches!(self, Self::Text(_) | Self::Note(_) | Self::Rule)
    }

    /// Text `y` copies: the value of a field, the title of an action.
    pub fn copy_text(&self) -> String {
        match self {
            Self::Text(text) | Self::Note(text) => text.clone(),
            Self::Field { value, .. } => value.clone(),
            Self::Rule => String::new(),
            Self::Action { title, .. } => title.to_string(),
        }
    }
}

/// One icon's part of a popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverSection {
    /// Icon described. The heading paints its glyph and
    /// [`Self::heading`].
    pub icon: IconKind,
    /// Colour role of the glyph, as the row paints it.
    pub role: SegRole,
    /// What the icon is on.
    pub target: IconTarget,
    /// Heading text after the glyph; `None` paints the catalog name.
    pub title: Option<String>,
    /// The meaning first, then fields, then actions.
    pub lines: Vec<PopoverLine>,
}

impl PopoverSection {
    /// Heading text after the glyph: [`Self::title`], else the catalog
    /// name of [`Self::icon`].
    pub fn heading(&self) -> &str {
        self.title.as_deref().unwrap_or(self.icon.spec().name)
    }
}

/// Target of icon `kind` painted on tree row `row`.
///
/// A PR badge describes the checkout's PR; every other icon describes the
/// row.
pub fn tree_icon_target(kind: IconKind, row: &VisibleRow) -> IconTarget {
    match (kind, row.repo.as_deref()) {
        (kind, Some(repo)) if kind.is_pull_request() => {
            IconTarget::PullRequest(PathBuf::from(repo))
        }
        _ => IconTarget::TreeRow(row.id.clone()),
    }
}

/// Stable id of a graph row: it names the same row across a reload and a
/// cursor move. `uncommitted`, `commit:<id>`, `stash:<id>` (the stash
/// ref when git gave no id), or `worktree:<path>`.
pub fn graph_row_id(row: &GraphRow) -> String {
    match row {
        GraphRow::Uncommitted { .. } => "uncommitted".into(),
        GraphRow::Commit { commit, .. } => format!("commit:{}", commit.id),
        GraphRow::Stash(stash) if stash.id.is_empty() => format!("stash:{}", stash.stash_ref),
        GraphRow::Stash(stash) => format!("stash:{}", stash.id),
        GraphRow::Worktree(worktree) => format!("worktree:{}", worktree.path),
    }
}

/// Catalog kind and target of graph icon `kind` painted for `row`. `part`
/// is the [`workspace_status_graph::IconSpan::part`] and `chip` its
/// [`workspace_status_graph::IconSpan::target`]. `None` for a PR badge or the
/// header chip (their checkout comes from the badge list or the chip), the
/// sync header and the footer hint (no row icon), and a worktree mark whose
/// index is not on the row.
///
/// A checkout / sync mark run is one icon: [`IconKind::ChipCheckout`] when
/// HEAD is on its branch, else [`IconKind::ChipSynced`].
/// [`icon_targets`] gives its two sections.
pub fn graph_icon(
    kind: GraphIconKind,
    part: Option<usize>,
    chip: Option<&PartTarget>,
    row: &GraphRow,
) -> Option<(IconKind, IconTarget)> {
    let on_row = |icon: IconKind| Some((icon, IconTarget::GraphRow(graph_row_id(row))));
    let on_chip = |icon: IconKind| {
        Some((
            icon,
            IconTarget::GraphChip {
                row: graph_row_id(row),
                chip: chip?.clone(),
            },
        ))
    };
    match kind {
        GraphIconKind::CommitNode => on_row(IconKind::GraphCommit),
        GraphIconKind::HeadNode => on_row(IconKind::GraphHeadCommit),
        GraphIconKind::StashNode => on_row(IconKind::GraphStash),
        GraphIconKind::Uncommitted => on_row(IconKind::GraphUncommitted),
        GraphIconKind::Comment => on_row(IconKind::Comment),
        GraphIconKind::ResolvedComment => on_row(IconKind::CommentResolved),
        GraphIconKind::Worktree => {
            let path = match row {
                GraphRow::Worktree(worktree) => &worktree.path,
                GraphRow::Commit { worktrees, .. } => &worktrees.get(part?)?.path,
                GraphRow::Uncommitted { .. } | GraphRow::Stash(_) => return None,
            };
            Some((
                IconKind::LinkedWorktree,
                IconTarget::GraphWorktree(path.clone()),
            ))
        }
        GraphIconKind::LocalChip => on_chip(IconKind::ChipLocal),
        GraphIconKind::DefaultChip => on_chip(IconKind::ChipDefault),
        GraphIconKind::RemoteChip => on_chip(IconKind::ChipRemote),
        GraphIconKind::TagChip => on_chip(IconKind::ChipTag),
        GraphIconKind::DetachedHeadChip => on_chip(IconKind::ChipDetachedHead),
        GraphIconKind::OverflowChip => on_chip(IconKind::ChipOverflow),
        GraphIconKind::ChipMarks => match chip? {
            PartTarget::Marks { checkout: true, .. } => on_chip(IconKind::ChipCheckout),
            _ => on_chip(IconKind::ChipSynced),
        },
        GraphIconKind::Badge
        | GraphIconKind::SyncHeader
        | GraphIconKind::HeaderChip
        | GraphIconKind::MoreBelow => None,
    }
}

/// The popover sections one painted icon opens: itself, and for a
/// checkout mark run that also holds the sync mark, the sync section after
/// the checkout section (one hit, two sections).
pub fn icon_targets(kind: IconKind, target: &IconTarget) -> Vec<(IconKind, IconTarget)> {
    let mut targets = vec![(kind, target.clone())];
    if let IconTarget::GraphChip {
        chip:
            PartTarget::Marks {
                checkout: true,
                remote: Some(_),
                ..
            },
        ..
    } = target
    {
        targets.push((IconKind::ChipSynced, target.clone()));
    }
    targets
}

/// Catalog kind of the graph sync header in `status`. Up to date reads as
/// [`IconKind::Synced`]: the header paints no mark then.
pub fn graph_sync_kind(status: SyncStatus) -> IconKind {
    match status {
        SyncStatus::Ahead => IconKind::Ahead,
        SyncStatus::Behind => IconKind::Behind,
        SyncStatus::Diverged => IconKind::Diverged,
        SyncStatus::NoUpstream => IconKind::NoUpstream,
        SyncStatus::UpToDate => IconKind::Synced,
    }
}

/// Sections of `targets` from live state, in order. A target whose icon is
/// gone (row removed, PR answer dropped, sync mark cleared) has none.
///
/// `now_unix` is the clock for relative ages (graph commit and stash
/// dates), the same clock the graph paint reads.
pub fn popover_sections(
    state: &AppState,
    targets: &[(IconKind, IconTarget)],
    now_unix: i64,
) -> Vec<PopoverSection> {
    let rows = SectionRows::new(state);
    targets
        .iter()
        .filter_map(|(kind, target)| section(state, &rows, *kind, target, now_unix))
        .collect()
}

/// The commit-file rows and graph rows the sections of one popover read.
/// Each list is built at most once per [`popover_sections`] call, however
/// many sections read it.
struct SectionRows<'a> {
    state: &'a AppState,
    commit_files: OnceCell<Vec<CommitFileRow>>,
    graph: OnceCell<Vec<GraphRow>>,
}

impl<'a> SectionRows<'a> {
    fn new(state: &'a AppState) -> Self {
        Self {
            state,
            commit_files: OnceCell::new(),
            graph: OnceCell::new(),
        }
    }

    /// The shown commit-file or compare list rows.
    fn commit_files(&self) -> &[CommitFileRow] {
        self.commit_files
            .get_or_init(|| self.state.commit_file_rows())
    }

    /// The visible graph rows; empty with no graph.
    fn graph(&self) -> &[GraphRow] {
        self.graph.get_or_init(|| {
            self.state
                .graph
                .as_ref()
                .map(|model| model.visible_rows())
                .unwrap_or_default()
        })
    }

    /// The graph row with [`graph_row_id`] `id`.
    fn graph_row(&self, id: &str) -> Option<&GraphRow> {
        self.graph().iter().find(|row| graph_row_id(row) == id)
    }
}

fn section(
    state: &AppState,
    rows: &SectionRows<'_>,
    kind: IconKind,
    target: &IconTarget,
    now_unix: i64,
) -> Option<PopoverSection> {
    match target {
        IconTarget::TreeRow(id) => {
            let row = state.rows.iter().find(|row| &row.id == id)?;
            tree_section(state, kind, row)
        }
        IconTarget::PullRequest(repo) => pr_section(state, repo, now_unix),
        IconTarget::CommitFileRow(id) => {
            // The folder summary paints files the list may fold away.
            let summary = || {
                let summary = state.folder_summary()?;
                state
                    .summary_rows(&summary)
                    .into_iter()
                    .find(|row| &row.id == id)
            };
            match rows.commit_files().iter().find(|row| &row.id == id) {
                Some(row) => commit_file_section(state, kind, row),
                None => commit_file_section(state, kind, &summary()?),
            }
        }
        IconTarget::ExplorerRow(rel) => explorer_row_section(state, kind, rel),
        IconTarget::GraphRow(id) => graph_row_section(state, rows, kind, id, now_unix),
        IconTarget::GraphWorktree(path) => graph_worktree_section(state, rows, path),
        IconTarget::GraphChip { row, chip } => graph_chip_section(state, rows, kind, row, chip),
        IconTarget::GraphSync => graph_sync_section(state),
        IconTarget::GraphMoreLines(row) => graph_more_lines_section(state, rows, row),
        IconTarget::CommitDetailMoreLines(id) => commit_detail_more_lines_section(state, id),
        IconTarget::LineComment {
            checkout,
            path,
            line,
        } => line_comment_section(target, &state.line_comments(checkout, path, *line)),
    }
}

/// Section of the comment mark on a line gutter: the comment fields over
/// `comments`, the comments behind that line's mark, as a row comment
/// mark lists its file's. `None` once the line has no comment (its mark
/// went away). Open and resolved are one slot, as on a graph row.
fn line_comment_section(target: &IconTarget, comments: &[&CommentEntry]) -> Option<PopoverSection> {
    if comments.is_empty() {
        return None;
    }
    let (kind, role) = if comments.iter().all(|entry| entry.resolved) {
        (IconKind::CommentResolved, SegRole::Muted)
    } else {
        (IconKind::Comment, SegRole::Heading)
    };
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    let actions = comment_fields(comments, &mut lines);
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role,
        target: target.clone(),
        title: None,
        lines,
    })
}

/// The segment of `segs` (every segment a row paints) that paints `kind`
/// now.
///
/// One slot stands for every sync kind, both merge kinds, every status
/// letter, and both comment marks, so a tick that turns behind into
/// ahead, open into merged, `M` into `S`, or open comments into resolved
/// keeps the section and shows the new mark.
fn painted_seg(segs: &NodeSegments, kind: IconKind) -> Option<TextSeg> {
    let slots: [fn(IconKind) -> bool; 4] = [
        is_sync_kind,
        is_merge_mark_kind,
        is_status_kind,
        is_comment_kind,
    ];
    let same_slot =
        |other: IconKind| other == kind || slots.iter().any(|slot| slot(kind) && slot(other));
    segs.segments
        .iter()
        .chain(&segs.trailing)
        .find(|seg| seg.icon.is_some_and(same_slot))
        .cloned()
}

/// Section of icon `kind` on tree row `row`: the catalog meaning, the
/// fields for that icon, then its actions. `None` when the row no longer
/// paints the icon.
fn tree_section(state: &AppState, kind: IconKind, row: &VisibleRow) -> Option<PopoverSection> {
    let seg = painted_seg(&state.tree_row_paint_segments(row), kind)?;
    let kind = seg.icon?;
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    let actions: Vec<Action> = match kind {
        IconKind::Workspace => {
            workspace_fields(state, row, &mut lines);
            vec![Action::Fetch, Action::Refresh, Action::ToggleShowHidden]
        }
        IconKind::Repo => {
            repo_fields(state, row, &mut lines);
            vec![
                Action::Fetch,
                Action::Pull,
                Action::Push,
                Action::Branch,
                Action::ViewRemotely,
            ]
        }
        IconKind::LinkedWorktree => {
            worktree_fields(
                &WorktreeFacts {
                    path: &row.chrome.path,
                    primary: row.primary_repo.as_deref(),
                    branch: Some(&row.chrome.branch),
                    default_override: row.chrome.default_branch_override.as_deref(),
                    current: false,
                    hidden: row.hidden,
                },
                &mut lines,
            );
            vec![
                Action::RemoveWorktree,
                Action::CompareVsDefault,
                Action::CopyEntityReference,
                Action::ViewRemotely,
            ]
        }
        IconKind::Branch => {
            branch_fields(state, row, &mut lines);
            vec![
                Action::Branch,
                Action::DefaultBranch,
                Action::GraphFocusBranches,
                Action::CompareVsDefault,
                Action::ViewRemotely,
            ]
        }
        IconKind::MergedIntoDefault | IconKind::OpenVsDefault => {
            lines.push(role_field(
                "vs",
                default_ref(state, row),
                SegRole::BranchDefault,
            ));
            let mut actions = vec![Action::CompareVsDefault, Action::DefaultBranch];
            let linked = row.chrome.checkout_kind == Some(CheckoutKind::Linked);
            if kind == IconKind::MergedIntoDefault && linked {
                actions.push(Action::RemoveWorktree);
            }
            actions
        }
        IconKind::Clean if row.kind == NodeKind::Group => {
            group_fields(state, row, &mut lines);
            vec![Action::FoldToggle, Action::FoldToggleSubtree]
        }
        kind if is_sync_kind(kind) => sync_fields(
            kind,
            &row.chrome.sync_note,
            row.chrome.upstream_rewrite.as_ref(),
            &mut lines,
        ),
        IconKind::StatusFailed => vec![Action::Refresh],
        IconKind::Hidden => vec![Action::ToggleShowHidden],
        IconKind::ChangeCount => {
            let counts = change_counts(state, &row_checkouts(state, row));
            change_count_fields(counts, &mut lines);
            vec![Action::StashMenu, Action::CompareVsDefault]
        }
        IconKind::WorktreeCount => {
            for checkout in family_checkouts(state, row) {
                let label = if checkout.chrome.checkout_kind == Some(CheckoutKind::Linked) {
                    "linked"
                } else {
                    "primary"
                };
                lines.push(field(label, checkout.label.clone()));
            }
            vec![Action::FoldToggle, Action::FoldToggleSubtree]
        }
        kind if is_comment_kind(kind) => {
            let comments = tree_row_comments(&state.comment_store, &state.snapshot, row);
            comment_fields(&comments, &mut lines)
        }
        kind => file_fields(kind, &tree_file_facts(state, row), &mut lines)?,
    };
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role: seg.role,
        target: IconTarget::TreeRow(row.id.clone()),
        title: None,
        lines,
    })
}

/// Section of icon `kind` on commit-file or compare list row `row`. Same
/// builders as a workspace tree file row; `None` when the row no longer
/// paints the icon.
fn commit_file_section(
    state: &AppState,
    kind: IconKind,
    row: &CommitFileRow,
) -> Option<PopoverSection> {
    let seg = painted_seg(&state.commit_file_row_paint_segments(row), kind)?;
    let kind = seg.icon?;
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    let actions = if is_comment_kind(kind) {
        comment_fields(&state.commit_file_row_comments(row), &mut lines)
    } else {
        file_fields(kind, &commit_file_facts(state, row), &mut lines)?
    };
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role: seg.role,
        target: IconTarget::CommitFileRow(row.id.clone()),
        title: None,
        lines,
    })
}

/// Section of icon `kind` on the active Explorer tree's row at `rel`. Same
/// builders as a workspace tree file or folder row: the comment fields over
/// the comments behind the row's mark, else the file-level fields with the
/// checkout's own change as both the row change and its sides. `None` when
/// the row is gone or no longer paints the icon.
fn explorer_row_section(state: &AppState, kind: IconKind, rel: &str) -> Option<PopoverSection> {
    let (checkout, row, status) = state.explorer_row_at(rel)?;
    let marks = state.explorer_row_marks(&checkout);
    let seg = painted_seg(
        &state.explorer_row_paint_segments(&row, &status, &marks),
        kind,
    )?;
    let kind = seg.icon?;
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    let actions = if is_comment_kind(kind) {
        let comments =
            checkout_file_comments(&state.comment_store, &state.snapshot, &checkout, rel);
        comment_fields(&comments, &mut lines)
    } else {
        let change = status.change(rel).cloned();
        let facts = FileFacts {
            path: rel.to_string(),
            sides: change.clone(),
            change,
            files: status.changed_files_under(rel),
        };
        file_fields(kind, &facts, &mut lines)?
    };
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role: seg.role,
        target: IconTarget::ExplorerRow(rel.to_string()),
        title: None,
        lines,
    })
}

/// What a file-level popover reads from its row, from the workspace tree
/// or a commit-file list.
struct FileFacts {
    /// Folder or file path (a section: the checkout path).
    path: String,
    /// The file row's change.
    change: Option<FileChange>,
    /// The checkout's own change for this path: index and worktree sides.
    /// `None` on a commit-file list, whose rows have one name-status
    /// letter.
    sides: Option<FileChange>,
    /// Changed files under a section or folder row.
    files: usize,
}

/// Files under tree node `id`, at any depth.
fn tree_file_count(state: &AppState, id: &str) -> usize {
    fn count(node: &TreeNode) -> usize {
        usize::from(node.kind == NodeKind::File) + node.children.iter().map(count).sum::<usize>()
    }
    find_node(&state.tree, id).map_or(0, count)
}

fn tree_file_facts(state: &AppState, row: &VisibleRow) -> FileFacts {
    let sides = row.file.as_ref().and_then(|file| {
        row_repo(state, row)?
            .changes
            .iter()
            .find(|change| change.path == file.path)
            .cloned()
    });
    FileFacts {
        path: row.chrome.path.clone(),
        change: row.file.clone(),
        sides,
        files: tree_file_count(state, &row.id),
    }
}

fn commit_file_facts(state: &AppState, row: &CommitFileRow) -> FileFacts {
    let change = row.file.as_ref().map(|file| {
        file_change_from_name_status(&file.status, file.path.clone(), file.old_path.clone())
    });
    let files = state.commit_drill_files().map_or(0, |files| {
        files
            .iter()
            .filter(|file| row.is_dir() && path_under_dir(&file.path, &row.path))
            .count()
    });
    FileFacts {
        path: row.path.clone(),
        change,
        sides: None,
        files,
    }
}

/// Fields of a file-level icon (section, folder, file type, status
/// letter, viewed eye) and its actions. `None` for any other kind.
///
/// Stage, Unstage, and Revert stay listed on a commit-file list: the gates
/// refuse them there, and the popover shows the line dim with the reason.
fn file_fields(
    kind: IconKind,
    facts: &FileFacts,
    lines: &mut Vec<PopoverLine>,
) -> Option<Vec<Action>> {
    let files = || field("files", counted(facts.files, "file", "files"));
    Some(match kind {
        IconKind::Staged => {
            lines.push(files());
            vec![Action::Unstage, Action::StashMenu]
        }
        IconKind::Changes => {
            lines.push(files());
            vec![Action::Stage, Action::StashMenu]
        }
        IconKind::Folder => {
            lines.extend([field("path", facts.path.clone()), files()]);
            vec![
                Action::Stage,
                Action::Unstage,
                Action::Revert,
                Action::CopyEntityReference,
            ]
        }
        IconKind::FileType => {
            lines.extend([
                field("type", format!("{} file", file_type_name(&facts.path))),
                field("path", facts.path.clone()),
            ]);
            vec![
                Action::Edit,
                Action::ExternalDiff,
                Action::CopyEntityReference,
                Action::ViewRemotely,
            ]
        }
        kind if is_status_kind(kind) => {
            let change = facts.change.as_ref()?;
            let path = match change.old_path.as_deref() {
                Some(old) => format!("{old} → {}", change.path),
                None => change.path.clone(),
            };
            lines.push(field("path", path));
            let mut actions = Vec::new();
            match facts.sides.as_ref() {
                Some(sides) => {
                    let (index, index_role) = side_text(sides.staged_status.as_deref(), false);
                    let (worktree, worktree_role) =
                        side_text(sides.unstaged_status.as_deref(), sides.untracked);
                    lines.extend([
                        role_field("index", index, index_role),
                        role_field("worktree", worktree, worktree_role),
                    ]);
                    if change.unstaged_status.is_some() || change.untracked {
                        actions.push(Action::Stage);
                    }
                    if change.staged_status.is_some() {
                        actions.push(Action::Unstage);
                    }
                }
                None => {
                    let letter = status_letter_from_change(change);
                    lines.push(role_field(
                        "change",
                        spec(letter.icon_kind()).name,
                        SegRole::from(letter.color_role()),
                    ));
                    actions.extend([Action::Stage, Action::Unstage]);
                }
            }
            actions.extend([Action::Revert, Action::ToggleReviewed]);
            actions
        }
        IconKind::Viewed => vec![Action::ToggleReviewed],
        _ => return None,
    })
}

/// One side of a workspace change in words (`modified`, `untracked`, or
/// `no change`) and the colour its status letter paints in.
fn side_text(status: Option<&str>, untracked: bool) -> (String, SegRole) {
    if untracked {
        return ("untracked".into(), SegRole::Added);
    }
    let (words, role) = match status {
        None => ("no change", SegRole::Muted),
        Some("A") => ("added", SegRole::Added),
        Some("M") => ("modified", SegRole::Modified),
        Some("D") => ("deleted", SegRole::Deleted),
        Some("R") => ("renamed", SegRole::Renamed),
        Some("C") => ("copied", SegRole::Renamed),
        Some("U") => ("conflict", SegRole::Deleted),
        Some("T") => ("type changed", SegRole::Modified),
        Some(other) => return (other.to_string(), SegRole::File),
    };
    (words.into(), role)
}

/// Most comment bodies a comment popover lists.
const COMMENT_BODIES_MAX: usize = 3;

/// Comment counts and the first line of the first
/// [`COMMENT_BODIES_MAX`] bodies; actions Comment and Copy comments.
fn comment_fields(comments: &[&CommentEntry], lines: &mut Vec<PopoverLine>) -> Vec<Action> {
    let resolved = comments.iter().filter(|entry| entry.resolved).count();
    lines.push(field(
        "comments",
        format!("{} open · {resolved} resolved", comments.len() - resolved),
    ));
    for entry in comments.iter().take(COMMENT_BODIES_MAX) {
        let first = entry.body.lines().next().unwrap_or_default().trim();
        let (label, role) = if entry.resolved {
            ("resolved", SegRole::Muted)
        } else {
            ("open", SegRole::File)
        };
        lines.push(role_field(label, first, role));
    }
    vec![Action::CommentStart, Action::ExportComments]
}

fn field(label: &'static str, value: impl Into<String>) -> PopoverLine {
    role_field(label, value, SegRole::File)
}

/// A field whose value paints in `role`.
fn role_field(label: &'static str, value: impl Into<String>, role: SegRole) -> PopoverLine {
    PopoverLine::Field {
        label,
        value: value.into(),
        role,
        link: false,
        bold: 0,
        mark: None,
    }
}

/// A field whose value is a URL: the painted value links to it.
fn link_field(label: &'static str, url: impl Into<String>) -> PopoverLine {
    PopoverLine::Field {
        label,
        value: url.into(),
        role: SegRole::File,
        link: true,
        bold: 0,
        mark: None,
    }
}

/// A field whose value paints in `role`, its first `bold` bytes bold.
fn bold_field(
    label: &'static str,
    value: impl Into<String>,
    role: SegRole,
    bold: usize,
) -> PopoverLine {
    PopoverLine::Field {
        label,
        value: value.into(),
        role,
        link: false,
        bold,
        mark: None,
    }
}

/// Colour role of branch `name`: the default-branch colour for the
/// default branch, else the feature-branch colour. Detached is muted.
fn branch_role(name: &str, default_override: Option<&str>) -> SegRole {
    if is_detached_head_branch(name) {
        SegRole::Muted
    } else if is_default_branch(name, default_override) {
        SegRole::BranchDefault
    } else {
        SegRole::BranchFeature
    }
}

/// `n <one>` or `n <many>`.
fn counted(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// Workspace root: name, change total, sync summary, repo count.
fn workspace_fields(state: &AppState, row: &VisibleRow, lines: &mut Vec<PopoverLine>) {
    lines.extend([
        field("name", state.tree.label.clone()),
        field("changes", format!("{} changed", row.chrome.change_count)),
        field("sync", row.chrome.sync_summary.clone()),
        field("repos", counted(repo_count(&state.tree), "repo", "repos")),
    ]);
}

/// Repo rows in the tree: a family counts once.
fn repo_count(node: &TreeNode) -> usize {
    usize::from(node.kind == NodeKind::Repo)
        + node
            .children
            .iter()
            .filter(|child| matches!(child.kind, NodeKind::Repo | NodeKind::Group))
            .map(repo_count)
            .sum::<usize>()
}

/// Repo row: path, branch (a single checkout), kind, change breakdown.
fn repo_fields(state: &AppState, row: &VisibleRow, lines: &mut Vec<PopoverLine>) {
    lines.push(field("path", row.chrome.path.clone()));
    if row.chrome.is_family {
        let checkouts = family_checkouts(state, row).len();
        lines.push(field(
            "kind",
            format!("family of {}", counted(checkouts, "checkout", "checkouts")),
        ));
    } else {
        let branch = row.chrome.branch.as_str();
        let role = branch_role(branch, row.chrome.default_branch_override.as_deref());
        lines.push(role_field("branch", branch, role));
        lines.push(field("kind", "primary checkout"));
    }
    let (staged, unstaged, untracked) = change_counts(state, &row_checkouts(state, row));
    lines.push(field(
        "changes",
        format!("{staged} staged · {unstaged} unstaged · {untracked} untracked"),
    ));
}

/// What a linked-worktree popover shows, from a tree row or the graph.
struct WorktreeFacts<'a> {
    /// Checkout path.
    path: &'a str,
    /// Primary checkout of the repository, when known.
    primary: Option<&'a str>,
    /// Checked-out branch; `None` or a detached name is detached.
    branch: Option<&'a str>,
    /// Configured default branch name, when set.
    default_override: Option<&'a str>,
    /// The graph's current checkout.
    current: bool,
    /// Listed as hidden in config.
    hidden: bool,
}

/// Linked worktree: path, primary checkout, branch or detached, then
/// `current` and `hidden` when they hold. The tree and the graph share
/// it.
fn worktree_fields(facts: &WorktreeFacts<'_>, lines: &mut Vec<PopoverLine>) {
    lines.push(field("path", facts.path));
    if let Some(primary) = facts.primary {
        lines.push(field("primary", primary));
    }
    let branch = facts.branch.unwrap_or_default();
    lines.push(role_field(
        "branch",
        branch_or_detached(branch),
        branch_role(branch, facts.default_override),
    ));
    if facts.current {
        lines.push(field("checkout", "current"));
    }
    if facts.hidden {
        lines.push(role_field("config", "hidden", SegRole::Muted));
    }
}

fn branch_or_detached(branch: &str) -> String {
    if is_detached_head_branch(branch) {
        "detached HEAD".into()
    } else {
        branch.to_string()
    }
}

/// Branch: name, default or feature, local branch count. A detached
/// checkout has no name: its kind says `detached HEAD` once.
fn branch_fields(state: &AppState, row: &VisibleRow, lines: &mut Vec<PopoverLine>) {
    let branch = row.chrome.branch.as_str();
    let default_override = row.chrome.default_branch_override.as_deref();
    let role = branch_role(branch, default_override);
    if is_detached_head_branch(branch) {
        lines.push(role_field("kind", "detached HEAD", role));
    } else {
        let kind = if is_default_branch(branch, default_override) {
            "default branch"
        } else {
            "feature branch"
        };
        lines.push(role_field("name", branch, role));
        lines.push(field("kind", kind));
    }
    if let Some(repo) = row_repo(state, row) {
        lines.push(field(
            "local",
            counted(repo.local_branches.len(), "branch", "branches"),
        ));
    }
}

/// The default-branch ref the merge mark compares with: the cached tip ref
/// (`origin/<default>` or `<default>`), else the configured default name,
/// else the first local `main`, `master`, or `develop` (the default-branch
/// rule), else `the default branch`.
fn default_ref(state: &AppState, row: &VisibleRow) -> String {
    let repo = row_repo(state, row);
    repo.and_then(|repo| repo.default_tip_ref.clone())
        .or_else(|| row.chrome.default_branch_override.clone())
        .or_else(|| {
            let local = &repo?.local_branches;
            DEFAULT_BRANCH_NAMES
                .iter()
                .find(|name| local.iter().any(|branch| branch == *name))
                .map(|name| (*name).to_string())
        })
        .unwrap_or_else(|| "the default branch".into())
}

/// No updates group: repo count and names.
fn group_fields(state: &AppState, row: &VisibleRow, lines: &mut Vec<PopoverLine>) {
    let names: Vec<&str> = find_node(&state.tree, &row.id)
        .map(|group| {
            group
                .children
                .iter()
                .map(|child| child.label.as_str())
                .collect()
        })
        .unwrap_or_default();
    lines.push(field("repos", counted(names.len(), "repo", "repos")));
    if !names.is_empty() {
        lines.push(field("names", names.join(", ")));
    }
}

/// Snapshot row of the checkout `row` stands for.
fn row_repo<'a>(state: &'a AppState, row: &VisibleRow) -> Option<&'a WorkspaceRepoSnapshot> {
    let path = row.repo.as_deref()?;
    state.snapshot.repos.iter().find(|repo| repo.repo == path)
}

/// Checkout rows under a repo family row, in tree order.
fn family_checkouts<'a>(state: &'a AppState, row: &VisibleRow) -> Vec<&'a TreeNode> {
    find_node(&state.tree, &row.id)
        .map(|node| {
            node.children
                .iter()
                .filter(|child| child.kind == NodeKind::Checkout)
                .collect()
        })
        .unwrap_or_default()
}

/// Checkout paths whose changes `row` counts: every checkout of a family,
/// else the row's own checkout.
fn row_checkouts(state: &AppState, row: &VisibleRow) -> Vec<String> {
    if row.chrome.is_family {
        family_checkouts(state, row)
            .into_iter()
            .filter_map(|checkout| checkout.repo.clone())
            .collect()
    } else {
        row.repo.iter().cloned().collect()
    }
}

/// Staged, unstaged, and untracked path counts of `checkouts`. A path
/// staged and changed again counts in both.
fn change_counts(state: &AppState, checkouts: &[String]) -> (usize, usize, usize) {
    let mut counts = (0, 0, 0);
    let changes = state
        .snapshot
        .repos
        .iter()
        .filter(|repo| checkouts.contains(&repo.repo))
        .flat_map(|repo| &repo.changes);
    for change in changes {
        if change.untracked {
            counts.2 += 1;
            continue;
        }
        counts.0 += usize::from(change.staged_status.is_some());
        counts.1 += usize::from(change.unstaged_status.is_some());
    }
    counts
}

/// `staged`, `unstaged`, and `untracked` count fields in the colours
/// their status letters paint in; a zero count is muted.
fn change_count_fields(
    (staged, unstaged, untracked): (usize, usize, usize),
    lines: &mut Vec<PopoverLine>,
) {
    for (label, n, role) in [
        ("staged", staged, SegRole::Added),
        ("unstaged", unstaged, SegRole::Modified),
        ("untracked", untracked, SegRole::Added),
    ] {
        let role = if n == 0 { SegRole::Muted } else { role };
        lines.push(role_field(label, n.to_string(), role));
    }
}

fn is_status_kind(kind: IconKind) -> bool {
    matches!(
        kind,
        IconKind::StatusAdded
            | IconKind::StatusStaged
            | IconKind::StatusStagedModified
            | IconKind::StatusModified
            | IconKind::StatusDeleted
            | IconKind::StatusRenamed
            | IconKind::StatusConflict
            | IconKind::StatusCopied
    )
}

fn is_comment_kind(kind: IconKind) -> bool {
    matches!(kind, IconKind::Comment | IconKind::CommentResolved)
}

fn is_sync_kind(kind: IconKind) -> bool {
    matches!(
        kind,
        IconKind::Ahead
            | IconKind::Behind
            | IconKind::Diverged
            | IconKind::NoUpstream
            | IconKind::Clean
    )
}

/// Sync mark fields from `sync_note` and its actions: Pull / Push /
/// Fetch as the state calls for. A diverged branch whose upstream was
/// force-pushed shows that text in place of the ahead / behind counts.
fn sync_fields(
    kind: IconKind,
    note: &str,
    rewrite: Option<&UpstreamRewrite>,
    lines: &mut Vec<PopoverLine>,
) -> Vec<Action> {
    match kind {
        IconKind::Ahead => {
            lines.push(role_field(
                "ahead",
                commits(note_count(note, "ahead by "), "not pushed"),
                SegRole::Added,
            ));
            vec![Action::Push, Action::Fetch]
        }
        IconKind::Behind => {
            lines.push(role_field(
                "behind",
                commits(note_count(note, "behind by "), "to pull"),
                SegRole::Deleted,
            ));
            vec![Action::Pull, Action::Fetch]
        }
        IconKind::Diverged => {
            let counts = note
                .strip_prefix("diverged (")
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or(note);
            let value = rewrite.map_or_else(|| counts.to_string(), upstream_rewrite_text);
            lines.push(role_field("sync", value, SegRole::Modified));
            vec![Action::Fetch, Action::Pull, Action::Push]
        }
        IconKind::NoUpstream => {
            if !note.is_empty() {
                lines.push(role_field("note", note, SegRole::Muted));
            }
            vec![Action::Push, Action::Fetch]
        }
        _ => vec![Action::Fetch],
    }
}

/// The number after `prefix` in a sync note (`ahead by 3 commits`).
fn note_count(note: &str, prefix: &str) -> Option<u64> {
    capture_count(note, prefix).parse().ok()
}

/// `3 commits <tail>`, `1 commit <tail>`, or `commits <tail>` with no count.
fn commits(count: Option<u64>, tail: &str) -> String {
    match count {
        Some(1) => format!("1 commit {tail}"),
        Some(n) => format!("{n} commits {tail}"),
        None => format!("commits {tail}"),
    }
}

/// Shown while the PR detail fetch runs.
pub const PR_DETAIL_LOADING: &str = "loading…";

/// Shown when the PR detail fetch failed.
pub const PR_DETAIL_FAILED: &str = "could not load details";

/// Title of the PR popover line that copies the PR URL.
pub const PR_COPY_URL_TITLE: &str = "Copy URL";

/// PR badge: the heading `<glyph> PR · <state label>`, the lines below in
/// this order (each only when it has data), a rule, then the actions.
///
/// | Line | Label | From |
/// | --- | --- | --- |
/// | ref and title (the title once the detail lands) | `number` | badge, detail |
/// | `loading…` or `could not load details` | (note) | detail state |
/// | forge state words, `DRAFT`, review decision | `state` | detail |
/// | `▸ <next action>`, or `merged <age>` | `next` | badge |
/// | author | `author` | detail |
/// | `head → base` | `branch` | detail |
/// | `after <base ref> (<base state>)` | `stack` | known PRs |
/// | checks summary, or GitLab `pipeline <status>` | `checks` | detail |
/// | failed, then running, check names (`+N more`) | (blank) | detail |
/// | review count | `reviews` / `approvals` | detail |
/// | blockers | `notes` | badge |
/// | update time | `updated` | detail |
/// | last successful lookup, and the last failure | `checked` | lookups |
/// | URL (`y` copies it) | `url` | badge |
///
/// Actions: Open PR (`gx`), the CI page by its [`CiLinkKind::label`]
/// (`gc`), Copy URL (`y`), and Refresh this PR (`r`) unless merged.
///
/// [`CiLinkKind::label`]: super::pr_status::CiLinkKind::label
fn pr_section(state: &AppState, repo: &Path, now_unix: i64) -> Option<PopoverSection> {
    let pr = state.pull_request_for(repo)?;
    let (_, role) = pr_badge_mark(state.ascii, pr.state);
    let detail = match state.pull_request_detail_for(repo) {
        Some(PrDetailState::Ready(detail)) => Some(detail),
        _ => None,
    };
    let pr_ref = pr.ref_text();
    let number = match detail.filter(|detail| !detail.title.is_empty()) {
        Some(detail) => format!("{pr_ref} {}", detail.title),
        None => pr_ref.clone(),
    };
    let mut lines = vec![bold_field("number", number, SegRole::Repo, pr_ref.len())];
    lines.extend(pr_detail_lines(state, repo, now_unix));
    lines.push(PopoverLine::Rule);
    lines.extend(PopoverLine::action(&Action::OpenPullRequest));
    lines.extend(PopoverLine::titled_action(
        &Action::OpenPullRequestCi,
        pr.ci_link.kind.label(),
    ));
    // `y` and `r` run these only inside the pinned popover (the palette
    // rows show no chip), so the lines carry the chip themselves.
    lines.extend(
        PopoverLine::titled_action(&Action::CopyPullRequestUrl, PR_COPY_URL_TITLE)
            .map(|line| line.with_keys("y")),
    );
    if pr.state != PrState::Merged {
        lines.extend(
            PopoverLine::action(&Action::RefreshPullRequest).map(|line| line.with_keys("r")),
        );
    }
    Some(PopoverSection {
        icon: pr_badge_kind(pr.state),
        role,
        target: IconTarget::PullRequest(repo.to_path_buf()),
        title: Some(format!("PR · {}", pr.state.label())),
        lines,
    })
}

/// The lines of the PR of checkout `repo` under its `number` line, as the
/// PR popover and the PRs tab detail pane show them: the `loading…` or
/// `could not load details` note, then `state` to `url` (see
/// [`pr_section`]), each only when it has data. Empty when the checkout
/// shows no PR.
pub(crate) fn pr_detail_lines(state: &AppState, repo: &Path, now_unix: i64) -> Vec<PopoverLine> {
    let Some(pr) = state.pull_request_for(repo) else {
        return Vec::new();
    };
    let ascii = state.ascii;
    let (_, role) = pr_badge_mark(ascii, pr.state);
    let detail_state = state.pull_request_detail_for(repo);
    let detail = match detail_state {
        Some(PrDetailState::Ready(detail)) => Some(detail),
        _ => None,
    };
    let stack_base = state.pr_stack_base(repo);
    let mut lines = Vec::new();
    match detail_state {
        Some(PrDetailState::Loading(_)) => lines.push(PopoverLine::Note(PR_DETAIL_LOADING.into())),
        Some(PrDetailState::Failed) => lines.push(PopoverLine::Note(PR_DETAIL_FAILED.into())),
        Some(PrDetailState::Ready(_)) | None => {}
    }
    if let Some(detail) = detail {
        lines.extend(pr_state_field(detail, role));
    }
    let next = match pr.next_action(stack_base) {
        Some(next) => format!("{} {}", if ascii { ">" } else { "▸" }, next.text()),
        None => match pr.updated_at {
            Some(unix) => format!("merged {}", relative_age(now_unix, unix)),
            None => "merged".into(),
        },
    };
    let next_len = next.len();
    lines.push(bold_field("next", next, role, next_len));
    if let Some(detail) = detail {
        if let Some(author) = &detail.author {
            lines.push(field("author", author.clone()));
        }
        if let (Some(head), Some(base)) = (&detail.head, &detail.base) {
            let arrow = if ascii { "->" } else { "→" };
            lines.push(role_field(
                "branch",
                format!("{head} {arrow} {base}"),
                SegRole::BranchFeature,
            ));
        }
    }
    if let Some(base) = stack_base {
        lines.push(role_field(
            "stack",
            format!("after {} ({})", base.ref_text(), base.state.label()),
            SegRole::Modified,
        ));
    }
    if let Some(detail) = detail {
        pr_check_fields(ascii, detail, &mut lines);
        lines.extend(pr_review_field(detail));
    }
    let notes = pr.notes.texts();
    if !notes.is_empty() {
        lines.push(role_field("notes", notes.join(" · "), SegRole::Modified));
    }
    if let Some(unix) = detail.and_then(|detail| detail.updated_at) {
        let local = format_local_timestamp(unix);
        let relative = format_relative_date(unix, now_unix);
        lines.push(field(
            "updated",
            if relative == local {
                local
            } else {
                format!("{local} ({relative})")
            },
        ));
    }
    lines.extend(pr_checked_field(state, repo, now_unix));
    lines.push(link_field("url", pr.url.clone()));
    lines
}

/// `state`: the forge state words, `DRAFT`, and the review decision, in
/// the badge colour `badge`. `None` when the forge sent none of them.
fn pr_state_field(detail: &PullRequestDetail, badge: SegRole) -> Option<PopoverLine> {
    let mut state = Vec::new();
    if !detail.state.is_empty() {
        state.push(detail.state.as_str());
    }
    if detail.draft {
        state.push("DRAFT");
    }
    if let Some(review) = detail.review {
        state.push(review.label());
    }
    (!state.is_empty()).then(|| role_field("state", state.join(" · "), badge))
}

/// `checks`, then the named failed and running checks (blank label, at
/// most [`super::pr_status::NAMED_CHECKS_MAX`]: a mark in the bucket colour,
/// `✗` / `…`, then the plain name) and `+N more`.
///
/// GitHub sums the checks ([`checks_text`]); GitLab names its head
/// pipeline status (`pipeline failed`). The colour follows the worst
/// bucket: failed, running, passed, else muted. A check with no name is
/// counted in `+N more`.
fn pr_check_fields(ascii: bool, detail: &PullRequestDetail, lines: &mut Vec<PopoverLine>) {
    let checks = &detail.checks;
    let checks_role = if checks.fail > 0 {
        SegRole::Deleted
    } else if checks.pending > 0 {
        SegRole::Modified
    } else if checks.pass > 0 {
        SegRole::Added
    } else {
        SegRole::Muted
    };
    let text = match detail.pipeline.as_deref() {
        Some(status) => format!("pipeline {status}"),
        None => checks_text(ascii, checks),
    };
    lines.push(role_field("checks", text, checks_role));
    let mut more = detail.more_checks;
    for check in &detail.check_names {
        if check.name.is_empty() {
            more += 1;
            continue;
        }
        let mark = match (check.bucket, ascii) {
            (CheckBucket::Fail, false) => ("✗", SegRole::Deleted),
            (CheckBucket::Fail, true) => ("fail", SegRole::Deleted),
            (CheckBucket::Pending, false) => ("…", SegRole::Modified),
            (CheckBucket::Pending, true) => ("pending", SegRole::Modified),
            (CheckBucket::Pass, false) => ("✓", SegRole::Added),
            (CheckBucket::Pass, true) => ("pass", SegRole::Added),
        };
        // Only the mark takes the bucket colour; the name stays plain.
        lines.push(PopoverLine::Field {
            label: "",
            value: check.name.clone(),
            role: SegRole::File,
            link: false,
            bold: 0,
            mark: Some(mark),
        });
    }
    if more > 0 {
        lines.push(role_field("", format!("+{more} more"), SegRole::Muted));
    }
}

/// GitHub `reviews <n> approved · <decision>` (no decision: the count
/// alone), or GitLab `approvals <given> of <required>`. Added when the
/// review rule is met ([`PrReview::Approved`]), else modified. `None` when
/// the forge sent no count.
fn pr_review_field(detail: &PullRequestDetail) -> Option<PopoverLine> {
    let role = if detail.review == Some(PrReview::Approved) {
        SegRole::Added
    } else {
        SegRole::Modified
    };
    Some(match detail.reviews? {
        ReviewCount::Approved(n) => {
            let value = match detail.review {
                Some(review) => format!("{n} approved · {}", review.label()),
                None => format!("{n} approved"),
            };
            role_field("reviews", value, role)
        }
        ReviewCount::Approvals { given, required } => {
            role_field("approvals", format!("{given} of {required}"), role)
        }
    })
}

/// `checked`: how long ago the last successful lookup of checkout `repo`
/// landed, muted (`12m ago`). After a failed lookup it says so in the
/// modified colour, with the CLI's reason when it gave one
/// (`12m ago · last lookup failed: HTTP 401`). `None` with neither.
fn pr_checked_field(state: &AppState, repo: &Path, now_unix: i64) -> Option<PopoverLine> {
    let age = state.pr_checked_at(repo).map(|at| {
        let secs = Instant::now().saturating_duration_since(at).as_secs();
        relative_age(now_unix, now_unix.saturating_sub_unsigned(secs))
    });
    let Some(failure) = state.pr_lookup_failure(repo) else {
        return age.map(|age| role_field("checked", age, SegRole::Muted));
    };
    let failed = match &failure.reason {
        Some(reason) => format!("last lookup failed: {reason}"),
        None => "last lookup failed".into(),
    };
    let value = match age {
        Some(age) => format!("{age} · {failed}"),
        None => failed,
    };
    Some(role_field("checked", value, SegRole::Modified))
}

/// `✓ P · ✗ F · … N` (ASCII: `P pass · F fail · N pending`), or `none`.
fn checks_text(ascii: bool, checks: &ChecksSummary) -> String {
    if *checks == ChecksSummary::default() {
        return "none".into();
    }
    if ascii {
        format!(
            "{} pass · {} fail · {} pending",
            checks.pass, checks.fail, checks.pending
        )
    } else {
        format!(
            "✓ {} · ✗ {} · … {}",
            checks.pass, checks.fail, checks.pending
        )
    }
}

/// Section of icon `kind` on the graph row with [`graph_row_id`] `id`:
/// a node, the uncommitted glyph, or the comment mark. `None` when the row
/// is gone or no longer paints the icon.
///
/// The commit and HEAD nodes are one slot, and so are both comment marks:
/// HEAD moving or the last comment resolving keeps the section and shows
/// the new kind.
fn graph_row_section(
    state: &AppState,
    rows: &SectionRows<'_>,
    kind: IconKind,
    id: &str,
    now_unix: i64,
) -> Option<PopoverSection> {
    let model = state.graph.as_ref()?;
    let row = rows.graph_row(id)?;
    let comments;
    let (kind, role) = if is_comment_kind(kind) {
        comments = state.graph_row_comments(row);
        if comments.is_empty() {
            return None;
        }
        if comments.iter().all(|entry| entry.resolved) {
            (IconKind::CommentResolved, SegRole::Muted)
        } else {
            (IconKind::Comment, SegRole::Heading)
        }
    } else {
        comments = Vec::new();
        match (kind, row) {
            (
                IconKind::GraphCommit | IconKind::GraphHeadCommit,
                GraphRow::Commit { is_head, .. },
            ) => {
                let kind = if *is_head {
                    IconKind::GraphHeadCommit
                } else {
                    IconKind::GraphCommit
                };
                (kind, SegRole::Heading)
            }
            (IconKind::GraphStash, GraphRow::Stash(_)) => (kind, SegRole::Heading),
            (IconKind::GraphUncommitted, GraphRow::Uncommitted { has_changes }) => {
                let role = if *has_changes {
                    SegRole::Modified
                } else {
                    SegRole::Muted
                };
                (kind, role)
            }
            _ => return None,
        }
    };
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    let actions = match row {
        _ if is_comment_kind(kind) => comment_fields(&comments, &mut lines),
        GraphRow::Commit { commit, .. } if kind == IconKind::GraphHeadCommit => {
            commit_fields(commit, now_unix, &mut lines);
            if let Some(sync) = model.sync.as_ref() {
                let branch = branch_role(&sync.branch, model.default_branch_override.as_deref());
                lines.push(role_field("head", format!("on {}", sync.branch), branch));
                lines.push(role_field(
                    "sync",
                    graph_sync_text(state, sync),
                    graph_sync_role(sync.status),
                ));
            }
            vec![
                Action::CompareCommitVsParent,
                Action::GraphCreateBranch,
                Action::ViewRemotely,
            ]
        }
        GraphRow::Commit { commit, .. } => {
            commit_fields(commit, now_unix, &mut lines);
            vec![
                Action::CompareCommitVsParent,
                Action::GraphCreateBranch,
                Action::GraphCheckout,
                Action::GraphMerge,
                Action::ViewRemotely,
            ]
        }
        GraphRow::Stash(stash) => {
            stash_fields(stash, now_unix, &mut lines);
            vec![
                Action::GraphStashApply,
                Action::GraphStashPop,
                Action::GraphStashDrop,
            ]
        }
        GraphRow::Uncommitted { has_changes } => {
            let words = if *has_changes {
                "uncommitted changes"
            } else {
                "working tree clean"
            };
            lines.push(role_field("state", words, role));
            let repo: Vec<String> = state
                .graph_identity
                .iter()
                .map(|(repo, _)| repo.clone())
                .collect();
            change_count_fields(change_counts(state, &repo), &mut lines);
            vec![Action::StashMenu, Action::Refresh]
        }
        GraphRow::Worktree(_) => return None,
    };
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::GraphRow(id.to_string()),
        title: None,
        lines,
    })
}

/// Commit node fields: short id, subject, author, date, parents, refs.
fn commit_fields(commit: &Commit, now_unix: i64, lines: &mut Vec<PopoverLine>) {
    lines.push(field("id", short_id(&commit.id)));
    lines.push(field("subject", commit.subject.clone()));
    author_and_date(
        &commit.author_name,
        commit.author_date_unix,
        now_unix,
        lines,
    );
    let parents: Vec<&str> = commit.parents.iter().map(|id| short_id(id)).collect();
    lines.push(field(
        "parents",
        match parents.len() {
            0 => "none (root commit)".to_string(),
            1 => parents[0].to_string(),
            _ => format!("{} (merge)", parents.join(" ")),
        },
    ));
    if !commit.refs.is_empty() {
        let names: Vec<&str> = commit.refs.iter().map(|r| r.name.as_str()).collect();
        lines.push(field("refs", names.join(", ")));
    }
}

/// Stash tip fields: `stash@{n}`, subject, author, date, parent.
fn stash_fields(stash: &Stash, now_unix: i64, lines: &mut Vec<PopoverLine>) {
    lines.push(field("ref", stash.stash_ref.clone()));
    lines.push(field("subject", stash.subject.clone()));
    author_and_date(&stash.author_name, stash.author_date_unix, now_unix, lines);
    if let Some(parent) = stash.parent_id.as_deref() {
        lines.push(field("parent", short_id(parent)));
    }
}

/// `author` and `date` fields, each when git reported it. The date is the
/// local time, then the relative age at `now_unix` when the graph paints
/// one.
fn author_and_date(author: &str, unix: i64, now_unix: i64, lines: &mut Vec<PopoverLine>) {
    if !author.is_empty() {
        lines.push(field("author", author));
    }
    if unix == 0 {
        return;
    }
    let local = format_local_timestamp(unix);
    let relative = format_relative_date(unix, now_unix);
    lines.push(field(
        "date",
        if relative == local {
            local
        } else {
            format!("{local} ({relative})")
        },
    ));
}

/// Colour of a graph sync state, as the tree paints its sync mark.
fn graph_sync_role(status: SyncStatus) -> SegRole {
    match status {
        SyncStatus::Ahead => SegRole::Added,
        SyncStatus::Behind => SegRole::Deleted,
        SyncStatus::Diverged => SegRole::Modified,
        SyncStatus::UpToDate | SyncStatus::NoUpstream => SegRole::Muted,
    }
}

/// HEAD sync from the graph header state: the ahead and behind counts
/// after the graph header glyphs ([`IconKind::Ahead`] and
/// [`IconKind::Behind`] in the active glyph mode), or `up to date` /
/// `no upstream`.
fn graph_sync_text(state: &AppState, sync: &workspace_status_graph::SyncState) -> String {
    let glyphs = if state.ascii { &ASCII } else { &UNICODE };
    match sync.status {
        SyncStatus::NoUpstream => "no upstream".into(),
        SyncStatus::UpToDate => "up to date".into(),
        SyncStatus::Ahead | SyncStatus::Behind | SyncStatus::Diverged => format!(
            "{}{} {}{}",
            glyphs.ahead, sync.ahead, glyphs.behind, sync.behind
        ),
    }
}

/// The graph worktree with `path`, and true when it is its own worktree
/// row (false: a mark on a commit row's spacer).
fn graph_worktree<'a>(rows: &'a SectionRows<'_>, path: &str) -> Option<(&'a Worktree, bool)> {
    rows.graph().iter().find_map(|row| match row {
        GraphRow::Worktree(worktree) => (worktree.path == path).then_some((worktree, true)),
        GraphRow::Commit { worktrees, .. } => worktrees
            .iter()
            .find(|worktree| worktree.path == path)
            .map(|worktree| (worktree, false)),
        GraphRow::Uncommitted { .. } | GraphRow::Stash(_) => None,
    })
}

/// Linked-worktree glyph on the graph: the shared worktree fields. On a
/// worktree row, Open PR and View remotely when a branch is checked out,
/// Remove worktree, and Copy entity reference. A worktree mark on a commit spacer lists no
/// action: selecting it selects the commit row, so those keys would act on
/// the commit, not on the worktree.
fn graph_worktree_section(
    state: &AppState,
    rows: &SectionRows<'_>,
    path: &str,
) -> Option<PopoverSection> {
    let (worktree, own_row) = graph_worktree(rows, path)?;
    let primary = state
        .snapshot
        .repos
        .iter()
        .find(|repo| repo.repo == path)
        .and_then(|repo| repo.primary_repo.as_deref());
    let mut lines = vec![PopoverLine::Text(
        spec(IconKind::LinkedWorktree).meaning.to_string(),
    )];
    worktree_fields(
        &WorktreeFacts {
            path,
            primary,
            branch: worktree.branch.as_deref(),
            default_override: state
                .graph
                .as_ref()
                .and_then(|model| model.default_branch_override.as_deref()),
            current: worktree.is_current,
            hidden: worktree.hidden,
        },
        &mut lines,
    );
    let mut actions = Vec::new();
    if own_row {
        if worktree
            .branch
            .as_deref()
            .is_some_and(|branch| !is_detached_head_branch(branch))
        {
            actions.extend([Action::OpenPullRequest, Action::ViewRemotely]);
        }
        actions.extend([Action::RemoveWorktree, Action::CopyEntityReference]);
    }
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: IconKind::LinkedWorktree,
        role: SegRole::Heading,
        target: IconTarget::GraphWorktree(path.to_string()),
        title: None,
        lines,
    })
}

/// Refs the chips of graph row `row` stand for, whether they are HEAD's,
/// and the commit they are on. A commit row paints its own; the footer of
/// the uncommitted row paints HEAD's; a worktree row's footer paints its
/// branch (no commit). `None` on a stash row.
fn chip_refs<'a>(
    model: &'a workspace_status_graph::GraphModel,
    row: &'a GraphRow,
) -> Option<(Vec<GraphRef>, bool, Option<&'a Commit>)> {
    Some(match row {
        GraphRow::Commit {
            commit, is_head, ..
        } => (commit.refs.clone(), *is_head, Some(commit)),
        GraphRow::Uncommitted { .. } => {
            let head = model.head_id.as_deref()?;
            let commit = model.commits.iter().find(|commit| commit.id == head)?;
            (commit.refs.clone(), true, Some(commit))
        }
        GraphRow::Worktree(worktree) => (
            worktree
                .branch
                .iter()
                .map(|name| GraphRef {
                    kind: RefKind::Local,
                    name: name.clone(),
                })
                .collect(),
            false,
            None,
        ),
        GraphRow::Stash(_) => return None,
    })
}

/// Catalog kind of a ref chip over `refs`, as the graph colours it: a tag,
/// a local branch (default or not), or a remote with no local match
/// (default when its short name is).
fn ref_chip_kind(refs: &[GraphRef], default_override: Option<&str>) -> Option<IconKind> {
    let first = refs.first()?;
    let short = match first.kind {
        RefKind::Remote => first
            .name
            .split_once('/')
            .map_or(first.name.as_str(), |(_, rest)| rest),
        RefKind::Local | RefKind::Tag => first.name.as_str(),
    };
    Some(match first.kind {
        RefKind::Tag => IconKind::ChipTag,
        _ if is_default_branch(short, default_override) => IconKind::ChipDefault,
        RefKind::Local => IconKind::ChipLocal,
        RefKind::Remote => IconKind::ChipRemote,
    })
}

/// Field label and value of one hidden chip in a `[+N]` popover, in the
/// chip's colour.
fn hidden_chip_field(chip: &PartTarget, default_override: Option<&str>) -> Option<PopoverLine> {
    match chip {
        PartTarget::Chip(refs) => {
            let label = match refs.as_slice() {
                [_, _, ..] => "branch",
                [one] => match one.kind {
                    RefKind::Local => "local",
                    RefKind::Remote => "remote",
                    RefKind::Tag => "tag",
                },
                [] => return None,
            };
            let role = chip_role(ref_chip_kind(refs, default_override)?);
            let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
            Some(role_field(label, names.join(" + "), role))
        }
        PartTarget::DetachedHead => Some(field("detached", "HEAD")),
        PartTarget::Marks { .. } | PartTarget::Overflow(_) => None,
    }
}

/// Colour role of a ref chip of `kind`, as the graph paints it.
fn chip_role(kind: IconKind) -> SegRole {
    match kind {
        IconKind::ChipDefault => SegRole::BranchDefault,
        IconKind::ChipRemote => SegRole::Dir,
        IconKind::ChipTag => SegRole::Modified,
        _ => SegRole::BranchFeature,
    }
}

/// Section of a graph chip on row `row_id`: a ref chip, the checkout or
/// sync half of a mark run, `[HEAD]`, or `[+N]`. `None` when the row is
/// gone or no longer holds the refs the chip names.
///
/// Actions that act on the focused commit (Checkout commit refs, Create
/// branch at commit) are listed only when the chip is on its own commit
/// row: a click on a spacer chip selects that commit. The footer of the
/// uncommitted row shows HEAD's chips, where those keys would act on the
/// uncommitted row instead. Pull, Push, Fetch, and the pickers act on the
/// checkout; off the tree the gates refuse them, and the line shows dim
/// with the reason. View remotely on a ref chip, a mark run, or `[HEAD]`
/// opens that ref's page (or HEAD's commit) on every row (see
/// `AppState::remote_target_for_icon`).
fn graph_chip_section(
    state: &AppState,
    rows: &SectionRows<'_>,
    kind: IconKind,
    row_id: &str,
    chip: &PartTarget,
) -> Option<PopoverSection> {
    // A compare tab paints no graph: its chips are the commit footer's,
    // whose row and model may come from the tab's own load.
    let footer_model;
    let footer_row;
    let (model, row) = if state.is_compare_tab() {
        footer_row = state
            .commit_detail_graph_row()
            .filter(|row| graph_row_id(row) == row_id)?;
        footer_model = state.commit_footer_model();
        (&*footer_model, &footer_row)
    } else {
        (state.graph.as_ref()?, rows.graph_row(row_id)?)
    };
    let (refs, is_head, commit) = chip_refs(model, row)?;
    let on_commit_row = matches!(row, GraphRow::Commit { .. });
    let head_branch = model.sync.as_ref().map(|sync| sync.branch.as_str());
    let default_override = model.default_branch_override.as_deref();
    let has = |kind: RefKind, name: &str| refs.iter().any(|r| r.kind == kind && r.name == name);
    let mut fields = Vec::new();
    let (kind, role, actions) = match (kind, chip) {
        (IconKind::ChipCheckout, PartTarget::Marks { branch, .. }) => {
            if !is_head || head_branch != Some(branch.as_str()) {
                return None;
            }
            fields.push(role_field(
                "head",
                format!("on {branch}"),
                branch_role(branch, default_override),
            ));
            (
                kind,
                SegRole::Heading,
                vec![
                    Action::Pull,
                    Action::Push,
                    Action::Fetch,
                    Action::ViewRemotely,
                ],
            )
        }
        (
            IconKind::ChipSynced,
            PartTarget::Marks {
                branch,
                checkout,
                remote: Some(remote),
            },
        ) => {
            if !has(RefKind::Local, branch) || !has(RefKind::Remote, remote) {
                return None;
            }
            fields.push(role_field(
                "branch",
                branch.clone(),
                branch_role(branch, default_override),
            ));
            fields.push(role_field(
                "sync",
                format!("in sync with {remote}"),
                SegRole::Dir,
            ));
            // With the checkout section above, its Pull / Push / Fetch and
            // View remotely cover the run. A synced branch that is not
            // checked out only fetches (pull and push act on the
            // checked-out branch) and opens that branch remotely.
            let actions = if *checkout {
                Vec::new()
            } else {
                vec![Action::Fetch, Action::ViewRemotely]
            };
            let role = if *checkout {
                SegRole::Heading
            } else {
                SegRole::Dir
            };
            (kind, role, actions)
        }
        (IconKind::ChipDetachedHead, PartTarget::DetachedHead) => {
            let commit = commit?;
            if !is_head || head_branch.is_some_and(|branch| !is_detached_head_branch(branch)) {
                return None;
            }
            fields.push(field(
                "head",
                format!("detached at {}", short_id(&commit.id)),
            ));
            let mut actions = vec![Action::Branch];
            if on_commit_row {
                actions.push(Action::GraphCreateBranch);
            }
            // Opens the commit of the row (HEAD), on every row.
            actions.push(Action::ViewRemotely);
            (kind, SegRole::Heading, actions)
        }
        (IconKind::ChipOverflow, PartTarget::Overflow(hidden)) => {
            // The hidden list is the one the chip had when it painted. A
            // ref that is gone since (a fetch pruned it, HEAD left the
            // detached commit) drops out; with none left, so does the
            // section.
            let detached =
                is_head && head_branch.is_none_or(is_detached_head_branch) && commit.is_some();
            let present = |chip: &&PartTarget| match chip {
                PartTarget::Chip(chip_refs) => chip_refs.iter().all(|r| has(r.kind, &r.name)),
                PartTarget::DetachedHead => detached,
                PartTarget::Marks { .. } | PartTarget::Overflow(_) => false,
            };
            let shown: Vec<PopoverLine> = hidden
                .iter()
                .filter(present)
                .filter_map(|chip| hidden_chip_field(chip, default_override))
                .collect();
            if shown.is_empty() {
                return None;
            }
            fields.extend(shown);
            let actions = if on_commit_row {
                vec![Action::GraphCheckout]
            } else {
                Vec::new()
            };
            (kind, SegRole::Heading, actions)
        }
        (_, PartTarget::Chip(chip_refs)) => {
            if chip_refs.iter().any(|r| !has(r.kind, &r.name)) {
                return None;
            }
            let kind = ref_chip_kind(chip_refs, default_override)?;
            let role = chip_role(kind);
            let first = chip_refs.first()?;
            fields.push(role_field("name", first.name.clone(), role));
            let what = match (first.kind, chip_refs.len()) {
                (RefKind::Local, 1) => "local branch",
                (RefKind::Local, _) => "local branch and remote",
                (RefKind::Remote, _) => "remote-tracking branch",
                (RefKind::Tag, _) => "tag",
            };
            fields.push(field("kind", what));
            if let Some(remote) = chip_refs.get(1) {
                fields.push(role_field("remote", remote.name.clone(), SegRole::Dir));
            }
            if kind == IconKind::ChipDefault {
                fields.push(field("default", "yes"));
            }
            let mut actions = match first.kind {
                RefKind::Local => vec![Action::GraphFocusBranches, Action::CompareVsBranch],
                RefKind::Remote => vec![Action::Fetch],
                RefKind::Tag => Vec::new(),
            };
            if on_commit_row {
                actions.push(Action::GraphCheckout);
            }
            // Opens this ref's page, not the row's commit.
            actions.push(Action::ViewRemotely);
            (kind, role, actions)
        }
        _ => return None,
    };
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    lines.extend(fields);
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::GraphChip {
            row: row_id.to_string(),
            chip: chip.clone(),
        },
        title: None,
        lines,
    })
}

/// The graph sync header: branch, ahead and behind counts, and the state
/// in words; Pull, Push, Fetch, View remotely (HEAD's branch). The kind
/// follows the live state ([`graph_sync_kind`]), so a pinned header
/// popover shows a fetch that moved it.
fn graph_sync_section(state: &AppState) -> Option<PopoverSection> {
    let sync = state.graph.as_ref()?.sync.as_ref()?;
    let kind = graph_sync_kind(sync.status);
    let default_override = state
        .graph
        .as_ref()
        .and_then(|model| model.default_branch_override.as_deref());
    let mut lines = vec![
        PopoverLine::Text(spec(kind).meaning.to_string()),
        role_field(
            "branch",
            sync.branch.clone(),
            branch_role(&sync.branch, default_override),
        ),
    ];
    let (words, role) = match sync.status {
        SyncStatus::Ahead => ("ahead", SegRole::Added),
        SyncStatus::Behind => ("behind", SegRole::Deleted),
        SyncStatus::Diverged => ("diverged", SegRole::Modified),
        SyncStatus::UpToDate => ("up to date", SegRole::Muted),
        SyncStatus::NoUpstream => ("no upstream", SegRole::Muted),
    };
    if matches!(
        sync.status,
        SyncStatus::Ahead | SyncStatus::Behind | SyncStatus::Diverged
    ) {
        lines.push(role_field(
            "ahead",
            commits(Some(sync.ahead.into()), "not pushed"),
            SegRole::Added,
        ));
        lines.push(role_field(
            "behind",
            commits(Some(sync.behind.into()), "to pull"),
            SegRole::Deleted,
        ));
    }
    lines.push(role_field("state", words, role));
    lines.extend(
        [
            Action::Pull,
            Action::Push,
            Action::Fetch,
            Action::ViewRemotely,
        ]
        .iter()
        .filter_map(PopoverLine::action),
    );
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::GraphSync,
        title: None,
        lines,
    })
}

/// The footer hint of graph row `row_id`: how many message lines are
/// below the shown ones; Collapse / expand and Taller message. `None` once
/// the row is not selected or every line shows.
fn graph_more_lines_section(
    state: &AppState,
    rows: &SectionRows<'_>,
    row_id: &str,
) -> Option<PopoverSection> {
    if graph_row_id(rows.graph().get(state.graph_cursor)?) != row_id {
        return None;
    }
    more_lines_section(
        state.graph_footer_lines_below(),
        IconTarget::GraphMoreLines(row_id.to_string()),
    )
}

/// The depth-2 commit-files footer hint of the commit or stash with
/// identity `id`: the same section as [`graph_more_lines_section`], with
/// the count of that footer. `None` once another source shows or every
/// line shows.
fn commit_detail_more_lines_section(state: &AppState, id: &str) -> Option<PopoverSection> {
    if state.commit_detail_msg_identity().as_deref() != Some(id) {
        return None;
    }
    more_lines_section(
        state.commit_detail_footer_lines_below(),
        IconTarget::CommitDetailMoreLines(id.to_string()),
    )
}

/// A commit-message footer's `more lines below` section: the meaning,
/// `below` hidden lines, then Collapse / expand and Taller message.
/// `None` when `below` is 0. Every footer hint uses it.
fn more_lines_section(below: usize, target: IconTarget) -> Option<PopoverSection> {
    if below == 0 {
        return None;
    }
    let mut lines = vec![
        PopoverLine::Text(spec(IconKind::GraphMoreBelow).meaning.to_string()),
        field("below", counted(below, "more line", "more lines")),
    ];
    lines.extend(
        [Action::ToggleCommitMsgExpand, Action::ResizeCommitMsg(1)]
            .iter()
            .filter_map(PopoverLine::action),
    );
    Some(PopoverSection {
        icon: IconKind::GraphMoreBelow,
        role: SegRole::Muted,
        target,
        title: None,
        lines,
    })
}

/// Every line of `sections`, in order. [`PopoverState::focus_line`]
/// indexes this list.
pub fn flat_lines(sections: &[PopoverSection]) -> Vec<&PopoverLine> {
    sections
        .iter()
        .flat_map(|section| section.lines.iter())
        .collect()
}

/// Line a new pinned popover focuses: the first action, else the first
/// field, else 0.
pub fn landing_line(lines: &[&PopoverLine]) -> usize {
    lines
        .iter()
        .position(|line| matches!(line, PopoverLine::Action { .. }))
        .or_else(|| lines.iter().position(|line| line.focusable()))
        .unwrap_or(0)
}

/// The focusable line `focus` stands for: itself, else the nearest one
/// after it, else the nearest one before it. `None` when no line focuses.
///
/// Content re-derives every frame, so a stored focus can land on a line
/// that moved or went away.
pub fn focused_line(lines: &[&PopoverLine], focus: usize) -> Option<usize> {
    let focus = focus.min(lines.len().checked_sub(1)?);
    (focus..lines.len())
        .chain((0..focus).rev())
        .find(|&index| lines[index].focusable())
}

/// Focus after `delta` focusable steps from `focus`, stopping at the ends.
pub fn step_line(lines: &[&PopoverLine], focus: usize, delta: i32) -> usize {
    let Some(mut at) = focused_line(lines, focus) else {
        return focus;
    };
    for _ in 0..delta.unsigned_abs() {
        let next = if delta > 0 {
            (at + 1..lines.len()).find(|&index| lines[index].focusable())
        } else {
            (0..at).rev().find(|&index| lines[index].focusable())
        };
        match next {
            Some(index) => at = index,
            None => break,
        }
    }
    at
}

/// Box of a `width` × `height` popover that hangs from `anchor` inside
/// `bounds`.
///
/// It opens on the row below the anchor, flips above it when the rows
/// below cannot hold it, and sits at the bottom of `bounds` when neither
/// side can. Its left edge is the anchor column, moved left so the box
/// stays inside `bounds`. A box larger than `bounds` shrinks to it.
pub fn popover_rect(anchor: Rect, bounds: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(bounds.width);
    let height = height.min(bounds.height);
    let below = anchor.y.saturating_add(1);
    let y = if below >= bounds.y && below.saturating_add(height) <= bounds.bottom() {
        below
    } else if anchor.y >= bounds.y.saturating_add(height) {
        anchor.y - height
    } else {
        bounds.bottom().saturating_sub(height).max(bounds.y)
    };
    let x = anchor
        .x
        .min(bounds.right().saturating_sub(width))
        .max(bounds.x);
    Rect::new(x, y, width, height)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::command_palette::PALETTE_COMMANDS;

    fn action_line(title: &str) -> PopoverLine {
        let command = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == title)
            .expect("palette row");
        PopoverLine::Action {
            command,
            title: command.title,
            keys: command.keys,
        }
    }

    fn field(value: &str) -> PopoverLine {
        super::field("x", value)
    }

    fn sync_value(lines: &[PopoverLine]) -> Option<&str> {
        lines.iter().find_map(|line| match line {
            PopoverLine::Field {
                label: "sync",
                value,
                ..
            } => Some(value.as_str()),
            _ => None,
        })
    }

    #[test]
    fn diverged_popover_shows_the_force_push_text_when_classified() {
        let rewrite = crate::git::UpstreamRewrite {
            upstream: "origin/main".into(),
            fork_point: "f".repeat(40),
            tip: "t".repeat(40),
            yours: 1,
            dropped: 2,
        };
        let note = "diverged (ahead 3, behind 1)";
        let mut lines = Vec::new();
        let actions = sync_fields(IconKind::Diverged, note, Some(&rewrite), &mut lines);
        assert_eq!(
            sync_value(&lines),
            Some("origin/main was force-pushed \u{b7} 2 dropped, 1 yours")
        );
        assert_eq!(actions, vec![Action::Fetch, Action::Pull, Action::Push]);

        let mut plain = Vec::new();
        sync_fields(IconKind::Diverged, note, None, &mut plain);
        assert_eq!(sync_value(&plain), Some("ahead 3, behind 1"));
    }

    #[test]
    fn worktree_branch_honours_the_configured_default() {
        let role = |branch, default_override| {
            let mut lines = Vec::new();
            worktree_fields(
                &WorktreeFacts {
                    path: "/w/wt",
                    primary: None,
                    branch: Some(branch),
                    default_override,
                    current: false,
                    hidden: false,
                },
                &mut lines,
            );
            lines.iter().find_map(|line| match line {
                PopoverLine::Field {
                    label: "branch",
                    role,
                    ..
                } => Some(*role),
                _ => None,
            })
        };
        assert_eq!(role("trunk", Some("trunk")), Some(SegRole::BranchDefault));
        assert_eq!(role("main", Some("trunk")), Some(SegRole::BranchFeature));
        assert_eq!(role("main", None), Some(SegRole::BranchDefault));
        assert_eq!(role("trunk", None), Some(SegRole::BranchFeature));
    }

    #[test]
    fn rect_opens_below_flips_above_and_clamps_right() {
        let bounds = Rect::new(0, 1, 80, 20);
        let icon = |x, y| Rect::new(x, y, 1, 1);
        assert_eq!(
            popover_rect(icon(10, 3), bounds, 30, 6),
            Rect::new(10, 4, 30, 6),
            "below the icon"
        );
        assert_eq!(
            popover_rect(icon(10, 20), bounds, 30, 6),
            Rect::new(10, 14, 30, 6),
            "bottom row flips above"
        );
        assert_eq!(
            popover_rect(icon(75, 3), bounds, 30, 6),
            Rect::new(50, 4, 30, 6),
            "right edge clamps left"
        );
        assert_eq!(
            popover_rect(icon(5, 10), bounds, 30, 30),
            Rect::new(5, 1, 30, 20),
            "taller than the bounds shrinks and sits at the top"
        );
        assert_eq!(
            popover_rect(icon(5, 4), Rect::new(0, 1, 80, 8), 20, 6),
            Rect::new(5, 3, 20, 6),
            "no room either side sits at the bottom"
        );
    }

    #[test]
    fn focus_moves_over_fields_and_actions_only() {
        let text = PopoverLine::Text("meaning".into());
        let url = field("https://example.test/pull/7");
        let pull = action_line("Pull behind");
        let fetch = action_line("Fetch remotes");
        let lines = vec![&text, &url, &pull, &fetch];
        assert_eq!(landing_line(&lines), 2, "lands on the first action");
        assert_eq!(step_line(&lines, 2, 1), 3);
        assert_eq!(step_line(&lines, 3, 1), 3, "stops at the last line");
        assert_eq!(step_line(&lines, 2, -1), 1);
        assert_eq!(step_line(&lines, 1, -1), 1, "the meaning never focuses");
        assert_eq!(focused_line(&lines, 0), Some(1));
        assert_eq!(focused_line(&lines, 9), Some(3), "past the end clamps");
        let only_text = vec![&text];
        assert_eq!(focused_line(&only_text, 0), None);
        assert_eq!(landing_line(&only_text), 0);
        assert_eq!(landing_line(&[&text, &url]), 1, "no action: first field");
    }

    #[test]
    fn lines_copy_the_value_or_title() {
        assert_eq!(field("#7").copy_text(), "#7");
        assert_eq!(action_line("Push").copy_text(), "Push");
        assert_eq!(PopoverLine::Text("m".into()).copy_text(), "m");
        assert_eq!(
            PopoverLine::action(&Action::Push),
            Some(action_line("Push"))
        );
        assert_eq!(PopoverLine::action(&Action::PopoverClose), None);
    }

    // ---- PR section: field lines (number … url) and the actions ----

    mod pr {
        use std::path::{Path, PathBuf};
        use std::time::{Duration, Instant};

        use super::super::*;
        use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};
        use crate::tui::action::Effect;
        use crate::tui::pr_status::{
            checks_ci_link, pipeline_ci_link, Check, CiLink, CiLinkKind, PrNotes,
        };
        use crate::tui::pull_request::{Forge, PrDetailLookup, PrLookup, PullRequest};
        use crate::tui::state::unix_now;

        const GH_REMOTE: &str = "git@github.com:acme/app.git";
        const GL_REMOTE: &str = "git@gitlab.com:acme/services/api.git";
        const GH_PR: &str = "https://github.com/acme/app/pull/318";
        const GL_MR: &str = "https://gitlab.com/acme/services/api/-/merge_requests/12";
        const FAILED_JOB: &str = "https://github.com/acme/app/actions/runs/1/job/2";
        const PIPELINE: &str = "https://gitlab.com/acme/services/api/-/pipelines/99";

        /// Both forges, as (remote, a PR in `state` on it).
        fn forges(state: PrState) -> [(&'static str, PullRequest); 2] {
            [(GH_REMOTE, gh_pr(state)), (GL_REMOTE, gl_mr(state))]
        }

        fn gh_pr(state: PrState) -> PullRequest {
            let mut pr = PullRequest::fixture(318, GH_PR, state);
            pr.title = "Retry failed uploads with backoff".into();
            pr.head = "feature/ABC-123-retry".into();
            pr
        }

        fn gl_mr(state: PrState) -> PullRequest {
            let mut pr = PullRequest::fixture(12, GL_MR, state);
            pr.title = "Add the retry API".into();
            pr.head = "feature/ABC-123-retry".into();
            pr
        }

        fn checkout(name: &str, branch: &str) -> RepoSnapshot {
            RepoSnapshot {
                repo: name.into(),
                branch: branch.into(),
                sync_status: SyncStatus::UpToDate,
                sync_note: String::new(),
                upstream_rewrite: None,
                head: "abc".into(),
                has_unstaged: false,
                has_staged: false,
                has_untracked: false,
                changes: Vec::new(),
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
                merged_into_default: None,
                default_branch_override: None,
                default_tip_ref: None,
                default_branch: None,
                local_branches: Vec::new(),
            }
        }

        /// One checkout per PR (named `c<i>`, on the PR's head branch),
        /// each PR the badge answer at `remote`, in ASCII mode.
        fn app_with(remote: &str, prs: &[PullRequest]) -> AppState {
            let names: Vec<String> = (0..prs.len()).map(|i| format!("c{i}")).collect();
            let repos: Vec<RepoSnapshot> = names
                .iter()
                .zip(prs)
                .map(|(name, pr)| checkout(name, &pr.head))
                .collect();
            let snapshot = build_workspace_snapshot(&repos, &[], false, &[]);
            let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
            let _ = app.due_pr_lookups();
            for (name, pr) in names.iter().zip(prs) {
                assert!(app.apply_pr_lookup(
                    Path::new(name),
                    &pr.head,
                    Some(remote.into()),
                    PrLookup::Found(pr.clone()),
                ));
            }
            app
        }

        /// What the popover holds open: a pinned PR section on checkout
        /// `name`.
        fn pin(app: &mut AppState, name: &str) -> (IconKind, IconTarget) {
            let kind = pr_badge_kind(app.pr_badge(Path::new(name)).expect("badge"));
            let target = (kind, IconTarget::PullRequest(PathBuf::from(name)));
            app.popover = Some(PopoverState {
                origin: PopoverOrigin::Pinned,
                targets: vec![target.clone()],
                anchor: None,
                focus_line: 0,
                owner: app.popover_owner(),
            });
            target
        }

        /// The PR section of checkout `name`, pinned: the detail fetch
        /// asked for (left loading), and `detail` landed when given.
        fn section_of(
            app: &mut AppState,
            name: &str,
            detail: Option<PrDetailLookup>,
        ) -> PopoverSection {
            let target = pin(app, name);
            let fetch = app.request_pr_details();
            if let Some(detail) = detail {
                let Effect::LookupPullRequestDetail {
                    repo,
                    branch,
                    remote,
                    number,
                    request,
                } = fetch
                else {
                    panic!("the pin asks for the detail: {fetch:?}");
                };
                assert!(app.apply_pr_detail(&repo, &branch, &remote, number, request, detail));
            }
            popover_sections(app, &[target], unix_now())
                .pop()
                .expect("PR section")
        }

        /// A line as text: `label: value` (`label: mark value` with a
        /// mark), `(note)`, `---` for the rule, `[title] keys` for an
        /// action.
        fn line_text(line: &PopoverLine) -> String {
            match line {
                PopoverLine::Field {
                    label,
                    value,
                    mark: Some((mark, _)),
                    ..
                } => format!("{label}: {mark} {value}"),
                PopoverLine::Field { label, value, .. } => format!("{label}: {value}"),
                PopoverLine::Text(text) | PopoverLine::Note(text) => format!("({text})"),
                PopoverLine::Rule => "---".into(),
                PopoverLine::Action { title, keys, .. } => format!("[{title}] {keys}"),
            }
        }

        fn texts(section: &PopoverSection) -> Vec<String> {
            section.lines.iter().map(line_text).collect()
        }

        /// The field whose [`line_text`] starts with `text`.
        fn field_of<'a>(section: &'a PopoverSection, text: &str) -> &'a PopoverLine {
            section
                .lines
                .iter()
                .find(|line| {
                    matches!(line, PopoverLine::Field { .. }) && line_text(line).starts_with(text)
                })
                .unwrap_or_else(|| panic!("no line {text:?} in {:?}", texts(section)))
        }

        /// Colour and bold head of the value of the field that starts with
        /// `text`.
        fn style_of(section: &PopoverSection, text: &str) -> (SegRole, usize) {
            match field_of(section, text) {
                PopoverLine::Field { role, bold, .. } => (*role, *bold),
                _ => unreachable!("a field"),
            }
        }

        /// Colour of the mark of the field that starts with `text`.
        fn mark_role_of(section: &PopoverSection, text: &str) -> Option<SegRole> {
            match field_of(section, text) {
                PopoverLine::Field { mark, .. } => mark.map(|(_, role)| role),
                _ => unreachable!("a field"),
            }
        }

        fn actions(section: &PopoverSection) -> Vec<String> {
            let lines = texts(section);
            let rule = lines.iter().position(|line| line == "---").expect("rule");
            lines[rule + 1..].to_vec()
        }

        fn check(bucket: CheckBucket, name: &str, url: Option<&str>) -> Check {
            Check {
                bucket,
                name: name.into(),
                url: url.map(str::to_string),
            }
        }

        fn gh_detail() -> PullRequestDetail {
            PullRequestDetail {
                number: 318,
                title: "Retry failed uploads with backoff".into(),
                state: "OPEN".into(),
                draft: false,
                author: Some("octo-user".into()),
                head: Some("feature/ABC-123-retry".into()),
                base: Some("main".into()),
                review: Some(PrReview::ReviewRequired),
                reviews: Some(ReviewCount::Approved(1)),
                checks: ChecksSummary {
                    pass: 6,
                    fail: 1,
                    pending: 2,
                },
                pipeline: None,
                check_names: vec![
                    check(CheckBucket::Fail, "test (ubuntu-latest)", Some(FAILED_JOB)),
                    check(CheckBucket::Pending, "build", None),
                ],
                more_checks: 1,
                notes: PrNotes::default(),
                ci_link: checks_ci_link(&[], GH_PR),
                updated_at: Some(unix_now() - 2 * 3600),
            }
        }

        fn gl_detail() -> PullRequestDetail {
            PullRequestDetail {
                number: 12,
                title: "Add the retry API".into(),
                state: "OPEN".into(),
                draft: false,
                author: Some("octo-user".into()),
                head: Some("feature/ABC-123-retry".into()),
                base: Some("main".into()),
                review: Some(PrReview::Approved),
                reviews: Some(ReviewCount::Approvals {
                    given: 2,
                    required: 2,
                }),
                checks: ChecksSummary {
                    pass: 1,
                    fail: 0,
                    pending: 0,
                },
                pipeline: Some("success".into()),
                check_names: Vec::new(),
                more_checks: 0,
                notes: PrNotes::default(),
                ci_link: pipeline_ci_link(Some(PIPELINE), GL_MR),
                updated_at: None,
            }
        }

        #[test]
        fn a_failed_github_pr_lists_every_line_in_order() {
            let mut pr = gh_pr(PrState::CiFailed);
            pr.failed_check = Some("test (ubuntu-latest)".into());
            pr.ci_link = CiLink {
                url: FAILED_JOB.into(),
                kind: CiLinkKind::FailedCheck,
            };
            let mut app = app_with(GH_REMOTE, &[pr]);
            let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(gh_detail())));
            assert_eq!(section.heading(), "PR · CI failed");
            assert_eq!(section.icon, IconKind::PrCiFailed);
            assert_eq!(section.role, SegRole::Deleted);
            let lines = texts(&section);
            assert_eq!(
                lines,
                vec![
                    "number: #318 Retry failed uploads with backoff",
                    "state: OPEN · REVIEW_REQUIRED",
                    "next: > fix CI",
                    "author: octo-user",
                    "branch: feature/ABC-123-retry -> main",
                    "checks: 6 pass · 1 fail · 2 pending",
                    ": fail test (ubuntu-latest)",
                    ": pending build",
                    ": +1 more",
                    "reviews: 1 approved · REVIEW_REQUIRED",
                    lines[10].as_str(),
                    "checked: just now",
                    "url: https://github.com/acme/app/pull/318",
                    "---",
                    "[Open PR] gx",
                    "[Open failed check] gc",
                    "[Copy URL] y",
                    "[Refresh this PR] r",
                ],
            );
            assert!(lines[10].starts_with("updated: ") && lines[10].ends_with(" (2h)"));
            assert_eq!(style_of(&section, "number"), (SegRole::Repo, "#318".len()));
            assert_eq!(style_of(&section, "state"), (SegRole::Deleted, 0));
            assert_eq!(
                style_of(&section, "next"),
                (SegRole::Deleted, "> fix CI".len())
            );
            assert_eq!(style_of(&section, "branch"), (SegRole::BranchFeature, 0));
            assert_eq!(style_of(&section, "checks"), (SegRole::Deleted, 0));
            // The mark takes the bucket colour; the check name stays plain.
            assert_eq!(style_of(&section, ": fail"), (SegRole::File, 0));
            assert_eq!(mark_role_of(&section, ": fail"), Some(SegRole::Deleted));
            assert_eq!(style_of(&section, ": pending"), (SegRole::File, 0));
            assert_eq!(mark_role_of(&section, ": pending"), Some(SegRole::Modified));
            assert_eq!(mark_role_of(&section, ": +1"), None);
            assert_eq!(style_of(&section, ": +1"), (SegRole::Muted, 0));
            assert_eq!(style_of(&section, "reviews"), (SegRole::Modified, 0));
            assert_eq!(style_of(&section, "checked"), (SegRole::Muted, 0));

            // Unicode glyph mode: the arrow, the next mark, and the check
            // marks.
            app.ascii = false;
            let section = section_of(&mut app, "c0", None);
            let lines = texts(&section);
            for want in [
                "next: ▸ fix CI",
                "branch: feature/ABC-123-retry → main",
                "checks: ✓ 6 · ✗ 1 · … 2",
                ": ✗ test (ubuntu-latest)",
                ": … build",
            ] {
                assert!(lines.iter().any(|line| line == want), "{want} in {lines:?}");
            }
        }

        #[test]
        fn a_ready_gitlab_mr_shows_its_pipeline_and_approvals() {
            let mut mr = gl_mr(PrState::Ready);
            mr.ci_link = pipeline_ci_link(Some(PIPELINE), GL_MR);
            let mut app = app_with(GL_REMOTE, &[mr]);
            let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(gl_detail())));
            assert_eq!(section.heading(), "PR · ready to merge");
            assert_eq!(section.icon, IconKind::PrReady);
            assert_eq!(
                texts(&section),
                vec![
                    "number: !12 Add the retry API",
                    "state: OPEN · APPROVED",
                    "next: > merge",
                    "author: octo-user",
                    "branch: feature/ABC-123-retry -> main",
                    "checks: pipeline success",
                    "approvals: 2 of 2",
                    "checked: just now",
                    "url: https://gitlab.com/acme/services/api/-/merge_requests/12",
                    "---",
                    "[Open PR] gx",
                    "[Open pipeline] gc",
                    "[Copy URL] y",
                    "[Refresh this PR] r",
                ]
            );
            assert_eq!(style_of(&section, "number"), (SegRole::Repo, "!12".len()));
            assert_eq!(
                style_of(&section, "next"),
                (SegRole::Added, "> merge".len())
            );
            assert_eq!(style_of(&section, "checks"), (SegRole::Added, 0));
            assert_eq!(style_of(&section, "approvals"), (SegRole::Added, 0));

            // Approvals that do not meet the rules colour modified, even
            // when the numbers match (an approver the rules do not count).
            let mut not_met = gl_detail();
            not_met.review = Some(PrReview::NotApproved);
            not_met.reviews = Some(ReviewCount::Approvals {
                given: 0,
                required: 1,
            });
            not_met.pipeline = Some("failed".into());
            not_met.checks = ChecksSummary {
                pass: 0,
                fail: 1,
                pending: 0,
            };
            not_met.check_names = vec![check(CheckBucket::Fail, "rspec 1/2", None)];
            let mut app = app_with(GL_REMOTE, &[gl_mr(PrState::CiFailed)]);
            let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(not_met)));
            assert_eq!(
                style_of(&section, "approvals: 0 of 1"),
                (SegRole::Modified, 0)
            );
            assert_eq!(
                style_of(&section, "checks: pipeline failed"),
                (SegRole::Deleted, 0)
            );
            // The mark takes the bucket colour; the job name stays plain.
            assert_eq!(style_of(&section, ": fail rspec 1/2"), (SegRole::File, 0));
            assert_eq!(
                mark_role_of(&section, ": fail rspec 1/2"),
                Some(SegRole::Deleted)
            );
        }

        /// A forge that sent no approval count (GitLab with a missing
        /// approvals field; GitHub likewise) shows no review line.
        #[test]
        fn no_review_count_drops_the_review_line_on_both_forges() {
            for (remote, pr) in forges(PrState::InReview) {
                let mut detail = if remote == GL_REMOTE {
                    gl_detail()
                } else {
                    gh_detail()
                };
                detail.reviews = None;
                let mut app = app_with(remote, &[pr]);
                let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(detail)));
                let lines = texts(&section);
                assert!(
                    !lines
                        .iter()
                        .any(|line| line.starts_with("approvals") || line.starts_with("reviews")),
                    "{remote}: {lines:?}"
                );
                assert!(
                    lines.iter().any(|line| line.starts_with("checks")),
                    "{lines:?}"
                );
            }
        }

        #[test]
        fn github_reviews_leave_out_a_missing_decision_and_colour_by_approval() {
            let mut app = app_with(GH_REMOTE, &[gh_pr(PrState::Ready)]);
            let mut detail = gh_detail();
            detail.review = None;
            detail.reviews = Some(ReviewCount::Approved(0));
            let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(detail)));
            assert_eq!(
                style_of(&section, "reviews: 0 approved"),
                (SegRole::Modified, 0)
            );
            assert!(texts(&section).contains(&"reviews: 0 approved".to_string()));

            let mut app = app_with(GH_REMOTE, &[gh_pr(PrState::Ready)]);
            let mut detail = gh_detail();
            detail.review = Some(PrReview::Approved);
            detail.reviews = Some(ReviewCount::Approved(2));
            let section = section_of(&mut app, "c0", Some(PrDetailLookup::Found(detail)));
            assert_eq!(
                style_of(&section, "reviews: 2 approved · APPROVED"),
                (SegRole::Added, 0)
            );
        }

        /// Every state on both forges: the heading, the next line, the CI
        /// action, and Refresh this PR on open PRs only.
        #[test]
        fn each_state_names_its_next_step_and_actions_on_both_forges() {
            for (state, label, next) in [
                (PrState::CiFailed, "CI failed", "next: > fix CI"),
                (PrState::Ready, "ready to merge", "next: > merge"),
                (PrState::CiRunning, "CI running", "next: > wait for CI"),
                (PrState::Draft, "draft", "next: > undraft"),
                (PrState::InReview, "in review", "next: > get review"),
                (PrState::Merged, "merged", "next: merged"),
            ] {
                for (remote, pr) in forges(state) {
                    let ci = match pr.forge {
                        Forge::GitHub => "[Open checks] gc",
                        Forge::GitLab => "[Open pipeline] gc",
                    };
                    let mut app = app_with(remote, &[pr]);
                    let section = section_of(&mut app, "c0", None);
                    let what = format!("{state:?} at {remote}");
                    assert_eq!(section.heading(), format!("PR · {label}"), "{what}");
                    assert_eq!(section.icon, pr_badge_kind(state), "{what}");
                    let lines = texts(&section);
                    assert!(lines.contains(&next.to_string()), "{what}: {lines:?}");
                    assert_eq!(
                        lines[1], "(loading…)",
                        "{what}: the note under the number line"
                    );
                    let mut want = vec!["[Open PR] gx", ci, "[Copy URL] y"];
                    if state != PrState::Merged {
                        want.push("[Refresh this PR] r");
                    }
                    assert_eq!(actions(&section), want, "{what}");
                }
            }
        }

        #[test]
        fn a_merged_pr_says_when_and_keeps_its_ci_label_rule() {
            for (remote, mut pr) in forges(PrState::Merged) {
                pr.updated_at = Some(unix_now() - 3 * 3600);
                if pr.forge == Forge::GitHub {
                    pr.ci_link = CiLink {
                        url: FAILED_JOB.into(),
                        kind: CiLinkKind::FailedCheck,
                    };
                }
                let github = pr.forge == Forge::GitHub;
                let mut app = app_with(remote, &[pr]);
                let section = section_of(&mut app, "c0", None);
                assert_eq!(
                    style_of(&section, "next: merged 3h ago"),
                    (SegRole::BranchFeature, "merged 3h ago".len())
                );
                let ci = if github {
                    "[Open failed check] gc"
                } else {
                    "[Open pipeline] gc"
                };
                assert_eq!(
                    actions(&section),
                    vec!["[Open PR] gx", ci, "[Copy URL] y"],
                    "{remote}"
                );
            }
        }

        #[test]
        fn a_stacked_pr_names_its_base_and_merges_it_first() {
            for remote in [GH_REMOTE, GL_REMOTE] {
                let mut base = if remote == GH_REMOTE {
                    gh_pr(PrState::CiFailed)
                } else {
                    gl_mr(PrState::CiFailed)
                };
                base.head = "feature/ABC-123-retry".into();
                let mut top = if remote == GH_REMOTE {
                    PullRequest::fixture(
                        319,
                        "https://github.com/acme/app/pull/319",
                        PrState::Ready,
                    )
                } else {
                    PullRequest::fixture(
                        13,
                        "https://gitlab.com/acme/services/api/-/merge_requests/13",
                        PrState::Ready,
                    )
                };
                top.head = "feature/ABC-125-retry-ui".into();
                top.base = "feature/ABC-123-retry".into();
                let base_ref = base.ref_text();
                let mut app = app_with(remote, &[base, top]);
                let section = section_of(&mut app, "c1", None);
                let lines = texts(&section);
                let stack = format!("stack: after {base_ref} (CI failed)");
                assert!(lines.contains(&stack), "{lines:?}");
                assert_eq!(style_of(&section, &stack), (SegRole::Modified, 0));
                let next = format!("next: > merge {base_ref} first");
                assert!(lines.contains(&next), "{lines:?}");
                let at = |text: &str| lines.iter().position(|line| line.starts_with(text));
                assert!(at("next") < at("stack") && at("stack") < at("checked"));

                // The base itself is no stack.
                let section = section_of(&mut app, "c0", None);
                assert!(!texts(&section).iter().any(|line| line.starts_with("stack")));
            }
        }

        #[test]
        fn conflicts_and_threads_show_as_notes_and_the_next_step() {
            for (remote, mut pr) in forges(PrState::InReview) {
                pr.notes = PrNotes {
                    conflicts: true,
                    unresolved_threads: true,
                    changes_requested: false,
                };
                let mut app = app_with(remote, &[pr]);
                let section = section_of(&mut app, "c0", None);
                let lines = texts(&section);
                assert!(
                    lines.contains(&"next: > fix conflicts".to_string()),
                    "{lines:?}"
                );
                assert_eq!(
                    style_of(&section, "notes: has conflicts · unresolved threads"),
                    (SegRole::Modified, 0),
                    "{remote}"
                );
            }
        }

        #[test]
        fn a_failing_lookup_keeps_the_pr_and_says_why_on_checked() {
            for (remote, pr) in forges(PrState::CiRunning) {
                let snapshot =
                    build_workspace_snapshot(&[checkout("c0", &pr.head)], &[], false, &[]);
                let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
                let _ = app.due_pr_lookups();
                let then = Instant::now()
                    .checked_sub(Duration::from_secs(12 * 60))
                    .expect("clock");
                let head = pr.head.clone();
                assert!(app.apply_pr_lookup_at(
                    Path::new("c0"),
                    &head,
                    Some(remote.into()),
                    PrLookup::Found(pr),
                    then,
                ));
                let section = section_of(&mut app, "c0", None);
                assert_eq!(style_of(&section, "checked: 12m ago"), (SegRole::Muted, 0));

                for (reason, want) in [
                    (
                        Some("HTTP 401: Bad credentials".to_string()),
                        "checked: 12m ago · last lookup failed: HTTP 401: Bad credentials",
                    ),
                    (None, "checked: 12m ago · last lookup failed"),
                ] {
                    assert!(app.apply_pr_lookup(
                        Path::new("c0"),
                        &head,
                        Some(remote.into()),
                        PrLookup::Failed(reason),
                    ));
                    let section = popover_sections(
                        &app,
                        &app.popover.clone().expect("pinned").targets,
                        unix_now(),
                    )
                    .pop()
                    .expect("the badge stays, so does the section");
                    let lines = texts(&section);
                    assert!(lines.contains(&want.to_string()), "{remote}: {lines:?}");
                    assert_eq!(style_of(&section, want), (SegRole::Modified, 0));
                }
            }
        }

        #[test]
        fn a_failed_detail_keeps_the_badge_lines_and_says_so() {
            for (remote, pr) in forges(PrState::InReview) {
                let pr_ref = pr.ref_text();
                let mut app = app_with(remote, &[pr]);
                let section = section_of(&mut app, "c0", Some(PrDetailLookup::Failed(None)));
                let lines = texts(&section);
                assert_eq!(lines[0], format!("number: {pr_ref}"));
                assert_eq!(lines[1], format!("({PR_DETAIL_FAILED})"));
                assert_eq!(lines[2], "next: > get review");
            }
        }
    }

    #[test]
    fn sync_counts_read_the_note() {
        assert_eq!(note_count("ahead by 3 commits", "ahead by "), Some(3));
        assert_eq!(note_count("behind by 12 commits", "behind by "), Some(12));
        assert_eq!(note_count("", "behind by "), None);
        assert_eq!(commits(Some(1), "to pull"), "1 commit to pull");
        assert_eq!(commits(Some(4), "not pushed"), "4 commits not pushed");
        assert_eq!(commits(None, "to pull"), "commits to pull");
    }
}

//! Icon popovers: a hover peek, a pinned popover, and their content.
//!
//! Paint records one [`super::state::IconHit`] per painted icon (a tagged
//! tree segment, a graph node, row glyph, or comment mark, a PR badge). The pointer resting on a hit for
//! [`PEEK_DWELL_MS`] opens a peek: paint only, never an input mode, so it
//! never takes a key. The pointer leaving both the icon and the peek starts
//! a [`PEEK_GRACE_MS`] grace, then the peek closes.
//!
//! A click on an icon, a click inside a peek, or `gh` pins a popover. A
//! pinned popover is [`super::keys::InputMode::Popover`]: `j` / `k` move
//! between its field and action lines, Enter or a click runs the focused
//! action, `y` copies the focused line, and a drag selects its text. Esc, a
//! click outside it, or a change of the focused row closes it.
//!
//! [`PopoverState`] keeps only what to describe (icon kind and target per
//! section). Every frame rebuilds the sections from live state
//! ([`popover_sections`]), so a watch tick under a pinned popover shows new
//! counts and a vanished icon drops its section. Builders are pure
//! functions of the [`AppState`] and one target, so a new icon kind adds one
//! builder arm here and nothing in the core.

use std::path::{Path, PathBuf};

use ratatui::layout::Rect;
use workspace_status_graph::{
    format_local_timestamp, format_relative_date, short_id, Commit, GraphIconKind, GraphRef,
    GraphRow, PartTarget, RefKind, Stash, SyncStatus, Worktree, ASCII, UNICODE,
};

use crate::helpers::{is_default_branch, is_detached_head_branch};
use crate::snapshot::{CheckoutKind, FileChange, WorkspaceRepoSnapshot};

use super::action::Action;
use super::command_palette::{command_for, PaletteCommand};
use super::comments::{tree_row_comments, CommentEntry};
use super::commit_files::CommitFileRow;
use super::gates::ListFocusTarget;
use super::icons::{capture_count, file_type_name, spec, status_letter_from_change, IconKind};
use super::pull_request::{ChecksSummary, PullRequestDetail};
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
    /// Focused tree row id, commit-file row id, [`graph_row_id`], or
    /// empty.
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
        /// Short lower-case label.
        label: &'static str,
        /// Value text.
        value: String,
    },
    /// An existing command: title and key chip from the palette catalog.
    /// Enter or a click runs its [`Action`].
    Action(&'static PaletteCommand),
}

impl PopoverLine {
    /// The palette row of `action` as a line. `None` when no row runs it.
    pub fn action(action: &Action) -> Option<Self> {
        command_for(action).map(Self::Action)
    }

    /// True for a line `j` / `k` can focus: a field or an action.
    pub fn focusable(&self) -> bool {
        !matches!(self, Self::Text(_) | Self::Note(_))
    }

    /// Text `y` copies: the value of a field, the title of an action.
    pub fn copy_text(&self) -> String {
        match self {
            Self::Text(text) | Self::Note(text) => text.clone(),
            Self::Field { value, .. } => value.clone(),
            Self::Action(command) => command.title.to_string(),
        }
    }
}

/// One icon's part of a popover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PopoverSection {
    /// Icon described. The heading paints its glyph and catalog name.
    pub icon: IconKind,
    /// Colour role of the glyph, as the row paints it.
    pub role: SegRole,
    /// What the icon is on.
    pub target: IconTarget,
    /// The meaning first, then fields, then actions.
    pub lines: Vec<PopoverLine>,
}

/// Target of icon `kind` painted on tree row `row`.
///
/// A PR badge describes the checkout's PR; every other icon describes the
/// row.
pub fn tree_icon_target(kind: IconKind, row: &VisibleRow) -> IconTarget {
    match (kind, row.repo.as_deref()) {
        (IconKind::PrOpen | IconKind::PrApproved | IconKind::PrMerged, Some(repo)) => {
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
/// [`workspace_status_graph::IconSpan::target`]. `None` for a PR badge (its
/// checkout comes from the badge list), the sync header and the footer
/// hint (no row icon), and a worktree mark whose index is not on the row.
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
        GraphIconKind::Badge | GraphIconKind::SyncHeader | GraphIconKind::MoreBelow => None,
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
    targets
        .iter()
        .filter_map(|(kind, target)| section(state, *kind, target, now_unix))
        .collect()
}

fn section(
    state: &AppState,
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
            let row = state
                .commit_file_rows()
                .into_iter()
                .find(|row| &row.id == id)?;
            commit_file_section(state, kind, &row)
        }
        IconTarget::GraphRow(id) => graph_row_section(state, kind, id, now_unix),
        IconTarget::GraphWorktree(path) => graph_worktree_section(state, path),
        IconTarget::GraphChip { row, chip } => graph_chip_section(state, kind, row, chip),
        IconTarget::GraphSync => graph_sync_section(state),
        IconTarget::GraphMoreLines(row) => graph_more_lines_section(state, row),
    }
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
            vec![Action::Fetch, Action::Refresh, Action::ToggleShowIgnored]
        }
        IconKind::Repo => {
            repo_fields(state, row, &mut lines);
            vec![Action::Fetch, Action::Pull, Action::Push, Action::Branch]
        }
        IconKind::LinkedWorktree => {
            worktree_fields(
                &WorktreeFacts {
                    path: &row.chrome.path,
                    primary: row.primary_repo.as_deref(),
                    branch: Some(&row.chrome.branch),
                    current: false,
                    ignored: row.ignored,
                },
                &mut lines,
            );
            vec![
                Action::RemoveWorktree,
                Action::CompareVsDefault,
                Action::CopyEntityReference,
            ]
        }
        IconKind::Branch => {
            branch_fields(state, row, &mut lines);
            vec![
                Action::Branch,
                Action::DefaultBranch,
                Action::GraphFocusBranches,
                Action::CompareVsDefault,
            ]
        }
        IconKind::MergedIntoDefault | IconKind::OpenVsDefault => {
            lines.push(field("vs", default_ref(state, row)));
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
        kind if is_sync_kind(kind) => sync_fields(kind, &row.chrome.sync_note, &mut lines),
        IconKind::StatusFailed => vec![Action::Refresh],
        IconKind::Ignored => vec![Action::ToggleShowIgnored],
        IconKind::ChangeCount => {
            let (staged, unstaged, untracked) = change_counts(state, &row_checkouts(state, row));
            lines.extend([
                field("staged", staged.to_string()),
                field("unstaged", unstaged.to_string()),
                field("untracked", untracked.to_string()),
            ]);
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
                    lines.extend([
                        field("index", side_text(sides.staged_status.as_deref(), false)),
                        field(
                            "worktree",
                            side_text(sides.unstaged_status.as_deref(), sides.untracked),
                        ),
                    ]);
                    if change.unstaged_status.is_some() || change.untracked {
                        actions.push(Action::Stage);
                    }
                    if change.staged_status.is_some() {
                        actions.push(Action::Unstage);
                    }
                }
                None => {
                    lines.push(field(
                        "change",
                        spec(status_letter_from_change(change).icon_kind()).name,
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

/// One side of a workspace change in words: `modified`, `untracked`, or
/// `no change`.
fn side_text(status: Option<&str>, untracked: bool) -> String {
    if untracked {
        return "untracked".into();
    }
    match status {
        None => "no change".into(),
        Some("A") => "added".into(),
        Some("M") => "modified".into(),
        Some("D") => "deleted".into(),
        Some("R") => "renamed".into(),
        Some("C") => "copied".into(),
        Some("U") => "conflict".into(),
        Some("T") => "type changed".into(),
        Some(other) => other.to_string(),
    }
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
        let label = if entry.resolved { "resolved" } else { "open" };
        lines.push(field(label, first));
    }
    vec![Action::CommentStart, Action::ExportComments]
}

fn field(label: &'static str, value: impl Into<String>) -> PopoverLine {
    PopoverLine::Field {
        label,
        value: value.into(),
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
        lines.push(field("branch", row.chrome.branch.clone()));
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
    /// The graph's current checkout.
    current: bool,
    /// Listed as ignored in config.
    ignored: bool,
}

/// Linked worktree: path, primary checkout, branch or detached, then
/// `current` and `ignored` when they hold. The tree and the graph share
/// it.
fn worktree_fields(facts: &WorktreeFacts<'_>, lines: &mut Vec<PopoverLine>) {
    lines.push(field("path", facts.path));
    if let Some(primary) = facts.primary {
        lines.push(field("primary", primary));
    }
    lines.push(field(
        "branch",
        branch_or_detached(facts.branch.unwrap_or_default()),
    ));
    if facts.current {
        lines.push(field("checkout", "current"));
    }
    if facts.ignored {
        lines.push(field("config", "ignored"));
    }
}

fn branch_or_detached(branch: &str) -> String {
    if is_detached_head_branch(branch) {
        "detached HEAD".into()
    } else {
        branch.to_string()
    }
}

/// Branch: name, default or feature, local branch count.
fn branch_fields(state: &AppState, row: &VisibleRow, lines: &mut Vec<PopoverLine>) {
    let branch = row.chrome.branch.as_str();
    let kind = if is_detached_head_branch(branch) {
        "detached HEAD"
    } else if is_default_branch(branch, row.chrome.default_branch_override.as_deref()) {
        "default branch"
    } else {
        "feature branch"
    };
    lines.push(field("name", branch_or_detached(branch)));
    lines.push(field("kind", kind));
    if let Some(repo) = row_repo(state, row) {
        lines.push(field(
            "local",
            counted(repo.local_branches.len(), "branch", "branches"),
        ));
    }
}

/// The default-branch ref the merge mark compares with: the cached tip ref
/// (`origin/<default>` or `<default>`), else the configured default name.
fn default_ref(state: &AppState, row: &VisibleRow) -> String {
    row_repo(state, row)
        .and_then(|repo| repo.default_tip_ref.clone())
        .or_else(|| row.chrome.default_branch_override.clone())
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
/// Fetch as the state calls for.
fn sync_fields(kind: IconKind, note: &str, lines: &mut Vec<PopoverLine>) -> Vec<Action> {
    match kind {
        IconKind::Ahead => {
            lines.push(field(
                "ahead",
                commits(note_count(note, "ahead by "), "not pushed"),
            ));
            vec![Action::Push, Action::Fetch]
        }
        IconKind::Behind => {
            lines.push(field(
                "behind",
                commits(note_count(note, "behind by "), "to pull"),
            ));
            vec![Action::Pull, Action::Fetch]
        }
        IconKind::Diverged => {
            let counts = note
                .strip_prefix("diverged (")
                .and_then(|rest| rest.strip_suffix(')'))
                .unwrap_or(note);
            lines.push(field("sync", counts));
            vec![Action::Fetch, Action::Pull, Action::Push]
        }
        IconKind::NoUpstream => {
            if !note.is_empty() {
                lines.push(field("note", note));
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

/// PR badge: meaning, number, the fetched detail, URL (`y` copies it), and
/// Open PR (`gx`).
///
/// The number and URL come from the badge answer at once. The detail adds
/// the title, state and review, author, `head → base`, checks, and update
/// time once it lands. While it loads, or when it failed, a muted line says
/// so.
fn pr_section(state: &AppState, repo: &Path, now_unix: i64) -> Option<PopoverSection> {
    let pr = state.pull_request_for(repo)?;
    let (_, role) = pr_badge_mark(state.ascii, pr.state);
    let kind = pr_badge_kind(pr.state);
    let mut lines = vec![PopoverLine::Text(spec(kind).meaning.to_string())];
    match state.pull_request_detail_for(repo) {
        Some(PrDetailState::Ready(detail)) => {
            pr_detail_fields(state.ascii, detail, now_unix, &mut lines)
        }
        other => {
            lines.push(field("number", format!("#{}", pr.number)));
            match other {
                Some(PrDetailState::Loading(_)) => {
                    lines.push(PopoverLine::Note(PR_DETAIL_LOADING.into()))
                }
                Some(PrDetailState::Failed) => {
                    lines.push(PopoverLine::Note(PR_DETAIL_FAILED.into()))
                }
                Some(PrDetailState::Ready(_)) | None => {}
            }
        }
    }
    lines.push(field("url", pr.url.clone()));
    lines.extend(PopoverLine::action(&Action::OpenPullRequest));
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::PullRequest(repo.to_path_buf()),
        lines,
    })
}

/// Fields of a fetched PR detail: `#N title`, state (draft, review),
/// author, branches, checks, and update time. Each optional one shows when
/// the forge sent it.
fn pr_detail_fields(
    ascii: bool,
    detail: &PullRequestDetail,
    now_unix: i64,
    lines: &mut Vec<PopoverLine>,
) {
    let number = format!("#{}", detail.number);
    lines.push(field(
        "number",
        if detail.title.is_empty() {
            number
        } else {
            format!("{number} {}", detail.title)
        },
    ));
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
    if !state.is_empty() {
        lines.push(field("state", state.join(" · ")));
    }
    if let Some(author) = &detail.author {
        lines.push(field("author", author.clone()));
    }
    if let (Some(head), Some(base)) = (&detail.head, &detail.base) {
        let arrow = if ascii { "->" } else { "→" };
        lines.push(field("branch", format!("{head} {arrow} {base}")));
    }
    lines.push(field("checks", checks_text(ascii, &detail.checks)));
    if let Some(unix) = detail.updated_at {
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
    kind: IconKind,
    id: &str,
    now_unix: i64,
) -> Option<PopoverSection> {
    let model = state.graph.as_ref()?;
    let rows = model.visible_rows();
    let row = rows.iter().find(|row| graph_row_id(row) == id)?;
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
                lines.push(field("head", format!("on {}", sync.branch)));
                lines.push(field("sync", graph_sync_text(state, sync)));
            }
            vec![Action::CompareCommitVsParent, Action::GraphCreateBranch]
        }
        GraphRow::Commit { commit, .. } => {
            commit_fields(commit, now_unix, &mut lines);
            vec![
                Action::CompareCommitVsParent,
                Action::GraphCreateBranch,
                Action::GraphCheckout,
                Action::GraphMerge,
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
            lines.push(field("state", words));
            let repo: Vec<String> = state
                .graph_identity
                .iter()
                .map(|(repo, _)| repo.clone())
                .collect();
            let (staged, unstaged, untracked) = change_counts(state, &repo);
            lines.extend([
                field("staged", staged.to_string()),
                field("unstaged", unstaged.to_string()),
                field("untracked", untracked.to_string()),
            ]);
            vec![Action::StashMenu, Action::Refresh]
        }
        GraphRow::Worktree(_) => return None,
    };
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::GraphRow(id.to_string()),
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

/// HEAD sync from the graph header state: `↑A ↓B` in the graph glyphs, or
/// `up to date` / `no upstream`.
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
fn graph_worktree(state: &AppState, path: &str) -> Option<(Worktree, bool)> {
    state
        .graph
        .as_ref()?
        .visible_rows()
        .into_iter()
        .find_map(|row| match row {
            GraphRow::Worktree(worktree) => (worktree.path == path).then_some((worktree, true)),
            GraphRow::Commit { worktrees, .. } => worktrees
                .into_iter()
                .find(|worktree| worktree.path == path)
                .map(|worktree| (worktree, false)),
            GraphRow::Uncommitted { .. } | GraphRow::Stash(_) => None,
        })
}

/// Linked-worktree glyph on the graph: the shared worktree fields. On a
/// worktree row, Open PR when a branch is checked out, Remove worktree,
/// and Copy entity reference. A worktree mark on a commit spacer lists no
/// action: selecting it selects the commit row, so those keys would act on
/// the commit, not on the worktree.
fn graph_worktree_section(state: &AppState, path: &str) -> Option<PopoverSection> {
    let (worktree, own_row) = graph_worktree(state, path)?;
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
            current: worktree.is_current,
            ignored: worktree.ignored,
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
            actions.push(Action::OpenPullRequest);
        }
        actions.extend([Action::RemoveWorktree, Action::CopyEntityReference]);
    }
    lines.extend(actions.iter().filter_map(PopoverLine::action));
    Some(PopoverSection {
        icon: IconKind::LinkedWorktree,
        role: SegRole::Heading,
        target: IconTarget::GraphWorktree(path.to_string()),
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

/// Field label and value of one hidden chip in a `[+N]` popover.
fn hidden_chip_field(chip: &PartTarget) -> Option<PopoverLine> {
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
            let names: Vec<&str> = refs.iter().map(|r| r.name.as_str()).collect();
            Some(field(label, names.join(" + ")))
        }
        PartTarget::DetachedHead => Some(field("detached", "HEAD")),
        PartTarget::Marks { .. } | PartTarget::Overflow(_) => None,
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
/// with the reason.
fn graph_chip_section(
    state: &AppState,
    kind: IconKind,
    row_id: &str,
    chip: &PartTarget,
) -> Option<PopoverSection> {
    let model = state.graph.as_ref()?;
    let rows = model.visible_rows();
    let row = rows.iter().find(|row| graph_row_id(row) == row_id)?;
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
            fields.push(field("head", format!("on {branch}")));
            (
                kind,
                SegRole::Heading,
                vec![Action::Pull, Action::Push, Action::Fetch],
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
            fields.push(field("branch", branch.clone()));
            fields.push(field("sync", format!("in sync with {remote}")));
            // With the checkout section above, its Pull / Push / Fetch
            // cover the run. A synced branch that is not checked out only
            // fetches: pull and push act on the checked-out branch.
            let actions = if *checkout {
                Vec::new()
            } else {
                vec![Action::Fetch]
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
            (kind, SegRole::Heading, actions)
        }
        (IconKind::ChipOverflow, PartTarget::Overflow(hidden)) => {
            let present = |chip: &PartTarget| match chip {
                PartTarget::Chip(chip_refs) => chip_refs.iter().all(|r| has(r.kind, &r.name)),
                _ => true,
            };
            if !hidden.iter().any(present) {
                return None;
            }
            fields.extend(hidden.iter().filter_map(hidden_chip_field));
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
            let first = chip_refs.first()?;
            fields.push(field("name", first.name.clone()));
            let what = match (first.kind, chip_refs.len()) {
                (RefKind::Local, 1) => "local branch",
                (RefKind::Local, _) => "local branch and remote",
                (RefKind::Remote, _) => "remote-tracking branch",
                (RefKind::Tag, _) => "tag",
            };
            fields.push(field("kind", what));
            if let Some(remote) = chip_refs.get(1) {
                fields.push(field("remote", remote.name.clone()));
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
            let role = match kind {
                IconKind::ChipDefault => SegRole::BranchDefault,
                IconKind::ChipRemote => SegRole::Dir,
                IconKind::ChipTag => SegRole::Modified,
                _ => SegRole::BranchFeature,
            };
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
        lines,
    })
}

/// The graph sync header: branch, ahead and behind counts, and the state
/// in words; Pull, Push, Fetch. The kind follows the live state
/// ([`graph_sync_kind`]), so a pinned header popover shows a fetch that
/// moved it.
fn graph_sync_section(state: &AppState) -> Option<PopoverSection> {
    let sync = state.graph.as_ref()?.sync.as_ref()?;
    let kind = graph_sync_kind(sync.status);
    let mut lines = vec![
        PopoverLine::Text(spec(kind).meaning.to_string()),
        field("branch", sync.branch.clone()),
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
        lines.push(field(
            "ahead",
            commits(Some(sync.ahead.into()), "not pushed"),
        ));
        lines.push(field(
            "behind",
            commits(Some(sync.behind.into()), "to pull"),
        ));
    }
    lines.push(field("state", words));
    lines.extend(
        [Action::Pull, Action::Push, Action::Fetch]
            .iter()
            .filter_map(PopoverLine::action),
    );
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::GraphSync,
        lines,
    })
}

/// The footer hint of graph row `row_id`: how many message lines are
/// below the shown ones; Collapse / expand and Taller message. `None` once
/// the row is not selected or every line shows.
fn graph_more_lines_section(state: &AppState, row_id: &str) -> Option<PopoverSection> {
    if graph_row_id(&state.focused_graph_row()?) != row_id {
        return None;
    }
    let below = state.graph_footer_lines_below();
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
        target: IconTarget::GraphMoreLines(row_id.to_string()),
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
        .position(|line| matches!(line, PopoverLine::Action(_)))
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
        PopoverLine::Action(
            PALETTE_COMMANDS
                .iter()
                .find(|command| command.title == title)
                .expect("palette row"),
        )
    }

    fn field(value: &str) -> PopoverLine {
        PopoverLine::Field {
            label: "x",
            value: value.into(),
        }
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

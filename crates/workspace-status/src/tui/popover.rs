//! Icon popovers: a hover peek, a pinned popover, and their content.
//!
//! Paint records one [`super::state::IconHit`] per painted icon (a tagged
//! tree segment, a PR badge). The pointer resting on a hit for
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

use crate::helpers::{is_default_branch, is_detached_head_branch};
use crate::snapshot::{CheckoutKind, FileChange, WorkspaceRepoSnapshot};

use super::action::Action;
use super::command_palette::{command_for, PaletteCommand};
use super::comments::{tree_row_comments, CommentEntry};
use super::commit_files::CommitFileRow;
use super::gates::ListFocusTarget;
use super::icons::{capture_count, file_type_name, spec, status_letter_from_change, IconKind};
use super::state::AppState;
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
    /// Focused tree row id, commit-file row id, graph cursor index, or
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
        !matches!(self, Self::Text(_))
    }

    /// Text `y` copies: the value of a field, the title of an action.
    pub fn copy_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
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

/// Sections of `targets` from live state, in order. A target whose icon is
/// gone (row removed, PR answer dropped, sync mark cleared) has none.
pub fn popover_sections(
    state: &AppState,
    targets: &[(IconKind, IconTarget)],
) -> Vec<PopoverSection> {
    targets
        .iter()
        .filter_map(|(kind, target)| section(state, *kind, target))
        .collect()
}

fn section(state: &AppState, kind: IconKind, target: &IconTarget) -> Option<PopoverSection> {
    match target {
        IconTarget::TreeRow(id) => {
            let row = state.rows.iter().find(|row| &row.id == id)?;
            tree_section(state, kind, row)
        }
        IconTarget::PullRequest(repo) => pr_section(state, repo),
        IconTarget::CommitFileRow(id) => {
            let row = state
                .commit_file_rows()
                .into_iter()
                .find(|row| &row.id == id)?;
            commit_file_section(state, kind, &row)
        }
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
            worktree_fields(row, &mut lines);
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

/// Linked worktree: path, primary checkout, branch or detached.
fn worktree_fields(row: &VisibleRow, lines: &mut Vec<PopoverLine>) {
    lines.push(field("path", row.chrome.path.clone()));
    if let Some(primary) = row.primary_repo.as_deref() {
        lines.push(field("primary", primary));
    }
    lines.push(field("branch", branch_or_detached(&row.chrome.branch)));
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

/// PR badge: meaning, number, URL (`y` copies it), and Open PR (`gx`).
fn pr_section(state: &AppState, repo: &Path) -> Option<PopoverSection> {
    let pr = state.pull_request_for(repo)?;
    let (_, role) = pr_badge_mark(state.ascii, pr.state);
    let kind = pr_badge_kind(pr.state);
    let mut lines = vec![
        PopoverLine::Text(spec(kind).meaning.to_string()),
        PopoverLine::Field {
            label: "number",
            value: format!("#{}", pr.number),
        },
        PopoverLine::Field {
            label: "url",
            value: pr.url.clone(),
        },
    ];
    lines.extend(PopoverLine::action(&Action::OpenPullRequest));
    Some(PopoverSection {
        icon: kind,
        role,
        target: IconTarget::PullRequest(repo.to_path_buf()),
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

//! Workspace tree from the same snapshot used by --plain / --json.
//!
//! Row chrome (icons, status letters, sync marks, workspace wording).

use std::collections::{BTreeMap, HashSet};

use crate::git::UpstreamRewrite;
use crate::helpers::{
    is_attention_sync_note, is_default_branch, is_detached_head_branch, visible_width,
    STATUS_FAILED_NOTE,
};
use crate::snapshot::{
    CheckoutKind, FileChange, SyncStatus, WorkspaceRepoSnapshot, WorkspaceSnapshot,
};

use super::icons::{
    file_icon, icon_branch, icon_changes, icon_clean, icon_comment, icon_comment_resolved,
    icon_folder, icon_hidden, icon_linked_worktree, icon_repo, icon_staged, icon_status_failed,
    icon_viewed, icon_workspace, status_letter_from_change, tui_file_badge, tui_merge_mark,
    tui_sync_mark, upstream_rewrite_short_text, IconKind, StatusColorRole,
};
use super::pr_status::PrState;

/// Structural node kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeKind {
    Workspace,
    Repo,
    Checkout,
    Group,
    /// Staged / Changes chrome when a checkout has any staged path.
    Section,
    Dir,
    File,
}

/// Paint / search chrome copied from the snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct NodeChrome {
    /// Repo path, dir path, or file path.
    pub path: String,
    pub branch: String,
    pub checkout_kind: Option<CheckoutKind>,
    pub merged_into_default: Option<bool>,
    pub sync_status: Option<SyncStatus>,
    pub sync_note: String,
    /// Set when this diverged branch's upstream was force-pushed under it.
    pub upstream_rewrite: Option<UpstreamRewrite>,
    /// `HEAD` sha. Watch identity only; not painted.
    pub head: String,
    pub change_count: usize,
    pub sync_summary: String,
    pub default_branch_override: Option<String>,
    /// Family container (primary + linked checkouts).
    pub is_family: bool,
}

/// One node in the workspace tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeNode {
    pub id: String,
    pub kind: NodeKind,
    /// Primary name: workspace label, repo path, dir display name, or file basename.
    pub label: String,
    /// Checkout path used for fetch / pull / default / graph.
    pub repo: Option<String>,
    pub primary_repo: Option<String>,
    pub hidden: bool,
    pub file: Option<FileChange>,
    pub children: Vec<TreeNode>,
    pub chrome: NodeChrome,
}

/// Styled run used by tree paint. Colours resolve from the active theme.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSeg {
    pub text: String,
    pub role: SegRole,
    pub hex: Option<&'static str>,
    pub bold: bool,
    pub dim: bool,
    /// Catalog icon this segment paints, if any. Paint records an icon hit
    /// for its glyph columns (spaces around the glyph excluded), and `gh`
    /// lists one popover section per tagged segment.
    pub icon: Option<IconKind>,
}

/// Semantic colour token for a tree segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegRole {
    Heading,
    Repo,
    Dir,
    File,
    Muted,
    Added,
    Modified,
    Deleted,
    Renamed,
    /// Reviewed eye. Maps to [`crate::tui::theme::Palette::viewed`].
    Viewed,
    BranchDefault,
    BranchFeature,
    /// Terminal default foreground: no colour of its own (`NO_COLOR`).
    Plain,
}

impl From<StatusColorRole> for SegRole {
    fn from(role: StatusColorRole) -> Self {
        match role {
            StatusColorRole::Added => SegRole::Added,
            StatusColorRole::Modified => SegRole::Modified,
            StatusColorRole::Deleted => SegRole::Deleted,
            StatusColorRole::Renamed => SegRole::Renamed,
            StatusColorRole::Muted => SegRole::Muted,
            StatusColorRole::File => SegRole::File,
        }
    }
}

/// Left run + right-aligned trailing run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeSegments {
    pub segments: Vec<TextSeg>,
    pub trailing: Vec<TextSeg>,
}

/// One painted row after fold.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VisibleRow {
    pub id: String,
    pub depth: usize,
    pub kind: NodeKind,
    /// Plain text of left + trailing — pane `/` search and tests.
    pub label: String,
    /// Right-aligned repo kind / status / sync / counts (no leading pad).
    pub trailing: String,
    /// Left run used by paint (linked-worktree / branch / file/folder glyphs + name).
    pub segments: Vec<TextSeg>,
    /// Right run used by paint (icons after the name, repo kind, sync / counts).
    pub trailing_segs: Vec<TextSeg>,
    pub repo: Option<String>,
    pub primary_repo: Option<String>,
    pub hidden: bool,
    pub file: Option<FileChange>,
    pub foldable: bool,
    pub folded: bool,
    pub in_no_updates: bool,
    pub chrome: NodeChrome,
}

impl Default for VisibleRow {
    fn default() -> Self {
        Self {
            id: String::new(),
            depth: 0,
            kind: NodeKind::File,
            label: String::new(),
            trailing: String::new(),
            segments: Vec::new(),
            trailing_segs: Vec::new(),
            repo: None,
            primary_repo: None,
            hidden: false,
            file: None,
            foldable: false,
            folded: false,
            in_no_updates: false,
            chrome: NodeChrome::default(),
        }
    }
}

/// Build the workspace tree from a snapshot (hidden repos may still be present).
/// `tree_mode` true is a directory trie. False is a flat path list.
/// `workspace_label` is the root name (cwd basename).
pub fn build_tree(
    snapshot: &WorkspaceSnapshot,
    tree_mode: bool,
    workspace_label: &str,
) -> TreeNode {
    let mut families: BTreeMap<String, Vec<&WorkspaceRepoSnapshot>> = BTreeMap::new();
    for repo in &snapshot.repos {
        let key = repo
            .primary_repo
            .clone()
            .unwrap_or_else(|| repo.repo.clone());
        families.entry(key).or_default().push(repo);
    }

    let mut attention = Vec::new();
    let mut idle = Vec::new();
    for (primary, members) in families {
        let needs = family_needs_attention(&members);
        let nodes = family_nodes(&primary, members, tree_mode);
        if needs {
            attention.extend(nodes);
        } else {
            idle.extend(nodes);
        }
    }

    let mut children = attention;
    if !idle.is_empty() {
        children.push(TreeNode {
            id: "group:no-updates".into(),
            kind: NodeKind::Group,
            label: "No updates".into(),
            repo: None,
            primary_repo: None,
            hidden: false,
            file: None,
            children: idle,
            chrome: NodeChrome::default(),
        });
    }

    let change_count = snapshot.repos.iter().map(|r| r.changes.len()).sum();
    TreeNode {
        id: "workspace".into(),
        kind: NodeKind::Workspace,
        label: workspace_label.to_string(),
        repo: None,
        primary_repo: None,
        hidden: false,
        file: None,
        children,
        chrome: NodeChrome {
            change_count,
            sync_summary: sync_summary(&snapshot.repos),
            ..NodeChrome::default()
        },
    }
}

/// Cwd basename used as the workspace row label`).
pub fn workspace_label_from_cwd(cwd: &std::path::Path) -> String {
    let name = cwd
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|s| !s.is_empty());
    match name {
        Some(name) => name,
        None => {
            let full = cwd.to_string_lossy();
            if full.is_empty() {
                "workspace".into()
            } else {
                full.into_owned()
            }
        }
    }
}

fn sync_summary(snapshots: &[WorkspaceRepoSnapshot]) -> String {
    if snapshots.is_empty() {
        return "no repos".into();
    }
    let mut behind = 0;
    let mut ahead = 0;
    let mut diverged = 0;
    let mut attention = 0;
    for s in snapshots {
        if is_attention_sync_note(&s.sync_note) {
            attention += 1;
        } else if s.sync_status == SyncStatus::Behind {
            behind += 1;
        } else if s.sync_status == SyncStatus::Ahead {
            ahead += 1;
        } else if s.sync_status == SyncStatus::Diverged {
            diverged += 1;
        }
    }
    let mut parts = Vec::new();
    if behind > 0 {
        parts.push(format!("{behind} behind"));
    }
    if ahead > 0 {
        parts.push(format!("{ahead} ahead"));
    }
    if diverged > 0 {
        parts.push(format!("{diverged} diverged"));
    }
    if attention > 0 {
        parts.push(format!("{attention} attention"));
    }
    if parts.is_empty() {
        "all current".into()
    } else {
        parts.join(", ")
    }
}

fn family_needs_attention(members: &[&WorkspaceRepoSnapshot]) -> bool {
    members.iter().any(|repo| checkout_needs_attention(repo))
}

fn checkout_needs_attention(repo: &WorkspaceRepoSnapshot) -> bool {
    if repo.has_unstaged || repo.has_staged || repo.has_untracked {
        return true;
    }
    if !is_default_branch(&repo.branch, repo.default_branch_override.as_deref()) {
        return true;
    }
    matches!(
        repo.sync_status,
        crate::snapshot::SyncStatus::Behind
            | crate::snapshot::SyncStatus::Ahead
            | crate::snapshot::SyncStatus::Diverged
    ) || is_attention_sync_note(&repo.sync_note)
}

fn family_nodes(
    primary: &str,
    mut members: Vec<&WorkspaceRepoSnapshot>,
    tree_mode: bool,
) -> Vec<TreeNode> {
    members.sort_by(|a, b| {
        let a_linked = i32::from(a.checkout_kind == CheckoutKind::Linked);
        let b_linked = i32::from(b.checkout_kind == CheckoutKind::Linked);
        a_linked.cmp(&b_linked).then_with(|| a.repo.cmp(&b.repo))
    });
    let has_linked = members
        .iter()
        .any(|m| m.checkout_kind == CheckoutKind::Linked);
    let has_primary = members
        .iter()
        .any(|m| m.checkout_kind == CheckoutKind::Primary);
    if has_linked && has_primary {
        return vec![family_container(primary, members, tree_mode)];
    }
    members
        .into_iter()
        .map(|m| repo_or_checkout(m, NodeKind::Repo, tree_mode))
        .collect()
}

fn family_container(
    primary: &str,
    members: Vec<&WorkspaceRepoSnapshot>,
    tree_mode: bool,
) -> TreeNode {
    let children: Vec<TreeNode> = members
        .iter()
        .map(|m| repo_or_checkout(m, NodeKind::Checkout, tree_mode))
        .collect();
    let hidden = members.iter().all(|m| m.hidden);
    let change_count = children.iter().map(|c| c.chrome.change_count).sum();
    let statuses: Vec<SyncStatus> = members.iter().map(|m| m.sync_status).collect();
    let worst = worst_sync_status(&statuses);
    let worst_member = members.iter().find(|m| m.sync_status == worst);
    let note_for_worst = worst_member
        .map(|m| m.sync_note.clone())
        .unwrap_or_default();
    let rewrite_for_worst = worst_member.and_then(|m| m.upstream_rewrite.clone());
    let primary_head = members
        .iter()
        .find(|m| m.checkout_kind == CheckoutKind::Primary)
        .or_else(|| members.first())
        .map(|m| m.head.clone())
        .unwrap_or_default();
    TreeNode {
        id: format!("repo:{primary}"),
        kind: NodeKind::Repo,
        label: primary.to_string(),
        repo: Some(primary.to_string()),
        primary_repo: None,
        hidden,
        file: None,
        children,
        chrome: NodeChrome {
            path: primary.to_string(),
            sync_status: Some(worst),
            sync_note: note_for_worst,
            upstream_rewrite: rewrite_for_worst,
            head: primary_head,
            change_count,
            is_family: true,
            checkout_kind: Some(CheckoutKind::Primary),
            merged_into_default: None,
            ..NodeChrome::default()
        },
    }
}

fn worst_sync_status(statuses: &[SyncStatus]) -> SyncStatus {
    let rank = |s: SyncStatus| match s {
        SyncStatus::Diverged => 4,
        SyncStatus::Behind => 3,
        SyncStatus::Ahead => 2,
        SyncStatus::NoUpstream => 1,
        SyncStatus::UpToDate => 0,
    };
    statuses
        .iter()
        .copied()
        .max_by_key(|s| rank(*s))
        .unwrap_or(SyncStatus::UpToDate)
}

fn repo_or_checkout(repo: &WorkspaceRepoSnapshot, kind: NodeKind, tree_mode: bool) -> TreeNode {
    let files = materialize_change_forest(repo, tree_mode);
    let label = if kind == NodeKind::Checkout {
        checkout_display_name(repo)
    } else {
        repo.repo.clone()
    };
    TreeNode {
        id: match kind {
            NodeKind::Checkout => format!("checkout:{}", repo.repo),
            _ => format!("repo:{}", repo.repo),
        },
        kind,
        label,
        repo: Some(repo.repo.clone()),
        primary_repo: repo.primary_repo.clone(),
        hidden: repo.hidden,
        file: None,
        children: files,
        chrome: NodeChrome {
            path: repo.repo.clone(),
            branch: repo.branch.clone(),
            checkout_kind: Some(repo.checkout_kind),
            merged_into_default: repo.merged_into_default,
            sync_status: Some(repo.sync_status),
            sync_note: repo.sync_note.clone(),
            upstream_rewrite: repo.upstream_rewrite.clone(),
            head: repo.head.clone(),
            change_count: repo.changes.len(),
            default_branch_override: repo.default_branch_override.clone(),
            ..NodeChrome::default()
        },
    }
}

/// Linked detached checkouts show the short worktree path, not `HEAD (detached)`.
fn checkout_display_name(repo: &WorkspaceRepoSnapshot) -> String {
    if repo.checkout_kind == CheckoutKind::Linked && is_detached_head_branch(&repo.branch) {
        linked_short_name(&repo.repo, repo.primary_repo.as_deref())
    } else {
        repo.branch.clone()
    }
}

fn linked_short_name(path: &str, primary_repo: Option<&str>) -> String {
    if let Some(primary) = primary_repo {
        let prefix = format!("{primary}/");
        if let Some(rest) = path.strip_prefix(&prefix) {
            return rest.to_string();
        }
    }
    path.rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(path)
        .to_string()
}

fn file_display_name(change: &FileChange) -> String {
    change
        .path
        .rsplit('/')
        .next()
        .filter(|part| !part.is_empty())
        .unwrap_or(change.path.as_str())
        .to_string()
}

#[derive(Default)]
struct MutableDir {
    dirs: BTreeMap<String, MutableDir>,
    files: Vec<FileChange>,
}

fn add_change(root: &mut MutableDir, change: &FileChange) {
    let mut parts: Vec<&str> = change
        .path
        .split('/')
        .filter(|part| !part.is_empty())
        .collect();
    if parts.pop().is_none() {
        return;
    }
    let mut node = root;
    for dir in parts {
        node = node.dirs.entry(dir.to_string()).or_default();
    }
    node.files.push(change.clone());
}

fn collapse_dir(name: String, mut node: MutableDir) -> (String, MutableDir) {
    let mut collapsed_name = name;
    while node.files.is_empty() && node.dirs.len() == 1 {
        let (child_name, child_node) = node.dirs.into_iter().next().expect("one child dir");
        collapsed_name = format!("{collapsed_name}/{child_name}");
        node = child_node;
    }
    (collapsed_name, node)
}

/// Id of the workspace tree row of changed file `path` in checkout
/// `repo`: `file:<repo>:<path>`. A file with changes on both sides also
/// has a Changes row, this id plus [`UNSTAGED_ROW_SUFFIX`]. Reviewed marks
/// and watch flashes are keyed by these ids; the Explorer tree reads them
/// for its row of the same file.
pub fn file_row_id(repo: &str, path: &str) -> String {
    format!("file:{repo}:{path}")
}

/// Suffix of the Changes-side row id of a file with changes on both sides
/// ([`file_row_id`]).
pub const UNSTAGED_ROW_SUFFIX: &str = "#unstaged";

fn make_file_node(
    repo: &WorkspaceRepoSnapshot,
    change: &FileChange,
    unstaged_suffix: bool,
) -> TreeNode {
    let id = file_row_id(&repo.repo, &change.path);
    let id = if unstaged_suffix {
        format!("{id}{UNSTAGED_ROW_SUFFIX}")
    } else {
        id
    };
    TreeNode {
        id,
        kind: NodeKind::File,
        label: file_display_name(change),
        repo: Some(repo.repo.clone()),
        primary_repo: repo.primary_repo.clone(),
        hidden: repo.hidden,
        file: Some(change.clone()),
        children: Vec::new(),
        chrome: NodeChrome {
            path: change.path.clone(),
            ..NodeChrome::default()
        },
    }
}

fn materialize_dir(
    repo: &WorkspaceRepoSnapshot,
    dir_path: &str,
    node: MutableDir,
    dual_files: &HashSet<String>,
    changes_side: bool,
) -> Vec<TreeNode> {
    let mut dir_entries: Vec<(String, MutableDir)> = node
        .dirs
        .into_iter()
        .map(|(name, child)| collapse_dir(name, child))
        .collect();
    dir_entries.sort_by(|a, b| a.0.cmp(&b.0));
    let mut file_entries = node.files;
    file_entries.sort_by(|a, b| a.path.cmp(&b.path));

    let mut children = Vec::new();
    for (name, child) in dir_entries {
        let full_path = if dir_path.is_empty() {
            name.clone()
        } else {
            format!("{dir_path}/{name}")
        };
        let id_suffix = if changes_side { "#unstaged" } else { "" };
        children.push(TreeNode {
            id: format!("dir:{}:{full_path}{id_suffix}", repo.repo),
            kind: NodeKind::Dir,
            label: name,
            repo: Some(repo.repo.clone()),
            primary_repo: repo.primary_repo.clone(),
            hidden: repo.hidden,
            file: None,
            children: materialize_dir(repo, &full_path, child, dual_files, changes_side),
            chrome: NodeChrome {
                path: full_path.clone(),
                ..NodeChrome::default()
            },
        });
    }
    for change in file_entries {
        let unstaged_suffix = changes_side && dual_files.contains(&change.path);
        children.push(make_file_node(repo, &change, unstaged_suffix));
    }
    children
}

/// Keep index status and clear the worktree side (Staged tree rows / bulk writes).
pub(crate) fn staged_side_change(change: &FileChange) -> FileChange {
    FileChange {
        path: change.path.clone(),
        staged_status: change.staged_status.clone(),
        unstaged_status: None,
        untracked: false,
        old_path: change.old_path.clone(),
    }
}

/// Keep worktree status and clear the index side (Changes tree rows / bulk writes).
pub(crate) fn changes_side_change(change: &FileChange) -> FileChange {
    FileChange {
        path: change.path.clone(),
        staged_status: None,
        unstaged_status: change.unstaged_status.clone(),
        untracked: change.untracked,
        old_path: change.old_path.clone(),
    }
}

fn forest_from_changes(
    repo: &WorkspaceRepoSnapshot,
    changes: &[FileChange],
    tree_mode: bool,
    dual_files: &HashSet<String>,
    changes_side: bool,
) -> Vec<TreeNode> {
    if !tree_mode {
        return changes
            .iter()
            .map(|change| {
                let unstaged_suffix = changes_side && dual_files.contains(&change.path);
                make_file_node(repo, change, unstaged_suffix)
            })
            .collect();
    }
    let mut root = MutableDir::default();
    for change in changes {
        add_change(&mut root, change);
    }
    materialize_dir(repo, "", root, dual_files, changes_side)
}

fn make_section_node(
    repo: &WorkspaceRepoSnapshot,
    side: &str,
    label: &str,
    children: Vec<TreeNode>,
) -> TreeNode {
    TreeNode {
        id: format!("section:{}:{side}", repo.repo),
        kind: NodeKind::Section,
        label: label.into(),
        repo: Some(repo.repo.clone()),
        primary_repo: repo.primary_repo.clone(),
        hidden: repo.hidden,
        file: None,
        children,
        chrome: NodeChrome {
            path: repo.repo.clone(),
            ..NodeChrome::default()
        },
    }
}

/// File / dir forest under a checkout. Matches `materializeChangeForest`.
///
/// When any path is staged, children are Staged / Changes section trees
/// (empty side omitted). Changes dirs always use a `#unstaged` id suffix.
/// Dual MS files use that suffix only when the same path is in both forests.
/// Otherwise the checkout's dirty files sit directly under the repo or
/// worktree, with today's file/dir ids.
fn materialize_change_forest(repo: &WorkspaceRepoSnapshot, tree_mode: bool) -> Vec<TreeNode> {
    let empty = HashSet::new();
    if !repo.changes.iter().any(|c| c.staged_status.is_some()) {
        return forest_from_changes(repo, &repo.changes, tree_mode, &empty, false);
    }

    let staged: Vec<FileChange> = repo
        .changes
        .iter()
        .filter(|c| c.staged_status.is_some())
        .map(staged_side_change)
        .collect();
    let changes: Vec<FileChange> = repo
        .changes
        .iter()
        .filter(|c| c.unstaged_status.is_some() || c.untracked)
        .map(changes_side_change)
        .collect();
    let staged_paths: HashSet<String> = staged.iter().map(|c| c.path.clone()).collect();
    let dual_files: HashSet<String> = changes
        .iter()
        .filter(|c| staged_paths.contains(&c.path))
        .map(|c| c.path.clone())
        .collect();

    let mut children = Vec::new();
    if !staged.is_empty() {
        children.push(make_section_node(
            repo,
            "staged",
            "Staged",
            forest_from_changes(repo, &staged, tree_mode, &empty, false),
        ));
    }
    if !changes.is_empty() {
        children.push(make_section_node(
            repo,
            "changes",
            "Changes",
            forest_from_changes(repo, &changes, tree_mode, &dual_files, true),
        ));
    }
    children
}

/// True when `path` is the dir itself or a child of it.
pub fn path_under_dir(path: &str, dir: &str) -> bool {
    path == dir || path.starts_with(&format!("{dir}/"))
}

/// Dir path from a `dir:{{repo}}:{{fullPath}}` row id.
///
/// A Changes-side suffix (`#unstaged`) is stripped so callers get the
/// filesystem path.
pub fn dir_path_from_id(id: &str, repo: &str) -> Option<String> {
    let prefix = format!("dir:{repo}:");
    let rest = id.strip_prefix(&prefix)?;
    Some(rest.strip_suffix("#unstaged").unwrap_or(rest).to_string())
}

/// Default folds: the No updates group starts closed.
pub fn default_folds(tree: &TreeNode) -> HashSet<String> {
    let mut folds = HashSet::new();
    if tree.children.iter().any(|c| c.id == "group:no-updates") {
        folds.insert("group:no-updates".into());
    }
    folds
}

/// Find a node by id.
pub fn find_node<'a>(node: &'a TreeNode, id: &str) -> Option<&'a TreeNode> {
    if node.id == id {
        return Some(node);
    }
    for child in &node.children {
        if let Some(hit) = find_node(child, id) {
            return Some(hit);
        }
    }
    None
}

fn walk_foldable(node: &TreeNode, out: &mut Vec<String>) {
    match node.kind {
        NodeKind::Workspace
        | NodeKind::Repo
        | NodeKind::Checkout
        | NodeKind::Group
        | NodeKind::Section
        | NodeKind::Dir => {
            out.push(node.id.clone());
            for child in &node.children {
                walk_foldable(child, out);
            }
        }
        NodeKind::File => {}
    }
}

/// Foldable ids for `focus_id` and every foldable descendant under it.
/// Empty when the id is missing or names a file.
pub fn collect_foldable_subtree_ids(tree: &TreeNode, focus_id: &str) -> Vec<String> {
    let Some(found) = find_node(tree, focus_id) else {
        return Vec::new();
    };
    if found.kind == NodeKind::File {
        return Vec::new();
    }
    let mut ids = Vec::new();
    walk_foldable(found, &mut ids);
    ids
}

/// Depth-first flatten, honoring `folds`. Glyphs follow `ascii` (`WS_STATUS_GLYPHS`).
pub fn flatten(tree: &TreeNode, folds: &HashSet<String>) -> Vec<VisibleRow> {
    flatten_with(tree, folds, false)
}

/// Flatten with an explicit ASCII glyph set.
pub fn flatten_with(tree: &TreeNode, folds: &HashSet<String>, ascii: bool) -> Vec<VisibleRow> {
    let tree_mode = detect_tree_mode(tree);
    let mut out = Vec::new();
    walk(tree, 0, folds, ascii, tree_mode, false, &mut out);
    out
}

fn detect_tree_mode(node: &TreeNode) -> bool {
    if node.kind == NodeKind::Dir {
        return true;
    }
    node.children.iter().any(detect_tree_mode)
}

fn walk(
    node: &TreeNode,
    depth: usize,
    folds: &HashSet<String>,
    ascii: bool,
    tree_mode: bool,
    in_no_updates: bool,
    out: &mut Vec<VisibleRow>,
) {
    let foldable = !node.children.is_empty();
    let folded = foldable && folds.contains(&node.id);
    let segs = node_segments(node, tree_mode, in_no_updates, ascii);
    let (label, right_raw) = segments_search_label(&segs);
    out.push(VisibleRow {
        id: node.id.clone(),
        depth,
        kind: node.kind,
        label,
        trailing: right_raw,
        segments: segs.segments,
        trailing_segs: segs.trailing,
        repo: node.repo.clone(),
        primary_repo: node.primary_repo.clone(),
        hidden: node.hidden,
        file: node.file.clone(),
        foldable,
        folded,
        in_no_updates,
        chrome: node.chrome.clone(),
    });
    if folded {
        return;
    }
    let child_in_no_updates = in_no_updates || node.kind == NodeKind::Group;
    for child in &node.children {
        walk(
            child,
            depth + 1,
            folds,
            ascii,
            tree_mode,
            child_in_no_updates,
            out,
        );
    }
}

fn icon_seg(text: &str, role: SegRole) -> TextSeg {
    TextSeg {
        text: format!("{text} "),
        role,
        hex: None,
        bold: false,
        dim: false,
        icon: None,
    }
}

fn text_seg(text: impl Into<String>, role: SegRole) -> TextSeg {
    TextSeg {
        text: text.into(),
        role,
        hex: None,
        bold: false,
        dim: false,
        icon: None,
    }
}

/// [`text_seg`] tagged with the catalog icon it paints.
fn icon_text_seg(text: impl Into<String>, role: SegRole, kind: IconKind) -> TextSeg {
    tagged(text_seg(text, role), kind)
}

/// `seg` tagged with the catalog icon or indicator it paints.
fn tagged(seg: TextSeg, kind: IconKind) -> TextSeg {
    TextSeg {
        icon: Some(kind),
        ..seg
    }
}

/// Muted `N` change count on a repo or checkout row.
fn change_count_seg(count: usize) -> TextSeg {
    icon_text_seg(count.to_string(), SegRole::Muted, IconKind::ChangeCount)
}

/// Catalog kind of the sync mark for `status`.
///
/// Up to date maps to [`IconKind::Clean`]: the only up-to-date mark the
/// tree paints is the clean check under No updates.
pub fn sync_icon_kind(status: SyncStatus) -> IconKind {
    match status {
        SyncStatus::Ahead => IconKind::Ahead,
        SyncStatus::Behind => IconKind::Behind,
        SyncStatus::Diverged => IconKind::Diverged,
        SyncStatus::NoUpstream => IconKind::NoUpstream,
        SyncStatus::UpToDate => IconKind::Clean,
    }
}

/// True when the clean / no-updates check should paint on a repo or checkout.
pub fn show_clean_check(in_no_updates: bool) -> bool {
    in_no_updates
}

/// Pin a repo glyph on the trailing run so a narrow pane can clip the
/// name without dropping that kind mark. Linked-worktree glyphs stay on
/// the left, immediately before the name. One space separates the glyph
/// from the rest of the run.
fn with_repo_kind_icon(trailing: Vec<TextSeg>, glyph: &str, role: SegRole) -> Vec<TextSeg> {
    with_leading_icons(vec![icon_text_seg(glyph, role, IconKind::Repo)], trailing)
}

fn repo_kind_role(hidden: bool) -> SegRole {
    if hidden {
        SegRole::Muted
    } else {
        SegRole::Heading
    }
}

fn sync_trailing(chrome: &NodeChrome, in_no_updates: bool, ascii: bool) -> Vec<TextSeg> {
    let Some(status) = chrome.sync_status else {
        return Vec::new();
    };
    // A failed `git status` has no real sync state: say so instead of the
    // no-upstream mark.
    if chrome.sync_note == STATUS_FAILED_NOTE {
        return vec![
            icon_text_seg(
                icon_status_failed(ascii),
                SegRole::Deleted,
                IconKind::StatusFailed,
            ),
            text_seg(format!(" {STATUS_FAILED_NOTE}"), SegRole::Deleted),
        ];
    }
    let role = SegRole::from(super::icons::sync_color_role(status));
    let kind = sync_icon_kind(status);
    if status == SyncStatus::UpToDate {
        if !show_clean_check(in_no_updates) {
            return Vec::new();
        }
        return vec![icon_text_seg(icon_clean(ascii), role, kind)];
    }
    let mark = icon_text_seg(tui_sync_mark(ascii, status, &chrome.sync_note), role, kind);
    match &chrome.upstream_rewrite {
        Some(rewrite) if status == SyncStatus::Diverged => vec![
            mark,
            text_seg(format!(" {}", upstream_rewrite_short_text(rewrite)), role),
        ],
        _ => vec![mark],
    }
}

/// Single-sided name-status → FileChange.
///
/// `unstaged_status` carries the letter so commit/stash rows keep A/M/D/R/C
/// instead of workspace staged-only `S`.
pub fn file_change_from_name_status(
    status: &str,
    path: impl Into<String>,
    old_path: Option<String>,
) -> FileChange {
    let letter = status.chars().next().unwrap_or('M').to_string();
    FileChange {
        path: path.into(),
        staged_status: None,
        unstaged_status: Some(letter),
        untracked: false,
        old_path,
    }
}

/// File-row segments shared by the workspace tree and commit-file lists.
pub fn file_change_segments(change: &FileChange, tree_mode: bool, ascii: bool) -> NodeSegments {
    file_segments(change, tree_mode, ascii)
}

/// Folder-row segments (workspace dirs and commit-file dirs).
pub fn dir_name_segments(name: &str, ascii: bool) -> NodeSegments {
    NodeSegments {
        segments: vec![
            tagged(icon_seg(icon_folder(ascii), SegRole::Dir), IconKind::Folder),
            text_seg(name, SegRole::Dir),
        ],
        trailing: Vec::new(),
    }
}

/// Search label (left + trailing) and the untrimmed trailing run.
pub fn segments_search_label(segs: &NodeSegments) -> (String, String) {
    let left: String = segs.segments.iter().map(|s| s.text.as_str()).collect();
    let right_raw: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    let right_for_label = right_raw.trim().to_string();
    let label = if right_for_label.is_empty() {
        left.trim_end().to_string()
    } else {
        format!("{}  {right_for_label}", left.trim_end())
    };
    (label, right_raw)
}

/// Devicon segment of a file row at `path`: the [`file_icon`] glyph and
/// one space, in the icon's own colour, tagged [`IconKind::FileType`]. The
/// workspace tree, the commit-file lists, and the Explorer tree share it.
pub fn file_type_seg(path: &str, ascii: bool) -> TextSeg {
    let icon = file_icon(ascii, path);
    TextSeg {
        text: format!("{} ", icon.glyph),
        role: SegRole::File,
        hex: icon.color,
        bold: false,
        dim: false,
        icon: Some(IconKind::FileType),
    }
}

fn file_segments(change: &FileChange, tree_mode: bool, ascii: bool) -> NodeSegments {
    let status = status_letter_from_change(change);
    let color = SegRole::from(status.color_role());
    let name = file_display_name(change);
    let dir = {
        let path = change.path.as_str();
        path.strip_suffix(name.as_str())
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string()
    };

    let mut segments = vec![file_type_seg(&change.path, ascii)];
    if let Some(old) = change.old_path.as_deref() {
        let old_name = if tree_mode {
            old.rsplit('/')
                .next()
                .filter(|p| !p.is_empty())
                .unwrap_or(old)
        } else {
            old
        };
        segments.push(text_seg(format!("{old_name} → "), SegRole::Muted));
    }
    segments.push(TextSeg {
        text: name,
        role: color,
        hex: None,
        bold: false,
        dim: false,
        icon: None,
    });
    if !tree_mode && !dir.is_empty() {
        segments.push(TextSeg {
            text: format!("  {dir}"),
            role: SegRole::Muted,
            hex: None,
            bold: false,
            dim: true,
            icon: None,
        });
    }

    let trailing = vec![TextSeg {
        text: tui_file_badge(change).to_string(),
        role: color,
        hex: None,
        bold: true,
        dim: false,
        icon: Some(status.icon_kind()),
    }];
    NodeSegments { segments, trailing }
}

/// Merge mark next to a checkout branch.
///
/// Linked extras paint the full [`tui_merge_mark`] (check or open). Primary
/// paints the check only when `merged_into_default` is `Some(true)`. Family
/// containers and other kinds omit the mark. Open-vs-default stays linked-only.
fn checkout_merge_mark(node: &TreeNode, ascii: bool) -> &'static str {
    if node.chrome.is_family {
        return "";
    }
    match node.chrome.checkout_kind {
        Some(CheckoutKind::Linked) => tui_merge_mark(ascii, node.chrome.merged_into_default),
        Some(CheckoutKind::Primary) if node.chrome.merged_into_default == Some(true) => {
            tui_merge_mark(ascii, Some(true))
        }
        _ => "",
    }
}

/// True for the merge mark kinds (merged, not merged).
pub fn is_merge_mark_kind(kind: IconKind) -> bool {
    matches!(kind, IconKind::MergedIntoDefault | IconKind::OpenVsDefault)
}

/// The merge mark glyph of a repo or checkout row, its own tagged segment
/// so it takes its own colour (`Added` merged, `Muted` open) and its own
/// icon hit. `None` when the row paints no mark.
fn merge_mark_seg(node: &TreeNode, ascii: bool) -> Option<TextSeg> {
    let mark = checkout_merge_mark(node, ascii);
    if mark.is_empty() {
        return None;
    }
    let (kind, role) = if node.chrome.merged_into_default == Some(true) {
        (IconKind::MergedIntoDefault, SegRole::Added)
    } else {
        (IconKind::OpenVsDefault, SegRole::Muted)
    };
    Some(icon_text_seg(mark, role, kind))
}

/// Hidden glyph of a repo row (muted).
fn hidden_seg(ascii: bool) -> TextSeg {
    icon_text_seg(icon_hidden(ascii), SegRole::Muted, IconKind::Hidden)
}

/// Put `icons` (bare glyph segments) at the start of the trailing run,
/// one space between neighbours and one space before the rest of the run.
///
/// These are the row icons that follow the name in paint order. On the
/// right-aligned trailing run a narrow pane clips the name, not them.
/// Every item of the run is a bare token, so the run packs flush right
/// with no padding at either end.
fn with_leading_icons(icons: Vec<TextSeg>, mut trailing: Vec<TextSeg>) -> Vec<TextSeg> {
    let mut run = Vec::new();
    for icon in icons {
        push_trailing_item(&mut run, icon);
    }
    if !run.is_empty() && !trailing.is_empty() {
        run.push(text_seg(" ", SegRole::Muted));
    }
    run.append(&mut trailing);
    run
}

/// Append the bare token `item` to the trailing `run`, one space after the
/// run's last item.
fn push_trailing_item(run: &mut Vec<TextSeg>, item: TextSeg) {
    if !run.is_empty() {
        run.push(text_seg(" ", SegRole::Muted));
    }
    run.push(item);
}

fn repo_segments(node: &TreeNode, in_no_updates: bool, ascii: bool) -> NodeSegments {
    let name_role = if node.hidden {
        SegRole::Muted
    } else {
        SegRole::Repo
    };
    if node.chrome.is_family {
        let wt_count = node
            .children
            .iter()
            .filter(|c| c.kind == NodeKind::Checkout)
            .count();
        let segments = vec![TextSeg {
            text: node.chrome.path.clone(),
            role: name_role,
            hex: None,
            bold: true,
            dim: false,
            icon: None,
        }];
        let mut trailing = sync_trailing(&node.chrome, in_no_updates, ascii);
        push_trailing_item(
            &mut trailing,
            icon_text_seg(
                format!("{wt_count} wt"),
                SegRole::Muted,
                IconKind::WorktreeCount,
            ),
        );
        if node.chrome.change_count > 0 {
            push_trailing_item(&mut trailing, change_count_seg(node.chrome.change_count));
        }
        trailing = with_repo_kind_icon(trailing, icon_repo(ascii), repo_kind_role(node.hidden));
        let hidden = node.hidden.then(|| hidden_seg(ascii));
        trailing = with_leading_icons(hidden.into_iter().collect(), trailing);
        return NodeSegments { segments, trailing };
    }

    let off_default = !is_default_branch(
        &node.chrome.branch,
        node.chrome.default_branch_override.as_deref(),
    );
    let merge = merge_mark_seg(node, ascii);
    let branch_role = if off_default {
        SegRole::BranchFeature
    } else {
        SegRole::BranchDefault
    };
    // Linked extras only. The primary checkout is a normal repo glyph.
    let linked = node.chrome.checkout_kind == Some(CheckoutKind::Linked);
    let kind_role = repo_kind_role(node.hidden);

    let mut segments = Vec::new();
    if linked {
        segments.push(tagged(
            icon_seg(icon_linked_worktree(ascii), kind_role),
            IconKind::LinkedWorktree,
        ));
        segments.push(TextSeg {
            text: linked_short_name(&node.chrome.path, node.primary_repo.as_deref()),
            role: name_role,
            hex: None,
            bold: true,
            dim: false,
            icon: None,
        });
        if let Some(primary) = node.primary_repo.as_deref() {
            segments.push(TextSeg {
                text: format!(" · {primary}"),
                role: SegRole::Muted,
                hex: None,
                bold: false,
                dim: true,
                icon: None,
            });
        }
    } else {
        segments.push(TextSeg {
            text: node.chrome.path.clone(),
            role: name_role,
            hex: None,
            bold: true,
            dim: false,
            icon: None,
        });
    }
    segments.push(text_seg("  ", SegRole::Muted));
    segments.push(text_seg(node.chrome.branch.clone(), branch_role));

    let mut trailing = sync_trailing(&node.chrome, in_no_updates, ascii);
    if node.chrome.change_count > 0 {
        push_trailing_item(&mut trailing, change_count_seg(node.chrome.change_count));
    }
    if !linked {
        trailing = with_repo_kind_icon(trailing, icon_repo(ascii), kind_role);
    }
    // Icons that follow the name in paint order, then the old trailing run.
    let mut icons = Vec::new();
    if node.hidden {
        icons.push(hidden_seg(ascii));
    }
    icons.push(icon_text_seg(
        icon_branch(ascii),
        branch_role,
        IconKind::Branch,
    ));
    icons.extend(merge);
    NodeSegments {
        segments,
        trailing: with_leading_icons(icons, trailing),
    }
}

fn checkout_segments(node: &TreeNode, in_no_updates: bool, ascii: bool) -> NodeSegments {
    let off_default = !is_default_branch(
        &node.chrome.branch,
        node.chrome.default_branch_override.as_deref(),
    );
    let merge = merge_mark_seg(node, ascii);
    let branch_role = if off_default {
        SegRole::BranchFeature
    } else {
        SegRole::BranchDefault
    };
    // Nested primary uses the branch glyph, never the linked-worktree mark.
    let linked = node.chrome.checkout_kind == Some(CheckoutKind::Linked);
    let (row_icon, row_kind) = if linked {
        (icon_linked_worktree(ascii), IconKind::LinkedWorktree)
    } else {
        (icon_branch(ascii), IconKind::Branch)
    };
    let main_label = if linked && is_detached_head_branch(&node.chrome.branch) {
        linked_short_name(&node.chrome.path, node.primary_repo.as_deref())
    } else {
        node.chrome.branch.clone()
    };
    let segments = vec![
        tagged(icon_seg(row_icon, SegRole::Heading), row_kind),
        TextSeg {
            text: main_label,
            role: branch_role,
            hex: None,
            bold: true,
            dim: false,
            icon: None,
        },
    ];
    let mut trailing = sync_trailing(&node.chrome, in_no_updates, ascii);
    if node.chrome.change_count > 0 {
        push_trailing_item(&mut trailing, change_count_seg(node.chrome.change_count));
    }
    NodeSegments {
        segments,
        trailing: with_leading_icons(merge.into_iter().collect(), trailing),
    }
}

fn workspace_segments(node: &TreeNode, ascii: bool) -> NodeSegments {
    NodeSegments {
        segments: vec![
            tagged(
                icon_seg(icon_workspace(ascii), SegRole::Heading),
                IconKind::Workspace,
            ),
            TextSeg {
                text: node.label.clone(),
                role: SegRole::Heading,
                hex: None,
                bold: true,
                dim: false,
                icon: None,
            },
        ],
        trailing: vec![icon_text_seg(
            format!(
                "{} changed · {}",
                node.chrome.change_count, node.chrome.sync_summary
            ),
            SegRole::Muted,
            IconKind::Workspace,
        )],
    }
}

/// PR counts of the workspace total on the workspace row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrTotals {
    /// PRs whose badge shows on a visible checkout (open and merged), each
    /// counted once.
    pub prs: usize,
    /// Those PRs whose next action is the author's.
    pub need_you: usize,
}

/// The workspace total for `totals`, widest first: `8 PRs · 2 need you`,
/// then `2 need you` alone. `8 PRs` is muted, the `·` dim, and `2 need you`
/// modified and bold. One reads `1 PR` and `1 needs you`. With no PR that
/// needs you only `8 PRs`; with no PR nothing. The count of PRs that need
/// the author is the part worth keeping, so it outlasts the PR count. A
/// `narrow` tree shows only that part: `2 need you`, else `8 PRs`.
fn pr_total_variants(totals: PrTotals, narrow: bool) -> Vec<Vec<TextSeg>> {
    if totals.prs == 0 {
        return Vec::new();
    }
    let noun = if totals.prs == 1 { "PR" } else { "PRs" };
    let prs = text_seg(format!("{} {noun}", totals.prs), SegRole::Muted);
    let verb = if totals.need_you == 1 {
        "needs"
    } else {
        "need"
    };
    let need_you = TextSeg {
        bold: true,
        ..text_seg(format!("{} {verb} you", totals.need_you), SegRole::Modified)
    };
    let dot = TextSeg {
        dim: true,
        ..text_seg(" · ", SegRole::Muted)
    };
    match (totals.need_you > 0, narrow) {
        (false, _) => vec![vec![prs]],
        (true, true) => vec![vec![need_you]],
        (true, false) => vec![vec![prs, dot, need_you.clone()], vec![need_you]],
    }
}

/// Workspace root trailing summary that fits `room` columns (pad included)
/// beside the name: `N changed · sync`, then two spaces and the PR total
/// (`8 PRs · 2 need you`, see [`PrTotals`]). The name is the row's
/// identity, so the summary gives way first. While any PR needs the author,
/// `2 need you` is the part worth keeping: the `8 PRs` part goes first,
/// then the change summary shortens beside `2 need you` (`N changed`, then
/// `N`), then `2 need you` stands alone. After that, and when no PR needs
/// the author, the PR total goes and `N changed · sync` goes to
/// `N changed`, then `N`, then nothing. A `narrow` tree (under
/// [`PR_REF_MIN_TREE_COLS`]) starts at the one-part total.
///
/// Returns the segments and the columns of the PR total that ends them
/// (`0` when the summary that fits shows no PR total), so the caller can
/// record the total's click area without reading it back from the text.
pub(crate) fn workspace_trailing_fit(
    chrome: &NodeChrome,
    totals: PrTotals,
    narrow: bool,
    room: usize,
) -> (Vec<TextSeg>, usize) {
    let n = chrome.change_count;
    let summary = |text: String| vec![icon_text_seg(text, SegRole::Muted, IconKind::Workspace)];
    let summaries = [
        summary(format!("{n} changed · {}", chrome.sync_summary)),
        summary(format!("{n} changed")),
        summary(n.to_string()),
    ];
    let width =
        |segs: &[TextSeg]| -> usize { segs.iter().map(|seg| visible_width(&seg.text)).sum() };
    let beside = |summary: &[TextSeg], total: &[TextSeg]| {
        let mut segs = summary.to_vec();
        segs.push(text_seg(PR_BADGE_GAP, SegRole::Muted));
        segs.extend_from_slice(total);
        (segs, width(total))
    };
    let totals_variants = pr_total_variants(totals, narrow);
    let mut candidates: Vec<(Vec<TextSeg>, usize)> = totals_variants
        .iter()
        .map(|total| beside(&summaries[0], total))
        .collect();
    // With a PR that needs the author the last variant is `2 need you`.
    if let Some(need_you) = totals_variants.last().filter(|_| totals.need_you > 0) {
        candidates.extend(summaries[1..].iter().map(|short| beside(short, need_you)));
        candidates.push((need_you.clone(), width(need_you)));
    }
    candidates
        .into_iter()
        .chain(summaries.map(|summary| (summary, 0)))
        .find(|(segs, _)| width(segs) < room)
        .unwrap_or_default()
}

/// Styled segments for a tree node — the TUI's only label source.
pub fn node_segments(
    node: &TreeNode,
    tree_mode: bool,
    in_no_updates: bool,
    ascii: bool,
) -> NodeSegments {
    match node.kind {
        NodeKind::Workspace => workspace_segments(node, ascii),
        NodeKind::Repo => repo_segments(node, in_no_updates, ascii),
        NodeKind::Checkout => checkout_segments(node, in_no_updates, ascii),
        NodeKind::Group => NodeSegments {
            segments: vec![
                tagged(icon_seg(icon_clean(ascii), SegRole::Muted), IconKind::Clean),
                text_seg("No updates", SegRole::Muted),
            ],
            trailing: vec![icon_text_seg(
                node.children.len().to_string(),
                SegRole::Muted,
                IconKind::Clean,
            )],
        },
        NodeKind::Section => {
            let (glyph, kind) = if node.id.ends_with(":staged") {
                (icon_staged(ascii), IconKind::Staged)
            } else {
                (icon_changes(ascii), IconKind::Changes)
            };
            NodeSegments {
                segments: vec![
                    tagged(icon_seg(glyph, SegRole::Heading), kind),
                    text_seg(node.label.clone(), SegRole::Heading),
                ],
                trailing: Vec::new(),
            }
        }
        NodeKind::Dir => dir_name_segments(&node.label, ascii),
        NodeKind::File => match node.file.as_ref() {
            Some(change) => file_segments(change, tree_mode, ascii),
            None => NodeSegments {
                segments: vec![text_seg(&node.label, SegRole::File)],
                trailing: Vec::new(),
            },
        },
    }
}

/// Prepend `icon_comment` (`"` / nf-fa-comment) when `commented`.
/// Resolved comments use `icon_comment_resolved` (`'` / nf-fa-comment-o)
/// and muted chrome. One space separates the mark from the rest of the
/// run. Uncommented rows keep `trailing` unchanged. Tree
/// and commit-file lists share this so the mark is the same trailing chrome.
pub fn with_comment_mark(
    trailing: Vec<TextSeg>,
    ascii: bool,
    commented: bool,
    resolved: bool,
) -> Vec<TextSeg> {
    if !commented {
        return trailing;
    }
    let (glyph, role, kind) = if resolved {
        (
            icon_comment_resolved(ascii),
            SegRole::Muted,
            IconKind::CommentResolved,
        )
    } else {
        (icon_comment(ascii), SegRole::Heading, IconKind::Comment)
    };
    with_leading_icons(vec![icon_text_seg(glyph, role, kind)], trailing)
}

/// [`painted_row_segments`] with no PR badge.
#[cfg(test)]
pub fn row_segments(
    row: &VisibleRow,
    ascii: bool,
    viewed: bool,
    commented: bool,
    resolved: bool,
) -> NodeSegments {
    painted_row_segments(row, ascii, viewed, commented, resolved, None)
}

/// Prepend the viewed eye (`icon_viewed`, bold [`SegRole::Viewed`]) to
/// `trailing` when `viewed`. Workspace tree and compare file rows share it.
pub fn with_viewed_mark(trailing: Vec<TextSeg>, ascii: bool, viewed: bool) -> Vec<TextSeg> {
    if !viewed {
        return trailing;
    }
    let eye = TextSeg {
        bold: true,
        ..icon_text_seg(icon_viewed(ascii), SegRole::Viewed, IconKind::Viewed)
    };
    with_leading_icons(vec![eye], trailing)
}

/// Glyph and colour role of the PR badge for `state`: the glyph of
/// [`pr_badge_kind`], and the state colour ([`pr_badge_role`] of a fresh
/// badge). The tree, the graph, the header chip, and the popover share this
/// mapping.
pub fn pr_badge_mark(ascii: bool, state: PrState) -> (&'static str, SegRole) {
    (
        pr_badge_kind(state).glyph(ascii),
        pr_badge_role(state, false),
    )
}

/// Colour role a PR mark in `state` paints in: [`SegRole::Muted`] when
/// `stale`, else the state colour. CI failed is [`SegRole::Deleted`], ready
/// to merge [`SegRole::Added`], CI running [`SegRole::Modified`], draft
/// [`SegRole::Muted`], in review [`SegRole::BranchDefault`], and merged
/// [`SegRole::BranchFeature`].
pub fn pr_badge_role(state: PrState, stale: bool) -> SegRole {
    if stale {
        return SegRole::Muted;
    }
    match state {
        PrState::CiFailed => SegRole::Deleted,
        PrState::Ready => SegRole::Added,
        PrState::CiRunning => SegRole::Modified,
        PrState::Draft => SegRole::Muted,
        PrState::InReview => SegRole::BranchDefault,
        PrState::Merged => SegRole::BranchFeature,
    }
}

/// Catalog kind of the PR badge for `state`: one kind per state.
pub fn pr_badge_kind(state: PrState) -> IconKind {
    match state {
        PrState::CiFailed => IconKind::PrCiFailed,
        PrState::Ready => IconKind::PrReady,
        PrState::CiRunning => IconKind::PrCiRunning,
        PrState::Draft => IconKind::PrDraft,
        PrState::InReview => IconKind::PrInReview,
        PrState::Merged => IconKind::PrMerged,
    }
}

/// What one PR mark shows: the state of the PR the branch has, or the
/// failure mark of a lookup that failed with no earlier answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PrMark {
    /// The forge named a PR in this state.
    State(PrState),
    /// The last lookup failed and no earlier answer is known
    /// ([`IconKind::PrLookupFailed`]).
    LookupFailed,
}

impl PrMark {
    /// Catalog kind of this mark: [`pr_badge_kind`] of a state, else
    /// [`IconKind::PrLookupFailed`] (`!` in both glyph modes). Every PR
    /// badge surface (tree, graph worktree row, graph header chip) paints
    /// through this.
    pub fn kind(self) -> IconKind {
        match self {
            Self::State(state) => pr_badge_kind(state),
            Self::LookupFailed => IconKind::PrLookupFailed,
        }
    }

    /// Colour role of this mark: [`pr_badge_role`] of a state, else
    /// [`SegRole::Deleted`]. A failure mark is never stale.
    pub fn role(self, stale: bool) -> SegRole {
        match self {
            Self::State(state) => pr_badge_role(state, stale),
            Self::LookupFailed => SegRole::Deleted,
        }
    }
}

/// Narrowest tree (columns) that paints PR refs. A narrower tree paints the
/// badge glyph alone on every row, so the glyph column stays straight, and
/// one part of the workspace PR total.
pub const PR_REF_MIN_TREE_COLS: usize = 50;

/// Blank columns between the rest of a trailing run and the PR badge (or
/// the workspace PR total).
const PR_BADGE_GAP: &str = "  ";

/// What the PR badge of one tree row paints: `<ref> <glyph>`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrBadge {
    /// Badge mark: picks the glyph and its colour ([`PrMark::kind`],
    /// [`PrMark::role`]).
    pub mark: PrMark,
    /// `#318` (GitHub) or `!12` (GitLab). `None` paints the glyph alone (a
    /// tree narrower than [`PR_REF_MIN_TREE_COLS`], or the failure mark).
    pub ref_text: Option<String>,
    /// The last successful lookup is old: the glyph paints muted too.
    pub stale: bool,
}

/// Checkout path of a tree row that can carry a PR badge.
///
/// A checkout row, or a repo row with one checkout (it paints the branch).
/// A repo row that groups checkouts and every other row give `None`.
pub fn pr_badge_repo(row: &VisibleRow) -> Option<&str> {
    match row.kind {
        NodeKind::Checkout => row.repo.as_deref(),
        NodeKind::Repo if !row.chrome.is_family => row.repo.as_deref(),
        _ => None,
    }
}

/// Append the PR badge `<ref> <glyph>` to the trailing run of a repo or
/// checkout row, two spaces after the rest of the run. The run is
/// right-aligned, so the glyph is the last cell of the row and every
/// badge glyph shares one column.
///
/// The ref paints muted and the glyph bold in its state colour
/// ([`PrMark::role`]); a stale badge paints both muted. A failed lookup
/// with no earlier answer paints the failure mark `!` alone, bold in the
/// deleted colour. Both segments
/// carry the badge's [`IconKind`], so hover, pin, Ctrl+click, and the link
/// cover ref and glyph.
///
/// Returns the index of the glyph in `segs.trailing`, or `None` when
/// `badge` is `None` or the label paints no branch text. Tree rows are
/// built without PR state, so paint adds the badge per frame, before the
/// comment and viewed marks.
pub fn with_pr_badge(
    segs: &mut NodeSegments,
    ascii: bool,
    badge: Option<&PrBadge>,
) -> Option<usize> {
    let badge = badge?;
    let paints_branch = segs
        .segments
        .iter()
        .any(|seg| matches!(seg.role, SegRole::BranchDefault | SegRole::BranchFeature));
    if !paints_branch {
        return None;
    }
    let kind = badge.mark.kind();
    let glyph = kind.glyph(ascii);
    let role = badge.mark.role(badge.stale);
    let trailing = &mut segs.trailing;
    if !trailing.is_empty() {
        trailing.push(text_seg(PR_BADGE_GAP, SegRole::Muted));
    }
    if let Some(ref_text) = &badge.ref_text {
        trailing.push(icon_text_seg(ref_text.clone(), SegRole::Muted, kind));
        trailing.push(text_seg(" ", SegRole::Muted));
    }
    trailing.push(TextSeg {
        bold: true,
        ..icon_text_seg(glyph, role, kind)
    });
    Some(trailing.len() - 1)
}

/// Force-push text of a row too narrow for the whole short text
/// ([`upstream_rewrite_short_text`]).
const UPSTREAM_REWRITE_TINY_TEXT: &str = " force-pushed";

/// Let the force-push text after a diverged mark give way before the
/// row's name.
///
/// The name is the label up to its first segment with no icon: the repo
/// name, or the branch of a checkout row. While the name and the trailing
/// run fit in `room` columns (the row less its edge, indent, and chevron),
/// the text stays whole and the rest of the label (the branch of a repo
/// row) clips as usual. Past that the text drops to ` force-pushed`, then
/// goes. So the run never has to cut the diverged mark off its left end to
/// keep the PR glyph in the last column. The mark stays the click target,
/// and its popover names the whole rewrite.
///
/// Paint and the tree pan limit call this on the same segments, before
/// the spinner and the sparkline join the run, so a fetch that starts does
/// not change the text.
pub fn fit_upstream_rewrite_text(segs: &mut NodeSegments, chrome: &NodeChrome, room: usize) {
    let Some(rewrite) = &chrome.upstream_rewrite else {
        return;
    };
    let full = format!(" {}", upstream_rewrite_short_text(rewrite));
    let Some(at) = segs
        .trailing
        .iter()
        .position(|seg| seg.icon.is_none() && seg.text == full)
    else {
        return;
    };
    let cols = |segs: &[TextSeg]| -> usize { segs.iter().map(|s| visible_width(&s.text)).sum() };
    let name_end = segs
        .segments
        .iter()
        .position(|seg| seg.icon.is_none())
        .map_or(segs.segments.len(), |at| at + 1);
    // The name, the run less the text, and the one-column pad before it.
    let rest = cols(&segs.segments[..name_end]) + cols(&segs.trailing) - visible_width(&full) + 1;
    if rest + visible_width(&full) <= room {
        return;
    }
    if rest + visible_width(UPSTREAM_REWRITE_TINY_TEXT) <= room {
        segs.trailing[at].text = UPSTREAM_REWRITE_TINY_TEXT.to_string();
    } else {
        segs.trailing.remove(at);
    }
}

/// Every segment one tree row paints: the row's segments plus the PR
/// badge, the comment mark, and the viewed eye.
///
/// The viewed eye is prepended on dirty file rows using `icon_viewed` —
/// nf-fa-eye `U+F06E` / `*`. Paint is bold [`SegRole::Viewed`] (teal/cyan),
/// not muted and not the clean check. The comment mark uses `icon_comment`
/// (`"` / nf-fa-comment) on rows that have a live comment, or
/// `icon_comment_resolved` (`'` / nf-fa-comment-o) when every comment on
/// that row is resolved. Both lead the trailing run; the PR badge `pr`
/// ends it ([`with_pr_badge`]).
///
/// Paint, pan, and `gh` read this, so the icons `gh` lists are the icons
/// the row paints, in paint order (label first, then the trailing run).
pub fn painted_row_segments(
    row: &VisibleRow,
    ascii: bool,
    viewed: bool,
    commented: bool,
    resolved: bool,
    pr: Option<&PrBadge>,
) -> NodeSegments {
    let mut segs = NodeSegments {
        segments: row.segments.clone(),
        trailing: row.trailing_segs.clone(),
    };
    if pr_badge_repo(row).is_some() {
        with_pr_badge(&mut segs, ascii, pr);
    }
    let trailing = with_comment_mark(segs.trailing, ascii, commented, resolved);
    NodeSegments {
        segments: segs.segments,
        trailing: with_viewed_mark(trailing, ascii, viewed && row.kind == NodeKind::File),
    }
}

/// Icons of `segs` in paint order: the label, then the trailing run. A
/// kind that tags two segments (the workspace glyph and its summary, the
/// No updates glyph and its count) is listed once.
pub fn segment_icons(segs: &NodeSegments) -> Vec<IconKind> {
    let mut kinds = Vec::new();
    for kind in segs
        .segments
        .iter()
        .chain(&segs.trailing)
        .filter_map(|seg| seg.icon)
    {
        if !kinds.contains(&kind) {
            kinds.push(kind);
        }
    }
    kinds
}

/// Visible snapshot used for the tree: hidden stay out, including
/// linked worktrees of a hidden primary. They return with `.` / `-a`.
pub fn visible_for_tree(snapshot: &WorkspaceSnapshot) -> WorkspaceSnapshot {
    crate::snapshot::visible_workspace_snapshot(snapshot)
}

/// First visible index, centred on `cursor`.
///
/// Shared by the workspace tree, graph, commit-file list, and file-diff
/// focused rows. When the focused row moves, the viewport keeps it as
/// close to the vertical middle as the list length allows.
pub(crate) fn list_viewport_start(row_count: usize, cursor: usize, height: usize) -> usize {
    let view_height = height.max(1);
    let max_start = row_count.saturating_sub(view_height);
    let ideal = cursor.saturating_sub(view_height / 2);
    ideal.min(max_start)
}

/// Cursor for a viewport that follows its cursor (`start_of(cursor)` is the
/// viewport top, never smaller for a later cursor, as
/// [`list_viewport_start`]) to start at `top`: `cursor` itself when its
/// viewport already starts there, else the nearest of `row_count` rows
/// whose viewport does. When no row starts there (a wrapped row taller
/// than the step), the first row whose viewport starts past `top`.
///
/// A scrollbar drag on such a viewport moves the cursor with it, as the
/// wheel does.
pub(crate) fn cursor_for_viewport_top(
    row_count: usize,
    cursor: usize,
    top: usize,
    start_of: impl Fn(usize) -> usize,
) -> usize {
    if row_count == 0 {
        return 0;
    }
    // First row in `0..row_count` where `pred` holds (`pred` is false, then true).
    let first = |pred: &dyn Fn(usize) -> bool| {
        let (mut lo, mut hi) = (0, row_count);
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if pred(mid) {
                hi = mid;
            } else {
                lo = mid + 1;
            }
        }
        lo
    };
    let lo = first(&|c| start_of(c) >= top);
    let past = first(&|c| start_of(c) > top);
    if lo >= past {
        return lo.min(row_count - 1);
    }
    cursor.clamp(lo, past - 1)
}

/// Painted window: `(start, visible_count)`.
pub(crate) fn visible_window(row_count: usize, cursor: usize, height: usize) -> (usize, usize) {
    let view_height = height.max(1);
    let start = list_viewport_start(row_count, cursor, view_height);
    let count = row_count.saturating_sub(start).min(view_height);
    (start, count)
}

#[cfg(test)]
mod tests;

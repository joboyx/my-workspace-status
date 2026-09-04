//! Workspace tree from the same snapshot used by --plain / --json.
//!
//! Row chrome (icons, status letters, sync marks, workspace wording).

use std::collections::{BTreeMap, HashSet};

use crate::helpers::{is_attention_sync_note, is_default_branch, is_detached_head_branch};
use crate::snapshot::{
    CheckoutKind, FileChange, SyncStatus, WorkspaceRepoSnapshot, WorkspaceSnapshot,
};

use super::icons::{
    file_icon, icon_branch, icon_changes, icon_clean, icon_comment, icon_comment_resolved,
    icon_folder, icon_ignored, icon_linked_worktree, icon_repo, icon_staged, icon_viewed,
    icon_workspace, status_letter_from_change, tui_file_badge, tui_merge_mark, tui_sync_mark,
    StatusColorRole,
};

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
    pub ignored: bool,
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
    /// Right-aligned status / sync / counts (no leading pad).
    pub trailing: String,
    /// Left run used by paint (icon + name).
    pub segments: Vec<TextSeg>,
    /// Right run used by paint (badge / sync / counts).
    pub trailing_segs: Vec<TextSeg>,
    pub repo: Option<String>,
    pub primary_repo: Option<String>,
    pub ignored: bool,
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
            ignored: false,
            file: None,
            foldable: false,
            folded: false,
            in_no_updates: false,
            chrome: NodeChrome::default(),
        }
    }
}

/// Build the workspace tree from a snapshot (ignored repos may still be present).
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
            ignored: false,
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
        ignored: false,
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
    let ignored = members.iter().all(|m| m.ignored);
    let change_count = children.iter().map(|c| c.chrome.change_count).sum();
    let statuses: Vec<SyncStatus> = members.iter().map(|m| m.sync_status).collect();
    let worst = worst_sync_status(&statuses);
    let note_for_worst = members
        .iter()
        .find(|m| m.sync_status == worst)
        .map(|m| m.sync_note.clone())
        .unwrap_or_default();
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
        ignored,
        file: None,
        children,
        chrome: NodeChrome {
            path: primary.to_string(),
            sync_status: Some(worst),
            sync_note: note_for_worst,
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
        ignored: repo.ignored,
        file: None,
        children: files,
        chrome: NodeChrome {
            path: repo.repo.clone(),
            branch: repo.branch.clone(),
            checkout_kind: Some(repo.checkout_kind),
            merged_into_default: repo.merged_into_default,
            sync_status: Some(repo.sync_status),
            sync_note: repo.sync_note.clone(),
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
        node = node
            .dirs
            .entry(dir.to_string())
            .or_insert_with(MutableDir::default);
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

fn make_file_node(
    repo: &WorkspaceRepoSnapshot,
    change: &FileChange,
    unstaged_suffix: bool,
) -> TreeNode {
    let id = if unstaged_suffix {
        format!("file:{}:{}#unstaged", repo.repo, change.path)
    } else {
        format!("file:{}:{}", repo.repo, change.path)
    };
    TreeNode {
        id,
        kind: NodeKind::File,
        label: file_display_name(change),
        repo: Some(repo.repo.clone()),
        primary_repo: repo.primary_repo.clone(),
        ignored: repo.ignored,
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
            ignored: repo.ignored,
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
        ignored: repo.ignored,
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
        ignored: node.ignored,
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
    }
}

fn text_seg(text: impl Into<String>, role: SegRole) -> TextSeg {
    TextSeg {
        text: text.into(),
        role,
        hex: None,
        bold: false,
        dim: false,
    }
}

/// True when the clean / no-updates check should paint on a repo or checkout.
pub fn show_clean_check(in_no_updates: bool) -> bool {
    in_no_updates
}

fn sync_trailing(chrome: &NodeChrome, in_no_updates: bool, ascii: bool) -> Vec<TextSeg> {
    let Some(status) = chrome.sync_status else {
        return Vec::new();
    };
    let role = SegRole::from(super::icons::sync_color_role(status));
    if status == SyncStatus::UpToDate {
        if !show_clean_check(in_no_updates) {
            return Vec::new();
        }
        return vec![text_seg(icon_clean(ascii), role)];
    }
    vec![text_seg(
        tui_sync_mark(ascii, status, &chrome.sync_note),
        role,
    )]
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
            icon_seg(icon_folder(ascii), SegRole::Dir),
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

fn file_segments(change: &FileChange, tree_mode: bool, ascii: bool) -> NodeSegments {
    let status = status_letter_from_change(change);
    let color = SegRole::from(status.color_role());
    let icon = file_icon(ascii, &change.path);
    let name = file_display_name(change);
    let dir = {
        let path = change.path.as_str();
        path.strip_suffix(name.as_str())
            .unwrap_or("")
            .trim_end_matches('/')
            .to_string()
    };

    let mut segments = vec![TextSeg {
        text: format!("{} ", icon.glyph),
        role: SegRole::File,
        hex: icon.color,
        bold: false,
        dim: false,
    }];
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
    });
    if !tree_mode && !dir.is_empty() {
        segments.push(TextSeg {
            text: format!("  {dir}"),
            role: SegRole::Muted,
            hex: None,
            bold: false,
            dim: true,
        });
    }

    let trailing = vec![TextSeg {
        text: tui_file_badge(change).to_string(),
        role: color,
        hex: None,
        bold: true,
        dim: false,
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

fn repo_segments(node: &TreeNode, in_no_updates: bool, ascii: bool) -> NodeSegments {
    let name_role = if node.ignored {
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
        let mut segments = vec![
            icon_seg(
                icon_repo(ascii),
                if node.ignored {
                    SegRole::Muted
                } else {
                    SegRole::Heading
                },
            ),
            TextSeg {
                text: node.chrome.path.clone(),
                role: name_role,
                hex: None,
                bold: true,
                dim: false,
            },
        ];
        if node.ignored {
            segments.push(text_seg(
                format!(" {}", icon_ignored(ascii)),
                SegRole::Muted,
            ));
        }
        let mut trailing = sync_trailing(&node.chrome, in_no_updates, ascii);
        let wt_prefix = if trailing.is_empty() { "" } else { "  " };
        trailing.push(text_seg(
            format!("{wt_prefix}{wt_count} wt"),
            SegRole::Muted,
        ));
        if node.chrome.change_count > 0 {
            trailing.push(text_seg(
                format!("  {}", node.chrome.change_count),
                SegRole::Muted,
            ));
        }
        return NodeSegments { segments, trailing };
    }

    let off_default = !is_default_branch(
        &node.chrome.branch,
        node.chrome.default_branch_override.as_deref(),
    );
    let merge = checkout_merge_mark(node, ascii);
    let branch_role = if off_default {
        SegRole::BranchFeature
    } else {
        SegRole::BranchDefault
    };
    // Linked extras only. The primary checkout is a normal repo glyph.
    let linked = node.chrome.checkout_kind == Some(CheckoutKind::Linked);
    let repo_icon = if linked {
        icon_linked_worktree(ascii)
    } else {
        icon_repo(ascii)
    };

    let mut segments = vec![icon_seg(
        repo_icon,
        if node.ignored {
            SegRole::Muted
        } else {
            SegRole::Heading
        },
    )];
    if linked {
        segments.push(TextSeg {
            text: linked_short_name(&node.chrome.path, node.primary_repo.as_deref()),
            role: name_role,
            hex: None,
            bold: true,
            dim: false,
        });
        if let Some(primary) = node.primary_repo.as_deref() {
            segments.push(TextSeg {
                text: format!(" · {primary}"),
                role: SegRole::Muted,
                hex: None,
                bold: false,
                dim: true,
            });
        }
    } else {
        segments.push(TextSeg {
            text: node.chrome.path.clone(),
            role: name_role,
            hex: None,
            bold: true,
            dim: false,
        });
    }
    if node.ignored {
        segments.push(icon_seg(
            &format!(" {}", icon_ignored(ascii)),
            SegRole::Muted,
        ));
    }
    let branch_text = if merge.is_empty() {
        node.chrome.branch.clone()
    } else {
        format!("{} {merge}", node.chrome.branch)
    };
    segments.push(text_seg("  ", SegRole::Muted));
    segments.push(icon_seg(icon_branch(ascii), branch_role));
    segments.push(text_seg(branch_text, branch_role));

    let mut trailing = sync_trailing(&node.chrome, in_no_updates, ascii);
    if node.chrome.change_count > 0 {
        trailing.push(text_seg(
            format!("  {}", node.chrome.change_count),
            SegRole::Muted,
        ));
    }
    NodeSegments { segments, trailing }
}

fn checkout_segments(node: &TreeNode, in_no_updates: bool, ascii: bool) -> NodeSegments {
    let off_default = !is_default_branch(
        &node.chrome.branch,
        node.chrome.default_branch_override.as_deref(),
    );
    let merge = checkout_merge_mark(node, ascii);
    let branch_role = if off_default {
        SegRole::BranchFeature
    } else {
        SegRole::BranchDefault
    };
    // Nested primary uses the branch glyph, never the linked-worktree mark.
    let linked = node.chrome.checkout_kind == Some(CheckoutKind::Linked);
    let row_icon = if linked {
        icon_linked_worktree(ascii)
    } else {
        icon_branch(ascii)
    };
    let main_label = if linked && is_detached_head_branch(&node.chrome.branch) {
        linked_short_name(&node.chrome.path, node.primary_repo.as_deref())
    } else {
        node.chrome.branch.clone()
    };
    let branch_text = if merge.is_empty() {
        main_label
    } else {
        format!("{main_label} {merge}")
    };
    let segments = vec![
        icon_seg(row_icon, SegRole::Heading),
        TextSeg {
            text: branch_text,
            role: branch_role,
            hex: None,
            bold: true,
            dim: false,
        },
    ];
    let mut trailing = sync_trailing(&node.chrome, in_no_updates, ascii);
    if node.chrome.change_count > 0 {
        trailing.push(text_seg(
            format!("  {}", node.chrome.change_count),
            SegRole::Muted,
        ));
    }
    NodeSegments { segments, trailing }
}

fn workspace_segments(node: &TreeNode, ascii: bool) -> NodeSegments {
    NodeSegments {
        segments: vec![
            icon_seg(icon_workspace(ascii), SegRole::Heading),
            TextSeg {
                text: node.label.clone(),
                role: SegRole::Heading,
                hex: None,
                bold: true,
                dim: false,
            },
        ],
        trailing: vec![text_seg(
            format!(
                "{} changed · {}",
                node.chrome.change_count, node.chrome.sync_summary
            ),
            SegRole::Muted,
        )],
    }
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
                icon_seg(icon_clean(ascii), SegRole::Muted),
                text_seg("No updates", SegRole::Muted),
            ],
            trailing: vec![text_seg(node.children.len().to_string(), SegRole::Muted)],
        },
        NodeKind::Section => {
            let glyph = if node.id.ends_with(":staged") {
                icon_staged(ascii)
            } else {
                icon_changes(ascii)
            };
            NodeSegments {
                segments: vec![
                    icon_seg(glyph, SegRole::Heading),
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
/// and muted chrome. Uncommented rows keep `trailing` unchanged. Tree
/// and commit-file lists share this so the mark is the same trailing chrome.
pub fn with_comment_mark(
    mut trailing: Vec<TextSeg>,
    ascii: bool,
    commented: bool,
    resolved: bool,
) -> Vec<TextSeg> {
    if !commented {
        return trailing;
    }
    let (glyph, role) = if resolved {
        (icon_comment_resolved(ascii), SegRole::Muted)
    } else {
        (icon_comment(ascii), SegRole::Heading)
    };
    let mut marked = vec![
        TextSeg {
            text: glyph.to_string(),
            role,
            hex: None,
            bold: false,
            dim: false,
        },
        text_seg(" ", SegRole::Muted),
    ];
    marked.append(&mut trailing);
    marked
}

/// Segments for a flattened row. Viewed eye is prepended on dirty file rows
/// using `icon_viewed` — nf-fa-eye `U+F06E` / `*`. Paint is bold
/// [`SegRole::Viewed`] (teal/cyan), not muted and not the clean check.
/// Comment mark uses `icon_comment` (`"` / nf-fa-comment) on rows that have
/// a live comment, or `icon_comment_resolved` (`'` / nf-fa-comment-o) when
/// every comment on that row is resolved.
pub fn row_segments(
    row: &VisibleRow,
    ascii: bool,
    viewed: bool,
    commented: bool,
    resolved: bool,
) -> NodeSegments {
    let mut trailing = with_comment_mark(row.trailing_segs.clone(), ascii, commented, resolved);
    if viewed && row.kind == NodeKind::File {
        let mut marked = vec![
            TextSeg {
                text: icon_viewed(ascii).to_string(),
                role: SegRole::Viewed,
                hex: None,
                bold: true,
                dim: false,
            },
            text_seg(" ", SegRole::Muted),
        ];
        marked.append(&mut trailing);
        trailing = marked;
    }
    NodeSegments {
        segments: row.segments.clone(),
        trailing,
    }
}

/// Visible snapshot used for the tree: hidden ignored stay out, including
/// linked worktrees of an ignored primary. They return with `.` / `-a`.
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

/// Painted window: `(start, visible_count)`.
pub(crate) fn visible_window(row_count: usize, cursor: usize, height: usize) -> (usize, usize) {
    let view_height = height.max(1);
    let start = list_viewport_start(row_count, cursor, view_height);
    let count = row_count.saturating_sub(start).min(view_height);
    (start, count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };

    fn repo(name: &str, ignored_dirty: bool, linked: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: ignored_dirty,
            has_staged: false,
            has_untracked: false,
            changes: if ignored_dirty {
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
            checkout_kind: if linked {
                CheckoutKind::Linked
            } else {
                CheckoutKind::Primary
            },
            primary_repo: if linked { Some("app".into()) } else { None },
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
        }
    }

    #[test]
    fn hidden_ignored_omitted_from_visible_tree() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("notes", true, false)],
            &["notes".into()],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true, "workspace");
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().any(|r| r.label.contains("app")));
        assert!(rows.iter().all(|r| !r.label.contains("notes")));
    }

    #[test]
    fn show_ignored_includes_notes() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("notes", true, false)],
            &["notes".into()],
            true,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true, "workspace");
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().any(|r| r.label.contains("notes")));
    }

    fn ignored_primary_and_linked_child() -> (RepoSnapshot, RepoSnapshot) {
        let primary = repo("app", true, false);
        let mut linked = repo("app/.worktrees/feat", true, true);
        linked.branch = "feature/linked-open".into();
        (primary, linked)
    }

    fn tree_rows(snapshot: &crate::snapshot::WorkspaceSnapshot) -> Vec<VisibleRow> {
        let tree = build_tree(&visible_for_tree(snapshot), true, "workspace");
        flatten_with(&tree, &HashSet::new(), true)
    }

    #[test]
    fn hidden_ignored_primary_omits_linked_child_from_visible_tree() {
        let (primary, linked) = ignored_primary_and_linked_child();
        let built = build_workspace_snapshot(&[primary, linked], &["app".into()], false, &[]);
        assert!(
            built.repos.iter().any(|r| r.repo == "app" && r.ignored),
            "primary stays tagged ignored"
        );
        assert!(
            built
                .repos
                .iter()
                .any(|r| r.repo == "app/.worktrees/feat" && !r.ignored),
            "linked child must not be retagged ignored"
        );
        let rows = tree_rows(&built);
        assert!(rows.iter().all(|r| r.id != "repo:app"));
        assert!(rows.iter().all(|r| r.id != "checkout:app"));
        assert!(
            rows.iter().all(|r| r.id != "repo:app/.worktrees/feat"),
            "linked child must not paint as a workspace-root orphan, got {:?}",
            rows.iter().map(|r| r.id.as_str()).collect::<Vec<_>>()
        );
        assert!(rows.iter().all(|r| r.id != "checkout:app/.worktrees/feat"));
        assert!(rows
            .iter()
            .all(|r| !r.label.contains("feature/linked-open")));
        assert!(rows.iter().all(|r| !r.label.contains("app/.worktrees")));
    }

    #[test]
    fn show_ignored_nests_linked_child_of_ignored_primary() {
        let (primary, linked) = ignored_primary_and_linked_child();
        let built = build_workspace_snapshot(&[primary, linked], &["app".into()], true, &[]);
        let rows = tree_rows(&built);
        let family = rows.iter().find(|r| r.id == "repo:app").expect("family");
        assert!(family.chrome.is_family);
        assert!(family.trailing.contains("2 wt"), "{}", family.trailing);
        let checkout = rows
            .iter()
            .find(|r| r.id == "checkout:app/.worktrees/feat")
            .expect("linked checkout");
        assert_eq!(checkout.kind, NodeKind::Checkout);
        assert!(checkout.label.contains("feature/linked-open"));
        assert!(
            checkout.label.contains('L'),
            "linked row uses ASCII L, got {}",
            checkout.label
        );
        assert!(
            rows.iter().all(|r| r.id != "repo:app/.worktrees/feat"),
            "shown family must not paint a workspace-root orphan"
        );
    }

    #[test]
    fn standalone_ignored_linked_path_omitted_until_shown() {
        let primary = repo("app", true, false);
        let mut linked = repo("app/.worktrees/feat", true, true);
        linked.branch = "feature/linked-open".into();
        let hidden = build_workspace_snapshot(
            &[primary.clone(), linked.clone()],
            &["app/.worktrees/feat".into()],
            false,
            &[],
        );
        let hidden_rows = tree_rows(&hidden);
        assert!(hidden_rows.iter().any(|r| r.id == "repo:app"));
        assert!(hidden_rows
            .iter()
            .all(|r| r.id != "checkout:app/.worktrees/feat"));
        assert!(hidden_rows
            .iter()
            .all(|r| r.id != "repo:app/.worktrees/feat"));
        assert!(hidden_rows
            .iter()
            .all(|r| !r.label.contains("feature/linked-open")));

        let shown = build_workspace_snapshot(
            &[primary, linked],
            &["app/.worktrees/feat".into()],
            true,
            &[],
        );
        let shown_rows = tree_rows(&shown);
        assert!(shown_rows.iter().any(|r| r.id == "repo:app"));
        assert!(shown_rows
            .iter()
            .any(|r| r.id == "checkout:app/.worktrees/feat"));
        assert!(shown_rows
            .iter()
            .all(|r| r.id != "repo:app/.worktrees/feat"));
    }

    #[test]
    fn no_updates_group_starts_folded() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("lib", false, false)],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true, "workspace");
        let folds = default_folds(&tree);
        assert!(folds.contains("group:no-updates"));
        let rows = flatten(&tree, &folds);
        assert!(rows.iter().any(|r| r.id == "group:no-updates" && r.folded));
        assert!(rows
            .iter()
            .all(|r| !r.label.contains("lib  main") || r.id == "group:no-updates"));
    }

    fn dirty_repo(name: &str, paths: &[&str]) -> RepoSnapshot {
        let mut snap = repo(name, true, false);
        snap.has_untracked = paths
            .iter()
            .any(|p| *p != "README.md" && *p != "src/lib.rs");
        snap.changes = paths
            .iter()
            .map(|path| FileChange {
                path: (*path).into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            })
            .collect();
        snap
    }

    #[test]
    fn tree_mode_inserts_dir_and_basename_file() {
        let built = build_workspace_snapshot(
            &[dirty_repo("app", &["src/lib.rs", "README.md"])],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true, "workspace");
        let rows = flatten(&tree, &HashSet::new());
        let dir = rows
            .iter()
            .find(|r| r.id == "dir:app:src")
            .expect("dir:app:src");
        assert_eq!(dir.kind, NodeKind::Dir);
        assert!(dir.foldable);
        assert!(dir.label.contains("src"));
        assert!(!dir.label.contains("lib.rs"));
        let lib = rows
            .iter()
            .find(|r| r.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert_eq!(lib.kind, NodeKind::File);
        assert!(lib.label.contains("lib.rs"));
        assert!(!lib.label.contains("src/lib.rs"));
        let readme = rows
            .iter()
            .find(|r| r.id == "file:app:README.md")
            .expect("README.md");
        assert!(readme.label.contains("README.md"));
        let dir_idx = rows.iter().position(|r| r.id == "dir:app:src").unwrap();
        let readme_idx = rows
            .iter()
            .position(|r| r.id == "file:app:README.md")
            .unwrap();
        let repo_idx = rows.iter().position(|r| r.id == "repo:app").unwrap();
        assert!(dir_idx < readme_idx);
        assert_eq!(rows[dir_idx + 1].id, "file:app:src/lib.rs");
        assert!(repo_idx < dir_idx);
    }

    #[test]
    fn flat_mode_is_full_paths_without_dir_rows() {
        let built = build_workspace_snapshot(
            &[dirty_repo("app", &["src/lib.rs", "README.md"])],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), false, "workspace");
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().all(|r| r.kind != NodeKind::Dir));
        let lib = rows
            .iter()
            .find(|r| r.id == "file:app:src/lib.rs")
            .expect("lib.rs");
        assert!(lib.label.contains("lib.rs"));
        assert!(lib.label.contains("src"));
        assert!(lib.trailing.contains("M"));
        assert!(!lib.label.contains("src/lib.rs"));
        assert!(rows.iter().any(|r| r.id == "file:app:README.md"));
    }

    #[test]
    fn collapse_single_child_dirs() {
        let built =
            build_workspace_snapshot(&[dirty_repo("app", &["src/foo/bar.rs"])], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "workspace");
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows
            .iter()
            .any(|r| r.id == "dir:app:src/foo" && r.label.contains("src/foo")));
        assert!(rows.iter().all(|r| r.id != "dir:app:src"));
        let file = rows
            .iter()
            .find(|r| r.id == "file:app:src/foo/bar.rs")
            .expect("bar.rs");
        assert!(file.label.contains("bar.rs"));
        assert!(!file.label.contains("src/foo/bar.rs"));
    }

    #[test]
    fn workspace_header_is_changed_files_not_dirty_repos() {
        let built = build_workspace_snapshot(
            &[
                dirty_repo("app", &["a.rs", "b.rs"]),
                repo("lib", false, false),
            ],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true, "demo");
        assert_eq!(tree.label, "demo");
        assert_eq!(tree.chrome.change_count, 2);
        assert_eq!(tree.chrome.sync_summary, "all current");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let ws = rows
            .iter()
            .find(|r| r.id == "workspace")
            .expect("workspace");
        assert!(ws.label.contains("demo"));
        assert!(ws.trailing.contains("2 changed · all current"));
        assert!(!ws.label.contains("dirty"));
        assert!(!ws.trailing.contains("dirty"));
    }

    #[test]
    fn status_letters_and_badge_are_trailing() {
        let mut snap = dirty_repo("app", &[]);
        snap.has_untracked = true;
        snap.has_staged = true;
        snap.has_unstaged = true;
        snap.changes = vec![
            FileChange {
                path: "new.ts".into(),
                staged_status: None,
                unstaged_status: None,
                untracked: true,
                old_path: None,
            },
            FileChange {
                path: "staged.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: None,
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "both.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            },
        ];
        let built = build_workspace_snapshot(&[snap], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let new = rows.iter().find(|r| r.id == "file:app:new.ts").expect("A");
        let staged = rows
            .iter()
            .find(|r| r.id == "file:app:staged.rs")
            .expect("S");
        let both_staged = rows
            .iter()
            .find(|r| r.id == "file:app:both.rs")
            .expect("staged both.rs");
        let both_unstaged = rows
            .iter()
            .find(|r| r.id == "file:app:both.rs#unstaged")
            .expect("unstaged both.rs");
        assert_eq!(new.trailing, "A ");
        assert_eq!(staged.trailing, "S ");
        assert_eq!(both_staged.trailing, "S ");
        assert_eq!(both_unstaged.trailing, "M ");
        assert_ne!(both_staged.id, both_unstaged.id);
        assert!(!both_staged.trailing.contains("MS"));
        assert!(!both_unstaged.trailing.contains("MS"));
        assert!(!new.label.contains('?'));
        assert!(!both_staged.label.contains("M+"));
        assert!(new.label.starts_with('·') || new.label.contains("new.ts"));
    }

    #[test]
    fn linked_checkout_is_labeled_by_branch_not_wt_path() {
        let mut primary = dirty_repo("app", &["src/a.ts"]);
        primary.branch = "main".into();
        let mut linked = repo("app/.worktrees/feat", true, true);
        linked.branch = "feature/login-page".into();
        linked.merged_into_default = Some(false);
        linked.changes = vec![FileChange {
            path: "src/b.ts".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let family = rows.iter().find(|r| r.id == "repo:app").expect("family");
        assert!(family.chrome.is_family);
        assert!(family.trailing.contains("2 wt"));
        let checkout = rows
            .iter()
            .find(|r| r.id == "checkout:app/.worktrees/feat")
            .expect("linked checkout");
        assert_eq!(checkout.kind, NodeKind::Checkout);
        assert!(checkout.label.contains("feature/login-page"));
        assert!(checkout.label.contains('L') || checkout.label.contains("feature/"));
        assert!(checkout.label.contains('o'), "open-vs-default ascii mark");
        assert!(!checkout.label.contains("wt "));
        assert!(
            !checkout.label.contains("app/.worktrees/feat") || checkout.label.contains("feature/")
        );
        let primary_row = rows
            .iter()
            .find(|r| r.id == "checkout:app")
            .expect("primary checkout");
        assert!(primary_row.label.contains("main"));
        assert!(primary_row.label.contains('&') || primary_row.label.contains("main"));
        assert_ne!(
            primary_row.segments.first().map(|s| s.text.trim()),
            Some(icon_linked_worktree(true)),
            "primary checkout must not use the linked-worktree glyph"
        );
        assert_eq!(
            checkout.segments.first().map(|s| s.text.trim()),
            Some(icon_linked_worktree(true)),
            "linked extra keeps the worktree glyph"
        );
        assert_eq!(
            family.segments.first().map(|s| s.text.trim()),
            Some(icon_repo(true)),
        );
        assert_eq!(
            primary_row.segments.first().map(|s| s.text.trim()),
            Some(icon_branch(true)),
        );
        assert!(
            !primary_row.label.contains('o'),
            "primary checkout omits the open-vs-default mark, got {}",
            primary_row.label
        );
    }

    #[test]
    fn primary_checkout_omits_open_vs_default_mark() {
        let mut primary = dirty_repo("app", &["src/a.ts"]);
        primary.branch = "feature/auth-refresh".into();
        primary.merged_into_default = Some(false);
        let mut linked = repo("app/.worktrees/feat", true, true);
        linked.branch = "feature/side-leaf".into();
        linked.merged_into_default = Some(false);
        let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let primary_row = rows
            .iter()
            .find(|r| r.id == "checkout:app")
            .expect("primary checkout");
        let linked_row = rows
            .iter()
            .find(|r| r.id == "checkout:app/.worktrees/feat")
            .expect("linked checkout");
        assert!(
            !primary_row.label.contains('o'),
            "primary must not paint open-vs-default, got {}",
            primary_row.label
        );
        assert!(
            linked_row.label.contains('o'),
            "linked extra keeps open-vs-default, got {}",
            linked_row.label
        );
    }

    #[test]
    fn nested_primary_merged_into_default_paints_check() {
        let mut primary = dirty_repo("app", &["src/a.ts"]);
        primary.branch = "feature/auth-landed".into();
        primary.merged_into_default = Some(true);
        let mut linked = repo("app/.worktrees/feat", true, true);
        linked.branch = "feature/side-landed".into();
        linked.merged_into_default = Some(true);
        let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let primary_row = rows
            .iter()
            .find(|r| r.id == "checkout:app")
            .expect("primary checkout");
        let linked_row = rows
            .iter()
            .find(|r| r.id == "checkout:app/.worktrees/feat")
            .expect("linked checkout");
        assert!(
            primary_row.label.contains('M'),
            "primary merged into default must paint the check, got {}",
            primary_row.label
        );
        assert!(
            !primary_row.label.contains('o'),
            "primary must not paint open-vs-default, got {}",
            primary_row.label
        );
        assert!(
            linked_row.label.contains('M'),
            "linked extra keeps the merged check, got {}",
            linked_row.label
        );
    }

    #[test]
    fn flat_primary_repo_merged_into_default_paints_check() {
        let mut app = repo("app", true, false);
        app.branch = "feature/auth-landed".into();
        app.merged_into_default = Some(true);
        let built = build_workspace_snapshot(&[app], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let app_row = rows.iter().find(|r| r.id == "repo:app").expect("repo:app");
        assert_eq!(app_row.kind, NodeKind::Repo);
        assert!(
            app_row.label.contains('M'),
            "flat primary merged into default must paint the check, got {}",
            app_row.label
        );
        assert!(
            !app_row.label.contains('o'),
            "flat primary must not paint open-vs-default, got {}",
            app_row.label
        );
    }

    #[test]
    fn flat_primary_repo_uses_repo_glyph_not_worktree() {
        let built = build_workspace_snapshot(&[repo("app", true, false)], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let app = rows.iter().find(|r| r.id == "repo:app").expect("repo:app");
        assert_eq!(
            app.segments.first().map(|s| s.text.trim()),
            Some(icon_repo(true))
        );
        assert_ne!(
            app.segments.first().map(|s| s.text.trim()),
            Some(icon_linked_worktree(true))
        );
    }

    #[test]
    fn sync_marks_on_repo_rows() {
        let mut ahead = repo("lib", false, false);
        ahead.sync_status = SyncStatus::Ahead;
        ahead.sync_note = "ahead by 2".into();
        ahead.branch = "main".into();
        let mut diverged = repo("ops", false, false);
        diverged.sync_status = SyncStatus::Diverged;
        diverged.sync_note = "ahead 1, behind 1".into();
        let built = build_workspace_snapshot(&[ahead, diverged], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        assert!(tree.chrome.sync_summary.contains("1 ahead"));
        assert!(tree.chrome.sync_summary.contains("1 diverged"));
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let lib = rows.iter().find(|r| r.id == "repo:lib").expect("lib");
        assert!(
            lib.trailing.contains('^') && lib.trailing.contains('2'),
            "{}",
            lib.trailing
        );
        let ops = rows.iter().find(|r| r.id == "repo:ops").expect("ops");
        assert!(ops.trailing.contains('Y'), "{}", ops.trailing);
        assert!(!ops.label.contains("wt "));
    }

    #[test]
    fn linked_only_does_not_invent_family_container() {
        let linked = repo("app/.worktrees/feat", true, true);
        let built = build_workspace_snapshot(&[linked], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        assert!(rows.iter().any(|r| r.id == "repo:app/.worktrees/feat"));
        assert!(rows.iter().all(|r| r.id != "repo:app"));
        assert!(rows.iter().all(|r| r.kind != NodeKind::Checkout));
        let row = rows
            .iter()
            .find(|r| r.id == "repo:app/.worktrees/feat")
            .expect("linked-only repo");
        assert!(!row.chrome.is_family);
        assert!(row.label.contains("main"));
        assert!(!row.label.contains("wt "));
    }

    #[test]
    fn ignored_uses_icon_not_bracket_text() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("notes", true, false)],
            &["notes".into()],
            true,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let notes = rows.iter().find(|r| r.id == "repo:notes").expect("notes");
        assert!(notes.label.contains(icon_ignored(true)), "{}", notes.label);
        assert!(!notes.label.contains("[ignored]"));
    }

    #[test]
    fn no_updates_count_is_trailing_not_parens() {
        let built = build_workspace_snapshot(
            &[repo("app", true, false), repo("lib", false, false)],
            &[],
            false,
            &[],
        );
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let folds = default_folds(&tree);
        let rows = flatten_with(&tree, &folds, true);
        let group = rows
            .iter()
            .find(|r| r.id == "group:no-updates")
            .expect("group");
        assert_eq!(group.trailing.trim(), "1");
        assert!(group.label.contains("No updates"));
        assert!(!group.label.contains("(1)"));
        assert!(!group.trailing.contains('('));
    }

    #[test]
    fn viewed_glyph_is_eye_not_circle() {
        let built = build_workspace_snapshot(&[dirty_repo("app", &["README.md"])], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let file = rows
            .iter()
            .find(|r| r.id == "file:app:README.md")
            .expect("file");
        let segs = row_segments(file, true, true, false, false);
        let trail: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(icon_viewed(true), "*");
        assert_eq!(icon_viewed(false), "\u{f06e}");
        assert!(trail.contains(icon_viewed(true)), "{trail}");
        assert!(!trail.contains('\u{25c9}'), "{trail}");
        let nerd = row_segments(file, false, true, false, false);
        let nerd_trail: String = nerd.trailing.iter().map(|s| s.text.as_str()).collect();
        assert!(nerd_trail.contains('\u{f06e}'), "{nerd_trail}");
        assert!(!nerd_trail.contains('\u{25c9}'), "{nerd_trail}");
        assert!(!nerd_trail.contains('\u{f07a}'), "{nerd_trail}");
        assert_eq!(
            nerd.trailing
                .iter()
                .filter(|s| s.text == icon_viewed(false))
                .count(),
            1,
            "{nerd_trail}"
        );
        let eye = nerd
            .trailing
            .iter()
            .find(|s| s.text == icon_viewed(false))
            .expect("eye");
        assert_eq!(eye.role, SegRole::Viewed);
        assert!(eye.bold);
        assert!(!eye.dim);
    }

    #[test]
    fn comment_glyph_is_quote_not_eye() {
        let built = build_workspace_snapshot(&[dirty_repo("app", &["README.md"])], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let file = rows
            .iter()
            .find(|r| r.id == "file:app:README.md")
            .expect("file");
        let segs = row_segments(file, true, false, true, false);
        let trail: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
        assert_eq!(icon_comment(true), "\"");
        assert!(trail.contains('"'), "{trail}");
        assert!(!trail.contains('*'), "{trail}");
        let unmarked = row_segments(file, true, false, false, false);
        let unmarked_trail: String = unmarked.trailing.iter().map(|s| s.text.as_str()).collect();
        assert!(!unmarked_trail.contains('"'), "{unmarked_trail}");
        let nerd = row_segments(file, false, false, true, false);
        let nerd_trail: String = nerd.trailing.iter().map(|s| s.text.as_str()).collect();
        assert!(nerd_trail.contains('\u{f075}'), "{nerd_trail}");
        let resolved = row_segments(file, true, false, true, true);
        let resolved_trail: String = resolved.trailing.iter().map(|s| s.text.as_str()).collect();
        assert!(resolved_trail.contains('\''), "{resolved_trail}");
        assert!(!resolved_trail.contains('"'), "{resolved_trail}");
    }

    #[test]
    fn viewport_centres_on_cursor() {
        assert_eq!(visible_window(100, 50, 10), (45, 10));
        assert_eq!(visible_window(8, 7, 20), (0, 8));
        assert_eq!(visible_window(40, 39, 2), (38, 2));
    }

    fn fc(path: &str, staged: Option<&str>, unstaged: Option<&str>, untracked: bool) -> FileChange {
        FileChange {
            path: path.into(),
            staged_status: staged.map(str::to_string),
            unstaged_status: unstaged.map(str::to_string),
            untracked,
            old_path: None,
        }
    }

    fn repo_with_changes(name: &str, changes: Vec<FileChange>) -> RepoSnapshot {
        let mut snap = repo(name, true, false);
        snap.has_staged = changes.iter().any(|c| c.staged_status.is_some());
        snap.has_unstaged = changes.iter().any(|c| c.unstaged_status.is_some());
        snap.has_untracked = changes.iter().any(|c| c.untracked);
        snap.changes = changes;
        snap
    }

    fn built_tree(changes: Vec<FileChange>, tree_mode: bool) -> TreeNode {
        let built = build_workspace_snapshot(&[repo_with_changes("app", changes)], &[], false, &[]);
        build_tree(&visible_for_tree(&built), tree_mode, "ws")
    }

    #[test]
    fn unstaged_only_keeps_flat_file_tree_without_sections() {
        let tree = built_tree(
            vec![
                fc("README.md", None, Some("M"), false),
                fc("src/lib.rs", None, Some("M"), false),
                fc("new.ts", None, None, true),
            ],
            true,
        );
        let repo_node = find_node(&tree, "repo:app").expect("repo:app");
        assert!(
            repo_node
                .children
                .iter()
                .all(|c| !c.id.starts_with("section:")),
            "no staged paths must not insert section chrome, got {:?}",
            repo_node
                .children
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>()
        );
        assert!(find_node(&tree, "section:app:staged").is_none());
        assert!(find_node(&tree, "section:app:changes").is_none());
        assert!(find_node(&tree, "dir:app:src").is_some());
        assert!(find_node(&tree, "file:app:src/lib.rs").is_some());
        assert!(find_node(&tree, "file:app:README.md").is_some());
        assert!(find_node(&tree, "file:app:new.ts").is_some());
        let rows = flatten_with(&tree, &HashSet::new(), true);
        assert!(rows.iter().all(|r| !r.id.starts_with("section:")));
        assert!(rows.iter().all(|r| !r.id.contains("#unstaged")));
    }

    #[test]
    fn staged_paths_split_checkout_into_staged_then_changes_trees() {
        let tree = built_tree(
            vec![
                fc("src/a.rs", Some("M"), None, false),
                fc("src/b.rs", None, Some("M"), false),
                fc("staged.rs", Some("A"), None, false),
                fc("new.ts", None, None, true),
            ],
            true,
        );
        let repo_node = find_node(&tree, "repo:app").expect("repo:app");
        assert_eq!(
            repo_node
                .children
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["section:app:staged", "section:app:changes"]
        );
        assert_eq!(repo_node.children[0].label, "Staged");
        assert_eq!(repo_node.children[1].label, "Changes");
        assert_eq!(repo_node.children[0].kind, NodeKind::Section);
        assert_eq!(repo_node.children[1].kind, NodeKind::Section);
        assert_eq!(repo_node.children[0].repo.as_deref(), Some("app"));
        assert!(repo_node.children[0].file.is_none());

        let staged = find_node(&tree, "section:app:staged").expect("staged");
        assert!(find_node(staged, "dir:app:src").is_some());
        assert!(find_node(staged, "file:app:src/a.rs").is_some());
        assert!(find_node(staged, "file:app:staged.rs").is_some());
        assert!(find_node(staged, "file:app:src/b.rs").is_none());
        assert!(find_node(staged, "file:app:new.ts").is_none());

        let changes = find_node(&tree, "section:app:changes").expect("changes");
        assert!(find_node(changes, "dir:app:src#unstaged").is_some());
        assert!(find_node(changes, "file:app:src/b.rs").is_some());
        assert!(find_node(changes, "file:app:new.ts").is_some());
        assert!(find_node(changes, "file:app:src/a.rs").is_none());
        assert!(find_node(changes, "file:app:staged.rs").is_none());
        assert!(find_node(changes, "dir:app:src").is_none());

        let rows = flatten_with(&tree, &HashSet::new(), true);
        let staged_file = rows
            .iter()
            .find(|r| r.id == "file:app:src/a.rs")
            .expect("staged a.rs");
        assert!(!staged_file.in_no_updates);
        assert_eq!(staged_file.trailing, "S ");
        let added = rows
            .iter()
            .find(|r| r.id == "file:app:staged.rs")
            .expect("staged add");
        assert_eq!(added.trailing, "A ");
        let untracked = rows
            .iter()
            .find(|r| r.id == "file:app:new.ts")
            .expect("untracked");
        assert_eq!(untracked.trailing, "A ");
    }

    #[test]
    fn hide_empty_section_when_all_staged() {
        let tree = built_tree(vec![fc("only.rs", Some("M"), None, false)], true);
        let repo_node = find_node(&tree, "repo:app").expect("repo:app");
        assert_eq!(
            repo_node
                .children
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["section:app:staged"]
        );
        assert!(find_node(&tree, "section:app:changes").is_none());
        assert!(find_node(&tree, "file:app:only.rs").is_some());
    }

    #[test]
    fn ms_dual_rows_split_letters_and_ids() {
        let tree = built_tree(vec![fc("both.rs", Some("M"), Some("M"), false)], true);
        let staged = find_node(&tree, "section:app:staged").expect("staged");
        let changes = find_node(&tree, "section:app:changes").expect("changes");
        let staged_file = find_node(staged, "file:app:both.rs").expect("staged file");
        let changes_file = find_node(changes, "file:app:both.rs#unstaged").expect("changes file");
        assert_eq!(staged_file.id, "file:app:both.rs");
        assert_eq!(changes_file.id, "file:app:both.rs#unstaged");
        assert_ne!(staged_file.id, changes_file.id);
        let staged_change = staged_file.file.as_ref().expect("staged FileChange");
        let changes_change = changes_file.file.as_ref().expect("changes FileChange");
        assert_eq!(staged_change.staged_status.as_deref(), Some("M"));
        assert!(staged_change.unstaged_status.is_none());
        assert!(!staged_change.untracked);
        assert!(changes_change.staged_status.is_none());
        assert_eq!(changes_change.unstaged_status.as_deref(), Some("M"));
        let rows = flatten_with(&tree, &HashSet::new(), true);
        assert_eq!(
            rows.iter()
                .find(|r| r.id == "file:app:both.rs")
                .expect("staged row")
                .trailing,
            "S "
        );
        assert_eq!(
            rows.iter()
                .find(|r| r.id == "file:app:both.rs#unstaged")
                .expect("changes row")
                .trailing,
            "M "
        );
    }

    #[test]
    fn section_glyphs_are_ascii_hash_tilde_and_nerd_package_pencil() {
        let tree = built_tree(
            vec![
                fc("staged.rs", Some("M"), None, false),
                fc("dirty.rs", None, Some("M"), false),
            ],
            true,
        );
        let ascii = flatten_with(&tree, &HashSet::new(), true);
        let nerd = flatten_with(&tree, &HashSet::new(), false);
        let ascii_staged = ascii
            .iter()
            .find(|r| r.id == "section:app:staged")
            .expect("ascii staged");
        let ascii_changes = ascii
            .iter()
            .find(|r| r.id == "section:app:changes")
            .expect("ascii changes");
        assert!(
            ascii_staged.label.contains("Staged"),
            "{}",
            ascii_staged.label
        );
        assert!(
            ascii_changes.label.contains("Changes"),
            "{}",
            ascii_changes.label
        );
        assert!(
            ascii_staged.label.contains('#'),
            "ASCII staged glyph, got {}",
            ascii_staged.label
        );
        assert!(
            ascii_changes.label.contains('~'),
            "ASCII changes glyph, got {}",
            ascii_changes.label
        );
        let nerd_staged = nerd
            .iter()
            .find(|r| r.id == "section:app:staged")
            .expect("nerd staged");
        let nerd_changes = nerd
            .iter()
            .find(|r| r.id == "section:app:changes")
            .expect("nerd changes");
        assert!(
            nerd_staged.label.contains('\u{f487}'),
            "nerd staged nf-oct-package, got {}",
            nerd_staged.label
        );
        assert!(
            nerd_changes.label.contains('\u{f040}'),
            "nerd changes nf-fa-pencil, got {}",
            nerd_changes.label
        );
        assert_eq!(crate::helpers::visible_width("\u{f487}"), 1);
        assert_eq!(crate::helpers::visible_width("\u{f040}"), 1);
        assert_eq!(ascii_staged.segments[0].role, SegRole::Heading);
        assert_eq!(ascii_changes.segments[0].role, SegRole::Heading);
    }

    #[test]
    fn dir_path_from_id_strips_unstaged_suffix() {
        assert_eq!(
            dir_path_from_id("dir:app:src", "app").as_deref(),
            Some("src")
        );
        assert_eq!(
            dir_path_from_id("dir:app:src#unstaged", "app").as_deref(),
            Some("src")
        );
        assert_eq!(
            dir_path_from_id("dir:app:src/foo#unstaged", "app").as_deref(),
            Some("src/foo")
        );
    }

    #[test]
    fn changes_dirs_keep_unstaged_suffix_when_collapse_names_differ() {
        let tree = built_tree(
            vec![
                fc("src/lib.rs", None, Some("M"), false),
                fc("src/deep/mod.rs", Some("M"), None, false),
            ],
            true,
        );
        let staged = find_node(&tree, "section:app:staged").expect("staged");
        let changes = find_node(&tree, "section:app:changes").expect("changes");
        assert!(find_node(staged, "dir:app:src/deep").is_some());
        assert!(find_node(staged, "file:app:src/deep/mod.rs").is_some());
        assert!(find_node(staged, "dir:app:src").is_none());
        assert!(find_node(changes, "dir:app:src#unstaged").is_some());
        assert!(find_node(changes, "file:app:src/lib.rs").is_some());
        assert!(find_node(changes, "dir:app:src").is_none());
        assert!(find_node(changes, "dir:app:src/deep").is_none());
    }

    #[test]
    fn split_flat_mode_keeps_file_lists_under_sections() {
        let tree = built_tree(
            vec![
                fc("src/a.rs", Some("M"), None, false),
                fc("src/b.rs", None, Some("M"), false),
            ],
            false,
        );
        let repo_node = find_node(&tree, "repo:app").expect("repo:app");
        assert_eq!(
            repo_node
                .children
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["section:app:staged", "section:app:changes"]
        );
        let staged = find_node(&tree, "section:app:staged").expect("staged");
        let changes = find_node(&tree, "section:app:changes").expect("changes");
        assert!(staged.children.iter().all(|c| c.kind != NodeKind::Dir));
        assert!(changes.children.iter().all(|c| c.kind != NodeKind::Dir));
        assert!(find_node(staged, "file:app:src/a.rs").is_some());
        assert!(find_node(changes, "file:app:src/b.rs").is_some());
        let rows = flatten(&tree, &HashSet::new());
        assert!(rows.iter().all(|r| r.kind != NodeKind::Dir));
    }
}

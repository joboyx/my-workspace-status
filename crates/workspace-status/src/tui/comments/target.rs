//! Comment targets, paint markers, and refresh GC.
//!
//! Live refs come from the workspace snapshot (`local_branches`, checkout
//! paths, repo identity). This module does not run git.

use std::collections::{BTreeSet, HashSet};

use crate::helpers::{is_counted_local_branch, is_default_branch, is_detached_head_branch};
use crate::snapshot::{CheckoutKind, WorkspaceSnapshot};

use super::super::commit_files::CommitFileRow;
use super::super::diff::{DiffCellKind, DiffRow};
use super::super::drill::CommitFileSource;
use super::super::tree::{
    dir_path_from_id, find_node, path_under_dir, NodeKind, TreeNode, VisibleRow,
};
use super::super::viewed::normalize_viewed_path;
use super::reference::{DiffSide, DiffSource, EntityRef};
use super::store::{ordered_line_range, repo_identity, CommentKey, CommentStore};
use workspace_status_graph::GraphRow;

/// Live refs used to drop stale comments on refresh.
#[derive(Clone, Debug, Default)]
pub struct CommentLiveSet {
    /// `(repo, branch)` pairs that still exist locally.
    pub branches: BTreeSet<(String, String)>,
    /// Checkout paths that still exist.
    pub worktrees: HashSet<String>,
    /// Repo identities that still have a checkout (commit comments).
    pub identities: HashSet<String>,
    /// `(repo, sha)` pairs that still resolve.
    pub commits: HashSet<(String, String)>,
}

/// Collect live branches, checkouts, and stored commit SHAs from `snapshot`.
///
/// Does not run git. `local_branches` is filled on the snapshot worker.
/// Skip-wipe (keep stored branch comments while porcelain is unreadable)
/// applies only when every checkout of that identity has an empty counted
/// list. A successful sibling's names win. Status-failed rows do not add
/// stored or last-good names when that identity already has counted
/// branches.
pub fn collect_live_set(snapshot: &WorkspaceSnapshot, store: &CommentStore) -> CommentLiveSet {
    let mut live = CommentLiveSet::default();
    let mut authoritative: HashSet<String> = HashSet::new();
    for repo in &snapshot.repos {
        let path = normalize_viewed_path(&repo.repo);
        live.worktrees.insert(path);
        let identity = repo_identity(&repo.repo, repo.primary_repo.as_deref());
        live.identities.insert(identity.clone());
        if repo.sync_note == "status failed" {
            continue;
        }
        let mut known = 0usize;
        for name in &repo.local_branches {
            if is_counted_local_branch(name) {
                live.branches.insert((identity.clone(), name.clone()));
                known += 1;
            }
        }
        if is_counted_local_branch(&repo.branch) {
            live.branches
                .insert((identity.clone(), repo.branch.clone()));
            known += 1;
        }
        if known > 0 {
            authoritative.insert(identity);
        }
    }
    for key in store.keys() {
        match key {
            CommentKey::Commit { repo, sha } | CommentKey::CommitLine { repo, sha, .. } => {
                if live.identities.contains(repo) {
                    live.commits.insert((repo.clone(), sha.clone()));
                }
            }
            CommentKey::Branch { repo, branch } | CommentKey::WorktreeLine { repo, branch, .. } => {
                if live.identities.contains(repo)
                    && !authoritative.contains(repo)
                    && is_counted_local_branch(branch)
                {
                    live.branches.insert((repo.clone(), branch.clone()));
                }
            }
            _ => {}
        }
    }
    live
}

/// Drop comments whose branch, worktree, or commit is gone.
pub fn gc_comments(store: &CommentStore, live: &CommentLiveSet) -> CommentStore {
    let mut next = CommentStore::new();
    for (key, body) in store {
        if comment_is_live(key, live) {
            next.insert(key.clone(), body.clone());
        }
    }
    next
}

fn comment_is_live(key: &CommentKey, live: &CommentLiveSet) -> bool {
    match key {
        CommentKey::Branch { repo, branch } => {
            if is_detached_head_branch(branch) {
                live.worktrees.contains(repo) || live.identities.contains(repo)
            } else {
                live.branches.contains(&(repo.clone(), branch.clone()))
            }
        }
        CommentKey::WorktreeLine { repo, branch, .. } => {
            if is_detached_head_branch(branch) {
                live.identities.contains(repo)
            } else {
                live.branches.contains(&(repo.clone(), branch.clone()))
            }
        }
        CommentKey::Worktree { path } => live.worktrees.contains(path),
        CommentKey::Commit { repo, sha } | CommentKey::CommitLine { repo, sha, .. } => {
            live.commits.contains(&(repo.clone(), sha.clone()))
        }
    }
}

/// First numbered line in the viewport (prefer the new/right side).
pub fn viewport_line_number(rows: &[DiffRow], scroll: u16) -> Option<u32> {
    for row in rows.iter().skip(scroll as usize) {
        if let Some(n) = row_line_number(row) {
            return Some(n);
        }
    }
    None
}

fn row_line_number(row: &DiffRow) -> Option<u32> {
    let DiffRow::Line { left, right } = row else {
        return None;
    };
    right.as_ref().and_then(|c| c.line_no).or(left.line_no)
}

/// Inclusive numbered-line span for painted rows `start..=end`.
///
/// Prefers the new/right side. Returns `None` when the range has no
/// numbered line.
pub fn viewport_line_range(rows: &[DiffRow], start: usize, end: usize) -> Option<(u32, u32)> {
    let lo = start.min(end);
    let hi = start.max(end);
    let mut first = None;
    let mut last = None;
    for row in rows
        .iter()
        .skip(lo)
        .take(hi.saturating_sub(lo).saturating_add(1))
    {
        let Some(n) = row_line_number(row) else {
            continue;
        };
        first = Some(first.map_or(n, |f: u32| f.min(n)));
        last = Some(last.map_or(n, |l: u32| l.max(n)));
    }
    Some((first?, last?))
}

/// Exactly one non-default local branch, if any.
pub fn sole_non_default_branch<'a, I>(names: I, override_name: Option<&str>) -> Option<String>
where
    I: IntoIterator<Item = &'a str>,
{
    let non_default: Vec<String> = names
        .into_iter()
        .filter(|name| !is_default_branch(name, override_name) && !is_detached_head_branch(name))
        .map(str::to_string)
        .collect();
    if non_default.len() == 1 {
        Some(non_default[0].clone())
    } else {
        None
    }
}

fn local_branch_names(snapshot: &WorkspaceSnapshot, repo_path: &str) -> Vec<String> {
    let Some(row) = snapshot.repos.iter().find(|r| r.repo == repo_path) else {
        return Vec::new();
    };
    let mut names: Vec<String> = row
        .local_branches
        .iter()
        .filter(|name| is_counted_local_branch(name))
        .cloned()
        .collect();
    if is_counted_local_branch(&row.branch) && !names.contains(&row.branch) {
        names.push(row.branch.clone());
    }
    names
}

/// Comment key for the focused tree / graph / diff, or `None` for a no-op.
pub fn resolve_comment_target(
    snapshot: &WorkspaceSnapshot,
    tree_row: Option<&VisibleRow>,
    graph_row: Option<&GraphRow>,
    graph_repo: Option<&str>,
    diff_focused: bool,
    diff_repo: Option<&str>,
    diff_path: Option<&str>,
    diff_source: Option<&CommitFileSource>,
    diff_line: Option<u32>,
    diff_end_line: Option<u32>,
) -> Option<CommentKey> {
    if diff_focused {
        return resolve_diff_target(
            snapshot,
            diff_repo,
            diff_path,
            diff_source,
            diff_line,
            diff_end_line,
        );
    }
    if let Some(row) = graph_row {
        let repo = graph_repo?;
        return resolve_graph_target(snapshot, repo, row);
    }
    resolve_tree_target(snapshot, tree_row?)
}

/// Old / new / unified from focused painted diff rows.
///
/// Split uses gutter numbers on each side. Inline uses cell kind (`Del` is
/// old, `Add` is new, context is both). Both sides, or neither, is unified.
pub fn diff_focus_side(rows: &[DiffRow], start: usize, end: usize) -> DiffSide {
    let lo = start.min(end);
    let hi = start.max(end);
    let mut has_old = false;
    let mut has_new = false;
    for row in rows
        .iter()
        .skip(lo)
        .take(hi.saturating_sub(lo).saturating_add(1))
    {
        let (old, new) = row_side_flags(row);
        has_old |= old;
        has_new |= new;
    }
    match (has_old, has_new) {
        (true, false) => DiffSide::Old,
        (false, true) => DiffSide::New,
        _ => DiffSide::Unified,
    }
}

fn row_side_flags(row: &DiffRow) -> (bool, bool) {
    let DiffRow::Line { left, right } = row else {
        return (false, false);
    };
    match right {
        Some(right) => (left.line_no.is_some(), right.line_no.is_some()),
        None => match left.kind {
            DiffCellKind::Del => (true, false),
            DiffCellKind::Add => (false, true),
            DiffCellKind::Ctx => (true, true),
            DiffCellKind::Empty | DiffCellKind::Meta => (false, false),
        },
    }
}

/// Entity reference for the focused tree / graph / commit-file / diff.
///
/// Covers file / dir / workspace / family / linked worktree / repo, graph
/// commit / stash / worktree, commit-file rows, and numbered diff lines.
/// Returns `None` when there is no copy target (group, empty focus, or a
/// diff with no numbered line).
pub fn resolve_entity_reference(
    snapshot: &WorkspaceSnapshot,
    tree_row: Option<&VisibleRow>,
    graph_row: Option<&GraphRow>,
    graph_repo: Option<&str>,
    diff_focused: bool,
    diff_repo: Option<&str>,
    diff_path: Option<&str>,
    diff_source: Option<&CommitFileSource>,
    diff_line: Option<u32>,
    diff_end_line: Option<u32>,
    diff_side: DiffSide,
    commit_file: Option<&CommitFileRow>,
    commit_file_repo: Option<&str>,
    workspace_path: Option<&str>,
) -> Option<EntityRef> {
    if diff_focused {
        return resolve_diff_entity(
            snapshot,
            diff_repo,
            diff_path,
            diff_source,
            diff_line,
            diff_end_line,
            diff_side,
        );
    }
    if let Some(row) = commit_file {
        return resolve_commit_file_entity(snapshot, commit_file_repo?, row);
    }
    if let Some(row) = graph_row {
        let repo = graph_repo?;
        return resolve_graph_entity(snapshot, repo, row);
    }
    resolve_tree_entity(snapshot, tree_row?, workspace_path)
}

fn checkout_primary<'a>(
    snapshot: &'a WorkspaceSnapshot,
    checkout: &str,
    row_primary: Option<&'a str>,
) -> Option<&'a str> {
    row_primary.or_else(|| {
        snapshot
            .repos
            .iter()
            .find(|r| r.repo == checkout)
            .and_then(|r| r.primary_repo.as_deref())
    })
}

fn entity_diff_source(source: Option<&CommitFileSource>) -> DiffSource {
    match source {
        Some(CommitFileSource::Commit { commit_id }) => DiffSource::Commit {
            sha: commit_id.clone(),
        },
        Some(CommitFileSource::Stash { stash_ref }) => DiffSource::Stash {
            stash_ref: stash_ref.clone(),
        },
        Some(CommitFileSource::Worktree) | None => DiffSource::Worktree,
    }
}

fn resolve_diff_entity(
    snapshot: &WorkspaceSnapshot,
    diff_repo: Option<&str>,
    diff_path: Option<&str>,
    diff_source: Option<&CommitFileSource>,
    diff_line: Option<u32>,
    diff_end_line: Option<u32>,
    side: DiffSide,
) -> Option<EntityRef> {
    let repo_path = diff_repo?;
    let path = diff_path?;
    let line = diff_line?;
    let (start_line, end_line) = ordered_line_range(line, diff_end_line.unwrap_or(line));
    let primary = checkout_primary(snapshot, repo_path, None);
    Some(EntityRef::diff(
        repo_path,
        primary,
        normalize_viewed_path(path),
        start_line,
        end_line,
        entity_diff_source(diff_source),
        side,
    ))
}

fn resolve_commit_file_entity(
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    row: &CommitFileRow,
) -> Option<EntityRef> {
    let primary = checkout_primary(snapshot, repo, None);
    let path = if row.path.is_empty() {
        return None;
    } else {
        row.path.clone()
    };
    if row.is_dir() {
        Some(EntityRef::dir(repo, primary, path))
    } else {
        Some(EntityRef::file(repo, primary, path))
    }
}

fn resolve_graph_entity(
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    row: &GraphRow,
) -> Option<EntityRef> {
    let primary = checkout_primary(snapshot, repo, None);
    match row {
        GraphRow::Commit { commit, .. } => {
            Some(EntityRef::commit(repo, primary, commit.id.clone()))
        }
        GraphRow::Uncommitted { .. } => Some(EntityRef::worktree(
            repo,
            primary,
            normalize_viewed_path(repo),
        )),
        GraphRow::Worktree(wt) => Some(EntityRef::worktree(
            repo,
            primary,
            normalize_viewed_path(&wt.path),
        )),
        GraphRow::Stash(stash) => Some(EntityRef::stash(
            repo,
            primary,
            stash.stash_ref.clone(),
            stash.id.clone(),
            stash.subject.clone(),
        )),
    }
}

fn resolve_tree_entity(
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
    workspace_path: Option<&str>,
) -> Option<EntityRef> {
    match row.kind {
        NodeKind::Group => None,
        NodeKind::Workspace => Some(EntityRef::workspace(workspace_path?)),
        NodeKind::File => {
            let repo_path = row.repo.as_deref()?;
            let path = file_row_path(row)?;
            let primary = checkout_primary(snapshot, repo_path, row.primary_repo.as_deref());
            Some(EntityRef::file(repo_path, primary, path))
        }
        NodeKind::Dir => {
            let repo_path = row.repo.as_deref()?;
            let path = dir_row_path(row, repo_path)?;
            let primary = checkout_primary(snapshot, repo_path, row.primary_repo.as_deref());
            Some(EntityRef::dir(repo_path, primary, path))
        }
        NodeKind::Repo | NodeKind::Checkout => {
            let repo_path = row.repo.as_deref()?;
            let primary = checkout_primary(snapshot, repo_path, row.primary_repo.as_deref());
            let path = if row.chrome.path.is_empty() {
                repo_path
            } else {
                row.chrome.path.as_str()
            };
            let is_family = row.chrome.is_family;
            let is_linked = row.chrome.checkout_kind == Some(CheckoutKind::Linked)
                || snapshot
                    .repos
                    .iter()
                    .find(|r| r.repo == repo_path)
                    .map(|r| r.checkout_kind)
                    == Some(CheckoutKind::Linked);
            if is_linked {
                Some(EntityRef::worktree(repo_path, primary, path))
            } else if is_family {
                Some(EntityRef::family(repo_path, primary, path))
            } else {
                Some(EntityRef::repo(repo_path, primary, path))
            }
        }
    }
}

fn file_row_path(row: &VisibleRow) -> Option<String> {
    if let Some(file) = row.file.as_ref() {
        if !file.path.is_empty() {
            return Some(file.path.clone());
        }
    }
    if row.chrome.path.is_empty() {
        None
    } else {
        Some(row.chrome.path.clone())
    }
}

fn dir_row_path(row: &VisibleRow, repo: &str) -> Option<String> {
    if !row.chrome.path.is_empty() {
        return Some(row.chrome.path.clone());
    }
    dir_path_from_id(&row.id, repo)
}

fn resolve_diff_target(
    snapshot: &WorkspaceSnapshot,
    diff_repo: Option<&str>,
    diff_path: Option<&str>,
    diff_source: Option<&CommitFileSource>,
    diff_line: Option<u32>,
    diff_end_line: Option<u32>,
) -> Option<CommentKey> {
    let repo_path = diff_repo?;
    let path = diff_path?;
    let line = diff_line?;
    let (line, end_line) = ordered_line_range(line, diff_end_line.unwrap_or(line));
    let snap = snapshot.repos.iter().find(|r| r.repo == repo_path);
    let identity = repo_identity(repo_path, snap.and_then(|r| r.primary_repo.as_deref()));
    match diff_source {
        Some(CommitFileSource::Commit { commit_id }) => Some(CommentKey::CommitLine {
            repo: identity,
            sha: commit_id.clone(),
            path: normalize_viewed_path(path),
            line,
            end_line,
        }),
        Some(CommitFileSource::Stash { .. }) => None,
        Some(CommitFileSource::Worktree) | None => {
            let branch = snap?.branch.clone();
            Some(CommentKey::WorktreeLine {
                repo: identity,
                branch,
                path: normalize_viewed_path(path),
                line,
                end_line,
            })
        }
    }
}

fn resolve_graph_target(
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    row: &GraphRow,
) -> Option<CommentKey> {
    let snap = snapshot.repos.iter().find(|r| r.repo == repo);
    let identity = repo_identity(repo, snap.and_then(|r| r.primary_repo.as_deref()));
    match row {
        GraphRow::Commit { commit, .. } => Some(CommentKey::Commit {
            repo: identity,
            sha: commit.id.clone(),
        }),
        GraphRow::Uncommitted { .. } => Some(CommentKey::Worktree {
            path: normalize_viewed_path(repo),
        }),
        GraphRow::Worktree(wt) => Some(CommentKey::Worktree {
            path: normalize_viewed_path(&wt.path),
        }),
        GraphRow::Stash(_) => None,
    }
}

fn resolve_tree_target(snapshot: &WorkspaceSnapshot, row: &VisibleRow) -> Option<CommentKey> {
    match row.kind {
        NodeKind::Workspace | NodeKind::Group | NodeKind::Dir | NodeKind::File => None,
        NodeKind::Repo | NodeKind::Checkout => {
            let repo_path = row.repo.as_deref()?;
            let snap = snapshot.repos.iter().find(|r| r.repo == repo_path);
            let identity = repo_identity(
                repo_path,
                row.primary_repo
                    .as_deref()
                    .or(snap.and_then(|r| r.primary_repo.as_deref())),
            );
            let override_name = row
                .chrome
                .default_branch_override
                .as_deref()
                .or(snap.and_then(|r| r.default_branch_override.as_deref()));
            let is_family = row.chrome.is_family;
            let is_linked = row.chrome.checkout_kind == Some(CheckoutKind::Linked)
                || snap.map(|r| r.checkout_kind) == Some(CheckoutKind::Linked);
            let branch = row.chrome.branch.as_str();
            if is_linked || is_detached_head_branch(branch) {
                return Some(CommentKey::Worktree {
                    path: normalize_viewed_path(repo_path),
                });
            }
            if is_family {
                return repo_root_attach(snapshot, repo_path, &identity, override_name);
            }
            if branch.is_empty() {
                return repo_root_attach(snapshot, repo_path, &identity, override_name);
            }
            if is_default_branch(branch, override_name) {
                return repo_root_attach(snapshot, repo_path, &identity, override_name);
            }
            Some(CommentKey::Branch {
                repo: identity,
                branch: branch.to_string(),
            })
        }
    }
}

fn repo_root_attach(
    snapshot: &WorkspaceSnapshot,
    git_repo: &str,
    identity: &str,
    override_name: Option<&str>,
) -> Option<CommentKey> {
    let names = local_branch_names(snapshot, git_repo);
    let branch = sole_non_default_branch(names.iter().map(String::as_str), override_name)?;
    Some(CommentKey::Branch {
        repo: identity.to_string(),
        branch,
    })
}

/// Pane list used to scope `y` markdown copy.
pub enum CommentExportList<'a> {
    /// Workspace tree (depth 0 left).
    Tree { row: Option<&'a VisibleRow> },
    /// Graph list (depth 0 right or depth 1 left).
    Graph {
        repo: Option<&'a str>,
        row: Option<&'a GraphRow>,
    },
    /// Commit-file list (depth 2 left, or depth 1 right).
    CommitFiles {
        repo: &'a str,
        source: &'a CommitFileSource,
        path: &'a str,
        is_dir: bool,
    },
    /// Focused numbered file diff.
    Diff {
        repo: &'a str,
        path: &'a str,
        source: Option<&'a CommitFileSource>,
    },
}

/// Live comments under the focused tree / graph / commit-file / diff row.
///
/// A file copies that path only. A folder copies descendants under that
/// path. Unrelated siblings stay out. The workspace row copies every live
/// comment.
pub fn comments_in_focus_scope(
    store: &CommentStore,
    snapshot: &WorkspaceSnapshot,
    tree: &TreeNode,
    list: CommentExportList<'_>,
) -> CommentStore {
    let scope = export_scope(snapshot, tree, list);
    store
        .iter()
        .filter(|(key, _)| scope.contains(key))
        .map(|(key, body)| (key.clone(), body.clone()))
        .collect()
}

enum ExportScope {
    All,
    Empty,
    Identities {
        ids: BTreeSet<String>,
        worktrees: BTreeSet<String>,
    },
    Checkout {
        identity: String,
        path: String,
        branch: String,
        extra: Option<CommentKey>,
    },
    WorktreeObject {
        path: String,
        identity: String,
        branch: String,
    },
    WorktreePath {
        identity: String,
        branch: String,
        path: String,
        prefix: bool,
    },
    Commit {
        identity: String,
        sha: String,
        path: Option<String>,
        prefix: bool,
    },
}

impl ExportScope {
    fn contains(&self, key: &CommentKey) -> bool {
        match self {
            Self::All => true,
            Self::Empty => false,
            Self::Identities { ids, worktrees } => identity_key_matches(key, ids, worktrees),
            Self::Checkout {
                identity,
                path,
                branch,
                extra,
            } => extra.as_ref() == Some(key) || checkout_key_matches(key, identity, path, branch),
            Self::WorktreeObject {
                path,
                identity,
                branch,
            } => match key {
                CommentKey::Worktree { path: p } => p == path,
                CommentKey::WorktreeLine {
                    repo, branch: b, ..
                } => repo == identity && b == branch,
                _ => false,
            },
            Self::WorktreePath {
                identity,
                branch,
                path,
                prefix,
            } => match key {
                CommentKey::WorktreeLine {
                    repo,
                    branch: b,
                    path: p,
                    ..
                } => repo == identity && b == branch && path_in_scope(p, path, *prefix),
                _ => false,
            },
            Self::Commit {
                identity,
                sha,
                path,
                prefix,
            } => match key {
                CommentKey::Commit { repo, sha: s } => {
                    path.is_none() && repo == identity && s == sha
                }
                CommentKey::CommitLine {
                    repo,
                    sha: s,
                    path: p,
                    ..
                } => {
                    repo == identity
                        && s == sha
                        && match path {
                            None => true,
                            Some(scope_path) => path_in_scope(p, scope_path, *prefix),
                        }
                }
                _ => false,
            },
        }
    }
}

fn path_in_scope(path: &str, scope: &str, prefix: bool) -> bool {
    if prefix {
        path_under_dir(path, scope)
    } else {
        path == scope
    }
}

fn identity_key_matches(
    key: &CommentKey,
    ids: &BTreeSet<String>,
    worktrees: &BTreeSet<String>,
) -> bool {
    match key {
        CommentKey::Branch { repo, .. }
        | CommentKey::Commit { repo, .. }
        | CommentKey::WorktreeLine { repo, .. }
        | CommentKey::CommitLine { repo, .. } => ids.contains(repo),
        CommentKey::Worktree { path } => worktrees.contains(path),
    }
}

fn checkout_key_matches(key: &CommentKey, identity: &str, path: &str, branch: &str) -> bool {
    match key {
        CommentKey::Branch { repo, branch: b } => repo == identity && b == branch,
        CommentKey::Worktree { path: p } => p == path || p == identity,
        CommentKey::WorktreeLine {
            repo, branch: b, ..
        } => repo == identity && b == branch,
        CommentKey::Commit { repo, .. } | CommentKey::CommitLine { repo, .. } => repo == identity,
    }
}

/// Checkout paths in `snapshot` whose [`repo_identity`] is in `ids`, plus
/// each identity path itself.
fn worktree_paths_for_identities(
    snapshot: &WorkspaceSnapshot,
    ids: &BTreeSet<String>,
) -> BTreeSet<String> {
    let mut paths = ids.clone();
    for repo in &snapshot.repos {
        let identity = repo_identity(&repo.repo, repo.primary_repo.as_deref());
        if ids.contains(&identity) {
            paths.insert(normalize_viewed_path(&repo.repo));
        }
    }
    paths
}

fn identities_scope(snapshot: &WorkspaceSnapshot, ids: BTreeSet<String>) -> ExportScope {
    let worktrees = worktree_paths_for_identities(snapshot, &ids);
    ExportScope::Identities { ids, worktrees }
}

fn worktree_on_identity(snapshot: &WorkspaceSnapshot, identity: &str, path: &str) -> bool {
    worktree_paths_for_identities(snapshot, &BTreeSet::from([identity.to_string()])).contains(path)
}

fn export_scope(
    snapshot: &WorkspaceSnapshot,
    tree: &TreeNode,
    list: CommentExportList<'_>,
) -> ExportScope {
    match list {
        CommentExportList::Tree { row } => tree_export_scope(snapshot, tree, row),
        CommentExportList::Graph { repo, row } => graph_export_scope(snapshot, repo, row),
        CommentExportList::CommitFiles {
            repo,
            source,
            path,
            is_dir,
        } => source_path_scope(snapshot, repo, source, path, is_dir),
        CommentExportList::Diff { repo, path, source } => match source {
            Some(src) => source_path_scope(snapshot, repo, src, path, false),
            None => source_path_scope(snapshot, repo, &CommitFileSource::Worktree, path, false),
        },
    }
}

fn tree_export_scope(
    snapshot: &WorkspaceSnapshot,
    tree: &TreeNode,
    row: Option<&VisibleRow>,
) -> ExportScope {
    let Some(row) = row else {
        return ExportScope::Empty;
    };
    match row.kind {
        NodeKind::Workspace => ExportScope::All,
        NodeKind::Group => match find_node(tree, &row.id) {
            Some(node) => identities_scope(snapshot, collect_node_identities(node)),
            None => ExportScope::Empty,
        },
        NodeKind::File => {
            let Some(file) = row.file.as_ref() else {
                return ExportScope::Empty;
            };
            let Some((identity, branch, _)) = row_checkout(snapshot, row) else {
                return ExportScope::Empty;
            };
            ExportScope::WorktreePath {
                identity,
                branch,
                path: normalize_viewed_path(&file.path),
                prefix: false,
            }
        }
        NodeKind::Dir => {
            let Some((identity, branch, repo_path)) = row_checkout(snapshot, row) else {
                return ExportScope::Empty;
            };
            let Some(dir) = dir_scope_path(row, &repo_path) else {
                return ExportScope::Empty;
            };
            ExportScope::WorktreePath {
                identity,
                branch,
                path: normalize_viewed_path(&dir),
                prefix: true,
            }
        }
        NodeKind::Repo | NodeKind::Checkout => {
            let Some((identity, branch, path)) = row_checkout(snapshot, row) else {
                return ExportScope::Empty;
            };
            if row.chrome.is_family {
                let mut ids = BTreeSet::new();
                ids.insert(identity);
                if let Some(node) = find_node(tree, &row.id) {
                    ids.extend(collect_node_identities(node));
                }
                return identities_scope(snapshot, ids);
            }
            ExportScope::Checkout {
                extra: resolve_tree_target(snapshot, row),
                identity,
                path,
                branch,
            }
        }
    }
}

fn graph_export_scope(
    snapshot: &WorkspaceSnapshot,
    repo: Option<&str>,
    row: Option<&GraphRow>,
) -> ExportScope {
    let Some(repo) = repo else {
        return ExportScope::Empty;
    };
    let Some(row) = row else {
        return ExportScope::Empty;
    };
    let snap = snapshot.repos.iter().find(|r| r.repo == repo);
    let identity = repo_identity(repo, snap.and_then(|r| r.primary_repo.as_deref()));
    match row {
        GraphRow::Commit { commit, .. } => ExportScope::Commit {
            identity,
            sha: commit.id.clone(),
            path: None,
            prefix: false,
        },
        GraphRow::Uncommitted { .. } => ExportScope::WorktreeObject {
            path: normalize_viewed_path(repo),
            identity,
            branch: snap.map(|r| r.branch.clone()).unwrap_or_default(),
        },
        GraphRow::Worktree(wt) => {
            let wt_path = normalize_viewed_path(&wt.path);
            let wt_snap = snapshot.repos.iter().find(|r| r.repo == wt.path);
            ExportScope::WorktreeObject {
                identity: repo_identity(&wt.path, wt_snap.and_then(|r| r.primary_repo.as_deref())),
                branch: wt_snap.map(|r| r.branch.clone()).unwrap_or_default(),
                path: wt_path,
            }
        }
        GraphRow::Stash(_) => ExportScope::Empty,
    }
}

fn source_path_scope(
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    source: &CommitFileSource,
    path: &str,
    prefix: bool,
) -> ExportScope {
    let snap = snapshot.repos.iter().find(|r| r.repo == repo);
    let identity = repo_identity(repo, snap.and_then(|r| r.primary_repo.as_deref()));
    let path = normalize_viewed_path(path);
    match source {
        CommitFileSource::Commit { commit_id } => ExportScope::Commit {
            identity,
            sha: commit_id.clone(),
            path: Some(path),
            prefix,
        },
        CommitFileSource::Worktree => ExportScope::WorktreePath {
            identity,
            branch: snap.map(|r| r.branch.clone()).unwrap_or_default(),
            path,
            prefix,
        },
        CommitFileSource::Stash { .. } => ExportScope::Empty,
    }
}

fn row_checkout(
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
) -> Option<(String, String, String)> {
    let repo_path = row.repo.as_deref()?;
    let snap = snapshot.repos.iter().find(|r| r.repo == repo_path);
    let identity = repo_identity(
        repo_path,
        row.primary_repo
            .as_deref()
            .or(snap.and_then(|r| r.primary_repo.as_deref())),
    );
    let branch = if !row.chrome.branch.is_empty() {
        row.chrome.branch.clone()
    } else {
        snap.map(|r| r.branch.clone()).unwrap_or_default()
    };
    Some((identity, branch, normalize_viewed_path(repo_path)))
}

fn dir_scope_path(row: &VisibleRow, repo_path: &str) -> Option<String> {
    if !row.chrome.path.is_empty() {
        return Some(row.chrome.path.clone());
    }
    dir_path_from_id(&row.id, repo_path)
}

fn collect_node_identities(node: &TreeNode) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    walk_identities(node, &mut out);
    out
}

fn walk_identities(node: &TreeNode, out: &mut BTreeSet<String>) {
    if let Some(repo) = node.repo.as_deref() {
        out.insert(repo_identity(repo, node.primary_repo.as_deref()));
    }
    for child in &node.children {
        walk_identities(child, out);
    }
}

/// True when `row` should paint a comment marker.
///
/// Family rows match [`CommentKey::Worktree`] for every checkout of the
/// family identity. Linked-worktree rows match this checkout path and the
/// identity path, not sibling worktrees. Matching uses snapshot
/// [`repo_identity`] and checkout paths, not a path prefix.
pub fn tree_row_has_comment(
    store: &CommentStore,
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
) -> bool {
    comments_resolved_state(store, |key| tree_key_on_row(snapshot, row, key)).is_some()
}

/// True when every comment that paints on `row` is resolved.
pub fn tree_row_comments_resolved(
    store: &CommentStore,
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
) -> bool {
    comments_resolved_state(store, |key| tree_key_on_row(snapshot, row, key)).unwrap_or(false)
}

fn tree_key_on_row(snapshot: &WorkspaceSnapshot, row: &VisibleRow, key: &CommentKey) -> bool {
    let Some(repo_path) = row.repo.as_deref() else {
        return false;
    };
    let snap = snapshot.repos.iter().find(|r| r.repo == repo_path);
    let identity = repo_identity(
        repo_path,
        row.primary_repo
            .as_deref()
            .or(snap.and_then(|r| r.primary_repo.as_deref())),
    );
    match row.kind {
        NodeKind::File => {
            let Some(file) = row.file.as_ref() else {
                return false;
            };
            let path = normalize_viewed_path(&file.path);
            match key {
                CommentKey::WorktreeLine {
                    repo,
                    path: p,
                    branch,
                    ..
                } => repo == &identity && p == &path && branch == &row.chrome.branch,
                CommentKey::CommitLine { repo, path: p, .. } => repo == &identity && p == &path,
                _ => false,
            }
        }
        NodeKind::Repo | NodeKind::Checkout => {
            if row.chrome.is_family {
                return match key {
                    CommentKey::Branch { repo, .. }
                    | CommentKey::Commit { repo, .. }
                    | CommentKey::WorktreeLine { repo, .. }
                    | CommentKey::CommitLine { repo, .. } => repo == &identity,
                    CommentKey::Worktree { path } => {
                        worktree_on_identity(snapshot, &identity, path)
                    }
                };
            }
            let is_linked = row.chrome.checkout_kind == Some(CheckoutKind::Linked)
                || snap.map(|r| r.checkout_kind) == Some(CheckoutKind::Linked);
            if is_linked {
                return matches!(
                    key,
                    CommentKey::Worktree { path }
                        if path == &normalize_viewed_path(repo_path) || path == &identity
                );
            }
            if is_detached_head_branch(&row.chrome.branch) {
                return matches!(
                    key,
                    CommentKey::Worktree { path } if path == &normalize_viewed_path(repo_path)
                );
            }
            if !row.chrome.branch.is_empty()
                && matches!(
                    key,
                    CommentKey::Branch { repo, branch }
                        if repo == &identity && branch == &row.chrome.branch
                )
            {
                return true;
            }
            matches!(
                key,
                CommentKey::Worktree { path } if path == &normalize_viewed_path(repo_path)
            )
        }
        _ => false,
    }
}

/// True when this graph visible row has an object comment or a file-line
/// comment that belongs on that row.
///
/// Commit rows match a commit object comment or any `CommitLine` for that
/// SHA. Uncommitted matches a worktree object comment or a working-tree
/// line comment for `branch`. Worktree rows match a worktree object comment
/// or a working-tree line comment for that checkout's branch. Stash rows
/// never mark.
pub fn graph_row_has_comment(
    store: &CommentStore,
    repo: &str,
    primary: Option<&str>,
    row: &GraphRow,
    branch: Option<&str>,
) -> bool {
    comments_resolved_state(store, |key| {
        graph_key_on_row(repo, primary, row, branch, key)
    })
    .is_some()
}

/// True when every comment that paints on this graph row is resolved.
pub fn graph_row_comments_resolved(
    store: &CommentStore,
    repo: &str,
    primary: Option<&str>,
    row: &GraphRow,
    branch: Option<&str>,
) -> bool {
    comments_resolved_state(store, |key| {
        graph_key_on_row(repo, primary, row, branch, key)
    })
    .unwrap_or(false)
}

fn graph_key_on_row(
    repo: &str,
    primary: Option<&str>,
    row: &GraphRow,
    branch: Option<&str>,
    key: &CommentKey,
) -> bool {
    let identity = repo_identity(repo, primary);
    match row {
        GraphRow::Commit { commit, .. } => match key {
            CommentKey::Commit { repo, sha } => repo == &identity && sha == &commit.id,
            CommentKey::CommitLine {
                repo,
                sha: line_sha,
                ..
            } => repo == &identity && line_sha == &commit.id,
            _ => false,
        },
        GraphRow::Uncommitted { .. } => match key {
            CommentKey::Worktree { path } => path == &normalize_viewed_path(repo),
            CommentKey::WorktreeLine {
                repo,
                branch: key_branch,
                ..
            } => branch.is_some_and(|b| repo == &identity && key_branch == b),
            _ => false,
        },
        GraphRow::Worktree(wt) => match key {
            CommentKey::Worktree { path } => path == &normalize_viewed_path(&wt.path),
            CommentKey::WorktreeLine {
                repo,
                branch: key_branch,
                ..
            } => wt
                .branch
                .as_deref()
                .is_some_and(|b| repo == &identity && key_branch == b),
            _ => false,
        },
        GraphRow::Stash(_) => false,
    }
}

/// True when this commit-file row has a line comment.
///
/// Directory rows never mark. Stash sources never mark.
pub fn commit_file_row_has_comment(
    store: &CommentStore,
    repo: &str,
    primary: Option<&str>,
    source: &CommitFileSource,
    path: &str,
    branch: Option<&str>,
) -> bool {
    comments_resolved_state(store, |key| {
        commit_file_key_on_row(repo, primary, source, path, branch, key)
    })
    .is_some()
}

/// True when every line comment on this commit-file row is resolved.
pub fn commit_file_row_comments_resolved(
    store: &CommentStore,
    repo: &str,
    primary: Option<&str>,
    source: &CommitFileSource,
    path: &str,
    branch: Option<&str>,
) -> bool {
    comments_resolved_state(store, |key| {
        commit_file_key_on_row(repo, primary, source, path, branch, key)
    })
    .unwrap_or(false)
}

fn commit_file_key_on_row(
    repo: &str,
    primary: Option<&str>,
    source: &CommitFileSource,
    path: &str,
    branch: Option<&str>,
    key: &CommentKey,
) -> bool {
    let identity = repo_identity(repo, primary);
    let path = normalize_viewed_path(path);
    match source {
        CommitFileSource::Commit { commit_id } => matches!(
            key,
            CommentKey::CommitLine {
                repo,
                sha,
                path: p,
                ..
            } if repo == &identity && sha == commit_id && p == &path
        ),
        CommitFileSource::Stash { .. } => false,
        CommitFileSource::Worktree => match key {
            CommentKey::WorktreeLine {
                repo,
                branch: key_branch,
                path: p,
                ..
            } => branch.is_some_and(|b| repo == &identity && key_branch == b && p == &path),
            _ => false,
        },
    }
}

/// True when this painted diff line number has a line comment.
pub fn diff_line_has_comment(
    store: &CommentStore,
    repo: &str,
    primary: Option<&str>,
    branch: Option<&str>,
    path: &str,
    source: Option<&CommitFileSource>,
    line: u32,
) -> bool {
    diff_line_comment_state(store, repo, primary, branch, path, source, line).is_some()
}

/// Resolve state for a painted diff line. `None` when no comment covers
/// `line`. `Some(true)` when every covering comment is resolved.
pub fn diff_line_comment_state(
    store: &CommentStore,
    repo: &str,
    primary: Option<&str>,
    branch: Option<&str>,
    path: &str,
    source: Option<&CommitFileSource>,
    line: u32,
) -> Option<bool> {
    comments_resolved_state(store, |key| {
        diff_line_key_covers(repo, primary, branch, path, source, line, key)
    })
}

fn diff_line_key_covers(
    repo: &str,
    primary: Option<&str>,
    branch: Option<&str>,
    path: &str,
    source: Option<&CommitFileSource>,
    line: u32,
    key: &CommentKey,
) -> bool {
    let identity = repo_identity(repo, primary);
    let path = normalize_viewed_path(path);
    match source {
        Some(CommitFileSource::Commit { commit_id }) => matches!(
            key,
            CommentKey::CommitLine {
                repo,
                sha,
                path: p,
                ..
            } if repo == &identity && sha == commit_id && p == &path && key.covers_line(line)
        ),
        Some(CommitFileSource::Stash { .. }) => false,
        Some(CommitFileSource::Worktree) | None => match key {
            CommentKey::WorktreeLine {
                repo,
                branch: b,
                path: p,
                ..
            } => {
                branch.is_some_and(|branch| repo == &identity && b == branch && p == &path)
                    && key.covers_line(line)
            }
            _ => false,
        },
    }
}

fn comments_resolved_state(
    store: &CommentStore,
    pred: impl Fn(&CommentKey) -> bool,
) -> Option<bool> {
    let mut any = false;
    let mut all_resolved = true;
    for (key, entry) in store {
        if pred(key) {
            any = true;
            if !entry.resolved {
                all_resolved = false;
            }
        }
    }
    any.then_some(all_resolved)
}

/// Stored line comment that covers the probe line on the same file.
///
/// Without visual-line highlight, `;` opens this key when one exists so
/// the overlay matches the `"` glyph. Visual-line `;` keeps the highlight
/// span and does not call this.
///
/// Overlap: smallest inclusive span, then lowest start line, then
/// [`CommentKey`] order. A one-line comment on that line wins over a
/// wider range.
pub fn covering_line_comment(store: &CommentStore, probe: &CommentKey) -> Option<CommentKey> {
    let n = match probe {
        CommentKey::WorktreeLine { line, .. } | CommentKey::CommitLine { line, .. } => *line,
        _ => return None,
    };
    store
        .keys()
        .filter(|key| same_file_line_comment(key, probe) && key.covers_line(n))
        .min_by(|a, b| {
            line_span_len(a)
                .cmp(&line_span_len(b))
                .then_with(|| line_span_start(a).cmp(&line_span_start(b)))
                .then_with(|| a.cmp(b))
        })
        .cloned()
}

fn same_file_line_comment(a: &CommentKey, b: &CommentKey) -> bool {
    match (a, b) {
        (
            CommentKey::WorktreeLine {
                repo, branch, path, ..
            },
            CommentKey::WorktreeLine {
                repo: repo2,
                branch: branch2,
                path: path2,
                ..
            },
        ) => repo == repo2 && branch == branch2 && path == path2,
        (
            CommentKey::CommitLine {
                repo, sha, path, ..
            },
            CommentKey::CommitLine {
                repo: repo2,
                sha: sha2,
                path: path2,
                ..
            },
        ) => repo == repo2 && sha == sha2 && path == path2,
        _ => false,
    }
}

fn line_span_len(key: &CommentKey) -> u32 {
    match key {
        CommentKey::WorktreeLine { line, end_line, .. }
        | CommentKey::CommitLine { line, end_line, .. } => {
            end_line.saturating_sub(*line).saturating_add(1)
        }
        _ => u32::MAX,
    }
}

fn line_span_start(key: &CommentKey) -> u32 {
    match key {
        CommentKey::WorktreeLine { line, .. } | CommentKey::CommitLine { line, .. } => *line,
        _ => u32::MAX,
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::{put_comment, CommentEntry, CommentKey, CommentStore};
    use super::*;
    use crate::helpers::DETACHED_HEAD_BRANCH;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::diff::{DiffCell, DiffCellKind, DiffContent, DiffRow};
    use crate::tui::tree::NodeChrome;
    use workspace_status_graph::Commit;

    fn snap(repo: &str, branch: &str, kind: CheckoutKind, primary: Option<&str>) -> RepoSnapshot {
        RepoSnapshot {
            repo: repo.into(),
            branch: branch.into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: false,
            has_staged: false,
            has_untracked: false,
            changes: Vec::new(),
            checkout_kind: kind,
            primary_repo: primary.map(str::to_string),
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
        }
    }

    fn snap_with_branches(
        repo: &str,
        branch: &str,
        kind: CheckoutKind,
        primary: Option<&str>,
        branches: &[&str],
    ) -> RepoSnapshot {
        let mut row = snap(repo, branch, kind, primary);
        row.local_branches = branches.iter().map(|s| (*s).to_string()).collect();
        row
    }

    #[test]
    fn gc_drops_gone_branch_keeps_commit() {
        let snapshot = build_workspace_snapshot(
            &[snap_with_branches(
                "app",
                "main",
                CheckoutKind::Primary,
                None,
                &["main"],
            )],
            &[],
            false,
            &[],
        );
        let branch = CommentKey::Branch {
            repo: "app".into(),
            branch: "feature/gone".into(),
        };
        let line = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "feature/gone".into(),
            path: "a.rs".into(),
            line: 1,
            end_line: 1,
        };
        let commit = CommentKey::Commit {
            repo: "app".into(),
            sha: "abc".into(),
        };
        let mut store = CommentStore::new();
        store = put_comment(&store, branch, "b");
        store = put_comment(&store, line, "l");
        store = put_comment(&store, commit.clone(), "c");
        let live = collect_live_set(&snapshot, &store);
        let next = gc_comments(&store, &live);
        assert!(next.get(&commit).is_some());
        assert_eq!(next.len(), 1);
    }

    #[test]
    fn gc_keeps_branch_comments_when_status_failed() {
        let mut row = snap(
            "app",
            crate::helpers::UNKNOWN_HEAD_BRANCH,
            CheckoutKind::Primary,
            None,
        );
        row.sync_note = "status failed".into();
        let snapshot = build_workspace_snapshot(&[row], &[], false, &[]);
        let branch = CommentKey::Branch {
            repo: "app".into(),
            branch: "topic/keep".into(),
        };
        let line = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "topic/keep".into(),
            path: "a.rs".into(),
            line: 1,
            end_line: 1,
        };
        let mut store = CommentStore::new();
        store = put_comment(&store, branch.clone(), "b");
        store = put_comment(&store, line.clone(), "l");
        let live = collect_live_set(&snapshot, &store);
        let next = gc_comments(&store, &live);
        assert_eq!(next.get(&branch).map(CommentEntry::as_str), Some("b"));
        assert_eq!(next.get(&line).map(CommentEntry::as_str), Some("l"));
        assert!(!live.branches.contains(&("app".into(), "(unknown)".into())));
    }

    #[test]
    fn gc_drops_deleted_branch_when_sibling_status_failed() {
        let primary = snap_with_branches(
            "app",
            "main",
            CheckoutKind::Primary,
            None,
            &["main", "feature/linked-open"],
        );
        let mut linked = snap(
            "app/.worktrees/feat",
            crate::helpers::UNKNOWN_HEAD_BRANCH,
            CheckoutKind::Linked,
            Some("app"),
        );
        linked.sync_note = "status failed".into();
        linked.local_branches = vec!["main".into(), "doomed".into(), "feature/linked-open".into()];
        let snapshot = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
        let branch = CommentKey::Branch {
            repo: "app".into(),
            branch: "doomed".into(),
        };
        let line = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "doomed".into(),
            path: "a.rs".into(),
            line: 1,
            end_line: 1,
        };
        let mut store = CommentStore::new();
        store = put_comment(&store, branch.clone(), "b");
        store = put_comment(&store, line.clone(), "l");
        let live = collect_live_set(&snapshot, &store);
        let next = gc_comments(&store, &live);
        assert!(next.get(&branch).is_none());
        assert!(next.get(&line).is_none());
        assert!(!live.branches.contains(&("app".into(), "doomed".into())));
    }

    #[test]
    fn detached_primary_row_is_worktree_path_key() {
        let snapshot = build_workspace_snapshot(
            &[snap(
                "app",
                DETACHED_HEAD_BRANCH,
                CheckoutKind::Primary,
                None,
            )],
            &[],
            false,
            &[],
        );
        let row = VisibleRow {
            kind: NodeKind::Repo,
            repo: Some("app".into()),
            chrome: crate::tui::tree::NodeChrome {
                branch: DETACHED_HEAD_BRANCH.into(),
                is_family: false,
                checkout_kind: Some(CheckoutKind::Primary),
                ..Default::default()
            },
            ..VisibleRow::default()
        };
        assert_eq!(
            resolve_tree_target(&snapshot, &row),
            Some(CommentKey::Worktree { path: "app".into() })
        );
        assert!(!matches!(
            resolve_tree_target(&snapshot, &row),
            Some(CommentKey::Branch { .. })
        ));
    }

    #[test]
    fn watch_gc_keeps_detached_worktree_comment() {
        let snapshot = build_workspace_snapshot(
            &[snap(
                "app",
                DETACHED_HEAD_BRANCH,
                CheckoutKind::Primary,
                None,
            )],
            &[],
            false,
            &[],
        );
        let key = CommentKey::Worktree { path: "app".into() };
        let store = put_comment(&CommentStore::new(), key.clone(), "detached note");
        let live = collect_live_set(&snapshot, &store);
        let next = gc_comments(&store, &live);
        assert_eq!(
            next.get(&key).map(CommentEntry::as_str),
            Some("detached note")
        );
    }

    #[test]
    fn viewport_line_prefers_right_number() {
        let rows = vec![
            DiffRow::Section(crate::tui::diff::DiffSection::Unstaged),
            DiffRow::Line {
                left: DiffCell {
                    kind: DiffCellKind::Ctx,
                    text: "a".into(),
                    line_no: Some(1),
                },
                right: Some(DiffCell {
                    kind: DiffCellKind::Add,
                    text: "b".into(),
                    line_no: Some(2),
                }),
            },
        ];
        assert_eq!(viewport_line_number(&rows, 0), Some(2));
        assert_eq!(viewport_line_number(&rows, 1), Some(2));
        assert_eq!(viewport_line_number(&[], 0), None);
        assert_eq!(viewport_line_range(&rows, 0, 1), Some((2, 2)));
        assert_eq!(viewport_line_range(&rows, 0, 0), None);
        assert_eq!(viewport_line_range(&[], 0, 0), None);
        let _ = DiffContent::default();
    }

    fn worktree_line(line: u32, end_line: u32) -> CommentKey {
        CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line,
            end_line,
        }
    }

    #[test]
    fn covering_line_picks_tightest_then_lowest_start() {
        let probe = worktree_line(2, 2);
        let wide = worktree_line(1, 4);
        let mid = worktree_line(2, 3);
        let one = worktree_line(2, 2);
        let other_file = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "other.md".into(),
            line: 1,
            end_line: 4,
        };
        let mut store = CommentStore::new();
        store = put_comment(&store, wide.clone(), "wide");
        store = put_comment(&store, other_file, "other");
        assert_eq!(covering_line_comment(&store, &probe), Some(wide.clone()));
        store = put_comment(&store, mid.clone(), "mid");
        assert_eq!(covering_line_comment(&store, &probe), Some(mid.clone()));
        store = put_comment(&store, one.clone(), "one");
        assert_eq!(covering_line_comment(&store, &probe), Some(one));

        let left = worktree_line(1, 3);
        let right = worktree_line(2, 4);
        let mut tied = CommentStore::new();
        tied = put_comment(&tied, right, "right");
        tied = put_comment(&tied, left.clone(), "left");
        assert_eq!(covering_line_comment(&tied, &probe), Some(left));
        assert!(covering_line_comment(&CommentStore::new(), &probe).is_none());
        assert!(covering_line_comment(&store, &worktree_line(9, 9)).is_none());
    }

    #[test]
    fn default_branch_row_is_repo_root_without_attach() {
        let snapshot = build_workspace_snapshot(
            &[snap("app", "main", CheckoutKind::Primary, None)],
            &[],
            false,
            &[],
        );
        let mut row = VisibleRow {
            kind: NodeKind::Repo,
            repo: Some("app".into()),
            chrome: crate::tui::tree::NodeChrome {
                branch: "main".into(),
                is_family: false,
                ..Default::default()
            },
            ..VisibleRow::default()
        };
        assert!(resolve_tree_target(&snapshot, &row).is_none());
        row.kind = NodeKind::Workspace;
        row.repo = None;
        assert!(resolve_tree_target(&snapshot, &row).is_none());
    }

    #[test]
    fn linked_worktree_row_is_path_key() {
        let snapshot = build_workspace_snapshot(
            &[snap(
                "app/.worktrees/feat",
                "feature/linked-open",
                CheckoutKind::Linked,
                Some("app"),
            )],
            &[],
            false,
            &[],
        );
        let row = VisibleRow {
            kind: NodeKind::Checkout,
            repo: Some("app/.worktrees/feat".into()),
            primary_repo: Some("app".into()),
            chrome: crate::tui::tree::NodeChrome {
                branch: "feature/linked-open".into(),
                checkout_kind: Some(CheckoutKind::Linked),
                ..Default::default()
            },
            ..VisibleRow::default()
        };
        assert_eq!(
            resolve_tree_target(&snapshot, &row),
            Some(CommentKey::Worktree {
                path: "app/.worktrees/feat".into()
            })
        );
    }

    fn dirty_file(path: &str) -> FileChange {
        FileChange {
            path: path.into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }
    }

    fn workspace_tree() -> TreeNode {
        TreeNode {
            id: "workspace".into(),
            kind: NodeKind::Workspace,
            label: "workspace".into(),
            repo: None,
            primary_repo: None,
            ignored: false,
            file: None,
            children: Vec::new(),
            chrome: NodeChrome::default(),
        }
    }

    fn scoped_bodies(
        store: &CommentStore,
        snapshot: &WorkspaceSnapshot,
        list: CommentExportList<'_>,
    ) -> Vec<String> {
        let scoped = comments_in_focus_scope(store, snapshot, &workspace_tree(), list);
        scoped.values().map(|entry| entry.body.clone()).collect()
    }

    fn folder_store() -> (WorkspaceSnapshot, CommentStore) {
        let snapshot = build_workspace_snapshot(
            &[snap("app", "main", CheckoutKind::Primary, None)],
            &[],
            false,
            &[],
        );
        let mut store = CommentStore::new();
        store = put_comment(
            &store,
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "folder1/file1".into(),
                line: 1,
                end_line: 1,
            },
            "note-file1",
        );
        store = put_comment(
            &store,
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "folder1/file2".into(),
                line: 2,
                end_line: 2,
            },
            "note-file2",
        );
        store = put_comment(
            &store,
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 1,
                end_line: 1,
            },
            "note-readme",
        );
        (snapshot, store)
    }

    fn file_row(path: &str) -> VisibleRow {
        VisibleRow {
            kind: NodeKind::File,
            repo: Some("app".into()),
            file: Some(dirty_file(path)),
            chrome: NodeChrome {
                path: path.into(),
                ..Default::default()
            },
            ..VisibleRow::default()
        }
    }

    fn dir_row(path: &str) -> VisibleRow {
        VisibleRow {
            kind: NodeKind::Dir,
            repo: Some("app".into()),
            id: format!("dir:app:{path}"),
            chrome: NodeChrome {
                path: path.into(),
                ..Default::default()
            },
            ..VisibleRow::default()
        }
    }

    #[test]
    fn export_file_scope_omits_folder_sibling() {
        let (snapshot, store) = folder_store();
        let row = file_row("folder1/file2");
        let bodies = scoped_bodies(
            &store,
            &snapshot,
            CommentExportList::Tree { row: Some(&row) },
        );
        assert_eq!(bodies, vec!["note-file2".to_string()]);
    }

    #[test]
    fn export_dir_scope_includes_descendants_omits_sibling_file() {
        let (snapshot, store) = folder_store();
        let row = dir_row("folder1");
        let mut bodies = scoped_bodies(
            &store,
            &snapshot,
            CommentExportList::Tree { row: Some(&row) },
        );
        bodies.sort();
        assert_eq!(
            bodies,
            vec!["note-file1".to_string(), "note-file2".to_string()]
        );
    }

    #[test]
    fn export_workspace_scope_includes_every_live_comment() {
        let (snapshot, store) = folder_store();
        let row = VisibleRow {
            kind: NodeKind::Workspace,
            ..VisibleRow::default()
        };
        let mut bodies = scoped_bodies(
            &store,
            &snapshot,
            CommentExportList::Tree { row: Some(&row) },
        );
        bodies.sort();
        assert_eq!(
            bodies,
            vec![
                "note-file1".to_string(),
                "note-file2".to_string(),
                "note-readme".to_string()
            ]
        );
    }

    #[test]
    fn export_graph_commit_omits_branch_and_sibling_commit() {
        let snapshot = build_workspace_snapshot(
            &[snap("merger", "feature/graph", CheckoutKind::Primary, None)],
            &[],
            false,
            &[],
        );
        let mut store = CommentStore::new();
        store = put_comment(
            &store,
            CommentKey::Branch {
                repo: "merger".into(),
                branch: "feature/graph".into(),
            },
            "branch-note",
        );
        store = put_comment(
            &store,
            CommentKey::Commit {
                repo: "merger".into(),
                sha: "aaa".into(),
            },
            "commit-a",
        );
        store = put_comment(
            &store,
            CommentKey::Commit {
                repo: "merger".into(),
                sha: "bbb".into(),
            },
            "commit-b",
        );
        let row = GraphRow::Commit {
            commit: Commit {
                id: "aaa".into(),
                ..Default::default()
            },
            is_head: false,
            worktrees: Vec::new(),
        };
        let bodies = scoped_bodies(
            &store,
            &snapshot,
            CommentExportList::Graph {
                repo: Some("merger"),
                row: Some(&row),
            },
        );
        assert_eq!(bodies, vec!["commit-a".to_string()]);
    }

    #[test]
    fn export_repo_row_includes_attached_branch_and_commits() {
        let snapshot = build_workspace_snapshot(
            &[snap_with_branches(
                "app",
                "main",
                CheckoutKind::Primary,
                None,
                &["main", "doomed"],
            )],
            &[],
            false,
            &[],
        );
        let mut store = CommentStore::new();
        store = put_comment(
            &store,
            CommentKey::Branch {
                repo: "app".into(),
                branch: "doomed".into(),
            },
            "attached",
        );
        store = put_comment(
            &store,
            CommentKey::Commit {
                repo: "app".into(),
                sha: "dead".into(),
            },
            "commit-note",
        );
        store = put_comment(
            &store,
            CommentKey::WorktreeLine {
                repo: "other".into(),
                branch: "main".into(),
                path: "x.rs".into(),
                line: 1,
                end_line: 1,
            },
            "sibling-repo",
        );
        let row = VisibleRow {
            kind: NodeKind::Repo,
            repo: Some("app".into()),
            chrome: NodeChrome {
                branch: "main".into(),
                ..Default::default()
            },
            ..VisibleRow::default()
        };
        let mut bodies = scoped_bodies(
            &store,
            &snapshot,
            CommentExportList::Tree { row: Some(&row) },
        );
        bodies.sort();
        assert_eq!(
            bodies,
            vec!["attached".to_string(), "commit-note".to_string()]
        );
    }

    fn commit_row(sha: &str) -> GraphRow {
        GraphRow::Commit {
            commit: workspace_status_graph::Commit {
                id: sha.into(),
                subject: "subject".into(),
                ..Default::default()
            },
            is_head: false,
            worktrees: Vec::new(),
        }
    }

    #[test]
    fn graph_commit_row_marks_object_and_file_comments() {
        let row = commit_row("abc");
        let mut store = CommentStore::new();
        assert!(!graph_row_has_comment(&store, "app", None, &row, None));
        store = put_comment(
            &store,
            CommentKey::Commit {
                repo: "app".into(),
                sha: "abc".into(),
            },
            "c",
        );
        assert!(graph_row_has_comment(&store, "app", None, &row, None));
        assert!(!graph_row_has_comment(
            &store,
            "app",
            None,
            &GraphRow::Uncommitted { has_changes: true },
            Some("main")
        ));
    }

    #[test]
    fn graph_commit_row_marks_file_line_comments() {
        let row = commit_row("abc");
        let store = put_comment(
            &CommentStore::new(),
            CommentKey::CommitLine {
                repo: "app".into(),
                sha: "abc".into(),
                path: "a.rs".into(),
                line: 3,
                end_line: 3,
            },
            "l",
        );
        assert!(graph_row_has_comment(&store, "app", None, &row, None));
        assert!(!graph_row_has_comment(
            &store,
            "app",
            None,
            &commit_row("other"),
            None
        ));
        assert!(!graph_row_has_comment(
            &store,
            "app",
            None,
            &GraphRow::Stash(workspace_status_graph::Stash::default()),
            None
        ));
    }

    #[test]
    fn graph_uncommitted_row_marks_worktree_line_comments() {
        let row = GraphRow::Uncommitted { has_changes: true };
        let store = put_comment(
            &CommentStore::new(),
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 2,
                end_line: 2,
            },
            "l",
        );
        assert!(graph_row_has_comment(
            &store,
            "app",
            None,
            &row,
            Some("main")
        ));
        assert!(!graph_row_has_comment(
            &store,
            "app",
            None,
            &row,
            Some("other")
        ));
        assert!(!graph_row_has_comment(&store, "app", None, &row, None));
    }

    #[test]
    fn commit_file_row_marks_line_comments_only() {
        let source = CommitFileSource::Commit {
            commit_id: "abc".into(),
        };
        let store = put_comment(
            &CommentStore::new(),
            CommentKey::CommitLine {
                repo: "app".into(),
                sha: "abc".into(),
                path: "src/lib.rs".into(),
                line: 1,
                end_line: 1,
            },
            "l",
        );
        assert!(commit_file_row_has_comment(
            &store,
            "app",
            None,
            &source,
            "src/lib.rs",
            None
        ));
        assert!(!commit_file_row_has_comment(
            &store,
            "app",
            None,
            &source,
            "README.md",
            None
        ));
        assert!(!commit_file_row_has_comment(
            &store,
            "app",
            None,
            &CommitFileSource::Stash {
                stash_ref: "stash@{0}".into()
            },
            "src/lib.rs",
            None
        ));
    }

    fn identity_vs_linked_family_snapshot() -> WorkspaceSnapshot {
        build_workspace_snapshot(
            &[
                snap("app", "main", CheckoutKind::Primary, None),
                snap(
                    ".worktrees/app/feat",
                    "feature/linked-open",
                    CheckoutKind::Linked,
                    Some("app"),
                ),
                snap(
                    ".worktrees/app/other",
                    "feature/linked-other",
                    CheckoutKind::Linked,
                    Some("app"),
                ),
                snap("other", "main", CheckoutKind::Primary, None),
            ],
            &[],
            false,
            &[],
        )
    }

    fn identity_vs_linked_family_store() -> CommentStore {
        let mut store = CommentStore::new();
        store = put_comment(
            &store,
            CommentKey::Worktree { path: "app".into() },
            "on-identity",
        );
        store = put_comment(
            &store,
            CommentKey::Worktree {
                path: ".worktrees/app/feat".into(),
            },
            "on-worktree",
        );
        store = put_comment(
            &store,
            CommentKey::Worktree {
                path: ".worktrees/app/other".into(),
            },
            "on-sibling",
        );
        store = put_comment(
            &store,
            CommentKey::Branch {
                repo: "app".into(),
                branch: "feature/linked-open".into(),
            },
            "on-branch",
        );
        store = put_comment(
            &store,
            CommentKey::Worktree {
                path: "other".into(),
            },
            "on-other",
        );
        store
    }

    fn family_app_row() -> VisibleRow {
        VisibleRow {
            kind: NodeKind::Repo,
            repo: Some("app".into()),
            chrome: NodeChrome {
                is_family: true,
                checkout_kind: Some(CheckoutKind::Primary),
                ..Default::default()
            },
            ..VisibleRow::default()
        }
    }

    fn linked_worktree_app_feat_row() -> VisibleRow {
        VisibleRow {
            kind: NodeKind::Checkout,
            repo: Some(".worktrees/app/feat".into()),
            primary_repo: Some("app".into()),
            chrome: NodeChrome {
                branch: "feature/linked-open".into(),
                checkout_kind: Some(CheckoutKind::Linked),
                ..Default::default()
            },
            ..VisibleRow::default()
        }
    }

    fn sorted_tree_bodies(
        store: &CommentStore,
        snapshot: &WorkspaceSnapshot,
        row: &VisibleRow,
    ) -> Vec<String> {
        let mut bodies = scoped_bodies(store, snapshot, CommentExportList::Tree { row: Some(row) });
        bodies.sort();
        bodies
    }

    #[test]
    fn export_family_row_includes_identity_and_linked_worktree_object_comments() {
        let snapshot = identity_vs_linked_family_snapshot();
        let store = identity_vs_linked_family_store();
        let row = family_app_row();
        assert_eq!(
            sorted_tree_bodies(&store, &snapshot, &row),
            vec![
                "on-branch".to_string(),
                "on-identity".to_string(),
                "on-sibling".to_string(),
                "on-worktree".to_string(),
            ]
        );
    }

    #[test]
    fn export_linked_worktree_row_includes_identity_and_worktree_object_comments() {
        let snapshot = identity_vs_linked_family_snapshot();
        let store = identity_vs_linked_family_store();
        let row = linked_worktree_app_feat_row();
        assert_eq!(
            sorted_tree_bodies(&store, &snapshot, &row),
            vec![
                "on-branch".to_string(),
                "on-identity".to_string(),
                "on-worktree".to_string(),
            ]
        );
    }

    #[test]
    fn family_row_marks_linked_worktree_object_comment() {
        let store = put_comment(
            &CommentStore::new(),
            CommentKey::Worktree {
                path: ".worktrees/app/feat".into(),
            },
            "on-worktree",
        );
        assert!(tree_row_has_comment(
            &store,
            &identity_vs_linked_family_snapshot(),
            &family_app_row()
        ));
    }

    #[test]
    fn linked_worktree_row_marks_identity_object_comment() {
        let store = put_comment(
            &CommentStore::new(),
            CommentKey::Worktree { path: "app".into() },
            "on-identity",
        );
        assert!(tree_row_has_comment(
            &store,
            &identity_vs_linked_family_snapshot(),
            &linked_worktree_app_feat_row()
        ));
    }

    #[test]
    fn linked_worktree_row_omits_sibling_worktree_object_comment() {
        let snapshot = identity_vs_linked_family_snapshot();
        let store = put_comment(
            &CommentStore::new(),
            CommentKey::Worktree {
                path: ".worktrees/app/other".into(),
            },
            "on-sibling",
        );
        assert!(tree_row_has_comment(&store, &snapshot, &family_app_row()));
        assert!(!tree_row_has_comment(
            &store,
            &snapshot,
            &linked_worktree_app_feat_row()
        ));
    }

    #[test]
    fn worktree_path_app_other_is_not_family_of_app() {
        let snapshot = identity_vs_linked_family_snapshot();
        let store = put_comment(
            &CommentStore::new(),
            CommentKey::Worktree {
                path: "app-other/x".into(),
            },
            "on-app-other",
        );
        let row = family_app_row();
        let bodies = sorted_tree_bodies(&store, &snapshot, &row);
        assert!(!bodies.iter().any(|b| b == "on-app-other"));
        assert!(!tree_row_has_comment(&store, &snapshot, &row));
    }

    fn entity_from_tree(
        snapshot: &WorkspaceSnapshot,
        row: &VisibleRow,
        ws: Option<&str>,
    ) -> EntityRef {
        resolve_entity_reference(
            snapshot,
            Some(row),
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
            ws,
        )
        .expect("tree entity")
    }

    #[test]
    fn entity_file_dir_workspace_repo_family_worktree() {
        let snapshot = build_workspace_snapshot(
            &[
                snap("app", "main", CheckoutKind::Primary, None),
                snap(
                    "app/.worktrees/feat",
                    "feature/linked-open",
                    CheckoutKind::Linked,
                    Some("app"),
                ),
            ],
            &[],
            false,
            &[],
        );
        let file = file_row("src/lib.rs");
        match entity_from_tree(&snapshot, &file, None) {
            EntityRef::File { repo, path } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "src/lib.rs");
            }
            other => panic!("{other:?}"),
        }
        let dir = dir_row("src");
        match entity_from_tree(&snapshot, &dir, None) {
            EntityRef::Dir { path, .. } => assert_eq!(path, "src"),
            other => panic!("{other:?}"),
        }
        let workspace = VisibleRow {
            kind: NodeKind::Workspace,
            ..VisibleRow::default()
        };
        match entity_from_tree(&snapshot, &workspace, Some("/tmp/ws")) {
            EntityRef::Workspace { path } => assert_eq!(path, "/tmp/ws"),
            other => panic!("{other:?}"),
        }
        let family = family_app_row();
        match entity_from_tree(&snapshot, &family, None) {
            EntityRef::Family { repo, path } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "app");
            }
            other => panic!("{other:?}"),
        }
        let linked = linked_worktree_app_feat_row();
        match entity_from_tree(&snapshot, &linked, None) {
            EntityRef::Worktree { repo, path } => {
                assert_eq!(repo, "app");
                assert_eq!(path, ".worktrees/app/feat");
            }
            other => panic!("{other:?}"),
        }
        let repo_row = VisibleRow {
            kind: NodeKind::Repo,
            repo: Some("app".into()),
            chrome: NodeChrome {
                path: "app".into(),
                checkout_kind: Some(CheckoutKind::Primary),
                ..Default::default()
            },
            ..VisibleRow::default()
        };
        match entity_from_tree(&snapshot, &repo_row, None) {
            EntityRef::Repo { repo, path } => {
                assert_eq!(repo, "app");
                assert_eq!(path, "app");
            }
            other => panic!("{other:?}"),
        }
        let group = VisibleRow {
            kind: NodeKind::Group,
            ..VisibleRow::default()
        };
        assert!(resolve_entity_reference(
            &snapshot,
            Some(&group),
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
            None,
        )
        .is_none());
        assert!(resolve_comment_target(
            &snapshot,
            Some(&file),
            None,
            None,
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .is_none());
    }

    #[test]
    fn entity_graph_commit_stash_worktree() {
        let snapshot = build_workspace_snapshot(
            &[snap("app", "main", CheckoutKind::Primary, None)],
            &[],
            false,
            &[],
        );
        let commit = GraphRow::Commit {
            commit: Commit {
                id: "abcdef1234567890abcd".into(),
                ..Commit::default()
            },
            is_head: true,
            worktrees: Vec::new(),
        };
        match resolve_entity_reference(
            &snapshot,
            None,
            Some(&commit),
            Some("app"),
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
        ) {
            Some(EntityRef::Commit { repo, sha }) => {
                assert_eq!(repo, "app");
                assert_eq!(sha, "abcdef1234567890abcd");
            }
            other => panic!("{other:?}"),
        }
        let stash = GraphRow::Stash(workspace_status_graph::Stash {
            id: "deadbeef1234567890ab".into(),
            stash_ref: "stash@{0}".into(),
            subject: "WIP".into(),
            ..workspace_status_graph::Stash::default()
        });
        match resolve_entity_reference(
            &snapshot,
            None,
            Some(&stash),
            Some("app"),
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
        ) {
            Some(EntityRef::Stash { stash_ref, sha, .. }) => {
                assert_eq!(stash_ref, "stash@{0}");
                assert_eq!(sha, "deadbeef1234567890ab");
            }
            other => panic!("{other:?}"),
        }
        assert!(resolve_comment_target(
            &snapshot,
            None,
            Some(&stash),
            Some("app"),
            false,
            None,
            None,
            None,
            None,
            None,
        )
        .is_none());
        let uncommitted = GraphRow::Uncommitted { has_changes: true };
        match resolve_entity_reference(
            &snapshot,
            None,
            Some(&uncommitted),
            Some("app"),
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
        ) {
            Some(EntityRef::Worktree { path, .. }) => assert_eq!(path, "app"),
            other => panic!("{other:?}"),
        }
        let worktree = GraphRow::Worktree(workspace_status_graph::Worktree {
            path: "app".into(),
            head_id: None,
            branch: Some("main".into()),
            ignored: false,
            is_current: false,
        });
        match resolve_entity_reference(
            &snapshot,
            None,
            Some(&worktree),
            Some("app"),
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
        ) {
            Some(entity) => {
                assert!(
                    matches!(&entity, EntityRef::Worktree { path, .. } if path == "app"),
                    "{entity:?}"
                );
                let text = super::super::format_entity_reference(&entity);
                assert!(text.starts_with("kind: worktree\n"), "{text}");
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn entity_commit_file_file_and_dir() {
        let snapshot = build_workspace_snapshot(
            &[snap("app", "main", CheckoutKind::Primary, None)],
            &[],
            false,
            &[],
        );
        let file = crate::tui::commit_files::CommitFileRow {
            id: "file:README.md".into(),
            depth: 0,
            kind: crate::tui::commit_files::CommitFileRowKind::File,
            label: "README.md".into(),
            trailing: String::new(),
            segments: Vec::new(),
            trailing_segs: Vec::new(),
            path: "README.md".into(),
            foldable: false,
            folded: false,
            file: None,
        };
        match resolve_entity_reference(
            &snapshot,
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
            Some(&file),
            Some("app"),
            None,
        ) {
            Some(EntityRef::File { path, .. }) => assert_eq!(path, "README.md"),
            other => panic!("{other:?}"),
        }
        let dir = crate::tui::commit_files::CommitFileRow {
            kind: crate::tui::commit_files::CommitFileRowKind::Dir,
            path: "src".into(),
            ..file.clone()
        };
        match resolve_entity_reference(
            &snapshot,
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
            Some(&dir),
            Some("app"),
            None,
        ) {
            Some(EntityRef::Dir { path, .. }) => assert_eq!(path, "src"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn entity_diff_side_from_painted_rows() {
        let add = DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Add,
                text: "+x".into(),
                line_no: Some(2),
            },
            right: None,
        };
        let del = DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Del,
                text: "-y".into(),
                line_no: Some(1),
            },
            right: None,
        };
        let ctx = DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Ctx,
                text: " z".into(),
                line_no: Some(1),
            },
            right: None,
        };
        assert_eq!(diff_focus_side(&[add.clone()], 0, 0), DiffSide::New);
        assert_eq!(diff_focus_side(&[del.clone()], 0, 0), DiffSide::Old);
        assert_eq!(diff_focus_side(&[ctx], 0, 0), DiffSide::Unified);
        assert_eq!(diff_focus_side(&[add, del], 0, 1), DiffSide::Unified);
        let snapshot = build_workspace_snapshot(
            &[snap("app", "main", CheckoutKind::Primary, None)],
            &[],
            false,
            &[],
        );
        match resolve_entity_reference(
            &snapshot,
            None,
            None,
            None,
            true,
            Some("app"),
            Some("README.md"),
            None,
            Some(2),
            Some(2),
            DiffSide::New,
            None,
            None,
            None,
        ) {
            Some(EntityRef::Diff {
                path,
                start_line,
                end_line,
                source,
                side,
                ..
            }) => {
                assert_eq!(path, "README.md");
                assert_eq!(start_line, 2);
                assert_eq!(end_line, 2);
                assert_eq!(source, DiffSource::Worktree);
                assert_eq!(side, DiffSide::New);
            }
            other => panic!("{other:?}"),
        }
    }
}

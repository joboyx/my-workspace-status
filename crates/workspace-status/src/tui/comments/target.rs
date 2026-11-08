//! Comment targets, paint markers, and refresh GC.
//!
//! Live refs come from the workspace snapshot (`local_branches`, checkout
//! paths, repo identity). This module does not run git. GC stats a
//! checkout directory only for a worktree key it would otherwise drop.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

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
use super::store::{ordered_line_range, repo_identity, CommentEntry, CommentKey, CommentStore};
use workspace_status_graph::GraphRow;

/// Live refs used to drop stale comments on refresh.
#[derive(Clone, Debug, Default)]
pub struct CommentLiveSet {
    /// `(repo, branch)` pairs that still exist locally.
    pub branches: BTreeSet<(String, String)>,
    /// Checkout paths that still exist.
    pub worktrees: HashSet<String>,
    /// Repo identities with at least one checkout in this snapshot. GC
    /// treats a key whose repo is not here as unknown and keeps it.
    pub identities: HashSet<String>,
    /// Identities whose branch list this snapshot can be trusted for: at
    /// least one checkout is not `status failed` and lists a counted
    /// branch.
    pub authoritative: HashSet<String>,
    /// Checkout path → repo identity, for every checkout of this snapshot
    /// and of earlier snapshots of the session. Lets GC tell which repo a
    /// checkout that is now missing belonged to.
    pub checkout_owners: HashMap<String, String>,
}

/// Collect live branches, checkouts, and repo identities from `snapshot`.
///
/// Does not run git. `local_branches` is filled on the snapshot worker.
/// Skip-wipe (keep stored branch comments while porcelain is unreadable)
/// applies only when every checkout of that identity has an empty counted
/// list. A successful sibling's names win. Status-failed rows do not add
/// stored or last-good names when that identity already has counted
/// branches.
///
/// `known_owners` is the [`CommentLiveSet::checkout_owners`] map of the
/// previous call in this session (empty at launch). The result keeps those
/// entries and adds the checkouts of `snapshot`.
pub fn collect_live_set(
    snapshot: &WorkspaceSnapshot,
    store: &CommentStore,
    known_owners: &HashMap<String, String>,
) -> CommentLiveSet {
    let mut live = CommentLiveSet {
        checkout_owners: known_owners.clone(),
        ..CommentLiveSet::default()
    };
    let mut authoritative: HashSet<String> = HashSet::new();
    for repo in &snapshot.repos {
        let path = normalize_viewed_path(&repo.repo);
        let identity = repo_identity(&repo.repo, repo.primary_repo.as_deref());
        live.checkout_owners.insert(path.clone(), identity.clone());
        live.worktrees.insert(path);
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
        if let CommentKey::Branch { repo, branch } | CommentKey::WorktreeLine { repo, branch, .. } =
            key
        {
            if live.identities.contains(repo)
                && !authoritative.contains(repo)
                && is_counted_local_branch(branch)
            {
                live.branches.insert((repo.clone(), branch.clone()));
            }
        }
    }
    live.authoritative = authoritative;
    live
}

/// Drop comments whose branch or worktree is gone.
///
/// GC drops a key only when the snapshot is authoritative for it. A key
/// whose repo identity is not in [`CommentLiveSet::identities`] is unknown
/// (the repo can be missing from one snapshot after a discovery or status
/// failure) and stays. Branch and line keys of a present repo follow
/// [`collect_live_set`] (which keeps them while that repo's branch list is
/// unreadable). Commit and commit-line keys always stay: the snapshot has
/// no list of reachable commits. A worktree key whose path is not in the
/// snapshot drops only when an earlier snapshot of the session showed which
/// repo owned that path, that repo is present and
/// [`CommentLiveSet::authoritative`] now, and the checkout directory
/// `cwd/<path>` is gone on disk. The directory check covers a linked
/// worktree that drops out of one snapshot because `git worktree list`
/// failed: `git worktree remove` deletes the directory, a failed listing
/// does not.
///
/// Runs on the caller's thread. The only I/O is one `stat` per worktree key
/// that passes the other checks, which is rare (a worktree just removed).
pub fn gc_comments(store: &CommentStore, live: &CommentLiveSet, cwd: &Path) -> CommentStore {
    let mut next = CommentStore::new();
    for (key, body) in store {
        if comment_is_live(key, live, cwd) {
            next.insert(key.clone(), body.clone());
        }
    }
    next
}

/// `false` only when `live` (and, for a worktree key, the disk) shows the
/// key's target is gone (see [`gc_comments`]). Unknown targets are live.
fn comment_is_live(key: &CommentKey, live: &CommentLiveSet, cwd: &Path) -> bool {
    match key {
        CommentKey::Branch { repo, branch } | CommentKey::WorktreeLine { repo, branch, .. } => {
            !live.identities.contains(repo)
                || is_detached_head_branch(branch)
                || live.branches.contains(&(repo.clone(), branch.clone()))
        }
        CommentKey::Worktree { path } => {
            live.worktrees.contains(path)
                || !live
                    .checkout_owners
                    .get(path)
                    .is_some_and(|owner| live.authoritative.contains(owner))
                || !checkout_dir_gone(cwd, path)
        }
        CommentKey::Commit { .. } | CommentKey::CommitLine { .. } => true,
    }
}

/// `true` only when `cwd/<path>` surely does not exist. A stat error other
/// than not-found (permissions, I/O) counts as present, so GC keeps the key.
fn checkout_dir_gone(cwd: &Path, path: &str) -> bool {
    matches!(cwd.join(path).try_exists(), Ok(false))
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

/// Comment source of a diff, or `None` for a commit-vs-working-tree
/// compare, which has no comment key (its comment actions refuse).
fn entity_diff_source(source: Option<&CommitFileSource>) -> Option<DiffSource> {
    Some(match source {
        Some(CommitFileSource::Commit { commit_id }) => DiffSource::Commit {
            sha: commit_id.clone(),
        },
        Some(CommitFileSource::Stash { stash_ref }) => DiffSource::Stash {
            stash_ref: stash_ref.clone(),
        },
        Some(CommitFileSource::Worktree) | None => DiffSource::Worktree,
        Some(CommitFileSource::Compare { base_ref, head, .. }) => DiffSource::Compare {
            base_ref: base_ref.clone(),
            head: head.clone(),
        },
        Some(CommitFileSource::CommitVsWorktree { .. }) => return None,
    })
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
        entity_diff_source(diff_source)?,
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
        NodeKind::Group | NodeKind::Section => None,
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
        // Commit and compare diffs key to the commit the lines belong to.
        Some(source) => Some(CommentKey::CommitLine {
            repo: identity,
            sha: source.line_commit()?.to_string(),
            path: normalize_viewed_path(path),
            line,
            end_line,
        }),
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
        NodeKind::Workspace
        | NodeKind::Group
        | NodeKind::Section
        | NodeKind::Dir
        | NodeKind::File => None,
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
        NodeKind::Group | NodeKind::Section => match find_node(tree, &row.id) {
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
        CommitFileSource::Worktree => ExportScope::WorktreePath {
            identity,
            branch: snap.map(|r| r.branch.clone()).unwrap_or_default(),
            path,
            prefix,
        },
        source => match source.line_commit() {
            Some(sha) => ExportScope::Commit {
                identity,
                sha: sha.to_string(),
                path: Some(path),
                prefix,
            },
            None => ExportScope::Empty,
        },
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

/// Comments that paint their mark on `row`, in store order.
pub fn tree_row_comments<'a>(
    store: &'a CommentStore,
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
) -> Vec<&'a CommentEntry> {
    store
        .iter()
        .filter(|(key, _)| tree_key_on_row(snapshot, row, key))
        .map(|(_, entry)| entry)
        .collect()
}

/// True when comment `key` paints its mark on the file row of `path` in a
/// checkout of repo `identity` on `branch`: a working-tree line comment of
/// that file on that branch, or a line comment of that file at any commit.
/// The workspace tree and the Explorer tree file rows share this match.
fn file_key_on_checkout(identity: &str, branch: &str, path: &str, key: &CommentKey) -> bool {
    file_key_path(identity, branch, key) == Some(normalize_viewed_path(path).as_str())
}

/// The file path whose row comment `key` marks in a checkout of repo
/// `identity` on `branch` ([`file_key_on_checkout`]); `None` for a key that
/// marks no file row there.
fn file_key_path<'k>(identity: &str, branch: &str, key: &'k CommentKey) -> Option<&'k str> {
    match key {
        CommentKey::WorktreeLine {
            repo,
            path,
            branch: b,
            ..
        } if repo == identity && b == branch => Some(path),
        CommentKey::CommitLine { repo, path, .. } if repo == identity => Some(path),
        _ => None,
    }
}

/// Repo identity and branch of checkout `repo` in `snapshot`, the scope a
/// file row's comments match in ([`file_key_on_checkout`]).
fn checkout_file_scope(snapshot: &WorkspaceSnapshot, repo: &str) -> (String, String) {
    let snap = snapshot.repos.iter().find(|r| r.repo == repo);
    (
        repo_identity(repo, snap.and_then(|r| r.primary_repo.as_deref())),
        snap.map(|r| r.branch.clone()).unwrap_or_default(),
    )
}

/// Resolve state of the comment mark of every commented file row in
/// checkout `repo` (the Explorer tree's file rows), with the workspace tree
/// file row's match, from one pass over `store`: path → `true` when every
/// comment on it is resolved. A path with no comment is absent. A tree
/// paint builds it once and probes it per row.
pub fn checkout_file_comment_states(
    store: &CommentStore,
    snapshot: &WorkspaceSnapshot,
    repo: &str,
) -> HashMap<String, bool> {
    let (identity, branch) = checkout_file_scope(snapshot, repo);
    let mut states: HashMap<String, bool> = HashMap::new();
    for (key, entry) in store {
        if let Some(path) = file_key_path(&identity, &branch, key) {
            let resolved = states.entry(path.to_string()).or_insert(true);
            *resolved &= entry.resolved;
        }
    }
    states
}

/// Comments behind the mark [`checkout_file_comment_states`] reads for
/// `path`, in store order.
pub fn checkout_file_comments<'a>(
    store: &'a CommentStore,
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    path: &str,
) -> Vec<&'a CommentEntry> {
    let (identity, branch) = checkout_file_scope(snapshot, repo);
    store
        .iter()
        .filter(|(key, _)| file_key_on_checkout(&identity, &branch, path, key))
        .map(|(_, entry)| entry)
        .collect()
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
            // A file row carries no branch chrome: its checkout's branch is
            // the snapshot's, as the Explorer tree file row reads it.
            let branch = if row.chrome.branch.is_empty() {
                snap.map_or("", |r| r.branch.as_str())
            } else {
                row.chrome.branch.as_str()
            };
            row.file
                .as_ref()
                .is_some_and(|file| file_key_on_checkout(&identity, branch, &file.path, key))
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

/// Comments that paint their mark on graph row `row`, in store order.
pub fn graph_row_comments<'a>(
    store: &'a CommentStore,
    repo: &str,
    primary: Option<&str>,
    row: &GraphRow,
    branch: Option<&str>,
) -> Vec<&'a CommentEntry> {
    store
        .iter()
        .filter(|(key, _)| graph_key_on_row(repo, primary, row, branch, key))
        .map(|(_, entry)| entry)
        .collect()
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
/// Directory rows never mark. Stash sources never mark. A compare source
/// marks line comments keyed to its head commit.
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

/// Line comments that paint their mark on this commit-file row, in store
/// order.
pub fn commit_file_row_comments<'a>(
    store: &'a CommentStore,
    repo: &str,
    primary: Option<&str>,
    source: &CommitFileSource,
    path: &str,
    branch: Option<&str>,
) -> Vec<&'a CommentEntry> {
    store
        .iter()
        .filter(|(key, _)| commit_file_key_on_row(repo, primary, source, path, branch, key))
        .map(|(_, entry)| entry)
        .collect()
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
        CommitFileSource::Worktree => match key {
            CommentKey::WorktreeLine {
                repo,
                branch: key_branch,
                path: p,
                ..
            } => branch.is_some_and(|b| repo == &identity && key_branch == b && p == &path),
            _ => false,
        },
        // Commit and compare rows mark their commit's line comments.
        source => matches!(
            key,
            CommentKey::CommitLine {
                repo,
                sha,
                path: p,
                ..
            } if repo == &identity && Some(sha.as_str()) == source.line_commit() && p == &path
        ),
    }
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

/// Line comments that cover painted diff `line`, in store order: the
/// comments behind the mark [`diff_line_comment_state`] reads.
pub fn diff_line_comments<'a>(
    store: &'a CommentStore,
    repo: &str,
    primary: Option<&str>,
    branch: Option<&str>,
    path: &str,
    source: Option<&CommitFileSource>,
    line: u32,
) -> Vec<&'a CommentEntry> {
    store
        .iter()
        .filter(|(key, _)| diff_line_key_covers(repo, primary, branch, path, source, line, key))
        .map(|(_, entry)| entry)
        .collect()
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
        // Commit and compare diffs paint their commit's line comments.
        Some(source) => matches!(
            key,
            CommentKey::CommitLine {
                repo,
                sha,
                path: p,
                ..
            } if repo == &identity
                && Some(sha.as_str()) == source.line_commit()
                && p == &path
                && key.covers_line(line)
        ),
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
mod tests;

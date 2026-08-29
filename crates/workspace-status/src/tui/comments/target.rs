//! Comment targets, paint markers, and refresh GC.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;

use crate::git::{list_local_branches, rev_parse_quiet};
use crate::helpers::is_default_branch;
use crate::snapshot::{CheckoutKind, WorkspaceSnapshot};

use super::super::diff::DiffRow;
use super::super::drill::CommitFileSource;
use super::super::tree::{NodeKind, VisibleRow};
use super::super::viewed::normalize_viewed_path;
use super::store::{repo_identity, CommentKey, CommentStore};
use workspace_status_graph::GraphRow;

/// Live refs used to drop stale comments on refresh.
#[derive(Clone, Debug, Default)]
pub struct CommentLiveSet {
    /// `(repo, branch)` pairs that still exist locally.
    pub branches: BTreeSet<(String, String)>,
    /// Checkout paths that still exist.
    pub worktrees: HashSet<String>,
    /// `(repo, sha)` pairs that still resolve.
    pub commits: HashSet<(String, String)>,
}

/// Collect live branches, checkouts, and stored commit SHAs that still exist.
pub fn collect_live_set(
    snapshot: &WorkspaceSnapshot,
    cwd: &Path,
    store: &CommentStore,
) -> CommentLiveSet {
    let mut live = CommentLiveSet::default();
    for repo in &snapshot.repos {
        let path = normalize_viewed_path(&repo.repo);
        live.worktrees.insert(path);
        let identity = repo_identity(&repo.repo, repo.primary_repo.as_deref());
        let dir = cwd.join(&repo.repo);
        for branch in list_local_branches(&dir) {
            live.branches
                .insert((identity.clone(), branch.name.clone()));
        }
    }
    let mut needed: HashSet<(String, String)> = HashSet::new();
    for key in store.keys() {
        match key {
            CommentKey::Commit { repo, sha } | CommentKey::CommitLine { repo, sha, .. } => {
                needed.insert((repo.clone(), sha.clone()));
            }
            _ => {}
        }
    }
    for (repo, sha) in needed {
        let dir = cwd.join(&repo);
        if commit_exists(&dir, &sha) {
            live.commits.insert((repo, sha));
        }
    }
    live
}

fn commit_exists(cwd: &Path, sha: &str) -> bool {
    if sha.trim().is_empty() {
        return false;
    }
    let peeled = format!("{sha}^{{commit}}");
    rev_parse_quiet(&peeled, cwd).is_some() || rev_parse_quiet(sha, cwd).is_some()
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
        CommentKey::Branch { repo, branch } | CommentKey::WorktreeLine { repo, branch, .. } => {
            live.branches.contains(&(repo.clone(), branch.clone()))
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
        let DiffRow::Line { left, right } = row else {
            continue;
        };
        if let Some(n) = right.as_ref().and_then(|c| c.line_no).or(left.line_no) {
            return Some(n);
        }
    }
    None
}

/// Exactly one non-default local branch, if any.
pub fn sole_non_default_branch(cwd: &Path, override_name: Option<&str>) -> Option<String> {
    let branches = list_local_branches(cwd);
    let non_default: Vec<String> = branches
        .into_iter()
        .map(|b| b.name)
        .filter(|name| !is_default_branch(name, override_name))
        .collect();
    if non_default.len() == 1 {
        Some(non_default[0].clone())
    } else {
        None
    }
}

/// Comment key for the focused tree / graph / diff, or `None` for a no-op.
pub fn resolve_comment_target(
    cwd: &Path,
    snapshot: &WorkspaceSnapshot,
    tree_row: Option<&VisibleRow>,
    graph_row: Option<&GraphRow>,
    graph_repo: Option<&str>,
    diff_focused: bool,
    diff_repo: Option<&str>,
    diff_path: Option<&str>,
    diff_source: Option<&CommitFileSource>,
    diff_line: Option<u32>,
) -> Option<CommentKey> {
    if diff_focused {
        return resolve_diff_target(snapshot, diff_repo, diff_path, diff_source, diff_line);
    }
    if let Some(row) = graph_row {
        let repo = graph_repo?;
        return resolve_graph_target(snapshot, repo, row);
    }
    resolve_tree_target(cwd, snapshot, tree_row?)
}

fn resolve_diff_target(
    snapshot: &WorkspaceSnapshot,
    diff_repo: Option<&str>,
    diff_path: Option<&str>,
    diff_source: Option<&CommitFileSource>,
    diff_line: Option<u32>,
) -> Option<CommentKey> {
    let repo_path = diff_repo?;
    let path = diff_path?;
    let line = diff_line?;
    let snap = snapshot.repos.iter().find(|r| r.repo == repo_path);
    let identity = repo_identity(repo_path, snap.and_then(|r| r.primary_repo.as_deref()));
    match diff_source {
        Some(CommitFileSource::Commit { commit_id }) => Some(CommentKey::CommitLine {
            repo: identity,
            sha: commit_id.clone(),
            path: normalize_viewed_path(path),
            line,
        }),
        Some(CommitFileSource::Stash { .. }) => None,
        Some(CommitFileSource::Worktree) | None => {
            let branch = snap?.branch.clone();
            Some(CommentKey::WorktreeLine {
                repo: identity,
                branch,
                path: normalize_viewed_path(path),
                line,
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

fn resolve_tree_target(
    cwd: &Path,
    snapshot: &WorkspaceSnapshot,
    row: &VisibleRow,
) -> Option<CommentKey> {
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
            if is_linked {
                return Some(CommentKey::Worktree {
                    path: normalize_viewed_path(repo_path),
                });
            }
            if is_family {
                return repo_root_attach(cwd, repo_path, &identity, override_name);
            }
            let branch = row.chrome.branch.as_str();
            if branch.is_empty() {
                return repo_root_attach(cwd, repo_path, &identity, override_name);
            }
            if is_default_branch(branch, override_name) {
                return repo_root_attach(cwd, repo_path, &identity, override_name);
            }
            Some(CommentKey::Branch {
                repo: identity,
                branch: branch.to_string(),
            })
        }
    }
}

fn repo_root_attach(
    cwd: &Path,
    git_repo: &str,
    identity: &str,
    override_name: Option<&str>,
) -> Option<CommentKey> {
    let dir = cwd.join(git_repo);
    let branch = sole_non_default_branch(&dir, override_name)?;
    Some(CommentKey::Branch {
        repo: identity.to_string(),
        branch,
    })
}

/// True when `row` should paint a comment marker.
pub fn tree_row_has_comment(store: &CommentStore, row: &VisibleRow) -> bool {
    if store.is_empty() {
        return false;
    }
    let Some(repo_path) = row.repo.as_deref() else {
        return false;
    };
    let identity = repo_identity(repo_path, row.primary_repo.as_deref());
    match row.kind {
        NodeKind::File => {
            let Some(file) = row.file.as_ref() else {
                return false;
            };
            let path = normalize_viewed_path(&file.path);
            store.keys().any(|key| match key {
                CommentKey::WorktreeLine {
                    repo,
                    path: p,
                    branch,
                    ..
                } => repo == &identity && p == &path && branch == &row.chrome.branch,
                CommentKey::CommitLine { repo, path: p, .. } => repo == &identity && p == &path,
                _ => false,
            })
        }
        NodeKind::Repo | NodeKind::Checkout => {
            let is_linked = row.chrome.checkout_kind == Some(CheckoutKind::Linked);
            if is_linked {
                return store.contains_key(&CommentKey::Worktree {
                    path: normalize_viewed_path(repo_path),
                });
            }
            if row.chrome.is_family {
                return store.keys().any(|key| match key {
                    CommentKey::Branch { repo, .. }
                    | CommentKey::Commit { repo, .. }
                    | CommentKey::WorktreeLine { repo, .. }
                    | CommentKey::CommitLine { repo, .. } => repo == &identity,
                    CommentKey::Worktree { path } => {
                        path == &identity || path.starts_with(&format!("{identity}/"))
                    }
                });
            }
            if !row.chrome.branch.is_empty()
                && store.contains_key(&CommentKey::Branch {
                    repo: identity.clone(),
                    branch: row.chrome.branch.clone(),
                })
            {
                return true;
            }
            store.contains_key(&CommentKey::Worktree {
                path: normalize_viewed_path(repo_path),
            })
        }
        _ => false,
    }
}

/// True when this graph visible-row index has an object comment.
pub fn graph_row_has_comment(
    store: &CommentStore,
    repo: &str,
    primary: Option<&str>,
    row: &GraphRow,
) -> bool {
    let identity = repo_identity(repo, primary);
    match row {
        GraphRow::Commit { commit, .. } => store.contains_key(&CommentKey::Commit {
            repo: identity,
            sha: commit.id.clone(),
        }),
        GraphRow::Uncommitted { .. } => store.contains_key(&CommentKey::Worktree {
            path: normalize_viewed_path(repo),
        }),
        GraphRow::Worktree(wt) => store.contains_key(&CommentKey::Worktree {
            path: normalize_viewed_path(&wt.path),
        }),
        GraphRow::Stash(_) => false,
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
    let identity = repo_identity(repo, primary);
    let path = normalize_viewed_path(path);
    match source {
        Some(CommitFileSource::Commit { commit_id }) => {
            store.contains_key(&CommentKey::CommitLine {
                repo: identity,
                sha: commit_id.clone(),
                path,
                line,
            })
        }
        Some(CommitFileSource::Stash { .. }) => false,
        Some(CommitFileSource::Worktree) | None => {
            let Some(branch) = branch else {
                return false;
            };
            store.contains_key(&CommentKey::WorktreeLine {
                repo: identity,
                branch: branch.to_string(),
                path,
                line,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::store::{put_comment, CommentKey, CommentStore};
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};
    use crate::tui::diff::{DiffCell, DiffCellKind, DiffContent, DiffRow};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

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
        }
    }

    #[test]
    fn gc_drops_gone_branch_keeps_commit() {
        let branch = CommentKey::Branch {
            repo: "app".into(),
            branch: "feature/gone".into(),
        };
        let line = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "feature/gone".into(),
            path: "a.rs".into(),
            line: 1,
        };
        let commit = CommentKey::Commit {
            repo: "app".into(),
            sha: "abc".into(),
        };
        let mut store = CommentStore::new();
        store = put_comment(&store, branch, "b");
        store = put_comment(&store, line, "l");
        store = put_comment(&store, commit.clone(), "c");
        let mut live = CommentLiveSet::default();
        live.commits.insert(("app".into(), "abc".into()));
        let next = gc_comments(&store, &live);
        assert!(next.get(&commit).is_some());
        assert_eq!(next.len(), 1);
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
        let _ = DiffContent::default();
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
        let tmp = std::env::temp_dir().join(format!(
            "ws-comments-no-git-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::create_dir_all(&tmp);
        assert!(resolve_tree_target(&tmp, &snapshot, &row).is_none());
        row.kind = NodeKind::Workspace;
        row.repo = None;
        assert!(resolve_tree_target(&tmp, &snapshot, &row).is_none());
        let _ = fs::remove_dir_all(&tmp);
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
            resolve_tree_target(Path::new("/"), &snapshot, &row),
            Some(CommentKey::Worktree {
                path: "app/.worktrees/feat".into()
            })
        );
    }
}

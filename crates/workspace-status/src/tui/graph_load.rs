//! Load a [`workspace_status_graph::GraphModel`] from git + the snapshot.

use std::path::Path;

use workspace_status_graph::{
    Commit, GraphModel, GraphRef, Stash, SyncState, SyncStatus, Worktree,
};

use crate::git::exec_git;
use crate::snapshot::{CheckoutKind, WorkspaceRepoSnapshot, WorkspaceSnapshot};

const GRAPH_WINDOW: usize = 40;

/// Identity used to keep graph scroll when the same row is still focused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GraphIdentity {
    pub repo: String,
    pub head: String,
}

/// Load commits, stash, worktrees, and sync for `repo`.
pub fn load_graph_model(
    cwd: &Path,
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    show_ignored: bool,
) -> (GraphModel, GraphIdentity) {
    let repo_dir = cwd.join(repo);
    let Some(row) = snapshot.repos.iter().find(|r| r.repo == repo) else {
        return (
            GraphModel::default(),
            GraphIdentity {
                repo: repo.into(),
                head: String::new(),
            },
        );
    };

    let head = exec_git(&["rev-parse", "HEAD"], &repo_dir);
    let commits = load_commits(&repo_dir);
    let stashes = load_stashes(&repo_dir);
    let worktrees = load_worktrees(snapshot, row, &head, show_ignored);
    let uncommitted = row.has_unstaged || row.has_staged || row.has_untracked;
    let model = GraphModel {
        commits,
        stashes,
        worktrees,
        head_id: if head.is_empty() { None } else { Some(head.clone()) },
        sync: Some(sync_from_snapshot(row)),
        show_ignored,
        uncommitted,
    };
    (
        model,
        GraphIdentity {
            repo: repo.to_string(),
            head,
        },
    )
}

fn load_commits(repo_dir: &Path) -> Vec<Commit> {
    let raw = exec_git(
        &[
            "log",
            "--max-count=40",
            "--pretty=format:%H%x00%s%x00%P",
        ],
        repo_dir,
    );
    let refs = load_refs(repo_dir);
    raw.lines()
        .filter(|l| !l.is_empty())
        .take(GRAPH_WINDOW)
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let id = parts.next()?.to_string();
            if id.is_empty() {
                return None;
            }
            let subject = parts.next().unwrap_or("").to_string();
            let parents = parts
                .next()
                .unwrap_or("")
                .split_whitespace()
                .map(str::to_string)
                .collect();
            let commit_refs = refs
                .iter()
                .filter(|(sha, _)| sha == &id)
                .map(|(_, graph_ref)| graph_ref.clone())
                .collect();
            Some(Commit {
                id,
                subject,
                parents,
                refs: commit_refs,
            })
        })
        .collect()
}

fn classify_ref(refname: &str, short: &str) -> Option<GraphRef> {
    if short.ends_with("/HEAD") {
        return None;
    }
    if refname.starts_with("refs/heads/") {
        Some(GraphRef::local(short))
    } else if refname.starts_with("refs/remotes/") {
        Some(GraphRef::remote(short))
    } else if refname.starts_with("refs/tags/") {
        Some(GraphRef::tag(short))
    } else {
        None
    }
}

fn load_refs(repo_dir: &Path) -> Vec<(String, GraphRef)> {
    let raw = exec_git(
        &[
            "for-each-ref",
            "--format=%(objectname)%00%(refname)%00%(refname:short)",
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
        repo_dir,
    );
    raw.lines()
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let sha = parts.next()?;
            let refname = parts.next()?;
            let short = parts.next().unwrap_or("");
            let graph_ref = classify_ref(refname, short)?;
            Some((sha.to_string(), graph_ref))
        })
        .collect()
}

fn load_stashes(repo_dir: &Path) -> Vec<Stash> {
    let raw = exec_git(&["stash", "list", "--format=%gd%x00%s%x00%P"], repo_dir);
    raw.lines()
        .filter(|l| !l.is_empty())
        .filter_map(|line| {
            let mut parts = line.split('\0');
            let stash_ref = parts.next()?.to_string();
            let subject = parts.next().unwrap_or("").to_string();
            let parent = parts
                .next()
                .and_then(|p| p.split_whitespace().next())
                .map(str::to_string);
            Some(Stash {
                stash_ref,
                subject,
                parent_id: parent.filter(|s| !s.is_empty()),
            })
        })
        .collect()
}

fn load_worktrees(
    snapshot: &WorkspaceSnapshot,
    focused: &WorkspaceRepoSnapshot,
    head: &str,
    show_ignored: bool,
) -> Vec<Worktree> {
    let primary = focused.primary_repo.as_deref().unwrap_or(&focused.repo);
    snapshot
        .repos
        .iter()
        .filter(|repo| {
            repo.checkout_kind == CheckoutKind::Linked
                && repo.primary_repo.as_deref() == Some(primary)
                && (show_ignored || !repo.ignored)
        })
        .map(|repo| Worktree {
            path: repo.repo.clone(),
            head_id: if repo.repo == focused.repo && !head.is_empty() {
                Some(head.to_string())
            } else {
                None
            },
            branch: Some(repo.branch.clone()),
            ignored: repo.ignored,
            is_current: repo.repo == focused.repo,
        })
        .collect()
}

fn sync_from_snapshot(repo: &WorkspaceRepoSnapshot) -> SyncState {
    let (ahead, behind) = parse_ahead_behind(&repo.sync_note);
    SyncState {
        branch: repo.branch.clone(),
        status: match repo.sync_status {
            crate::snapshot::SyncStatus::UpToDate => SyncStatus::UpToDate,
            crate::snapshot::SyncStatus::NoUpstream => SyncStatus::NoUpstream,
            crate::snapshot::SyncStatus::Ahead => SyncStatus::Ahead,
            crate::snapshot::SyncStatus::Behind => SyncStatus::Behind,
            crate::snapshot::SyncStatus::Diverged => SyncStatus::Diverged,
        },
        ahead,
        behind,
    }
}

fn parse_ahead_behind(note: &str) -> (u32, u32) {
    let ahead = capture(note, "ahead ");
    let ahead = if ahead == 0 {
        capture(note, "ahead by ")
    } else {
        ahead
    };
    let behind = capture(note, "behind ");
    let behind = if behind == 0 {
        capture(note, "behind by ")
    } else {
        behind
    };
    (ahead, behind)
}

fn capture(note: &str, label: &str) -> u32 {
    note.split(label)
        .nth(1)
        .and_then(|s| {
            s.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .ok()
        })
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_diverged_counts() {
        assert_eq!(
            parse_ahead_behind("diverged (ahead 2, behind 1)"),
            (2, 1)
        );
        assert_eq!(parse_ahead_behind("ahead by 3 commits"), (3, 0));
        assert_eq!(parse_ahead_behind("behind by 4 commits"), (0, 4));
    }

    #[test]
    fn classify_ref_kinds() {
        assert_eq!(
            classify_ref("refs/heads/main", "main"),
            Some(GraphRef::local("main"))
        );
        assert_eq!(
            classify_ref("refs/remotes/origin/main", "origin/main"),
            Some(GraphRef::remote("origin/main"))
        );
        assert_eq!(
            classify_ref("refs/remotes/upstream/main", "upstream/main"),
            Some(GraphRef::remote("upstream/main"))
        );
        assert_eq!(
            classify_ref("refs/tags/v1.0", "v1.0"),
            Some(GraphRef::tag("v1.0"))
        );
        assert_eq!(classify_ref("refs/remotes/origin/HEAD", "origin/HEAD"), None);
    }
}

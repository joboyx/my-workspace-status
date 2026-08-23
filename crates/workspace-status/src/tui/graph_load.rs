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
        head_id: if head.is_empty() {
            None
        } else {
            Some(head.clone())
        },
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
            "--pretty=format:%H%x00%P%x00%s%x00%an%x00%at",
        ],
        repo_dir,
    );
    let refs = load_refs(repo_dir);
    raw.lines()
        .filter(|l| !l.is_empty())
        .take(GRAPH_WINDOW)
        .filter_map(|line| parse_commit_line(line, &refs))
        .collect()
}

fn parse_commit_line(line: &str, refs: &[(String, GraphRef)]) -> Option<Commit> {
    let mut parts = line.split('\0');
    let id = parts.next()?.to_string();
    if id.is_empty() {
        return None;
    }
    let parents = parts
        .next()
        .unwrap_or("")
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let subject = parts.next().unwrap_or("").to_string();
    let author_name = parts.next().unwrap_or("").to_string();
    let author_date_unix = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
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
        author_name,
        author_date_unix,
    })
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
    let raw = exec_git(
        &[
            "stash",
            "list",
            "--format=%gd%x00%H%x00%P%x00%s%x00%at%x00%an",
        ],
        repo_dir,
    );
    raw.lines()
        .filter(|l| !l.is_empty())
        .filter_map(parse_stash_line)
        .collect()
}

fn parse_stash_line(line: &str) -> Option<Stash> {
    let mut parts = line.split('\0');
    let stash_ref = parts.next()?.to_string();
    let id = parts.next()?.to_string();
    if stash_ref.is_empty() || id.is_empty() {
        return None;
    }
    let parent = parts
        .next()
        .and_then(|p| p.split_whitespace().next())
        .map(str::to_string);
    let subject = parts.next().unwrap_or("").to_string();
    let author_date_unix = parts.next().unwrap_or("0").parse::<i64>().unwrap_or(0);
    let author_name = parts.next().unwrap_or("").to_string();
    Some(Stash {
        id,
        stash_ref,
        subject,
        author_name,
        author_date_unix,
        parent_id: parent.filter(|s| !s.is_empty()),
    })
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
        assert_eq!(parse_ahead_behind("diverged (ahead 2, behind 1)"), (2, 1));
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
        assert_eq!(
            classify_ref("refs/remotes/origin/HEAD", "origin/HEAD"),
            None
        );
    }

    #[test]
    fn parse_commit_line_loads_author_and_date() {
        let line = "abc123\0parent1 parent2\0fix login\0Ada Lovelace\x001700000000";
        let commit = parse_commit_line(line, &[]).expect("commit");
        assert_eq!(commit.id, "abc123");
        assert_eq!(commit.parents, vec!["parent1", "parent2"]);
        assert_eq!(commit.subject, "fix login");
        assert_eq!(commit.author_name, "Ada Lovelace");
        assert_eq!(commit.author_date_unix, 1_700_000_000);
    }

    #[test]
    fn parse_stash_line_loads_id_date_and_author() {
        let line =
            "stash@{1}\0s1abcdef\0parentsha othersha\0WIP on main\x001700000000\0Ada Lovelace";
        let stash = parse_stash_line(line).expect("stash");
        assert_eq!(stash.stash_ref, "stash@{1}");
        assert_eq!(stash.id, "s1abcdef");
        assert_eq!(stash.parent_id.as_deref(), Some("parentsha"));
        assert_eq!(stash.subject, "WIP on main");
        assert_eq!(stash.author_date_unix, 1_700_000_000);
        assert_eq!(stash.author_name, "Ada Lovelace");
    }

    #[test]
    fn parse_stash_line_skips_missing_id() {
        assert!(parse_stash_line("stash@{0}\0\0parent\0wip\x001\0Ada").is_none());
    }
}

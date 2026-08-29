//! Local TUI comments and markdown export.
//!
//! Persist under `$XDG_STATE_HOME/my-workspace-status/comments.json`.
//! `WS_STATUS_COMMENT_STORE` overrides that path. Comments never write into
//! user git repos.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::git::{list_local_branches, rev_parse_quiet};
use crate::helpers::is_default_branch;
use crate::snapshot::{CheckoutKind, WorkspaceSnapshot};

use super::diff::DiffRow;
use super::drill::CommitFileSource;
use super::tree::{NodeKind, VisibleRow};
use super::viewed::normalize_viewed_path;
use workspace_status_graph::GraphRow;

/// On-disk store version. Unknown versions load as empty.
pub const COMMENT_STORE_VERSION: u32 = 1;

/// identity → body.
pub type CommentStore = BTreeMap<CommentKey, String>;

/// One persisted comment key.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum CommentKey {
    /// Object comment on a local branch (`repo` + branch name).
    Branch { repo: String, branch: String },
    /// Object comment on a commit (`repo` + SHA).
    Commit { repo: String, sha: String },
    /// Object comment on a linked or primary checkout path.
    Worktree { path: String },
    /// Line comment on a working-tree file diff.
    WorktreeLine {
        repo: String,
        branch: String,
        path: String,
        line: u32,
    },
    /// Line comment on a commit file diff.
    CommitLine {
        repo: String,
        sha: String,
        path: String,
        line: u32,
    },
}

/// Overlay while typing a comment.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentPrompt {
    /// Store key written on Enter.
    pub key: CommentKey,
    /// Body being edited.
    pub body: String,
    /// One-line target shown in the overlay.
    pub label: String,
}

/// Overlay that shows exported markdown after copy.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommentExport {
    /// Markdown copied to the clipboard.
    pub markdown: String,
}

/// Default JSON path. `WS_STATUS_COMMENT_STORE` wins for tests.
pub fn comment_store_path() -> PathBuf {
    comment_store_path_from_env(|key| std::env::var(key).ok())
}

/// Resolve the store path from an env lookup.
pub fn comment_store_path_from_env<F>(mut get: F) -> PathBuf
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(override_path) = get("WS_STATUS_COMMENT_STORE") {
        let trimmed = override_path.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    let state_home = get("XDG_STATE_HOME")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = get("HOME")
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/"));
            home.join(".local").join("state")
        });
    state_home.join("my-workspace-status").join("comments.json")
}

/// Repo identity for branch and commit keys (primary path when linked).
pub fn repo_identity(repo: &str, primary_repo: Option<&str>) -> String {
    normalize_viewed_path(primary_repo.unwrap_or(repo))
}

/// Upsert or delete. Empty / whitespace-only body deletes.
pub fn put_comment(store: &CommentStore, key: CommentKey, body: &str) -> CommentStore {
    let mut next = store.clone();
    let trimmed = body.trim();
    if trimmed.is_empty() {
        next.remove(&key);
    } else {
        next.insert(key, body.to_string());
    }
    next
}

#[derive(serde::Serialize, serde::Deserialize)]
struct CommentFile {
    version: u32,
    entries: Vec<CommentRecord>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CommentRecord {
    Branch {
        repo: String,
        branch: String,
        body: String,
    },
    Commit {
        repo: String,
        sha: String,
        body: String,
    },
    Worktree {
        path: String,
        body: String,
    },
    WorktreeLine {
        repo: String,
        branch: String,
        path: String,
        line: u32,
        body: String,
    },
    CommitLine {
        repo: String,
        sha: String,
        path: String,
        line: u32,
        body: String,
    },
}

impl CommentKey {
    fn into_record(self, body: String) -> CommentRecord {
        match self {
            Self::Branch { repo, branch } => CommentRecord::Branch { repo, branch, body },
            Self::Commit { repo, sha } => CommentRecord::Commit { repo, sha, body },
            Self::Worktree { path } => CommentRecord::Worktree { path, body },
            Self::WorktreeLine {
                repo,
                branch,
                path,
                line,
            } => CommentRecord::WorktreeLine {
                repo,
                branch,
                path,
                line,
                body,
            },
            Self::CommitLine {
                repo,
                sha,
                path,
                line,
            } => CommentRecord::CommitLine {
                repo,
                sha,
                path,
                line,
                body,
            },
        }
    }
}

impl CommentRecord {
    fn into_pair(self) -> Option<(CommentKey, String)> {
        let (key, body) = match self {
            Self::Branch { repo, branch, body } => (CommentKey::Branch { repo, branch }, body),
            Self::Commit { repo, sha, body } => (CommentKey::Commit { repo, sha }, body),
            Self::Worktree { path, body } => (CommentKey::Worktree { path }, body),
            Self::WorktreeLine {
                repo,
                branch,
                path,
                line,
                body,
            } => (
                CommentKey::WorktreeLine {
                    repo,
                    branch,
                    path,
                    line,
                },
                body,
            ),
            Self::CommitLine {
                repo,
                sha,
                path,
                line,
                body,
            } => (
                CommentKey::CommitLine {
                    repo,
                    sha,
                    path,
                    line,
                },
                body,
            ),
        };
        if body.trim().is_empty() {
            None
        } else {
            Some((key, body))
        }
    }
}

/// Load a comment store. Missing or malformed files become empty.
pub fn load_comment_store(file_path: &Path) -> CommentStore {
    let Ok(text) = fs::read_to_string(file_path) else {
        return CommentStore::new();
    };
    let Ok(parsed) = serde_json::from_str::<CommentFile>(&text) else {
        return CommentStore::new();
    };
    if parsed.version != COMMENT_STORE_VERSION {
        return CommentStore::new();
    }
    let mut out = CommentStore::new();
    for record in parsed.entries {
        if let Some((key, body)) = record.into_pair() {
            out.insert(key, body);
        }
    }
    out
}

/// Persist `store` as versioned JSON. Best-effort: disk errors must not crash the TUI.
pub fn save_comment_store(store: &CommentStore, file_path: &Path) {
    if let Some(parent) = file_path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let file = CommentFile {
        version: COMMENT_STORE_VERSION,
        entries: store
            .iter()
            .map(|(k, v)| k.clone().into_record(v.clone()))
            .collect(),
    };
    let Ok(mut body) = serde_json::to_string_pretty(&file) else {
        return;
    };
    body.push('\n');
    let tmp = file_path.with_extension("json.tmp");
    if let Ok(mut f) = fs::File::create(&tmp) {
        if f.write_all(body.as_bytes()).is_ok() && f.flush().is_ok() {
            let _ = fs::rename(&tmp, file_path);
            return;
        }
    }
    let _ = fs::write(file_path, &body);
    let _ = fs::remove_file(&tmp);
}

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

/// Overlay / export label for a key.
pub fn comment_key_label(key: &CommentKey) -> String {
    match key {
        CommentKey::Branch { repo, branch } => format!("{repo} · branch {branch}"),
        CommentKey::Commit { repo, sha } => {
            format!("{repo} · commit {}", short_sha(sha))
        }
        CommentKey::Worktree { path } => format!("{path} · worktree"),
        CommentKey::WorktreeLine {
            repo,
            branch,
            path,
            line,
        } => format!("{repo} · branch {branch} · {path}:{line}"),
        CommentKey::CommitLine {
            repo,
            sha,
            path,
            line,
        } => format!("{repo} · commit {} · {path}:{line}", short_sha(sha)),
    }
}

fn short_sha(sha: &str) -> &str {
    if sha.len() >= 7 {
        &sha[..7]
    } else {
        sha
    }
}

/// Markdown for live comments. Empty store → a short empty notice.
pub fn export_markdown(store: &CommentStore) -> String {
    if store.is_empty() {
        return "# Comments\n\nNo comments.\n".to_string();
    }
    let mut out = String::from("# Comments\n");
    for (key, body) in store {
        out.push('\n');
        match key {
            CommentKey::Branch { repo, branch } => {
                out.push_str(&format!("## {repo} — branch `{branch}`\n\n"));
                out.push_str(&format!("{}\n", body.trim_end()));
            }
            CommentKey::Commit { repo, sha } => {
                out.push_str(&format!("## {repo} — commit `{sha}`\n\n"));
                out.push_str(&format!("{}\n", body.trim_end()));
            }
            CommentKey::Worktree { path } => {
                out.push_str(&format!("## {path} — worktree\n\n"));
                out.push_str(&format!("{}\n", body.trim_end()));
            }
            CommentKey::WorktreeLine {
                repo,
                branch,
                path,
                line,
            } => {
                out.push_str(&format!("## {repo} — branch `{branch}`\n\n"));
                out.push_str(&format!("- `{path}`:{line} — {}\n", body.trim_end()));
            }
            CommentKey::CommitLine {
                repo,
                sha,
                path,
                line,
            } => {
                out.push_str(&format!("## {repo} — commit `{sha}`\n\n"));
                out.push_str(&format!("- `{path}`:{line} — {}\n", body.trim_end()));
            }
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
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

/// Copy `text` to the clipboard. OSC 52 when stdout is a TTY, then a host tool.
pub fn copy_to_clipboard(text: &str) -> bool {
    let mut ok = false;
    if io::stdout().is_terminal() {
        let payload = base64_encode(text.as_bytes());
        let seq = format!("\x1b]52;c;{payload}\x07");
        let mut out = io::stdout().lock();
        if out.write_all(seq.as_bytes()).is_ok() && out.flush().is_ok() {
            ok = true;
        }
    }
    for argv in [
        &["wl-copy"][..],
        &["xclip", "-selection", "clipboard"][..],
        &["pbcopy"][..],
    ] {
        if pipe_to(argv, text) {
            ok = true;
            break;
        }
    }
    ok
}

fn pipe_to(argv: &[&str], text: &str) -> bool {
    let Some((bin, args)) = argv.split_first() else {
        return false;
    };
    let Ok(mut child) = Command::new(bin)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    let ok = child
        .stdin
        .as_mut()
        .map(|stdin| stdin.write_all(text.as_bytes()).is_ok())
        .unwrap_or(false);
    child.wait().map(|s| s.success()).unwrap_or(false) && ok
}

fn base64_encode(data: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i];
        let b1 = data.get(i + 1).copied();
        let b2 = data.get(i + 2).copied();
        out.push(TABLE[(b0 >> 2) as usize] as char);
        out.push(TABLE[(((b0 & 0x03) << 4) | (b1.unwrap_or(0) >> 4)) as usize] as char);
        if b1.is_some() {
            out.push(
                TABLE[(((b1.unwrap_or(0) & 0x0f) << 2) | (b2.unwrap_or(0) >> 6)) as usize] as char,
            );
        } else {
            out.push('=');
        }
        if b2.is_some() {
            out.push(TABLE[(b2.unwrap_or(0) & 0x3f) as usize] as char);
        } else if b1.is_some() {
            out.push('=');
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::diff::{DiffCell, DiffCellKind, DiffContent, DiffRow};
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};
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
    fn store_path_prefers_override_then_xdg() {
        let _ = comment_store_path();
        assert_eq!(
            comment_store_path_from_env(|k| match k {
                "WS_STATUS_COMMENT_STORE" => Some("/tmp/comments.json".into()),
                _ => None,
            }),
            PathBuf::from("/tmp/comments.json")
        );
        assert_eq!(
            comment_store_path_from_env(|k| match k {
                "XDG_STATE_HOME" => Some("/xdg/state".into()),
                "HOME" => Some("/home/user".into()),
                _ => None,
            }),
            PathBuf::from("/xdg/state/my-workspace-status/comments.json")
        );
        assert_eq!(
            comment_store_path_from_env(|k| match k {
                "HOME" => Some("/home/user".into()),
                _ => None,
            }),
            PathBuf::from("/home/user/.local/state/my-workspace-status/comments.json")
        );
    }

    #[test]
    fn empty_body_deletes() {
        let key = CommentKey::Branch {
            repo: "app".into(),
            branch: "feature/x".into(),
        };
        let mut store = CommentStore::new();
        store = put_comment(&store, key.clone(), "note");
        assert_eq!(store.get(&key).map(String::as_str), Some("note"));
        store = put_comment(&store, key.clone(), "  ");
        assert!(store.is_empty());
    }

    #[test]
    fn load_save_round_trip_json() {
        let dir = std::env::temp_dir().join(format!(
            "ws-comments-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("comments.json");
        let key = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 2,
        };
        let store = put_comment(&CommentStore::new(), key.clone(), "wt line");
        save_comment_store(&store, &file);
        let loaded = load_comment_store(&file);
        assert_eq!(loaded.get(&key).map(String::as_str), Some("wt line"));
        assert!(load_comment_store(&dir.join("missing.json")).is_empty());
        let _ = fs::remove_dir_all(&dir);
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
    fn export_markdown_lists_live_omits_chrome() {
        let mut store = CommentStore::new();
        store = put_comment(
            &store,
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 2,
            },
            "dirty line",
        );
        store = put_comment(
            &store,
            CommentKey::Commit {
                repo: "merger".into(),
                sha: "deadbeef".into(),
            },
            "commit note",
        );
        let md = export_markdown(&store);
        assert!(md.contains("# Comments"));
        assert!(md.contains("app"));
        assert!(md.contains("branch `main`"));
        assert!(md.contains("`README.md`:2"));
        assert!(md.contains("dirty line"));
        assert!(md.contains("merger"));
        assert!(md.contains("commit `deadbeef`"));
        assert!(md.contains("commit note"));
        assert!(!md.contains("tokyo-night"));
        assert!(!md.contains("\"kind\""));
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

    #[test]
    fn base64_encode_known_vector() {
        assert_eq!(base64_encode(b"hi"), "aGk=");
        assert_eq!(base64_encode(b"hi!"), "aGkh");
    }
}

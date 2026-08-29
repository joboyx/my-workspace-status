//! Versioned JSON store and comment keys.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use super::super::viewed::normalize_viewed_path;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

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
}

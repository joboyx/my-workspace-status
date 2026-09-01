//! Versioned JSON store and comment keys.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use super::super::persist::persist_with_lock;
use super::super::viewed::normalize_viewed_path;

/// On-disk store version written by save. Load accepts `1` and `2`.
///
/// Version `1` is a flat `entries` list (the current workspace bucket).
/// Version `2` namespaces those records under `workspaces.<id>.entries`.
/// Unknown versions load as empty. `resolved` stays additive on each entry.
pub const COMMENT_STORE_VERSION: u32 = 2;

/// identity → body plus resolve state.
pub type CommentStore = BTreeMap<CommentKey, CommentEntry>;

/// One persisted comment.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommentEntry {
    /// Comment text. Empty / whitespace-only bodies do not persist.
    pub body: String,
    /// True when the operator marked this comment resolved.
    pub resolved: bool,
}

impl CommentEntry {
    /// Open (unresolved) comment with `body`.
    pub fn open(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            resolved: false,
        }
    }

    /// Body as `&str`.
    pub fn as_str(&self) -> &str {
        self.body.as_str()
    }
}

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
        /// Inclusive start (1-based).
        line: u32,
        /// Inclusive end. Equal to [`Self::WorktreeLine::line`] for one line.
        end_line: u32,
    },
    /// Line comment on a commit file diff.
    CommitLine {
        repo: String,
        sha: String,
        path: String,
        /// Inclusive start (1-based).
        line: u32,
        /// Inclusive end. Equal to [`Self::CommitLine::line`] for one line.
        end_line: u32,
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
///
/// A replace keeps the existing [`CommentEntry::resolved`] flag. New keys
/// start unresolved. Use [`put_comment_entry`] to set resolve state.
pub fn put_comment(store: &CommentStore, key: CommentKey, body: &str) -> CommentStore {
    let resolved = store.get(&key).is_some_and(|entry| entry.resolved);
    put_comment_entry(store, key, body, resolved)
}

/// Upsert or delete with an explicit resolve flag.
pub fn put_comment_entry(
    store: &CommentStore,
    key: CommentKey,
    body: &str,
    resolved: bool,
) -> CommentStore {
    let mut next = store.clone();
    let trimmed = body.trim();
    if trimmed.is_empty() {
        next.remove(&key);
    } else {
        next.insert(
            key,
            CommentEntry {
                body: body.to_string(),
                resolved,
            },
        );
    }
    next
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CommentFile {
    version: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    entries: Vec<CommentRecord>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    workspaces: BTreeMap<String, CommentWorkspaceFile>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct CommentWorkspaceFile {
    entries: Vec<CommentRecord>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
enum CommentRecord {
    Branch {
        repo: String,
        branch: String,
        body: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        resolved: bool,
    },
    Commit {
        repo: String,
        sha: String,
        body: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        resolved: bool,
    },
    Worktree {
        path: String,
        body: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        resolved: bool,
    },
    WorktreeLine {
        repo: String,
        branch: String,
        path: String,
        line: u32,
        #[serde(rename = "endLine", default, skip_serializing_if = "Option::is_none")]
        end_line: Option<u32>,
        body: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        resolved: bool,
    },
    CommitLine {
        repo: String,
        sha: String,
        path: String,
        line: u32,
        #[serde(rename = "endLine", default, skip_serializing_if = "Option::is_none")]
        end_line: Option<u32>,
        body: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        resolved: bool,
    },
}

/// Inclusive start/end with start ≤ end.
pub fn ordered_line_range(a: u32, b: u32) -> (u32, u32) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

fn record_end_line(line: u32, end_line: u32) -> Option<u32> {
    if end_line == line {
        None
    } else {
        Some(end_line)
    }
}

fn key_end_line(line: u32, end_line: Option<u32>) -> u32 {
    end_line.unwrap_or(line)
}

impl CommentKey {
    /// True when this line comment covers `n` (object keys are false).
    pub fn covers_line(&self, n: u32) -> bool {
        match self {
            Self::WorktreeLine { line, end_line, .. } | Self::CommitLine { line, end_line, .. } => {
                n >= *line && n <= *end_line
            }
            _ => false,
        }
    }

    fn into_record(self, entry: CommentEntry) -> CommentRecord {
        match self {
            Self::Branch { repo, branch } => CommentRecord::Branch {
                repo,
                branch,
                body: entry.body,
                resolved: entry.resolved,
            },
            Self::Commit { repo, sha } => CommentRecord::Commit {
                repo,
                sha,
                body: entry.body,
                resolved: entry.resolved,
            },
            Self::Worktree { path } => CommentRecord::Worktree {
                path,
                body: entry.body,
                resolved: entry.resolved,
            },
            Self::WorktreeLine {
                repo,
                branch,
                path,
                line,
                end_line,
            } => CommentRecord::WorktreeLine {
                repo,
                branch,
                path,
                line,
                end_line: record_end_line(line, end_line),
                body: entry.body,
                resolved: entry.resolved,
            },
            Self::CommitLine {
                repo,
                sha,
                path,
                line,
                end_line,
            } => CommentRecord::CommitLine {
                repo,
                sha,
                path,
                line,
                end_line: record_end_line(line, end_line),
                body: entry.body,
                resolved: entry.resolved,
            },
        }
    }
}

impl CommentRecord {
    fn into_pair(self) -> Option<(CommentKey, CommentEntry)> {
        let (key, body, resolved) = match self {
            Self::Branch {
                repo,
                branch,
                body,
                resolved,
            } => (CommentKey::Branch { repo, branch }, body, resolved),
            Self::Commit {
                repo,
                sha,
                body,
                resolved,
            } => (CommentKey::Commit { repo, sha }, body, resolved),
            Self::Worktree {
                path,
                body,
                resolved,
            } => (CommentKey::Worktree { path }, body, resolved),
            Self::WorktreeLine {
                repo,
                branch,
                path,
                line,
                end_line,
                body,
                resolved,
            } => {
                let (line, end_line) = ordered_line_range(line, key_end_line(line, end_line));
                (
                    CommentKey::WorktreeLine {
                        repo,
                        branch,
                        path,
                        line,
                        end_line,
                    },
                    body,
                    resolved,
                )
            }
            Self::CommitLine {
                repo,
                sha,
                path,
                line,
                end_line,
                body,
                resolved,
            } => {
                let (line, end_line) = ordered_line_range(line, key_end_line(line, end_line));
                (
                    CommentKey::CommitLine {
                        repo,
                        sha,
                        path,
                        line,
                        end_line,
                    },
                    body,
                    resolved,
                )
            }
        };
        if body.trim().is_empty() {
            None
        } else {
            Some((key, CommentEntry { body, resolved }))
        }
    }
}

fn records_to_store(records: Vec<CommentRecord>) -> CommentStore {
    let mut out = CommentStore::new();
    for record in records {
        if let Some((key, body)) = record.into_pair() {
            out.insert(key, body);
        }
    }
    out
}

fn store_to_records(store: &CommentStore) -> Vec<CommentRecord> {
    store
        .iter()
        .map(|(k, v)| k.clone().into_record(v.clone()))
        .collect()
}

fn load_comment_workspaces(text: &str, workspace_id: &str) -> BTreeMap<String, CommentStore> {
    let Ok(parsed) = serde_json::from_str::<CommentFile>(text) else {
        return BTreeMap::new();
    };
    match parsed.version {
        1 => {
            let mut map = BTreeMap::new();
            map.insert(workspace_id.to_string(), records_to_store(parsed.entries));
            map
        }
        2 => parsed
            .workspaces
            .into_iter()
            .map(|(id, bucket)| (id, records_to_store(bucket.entries)))
            .collect(),
        _ => BTreeMap::new(),
    }
}

fn comment_file_v2(workspaces: BTreeMap<String, CommentStore>) -> CommentFile {
    CommentFile {
        version: COMMENT_STORE_VERSION,
        entries: Vec::new(),
        workspaces: workspaces
            .into_iter()
            .map(|(id, store)| {
                (
                    id,
                    CommentWorkspaceFile {
                        entries: store_to_records(&store),
                    },
                )
            })
            .collect(),
    }
}

fn encode_comment_file(file: &CommentFile) -> io::Result<Vec<u8>> {
    let mut body = serde_json::to_vec_pretty(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    body.push(b'\n');
    Ok(body)
}

/// Load the current workspace bucket. Missing or malformed files become empty.
pub fn load_comment_store(file_path: &Path, workspace_id: &str) -> CommentStore {
    let Ok(text) = fs::read_to_string(file_path) else {
        return CommentStore::new();
    };
    load_comment_workspaces(&text, workspace_id)
        .remove(workspace_id)
        .unwrap_or_default()
}

/// Persist `store` as the current workspace bucket (version 2).
///
/// Locks a sibling `*.lock` file, keeps every other workspace bucket, then
/// atomic-writes. Disk errors return [`io::Result`].
pub fn save_comment_store(
    store: &CommentStore,
    file_path: &Path,
    workspace_id: &str,
) -> io::Result<()> {
    persist_with_lock(file_path, |existing| {
        let mut workspaces = match existing.and_then(|bytes| std::str::from_utf8(bytes).ok()) {
            Some(text) => load_comment_workspaces(text, workspace_id),
            None => BTreeMap::new(),
        };
        workspaces.insert(workspace_id.to_string(), store.clone());
        encode_comment_file(&comment_file_v2(workspaces))
    })
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
        assert_eq!(store.get(&key).map(CommentEntry::as_str), Some("note"));
        assert!(!store.get(&key).is_some_and(|e| e.resolved));
        store = put_comment_entry(&store, key.clone(), "note", true);
        store = put_comment(&store, key.clone(), "edited");
        assert_eq!(store.get(&key).map(CommentEntry::as_str), Some("edited"));
        assert!(
            store.get(&key).is_some_and(|e| e.resolved),
            "body replace keeps resolved"
        );
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
            end_line: 2,
        };
        let store = put_comment(&CommentStore::new(), key.clone(), "wt line");
        save_comment_store(&store, &file, "test-ws").unwrap();
        let loaded = load_comment_store(&file, "test-ws");
        assert_eq!(loaded.get(&key).map(CommentEntry::as_str), Some("wt line"));
        assert!(!loaded.get(&key).is_some_and(|e| e.resolved));
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            !text.contains("endLine"),
            "single-line comments omit endLine: {text}"
        );
        let range = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 1,
            end_line: 2,
        };
        let store = put_comment(&CommentStore::new(), range.clone(), "span");
        save_comment_store(&store, &file, "test-ws").unwrap();
        let loaded = load_comment_store(&file, "test-ws");
        assert_eq!(loaded.get(&range).map(CommentEntry::as_str), Some("span"));
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("\"endLine\": 2"),
            "range keeps endLine: {text}"
        );
        let legacy = file.with_file_name("legacy.json");
        fs::write(
            &legacy,
            r#"{
  "version": 1,
  "entries": [
    {
      "kind": "worktreeLine",
      "repo": "app",
      "branch": "main",
      "path": "README.md",
      "line": 3,
      "body": "old"
    }
  ]
}
"#,
        )
        .unwrap();
        let loaded = load_comment_store(&legacy, "test-ws");
        let old = CommentKey::WorktreeLine {
            repo: "app".into(),
            branch: "main".into(),
            path: "README.md".into(),
            line: 3,
            end_line: 3,
        };
        assert_eq!(loaded.get(&old).map(CommentEntry::as_str), Some("old"));
        assert!(!loaded.get(&old).is_some_and(|e| e.resolved));
        let resolved_key = CommentKey::Commit {
            repo: "app".into(),
            sha: "deadbeef".into(),
        };
        let store = put_comment_entry(&CommentStore::new(), resolved_key.clone(), "done", true);
        save_comment_store(&store, &file, "test-ws").unwrap();
        let loaded = load_comment_store(&file, "test-ws");
        assert_eq!(
            loaded.get(&resolved_key).map(CommentEntry::as_str),
            Some("done")
        );
        assert!(loaded.get(&resolved_key).is_some_and(|e| e.resolved));
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("\"resolved\": true"),
            "resolved comments persist the flag: {text}"
        );
        assert!(load_comment_store(&dir.join("missing.json"), "test-ws").is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_json(prefix: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("comments.json");
        (dir, file)
    }

    fn branch_key(branch: &str) -> CommentKey {
        CommentKey::Branch {
            repo: "app".into(),
            branch: branch.into(),
        }
    }

    #[test]
    fn gc_persist_keeps_other_workspace_comments() {
        use super::super::target::{gc_comments, CommentLiveSet};
        let (dir, file) = temp_json("ws-comments-gc");
        let key_a = branch_key("main");
        let key_b = branch_key("topic");
        let store_a = put_comment(&CommentStore::new(), key_a.clone(), "note-a");
        let store_b = put_comment(&CommentStore::new(), key_b.clone(), "note-b");
        save_comment_store(&store_a, &file, "ws-a").unwrap();
        save_comment_store(&store_b, &file, "ws-b").unwrap();
        let gced = gc_comments(&store_a, &CommentLiveSet::default());
        save_comment_store(&gced, &file, "ws-a").unwrap();
        let loaded_b = load_comment_store(&file, "ws-b");
        assert_eq!(
            loaded_b.get(&key_b).map(CommentEntry::as_str),
            Some("note-b"),
            "GC of workspace A must not drop or replace workspace B"
        );
        assert!(
            load_comment_store(&file, "ws-a").is_empty(),
            "empty live set must drop workspace A comments"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_comment_store_returns_err_when_unwritable() {
        let (dir, _) = temp_json("ws-comments-err");
        let blocker = dir.join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        let store = put_comment(&CommentStore::new(), branch_key("main"), "note");
        let result = save_comment_store(&store, &blocker.join("comments.json"), "ws-a");
        assert!(
            result.is_err(),
            "save to an unwritable path must return Err, got {result:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_version_1_comments_as_current_workspace_bucket() {
        let (dir, file) = temp_json("ws-comments-v1");
        fs::write(
            &file,
            r#"{
  "version": 1,
  "entries": [
    {
      "kind": "branch",
      "repo": "app",
      "branch": "main",
      "body": "legacy-note"
    }
  ]
}
"#,
        )
        .unwrap();
        let loaded = load_comment_store(&file, "ws-current");
        assert_eq!(
            loaded.get(&branch_key("main")).map(CommentEntry::as_str),
            Some("legacy-note"),
            "version-1 file must load as the current workspace bucket"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_comment_store_merges_other_workspace_bucket() {
        let (dir, file) = temp_json("ws-comments-merge");
        fs::write(
            &file,
            r#"{
  "version": 2,
  "workspaces": {
    "ws-b": {
      "entries": [
        {
          "kind": "branch",
          "repo": "app",
          "branch": "topic",
          "body": "note-b"
        }
      ]
    }
  }
}
"#,
        )
        .unwrap();
        let store_a = put_comment(&CommentStore::new(), branch_key("main"), "note-a");
        save_comment_store(&store_a, &file, "ws-a").unwrap();
        assert_eq!(
            load_comment_store(&file, "ws-a")
                .get(&branch_key("main"))
                .map(CommentEntry::as_str),
            Some("note-a")
        );
        assert_eq!(
            load_comment_store(&file, "ws-b")
                .get(&branch_key("topic"))
                .map(CommentEntry::as_str),
            Some("note-b"),
            "persist of workspace A must keep workspace B"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}

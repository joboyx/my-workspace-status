//! Persist viewed marks (path, identity, and fingerprint).

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::persist::{persist_with_lock, LEGACY_WORKSPACE_ID};
use crate::snapshot::{FileChange, WorkspaceSnapshot};

/// On-disk store version written by save. Load accepts `1` and `2`.
///
/// Version `1` is a flat identity → fingerprint map with no workspace id.
/// Persist keeps those records under `__legacy__`. Version `2` namespaces
/// that map under `workspaces.<id>.entries`. Load uses the current workspace
/// bucket when that key exists. Otherwise it uses the legacy map.
/// Unknown versions load as empty. Persist of a present file that is not
/// valid version-1 or version-2 UTF-8 JSON returns an error and leaves the
/// bytes unchanged.
pub const VIEWED_STORE_VERSION: u32 = 2;

/// Files larger than this hash size only (`huge:<len>`).
pub const HUGE_FILE_BYTES: u64 = 1_000_000;

/// identity → fingerprint captured at mark time.
pub type ViewedStore = BTreeMap<String, String>;

/// Default JSON path. `WS_STATUS_VIEWED_STORE` wins for tests.
pub fn viewed_store_path() -> PathBuf {
    viewed_store_path_from_env(|key| std::env::var(key).ok())
}

/// Resolve the store path from an env lookup.
pub fn viewed_store_path_from_env<F>(mut get: F) -> PathBuf
where
    F: FnMut(&str) -> Option<String>,
{
    if let Some(override_path) = get("WS_STATUS_VIEWED_STORE") {
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
    state_home
        .join("my-workspace-status")
        .join("viewed-files.json")
}

/// Normalize a repo or file path for identity keys (posix, no `./`).
pub fn normalize_viewed_path(value: &str) -> String {
    let mut out = value.replace('\\', "/");
    while out.contains("//") {
        out = out.replace("//", "/");
    }
    if let Some(stripped) = out.strip_prefix("./") {
        out = stripped.to_string();
    }
    while out.ends_with('/') && out.len() > 1 {
        out.pop();
    }
    out
}

/// Workspace identity for comment and viewed persist buckets.
///
/// SHA-256 hex of the canonical cwd path (posix-normalized). When
/// canonicalize fails, the given path is hashed instead.
pub fn workspace_store_id(cwd: &Path) -> String {
    let raw = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let posix = normalize_viewed_path(&raw.to_string_lossy());
    hex_encode(&Sha256::digest(posix.as_bytes()))
}

/// Stable identity: workspace-relative repo path + repo-relative file path.
pub fn viewed_identity(repo_path: &str, file_path: &str) -> String {
    format!(
        "{}\0{}",
        normalize_viewed_path(repo_path),
        normalize_viewed_path(file_path)
    )
}

/// Inputs hashed into a viewed fingerprint.
pub struct ViewedFingerprintInput<'a> {
    pub staged_status: Option<&'a str>,
    pub unstaged_status: Option<&'a str>,
    pub untracked: bool,
    pub old_path: Option<&'a str>,
    pub content: &'a [u8],
}

/// SHA-256 of status letters plus worktree (or supplied) content.
pub fn viewed_fingerprint(input: ViewedFingerprintInput<'_>) -> String {
    let status = format!(
        "{}\0{}\0{}\0{}",
        input.staged_status.unwrap_or(""),
        input.unstaged_status.unwrap_or(""),
        if input.untracked { "1" } else { "0" },
        input.old_path.unwrap_or(""),
    );
    let mut hasher = Sha256::new();
    hasher.update(status.as_bytes());
    hasher.update(b"\n");
    hasher.update(input.content);
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

/// True when `store` has `identity` with this exact fingerprint.
pub fn is_viewed(store: &ViewedStore, identity: &str, fingerprint: &str) -> bool {
    store.get(identity).is_some_and(|fp| fp == fingerprint)
}

/// Toggle a mark. Same identity + fingerprint unmarks; otherwise marks.
pub fn toggle_viewed(store: &ViewedStore, identity: &str, fingerprint: &str) -> ViewedStore {
    let mut next = store.clone();
    if is_viewed(store, identity, fingerprint) {
        next.remove(identity);
    } else {
        next.insert(identity.to_string(), fingerprint.to_string());
    }
    next
}

/// Drop marks whose file is gone or whose fingerprint no longer matches.
/// Returns `store` when nothing changed.
pub fn reconcile_viewed(store: &ViewedStore, current: &HashMap<String, String>) -> ViewedStore {
    let mut changed = false;
    let mut next = ViewedStore::new();
    for (identity, fingerprint) in store {
        match current.get(identity) {
            Some(now) if now == fingerprint => {
                next.insert(identity.clone(), fingerprint.clone());
            }
            _ => changed = true,
        }
    }
    if changed {
        next
    } else {
        store.clone()
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ViewedFile {
    version: u32,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    entries: BTreeMap<String, ViewedEntry>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    workspaces: BTreeMap<String, ViewedWorkspaceFile>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct ViewedWorkspaceFile {
    entries: BTreeMap<String, ViewedEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ViewedEntry {
    fingerprint: String,
}

fn entries_to_store(entries: BTreeMap<String, ViewedEntry>) -> ViewedStore {
    let mut out = ViewedStore::new();
    for (key, value) in entries {
        if !value.fingerprint.is_empty() {
            out.insert(key, value.fingerprint);
        }
    }
    out
}

fn store_to_entries(store: &ViewedStore) -> BTreeMap<String, ViewedEntry> {
    store
        .iter()
        .map(|(k, v)| {
            (
                k.clone(),
                ViewedEntry {
                    fingerprint: v.clone(),
                },
            )
        })
        .collect()
}

fn parse_viewed_workspaces(text: &str) -> io::Result<BTreeMap<String, ViewedStore>> {
    let parsed: ViewedFile = serde_json::from_str(text)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    match parsed.version {
        1 => {
            let mut map = BTreeMap::new();
            map.insert(
                LEGACY_WORKSPACE_ID.to_string(),
                entries_to_store(parsed.entries),
            );
            Ok(map)
        }
        2 => Ok(parsed
            .workspaces
            .into_iter()
            .map(|(id, bucket)| (id, entries_to_store(bucket.entries)))
            .collect()),
        version => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported viewed store version {version}"),
        )),
    }
}

fn load_viewed_workspaces(text: &str) -> BTreeMap<String, ViewedStore> {
    parse_viewed_workspaces(text).unwrap_or_default()
}

fn viewed_store_for_workspace(
    workspaces: &BTreeMap<String, ViewedStore>,
    workspace_id: &str,
) -> ViewedStore {
    workspaces
        .get(workspace_id)
        .or_else(|| workspaces.get(LEGACY_WORKSPACE_ID))
        .cloned()
        .unwrap_or_default()
}

fn viewed_file_v2(workspaces: BTreeMap<String, ViewedStore>) -> ViewedFile {
    ViewedFile {
        version: VIEWED_STORE_VERSION,
        entries: BTreeMap::new(),
        workspaces: workspaces
            .into_iter()
            .map(|(id, store)| {
                (
                    id,
                    ViewedWorkspaceFile {
                        entries: store_to_entries(&store),
                    },
                )
            })
            .collect(),
    }
}

fn encode_viewed_file(file: &ViewedFile) -> io::Result<Vec<u8>> {
    let mut body = serde_json::to_vec_pretty(file)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    body.push(b'\n');
    Ok(body)
}

/// Load the current workspace bucket. Missing or malformed files become empty.
/// When that bucket is missing, load uses `__legacy__` (version-1 records).
pub fn load_viewed_store(file_path: &Path, workspace_id: &str) -> ViewedStore {
    let Ok(text) = fs::read_to_string(file_path) else {
        return ViewedStore::new();
    };
    viewed_store_for_workspace(&load_viewed_workspaces(&text), workspace_id)
}

/// Persist `store` as the current workspace bucket (version 2).
///
/// Locks a sibling `*.lock` file, keeps every other workspace bucket
/// (including `__legacy__`), then atomic-writes. A missing file starts an
/// empty map. A present file that is not valid version-1 or version-2 UTF-8
/// JSON returns [`io::Error`] and leaves the bytes unchanged. Other disk
/// errors also return [`io::Result`].
pub fn save_viewed_store(
    store: &ViewedStore,
    file_path: &Path,
    workspace_id: &str,
) -> io::Result<()> {
    persist_with_lock(file_path, |existing| {
        let mut workspaces = match existing {
            None => BTreeMap::new(),
            Some(bytes) => {
                let text = std::str::from_utf8(bytes)
                    .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
                parse_viewed_workspaces(text)?
            }
        };
        if workspace_id == LEGACY_WORKSPACE_ID {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "reserved workspace id",
            ));
        }
        workspaces.insert(workspace_id.to_string(), store.clone());
        encode_viewed_file(&viewed_file_v2(workspaces))
    })
}

/// Worktree bytes (or `missing` / `huge:<len>`) hashed with the status token.
pub fn fingerprint_file_change(cwd: &Path, repo: &str, file: &FileChange) -> String {
    let abs = cwd.join(repo).join(&file.path);
    let content = match fs::metadata(&abs).and_then(|m| {
        if m.len() > HUGE_FILE_BYTES {
            Ok(format!("huge:{}", m.len()).into_bytes())
        } else {
            fs::read(&abs)
        }
    }) {
        Ok(bytes) => bytes,
        Err(_) => b"missing".to_vec(),
    };
    viewed_fingerprint(ViewedFingerprintInput {
        staged_status: file.staged_status.as_deref(),
        unstaged_status: file.unstaged_status.as_deref(),
        untracked: file.untracked,
        old_path: file.old_path.as_deref(),
        content: &content,
    })
}

/// Current identity → fingerprint map for every dirty file in the snapshot.
pub fn collect_current_fingerprints(
    snapshot: &WorkspaceSnapshot,
    cwd: &Path,
) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for repo in &snapshot.repos {
        for file in &repo.changes {
            let identity = viewed_identity(&repo.repo, &file.path);
            out.insert(identity, fingerprint_file_change(cwd, &repo.repo, file));
        }
    }
    out
}

/// Row ids whose stored fingerprint still matches the worktree.
pub fn viewed_row_ids(
    snapshot: &WorkspaceSnapshot,
    store: &ViewedStore,
    cwd: &Path,
) -> std::collections::HashSet<String> {
    if store.is_empty() {
        return std::collections::HashSet::new();
    }
    let current = collect_current_fingerprints(snapshot, cwd);
    let mut ids = std::collections::HashSet::new();
    for repo in &snapshot.repos {
        for file in &repo.changes {
            let identity = viewed_identity(&repo.repo, &file.path);
            if let Some(fp) = current.get(&identity) {
                if is_viewed(store, &identity, fp) {
                    ids.insert(format!("file:{}:{}", repo.repo, file.path));
                }
            }
        }
    }
    ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn identity_normalizes_repo_and_file_paths() {
        assert_eq!(
            viewed_identity("./demo/", "src\\\\a.ts"),
            viewed_identity("demo", "src/a.ts")
        );
    }

    #[test]
    fn workspace_store_id_is_stable_hex() {
        let id = workspace_store_id(Path::new("/tmp"));
        assert_eq!(id.len(), 64);
        assert!(id.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(id, workspace_store_id(Path::new("/tmp")));
    }

    #[test]
    fn fingerprint_matches_golden() {
        let fp = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: None,
            unstaged_status: Some("M"),
            untracked: false,
            old_path: None,
            content: b"x\n",
        });
        assert_eq!(
            fp,
            "67da1a55766001c9402f9ca0bcf83d79c6793d1633d3dbff4bafe342c363531e"
        );
    }

    #[test]
    fn fingerprint_changes_with_status_and_bytes() {
        let dirty = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: None,
            unstaged_status: Some("M"),
            untracked: false,
            old_path: None,
            content: b"hello\n",
        });
        let staged = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: Some("M"),
            unstaged_status: None,
            untracked: false,
            old_path: None,
            content: b"hello\n",
        });
        let changed = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: None,
            unstaged_status: Some("M"),
            untracked: false,
            old_path: None,
            content: b"two\n",
        });
        assert_ne!(dirty, staged);
        assert_ne!(dirty, changed);
        assert_eq!(
            dirty,
            viewed_fingerprint(ViewedFingerprintInput {
                staged_status: None,
                unstaged_status: Some("M"),
                untracked: false,
                old_path: None,
                content: b"hello\n",
            })
        );
    }

    #[test]
    fn toggle_and_reconcile() {
        let identity = viewed_identity("demo", "src/a.ts");
        let marked = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: None,
            unstaged_status: Some("M"),
            untracked: false,
            old_path: None,
            content: b"x\n",
        });
        let changed = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: None,
            unstaged_status: Some("M"),
            untracked: false,
            old_path: None,
            content: b"y\n",
        });
        let mut store = ViewedStore::new();
        store = toggle_viewed(&store, &identity, &marked);
        assert!(is_viewed(&store, &identity, &marked));
        store = toggle_viewed(&store, &identity, &marked);
        assert!(!is_viewed(&store, &identity, &marked));
        store = toggle_viewed(&store, &identity, &marked);
        let dropped = reconcile_viewed(&store, &HashMap::from([(identity.clone(), changed)]));
        assert!(dropped.is_empty());
        let kept = reconcile_viewed(&store, &HashMap::from([(identity.clone(), marked.clone())]));
        assert_eq!(kept, store);
        let gone = reconcile_viewed(&store, &HashMap::new());
        assert!(gone.is_empty());
    }

    #[test]
    fn viewed_store_path_ends_with_store_name() {
        assert!(viewed_store_path().ends_with("viewed-files.json"));
    }

    #[test]
    fn store_path_prefers_override_then_xdg() {
        assert_eq!(
            viewed_store_path_from_env(|k| match k {
                "WS_STATUS_VIEWED_STORE" => Some("/tmp/viewed.json".into()),
                _ => None,
            }),
            PathBuf::from("/tmp/viewed.json")
        );
        assert_eq!(
            viewed_store_path_from_env(|k| match k {
                "XDG_STATE_HOME" => Some("/xdg/state".into()),
                "HOME" => Some("/home/user".into()),
                _ => None,
            }),
            PathBuf::from("/xdg/state/my-workspace-status/viewed-files.json")
        );
        assert_eq!(
            viewed_store_path_from_env(|k| match k {
                "HOME" => Some("/home/user".into()),
                _ => None,
            }),
            PathBuf::from("/home/user/.local/state/my-workspace-status/viewed-files.json")
        );
    }

    #[test]
    fn load_save_round_trip_json() {
        let dir = std::env::temp_dir().join(format!(
            "ws-viewed-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("viewed-files.json");
        let identity = viewed_identity("demo", "src/a.ts");
        let fingerprint = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: None,
            unstaged_status: Some("M"),
            untracked: false,
            old_path: None,
            content: b"x\n",
        });
        let json = "{\n  \"version\": 1,\n  \"entries\": {\n    \"demo\\u0000src/a.ts\": {\n      \"fingerprint\": \"67da1a55766001c9402f9ca0bcf83d79c6793d1633d3dbff4bafe342c363531e\"\n    }\n  }\n}\n";
        fs::write(&file, json).unwrap();
        let loaded = load_viewed_store(&file, "test-ws");
        assert_eq!(
            loaded.get(&identity).map(String::as_str),
            Some(fingerprint.as_str())
        );
        save_viewed_store(&loaded, &file, "test-ws").unwrap();
        let again = load_viewed_store(&file, "test-ws");
        assert_eq!(again, loaded);
        assert!(load_viewed_store(&dir.join("missing.json"), "test-ws").is_empty());
        let blocker = dir.join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        let _ = save_viewed_store(&loaded, &blocker.join("viewed-files.json"), "test-ws");
        let _ = fs::remove_dir_all(&dir);
    }

    fn temp_viewed(prefix: &str) -> (PathBuf, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("viewed-files.json");
        (dir, file)
    }

    fn sample_mark(path: &str) -> (String, String) {
        let identity = viewed_identity("demo", path);
        let fingerprint = viewed_fingerprint(ViewedFingerprintInput {
            staged_status: None,
            unstaged_status: Some("M"),
            untracked: false,
            old_path: None,
            content: b"x\n",
        });
        (identity, fingerprint)
    }

    #[test]
    fn reconcile_persist_keeps_other_workspace_marks() {
        let (dir, file) = temp_viewed("ws-viewed-gc");
        let (id_a, fp_a) = sample_mark("a.ts");
        let (id_b, fp_b) = sample_mark("b.ts");
        let store_a = toggle_viewed(&ViewedStore::new(), &id_a, &fp_a);
        let store_b = toggle_viewed(&ViewedStore::new(), &id_b, &fp_b);
        save_viewed_store(&store_a, &file, "ws-a").unwrap();
        save_viewed_store(&store_b, &file, "ws-b").unwrap();
        let gced = reconcile_viewed(&store_a, &HashMap::new());
        save_viewed_store(&gced, &file, "ws-a").unwrap();
        let loaded_b = load_viewed_store(&file, "ws-b");
        assert_eq!(
            loaded_b.get(&id_b).map(String::as_str),
            Some(fp_b.as_str()),
            "reconcile of workspace A must not drop or replace workspace B"
        );
        assert!(
            load_viewed_store(&file, "ws-a").is_empty(),
            "empty current set must drop workspace A marks"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_viewed_store_returns_err_when_unwritable() {
        let (dir, _) = temp_viewed("ws-viewed-err");
        let blocker = dir.join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        let (identity, fingerprint) = sample_mark("a.ts");
        let store = toggle_viewed(&ViewedStore::new(), &identity, &fingerprint);
        let result = save_viewed_store(&store, &blocker.join("viewed-files.json"), "ws-a");
        assert!(
            result.is_err(),
            "save to an unwritable path must return Err, got {result:?}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn load_version_1_viewed_when_workspace_bucket_missing() {
        let (dir, file) = temp_viewed("ws-viewed-v1");
        let (identity, fingerprint) = sample_mark("src/a.ts");
        fs::write(
            &file,
            "{\n  \"version\": 1,\n  \"entries\": {\n    \"demo\\u0000src/a.ts\": {\n      \"fingerprint\": \"67da1a55766001c9402f9ca0bcf83d79c6793d1633d3dbff4bafe342c363531e\"\n    }\n  }\n}\n",
        )
        .unwrap();
        let loaded = load_viewed_store(&file, "ws-current");
        assert_eq!(
            loaded.get(&identity).map(String::as_str),
            Some(fingerprint.as_str()),
            "version-1 file must load when the current workspace bucket is missing"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_from_other_workspace_keeps_version_1_viewed_records() {
        let (dir, file) = temp_viewed("ws-viewed-v1-keep");
        let (id_a, fp_a) = sample_mark("a.ts");
        let (id_b, fp_b) = sample_mark("b.ts");
        fs::write(
            &file,
            format!(
                "{{\n  \"version\": 1,\n  \"entries\": {{\n    {}: {{\n      \"fingerprint\": \"{}\"\n    }}\n  }}\n}}\n",
                serde_json::to_string(&id_a).unwrap(),
                fp_a
            ),
        )
        .unwrap();
        let store_b = toggle_viewed(&ViewedStore::new(), &id_b, &fp_b);
        save_viewed_store(&store_b, &file, "ws-b").unwrap();
        let text = fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("\"__legacy__\""),
            "version-1 records must stay under the reserved legacy key: {text}"
        );
        assert_eq!(
            load_viewed_store(&file, "ws-a")
                .get(&id_a)
                .map(String::as_str),
            Some(fp_a.as_str()),
            "save from workspace B against a version-1 file must leave A-shaped records loadable"
        );
        assert_eq!(
            load_viewed_store(&file, "ws-b")
                .get(&id_b)
                .map(String::as_str),
            Some(fp_b.as_str())
        );
        let gced = reconcile_viewed(&store_b, &HashMap::new());
        save_viewed_store(&gced, &file, "ws-b").unwrap();
        assert_eq!(
            load_viewed_store(&file, "ws-a")
                .get(&id_a)
                .map(String::as_str),
            Some(fp_a.as_str()),
            "GC of workspace B must not drop version-1 records under the legacy key"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_viewed_store_merges_other_workspace_bucket() {
        let (dir, file) = temp_viewed("ws-viewed-merge");
        let (id_a, fp_a) = sample_mark("a.ts");
        let (id_b, fp_b) = sample_mark("b.ts");
        fs::write(
            &file,
            format!(
                "{{\n  \"version\": 2,\n  \"workspaces\": {{\n    \"ws-b\": {{\n      \"entries\": {{\n        {}: {{\n          \"fingerprint\": \"{}\"\n        }}\n      }}\n    }}\n  }}\n}}\n",
                serde_json::to_string(&id_b).unwrap(),
                fp_b
            ),
        )
        .unwrap();
        let store_a = toggle_viewed(&ViewedStore::new(), &id_a, &fp_a);
        save_viewed_store(&store_a, &file, "ws-a").unwrap();
        assert_eq!(
            load_viewed_store(&file, "ws-a")
                .get(&id_a)
                .map(String::as_str),
            Some(fp_a.as_str())
        );
        assert_eq!(
            load_viewed_store(&file, "ws-b")
                .get(&id_b)
                .map(String::as_str),
            Some(fp_b.as_str()),
            "persist of workspace A must keep workspace B"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_viewed_store_refuses_unreadable_existing_bytes() {
        let (dir, file) = temp_viewed("ws-viewed-junk");
        let (id_b, fp_b) = sample_mark("b.ts");
        fs::write(
            &file,
            format!(
                "{{\n  \"version\": 2,\n  \"workspaces\": {{\n    \"ws-b\": {{\n      \"entries\": {{\n        {}: {{\n          \"fingerprint\": \"{}\"\n        }}\n      }}\n    }}\n  }}\n}}\n",
                serde_json::to_string(&id_b).unwrap(),
                fp_b
            ),
        )
        .unwrap();
        fs::write(&file, "not-json").unwrap();
        let (id_a, fp_a) = sample_mark("a.ts");
        let store_a = toggle_viewed(&ViewedStore::new(), &id_a, &fp_a);
        let result = save_viewed_store(&store_a, &file, "ws-a");
        assert!(
            result.is_err(),
            "save over unreadable bytes must return Err, got {result:?}"
        );
        assert_eq!(
            fs::read_to_string(&file).unwrap(),
            "not-json",
            "unreadable existing bytes must stay unchanged"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn reconcile_on_refresh_drops_changed_fingerprint() {
        let dir = std::env::temp_dir().join(format!(
            "ws-viewed-refresh-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(dir.join("demo")).unwrap();
        fs::write(dir.join("demo/src").join("a.ts"), "x\n").ok();
        fs::create_dir_all(dir.join("demo/src")).unwrap();
        fs::write(dir.join("demo/src/a.ts"), "x\n").unwrap();
        let change = FileChange {
            path: "src/a.ts".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        };
        let snap = build_workspace_snapshot(
            &[RepoSnapshot {
                repo: "demo".into(),
                branch: "main".into(),
                sync_status: SyncStatus::NoUpstream,
                sync_note: String::new(),
                head: String::new(),
                has_unstaged: true,
                has_staged: false,
                has_untracked: false,
                changes: vec![change.clone()],
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
                merged_into_default: None,
                default_branch_override: None,
                local_branches: Vec::new(),
            }],
            &[],
            false,
            &[],
        );
        let identity = viewed_identity("demo", "src/a.ts");
        let fp = fingerprint_file_change(&dir, "demo", &change);
        let store = toggle_viewed(&ViewedStore::new(), &identity, &fp);
        assert!(viewed_row_ids(&snap, &store, &dir).contains("file:demo:src/a.ts"));
        fs::write(dir.join("demo/src/a.ts"), "y\n").unwrap();
        let current = collect_current_fingerprints(&snap, &dir);
        let next = reconcile_viewed(&store, &current);
        assert!(next.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

//! Persist viewed marks. Same path, identity, and fingerprint as the Ink TUI.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::snapshot::{FileChange, WorkspaceSnapshot};

/// On-disk store version. Unknown versions load as empty.
pub const VIEWED_STORE_VERSION: u32 = 1;

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

#[derive(serde::Serialize, serde::Deserialize)]
struct ViewedFile {
    version: u32,
    entries: BTreeMap<String, ViewedEntry>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ViewedEntry {
    fingerprint: String,
}

/// Load a viewed store. Missing or malformed files become empty.
pub fn load_viewed_store(file_path: &Path) -> ViewedStore {
    let Ok(text) = fs::read_to_string(file_path) else {
        return ViewedStore::new();
    };
    let Ok(parsed) = serde_json::from_str::<ViewedFile>(&text) else {
        return ViewedStore::new();
    };
    if parsed.version != VIEWED_STORE_VERSION {
        return ViewedStore::new();
    }
    let mut out = ViewedStore::new();
    for (key, value) in parsed.entries {
        if !value.fingerprint.is_empty() {
            out.insert(key, value.fingerprint);
        }
    }
    out
}

/// Persist `store` as versioned JSON. Best-effort: disk errors must not crash the TUI.
pub fn save_viewed_store(store: &ViewedStore, file_path: &Path) {
    if let Some(parent) = file_path.parent() {
        if fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let file = ViewedFile {
        version: VIEWED_STORE_VERSION,
        entries: store
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    ViewedEntry {
                        fingerprint: v.clone(),
                    },
                )
            })
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
    let _ = fs::write(file_path, body);
    let _ = fs::remove_file(&tmp);
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
    fn fingerprint_matches_ink_golden() {
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
        let kept = reconcile_viewed(
            &store,
            &HashMap::from([(identity.clone(), marked.clone())]),
        );
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
    fn load_save_round_trip_ink_json() {
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
        let ink = "{\n  \"version\": 1,\n  \"entries\": {\n    \"demo\\u0000src/a.ts\": {\n      \"fingerprint\": \"67da1a55766001c9402f9ca0bcf83d79c6793d1633d3dbff4bafe342c363531e\"\n    }\n  }\n}\n";
        fs::write(&file, ink).unwrap();
        let loaded = load_viewed_store(&file);
        assert_eq!(loaded.get(&identity).map(String::as_str), Some(fingerprint.as_str()));
        save_viewed_store(&loaded, &file);
        let again = load_viewed_store(&file);
        assert_eq!(again, loaded);
        assert!(load_viewed_store(&dir.join("missing.json")).is_empty());
        let blocker = dir.join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        save_viewed_store(&loaded, &blocker.join("viewed-files.json"));
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
                has_unstaged: true,
                has_staged: false,
                has_untracked: false,
                changes: vec![change.clone()],
                checkout_kind: CheckoutKind::Primary,
                primary_repo: None,
                merged_into_default: None,
                default_branch_override: None,
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

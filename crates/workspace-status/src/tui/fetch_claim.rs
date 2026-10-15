//! Workspace-scoped claim and outcome record for the scheduled background fetch.
//!
//! Every `ws` instance in one workspace shares one scheduled background
//! fetch. An instance runs it only while it holds a non-blocking lock on a
//! per-workspace claim file and the last background fetch for that workspace
//! started at least its own interval ago (a failed one still counts). When
//! the fetch ends, the claim writes the outcome (generation, failed flag,
//! start time) to one JSON file under the state directory
//! (`$XDG_STATE_HOME/my-workspace-status/background-fetch.json`), keyed by
//! [`super::viewed::workspace_store_id`]. The other instances read that
//! record to show the same outcome.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fs4::fs_std::FileExt;
use serde::{Deserialize, Serialize};

use super::persist::{persist_with_lock, try_lock_sibling};

/// On-disk store version written by save. Load accepts only this version.
const BACKGROUND_FETCH_STORE_VERSION: u32 = 1;

/// File name under the state directory.
const STORE_FILE_NAME: &str = "background-fetch.json";

/// Slack (ms) on the interval check. The tick timer restarts just before
/// the claim reads the wall clock, so the next tick can land a little short
/// of one interval after the recorded start.
const INTERVAL_TOLERANCE_MS: u64 = 1000;

/// On-disk shape:
/// `{"version": 1, "workspaces": {"<id>": {"generation": 1, "failed": false,
/// "startedUnixMs": 0}}}`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoreFile {
    version: u32,
    #[serde(default)]
    workspaces: BTreeMap<String, BackgroundFetchRecord>,
}

/// Outcome of the last completed background fetch in one workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackgroundFetchRecord {
    /// Count of completed background fetches; each finish adds one.
    pub(crate) generation: u64,
    /// True when that fetch failed for at least one repository.
    pub(crate) failed: bool,
    /// Wall-clock time the fetch was claimed (started), in Unix
    /// milliseconds. The next claim counts the interval from it.
    pub(crate) started_unix_ms: u64,
}

/// Resolve the store path from an env lookup.
///
/// `$XDG_STATE_HOME/my-workspace-status/background-fetch.json` (else
/// `$HOME/.local/state/...`). There is no env or config override.
pub(crate) fn background_fetch_store_path_from_env<F>(mut get: F) -> PathBuf
where
    F: FnMut(&str) -> Option<String>,
{
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
    state_home.join("my-workspace-status").join(STORE_FILE_NAME)
}

/// Parse `bytes` as a version-1 store.
///
/// `None` unless the bytes are a UTF-8 JSON object with the current
/// `version` and well-formed workspace entries. The object check keeps a
/// JSON array out, which serde would otherwise accept as a struct.
fn parse_store(bytes: &[u8]) -> Option<StoreFile> {
    serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .filter(serde_json::Value::is_object)
        .and_then(|value| serde_json::from_value::<StoreFile>(value).ok())
        .filter(|file| file.version == BACKGROUND_FETCH_STORE_VERSION)
}

/// Check the bytes of a present store file the way
/// [`BackgroundFetchClaim::finish`] does before a save: they must parse as a
/// version-1 store. `Err` says why a save would refuse the file and leave it
/// unchanged. Read-only.
pub(crate) fn check_background_fetch_store_bytes(bytes: &[u8]) -> Result<(), String> {
    parse_store(bytes)
        .map(|_| ())
        .ok_or_else(|| "not a background-fetch store".to_string())
}

/// Wall-clock time now in Unix milliseconds (`0` before the epoch).
///
/// The claim and the record compare times across processes, so they use
/// this clock, not [`std::time::Instant`].
pub(crate) fn unix_now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_millis() as u64)
}

/// The record for `workspace_id` in the store at `path`.
///
/// `None` when the file is missing or unreadable, is not a version-1 store,
/// or has no entry for that workspace.
pub(crate) fn load_background_fetch_record(
    path: &Path,
    workspace_id: &str,
) -> Option<BackgroundFetchRecord> {
    let bytes = fs::read(path).ok()?;
    parse_store(&bytes)?.workspaces.get(workspace_id).copied()
}

/// The locked claim file. Drop unlocks it explicitly before the close: a
/// child process forked at that moment (git, on a worker thread) still holds
/// a copy of the descriptor until its exec, and a close alone would leave the
/// lock held through that copy.
#[derive(Debug)]
struct HeldLock(fs::File);

impl Drop for HeldLock {
    fn drop(&mut self) {
        // Nothing to recover; the close below releases it at the latest.
        let _ = FileExt::unlock(&self.0);
    }
}

/// Exclusive right to run the scheduled background fetch for one workspace.
///
/// Holds the locked claim file; dropping the claim (or the process dying)
/// releases it. [`Self::finish`] records the outcome, then releases it.
#[derive(Debug)]
pub(crate) struct BackgroundFetchClaim {
    lock: HeldLock,
    store_path: PathBuf,
    workspace_id: String,
    /// Wall-clock claim time in Unix ms; [`Self::finish`] records it.
    started_unix_ms: u64,
}

/// Try to claim the scheduled background fetch for `workspace_id`.
///
/// Takes a non-blocking exclusive lock on the sibling file
/// `<path>.<workspace_id>.claim.lock` (parent directories are created).
/// `Ok(None)` when another instance holds the lock, or when the record shows
/// a start at or before `now_unix_ms` that is less than `interval_ms` old,
/// less [`INTERVAL_TOLERANCE_MS`] of timer jitter; the lock is released in
/// that case. A failed fetch counts too. A start time in the future does not
/// block the claim. Other I/O errors return `Err`. The claim keeps
/// `now_unix_ms` as its start time.
///
/// `interval_ms == 0` always claims: the caller skips the call when the
/// background fetch is off. `workspace_id` must be a
/// [`super::viewed::workspace_store_id`] hex string, because it is part of
/// the lock file name. `now_unix_ms` is wall-clock time ([`unix_now_ms`]).
pub(crate) fn try_claim_background_fetch(
    path: &Path,
    workspace_id: &str,
    interval_ms: u64,
    now_unix_ms: u64,
) -> io::Result<Option<BackgroundFetchClaim>> {
    let suffix = format!(".{workspace_id}.claim.lock");
    let Some(lock) = try_lock_sibling(path, &suffix)?.map(HeldLock) else {
        return Ok(None);
    };
    if let Some(record) = load_background_fetch_record(path, workspace_id) {
        let started = record.started_unix_ms;
        if started <= now_unix_ms
            && (now_unix_ms - started).saturating_add(INTERVAL_TOLERANCE_MS) < interval_ms
        {
            return Ok(None);
        }
    }
    Ok(Some(BackgroundFetchClaim {
        lock,
        store_path: path.to_path_buf(),
        workspace_id: workspace_id.to_string(),
        started_unix_ms: now_unix_ms,
    }))
}

impl BackgroundFetchClaim {
    /// Record the outcome of the claimed fetch, then release the claim.
    ///
    /// Under the store lock ([`persist_with_lock`]), writes generation =
    /// stored generation + 1 (or 1), `failed`, and `startedUnixMs` = the
    /// claim time for this workspace and keeps every other workspace entry.
    /// Returns the new generation. A present store file that does not parse
    /// as a version-1 store is left byte-identical and the result is an
    /// [`io::ErrorKind::InvalidData`] error. The claim is released on every
    /// path.
    pub(crate) fn finish(self, failed: bool) -> io::Result<u64> {
        let mut generation = 0;
        persist_with_lock(&self.store_path, |existing| {
            let mut file = match existing {
                None => StoreFile::default(),
                Some(bytes) => parse_store(bytes).ok_or_else(|| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        "not a background-fetch store, left unchanged",
                    )
                })?,
            };
            file.version = BACKGROUND_FETCH_STORE_VERSION;
            generation = file
                .workspaces
                .get(&self.workspace_id)
                .map_or(0, |record| record.generation)
                .saturating_add(1);
            file.workspaces.insert(
                self.workspace_id.clone(),
                BackgroundFetchRecord {
                    generation,
                    failed,
                    started_unix_ms: self.started_unix_ms,
                },
            );
            let mut body = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
            body.push(b'\n');
            Ok(body)
        })?;
        drop(self.lock);
        Ok(generation)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::unique_dir;

    const MIN: u64 = 60_000;

    fn temp_store() -> (PathBuf, PathBuf) {
        let dir = unique_dir("ws-fetch-claim");
        let path = dir.join("state").join(STORE_FILE_NAME);
        (dir, path)
    }

    fn write_record(path: &Path, workspace_id: &str, record: BackgroundFetchRecord) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut file = StoreFile {
            version: BACKGROUND_FETCH_STORE_VERSION,
            workspaces: BTreeMap::new(),
        };
        file.workspaces.insert(workspace_id.to_string(), record);
        fs::write(path, serde_json::to_vec(&file).unwrap()).unwrap();
    }

    fn started_at(started_unix_ms: u64) -> BackgroundFetchRecord {
        BackgroundFetchRecord {
            generation: 3,
            failed: false,
            started_unix_ms,
        }
    }

    #[test]
    fn fetch_claim_store_path_xdg_then_home() {
        let xdg_and_home = |k: &str| match k {
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            "HOME" => Some("/home/demo".into()),
            _ => None,
        };
        assert_eq!(
            background_fetch_store_path_from_env(xdg_and_home),
            PathBuf::from("/xdg/state/my-workspace-status/background-fetch.json"),
            "XDG beats HOME"
        );
        for blank_xdg in [None, Some(""), Some("  ")] {
            let home_only = |k: &str| match k {
                "XDG_STATE_HOME" => blank_xdg.map(str::to_string),
                "HOME" => Some("/home/demo".into()),
                _ => None,
            };
            assert_eq!(
                background_fetch_store_path_from_env(home_only),
                PathBuf::from("/home/demo/.local/state/my-workspace-status/background-fetch.json"),
                "XDG {blank_xdg:?} falls through to HOME"
            );
        }
        let ui_store_env = |k: &str| match k {
            "WS_STATUS_UI_STORE" => Some("/env/ui-state.json".into()),
            "XDG_STATE_HOME" => Some("/xdg/state".into()),
            _ => None,
        };
        assert_eq!(
            background_fetch_store_path_from_env(ui_store_env),
            PathBuf::from("/xdg/state/my-workspace-status/background-fetch.json"),
            "no store override env"
        );
    }

    #[test]
    fn fetch_claim_busy_lock_returns_none() {
        let (dir, path) = temp_store();
        let held = try_claim_background_fetch(&path, "ws-a", MIN, 10 * MIN)
            .unwrap()
            .expect("first claim wins");
        assert!(
            try_claim_background_fetch(&path, "ws-a", MIN, 10 * MIN)
                .unwrap()
                .is_none(),
            "second open of the same claim file must not win while the first holds it"
        );
        drop(held);
        assert!(
            try_claim_background_fetch(&path, "ws-a", MIN, 10 * MIN)
                .unwrap()
                .is_some(),
            "drop releases the claim"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_recent_start_returns_none_and_releases() {
        let (dir, path) = temp_store();
        let now = 100 * MIN;
        write_record(&path, "ws-a", started_at(now - 2 * MIN));
        assert!(
            try_claim_background_fetch(&path, "ws-a", 5 * MIN, now)
                .unwrap()
                .is_none(),
            "a start within the interval blocks the claim"
        );
        assert!(
            try_claim_background_fetch(&path, "ws-a", 2 * MIN, now)
                .unwrap()
                .is_some(),
            "the lock was released, and a shorter interval that has elapsed may claim"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_stale_start_returns_some() {
        let (dir, path) = temp_store();
        let now = 100 * MIN;
        write_record(&path, "ws-a", started_at(now - 5 * MIN));
        assert!(
            try_claim_background_fetch(&path, "ws-a", 5 * MIN, now)
                .unwrap()
                .is_some(),
            "a start exactly one interval old is stale"
        );
        write_record(&path, "ws-a", started_at(now - 6 * MIN));
        assert!(try_claim_background_fetch(&path, "ws-a", 5 * MIN, now)
            .unwrap()
            .is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_interval_boundary_allows_timer_jitter() {
        let (dir, path) = temp_store();
        let started = 100 * MIN;
        let interval = 5 * MIN;
        write_record(&path, "ws-a", started_at(started));
        assert!(
            try_claim_background_fetch(&path, "ws-a", interval, started + interval - 1)
                .unwrap()
                .is_some(),
            "a tick one timer period after the start, a little early, fetches"
        );
        assert!(
            try_claim_background_fetch(
                &path,
                "ws-a",
                interval,
                started + interval - INTERVAL_TOLERANCE_MS
            )
            .unwrap()
            .is_some(),
            "interval less the tolerance fetches"
        );
        assert!(
            try_claim_background_fetch(
                &path,
                "ws-a",
                interval,
                started + interval - INTERVAL_TOLERANCE_MS - 1
            )
            .unwrap()
            .is_none(),
            "one ms earlier skips"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_records_the_claim_time_not_the_finish_time() {
        let (dir, path) = temp_store();
        let started = 100 * MIN;
        let claim = try_claim_background_fetch(&path, "ws-a", 5 * MIN, started)
            .unwrap()
            .unwrap();
        assert_eq!(claim.finish(true).unwrap(), 1);
        assert_eq!(
            load_background_fetch_record(&path, "ws-a").map(|record| record.started_unix_ms),
            Some(started)
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_future_start_returns_some() {
        let (dir, path) = temp_store();
        let now = 100 * MIN;
        write_record(&path, "ws-a", started_at(now + MIN));
        assert!(
            try_claim_background_fetch(&path, "ws-a", 5 * MIN, now)
                .unwrap()
                .is_some(),
            "a start time in the future counts as stale"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_missing_record_returns_some_and_creates_dirs() {
        let (dir, path) = temp_store();
        assert!(!dir.exists());
        assert!(try_claim_background_fetch(&path, "ws-a", 5 * MIN, MIN)
            .unwrap()
            .is_some());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_workspaces_do_not_contend() {
        let (dir, path) = temp_store();
        let now = 100 * MIN;
        let _a = try_claim_background_fetch(&path, "ws-a", MIN, now)
            .unwrap()
            .expect("ws-a claims");
        let b = try_claim_background_fetch(&path, "ws-b", MIN, now)
            .unwrap()
            .expect("ws-b claims while ws-a holds its own claim");
        b.finish(false).unwrap();
        assert!(
            try_claim_background_fetch(&path, "ws-a", MIN, now)
                .unwrap()
                .is_none(),
            "ws-a is still held"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_finish_bumps_generation_and_keeps_other_workspace() {
        let (dir, path) = temp_store();
        let now = 100 * MIN;
        let other = BackgroundFetchRecord {
            generation: 7,
            failed: true,
            started_unix_ms: now - MIN,
        };
        write_record(&path, "ws-b", other);

        let claim = try_claim_background_fetch(&path, "ws-a", MIN, now)
            .unwrap()
            .unwrap();
        assert_eq!(claim.finish(true).unwrap(), 1, "first generation");
        assert_eq!(
            load_background_fetch_record(&path, "ws-a"),
            Some(BackgroundFetchRecord {
                generation: 1,
                failed: true,
                started_unix_ms: now,
            })
        );

        let later = now + 2 * MIN;
        let claim = try_claim_background_fetch(&path, "ws-a", MIN, later)
            .unwrap()
            .expect("finish released the claim");
        assert_eq!(claim.finish(false).unwrap(), 2);
        assert_eq!(
            load_background_fetch_record(&path, "ws-a"),
            Some(BackgroundFetchRecord {
                generation: 2,
                failed: false,
                started_unix_ms: later,
            })
        );
        assert_eq!(
            load_background_fetch_record(&path, "ws-b"),
            Some(other),
            "finish of ws-a keeps ws-b"
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!(
                "{{\n  \"version\": 1,\n  \"workspaces\": {{\n    \"ws-a\": {{\n      \"generation\": 2,\n      \"failed\": false,\n      \"startedUnixMs\": {later}\n    }},\n    \"ws-b\": {{\n      \"generation\": 7,\n      \"failed\": true,\n      \"startedUnixMs\": {}\n    }}\n  }}\n}}\n",
                now - MIN
            )
        );
        assert!(
            try_claim_background_fetch(&path, "ws-a", MIN, later + MIN)
                .unwrap()
                .is_some(),
            "finish released the claim"
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_invalid_store_loads_none_and_save_leaves_bytes() {
        let (dir, path) = temp_store();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let now = 100 * MIN;
        for body in [
            "not-json",
            "",
            "[1, {}]\n",
            "{\"workspaces\":{}}\n",
            "{\"version\":2,\"workspaces\":{}}\n",
            "{\"version\":1,\"workspaces\":{\"ws-a\":{\"generation\":1}}}\n",
            "{\"version\":1,\"workspaces\":[]}\n",
        ] {
            fs::write(&path, body).unwrap();
            assert_eq!(
                load_background_fetch_record(&path, "ws-a"),
                None,
                "{body:?}"
            );
            let claim = try_claim_background_fetch(&path, "ws-a", 5 * MIN, now)
                .unwrap()
                .expect("an unreadable record does not block the claim");
            let err = claim.finish(true).unwrap_err();
            assert_eq!(err.kind(), io::ErrorKind::InvalidData, "{body:?}");
            assert_eq!(fs::read_to_string(&path).unwrap(), body, "{body:?}");
            assert!(
                try_claim_background_fetch(&path, "ws-a", 5 * MIN, now)
                    .unwrap()
                    .is_some(),
                "a failed finish still releases the claim: {body:?}"
            );
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn fetch_claim_unwritable_state_dir_returns_err() {
        let dir = unique_dir("ws-fetch-claim-err");
        fs::create_dir_all(&dir).unwrap();
        let blocker = dir.join("not-a-dir");
        fs::write(&blocker, "x").unwrap();
        let result = try_claim_background_fetch(&blocker.join(STORE_FILE_NAME), "ws-a", MIN, MIN);
        assert!(result.is_err(), "got {result:?}");
        let _ = fs::remove_dir_all(dir);
    }
}

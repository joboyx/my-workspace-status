//! Live snapshot poll. `WS_STATUS_WATCH_MS=0` disables.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use super::icons::status_letter_from_change;
use super::tree::{NodeKind, TreeNode, VisibleRow};

/// Default poll period. Matches the Ink TUI.
pub const DEFAULT_WATCH_MS: u64 = 3000;
/// Faster than this spends more time in git than in the UI.
pub const MIN_WATCH_MS: u64 = 500;
/// How long a changed row stays highlighted.
pub const FLASH_MS: u64 = 800;

/// Poll period from `WS_STATUS_WATCH_MS`. `0` disables. Missing / invalid → default.
pub fn watch_interval_ms(raw: Option<&str>) -> u64 {
    let Some(raw) = raw else {
        return DEFAULT_WATCH_MS;
    };
    if raw.is_empty() {
        return DEFAULT_WATCH_MS;
    }
    let Ok(parsed) = raw.parse::<i64>() else {
        return DEFAULT_WATCH_MS;
    };
    if parsed < 0 {
        return DEFAULT_WATCH_MS;
    }
    if parsed == 0 {
        return 0;
    }
    (parsed as u64).max(MIN_WATCH_MS)
}

/// Ink `changeSignatures` disk token: `size:mtimeMs`, or `gone` when missing.
fn file_disk_token(cwd: &Path, repo: &str, rel: &str) -> String {
    let abs = cwd.join(repo).join(rel);
    match fs::metadata(&abs) {
        Ok(meta) => {
            let mtime_ms = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis())
                .unwrap_or(0);
            format!("{}:{mtime_ms}", meta.len())
        }
        Err(_) => "gone".into(),
    }
}

/// Semantic signature for one painted row.
///
/// File rows match Ink `changeSignatures`: status letter plus `size:mtimeMs`
/// (or `gone`). An in-place save of an already-modified file therefore flashes.
/// Chrome rows use path / branch / sync / change count so glyph paint does not
/// count as a semantic update.
pub fn row_signature(row: &VisibleRow, cwd: &Path) -> String {
    match row.kind {
        NodeKind::File => {
            let status = row
                .file
                .as_ref()
                .map(status_letter_from_change)
                .unwrap_or(super::icons::FileStatusLetter::M);
            let disk = match (row.repo.as_deref(), row.file.as_ref()) {
                (Some(repo), Some(file)) => file_disk_token(cwd, repo, &file.path),
                _ => "gone".into(),
            };
            format!("{}:{disk}", status.as_str())
        }
        _ => format!(
            "chrome:{}:{}:{}:{}:{}",
            row.chrome.path,
            row.chrome.branch,
            row.chrome.sync_status.map(|s| s.as_str()).unwrap_or(""),
            row.chrome.change_count,
            row.folded
        ),
    }
}

/// Signatures keyed by row id from a full (unfolded) walk so folded files
/// still participate in change detection.
pub fn tree_signatures(tree: &TreeNode, cwd: &Path) -> BTreeMap<String, String> {
    let rows = super::tree::flatten(tree, &std::collections::HashSet::new());
    rows.into_iter()
        .map(|row| {
            let sig = row_signature(&row, cwd);
            (row.id, sig)
        })
        .collect()
}

/// Ids whose signature appeared or changed. Removals are included.
/// The whole tree is not treated as one change.
pub fn changed_row_ids(
    before: &BTreeMap<String, String>,
    after: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut out = Vec::new();
    for (id, sig) in after {
        if before.get(id) != Some(sig) {
            out.push(id.clone());
        }
    }
    for id in before.keys() {
        if !after.contains_key(id) {
            out.push(id.clone());
        }
    }
    out.sort();
    out.dedup();
    out
}

/// True when `elapsed` is still inside the flash window.
pub fn flash_active(elapsed: Duration) -> bool {
    elapsed.as_millis() < u128::from(FLASH_MS)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::FileChange;
    use std::path::PathBuf;

    fn modified_file_row(repo: &str, path: &str) -> VisibleRow {
        VisibleRow {
            id: format!("file:{repo}:{path}"),
            depth: 2,
            kind: NodeKind::File,
            label: format!("M {path}"),
            repo: Some(repo.into()),
            file: Some(FileChange {
                path: path.into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }),
            ..VisibleRow::default()
        }
    }

    fn tmp_workspace(prefix: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "{prefix}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn zero_disables() {
        assert_eq!(watch_interval_ms(Some("0")), 0);
    }

    #[test]
    fn default_and_clamp() {
        assert_eq!(watch_interval_ms(None), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("")), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("-1")), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("abc")), DEFAULT_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("100")), MIN_WATCH_MS);
        assert_eq!(watch_interval_ms(Some("5000")), 5000);
    }

    #[test]
    fn only_changed_ids_flash() {
        let mut before = BTreeMap::new();
        before.insert("file:app:a".into(), "M:12:1".into());
        before.insert("file:app:b".into(), "M:12:1".into());
        before.insert("repo:app".into(), "chrome:app:false:app".into());
        let mut after = before.clone();
        after.insert("file:app:a".into(), "S:12:1".into());
        let changed = changed_row_ids(&before, &after);
        assert_eq!(changed, vec!["file:app:a".to_string()]);
    }

    #[test]
    fn identical_maps_flash_nothing() {
        let mut map = BTreeMap::new();
        map.insert("workspace".into(), "chrome:ws:false:".into());
        assert!(changed_row_ids(&map, &map).is_empty());
    }

    #[test]
    fn in_place_save_of_already_modified_file_changes_signature() {
        let cwd = tmp_workspace("ws-watch-sig");
        let rel = Path::new("demo/src");
        fs::create_dir_all(cwd.join(rel)).unwrap();
        let file = cwd.join("demo/src/main.ts");
        fs::write(&file, "a\n").unwrap();
        let row = modified_file_row("demo", "src/main.ts");

        let first = row_signature(&row, &cwd);
        assert!(first.starts_with("M:"), "{first}");
        assert!(!first.ends_with(":gone"), "{first}");

        let again = row_signature(&row, &cwd);
        assert_eq!(first, again);

        fs::write(&file, "a\nb\n").unwrap();
        let after_save = row_signature(&row, &cwd);
        assert_ne!(
            first, after_save,
            "same-letter in-place save must change the watch signature"
        );
        assert!(after_save.starts_with("M:"), "{after_save}");

        let _ = fs::remove_dir_all(&cwd);
    }

    #[test]
    fn missing_worktree_file_signs_gone() {
        let row = modified_file_row("demo", "src/gone.ts");
        let sig = row_signature(&row, Path::new("/nonexistent"));
        assert_eq!(sig, "M:gone");
    }
}

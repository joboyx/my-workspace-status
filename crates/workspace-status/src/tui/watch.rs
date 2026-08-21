//! Live snapshot poll. `WS_STATUS_WATCH_MS=0` disables.

use std::collections::BTreeMap;
use std::time::Duration;

use super::tree::{TreeNode, VisibleRow};

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

/// Semantic signature for one painted row. File rows use git letters only
/// (watch does not fetch). Chrome rows use branch / sync / child count.
pub fn row_signature(row: &VisibleRow) -> String {
    match row.kind {
        super::tree::NodeKind::File => {
            let file = row.file.as_ref();
            format!(
                "file:{}:{}:{}",
                file.and_then(|f| f.staged_status.as_deref()).unwrap_or("-"),
                file.and_then(|f| f.unstaged_status.as_deref()).unwrap_or("-"),
                file.map(|f| f.untracked).unwrap_or(false)
            )
        }
        _ => format!(
            "chrome:{}:{}:{}",
            row.label,
            row.folded,
            row.repo.as_deref().unwrap_or("")
        ),
    }
}

/// Signatures keyed by row id from a full (unfolded) walk so folded files
/// still participate in change detection.
pub fn tree_signatures(tree: &TreeNode) -> BTreeMap<String, String> {
    let rows = super::tree::flatten(tree, &std::collections::HashSet::new());
    rows.into_iter()
        .map(|row| {
            let sig = row_signature(&row);
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
        before.insert("file:app:a".into(), "file:-:M:false".into());
        before.insert("file:app:b".into(), "file:-:M:false".into());
        before.insert("repo:app".into(), "chrome:app:false:app".into());
        let mut after = before.clone();
        after.insert("file:app:a".into(), "file:M:-:false".into());
        let changed = changed_row_ids(&before, &after);
        assert_eq!(changed, vec!["file:app:a".to_string()]);
    }

    #[test]
    fn identical_maps_flash_nothing() {
        let mut map = BTreeMap::new();
        map.insert("workspace".into(), "chrome:ws:false:".into());
        assert!(changed_row_ids(&map, &map).is_empty());
    }
}

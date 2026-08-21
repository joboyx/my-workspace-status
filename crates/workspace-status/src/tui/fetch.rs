//! Background fetch period. Independent of [`super::watch`].

use crate::snapshot::WorkspaceSnapshot;

use super::ops::{op_targets, Op};

/// Default background-fetch period (5 minutes).
pub const DEFAULT_FETCH_MS: u64 = 300_000;
/// Floor when fetch is enabled.
pub const MIN_FETCH_MS: u64 = 30_000;

/// Poll period from `WS_STATUS_FETCH_MS`. `0` disables. Missing / invalid → default.
pub fn fetch_interval_ms(raw: Option<&str>) -> u64 {
    let Some(raw) = raw else {
        return DEFAULT_FETCH_MS;
    };
    if raw.is_empty() {
        return DEFAULT_FETCH_MS;
    }
    let Ok(parsed) = raw.parse::<i64>() else {
        return DEFAULT_FETCH_MS;
    };
    if parsed < 0 {
        return DEFAULT_FETCH_MS;
    }
    if parsed == 0 {
        return 0;
    }
    (parsed as u64).max(MIN_FETCH_MS)
}

/// Visible primary checkouts only. Hidden ignored stay out.
/// Linked worktrees are omitted (the timer does not use tree focus).
pub fn background_fetch_targets(
    snapshot: &WorkspaceSnapshot,
    show_ignored: bool,
) -> Vec<String> {
    op_targets(snapshot, None, show_ignored, Op::Fetch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus,
    };

    fn snap(name: &str, linked: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            has_unstaged: false,
            has_staged: false,
            has_untracked: false,
            changes: Vec::new(),
            checkout_kind: if linked {
                CheckoutKind::Linked
            } else {
                CheckoutKind::Primary
            },
            primary_repo: if linked { Some("app".into()) } else { None },
            merged_into_default: None,
            default_branch_override: None,
        }
    }

    #[test]
    fn zero_disables() {
        assert_eq!(fetch_interval_ms(Some("0")), 0);
    }

    #[test]
    fn default_and_clamp() {
        assert_eq!(fetch_interval_ms(None), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("")), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("-1")), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("nope")), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("1000")), MIN_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("600000")), 600_000);
    }

    #[test]
    fn background_targets_are_visible_primaries_only() {
        let snapshot = build_workspace_snapshot(
            &[
                snap("app", false),
                snap("app/.worktrees/feat", true),
                snap("notes", false),
            ],
            &["notes".into()],
            false,
            &[],
        );
        assert_eq!(background_fetch_targets(&snapshot, false), vec!["app"]);
        let shown = background_fetch_targets(&snapshot, true);
        assert_eq!(shown, vec!["app", "notes"]);
        assert!(!shown.iter().any(|r| r.contains("worktrees")));
    }
}

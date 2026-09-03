//! Background fetch period. Independent of [`super::watch`].
//!
//! The timer fires [`super::action::Action::FetchTick`]. Manual `f` and that
//! tick run a capped worker pool (`FETCH_CONCURRENCY` = 10;
//! `WS_STATUS_FETCH_CONCURRENCY`) so independent checkouts overlap.

use crate::snapshot::{checkout_is_hidden_ignored, WorkspaceSnapshot};

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

/// Snapshot paths the background fetch timer may touch.
///
/// Every checkout except hidden ignored, including linked worktrees.
/// When ignored repos are shown, those paths are included too.
/// Manual key `f` stays on [`super::ops::op_targets`] (focus-scoped).
pub fn background_fetch_targets(snapshot: &WorkspaceSnapshot, show_ignored: bool) -> Vec<String> {
    snapshot
        .repos
        .iter()
        .filter(|repo| show_ignored || !checkout_is_hidden_ignored(repo, &snapshot.ignored_repos))
        .map(|repo| repo.repo.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};

    fn snap(name: &str, primary: Option<&str>) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: false,
            has_staged: false,
            has_untracked: false,
            changes: Vec::new(),
            checkout_kind: if primary.is_some() {
                CheckoutKind::Linked
            } else {
                CheckoutKind::Primary
            },
            primary_repo: primary.map(str::to_string),
            merged_into_default: None,
            default_branch_override: None,
            local_branches: Vec::new(),
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
    fn background_targets_include_linked_worktrees_and_shown_ignored() {
        let snapshot = build_workspace_snapshot(
            &[
                snap("app", None),
                snap("app/.worktrees/feat", Some("app")),
                snap("notes", None),
                snap("notes/.worktrees/feat", Some("notes")),
            ],
            &["notes".into()],
            false,
            &[],
        );
        assert_eq!(
            background_fetch_targets(&snapshot, false),
            vec!["app", "app/.worktrees/feat"]
        );
        assert_eq!(
            background_fetch_targets(&snapshot, true),
            vec![
                "app",
                "app/.worktrees/feat",
                "notes",
                "notes/.worktrees/feat",
            ]
        );
    }
}

//! Background fetch period. Independent of [`super::watch`].
//!
//! The timer fires [`super::action::Action::FetchTick`]. Manual `f` and that
//! tick enqueue on the per-gitdir remote queue in [`super::effect`]
//! (cap: `WS_STATUS_FETCH_CONCURRENCY`, else the config key
//! `fetchConcurrency`, else `FETCH_CONCURRENCY` = 10) so independent gitdirs
//! overlap.

use crate::snapshot::{checkout_is_hidden_ignored, WorkspaceSnapshot};

/// Default background-fetch period (5 minutes).
pub const DEFAULT_FETCH_MS: u64 = 300_000;
/// Floor when fetch is enabled.
pub const MIN_FETCH_MS: u64 = 30_000;

/// Poll period: a valid `WS_STATUS_FETCH_MS` (`raw`, an integer >= 0) wins, then
/// the config `fetchMs` value, then [`DEFAULT_FETCH_MS`]. `0` disables. Values below
/// [`MIN_FETCH_MS`] clamp up. A missing, empty, negative, or non-numeric env
/// value falls through to the config value.
pub fn fetch_interval_ms(raw: Option<&str>, config: Option<u64>) -> u64 {
    let from_env = raw
        .and_then(|raw| raw.parse::<i64>().ok())
        .and_then(|n| u64::try_from(n).ok());
    match from_env.or(config) {
        None => DEFAULT_FETCH_MS,
        Some(0) => 0,
        Some(n) => n.max(MIN_FETCH_MS),
    }
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
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    #[test]
    fn zero_disables() {
        assert_eq!(fetch_interval_ms(Some("0"), None), 0);
    }

    #[test]
    fn default_and_clamp() {
        assert_eq!(fetch_interval_ms(None, None), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some(""), None), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("-1"), None), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("nope"), None), DEFAULT_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("1000"), None), MIN_FETCH_MS);
        assert_eq!(fetch_interval_ms(Some("600000"), None), 600_000);
    }

    #[test]
    fn fetch_interval_ms_env_beats_config_and_bad_env_falls_through() {
        assert_eq!(
            fetch_interval_ms(None, Some(120000)),
            120000,
            "config when env unset"
        );
        assert_eq!(fetch_interval_ms(None, Some(0)), 0, "config 0 disables");
        assert_eq!(
            fetch_interval_ms(None, Some(1)),
            MIN_FETCH_MS,
            "config clamps like env"
        );
        assert_eq!(
            fetch_interval_ms(Some("0"), Some(120000)),
            0,
            "valid env wins"
        );
        for bad in ["", "abc", "-1"] {
            assert_eq!(
                fetch_interval_ms(Some(bad), Some(120000)),
                120000,
                "env {bad:?}"
            );
        }
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

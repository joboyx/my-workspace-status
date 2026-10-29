//! PR poll period, failure backoff, and the stale rule. Pure functions.
//!
//! One app-wide cycle looks up the PR of every pollable checkout again
//! ([`crate::tui::state::AppState::due_pr_polls`]). The event loop fires it
//! on its own timer, independent of [`super::watch`] and [`super::fetch`].
//! The lookups run on the worker path of the badge lookups.

use std::time::Duration;

/// Poll period while any pollable PR has CI running or is ready to merge.
pub const PR_POLL_HOT_MS: u64 = 60_000;
/// Poll period otherwise.
pub const PR_POLL_CALM_MS: u64 = 180_000;
/// Longest period a failing forge host backs off to (15 minutes).
pub const PR_POLL_BACKOFF_MAX_MS: u64 = 900_000;

/// How PR polling runs. Resolved once at startup by [`pr_poll_mode`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PrPollMode {
    /// No polling (`WS_STATUS_PR_POLL_MS=0`, or `prPolling: false`). Lookups
    /// on a branch change, `r`, and a full reload still run.
    Off,
    /// [`PR_POLL_HOT_MS`] while any pollable PR has CI running or is ready
    /// to merge, else [`PR_POLL_CALM_MS`].
    #[default]
    HotCalm,
    /// Every this many ms (`WS_STATUS_PR_POLL_MS`), hot or calm.
    Every(u64),
}

/// Poll mode from `WS_STATUS_PR_POLL_MS` (`raw`) and the config `prPolling`
/// value.
///
/// A valid env value (an integer >= 0) wins: `0` is [`PrPollMode::Off`],
/// any other value is [`PrPollMode::Every`] with no floor. A missing, empty,
/// negative, or non-numeric env value falls through: config `false` is
/// [`PrPollMode::Off`], else [`PrPollMode::HotCalm`].
pub fn pr_poll_mode(raw: Option<&str>, config: Option<bool>) -> PrPollMode {
    let from_env = raw
        .and_then(|raw| raw.parse::<i64>().ok())
        .and_then(|n| u64::try_from(n).ok());
    match (from_env, config) {
        (Some(0), _) | (None, Some(false)) => PrPollMode::Off,
        (Some(ms), _) => PrPollMode::Every(ms),
        (None, _) => PrPollMode::HotCalm,
    }
}

/// Poll period in ms for `mode`; `hot` is true while any pollable PR has CI
/// running or is ready to merge. `0` means polling is off.
pub fn pr_poll_interval_ms(mode: PrPollMode, hot: bool) -> u64 {
    match mode {
        PrPollMode::Off => 0,
        PrPollMode::Every(ms) => ms,
        PrPollMode::HotCalm if hot => PR_POLL_HOT_MS,
        PrPollMode::HotCalm => PR_POLL_CALM_MS,
    }
}

/// Period of a forge host after `failures` failed polls in a row: the poll
/// period doubled once per failure, at most [`PR_POLL_BACKOFF_MAX_MS`], and
/// never shorter than `interval_ms`.
pub fn pr_poll_backoff_ms(interval_ms: u64, failures: u32) -> u64 {
    let doubled = interval_ms.saturating_mul(1u64 << failures.min(32));
    doubled.min(PR_POLL_BACKOFF_MAX_MS).max(interval_ms)
}

/// True when a PR answer of `age` is stale: older than twice the planned
/// poll period `interval_ms`. Never with polling off (`0`).
pub fn pr_answer_is_stale(age: Duration, interval_ms: u64) -> bool {
    interval_ms > 0 && age > Duration::from_millis(interval_ms.saturating_mul(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_is_hot_or_calm_by_default() {
        let mode = pr_poll_mode(None, None);
        assert_eq!(mode, PrPollMode::HotCalm);
        assert_eq!(pr_poll_interval_ms(mode, true), 60_000);
        assert_eq!(pr_poll_interval_ms(mode, false), 180_000);
        assert_eq!(pr_poll_mode(None, Some(true)), PrPollMode::HotCalm);
    }

    #[test]
    fn env_overrides_both_intervals_without_a_floor() {
        for config in [None, Some(true), Some(false)] {
            let mode = pr_poll_mode(Some("300"), config);
            assert_eq!(mode, PrPollMode::Every(300), "config {config:?}");
            assert_eq!(pr_poll_interval_ms(mode, true), 300);
            assert_eq!(pr_poll_interval_ms(mode, false), 300);
        }
    }

    #[test]
    fn env_zero_or_config_false_turns_polling_off() {
        for config in [None, Some(true), Some(false)] {
            let mode = pr_poll_mode(Some("0"), config);
            assert_eq!(mode, PrPollMode::Off, "env 0, config {config:?}");
        }
        let off = pr_poll_mode(None, Some(false));
        assert_eq!(off, PrPollMode::Off, "config false");
        assert_eq!(pr_poll_interval_ms(off, true), 0);
        assert_eq!(pr_poll_interval_ms(off, false), 0);
    }

    #[test]
    fn a_bad_env_value_falls_through_to_config() {
        for bad in ["", " ", "abc", "-1", "1.5", " 300"] {
            assert_eq!(
                pr_poll_mode(Some(bad), None),
                PrPollMode::HotCalm,
                "env {bad:?}"
            );
            assert_eq!(
                pr_poll_mode(Some(bad), Some(false)),
                PrPollMode::Off,
                "env {bad:?}, config false"
            );
        }
    }

    #[test]
    fn backoff_doubles_per_failure_up_to_fifteen_minutes() {
        let steps: Vec<u64> = (0..6).map(|n| pr_poll_backoff_ms(60_000, n)).collect();
        assert_eq!(
            steps,
            vec![60_000, 120_000, 240_000, 480_000, 900_000, 900_000]
        );
        assert_eq!(pr_poll_backoff_ms(180_000, 1), 360_000);
        assert_eq!(pr_poll_backoff_ms(180_000, 3), 900_000, "capped");
        assert_eq!(pr_poll_backoff_ms(300, 2), 1_200);
        assert_eq!(pr_poll_backoff_ms(300, 200), 900_000, "no overflow");
        assert_eq!(
            pr_poll_backoff_ms(1_200_000, 1),
            1_200_000,
            "never shorter than the poll period"
        );
    }

    #[test]
    fn an_answer_is_stale_after_twice_the_planned_interval() {
        let ms = Duration::from_millis;
        assert!(!pr_answer_is_stale(ms(120_000), 60_000), "exactly 2x");
        assert!(pr_answer_is_stale(ms(120_001), 60_000));
        assert!(!pr_answer_is_stale(ms(359_000), 180_000));
        assert!(pr_answer_is_stale(ms(361_000), 180_000));
        assert!(
            !pr_answer_is_stale(ms(u64::MAX / 2), 0),
            "never stale with polling off"
        );
    }
}

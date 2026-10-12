//! Commit-activity sparkline on tree repo rows.
//!
//! A worker counts the commits on each repo's local branches over the last
//! [`ACTIVITY_DAYS`] days ([`load_activity`], one `git log` per repo). The
//! tree paints those day counts as a short bar run (`▁▂▅▇`) at the end of
//! the repo row. The job never runs on the draw / event thread: the loop
//! queues it after the first frame, after a Pull wave, and on a full
//! reload (`r`).

use std::path::Path;

use super::tree::{NodeKind, VisibleRow};
use crate::snapshot::WorkspaceSnapshot;

/// Days one sparkline covers, today included.
pub const ACTIVITY_DAYS: usize = 14;

/// Narrowest tree pane (columns) that paints a sparkline. A narrower pane
/// keeps the room for the row label.
pub const SPARK_MIN_TREE_COLS: usize = 48;

/// Bar glyphs, lowest to highest.
pub const SPARK_BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// ASCII-glyph bars, lowest to highest.
pub const SPARK_BARS_ASCII: [char; 8] = ['_', '.', ',', ':', '-', '=', '+', '#'];

/// Seconds in one bucket.
const DAY_SECS: i64 = 86_400;

/// Commit counts per day: index `ACTIVITY_DAYS - 1` is the last 24 hours
/// before `now_unix`, each lower index one whole day further back.
///
/// Times older than the window are dropped. A time after `now_unix`
/// (clock skew) counts as today.
pub fn bucket_days(times: &[i64], now_unix: i64) -> [u32; ACTIVITY_DAYS] {
    let mut buckets = [0u32; ACTIVITY_DAYS];
    for &time in times {
        let back = (now_unix - time).max(0) / DAY_SECS;
        let Ok(back) = usize::try_from(back) else {
            continue;
        };
        if back < ACTIVITY_DAYS {
            buckets[ACTIVITY_DAYS - 1 - back] += 1;
        }
    }
    buckets
}

/// One bar per day, scaled to the busiest day of this repo.
///
/// A day with no commits paints the lowest bar; any commit lifts a day at
/// least one level. `None` when every day is zero, so a quiet repo paints
/// nothing.
pub fn sparkline(buckets: &[u32; ACTIVITY_DAYS], ascii: bool) -> Option<String> {
    let max = buckets.iter().copied().max().unwrap_or(0);
    if max == 0 {
        return None;
    }
    let bars = if ascii { SPARK_BARS_ASCII } else { SPARK_BARS };
    let top = (bars.len() - 1) as u64;
    let max = u64::from(max);
    Some(
        buckets
            .iter()
            .map(|&count| {
                // Round up so one commit never looks like zero.
                let level = (u64::from(count) * top).div_ceil(max);
                bars[usize::try_from(level).unwrap_or(0).min(bars.len() - 1)]
            })
            .collect(),
    )
}

/// Activity key of one snapshot checkout: its primary checkout, else the
/// checkout itself. Linked worktrees share refs with their primary, so a
/// family counts once.
fn activity_key(primary_repo: Option<&str>, repo: &str) -> String {
    primary_repo.unwrap_or(repo).to_string()
}

/// Activity key of a tree repo row (`None` for every other row kind).
///
/// A family row's `repo` is the primary; a single-checkout row uses its
/// `primary_repo` when it has one, the same key [`activity_targets`] uses.
pub fn row_activity_key(row: &VisibleRow) -> Option<&str> {
    if row.kind != NodeKind::Repo {
        return None;
    }
    row.primary_repo.as_deref().or(row.repo.as_deref())
}

/// Activity keys to count, once each, in snapshot order.
///
/// `only` limits the list to the families of those checkout paths (the
/// repos a Pull wave touched); `None` takes every repo in the snapshot.
pub fn activity_targets(snapshot: &WorkspaceSnapshot, only: Option<&[String]>) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for row in &snapshot.repos {
        if only.is_some_and(|only| !only.contains(&row.repo)) {
            continue;
        }
        let key = activity_key(row.primary_repo.as_deref(), &row.repo);
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// Day counts for each key, read one repo after another under `cwd`.
///
/// Blocking: one `git log` per repo. Worker only, never on the loop thread.
pub fn load_activity(
    cwd: &Path,
    checkouts: Vec<String>,
    now_unix: i64,
) -> Vec<(String, [u32; ACTIVITY_DAYS])> {
    checkouts
        .into_iter()
        .map(|key| {
            let days = u32::try_from(ACTIVITY_DAYS).unwrap_or(u32::MAX);
            let times = crate::git::commit_times_since(&cwd.join(&key), days);
            let buckets = bucket_days(&times, now_unix);
            (key, buckets)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    #[test]
    fn bucket_days_puts_today_last_and_drops_older() {
        let times = [
            NOW,                     // today
            NOW - 10,                // today
            NOW + 30,                // skew: today
            NOW - DAY_SECS,          // one day back
            NOW - 13 * DAY_SECS - 1, // oldest kept day
            NOW - 14 * DAY_SECS,     // dropped
            NOW - 40 * DAY_SECS,     // dropped
        ];
        let buckets = bucket_days(&times, NOW);
        assert_eq!(buckets[ACTIVITY_DAYS - 1], 3);
        assert_eq!(buckets[ACTIVITY_DAYS - 2], 1);
        assert_eq!(buckets[0], 1);
        assert_eq!(buckets.iter().sum::<u32>(), 5);
    }

    #[test]
    fn sparkline_scales_to_eight_levels() {
        let mut buckets = [0u32; ACTIVITY_DAYS];
        for (i, slot) in buckets.iter_mut().take(8).enumerate() {
            *slot = i as u32 * 3;
        }
        let line = sparkline(&buckets, false).expect("activity");
        assert_eq!(line.chars().count(), ACTIVITY_DAYS);
        assert!(line.starts_with("▁▂▃▄▅▆▇█"), "{line}");
        assert!(line.ends_with("▁▁▁▁▁▁"), "{line}");

        // Scaled to the repo's own max: one commit at max 1 is a full bar.
        let mut one = [0u32; ACTIVITY_DAYS];
        one[ACTIVITY_DAYS - 1] = 1;
        assert!(sparkline(&one, false).unwrap().ends_with('█'));

        // Any commit lifts a day above zero.
        let mut few = [0u32; ACTIVITY_DAYS];
        few[0] = 1;
        few[1] = 100;
        let line: Vec<char> = sparkline(&few, false).unwrap().chars().collect();
        assert_eq!(line[0], '▂');
        assert_eq!(line[1], '█');
    }

    #[test]
    fn sparkline_ascii_fallback_uses_ascii_bars() {
        let mut buckets = [0u32; ACTIVITY_DAYS];
        for (i, slot) in buckets.iter_mut().take(8).enumerate() {
            *slot = i as u32;
        }
        let line = sparkline(&buckets, true).expect("activity");
        assert!(line.is_ascii(), "{line}");
        assert!(line.starts_with("_.,:-=+#"), "{line}");
        assert!(line.chars().all(|c| SPARK_BARS_ASCII.contains(&c)));
    }

    #[test]
    fn all_zero_activity_paints_nothing() {
        assert_eq!(sparkline(&[0; ACTIVITY_DAYS], false), None);
        assert_eq!(sparkline(&[0; ACTIVITY_DAYS], true), None);
        assert_eq!(bucket_days(&[], NOW), [0; ACTIVITY_DAYS]);
    }
}

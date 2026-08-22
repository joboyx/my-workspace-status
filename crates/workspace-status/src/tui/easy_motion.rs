//! EasyMotion labels and prefix resolve (Ink `easyMotion.ts`).
//!
//! Labels cover the painted viewport only: `a`–`z`, then `aa`, `ab`, …

const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyz";

/// How a typed prefix compares to assigned labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EasyMotionResolve {
    Partial,
    Hit { index: usize },
    Miss,
}

/// Labels `a`–`z`, then `aa`, `ab`, … enough for `count`.
pub fn easy_motion_labels(count: usize) -> Vec<String> {
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        if i < 26 {
            out.push((ALPHA[i] as char).to_string());
            continue;
        }
        let n = i - 26;
        let first = n / 26;
        let second = n % 26;
        out.push(format!("{}{}", ALPHA[first] as char, ALPHA[second] as char));
    }
    out
}

/// Resolve typed prefix against assigned labels.
pub fn resolve_easy_motion_label(labels: &[String], typed: &str) -> EasyMotionResolve {
    if typed.is_empty() {
        return EasyMotionResolve::Partial;
    }
    if let Some(index) = labels.iter().position(|l| l == typed) {
        return EasyMotionResolve::Hit { index };
    }
    if labels.iter().any(|l| l.starts_with(typed)) {
        return EasyMotionResolve::Partial;
    }
    EasyMotionResolve::Miss
}

/// Resolve a typed prefix against the painted visible window.
///
/// On hit, `index` is the absolute list index (`start + label slot`).
pub fn resolve_easy_motion_jump(
    visible_count: usize,
    start: usize,
    typed: &str,
) -> EasyMotionResolve {
    match resolve_easy_motion_label(&easy_motion_labels(visible_count), typed) {
        EasyMotionResolve::Hit { index } => EasyMotionResolve::Hit {
            index: start + index,
        },
        other => other,
    }
}

/// First visible index, centred on `cursor` (Ink `treeViewportStart`).
pub fn list_viewport_start(row_count: usize, cursor: usize, height: usize) -> usize {
    let view_height = height.max(1);
    let max_start = row_count.saturating_sub(view_height);
    let ideal = cursor.saturating_sub(view_height / 2);
    ideal.min(max_start)
}

/// Painted window: `(start, visible_count)`.
pub fn visible_window(row_count: usize, cursor: usize, height: usize) -> (usize, usize) {
    let view_height = height.max(1);
    let start = list_viewport_start(row_count, cursor, view_height);
    let count = row_count.saturating_sub(start).min(view_height);
    (start, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_a_z_then_aa() {
        let labels = easy_motion_labels(28);
        assert_eq!(labels[0], "a");
        assert_eq!(labels[25], "z");
        assert_eq!(labels[26], "aa");
        assert_eq!(labels[27], "ab");
    }

    #[test]
    fn resolve_hit_partial_miss() {
        let labels = easy_motion_labels(28);
        assert_eq!(
            resolve_easy_motion_label(&labels, "a"),
            EasyMotionResolve::Hit { index: 0 }
        );
        assert_eq!(
            resolve_easy_motion_label(&labels, "aa"),
            EasyMotionResolve::Hit { index: 26 }
        );
        assert_eq!(
            resolve_easy_motion_label(&labels, "q"),
            EasyMotionResolve::Hit { index: 16 }
        );
        assert_eq!(
            resolve_easy_motion_label(&["aa".into(), "ab".into()], "a"),
            EasyMotionResolve::Partial
        );
        assert_eq!(
            resolve_easy_motion_label(&labels, "zz"),
            EasyMotionResolve::Miss
        );
    }

    #[test]
    fn jump_is_viewport_relative() {
        assert_eq!(
            resolve_easy_motion_jump(20, 30, "a"),
            EasyMotionResolve::Hit { index: 30 }
        );
        assert_eq!(
            resolve_easy_motion_jump(20, 30, "b"),
            EasyMotionResolve::Hit { index: 31 }
        );
        assert_eq!(
            resolve_easy_motion_jump(28, 10, "aa"),
            EasyMotionResolve::Hit { index: 36 }
        );
        assert_eq!(resolve_easy_motion_jump(28, 0, "zz"), EasyMotionResolve::Miss);
    }

    #[test]
    fn viewport_centres_on_cursor() {
        assert_eq!(visible_window(100, 50, 10), (45, 10));
        assert_eq!(visible_window(8, 7, 20), (0, 8));
        assert_eq!(visible_window(40, 39, 2), (38, 2));
    }
}

//! Current-line blame annotation text.
//!
//! [`crate::git::blame_line`] reads who last changed a line; this module
//! turns that into the short end-of-line text the diff and file panes
//! paint: `{author}, {age} · {sha7} · {subject}`.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::git::LineBlame;

/// Narrowest annotation worth painting. Below this the pane paints none.
const MIN_ANNOTATION_COLS: usize = 12;

/// Short age of `then_unix` at `now_unix`: `just now`, `5m ago`, `3h ago`,
/// `3d ago`, `2mo ago` (30 days and up), `1y ago` (365 days and up).
///
/// A time in the future reads `just now`.
pub fn relative_age(now_unix: i64, then_unix: i64) -> String {
    let secs = now_unix.saturating_sub(then_unix);
    let days = secs / 86_400;
    if secs < 60 {
        "just now".into()
    } else if secs < 3_600 {
        format!("{}m ago", secs / 60)
    } else if days < 1 {
        format!("{}h ago", secs / 3_600)
    } else if days < 30 {
        format!("{days}d ago")
    } else if days < 365 {
        format!("{}mo ago", days / 30)
    } else {
        format!("{}y ago", days / 365)
    }
}

/// Annotation for one blamed line at `now_unix`.
///
/// `{author}, {age} · {sha7} · {summary}`, or `You · uncommitted` when
/// the line is not committed yet.
pub fn annotation_text(blame: &LineBlame, now_unix: i64) -> String {
    if blame.uncommitted {
        return "You · uncommitted".into();
    }
    let sha7: String = blame.sha.chars().take(7).collect();
    format!(
        "{}, {} · {sha7} · {}",
        blame.author,
        relative_age(now_unix, blame.author_time),
        blame.summary
    )
}

/// `text` cut to `max_cols` display columns, ending in `…` when cut.
///
/// The subject is last, so a cut shortens it first. `None` when fewer
/// than 12 columns are free.
pub fn fit_annotation(text: &str, max_cols: usize) -> Option<String> {
    if max_cols < MIN_ANNOTATION_COLS {
        return None;
    }
    if text.width() <= max_cols {
        return Some(text.to_string());
    }
    let mut out = String::new();
    let mut used = 0;
    for ch in text.chars() {
        let w = ch.width().unwrap_or(0);
        if used + w > max_cols - 1 {
            break;
        }
        out.push(ch);
        used += w;
    }
    out.push('…');
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn committed(author_time: i64, summary: &str) -> LineBlame {
        LineBlame {
            sha: "0123456789abcdef0123456789abcdef01234567".into(),
            author: "Ada".into(),
            author_time,
            summary: summary.into(),
            orig_line: 3,
            filename: "src/lib.rs".into(),
            previous: None,
            boundary: false,
            uncommitted: false,
        }
    }

    #[test]
    fn relative_age_steps_through_each_unit() {
        let ago = |secs: i64| relative_age(NOW, NOW - secs);
        assert_eq!(ago(0), "just now");
        assert_eq!(ago(59), "just now");
        assert_eq!(ago(-3_600), "just now", "future time");
        assert_eq!(ago(5 * 60), "5m ago");
        assert_eq!(ago(3 * 3_600), "3h ago");
        assert_eq!(ago(3 * 86_400), "3d ago");
        assert_eq!(ago(29 * 86_400), "29d ago");
        assert_eq!(ago(30 * 86_400), "1mo ago");
        assert_eq!(ago(64 * 86_400), "2mo ago");
        assert_eq!(ago(365 * 86_400), "1y ago");
        assert_eq!(ago(800 * 86_400), "2y ago");
    }

    #[test]
    fn annotation_text_joins_author_age_sha_and_subject() {
        let blame = committed(NOW - 2 * 86_400, "fix the parser");
        assert_eq!(
            annotation_text(&blame, NOW),
            "Ada, 2d ago · 0123456 · fix the parser"
        );
        let uncommitted = LineBlame {
            sha: "0".repeat(40),
            uncommitted: true,
            ..blame
        };
        assert_eq!(annotation_text(&uncommitted, NOW), "You · uncommitted");
    }

    #[test]
    fn fit_annotation_cuts_by_display_width() {
        let text = "Ada, 2d ago · 0123456 · fix the parser";
        assert_eq!(fit_annotation(text, 80).as_deref(), Some(text));
        assert_eq!(
            fit_annotation(text, 30).as_deref(),
            Some("Ada, 2d ago · 0123456 · fix t…")
        );
        // Wide glyphs count two columns; the cut never splits one.
        let wide = "Ada, 2d ago · 0123456 · 修正する";
        let cut = fit_annotation(wide, 28).expect("fits");
        assert_eq!(cut, "Ada, 2d ago · 0123456 · 修…");
        assert!(cut.width() <= 28);
    }

    #[test]
    fn fit_annotation_is_none_below_twelve_columns() {
        assert_eq!(fit_annotation("You · uncommitted", 11), None);
        assert_eq!(
            fit_annotation("You · uncommitted", 12).as_deref(),
            Some("You · uncom…")
        );
    }
}

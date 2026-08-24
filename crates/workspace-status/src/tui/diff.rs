//! File-diff load, unified-diff parse, and numbered rows.
//!
//! Path header, line-number gutter, and STAGED / UNSTAGED / NEW labels.
//! Intra-line / syntax highlight stays out of scope.

use std::path::Path;

use crate::git::{exec_git, git_diff_args};
use crate::snapshot::FileChange;

use super::split::DiffMode;

/// Stub huge / binary untracked files above ~1 MB.
const HUGE_FILE_BYTES: u64 = 1_000_000;

/// Gutter rule between line numbers and the sign.
pub const DIFF_RULE: char = '│';

/// Staged / unstaged / untracked section label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSection {
    Staged,
    Unstaged,
    New,
}

/// Cell kind for one side of a diff row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffCellKind {
    Add,
    Del,
    Ctx,
    Meta,
    Empty,
}

/// One side of a painted diff line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffCell {
    pub kind: DiffCellKind,
    pub text: String,
    /// 1-based line number in the gutter (`None` for meta / empty).
    pub line_no: Option<u32>,
}

/// One painted body row (section, hunk header, or inline / split line).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DiffRow {
    Section(DiffSection),
    Hunk {
        text: String,
    },
    Line {
        left: DiffCell,
        right: Option<DiffCell>,
    },
}

/// Parsed unified-diff line.
#[derive(Clone, Debug, PartialEq, Eq)]
struct ParsedLine {
    kind: DiffCellKind,
    text: String,
    old_no: Option<u32>,
    new_no: Option<u32>,
}

/// One `@@` hunk.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Hunk {
    header: String,
    lines: Vec<ParsedLine>,
}

/// Staged + unstaged unified text for one file.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffContent {
    pub staged: String,
    pub unstaged: String,
    /// Label the unstaged section `NEW` (untracked synthesised as all-add).
    pub is_new: bool,
}

impl DiffContent {
    /// Unstaged-only unified text (commit / stash diffs and tests).
    pub fn from_unified(text: impl Into<String>) -> Self {
        Self {
            staged: String::new(),
            unstaged: text.into(),
            is_new: false,
        }
    }

    /// Join raw git lines into unstaged-only content.
    pub fn from_lines(lines: Vec<String>) -> Self {
        Self::from_unified(lines.join("\n"))
    }

    /// True when both slots are empty or whitespace.
    #[allow(dead_code)]
    pub fn is_blank(&self) -> bool {
        self.staged.trim().is_empty() && self.unstaged.trim().is_empty()
    }
}

/// Load a unified diff for one dirty file. Untracked files synthesise an all-add hunk.
/// `context` of `Some(n)` adds `-Un`.
pub fn load_file_diff(
    cwd: &Path,
    repo: &str,
    change: &FileChange,
    context: Option<u32>,
) -> DiffContent {
    let repo_dir = cwd.join(repo);
    if change.untracked {
        return untracked_content(&repo_dir, &change.path);
    }
    let mut staged = String::new();
    let mut unstaged = String::new();
    if change.staged_status.is_some() {
        let args = git_diff_args(&["diff", "--cached"], &change.path, context);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        staged = exec_git(&refs, &repo_dir);
    }
    if change.unstaged_status.is_some() {
        let args = git_diff_args(&["diff"], &change.path, context);
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        unstaged = exec_git(&refs, &repo_dir);
    }
    DiffContent {
        staged,
        unstaged,
        is_new: false,
    }
}

fn untracked_content(repo_dir: &Path, path: &str) -> DiffContent {
    let unstaged = read_untracked_as_diff(&repo_dir.join(path), path);
    let is_new = !unstaged.is_empty();
    DiffContent {
        staged: String::new(),
        unstaged,
        is_new,
    }
}

/// Read an untracked worktree file as a unified diff body.
fn read_untracked_as_diff(abs: &Path, rel_path: &str) -> String {
    let Ok(meta) = std::fs::metadata(abs) else {
        return String::new();
    };
    if !meta.is_file() {
        return String::new();
    }
    if meta.len() > HUGE_FILE_BYTES {
        return binary_stub(rel_path);
    }
    let Ok(buf) = std::fs::read(abs) else {
        return String::new();
    };
    if buf.contains(&0) {
        return binary_stub(rel_path);
    }
    let text = String::from_utf8_lossy(&buf);
    synthesize_all_add_diff(&text)
}

fn binary_stub(rel_path: &str) -> String {
    format!("Binary files /dev/null and b/{rel_path} differ\n")
}

/// Build a unified all-add diff from file text (no file headers).
fn synthesize_all_add_diff(text: &str) -> String {
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut raw: Vec<&str> = normalized.split('\n').collect();
    if raw.last() == Some(&"") {
        raw.pop();
    }
    if raw.is_empty() {
        return "@@ -0,0 +0,0 @@\n".into();
    }
    let mut out = format!("@@ -0,0 +1,{} @@\n", raw.len());
    for line in raw {
        out.push('+');
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// Parse a unified diff into hunks. File headers are skipped until the first `@@`.
fn parse_unified_diff(text: &str) -> Vec<Hunk> {
    if text.trim().is_empty() {
        return Vec::new();
    }
    let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
    let mut raw_lines: Vec<&str> = normalized.split('\n').collect();
    if raw_lines.last() == Some(&"") {
        raw_lines.pop();
    }

    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<usize> = None;
    let mut old_no: u32 = 0;
    let mut new_no: u32 = 0;

    for line in raw_lines {
        if is_binary_marker(line) {
            hunks.push(Hunk {
                header: String::new(),
                lines: vec![ParsedLine {
                    kind: DiffCellKind::Meta,
                    text: line.to_string(),
                    old_no: None,
                    new_no: None,
                }],
            });
            current = None;
            continue;
        }

        if line.starts_with("@@") {
            let (old_start, new_start) = hunk_starts(line);
            old_no = old_start;
            new_no = new_start;
            hunks.push(Hunk {
                header: line.to_string(),
                lines: Vec::new(),
            });
            current = Some(hunks.len() - 1);
            continue;
        }

        let Some(idx) = current else {
            continue;
        };
        let hunk = &mut hunks[idx];

        if line.starts_with('\\') {
            hunk.lines.push(ParsedLine {
                kind: DiffCellKind::Meta,
                text: line.to_string(),
                old_no: None,
                new_no: None,
            });
            continue;
        }

        if let Some(text) = line.strip_prefix('+') {
            hunk.lines.push(ParsedLine {
                kind: DiffCellKind::Add,
                text: text.to_string(),
                old_no: None,
                new_no: Some(new_no),
            });
            new_no = new_no.saturating_add(1);
            continue;
        }
        if let Some(text) = line.strip_prefix('-') {
            hunk.lines.push(ParsedLine {
                kind: DiffCellKind::Del,
                text: text.to_string(),
                old_no: Some(old_no),
                new_no: None,
            });
            old_no = old_no.saturating_add(1);
            continue;
        }
        if line.starts_with(' ') || line.is_empty() {
            let text = line.strip_prefix(' ').unwrap_or(line);
            hunk.lines.push(ParsedLine {
                kind: DiffCellKind::Ctx,
                text: text.to_string(),
                old_no: Some(old_no),
                new_no: Some(new_no),
            });
            old_no = old_no.saturating_add(1);
            new_no = new_no.saturating_add(1);
            continue;
        }

        hunk.lines.push(ParsedLine {
            kind: DiffCellKind::Meta,
            text: line.to_string(),
            old_no: None,
            new_no: None,
        });
    }

    hunks
}

fn is_binary_marker(line: &str) -> bool {
    line.starts_with("Binary files ") && line.ends_with(" differ")
}

/// `@@ -OLD[,n] +NEW[,n] @@` → start line numbers. Garbage headers yield `(0, 0)`.
fn hunk_starts(line: &str) -> (u32, u32) {
    let Some(rest) = line.strip_prefix("@@") else {
        return (0, 0);
    };
    let rest = rest.trim_start();
    let mut parts = rest.split_whitespace();
    let old = parts.next().unwrap_or("");
    let new = parts.next().unwrap_or("");
    (
        parse_hunk_count(old.trim_start_matches('-')),
        parse_hunk_count(new.trim_start_matches('+')),
    )
}

fn parse_hunk_count(token: &str) -> u32 {
    token
        .split(',')
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

fn empty_cell() -> DiffCell {
    DiffCell {
        kind: DiffCellKind::Empty,
        text: String::new(),
        line_no: None,
    }
}

fn cell_from_line(line: &ParsedLine, side: Side) -> DiffCell {
    let line_no = match side {
        Side::Old => line.old_no.or(line.new_no),
        Side::New => line.new_no.or(line.old_no),
    };
    DiffCell {
        kind: line.kind,
        text: line.text.clone(),
        line_no,
    }
}

enum Side {
    Old,
    New,
}

fn inline_rows(hunks: &[Hunk]) -> Vec<DiffRow> {
    let mut out = Vec::new();
    for hunk in hunks {
        if !hunk.header.is_empty() {
            out.push(DiffRow::Hunk {
                text: hunk.header.clone(),
            });
        }
        for line in &hunk.lines {
            out.push(DiffRow::Line {
                left: cell_from_line(line, Side::New),
                right: None,
            });
        }
    }
    out
}

fn pair_hunk(hunk: &Hunk) -> Vec<DiffRow> {
    let mut out = Vec::new();
    let lines = &hunk.lines;
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if line.kind == DiffCellKind::Meta {
            out.push(DiffRow::Line {
                left: cell_from_line(line, Side::Old),
                right: None,
            });
            i += 1;
            continue;
        }
        if line.kind == DiffCellKind::Ctx {
            out.push(DiffRow::Line {
                left: cell_from_line(line, Side::Old),
                right: Some(cell_from_line(line, Side::New)),
            });
            i += 1;
            continue;
        }
        let mut dels = Vec::new();
        let mut adds = Vec::new();
        while i < lines.len() && lines[i].kind == DiffCellKind::Del {
            dels.push(&lines[i]);
            i += 1;
        }
        while i < lines.len() && lines[i].kind == DiffCellKind::Add {
            adds.push(&lines[i]);
            i += 1;
        }
        let pair_count = dels.len().max(adds.len());
        for j in 0..pair_count {
            out.push(DiffRow::Line {
                left: dels
                    .get(j)
                    .map(|line| cell_from_line(line, Side::Old))
                    .unwrap_or_else(empty_cell),
                right: Some(
                    adds.get(j)
                        .map(|line| cell_from_line(line, Side::New))
                        .unwrap_or_else(empty_cell),
                ),
            });
        }
    }
    out
}

fn side_by_side_rows(hunks: &[Hunk]) -> Vec<DiffRow> {
    let mut out = Vec::new();
    for hunk in hunks {
        if !hunk.header.is_empty() {
            out.push(DiffRow::Hunk {
                text: hunk.header.clone(),
            });
        }
        out.extend(pair_hunk(hunk));
    }
    out
}

/// Rows for both diff sections. Empty sections are omitted.
pub fn build_diff_rows(content: &DiffContent, mode: DiffMode) -> Vec<DiffRow> {
    let render = if mode == DiffMode::SideBySide {
        side_by_side_rows
    } else {
        inline_rows
    };
    let mut out = Vec::new();
    let staged = parse_unified_diff(&content.staged);
    if !staged.is_empty() {
        out.push(DiffRow::Section(DiffSection::Staged));
        out.extend(render(&staged));
    }
    let unstaged = parse_unified_diff(&content.unstaged);
    if !unstaged.is_empty() {
        let section = if content.is_new {
            DiffSection::New
        } else {
            DiffSection::Unstaged
        };
        out.push(DiffRow::Section(section));
        out.extend(render(&unstaged));
    }
    out
}

/// Widest line number in the rows — sizes the gutter column. Floor is 2.
pub fn gutter_width(rows: &[DiffRow]) -> usize {
    let mut max = 0u32;
    for row in rows {
        if let DiffRow::Line { left, right } = row {
            max = max.max(left.line_no.unwrap_or(0));
            if let Some(right) = right {
                max = max.max(right.line_no.unwrap_or(0));
            }
        }
    }
    max.to_string().len().max(2)
}

/// Section header text.
pub fn section_header(section: DiffSection) -> &'static str {
    match section {
        DiffSection::Staged => "STAGED",
        DiffSection::Unstaged => "UNSTAGED",
        DiffSection::New => "NEW",
    }
}

/// Sign glyph for a cell kind.
pub fn cell_sign(kind: DiffCellKind) -> char {
    match kind {
        DiffCellKind::Add => '+',
        DiffCellKind::Del => '-',
        DiffCellKind::Ctx | DiffCellKind::Meta | DiffCellKind::Empty => ' ',
    }
}

/// Code-column width inside one cell (`width - gutter - " │ " - sign`).
pub fn cell_code_width(col_width: usize, gutter: usize) -> usize {
    col_width.saturating_sub(gutter.saturating_add(4)).max(1)
}

/// User-facing layout word, including the narrow-fallback note.
pub fn diff_pane_mode_label(mode: DiffMode, effective: DiffMode) -> &'static str {
    if mode == DiffMode::SideBySide && effective == DiffMode::Inline {
        "inline (too narrow)"
    } else if effective == DiffMode::Inline {
        "inline"
    } else {
        "split"
    }
}

/// One-line `{path}  inline|split · full?` header (plus pan / scroll when set).
pub fn diff_pane_header(
    path: &str,
    mode_label: &str,
    full: bool,
    pan: u16,
    start: usize,
    view_h: usize,
    row_count: usize,
) -> String {
    let title = if path.is_empty() { "Diff" } else { path };
    let mut extra = format!("  {mode_label}");
    if full {
        extra.push_str(" · full");
    }
    if pan > 0 {
        extra.push_str(&format!(" · pan {pan}"));
    }
    if row_count > view_h {
        let shown = (start + view_h).min(row_count);
        extra.push_str(&format!("  {shown}/{row_count}"));
    }
    format!("{title}{extra}")
}

/// Search text for one painted row (code / hunk / section, not raw git).
pub fn row_search_text(row: &DiffRow) -> String {
    match row {
        DiffRow::Section(section) => section_header(*section).to_string(),
        DiffRow::Hunk { text } => text.clone(),
        DiffRow::Line { left, right } => {
            let mut out = left.text.clone();
            if let Some(right) = right {
                if !right.text.is_empty() {
                    out.push(' ');
                    out.push_str(&right.text);
                }
            }
            out
        }
    }
}

/// Clamp vertical diff scroll so PageDown cannot grow past EOF.
pub fn clamp_diff_scroll(scroll: usize, row_count: usize, view_h: usize) -> usize {
    let max_start = row_count.saturating_sub(view_h.max(1));
    scroll.min(max_start)
}

/// Scroll so `row_index` stays in the upper third of `view_h`.
pub fn scroll_to_keep_row(row_index: usize, view_h: usize, row_count: usize) -> u16 {
    let view_h = view_h.max(1);
    let max_start = row_count.saturating_sub(view_h);
    let prefer = view_h / 3;
    row_index.saturating_sub(prefer).min(max_start) as u16
}

/// First visible add/del in the viewport, else nearest hunk at/above scroll.
pub fn anchor_row_index(rows: &[DiffRow], scroll: usize, view_h: usize) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let start = scroll.min(rows.len() - 1);
    let end = (start + view_h.max(1)).min(rows.len());
    for i in start..end {
        if is_change_row(&rows[i]) {
            return i;
        }
    }
    for i in (0..=start).rev() {
        if matches!(rows[i], DiffRow::Hunk { .. }) {
            return i;
        }
    }
    start
}

fn is_change_row(row: &DiffRow) -> bool {
    match row {
        DiffRow::Line { left, right } => {
            matches!(left.kind, DiffCellKind::Add | DiffCellKind::Del)
                || right
                    .as_ref()
                    .is_some_and(|cell| matches!(cell.kind, DiffCellKind::Add | DiffCellKind::Del))
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "\
diff --git a/hello.ts b/hello.ts
index 1111111..2222222 100644
--- a/hello.ts
+++ b/hello.ts
@@ -10,3 +10,4 @@
 line1
-line2
+line2 changed
 line3
+line4
";

    #[test]
    fn parse_seeds_counters_from_hunk_header() {
        let hunks = parse_unified_diff(FIXTURE);
        assert_eq!(hunks.len(), 1);
        let kinds: Vec<_> = hunks[0]
            .lines
            .iter()
            .map(|l| (l.kind, l.old_no, l.new_no))
            .collect();
        assert_eq!(
            kinds,
            vec![
                (DiffCellKind::Ctx, Some(10), Some(10)),
                (DiffCellKind::Del, Some(11), None),
                (DiffCellKind::Add, None, Some(11)),
                (DiffCellKind::Ctx, Some(12), Some(12)),
                (DiffCellKind::Add, None, Some(13)),
            ]
        );
    }

    #[test]
    fn parse_tolerates_garbage_hunk_header() {
        let hunks = parse_unified_diff("@@ garbage @@\n+a\n");
        assert_eq!(hunks[0].lines[0].kind, DiffCellKind::Add);
        assert_eq!(hunks[0].lines[0].new_no, Some(0));
    }

    #[test]
    fn build_diff_rows_emits_section_per_nonempty_slot() {
        let staged = build_diff_rows(
            &DiffContent {
                staged: FIXTURE.into(),
                unstaged: String::new(),
                is_new: false,
            },
            DiffMode::Inline,
        );
        assert_eq!(staged[0], DiffRow::Section(DiffSection::Staged));
        assert!(!staged
            .iter()
            .any(|r| matches!(r, DiffRow::Section(DiffSection::Unstaged))));

        let both = build_diff_rows(
            &DiffContent {
                staged: FIXTURE.into(),
                unstaged: FIXTURE.into(),
                is_new: false,
            },
            DiffMode::Inline,
        );
        let sections: Vec<_> = both
            .iter()
            .filter_map(|r| match r {
                DiffRow::Section(s) => Some(*s),
                _ => None,
            })
            .collect();
        assert_eq!(sections, vec![DiffSection::Staged, DiffSection::Unstaged]);
        assert!(build_diff_rows(&DiffContent::default(), DiffMode::Inline).is_empty());
    }

    #[test]
    fn untracked_section_is_new() {
        let rows = build_diff_rows(
            &DiffContent {
                staged: String::new(),
                unstaged: FIXTURE.into(),
                is_new: true,
            },
            DiffMode::Inline,
        );
        assert_eq!(rows[0], DiffRow::Section(DiffSection::New));
    }

    #[test]
    fn inline_mode_one_cell_per_line() {
        let rows = build_diff_rows(
            &DiffContent {
                staged: FIXTURE.into(),
                unstaged: String::new(),
                is_new: false,
            },
            DiffMode::Inline,
        );
        let lines: Vec<_> = rows
            .iter()
            .filter(|r| matches!(r, DiffRow::Line { .. }))
            .collect();
        assert_eq!(lines.len(), 5);
        assert!(lines
            .iter()
            .all(|r| matches!(r, DiffRow::Line { right: None, .. })));
    }

    #[test]
    fn side_by_side_pairs_del_with_add() {
        let rows = build_diff_rows(
            &DiffContent {
                staged: FIXTURE.into(),
                unstaged: String::new(),
                is_new: false,
            },
            DiffMode::SideBySide,
        );
        let changed = rows.iter().find_map(|r| match r {
            DiffRow::Line { left, right } if left.text == "line2" => Some((left, right)),
            _ => None,
        });
        let (left, right) = changed.expect("del row");
        assert_eq!(left.kind, DiffCellKind::Del);
        assert_eq!(right.as_ref().map(|c| c.kind), Some(DiffCellKind::Add));
        assert_eq!(
            right.as_ref().map(|c| c.text.as_str()),
            Some("line2 changed")
        );

        let added = rows.iter().find_map(|r| match r {
            DiffRow::Line { left, right }
                if right.as_ref().map(|c| c.text.as_str()) == Some("line4") =>
            {
                Some(left.kind)
            }
            _ => None,
        });
        assert_eq!(added, Some(DiffCellKind::Empty));
    }

    #[test]
    fn side_by_side_zips_del_add_runs() {
        let rows = build_diff_rows(
            &DiffContent::from_unified("@@ -1,2 +1,2 @@\n-a\n-b\n+a2\n+b2\n"),
            DiffMode::SideBySide,
        );
        let pairs: Vec<_> = rows
            .iter()
            .filter_map(|r| match r {
                DiffRow::Line { left, right } => {
                    Some((left.text.as_str(), right.as_ref().map(|c| c.text.as_str())))
                }
                _ => None,
            })
            .collect();
        assert_eq!(pairs, vec![("a", Some("a2")), ("b", Some("b2"))]);
    }

    #[test]
    fn binary_marker_is_meta_cell() {
        let rows = build_diff_rows(
            &DiffContent::from_unified("Binary files a/x and b/x differ\n"),
            DiffMode::Inline,
        );
        let DiffRow::Line { left, .. } = &rows
            .iter()
            .find(|r| matches!(r, DiffRow::Line { .. }))
            .unwrap()
        else {
            panic!("expected line");
        };
        assert_eq!(left.kind, DiffCellKind::Meta);
        assert!(left.text.contains("Binary files"));
    }

    #[test]
    fn gutter_width_floors_at_two_and_grows() {
        assert_eq!(gutter_width(&[]), 2);
        let small = build_diff_rows(
            &DiffContent {
                staged: FIXTURE.into(),
                unstaged: String::new(),
                is_new: false,
            },
            DiffMode::Inline,
        );
        assert_eq!(gutter_width(&small), 2);
        let big = build_diff_rows(
            &DiffContent::from_unified("@@ -1200,1 +1200,1 @@\n-a\n+b\n"),
            DiffMode::Inline,
        );
        assert_eq!(gutter_width(&big), 4);
    }

    #[test]
    fn header_includes_path_mode_and_optional_full() {
        assert_eq!(
            diff_pane_header("app/README.md", "inline", false, 0, 0, 20, 5),
            "app/README.md  inline"
        );
        assert_eq!(
            diff_pane_header("src/lib.rs", "split", true, 3, 0, 10, 40),
            "src/lib.rs  split · full · pan 3  10/40"
        );
        assert_eq!(
            diff_pane_mode_label(DiffMode::SideBySide, DiffMode::Inline),
            "inline (too narrow)"
        );
    }

    #[test]
    fn blank_content_and_from_lines() {
        assert!(DiffContent::default().is_blank());
        assert!(!DiffContent::from_unified("@@ -1 +1 @@\n+a\n").is_blank());
        let from_vec = DiffContent::from_lines(vec!["@@ -1 +1 @@".into(), "+a".into()]);
        assert!(!from_vec.is_blank());
    }

    #[test]
    fn synthesize_empty_and_body() {
        assert_eq!(synthesize_all_add_diff(""), "@@ -0,0 +0,0 @@\n");
        assert_eq!(
            synthesize_all_add_diff("a\nb\n"),
            "@@ -0,0 +1,2 @@\n+a\n+b\n"
        );
    }

    #[test]
    fn anchor_prefers_visible_add_or_del() {
        let rows = build_diff_rows(
            &DiffContent::from_unified("@@ -1,2 +1,2 @@\n ctx\n-old\n+new\n"),
            DiffMode::Inline,
        );
        // 0 section, 1 hunk, 2 ctx, 3 del, 4 add
        assert_eq!(anchor_row_index(&rows, 0, 10), 3);
        assert_eq!(scroll_to_keep_row(3, 9, rows.len()), 0);
    }
}

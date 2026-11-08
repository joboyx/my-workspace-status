//! File-diff load, unified-diff parse, and numbered rows.
//!
//! Path header, line-number gutter, and STAGED / UNSTAGED / NEW labels.
//! Syntax highlighting lives in [`super::syntax`] and paint. Rows stay
//! line-based git output; the changed-word highlight is a paint overlay
//! from [`super::word_diff`].

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::file_index::paintable_line;
use crate::git::{exec_git_stdout, git_diff_args};
use crate::snapshot::FileChange;

use super::search::wrap_col_starts;
use super::split::{side_by_side_column_widths, DiffMode, DIFF_CURSOR_BAR_COLS};
use super::tree::list_viewport_start;

/// Untracked files above ~1 MB are not previewed: binary ones get the
/// binary stub, text ones the too-large stub.
const HUGE_FILE_BYTES: u64 = 1_000_000;

/// Leading bytes scanned for a NUL to call a huge untracked file binary
/// (git's own heuristic window).
const BINARY_SNIFF_BYTES: u64 = 8000;

/// Start of the stub line for an untracked text file over [`HUGE_FILE_BYTES`].
const TOO_LARGE_PREFIX: &str = "too large to preview (";

/// Gutter rule between line numbers and the sign.
pub const DIFF_RULE: char = '│';

/// Staged / unstaged / untracked / committed section label.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DiffSection {
    Staged,
    Unstaged,
    New,
    /// Compare-tab three-dot range. Never staged or unstaged.
    Committed,
    /// Commit-vs-working-tree compare tab: the new side is the file on
    /// disk. Never staged, unstaged, or reverted.
    Worktree,
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
    /// Paintable line text ([`paintable_line`]): tabs expanded to spaces
    /// and control characters as U+FFFD. Partial patches read the parsed
    /// line, never this.
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
    /// `git diff` failed for one side while the other loaded: the
    /// [`diff_failed_text`] line, so the missing section does not read as
    /// "no changes".
    Error {
        text: String,
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
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DiffContent {
    pub staged: String,
    pub unstaged: String,
    /// Label the unstaged section `NEW` (untracked synthesised as all-add).
    pub is_new: bool,
    /// Compare-tab diff: label the unstaged section `COMMITTED`, and let
    /// [`build_partial_patch`] build only
    /// [`PartialPatchKind::RevertCommitted`] from it.
    pub is_committed: bool,
    /// Label the unstaged section `WORKING TREE` (commit-vs-working-tree
    /// compare tabs). Line writes refuse it.
    pub vs_worktree: bool,
    /// Paint the unstaged section's label as `COMMITTED` and change
    /// nothing else (Workspace depth-2 commit and stash drill diffs). Row
    /// bindings and [`RowLineRef::section`] stay UNSTAGED, so
    /// [`build_partial_patch`], blame, and go to line read the diff as an
    /// unstaged one; [`Self::is_committed`] stays false.
    pub committed_label: bool,
    /// Git's reason when `git diff` failed, so the pane does not read as
    /// an empty diff.
    pub error: Option<String>,
}

impl DiffContent {
    /// Unstaged-only unified text (worktree drill diffs and tests).
    pub fn from_unified(text: impl Into<String>) -> Self {
        Self {
            staged: String::new(),
            unstaged: text.into(),
            is_new: false,
            is_committed: false,
            vs_worktree: false,
            committed_label: false,
            error: None,
        }
    }

    /// Compare-tab unified text. The section label is `COMMITTED`.
    pub fn from_compare_lines(lines: Vec<String>) -> Self {
        Self {
            staged: String::new(),
            unstaged: lines.join("\n"),
            is_new: false,
            is_committed: true,
            vs_worktree: false,
            committed_label: false,
            error: None,
        }
    }

    /// Commit-vs-working-tree unified text. The section label is
    /// `WORKING TREE`.
    pub fn from_worktree_compare_lines(lines: Vec<String>) -> Self {
        Self {
            vs_worktree: true,
            ..Self::from_lines(lines)
        }
    }

    /// Join raw git lines into unstaged-only content.
    pub fn from_lines(lines: Vec<String>) -> Self {
        Self::from_unified(lines.join("\n"))
    }

    /// Depth-2 commit or stash drill text ([`Self::from_lines`]). The
    /// section label is `COMMITTED` ([`Self::committed_label`]).
    pub fn from_commit_lines(lines: Vec<String>) -> Self {
        Self {
            committed_label: true,
            ..Self::from_lines(lines)
        }
    }

    /// True when both slots are empty or whitespace.
    #[allow(dead_code)]
    pub fn is_blank(&self) -> bool {
        self.staged.trim().is_empty() && self.unstaged.trim().is_empty()
    }

    /// Identity for TUI syntax-span reuse.
    ///
    /// Watch and compare reloads assign a new `DiffContent` value in place, so
    /// the field address and row count can stay the same while the text
    /// changes. Paint must key the span cache on this hash, not on a pointer.
    pub(crate) fn syntax_fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
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
    let mut error = None;
    if change.staged_status.is_some() {
        let args = git_diff_args(&["diff", "--cached"], &change.path, context);
        match git_diff_text(&args, &repo_dir) {
            Ok(text) => staged = text,
            Err(reason) => error = Some(reason),
        }
    }
    if change.unstaged_status.is_some() {
        let args = git_diff_args(&["diff"], &change.path, context);
        match git_diff_text(&args, &repo_dir) {
            Ok(text) => unstaged = text,
            Err(reason) => error = error.or(Some(reason)),
        }
    }
    DiffContent {
        staged,
        unstaged,
        is_new: false,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error,
    }
}

/// `git diff` stdout, trimmed like [`exec_git`]. `Err` is git's reason line.
pub(crate) fn git_diff_text(args: &[String], dir: &Path) -> Result<String, String> {
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    exec_git_stdout(&refs, dir).map(|text| text.trim().to_string())
}

fn untracked_content(repo_dir: &Path, path: &str) -> DiffContent {
    let unstaged = read_untracked_as_diff(&repo_dir.join(path), path);
    let is_new = !unstaged.is_empty();
    DiffContent {
        staged: String::new(),
        unstaged,
        is_new,
        is_committed: false,
        vs_worktree: false,
        committed_label: false,
        error: None,
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
        return if head_has_nul(abs) {
            binary_stub(rel_path)
        } else {
            too_large_stub(meta.len())
        };
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

/// Stub for an untracked text file over [`HUGE_FILE_BYTES`] (`N.N MB`).
fn too_large_stub(len: u64) -> String {
    format!("{TOO_LARGE_PREFIX}{:.1} MB)\n", len as f64 / 1_000_000.0)
}

/// True when the first [`BINARY_SNIFF_BYTES`] of `abs` hold a NUL. An
/// unreadable file counts as binary.
fn head_has_nul(abs: &Path) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(abs) else {
        return true;
    };
    let mut head = Vec::new();
    if file
        .take(BINARY_SNIFF_BYTES)
        .read_to_end(&mut head)
        .is_err()
    {
        return true;
    }
    head.contains(&0)
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

/// A whole-file stub line (binary, or untracked too large to preview)
/// that stands for the file's hunks.
fn is_binary_marker(line: &str) -> bool {
    (line.starts_with("Binary files ") && line.ends_with(" differ"))
        || (line.starts_with(TOO_LARGE_PREFIX) && line.ends_with(" MB)"))
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
        text: paintable_line(&line.text),
        line_no,
    }
}

enum Side {
    Old,
    New,
}

/// Painted-row map back onto a parsed hunk line.
#[derive(Clone, Debug)]
enum RowBind {
    Section(DiffSection),
    Hunk {
        section: DiffSection,
        hunk: usize,
    },
    Line {
        section: DiffSection,
        hunk: usize,
        parsed: Vec<usize>,
    },
    /// [`DiffRow::Error`]: no patch lines.
    Error,
}

/// Apply target and direction for a visual-line partial patch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PartialPatchKind {
    /// `git apply --cached` of unstaged add/del lines.
    Stage,
    /// `git apply --reverse --cached` of staged add/del lines.
    Unstage,
    /// `git apply --reverse` of unstaged add/del lines (worktree only).
    Revert,
    /// `git apply --reverse` of COMMITTED (compare tab) add/del lines onto
    /// the worktree. The caller proves the worktree file is the compare
    /// head blob, so the post-image matches it and the reverse apply puts
    /// the merge-base lines back.
    RevertCommitted,
}

fn pair_hunk(hunk: &Hunk) -> Vec<(DiffRow, Vec<usize>)> {
    let mut out = Vec::new();
    let lines = &hunk.lines;
    let mut i = 0;
    while i < lines.len() {
        let line = &lines[i];
        if line.kind == DiffCellKind::Meta {
            out.push((
                DiffRow::Line {
                    left: cell_from_line(line, Side::Old),
                    right: None,
                },
                vec![i],
            ));
            i += 1;
            continue;
        }
        if line.kind == DiffCellKind::Ctx {
            out.push((
                DiffRow::Line {
                    left: cell_from_line(line, Side::Old),
                    right: Some(cell_from_line(line, Side::New)),
                },
                vec![i],
            ));
            i += 1;
            continue;
        }
        let mut dels = Vec::new();
        let mut adds = Vec::new();
        while i < lines.len() && lines[i].kind == DiffCellKind::Del {
            dels.push(i);
            i += 1;
        }
        while i < lines.len() && lines[i].kind == DiffCellKind::Add {
            adds.push(i);
            i += 1;
        }
        let pair_count = dels.len().max(adds.len());
        for j in 0..pair_count {
            let mut parsed = Vec::new();
            let left = if let Some(&idx) = dels.get(j) {
                parsed.push(idx);
                cell_from_line(&lines[idx], Side::Old)
            } else {
                empty_cell()
            };
            let right = if let Some(&idx) = adds.get(j) {
                parsed.push(idx);
                Some(cell_from_line(&lines[idx], Side::New))
            } else {
                Some(empty_cell())
            };
            out.push((DiffRow::Line { left, right }, parsed));
        }
    }
    out
}

fn push_hunk_rows(
    out: &mut Vec<(DiffRow, RowBind)>,
    section: DiffSection,
    hunk_idx: usize,
    hunk: &Hunk,
    mode: DiffMode,
) {
    if !hunk.header.is_empty() {
        out.push((
            DiffRow::Hunk {
                text: paintable_line(&hunk.header),
            },
            RowBind::Hunk {
                section,
                hunk: hunk_idx,
            },
        ));
    }
    if mode == DiffMode::SideBySide {
        for (row, parsed) in pair_hunk(hunk) {
            out.push((
                row,
                RowBind::Line {
                    section,
                    hunk: hunk_idx,
                    parsed,
                },
            ));
        }
        return;
    }
    for (line_idx, line) in hunk.lines.iter().enumerate() {
        out.push((
            DiffRow::Line {
                left: cell_from_line(line, Side::New),
                right: None,
            },
            RowBind::Line {
                section,
                hunk: hunk_idx,
                parsed: vec![line_idx],
            },
        ));
    }
}

fn annotated_diff_rows(content: &DiffContent, mode: DiffMode) -> Vec<(DiffRow, RowBind)> {
    let mut out = Vec::new();
    let staged = parse_unified_diff(&content.staged);
    if !staged.is_empty() {
        out.push((
            DiffRow::Section(DiffSection::Staged),
            RowBind::Section(DiffSection::Staged),
        ));
        for (idx, hunk) in staged.iter().enumerate() {
            push_hunk_rows(&mut out, DiffSection::Staged, idx, hunk, mode);
        }
    }
    let unstaged = parse_unified_diff(&content.unstaged);
    if !unstaged.is_empty() {
        let section = if content.vs_worktree {
            DiffSection::Worktree
        } else if content.is_committed {
            DiffSection::Committed
        } else if content.is_new {
            DiffSection::New
        } else {
            DiffSection::Unstaged
        };
        // Only the painted label changes; the binding keeps `section`.
        let label = if content.committed_label {
            DiffSection::Committed
        } else {
            section
        };
        out.push((DiffRow::Section(label), RowBind::Section(section)));
        for (idx, hunk) in unstaged.iter().enumerate() {
            push_hunk_rows(&mut out, section, idx, hunk, mode);
        }
    }
    // With no rows at all the pane paints the failure as its empty text.
    if let (Some(reason), false) = (content.error.as_deref(), out.is_empty()) {
        out.push((
            DiffRow::Error {
                text: diff_failed_text(reason),
            },
            RowBind::Error,
        ));
    }
    out
}

/// Diff-pane text for a failed `git diff`: git's `reason` after a fixed
/// prefix.
pub fn diff_failed_text(reason: &str) -> String {
    paintable_line(&format!("git diff failed: {reason}"))
}

/// Rows for both diff sections. Empty sections are omitted.
pub fn build_diff_rows(content: &DiffContent, mode: DiffMode) -> Vec<DiffRow> {
    annotated_diff_rows(content, mode)
        .into_iter()
        .map(|(row, _)| row)
        .collect()
}

/// Painted row of `content` in `mode` (a diff cursor index) that shows
/// 1-based line `new_line` of the new side: a context or added line of the
/// unstaged (or NEW) section first, else of the staged section. `None`
/// when no row shows it.
pub fn row_for_new_line(content: &DiffContent, mode: DiffMode, new_line: u32) -> Option<usize> {
    let rows = annotated_diff_rows(content, mode);
    let staged = parse_unified_diff(&content.staged);
    let unstaged = parse_unified_diff(&content.unstaged);
    let find = |in_staged: bool| {
        rows.iter().position(|(_, bind)| {
            let RowBind::Line {
                section,
                hunk,
                parsed,
            } = bind
            else {
                return false;
            };
            if (*section == DiffSection::Staged) != in_staged {
                return false;
            }
            let hunks = if in_staged { &staged } else { &unstaged };
            hunks.get(*hunk).is_some_and(|hunk| {
                parsed
                    .iter()
                    .filter_map(|&i| hunk.lines.get(i))
                    .any(|line| {
                        matches!(line.kind, DiffCellKind::Add | DiffCellKind::Ctx)
                            && line.new_no == Some(new_line)
                    })
            })
        })
    };
    find(false).or_else(|| find(true))
}

/// The source line behind one painted diff row: its section, kind, and
/// line numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RowLineRef {
    /// Section the row sits in (STAGED, UNSTAGED, NEW, COMMITTED,
    /// WORKING TREE).
    pub section: DiffSection,
    /// [`DiffCellKind::Add`], [`DiffCellKind::Del`], or [`DiffCellKind::Ctx`].
    pub kind: DiffCellKind,
    /// 1-based line in the old side. `None` for an added line.
    pub old_no: Option<u32>,
    /// 1-based line in the new side. `None` for a deleted line.
    pub new_no: Option<u32>,
}

/// The source line behind painted row `row` of `content` in `mode`.
///
/// Uses the same row list as [`build_diff_rows`], so `row` is the diff
/// cursor index. A split row that pairs a deleted and an added line
/// gives the added (new-side) line; a row with only a deleted line gives
/// that line. Section labels, hunk headers, `\ No newline` and binary
/// markers, error rows, and out-of-range rows give `None`.
pub fn row_line_ref(content: &DiffContent, mode: DiffMode, row: usize) -> Option<RowLineRef> {
    let rows = annotated_diff_rows(content, mode);
    let (_, bind) = rows.get(row)?;
    let RowBind::Line { section, .. } = bind else {
        return None;
    };
    let text = match section {
        DiffSection::Staged => &content.staged,
        _ => &content.unstaged,
    };
    bind_line_ref(bind, &parse_unified_diff(text))
}

/// Painted row that go to line `line` (1-based) lands on, or `None` when
/// no row of `content` shows a source line (empty, binary, or failed diff).
///
/// A row counts by its [`row_line_ref`] new-side number. Deleted-only rows
/// count by their old-side number only when no row has a new side (a
/// deleted file). With no row on `line` the nearest row wins, the earlier
/// one on a tie, so a line between hunks lands next to it and a line past
/// the end lands on the last numbered row.
pub fn row_for_line(content: &DiffContent, mode: DiffMode, line: u32) -> Option<usize> {
    let staged = parse_unified_diff(&content.staged);
    let unstaged = parse_unified_diff(&content.unstaged);
    let refs: Vec<(usize, RowLineRef)> = annotated_diff_rows(content, mode)
        .iter()
        .enumerate()
        .filter_map(|(row, (_, bind))| {
            let hunks = match bind {
                RowBind::Line {
                    section: DiffSection::Staged,
                    ..
                } => &staged,
                _ => &unstaged,
            };
            bind_line_ref(bind, hunks).map(|line_ref| (row, line_ref))
        })
        .collect();
    let has_new_side = refs.iter().any(|(_, line_ref)| line_ref.new_no.is_some());
    refs.iter()
        .filter_map(|(row, line_ref)| {
            let number = if has_new_side {
                line_ref.new_no
            } else {
                line_ref.old_no
            }?;
            Some((number.abs_diff(line), *row))
        })
        .min()
        .map(|(_, row)| row)
}

/// [`RowLineRef`] of one annotated row, read from its section's `hunks`.
fn bind_line_ref(bind: &RowBind, hunks: &[Hunk]) -> Option<RowLineRef> {
    let RowBind::Line {
        section,
        hunk,
        parsed,
    } = bind
    else {
        return None;
    };
    let lines = &hunks.get(*hunk)?.lines;
    let picked: Vec<&ParsedLine> = parsed.iter().filter_map(|&i| lines.get(i)).collect();
    let line = picked
        .iter()
        .find(|l| matches!(l.kind, DiffCellKind::Add | DiffCellKind::Ctx))
        .or_else(|| picked.iter().find(|l| l.kind == DiffCellKind::Del))?;
    Some(RowLineRef {
        section: *section,
        kind: line.kind,
        old_no: line.old_no,
        new_no: line.new_no,
    })
}

/// Build a `git apply` patch for painted rows `start..=end`.
///
/// Stage reads the unstaged (or NEW) section. Unstage reads STAGED. Revert
/// reads UNSTAGED only (index to worktree) and refuses NEW files.
/// RevertCommitted reads COMMITTED only (a compare diff) with Revert's
/// rules; every other kind refuses a COMMITTED diff.
/// Context lines in the range stay as context. Stage applies forward, so
/// unselected additions drop and unselected deletions become context.
/// Unstage and Revert apply with `--reverse`, so the post-image must match
/// the target: unselected additions become context and unselected deletions
/// drop. A `-` run and its `+` run interleave by pair, so a selected pair
/// keeps its place. Returns `Err` when the range cannot become a valid patch (fail
/// closed — never a whole-file patch).
pub fn build_partial_patch(
    content: &DiffContent,
    mode: DiffMode,
    start: usize,
    end: usize,
    kind: PartialPatchKind,
    path: &str,
) -> Result<String, String> {
    if path.is_empty() {
        return Err(partial_fail(kind));
    }
    // The working-tree side of a compare tab has no index or commit to
    // write; the compare refusals stop it first.
    if content.vs_worktree {
        return Err(partial_fail(kind));
    }
    if content.is_committed != (kind == PartialPatchKind::RevertCommitted) {
        return Err(if content.is_committed {
            committed_fail(kind)
        } else {
            partial_fail(kind)
        });
    }
    let annotated = annotated_diff_rows(content, mode);
    if annotated.is_empty() {
        return Err(nothing_fail(kind));
    }
    let lo = start.min(end);
    let hi = start.max(end);
    if lo >= annotated.len() {
        return Err(nothing_fail(kind));
    }
    let hi = hi.min(annotated.len() - 1);

    let mut sections = HashSet::new();
    let mut whole_hunks: HashSet<(DiffSection, usize)> = HashSet::new();
    let mut line_hits: HashMap<(DiffSection, usize), HashSet<usize>> = HashMap::new();
    for bind in annotated[lo..=hi].iter().map(|(_, bind)| bind) {
        match bind {
            RowBind::Section(section) => {
                sections.insert(*section);
            }
            RowBind::Hunk { section, hunk } => {
                sections.insert(*section);
                whole_hunks.insert((*section, *hunk));
            }
            RowBind::Line {
                section,
                hunk,
                parsed,
            } => {
                sections.insert(*section);
                line_hits
                    .entry((*section, *hunk))
                    .or_default()
                    .extend(parsed.iter().copied());
            }
            RowBind::Error => {}
        }
    }

    let change_sections: HashSet<DiffSection> = sections
        .into_iter()
        .filter(|section| {
            matches!(
                section,
                DiffSection::Staged
                    | DiffSection::Unstaged
                    | DiffSection::New
                    | DiffSection::Committed
            )
        })
        .collect();
    if change_sections.contains(&DiffSection::Staged)
        && (change_sections.contains(&DiffSection::Unstaged)
            || change_sections.contains(&DiffSection::New))
    {
        return Err("highlight spans staged and unstaged".into());
    }

    let (want, raw, is_new) = match kind {
        PartialPatchKind::Stage => {
            if change_sections.contains(&DiffSection::Staged) && change_sections.len() == 1 {
                return Err(nothing_fail(kind));
            }
            let section = if content.is_new {
                DiffSection::New
            } else {
                DiffSection::Unstaged
            };
            if !change_sections.contains(&section) {
                return Err(nothing_fail(kind));
            }
            (section, content.unstaged.as_str(), content.is_new)
        }
        PartialPatchKind::Unstage => {
            if !change_sections.contains(&DiffSection::Staged) {
                return Err(nothing_fail(kind));
            }
            (DiffSection::Staged, content.staged.as_str(), false)
        }
        PartialPatchKind::Revert => {
            if content.is_new || change_sections.contains(&DiffSection::New) {
                return Err("cannot revert lines of a new file".into());
            }
            if !change_sections.contains(&DiffSection::Unstaged) {
                return Err(nothing_fail(kind));
            }
            (DiffSection::Unstaged, content.unstaged.as_str(), false)
        }
        PartialPatchKind::RevertCommitted => {
            if !change_sections.contains(&DiffSection::Committed) {
                return Err(nothing_fail(kind));
            }
            (DiffSection::Committed, content.unstaged.as_str(), false)
        }
    };

    if raw.trim().is_empty() {
        return Err(nothing_fail(kind));
    }
    let header = patch_file_header(raw, path, is_new);
    if header.contains("rename from") || header.contains("copy from") {
        return Err(partial_fail(kind));
    }
    // An intent-to-add file (`git add -N`) is not `is_new`, but its unstaged
    // diff is a new-file patch: a reverse apply of a whole hunk deletes it.
    let revert = matches!(
        kind,
        PartialPatchKind::Revert | PartialPatchKind::RevertCommitted
    );
    if revert && header.contains("new file mode") {
        return Err("cannot revert lines of a new file".into());
    }
    // A reverse worktree apply would also undo a mode change or recreate a
    // deleted file. Line revert covers content only.
    if revert && (header.contains("old mode") || header.contains("deleted file mode")) {
        return Err(partial_fail(kind));
    }
    let hunks = parse_unified_diff(raw);
    if hunks.iter().any(|hunk| hunk.header.is_empty()) {
        return Err(binary_fail(kind));
    }

    let mut body = String::new();
    // Line-count shift of the hunks emitted so far (other side − target).
    let mut shift = 0i64;
    for (idx, hunk) in hunks.iter().enumerate() {
        let key = (want, idx);
        let selected = if whole_hunks.contains(&key) {
            None
        } else {
            line_hits.get(&key)
        };
        if selected.is_none() && !whole_hunks.contains(&key) {
            continue;
        }
        if let Some((text, hunk_shift)) = transform_hunk(hunk, selected, kind, shift)? {
            body.push_str(&text);
            shift += hunk_shift;
        }
    }
    if body.is_empty() {
        return Err(nothing_fail(kind));
    }
    Ok(format!("{header}{body}"))
}

fn nothing_fail(kind: PartialPatchKind) -> String {
    match kind {
        PartialPatchKind::Stage => "nothing to stage in highlight".into(),
        PartialPatchKind::Unstage => "nothing to unstage in highlight".into(),
        PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
            "nothing to revert in highlight".into()
        }
    }
}

fn committed_fail(kind: PartialPatchKind) -> String {
    match kind {
        PartialPatchKind::Stage => "cannot stage a committed diff".into(),
        PartialPatchKind::Unstage => "cannot unstage a committed diff".into(),
        PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
            "cannot revert a committed diff".into()
        }
    }
}

fn binary_fail(kind: PartialPatchKind) -> String {
    match kind {
        PartialPatchKind::Stage => "cannot stage a binary highlight".into(),
        PartialPatchKind::Unstage => "cannot unstage a binary highlight".into(),
        PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
            "cannot revert a binary highlight".into()
        }
    }
}

fn partial_fail(kind: PartialPatchKind) -> String {
    match kind {
        PartialPatchKind::Stage => "cannot stage highlight".into(),
        PartialPatchKind::Unstage => "cannot unstage highlight".into(),
        PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
            "cannot revert highlight".into()
        }
    }
}

fn patch_file_header(raw: &str, path: &str, is_new: bool) -> String {
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut header = String::new();
    for line in normalized.lines() {
        if line.starts_with("@@") || is_binary_marker(line) {
            break;
        }
        if line.starts_with("diff --git")
            || line.starts_with("index ")
            || line.starts_with("new file mode")
            || line.starts_with("deleted file mode")
            || line.starts_with("old mode")
            || line.starts_with("new mode")
            || line.starts_with("similarity index")
            || line.starts_with("rename from")
            || line.starts_with("rename to")
            || line.starts_with("copy from")
            || line.starts_with("copy to")
            || line.starts_with("--- ")
            || line.starts_with("+++ ")
        {
            header.push_str(line);
            header.push('\n');
        }
    }
    if header.is_empty() {
        if is_new {
            format!(
                "diff --git a/{path} b/{path}\nnew file mode 100644\n--- /dev/null\n+++ b/{path}\n"
            )
        } else {
            format!("diff --git a/{path} b/{path}\n--- a/{path}\n+++ b/{path}\n")
        }
    } else {
        header
    }
}

fn hunk_header_suffix(header: &str) -> &str {
    let rest = header.strip_prefix("@@").unwrap_or(header);
    match rest.find("@@") {
        Some(idx) => &rest[idx + 2..],
        None => "",
    }
}

/// One body line of a rewritten hunk: sign, text, and the
/// `\ No newline at end of file` marker that follows it, if any.
struct PatchLine<'a> {
    sign: char,
    text: &'a str,
    no_eol: Option<&'a str>,
}

/// Hunk line index in pair order: each `-` run and the `+` run after it
/// interleave by pair index (`-1 +1 -2 +2 …`, then the leftovers), and
/// each `\ No newline at end of file` marker rides with its line.
fn pair_ordered_lines(hunk: &Hunk) -> Vec<(usize, Option<&str>)> {
    let mut items: Vec<(usize, Option<&str>)> = Vec::new();
    for (idx, line) in hunk.lines.iter().enumerate() {
        match line.kind {
            DiffCellKind::Ctx | DiffCellKind::Add | DiffCellKind::Del => items.push((idx, None)),
            DiffCellKind::Meta => {
                if let Some(last) = items.last_mut() {
                    last.1 = Some(&line.text);
                }
            }
            DiffCellKind::Empty => {}
        }
    }
    let kind = |item: &(usize, Option<&str>)| hunk.lines[item.0].kind;
    let mut out = Vec::with_capacity(items.len());
    let mut i = 0;
    while i < items.len() {
        if kind(&items[i]) == DiffCellKind::Ctx {
            out.push(items[i]);
            i += 1;
            continue;
        }
        let dels_start = i;
        while i < items.len() && kind(&items[i]) == DiffCellKind::Del {
            i += 1;
        }
        let adds_start = i;
        while i < items.len() && kind(&items[i]) == DiffCellKind::Add {
            i += 1;
        }
        let dels = &items[dels_start..adds_start];
        let adds = &items[adds_start..i];
        for j in 0..dels.len().max(adds.len()) {
            out.extend(dels.get(j).copied());
            out.extend(adds.get(j).copied());
        }
    }
    out
}

/// Keep the selected add/del lines of `hunk` and rewrite the rest.
///
/// Forward (Stage, `git apply`): the pre-image must match the target, so
/// an unselected addition drops and an unselected deletion becomes
/// context. Reverse (Unstage / Revert, `git apply --reverse`): the
/// post-image must match the target, so an unselected addition becomes
/// context and an unselected deletion drops.
///
/// Lines go out in [`pair_ordered_lines`] order, so a selected pair lands
/// where its deletion was: the side that must match the target keeps its
/// original order, and the other side changes pair by pair.
///
/// Start lines: the target side keeps the start from `hunk`. The other
/// side starts at the target start plus `shift`, the line-count change of
/// the hunks already emitted (git applies hunks in order and uses that
/// side as the position hint). Returns the hunk text and its own shift.
fn transform_hunk(
    hunk: &Hunk,
    selected_lines: Option<&HashSet<usize>>,
    kind: PartialPatchKind,
    shift: i64,
) -> Result<Option<(String, i64)>, String> {
    if hunk.header.is_empty() {
        return Err(binary_fail(kind));
    }
    let reverse = kind != PartialPatchKind::Stage;
    let whole = selected_lines.is_none();
    let mut lines: Vec<PatchLine> = Vec::new();
    let mut has_change = false;
    for (idx, no_eol) in pair_ordered_lines(hunk) {
        let line = &hunk.lines[idx];
        let take = whole || selected_lines.is_some_and(|set| set.contains(&idx));
        let add = line.kind == DiffCellKind::Add;
        let sign = match line.kind {
            DiffCellKind::Ctx => ' ',
            _ if take => {
                has_change = true;
                if add {
                    '+'
                } else {
                    '-'
                }
            }
            // Forward keeps an unselected deletion as context; reverse
            // keeps an unselected addition as context. The other one is
            // not in the target file and drops.
            _ if add == reverse => ' ',
            _ => continue,
        };
        lines.push(PatchLine {
            sign,
            text: &line.text,
            no_eol,
        });
    }
    if !has_change {
        return Ok(None);
    }
    let lines = place_no_eol_markers(lines, reverse).ok_or_else(|| partial_fail(kind))?;
    let mut body = String::new();
    let mut old_count = 0u32;
    let mut new_count = 0u32;
    for line in &lines {
        if line.sign != '+' {
            old_count = old_count.saturating_add(1);
        }
        if line.sign != '-' {
            new_count = new_count.saturating_add(1);
        }
        body.push(line.sign);
        body.push_str(line.text);
        body.push('\n');
        if let Some(marker) = line.no_eol {
            body.push_str(marker);
            body.push('\n');
        }
    }
    let (old_start, new_start) = hunk_starts(&hunk.header);
    let (target_start, target_count, other_count) = if reverse {
        (new_start, new_count, old_count)
    } else {
        (old_start, old_count, new_count)
    };
    let other_start = shifted_start(target_start, target_count, other_count, shift);
    let (old_start, new_start) = if reverse {
        (other_start, target_start)
    } else {
        (target_start, other_start)
    };
    let suffix = hunk_header_suffix(&hunk.header);
    Ok(Some((
        format!("@@ -{old_start},{old_count} +{new_start},{new_count} @@{suffix}\n{body}"),
        i64::from(other_count) - i64::from(target_count),
    )))
}

/// Start of the non-target side of a rewritten hunk.
///
/// Git's zero-count convention: a side with count 0 names the line before
/// the insertion point (`-4,0` inserts after line 4); otherwise the start
/// is the first line. Both sides share the line before the hunk, moved by
/// `shift`.
fn shifted_start(target_start: u32, target_count: u32, other_count: u32, shift: i64) -> u32 {
    let target_before = if target_count == 0 {
        i64::from(target_start)
    } else {
        i64::from(target_start) - 1
    };
    let other_before = (target_before + shift).max(0);
    let start = if other_count == 0 {
        other_before
    } else {
        other_before + 1
    };
    u32::try_from(start).unwrap_or(u32::MAX)
}

/// Keep each no-newline marker valid after [`transform_hunk`] drops lines.
///
/// A marker may only follow the last line of its side (old: ` ` and `-`;
/// new: ` ` and `+`), or git joins that line with the next one. The side
/// that must match the target (old forward, new reverse) keeps every line,
/// so its marker line stays last. On the other side:
///
/// - a `-` / `+` line that is no longer last loses its marker (it now ends
///   with a newline, because a line follows it);
/// - a context line that is last only on the target side splits into a `-`
///   line and a `+` line, so each side gets its own ending.
///
/// `None` when a marker line is not last on the target side (fail closed).
fn place_no_eol_markers(lines: Vec<PatchLine>, reverse: bool) -> Option<Vec<PatchLine>> {
    let last_old = lines.iter().rposition(|line| line.sign != '+');
    let last_new = lines.iter().rposition(|line| line.sign != '-');
    let mut out = Vec::with_capacity(lines.len() + 1);
    for (idx, line) in lines.into_iter().enumerate() {
        let Some(marker) = line.no_eol else {
            out.push(line);
            continue;
        };
        let old_last = last_old == Some(idx);
        let new_last = last_new == Some(idx);
        let (target_last, other_last) = if reverse {
            (new_last, old_last)
        } else {
            (old_last, new_last)
        };
        match line.sign {
            ' ' if target_last && other_last => out.push(line),
            ' ' if target_last => {
                out.push(PatchLine {
                    sign: '-',
                    text: line.text,
                    no_eol: (!reverse).then_some(marker),
                });
                out.push(PatchLine {
                    sign: '+',
                    text: line.text,
                    no_eol: reverse.then_some(marker),
                });
            }
            ' ' => return None,
            sign => {
                let on_target = (sign == '+') == reverse;
                let last = if sign == '+' { new_last } else { old_last };
                if on_target && !last {
                    return None;
                }
                out.push(PatchLine {
                    no_eol: last.then_some(marker),
                    ..line
                });
            }
        }
    }
    Some(out)
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
        DiffSection::Committed => "COMMITTED",
        DiffSection::Worktree => "WORKING TREE",
    }
}

/// Cells diff row `row` paints with a line gutter, in paint order: the
/// left cell, then the right cell when the row paints side by side
/// (`split`). Section, hunk, and error rows have none.
pub fn diff_row_gutter_cells(row: &DiffRow, split: bool) -> Vec<&DiffCell> {
    match row {
        DiffRow::Line {
            left,
            right: Some(right),
        } if split => vec![left, right],
        DiffRow::Line { left, .. } => vec![left],
        DiffRow::Section(_) | DiffRow::Hunk { .. } | DiffRow::Error { .. } => Vec::new(),
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

/// Columns left for file-diff text after the cursor bar (`▌` / space).
pub fn diff_row_content_width(pane_width: usize) -> usize {
    pane_width
        .saturating_sub(usize::from(DIFF_CURSOR_BAR_COLS))
        .max(1)
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

/// `{path}  inline|split · full? · wrap?` header text (plus pan / scroll when set).
///
/// The pane wraps the path part over [`diff_pane_header_rows`] rows; the
/// extras follow it on its last row.
pub fn diff_pane_header(
    path: &str,
    mode_label: &str,
    full: bool,
    wrap: bool,
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
    if wrap {
        extra.push_str(" · wrap");
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

/// Rows the file-diff path header takes in a pane of `width` × `height`.
///
/// The title (`path`, or `Diff` when empty) wraps by display columns at
/// `width`. The count is capped at half of `height` (floor, at least 1)
/// so the diff body always keeps rows. A zero width or a pane of one row
/// or less gives 1. The muted extras and the scroll position do not
/// count, so the body does not move while it scrolls or pans.
pub fn diff_pane_header_rows(path: &str, width: u16, height: u16) -> u16 {
    if width == 0 || height <= 1 {
        return 1;
    }
    let title = if path.is_empty() { "Diff" } else { path };
    let rows = wrap_col_starts(title, width as usize).len();
    let cap = (height / 2).max(1);
    (rows.min(cap as usize) as u16).max(1)
}

/// Search text for one painted row (code / hunk / section, not raw git).
pub fn row_search_text(row: &DiffRow) -> String {
    match row {
        DiffRow::Section(section) => section_header(*section).to_string(),
        DiffRow::Hunk { text } | DiffRow::Error { text } => text.clone(),
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

/// Visual rows one logical [`DiffRow`] occupies when soft-wrap is on.
///
/// Split rows use the taller side. Continuation rows are extra visual
/// lines of the same logical row (blank gutter / sign on those lines).
pub fn diff_row_visual_height(
    row: &DiffRow,
    content_w: u16,
    gutter_with_mark: usize,
    split: bool,
    split_fraction: f64,
) -> usize {
    let width = (content_w as usize).max(1);
    match row {
        DiffRow::Section(section) => {
            let text = format!(" {} ", section_header(*section));
            wrap_col_starts(&text, width).len().max(1)
        }
        DiffRow::Hunk { text } | DiffRow::Error { text } => {
            wrap_col_starts(text, width).len().max(1)
        }
        DiffRow::Line { left, right } if split && right.is_some() => {
            let cols = side_by_side_column_widths(content_w, split_fraction);
            let left_h = wrap_col_starts(
                &left.text,
                cell_code_width(cols.left_width as usize, gutter_with_mark),
            )
            .len()
            .max(1);
            let right_h = wrap_col_starts(
                &right.as_ref().unwrap().text,
                cell_code_width(cols.right_width as usize, gutter_with_mark),
            )
            .len()
            .max(1);
            left_h.max(right_h)
        }
        DiffRow::Line { left, .. } => {
            wrap_col_starts(&left.text, cell_code_width(width, gutter_with_mark))
                .len()
                .max(1)
        }
    }
}

/// Visual height of every logical row when wrap is on.
pub fn diff_wrap_row_heights(
    rows: &[DiffRow],
    content_w: u16,
    gutter_with_mark: usize,
    split: bool,
    split_fraction: f64,
) -> Vec<usize> {
    rows.iter()
        .map(|row| diff_row_visual_height(row, content_w, gutter_with_mark, split, split_fraction))
        .collect()
}

/// First logical row of a wrap viewport that keeps `cursor` visible.
///
/// Uses the same middle-bias as tree `list_viewport_start`, then snaps to a
/// logical-row boundary so a wrap part is never the first painted line
/// of a new viewport.
pub fn wrap_viewport_start(heights: &[usize], cursor: usize, view_h: usize) -> usize {
    if heights.is_empty() {
        return 0;
    }
    let view_h = view_h.max(1);
    let cursor = cursor.min(heights.len() - 1);
    let prefix: usize = heights[..cursor].iter().copied().map(|h| h.max(1)).sum();
    let total: usize = heights.iter().copied().map(|h| h.max(1)).sum();
    let visual_start = list_viewport_start(total, prefix, view_h);
    let mut acc = 0usize;
    for (i, h) in heights.iter().copied().enumerate() {
        let h = h.max(1);
        if visual_start < acc.saturating_add(h) {
            return i;
        }
        acc = acc.saturating_add(h);
    }
    heights.len() - 1
}

#[cfg(test)]
/// Scroll so `row_index` stays in the upper third of `view_h`.
pub fn scroll_to_keep_row(row_index: usize, view_h: usize, row_count: usize) -> u16 {
    let view_h = view_h.max(1);
    let max_start = row_count.saturating_sub(view_h);
    let prefer = view_h / 3;
    row_index.saturating_sub(prefer).min(max_start) as u16
}

/// Search text of the first visible add/del (else hunk) at `scroll`.
///
/// Full-file reload stores this string. A numeric index from the hunk-only
/// list would scroll to the file start.
pub fn anchor_row_text(rows: &[DiffRow], scroll: usize, view_h: usize) -> String {
    let idx = anchor_row_index(rows, scroll, view_h);
    rows.get(idx).map(row_search_text).unwrap_or_default()
}

/// Index of `needle` in the reloaded row list.
///
/// Exact search text wins. Then a substring. Then the first change at the
/// top of the new list.
pub fn find_anchor_row(rows: &[DiffRow], needle: &str) -> usize {
    if !needle.is_empty() {
        if let Some(i) = rows.iter().position(|r| row_search_text(r) == needle) {
            return i;
        }
        if let Some(i) = rows
            .iter()
            .position(|r| row_search_text(r).contains(needle))
        {
            return i;
        }
    }
    anchor_row_index(rows, 0, rows.len().max(1))
}

/// First visible add/del in the viewport, else nearest hunk at/above scroll.
pub fn anchor_row_index(rows: &[DiffRow], scroll: usize, view_h: usize) -> usize {
    if rows.is_empty() {
        return 0;
    }
    let start = scroll.min(rows.len() - 1);
    let end = (start + view_h.max(1)).min(rows.len());
    for (i, row) in rows.iter().enumerate().take(end).skip(start) {
        if is_change_row(row) {
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
mod tests;

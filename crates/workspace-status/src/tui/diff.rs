//! File-diff load, unified-diff parse, and numbered rows.
//!
//! Path header, line-number gutter, and STAGED / UNSTAGED / NEW labels.
//! Syntax highlighting lives in [`super::syntax`] and paint. Intra-line
//! word diff stays out of scope.

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

use crate::git::{exec_git, git_diff_args};
use crate::snapshot::FileChange;

use super::search::wrap_col_starts;
use super::split::{side_by_side_column_widths, DiffMode};
use super::tree::list_viewport_start;

/// Stub huge / binary untracked files above ~1 MB.
const HUGE_FILE_BYTES: u64 = 1_000_000;

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
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct DiffContent {
    pub staged: String,
    pub unstaged: String,
    /// Label the unstaged section `NEW` (untracked synthesised as all-add).
    pub is_new: bool,
    /// Label the unstaged section `COMMITTED` (compare tabs).
    pub is_committed: bool,
}

impl DiffContent {
    /// Unstaged-only unified text (commit / stash diffs and tests).
    pub fn from_unified(text: impl Into<String>) -> Self {
        Self {
            staged: String::new(),
            unstaged: text.into(),
            is_new: false,
            is_committed: false,
        }
    }

    /// Compare-tab unified text. The section label is `COMMITTED`.
    pub fn from_compare_lines(lines: Vec<String>) -> Self {
        Self {
            staged: String::new(),
            unstaged: lines.join("\n"),
            is_new: false,
            is_committed: true,
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
        is_committed: false,
    }
}

fn untracked_content(repo_dir: &Path, path: &str) -> DiffContent {
    let unstaged = read_untracked_as_diff(&repo_dir.join(path), path);
    let is_new = !unstaged.is_empty();
    DiffContent {
        staged: String::new(),
        unstaged,
        is_new,
        is_committed: false,
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
                text: hunk.header.clone(),
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
        let section = if content.is_committed {
            DiffSection::Committed
        } else if content.is_new {
            DiffSection::New
        } else {
            DiffSection::Unstaged
        };
        out.push((DiffRow::Section(section), RowBind::Section(section)));
        for (idx, hunk) in unstaged.iter().enumerate() {
            push_hunk_rows(&mut out, section, idx, hunk, mode);
        }
    }
    out
}

/// Rows for both diff sections. Empty sections are omitted.
pub fn build_diff_rows(content: &DiffContent, mode: DiffMode) -> Vec<DiffRow> {
    annotated_diff_rows(content, mode)
        .into_iter()
        .map(|(row, _)| row)
        .collect()
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
    pane_width.saturating_sub(1).max(1)
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
        DiffRow::Hunk { text } => wrap_col_starts(text, width).len().max(1),
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
                is_committed: false,
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
                is_committed: false,
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
                is_committed: false,
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
                is_committed: false,
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
                is_committed: false,
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
                is_committed: false,
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
    fn header_rows_fit_on_one_row_for_a_short_path() {
        assert_eq!(diff_pane_header_rows("app/README.md", 40, 20), 1);
        assert_eq!(
            diff_pane_header_rows("", 40, 20),
            1,
            "empty path shows `Diff`"
        );
        assert_eq!(diff_pane_header_rows("abcd", 4, 20), 1, "exact fit");
    }

    #[test]
    fn header_rows_wrap_a_long_path_by_display_columns() {
        assert_eq!(diff_pane_header_rows("abcde", 4, 20), 2);
        assert_eq!(diff_pane_header_rows(&"x".repeat(25), 10, 20), 3);
        // Four wide glyphs are eight columns: two rows at width 4, not one.
        assert_eq!(diff_pane_header_rows("日本語字", 4, 20), 2);
        assert_eq!(
            diff_pane_header_rows("日本語字", 5, 20),
            2,
            "a wide glyph never splits"
        );
    }

    #[test]
    fn header_rows_cap_at_half_the_pane_height() {
        let path = "x".repeat(100);
        assert_eq!(diff_pane_header_rows(&path, 10, 12), 6);
        assert_eq!(diff_pane_header_rows(&path, 10, 7), 3, "floor of half");
        assert_eq!(diff_pane_header_rows(&path, 10, 3), 1);
    }

    #[test]
    fn header_rows_are_one_on_a_tiny_or_zero_width_pane() {
        let path = "x".repeat(100);
        assert_eq!(diff_pane_header_rows(&path, 10, 1), 1);
        assert_eq!(diff_pane_header_rows(&path, 10, 0), 1);
        assert_eq!(diff_pane_header_rows(&path, 0, 20), 1);
    }

    #[test]
    fn header_includes_path_mode_and_optional_full() {
        assert_eq!(
            diff_pane_header("app/README.md", "inline", false, false, 0, 0, 20, 5),
            "app/README.md  inline"
        );
        assert_eq!(
            diff_pane_header("src/lib.rs", "split", true, false, 3, 0, 10, 40),
            "src/lib.rs  split · full · pan 3  10/40"
        );
        assert_eq!(
            diff_pane_header("app/x.rs", "inline", false, true, 0, 0, 20, 5),
            "app/x.rs  inline · wrap"
        );
        assert_eq!(
            diff_pane_mode_label(DiffMode::SideBySide, DiffMode::Inline),
            "inline (too narrow)"
        );
    }

    #[test]
    fn wrap_height_uses_display_columns_and_viewport_snaps() {
        let long = DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Add,
                text: "n".repeat(20),
                line_no: Some(1),
            },
            right: None,
        };
        let height = diff_row_visual_height(&long, 24, 3, false, 0.5);
        assert!(
            height >= 2,
            "20-col add must wrap in the code window: {height}"
        );
        let short = DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Add,
                text: "ok".into(),
                line_no: Some(1),
            },
            right: None,
        };
        assert_eq!(diff_row_visual_height(&short, 24, 3, false, 0.5), 1);
        assert_eq!(wrap_viewport_start(&[1, 1, 1, 1], 2, 2), 1);
        assert_eq!(wrap_viewport_start(&[3, 1, 1], 0, 2), 0);
        assert_eq!(wrap_viewport_start(&[], 0, 10), 0);
    }

    #[test]
    fn blank_content_and_from_lines() {
        assert!(DiffContent::default().is_blank());
        assert!(!DiffContent::from_unified("@@ -1 +1 @@\n+a\n").is_blank());
        let from_vec = DiffContent::from_lines(vec!["@@ -1 +1 @@".into(), "+a".into()]);
        assert!(!from_vec.is_blank());
    }

    #[test]
    fn syntax_fingerprint_changes_when_same_length_text_changes() {
        let alpha = DiffContent::from_lines(vec![
            "@@ -1,3 +1,4 @@".into(),
            " {".into(),
            r#"-  "ttlMs": 5000"#.into(),
            r#"+  "ttlMs": 2000"#.into(),
            r#"+  "name": "alpha-syntax""#.into(),
            " }".into(),
        ]);
        let omega = DiffContent::from_lines(vec![
            "@@ -1,3 +1,4 @@".into(),
            " {".into(),
            r#"-  "ttlMs": 5000"#.into(),
            r#"+  "ttlMs": 2000"#.into(),
            r#"+  "name": "omega-syntax""#.into(),
            " }".into(),
        ]);
        assert_eq!(alpha.unstaged.len(), omega.unstaged.len());
        assert_ne!(alpha.syntax_fingerprint(), omega.syntax_fingerprint());
        assert_eq!(
            alpha.syntax_fingerprint(),
            alpha.clone().syntax_fingerprint()
        );
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

    #[test]
    fn find_anchor_row_follows_change_text_not_old_index() {
        let short = build_diff_rows(
            &DiffContent::from_unified("@@ -46,3 +46,3 @@\n ctx\n-old-line\n+new-line\n"),
            DiffMode::Inline,
        );
        let needle = anchor_row_text(&short, 0, 10);
        let old_idx = anchor_row_index(&short, 0, 10);
        let mut body = String::from("@@ -1,20 +1,20 @@\n");
        for i in 0..15 {
            body.push_str(&format!(" pad-{i}\n"));
        }
        body.push_str("-old-line\n+new-line\n ctx\n");
        let long = build_diff_rows(&DiffContent::from_unified(body), DiffMode::Inline);
        let new_idx = find_anchor_row(&long, &needle);
        assert!(
            new_idx > old_idx,
            "full-file change must sit later than the hunk-only index ({old_idx} vs {new_idx})"
        );
        assert_eq!(row_search_text(&long[new_idx]), needle);
        assert!(
            scroll_to_keep_row(new_idx, 8, long.len()) > scroll_to_keep_row(old_idx, 8, long.len()),
            "stale hunk-only index would not keep the change in view"
        );
    }

    const TWO_HUNKS: &str = "\
diff --git a/regions.txt b/regions.txt
index 1111111..2222222 100644
--- a/regions.txt
+++ b/regions.txt
@@ -1,4 +1,4 @@
 keep-a
 keep-b
-ALPHA-OLD
+ALPHA-NEW
 keep-c
@@ -20,4 +20,4 @@
 keep-x
 keep-y
-OMEGA-OLD
+OMEGA-NEW
 keep-z
";

    fn two_hunk_content() -> DiffContent {
        DiffContent {
            staged: String::new(),
            unstaged: TWO_HUNKS.into(),
            is_new: false,
            is_committed: false,
        }
    }

    fn first_hunk_row_span(mode: DiffMode) -> (usize, usize) {
        let rows = build_diff_rows(&two_hunk_content(), mode);
        let first = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Hunk { .. }))
            .expect("first hunk");
        let second = rows
            .iter()
            .enumerate()
            .skip(first + 1)
            .find_map(|(i, r)| matches!(r, DiffRow::Hunk { .. }).then_some(i))
            .expect("second hunk");
        (first, second - 1)
    }

    #[test]
    fn partial_patch_keeps_only_the_highlighted_hunk() {
        let content = two_hunk_content();
        let (start, end) = first_hunk_row_span(DiffMode::Inline);
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            start,
            end,
            PartialPatchKind::Stage,
            "regions.txt",
        )
        .expect("patch");
        assert!(
            patch.contains("ALPHA-NEW") && patch.contains("ALPHA-OLD"),
            "{patch}"
        );
        assert!(
            !patch.contains("OMEGA-NEW") && !patch.contains("OMEGA-OLD"),
            "{patch}"
        );
        assert!(
            patch.contains("diff --git a/regions.txt b/regions.txt"),
            "{patch}"
        );
    }

    #[test]
    fn partial_patch_stages_one_add_line_inside_a_hunk() {
        let content = DiffContent::from_unified(FIXTURE);
        let rows = build_diff_rows(&content, DiffMode::Inline);
        let line4 = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == "line4"))
            .expect("line4");
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            line4,
            line4,
            PartialPatchKind::Stage,
            "hello.ts",
        )
        .expect("patch");
        assert!(patch.contains("+line4"), "{patch}");
        assert!(!patch.contains("line2 changed"), "{patch}");
        assert!(patch.contains(" line2\n"), "{patch}");
    }

    #[test]
    fn partial_patch_fails_closed_on_context_only_and_committed() {
        let content = two_hunk_content();
        let rows = build_diff_rows(&content, DiffMode::Inline);
        let ctx = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == "keep-a"))
            .expect("context");
        let err = build_partial_patch(
            &content,
            DiffMode::Inline,
            ctx,
            ctx,
            PartialPatchKind::Stage,
            "regions.txt",
        )
        .unwrap_err();
        assert!(err.contains("nothing to stage"), "{err}");

        let committed = DiffContent {
            staged: String::new(),
            unstaged: TWO_HUNKS.into(),
            is_new: false,
            is_committed: true,
        };
        let err = build_partial_patch(
            &committed,
            DiffMode::Inline,
            0,
            3,
            PartialPatchKind::Stage,
            "regions.txt",
        )
        .unwrap_err();
        assert!(err.contains("committed"), "{err}");
    }

    #[test]
    fn partial_patch_unstages_from_the_staged_section() {
        let content = DiffContent {
            staged: TWO_HUNKS.into(),
            unstaged: String::new(),
            is_new: false,
            is_committed: false,
        };
        let rows = build_diff_rows(&content, DiffMode::Inline);
        let first = rows
            .iter()
            .position(|r| matches!(r, DiffRow::Hunk { .. }))
            .expect("hunk");
        let second = rows
            .iter()
            .enumerate()
            .skip(first + 1)
            .find_map(|(i, r)| matches!(r, DiffRow::Hunk { .. }).then_some(i))
            .expect("second");
        let patch = build_partial_patch(
            &content,
            DiffMode::Inline,
            first,
            second - 1,
            PartialPatchKind::Unstage,
            "regions.txt",
        )
        .expect("patch");
        assert!(
            patch.contains("ALPHA-NEW") && !patch.contains("OMEGA-NEW"),
            "{patch}"
        );
        let err = build_partial_patch(
            &content,
            DiffMode::Inline,
            first,
            second - 1,
            PartialPatchKind::Stage,
            "regions.txt",
        )
        .unwrap_err();
        assert!(err.contains("nothing to stage"), "{err}");
    }

    const BINARY_STUB: &str = "Binary files a/x and b/x differ\n";

    #[test]
    fn partial_patch_fails_closed_on_binary() {
        let unstaged = DiffContent::from_unified(BINARY_STUB);
        let end = build_diff_rows(&unstaged, DiffMode::Inline)
            .len()
            .saturating_sub(1);
        let err = build_partial_patch(
            &unstaged,
            DiffMode::Inline,
            0,
            end,
            PartialPatchKind::Stage,
            "x.bin",
        )
        .unwrap_err();
        assert!(err.contains("cannot stage a binary highlight"), "{err}");

        let staged = DiffContent {
            staged: BINARY_STUB.into(),
            unstaged: String::new(),
            is_new: false,
            is_committed: false,
        };
        let end = build_diff_rows(&staged, DiffMode::Inline)
            .len()
            .saturating_sub(1);
        let err = build_partial_patch(
            &staged,
            DiffMode::Inline,
            0,
            end,
            PartialPatchKind::Unstage,
            "x.bin",
        )
        .unwrap_err();
        assert!(err.contains("cannot unstage a binary highlight"), "{err}");
    }

    #[test]
    fn partial_patch_fails_closed_on_mixed_sections() {
        let content = DiffContent {
            staged: TWO_HUNKS.into(),
            unstaged: TWO_HUNKS.into(),
            is_new: false,
            is_committed: false,
        };
        let end = build_diff_rows(&content, DiffMode::Inline).len() - 1;
        let err = build_partial_patch(
            &content,
            DiffMode::Inline,
            0,
            end,
            PartialPatchKind::Stage,
            "regions.txt",
        )
        .unwrap_err();
        assert!(err.contains("highlight spans staged and unstaged"), "{err}");
        let err = build_partial_patch(
            &content,
            DiffMode::Inline,
            0,
            end,
            PartialPatchKind::Unstage,
            "regions.txt",
        )
        .unwrap_err();
        assert!(err.contains("highlight spans staged and unstaged"), "{err}");
    }

    fn revert_patch(content: &DiffContent, start: usize, end: usize) -> Result<String, String> {
        build_partial_patch(
            content,
            DiffMode::Inline,
            start,
            end,
            PartialPatchKind::Revert,
            "regions.txt",
        )
    }

    fn line_row(content: &DiffContent, text: &str) -> usize {
        build_diff_rows(content, DiffMode::Inline)
            .iter()
            .position(|r| matches!(r, DiffRow::Line { left, .. } if left.text == text))
            .unwrap_or_else(|| panic!("row {text}"))
    }

    const PAIRED_HUNK: &str = "\
diff --git a/regions.txt b/regions.txt
index 1111111..2222222 100644
--- a/regions.txt
+++ b/regions.txt
@@ -1,4 +1,4 @@
 ctx-a
-old-1
-old-2
+new-1
+new-2
 ctx-b
";

    #[test]
    fn revert_patch_mirrors_the_stage_transform() {
        let content = DiffContent::from_unified(PAIRED_HUNK);
        let row = line_row(&content, "new-1");
        let patch = revert_patch(&content, row, row).expect("patch");
        let hunk = &patch[patch.find("@@").expect("hunk")..];
        assert_eq!(
            hunk, "@@ -1,3 +1,4 @@\n ctx-a\n+new-1\n new-2\n ctx-b\n",
            "unselected + is context, unselected - drops"
        );
        let stage = build_partial_patch(
            &content,
            DiffMode::Inline,
            row,
            row,
            PartialPatchKind::Stage,
            "regions.txt",
        )
        .expect("stage patch");
        let hunk = &stage[stage.find("@@").expect("hunk")..];
        assert_eq!(
            hunk, "@@ -1,4 +1,5 @@\n ctx-a\n old-1\n+new-1\n old-2\n ctx-b\n",
            "stage keeps the forward transform; new-1 lands in pair order"
        );
    }

    #[test]
    fn revert_patch_fails_closed() {
        let content = two_hunk_content();
        let ctx = line_row(&content, "keep-a");
        let err = revert_patch(&content, ctx, ctx).unwrap_err();
        assert_eq!(err, "nothing to revert in highlight");

        let committed = DiffContent {
            is_committed: true,
            ..two_hunk_content()
        };
        let err = revert_patch(&committed, 0, 3).unwrap_err();
        assert_eq!(err, "cannot revert a committed diff");

        let binary = DiffContent::from_unified(BINARY_STUB);
        let end = build_diff_rows(&binary, DiffMode::Inline).len() - 1;
        let err = revert_patch(&binary, 0, end).unwrap_err();
        assert_eq!(err, "cannot revert a binary highlight");

        let new_file = DiffContent {
            staged: String::new(),
            unstaged: synthesize_all_add_diff("one\ntwo\n"),
            is_new: true,
            is_committed: false,
        };
        let end = build_diff_rows(&new_file, DiffMode::Inline).len() - 1;
        let err = revert_patch(&new_file, 0, end).unwrap_err();
        assert_eq!(err, "cannot revert lines of a new file");

        // `git add -N`: tracked, so not `is_new`, but the unstaged diff is
        // a new-file patch.
        let intent_to_add = DiffContent::from_unified(
            "diff --git a/regions.txt b/regions.txt\nnew file mode 100644\nindex 0000000..1111111\n--- /dev/null\n+++ b/regions.txt\n@@ -0,0 +1,2 @@\n+one\n+two\n",
        );
        assert!(!intent_to_add.is_new);
        let end = build_diff_rows(&intent_to_add, DiffMode::Inline).len() - 1;
        let err = revert_patch(&intent_to_add, 0, end).unwrap_err();
        assert_eq!(err, "cannot revert lines of a new file");

        let staged_only = DiffContent {
            staged: TWO_HUNKS.into(),
            unstaged: String::new(),
            is_new: false,
            is_committed: false,
        };
        let end = build_diff_rows(&staged_only, DiffMode::Inline).len() - 1;
        let err = revert_patch(&staged_only, 0, end).unwrap_err();
        assert_eq!(err, "nothing to revert in highlight");

        let mixed = DiffContent {
            staged: TWO_HUNKS.into(),
            unstaged: TWO_HUNKS.into(),
            is_new: false,
            is_committed: false,
        };
        let end = build_diff_rows(&mixed, DiffMode::Inline).len() - 1;
        let err = revert_patch(&mixed, 0, end).unwrap_err();
        assert_eq!(err, "highlight spans staged and unstaged");

        let renamed = DiffContent::from_unified(
            "diff --git a/old.txt b/regions.txt\nsimilarity index 90%\nrename from old.txt\nrename to regions.txt\n--- a/old.txt\n+++ b/regions.txt\n@@ -1,2 +1,2 @@\n keep\n-a\n+b\n",
        );
        let end = build_diff_rows(&renamed, DiffMode::Inline).len() - 1;
        let err = revert_patch(&renamed, 0, end).unwrap_err();
        assert_eq!(err, "cannot revert highlight");

        // A reverse apply would undo the mode change or recreate the file.
        for header in [
            "old mode 100644\nnew mode 100755\n",
            "deleted file mode 100644\n",
        ] {
            let content = DiffContent::from_unified(format!(
                "diff --git a/regions.txt b/regions.txt\n{header}--- a/regions.txt\n+++ b/regions.txt\n@@ -1,2 +1,2 @@\n keep\n-a\n+b\n"
            ));
            let end = build_diff_rows(&content, DiffMode::Inline).len() - 1;
            let err = revert_patch(&content, 0, end).unwrap_err();
            assert_eq!(err, "cannot revert highlight", "{header}");
        }
    }

    mod real_git {
        use std::fs;
        use std::path::Path;

        use super::*;
        use crate::git::{apply_cached_patch, apply_worktree_patch_reverse, blob_bytes};
        use crate::testutil::{git, init_repo, unique_dir};

        const REGIONS: &str = "\
keep-a
keep-b
keep-c
ALPHA-OLD
keep-d
keep-e
keep-f
pad-1
pad-2
pad-3
pad-4
pad-5
pad-6
OMEGA-OLD
keep-x
keep-y
keep-z
";

        fn repo_with_regions(prefix: &str) -> std::path::PathBuf {
            let dir = unique_dir(prefix);
            init_repo(&dir);
            fs::write(dir.join("regions.txt"), REGIONS).unwrap();
            git(&dir, &["add", "regions.txt"]);
            git(&dir, &["commit", "-q", "-m", "regions"]);
            dir
        }

        fn content(dir: &Path) -> DiffContent {
            DiffContent {
                staged: exec_git(&["diff", "--cached", "--", "regions.txt"], dir),
                unstaged: exec_git(&["diff", "--", "regions.txt"], dir),
                is_new: false,
                is_committed: false,
            }
        }

        fn worktree(dir: &Path) -> String {
            fs::read_to_string(dir.join("regions.txt")).unwrap()
        }

        /// `main` commits [`REGIONS`]; `feature` commits ALPHA-NEW and
        /// OMEGA-NEW and a new `summary.txt`, and is checked out clean.
        /// Returns the dir and the compare content for `path`.
        fn committed_compare(prefix: &str, path: &str) -> (std::path::PathBuf, DiffContent) {
            let dir = repo_with_regions(prefix);
            git(&dir, &["checkout", "-q", "-b", "feature"]);
            let edited = REGIONS
                .replace("ALPHA-OLD", "ALPHA-NEW")
                .replace("OMEGA-OLD", "OMEGA-NEW");
            fs::write(dir.join("regions.txt"), edited).unwrap();
            fs::write(dir.join("summary.txt"), "summary\n").unwrap();
            git(&dir, &["add", "."]);
            git(&dir, &["commit", "-q", "-m", "feature"]);
            let lines = crate::git::diff_compare_file_ctx(&dir, "main", "HEAD", path, None, None)
                .expect("compare diff");
            (dir, DiffContent::from_compare_lines(lines))
        }

        #[test]
        fn revert_committed_restores_one_compare_hunk_to_the_merge_base() {
            let (dir, content) = committed_compare("ws-diff-revert-committed", "regions.txt");
            let rows = build_diff_rows(&content, DiffMode::Inline);
            assert!(matches!(rows[0], DiffRow::Section(DiffSection::Committed)));
            let alpha = line_row(&content, "ALPHA-NEW");
            let patch = build_partial_patch(
                &content,
                DiffMode::Inline,
                1,
                alpha,
                PartialPatchKind::RevertCommitted,
                "regions.txt",
            )
            .expect("patch");
            apply_worktree_patch_reverse(&dir, &patch).expect("revert");

            assert_eq!(worktree(&dir), REGIONS.replace("OMEGA-OLD", "OMEGA-NEW"));
            assert!(exec_git(&["diff", "--cached"], &dir).is_empty());

            // Plain Revert still refuses a committed diff.
            let err = build_partial_patch(
                &content,
                DiffMode::Inline,
                1,
                alpha,
                PartialPatchKind::Revert,
                "regions.txt",
            )
            .unwrap_err();
            assert_eq!(err, "cannot revert a committed diff");
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn revert_committed_refuses_new_files_and_worktree_diffs() {
            let (dir, added) = committed_compare("ws-diff-revert-committed-new", "summary.txt");
            let end = build_diff_rows(&added, DiffMode::Inline).len() - 1;
            let err = build_partial_patch(
                &added,
                DiffMode::Inline,
                0,
                end,
                PartialPatchKind::RevertCommitted,
                "summary.txt",
            )
            .unwrap_err();
            assert_eq!(err, "cannot revert lines of a new file");

            fs::write(dir.join("regions.txt"), "dirty\n").unwrap();
            let worktree_diff = content(&dir);
            let err = build_partial_patch(
                &worktree_diff,
                DiffMode::Inline,
                0,
                3,
                PartialPatchKind::RevertCommitted,
                "regions.txt",
            )
            .unwrap_err();
            assert_eq!(err, "cannot revert highlight");
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn revert_range_restores_only_the_highlighted_hunk() {
            let dir = repo_with_regions("ws-diff-revert-hunk");
            let staged_body = REGIONS.replace("keep-x", "KEEP-X-STAGED");
            fs::write(dir.join("regions.txt"), &staged_body).unwrap();
            git(&dir, &["add", "regions.txt"]);
            let edited = staged_body
                .replace("ALPHA-OLD", "ALPHA-NEW")
                .replace("OMEGA-OLD", "OMEGA-NEW");
            fs::write(dir.join("regions.txt"), &edited).unwrap();
            let cached_before = exec_git(&["diff", "--cached"], &dir);

            let content = content(&dir);
            let rows = build_diff_rows(&content, DiffMode::Inline);
            let unstaged = rows
                .iter()
                .position(|r| matches!(r, DiffRow::Section(DiffSection::Unstaged)))
                .expect("unstaged section");
            let alpha = line_row(&content, "ALPHA-NEW");
            let patch = build_partial_patch(
                &content,
                DiffMode::Inline,
                unstaged + 1,
                alpha,
                PartialPatchKind::Revert,
                "regions.txt",
            )
            .expect("patch");
            apply_worktree_patch_reverse(&dir, &patch).expect("revert");

            assert_eq!(worktree(&dir), edited.replace("ALPHA-NEW", "ALPHA-OLD"));
            assert_eq!(exec_git(&["diff", "--cached"], &dir), cached_before);
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn revert_range_drops_one_added_line_and_keeps_its_neighbour() {
            let dir = repo_with_regions("ws-diff-revert-line");
            let edited = REGIONS
                .replace("keep-b\n", "keep-b\nadd-1\nadd-2\n")
                .replace("keep-c", "KEEP-C-NEW");
            fs::write(dir.join("regions.txt"), &edited).unwrap();

            let content = content(&dir);
            let row = line_row(&content, "add-1");
            let patch = build_partial_patch(
                &content,
                DiffMode::Inline,
                row,
                row,
                PartialPatchKind::Revert,
                "regions.txt",
            )
            .expect("patch");
            apply_worktree_patch_reverse(&dir, &patch).expect("revert");

            assert_eq!(worktree(&dir), edited.replace("add-1\n", ""));
            assert!(exec_git(&["diff", "--cached"], &dir).is_empty());
            let _ = fs::remove_dir_all(&dir);
        }

        #[test]
        fn split_mode_revert_of_one_paired_row_keeps_the_other_edits() {
            let dir = repo_with_regions("ws-diff-revert-split");
            let edited = REGIONS
                .replace("keep-b", "KEEP-B")
                .replace("keep-d", "KEEP-D");
            fs::write(dir.join("regions.txt"), &edited).unwrap();

            let content = content(&dir);
            let rows = build_diff_rows(&content, DiffMode::SideBySide);
            let paired = rows
                .iter()
                .position(|r| {
                    matches!(r, DiffRow::Line { left, right: Some(right) }
                        if left.text == "keep-b" && right.text == "KEEP-B")
                })
                .expect("paired keep-b / KEEP-B row");
            let hunks = rows
                .iter()
                .filter(|r| matches!(r, DiffRow::Hunk { .. }))
                .count();
            assert_eq!(hunks, 1, "both edits share one hunk");
            let patch = build_partial_patch(
                &content,
                DiffMode::SideBySide,
                paired,
                paired,
                PartialPatchKind::Revert,
                "regions.txt",
            )
            .expect("patch");
            apply_worktree_patch_reverse(&dir, &patch).expect("revert");

            assert_eq!(worktree(&dir), edited.replace("KEEP-B", "keep-b"));
            assert!(exec_git(&["diff", "--cached"], &dir).is_empty());
            let _ = fs::remove_dir_all(&dir);
        }

        /// Change `keep-b` / `keep-c` (one run of two pairs), highlight one
        /// row, apply `kind`, and return (index, worktree) for regions.txt.
        fn pair_run_apply(
            mode: DiffMode,
            kind: PartialPatchKind,
            left: &str,
            right: Option<&str>,
        ) -> (String, String) {
            let dir = repo_with_regions("ws-diff-pair-run");
            let edited = REGIONS
                .replace("keep-b", "KEEP-B")
                .replace("keep-c", "KEEP-C");
            fs::write(dir.join("regions.txt"), &edited).unwrap();
            let content = content(&dir);
            let row = build_diff_rows(&content, mode)
                .iter()
                .position(|r| match r {
                    DiffRow::Line { left: l, right: r } => {
                        l.text == left
                            && right.is_none_or(|want| r.as_ref().is_some_and(|r| r.text == want))
                    }
                    _ => false,
                })
                .unwrap_or_else(|| panic!("row {left} / {right:?}"));
            let patch =
                build_partial_patch(&content, mode, row, row, kind, "regions.txt").expect("patch");
            match kind {
                PartialPatchKind::Stage => apply_cached_patch(&dir, &patch, false),
                PartialPatchKind::Unstage => apply_cached_patch(&dir, &patch, true),
                PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
                    apply_worktree_patch_reverse(&dir, &patch)
                }
            }
            .unwrap_or_else(|err| panic!("{kind:?} {left}: {err}\n{patch}"));
            let index = blob_bytes(&dir, "", "regions.txt").expect("index blob");
            let out = (String::from_utf8(index).unwrap(), worktree(&dir));
            let _ = fs::remove_dir_all(&dir);
            out
        }

        #[test]
        fn one_pair_of_a_change_run_keeps_line_order() {
            use DiffMode::{Inline, SideBySide};
            use PartialPatchKind::{Revert, Stage};
            let edited = REGIONS
                .replace("keep-b", "KEEP-B")
                .replace("keep-c", "KEEP-C");
            let only_c = REGIONS.replace("keep-c", "KEEP-C");
            let only_b = REGIONS.replace("keep-b", "KEEP-B");
            let both_b = REGIONS.replace("keep-b\n", "keep-b\nKEEP-B\n");
            let c_back = edited.replace("KEEP-B\n", "KEEP-B\nkeep-c\n");
            // Split binds a pair to one row. Inline has no row for a whole
            // pair, so it picks the one line whose place depends on the pair.
            let cases = [
                (
                    SideBySide,
                    Stage,
                    "keep-c",
                    Some("KEEP-C"),
                    &only_c,
                    &edited,
                ),
                (
                    SideBySide,
                    Stage,
                    "keep-b",
                    Some("KEEP-B"),
                    &only_b,
                    &edited,
                ),
                (
                    SideBySide,
                    Revert,
                    "keep-c",
                    Some("KEEP-C"),
                    &REGIONS.to_string(),
                    &only_b,
                ),
                (
                    SideBySide,
                    Revert,
                    "keep-b",
                    Some("KEEP-B"),
                    &REGIONS.to_string(),
                    &only_c,
                ),
                (Inline, Stage, "KEEP-B", None, &both_b, &edited),
                (
                    Inline,
                    Revert,
                    "keep-c",
                    None,
                    &REGIONS.to_string(),
                    &c_back,
                ),
            ];
            for (mode, kind, left, right, index, worktree) in cases {
                assert_eq!(
                    pair_run_apply(mode, kind, left, right),
                    (index.clone(), worktree.clone()),
                    "{mode:?} {kind:?} {left}"
                );
            }
        }

        /// `l1`..`l30` with the chosen edits: `X1` / `X2` after `l2` (the
        /// first hunk changes the line count), `l12` gone, `ADD-Y` after
        /// `l22`.
        fn shifted_body(x1: bool, x2: bool, drop_l12: bool, add_y: bool) -> String {
            let mut out = String::new();
            for n in 1..=30 {
                if !(drop_l12 && n == 12) {
                    out.push_str(&format!("l{n}\n"));
                }
                if n == 2 {
                    if x1 {
                        out.push_str("X1\n");
                    }
                    if x2 {
                        out.push_str("X2\n");
                    }
                }
                if n == 22 && add_y {
                    out.push_str("ADD-Y\n");
                }
            }
            out
        }

        /// Highlight rows `from..=to` of a three-hunk diff, apply `kind`,
        /// and return (index, worktree). `context` sets `diff.context`.
        fn shifted_apply(
            context: Option<&str>,
            kind: PartialPatchKind,
            from: &str,
            to: &str,
        ) -> (String, String) {
            let dir = unique_dir("ws-diff-hunk-start");
            init_repo(&dir);
            if let Some(n) = context {
                git(&dir, &["config", "diff.context", n]);
            }
            let file = dir.join("lines.txt");
            fs::write(&file, shifted_body(false, false, false, false)).unwrap();
            git(&dir, &["add", "lines.txt"]);
            git(&dir, &["commit", "-q", "-m", "lines"]);
            fs::write(&file, shifted_body(true, true, true, true)).unwrap();
            if kind == PartialPatchKind::Unstage {
                git(&dir, &["add", "lines.txt"]);
            }
            let content = DiffContent {
                staged: exec_git(&["diff", "--cached", "--", "lines.txt"], &dir),
                unstaged: exec_git(&["diff", "--", "lines.txt"], &dir),
                is_new: false,
                is_committed: false,
            };
            let hunks = parse_unified_diff(if kind == PartialPatchKind::Unstage {
                &content.staged
            } else {
                &content.unstaged
            })
            .len();
            assert_eq!(hunks, 3, "three separate hunks at context {context:?}");
            let patch = build_partial_patch(
                &content,
                DiffMode::Inline,
                line_row(&content, from),
                line_row(&content, to),
                kind,
                "lines.txt",
            )
            .expect("patch");
            match kind {
                PartialPatchKind::Stage => apply_cached_patch(&dir, &patch, false),
                PartialPatchKind::Unstage => apply_cached_patch(&dir, &patch, true),
                PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
                    apply_worktree_patch_reverse(&dir, &patch)
                }
            }
            .unwrap_or_else(|err| panic!("{kind:?} {from}..{to}: {err}\n{patch}"));
            let index = blob_bytes(&dir, "", "lines.txt").expect("index blob");
            let out = (
                String::from_utf8(index).unwrap(),
                fs::read_to_string(&file).unwrap(),
            );
            let _ = fs::remove_dir_all(&dir);
            out
        }

        #[test]
        fn later_hunks_land_after_skipped_or_partial_earlier_hunks() {
            use PartialPatchKind::{Revert, Stage, Unstage};
            let base = shifted_body(false, false, false, false);
            let edited = shifted_body(true, true, true, true);
            // (from, to, stage result, unstage / revert result)
            let picks = [
                (
                    "ADD-Y",
                    "ADD-Y",
                    shifted_body(false, false, false, true),
                    shifted_body(true, true, true, false),
                ),
                (
                    "l12",
                    "l12",
                    shifted_body(false, false, true, false),
                    shifted_body(true, true, false, true),
                ),
                (
                    "X2",
                    "l12",
                    shifted_body(false, true, true, false),
                    shifted_body(true, false, false, true),
                ),
            ];
            for context in [Some("0"), None] {
                for (from, to, staged, undone) in &picks {
                    let label = format!("context {context:?} {from}..{to}");
                    assert_eq!(
                        shifted_apply(context, Stage, from, to),
                        (staged.clone(), edited.clone()),
                        "Stage {label}"
                    );
                    assert_eq!(
                        shifted_apply(context, Unstage, from, to),
                        (undone.clone(), edited.clone()),
                        "Unstage {label}"
                    );
                    assert_eq!(
                        shifted_apply(context, Revert, from, to),
                        (base.clone(), undone.clone()),
                        "Revert {label}"
                    );
                }
            }
        }

        const NO_EOL_BASE: &str = "a\nb\nlast";
        const NO_EOL_EDIT: &str = "a\nb\nLAST";

        /// Highlight one row of a last-line change in a file with no
        /// trailing newline, apply `kind`, and return (index, worktree).
        fn no_eol_apply(
            prefix: &str,
            kind: PartialPatchKind,
            row: &str,
        ) -> Result<(String, String), String> {
            let dir = unique_dir(prefix);
            init_repo(&dir);
            let file = dir.join("no-eol.txt");
            fs::write(&file, NO_EOL_BASE).unwrap();
            git(&dir, &["add", "no-eol.txt"]);
            git(&dir, &["commit", "-q", "-m", "no eol"]);
            fs::write(&file, NO_EOL_EDIT).unwrap();
            if kind == PartialPatchKind::Unstage {
                git(&dir, &["add", "no-eol.txt"]);
            }
            let content = DiffContent {
                staged: exec_git(&["diff", "--cached", "--", "no-eol.txt"], &dir),
                unstaged: exec_git(&["diff", "--", "no-eol.txt"], &dir),
                is_new: false,
                is_committed: false,
            };
            let row = line_row(&content, row);
            let result =
                build_partial_patch(&content, DiffMode::Inline, row, row, kind, "no-eol.txt")
                    .and_then(|patch| match kind {
                        PartialPatchKind::Stage => apply_cached_patch(&dir, &patch, false),
                        PartialPatchKind::Unstage => apply_cached_patch(&dir, &patch, true),
                        PartialPatchKind::Revert | PartialPatchKind::RevertCommitted => {
                            apply_worktree_patch_reverse(&dir, &patch)
                        }
                    })
                    .map(|()| {
                        let index = blob_bytes(&dir, "", "no-eol.txt").expect("index blob");
                        (
                            String::from_utf8(index).unwrap(),
                            fs::read_to_string(&file).unwrap(),
                        )
                    });
            let _ = fs::remove_dir_all(&dir);
            result
        }

        #[test]
        fn no_eol_last_line_moves_one_side_without_joining_lines() {
            use PartialPatchKind::{Revert, Stage, Unstage};
            let restored = "a\nb\nlast\nLAST";
            let cases: [(PartialPatchKind, &str, &str, &str); 6] = [
                (Stage, "last", "a\nb\n", NO_EOL_EDIT),
                (Stage, "LAST", restored, NO_EOL_EDIT),
                (Unstage, "last", restored, NO_EOL_EDIT),
                (Unstage, "LAST", "a\nb\n", NO_EOL_EDIT),
                (Revert, "last", NO_EOL_BASE, restored),
                (Revert, "LAST", NO_EOL_BASE, "a\nb\n"),
            ];
            for (kind, row, index, worktree) in cases {
                let got = no_eol_apply("ws-diff-no-eol", kind, row)
                    .unwrap_or_else(|err| panic!("{kind:?} {row}: {err}"));
                assert_eq!(
                    got,
                    (index.to_string(), worktree.to_string()),
                    "{kind:?} {row}"
                );
            }
        }

        #[test]
        fn unstage_range_keeps_an_adjacent_staged_addition() {
            let dir = repo_with_regions("ws-diff-unstage-line");
            let edited = REGIONS.replace("keep-b\n", "keep-b\nadd-1\nadd-2\n");
            fs::write(dir.join("regions.txt"), &edited).unwrap();
            git(&dir, &["add", "regions.txt"]);

            let content = content(&dir);
            let row = line_row(&content, "add-1");
            let patch = build_partial_patch(
                &content,
                DiffMode::Inline,
                row,
                row,
                PartialPatchKind::Unstage,
                "regions.txt",
            )
            .expect("patch");
            apply_cached_patch(&dir, &patch, true).expect("unstage one line");

            let cached = exec_git(&["diff", "--cached"], &dir);
            let unstaged = exec_git(&["diff"], &dir);
            assert!(
                cached.contains("+add-2") && !cached.contains("add-1"),
                "{cached}"
            );
            assert!(
                unstaged.contains("+add-1") && !unstaged.contains("+add-2"),
                "{unstaged}"
            );
            assert_eq!(worktree(&dir), edited);
            let _ = fs::remove_dir_all(&dir);
        }
    }
}

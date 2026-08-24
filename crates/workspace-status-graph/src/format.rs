//! Headless line format for graph rows. The widget paints these strings.

use crate::glyphs::GlyphSet;
use crate::model::{Commit, GraphRef, GraphRow, RefKind, Stash, SyncState, SyncStatus, Worktree};

/// Prefer keeping at least this many left columns before dropping right meta.
const MIN_LEFT_KEEP: usize = 12;

/// Cap for the author column.
const AUTHOR_COL_MAX: usize = 16;

/// Format the sync header line (`branch` plus ahead/behind marks).
pub fn format_sync(sync: &SyncState, glyphs: &GlyphSet) -> String {
    let mut parts = vec![sync.branch.clone()];
    match sync.status {
        SyncStatus::NoUpstream => parts.push("no-upstream".to_string()),
        SyncStatus::UpToDate => {}
        SyncStatus::Ahead | SyncStatus::Behind | SyncStatus::Diverged => {
            if sync.ahead > 0 {
                parts.push(format!("{}{}", glyphs.ahead, sync.ahead));
            }
            if sync.behind > 0 {
                parts.push(format!("{}{}", glyphs.behind, sync.behind));
            }
        }
    }
    parts.join(" ")
}

/// Relative ages only up to 3 hours (iOS notification style). Older
/// timestamps paint as UTC `YYYY-MM-DD HH:MM`.
pub const RELATIVE_DATE_LIMIT_SECS: i64 = 3 * 3600;

/// One styled run of graph label / spacer text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelPart {
    /// Visible text.
    pub text: String,
    /// How the widget should colour this run.
    pub kind: LabelKind,
}

/// Semantic colour role for a [`LabelPart`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    /// Commit / stash subject (stronger than meta).
    Subject,
    /// Hash, date, author, worktree marks, padding.
    Meta,
    /// Detached `[HEAD]` / checkout glyph.
    ChipHead,
    /// Local (or merged) non-default branch chip.
    ChipLocal,
    /// Default-branch chip (`main` / override).
    ChipDefault,
    /// Unmatched remote-tracking chip.
    ChipRemote,
    /// Tag chip.
    ChipTag,
    /// Hidden leftover branch/tag count (`[+N]`) when chips do not fit.
    Overflow,
}

/// Chip text for leftover hidden branch/tag refs (`[+N]`).
///
/// Always a whole chip (brackets). Callers reserve this width so the
/// spacer never mid-clips a hidden-ref count the way a raw `+N` blends
/// into hash/date meta.
pub fn overflow_chip_text(hidden: usize) -> String {
    format!("[+{hidden}]")
}

/// Compact age or absolute timestamp.
pub fn format_relative_date(unix: i64, now_unix: i64) -> String {
    let delta = (now_unix - unix).max(0);
    if delta <= RELATIVE_DATE_LIMIT_SECS {
        if delta < 60 {
            return "just now".into();
        }
        if delta < 3600 {
            return format!("{}m", delta / 60);
        }
        return format!("{}h", delta / 3600);
    }
    format_utc_timestamp(unix)
}

/// UTC `YYYY-MM-DD HH:MM` without extra crates.
pub fn format_utc_timestamp(unix: i64) -> String {
    let unix = unix.max(0);
    let days = unix.div_euclid(86_400);
    let rem = unix.rem_euclid(86_400) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}")
}

/// Howard Hinnant civil-from-days: days since Unix epoch → UTC Y-M-D.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y as i32, m as u32, d as u32)
}

/// Format one visible row. Headless callers share this function.
/// The widget paints a multi-lane gutter and uses [`format_label`].
pub fn format_row(row: &GraphRow, glyphs: &GlyphSet) -> String {
    match row {
        GraphRow::Uncommitted { has_changes } => {
            if *has_changes {
                format!("{} uncommitted changes", glyphs.uncommitted)
            } else {
                format!("{} working tree clean", glyphs.uncommitted)
            }
        }
        GraphRow::Stash(stash) => format_stash(stash, glyphs),
        GraphRow::Commit {
            commit, is_head, ..
        } => {
            let node = if *is_head {
                glyphs.head_commit
            } else {
                glyphs.commit
            };
            format!("{} {}", node, format_commit_subject(commit))
        }
        GraphRow::Worktree(worktree) => format_worktree(worktree, glyphs),
    }
}

/// Label after the gutter. Commit and stash drop the node glyph because
/// the gutter already paints it. Subjects stay on this line; refs /
/// `stash@{n}`, hash, date, and author live on the spacer beneath.
pub fn format_label(row: &GraphRow, glyphs: &GlyphSet) -> String {
    match row {
        GraphRow::Uncommitted { .. } | GraphRow::Worktree(_) => format_row(row, glyphs),
        GraphRow::Stash(stash) => stash.subject.clone(),
        GraphRow::Commit { commit, .. } => format_commit_subject(commit),
    }
}

/// Inputs for [`format_commit_spacer`].
pub struct CommitSpacerOpts<'a> {
    /// Commit whose spacer is painted.
    pub commit: &'a Commit,
    /// True when this commit is HEAD.
    pub is_head: bool,
    /// Worktrees whose HEAD is this commit.
    pub worktrees: &'a [Worktree],
    /// Checked-out branch name from sync chrome (`None` when unknown).
    pub head_branch: Option<&'a str>,
    /// Glyph set for checkout / sync marks and worktree icons.
    pub glyphs: &'a GlyphSet,
    /// Columns after the gutter gap. Refs stay left; meta is right-anchored.
    pub available: usize,
    /// Padded relative-date column width (rails across rows).
    pub date_width: usize,
    /// Padded author column width (rails across rows).
    pub author_width: usize,
    /// Clock for relative dates (unix seconds).
    pub now_unix: i64,
    /// Configured default-branch override (`None` → main/master/develop).
    pub default_branch_override: Option<&'a str>,
}

/// Which right-meta columns fit beside a left flex region.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetaCols {
    /// Short hash.
    pub hash: bool,
    /// Relative date.
    pub date: bool,
    /// Author name.
    pub author: bool,
}

/// Right-anchored ` hash date author` string; drop order hash → date → author.
/// Empty author omits the author column even when `cols.author` is true.
pub fn meta_columns_text(hash: &str, date: &str, author: &str, cols: MetaCols) -> String {
    let mut meta = String::new();
    if cols.hash {
        meta.push(' ');
        meta.push_str(hash);
    }
    if cols.date {
        meta.push(' ');
        meta.push_str(date);
    }
    if cols.author && !author.is_empty() {
        meta.push(' ');
        meta.push_str(author);
    }
    meta
}

/// Choose which right-meta columns fit beside a left flex region.
///
/// Prefers keeping `min_left_keep` columns for refs. Drop order: hash → date → author.
pub fn pick_meta_columns(
    available_after_gutter: usize,
    hash: &str,
    date: &str,
    author: &str,
    min_left_keep: usize,
) -> MetaCols {
    let candidates = [
        MetaCols {
            hash: true,
            date: true,
            author: true,
        },
        MetaCols {
            hash: false,
            date: true,
            author: true,
        },
        MetaCols {
            hash: false,
            date: false,
            author: true,
        },
        MetaCols {
            hash: false,
            date: false,
            author: false,
        },
    ];
    for cols in candidates {
        let meta = meta_columns_text(hash, date, author, cols);
        let room = available_after_gutter.saturating_sub(meta.chars().count());
        if room >= min_left_keep || (!cols.hash && !cols.date && !cols.author) {
            return cols;
        }
    }
    MetaCols {
        hash: false,
        date: false,
        author: false,
    }
}

/// Date / author column widths shared across a commit window.
pub fn meta_column_widths<'a>(
    commits: impl IntoIterator<Item = &'a Commit>,
    now_unix: i64,
) -> (usize, usize) {
    meta_column_widths_with_stashes(commits, std::iter::empty(), now_unix)
}

/// Date / author rails across commits and stashes so spacer columns line up.
pub fn meta_column_widths_with_stashes<'a>(
    commits: impl IntoIterator<Item = &'a Commit>,
    stashes: impl IntoIterator<Item = &'a Stash>,
    now_unix: i64,
) -> (usize, usize) {
    let mut date_width = 4usize;
    let mut author_width = 1usize;
    for commit in commits {
        date_width = date_width.max(format_relative_date(commit.author_date_unix, now_unix).len());
        let name_len = commit
            .author_name
            .chars()
            .count()
            .max(1)
            .min(AUTHOR_COL_MAX);
        author_width = author_width.max(name_len);
    }
    for stash in stashes {
        date_width = date_width.max(format_relative_date(stash.author_date_unix, now_unix).len());
        let name_len = stash.author_name.chars().count().max(1).min(AUTHOR_COL_MAX);
        author_width = author_width.max(name_len);
    }
    (date_width, author_width.min(AUTHOR_COL_MAX))
}

/// Spacer under a commit: `[refs…][pad][hash][ ][date][ ][author]`.
///
/// Drop order when narrow: hash → date → author (keep refs).
pub fn format_commit_spacer(opts: CommitSpacerOpts<'_>) -> String {
    assemble_commit_spacer(opts).0
}

/// Commit spacer string plus coloured [`LabelPart`]s (chips, overflow, meta).
pub fn assemble_commit_spacer(opts: CommitSpacerOpts<'_>) -> (String, Vec<LabelPart>) {
    let (hash, date, author) = padded_meta(
        &opts.commit.id,
        &opts.commit.author_name,
        opts.commit.author_date_unix,
        opts.date_width,
        opts.author_width,
        opts.now_unix,
    );
    // Worktree marks stay visible; leftover *ref* chips collapse to `[+N]`.
    let mut prefix = Vec::new();
    for worktree in opts.worktrees {
        let mark = worktree_mark(worktree, opts.glyphs);
        if !mark.is_empty() {
            prefix.push(vec![LabelPart {
                text: mark,
                kind: LabelKind::Meta,
            }]);
        }
    }
    let refs = commit_ref_chip_groups(
        &opts.commit.refs,
        opts.is_head,
        opts.head_branch,
        opts.glyphs,
        opts.default_branch_override,
    );
    let parts = assemble_spacer_parts(&prefix, &refs, &hash, &date, &author, opts.available);
    (parts_text(&parts), parts)
}

/// Inputs for [`format_stash_spacer`].
pub struct StashSpacerOpts<'a> {
    /// Stash whose spacer is painted.
    pub stash: &'a Stash,
    /// Columns after the gutter gap. `stash@{n}` stays left; meta is right-anchored.
    pub available: usize,
    /// Padded relative-date column width (rails across rows).
    pub date_width: usize,
    /// Padded author column width (rails across rows).
    pub author_width: usize,
    /// Clock for relative dates (unix seconds).
    pub now_unix: i64,
}

/// Spacer under a stash: `[stash@{n}][pad][hash][ ][date][ ][author]`.
///
/// Drop order when narrow: hash → date → author (keep `stash@{n}`).
pub fn format_stash_spacer(opts: StashSpacerOpts<'_>) -> String {
    assemble_stash_spacer(opts).0
}

/// Stash spacer string plus coloured [`LabelPart`]s.
pub fn assemble_stash_spacer(opts: StashSpacerOpts<'_>) -> (String, Vec<LabelPart>) {
    let (hash, date, author) = padded_meta(
        &opts.stash.id,
        &opts.stash.author_name,
        opts.stash.author_date_unix,
        opts.date_width,
        opts.author_width,
        opts.now_unix,
    );
    let parts = assemble_spacer_parts(
        &[vec![LabelPart {
            text: opts.stash.stash_ref.clone(),
            kind: LabelKind::Meta,
        }]],
        &[],
        &hash,
        &date,
        &author,
        opts.available,
    );
    (parts_text(&parts), parts)
}

fn padded_meta(
    id: &str,
    author_name: &str,
    author_date_unix: i64,
    date_width: usize,
    author_width: usize,
    now_unix: i64,
) -> (String, String, String) {
    let hash = short_id(id).to_string();
    let date_raw = format_relative_date(author_date_unix, now_unix);
    let date = pad_right(&trunc(&date_raw, date_width), date_width);
    let author = pad_right(&trunc(author_name, author_width), author_width);
    (hash, date, author)
}

/// Left flex + right-anchored meta. Drop order hash → date → author.
///
/// `prefix` (worktree marks) stays intact. Leftover *ref* chips in
/// `overflowable` collapse to `[+N]` (never mid-chip `…`).
fn assemble_spacer_parts(
    prefix: &[Vec<LabelPart>],
    overflowable: &[Vec<LabelPart>],
    hash: &str,
    date: &str,
    author: &str,
    available: usize,
) -> Vec<LabelPart> {
    if available == 0 {
        return Vec::new();
    }
    let cols = pick_meta_columns(available, hash, date, author, MIN_LEFT_KEEP);
    let meta = meta_columns_text(hash, date, author, cols);
    let ref_budget = available.saturating_sub(meta.chars().count());
    let mut left = join_chip_groups(prefix);
    let prefix_w = parts_width(&left);
    let gap = usize::from(!left.is_empty() && !overflowable.is_empty());
    let chip_budget = ref_budget.saturating_sub(prefix_w.saturating_add(gap));
    let chips = fit_chip_groups(overflowable, chip_budget);
    if !chips.is_empty() {
        if !left.is_empty() {
            left.push(LabelPart {
                text: " ".into(),
                kind: LabelKind::Meta,
            });
        }
        left.extend(chips);
    }
    let left_len = parts_width(&left);
    let meta_len = meta.chars().count();
    if meta.is_empty() {
        pad_parts_right(&mut left, available);
        return clamp_parts_to_width(left, available);
    }
    let pad = available.saturating_sub(left_len).saturating_sub(meta_len);
    if pad > 0 {
        left.push(LabelPart {
            text: " ".repeat(pad),
            kind: LabelKind::Meta,
        });
    }
    if !meta.is_empty() {
        left.push(LabelPart {
            text: meta,
            kind: LabelKind::Meta,
        });
    }
    clamp_parts_to_width(left, available)
}

fn join_chip_groups(groups: &[Vec<LabelPart>]) -> Vec<LabelPart> {
    let mut out = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        if i > 0 {
            out.push(LabelPart {
                text: " ".into(),
                kind: LabelKind::Meta,
            });
        }
        out.extend(g.iter().cloned());
    }
    out
}

/// Cap the spacer at `available` columns. Trailing meta/padding drop first so
/// a leftover `[+N]` chip stays on the row instead of growing past the pane.
fn clamp_parts_to_width(mut parts: Vec<LabelPart>, available: usize) -> Vec<LabelPart> {
    if available == 0 {
        return Vec::new();
    }
    while parts_width(&parts) > available {
        let extra = parts_width(&parts) - available;
        let Some(last) = parts.last_mut() else {
            break;
        };
        let last_w = last.text.chars().count();
        if last_w > extra {
            last.text = last.text.chars().take(last_w - extra).collect();
            if last.text.is_empty() {
                parts.pop();
            }
        } else {
            parts.pop();
        }
    }
    parts
}

pub(crate) fn parts_text(parts: &[LabelPart]) -> String {
    parts.iter().map(|p| p.text.as_str()).collect()
}

fn parts_width(parts: &[LabelPart]) -> usize {
    parts.iter().map(|p| p.text.chars().count()).sum()
}

/// Skip `offset` columns then keep at most `width` columns of `parts`.
pub(crate) fn slice_label_parts(
    parts: &[LabelPart],
    offset: usize,
    width: usize,
) -> Vec<LabelPart> {
    if width == 0 {
        return Vec::new();
    }
    let mut skip = offset;
    let mut remain = width;
    let mut out = Vec::new();
    for part in parts {
        let n = part.text.chars().count();
        if skip >= n {
            skip -= n;
            continue;
        }
        let sliced: String = part.text.chars().skip(skip).take(remain).collect();
        skip = 0;
        let used = sliced.chars().count();
        if !sliced.is_empty() {
            let mut p = part.clone();
            p.text = sliced;
            out.push(p);
        }
        remain = remain.saturating_sub(used);
        if remain == 0 {
            break;
        }
    }
    out
}

/// Truncate styled runs: keep the start, append `…` when over `max`.
pub(crate) fn trunc_label_parts(parts: &[LabelPart], max: usize) -> Vec<LabelPart> {
    if max == 0 {
        return Vec::new();
    }
    let count = parts_width(parts);
    if count <= max {
        return parts.to_vec();
    }
    if max == 1 {
        return vec![LabelPart {
            text: "…".into(),
            kind: LabelKind::Meta,
        }];
    }
    let mut out = slice_label_parts(parts, 0, max - 1);
    out.push(LabelPart {
        text: "…".into(),
        kind: LabelKind::Meta,
    });
    out
}

fn pad_parts_right(parts: &mut Vec<LabelPart>, available: usize) {
    let len = parts_width(parts);
    if len < available {
        parts.push(LabelPart {
            text: " ".repeat(available - len),
            kind: LabelKind::Meta,
        });
    }
}

/// Keep whole chip groups that fit; leftover branch/tag count becomes `[+N]`.
fn fit_chip_groups(groups: &[Vec<LabelPart>], budget: usize) -> Vec<LabelPart> {
    if groups.is_empty() || budget == 0 {
        return Vec::new();
    }
    let n = groups.len();
    for k in (0..=n).rev() {
        let hidden = n - k;
        let mut width = 0usize;
        for (i, g) in groups.iter().take(k).enumerate() {
            if i > 0 {
                width += 1;
            }
            width += parts_width(g);
        }
        let ov = if hidden > 0 {
            overflow_chip_text(hidden).chars().count() + usize::from(k > 0)
        } else {
            0
        };
        if width + ov <= budget {
            let mut out = Vec::new();
            for (i, g) in groups.iter().take(k).enumerate() {
                if i > 0 {
                    out.push(LabelPart {
                        text: " ".into(),
                        kind: LabelKind::Meta,
                    });
                }
                out.extend(g.iter().cloned());
            }
            if hidden > 0 {
                if k > 0 {
                    out.push(LabelPart {
                        text: " ".into(),
                        kind: LabelKind::Meta,
                    });
                }
                out.push(LabelPart {
                    text: overflow_chip_text(hidden),
                    kind: LabelKind::Overflow,
                });
            }
            return out;
        }
    }
    match overflow_token(n, budget) {
        Some(token) => vec![LabelPart {
            text: token,
            kind: LabelKind::Overflow,
        }],
        None => Vec::new(),
    }
}

fn overflow_token(hidden: usize, budget: usize) -> Option<String> {
    if hidden == 0 || budget == 0 {
        return None;
    }
    let chip = overflow_chip_text(hidden);
    if chip.chars().count() <= budget {
        return Some(chip);
    }
    let bare = format!("+{hidden}");
    if bare.chars().count() <= budget {
        return Some(bare);
    }
    None
}

/// Subject-only label for the commit row (refs live on the spacer beneath).
pub fn format_commit_subject(commit: &Commit) -> String {
    commit.subject.clone()
}

/// `[HEAD]` (detached only) + merged ref chips.
pub fn format_commit_ref_chips(
    refs: &[GraphRef],
    is_head: bool,
    head_branch: Option<&str>,
    glyphs: &GlyphSet,
) -> String {
    format_commit_ref_chips_with(refs, is_head, head_branch, glyphs, None)
}

/// Like [`format_commit_ref_chips`] with a default-branch override for colour/sort.
pub fn format_commit_ref_chips_with(
    refs: &[GraphRef],
    is_head: bool,
    head_branch: Option<&str>,
    glyphs: &GlyphSet,
    default_branch_override: Option<&str>,
) -> String {
    parts_text(&commit_ref_chip_parts(
        refs,
        is_head,
        head_branch,
        glyphs,
        default_branch_override,
    ))
}

/// Styled chip runs for a commit (same groups as the spacer; no overflow).
pub(crate) fn commit_ref_chip_parts(
    refs: &[GraphRef],
    is_head: bool,
    head_branch: Option<&str>,
    glyphs: &GlyphSet,
    default_branch_override: Option<&str>,
) -> Vec<LabelPart> {
    join_chip_groups(&commit_ref_chip_groups(
        refs,
        is_head,
        head_branch,
        glyphs,
        default_branch_override,
    ))
}

/// One visual chip as styled runs (`[HEAD]`, `[main]`, `[+N]` later).
fn commit_ref_chip_groups(
    refs: &[GraphRef],
    is_head: bool,
    head_branch: Option<&str>,
    glyphs: &GlyphSet,
    default_branch_override: Option<&str>,
) -> Vec<Vec<LabelPart>> {
    let mut groups = Vec::new();
    if show_detached_head_chip(head_branch, is_head) {
        groups.push(vec![LabelPart {
            text: "[HEAD]".into(),
            kind: LabelKind::ChipHead,
        }]);
    }
    for chip in merge_commit_ref_chips(refs, default_branch_override) {
        let checkout = chip_is_checkout(&chip, head_branch, is_head);
        groups.push(merged_chip_parts(
            &chip,
            checkout,
            glyphs,
            default_branch_override,
        ));
    }
    groups
}

/// First seven characters of a commit id (or the whole id when shorter).
pub fn short_id(id: &str) -> &str {
    let end = id.char_indices().nth(7).map(|(i, _)| i).unwrap_or(id.len());
    &id[..end]
}

fn format_stash(stash: &Stash, glyphs: &GlyphSet) -> String {
    format!("{} {}  {}", glyphs.stash, stash.stash_ref, stash.subject)
}

fn format_worktree(worktree: &Worktree, glyphs: &GlyphSet) -> String {
    let mut line = worktree_mark(worktree, glyphs);
    if worktree.is_current {
        line.push_str("  [HEAD]");
    }
    line
}

fn worktree_mark(worktree: &Worktree, glyphs: &GlyphSet) -> String {
    let mut mark = format!("{} {}", glyphs.worktree, worktree.path);
    if let Some(branch) = &worktree.branch {
        mark.push(' ');
        mark.push_str(branch);
    }
    if worktree.ignored {
        mark.push_str("  [ignored]");
    }
    mark
}

fn trunc(text: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    if max == 1 {
        return "…".into();
    }
    let keep: String = text.chars().take(max - 1).collect();
    format!("{keep}…")
}

fn pad_right(text: &str, width: usize) -> String {
    let count = text.chars().count();
    if count >= width {
        return trunc(text, width);
    }
    format!("{text}{}", " ".repeat(width - count))
}

fn remote_short_name(remote_ref: &str) -> &str {
    remote_ref
        .split_once('/')
        .map(|(_, rest)| rest)
        .unwrap_or(remote_ref)
}

pub(crate) fn is_default_branch(name: &str, override_name: Option<&str>) -> bool {
    if let Some(over) = override_name {
        return name == over;
    }
    matches!(name, "main" | "master" | "develop")
}

fn is_detached_head_branch(branch: &str) -> bool {
    branch.is_empty() || branch == "HEAD (detached)" || branch == "HEAD" || branch == "(detached)"
}

fn show_detached_head_chip(head_branch: Option<&str>, is_head: bool) -> bool {
    if !is_head {
        return false;
    }
    match head_branch {
        None | Some("") => true,
        Some(branch) => is_detached_head_branch(branch),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum MergedRefChip {
    Merged(String),
    Local(String),
    Remote(String),
    Tag(String),
}

impl MergedRefChip {
    fn name(&self) -> &str {
        match self {
            Self::Merged(n) | Self::Local(n) | Self::Remote(n) | Self::Tag(n) => n,
        }
    }

    fn sort_key(&self, default_branch_override: Option<&str>) -> u8 {
        match self {
            Self::Merged(n) | Self::Local(n) => {
                if is_default_branch(n, default_branch_override) {
                    0
                } else {
                    1
                }
            }
            Self::Remote(n) => {
                if is_default_branch(remote_short_name(n), default_branch_override) {
                    0
                } else {
                    2
                }
            }
            Self::Tag(_) => 3,
        }
    }
}

/// Merge local foo + remote */foo into one chip; leave unmatched remotes/tags.
fn merge_commit_ref_chips(
    refs: &[GraphRef],
    default_branch_override: Option<&str>,
) -> Vec<MergedRefChip> {
    let locals: Vec<&GraphRef> = refs.iter().filter(|r| r.kind == RefKind::Local).collect();
    let remotes: Vec<&GraphRef> = refs.iter().filter(|r| r.kind == RefKind::Remote).collect();
    let tags: Vec<&GraphRef> = refs.iter().filter(|r| r.kind == RefKind::Tag).collect();
    let mut used_remote = vec![false; remotes.len()];
    let mut chips = Vec::new();

    for local in locals {
        let hit = remotes.iter().enumerate().position(|(i, remote)| {
            !used_remote[i] && remote_short_name(&remote.name) == local.name
        });
        if let Some(i) = hit {
            used_remote[i] = true;
            chips.push(MergedRefChip::Merged(local.name.clone()));
        } else {
            chips.push(MergedRefChip::Local(local.name.clone()));
        }
    }
    for (i, remote) in remotes.iter().enumerate() {
        if !used_remote[i] {
            chips.push(MergedRefChip::Remote(remote.name.clone()));
        }
    }
    for tag in tags {
        chips.push(MergedRefChip::Tag(tag.name.clone()));
    }

    let mut indexed: Vec<(usize, MergedRefChip)> = chips.into_iter().enumerate().collect();
    indexed.sort_by(|a, b| {
        a.1.sort_key(default_branch_override)
            .cmp(&b.1.sort_key(default_branch_override))
            .then(a.0.cmp(&b.0))
    });
    indexed.into_iter().map(|(_, chip)| chip).collect()
}

fn chip_is_checkout(chip: &MergedRefChip, head_branch: Option<&str>, is_head: bool) -> bool {
    if !is_head {
        return false;
    }
    let Some(branch) = head_branch else {
        return false;
    };
    if branch.is_empty() || is_detached_head_branch(branch) {
        return false;
    }
    match chip {
        MergedRefChip::Local(name) | MergedRefChip::Merged(name) => name == branch,
        MergedRefChip::Remote(_) | MergedRefChip::Tag(_) => false,
    }
}

fn format_merged_chip(chip: &MergedRefChip, is_checkout: bool, glyphs: &GlyphSet) -> String {
    parts_text(&merged_chip_parts(chip, is_checkout, glyphs, None))
}

fn merged_chip_parts(
    chip: &MergedRefChip,
    is_checkout: bool,
    glyphs: &GlyphSet,
    default_branch_override: Option<&str>,
) -> Vec<LabelPart> {
    let kind = match chip {
        MergedRefChip::Tag(_) => LabelKind::ChipTag,
        MergedRefChip::Remote(n) => {
            if is_default_branch(remote_short_name(n), default_branch_override) {
                LabelKind::ChipDefault
            } else {
                LabelKind::ChipRemote
            }
        }
        MergedRefChip::Local(n) | MergedRefChip::Merged(n) => {
            if is_default_branch(n, default_branch_override) {
                LabelKind::ChipDefault
            } else {
                LabelKind::ChipLocal
            }
        }
    };
    match chip {
        MergedRefChip::Tag(_) | MergedRefChip::Remote(_) => vec![LabelPart {
            text: format!("[{}]", chip.name()),
            kind,
        }],
        MergedRefChip::Local(_) | MergedRefChip::Merged(_) => {
            let mut parts = vec![LabelPart {
                text: "[".into(),
                kind,
            }];
            if is_checkout {
                parts.push(LabelPart {
                    text: glyphs.checkout_mark.to_string(),
                    kind: LabelKind::ChipHead,
                });
            }
            if matches!(chip, MergedRefChip::Merged(_)) {
                parts.push(LabelPart {
                    text: glyphs.sync_mark.to_string(),
                    kind: LabelKind::ChipRemote,
                });
            }
            parts.push(LabelPart {
                text: chip.name().to_string(),
                kind,
            });
            parts.push(LabelPart {
                text: "]".into(),
                kind,
            });
            parts
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::{ASCII, UNICODE};
    use crate::model::{Commit, GraphRow, Stash, Worktree};

    #[test]
    fn short_id_keeps_ids_shorter_than_seven() {
        assert_eq!(short_id("abc"), "abc");
        assert_eq!(short_id("abcdefg"), "abcdefg");
        assert_eq!(short_id("abcdefghij"), "abcdefg");
    }

    #[test]
    fn format_relative_date_uses_three_hour_cutoff() {
        let now = 1_700_000_000;
        assert_eq!(format_relative_date(now, now), "just now");
        assert_eq!(format_relative_date(now - 59, now), "just now");
        assert_eq!(format_relative_date(now - 60, now), "1m");
        assert_eq!(format_relative_date(now - 3599, now), "59m");
        assert_eq!(format_relative_date(now - 3600, now), "1h");
        assert_eq!(format_relative_date(now - 3 * 3600, now), "3h");
        assert_eq!(
            format_relative_date(now - 3 * 3600 - 1, now),
            "2023-11-14 19:13"
        );
        assert_eq!(format_relative_date(now - 86400, now), "2023-11-13 22:13");
        assert_eq!(format_relative_date(now + 10, now), "just now");
    }

    #[test]
    fn pick_meta_columns_drops_hash_then_date_then_author() {
        let hash = "abcdefg";
        let date = "12d ";
        let author = "Ada Lovelace     ";
        assert_eq!(
            pick_meta_columns(80, hash, date, author, MIN_LEFT_KEEP),
            MetaCols {
                hash: true,
                date: true,
                author: true
            }
        );
        let with_all = meta_columns_text(
            hash,
            date,
            author,
            MetaCols {
                hash: true,
                date: true,
                author: true,
            },
        )
        .chars()
        .count();
        let tight = MIN_LEFT_KEEP + with_all - 1;
        let cols = pick_meta_columns(tight, hash, date, author, MIN_LEFT_KEEP);
        assert!(!cols.hash, "{cols:?}");
        let hash_dropped = meta_columns_text(
            hash,
            date,
            author,
            MetaCols {
                hash: false,
                date: true,
                author: true,
            },
        )
        .chars()
        .count();
        let tighter = MIN_LEFT_KEEP + hash_dropped - 1;
        let cols = pick_meta_columns(tighter, hash, date, author, MIN_LEFT_KEEP);
        assert!(!cols.hash && !cols.date, "{cols:?}");
        let date_dropped = meta_columns_text(
            hash,
            date,
            author,
            MetaCols {
                hash: false,
                date: false,
                author: true,
            },
        )
        .chars()
        .count();
        let tightest = MIN_LEFT_KEEP + date_dropped - 1;
        let cols = pick_meta_columns(tightest, hash, date, author, MIN_LEFT_KEEP);
        assert!(!cols.hash && !cols.date && !cols.author, "{cols:?}");
    }

    #[test]
    fn format_sync_ascii_uses_caret_and_v() {
        let sync = SyncState {
            branch: "main".into(),
            status: SyncStatus::Diverged,
            ahead: 2,
            behind: 3,
        };
        assert_eq!(format_sync(&sync, &ASCII), "main ^2 v3");
    }

    fn sample_commit_row() -> GraphRow {
        GraphRow::Commit {
            commit: Commit {
                id: "aaa1111bbbb".into(),
                subject: "add graph crate".into(),
                parents: Vec::new(),
                refs: vec!["main".into(), "origin/main".into()],
                author_name: "Ada Lovelace".into(),
                author_date_unix: 1_700_000_000 - 3600,
            },
            is_head: true,
            worktrees: vec![Worktree {
                path: ".worktrees/feature/graph".into(),
                head_id: Some("aaa1111bbbb".into()),
                branch: Some("feature/graph".into()),
                ignored: false,
                is_current: false,
            }],
        }
    }

    #[test]
    fn format_row_commit_is_subject_only() {
        let row = sample_commit_row();
        assert_eq!(format_row(&row, &UNICODE), "⊙ add graph crate");
        assert_eq!(format_label(&row, &UNICODE), "add graph crate");
    }

    #[test]
    fn format_commit_spacer_puts_refs_hash_date_author() {
        let GraphRow::Commit {
            commit,
            is_head,
            worktrees,
        } = sample_commit_row()
        else {
            panic!("commit row");
        };
        let line = format_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head,
            worktrees: &worktrees,
            head_branch: Some("main"),
            glyphs: &UNICODE,
            available: 120,
            date_width: 4,
            author_width: 12,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        });
        assert!(
            line.contains("[main]"),
            "checkout + synced main chip: {line}"
        );
        assert!(
            !line.contains("origin/main"),
            "merged origin/main should fold into main: {line}"
        );
        assert!(line.contains("aaa1111"), "{line}");
        assert!(line.contains("1h"), "{line}");
        assert!(line.contains("Ada Lovelace"), "{line}");
        assert!(line.contains(".worktrees/feature/graph"), "{line}");
        let hash_at = line.find("aaa1111").expect("hash");
        let ref_at = line.find("[main]").expect("ref");
        assert!(ref_at < hash_at, "refs left of meta: {line}");
    }

    #[test]
    fn format_commit_spacer_keeps_unmatched_origin_ref() {
        let commit = Commit {
            id: "abcdefg".into(),
            subject: "topic".into(),
            refs: vec!["origin/feature/x".into()],
            ..Commit::default()
        };
        let line = format_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &[],
            head_branch: Some("main"),
            glyphs: &UNICODE,
            available: 80,
            date_width: 4,
            author_width: 1,
            now_unix: 0,
            default_branch_override: None,
        });
        assert!(line.contains("[origin/feature/x]"), "{line}");
    }

    #[test]
    fn format_commit_spacer_drops_hash_before_refs() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "topic".into(),
            refs: vec!["feature/long-branch-name".into()],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let line = format_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &[],
            head_branch: None,
            glyphs: &ASCII,
            available: 20,
            date_width: 3,
            author_width: 3,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        });
        assert!(
            line.contains("[feature") || line.contains("feature/") || line.contains('+'),
            "keep refs or [+N] when dropping hash: {line}"
        );
        assert!(
            !line.contains("abcdefg"),
            "hash should drop before refs: {line}"
        );
    }

    fn sample_stash() -> Stash {
        Stash {
            id: "s1abcdefhhhh".into(),
            stash_ref: "stash@{0}".into(),
            subject: "WIP on main".into(),
            author_name: "Ada Lovelace".into(),
            author_date_unix: 1_700_000_000 - 86400,
            parent_id: Some("aaa1111".into()),
        }
    }

    #[test]
    fn format_uncommitted_label_is_dirty_or_clean() {
        let dirty = GraphRow::Uncommitted { has_changes: true };
        let clean = GraphRow::Uncommitted { has_changes: false };
        assert_eq!(format_row(&dirty, &UNICODE), "○ uncommitted changes");
        assert_eq!(format_row(&clean, &UNICODE), "○ working tree clean");
        assert_eq!(format_label(&clean, &ASCII), "o working tree clean");
    }

    #[test]
    fn format_label_stash_is_subject_only() {
        let row = GraphRow::Stash(sample_stash());
        assert_eq!(format_label(&row, &UNICODE), "WIP on main");
        assert_eq!(format_row(&row, &UNICODE), "◇ stash@{0}  WIP on main");
    }

    #[test]
    fn format_stash_spacer_puts_ref_hash_date_author() {
        let stash = sample_stash();
        let line = format_stash_spacer(StashSpacerOpts {
            stash: &stash,
            available: 80,
            date_width: 16,
            author_width: 12,
            now_unix: 1_700_000_000,
        });
        assert!(line.contains("stash@{0}"), "{line}");
        assert!(line.contains("s1abcde"), "{line}");
        assert!(line.contains("2023-11-13 22:13"), "{line}");
        assert!(line.contains("Ada Lovelace"), "{line}");
        let ref_at = line.find("stash@{0}").expect("ref");
        let hash_at = line.find("s1abcde").expect("hash");
        assert!(ref_at < hash_at, "stash@{{n}} left of meta: {line}");
    }

    #[test]
    fn format_stash_spacer_drops_hash_before_ref() {
        let stash = Stash {
            id: "abcdefghhhh".into(),
            stash_ref: "stash@{0}".into(),
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Stash::default()
        };
        let line = format_stash_spacer(StashSpacerOpts {
            stash: &stash,
            available: 20,
            date_width: 3,
            author_width: 3,
            now_unix: 1_700_000_000,
        });
        assert!(
            !line.contains("abcdefg"),
            "hash should drop before stash@{{n}}: {line}"
        );
    }

    #[test]
    fn detached_head_keeps_head_chip() {
        let chips = format_commit_ref_chips(
            &[GraphRef::local("main")],
            true,
            Some("HEAD (detached)"),
            &ASCII,
        );
        assert!(chips.starts_with("[HEAD]"), "{chips}");
        assert!(chips.contains("[main]"), "{chips}");
    }

    #[test]
    fn named_head_uses_checkout_mark_not_head_chip() {
        let chips = format_commit_ref_chips(&[GraphRef::local("main")], true, Some("main"), &ASCII);
        assert!(!chips.contains("[HEAD]"), "{chips}");
        assert_eq!(chips, "[+main]");
    }

    #[test]
    fn format_row_ignored_worktree() {
        let row = GraphRow::Worktree(Worktree {
            path: "notes".into(),
            head_id: None,
            branch: None,
            ignored: true,
            is_current: false,
        });
        assert_eq!(format_row(&row, &UNICODE), " notes  [ignored]");
        assert_eq!(format_row(&row, &ASCII), "L notes  [ignored]");
        assert!(!format_row(&row, &UNICODE).contains("🔗"));
    }

    #[test]
    fn format_commit_spacer_hides_overflow_chips_with_count() {
        let commit = Commit {
            id: "abcdefg".into(),
            subject: "topic".into(),
            refs: vec![
                "main".into(),
                "feature/long-name".into(),
                GraphRef::tag("v1"),
            ],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let line = format_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &[],
            head_branch: None,
            glyphs: &ASCII,
            available: 28,
            date_width: 3,
            author_width: 3,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        });
        assert!(line.contains("[main]"), "kept first chip: {line}");
        assert!(line.contains("[+2]"), "exact overflow chip: {line}");
        assert!(
            !line.contains("feature/long-name"),
            "overflow chip must not remain: {line}"
        );
        assert!(
            !line.contains("[v1]"),
            "overflow tag must not remain: {line}"
        );
        assert!(!line.contains('…'), "must not mid-chip ellipsis: {line}");
        let overflow = assemble_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &[],
            head_branch: None,
            glyphs: &ASCII,
            available: 28,
            date_width: 3,
            author_width: 3,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        })
        .1;
        assert!(
            overflow
                .iter()
                .any(|p| p.kind == LabelKind::Overflow && p.text == "[+2]"),
            "overflow run must be LabelKind::Overflow: {overflow:?}"
        );
    }

    #[test]
    fn overflow_chip_counts_hidden_tags() {
        let commit = Commit {
            id: "abcdefg".into(),
            subject: "topic".into(),
            refs: vec![
                GraphRef::tag("v1.0.0"),
                GraphRef::tag("v1.1.0"),
                GraphRef::tag("release-candidate"),
            ],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let line = format_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &[],
            head_branch: None,
            glyphs: &ASCII,
            available: 24,
            date_width: 3,
            author_width: 3,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        });
        assert!(line.contains("[+"), "tag overflow must show [+N]: {line}");
        let hidden = 3usize.saturating_sub(
            usize::from(line.contains("[v1.0.0]"))
                + usize::from(line.contains("[v1.1.0]"))
                + usize::from(line.contains("[release-candidate]")),
        );
        assert!(hidden > 0, "at least one tag must hide: {line}");
        assert!(
            line.contains(&overflow_chip_text(hidden)),
            "overflow count includes tags: {line}"
        );
        assert!(!line.contains('…'), "must not mid-chip ellipsis: {line}");
    }

    #[test]
    fn overflow_chip_counts_branches_and_tags_together() {
        let commit = Commit {
            id: "abcdefg".into(),
            subject: "topic".into(),
            refs: vec![
                GraphRef::local("main"),
                GraphRef::local("feature/x"),
                GraphRef::tag("v2"),
                GraphRef::tag("nightly"),
            ],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 60,
            ..Commit::default()
        };
        let (line, parts) = assemble_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &[],
            head_branch: None,
            glyphs: &ASCII,
            available: 22,
            date_width: 3,
            author_width: 3,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        });
        let overflow = parts
            .iter()
            .find(|p| p.kind == LabelKind::Overflow)
            .expect("overflow chip");
        assert!(
            overflow.text.starts_with("[+") && overflow.text.ends_with(']'),
            "overflow must be a chip: {line}"
        );
        assert!(
            !line.contains("[feature/x]") || !line.contains("[v2]") || !line.contains("[nightly]"),
            "some branch/tag chips must hide: {line}"
        );
        assert!(line.contains("[main]"), "kept default branch: {line}");
    }

    #[test]
    fn worktree_marks_stay_visible_when_refs_overflow() {
        let commit = Commit {
            id: "abcdefg".into(),
            subject: "topic".into(),
            refs: vec![
                GraphRef::local("main"),
                GraphRef::local("feature/x"),
                GraphRef::tag("v2"),
            ],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 60,
            ..Commit::default()
        };
        let worktrees = [Worktree {
            path: "notes".into(),
            head_id: Some(commit.id.clone()),
            branch: None,
            ignored: false,
            is_current: false,
        }];
        let (line, parts) = assemble_commit_spacer(CommitSpacerOpts {
            commit: &commit,
            is_head: false,
            worktrees: &worktrees,
            head_branch: None,
            glyphs: &ASCII,
            available: 28,
            date_width: 3,
            author_width: 3,
            now_unix: 1_700_000_000,
            default_branch_override: None,
        });
        assert!(
            line.contains("notes"),
            "worktree mark stays visible: {line}"
        );
        assert!(
            parts.iter().any(|p| p.kind == LabelKind::Overflow),
            "hidden branch/tag refs still show [+N]: {line}"
        );
        assert!(
            !line.contains("[feature/x]") || !line.contains("[v2]"),
            "some refs hide: {line}"
        );
    }

    #[test]
    fn commit_spacer_never_exceeds_available_with_many_refs() {
        let refs: Vec<GraphRef> = (0..20)
            .map(|i| {
                if i % 2 == 0 {
                    GraphRef::local(format!("feature/branch-{i:02}"))
                } else {
                    GraphRef::tag(format!("v{i}.0.0-long"))
                }
            })
            .collect();
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "topic".into(),
            refs,
            author_name: "Ada Lovelace".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        for available in [8, 16, 24, 40, 64] {
            let (line, parts) = assemble_commit_spacer(CommitSpacerOpts {
                commit: &commit,
                is_head: false,
                worktrees: &[],
                head_branch: None,
                glyphs: &ASCII,
                available,
                date_width: 10,
                author_width: 12,
                now_unix: 1_700_000_000,
                default_branch_override: None,
            });
            assert!(
                line.chars().count() <= available,
                "spacer grew past pane: available={available} len={} line={line:?}",
                line.chars().count()
            );
            assert!(
                parts.iter().any(|p| p.kind == LabelKind::Overflow),
                "many refs must overflow: available={available} line={line}"
            );
            assert!(!line.contains('…'), "must not mid-chip ellipsis: {line}");
        }
    }
}

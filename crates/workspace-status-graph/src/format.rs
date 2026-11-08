//! Headless line format for graph rows. The widget paints these strings.

use crate::glyphs::GlyphSet;
use crate::model::{Commit, GraphRef, GraphRow, RefKind, Stash, SyncState, SyncStatus, Worktree};

/// Prefer keeping at least this many left columns before dropping right meta.
const MIN_LEFT_KEEP: usize = 12;

/// Cap for the author column.
const AUTHOR_COL_MAX: usize = 16;

/// Format the sync header line (`branch` plus ahead/behind marks).
pub fn format_sync(sync: &SyncState, glyphs: &GlyphSet) -> String {
    parts_text(&format_sync_parts(sync, glyphs, None))
}

/// [`format_sync`] as coloured runs: the branch as a branch chip kind
/// (default or local), then [`LabelKind::SyncAhead`] /
/// [`LabelKind::SyncBehind`], or `no-upstream` / `upstream-gone` as meta.
pub(crate) fn format_sync_parts(
    sync: &SyncState,
    glyphs: &GlyphSet,
    default_branch_override: Option<&str>,
) -> Vec<LabelPart> {
    let branch = if is_default_branch(&sync.branch, default_branch_override) {
        LabelKind::ChipDefault
    } else {
        LabelKind::ChipLocal
    };
    let mut parts = vec![plain_part(sync.branch.clone(), branch)];
    let mut push = |text: String, kind: LabelKind| {
        parts.push(plain_part(" ".into(), LabelKind::Meta));
        parts.push(plain_part(text, kind));
    };
    match sync.status {
        SyncStatus::NoUpstream => push("no-upstream".into(), LabelKind::Meta),
        SyncStatus::Gone => push("upstream-gone".into(), LabelKind::Meta),
        SyncStatus::UpToDate => {}
        SyncStatus::Ahead | SyncStatus::Behind | SyncStatus::Diverged => {
            if sync.ahead > 0 {
                push(
                    format!("{}{}", glyphs.ahead, sync.ahead),
                    LabelKind::SyncAhead,
                );
            }
            if sync.behind > 0 {
                push(
                    format!("{}{}", glyphs.behind, sync.behind),
                    LabelKind::SyncBehind,
                );
            }
        }
    }
    parts
}

/// A run with no chip target.
fn plain_part(text: String, kind: LabelKind) -> LabelPart {
    LabelPart {
        text,
        kind,
        target: None,
    }
}

/// Relative ages only up to 3 hours (iOS notification style). Older
/// timestamps paint as local `YYYY-MM-DD HH:MM`.
pub const RELATIVE_DATE_LIMIT_SECS: i64 = 3 * 3600;

/// One styled run of graph label / spacer text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LabelPart {
    /// Visible text.
    pub text: String,
    /// How the widget should colour this run.
    pub kind: LabelKind,
    /// The ref chip, mark run, or overflow chip this run belongs to. Every
    /// run of one chip carries the same target, and fit, truncation, and
    /// slicing keep it. `None` for subjects, meta, and padding.
    pub target: Option<PartTarget>,
}

impl LabelPart {
    /// The icon this run paints and its part identity
    /// ([`crate::IconSpan::part`]): a row glyph ([`LabelKind::icon`]) or a
    /// chip ([`PartTarget::icon`]). `None` for text runs.
    pub fn icon(&self) -> Option<(crate::GraphIconKind, Option<usize>)> {
        self.kind
            .icon()
            .or_else(|| Some((self.target.as_ref()?.icon(self.kind), None)))
    }
}

/// What a chip run stands for: the identity a popover reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PartTarget {
    /// A ref chip. One ref, or a local branch and then its same-name
    /// remote-tracking ref (a merged chip).
    Chip(Vec<GraphRef>),
    /// The checkout and sync marks inside the chip of local branch
    /// `branch`. One run: `checkout` when HEAD is on it, `remote` (the
    /// same-name remote-tracking ref) when the marks include the sync mark.
    Marks {
        /// Local branch of the chip.
        branch: String,
        /// HEAD is on `branch`.
        checkout: bool,
        /// Remote-tracking ref on the same commit, when synced.
        remote: Option<String>,
    },
    /// Detached `[HEAD]` chip.
    DetachedHead,
    /// `[+N]`: the target of each chip it hides, in chip order.
    Overflow(Vec<PartTarget>),
}

impl PartTarget {
    /// Icon kind of a chip run with this target painted as `kind`.
    pub fn icon(&self, kind: LabelKind) -> crate::GraphIconKind {
        use crate::GraphIconKind as G;
        match self {
            Self::Marks { .. } => G::ChipMarks,
            Self::DetachedHead => G::DetachedHeadChip,
            Self::Overflow(_) => G::OverflowChip,
            Self::Chip(_) => match kind {
                LabelKind::ChipDefault => G::DefaultChip,
                LabelKind::ChipRemote => G::RemoteChip,
                LabelKind::ChipTag => G::TagChip,
                _ => G::LocalChip,
            },
        }
    }
}

/// Semantic colour role for a [`LabelPart`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LabelKind {
    /// Commit / stash subject (stronger than meta).
    Subject,
    /// Worktree marks, separators, padding.
    Meta,
    /// Short commit id: the hash column of a commit / stash row or the
    /// commit / stash hash of the selection footer (`parent: None`), or the
    /// short id of parent `n` in the footer's parents group
    /// (`parent: Some(n)`, git parent order). Meta colour, not italic.
    MetaHash {
        /// Index into the commit's `parents`, or `None` for the row's own id.
        parent: Option<usize>,
    },
    /// Relative date column of a commit / stash row, or the date run of
    /// the selection footer. Meta colour, painted italic.
    MetaDate,
    /// Author column of a commit row, or the author run of the selection
    /// footer. Meta colour, painted italic.
    MetaAuthor,
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
    /// Fully hidden leftover branch/tag count (`[+N]`). Truncated chips
    /// are visible and do not count toward `N`.
    Overflow,
    /// Uncommitted-row glyph (`○` / `o`). `dirty` is true when the
    /// worktree or index has changes.
    UncommittedMark {
        /// The row reads `uncommitted changes`, not `working tree clean`.
        dirty: bool,
    },
    /// Linked-worktree glyph (U+F481 / `L`) of a worktree row or of a
    /// worktree mark on a commit spacer.
    WorktreeMark {
        /// Index into the commit row's `worktrees`. `0` on a worktree row.
        worktree: usize,
    },
    /// Sync header ahead count (ahead glyph and number).
    SyncAhead,
    /// Sync header behind count (behind glyph and number).
    SyncBehind,
}

impl LabelKind {
    /// The icon a part of this kind paints, and its part identity
    /// ([`crate::IconSpan::part`]). The commit meta runs (hash, date,
    /// author) are text icons: hovering them opens a commit popover. `None`
    /// for the other text runs.
    pub fn icon(self) -> Option<(crate::GraphIconKind, Option<usize>)> {
        match self {
            Self::UncommittedMark { .. } => Some((crate::GraphIconKind::Uncommitted, None)),
            Self::WorktreeMark { worktree } => {
                Some((crate::GraphIconKind::Worktree, Some(worktree)))
            }
            Self::MetaHash { parent } => Some((crate::GraphIconKind::CommitHash, parent)),
            Self::MetaDate => Some((crate::GraphIconKind::CommitDate, None)),
            Self::MetaAuthor => Some((crate::GraphIconKind::CommitAuthor, None)),
            _ => None,
        }
    }
}

/// Ellipsis inside a truncated chip name. Brackets stay (`[feat…]`).
const CHIP_NAME_ELLIPSIS: &str = "…";

/// Chip text for fully hidden leftover branch/tag refs (`[+N]`).
///
/// Always a whole chip (brackets). Callers reserve this width so the
/// overflow count is never mid-clipped the way a raw `+N` blends into
/// hash/date meta. A chip whose name is merely truncated with `…` is
/// not hidden and must not be included in `N`.
pub fn overflow_chip_text(hidden: usize) -> String {
    format!("[+{hidden}]")
}

/// Compact age or absolute timestamp.
///
/// Older than [`RELATIVE_DATE_LIMIT_SECS`] paints the operator-local
/// civil clock (`YYYY-MM-DD HH:MM`). Search still indexes the UTC form
/// via [`format_utc_timestamp`].
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
    format_local_timestamp(unix)
}

/// Operator-local `YYYY-MM-DD HH:MM`. Falls back to UTC when libc
/// localtime is unavailable.
pub fn format_local_timestamp(unix: i64) -> String {
    match local_civil(unix) {
        Some((year, month, day, hour, min, _)) => {
            format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}")
        }
        None => format_utc_timestamp(unix),
    }
}

/// Operator-local `YYYY-MM-DD HH:MM:SS`. Falls back to UTC when libc
/// localtime is unavailable.
pub fn format_local_exact(unix: i64) -> String {
    match local_civil(unix) {
        Some((year, month, day, hour, min, sec)) => {
            format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}")
        }
        None => format!("{}:{:02}", format_utc_timestamp(unix), unix.rem_euclid(60)),
    }
}

/// A UTC offset as `UTC+02:00`, `UTC-05:30`, or `UTC+00:00`.
///
/// `offset_secs` is seconds east of UTC, as git records a commit date.
pub fn format_utc_offset(offset_secs: i32) -> String {
    let sign = if offset_secs < 0 { '-' } else { '+' };
    let abs = offset_secs.unsigned_abs();
    format!("UTC{sign}{:02}:{:02}", abs / 3600, (abs % 3600) / 60)
}

/// UTC `YYYY-MM-DD HH:MM` without extra crates. Stable search form.
pub fn format_utc_timestamp(unix: i64) -> String {
    format_timestamp_at_offset(unix, 0)
}

/// Civil `YYYY-MM-DD HH:MM` at a fixed UTC offset (seconds east of UTC).
fn format_timestamp_at_offset(unix: i64, offset_secs: i32) -> String {
    let unix = unix.saturating_add(i64::from(offset_secs)).max(0);
    let days = unix.div_euclid(86_400);
    let rem = unix.rem_euclid(86_400) as u32;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{min:02}")
}

/// Year, month, day, hour, minute, second in the operator timezone.
fn local_civil(unix: i64) -> Option<(i32, u32, u32, u32, u32, u32)> {
    #[cfg(unix)]
    {
        unix_local_civil(unix)
    }
    #[cfg(windows)]
    {
        windows_local_civil(unix)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = unix;
        None
    }
}

#[cfg(unix)]
fn unix_local_civil(unix: i64) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let t = unix as libc::time_t;
    if t as i64 != unix {
        return None;
    }
    // SAFETY: `libc::tm` is plain integers (and on some targets a nullable
    // pointer), so the all-zero value is valid. `localtime_r` writes every field
    // on success, or returns null without requiring the contents.
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    // SAFETY: `t` and `tm` are live locals, so both pointers are valid for the
    // call. `localtime_r` keeps no pointer after it returns.
    let ptr = unsafe { libc::localtime_r(&t, &mut tm) };
    if ptr.is_null() {
        return None;
    }
    Some(civil_from_tm(&tm))
}

#[cfg(windows)]
fn windows_local_civil(unix: i64) -> Option<(i32, u32, u32, u32, u32, u32)> {
    let t = unix as libc::time_t;
    if t as i64 != unix {
        return None;
    }
    // SAFETY: `libc::tm` is plain integers, so the all-zero value is valid.
    // `localtime_s` writes every field on success (return 0).
    let mut tm = unsafe { std::mem::zeroed::<libc::tm>() };
    // SAFETY: `t` and `tm` are live locals, so both pointers are valid for the
    // call. `localtime_s` keeps no pointer after it returns.
    if unsafe { libc::localtime_s(&mut tm, &t) } != 0 {
        return None;
    }
    Some(civil_from_tm(&tm))
}

#[cfg(any(unix, windows))]
fn civil_from_tm(tm: &libc::tm) -> (i32, u32, u32, u32, u32, u32) {
    (
        tm.tm_year + 1900,
        (tm.tm_mon + 1) as u32,
        tm.tm_mday as u32,
        tm.tm_hour as u32,
        tm.tm_min as u32,
        tm.tm_sec as u32,
    )
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

/// [`format_label`] as coloured runs.
///
/// The uncommitted glyph and the worktree glyph are their own parts
/// ([`LabelKind::UncommittedMark`], [`LabelKind::WorktreeMark`]). A
/// worktree row paints `[hidden]` as meta and `[HEAD]` as the head mark.
/// Every other label is one subject run. The text is [`format_label`].
pub(crate) fn format_label_parts(row: &GraphRow, glyphs: &GlyphSet) -> Vec<LabelPart> {
    match row {
        GraphRow::Uncommitted { has_changes } => {
            let text = if *has_changes {
                " uncommitted changes"
            } else {
                " working tree clean"
            };
            vec![
                LabelPart {
                    text: glyphs.uncommitted.to_string(),
                    kind: LabelKind::UncommittedMark {
                        dirty: *has_changes,
                    },
                    target: None,
                },
                LabelPart {
                    text: text.to_string(),
                    kind: LabelKind::Subject,
                    target: None,
                },
            ]
        }
        GraphRow::Worktree(worktree) => {
            let mut parts = worktree_mark_parts(worktree, glyphs, 0, LabelKind::Subject);
            if worktree.is_current {
                parts.push(LabelPart {
                    text: "  [HEAD]".into(),
                    kind: LabelKind::ChipHead,
                    target: None,
                });
            }
            parts
        }
        GraphRow::Stash(_) | GraphRow::Commit { .. } => {
            let text = format_label(row, glyphs);
            if text.is_empty() {
                Vec::new()
            } else {
                vec![LabelPart {
                    text,
                    kind: LabelKind::Subject,
                    target: None,
                }]
            }
        }
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
    parts_text(&meta_columns_parts(hash, date, author, cols))
}

/// Styled runs of [`meta_columns_text`]. Each column is a `" "`
/// [`LabelKind::Meta`] run, then its value with the right pad trimmed
/// (hash [`LabelKind::MetaHash`], date [`LabelKind::MetaDate`], author
/// [`LabelKind::MetaAuthor`]), then the pad as a [`LabelKind::Meta`] run
/// when there is one. The value run alone is the column's icon span.
/// Columns that `cols` drops (and an empty author) emit no run.
pub fn meta_columns_parts(hash: &str, date: &str, author: &str, cols: MetaCols) -> Vec<LabelPart> {
    let mut parts = Vec::new();
    let mut push = |text: &str, kind| {
        let meta = |text: &str| LabelPart {
            text: text.to_string(),
            kind: LabelKind::Meta,
            target: None,
        };
        parts.push(meta(" "));
        let value = text.trim_end_matches(' ');
        if !value.is_empty() {
            parts.push(LabelPart {
                text: value.to_string(),
                kind,
                target: None,
            });
        }
        let pad = &text[value.len()..];
        if !pad.is_empty() {
            parts.push(meta(pad));
        }
    };
    if cols.hash {
        push(hash, LabelKind::MetaHash { parent: None });
    }
    if cols.date {
        push(date, LabelKind::MetaDate);
    }
    if cols.author && !author.is_empty() {
        push(author, LabelKind::MetaAuthor);
    }
    parts
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
        let name_len = commit.author_name.chars().count().clamp(1, AUTHOR_COL_MAX);
        author_width = author_width.max(name_len);
    }
    for stash in stashes {
        date_width = date_width.max(format_relative_date(stash.author_date_unix, now_unix).len());
        let name_len = stash.author_name.chars().count().clamp(1, AUTHOR_COL_MAX);
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
///
/// Linked extras whose branch is already a ref chip (or the current checkout)
/// are not repeated as a path prefix — the spacer uses the same chip string as
/// the selection footer. Other worktree marks stay visible. Leftover *ref*
/// chips: paint a truncated name (`[feat…]`) when part of it fits; `[+N]`
/// counts only chips that are fully hidden after that.
pub fn assemble_commit_spacer(opts: CommitSpacerOpts<'_>) -> (String, Vec<LabelPart>) {
    let (hash, date, author) = padded_meta(
        &opts.commit.id,
        &opts.commit.author_name,
        opts.commit.author_date_unix,
        opts.date_width,
        opts.author_width,
        opts.now_unix,
    );
    let mut prefix = Vec::new();
    for (index, worktree) in opts.worktrees.iter().enumerate() {
        if worktree_covered_by_ref_chip(worktree, opts.commit, opts.is_head, opts.head_branch) {
            continue;
        }
        prefix.push(worktree_mark_parts(
            worktree,
            opts.glyphs,
            index,
            LabelKind::Meta,
        ));
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
            target: None,
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
/// `prefix` (worktree marks that are not already a ref chip) stays intact.
/// Leftover *ref* chips in `overflowable`: truncate the next name with `…`
/// when part of it fits; `[+N]` only for chips fully hidden after that.
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
    let meta = meta_columns_parts(hash, date, author, cols);
    let meta_len = parts_width(&meta);
    let ref_budget = available.saturating_sub(meta_len);
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
                target: None,
            });
        }
        left.extend(chips);
    }
    let left_len = parts_width(&left);
    if meta.is_empty() {
        pad_parts_right(&mut left, available);
        return clamp_parts_to_width(left, available);
    }
    let pad = available.saturating_sub(left_len).saturating_sub(meta_len);
    if pad > 0 {
        left.push(LabelPart {
            text: " ".repeat(pad),
            kind: LabelKind::Meta,
            target: None,
        });
    }
    left.extend(meta);
    clamp_parts_to_width(left, available)
}

fn join_chip_groups(groups: &[Vec<LabelPart>]) -> Vec<LabelPart> {
    let mut out = Vec::new();
    for (i, g) in groups.iter().enumerate() {
        if i > 0 {
            out.push(LabelPart {
                text: " ".into(),
                kind: LabelKind::Meta,
                target: None,
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
            target: None,
        }];
    }
    let mut out = slice_label_parts(parts, 0, max - 1);
    out.push(LabelPart {
        text: "…".into(),
        kind: LabelKind::Meta,
        target: None,
    });
    out
}

/// Wrap styled runs onto rows of at most `width` columns (chars, like
/// [`parts_width`]).
///
/// A line that fits is one row, unchanged. Otherwise the line breaks at
/// its ` · ` group separators ([`is_group_separator`]), and a separator at
/// a row break is dropped. A group wider than `width` starts its own row
/// and breaks after its untargeted meta runs that end with a blank
/// (`parent `, the blank between parent ids or chips); the blank at such a
/// break is trimmed. A piece still wider than `width` is cut by columns,
/// and every cut keeps its run's kind and target. `width == 0` or no runs
/// give one empty row.
pub(crate) fn wrap_label_parts(parts: &[LabelPart], width: usize) -> Vec<Vec<LabelPart>> {
    wrap_rows(parts, width)
        .into_iter()
        .map(|row| row.parts)
        .collect()
}

/// [`wrap_label_parts`] cut to at most `max_rows` rows.
///
/// When rows are cut, the last kept row is [`trunc_label_parts`] of the
/// rest of the line from that row's first column, so `max_rows == 1` gives
/// exactly `vec![trunc_label_parts(parts, width)]`. `max_rows == 0` gives
/// no rows.
pub(crate) fn wrap_label_parts_capped(
    parts: &[LabelPart],
    width: usize,
    max_rows: usize,
) -> Vec<Vec<LabelPart>> {
    if max_rows == 0 {
        return Vec::new();
    }
    let rows = wrap_rows(parts, width);
    if rows.len() <= max_rows {
        return rows.into_iter().map(|row| row.parts).collect();
    }
    let last_start = rows[max_rows - 1].start;
    let mut out: Vec<Vec<LabelPart>> = rows
        .into_iter()
        .take(max_rows - 1)
        .map(|row| row.parts)
        .collect();
    out.push(trunc_label_parts(
        &slice_label_parts(parts, last_start, usize::MAX),
        width,
    ));
    out
}

/// One row of [`wrap_rows`].
struct WrapRow {
    /// Column of the row's first cell in the unwrapped line.
    start: usize,
    parts: Vec<LabelPart>,
}

impl WrapRow {
    fn width(&self) -> usize {
        parts_width(&self.parts)
    }

    /// Trim the blank a break point leaves at the end of the row.
    fn trim_break_blank(&mut self) {
        while let Some(last) = self.parts.last_mut() {
            if !is_break_blank(last) {
                return;
            }
            let trimmed = last.text.trim_end_matches(' ').len();
            last.text.truncate(trimmed);
            if !last.text.is_empty() {
                return;
            }
            self.parts.pop();
        }
    }
}

/// A run of the unwrapped line with the column it starts at.
struct WrapPiece {
    start: usize,
    parts: Vec<LabelPart>,
}

impl WrapPiece {
    /// Width without the trailing break blank, the width it takes at the
    /// end of a row.
    fn end_width(&self) -> usize {
        let mut width = parts_width(&self.parts);
        for part in self.parts.iter().rev() {
            if !is_break_blank(part) {
                break;
            }
            let text_w = part.text.chars().count();
            let kept = part.text.trim_end_matches(' ').chars().count();
            width -= text_w - kept;
            if kept > 0 {
                break;
            }
        }
        width
    }
}

/// Text of the run between two selection-footer meta groups.
const META_GROUP_SEPARATOR: &str = " · ";

/// The run between two selection-footer meta groups: the one the footer
/// meta line joins its groups with, and the one [`wrap_label_parts`] breaks
/// at and puts back between groups that share a row.
pub(crate) fn meta_group_separator() -> LabelPart {
    LabelPart {
        text: META_GROUP_SEPARATOR.into(),
        kind: LabelKind::Meta,
        target: None,
    }
}

/// A [`meta_group_separator`] run.
fn is_group_separator(part: &LabelPart) -> bool {
    *part == meta_group_separator()
}

/// An untargeted meta run that ends with a blank: a row may break after it.
fn is_break_blank(part: &LabelPart) -> bool {
    part.kind == LabelKind::Meta && part.target.is_none() && part.text.ends_with(' ')
}

/// Split `parts` at the separators into groups, each with its start column.
fn wrap_groups(parts: &[LabelPart]) -> Vec<WrapPiece> {
    let mut groups = Vec::new();
    let mut current = WrapPiece {
        start: 0,
        parts: Vec::new(),
    };
    let mut col = 0;
    for part in parts {
        let w = part.text.chars().count();
        if is_group_separator(part) {
            groups.push(std::mem::replace(
                &mut current,
                WrapPiece {
                    start: col + w,
                    parts: Vec::new(),
                },
            ));
        } else {
            current.parts.push(part.clone());
        }
        col += w;
    }
    groups.push(current);
    groups.into_iter().filter(|g| !g.parts.is_empty()).collect()
}

/// Split a group after each of its inner break blanks ([`is_break_blank`]).
fn wrap_group_pieces(group: WrapPiece) -> Vec<WrapPiece> {
    let mut pieces = Vec::new();
    let mut current = WrapPiece {
        start: group.start,
        parts: Vec::new(),
    };
    let mut col = group.start;
    let last = group.parts.len().saturating_sub(1);
    for (i, part) in group.parts.into_iter().enumerate() {
        col += part.text.chars().count();
        let breaks = i < last && is_break_blank(&part);
        current.parts.push(part);
        if breaks {
            pieces.push(std::mem::replace(
                &mut current,
                WrapPiece {
                    start: col,
                    parts: Vec::new(),
                },
            ));
        }
    }
    pieces.push(current);
    pieces
}

/// Greedy rows for [`wrap_label_parts`], each with its start column.
fn wrap_rows(parts: &[LabelPart], width: usize) -> Vec<WrapRow> {
    if width == 0 || parts.is_empty() || parts_width(parts) <= width {
        let parts = if width == 0 {
            Vec::new()
        } else {
            parts.to_vec()
        };
        return vec![WrapRow { start: 0, parts }];
    }
    let mut rows: Vec<WrapRow> = Vec::new();
    let mut open: Option<WrapRow> = None;
    let close = |rows: &mut Vec<WrapRow>, open: &mut Option<WrapRow>| {
        if let Some(mut row) = open.take() {
            row.trim_break_blank();
            if !row.parts.is_empty() {
                rows.push(row);
            }
        }
    };
    let separator_w = META_GROUP_SEPARATOR.chars().count();
    for group in wrap_groups(parts) {
        let group_w = parts_width(&group.parts);
        if let Some(row) = open.as_mut() {
            if row.width() + separator_w + group_w <= width {
                row.parts.push(meta_group_separator());
                row.parts.extend(group.parts);
                continue;
            }
        }
        close(&mut rows, &mut open);
        if group_w <= width {
            open = Some(WrapRow {
                start: group.start,
                parts: group.parts,
            });
            continue;
        }
        for piece in wrap_group_pieces(group) {
            if let Some(row) = open.as_mut() {
                if row.width() + piece.end_width() <= width {
                    row.parts.extend(piece.parts);
                    continue;
                }
            }
            close(&mut rows, &mut open);
            if piece.end_width() <= width {
                open = Some(WrapRow {
                    start: piece.start,
                    parts: piece.parts,
                });
                continue;
            }
            // Cut by columns, the trailing break blank left out so no slice
            // is blank only.
            let piece_w = piece.end_width();
            let mut offset = 0;
            while offset < piece_w {
                close(&mut rows, &mut open);
                open = Some(WrapRow {
                    start: piece.start + offset,
                    parts: slice_label_parts(&piece.parts, offset, width.min(piece_w - offset)),
                });
                offset += width;
            }
            // The last slice stays open: put the trailing break blank back so
            // a next piece on the same row keeps its blank. A row break
            // trims it ([`WrapRow::trim_break_blank`]).
            if let Some(row) = open.as_mut() {
                row.parts
                    .extend(slice_label_parts(&piece.parts, piece_w, usize::MAX));
            }
        }
    }
    close(&mut rows, &mut open);
    rows
}

fn pad_parts_right(parts: &mut Vec<LabelPart>, available: usize) {
    let len = parts_width(parts);
    if len < available {
        parts.push(LabelPart {
            text: " ".repeat(available - len),
            kind: LabelKind::Meta,
            target: None,
        });
    }
}

/// Fit as many full chips as possible. If leftover room can hold part of the
/// next name, paint that chip truncated with `…` (brackets stay). `[+N]`
/// counts only chips fully hidden after the visible (full or truncated)
/// chips — omit it when the last painted chip is merely truncated.
fn fit_chip_groups(groups: &[Vec<LabelPart>], budget: usize) -> Vec<LabelPart> {
    if groups.is_empty() || budget == 0 {
        return Vec::new();
    }
    let n = groups.len();
    for k in (0..=n).rev() {
        let full = join_chip_groups(&groups[..k]);
        let full_w = parts_width(&full);
        if k == n {
            if full_w <= budget {
                return full;
            }
            continue;
        }

        let gap = usize::from(k > 0);
        let hidden_after_trunc = n - k - 1;
        let ov_after = overflow_reserve(hidden_after_trunc, true);
        let trunc_budget = budget
            .saturating_sub(full_w)
            .saturating_sub(gap)
            .saturating_sub(ov_after);
        if let Some(truncated) = truncate_chip_group(&groups[k], trunc_budget) {
            let mut out = full;
            if k > 0 {
                out.push(space_part());
            }
            out.extend(truncated);
            if hidden_after_trunc > 0 {
                out.push(space_part());
                out.push(overflow_part(&groups[k + 1..]));
            }
            return out;
        }

        let hidden = n - k;
        let ov = overflow_reserve(hidden, k > 0);
        if full_w + ov <= budget {
            let mut out = full;
            if hidden > 0 {
                if k > 0 {
                    out.push(space_part());
                }
                out.push(overflow_part(&groups[k..]));
            }
            return out;
        }
    }
    match overflow_token(n, budget) {
        Some(token) => vec![LabelPart {
            text: token,
            kind: LabelKind::Overflow,
            target: Some(overflow_target(groups)),
        }],
        None => Vec::new(),
    }
}

fn space_part() -> LabelPart {
    LabelPart {
        text: " ".into(),
        kind: LabelKind::Meta,
        target: None,
    }
}

/// `[+N]` for the fully hidden chip groups `hidden`.
fn overflow_part(hidden: &[Vec<LabelPart>]) -> LabelPart {
    LabelPart {
        text: overflow_chip_text(hidden.len()),
        kind: LabelKind::Overflow,
        target: Some(overflow_target(hidden)),
    }
}

/// [`PartTarget::Overflow`] of the hidden chip groups `hidden`: the target
/// of each group's first run.
fn overflow_target(hidden: &[Vec<LabelPart>]) -> PartTarget {
    PartTarget::Overflow(
        hidden
            .iter()
            .filter_map(|group| group.iter().find_map(|part| part.target.clone()))
            .collect(),
    )
}

fn overflow_reserve(hidden: usize, preceded: bool) -> usize {
    if hidden == 0 {
        return 0;
    }
    overflow_chip_text(hidden).chars().count() + usize::from(preceded)
}

/// Paint `group` into `budget` columns: keep `[` `]`, truncate the name with
/// `…` when some of it still fits. Returns `None` when not even one name
/// character plus ellipsis and brackets fit (drop the whole chip).
fn truncate_chip_group(group: &[LabelPart], budget: usize) -> Option<Vec<LabelPart>> {
    if group.is_empty() || budget == 0 {
        return None;
    }
    let full_w = parts_width(group);
    if full_w <= budget {
        return Some(group.to_vec());
    }
    let ellipsis_w = CHIP_NAME_ELLIPSIS.chars().count();
    let min_w = 2 + 1 + ellipsis_w;
    if budget < min_w {
        return None;
    }

    if group.len() >= 3 && group[0].text == "[" && group[group.len() - 1].text == "]" {
        let interior = &group[1..group.len() - 1];
        let (name, marks) = interior.split_last()?;
        if name.text.is_empty() {
            return None;
        }
        if let Some(out) = truncate_named_chip(
            &group[0],
            marks,
            name,
            &group[group.len() - 1],
            budget,
            true,
        ) {
            return Some(out);
        }
        return truncate_named_chip(
            &group[0],
            marks,
            name,
            &group[group.len() - 1],
            budget,
            false,
        );
    }

    if group.len() != 1 {
        return None;
    }
    let part = &group[0];
    if !part.text.starts_with('[') || !part.text.ends_with(']') {
        return None;
    }
    let mut body: Vec<char> = part.text.chars().skip(1).collect();
    if body.pop() != Some(']') || body.is_empty() {
        return None;
    }
    let name: String = body.into_iter().collect();
    let name_budget = budget.saturating_sub(2);
    if name_budget < 1 + ellipsis_w {
        return None;
    }
    let keep = name_budget - ellipsis_w;
    let prefix: String = name.chars().take(keep).collect();
    if prefix.is_empty() || prefix.chars().count() >= name.chars().count() {
        return None;
    }
    Some(vec![LabelPart {
        text: format!("[{prefix}{CHIP_NAME_ELLIPSIS}]"),
        kind: part.kind,
        target: part.target.clone(),
    }])
}

fn truncate_named_chip(
    open: &LabelPart,
    marks: &[LabelPart],
    name: &LabelPart,
    close: &LabelPart,
    budget: usize,
    keep_marks: bool,
) -> Option<Vec<LabelPart>> {
    let ellipsis_w = CHIP_NAME_ELLIPSIS.chars().count();
    let marks_w = if keep_marks { parts_width(marks) } else { 0 };
    let name_budget = budget.saturating_sub(2).saturating_sub(marks_w);
    if name_budget < 1 + ellipsis_w {
        return None;
    }
    let keep = name_budget - ellipsis_w;
    let prefix: String = name.text.chars().take(keep).collect();
    if prefix.is_empty() {
        return None;
    }
    if prefix.chars().count() >= name.text.chars().count() {
        return None;
    }
    let mut out = vec![open.clone()];
    if keep_marks {
        out.extend(marks.iter().cloned());
    }
    out.push(LabelPart {
        text: format!("{prefix}{CHIP_NAME_ELLIPSIS}"),
        kind: name.kind,
        target: name.target.clone(),
    });
    out.push(close.clone());
    if parts_width(&out) > budget {
        return None;
    }
    Some(out)
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

/// Default message rows of an expanded commit-message footer (not counting
/// the footer meta row).
///
/// This is the fixed expanded height, not a grow-up-to cap: a 1-line message
/// still takes this many rows (blank padding), so moving between commits
/// never changes the list height. The graph footer scrolls a longer message.
/// The commit-files footer ends the last visible line with `…`.
pub const COMMIT_MSG_LINES_DEFAULT: usize = 4;

/// Fewest message rows an expanded commit-message footer can be set to.
pub const COMMIT_MSG_LINES_MIN: usize = 1;

/// Most message rows an expanded commit-message footer can be set to.
pub const COMMIT_MSG_LINES_MAX: usize = 20;

/// Subject plus body for expand chrome. Empty body keeps the subject only.
pub fn format_commit_message(subject: &str, body: &str) -> String {
    let body = body.trim_end();
    if body.is_empty() {
        subject.to_string()
    } else if subject.is_empty() {
        body.to_string()
    } else {
        format!("{subject}\n\n{body}")
    }
}

/// Wrap `text` to `width` columns (char count, same as footer truncate).
///
/// Existing newlines stay. Overlong lines hard-break. Caps at `max_lines`.
/// Any leftover that does not fit (the rest of a line or later lines) ends
/// the last line with `…`.
pub fn wrap_commit_message(text: &str, width: usize, max_lines: usize) -> Vec<String> {
    let width = width.max(1);
    let max_lines = max_lines.max(1);
    let mut lines: Vec<String> = Vec::new();
    'paras: for para in text.split('\n') {
        if para.is_empty() {
            lines.push(String::new());
        }
        let chars: Vec<char> = para.chars().collect();
        for chunk in chars.chunks(width) {
            lines.push(chunk.iter().collect());
            if lines.len() > max_lines {
                break 'paras;
            }
        }
        if lines.len() > max_lines {
            break;
        }
    }
    if lines.len() > max_lines {
        lines.truncate(max_lines);
        if let Some(last) = lines.last_mut() {
            let mut kept: String = last.chars().take(width - 1).collect();
            kept.push('…');
            *last = kept;
        }
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

/// `[HEAD]` (detached only) + merged ref chips.
///
/// On a commit with several refs, the checked-out branch chip is first,
/// then default-branch / other locals / remotes / tags.
pub fn format_commit_ref_chips(
    refs: &[GraphRef],
    is_head: bool,
    head_branch: Option<&str>,
    glyphs: &GlyphSet,
) -> String {
    format_commit_ref_chips_with(refs, is_head, head_branch, glyphs, None)
}

/// Like [`format_commit_ref_chips`] with a default-branch override for colour/sort.
///
/// Checkout rank still leads; the override only affects the remaining
/// default-branch / other-local / remote / tag order.
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
///
/// Named checkout chips sort before other refs; detached `[HEAD]` stays a
/// prefix chip, not part of that sort.
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
            target: Some(PartTarget::DetachedHead),
        }]);
    }
    for chip in merge_commit_ref_chips(refs, is_head, head_branch, default_branch_override) {
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
    parts_text(&worktree_mark_parts(worktree, glyphs, 0, LabelKind::Meta))
}

/// Worktree mark as parts: the glyph ([`LabelKind::WorktreeMark`] with
/// `index`), ` path branch` as `text`, then `  [hidden]` as meta.
fn worktree_mark_parts(
    worktree: &Worktree,
    glyphs: &GlyphSet,
    index: usize,
    text: LabelKind,
) -> Vec<LabelPart> {
    let mut name = format!(" {}", worktree.path);
    if let Some(branch) = &worktree.branch {
        name.push(' ');
        name.push_str(branch);
    }
    let mut parts = vec![
        LabelPart {
            text: glyphs.worktree.to_string(),
            kind: LabelKind::WorktreeMark { worktree: index },
            target: None,
        },
        LabelPart {
            text: name,
            kind: text,
            target: None,
        },
    ];
    if worktree.hidden {
        parts.push(LabelPart {
            text: "  [hidden]".into(),
            kind: LabelKind::Meta,
            target: None,
        });
    }
    parts
}

/// True when the footer branch chip already represents this checkout.
///
/// The current checkout and any linked extra whose local branch is on the
/// commit must not paint `path` + branch as a second prefix next to that chip.
fn worktree_covered_by_ref_chip(
    worktree: &Worktree,
    commit: &Commit,
    is_head: bool,
    head_branch: Option<&str>,
) -> bool {
    if worktree.is_current {
        return true;
    }
    let Some(branch) = worktree.branch.as_deref() else {
        return false;
    };
    if branch.is_empty() || is_detached_head_branch(branch) {
        return false;
    }
    if is_head && head_branch == Some(branch) {
        return true;
    }
    commit
        .refs
        .iter()
        .any(|graph_ref| graph_ref.kind == RefKind::Local && graph_ref.name == branch)
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
    /// Local branch and its same-name remote-tracking ref.
    Merged(String, String),
    Local(String),
    Remote(String),
    Tag(String),
}

impl MergedRefChip {
    fn name(&self) -> &str {
        match self {
            Self::Merged(n, _) | Self::Local(n) | Self::Remote(n) | Self::Tag(n) => n,
        }
    }

    /// The refs the chip stands for: a merged chip lists its local branch,
    /// then its remote.
    fn refs(&self) -> Vec<GraphRef> {
        let one = |kind, name: &String| GraphRef {
            kind,
            name: name.clone(),
        };
        match self {
            Self::Merged(local, remote) => {
                vec![one(RefKind::Local, local), one(RefKind::Remote, remote)]
            }
            Self::Local(name) => vec![one(RefKind::Local, name)],
            Self::Remote(name) => vec![one(RefKind::Remote, name)],
            Self::Tag(name) => vec![one(RefKind::Tag, name)],
        }
    }

    /// Rank after checkout: default branch, other locals, unmatched remotes, tags.
    fn sort_key(&self, default_branch_override: Option<&str>) -> u8 {
        match self {
            Self::Merged(n, _) | Self::Local(n) => {
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
///
/// Sort is checkout ([`chip_is_checkout`]) first, then [`MergedRefChip::sort_key`],
/// then the original index. Named HEAD on this commit leads; detached HEAD
/// is a separate `[HEAD]` chip and does not change this order.
fn merge_commit_ref_chips(
    refs: &[GraphRef],
    is_head: bool,
    head_branch: Option<&str>,
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
            chips.push(MergedRefChip::Merged(
                local.name.clone(),
                remotes[i].name.clone(),
            ));
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
        let a_checkout = if chip_is_checkout(&a.1, head_branch, is_head) {
            0u8
        } else {
            1
        };
        let b_checkout = if chip_is_checkout(&b.1, head_branch, is_head) {
            0u8
        } else {
            1
        };
        a_checkout
            .cmp(&b_checkout)
            .then(
                a.1.sort_key(default_branch_override)
                    .cmp(&b.1.sort_key(default_branch_override)),
            )
            .then(a.0.cmp(&b.0))
    });
    indexed.into_iter().map(|(_, chip)| chip).collect()
}

/// True when this local/merged chip is the named HEAD branch on this commit.
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
        MergedRefChip::Local(name) | MergedRefChip::Merged(name, _) => name == branch,
        MergedRefChip::Remote(_) | MergedRefChip::Tag(_) => false,
    }
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
        MergedRefChip::Local(n) | MergedRefChip::Merged(n, _) => {
            if is_default_branch(n, default_branch_override) {
                LabelKind::ChipDefault
            } else {
                LabelKind::ChipLocal
            }
        }
    };
    let target = Some(PartTarget::Chip(chip.refs()));
    let part = |text: String, kind: LabelKind| LabelPart {
        text,
        kind,
        target: target.clone(),
    };
    match chip {
        MergedRefChip::Tag(_) | MergedRefChip::Remote(_) => {
            vec![part(format!("[{}]", chip.name()), kind)]
        }
        MergedRefChip::Local(_) | MergedRefChip::Merged(..) => {
            let mut parts = vec![part("[".into(), kind)];
            // Checkout + sync must be one run so paint cannot emit two icon
            // spans. Footer already concatenates them as `[checkout][sync][name]`.
            let remote = match chip {
                MergedRefChip::Merged(_, remote) => Some(remote.clone()),
                _ => None,
            };
            let mut marks = String::new();
            if is_checkout {
                marks.push_str(glyphs.checkout_mark);
            }
            if remote.is_some() {
                marks.push_str(glyphs.sync_mark);
            }
            if !marks.is_empty() {
                parts.push(LabelPart {
                    text: marks,
                    kind: if is_checkout {
                        LabelKind::ChipHead
                    } else {
                        LabelKind::ChipRemote
                    },
                    target: Some(PartTarget::Marks {
                        branch: chip.name().to_string(),
                        checkout: is_checkout,
                        remote,
                    }),
                });
            }
            parts.push(part(chip.name().to_string(), kind));
            parts.push(part("]".into(), kind));
            parts
        }
    }
}

#[cfg(test)]
mod tests;

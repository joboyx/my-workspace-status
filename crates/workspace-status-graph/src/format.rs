//! Headless line format for graph rows. The widget paints these strings.

use crate::glyphs::GlyphSet;
use crate::model::{Commit, GraphRef, GraphRow, RefKind, Stash, SyncState, SyncStatus, Worktree};

/// Prefer keeping at least this many left columns before dropping right meta.
const MIN_LEFT_KEEP: usize = 12;

/// Cap for the author column (Ink `authorWidth`).
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

/// Compact relative date. Matches Ink `formatRelativeDate` exactly.
pub fn format_relative_date(unix: i64, now_unix: i64) -> String {
    let delta = (now_unix - unix).max(0);
    if delta < 60 {
        return "just now".into();
    }
    if delta < 3600 {
        return format!("{}m", delta / 60);
    }
    if delta < 86400 {
        return format!("{}h", delta / 3600);
    }
    if delta < 86400 * 14 {
        return format!("{}d", delta / 86400);
    }
    if delta < 86400 * 70 {
        return format!("{}w", delta / (86400 * 7));
    }
    format!("{}y", delta / (86400 * 365))
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

/// Date / author column widths shared across a commit window (Ink rails).
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
    let (hash, date, author) = padded_meta(
        &opts.commit.id,
        &opts.commit.author_name,
        opts.commit.author_date_unix,
        opts.date_width,
        opts.author_width,
        opts.now_unix,
    );
    let mut left = format_commit_ref_chips(
        &opts.commit.refs,
        opts.is_head,
        opts.head_branch,
        opts.glyphs,
    );
    for worktree in opts.worktrees {
        if !left.is_empty() {
            left.push(' ');
        }
        left.push_str(&worktree_mark(worktree, opts.glyphs));
    }
    assemble_spacer(&left, &hash, &date, &author, opts.available)
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
    let (hash, date, author) = padded_meta(
        &opts.stash.id,
        &opts.stash.author_name,
        opts.stash.author_date_unix,
        opts.date_width,
        opts.author_width,
        opts.now_unix,
    );
    assemble_spacer(&opts.stash.stash_ref, &hash, &date, &author, opts.available)
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
fn assemble_spacer(left: &str, hash: &str, date: &str, author: &str, available: usize) -> String {
    if available == 0 {
        return String::new();
    }
    let cols = pick_meta_columns(available, hash, date, author, MIN_LEFT_KEEP);
    let meta = meta_columns_text(hash, date, author, cols);
    let ref_budget = available.saturating_sub(meta.chars().count());
    let left = if ref_budget == 0 {
        String::new()
    } else {
        trunc(left, ref_budget)
    };
    if meta.is_empty() {
        return pad_right(&left, available);
    }
    let left_len = left.chars().count();
    let meta_len = meta.chars().count();
    let pad = available.saturating_sub(left_len).saturating_sub(meta_len);
    format!("{left}{}{meta}", " ".repeat(pad))
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
    let mut parts = Vec::new();
    if show_detached_head_chip(head_branch, is_head) {
        parts.push("[HEAD]".to_string());
    }
    for chip in merge_commit_ref_chips(refs) {
        let checkout = chip_is_checkout(&chip, head_branch, is_head);
        parts.push(format_merged_chip(&chip, checkout, glyphs));
    }
    parts.join(" ")
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

fn is_default_branch(name: &str) -> bool {
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

    fn sort_key(&self) -> u8 {
        match self {
            Self::Merged(n) | Self::Local(n) => {
                if is_default_branch(n) {
                    0
                } else {
                    1
                }
            }
            Self::Remote(n) => {
                if is_default_branch(remote_short_name(n)) {
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
fn merge_commit_ref_chips(refs: &[GraphRef]) -> Vec<MergedRefChip> {
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
    indexed.sort_by(|a, b| a.1.sort_key().cmp(&b.1.sort_key()).then(a.0.cmp(&b.0)));
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
    match chip {
        MergedRefChip::Tag(_) | MergedRefChip::Remote(_) => format!("[{}]", chip.name()),
        MergedRefChip::Local(_) | MergedRefChip::Merged(_) => {
            let mut inner = String::new();
            if is_checkout {
                inner.push_str(glyphs.checkout_mark);
            }
            if matches!(chip, MergedRefChip::Merged(_)) {
                inner.push_str(glyphs.sync_mark);
            }
            inner.push_str(chip.name());
            format!("[{inner}]")
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
    fn format_relative_date_matches_ink_buckets() {
        let now = 1_700_000_000;
        assert_eq!(format_relative_date(now, now), "just now");
        assert_eq!(format_relative_date(now - 59, now), "just now");
        assert_eq!(format_relative_date(now - 60, now), "1m");
        assert_eq!(format_relative_date(now - 3599, now), "59m");
        assert_eq!(format_relative_date(now - 3600, now), "1h");
        assert_eq!(format_relative_date(now - 86399, now), "23h");
        assert_eq!(format_relative_date(now - 86400, now), "1d");
        assert_eq!(format_relative_date(now - 86400 * 13, now), "13d");
        assert_eq!(format_relative_date(now - 86400 * 14, now), "2w");
        assert_eq!(format_relative_date(now - 86400 * 69, now), "9w");
        assert_eq!(format_relative_date(now - 86400 * 70, now), "0y");
        assert_eq!(format_relative_date(now - 86400 * 365, now), "1y");
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
        });
        assert!(
            line.contains("[feature") || line.contains("feature/"),
            "keep refs when dropping hash: {line}"
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
            date_width: 4,
            author_width: 12,
            now_unix: 1_700_000_000,
        });
        assert!(line.contains("stash@{0}"), "{line}");
        assert!(line.contains("s1abcde"), "{line}");
        assert!(line.contains("1d"), "{line}");
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
        assert!(line.contains("stash@{0}"), "keep stash@{{n}}: {line}");
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
    }
}

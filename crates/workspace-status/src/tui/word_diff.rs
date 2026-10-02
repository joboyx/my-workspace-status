//! Intra-line word-change ranges for paired modified diff lines.
//!
//! This is a paint overlay over line-based `git diff` output. Git stays
//! line-based: hunks, line pairing, and partial patches do not change. For a
//! deleted line and the added line paired with it, this module finds the
//! changed words so paint can mark them on top of the add/del row
//! background, like the VS Code diff editor.
//!
//! Tokens are words (`alphanumeric` or `_` runs), whitespace runs, and
//! single other chars. A token-level LCS marks the changed tokens. Ranges
//! are byte ranges into the cell text and always land on char boundaries.

use std::ops::Range;

use super::diff::{DiffCellKind, DiffRow};
use crate::helpers::visible_width;

/// Skip lines longer than this many characters (same budget as syntax
/// highlighting), so a huge diff line cannot stall a paint.
const MAX_WORD_DIFF_CHARS: usize = 4096;

/// Upper bound on LCS table cells (old tokens × new tokens after the common
/// prefix and suffix are trimmed).
const MAX_LCS_CELLS: usize = 250_000;

/// Zero-width joiner: the char after it joins the same glyph cluster.
const ZWJ: char = '\u{200d}';

/// Token class for the tokenizer.
#[derive(Clone, Copy, PartialEq, Eq)]
enum TokenClass {
    Word,
    Space,
    Other,
}

fn char_class(c: char) -> TokenClass {
    if c.is_alphanumeric() || c == '_' {
        TokenClass::Word
    } else if c.is_whitespace() {
        TokenClass::Space
    } else {
        TokenClass::Other
    }
}

/// One token: its byte range in the line and its class.
struct Token {
    range: Range<usize>,
    class: TokenClass,
}

/// True for combining marks and emoji skin-tone modifiers: they attach to
/// the char before them even when `visible_width` gives them a column.
fn extends_cluster(c: char) -> bool {
    matches!(
        c,
        '\u{0300}'..='\u{036f}'
            | '\u{1ab0}'..='\u{1aff}'
            | '\u{1dc0}'..='\u{1dff}'
            | '\u{20d0}'..='\u{20ff}'
            | '\u{fe20}'..='\u{fe2f}'
            | '\u{1f3fb}'..='\u{1f3ff}'
    )
}

/// Split `text` into word, whitespace, and single-char tokens.
///
/// A zero-width char (VS16, ZWJ), a combining mark, a skin-tone modifier,
/// and the char after a ZWJ extend the previous token, so a glyph cluster
/// is never split.
fn tokenize(text: &str) -> Vec<Token> {
    let mut tokens: Vec<Token> = Vec::new();
    let mut after_zwj = false;
    for (start, c) in text.char_indices() {
        let end = start + c.len_utf8();
        let class = char_class(c);
        let joins_cluster =
            after_zwj || extends_cluster(c) || visible_width(c.encode_utf8(&mut [0; 4])) == 0;
        after_zwj = c == ZWJ;
        if let Some(last) = tokens.last_mut() {
            let same_run = class == last.class && class != TokenClass::Other;
            if joins_cluster || same_run {
                last.range.end = end;
                continue;
            }
        }
        tokens.push(Token {
            range: start..end,
            class,
        });
    }
    tokens
}

/// Changed byte ranges on the old (del) line and on the new (add) line.
pub(crate) type WordChangeRanges = (Vec<Range<usize>>, Vec<Range<usize>>);

/// Byte ranges of changed text on the old (del) and new (add) line.
///
/// Returns `None` when the texts are identical, either text is over the
/// character budget, the token diff is over the work budget, or the lines
/// share no non-whitespace token (a full rewrite: the row background
/// already says that). One side may be empty (a pure insertion or deletion
/// inside the line). A change inside a word marks the whole word.
pub(crate) fn word_change_ranges(old: &str, new: &str) -> Option<WordChangeRanges> {
    if old == new
        || old.chars().count() > MAX_WORD_DIFF_CHARS
        || new.chars().count() > MAX_WORD_DIFF_CHARS
    {
        return None;
    }
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    let old_text = |i: usize| &old[old_tokens[i].range.clone()];
    let new_text = |j: usize| &new[new_tokens[j].range.clone()];

    let mut prefix = 0;
    while prefix < old_tokens.len().min(new_tokens.len()) && old_text(prefix) == new_text(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_tokens.len().min(new_tokens.len()) - prefix
        && old_text(old_tokens.len() - 1 - suffix) == new_text(new_tokens.len() - 1 - suffix)
    {
        suffix += 1;
    }
    let old_mid = prefix..old_tokens.len() - suffix;
    let new_mid = prefix..new_tokens.len() - suffix;
    if old_mid.len().saturating_mul(new_mid.len()) > MAX_LCS_CELLS {
        return None;
    }

    let mut old_kept = vec![false; old_tokens.len()];
    let mut new_kept = vec![false; new_tokens.len()];
    for i in (0..prefix).chain(old_mid.end..old_tokens.len()) {
        old_kept[i] = true;
    }
    for j in (0..prefix).chain(new_mid.end..new_tokens.len()) {
        new_kept[j] = true;
    }
    for (i, j) in lcs_pairs(old_mid.len(), new_mid.len(), |i, j| {
        old_text(old_mid.start + i) == new_text(new_mid.start + j)
    }) {
        old_kept[old_mid.start + i] = true;
        new_kept[new_mid.start + j] = true;
    }

    let shares_text = old_tokens
        .iter()
        .zip(&old_kept)
        .any(|(token, &kept)| kept && token.class != TokenClass::Space);
    if !shares_text {
        return None;
    }
    Some((
        changed_ranges(&old_tokens, &old_kept),
        changed_ranges(&new_tokens, &new_kept),
    ))
}

/// Index pairs `(i, j)` of one longest common subsequence of two sequences
/// of length `n` and `m`, in order.
fn lcs_pairs(n: usize, m: usize, eq: impl Fn(usize, usize) -> bool) -> Vec<(usize, usize)> {
    // `table[i][j]` = LCS length of the suffixes starting at `i` and `j`.
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[i * width + j] = if eq(i, j) {
                table[(i + 1) * width + j + 1] + 1
            } else {
                table[(i + 1) * width + j].max(table[i * width + j + 1])
            };
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if eq(i, j) {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if table[(i + 1) * width + j] >= table[i * width + j + 1] {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

/// Merge runs of adjacent changed tokens into byte ranges.
fn changed_ranges(tokens: &[Token], kept: &[bool]) -> Vec<Range<usize>> {
    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut prev_changed = false;
    for (token, &kept) in tokens.iter().zip(kept) {
        if kept {
            prev_changed = false;
            continue;
        }
        match ranges.last_mut() {
            Some(last) if prev_changed => last.end = token.range.end,
            _ => ranges.push(token.range.clone()),
        }
        prev_changed = true;
    }
    ranges
}

/// Per-row word-change byte ranges for the painted diff rows.
///
/// Indexed by row in the slice passed to [`diff_word_ranges`]. `left`
/// holds ranges into the row's left cell text, `right` into its right cell
/// text (split mode only).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiffWordRanges {
    left: Vec<Vec<Range<usize>>>,
    right: Vec<Vec<Range<usize>>>,
}

impl DiffWordRanges {
    /// Ranges into the left cell text of `row`. Empty when there are none
    /// or `row` is out of range.
    pub(crate) fn left(&self, row: usize) -> &[Range<usize>] {
        self.left.get(row).map_or(&[], Vec::as_slice)
    }

    /// Ranges into the right cell text of `row`. Empty when there are none
    /// or `row` is out of range.
    pub(crate) fn right(&self, row: usize) -> &[Range<usize>] {
        self.right.get(row).map_or(&[], Vec::as_slice)
    }
}

/// Paired del/add rows: split rows pair within one row, inline rows pair
/// a del row with a later add row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RowPair {
    Split(usize),
    Inline { del: usize, add: usize },
}

fn is_inline_line(row: &DiffRow, kind: DiffCellKind) -> bool {
    matches!(row, DiffRow::Line { left, right: None } if left.kind == kind)
}

/// Every del/add row pair in `rows`, in row order.
fn row_pairs(rows: &[DiffRow]) -> Vec<RowPair> {
    let mut pairs = Vec::new();
    let mut i = 0;
    while i < rows.len() {
        match &rows[i] {
            DiffRow::Line {
                left,
                right: Some(right),
            } => {
                if left.kind == DiffCellKind::Del && right.kind == DiffCellKind::Add {
                    pairs.push(RowPair::Split(i));
                }
                i += 1;
            }
            row if is_inline_line(row, DiffCellKind::Del) => {
                let del_start = i;
                while i < rows.len() && is_inline_line(&rows[i], DiffCellKind::Del) {
                    i += 1;
                }
                let add_start = i;
                while i < rows.len() && is_inline_line(&rows[i], DiffCellKind::Add) {
                    i += 1;
                }
                let count = (add_start - del_start).min(i - add_start);
                pairs.extend((0..count).map(|j| RowPair::Inline {
                    del: del_start + j,
                    add: add_start + j,
                }));
            }
            _ => i += 1,
        }
    }
    pairs
}

/// Ranges for every paired row with at least one side inside `visible`.
///
/// Split rows pair their own del (left) and add (right) cells. Inline rows
/// pair a run of del rows with the run of add rows right after it, by
/// index; extra rows of the longer run stay unpaired. A partner outside
/// `visible` is still paired. Pairs fully outside `visible` are not
/// computed.
pub(crate) fn diff_word_ranges(rows: &[DiffRow], visible: Range<usize>) -> DiffWordRanges {
    let mut out = DiffWordRanges {
        left: vec![Vec::new(); rows.len()],
        right: vec![Vec::new(); rows.len()],
    };
    for pair in row_pairs(rows) {
        let (del, add) = match pair {
            RowPair::Split(row) => (row, row),
            RowPair::Inline { del, add } => (del, add),
        };
        if !visible.contains(&del) && !visible.contains(&add) {
            continue;
        }
        let old_text = match &rows[del] {
            DiffRow::Line { left, .. } => left.text.as_str(),
            _ => continue,
        };
        let new_text = match (&rows[add], pair) {
            (DiffRow::Line { right: Some(r), .. }, RowPair::Split(_)) => r.text.as_str(),
            (DiffRow::Line { left, .. }, RowPair::Inline { .. }) => left.text.as_str(),
            _ => continue,
        };
        let Some((old_ranges, new_ranges)) = word_change_ranges(old_text, new_text) else {
            continue;
        };
        out.left[del] = old_ranges;
        match pair {
            RowPair::Split(row) => out.right[row] = new_ranges,
            RowPair::Inline { add, .. } => out.left[add] = new_ranges,
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::diff::{DiffCell, DiffSection};

    fn cell(kind: DiffCellKind, text: &str) -> DiffCell {
        DiffCell {
            kind,
            text: text.to_string(),
            line_no: Some(1),
        }
    }

    fn inline(kind: DiffCellKind, text: &str) -> DiffRow {
        DiffRow::Line {
            left: cell(kind, text),
            right: None,
        }
    }

    fn split(left: DiffCell, right: DiffCell) -> DiffRow {
        DiffRow::Line {
            left,
            right: Some(right),
        }
    }

    /// One range as a `Vec`, so assertions read without a one-range literal.
    fn one(start: usize, end: usize) -> Vec<Range<usize>> {
        std::iter::once(start..end).collect()
    }

    fn texts<'a>(text: &'a str, ranges: &[Range<usize>]) -> Vec<&'a str> {
        ranges.iter().map(|r| &text[r.clone()]).collect()
    }

    /// Changed text on each side, for readable assertions.
    fn changed<'a>(old: &'a str, new: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
        let (o, n) = word_change_ranges(old, new).expect("ranges");
        (texts(old, &o), texts(new, &n))
    }

    #[test]
    fn one_changed_word_has_exact_ranges() {
        let ranges = word_change_ranges("let count = 1;", "let total = 1;");
        assert_eq!(ranges, Some((one(4, 9), one(4, 9))));
    }

    #[test]
    fn punctuation_is_a_token_boundary() {
        assert_eq!(changed("foo,bar", "foo,baz"), (vec!["bar"], vec!["baz"]));
    }

    #[test]
    fn change_inside_a_word_marks_the_whole_word() {
        assert_eq!(
            changed("let count = 1;", "let counter = 1;"),
            (vec!["count"], vec!["counter"])
        );
    }

    #[test]
    fn separate_edits_are_not_bridged() {
        let (old, new) = changed("a x b y c", "a X b Y c");
        assert_eq!(old, vec!["x", "y"]);
        assert_eq!(new, vec!["X", "Y"]);
    }

    #[test]
    fn adjacent_changed_tokens_merge() {
        assert_eq!(changed("call(a);", "call(x+y);"), (vec!["a"], vec!["x+y"]));
    }

    #[test]
    fn pure_insertion_leaves_old_side_empty() {
        let ranges = word_change_ranges("foo(a)", "foo(a, b)");
        assert_eq!(ranges, Some((vec![], one(5, 8))));
    }

    #[test]
    fn whitespace_only_edit_marks_the_whitespace() {
        assert_eq!(changed("a  b", "a b"), (vec!["  "], vec![" "]));
        assert_eq!(changed("let x = 1;", "let x = 1;  "), (vec![], vec!["  "]));
    }

    #[test]
    fn emoji_ranges_stay_on_char_boundaries() {
        let old = "status ✅ ok";
        let new = "status ❌ ok";
        let (o, n) = word_change_ranges(old, new).expect("ranges");
        assert_eq!(texts(old, &o), vec!["✅"]);
        assert_eq!(texts(new, &n), vec!["❌"]);
        for r in &o {
            assert!(old.is_char_boundary(r.start) && old.is_char_boundary(r.end));
        }
        for r in &n {
            assert!(new.is_char_boundary(r.start) && new.is_char_boundary(r.end));
        }
    }

    #[test]
    fn emoji_is_its_own_token() {
        assert_eq!(changed("go🚀now", "go🔥now"), (vec!["🚀"], vec!["🔥"]));
    }

    #[test]
    fn vs16_and_zwj_clusters_are_not_split() {
        assert_eq!(
            changed("love ❤️ it", "love 💙 it"),
            (vec!["❤️"], vec!["💙"])
        );
        assert_eq!(
            changed("dev 👨‍💻 here", "dev 👩‍💻 here"),
            (vec!["👨‍💻"], vec!["👩‍💻"])
        );
    }

    #[test]
    fn combining_mark_edit_marks_the_whole_word() {
        assert_eq!(
            changed("a nai\u{0308}ve b", "a naive b"),
            (vec!["nai\u{0308}ve"], vec!["naive"])
        );
    }

    #[test]
    fn skin_tone_edit_marks_the_whole_emoji() {
        assert_eq!(changed("ok 👍🏽 now", "ok 👍🏿 now"), (vec!["👍🏽"], vec!["👍🏿"]));
    }

    #[test]
    fn identical_lines_have_no_ranges() {
        assert_eq!(word_change_ranges("same line", "same line"), None);
    }

    #[test]
    fn total_rewrite_has_no_ranges() {
        assert_eq!(word_change_ranges("alpha beta", "gamma delta"), None);
    }

    #[test]
    fn over_char_budget_has_no_ranges() {
        let old = format!("{} a", "x".repeat(MAX_WORD_DIFF_CHARS));
        let new = format!("{} b", "x".repeat(MAX_WORD_DIFF_CHARS));
        assert_eq!(word_change_ranges(&old, &new), None);
    }

    #[test]
    fn over_lcs_budget_has_no_ranges() {
        // ~1000 tokens per side between a shared prefix and suffix.
        let old = format!("start {} end", "a ".repeat(500));
        let new = format!("start {} end", "b,".repeat(500));
        assert_eq!(word_change_ranges(&old, &new), None);
    }

    #[test]
    fn inline_pairs_by_index_with_uneven_runs() {
        let rows = vec![
            DiffRow::Hunk {
                text: "@@ -1,2 +1,3 @@".into(),
            },
            inline(DiffCellKind::Del, "let a = 1;"),
            inline(DiffCellKind::Add, "let a = 2;"),
            inline(DiffCellKind::Add, "let b = 3;"),
        ];
        let ranges = diff_word_ranges(&rows, 0..rows.len());
        assert_eq!(ranges.left(1), one(8, 9));
        assert_eq!(ranges.left(2), one(8, 9));
        assert!(ranges.left(3).is_empty());
        assert!(ranges.left(0).is_empty());
        assert!(ranges.right(1).is_empty());
        assert!(ranges.left(99).is_empty());
    }

    #[test]
    fn split_row_pairs_left_and_right() {
        let rows = vec![
            split(
                cell(DiffCellKind::Ctx, "fn main() {"),
                cell(DiffCellKind::Ctx, "fn main() {"),
            ),
            split(
                cell(DiffCellKind::Del, "    run(1);"),
                cell(DiffCellKind::Add, "    run(2);"),
            ),
            split(
                cell(DiffCellKind::Del, "    old();"),
                cell(DiffCellKind::Empty, ""),
            ),
        ];
        let ranges = diff_word_ranges(&rows, 0..rows.len());
        assert_eq!(ranges.left(1), one(8, 9));
        assert_eq!(ranges.right(1), one(8, 9));
        for row in [0, 2] {
            assert!(ranges.left(row).is_empty());
            assert!(ranges.right(row).is_empty());
        }
    }

    #[test]
    fn meta_between_runs_breaks_pairing() {
        let rows = vec![
            inline(DiffCellKind::Del, "let a = 1;"),
            inline(DiffCellKind::Meta, "\\ No newline at end of file"),
            inline(DiffCellKind::Add, "let a = 2;"),
        ];
        let ranges = diff_word_ranges(&rows, 0..rows.len());
        for row in 0..rows.len() {
            assert!(ranges.left(row).is_empty());
        }
    }

    #[test]
    fn pure_add_and_del_rows_have_no_ranges() {
        let rows = vec![
            DiffRow::Section(DiffSection::Unstaged),
            inline(DiffCellKind::Add, "let a = 2;"),
            inline(DiffCellKind::Ctx, "ctx"),
            inline(DiffCellKind::Del, "let a = 1;"),
            inline(DiffCellKind::Ctx, "ctx"),
        ];
        let ranges = diff_word_ranges(&rows, 0..rows.len());
        for row in 0..rows.len() {
            assert!(ranges.left(row).is_empty());
        }
    }

    #[test]
    fn partner_above_visible_start_is_still_paired() {
        let rows = vec![
            inline(DiffCellKind::Del, "let a = 1;"),
            inline(DiffCellKind::Del, "let b = 1;"),
            inline(DiffCellKind::Add, "let a = 2;"),
            inline(DiffCellKind::Add, "let b = 2;"),
        ];
        let ranges = diff_word_ranges(&rows, 3..4);
        assert_eq!(ranges.left(3), one(8, 9));
        assert_eq!(ranges.left(1), one(8, 9));
        // Pair 0↔2 lies fully outside `visible`.
        assert!(ranges.left(0).is_empty());
        assert!(ranges.left(2).is_empty());
    }

    #[test]
    fn rows_outside_visible_are_not_computed() {
        let rows = vec![
            split(
                cell(DiffCellKind::Del, "let a = 1;"),
                cell(DiffCellKind::Add, "let a = 2;"),
            ),
            split(
                cell(DiffCellKind::Del, "let b = 1;"),
                cell(DiffCellKind::Add, "let b = 2;"),
            ),
        ];
        let ranges = diff_word_ranges(&rows, 1..2);
        assert!(ranges.left(0).is_empty());
        assert!(ranges.right(0).is_empty());
        assert_eq!(ranges.left(1), one(8, 9));
        assert_eq!(ranges.right(1), one(8, 9));
    }
}

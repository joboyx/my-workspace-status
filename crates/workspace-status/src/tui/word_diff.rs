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
//! The comparison reads each cell's word key, which keeps the columns of
//! an expanded tab as `\t`, so a tab ↔ spaces change is not invisible.

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
/// character budget, the token diff is over the work budget, the lines
/// share no non-whitespace token (a full rewrite: the row background
/// already says that), or nothing is marked. One side may be empty (a pure
/// insertion or deletion inside the line). A change inside a word marks
/// the whole word.
///
/// With `ignore_whitespace` (content loaded with `git diff -w`),
/// whitespace tokens take no part in the comparison and are never marked:
/// a changed, inserted, or removed whitespace run stays on the row
/// background, and a whitespace-only difference returns `None`. Two
/// changed words with only whitespace between them paint as two ranges.
pub(crate) fn word_change_ranges(
    old: &str,
    new: &str,
    ignore_whitespace: bool,
) -> Option<WordChangeRanges> {
    if old == new
        || old.chars().count() > MAX_WORD_DIFF_CHARS
        || new.chars().count() > MAX_WORD_DIFF_CHARS
    {
        return None;
    }
    let old_tokens = tokenize(old);
    let new_tokens = tokenize(new);
    // Indices of the tokens that take part in the comparison.
    let compared = |tokens: &[Token]| -> Vec<usize> {
        (0..tokens.len())
            .filter(|&i| !ignore_whitespace || tokens[i].class != TokenClass::Space)
            .collect()
    };
    let old_seq = compared(&old_tokens);
    let new_seq = compared(&new_tokens);
    let old_text = |i: usize| &old[old_tokens[old_seq[i]].range.clone()];
    let new_text = |j: usize| &new[new_tokens[new_seq[j]].range.clone()];

    let common = old_seq.len().min(new_seq.len());
    let mut prefix = 0;
    while prefix < common && old_text(prefix) == new_text(prefix) {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < common - prefix
        && old_text(old_seq.len() - 1 - suffix) == new_text(new_seq.len() - 1 - suffix)
    {
        suffix += 1;
    }
    let old_mid = prefix..old_seq.len() - suffix;
    let new_mid = prefix..new_seq.len() - suffix;
    if old_mid.len().saturating_mul(new_mid.len()) > MAX_LCS_CELLS {
        return None;
    }

    // Tokens left out of the comparison count as kept: never marked.
    let mut old_kept = vec![true; old_tokens.len()];
    let mut new_kept = vec![true; new_tokens.len()];
    for i in old_mid.clone() {
        old_kept[old_seq[i]] = false;
    }
    for j in new_mid.clone() {
        new_kept[new_seq[j]] = false;
    }
    for (i, j) in lcs_pairs(old_mid.len(), new_mid.len(), |i, j| {
        old_text(old_mid.start + i) == new_text(new_mid.start + j)
    }) {
        old_kept[old_seq[old_mid.start + i]] = true;
        new_kept[new_seq[new_mid.start + j]] = true;
    }

    let shares_text = old_tokens
        .iter()
        .zip(&old_kept)
        .any(|(token, &kept)| kept && token.class != TokenClass::Space);
    if !shares_text {
        return None;
    }
    let old_ranges = changed_ranges(&old_tokens, &old_kept);
    let new_ranges = changed_ranges(&new_tokens, &new_kept);
    if old_ranges.is_empty() && new_ranges.is_empty() {
        return None;
    }
    Some((old_ranges, new_ranges))
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
/// computed. `ignore_whitespace` is the shown content's `-w` marker and
/// goes to [`word_change_ranges`].
///
/// Each side compares its [`DiffCell::word_key`](super::diff::DiffCell::word_key),
/// not the painted text: a tab and the spaces it expanded to differ, so a
/// tab ↔ spaces change marks the expanded columns (unless whitespace is
/// ignored).
pub(crate) fn diff_word_ranges(
    rows: &[DiffRow],
    visible: Range<usize>,
    ignore_whitespace: bool,
) -> DiffWordRanges {
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
        // Word keys keep tab columns as `\t`, so a tab that became spaces
        // still differs; ranges into a key are ranges into the cell text.
        let old_text = match &rows[del] {
            DiffRow::Line { left, .. } => left.word_key(),
            _ => continue,
        };
        let new_text = match (&rows[add], pair) {
            (DiffRow::Line { right: Some(r), .. }, RowPair::Split(_)) => r.word_key(),
            (DiffRow::Line { left, .. }, RowPair::Inline { .. }) => left.word_key(),
            _ => continue,
        };
        let Some((old_ranges, new_ranges)) =
            word_change_ranges(old_text, new_text, ignore_whitespace)
        else {
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
            ..Default::default()
        }
    }

    /// Inline row built from the source line, the way the parser builds it
    /// (tabs expanded, tab columns kept in the word key).
    fn source_row(kind: DiffCellKind, line: &str) -> DiffRow {
        DiffRow::Line {
            left: DiffCell::from_source(kind, line, Some(1)),
            right: None,
        }
    }

    /// Word ranges of an inline del/add pair built from source lines.
    fn source_pair(old: &str, new: &str, ignore_whitespace: bool) -> DiffWordRanges {
        let rows = vec![
            source_row(DiffCellKind::Del, old),
            source_row(DiffCellKind::Add, new),
        ];
        diff_word_ranges(&rows, 0..rows.len(), ignore_whitespace)
    }

    const TAB_LINE: &str = "\tTab indented line";
    const SPACES_LINE: &str = "    Tab indented line";

    #[test]
    fn tab_to_spaces_marks_the_indent_on_both_sides() {
        let ranges = source_pair(TAB_LINE, SPACES_LINE, false);
        assert_eq!(ranges.left(0), one(0, 4));
        assert_eq!(ranges.left(1), one(0, 4));
        // Spaces back to a tab marks the same columns.
        let back = source_pair(SPACES_LINE, TAB_LINE, false);
        assert_eq!(back.left(0), one(0, 4));
        assert_eq!(back.left(1), one(0, 4));
    }

    #[test]
    fn split_tab_to_spaces_marks_the_indent_on_both_sides() {
        let rows = vec![split(
            DiffCell::from_source(DiffCellKind::Del, TAB_LINE, Some(1)),
            DiffCell::from_source(DiffCellKind::Add, SPACES_LINE, Some(1)),
        )];
        let ranges = diff_word_ranges(&rows, 0..1, false);
        assert_eq!(ranges.left(0), one(0, 4));
        assert_eq!(ranges.right(0), one(0, 4));
    }

    #[test]
    fn tab_kept_as_tab_is_not_marked() {
        let ranges = source_pair("\tlet a = 1;", "\tlet a = 2;", false);
        assert_eq!(ranges.left(0), one(12, 13));
        assert_eq!(ranges.left(1), one(12, 13));
        let same = source_pair("\tlet a = 1;", "\tlet a = 1;  ", false);
        assert!(same.left(0).is_empty());
        assert_eq!(same.left(1), one(14, 16));
    }

    #[test]
    fn tab_inside_a_line_vs_spaces_marks_the_expanded_columns() {
        // `a\tb` paints `a   b`: the tab fills columns 1..4.
        let ranges = source_pair("let a\t= 1;", "let a   = 1;", false);
        assert_eq!(ranges.left(0), one(5, 8));
        assert_eq!(ranges.left(1), one(5, 8));
    }

    #[test]
    fn ignore_whitespace_leaves_tab_to_spaces_unmarked() {
        for (old, new) in [(TAB_LINE, SPACES_LINE), ("let a\t= 1;", "let a   = 1;")] {
            let ranges = source_pair(old, new, true);
            assert!(
                ranges.left(0).is_empty() && ranges.left(1).is_empty(),
                "{old:?} {new:?}"
            );
        }
        // A real word change next to the tab still marks the word only.
        let ranges = source_pair("\tlet a = 1;", "    let a = 2;", true);
        assert_eq!(ranges.left(0), one(12, 13));
        assert_eq!(ranges.left(1), one(12, 13));
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
        let (o, n) = word_change_ranges(old, new, false).expect("ranges");
        (texts(old, &o), texts(new, &n))
    }

    #[test]
    fn one_changed_word_has_exact_ranges() {
        let ranges = word_change_ranges("let count = 1;", "let total = 1;", false);
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
        let ranges = word_change_ranges("foo(a)", "foo(a, b)", false);
        assert_eq!(ranges, Some((vec![], one(5, 8))));
    }

    #[test]
    fn whitespace_only_edit_marks_the_whitespace() {
        assert_eq!(changed("a  b", "a b"), (vec!["  "], vec![" "]));
        assert_eq!(changed("let x = 1;", "let x = 1;  "), (vec![], vec!["  "]));
    }

    /// Changed text on each side with `ignore_whitespace` on.
    fn changed_w<'a>(old: &'a str, new: &'a str) -> (Vec<&'a str>, Vec<&'a str>) {
        let (o, n) = word_change_ranges(old, new, true).expect("ranges");
        (texts(old, &o), texts(new, &n))
    }

    const ONE_WORD: &str = "The real change on this line is one word, also extra spaces are added.";
    const TWO_WORDS: &str =
        "The real change on this line is two words, also  extra spaces are added.";

    #[test]
    fn ignore_whitespace_marks_only_the_real_change() {
        assert_eq!(
            changed_w(ONE_WORD, TWO_WORDS),
            (vec!["one", "word"], vec!["two", "words"])
        );
        // Without the flag the widened space is marked too.
        assert_eq!(
            changed(ONE_WORD, TWO_WORDS),
            (vec!["one", "word", " "], vec!["two", "words", "  "])
        );
    }

    #[test]
    fn ignore_whitespace_skips_whitespace_inserted_on_one_side() {
        assert_eq!(changed_w("call(a,b)", "call(a, c)"), (vec!["b"], vec!["c"]));
        assert_eq!(
            changed_w("  let x = old;", "let x = new;   "),
            (vec!["old"], vec!["new"])
        );
        // Without the flag the inserted space joins the changed word.
        assert_eq!(changed("call(a,b)", "call(a, c)"), (vec!["b"], vec![" c"]));
    }

    #[test]
    fn ignore_whitespace_whitespace_only_change_has_no_ranges() {
        for (old, new) in [
            ("a  b", "a b"),
            ("x = 1;", "    x = 1;"),
            ("x = 1;", "x = 1;  "),
            ("x\t= 1;", "x = 1;"),
        ] {
            assert_eq!(word_change_ranges(old, new, true), None, "{old:?} {new:?}");
        }
    }

    /// Whitespace between two changed words is never marked, so the words
    /// paint as two ranges, even when that whitespace changed too.
    #[test]
    fn ignore_whitespace_splits_changed_words_at_whitespace() {
        let (old, new) = ("a x y b", "a X  Y b");
        let (o, n) = word_change_ranges(old, new, true).expect("ranges");
        assert_eq!(o, vec![2..3, 4..5]);
        assert_eq!(n, vec![2..3, 5..6]);
        assert_eq!(changed(old, new), (vec!["x y"], vec!["X  Y"]));
    }

    #[test]
    fn ignore_whitespace_keeps_the_shared_text_rule() {
        assert_eq!(word_change_ranges("alpha beta", "gamma  delta", true), None);
    }

    #[test]
    fn diff_word_ranges_passes_the_flag_on() {
        let rows = vec![
            inline(DiffCellKind::Del, "let a =  1;"),
            inline(DiffCellKind::Add, "let a = 2;"),
        ];
        let ignored = diff_word_ranges(&rows, 0..rows.len(), true);
        assert_eq!(ignored.left(0), one(9, 10));
        assert_eq!(ignored.left(1), one(8, 9));
        let shown = diff_word_ranges(&rows, 0..rows.len(), false);
        assert_eq!(shown.left(0), one(7, 10));
        assert_eq!(shown.left(1), one(7, 9));
    }

    #[test]
    fn emoji_ranges_stay_on_char_boundaries() {
        let old = "status ✅ ok";
        let new = "status ❌ ok";
        let (o, n) = word_change_ranges(old, new, false).expect("ranges");
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
        assert_eq!(word_change_ranges("same line", "same line", false), None);
    }

    #[test]
    fn total_rewrite_has_no_ranges() {
        assert_eq!(word_change_ranges("alpha beta", "gamma delta", false), None);
    }

    #[test]
    fn over_char_budget_has_no_ranges() {
        let old = format!("{} a", "x".repeat(MAX_WORD_DIFF_CHARS));
        let new = format!("{} b", "x".repeat(MAX_WORD_DIFF_CHARS));
        assert_eq!(word_change_ranges(&old, &new, false), None);
    }

    #[test]
    fn over_lcs_budget_has_no_ranges() {
        // ~1000 tokens per side between a shared prefix and suffix.
        let old = format!("start {} end", "a ".repeat(500));
        let new = format!("start {} end", "b,".repeat(500));
        assert_eq!(word_change_ranges(&old, &new, false), None);
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
        let ranges = diff_word_ranges(&rows, 0..rows.len(), false);
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
        let ranges = diff_word_ranges(&rows, 0..rows.len(), false);
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
        let ranges = diff_word_ranges(&rows, 0..rows.len(), false);
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
        let ranges = diff_word_ranges(&rows, 0..rows.len(), false);
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
        let ranges = diff_word_ranges(&rows, 3..4, false);
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
        let ranges = diff_word_ranges(&rows, 1..2, false);
        assert!(ranges.left(0).is_empty());
        assert!(ranges.right(0).is_empty());
        assert_eq!(ranges.left(1), one(8, 9));
        assert_eq!(ranges.right(1), one(8, 9));
    }
}

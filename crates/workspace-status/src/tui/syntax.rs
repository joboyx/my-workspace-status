//! File-aware syntax highlighting for painted diff cells and file tabs.
//!
//! Language comes from the file path (basename, then extension), then an
//! optional first-line shebang. Unknown paths use Plain Text.
//! Highlighting sets **foreground** only. Add/del row backgrounds stay in
//! the paint layer. Changed-word runs from [`super::word_diff`] are flagged
//! here so paint can put the word background behind them.

use std::ops::Range;
use std::path::Path;
use std::sync::{Arc, OnceLock};

use ratatui::style::Color;
use two_face::re_exports::syntect::easy::HighlightLines;
use two_face::re_exports::syntect::highlighting::Color as SyntectColor;
use two_face::re_exports::syntect::parsing::{SyntaxReference, SyntaxSet};
use two_face::theme::{extra as extra_themes, EmbeddedLazyThemeSet, EmbeddedThemeName};

use super::diff::{DiffCell, DiffCellKind, DiffRow};
use super::theme::ThemeId;
use super::word_diff::diff_word_ranges;
use crate::helpers::visible_width;

/// Skip highlighting past this many characters. The rest uses the fallback
/// foreground. Stops a huge diff line from stalling a paint.
const MAX_HIGHLIGHT_CHARS: usize = 4096;

/// Lines a file-tab window feeds the highlighter above its first painted
/// line, so a block comment or string opened just above starts in the
/// right state without parsing the whole file.
pub(crate) const FILE_HIGHLIGHT_LOOKBACK: usize = 200;

/// WCAG contrast floor for syntax fg on an add/del row background.
const ROW_BG_CONTRAST_FLOOR: f64 = 3.0;

static SYNTAX_SET: OnceLock<SyntaxSet> = OnceLock::new();
static THEME_SET: OnceLock<EmbeddedLazyThemeSet> = OnceLock::new();

fn syntax_set() -> &'static SyntaxSet {
    // Diff cells have no trailing newline. `extra_newlines` syntaxes expect one.
    SYNTAX_SET.get_or_init(two_face::syntax::extra_no_newlines)
}

fn theme_set() -> &'static EmbeddedLazyThemeSet {
    THEME_SET.get_or_init(extra_themes)
}

fn syntect_theme_name(id: ThemeId) -> EmbeddedThemeName {
    match id {
        ThemeId::TokyoNight => EmbeddedThemeName::Nord,
        ThemeId::Monokai => EmbeddedThemeName::MonokaiExtended,
        ThemeId::Dracula => EmbeddedThemeName::Dracula,
        ThemeId::GruvboxDark => EmbeddedThemeName::GruvboxDark,
        ThemeId::CatppuccinMocha => EmbeddedThemeName::Base16MochaDark,
    }
}

#[cfg(test)]
/// Sublime / two-face syntax name for `path`.
///
/// `first_line` is a shebang / mode-line fallback when the path has no
/// known basename or extension. The paint path passes `None`.
pub(crate) fn language_name(path: &str, first_line: Option<&str>) -> String {
    syntax_for(path, first_line).name.clone()
}

/// True when highlighting would be a single fallback colour.
pub(crate) fn is_plain_text(path: &str, first_line: Option<&str>) -> bool {
    syntax_for(path, first_line)
        .name
        .eq_ignore_ascii_case("Plain Text")
}

fn syntax_for(path: &str, first_line: Option<&str>) -> &'static SyntaxReference {
    let set = syntax_set();
    let normalized = path.replace('\\', "/");
    let file_name = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(normalized.as_str());

    if let Some(syntax) = syntax_from_basename(set, file_name) {
        return syntax;
    }

    if let Some(ext) = Path::new(&normalized)
        .extension()
        .and_then(|ext| ext.to_str())
    {
        if let Some(syntax) = syntax_from_extension(set, ext) {
            return syntax;
        }
    }

    if let Some(line) = first_line {
        if let Some(syntax) = set.find_syntax_by_first_line(line) {
            return syntax;
        }
    }

    set.find_syntax_plain_text()
}

fn syntax_from_basename<'a>(set: &'a SyntaxSet, file_name: &str) -> Option<&'a SyntaxReference> {
    let lower = file_name.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "makefile" | "gnumakefile" | "makefile.am" => Some("Makefile"),
        "dockerfile" | "containerfile" => Some("Dockerfile"),
        "cmakelists.txt" => Some("CMake"),
        ".bashrc" | ".zshrc" | ".bash_profile" | ".profile" => Some("Bash"),
        "cargo.toml" | "pyproject.toml" => Some("TOML"),
        _ => None,
    }?;
    set.find_syntax_by_name(mapped)
        .or_else(|| set.find_syntax_by_token(mapped))
}

fn syntax_from_extension<'a>(set: &'a SyntaxSet, ext: &str) -> Option<&'a SyntaxReference> {
    let lower = ext.to_ascii_lowercase();
    if let Some(syntax) = set.find_syntax_by_extension(&lower) {
        return Some(syntax);
    }
    match lower.as_str() {
        "yml" => set
            .find_syntax_by_extension("yaml")
            .or_else(|| set.find_syntax_by_name("YAML")),
        "jsonc" | "json5" => set.find_syntax_by_extension("json"),
        "tsx" => set
            .find_syntax_by_name("TypeScriptReact")
            .or_else(|| set.find_syntax_by_name("TypeScript"))
            .or_else(|| set.find_syntax_by_extension("js")),
        "ts" | "mts" | "cts" => set
            .find_syntax_by_name("TypeScript")
            .or_else(|| set.find_syntax_by_extension("js")),
        "zsh" | "bash" | "ksh" => set
            .find_syntax_by_extension("sh")
            .or_else(|| set.find_syntax_by_name("Bash")),
        _ => None,
    }
}

#[cfg(test)]
/// Foreground spans for one source line. Never sets a background colour.
///
/// Low-contrast tokens against `row_bg` fall back to `fallback` so add/del
/// tints stay readable. Plain Text and highlight failures are one span.
pub(crate) fn highlight_spans(
    path: &str,
    line: &str,
    theme: ThemeId,
    fallback: Color,
    row_bg: Option<Color>,
) -> Vec<(String, Color)> {
    if line.is_empty() {
        return Vec::new();
    }
    let raw = highlight_range(path, line, theme, fallback);
    resolve_spans(raw, &[], fallback, row_bg, None)
        .into_iter()
        .map(|span| (span.text, span.fg))
        .collect()
}

/// One painted run of diff code text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodeSpan {
    /// Text of the run.
    pub text: String,
    /// Foreground, already checked for contrast against the background
    /// painted behind this run.
    pub fg: Color,
    /// True for changed text on a paired add/del line. Paint puts the
    /// word background behind it instead of the row background.
    pub word: bool,
}

/// Add/del backgrounds that syntax foregrounds must stay readable on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DiffBackgrounds {
    /// Add-line row background.
    pub add: Color,
    /// Del-line row background.
    pub del: Color,
    /// Changed-word background on an add line.
    pub add_word: Color,
    /// Changed-word background on a del line.
    pub del_word: Color,
}

/// Token foregrounds for each file-diff row, old-file and new-file streams.
///
/// Syntect parse state is kept across lines **inside one hunk**. Section and
/// hunk header rows start a new pair of highlighters so omitted lines cannot
/// leak into a later hunk or a staged/unstaged section. Add lines feed the
/// new stream. Del lines feed the old stream. Context feeds both on inline
/// rows, or the matching side on split rows. Spans are split at the
/// changed-word ranges from [`diff_word_ranges`].
pub(crate) struct DiffSyntaxSpans {
    left: Vec<Vec<CodeSpan>>,
    right: Vec<Vec<CodeSpan>>,
}

impl DiffSyntaxSpans {
    /// Spans of the left (or only) cell of `row`. Empty off the viewport.
    pub(crate) fn left(&self, row: usize) -> &[CodeSpan] {
        self.left.get(row).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Spans of the right cell of `row` (split rows only).
    pub(crate) fn right(&self, row: usize) -> &[CodeSpan] {
        self.right.get(row).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Identity for one paint of file-diff syntax spans.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiffSyntaxKey {
    pub path: String,
    pub theme: ThemeId,
    pub fallback: Color,
    /// Row and word backgrounds the contrast floor checks against.
    pub bgs: DiffBackgrounds,
    pub visible_start: usize,
    pub visible_end: usize,
    /// Painted diff-text identity. Must change when unified text changes.
    pub cache_id: u64,
}

/// Last computed syntax spans. Paint reuses the `Arc` when the key matches.
pub(crate) struct CachedDiffSyntax {
    key: DiffSyntaxKey,
    spans: Arc<DiffSyntaxSpans>,
}

/// Highlight `visible` line rows. Off-viewport rows keep empty span lists.
///
/// Each hunk/section is an independent parse stream. Lines above the
/// viewport in the same hunk still feed the highlighter so multiline
/// tokens stay correct, but those rows do not store spans. Visible paired
/// add/del cells get their changed-word runs flagged.
pub(crate) fn highlight_diff_rows(
    path: &str,
    rows: &[DiffRow],
    theme: ThemeId,
    fallback: Color,
    bgs: DiffBackgrounds,
    visible: Range<usize>,
) -> DiffSyntaxSpans {
    let n = rows.len();
    let visible = visible.start.min(n)..visible.end.min(n);
    let mut left = vec![Vec::new(); n];
    let mut right = vec![Vec::new(); n];
    if visible.is_empty() {
        return DiffSyntaxSpans { left, right };
    }
    let words = diff_word_ranges(rows, visible.clone());
    let finish = |cell: &DiffCell, raw: Vec<(String, Color)>, ranges: &[Range<usize>]| {
        resolve_spans(
            raw,
            ranges,
            fallback,
            cell_bg(cell.kind, bgs),
            word_bg(cell.kind, bgs),
        )
    };
    if is_plain_text(path, None) {
        for idx in visible {
            let Some(DiffRow::Line {
                left: cell,
                right: other,
            }) = rows.get(idx)
            else {
                continue;
            };
            left[idx] = finish(cell, plain_cell_spans(cell, fallback), words.left(idx));
            if let Some(other) = other {
                right[idx] = finish(other, plain_cell_spans(other, fallback), words.right(idx));
            }
        }
        return DiffSyntaxSpans { left, right };
    }
    let syntax = syntax_for(path, None);
    let syn_theme = theme_set().get(syntect_theme_name(theme));
    let mut i = 0;
    while i < n {
        match &rows[i] {
            DiffRow::Line { .. } => {
                let start = i;
                while i < n && matches!(rows[i], DiffRow::Line { .. }) {
                    i += 1;
                }
                let end = i;
                if end <= visible.start || start >= visible.end {
                    continue;
                }
                let feed_end = end.min(visible.end);
                let mut old_hl = HighlightLines::new(syntax, syn_theme);
                let mut new_hl = HighlightLines::new(syntax, syn_theme);
                for idx in start..feed_end {
                    let DiffRow::Line {
                        left: cell,
                        right: other,
                    } = &rows[idx]
                    else {
                        continue;
                    };
                    let store = idx >= visible.start && idx < visible.end;
                    if let Some(other) = other {
                        let left_raw =
                            feed_split_cell(cell, false, &mut old_hl, &mut new_hl, fallback);
                        let right_raw =
                            feed_split_cell(other, true, &mut old_hl, &mut new_hl, fallback);
                        if store {
                            left[idx] = finish(cell, left_raw, words.left(idx));
                            right[idx] = finish(other, right_raw, words.right(idx));
                        }
                    } else {
                        let raw = feed_inline_cell(cell, &mut old_hl, &mut new_hl, fallback);
                        if store {
                            left[idx] = finish(cell, raw, words.left(idx));
                        }
                    }
                }
            }
            DiffRow::Section(_) | DiffRow::Hunk { .. } | DiffRow::Error { .. } => i += 1,
        }
    }
    DiffSyntaxSpans { left, right }
}

/// Token spans for the file-tab lines in `window`, one list per line.
///
/// One parse stream starts [`FILE_HIGHLIGHT_LOOKBACK`] lines above the
/// window and runs to its end; only the window's spans are kept. Text past
/// [`MAX_HIGHLIGHT_CHARS`] on a line uses `fallback`, as in diffs. The
/// language comes from `path`, then the file's first line (shebang).
pub(crate) fn highlight_file_window(
    path: &str,
    lines: &[String],
    theme: ThemeId,
    fallback: Color,
    window: Range<usize>,
) -> Vec<Vec<CodeSpan>> {
    let end = window.end.min(lines.len());
    let start = window.start.min(end);
    let first = lines.first().map(String::as_str);
    let finish = |raw| resolve_spans(raw, &[], fallback, None, None);
    if is_plain_text(path, first) {
        return lines[start..end]
            .iter()
            .map(|line| {
                if line.is_empty() {
                    Vec::new()
                } else {
                    finish(vec![(line.clone(), fallback)])
                }
            })
            .collect();
    }
    let syntax = syntax_for(path, first);
    let syn_theme = theme_set().get(syntect_theme_name(theme));
    let mut highlighter = HighlightLines::new(syntax, syn_theme);
    let mut out = Vec::with_capacity(end - start);
    for (idx, line) in lines
        .iter()
        .enumerate()
        .take(end)
        .skip(start.saturating_sub(FILE_HIGHLIGHT_LOOKBACK))
    {
        let raw = feed_line(&mut highlighter, line, fallback);
        if idx >= start {
            out.push(finish(raw));
        }
    }
    out
}

/// Reuse `cache` when `key` matches. Misses call [`highlight_diff_rows`]
/// with the path, theme, colours, and viewport from `key`.
pub(crate) fn cached_highlight_diff_rows(
    cache: &mut Option<CachedDiffSyntax>,
    key: DiffSyntaxKey,
    rows: &[DiffRow],
) -> Arc<DiffSyntaxSpans> {
    if let Some(hit) = cache.as_ref() {
        if hit.key == key {
            return Arc::clone(&hit.spans);
        }
    }
    let spans = Arc::new(highlight_diff_rows(
        &key.path,
        rows,
        key.theme,
        key.fallback,
        key.bgs,
        key.visible_start..key.visible_end,
    ));
    *cache = Some(CachedDiffSyntax {
        key,
        spans: Arc::clone(&spans),
    });
    spans
}

fn cell_bg(kind: DiffCellKind, bgs: DiffBackgrounds) -> Option<Color> {
    match kind {
        DiffCellKind::Add => Some(bgs.add),
        DiffCellKind::Del => Some(bgs.del),
        DiffCellKind::Ctx | DiffCellKind::Meta | DiffCellKind::Empty => None,
    }
}

fn word_bg(kind: DiffCellKind, bgs: DiffBackgrounds) -> Option<Color> {
    match kind {
        DiffCellKind::Add => Some(bgs.add_word),
        DiffCellKind::Del => Some(bgs.del_word),
        DiffCellKind::Ctx | DiffCellKind::Meta | DiffCellKind::Empty => None,
    }
}

/// Raw Plain Text spans for one cell: the whole text in `fallback`.
fn plain_cell_spans(cell: &DiffCell, fallback: Color) -> Vec<(String, Color)> {
    match cell.kind {
        DiffCellKind::Empty | DiffCellKind::Meta => Vec::new(),
        _ => vec![(cell.text.clone(), fallback)],
    }
}

/// Split raw `(text, syntect fg)` spans at the word `ranges` (byte ranges
/// into the joined text) and apply the contrast floor per run: word runs
/// against `word_bg`, the rest against `row_bg`. Neighbours with the same
/// fg and flag merge. Without a `word_bg` nothing is flagged.
fn resolve_spans(
    raw: Vec<(String, Color)>,
    ranges: &[Range<usize>],
    fallback: Color,
    row_bg: Option<Color>,
    word_bg: Option<Color>,
) -> Vec<CodeSpan> {
    let ranges = if word_bg.is_some() { ranges } else { &[] };
    let mut out: Vec<CodeSpan> = Vec::new();
    let mut pos = 0usize;
    for (text, raw_fg) in raw {
        let end = pos + text.len();
        let mut at = pos;
        while at < end {
            let (word, next) = word_segment(ranges, at, end);
            let piece = &text[at - pos..next - pos];
            let fg = readable_fg(raw_fg, if word { word_bg } else { row_bg }, fallback);
            match out.last_mut() {
                Some(last) if last.fg == fg && last.word == word => last.text.push_str(piece),
                _ => out.push(CodeSpan {
                    text: piece.to_string(),
                    fg,
                    word,
                }),
            }
            at = next;
        }
        pos = end;
    }
    out
}

/// Whether byte `at` is inside a word range, and where that run ends
/// (capped at `end`). `ranges` are sorted and disjoint.
fn word_segment(ranges: &[Range<usize>], at: usize, end: usize) -> (bool, usize) {
    for range in ranges {
        if range.end <= at {
            continue;
        }
        if range.start <= at {
            return (true, range.end.min(end));
        }
        return (false, range.start.min(end));
    }
    (false, end)
}

/// Raw spans for an inline cell. Context feeds both streams.
fn feed_inline_cell(
    cell: &DiffCell,
    old_hl: &mut HighlightLines<'_>,
    new_hl: &mut HighlightLines<'_>,
    fallback: Color,
) -> Vec<(String, Color)> {
    match cell.kind {
        DiffCellKind::Empty | DiffCellKind::Meta => Vec::new(),
        DiffCellKind::Add => feed_line(new_hl, &cell.text, fallback),
        DiffCellKind::Del => feed_line(old_hl, &cell.text, fallback),
        DiffCellKind::Ctx => {
            let spans = feed_line(new_hl, &cell.text, fallback);
            let _ = feed_line(old_hl, &cell.text, fallback);
            spans
        }
    }
}

/// Raw spans for one side of a split row.
fn feed_split_cell(
    cell: &DiffCell,
    is_right: bool,
    old_hl: &mut HighlightLines<'_>,
    new_hl: &mut HighlightLines<'_>,
    fallback: Color,
) -> Vec<(String, Color)> {
    let use_new = if is_right {
        matches!(cell.kind, DiffCellKind::Add | DiffCellKind::Ctx)
    } else {
        matches!(cell.kind, DiffCellKind::Add)
    };
    match cell.kind {
        DiffCellKind::Empty | DiffCellKind::Meta => Vec::new(),
        _ if use_new => feed_line(new_hl, &cell.text, fallback),
        _ => feed_line(old_hl, &cell.text, fallback),
    }
}

fn split_highlight_budget(line: &str) -> (&str, &str) {
    if line.chars().count() <= MAX_HIGHLIGHT_CHARS {
        return (line, "");
    }
    let mut end = 0;
    for (count, (idx, ch)) in line.char_indices().enumerate() {
        if count == MAX_HIGHLIGHT_CHARS {
            end = idx;
            break;
        }
        end = idx + ch.len_utf8();
    }
    (&line[..end], &line[end..])
}

#[cfg(test)]
fn highlight_range(
    path: &str,
    line: &str,
    theme: ThemeId,
    fallback: Color,
) -> Vec<(String, Color)> {
    if line.is_empty() || is_plain_text(path, None) {
        return vec![(line.to_string(), fallback)];
    }
    let syntax = syntax_for(path, None);
    let syn_theme = theme_set().get(syntect_theme_name(theme));
    let mut highlighter = HighlightLines::new(syntax, syn_theme);
    feed_line(&mut highlighter, line, fallback)
}

#[cfg(test)]
thread_local! {
    static FED_LINES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_syntax_fed_lines() -> usize {
    FED_LINES.with(|count| count.replace(0))
}

/// Raw `(text, syntect fg)` spans for one line. Text past
/// [`MAX_HIGHLIGHT_CHARS`] uses `fallback`. The contrast floor is applied
/// later by [`resolve_spans`], once the background behind each run is known.
fn feed_line(
    highlighter: &mut HighlightLines<'_>,
    line: &str,
    fallback: Color,
) -> Vec<(String, Color)> {
    if line.is_empty() {
        return Vec::new();
    }
    #[cfg(test)]
    FED_LINES.with(|count| count.set(count.get() + 1));
    let (head, tail) = split_highlight_budget(line);
    let mut out = feed_head(highlighter, head, fallback);
    if !tail.is_empty() {
        push_raw(&mut out, tail, fallback);
    }
    if out.is_empty() {
        vec![(line.to_string(), fallback)]
    } else {
        out
    }
}

fn feed_head(
    highlighter: &mut HighlightLines<'_>,
    line: &str,
    fallback: Color,
) -> Vec<(String, Color)> {
    if line.is_empty() {
        return Vec::new();
    }
    let set = syntax_set();
    let Ok(ranges) = highlighter.highlight_line(line, set) else {
        return vec![(line.to_string(), fallback)];
    };
    let mut out: Vec<(String, Color)> = Vec::new();
    for (style, text) in ranges {
        if text.is_empty() {
            continue;
        }
        let fg = syntect_fg(style.foreground).map_or(fallback, |(r, g, b)| Color::Rgb(r, g, b));
        push_raw(&mut out, text, fg);
    }
    out
}

/// Append `text` in `fg`, merging into the last span when the fg matches.
fn push_raw(out: &mut Vec<(String, Color)>, text: &str, fg: Color) {
    match out.last_mut() {
        Some(last) if last.1 == fg => last.0.push_str(text),
        _ => out.push((text.to_string(), fg)),
    }
}

fn syntect_fg(color: SyntectColor) -> Option<(u8, u8, u8)> {
    if color.a == 0 {
        return None;
    }
    Some((color.r, color.g, color.b))
}

/// `fg` when it meets the 3:1 contrast floor on `row_bg`, else `fallback`.
/// No `row_bg` keeps `fg`.
pub(crate) fn readable_fg(fg: Color, row_bg: Option<Color>, fallback: Color) -> Color {
    let Some(bg) = row_bg else {
        return fg;
    };
    if contrast_ratio(fg, bg) >= ROW_BG_CONTRAST_FLOOR {
        fg
    } else {
        fallback
    }
}

fn srgb_lin(c: u8) -> f64 {
    let c = f64::from(c) / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn relative_luminance(color: Color) -> Option<f64> {
    let Color::Rgb(r, g, b) = color else {
        return None;
    };
    Some(0.2126 * srgb_lin(r) + 0.7152 * srgb_lin(g) + 0.0722 * srgb_lin(b))
}

fn contrast_ratio(fg: Color, bg: Color) -> f64 {
    let (Some(l1), Some(l2)) = (relative_luminance(fg), relative_luminance(bg)) else {
        return ROW_BG_CONTRAST_FLOOR;
    };
    let (lighter, darker) = if l1 > l2 { (l1, l2) } else { (l2, l1) };
    (lighter + 0.05) / (darker + 0.05)
}

/// Skip `offset` display columns, then take up to `width` columns.
///
/// Never splits a char. Neighbours merge only when fg and word flag match.
pub(crate) fn slice_styled_cols(parts: &[CodeSpan], offset: usize, width: usize) -> Vec<CodeSpan> {
    if width == 0 {
        return Vec::new();
    }
    let mut skipped = 0usize;
    let mut taken = 0usize;
    let mut out: Vec<CodeSpan> = Vec::new();
    for part in parts {
        for ch in part.text.chars() {
            let mut buf = [0u8; 4];
            let s = ch.encode_utf8(&mut buf);
            let cw = visible_width(s);
            if skipped < offset {
                skipped = skipped.saturating_add(cw);
                continue;
            }
            if taken.saturating_add(cw) > width {
                return out;
            }
            taken = taken.saturating_add(cw);
            match out.last_mut() {
                Some(last) if last.fg == part.fg && last.word == part.word => last.text.push(ch),
                _ => out.push(CodeSpan {
                    text: ch.to_string(),
                    fg: part.fg,
                    word: part.word,
                }),
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::diff::{DiffCell, DiffCellKind, DiffRow, DiffSection};
    use std::ops::Range;
    use std::sync::Arc;

    const FALLBACK: Color = Color::Rgb(0xc0, 0xca, 0xf5);
    const ADD_BG: Color = Color::Rgb(0x3f, 0x4d, 0x39);
    const DEL_BG: Color = Color::Rgb(0x58, 0x34, 0x43);
    const BGS: DiffBackgrounds = DiffBackgrounds {
        add: ADD_BG,
        del: DEL_BG,
        add_word: Color::Rgb(0x42, 0x68, 0x32),
        del_word: Color::Rgb(0x81, 0x3d, 0x59),
    };

    /// `(text, fg)` pairs, to compare with [`highlight_spans`].
    fn tuples(spans: &[CodeSpan]) -> Vec<(String, Color)> {
        spans.iter().map(|s| (s.text.clone(), s.fg)).collect()
    }

    fn line_row(kind: DiffCellKind, text: &str, line_no: u32) -> DiffRow {
        DiffRow::Line {
            left: DiffCell {
                kind,
                text: text.into(),
                line_no: Some(line_no),
            },
            right: None,
        }
    }

    fn highlight_all(path: &str, rows: &[DiffRow]) -> DiffSyntaxSpans {
        highlight_diff_rows(
            path,
            rows,
            ThemeId::TokyoNight,
            FALLBACK,
            BGS,
            0..rows.len(),
        )
    }

    fn lang(path: &str) -> String {
        language_name(path, None)
    }

    fn contains_ci(name: &str, needle: &str) -> bool {
        name.to_ascii_lowercase()
            .contains(&needle.to_ascii_lowercase())
    }

    #[test]
    fn language_from_common_extensions() {
        assert!(contains_ci(&lang("app/pack.json"), "json"));
        assert!(contains_ci(&lang("deploy.yaml"), "yaml"));
        assert!(contains_ci(&lang("deploy.yml"), "yaml"));
        assert!(contains_ci(&lang("run.sh"), "bash") || contains_ci(&lang("run.sh"), "shell"));
        assert!(contains_ci(&lang("run.bash"), "bash") || contains_ci(&lang("run.bash"), "shell"));
        assert!(contains_ci(&lang("src/lib.rs"), "rust"));
        assert!(
            contains_ci(&lang("src/auth.ts"), "typescript")
                || contains_ci(&lang("src/auth.ts"), "javascript")
        );
        assert_eq!(lang("notes.unknownext"), "Plain Text");
        assert_eq!(lang("no-extension"), "Plain Text");
    }

    #[test]
    fn language_from_basename_and_shebang() {
        assert!(contains_ci(&lang("Makefile"), "makefile"));
        assert!(contains_ci(&lang("Dockerfile"), "docker"));
        let shell = language_name("bin/run", Some("#!/bin/bash"));
        assert!(
            contains_ci(&shell, "bash") || contains_ci(&shell, "shell"),
            "shebang should select a shell syntax, got {shell}"
        );
    }

    #[test]
    fn json_yaml_shell_use_more_than_one_foreground() {
        let json = highlight_spans(
            "pack.json",
            r#"{ "ttlMs": 2000, "name": "alpha" }"#,
            ThemeId::TokyoNight,
            Color::Rgb(0xc0, 0xca, 0xf5),
            None,
        );
        let yaml = highlight_spans(
            "deploy.yml",
            "kind: Service",
            ThemeId::TokyoNight,
            Color::Rgb(0xc0, 0xca, 0xf5),
            None,
        );
        let shell = highlight_spans(
            "run.sh",
            r#"echo "syntax-shell""#,
            ThemeId::TokyoNight,
            Color::Rgb(0xc0, 0xca, 0xf5),
            None,
        );
        for (label, spans) in [("json", &json), ("yaml", &yaml), ("shell", &shell)] {
            let colors: std::collections::HashSet<_> = spans.iter().map(|(_, c)| *c).collect();
            assert!(
                colors.len() >= 2,
                "{label} should paint more than one token colour: {spans:?}"
            );
        }
    }

    #[test]
    fn json_hunk_stream_keeps_token_colours_on_property_line() {
        let rows = vec![
            DiffRow::Line {
                left: DiffCell {
                    kind: DiffCellKind::Ctx,
                    text: " {".into(),
                    line_no: Some(1),
                },
                right: None,
            },
            DiffRow::Line {
                left: DiffCell {
                    kind: DiffCellKind::Add,
                    text: r#"  "name": "alpha-syntax""#.into(),
                    line_no: Some(2),
                },
                right: None,
            },
        ];
        let spans = highlight_all("pack.json", &rows);
        let colors: std::collections::HashSet<_> = spans.left(1).iter().map(|s| s.fg).collect();
        assert!(
            colors.len() >= 2,
            "property line after '{{' should keep JSON token colours: {colors:?} {:?}",
            spans.left(1)
        );
    }

    #[test]
    fn highlight_is_foreground_only_and_plain_is_fallback() {
        let fallback = Color::Rgb(0xc0, 0xca, 0xf5);
        let spans = highlight_spans(
            "notes.txt",
            "hello world",
            ThemeId::TokyoNight,
            fallback,
            None,
        );
        assert_eq!(spans, vec![("hello world".into(), fallback)]);
    }

    #[test]
    fn low_contrast_token_falls_back_on_row_background() {
        let fallback = Color::Rgb(0xc0, 0xca, 0xf5);
        let row_bg = Color::Rgb(0x3f, 0x4d, 0x39);
        let spans = highlight_spans(
            "pack.json",
            r#"{ "ttlMs": 2000 }"#,
            ThemeId::TokyoNight,
            fallback,
            Some(row_bg),
        );
        for (_, fg) in &spans {
            assert!(
                contrast_ratio(*fg, row_bg) >= ROW_BG_CONTRAST_FLOOR,
                "syntax fg {fg:?} must stay readable on add bg {row_bg:?}"
            );
        }
    }

    /// Text of the runs flagged `word`, in order.
    fn word_texts(spans: &[CodeSpan]) -> Vec<String> {
        spans
            .iter()
            .filter(|s| s.word)
            .map(|s| s.text.clone())
            .collect()
    }

    fn modified_pair() -> Vec<DiffRow> {
        vec![
            DiffRow::Hunk {
                text: "@@ -1,1 +1,1 @@".into(),
            },
            line_row(DiffCellKind::Del, "let total = price * qty;", 1),
            line_row(DiffCellKind::Add, "let total = price * count;", 1),
            line_row(DiffCellKind::Add, "let extra = 1;", 2),
        ]
    }

    #[test]
    fn paired_lines_flag_changed_words_only() {
        for path in ["calc.rs", "notes.txt"] {
            let spans = highlight_all(path, &modified_pair());
            assert_eq!(word_texts(spans.left(1)), vec!["qty"], "{path}");
            assert_eq!(word_texts(spans.left(2)), vec!["count"], "{path}");
            assert!(word_texts(spans.left(3)).is_empty(), "{path} pure add");
            let joined: String = spans.left(2).iter().map(|s| s.text.as_str()).collect();
            assert_eq!(joined, "let total = price * count;", "{path}");
        }
    }

    #[test]
    fn split_row_flags_both_sides() {
        let cell = |kind, text: &str| DiffCell {
            kind,
            text: text.into(),
            line_no: Some(1),
        };
        let rows = vec![DiffRow::Line {
            left: cell(DiffCellKind::Del, "let total = price * qty;"),
            right: Some(cell(DiffCellKind::Add, "let total = price * count;")),
        }];
        let spans = highlight_all("calc.rs", &rows);
        assert_eq!(word_texts(spans.left(0)), vec!["qty"]);
        assert_eq!(word_texts(spans.right(0)), vec!["count"]);
    }

    #[test]
    fn word_run_fg_is_readable_on_the_word_background() {
        let rows = vec![
            line_row(DiffCellKind::Del, r#"  "ttlMs": 5000, "on": true"#, 1),
            line_row(DiffCellKind::Add, r#"  "ttlMs": 2000, "on": false"#, 1),
        ];
        let spans = highlight_all("pack.json", &rows);
        for (row, row_bg, word_bg) in [(0, DEL_BG, BGS.del_word), (1, ADD_BG, BGS.add_word)] {
            let line = spans.left(row);
            assert!(line.iter().any(|s| s.word), "row {row}: {line:?}");
            for span in line {
                let bg = if span.word { word_bg } else { row_bg };
                assert!(
                    span.fg == FALLBACK || contrast_ratio(span.fg, bg) >= ROW_BG_CONTRAST_FLOOR,
                    "row {row} {span:?} must stay readable on {bg:?}"
                );
            }
        }
    }

    #[test]
    fn contrast_floor_uses_the_background_behind_each_run() {
        // Readable on the row bg, unreadable on the word bg: only the word
        // run falls back.
        let fg = Color::Rgb(0xff, 0xff, 0xff);
        let row_bg = Color::Rgb(0x00, 0x00, 0x00);
        let word_bg = Color::Rgb(0xf0, 0xf0, 0xf0);
        let spans = resolve_spans(
            vec![("ab cd".into(), fg)],
            std::slice::from_ref(&(3..5)),
            FALLBACK,
            Some(row_bg),
            Some(word_bg),
        );
        let expected = vec![
            CodeSpan {
                text: "ab ".into(),
                fg,
                word: false,
            },
            CodeSpan {
                text: "cd".into(),
                fg: FALLBACK,
                word: true,
            },
        ];
        assert_eq!(spans, expected);
    }

    #[test]
    fn word_run_keeps_raw_fg_that_only_the_word_background_can_carry() {
        // Unreadable on the row bg, readable on the word bg: the row run
        // falls back, the word run keeps the raw syntect fg.
        let fg = Color::Rgb(0x10, 0x10, 0x10);
        let row_bg = Color::Rgb(0x00, 0x00, 0x00);
        let word_bg = Color::Rgb(0xf0, 0xf0, 0xf0);
        assert!(contrast_ratio(fg, row_bg) < ROW_BG_CONTRAST_FLOOR);
        assert!(contrast_ratio(fg, word_bg) >= ROW_BG_CONTRAST_FLOOR);
        let spans = resolve_spans(
            vec![("ab cd".into(), fg)],
            std::slice::from_ref(&(3..5)),
            FALLBACK,
            Some(row_bg),
            Some(word_bg),
        );
        let expected = vec![
            CodeSpan {
                text: "ab ".into(),
                fg: FALLBACK,
                word: false,
            },
            CodeSpan {
                text: "cd".into(),
                fg,
                word: true,
            },
        ];
        assert_eq!(spans, expected);
    }

    #[test]
    fn file_window_keeps_lookback_state_and_only_window_spans() {
        let mut lines: Vec<String> = vec!["/* opened above".into()];
        lines.extend((0..10).map(|i| format!("still comment {i}")));
        lines.push("*/".into());
        lines.push("fn main() {}".into());
        let window = highlight_file_window("a.rs", &lines, ThemeId::TokyoNight, FALLBACK, 5..13);
        assert_eq!(window.len(), 8);
        let comment_fg = window[0][0].fg;
        let alone =
            highlight_file_window("a.rs", &lines[5..6], ThemeId::TokyoNight, FALLBACK, 0..1);
        assert_ne!(alone[0][0].fg, comment_fg, "look-back sets comment state");
        let code = &window[7];
        assert_eq!(
            code.iter()
                .map(|span| span.text.as_str())
                .collect::<String>(),
            "fn main() {}"
        );
        assert!(code.len() > 1, "code after the comment is tokenised");

        let plain = highlight_file_window(
            "notes.unknownext",
            &lines,
            ThemeId::TokyoNight,
            FALLBACK,
            0..2,
        );
        assert_eq!(plain.len(), 2);
        assert_eq!(plain[0][0].fg, FALLBACK);
        assert!(
            highlight_file_window("a.rs", &lines, ThemeId::TokyoNight, FALLBACK, 40..50).is_empty()
        );
    }

    #[test]
    fn slice_styled_cols_keeps_word_flag_and_splits_on_it() {
        let span = |text: &str, word| CodeSpan {
            text: text.into(),
            fg: Color::Red,
            word,
        };
        let parts = vec![span("a😀", false), span("bc", true), span("d", false)];
        assert_eq!(
            slice_styled_cols(&parts, 1, 4),
            vec![span("😀", false), span("bc", true)]
        );
        assert_eq!(
            slice_styled_cols(&parts, 3, 9),
            vec![span("bc", true), span("d", false)]
        );
    }

    #[test]
    fn slice_styled_cols_keeps_emoji_display_width() {
        let emoji = '😀';
        assert_eq!(visible_width(&emoji.to_string()), 2);
        let span = |text: &str, fg: Color| CodeSpan {
            text: text.into(),
            fg,
            word: false,
        };
        let parts = vec![
            span("A", Color::Red),
            span(&emoji.to_string(), Color::Green),
            span("B", Color::Blue),
        ];
        let sliced = slice_styled_cols(&parts, 1, 2);
        assert_eq!(sliced, vec![span(&emoji.to_string(), Color::Green)]);
        let clipped = slice_styled_cols(&parts, 0, 2);
        assert_eq!(clipped, vec![span("A", Color::Red)]);
    }

    fn syntax_key(visible: Range<usize>, cache_id: u64) -> DiffSyntaxKey {
        DiffSyntaxKey {
            path: "pack.json".into(),
            theme: ThemeId::TokyoNight,
            fallback: FALLBACK,
            bgs: BGS,
            visible_start: visible.start,
            visible_end: visible.end,
            cache_id,
        }
    }

    #[test]
    fn viewport_skips_offscreen_hunks_and_stores_no_spans() {
        let mut rows = vec![DiffRow::Hunk {
            text: "@@ -1,40 +1,40 @@".into(),
        }];
        for i in 0..40 {
            rows.push(line_row(DiffCellKind::Add, r#"  "keep": true"#, i + 1));
        }
        rows.push(DiffRow::Hunk {
            text: "@@ -80,40 +80,40 @@".into(),
        });
        for i in 0..40 {
            rows.push(line_row(DiffCellKind::Add, r#"  "later": false"#, 80 + i));
        }
        let _ = take_syntax_fed_lines();
        let visible = 0..3;
        let spans = highlight_diff_rows(
            "pack.json",
            &rows,
            ThemeId::TokyoNight,
            FALLBACK,
            BGS,
            visible.clone(),
        );
        let fed = take_syntax_fed_lines();
        assert!(
            fed > 0 && fed < 10,
            "only the visible hunk prefix should feed syntect, got {fed}"
        );
        assert!(!spans.left(1).is_empty(), "visible add line stores spans");
        assert!(
            spans.left(20).is_empty(),
            "off-viewport line in the same hunk must not store spans"
        );
        assert!(
            spans.left(50).is_empty(),
            "later hunk outside the viewport must not store spans"
        );
        assert!(
            spans.left(42).is_empty(),
            "second hunk header has no syntax spans"
        );
    }

    #[test]
    fn cached_highlight_skips_syntect_on_unchanged_rows() {
        let rows = vec![
            DiffRow::Hunk {
                text: "@@ -1,2 +1,2 @@".into(),
            },
            line_row(DiffCellKind::Ctx, " {", 1),
            line_row(DiffCellKind::Add, r#"  "name": "alpha-syntax""#, 2),
        ];
        let visible = 0..rows.len();
        let key = syntax_key(visible.clone(), 7);
        let mut cache = None;
        let _ = take_syntax_fed_lines();
        let first = cached_highlight_diff_rows(&mut cache, key.clone(), &rows);
        let fed_first = take_syntax_fed_lines();
        let second = cached_highlight_diff_rows(&mut cache, key, &rows);
        let fed_second = take_syntax_fed_lines();
        assert!(fed_first > 0, "first paint feeds syntect");
        assert_eq!(fed_second, 0, "unchanged rows must not re-feed syntect");
        assert!(
            Arc::ptr_eq(&first, &second),
            "cache hit returns the same spans allocation"
        );
    }

    #[test]
    fn cached_highlight_misses_when_cache_id_changes_for_same_size_rows() {
        let alpha = vec![
            DiffRow::Hunk {
                text: "@@ -1,2 +1,2 @@".into(),
            },
            line_row(DiffCellKind::Ctx, " {", 1),
            line_row(DiffCellKind::Add, r#"  "name": "alpha-syntax""#, 2),
        ];
        let omega = vec![
            DiffRow::Hunk {
                text: "@@ -1,2 +1,2 @@".into(),
            },
            line_row(DiffCellKind::Ctx, " {", 1),
            line_row(DiffCellKind::Add, r#"  "name": "omega-syntax""#, 2),
        ];
        assert_eq!(alpha.len(), omega.len());
        let visible = 0..alpha.len();
        let mut cache = None;
        let first = cached_highlight_diff_rows(&mut cache, syntax_key(visible.clone(), 1), &alpha);
        let second = cached_highlight_diff_rows(&mut cache, syntax_key(visible.clone(), 2), &omega);
        assert!(
            !Arc::ptr_eq(&first, &second),
            "a new cache_id must not reuse the previous span list"
        );
        let omega_text: String = second
            .left(2)
            .iter()
            .map(|span| span.text.as_str())
            .collect();
        assert!(
            omega_text.contains("omega-syntax"),
            "second paint must store the replacement text: {omega_text:?}"
        );
        assert!(
            !omega_text.contains("alpha-syntax"),
            "second paint must not keep the prior cached text: {omega_text:?}"
        );
        let omega_fgs: std::collections::HashSet<_> =
            second.left(2).iter().map(|span| span.fg).collect();
        assert!(
            omega_fgs.len() >= 2,
            "replacement JSON must still get token colours: {omega_fgs:?}"
        );
    }

    #[test]
    fn cached_word_runs_reuse_on_same_key_and_refresh_on_new_cache_id() {
        let rows_with = |new: &str| {
            vec![
                line_row(DiffCellKind::Del, r#"  "ttlMs": 5000"#, 1),
                line_row(DiffCellKind::Add, new, 1),
            ]
        };
        let alpha = rows_with(r#"  "ttlMs": 2000"#);
        let omega = rows_with(r#"  "maxMs": 5000"#);
        let visible = 0..alpha.len();
        let mut cache = None;
        let first = cached_highlight_diff_rows(&mut cache, syntax_key(visible.clone(), 1), &alpha);
        assert_eq!(word_texts(first.left(0)), vec!["5000"]);
        assert_eq!(word_texts(first.left(1)), vec!["2000"]);
        let _ = take_syntax_fed_lines();
        let again = cached_highlight_diff_rows(&mut cache, syntax_key(visible.clone(), 1), &alpha);
        assert_eq!(take_syntax_fed_lines(), 0, "same key must not recompute");
        assert!(Arc::ptr_eq(&first, &again));
        let reloaded = cached_highlight_diff_rows(&mut cache, syntax_key(visible, 2), &omega);
        assert!(!Arc::ptr_eq(&first, &reloaded));
        assert_eq!(word_texts(reloaded.left(0)), vec!["ttlMs"]);
        assert_eq!(word_texts(reloaded.left(1)), vec!["maxMs"]);
    }

    #[test]
    fn json_hunk_boundary_resets_parse_state() {
        let within = vec![
            line_row(DiffCellKind::Ctx, " {", 1),
            line_row(DiffCellKind::Add, r#"  "name": "alpha-syntax""#, 2),
        ];
        let across = vec![
            DiffRow::Hunk {
                text: "@@ -1,1 +1,1 @@".into(),
            },
            line_row(DiffCellKind::Ctx, " {", 1),
            DiffRow::Hunk {
                text: "@@ -20,1 +20,1 @@".into(),
            },
            line_row(DiffCellKind::Add, r#"  "name": "alpha-syntax""#, 20),
        ];
        let within_spans = highlight_all("pack.json", &within);
        let across_spans = highlight_all("pack.json", &across);
        let fresh = highlight_spans(
            "pack.json",
            r#"  "name": "alpha-syntax""#,
            ThemeId::TokyoNight,
            FALLBACK,
            Some(ADD_BG),
        );
        assert!(
            within_spans
                .left(1)
                .iter()
                .map(|s| s.fg)
                .collect::<std::collections::HashSet<_>>()
                .len()
                >= 2,
            "one hunk still carries parse state: {:?}",
            within_spans.left(1)
        );
        assert_eq!(
            tuples(across_spans.left(3)),
            fresh,
            "a later hunk must match a fresh highlighter, not the prior hunk stream"
        );
        assert_ne!(
            across_spans.left(3),
            within_spans.left(1),
            "omitted hunk gap must not keep the open-object parse state"
        );
    }

    #[test]
    fn json_section_boundary_resets_parse_state() {
        let rows = vec![
            DiffRow::Section(DiffSection::Staged),
            DiffRow::Hunk {
                text: "@@ -1,1 +1,1 @@".into(),
            },
            line_row(DiffCellKind::Ctx, " {", 1),
            DiffRow::Section(DiffSection::Unstaged),
            DiffRow::Hunk {
                text: "@@ -1,1 +1,1 @@".into(),
            },
            line_row(DiffCellKind::Add, r#"  "name": "alpha-syntax""#, 1),
        ];
        let spans = highlight_all("pack.json", &rows);
        let fresh = highlight_spans(
            "pack.json",
            r#"  "name": "alpha-syntax""#,
            ThemeId::TokyoNight,
            FALLBACK,
            Some(ADD_BG),
        );
        assert_eq!(
            tuples(spans.left(5)),
            fresh,
            "unstaged section must not inherit staged parse state"
        );
    }

    #[test]
    fn json_split_hunk_boundary_resets_old_stream() {
        let open = DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Del,
                text: " {".into(),
                line_no: Some(1),
            },
            right: Some(DiffCell {
                kind: DiffCellKind::Empty,
                text: String::new(),
                line_no: None,
            }),
        };
        let later = DiffRow::Line {
            left: DiffCell {
                kind: DiffCellKind::Del,
                text: r#"  "name": "alpha-syntax""#.into(),
                line_no: Some(20),
            },
            right: Some(DiffCell {
                kind: DiffCellKind::Empty,
                text: String::new(),
                line_no: None,
            }),
        };
        let within = vec![open.clone(), later.clone()];
        let across = vec![
            DiffRow::Hunk {
                text: "@@ -1,1 +1,0 @@".into(),
            },
            open,
            DiffRow::Hunk {
                text: "@@ -20,1 +20,0 @@".into(),
            },
            later,
        ];
        let within_spans = highlight_all("pack.json", &within);
        let across_spans = highlight_all("pack.json", &across);
        let fresh = highlight_spans(
            "pack.json",
            r#"  "name": "alpha-syntax""#,
            ThemeId::TokyoNight,
            FALLBACK,
            Some(DEL_BG),
        );
        assert_eq!(tuples(across_spans.left(3)), fresh);
        assert_ne!(across_spans.left(3), within_spans.left(1));
    }
}

//! File-aware syntax highlighting for painted diff cells.
//!
//! Language comes from the file path (basename, then extension), then an
//! optional first-line shebang. Unknown paths use Plain Text.
//! Highlighting sets **foreground** only. Add/del row backgrounds stay in
//! the paint layer. Intra-line word diff is separate and still out of scope.

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
use crate::helpers::visible_width;

/// Skip highlighting past this many characters. The rest uses the fallback
/// foreground. Stops a huge diff line from stalling a paint.
const MAX_HIGHLIGHT_CHARS: usize = 4096;

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
    let (head, tail) = split_highlight_budget(line);
    let mut spans = highlight_range(path, head, theme, fallback, row_bg);
    if !tail.is_empty() {
        let fg = readable_fg(fallback, row_bg, fallback);
        if let Some(last) = spans.last_mut() {
            if last.1 == fg {
                last.0.push_str(tail);
            } else {
                spans.push((tail.to_string(), fg));
            }
        } else {
            spans.push((tail.to_string(), fg));
        }
    }
    if spans.is_empty() {
        vec![(line.to_string(), readable_fg(fallback, row_bg, fallback))]
    } else {
        spans
    }
}

/// Token foregrounds for each file-diff row, old-file and new-file streams.
///
/// Syntect parse state is kept across lines **inside one hunk**. Section and
/// hunk header rows start a new pair of highlighters so omitted lines cannot
/// leak into a later hunk or a staged/unstaged section. Add lines feed the
/// new stream. Del lines feed the old stream. Context feeds both on inline
/// rows, or the matching side on split rows.
pub(crate) struct DiffSyntaxSpans {
    left: Vec<Vec<(String, Color)>>,
    right: Vec<Vec<(String, Color)>>,
}

impl DiffSyntaxSpans {
    pub(crate) fn left(&self, row: usize) -> &[(String, Color)] {
        self.left.get(row).map(Vec::as_slice).unwrap_or(&[])
    }

    pub(crate) fn right(&self, row: usize) -> &[(String, Color)] {
        self.right.get(row).map(Vec::as_slice).unwrap_or(&[])
    }
}

/// Identity for one paint of file-diff syntax spans.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DiffSyntaxKey {
    pub path: String,
    pub theme: ThemeId,
    pub fallback: Color,
    pub add_bg: Color,
    pub del_bg: Color,
    pub visible_start: usize,
    pub visible_end: usize,
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
/// tokens stay correct, but those rows do not store spans.
pub(crate) fn highlight_diff_rows(
    path: &str,
    rows: &[DiffRow],
    theme: ThemeId,
    fallback: Color,
    add_bg: Color,
    del_bg: Color,
    visible: Range<usize>,
) -> DiffSyntaxSpans {
    let n = rows.len();
    let visible = visible.start.min(n)..visible.end.min(n);
    let mut left = vec![Vec::new(); n];
    let mut right = vec![Vec::new(); n];
    if visible.is_empty() {
        return DiffSyntaxSpans { left, right };
    }
    if is_plain_text(path, None) {
        fill_plain_visible(
            rows, &visible, fallback, add_bg, del_bg, &mut left, &mut right,
        );
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
                        let left_spans = feed_split_cell(
                            cell,
                            false,
                            &mut old_hl,
                            &mut new_hl,
                            fallback,
                            add_bg,
                            del_bg,
                        );
                        let right_spans = feed_split_cell(
                            other,
                            true,
                            &mut old_hl,
                            &mut new_hl,
                            fallback,
                            add_bg,
                            del_bg,
                        );
                        if store {
                            left[idx] = left_spans;
                            right[idx] = right_spans;
                        }
                    } else {
                        let spans = feed_inline_cell(
                            cell,
                            &mut old_hl,
                            &mut new_hl,
                            fallback,
                            add_bg,
                            del_bg,
                        );
                        if store {
                            left[idx] = spans;
                        }
                    }
                }
            }
            DiffRow::Section(_) | DiffRow::Hunk { .. } => i += 1,
        }
    }
    DiffSyntaxSpans { left, right }
}

/// Reuse `cache` when `key` matches. Misses call [`highlight_diff_rows`].
pub(crate) fn cached_highlight_diff_rows(
    cache: &mut Option<CachedDiffSyntax>,
    key: DiffSyntaxKey,
    path: &str,
    rows: &[DiffRow],
    theme: ThemeId,
    fallback: Color,
    add_bg: Color,
    del_bg: Color,
    visible: Range<usize>,
) -> Arc<DiffSyntaxSpans> {
    if let Some(hit) = cache.as_ref() {
        if hit.key == key {
            return Arc::clone(&hit.spans);
        }
    }
    let spans = Arc::new(highlight_diff_rows(
        path, rows, theme, fallback, add_bg, del_bg, visible,
    ));
    *cache = Some(CachedDiffSyntax {
        key,
        spans: Arc::clone(&spans),
    });
    spans
}

fn fill_plain_visible(
    rows: &[DiffRow],
    visible: &Range<usize>,
    fallback: Color,
    add_bg: Color,
    del_bg: Color,
    left: &mut [Vec<(String, Color)>],
    right: &mut [Vec<(String, Color)>],
) {
    for idx in visible.clone() {
        let Some(DiffRow::Line {
            left: cell,
            right: other,
        }) = rows.get(idx)
        else {
            continue;
        };
        left[idx] = plain_cell_spans(cell, fallback, add_bg, del_bg);
        right[idx] = other
            .as_ref()
            .map(|cell| plain_cell_spans(cell, fallback, add_bg, del_bg))
            .unwrap_or_default();
    }
}

fn cell_bg(kind: DiffCellKind, add_bg: Color, del_bg: Color) -> Option<Color> {
    match kind {
        DiffCellKind::Add => Some(add_bg),
        DiffCellKind::Del => Some(del_bg),
        DiffCellKind::Ctx | DiffCellKind::Meta | DiffCellKind::Empty => None,
    }
}

fn plain_cell_spans(
    cell: &DiffCell,
    fallback: Color,
    add_bg: Color,
    del_bg: Color,
) -> Vec<(String, Color)> {
    match cell.kind {
        DiffCellKind::Empty | DiffCellKind::Meta => Vec::new(),
        _ => vec![(
            cell.text.clone(),
            readable_fg(fallback, cell_bg(cell.kind, add_bg, del_bg), fallback),
        )],
    }
}

fn feed_inline_cell(
    cell: &DiffCell,
    old_hl: &mut HighlightLines<'_>,
    new_hl: &mut HighlightLines<'_>,
    fallback: Color,
    add_bg: Color,
    del_bg: Color,
) -> Vec<(String, Color)> {
    let bg = cell_bg(cell.kind, add_bg, del_bg);
    match cell.kind {
        DiffCellKind::Empty | DiffCellKind::Meta => Vec::new(),
        DiffCellKind::Add => feed_line(new_hl, &cell.text, fallback, bg),
        DiffCellKind::Del => feed_line(old_hl, &cell.text, fallback, bg),
        DiffCellKind::Ctx => {
            let spans = feed_line(new_hl, &cell.text, fallback, bg);
            let _ = feed_line(old_hl, &cell.text, fallback, bg);
            spans
        }
    }
}

fn feed_split_cell(
    cell: &DiffCell,
    is_right: bool,
    old_hl: &mut HighlightLines<'_>,
    new_hl: &mut HighlightLines<'_>,
    fallback: Color,
    add_bg: Color,
    del_bg: Color,
) -> Vec<(String, Color)> {
    let bg = cell_bg(cell.kind, add_bg, del_bg);
    let use_new = if is_right {
        matches!(cell.kind, DiffCellKind::Add | DiffCellKind::Ctx)
    } else {
        matches!(cell.kind, DiffCellKind::Add)
    };
    match cell.kind {
        DiffCellKind::Empty | DiffCellKind::Meta => Vec::new(),
        _ if use_new => feed_line(new_hl, &cell.text, fallback, bg),
        _ => feed_line(old_hl, &cell.text, fallback, bg),
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

fn highlight_range(
    path: &str,
    line: &str,
    theme: ThemeId,
    fallback: Color,
    row_bg: Option<Color>,
) -> Vec<(String, Color)> {
    if line.is_empty() || is_plain_text(path, None) {
        return vec![(line.to_string(), readable_fg(fallback, row_bg, fallback))];
    }
    let syntax = syntax_for(path, None);
    let syn_theme = theme_set().get(syntect_theme_name(theme));
    let mut highlighter = HighlightLines::new(syntax, syn_theme);
    feed_line(&mut highlighter, line, fallback, row_bg)
}

#[cfg(test)]
thread_local! {
    static FED_LINES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn take_syntax_fed_lines() -> usize {
    FED_LINES.with(|count| count.replace(0))
}

fn feed_line(
    highlighter: &mut HighlightLines<'_>,
    line: &str,
    fallback: Color,
    row_bg: Option<Color>,
) -> Vec<(String, Color)> {
    if line.is_empty() {
        return Vec::new();
    }
    #[cfg(test)]
    FED_LINES.with(|count| count.set(count.get() + 1));
    let (head, tail) = split_highlight_budget(line);
    let mut out = feed_head(highlighter, head, fallback, row_bg);
    if !tail.is_empty() {
        let fg = readable_fg(fallback, row_bg, fallback);
        if let Some(last) = out.last_mut() {
            if last.1 == fg {
                last.0.push_str(tail);
            } else {
                out.push((tail.to_string(), fg));
            }
        } else {
            out.push((tail.to_string(), fg));
        }
    }
    if out.is_empty() {
        vec![(line.to_string(), readable_fg(fallback, row_bg, fallback))]
    } else {
        out
    }
}

fn feed_head(
    highlighter: &mut HighlightLines<'_>,
    line: &str,
    fallback: Color,
    row_bg: Option<Color>,
) -> Vec<(String, Color)> {
    if line.is_empty() {
        return Vec::new();
    }
    let set = syntax_set();
    let Ok(ranges) = highlighter.highlight_line(line, set) else {
        return vec![(line.to_string(), readable_fg(fallback, row_bg, fallback))];
    };
    let mut out: Vec<(String, Color)> = Vec::new();
    for (style, text) in ranges {
        if text.is_empty() {
            continue;
        }
        let fg = syntect_fg(style.foreground).map_or(fallback, |(r, g, b)| Color::Rgb(r, g, b));
        let fg = readable_fg(fg, row_bg, fallback);
        if let Some(last) = out.last_mut() {
            if last.1 == fg {
                last.0.push_str(text);
                continue;
            }
        }
        out.push((text.to_string(), fg));
    }
    out
}

fn syntect_fg(color: SyntectColor) -> Option<(u8, u8, u8)> {
    if color.a == 0 {
        return None;
    }
    Some((color.r, color.g, color.b))
}

fn readable_fg(fg: Color, row_bg: Option<Color>, fallback: Color) -> Color {
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
pub(crate) fn slice_styled_cols(
    parts: &[(String, Color)],
    offset: usize,
    width: usize,
) -> Vec<(String, Color)> {
    if width == 0 {
        return Vec::new();
    }
    let mut skipped = 0usize;
    let mut taken = 0usize;
    let mut out: Vec<(String, Color)> = Vec::new();
    for (text, color) in parts {
        for ch in text.chars() {
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
            if let Some(last) = out.last_mut() {
                if last.1 == *color {
                    last.0.push(ch);
                    taken = taken.saturating_add(cw);
                    continue;
                }
            }
            out.push((ch.to_string(), *color));
            taken = taken.saturating_add(cw);
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
            ADD_BG,
            DEL_BG,
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
        let colors: std::collections::HashSet<_> = spans.left(1).iter().map(|(_, c)| *c).collect();
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

    #[test]
    fn slice_styled_cols_keeps_emoji_display_width() {
        let emoji = '😀';
        assert_eq!(visible_width(&emoji.to_string()), 2);
        let parts = vec![
            ("A".into(), Color::Red),
            (emoji.to_string(), Color::Green),
            ("B".into(), Color::Blue),
        ];
        let sliced = slice_styled_cols(&parts, 1, 2);
        assert_eq!(sliced, vec![(emoji.to_string(), Color::Green)]);
        let clipped = slice_styled_cols(&parts, 0, 2);
        assert_eq!(clipped, vec![("A".into(), Color::Red)]);
    }

    fn syntax_key(visible: Range<usize>, cache_id: u64) -> DiffSyntaxKey {
        DiffSyntaxKey {
            path: "pack.json".into(),
            theme: ThemeId::TokyoNight,
            fallback: FALLBACK,
            add_bg: ADD_BG,
            del_bg: DEL_BG,
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
            ADD_BG,
            DEL_BG,
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
        let first = cached_highlight_diff_rows(
            &mut cache,
            key.clone(),
            "pack.json",
            &rows,
            ThemeId::TokyoNight,
            FALLBACK,
            ADD_BG,
            DEL_BG,
            visible.clone(),
        );
        let fed_first = take_syntax_fed_lines();
        let second = cached_highlight_diff_rows(
            &mut cache,
            key,
            "pack.json",
            &rows,
            ThemeId::TokyoNight,
            FALLBACK,
            ADD_BG,
            DEL_BG,
            visible.clone(),
        );
        let fed_second = take_syntax_fed_lines();
        assert!(fed_first > 0, "first paint feeds syntect");
        assert_eq!(fed_second, 0, "unchanged rows must not re-feed syntect");
        assert!(
            Arc::ptr_eq(&first, &second),
            "cache hit returns the same spans allocation"
        );
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
                .map(|(_, c)| *c)
                .collect::<std::collections::HashSet<_>>()
                .len()
                >= 2,
            "one hunk still carries parse state: {:?}",
            within_spans.left(1)
        );
        assert_eq!(
            across_spans.left(3),
            fresh.as_slice(),
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
            spans.left(5),
            fresh.as_slice(),
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
        assert_eq!(across_spans.left(3), fresh.as_slice());
        assert_ne!(across_spans.left(3), within_spans.left(1));
    }
}

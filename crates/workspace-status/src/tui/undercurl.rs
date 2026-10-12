//! Curly coloured underline (SGR 4:3 + SGR 58) under the code of commented
//! lines, painted over the finished frame.
//!
//! Ratatui 0.29 (built without `underline-color`) has no underline colour
//! or style, and its buffer diff would undo a curl with a plain `\x1b[4m`.
//! So the frame paints the commented code with [`Modifier::UNDERLINED`] (the
//! fallback every terminal shows), records each painted run as an
//! [`UndercurlHit`], and the event loop prints those cells again after the
//! draw, with a curly underline in the hit's colour.
//!
//! [`undercurl_enabled`] decides once at launch whether the upgrade runs:
//! only on a truecolor terminal, never under `NO_COLOR` or ASCII glyphs.
//! The plain underline stays in every case.

use std::io::{self, Write};

use crossterm::cursor::{MoveTo, RestorePosition, SavePosition};
use crossterm::queue;
use crossterm::style::{Attribute, SetAttribute, SetBackgroundColor, SetForegroundColor};
use ratatui::buffer::{Buffer, Cell};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier};
use unicode_width::UnicodeWidthStr;

use super::hyperlink::LinkHit;

/// A one-row screen span of commented code to reprint with a curly
/// underline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndercurlHit {
    /// 0-based screen row of the span.
    pub y: u16,
    /// 0-based first screen column of the span.
    pub x: u16,
    /// Span width in display columns.
    pub width: u16,
    /// Underline colour (SGR 58).
    pub color: Color,
}

/// The cells of one [`UndercurlHit`], read from a finished frame buffer.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UndercurlCells {
    /// Underline colour of the run.
    color: Color,
    /// `(x, y, cell)` to print again, wide-glyph continuations left out.
    cells: Vec<(u16, u16, Cell)>,
}

/// True when the terminal gets the curly underline: `COLORTERM` is
/// `truecolor` or `24bit` (any case), and neither `NO_COLOR` nor ASCII
/// glyphs is on.
///
/// The colon form `4:3` and `58:2::r:g:b` need a modern terminal; an older
/// one keeps the plain underline the frame already paints.
pub(crate) fn undercurl_enabled(no_color: bool, ascii: bool, colorterm: Option<&str>) -> bool {
    let truecolor = colorterm.is_some_and(|value| {
        let value = value.trim();
        value.eq_ignore_ascii_case("truecolor") || value.eq_ignore_ascii_case("24bit")
    });
    truecolor && !no_color && !ascii
}

/// The cells of row `y` from column `x` for `width` columns, as
/// `(x, y, cell)`.
///
/// The span is clipped to the buffer; a row off the buffer gives none. A
/// wide glyph is read once: the cells it covers after the first are
/// skipped, as is a cell with `skip` set, the same way ratatui's diff skips
/// them.
pub(crate) fn span_cells(buffer: &Buffer, y: u16, x: u16, width: u16) -> Vec<(u16, u16, Cell)> {
    let area = buffer.area;
    if y < area.top() || y >= area.bottom() {
        return Vec::new();
    }
    let start = x.max(area.left());
    let end = x.saturating_add(width).min(area.right());
    let mut cells = Vec::new();
    let mut to_skip = 0usize;
    for x in start..end {
        let cell = &buffer[(x, y)];
        if to_skip == 0 && !cell.skip {
            cells.push((x, y, cell.clone()));
        }
        to_skip = if to_skip > 0 {
            to_skip - 1
        } else {
            cell.symbol().width().saturating_sub(1)
        };
    }
    cells
}

/// Runs of [`Modifier::UNDERLINED`] cells inside `area` of `buffer`, one
/// [`UndercurlHit`] in `color` per run and row.
///
/// A wide glyph's run covers its continuation cells. Call it right after a
/// pane paints its rows, before anything paints over them.
pub(crate) fn underlined_runs(buffer: &Buffer, area: Rect, color: Color) -> Vec<UndercurlHit> {
    let area = area.intersection(buffer.area);
    let mut hits = Vec::new();
    for y in area.top()..area.bottom() {
        let mut run: Option<(u16, u16)> = None;
        let mut x = area.left();
        while x < area.right() {
            let cell = &buffer[(x, y)];
            if cell.skip || !cell.modifier.contains(Modifier::UNDERLINED) {
                hits.extend(run.take().map(|(start, end)| hit(y, start, end, color)));
                x += 1;
                continue;
            }
            let wide = u16::try_from(cell.symbol().width().max(1)).unwrap_or(1);
            let end = x.saturating_add(wide).min(area.right());
            run = match run {
                Some((start, run_end)) if run_end == x => Some((start, end)),
                other => {
                    hits.extend(other.map(|(start, end)| hit(y, start, end, color)));
                    Some((x, end))
                }
            };
            x = end;
        }
        hits.extend(run.map(|(start, end)| hit(y, start, end, color)));
    }
    hits
}

fn hit(y: u16, start: u16, end: u16, color: Color) -> UndercurlHit {
    UndercurlHit {
        y,
        x: start,
        width: end - start,
        color,
    }
}

/// Drop every hit that overlaps `rect`: a box painted there hides its
/// cells.
pub(crate) fn drop_undercurls_under(hits: &mut Vec<UndercurlHit>, rect: Rect) {
    hits.retain(|hit| !rect.intersects(Rect::new(hit.x, hit.y, hit.width, 1)));
}

/// Cut the columns of every link in `links` out of `hits`, splitting a hit
/// that a link crosses.
///
/// The OSC 8 overlay reprints a link cell through ratatui's plain underline,
/// and a curl reprint without OSC 8 would clear the link. So a link cell
/// keeps its link and the plain underline; the curl skips it.
pub(crate) fn cut_link_cells(hits: &[UndercurlHit], links: &[LinkHit]) -> Vec<UndercurlHit> {
    let mut out = Vec::with_capacity(hits.len());
    for hit in hits {
        let mut parts = vec![(hit.x, hit.x.saturating_add(hit.width))];
        for link in links.iter().filter(|link| link.y == hit.y) {
            let (cut_start, cut_end) = (link.x, link.x.saturating_add(link.width));
            parts = parts
                .into_iter()
                .flat_map(|(start, end)| {
                    [(start, end.min(cut_start)), (start.max(cut_end), end)]
                        .into_iter()
                        .filter(|(start, end)| start < end)
                })
                .collect();
        }
        out.extend(
            parts
                .into_iter()
                .map(|(start, end)| self::hit(hit.y, start, end, hit.color)),
        );
    }
    out
}

/// Read the cells of each hit in `hits` from `buffer`. A hit off the
/// buffer is left out.
pub(crate) fn undercurl_cells(buffer: &Buffer, hits: &[UndercurlHit]) -> Vec<UndercurlCells> {
    hits.iter()
        .filter_map(|hit| {
            let cells = span_cells(buffer, hit.y, hit.x, hit.width);
            (!cells.is_empty()).then_some(UndercurlCells {
                color: hit.color,
                cells,
            })
        })
        .collect()
}

/// SGR attributes for `modifier`, less the underline the curl replaces.
fn attributes(modifier: Modifier) -> impl Iterator<Item = Attribute> {
    [
        (Modifier::BOLD, Attribute::Bold),
        (Modifier::DIM, Attribute::Dim),
        (Modifier::ITALIC, Attribute::Italic),
        (Modifier::SLOW_BLINK, Attribute::SlowBlink),
        (Modifier::RAPID_BLINK, Attribute::RapidBlink),
        (Modifier::REVERSED, Attribute::Reverse),
        (Modifier::HIDDEN, Attribute::Hidden),
        (Modifier::CROSSED_OUT, Attribute::CrossedOut),
    ]
    .into_iter()
    .filter(move |(flag, _)| modifier.contains(*flag))
    .map(|(_, attr)| attr)
}

/// SGR 58 underline colour: `58:2::r:g:b` for RGB, `58:5:n` for an indexed
/// colour, nothing for a named one (the curl then takes the text colour).
fn underline_color_sgr(color: Color) -> Option<String> {
    match color {
        Color::Rgb(r, g, b) => Some(format!("\x1b[58:2::{r}:{g}:{b}m")),
        Color::Indexed(n) => Some(format!("\x1b[58:5:{n}m")),
        _ => None,
    }
}

/// Print each run's cells again with a curly underline (`\x1b[4:3m`) in the
/// run's colour (SGR 58), then reset (`\x1b[59m\x1b[0m`) and flush.
///
/// Each cell keeps its fg, bg, and other attributes. The cursor goes back
/// to where it was. Writes nothing when `enabled` is false or `runs` is
/// empty. This does not go through ratatui's `Backend::draw`: its modifier
/// diff would print a plain underline and undo the curl.
pub(crate) fn write_undercurls<W: Write>(
    out: &mut W,
    runs: &[UndercurlCells],
    enabled: bool,
) -> io::Result<()> {
    if !enabled || runs.is_empty() {
        return Ok(());
    }
    queue!(out, SavePosition)?;
    for run in runs {
        let curl_color = underline_color_sgr(run.color);
        let mut style: Option<(Color, Color, Modifier)> = None;
        let mut next_x: Option<(u16, u16)> = None;
        for (x, y, cell) in &run.cells {
            if next_x != Some((*x, *y)) {
                queue!(out, MoveTo(*x, *y))?;
            }
            let cell_style = (cell.fg, cell.bg, cell.modifier);
            if style != Some(cell_style) {
                write!(out, "\x1b[0m")?;
                for attr in attributes(cell.modifier) {
                    queue!(out, SetAttribute(attr))?;
                }
                queue!(
                    out,
                    SetForegroundColor(cell.fg.into()),
                    SetBackgroundColor(cell.bg.into())
                )?;
                write!(out, "\x1b[4:3m")?;
                if let Some(sgr) = &curl_color {
                    write!(out, "{sgr}")?;
                }
                style = Some(cell_style);
            }
            write!(out, "{}", cell.symbol())?;
            let advance = u16::try_from(cell.symbol().width().max(1)).unwrap_or(1);
            next_x = Some((x.saturating_add(advance), *y));
        }
    }
    write!(out, "\x1b[59m\x1b[0m")?;
    queue!(out, RestorePosition)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Style;

    const CURL: Color = Color::Rgb(0xe0, 0xaf, 0x68);

    fn curl(y: u16, x: u16, width: u16) -> UndercurlHit {
        hit(y, x, x + width, CURL)
    }

    /// Every `ESC [ <row> ; <col> H` cursor move in `out`, 1-based.
    fn cursor_moves(out: &str) -> Vec<(u16, u16)> {
        out.split("\x1b[")
            .skip(1)
            .filter_map(|seq| {
                let end = seq.find(|c: char| !c.is_ascii_digit() && c != ';')?;
                if !seq[end..].starts_with('H') {
                    return None;
                }
                let (row, col) = seq[..end].split_once(';')?;
                Some((row.parse().ok()?, col.parse().ok()?))
            })
            .collect()
    }

    fn painted(buffer: &Buffer, hits: &[UndercurlHit], enabled: bool) -> String {
        let mut out = Vec::new();
        write_undercurls(&mut out, &undercurl_cells(buffer, hits), enabled).unwrap();
        String::from_utf8(out).unwrap()
    }

    #[test]
    fn writes_curly_underline_with_rgb_colour_then_resets() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 20, 2));
        let style = Style::default()
            .fg(Color::Rgb(1, 2, 3))
            .bg(Color::Rgb(4, 5, 6))
            .add_modifier(Modifier::UNDERLINED | Modifier::BOLD);
        buffer.set_string(3, 1, "let", style);

        let out = painted(&buffer, &[curl(1, 3, 3)], true);

        let curl_at = out.find("\x1b[4:3m").unwrap_or_else(|| panic!("{out:?}"));
        let color_at = out
            .find("\x1b[58:2::224:175:104m")
            .unwrap_or_else(|| panic!("{out:?}"));
        let text_at = out.find("let").unwrap();
        let reset_at = out.rfind("\x1b[59m\x1b[0m").unwrap();
        assert!(curl_at < text_at && color_at < text_at && text_at < reset_at);
        // Cell colours and bold come back; no plain underline undoes the curl.
        assert!(out.contains("\x1b[38;2;1;2;3m"), "{out:?}");
        assert!(out.contains("\x1b[48;2;4;5;6m"), "{out:?}");
        assert!(out.contains("\x1b[1m"), "{out:?}");
        assert!(!out.contains("\x1b[4m"), "{out:?}");
        // Cursor: saved first, moved to the run, restored last.
        assert!(out.starts_with("\x1b7"), "{out:?}");
        assert_eq!(cursor_moves(&out), vec![(2, 4)], "{out:?}");
        assert!(out.ends_with("\x1b8"), "{out:?}");
    }

    #[test]
    fn disabled_writes_nothing() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        buffer.set_string(0, 0, "x", Style::default());
        assert_eq!(painted(&buffer, &[curl(0, 0, 1)], false), "");
        assert_eq!(painted(&buffer, &[], true), "");
    }

    #[test]
    fn enabled_only_for_truecolor_without_no_color_or_ascii() {
        assert!(undercurl_enabled(false, false, Some("truecolor")));
        assert!(undercurl_enabled(false, false, Some("24bit")));
        assert!(undercurl_enabled(false, false, Some("TrueColor")));
        assert!(!undercurl_enabled(false, false, None));
        assert!(!undercurl_enabled(false, false, Some("")));
        assert!(!undercurl_enabled(false, false, Some("256color")));
        assert!(!undercurl_enabled(true, false, Some("truecolor")));
        assert!(!undercurl_enabled(false, true, Some("truecolor")));
    }

    #[test]
    fn wide_glyph_prints_once() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        buffer.set_string(0, 0, "a界b", Style::default());
        let xs: Vec<u16> = span_cells(&buffer, 0, 0, 4)
            .iter()
            .map(|(x, _, _)| *x)
            .collect();
        assert_eq!(xs, vec![0, 1, 3]);

        let out = painted(&buffer, &[curl(0, 0, 4)], true);
        assert_eq!(out.matches('界').count(), 1, "{out:?}");
        // The wide glyph moves the cursor two columns: no extra MoveTo.
        assert_eq!(cursor_moves(&out), vec![(1, 1)], "{out:?}");
    }

    #[test]
    fn reset_colours_write_the_terminal_defaults() {
        // An unpainted cell (terminal background mode): fg and bg `Reset`.
        let mut buffer = Buffer::empty(Rect::new(0, 0, 10, 1));
        buffer.set_string(2, 0, "x", Style::default());
        assert_eq!(buffer[(2, 0)].bg, Color::Reset);
        let out = painted(&buffer, &[curl(0, 2, 1)], true);
        let bg_at = out.find("\x1b[49m").unwrap_or_else(|| panic!("{out:?}"));
        let fg_at = out.find("\x1b[39m").unwrap_or_else(|| panic!("{out:?}"));
        let text_at = out.find('x').unwrap();
        assert!(bg_at < text_at && fg_at < text_at, "{out:?}");
        assert!(out.contains("\x1b[4:3m"), "{out:?}");
    }

    #[test]
    fn underlined_runs_cover_wide_glyphs_and_split_on_gaps() {
        let mut buffer = Buffer::empty(Rect::new(0, 0, 12, 2));
        let under = Style::default().add_modifier(Modifier::UNDERLINED);
        buffer.set_string(1, 0, "a界b", under);
        buffer.set_string(6, 0, "cd", under);
        buffer.set_string(0, 1, "plain", Style::default());
        let runs = underlined_runs(&buffer, buffer.area, CURL);
        assert_eq!(runs, vec![curl(0, 1, 4), curl(0, 6, 2)]);
        // A narrower area clips the run.
        let clipped = underlined_runs(&buffer, Rect::new(0, 0, 3, 1), CURL);
        assert_eq!(clipped, vec![curl(0, 1, 2)]);
    }

    #[test]
    fn link_cells_are_cut_out_of_a_curl() {
        let link = |y: u16, x: u16, width: u16| LinkHit {
            y,
            x,
            width,
            url: "https://x/y".into(),
        };
        let hits = [curl(2, 0, 10), curl(3, 0, 4)];
        let cut = cut_link_cells(&hits, &[link(2, 3, 2), link(3, 0, 4), link(5, 0, 9)]);
        assert_eq!(cut, vec![curl(2, 0, 3), curl(2, 5, 5)]);
    }

    #[test]
    fn dropped_under_a_box() {
        let mut hits = vec![curl(1, 0, 4), curl(4, 0, 4)];
        drop_undercurls_under(&mut hits, Rect::new(2, 3, 5, 3));
        assert_eq!(hits, vec![curl(1, 0, 4)]);
    }
}

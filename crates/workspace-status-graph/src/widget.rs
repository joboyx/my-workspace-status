//! Ratatui [`Widget`] for [`GraphModel`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget};

use crate::chrome::{
    graph_chrome_budget, selection_detail_parts, GraphFooterSelection, LOADING_OLDER,
};
use crate::format::{format_label, format_sync, slice_label_parts, LabelKind, LabelPart};
use crate::glyphs::{ASCII, UNICODE};
use crate::gutter::graph_gutter_cap;
use crate::lane_colors::{default_lane_colors, lane_fg};
use crate::model::GraphModel;
use crate::paint::{paint_model_with, PaintOpts, PaintedLine};

/// Subject, meta, and ref-chip colours for graph labels.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphLabelPalette {
    /// Commit subject.
    pub subject: Color,
    /// Hash, date, author, worktree marks, padding.
    pub meta: Color,
    /// Feature / local branch chips.
    pub branch_local: Color,
    /// Default-branch chips.
    pub branch_default: Color,
    /// Remote-tracking chips.
    pub remote: Color,
    /// Tag chips.
    pub tag: Color,
    /// Checkout / `[HEAD]` mark.
    pub head_mark: Color,
    /// Fully hidden leftover branch/tag chip (`[+N]`). Distinct from muted
    /// meta. Omitted when the last painted chip is only truncated.
    pub overflow: Color,
}

/// Renderable git-graph widget.
///
/// Paint with [`Widget::render`]. Tests use a `Buffer` or `TestBackend`.
/// The widget does not read a TTY.
#[derive(Clone, Copy, Debug)]
pub struct GraphWidget<'a> {
    model: &'a GraphModel,
    ascii: bool,
    gutter_width: Option<u16>,
    selected: Option<usize>,
    scroll: u16,
    now_unix: Option<i64>,
    loading_older: bool,
    lane_colors: &'a [Color],
    search_matches: &'a [usize],
    search_bg: Option<Color>,
    flash_rows: &'a [(usize, Color)],
    commented_rows: &'a [usize],
    /// Comment glyph (`ICON_COMMENT`: `"` / nf-fa-comment). Empty uses `"`.
    comment_glyph: &'a str,
    /// Selectable rows whose comments are all resolved.
    resolved_comment_rows: &'a [usize],
    /// Glyph for [`Self::resolved_comment_rows`]. Empty uses `'`.
    resolved_comment_glyph: &'a str,
    cursor_fg: Color,
    cursor_bg: Option<Color>,
    label_palette: Option<GraphLabelPalette>,
    col_offset: u16,
}

impl<'a> GraphWidget<'a> {
    /// Build a widget over `model`. Unicode glyphs are the default.
    pub fn new(model: &'a GraphModel) -> Self {
        Self {
            model,
            ascii: false,
            gutter_width: None,
            selected: None,
            scroll: 0,
            now_unix: None,
            loading_older: false,
            lane_colors: &[],
            search_matches: &[],
            search_bg: None,
            flash_rows: &[],
            commented_rows: &[],
            comment_glyph: "\"",
            resolved_comment_rows: &[],
            resolved_comment_glyph: "'",
            cursor_fg: Color::Cyan,
            cursor_bg: None,
            label_palette: None,
            col_offset: 0,
        }
    }

    /// Use ASCII glyphs when `ascii` is true.
    pub fn ascii(mut self, ascii: bool) -> Self {
        self.ascii = ascii;
        self
    }

    /// Cap the gutter at `width` columns. Topology still uses the full
    /// lane model; paint clips every row to the same left-aligned window.
    pub fn gutter_width(mut self, width: u16) -> Self {
        self.gutter_width = Some(width);
        self
    }

    /// Highlight the visible-row index, if any.
    pub fn selected(mut self, index: Option<usize>) -> Self {
        self.selected = index;
        self
    }

    /// Skip this many painted lines after the sync header.
    pub fn scroll(mut self, scroll: u16) -> Self {
        self.scroll = scroll;
        self
    }

    /// Freeze the relative-date clock (unix seconds). Tests pass a fixed instant.
    pub fn now_unix(mut self, unix: i64) -> Self {
        self.now_unix = Some(unix);
        self
    }

    /// Paint `loading older…` under the list while the next window loads.
    pub fn loading_older(mut self, loading: bool) -> Self {
        self.loading_older = loading;
        self
    }

    /// Per-cell gutter colours. Empty uses [`crate::DEFAULT_LANE_COLORS`].
    pub fn lane_colors(mut self, colors: &'a [Color]) -> Self {
        self.lane_colors = colors;
        self
    }

    /// Paint search-match background on selectable graph rows.
    ///
    /// `indices` are [`GraphModel::visible_rows`] indexes. Spacers stay
    /// unhighlighted. [`Self::selected`] still wins over a match.
    pub fn search_matches(mut self, indices: &'a [usize], bg: Color) -> Self {
        self.search_matches = indices;
        self.search_bg = Some(bg);
        self
    }

    /// Paint flash background on graph rows (node + spacers).
    ///
    /// `rows` are [`GraphModel::visible_rows`] indexes with the fade colour.
    /// Cursor and search still win. Unlike search, spacers follow the node.
    pub fn flash_rows(mut self, rows: &'a [(usize, Color)]) -> Self {
        self.flash_rows = rows;
        self
    }

    /// Mark commented selectable rows (`ICON_COMMENT` after the gutter).
    ///
    /// `indices` are [`GraphModel::visible_rows`] indexes. Selected rows keep
    /// `▌`. The comment glyph stays visible on the selected row. Spacers
    /// stay unmarked. Uncommented rows do not reserve a column.
    pub fn commented_rows(mut self, indices: &'a [usize]) -> Self {
        self.commented_rows = indices;
        self
    }

    /// Glyph for [`Self::commented_rows`]. Default `"`.
    ///
    /// The TUI passes `icon_comment` so ASCII `"` and nerd nf-fa-comment
    /// match tree and diff marks.
    pub fn comment_glyph(mut self, glyph: &'a str) -> Self {
        self.comment_glyph = glyph;
        self
    }

    /// Mark rows whose comments are all resolved (`ICON_COMMENT_RESOLVED`).
    ///
    /// Open [`Self::commented_rows`] win when a row is in both lists.
    pub fn resolved_comment_rows(mut self, indices: &'a [usize]) -> Self {
        self.resolved_comment_rows = indices;
        self
    }

    /// Glyph for [`Self::resolved_comment_rows`]. Default `'`.
    pub fn resolved_comment_glyph(mut self, glyph: &'a str) -> Self {
        self.resolved_comment_glyph = glyph;
        self
    }

    /// Cursor bar (`▌`) plus `cursorBg`. Spacers keep the background only.
    pub fn cursor_style(mut self, fg: Color, bg: Color) -> Self {
        self.cursor_fg = fg;
        self.cursor_bg = Some(bg);
        self
    }

    /// Colour commit subjects, meta, ref chips, the hidden-ref overflow chip,
    /// and the matching chip runs on the 2-line selection footer.
    pub fn label_palette(mut self, palette: GraphLabelPalette) -> Self {
        self.label_palette = Some(palette);
        self
    }

    /// Skip this many columns of the label (gutter stays put). Clips to the
    /// pane; rows do not grow.
    pub fn col_offset(mut self, offset: u16) -> Self {
        self.col_offset = offset;
        self
    }
}

/// Thumb offset from the track top (or left), and thumb length, matching
/// [`GraphWidget`]'s ratatui `Scrollbar`.
///
/// Paint uses `ScrollbarState::new(content_len.saturating_sub(1)).position(scroll)`
/// with no begin/end symbols and default viewport (`track_height`). `None` when
/// ratatui would skip the bar (`content_len < 2` or a zero-height track).
/// Reused for the horizontal track (`track_height` is then the track width).
pub fn graph_scrollbar_thumb(
    content_len: usize,
    scroll: u16,
    track_height: u16,
) -> Option<(u16, u16)> {
    let content_length = content_len.saturating_sub(1);
    if content_length == 0 || track_height == 0 {
        return None;
    }
    let track_length = f64::from(track_height);
    let viewport_length = f64::from(track_height);
    let max_position = content_length.saturating_sub(1) as f64;
    let start_position = (f64::from(scroll)).clamp(0.0, max_position);
    let max_viewport_position = max_position + viewport_length;
    if max_viewport_position <= 0.0 {
        return None;
    }
    let end_position = start_position + viewport_length;
    let thumb_start = start_position * track_length / max_viewport_position;
    let thumb_end = end_position * track_length / max_viewport_position;
    let thumb_start = thumb_start.round().clamp(0.0, track_length - 1.0) as u16;
    let thumb_end = thumb_end.round().clamp(0.0, track_length) as u16;
    let thumb_length = thumb_end.saturating_sub(thumb_start).max(1);
    Some((thumb_start, thumb_length))
}

/// Vertical graph scrollbar is painted only after the list leaves the top.
pub fn graph_vscroll_visible(scroll: u16) -> bool {
    scroll > 0
}

/// Horizontal graph scrollbar is painted only after the viewport leaves the
/// left edge.
pub fn graph_hscroll_visible(col_offset: u16) -> bool {
    col_offset > 0
}

/// Max `col_offset` so the longest label can sit in the viewport.
///
/// `vscroll` is true when the 1-column vertical bar is reserved.
pub fn graph_col_max(model: &GraphModel, ascii: bool, pane_width: u16, vscroll: bool) -> usize {
    let glyphs = if ascii { &ASCII } else { &UNICODE };
    let pane = pane_width.max(1) as usize;
    let inner = if vscroll {
        pane.saturating_sub(1)
    } else {
        pane
    };
    let cap = graph_gutter_cap(inner.max(1));
    let label_viewport = inner
        .saturating_sub(1)
        .saturating_sub(cap)
        .saturating_sub(1);
    let longest = model
        .visible_rows()
        .iter()
        .map(|row| format_label(row, glyphs).chars().count())
        .max()
        .unwrap_or(0);
    longest.saturating_sub(label_viewport.max(1))
}

impl Widget for GraphWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let vscroll = graph_vscroll_visible(self.scroll);
        let hscroll = graph_hscroll_visible(self.col_offset);
        let v_cols = u16::from(vscroll);
        let pane = area.width.saturating_sub(v_cols) as usize;
        let cap = Some(match self.gutter_width {
            Some(w) => w as usize,
            None => graph_gutter_cap(pane.max(1)),
        });
        let default_colors = default_lane_colors();
        let lane_colors: &[Color] = if self.lane_colors.is_empty() {
            &default_colors
        } else {
            self.lane_colors
        };
        let fallback = Color::Reset;
        let now = self.now_unix.unwrap_or_else(now_unix_secs);
        let chrome =
            graph_chrome_budget(area.height, self.loading_older, self.model.sync.is_some());
        let mut y = area.y;
        let list_bottom = area
            .y
            .saturating_add(u16::from(chrome.header) + chrome.list_height);

        if chrome.header {
            if let Some(sync) = &self.model.sync {
                put_text_line(
                    buf,
                    area.x,
                    y,
                    area.width,
                    &format_sync(sync, glyphs),
                    false,
                    fallback,
                );
                y = y.saturating_add(1);
            }
        }

        let skip = self.scroll as usize;
        let line_width = area.width.saturating_sub(1 + v_cols) as usize; // cursor + optional scrollbar
        let painted = paint_model_with(
            self.model,
            glyphs,
            PaintOpts {
                gutter_width: cap,
                line_width: Some(line_width.max(1)),
                now_unix: self.now_unix,
            },
        );
        let content_len = painted.len();
        let list_top = y;
        let list_height = list_bottom.saturating_sub(list_top);
        for line in painted.iter().skip(skip) {
            if y >= list_bottom {
                break;
            }
            let selected = self.selected.is_some() && line.row_index == self.selected;
            let search_match = line.selectable
                && line
                    .row_index
                    .is_some_and(|i| self.search_matches.contains(&i));
            let open_comment = line.selectable
                && line
                    .row_index
                    .is_some_and(|i| self.commented_rows.contains(&i));
            let resolved_comment = line.selectable
                && !open_comment
                && line
                    .row_index
                    .is_some_and(|i| self.resolved_comment_rows.contains(&i));
            let commented = open_comment || resolved_comment;
            let flash_bg = line.row_index.and_then(|i| {
                self.flash_rows
                    .iter()
                    .find(|(idx, _)| *idx == i)
                    .map(|(_, color)| *color)
            });
            put_painted_line(
                buf,
                area.x,
                y,
                area.width.saturating_sub(v_cols),
                line,
                selected,
                search_match,
                commented,
                self.search_bg,
                flash_bg,
                self.cursor_fg,
                self.cursor_bg,
                lane_colors,
                fallback,
                self.label_palette,
                self.col_offset,
                if resolved_comment {
                    self.resolved_comment_glyph
                } else {
                    self.comment_glyph
                },
            );
            y = y.saturating_add(1);
        }
        if vscroll && list_height > 0 && area.width > 0 {
            let mut sb_state = ScrollbarState::new(content_len.saturating_sub(1))
                .position((skip).min(content_len.saturating_sub(1)));
            let sb_area = Rect {
                x: area.x.saturating_add(area.width.saturating_sub(1)),
                y: list_top,
                width: 1,
                height: list_height,
            };
            StatefulWidget::render(
                Scrollbar::new(ScrollbarOrientation::VerticalRight)
                    .begin_symbol(None)
                    .end_symbol(None),
                sb_area,
                buf,
                &mut sb_state,
            );
        }
        if hscroll && list_height > 0 && area.width > 0 {
            let max = graph_col_max(self.model, self.ascii, area.width, vscroll);
            if max > 0 {
                let mut sb_state =
                    ScrollbarState::new(max).position((self.col_offset as usize).min(max));
                let sb_area = Rect {
                    x: area.x,
                    y: list_bottom.saturating_sub(1),
                    width: area.width.saturating_sub(v_cols).max(1),
                    height: 1,
                };
                StatefulWidget::render(
                    Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
                        .begin_symbol(None)
                        .end_symbol(None),
                    sb_area,
                    buf,
                    &mut sb_state,
                );
            }
        }

        let mut footer_y = area.y.saturating_add(area.height);
        if chrome.older {
            footer_y = footer_y.saturating_sub(1);
            put_text_line(
                buf,
                area.x,
                footer_y,
                area.width,
                LOADING_OLDER,
                false,
                fallback,
            );
        }
        if chrome.footer {
            footer_y = footer_y.saturating_sub(2);
            let rows = self.model.visible_rows();
            let selected = self.selected.and_then(|i| rows.get(i));
            let [line1, line2] = selection_detail_parts(
                self.model,
                GraphFooterSelection::from(selected),
                glyphs,
                area.width as usize,
                now,
            );
            put_parts_line(
                buf,
                area.x,
                footer_y,
                area.width,
                &line1,
                self.label_palette,
                fallback,
            );
            put_parts_line(
                buf,
                area.x,
                footer_y.saturating_add(1),
                area.width,
                &line2,
                self.label_palette,
                fallback,
            );
        }
    }
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn put_painted_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    line: &PaintedLine,
    selected: bool,
    search_match: bool,
    commented: bool,
    search_bg: Option<Color>,
    flash_bg: Option<Color>,
    cursor_fg: Color,
    cursor_bg: Option<Color>,
    lane_colors: &[Color],
    fallback: Color,
    palette: Option<GraphLabelPalette>,
    col_offset: u16,
    comment_glyph: &str,
) {
    if width == 0 {
        return;
    }
    let row = Rect::new(x, y, width, 1);
    let bar = if selected && line.selectable {
        "▌"
    } else {
        " "
    };
    let row_bg = if selected {
        cursor_bg
    } else if search_match {
        search_bg
    } else {
        flash_bg
    };
    let mut bar_style = Style::default().fg(cursor_fg).add_modifier(Modifier::BOLD);
    let mut row_style = Style::default();
    if let Some(bg) = row_bg {
        bar_style = bar_style.bg(bg);
        row_style = row_style.bg(bg);
        buf.set_style(row, row_style);
    }
    buf[(x, y)].set_symbol(bar);
    buf[(x, y)].set_style(bar_style);

    let end = x.saturating_add(width);
    let mut col = x.saturating_add(1);
    for cell in &line.gutter {
        if col >= end {
            break;
        }
        let fg = lane_fg(cell.color_lane, lane_colors, fallback);
        let mut style = Style::default().fg(fg);
        if let Some(bg) = row_bg {
            style = style.bg(bg);
        }
        // One GraphCell = one buffer column. Overwrite any wide-glyph
        // continuation so rails stay put when the label clips.
        buf[(col, y)].set_symbol(&cell.ch);
        buf[(col, y)].set_style(style);
        col = col.saturating_add(1);
    }
    if !line.gutter.is_empty() && col < end {
        let mut style = Style::default();
        if let Some(bg) = row_bg {
            style = style.bg(bg);
        }
        buf[(col, y)].set_symbol(" ");
        buf[(col, y)].set_style(style);
        col = col.saturating_add(1);
    }
    if commented && line.selectable && col < end {
        let glyph = if comment_glyph.is_empty() {
            "\""
        } else {
            comment_glyph
        };
        let mut mark_style = Style::default().fg(cursor_fg).add_modifier(Modifier::BOLD);
        if let Some(bg) = row_bg {
            mark_style = mark_style.bg(bg);
        }
        buf[(col, y)].set_symbol(glyph);
        buf[(col, y)].set_style(mark_style);
        col = col.saturating_add(1);
        if col < end {
            let mut gap = Style::default();
            if let Some(bg) = row_bg {
                gap = gap.bg(bg);
            }
            buf[(col, y)].set_symbol(" ");
            buf[(col, y)].set_style(gap);
            col = col.saturating_add(1);
        }
    }
    let label_w = end.saturating_sub(col);
    if label_w == 0 {
        return;
    }
    let sliced = slice_line_label(line, col_offset as usize, label_w as usize);
    Line::from(label_spans(&sliced, palette, fallback))
        .style(row_style)
        .render(Rect::new(col, y, label_w, 1), buf);
}

fn slice_line_label(line: &PaintedLine, offset: usize, width: usize) -> PaintedLine {
    let mut out = line.clone();
    if out.parts.is_empty() {
        out.label = out.label.chars().skip(offset).take(width).collect();
        return out;
    }
    out.parts = slice_label_parts(&out.parts, offset, width);
    out.label = out.parts.iter().map(|p| p.text.as_str()).collect();
    out
}

fn label_spans(
    line: &PaintedLine,
    palette: Option<GraphLabelPalette>,
    fallback: Color,
) -> Vec<Span<'static>> {
    if line.parts.is_empty() {
        let color = match (palette, line.selectable) {
            (Some(pal), true) => pal.subject,
            (Some(pal), false) => pal.meta,
            (None, _) => fallback,
        };
        return vec![Span::styled(line.label.clone(), Style::default().fg(color))];
    }
    spans_from_parts(&line.parts, palette, fallback)
}

fn label_kind_color(kind: LabelKind, pal: GraphLabelPalette) -> Color {
    match kind {
        LabelKind::Subject => pal.subject,
        LabelKind::Meta => pal.meta,
        LabelKind::ChipHead => pal.head_mark,
        LabelKind::ChipLocal => pal.branch_local,
        LabelKind::ChipDefault => pal.branch_default,
        LabelKind::ChipRemote => pal.remote,
        LabelKind::ChipTag => pal.tag,
        LabelKind::Overflow => pal.overflow,
    }
}

fn spans_from_parts(
    parts: &[LabelPart],
    palette: Option<GraphLabelPalette>,
    fallback: Color,
) -> Vec<Span<'static>> {
    if parts.is_empty() {
        return Vec::new();
    }
    match palette {
        None => {
            let text: String = parts.iter().map(|p| p.text.as_str()).collect();
            vec![Span::styled(text, Style::default().fg(fallback))]
        }
        Some(pal) => parts
            .iter()
            .map(|p| {
                let mut style = Style::default().fg(label_kind_color(p.kind, pal));
                if p.kind == LabelKind::Overflow {
                    style = style.add_modifier(Modifier::BOLD);
                }
                Span::styled(p.text.clone(), style)
            })
            .collect(),
    }
}

fn put_parts_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    parts: &[LabelPart],
    palette: Option<GraphLabelPalette>,
    fallback: Color,
) {
    Line::from(spans_from_parts(parts, palette, fallback)).render(Rect::new(x, y, width, 1), buf);
}

fn put_text_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    _selected: bool,
    fg: Color,
) {
    let style = Style::default().fg(fg);
    Line::from(Span::styled(text.to_string(), style)).render(Rect::new(x, y, width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::action::Effect;
    use crate::chrome::{graph_chrome_budget, selection_detail_lines, GraphFooterSelection};
    use crate::format::{format_local_timestamp, overflow_chip_text};
    use crate::hex_color;
    use crate::model::{Commit, GraphRef, GraphRow, Stash, SyncState, SyncStatus, Worktree};
    use crate::paint::{paint_model, paint_model_with, PaintOpts};
    use crate::topology::cells_text;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    const NOW: i64 = 1_700_000_000;

    fn commit(id: &str, subject: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.into(),
            subject: subject.into(),
            parents: parents.iter().map(|p| (*p).to_string()).collect(),
            refs: Vec::new(),
            author_name: "Ada".into(),
            author_date_unix: NOW - 3600,
        }
    }

    fn worktree(path: &str, head: &str, ignored: bool) -> Worktree {
        Worktree {
            path: path.into(),
            head_id: Some(head.into()),
            branch: Some("feature/graph".into()),
            ignored,
            is_current: false,
        }
    }

    fn sample_model() -> GraphModel {
        let head = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let parent = "bbb2222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        GraphModel {
            commits: vec![
                Commit {
                    id: head.into(),
                    subject: "add graph crate".into(),
                    parents: vec![parent.into()],
                    refs: vec!["main".into()],
                    author_name: "Ada Lovelace".into(),
                    author_date_unix: NOW - 120,
                },
                commit(parent, "prior commit", &[]),
            ],
            stashes: vec![Stash {
                id: "ccc3333ccccccccccccccccccccccccccccccc".into(),
                stash_ref: "stash@{0}".into(),
                subject: "WIP on main".into(),
                author_name: "Ada Lovelace".into(),
                author_date_unix: NOW - 86400,
                parent_id: Some(head.into()),
            }],
            worktrees: vec![
                worktree(".worktrees/feature/graph", head, false),
                worktree("notes", head, true),
            ],
            head_id: Some(head.into()),
            sync: Some(SyncState {
                branch: "main".into(),
                status: SyncStatus::Ahead,
                ahead: 1,
                behind: 0,
            }),
            uncommitted: Some(true),
            window: 2,
            ..GraphModel::default()
        }
    }

    fn merge_model() -> GraphModel {
        GraphModel {
            commits: vec![
                commit("m999", "merge", &["a111", "b222"]),
                commit("a111", "left", &["r000"]),
                commit("b222", "right", &["r000"]),
                commit("r000", "root", &[]),
            ],
            head_id: Some("m999".into()),
            ..GraphModel::default()
        }
    }

    fn two_parent_join_model() -> GraphModel {
        GraphModel {
            commits: vec![
                commit("mainTip", "main tip", &["base"]),
                commit("tipA", "tip A", &["base"]),
                commit("tipB", "tip B", &["base"]),
                commit("base", "shared parent", &[]),
            ],
            head_id: Some("mainTip".into()),
            ..GraphModel::default()
        }
    }

    fn render_lines(model: &GraphModel, width: u16, height: u16, ascii: bool) -> Vec<String> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(model)
                    .ascii(ascii)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        (0..height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..width {
                    let symbol = buffer[(x, y)].symbol();
                    if !symbol.is_empty() {
                        line.push_str(symbol);
                    }
                }
                line.trim_end().to_string()
            })
            .filter(|line| !line.is_empty())
            .collect()
    }

    #[test]
    fn paints_head_sync_stash_and_worktree() {
        let lines = render_lines(&sample_model(), 120, 16, false);
        let joined = lines.join("\n");
        assert!(joined.contains("main ↑1"), "sync header: {joined}");
        assert!(
            joined.contains("○ uncommitted changes"),
            "uncommitted: {joined}"
        );
        assert!(joined.contains("◇"), "stash diamond: {joined}");
        assert!(joined.contains("stash@{0}"), "stash ref: {joined}");
        assert!(joined.contains("WIP on main"), "stash subject: {joined}");
        assert!(joined.contains("aaa1111"), "{joined}");
        assert!(
            joined.contains("[main]") || joined.contains("[main]"),
            "HEAD branch chip: {joined}"
        );
        assert!(joined.contains(".worktrees/feature/graph"), "{joined}");
        assert!(
            joined.contains(""),
            "linked worktree uses ICON_LINKED_WORKTREE: {joined}"
        );
        assert!(!joined.contains("🔗"), "emoji desyncs the gutter: {joined}");
        assert!(joined.contains("bbb2222"), "{joined}");
        assert!(joined.contains("prior commit"), "{joined}");
    }

    #[test]
    fn stash_sits_above_parent_commit() {
        let lines = render_lines(&sample_model(), 120, 16, false);
        let stash = lines
            .iter()
            .position(|l| l.contains("stash@{0}"))
            .expect("stash row");
        let head = lines
            .iter()
            .position(|l| l.contains("add graph crate"))
            .expect("head row");
        assert!(stash < head, "stash {stash} should sit above HEAD {head}");
    }

    #[test]
    fn commit_paints_subject_then_refs_hash_date_author() {
        let model = sample_model();
        let lines = render_lines(&model, 120, 16, false);
        let subject = lines
            .iter()
            .position(|l| l.contains("add graph crate"))
            .expect("subject row");
        assert!(
            !lines[subject].contains("aaa1111"),
            "hash must not sit on the subject: {}",
            lines[subject]
        );
        assert!(
            !lines[subject].contains("[main]") && !lines[subject].contains("[main]"),
            "refs must not sit on the subject: {}",
            lines[subject]
        );
        let meta = &lines[subject + 1];
        assert!(meta.contains("aaa1111"), "short hash on spacer: {meta}");
        assert!(
            meta.contains("[main]") || meta.contains("[main]"),
            "branch chip on spacer: {meta}"
        );
        let painted = paint_model_with(
            &model,
            &UNICODE,
            PaintOpts {
                now_unix: Some(NOW),
                line_width: Some(200),
                ..PaintOpts::default()
            },
        );
        let subject_line = painted
            .iter()
            .find(|l| l.label.contains("add graph crate"))
            .expect("subject painted");
        assert!(subject_line.selectable);
        let spacer = painted
            .iter()
            .skip_while(|l| !l.label.contains("add graph crate"))
            .nth(1)
            .expect("commit spacer");
        assert!(!spacer.selectable);
        assert_eq!(spacer.row_index, subject_line.row_index);
        assert!(spacer.label.contains("aaa1111"), "{}", spacer.label);
        assert!(spacer.label.contains("2m"), "{}", spacer.label);
        assert!(spacer.label.contains("Ada Lovelace"), "{}", spacer.label);
    }

    #[test]
    fn stash_paints_subject_then_ref_hash_date_author() {
        let model = sample_model();
        let lines = render_lines(&model, 120, 16, false);
        let subject = lines
            .iter()
            .position(|l| l.contains("WIP on main"))
            .expect("stash subject row");
        assert!(
            !lines[subject].contains("ccc3333"),
            "hash must not sit on the subject: {}",
            lines[subject]
        );
        assert!(
            !lines[subject].contains("stash@{0}"),
            "stash@{{n}} must not sit on the subject: {}",
            lines[subject]
        );
        let meta = &lines[subject + 1];
        assert!(meta.contains("stash@{0}"), "stash ref on spacer: {meta}");
        assert!(meta.contains("ccc3333"), "short hash on spacer: {meta}");
        let painted = paint_model_with(
            &model,
            &UNICODE,
            PaintOpts {
                now_unix: Some(NOW),
                line_width: Some(200),
                ..PaintOpts::default()
            },
        );
        let subject_line = painted
            .iter()
            .find(|l| l.selectable && l.label.contains("WIP on main"))
            .expect("stash subject painted");
        assert!(subject_line.selectable);
        let spacer = painted
            .iter()
            .skip_while(|l| !(l.selectable && l.label.contains("WIP on main")))
            .nth(1)
            .expect("stash spacer");
        assert!(!spacer.selectable);
        assert_eq!(spacer.row_index, subject_line.row_index);
        assert!(spacer.label.contains("stash@{0}"), "{}", spacer.label);
        assert!(spacer.label.contains("ccc3333"), "{}", spacer.label);
        assert!(
            spacer.label.contains(&format_local_timestamp(NOW - 86400)),
            "{}",
            spacer.label
        );
        assert!(spacer.label.contains("Ada Lovelace"), "{}", spacer.label);
        assert!(
            !subject_line.label.contains("stash@{0}"),
            "node is subject-only: {}",
            subject_line.label
        );
    }

    #[test]
    fn stash_leaf_sits_on_spur_not_fake_lane() {
        let model = sample_model();
        let lines = render_lines(&model, 120, 16, false);
        let joined = lines.join("\n");
        let painted = paint_model(&model, &UNICODE, None);
        let stash_line = painted
            .iter()
            .find(|l| l.selectable && l.label.contains("WIP on main"))
            .expect("stash painted");
        let diamond_idx = stash_line
            .gutter
            .iter()
            .position(|c| c.ch == "◇")
            .expect("◇ in gutter");
        assert!(
            diamond_idx >= crate::CELL_W,
            "◇ must sit on a spur off stash^1, not lane 0: {}",
            cells_text(&stash_line.gutter)
        );
        assert_eq!(
            stash_line.gutter[diamond_idx].color_lane,
            Some(0),
            "spur colour is stash^1 lane, not a fake lane palette"
        );
        let spacer = painted
            .iter()
            .skip_while(|l| !(l.selectable && l.label.contains("WIP on main")))
            .nth(1)
            .expect("stash spacer");
        assert_eq!(
            spacer.gutter[diamond_idx].ch,
            UNICODE.vertical,
            "spacer must carry the short spur, got {}",
            cells_text(&spacer.gutter)
        );
        assert!(!spacer.selectable);
        assert_eq!(spacer.row_index, stash_line.row_index);
        assert!(spacer.label.contains("stash@{0}"), "{}", spacer.label);
        assert!(
            joined.contains('╯') || joined.contains('╰'),
            "join elbow on stash^1: {joined}"
        );
        assert!(
            !stash_line.label.contains('◇'),
            "diamond is gutter-only: {}",
            stash_line.label
        );
    }

    #[test]
    fn paints_merge_open_and_join() {
        let lines = render_lines(&merge_model(), 80, 16, false);
        let joined = lines.join("\n");
        assert!(
            joined.contains('╮') || joined.contains('╭'),
            "open: {joined}"
        );
        assert!(
            joined.contains('╯') || joined.contains('╰'),
            "join: {joined}"
        );
        assert!(joined.contains("merge"), "{joined}");
        assert!(joined.contains("left"), "{joined}");
        assert!(joined.contains("right"), "{joined}");
        let painted = paint_model(&merge_model(), &UNICODE, None);
        let merge = painted
            .iter()
            .find(|l| l.label.contains("merge"))
            .expect("merge row");
        let gutter = cells_text(&merge.gutter);
        assert!(gutter.contains('╮'), "merge open corner: {gutter}");
        let root = painted
            .iter()
            .find(|l| l.label.contains("root"))
            .expect("root row");
        let root_g = cells_text(&root.gutter);
        assert!(root_g.contains('╯'), "root join: {root_g}");
    }

    #[test]
    fn paints_two_parent_join() {
        let lines = render_lines(&two_parent_join_model(), 80, 16, false);
        let joined = lines.join("\n");
        assert!(joined.contains("tip A"), "{joined}");
        assert!(joined.contains("tip B"), "{joined}");
        let painted = paint_model(&two_parent_join_model(), &UNICODE, None);
        let base = painted
            .iter()
            .find(|l| l.label.contains("shared parent"))
            .expect("base row");
        let gutter = cells_text(&base.gutter);
        assert!(
            gutter.contains('╯') || gutter.contains('┴'),
            "two-parent join: {gutter}"
        );
        let tip_a = painted.iter().find(|l| l.label.contains("tip A")).unwrap();
        let tip_b = painted.iter().find(|l| l.label.contains("tip B")).unwrap();
        let lane_a = tip_a
            .gutter
            .iter()
            .position(|c| c.role == crate::CellRole::Node)
            .unwrap();
        let lane_b = tip_b
            .gutter
            .iter()
            .position(|c| c.role == crate::CellRole::Node)
            .unwrap();
        assert_ne!(lane_a, lane_b, "sibling tips must keep distinct lanes");
    }

    #[test]
    fn hidden_ignored_worktree_is_omitted() {
        let lines = render_lines(&sample_model(), 120, 16, false);
        let joined = lines.join("\n");
        assert!(
            !joined.contains("notes"),
            "hidden ignored worktree leaked: {joined}"
        );
        assert!(
            !joined.contains("[ignored]"),
            "ignored mark leaked: {joined}"
        );
    }

    #[test]
    fn show_ignored_includes_hidden_worktree() {
        let mut model = sample_model();
        assert_eq!(model.dispatch(Action::SetShowIgnored(true)), Effect::None);
        let lines = render_lines(&model, 120, 16, false);
        let joined = lines.join("\n");
        assert!(joined.contains("notes"), "expected notes path: {joined}");
        assert!(
            joined.contains("[ignored]"),
            "expected ignored mark: {joined}"
        );
    }

    #[test]
    fn toggle_show_ignored_flips_visibility() {
        let mut model = sample_model();
        assert!(!model.show_ignored);
        assert_eq!(model.dispatch(Action::ToggleShowIgnored), Effect::None);
        assert!(model.show_ignored);
        let shown = model.visible_rows();
        assert!(
            shown.iter().any(|row| match row {
                crate::GraphRow::Commit { worktrees, .. } => {
                    worktrees.iter().any(|wt| wt.path == "notes")
                }
                _ => false,
            }),
            "toggle should attach the ignored worktree to HEAD"
        );
        model.dispatch(Action::ToggleShowIgnored);
        assert!(!model.show_ignored);
        let hidden = model.visible_rows();
        assert!(hidden.iter().all(|row| match row {
            crate::GraphRow::Commit { worktrees, .. } => {
                worktrees.iter().all(|wt| wt.path != "notes")
            }
            crate::GraphRow::Worktree(wt) => wt.path != "notes",
            _ => true,
        }));
    }

    #[test]
    fn ascii_glyphs_replace_unicode_nodes() {
        let lines = render_lines(&sample_model(), 120, 16, true);
        let joined = lines.join("\n");
        assert!(joined.contains("main ^1"), "{joined}");
        assert!(joined.contains("o uncommitted changes"), "{joined}");
        assert!(joined.contains('s'), "{joined}");
        assert!(joined.contains("stash@{0}"), "{joined}");
        assert!(joined.contains("@"), "{joined}");
        assert!(joined.contains("aaa1111"), "{joined}");
        assert!(joined.contains("*"), "{joined}");
        assert!(joined.contains("bbb2222"), "{joined}");
        assert!(joined.contains("L .worktrees/feature/graph"), "{joined}");
        assert!(!joined.contains("🔗"), "emoji desyncs the gutter: {joined}");
        assert!(!joined.contains("⊙"), "{joined}");
        assert!(!joined.contains("●"), "{joined}");
        let merge_lines = render_lines(&merge_model(), 80, 16, true);
        let merge_joined = merge_lines.join("\n");
        assert!(
            merge_joined.contains('\\') || merge_joined.contains('/'),
            "ASCII merge corners: {merge_joined}"
        );
        assert!(merge_joined.contains('*'), "{merge_joined}");
    }

    #[test]
    fn empty_model_renders_no_rows() {
        let lines = render_lines(&GraphModel::default(), 20, 2, false);
        assert!(lines.is_empty(), "{lines:?}");
    }

    #[test]
    fn gutter_cells_use_distinct_lane_colors() {
        let model = merge_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let colors = crate::default_lane_colors();
        let mut seen = std::collections::HashSet::new();
        for y in 0..16u16 {
            for x in 0..20u16 {
                let cell = &buffer[(x, y)];
                if cell.symbol() == " " || cell.symbol().is_empty() {
                    continue;
                }
                if let Color::Rgb(r, g, b) = cell.fg {
                    if colors.iter().any(
                        |c| matches!(c, Color::Rgb(cr, cg, cb) if *cr == r && *cg == g && *cb == b),
                    ) {
                        seen.insert((r, g, b));
                    }
                }
            }
        }
        assert!(
            seen.len() >= 2,
            "expected at least two lane colours on the merge gutter, got {seen:?}"
        );
    }

    #[test]
    fn paints_two_line_selection_footer() {
        let model = sample_model();
        let lines = render_lines(&model, 80, 16, false);
        let joined = lines.join("\n");
        assert!(
            joined.contains("no selection")
                || joined.contains("Uncommitted changes")
                || joined.contains("add graph crate"),
            "selection footer: {joined}"
        );
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut last = String::new();
        let mut prev = String::new();
        for y in 0..16u16 {
            let mut line = String::new();
            for x in 0..80u16 {
                line.push_str(buffer[(x, y)].symbol());
            }
            let trimmed = line.trim_end().to_string();
            if !trimmed.is_empty() {
                prev = last;
                last = trimmed;
            }
        }
        assert!(
            prev.contains("Uncommitted changes"),
            "footer subject: {prev}"
        );
        assert!(
            last.contains("main"),
            "uncommitted footer lists HEAD refs: {last}"
        );
        assert!(
            !last.contains("not a commit"),
            "HEAD has refs so fallback is unused: {last}"
        );
    }

    #[test]
    fn paints_clean_working_tree_row() {
        let mut model = sample_model();
        model.uncommitted = Some(false);
        let joined = render_lines(&model, 120, 16, false).join("\n");
        assert!(
            joined.contains("○ working tree clean"),
            "always-on clean row: {joined}"
        );
        assert!(
            !joined.contains("○ uncommitted changes"),
            "dirty label must not appear when clean: {joined}"
        );
    }

    #[test]
    fn paints_stash_selection_footer() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(1))
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut last = String::new();
        let mut prev = String::new();
        for y in 0..16u16 {
            let mut line = String::new();
            for x in 0..80u16 {
                line.push_str(buffer[(x, y)].symbol());
            }
            let trimmed = line.trim_end().to_string();
            if !trimmed.is_empty() {
                prev = last;
                last = trimmed;
            }
        }
        assert!(prev.contains("WIP on main"), "stash footer subject: {prev}");
        assert!(
            last.contains("stash@{0}")
                && last.contains("ccc3333")
                && last.contains(&format_local_timestamp(NOW - 86400)),
            "stash footer meta ref · hash · date: {last}"
        );
        assert!(!last.contains("Ada"), "stash footer has no author: {last}");
    }

    #[test]
    fn paints_loading_older_status() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .loading_older(true)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut joined = String::new();
        for y in 0..16u16 {
            for x in 0..80u16 {
                joined.push_str(buffer[(x, y)].symbol());
            }
            joined.push('\n');
        }
        assert!(
            joined.contains("loading older…"),
            "loading older status: {joined}"
        );
    }

    #[test]
    fn search_match_paints_bg_on_selectable_row_not_cursor() {
        let model = sample_model();
        let rows = model.visible_rows();
        let stash_idx = rows
            .iter()
            .position(|row| matches!(row, GraphRow::Stash(_)))
            .expect("stash row");
        let bg = Color::Rgb(187, 154, 247);
        let matches = [stash_idx];
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .search_matches(&matches, bg)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut stash_line_y = None;
        let mut stash_spacer_y = None;
        for y in 0..16u16 {
            let mut line = String::new();
            for x in 0..80u16 {
                line.push_str(buffer[(x, y)].symbol());
            }
            if line.contains("WIP on main") {
                stash_line_y = Some(y);
                stash_spacer_y = Some(y.saturating_add(1));
                break;
            }
        }
        let y = stash_line_y.expect("stash subject line");
        let mut saw_match_bg = false;
        for x in 0..80u16 {
            if buffer[(x, y)].bg == bg {
                saw_match_bg = true;
                break;
            }
        }
        assert!(
            saw_match_bg,
            "search match should paint filter bg on the stash node"
        );
        if let Some(spacer_y) = stash_spacer_y {
            let spacer_has_match = (0..80u16).any(|x| buffer[(x, spacer_y)].bg == bg);
            assert!(
                !spacer_has_match,
                "stash spacer is not a searchMatchIds target"
            );
        }
    }

    #[test]
    fn flash_paints_node_and_spacer_cursor_wins() {
        let model = sample_model();
        let rows = model.visible_rows();
        let stash_idx = rows
            .iter()
            .position(|row| matches!(row, GraphRow::Stash(_)))
            .expect("stash row");
        let flash = Color::Rgb(61, 82, 54);
        let cursor_bg = Color::Rgb(40, 52, 87);
        let flashes = [(stash_idx, flash)];
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(stash_idx))
                    .flash_rows(&flashes)
                    .cursor_style(Color::Cyan, cursor_bg)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut stash_line_y = None;
        let mut stash_spacer_y = None;
        for y in 0..16u16 {
            let mut line = String::new();
            for x in 0..80u16 {
                line.push_str(buffer[(x, y)].symbol());
            }
            if line.contains("WIP on main") {
                stash_line_y = Some(y);
                stash_spacer_y = Some(y.saturating_add(1));
                break;
            }
        }
        let y = stash_line_y.expect("stash subject line");
        assert!(
            (0..80u16).any(|x| buffer[(x, y)].bg == cursor_bg),
            "cursor background wins over flash on the node"
        );
        assert!(
            (0..80u16).all(|x| buffer[(x, y)].bg != flash),
            "flash must not paint over the cursor"
        );
        if let Some(spacer_y) = stash_spacer_y {
            assert!(
                (0..80u16).any(|x| buffer[(x, spacer_y)].bg == cursor_bg),
                "cursor background follows the stash spacer"
            );
        }

        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .flash_rows(&flashes)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut stash_line_y = None;
        let mut stash_spacer_y = None;
        for y in 0..16u16 {
            let mut line = String::new();
            for x in 0..80u16 {
                line.push_str(buffer[(x, y)].symbol());
            }
            if line.contains("WIP on main") {
                stash_line_y = Some(y);
                stash_spacer_y = Some(y.saturating_add(1));
                break;
            }
        }
        let y = stash_line_y.expect("stash subject line");
        assert!(
            (0..80u16).any(|x| buffer[(x, y)].bg == flash),
            "flash paints the stash node"
        );
        if let Some(spacer_y) = stash_spacer_y {
            assert!(
                (0..80u16).any(|x| buffer[(x, spacer_y)].bg == flash),
                "flash follows the stash spacer"
            );
        }
    }

    #[test]
    fn graph_scrollbar_thumb_matches_painted_handle() {
        let mut commits = Vec::new();
        for i in 0..24 {
            let id = format!("c{i:02}{}", "a".repeat(36));
            let parents = if i + 1 < 24 {
                vec![format!("c{:02}{}", i + 1, "a".repeat(36))]
            } else {
                Vec::new()
            };
            commits.push(Commit {
                id,
                subject: format!("commit {i}"),
                parents,
                refs: Vec::new(),
                author_name: "Ada".into(),
                author_date_unix: NOW - 3600,
            });
        }
        let head = commits[0].id.clone();
        let model = GraphModel {
            commits,
            head_id: Some(head),
            uncommitted: Some(false),
            window: 24,
            ..GraphModel::default()
        };
        let width = 40u16;
        let height = 16u16;
        let scroll = 8u16;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .ascii(true)
                    .scroll(scroll)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let chrome = graph_chrome_budget(height, false, false);
        let list_top = u16::from(chrome.header);
        let painted = paint_model_with(
            &model,
            &ASCII,
            PaintOpts {
                now_unix: Some(NOW),
                line_width: Some(width.saturating_sub(2) as usize),
                ..PaintOpts::default()
            },
        );
        let (thumb_off, thumb_len) =
            graph_scrollbar_thumb(painted.len(), scroll, chrome.list_height).expect("thumb");
        let sb_x = width.saturating_sub(1);
        for i in 0..chrome.list_height {
            let y = list_top.saturating_add(i);
            let cell = buffer[(sb_x, y)].symbol();
            let on_thumb = i >= thumb_off && i < thumb_off.saturating_add(thumb_len);
            if on_thumb {
                assert_eq!(cell, "█", "thumb cell at y={y} off={i}");
            } else {
                assert_ne!(cell, "█", "track must not paint thumb at y={y} off={i}");
            }
        }
    }

    fn tall_linear_model(n: usize) -> GraphModel {
        let mut commits = Vec::new();
        for i in 0..n {
            let id = format!("c{i:02}{}", "a".repeat(36));
            let parents = if i + 1 < n {
                vec![format!("c{:02}{}", i + 1, "a".repeat(36))]
            } else {
                Vec::new()
            };
            commits.push(Commit {
                id,
                subject: format!("commit {i}"),
                parents,
                refs: Vec::new(),
                author_name: "Ada".into(),
                author_date_unix: NOW - 3600,
            });
        }
        let head = commits[0].id.clone();
        GraphModel {
            commits,
            head_id: Some(head),
            uncommitted: Some(false),
            window: n,
            ..GraphModel::default()
        }
    }

    fn render_graph(
        model: &GraphModel,
        width: u16,
        height: u16,
        scroll: u16,
        col_offset: u16,
    ) -> Buffer {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(model)
                    .ascii(true)
                    .scroll(scroll)
                    .col_offset(col_offset)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        terminal.backend().buffer().clone()
    }

    #[test]
    fn vertical_scrollbar_hidden_at_top_shown_when_scrolled() {
        let model = tall_linear_model(24);
        let width = 40u16;
        let height = 16u16;
        let chrome = graph_chrome_budget(height, false, false);
        let at_top = render_graph(&model, width, height, 0, 0);
        let sb_x = width.saturating_sub(1);
        for i in 0..chrome.list_height {
            let y = u16::from(chrome.header).saturating_add(i);
            assert_ne!(
                at_top[(sb_x, y)].symbol(),
                "█",
                "no vertical thumb at top y={y}"
            );
        }
        let scrolled = render_graph(&model, width, height, 8, 0);
        let painted = paint_model_with(
            &model,
            &ASCII,
            PaintOpts {
                now_unix: Some(NOW),
                line_width: Some(width.saturating_sub(2) as usize),
                ..PaintOpts::default()
            },
        );
        let (thumb_off, thumb_len) =
            graph_scrollbar_thumb(painted.len(), 8, chrome.list_height).expect("thumb");
        let thumb_y = u16::from(chrome.header).saturating_add(thumb_off);
        assert_eq!(scrolled[(sb_x, thumb_y)].symbol(), "█");
        assert!(thumb_len >= 1);
    }

    #[test]
    fn horizontal_scrollbar_hidden_at_left_shown_when_panned() {
        let marker = "UNIQUE_GRAPH_TAIL_xyz";
        let subject = format!("{}{marker}", "n".repeat(60));
        let model = GraphModel {
            commits: vec![commit(
                "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                &subject,
                &[],
            )],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: Some(false),
            window: 1,
            ..GraphModel::default()
        };
        let width = 36u16;
        let height = 8u16;
        let chrome = graph_chrome_budget(height, false, false);
        let list_bottom = u16::from(chrome.header) + chrome.list_height;
        let h_y = list_bottom.saturating_sub(1);
        let at_left = render_graph(&model, width, height, 0, 0);
        let left_row: String = (0..width)
            .map(|x| at_left[(x, h_y)].symbol().to_string())
            .collect();
        assert!(
            !left_row.contains('█'),
            "no horizontal thumb at left edge: {left_row}"
        );
        let panned = render_graph(&model, width, height, 0, 50);
        let panned_row: String = (0..width)
            .map(|x| panned[(x, h_y)].symbol().to_string())
            .collect();
        assert!(
            panned_row.contains('█'),
            "panned viewport must paint a horizontal thumb: {panned_row}"
        );
    }

    #[test]
    fn zero_area_does_not_panic() {
        let model = sample_model();
        let mut buf = Buffer::empty(Rect::new(0, 0, 0, 0));
        GraphWidget::new(&model).render(buf.area, &mut buf);
    }

    #[test]
    fn standalone_ignored_worktree_without_commit() {
        let mut model = GraphModel {
            worktrees: vec![Worktree {
                path: "vendor/secret".into(),
                head_id: None,
                branch: None,
                ignored: true,
                is_current: false,
            }],
            ..GraphModel::default()
        };
        assert!(model.visible_rows().is_empty());
        model.show_ignored = true;
        let rows = model.visible_rows();
        assert_eq!(rows.len(), 1);
        let lines = render_lines(&model, 40, 3, false);
        assert!(
            lines
                .iter()
                .any(|l| l.contains("vendor/secret") && l.contains("[ignored]")),
            "{lines:?}"
        );
    }

    #[test]
    fn capped_gutter_keeps_rail_columns_when_labels_clip() {
        let mut model = merge_model();
        for commit in &mut model.commits {
            commit.subject = format!("{} {}", commit.subject, "n".repeat(48));
        }
        let painted = paint_model_with(
            &model,
            &UNICODE,
            PaintOpts {
                gutter_width: Some(2),
                line_width: Some(24),
                now_unix: Some(NOW),
            },
        );
        let gutters: Vec<&[crate::GraphCell]> = painted
            .iter()
            .filter(|l| !l.gutter.is_empty())
            .map(|l| l.gutter.as_slice())
            .collect();
        assert!(gutters.len() >= 4, "merge rows");
        let width = gutters[0].len();
        assert_eq!(width, 2);
        assert!(
            gutters.iter().all(|g| g.len() == width),
            "shared clip width"
        );
        let left = painted
            .iter()
            .find(|l| l.selectable && l.label.contains("left"))
            .expect("left");
        let right = painted
            .iter()
            .find(|l| l.selectable && l.label.contains("right"))
            .expect("right");
        assert_eq!(left.gutter[0].role, crate::CellRole::Node);
        assert_eq!(
            right.gutter[0].ch,
            UNICODE.vertical,
            "side-lane node must not steal column 0: {}",
            cells_text(&right.gutter)
        );
        assert_ne!(right.gutter[0].role, crate::CellRole::Node);

        let width = 36u16;
        let height = 12u16;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .gutter_width(2)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let chrome = graph_chrome_budget(height, false, false);
        let list_bottom = u16::from(chrome.header) + chrome.list_height;
        let mut spine_x: Option<u16> = None;
        for y in u16::from(chrome.header)..list_bottom {
            for x in 0..width.saturating_sub(1) {
                let sym = buffer[(x, y)].symbol();
                if sym == UNICODE.vertical || sym == UNICODE.commit || sym == UNICODE.head_commit {
                    match spine_x {
                        None => spine_x = Some(x),
                        Some(col) => {
                            assert_eq!(x, col, "rail/node at ({x},{y}) drifted from column {col}")
                        }
                    }
                    break;
                }
            }
        }
        assert!(spine_x.is_some(), "expected a spine column in the list");
        let joined = (0..height)
            .map(|y| {
                let mut line = String::new();
                for x in 0..width {
                    line.push_str(buffer[(x, y)].symbol());
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            !joined.contains("nnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnnn"),
            "narrow pane must clip long subjects:\n{joined}"
        );
    }

    #[test]
    fn gutter_width_caps_paint() {
        let model = merge_model();
        let backend = TestBackend::new(40, 10);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .gutter_width(2)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut line = String::new();
        for x in 0..40 {
            line.push_str(buffer[(x, 0)].symbol());
        }
        assert!(line.contains("merge"), "{line}");
    }

    #[test]
    fn selected_row_paints_cursor_bar() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .cursor_style(Color::Cyan, Color::DarkGray)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut saw_bar = false;
        let mut saw_reversed = false;
        for y in 0..16u16 {
            for x in 0..80u16 {
                let cell = &buffer[(x, y)];
                if cell.symbol() == "▌" {
                    saw_bar = true;
                }
                if cell.modifier.contains(Modifier::REVERSED) {
                    saw_reversed = true;
                }
            }
        }
        assert!(saw_bar, "selected graph row should paint ▌");
        assert!(!saw_reversed, "graph cursor should not use reverse video");
    }

    #[test]
    fn unselected_commented_row_paints_quote_mark() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .commented_rows(&[1, 2, 3, 4, 5])
                    .cursor_style(Color::Cyan, Color::DarkGray)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut saw_quote = false;
        let mut quote_on_cursor = false;
        for y in 0..16u16 {
            let mut has_quote = false;
            let mut has_bar = false;
            for x in 0..80u16 {
                let sym = buffer[(x, y)].symbol();
                if sym == "\"" {
                    has_quote = true;
                    saw_quote = true;
                }
                if sym == "▌" {
                    has_bar = true;
                }
            }
            if has_quote && has_bar {
                quote_on_cursor = true;
            }
        }
        assert!(saw_quote, "unselected commented graph row should paint \"");
        assert!(
            !quote_on_cursor,
            "unselected commented rows should not steal the cursor bar"
        );
    }

    #[test]
    fn selected_commented_row_keeps_bar_and_quote() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(1))
                    .commented_rows(&[1])
                    .cursor_style(Color::Cyan, Color::DarkGray)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut saw_both = false;
        for y in 0..16u16 {
            let mut has_quote = false;
            let mut has_bar = false;
            for x in 0..80u16 {
                let sym = buffer[(x, y)].symbol();
                if sym == "\"" {
                    has_quote = true;
                }
                if sym == "▌" {
                    has_bar = true;
                }
            }
            if has_quote && has_bar {
                saw_both = true;
                break;
            }
        }
        assert!(
            saw_both,
            "selected commented graph row should paint ▌ and \""
        );
    }

    #[test]
    fn uncommented_row_does_not_paint_quote_mark() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .cursor_style(Color::Cyan, Color::DarkGray)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        for y in 0..16u16 {
            for x in 0..80u16 {
                assert_ne!(
                    buffer[(x, y)].symbol(),
                    "\"",
                    "uncommented graph must not paint ICON_COMMENT"
                );
            }
        }
    }

    #[test]
    fn commented_row_uses_supplied_glyph() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .commented_rows(&[1])
                    .comment_glyph("\u{f075}")
                    .cursor_style(Color::Cyan, Color::DarkGray)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut saw_nerd = false;
        for y in 0..16u16 {
            for x in 0..80u16 {
                if buffer[(x, y)].symbol() == "\u{f075}" {
                    saw_nerd = true;
                    break;
                }
            }
        }
        assert!(
            saw_nerd,
            "commented graph row should paint the supplied glyph"
        );
    }

    #[test]
    fn resolved_comment_row_uses_resolved_glyph() {
        let model = sample_model();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .selected(Some(0))
                    .commented_rows(&[1])
                    .resolved_comment_rows(&[1, 2])
                    .resolved_comment_glyph("'")
                    .cursor_style(Color::Cyan, Color::DarkGray)
                    .now_unix(NOW)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut saw_open = false;
        let mut saw_resolved = false;
        for y in 0..16u16 {
            for x in 0..80u16 {
                match buffer[(x, y)].symbol() {
                    "\"" => saw_open = true,
                    "'" => saw_resolved = true,
                    _ => {}
                }
            }
        }
        assert!(
            saw_open,
            "open commented_rows must win when a row is also in resolved_comment_rows"
        );
        assert!(
            saw_resolved,
            "resolved-only graph row should paint the resolved glyph"
        );
    }

    fn many_ref_commit() -> Commit {
        let mut refs = vec![
            GraphRef::local("main"),
            GraphRef::local("feature/long-topic"),
        ];
        for i in 0..6 {
            refs.push(GraphRef::tag(format!("v1.{i}.0")));
        }
        Commit {
            id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            subject: "tag soup".into(),
            refs,
            author_name: "Ada Lovelace".into(),
            author_date_unix: NOW - 120,
            ..Commit::default()
        }
    }

    fn test_label_palette() -> GraphLabelPalette {
        GraphLabelPalette {
            subject: hex_color("#c0caf5"),
            meta: hex_color("#565f89"),
            branch_local: hex_color("#7aa2f7"),
            branch_default: hex_color("#9ece6a"),
            remote: hex_color("#7dcfff"),
            tag: hex_color("#e0af68"),
            head_mark: hex_color("#ff9e64"),
            overflow: hex_color("#7dcfff"),
        }
    }

    #[test]
    fn narrow_row_shows_overflow_chip_for_hidden_branches_and_tags() {
        let commit = many_ref_commit();
        let model = GraphModel {
            commits: vec![commit.clone()],
            head_id: Some(commit.id.clone()),
            uncommitted: Some(false),
            window: 1,
            ..GraphModel::default()
        };
        let width = 42u16;
        let painted = paint_model_with(
            &model,
            &crate::ASCII,
            PaintOpts {
                now_unix: Some(NOW),
                line_width: Some(width as usize),
                ..PaintOpts::default()
            },
        );
        let spacer = painted
            .iter()
            .find(|l| !l.selectable && l.label.contains("[main]"))
            .or_else(|| painted.iter().find(|l| !l.selectable))
            .expect("commit spacer");
        assert!(
            spacer.label.chars().count() <= width as usize,
            "row must not grow past pane: {}",
            spacer.label
        );
        let overflow = spacer
            .parts
            .iter()
            .find(|p| p.kind == crate::LabelKind::Overflow)
            .expect("overflow part");
        assert!(
            overflow.text.starts_with("[+") && overflow.text.ends_with(']'),
            "overflow chip: {}",
            overflow.text
        );
        assert!(
            !spacer.label.contains("[v1.5.0]") || !spacer.label.contains("[feature/long-topic]"),
            "some branch/tag chips must hide: {}",
            spacer.label
        );

        let row = GraphRow::Commit {
            commit: commit.clone(),
            is_head: true,
            worktrees: Vec::new(),
        };
        let [_, footer] = selection_detail_lines(
            &model,
            GraphFooterSelection::Row(&row),
            &crate::ASCII,
            200,
            NOW,
        );
        assert!(footer.contains("[main]"), "{footer}");
        assert!(footer.contains("[feature/long-topic]"), "{footer}");
        for i in 0..6 {
            assert!(
                footer.contains(&format!("[v1.{i}.0]")),
                "footer keeps tag v1.{i}.0: {footer}"
            );
        }
        assert!(
            !footer.contains(&overflow_chip_text(1)),
            "footer lists refs instead of collapsing them: {footer}"
        );
        assert!(
            footer.contains("[feature/long-topic]"),
            "pan/footer keep the full name: {footer}"
        );
    }

    #[test]
    fn narrow_row_truncates_last_visible_chip_without_overflow_count() {
        let commit = Commit {
            id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            subject: "topic".into(),
            refs: vec![
                GraphRef::local("main"),
                GraphRef::local("feature/long-name"),
            ],
            author_name: "Ada".into(),
            author_date_unix: NOW - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            uncommitted: Some(false),
            window: 1,
            ..GraphModel::default()
        };
        let width = 28u16;
        let painted = paint_model_with(
            &model,
            &crate::ASCII,
            PaintOpts {
                now_unix: Some(NOW),
                line_width: Some(width as usize),
                ..PaintOpts::default()
            },
        );
        let spacer = painted
            .iter()
            .find(|l| !l.selectable && l.label.contains("[main]"))
            .expect("commit spacer");
        assert!(
            spacer.label.contains('…') && spacer.label.contains("…]"),
            "next chip name truncates in brackets: {}",
            spacer.label
        );
        assert!(
            spacer
                .parts
                .iter()
                .all(|p| p.kind != crate::LabelKind::Overflow),
            "truncated last chip is not [+N]: {}",
            spacer.label
        );
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let [_, footer] = selection_detail_lines(
            &model,
            GraphFooterSelection::Row(&row),
            &crate::ASCII,
            200,
            NOW,
        );
        assert!(
            footer.contains("[feature/long-name]"),
            "footer lists the full name: {footer}"
        );
    }

    #[test]
    fn overflow_chip_uses_overflow_colour_not_muted_meta() {
        let commit = many_ref_commit();
        let model = GraphModel {
            commits: vec![commit],
            uncommitted: Some(false),
            window: 1,
            ..GraphModel::default()
        };
        let pal = test_label_palette();
        let backend = TestBackend::new(48, 10);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .ascii(true)
                    .now_unix(NOW)
                    .label_palette(pal)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let mut saw_overflow = false;
        let mut overflow_uses_meta = false;
        for y in 0..10u16 {
            let mut x = 0u16;
            while x < 48 {
                let cell = &buffer[(x, y)];
                if cell.symbol() != "[" {
                    x += 1;
                    continue;
                }
                let next = buffer[(x.saturating_add(1), y)].symbol();
                if next != "+" {
                    x += 1;
                    continue;
                }
                saw_overflow = true;
                assert_eq!(cell.fg, pal.overflow, "overflow chip colour");
                assert!(
                    cell.modifier.contains(Modifier::BOLD),
                    "overflow chip must be bold"
                );
                if cell.fg == pal.meta {
                    overflow_uses_meta = true;
                }
                x += 1;
            }
        }
        assert!(saw_overflow, "expected [+N] on the narrow graph row");
        assert!(
            !overflow_uses_meta,
            "overflow must not use muted meta colour"
        );
    }

    fn row_text_and_fg(buffer: &Buffer, y: u16, width: u16) -> (String, Vec<Color>) {
        let mut text = String::new();
        let mut colors = Vec::new();
        for x in 0..width {
            let cell = &buffer[(x, y)];
            let sym = cell.symbol();
            if sym.is_empty() {
                continue;
            }
            for _ in 0..sym.chars().count() {
                colors.push(cell.fg);
            }
            text.push_str(sym);
        }
        (text, colors)
    }

    fn fg_at(text: &str, colors: &[Color], needle: &str, offset: usize) -> Color {
        let i = text
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in {text:?}"));
        colors[i + offset]
    }

    #[test]
    fn footer_ref_chips_reuse_row_chip_palette() {
        let commit = Commit {
            id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            subject: "palette footer".into(),
            refs: vec![
                GraphRef::local("main"),
                GraphRef::local("topic"),
                GraphRef::remote("origin/other"),
                GraphRef::tag("v1"),
            ],
            author_name: "Ada".into(),
            author_date_unix: NOW - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            sync: Some(SyncState {
                branch: "main".into(),
                status: SyncStatus::UpToDate,
                ahead: 0,
                behind: 0,
            }),
            uncommitted: Some(false),
            window: 1,
            ..GraphModel::default()
        };
        let pal = test_label_palette();
        let width = 100u16;
        let height = 16u16;
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).expect("test backend");
        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .ascii(true)
                    .selected(Some(1))
                    .now_unix(NOW)
                    .label_palette(pal)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();

        let mut spacer: Option<(String, Vec<Color>)> = None;
        for y in 0..height.saturating_sub(2) {
            let row = row_text_and_fg(buffer, y, width);
            if row.0.contains("[+main]") && row.0.contains("[v1]") {
                spacer = Some(row);
                break;
            }
        }
        let (spacer_text, spacer_fg) = spacer.expect("commit spacer with chips");
        let (footer_text, footer_fg) = row_text_and_fg(buffer, height - 1, width);
        assert!(
            footer_text.contains("[+main]") && footer_text.contains("[topic]"),
            "footer lists branch chips: {footer_text}"
        );
        assert!(
            footer_text.contains("[v1]") && footer_text.contains("[origin/other]"),
            "footer lists tag and remote chips: {footer_text}"
        );

        let head = fg_at(&footer_text, &footer_fg, "[+main]", 1);
        let default = fg_at(&footer_text, &footer_fg, "[+main]", 2);
        let local = fg_at(&footer_text, &footer_fg, "[topic]", 1);
        let remote = fg_at(&footer_text, &footer_fg, "[origin/other]", 1);
        let tag = fg_at(&footer_text, &footer_fg, "[v1]", 1);
        assert_eq!(head, pal.head_mark, "HEAD checkout mark");
        assert_eq!(default, pal.branch_default, "default branch chip");
        assert_eq!(local, pal.branch_local, "feature branch chip");
        assert_eq!(remote, pal.remote, "remote chip");
        assert_eq!(tag, pal.tag, "tag chip");
        assert_ne!(head, default);
        assert_ne!(default, local);
        assert_ne!(local, tag);
        assert_ne!(tag, remote);
        assert_ne!(default, pal.meta);
        assert_eq!(
            head,
            fg_at(&spacer_text, &spacer_fg, "[+main]", 1),
            "footer HEAD mark must match the row chip"
        );
        assert_eq!(
            default,
            fg_at(&spacer_text, &spacer_fg, "[+main]", 2),
            "footer default branch must match the row chip"
        );
        assert_eq!(
            local,
            fg_at(&spacer_text, &spacer_fg, "[topic]", 1),
            "footer feature branch must match the row chip"
        );
        assert_eq!(
            tag,
            fg_at(&spacer_text, &spacer_fg, "[v1]", 1),
            "footer tag must match the row chip"
        );
        assert_eq!(
            remote,
            fg_at(&spacer_text, &spacer_fg, "[origin/other]", 1),
            "footer remote must match the row chip"
        );

        terminal
            .draw(|frame| {
                GraphWidget::new(&model)
                    .ascii(true)
                    .selected(Some(0))
                    .now_unix(NOW)
                    .label_palette(pal)
                    .render(frame.area(), frame.buffer_mut());
            })
            .expect("draw");
        let buffer = terminal.backend().buffer();
        let (wt_footer, wt_fg) = row_text_and_fg(buffer, height - 1, width);
        assert_eq!(
            fg_at(&wt_footer, &wt_fg, "[+main]", 1),
            pal.head_mark,
            "working-tree footer HEAD mark: {wt_footer}"
        );
        assert_eq!(
            fg_at(&wt_footer, &wt_fg, "[v1]", 1),
            pal.tag,
            "working-tree footer tag: {wt_footer}"
        );
    }

    fn merged_head_model() -> GraphModel {
        let id = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        GraphModel {
            commits: vec![Commit {
                id: id.into(),
                subject: "merged head chip".into(),
                refs: vec![GraphRef::local("main"), GraphRef::remote("origin/main")],
                author_name: "Ada".into(),
                author_date_unix: NOW - 120,
                ..Commit::default()
            }],
            head_id: Some(id.into()),
            sync: Some(SyncState {
                branch: "main".into(),
                status: SyncStatus::UpToDate,
                ahead: 0,
                behind: 0,
            }),
            worktrees: vec![Worktree {
                path: ".worktrees/recon".into(),
                head_id: Some(id.into()),
                branch: Some("main".into()),
                ignored: false,
                is_current: true,
            }],
            uncommitted: Some(false),
            window: 1,
            ..GraphModel::default()
        }
    }

    fn first_bracket_chip(line: &str) -> &str {
        let start = line
            .find('[')
            .unwrap_or_else(|| panic!("no chip in {line}"));
        let rel_end = line[start..]
            .find(']')
            .unwrap_or_else(|| panic!("unclosed chip in {line}"));
        &line[start..=start + rel_end]
    }

    #[test]
    fn merged_head_chip_paints_one_chip_matching_footer() {
        let model = merged_head_model();
        let pal = test_label_palette();
        let width = 80u16;
        let height = 12u16;

        let paint_chip = |ascii: bool, want: &str| {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test backend");
            terminal
                .draw(|frame| {
                    GraphWidget::new(&model)
                        .ascii(ascii)
                        .selected(Some(1))
                        .now_unix(NOW)
                        .label_palette(pal)
                        .render(frame.area(), frame.buffer_mut());
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            let mut spacer = None;
            for y in 0..height.saturating_sub(2) {
                let (text, _) = row_text_and_fg(buffer, y, width);
                if text.contains("aaa1111") && text.contains(want) {
                    spacer = Some(text);
                    break;
                }
            }
            let spacer = spacer.unwrap_or_else(|| {
                panic!("commit spacer must paint {want}");
            });
            let (footer, _) = row_text_and_fg(buffer, height - 1, width);
            assert!(footer.contains(want), "footer must paint {want}: {footer}");
            assert_eq!(
                first_bracket_chip(&spacer),
                first_bracket_chip(&footer),
                "painted spacer chip must equal footer chip\nspacer={spacer}\nfooter={footer}"
            );
            assert!(
                !spacer.contains(".worktrees"),
                "worktree path must not be a second chip on the row: {spacer}"
            );
            let wt_glyph = if ascii {
                ASCII.worktree
            } else {
                UNICODE.worktree
            };
            assert!(
                !spacer.contains(wt_glyph),
                "worktree glyph must not prefix the footer chip: {spacer}"
            );
            assert!(
                !spacer.contains("[]") && !spacer.contains("[]") && !spacer.contains("[=]"),
                "marks must not be separate chips on the row: {spacer}"
            );
            let painted = paint_model_with(
                &model,
                if ascii { &ASCII } else { &UNICODE },
                PaintOpts {
                    now_unix: Some(NOW),
                    line_width: Some(width as usize),
                    ..PaintOpts::default()
                },
            );
            let spacer_line = painted
                .iter()
                .find(|l| !l.selectable && l.label.contains(want))
                .expect("painted spacer");
            assert!(
                spacer_line.label.contains(want),
                "painted row label: {}",
                spacer_line.label
            );
            let mark_runs: Vec<&str> = spacer_line
                .parts
                .iter()
                .filter(|p| {
                    p.kind == crate::LabelKind::ChipHead || p.kind == crate::LabelKind::ChipRemote
                })
                .filter(|p| !p.text.contains('[') && !p.text.contains(']'))
                .map(|p| p.text.as_str())
                .collect();
            assert_eq!(
                mark_runs.len(),
                1,
                "checkout+sync must be one painted run: {:?}",
                spacer_line.parts
            );
        };

        paint_chip(false, "[main]");
        paint_chip(true, "[+=main]");
    }

    #[test]
    fn col_offset_reveals_clipped_subject() {
        let marker = "UNIQUE_GRAPH_TAIL_xyz";
        let subject = format!("{}{marker}", "n".repeat(60));
        let model = GraphModel {
            commits: vec![commit(
                "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
                &subject,
                &[],
            )],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: Some(false),
            window: 1,
            ..GraphModel::default()
        };
        let width = 36u16;
        let height = 8u16;
        let clipped = {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test backend");
            terminal
                .draw(|frame| {
                    GraphWidget::new(&model)
                        .ascii(true)
                        .now_unix(NOW)
                        .render(frame.area(), frame.buffer_mut());
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            (0..height)
                .map(|y| {
                    let mut line = String::new();
                    for x in 0..width {
                        line.push_str(buffer[(x, y)].symbol());
                    }
                    line
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(
            !clipped.contains(marker),
            "narrow pane must clip the subject tail: {clipped}"
        );
        let panned = {
            let backend = TestBackend::new(width, height);
            let mut terminal = Terminal::new(backend).expect("test backend");
            terminal
                .draw(|frame| {
                    GraphWidget::new(&model)
                        .ascii(true)
                        .now_unix(NOW)
                        .col_offset(50)
                        .render(frame.area(), frame.buffer_mut());
                })
                .expect("draw");
            let buffer = terminal.backend().buffer();
            (0..height)
                .map(|y| {
                    let mut line = String::new();
                    for x in 0..width {
                        line.push_str(buffer[(x, y)].symbol());
                    }
                    line
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert!(
            panned.contains(marker),
            "panning must reveal the clipped tail: {panned}"
        );
        assert!(
            panned
                .lines()
                .all(|l| l.chars().count() <= width as usize + 4),
            "panning must not grow the row past the pane"
        );
    }
}

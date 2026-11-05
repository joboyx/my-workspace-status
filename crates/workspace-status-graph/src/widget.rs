//! Ratatui [`Widget`] for [`GraphModel`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget, Widget};

use crate::chrome::{
    graph_chrome_budget_for_meta, graph_footer_request, selection_footer, GraphFooterSelection,
    SelectionFooter, FOOTER_RULE_ROWS,
};
use crate::footer::paint_footer_message;
use crate::format::{
    format_label, format_sync_parts, parts_text, slice_label_parts, wrap_label_parts_capped,
    LabelKind, LabelPart, PartTarget, COMMIT_MSG_LINES_DEFAULT, COMMIT_MSG_LINES_MAX,
    COMMIT_MSG_LINES_MIN,
};
use crate::glyphs::{ASCII, UNICODE};
use crate::gutter::graph_gutter_cap;
use crate::lane_colors::{default_lane_colors, lane_fg};
use crate::model::{GraphModel, GraphRow};
use crate::paint::{paint_model_with, painted_line_count, PaintOpts, PaintedLine};
use crate::topology::CellRole;

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
    /// Linked-worktree glyph on a worktree row or a spacer worktree mark.
    pub worktree: Color,
    /// Uncommitted glyph on a dirty working tree. A clean one is [`Self::meta`].
    pub dirty: Color,
    /// Open comment mark ([`GraphWidget::commented_rows`]).
    pub comment: Color,
    /// Resolved comment mark ([`GraphWidget::resolved_comment_rows`]).
    pub comment_resolved: Color,
    /// Sync header ahead count ([`LabelKind::SyncAhead`]).
    pub ahead: Color,
    /// Sync header behind count ([`LabelKind::SyncBehind`]).
    pub behind: Color,
}

/// What a painted graph icon is (see [`IconSpan`]).
///
/// Nodes are the selectable line's gutter cell with
/// [`crate::CellRole::Node`]. Rails, junctions, spacer cells, and blanks are
/// chrome and have no icon.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GraphIconKind {
    /// Commit node (`●` / `*`).
    CommitNode,
    /// HEAD commit node (`⊙` / `@`).
    HeadNode,
    /// Stash tip (`◇` / `s`).
    StashNode,
    /// Uncommitted-row glyph (`○` / `o`).
    Uncommitted,
    /// Linked-worktree glyph on a worktree row or a spacer worktree mark.
    Worktree,
    /// Open comment mark after the gutter.
    Comment,
    /// Resolved comment mark after the gutter.
    ResolvedComment,
    /// A [`GraphWidget::row_badges`] glyph.
    Badge,
    /// Local branch chip ([`crate::PartTarget::Chip`]).
    LocalChip,
    /// Default-branch chip, local or remote.
    DefaultChip,
    /// Remote-tracking chip with no local match.
    RemoteChip,
    /// Tag chip.
    TagChip,
    /// Checkout and sync marks inside a branch chip: one run
    /// ([`crate::PartTarget::Marks`]).
    ChipMarks,
    /// Detached `[HEAD]` chip.
    DetachedHeadChip,
    /// `[+N]` chip of hidden refs ([`crate::PartTarget::Overflow`]).
    OverflowChip,
    /// The sync header line: branch and ahead / behind marks.
    SyncHeader,
    /// The [`GraphWidget::header_chip`] run, flush right on the sync header
    /// line.
    HeaderChip,
    /// The selection footer's `more lines below` hint.
    MoreBelow,
    /// Short commit id on a commit / stash spacer or in the selection
    /// footer ([`crate::LabelKind::MetaHash`]). In the footer's parents
    /// group, [`IconSpan::part`] names the parent.
    CommitHash,
    /// Commit date on a commit / stash spacer or in the selection footer
    /// ([`crate::LabelKind::MetaDate`]).
    CommitDate,
    /// Commit author on a commit spacer or in the selection footer
    /// ([`crate::LabelKind::MetaAuthor`]).
    CommitAuthor,
}

/// Painted cells of one graph icon.
///
/// [`GraphWidget::render_with_icon_spans`] returns one per icon that is on
/// screen. Coordinates are absolute buffer cells.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IconSpan {
    /// What the icon is.
    pub kind: GraphIconKind,
    /// [`GraphModel::visible_rows`] index of the row. A spacer worktree
    /// mark or chip carries its commit row; a selection-footer chip or the
    /// footer hint carries the selected row. `None` for the sync header and
    /// its chip.
    pub row_index: Option<usize>,
    /// First painted column of the glyph.
    pub x: u16,
    /// Buffer row of the glyph.
    pub y: u16,
    /// Painted width of the glyph in columns.
    pub width: u16,
    /// Which part of the row the icon stands for, when a row can paint more
    /// than one of its kind: for [`GraphIconKind::Worktree`], the index into
    /// the commit row's `worktrees` (`0` on a worktree row); for
    /// [`GraphIconKind::CommitHash`], `Some(n)` is parent `n` of the commit
    /// (git parent order) and `None` the row's own id. `None` for the other
    /// kinds.
    pub part: Option<usize>,
    /// The chip a chip span stands for ([`LabelPart::target`]). `None` for
    /// the other kinds.
    pub target: Option<PartTarget>,
}

/// Blank columns between the sync header text and the
/// [`GraphWidget::header_chip`] run.
pub const HEADER_CHIP_GAP: usize = 2;

/// Node icon of `row`'s selectable line: commit, HEAD, or stash.
fn node_icon_kind(row: &GraphRow) -> Option<GraphIconKind> {
    match row {
        GraphRow::Commit { is_head: true, .. } => Some(GraphIconKind::HeadNode),
        GraphRow::Commit { .. } => Some(GraphIconKind::CommitNode),
        GraphRow::Stash(_) => Some(GraphIconKind::StashNode),
        GraphRow::Uncommitted { .. } | GraphRow::Worktree(_) => None,
    }
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
    lane_colors: &'a [Color],
    search_matches: &'a [usize],
    search_bg: Option<Color>,
    search_fg: Option<Color>,
    flash_rows: &'a [(usize, Color)],
    commented_rows: &'a [usize],
    /// Comment glyph (`ICON_COMMENT`: `"` / nf-fa-comment). Empty uses `"`.
    comment_glyph: &'a str,
    /// Selectable rows whose comments are all resolved.
    resolved_comment_rows: &'a [usize],
    /// Glyph for [`Self::resolved_comment_rows`]. Empty uses `'`.
    resolved_comment_glyph: &'a str,
    /// `(row index, glyph, colour)` painted at the row's right edge.
    row_badges: &'a [(usize, &'a str, Color)],
    /// Right-aligned run on the sync header line: candidates, widest first.
    header_chip: &'a [Line<'a>],
    cursor_fg: Color,
    cursor_bg: Option<Color>,
    cursor_inactive_fg: Color,
    cursor_inactive_bg: Option<Color>,
    /// When true, paint focused `▌` / `cursorBg`. When false, paint the
    /// thinner unfocused marker and `cursorBgInactive`. [`Self::selected`]
    /// still drives the selection footer.
    cursor_bar: bool,
    label_palette: Option<GraphLabelPalette>,
    col_offset: u16,
    /// Wrap the selection-footer message (subject + body). List rows stay
    /// one line. Session flag from the TUI (`M`).
    commit_msg_expand: bool,
    /// First painted message line when an expanded message is taller than
    /// the footer. Clamped at paint.
    commit_msg_scroll: usize,
    /// Fixed message rows of the expanded footer (see
    /// [`Self::commit_msg_lines`]).
    commit_msg_lines: usize,
    /// Footer rule glyph (see [`Self::footer_rule`]). `None` uses `▁`
    /// (`_` in ASCII mode).
    footer_rule_glyph: Option<&'a str>,
    /// Footer rule colour (see [`Self::footer_rule`]).
    footer_rule_color: Color,
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
            lane_colors: &[],
            search_matches: &[],
            search_bg: None,
            search_fg: None,
            flash_rows: &[],
            commented_rows: &[],
            comment_glyph: "\"",
            resolved_comment_rows: &[],
            resolved_comment_glyph: "'",
            row_badges: &[],
            header_chip: &[],
            cursor_fg: Color::Cyan,
            cursor_bg: None,
            cursor_inactive_fg: Color::Gray,
            cursor_inactive_bg: None,
            cursor_bar: true,
            label_palette: None,
            col_offset: 0,
            commit_msg_expand: false,
            commit_msg_scroll: 0,
            commit_msg_lines: COMMIT_MSG_LINES_DEFAULT,
            footer_rule_glyph: None,
            footer_rule_color: Color::DarkGray,
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

    /// Per-cell gutter colours. Empty uses [`crate::DEFAULT_LANE_COLORS`].
    pub fn lane_colors(mut self, colors: &'a [Color]) -> Self {
        self.lane_colors = colors;
        self
    }

    /// Paint search-match rows on selectable graph rows: `bg` behind the
    /// row and `fg` on its label, so the text stays readable whatever
    /// colour the label had.
    ///
    /// `indices` are [`GraphModel::visible_rows`] indexes. Spacers stay
    /// unhighlighted. [`Self::selected`] still wins over a match.
    pub fn search_matches(mut self, indices: &'a [usize], bg: Color, fg: Color) -> Self {
        self.search_matches = indices;
        self.search_bg = Some(bg);
        self.search_fg = Some(fg);
        self
    }

    /// Paint flash background on graph rows (node + spacers).
    ///
    /// `rows` are [`GraphModel::visible_rows`] indexes with the fade colour.
    /// Flash background wins over cursor and search. The cursor bar (`▌`)
    /// still marks the selected node and its spacer. Unlike search,
    /// spacers follow the node.
    pub fn flash_rows(mut self, rows: &'a [(usize, Color)]) -> Self {
        self.flash_rows = rows;
        self
    }

    /// Mark commented selectable rows (`ICON_COMMENT` after the gutter).
    ///
    /// `indices` are [`GraphModel::visible_rows`] indexes. Selected rows keep
    /// `▌`. The comment glyph stays visible on the selected row. Spacers
    /// stay unmarked. Uncommented rows do not reserve a column. With a
    /// [`Self::label_palette`] the mark paints in
    /// [`GraphLabelPalette::comment`] (resolved:
    /// [`GraphLabelPalette::comment_resolved`]); without one it paints in
    /// the bold cursor colour.
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

    /// Paint a badge glyph at the right edge of selectable rows.
    ///
    /// Each entry is a [`GraphModel::visible_rows`] index, the glyph (one
    /// column), and its colour; the glyph paints bold. The badge sits flush
    /// right in the label area (left of the vertical scrollbar column). The
    /// label gets the columns left of it less one space and clips first, so
    /// a narrow pane never cuts the badge. [`Self::col_offset`] pans the
    /// label only; the badge stays put. The gutter is not touched. Spacers
    /// and rows not listed get no badge and reserve no column. A
    /// search-match row paints the glyph in the search foreground, like its
    /// label. The [`GraphIconKind::Badge`] spans of
    /// [`Self::render_with_icon_spans`] tell where each badge landed.
    pub fn row_badges(mut self, badges: &'a [(usize, &'a str, Color)]) -> Self {
        self.row_badges = badges;
        self
    }

    /// Paint a run flush right on the sync header line, in its own styles.
    ///
    /// `candidates` go widest first. The first one that fits right of the
    /// branch and sync marks, with [`HEADER_CHIP_GAP`] blank columns
    /// between, paints so that it ends at the right edge of the area; the
    /// sync text keeps its columns. None fits, or no sync header shows: no
    /// run. The run does not pan. Its [`GraphIconKind::HeaderChip`] span of
    /// [`Self::render_with_icon_spans`] tells where it landed; its width
    /// tells which candidate.
    pub fn header_chip(mut self, candidates: &'a [Line<'a>]) -> Self {
        self.header_chip = candidates;
        self
    }

    /// Cursor bar (`▌`) plus `cursorBg`. The selected row's spacer (commit
    /// hash and chips, stash meta) gets the bar and the background too.
    pub fn cursor_style(mut self, fg: Color, bg: Color) -> Self {
        self.cursor_fg = fg;
        self.cursor_bg = Some(bg);
        self
    }

    /// Paint focused `▌` / `cursorBg` when true (default).
    ///
    /// False still marks [`Self::selected`] with the thinner unfocused
    /// marker and [`Self::cursor_inactive_style`]. The selection footer
    /// still tracks the cursor.
    pub fn cursor_bar(mut self, on: bool) -> Self {
        self.cursor_bar = on;
        self
    }

    /// Unfocused selected-row colours. Used when [`Self::cursor_bar`] is off.
    pub fn cursor_inactive_style(mut self, fg: Color, bg: Color) -> Self {
        self.cursor_inactive_fg = fg;
        self.cursor_inactive_bg = Some(bg);
        self
    }

    /// Colour commit subjects, meta, ref chips, the hidden-ref overflow chip,
    /// and the matching chip runs on the selection footer.
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

    /// Expand the selection footer to the full commit / stash message.
    ///
    /// Graph list rows stay one line. Default is collapsed (truncated
    /// subject, then the meta rows pinned at the bottom).
    pub fn commit_msg_expand(mut self, expand: bool) -> Self {
        self.commit_msg_expand = expand;
        self
    }

    /// Scroll an expanded footer message that does not fit the footer.
    ///
    /// `offset` is the first painted message line. The meta rows stay
    /// pinned at the bottom and a scrollbar marks the position. Clamped to
    /// [`SelectionFooter::scroll_max`]. Default is `0`.
    pub fn commit_msg_scroll(mut self, offset: usize) -> Self {
        self.commit_msg_scroll = offset;
        self
    }

    /// Fixed message rows of the expanded selection footer.
    ///
    /// The footer requests `n` message rows plus one meta row
    /// ([`graph_footer_request`]) whatever the selected message is: a short
    /// message leaves blank rows, a longer one scrolls. A meta line that
    /// wraps adds its extra rows on top and takes them from the list
    /// ([`crate::graph_chrome_budget_for_meta`]). Clamped to
    /// [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`]. Default is
    /// [`COMMIT_MSG_LINES_DEFAULT`]. The collapsed footer ignores it.
    pub fn commit_msg_lines(mut self, n: usize) -> Self {
        self.commit_msg_lines = n.clamp(COMMIT_MSG_LINES_MIN, COMMIT_MSG_LINES_MAX);
        self
    }

    /// Glyph and colour of the rule on the selection footer's first row,
    /// repeated across the pane width.
    ///
    /// The rule parts the footer from the commit list; it is one of the
    /// footer rows ([`crate::FOOTER_RULE_ROWS`]). `glyph` is one column
    /// wide; the TUI passes `▁` (`_` in ASCII mode) and its `border_dim`
    /// colour. Default is `▁` (`_` with [`Self::ascii`]) in dark grey.
    pub fn footer_rule(mut self, glyph: &'a str, color: Color) -> Self {
        self.footer_rule_glyph = Some(glyph);
        self.footer_rule_color = color;
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

/// Vertical graph scrollbar is painted whenever `content_len` painted lines
/// overflow a `viewport`-row list, at the top too, so clipped rows always
/// show a bar.
pub fn graph_vscroll_visible(content_len: usize, viewport: u16, scroll: u16) -> bool {
    scroll > 0 || content_len > usize::from(viewport)
}

/// Horizontal graph scrollbar is painted only after the viewport leaves the
/// left edge.
pub fn graph_hscroll_visible(col_offset: u16) -> bool {
    col_offset > 0
}

/// Max `col_offset` so the longest label can sit in the viewport.
///
/// `vscroll` is true when the 1-column vertical bar is reserved. `badges`
/// are the [`GraphWidget::row_badges`] entries (the colour is not read): a
/// badged row's label viewport loses the glyph and one space at the right
/// edge, so pan can bring the end of its label in view left of the badge.
pub fn graph_col_max(
    model: &GraphModel,
    ascii: bool,
    pane_width: u16,
    vscroll: bool,
    badges: &[(usize, &str, Color)],
) -> usize {
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
    model
        .visible_rows()
        .iter()
        .enumerate()
        .map(|(index, row)| {
            let badge = badges
                .iter()
                .find(|(idx, _, _)| *idx == index)
                .map(|(_, glyph, _)| Span::raw(*glyph).width())
                .filter(|w| *w > 0)
                .map_or(0, |w| w + 1);
            let viewport = label_viewport.saturating_sub(badge).max(1);
            format_label(row, glyphs)
                .chars()
                .count()
                .saturating_sub(viewport)
        })
        .max()
        .unwrap_or(0)
}

impl Widget for GraphWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        self.render_with_icon_spans(area, buf);
    }
}

impl GraphWidget<'_> {
    /// Paint like [`Widget::render`] and return the painted cells of every
    /// graph icon that is on screen, in paint order (top to bottom: the
    /// sync header and its chip, then per list line: node, comment mark,
    /// label glyphs and chips left to right, the badge at the right edge;
    /// then the selection footer's chips and its more-lines hint).
    ///
    /// Icons are the selectable line's node cell, the comment mark, the
    /// uncommitted and worktree glyphs (worktree rows and spacer worktree
    /// marks), [`Self::row_badges`], the ref chips of commit spacers and of
    /// the selection footer (one span per chip run, see
    /// [`LabelPart::target`]), the sync header, [`Self::header_chip`], and
    /// the footer hint. Rails and junctions are not icons.
    /// An icon scrolled out of the list, cut by [`Self::col_offset`],
    /// clipped by the gutter cap or the pane width, or under the horizontal
    /// scrollbar row is left out. The badge does not pan, so the offset
    /// never cuts it. Callers use the spans for mouse hit tests.
    pub fn render_with_icon_spans(self, area: Rect, buf: &mut Buffer) -> Vec<IconSpan> {
        let mut icon_spans = Vec::new();
        if area.width == 0 || area.height == 0 {
            return icon_spans;
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let default_colors = default_lane_colors();
        let lane_colors: &[Color] = if self.lane_colors.is_empty() {
            &default_colors
        } else {
            self.lane_colors
        };
        let fallback = Color::Reset;
        let now = self.now_unix.unwrap_or_else(now_unix_secs);
        let rows = self.model.visible_rows();
        let selected = self.selected.and_then(|i| rows.get(i));
        let footer_sel = GraphFooterSelection::from(selected);
        let footer_width = usize::from(area.width);
        let footer = selection_footer(
            self.model,
            footer_sel,
            glyphs,
            footer_width,
            now,
            self.commit_msg_expand,
        );
        let chrome = graph_chrome_budget_for_meta(
            area.height,
            self.model.sync.is_some(),
            graph_footer_request(self.commit_msg_expand, self.commit_msg_lines),
            u16::try_from(footer.meta_rows(footer_width)).unwrap_or(u16::MAX),
        );
        // The painted line count does not depend on the width, so the bar
        // is decided before the one paint at the width it leaves.
        let vscroll = graph_vscroll_visible(
            painted_line_count(self.model),
            chrome.list_height,
            self.scroll,
        );
        let hscroll = graph_hscroll_visible(self.col_offset);
        let v_cols = u16::from(vscroll);
        let pane = area.width.saturating_sub(v_cols) as usize;
        let cap = match self.gutter_width {
            Some(w) => w as usize,
            None => graph_gutter_cap(pane.max(1)),
        };
        // Cursor column plus the optional scrollbar.
        let line_width = area.width.saturating_sub(1 + v_cols) as usize;
        let painted = paint_model_with(
            self.model,
            glyphs,
            PaintOpts {
                gutter_width: Some(cap),
                line_width: Some(line_width.max(1)),
                now_unix: self.now_unix,
            },
        );
        let mut y = area.y;
        let list_bottom = area
            .y
            .saturating_add(u16::from(chrome.header) + chrome.list_height);

        if chrome.header {
            if let Some(sync) = &self.model.sync {
                let parts =
                    format_sync_parts(sync, glyphs, self.model.default_branch_override.as_deref());
                put_parts_line(
                    buf,
                    area.x,
                    y,
                    area.width,
                    &parts,
                    self.label_palette,
                    fallback,
                );
                let sync_width = Span::raw(parts_text(&parts)).width();
                let width = u16::try_from(sync_width)
                    .unwrap_or(u16::MAX)
                    .min(area.width);
                if width > 0 {
                    icon_spans.push(IconSpan {
                        kind: GraphIconKind::SyncHeader,
                        row_index: None,
                        x: area.x,
                        y,
                        width,
                        part: None,
                        target: None,
                    });
                }
                let room = usize::from(area.width).saturating_sub(sync_width + HEADER_CHIP_GAP);
                if let Some(chip) = self.header_chip.iter().find(|chip| chip.width() <= room) {
                    // `chip.width() <= room < area.width`, so it fits a u16.
                    let chip_w = chip.width() as u16;
                    let chip_x = area.right().saturating_sub(chip_w);
                    chip.clone().render(Rect::new(chip_x, y, chip_w, 1), buf);
                    if chip_w > 0 {
                        icon_spans.push(IconSpan {
                            kind: GraphIconKind::HeaderChip,
                            row_index: None,
                            x: chip_x,
                            y,
                            width: chip_w,
                            part: None,
                            target: None,
                        });
                    }
                }
                y = y.saturating_add(1);
            }
        }

        let skip = self.scroll as usize;
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
            let node = line
                .row_index
                .filter(|_| line.selectable)
                .and_then(|i| rows.get(i))
                .and_then(node_icon_kind);
            let flash_bg = line.row_index.and_then(|i| {
                self.flash_rows
                    .iter()
                    .find(|(idx, _)| *idx == i)
                    .map(|(_, color)| *color)
            });
            let badge = line
                .row_index
                .filter(|_| line.selectable)
                .and_then(|i| self.row_badges.iter().find(|(idx, _, _)| *idx == i))
                .map(|(_, glyph, color)| (*glyph, *color));
            let icons = put_painted_line(
                buf,
                area.x,
                y,
                area.width.saturating_sub(v_cols),
                line,
                RowFlags {
                    selected,
                    cursor_bar: self.cursor_bar,
                    search_match,
                    commented,
                    comment_resolved: resolved_comment,
                    node,
                },
                RowColors {
                    search_bg: self.search_bg,
                    search_fg: self.search_fg,
                    flash_bg,
                    cursor_fg: self.cursor_fg,
                    cursor_bg: self.cursor_bg,
                    cursor_inactive_fg: self.cursor_inactive_fg,
                    cursor_inactive_bg: self.cursor_inactive_bg,
                    lane_colors,
                    fallback,
                    palette: self.label_palette,
                },
                self.col_offset,
                if resolved_comment {
                    self.resolved_comment_glyph
                } else {
                    self.comment_glyph
                },
                badge,
            );
            if let Some(row_index) = line.row_index {
                icon_spans.extend(icons.into_iter().map(|icon| IconSpan {
                    kind: icon.kind,
                    row_index: Some(row_index),
                    x: icon.x,
                    y,
                    width: icon.width,
                    part: icon.part,
                    target: icon.target,
                }));
            }
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
            let max = graph_col_max(self.model, self.ascii, area.width, vscroll, self.row_badges);
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
                // The bar paints over the last list row and hides its icons.
                icon_spans.retain(|span| span.y != sb_area.y);
            }
        }

        if chrome.footer && chrome.footer_height > 0 {
            let footer_y = area
                .y
                .saturating_add(area.height)
                .saturating_sub(chrome.footer_height);
            let default_rule = if self.ascii { "_" } else { "\u{2581}" };
            let rule = self.footer_rule_glyph.unwrap_or(default_rule);
            put_text_line(
                buf,
                area.x,
                footer_y,
                area.width,
                &rule.repeat(usize::from(area.width)),
                false,
                self.footer_rule_color,
            );
            let body = chrome.footer_body_height();
            if body > 0 {
                let body_area = Rect {
                    x: area.x,
                    y: footer_y.saturating_add(FOOTER_RULE_ROWS),
                    width: area.width,
                    height: body,
                };
                icon_spans.extend(paint_selection_footer(
                    buf,
                    body_area,
                    &footer,
                    self.commit_msg_scroll,
                    glyphs.more_below,
                    self.label_palette,
                    self.selected,
                ));
            }
        }
        icon_spans
    }
}

/// Paint a selection-footer body (the rows under the rule) into `area`.
///
/// `footer` is [`selection_footer`]: message lines and the whole meta line.
/// The meta line wraps at `area.width` onto the meta rows, pinned at the
/// bottom of the area; at most `area.height` meta rows paint, and a meta
/// line that needs more ends its last row with `…`. Message lines fill the
/// rows above the meta rows from the top; a short message leaves blank
/// rows. A taller message scrolls by `scroll`, gets a 1-column scrollbar,
/// and ends its last visible row with a `<arrow><K>` hint (K hidden lines
/// below) in the palette's meta colour, left of the scrollbar. The message
/// rows paint through [`paint_footer_message`]. Each part paints in its
/// [`GraphLabelPalette`] colour (`Color::Reset` with no palette).
///
/// The graph selection footer ([`GraphWidget`]) and the app's commit-files
/// footer both paint here, so the same commit paints the same rows, chips,
/// and hint on every surface. Returns the chip and commit meta (hash,
/// parent ids, author, date) spans of the painted lines ([`part_icons`]),
/// each on the meta row it paints on, and the hint span
/// ([`GraphIconKind::MoreBelow`]), each with `row_index`.
pub fn paint_selection_footer(
    buf: &mut Buffer,
    area: Rect,
    footer: &SelectionFooter,
    scroll: usize,
    arrow: &str,
    palette: Option<GraphLabelPalette>,
    row_index: Option<usize>,
) -> Vec<IconSpan> {
    let fallback = Color::Reset;
    let mut spans = Vec::new();
    if area.height == 0 {
        return spans;
    }
    let meta = wrap_label_parts_capped(
        &footer.meta,
        usize::from(area.width),
        usize::from(area.height),
    );
    // `meta` has 1..=area.height rows, so this fits a u16.
    let meta_h = meta.len() as u16;
    let lines = &footer.message;
    let mut put = |buf: &mut Buffer, y: u16, line: &[LabelPart]| {
        put_parts_line(buf, area.x, y, area.width, line, palette, fallback);
        spans.extend(
            part_icons(line, area.x, area.width)
                .into_iter()
                .map(|icon| IconSpan {
                    kind: icon.kind,
                    row_index,
                    x: icon.x,
                    y,
                    width: icon.width,
                    part: icon.part,
                    target: icon.target,
                }),
        );
    };
    // Message rows: the body less the pinned meta rows.
    let message_area = Rect {
        height: area.height - meta_h,
        ..area
    };
    let hint_fg = palette.map_or(fallback, |pal| pal.meta);
    let painted = paint_footer_message(
        buf,
        message_area,
        lines.len(),
        scroll,
        arrow,
        hint_fg,
        |buf, i, y| put(buf, y, &lines[i]),
    );
    let meta_top = area.bottom() - meta_h;
    for (i, row) in meta.iter().enumerate() {
        put(buf, meta_top + i as u16, row);
    }
    // The span leaves out the hint's leading blank.
    spans.extend(painted.more_below.map(|hint| IconSpan {
        kind: GraphIconKind::MoreBelow,
        row_index,
        x: hint.x,
        y: hint.y,
        width: hint.width,
        part: None,
        target: None,
    }));
    spans
}

fn now_unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Per-row state for [`put_painted_line`].
///
/// These were positional `bool`s in a row, so a transposed pair at the
/// call site compiled and painted the wrong chrome.
struct RowFlags {
    /// Line belongs to the selected row: its node line or its spacer.
    selected: bool,
    cursor_bar: bool,
    search_match: bool,
    commented: bool,
    /// Every comment on the row is resolved (with [`Self::commented`]).
    comment_resolved: bool,
    /// Node icon of the selectable line, if its row has one.
    node: Option<GraphIconKind>,
}

/// One icon [`put_painted_line`] painted on its line.
struct LineIcon {
    kind: GraphIconKind,
    part: Option<usize>,
    x: u16,
    width: u16,
    /// [`IconSpan::target`].
    target: Option<PartTarget>,
}

/// Colours [`put_painted_line`] paints a row with. All come from the widget.
struct RowColors<'a> {
    search_bg: Option<Color>,
    search_fg: Option<Color>,
    flash_bg: Option<Color>,
    cursor_fg: Color,
    cursor_bg: Option<Color>,
    cursor_inactive_fg: Color,
    cursor_inactive_bg: Option<Color>,
    lane_colors: &'a [Color],
    fallback: Color,
    palette: Option<GraphLabelPalette>,
}

/// Paint one graph line. Returns the icons painted on screen, in paint
/// order: the node cell, the comment mark, the label glyphs, the badge.
fn put_painted_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    line: &PaintedLine,
    flags: RowFlags,
    colors: RowColors<'_>,
    col_offset: u16,
    comment_glyph: &str,
    badge: Option<(&str, Color)>,
) -> Vec<LineIcon> {
    let RowFlags {
        selected,
        cursor_bar,
        search_match,
        commented,
        comment_resolved,
        node,
    } = flags;
    let RowColors {
        search_bg,
        search_fg,
        flash_bg,
        cursor_fg,
        cursor_bg,
        cursor_inactive_fg,
        cursor_inactive_bg,
        lane_colors,
        fallback,
        palette,
    } = colors;
    let mut icons = Vec::new();
    if width == 0 {
        return icons;
    }
    let row = Rect::new(x, y, width, 1);
    // Spacers keep their row's index, so the bar marks both lines of the
    // selected commit or stash and no other spacer.
    let bar = if !selected {
        " "
    } else if cursor_bar {
        "▌"
    } else {
        "▏"
    };
    let row_bg = if flash_bg.is_some() {
        flash_bg
    } else if selected && cursor_bar {
        cursor_bg
    } else if selected {
        cursor_inactive_bg
    } else if search_match {
        search_bg
    } else {
        None
    };
    // A search-match row paints its rails and every label part in the search
    // foreground: chip and lane colours can equal the search background.
    let match_fg = (flash_bg.is_none() && !selected && search_match)
        .then_some(search_fg)
        .flatten();
    let mut bar_style = if cursor_bar {
        Style::default().fg(cursor_fg).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(cursor_inactive_fg)
    };
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
        let fg = match_fg.unwrap_or_else(|| lane_fg(cell.color_lane, lane_colors, fallback));
        let mut style = Style::default().fg(fg);
        if let Some(bg) = row_bg {
            style = style.bg(bg);
        }
        // One GraphCell = one buffer column. Overwrite any wide-glyph
        // continuation so rails stay put when the label clips.
        buf[(col, y)].set_symbol(&cell.ch);
        buf[(col, y)].set_style(style);
        if let Some(kind) = node.filter(|_| cell.role == CellRole::Node) {
            icons.push(LineIcon {
                kind,
                part: None,
                x: col,
                width: 1,
                target: None,
            });
        }
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
        // The mark colour can equal the search background, so a match row
        // paints the mark in the search foreground like its rails and label.
        // With a label palette it matches the tree's comment mark (heading
        // open, muted resolved); without one it is the bold cursor colour.
        let mut mark_style = match palette {
            Some(pal) => Style::default().fg(match_fg.unwrap_or(if comment_resolved {
                pal.comment_resolved
            } else {
                pal.comment
            })),
            None => Style::default()
                .fg(match_fg.unwrap_or(cursor_fg))
                .add_modifier(Modifier::BOLD),
        };
        if let Some(bg) = row_bg {
            mark_style = mark_style.bg(bg);
        }
        buf[(col, y)].set_symbol(glyph);
        buf[(col, y)].set_style(mark_style);
        icons.push(LineIcon {
            kind: if comment_resolved {
                GraphIconKind::ResolvedComment
            } else {
                GraphIconKind::Comment
            },
            part: None,
            x: col,
            width: u16::try_from(Span::raw(glyph).width()).unwrap_or(1),
            target: None,
        });
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
    let room = end.saturating_sub(col);
    // The badge sits flush right; the label gets the room left of it and
    // one space, so a narrow pane clips the label, never the badge.
    let badge = badge
        .map(|(glyph, color)| (fit_badge_glyph(glyph, room), color))
        .filter(|((glyph, _), _)| !glyph.is_empty());
    let label_w = match badge {
        Some(((_, glyph_w), _)) => room.saturating_sub(glyph_w.saturating_add(1)),
        None => room,
    };
    if label_w > 0 {
        let sliced = slice_line_label(line, col_offset as usize, label_w as usize);
        let mut spans = label_spans(&sliced, palette, fallback);
        if let Some(fg) = match_fg {
            for span in &mut spans {
                span.style = span.style.fg(fg);
            }
        }
        let painted = Line::from(spans);
        let painted_w = u16::try_from(painted.width())
            .unwrap_or(u16::MAX)
            .min(label_w);
        painted
            .style(row_style)
            .render(Rect::new(col, y, label_w, 1), buf);
        icons.extend(label_icons(
            line,
            &sliced,
            col,
            col_offset as usize,
            painted_w,
        ));
    }
    let Some(((glyph, glyph_w), color)) = badge else {
        return icons;
    };
    let glyph_x = end.saturating_sub(glyph_w);
    // The space and any label cells the label left blank keep the row
    // background (selection, flash, search).
    for gap_x in col.saturating_add(label_w)..glyph_x {
        buf[(gap_x, y)].set_symbol(" ");
        buf[(gap_x, y)].set_style(row_style);
    }
    let mut style = Style::default()
        .fg(match_fg.unwrap_or(color))
        .add_modifier(Modifier::BOLD);
    if let Some(bg) = row_bg {
        style = style.bg(bg);
    }
    buf.set_stringn(glyph_x, y, glyph, usize::from(glyph_w), style);
    icons.push(LineIcon {
        kind: GraphIconKind::Badge,
        part: None,
        x: glyph_x,
        width: glyph_w,
        target: None,
    });
    icons
}

/// The tail of a badge `glyph` that fits in `room` columns, and its width.
///
/// A badge too wide for the room drops chars from the left, so its right
/// edge stays put. An empty or zero-width glyph comes back empty.
fn fit_badge_glyph(glyph: &str, room: u16) -> (&str, u16) {
    let mut rest = glyph;
    loop {
        let w = u16::try_from(Span::raw(rest).width()).unwrap_or(u16::MAX);
        if w <= room {
            return if w == 0 { ("", 0) } else { (rest, w) };
        }
        let mut chars = rest.chars();
        chars.next();
        rest = chars.as_str();
    }
}

/// Icons among the label parts of `line` that are wholly on screen.
///
/// `sliced` is the label as painted from column `col`: `line` cut by
/// `offset` columns and `painted_w` wide. A glyph cut by the offset or the
/// pane edge is left out.
fn label_icons(
    line: &PaintedLine,
    sliced: &PaintedLine,
    col: u16,
    offset: usize,
    painted_w: u16,
) -> Vec<LineIcon> {
    let mut icons = Vec::new();
    let mut start = 0;
    for part in &line.parts {
        let chars = part.text.chars().count();
        let at = start;
        start += chars;
        let Some((kind, part_id)) = part.kind.icon() else {
            continue;
        };
        if at < offset || chars == 0 {
            continue;
        }
        let before: String = sliced.label.chars().take(at - offset).collect();
        let x = u16::try_from(Span::raw(before).width()).unwrap_or(u16::MAX);
        let width = u16::try_from(Span::raw(part.text.as_str()).width()).unwrap_or(u16::MAX);
        if width == 0 || x.saturating_add(width) > painted_w {
            continue;
        }
        icons.push(LineIcon {
            kind,
            part: part_id,
            x: col.saturating_add(x),
            width,
            target: None,
        });
    }
    icons.extend(chip_icons(&sliced.parts, col, painted_w));
    icons.sort_by_key(|icon| icon.x);
    icons
}

/// Icon spans of a footer line's `parts` painted from column `x`, at most
/// `width` columns: one per part with a kind icon
/// ([`LabelKind::icon`]: commit hash, date, author), then the
/// [`chip_icons`], sorted by column. A part cut by the pane edge keeps a
/// span over its visible cells, like a chip.
fn part_icons(parts: &[LabelPart], x: u16, width: u16) -> Vec<LineIcon> {
    let mut icons = Vec::new();
    let mut at: u16 = 0;
    for part in parts {
        let w = u16::try_from(Span::raw(part.text.as_str()).width()).unwrap_or(u16::MAX);
        let start = at;
        at = at.saturating_add(w);
        let Some((kind, part_id)) = part.kind.icon() else {
            continue;
        };
        let w = w.min(width.saturating_sub(start));
        if w == 0 {
            continue;
        }
        icons.push(LineIcon {
            kind,
            part: part_id,
            x: x.saturating_add(start),
            width: w,
            target: None,
        });
    }
    icons.extend(chip_icons(parts, x, width));
    icons.sort_by_key(|icon| icon.x);
    icons
}

/// Chip spans of `parts` painted from column `x`, at most `width` columns.
///
/// One span per run of adjacent parts with the same target and icon kind,
/// so a chip is one span and the checkout / sync marks inside it are their
/// own. A chip cut by the pane edge keeps a span over its visible cells.
fn chip_icons(parts: &[LabelPart], x: u16, width: u16) -> Vec<LineIcon> {
    let mut icons: Vec<LineIcon> = Vec::new();
    let mut at: u16 = 0;
    for part in parts {
        let w = u16::try_from(Span::raw(part.text.as_str()).width()).unwrap_or(u16::MAX);
        let start = at;
        at = at.saturating_add(w);
        let Some(target) = part.target.as_ref() else {
            continue;
        };
        let w = w.min(width.saturating_sub(start));
        if w == 0 {
            continue;
        }
        let kind = target.icon(part.kind);
        match icons.last_mut() {
            Some(last)
                if last.kind == kind
                    && last.target.as_ref() == Some(target)
                    && last.x.saturating_add(last.width) == x.saturating_add(start) =>
            {
                last.width = last.width.saturating_add(w);
            }
            _ => icons.push(LineIcon {
                kind,
                part: None,
                x: x.saturating_add(start),
                width: w,
                target: Some(target.clone()),
            }),
        }
    }
    icons
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
        LabelKind::Meta
        | LabelKind::MetaHash { .. }
        | LabelKind::MetaDate
        | LabelKind::MetaAuthor => pal.meta,
        LabelKind::ChipHead => pal.head_mark,
        LabelKind::ChipLocal => pal.branch_local,
        LabelKind::ChipDefault => pal.branch_default,
        LabelKind::ChipRemote => pal.remote,
        LabelKind::ChipTag => pal.tag,
        LabelKind::Overflow => pal.overflow,
        LabelKind::UncommittedMark { dirty: true } => pal.dirty,
        LabelKind::UncommittedMark { dirty: false } => pal.meta,
        LabelKind::WorktreeMark { .. } => pal.worktree,
        LabelKind::SyncAhead => pal.ahead,
        LabelKind::SyncBehind => pal.behind,
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
                match p.kind {
                    LabelKind::Overflow => style = style.add_modifier(Modifier::BOLD),
                    // Italic is a modifier, not a colour: it stays under
                    // NO_COLOR, and terminals without italics show plain text.
                    LabelKind::MetaDate | LabelKind::MetaAuthor => {
                        style = style.add_modifier(Modifier::ITALIC);
                    }
                    _ => {}
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
mod tests;

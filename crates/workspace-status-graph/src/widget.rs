//! Ratatui [`Widget`] for [`GraphModel`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::chrome::{
    graph_chrome_budget, selection_detail_lines, GraphFooterSelection, LOADING_OLDER,
};
use crate::format::format_sync;
use crate::glyphs::{ASCII, UNICODE};
use crate::lane_colors::{cells_to_spans, default_lane_colors};
use crate::model::GraphModel;
use crate::paint::{paint_model_with, PaintOpts, PaintedLine};

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
        }
    }

    /// Use ASCII glyphs when `ascii` is true.
    pub fn ascii(mut self, ascii: bool) -> Self {
        self.ascii = ascii;
        self
    }

    /// Cap the gutter at `width` columns. Topology still uses the full
    /// lane model; paint clips to this width.
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

    /// Paint Ink `searchMatchIds` background on selectable graph rows.
    ///
    /// `indices` are [`GraphModel::visible_rows`] indexes. Spacers stay
    /// unhighlighted. [`Self::selected`] still wins over a match.
    pub fn search_matches(mut self, indices: &'a [usize], bg: Color) -> Self {
        self.search_matches = indices;
        self.search_bg = Some(bg);
        self
    }
}

impl Widget for GraphWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let cap = self.gutter_width.map(|w| w as usize);
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
        let painted = paint_model_with(
            self.model,
            glyphs,
            PaintOpts {
                gutter_width: cap,
                line_width: Some(area.width as usize),
                now_unix: self.now_unix,
            },
        );
        for line in painted.into_iter().skip(skip) {
            if y >= list_bottom {
                break;
            }
            let selected = self.selected.is_some() && line.row_index == self.selected;
            let search_match = line.selectable
                && line
                    .row_index
                    .is_some_and(|i| self.search_matches.contains(&i));
            put_painted_line(
                buf,
                area.x,
                y,
                area.width,
                &line,
                selected,
                search_match,
                self.search_bg,
                lane_colors,
                fallback,
            );
            y = y.saturating_add(1);
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
            let [line1, line2] = selection_detail_lines(
                self.model,
                GraphFooterSelection::from(selected),
                glyphs,
                area.width as usize,
                now,
            );
            put_text_line(buf, area.x, footer_y, area.width, &line1, false, fallback);
            put_text_line(
                buf,
                area.x,
                footer_y.saturating_add(1),
                area.width,
                &line2,
                false,
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
    search_bg: Option<Color>,
    lane_colors: &[Color],
    fallback: Color,
) {
    let mut spans: Vec<Span> = Vec::new();
    if !line.gutter.is_empty() {
        spans.extend(cells_to_spans(&line.gutter, lane_colors, fallback));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(line.label.clone()));
    let mut style = Style::default();
    if selected {
        style = style.add_modifier(Modifier::REVERSED);
    } else if search_match {
        if let Some(bg) = search_bg {
            style = style.bg(bg);
        }
    }
    Line::from(spans)
        .style(style)
        .render(Rect::new(x, y, width, 1), buf);
}

fn put_text_line(
    buf: &mut Buffer,
    x: u16,
    y: u16,
    width: u16,
    text: &str,
    selected: bool,
    fg: Color,
) {
    let mut style = Style::default().fg(fg);
    if selected {
        style = style.add_modifier(Modifier::REVERSED);
    }
    Line::from(Span::styled(text.to_string(), style)).render(Rect::new(x, y, width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::action::Effect;
    use crate::model::{Commit, GraphRow, Stash, SyncState, SyncStatus, Worktree};
    use crate::paint::paint_model;
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
        assert!(spacer.label.contains("1d"), "{}", spacer.label);
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
        assert!(last.contains("worktree"), "footer meta: {last}");
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
            last.contains("stash@{0}") && last.contains("ccc3333") && last.contains("1d"),
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
}

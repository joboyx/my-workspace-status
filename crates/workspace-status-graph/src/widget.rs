//! Ratatui [`Widget`] for [`GraphModel`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

use crate::format::format_sync;
use crate::glyphs::{ASCII, UNICODE};
use crate::model::GraphModel;
use crate::paint::paint_model;

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
}

impl Widget for GraphWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let cap = self.gutter_width.map(|w| w as usize);
        let mut y = area.y;
        let bottom = area.y.saturating_add(area.height);

        if let Some(sync) = &self.model.sync {
            if y < bottom {
                put_line(buf, area.x, y, area.width, &format_sync(sync, glyphs), false);
                y = y.saturating_add(1);
            }
        }

        let skip = self.scroll as usize;
        for line in paint_model(self.model, glyphs, cap).into_iter().skip(skip) {
            if y >= bottom {
                break;
            }
            let selected = self.selected.is_some() && line.row_index == self.selected;
            put_line(buf, area.x, y, area.width, &line.text(), selected);
            y = y.saturating_add(1);
        }
    }
}

fn put_line(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str, selected: bool) {
    let style = if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };
    Line::from(Span::styled(text.to_string(), style)).render(Rect::new(x, y, width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::action::Effect;
    use crate::model::{Commit, Stash, SyncState, SyncStatus, Worktree};
    use crate::topology::cells_text;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn commit(id: &str, subject: &str, parents: &[&str]) -> Commit {
        Commit {
            id: id.into(),
            subject: subject.into(),
            parents: parents.iter().map(|p| (*p).to_string()).collect(),
            refs: Vec::new(),
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
                },
                commit(parent, "prior commit", &[]),
            ],
            stashes: vec![Stash {
                stash_ref: "stash@{0}".into(),
                subject: "WIP on main".into(),
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
            show_ignored: false,
            uncommitted: true,
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
        assert!(joined.contains("○ dirty"), "uncommitted: {joined}");
        assert!(joined.contains("◇"), "stash diamond: {joined}");
        assert!(joined.contains("stash@{0}"), "stash ref: {joined}");
        assert!(joined.contains("WIP on main"), "stash subject: {joined}");
        assert!(joined.contains("aaa1111"), "{joined}");
        assert!(joined.contains("[HEAD]"), "{joined}");
        assert!(joined.contains(".worktrees/feature/graph"), "{joined}");
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
            .position(|l| l.contains("[HEAD]"))
            .expect("head row");
        assert!(stash < head, "stash {stash} should sit above HEAD {head}");
    }

    #[test]
    fn stash_leaf_sits_on_spur_not_fake_lane() {
        let model = sample_model();
        let lines = render_lines(&model, 120, 16, false);
        let joined = lines.join("\n");
        let painted = paint_model(&model, &UNICODE, None);
        let stash_line = painted
            .iter()
            .find(|l| l.label.contains("stash@{0}"))
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
            .skip_while(|l| !l.label.contains("stash@{0}"))
            .nth(1)
            .expect("stash spacer");
        assert_eq!(
            spacer.gutter[diamond_idx].ch,
            UNICODE.vertical,
            "spacer must carry the short spur, got {}",
            cells_text(&spacer.gutter)
        );
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
        assert!(joined.contains('╮') || joined.contains('╭'), "open: {joined}");
        assert!(joined.contains('╯') || joined.contains('╰'), "join: {joined}");
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
        assert!(joined.contains("[ignored]"), "expected ignored mark: {joined}");
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
        assert!(joined.contains("o dirty"), "{joined}");
        assert!(joined.contains('s'), "{joined}");
        assert!(joined.contains("stash@{0}"), "{joined}");
        assert!(joined.contains("@"), "{joined}");
        assert!(joined.contains("aaa1111"), "{joined}");
        assert!(joined.contains("*"), "{joined}");
        assert!(joined.contains("bbb2222"), "{joined}");
        assert!(joined.contains("wt .worktrees/feature/graph"), "{joined}");
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
        let lines = render_lines(&GraphModel::default(), 20, 4, false);
        assert!(lines.is_empty(), "{lines:?}");
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

//! Ratatui [`Widget`] for [`GraphModel`].

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Widget;

use crate::format::{format_row, format_sync};
use crate::glyphs::{ASCII, UNICODE};
use crate::model::GraphModel;

/// Renderable git-graph widget.
///
/// Paint with [`Widget::render`]. Tests use a `Buffer` or `TestBackend`.
/// The widget does not read a TTY.
#[derive(Clone, Copy, Debug)]
pub struct GraphWidget<'a> {
    model: &'a GraphModel,
    ascii: bool,
}

impl<'a> GraphWidget<'a> {
    /// Build a widget over `model`. Unicode glyphs are the default.
    pub fn new(model: &'a GraphModel) -> Self {
        Self {
            model,
            ascii: false,
        }
    }

    /// Use ASCII glyphs when `ascii` is true.
    pub fn ascii(mut self, ascii: bool) -> Self {
        self.ascii = ascii;
        self
    }
}

impl Widget for GraphWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        let mut y = area.y;
        let bottom = area.y.saturating_add(area.height);

        if let Some(sync) = &self.model.sync {
            if y < bottom {
                put_line(buf, area.x, y, area.width, &format_sync(sync, glyphs));
                y = y.saturating_add(1);
            }
        }

        for row in self.model.visible_rows() {
            if y >= bottom {
                break;
            }
            put_line(buf, area.x, y, area.width, &format_row(&row, glyphs));
            y = y.saturating_add(1);
        }
    }
}

fn put_line(buf: &mut Buffer, x: u16, y: u16, width: u16, text: &str) {
    Line::from(text).render(Rect::new(x, y, width, 1), buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::action::Action;
    use crate::action::Effect;
    use crate::model::{Commit, Stash, SyncState, SyncStatus, Worktree};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn commit(id: &str, subject: &str) -> Commit {
        Commit {
            id: id.into(),
            subject: subject.into(),
            parents: Vec::new(),
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
                commit(parent, "prior commit"),
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
        let lines = render_lines(&sample_model(), 120, 8, false);
        let joined = lines.join("\n");
        assert!(joined.contains("main ↑1"), "sync header: {joined}");
        assert!(joined.contains("○ dirty"), "uncommitted: {joined}");
        assert!(
            joined.contains("◇ stash@{0}  WIP on main"),
            "stash: {joined}"
        );
        assert!(joined.contains("⊙ aaa1111  add graph crate  [HEAD]"), "{joined}");
        assert!(joined.contains(".worktrees/feature/graph"), "{joined}");
        assert!(joined.contains("● bbb2222  prior commit"), "{joined}");
    }

    #[test]
    fn stash_sits_above_parent_commit() {
        let lines = render_lines(&sample_model(), 120, 8, false);
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
    fn hidden_ignored_worktree_is_omitted() {
        let lines = render_lines(&sample_model(), 120, 8, false);
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
        let lines = render_lines(&model, 120, 8, false);
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
        let lines = render_lines(&sample_model(), 120, 8, true);
        let joined = lines.join("\n");
        assert!(joined.contains("main ^1"), "{joined}");
        assert!(joined.contains("o dirty"), "{joined}");
        assert!(joined.contains("s stash@{0}"), "{joined}");
        assert!(joined.contains("@ aaa1111"), "{joined}");
        assert!(joined.contains("* bbb2222"), "{joined}");
        assert!(joined.contains("wt .worktrees/feature/graph"), "{joined}");
        assert!(!joined.contains("⊙"), "{joined}");
        assert!(!joined.contains("●"), "{joined}");
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
            lines.iter().any(|l| l.contains("vendor/secret") && l.contains("[ignored]")),
            "{lines:?}"
        );
    }
}

//! Ratatui paint for the tree, graph / diff pane, status, and help overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;
use workspace_status_graph::GraphWidget;

use std::time::Instant;

use super::state::{AppState, FocusPane};
use super::tree::NodeKind;
use super::watch::flash_active;

/// Draw one frame. Updates `state.layout` for mouse hits.
pub fn draw(frame: &mut Frame<'_>, state: &mut AppState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(42), Constraint::Percentage(58)])
        .split(chunks[0]);

    let tree_block = Block::default()
        .borders(Borders::ALL)
        .title(if state.focus == FocusPane::Left {
            " tree "
        } else {
            " tree"
        })
        .border_style(pane_border(state.focus == FocusPane::Left));
    let tree_inner = tree_block.inner(panes[0]);
    frame.render_widget(tree_block, panes[0]);
    draw_tree(frame, tree_inner, state);

    let right_title = if state.right_is_diff() {
        if state.focus == FocusPane::Right {
            " diff "
        } else {
            " diff"
        }
    } else if state.focus == FocusPane::Right {
        " graph "
    } else {
        " graph"
    };
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(right_title)
        .border_style(pane_border(state.focus == FocusPane::Right));
    let right_inner = right_block.inner(panes[1]);
    frame.render_widget(right_block, panes[1]);
    draw_right(frame, right_inner, state);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            state.status.clone(),
            Style::default().fg(Color::DarkGray),
        ))),
        chunks[1],
    );

    state.layout.tree_x = tree_inner.x;
    state.layout.tree_y = tree_inner.y;
    state.layout.tree_width = tree_inner.width;
    state.layout.tree_height = tree_inner.height;
    state.layout.right_x = panes[1].x;

    if state.help_open {
        draw_help(frame, area);
    } else if state.stash_menu.is_some() {
        draw_stash_menu(frame, area, state);
    } else if state.create_branch.is_some() {
        draw_create_branch(frame, area, state);
    } else if state.branch_picker.is_some() {
        draw_branch_picker(frame, area, state);
    }
}

fn pane_border(focused: bool) -> Style {
    if focused {
        Style::default().fg(Color::Cyan)
    } else {
        Style::default().fg(Color::DarkGray)
    }
}

fn draw_tree(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let height = area.height as usize;
    let cursor = state.cursor;
    let start = if cursor >= height {
        cursor + 1 - height
    } else {
        0
    };
    state.layout.list_offset = start;
    let mut lines = Vec::new();
    for (i, row) in state.rows.iter().enumerate().skip(start).take(height) {
        let chevron = if row.foldable {
            if row.folded {
                if state.ascii { ">" } else { "▸" }
            } else if state.ascii {
                "v"
            } else {
                "▾"
            }
        } else {
            " "
        };
        let mark = if row.kind == NodeKind::File && state.reviewed.contains(&row.id) {
            if state.ascii { "* " } else { "◉ " }
        } else if row.id == "group:no-updates" {
            if state.ascii { ". " } else { "✓ " }
        } else {
            ""
        };
        let indent = "  ".repeat(row.depth);
        let text = format!("{indent}{chevron} {mark}{}", row.label);
        let mut style = Style::default();
        if i == cursor {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let flashing = state
            .flashes
            .get(&row.id)
            .is_some_and(|at| flash_active(Instant::now().saturating_duration_since(*at)));
        if flashing && i != cursor {
            style = style.fg(Color::LightYellow).add_modifier(Modifier::BOLD);
        } else if row.ignored {
            style = style.fg(Color::DarkGray);
        } else if row.kind == NodeKind::File {
            style = style.fg(Color::Yellow);
        } else if row.kind == NodeKind::Group {
            style = style.fg(Color::DarkGray);
        }
        lines.push(Line::from(Span::styled(text, style)));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn draw_right(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if state.right_is_diff() {
        if state.diff_lines.is_empty() {
            frame.render_widget(Paragraph::new("select a dirty file"), area);
            return;
        }
        let skip = state.diff_scroll as usize;
        let lines: Vec<Line> = state
            .diff_lines
            .iter()
            .skip(skip)
            .take(area.height as usize)
            .map(|line| Line::from(Span::styled(line.clone(), diff_style(line))))
            .collect();
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }
    if let Some(model) = &state.graph {
        GraphWidget::new(model)
            .ascii(state.ascii)
            .render(area, frame.buffer_mut());
        return;
    }
    frame.render_widget(
        Paragraph::new("focus a repo for the graph, or a file for its diff"),
        area,
    );
}

fn diff_style(line: &str) -> Style {
    if line.starts_with('+') && !line.starts_with("+++") {
        Style::default().fg(Color::Green)
    } else if line.starts_with('-') && !line.starts_with("---") {
        Style::default().fg(Color::Red)
    } else if line.starts_with("@@") {
        Style::default().fg(Color::Cyan)
    } else if line == "staged" || line == "unstaged" || line.starts_with("untracked") {
        Style::default().fg(Color::Magenta)
    } else {
        Style::default()
    }
}

fn draw_help(frame: &mut Frame<'_>, area: Rect) {
    let width = 48.min(area.width.saturating_sub(4));
    let height = 19.min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width, height);
    frame.render_widget(Clear, rect);
    let body = "\
q  quit                 ?  close this help
j/k  move               arrows  same
z  fold                 h/l  close / open
.  show ignored         space  mark reviewed
/  search               n/N  next / prev
s  stage                u  unstage
x  revert (y/n)         e  edit
f  fetch                p  pull behind
d  default branch       r  refresh
P  push                 S  stash menu
b  branch picker        C  create (in picker)
W  remove worktree      Tab  other pane
click  select row";
    frame.render_widget(
        Paragraph::new(body)
            .block(Block::default().borders(Borders::ALL).title(" keys "))
            .wrap(Wrap { trim: false }),
        rect,
    );
}

fn overlay_rect(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2)).max(10);
    let height = height.min(area.height.saturating_sub(2)).max(4);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect::new(x, y, width, height)
}

fn draw_stash_menu(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(ops) = state.stash_menu.as_ref() else {
        return;
    };
    let height = (ops.len() as u16).saturating_add(4).min(12);
    let rect = overlay_rect(area, 48, height);
    frame.render_widget(Clear, rect);
    let mut lines = vec![Line::from(Span::styled(
        " Stash ",
        Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
    ))];
    for op in ops {
        let detail = op
            .stash_ref
            .as_deref()
            .unwrap_or("");
        let text = if detail.is_empty() {
            format!(" {}  {}", op.key, op.label)
        } else {
            format!(" {}  {}  {}", op.key, op.label, detail)
        };
        lines.push(Line::from(text));
    }
    lines.push(Line::from(Span::styled(
        " Esc cancel",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" stash "))
            .wrap(Wrap { trim: false }),
        rect,
    );
}

fn draw_branch_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(picker) = state.branch_picker.as_ref() else {
        return;
    };
    let visible = picker.visible();
    let height = (visible.len() as u16).saturating_add(5).min(16);
    let rect = overlay_rect(area, 52, height);
    frame.render_widget(Clear, rect);
    let mut lines = vec![Line::from(format!(
        " Branch {}  filter: {}",
        picker.repo,
        if picker.filter.is_empty() {
            "…".into()
        } else {
            picker.filter.clone()
        }
    ))];
    if visible.is_empty() {
        lines.push(Line::from("  No matching branches"));
    } else {
        for (i, branch) in visible.iter().enumerate() {
            let mark = if branch.current { "* " } else { "  " };
            let cursor = if i == picker.cursor { "❯ " } else { "  " };
            lines.push(Line::from(format!("{cursor}{mark}{}", branch.name)));
        }
    }
    lines.push(Line::from(Span::styled(
        " j/k move · type to filter · Enter checkout · C create · Esc close",
        Style::default().fg(Color::DarkGray),
    )));
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(" branch "))
            .wrap(Wrap { trim: false }),
        rect,
    );
}

fn draw_create_branch(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(create) = state.create_branch.as_ref() else {
        return;
    };
    let rect = overlay_rect(area, 48, 6);
    frame.render_widget(Clear, rect);
    let body = format!(
        " Create branch\n  name: {}\n Enter confirm · Esc cancel",
        if create.name.is_empty() {
            "…"
        } else {
            create.name.as_str()
        }
    );
    frame.render_widget(
        Paragraph::new(body)
            .block(Block::default().borders(Borders::ALL).title(" create "))
            .wrap(Wrap { trim: false }),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::tui::state::AppState;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;
    use workspace_status_graph::{Commit, GraphModel};

    fn repo(name: &str, dirty: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            has_unstaged: dirty,
            has_staged: false,
            has_untracked: false,
            changes: if dirty {
                vec![FileChange {
                    path: "README.md".into(),
                    staged_status: None,
                    unstaged_status: Some("M".into()),
                    untracked: false,
                    old_path: None,
                }]
            } else {
                vec![]
            },
            checkout_kind: crate::snapshot::CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
        }
    }

    fn buffer_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        let area = buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn paints_tree_and_graph() {
        let snapshot = build_workspace_snapshot(
            &[repo("app", true), repo("lib", false)],
            &[],
            false,
            &[],
        );
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: "seed".into(),
                parents: Vec::new(),
                refs: vec!["main".into()],
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: true,
            ..GraphModel::default()
        });
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut state))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("app"), "{text}");
        assert!(text.contains("README.md"), "{text}");
        assert!(text.contains("seed") || text.contains("dirty") || text.contains("aaa1111"), "{text}");
    }

    #[test]
    fn help_overlay_is_short() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| draw(frame, &mut state))
            .unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("q  quit"), "{text}");
        assert!(text.contains(".  show ignored"), "{text}");
        assert!(text.contains("S  stash menu"), "{text}");
        assert!(text.contains("P  push"), "{text}");
        assert!(text.contains("W  remove worktree"), "{text}");
        assert!(!text.contains("EasyMotion"), "{text}");
    }
}

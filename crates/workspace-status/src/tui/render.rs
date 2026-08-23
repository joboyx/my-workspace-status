//! Ratatui paint for the tree, graph / diff pane, status, and help overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use ratatui::Frame;
use workspace_status_graph::{paint_model, GraphWidget, ASCII, UNICODE};

use std::time::Instant;

use super::diff::{
    cell_code_width, cell_sign, diff_pane_header, diff_pane_mode_label, gutter_width,
    section_header, DiffCell, DiffCellKind, DiffRow, DiffSection, DIFF_RULE,
};
use super::drill::DrillView;
use super::easy_motion::{easy_motion_labels, visible_window};
use super::icons::{
    truncate_visible, CURSOR_BAR, FOLD_COLLAPSED, FOLD_COLLAPSED_ASCII, FOLD_EXPANDED,
    FOLD_EXPANDED_ASCII,
};
use super::search::slice_visible;
use super::split::{
    diff_split_rule_x, effective_diff_mode, is_side_by_side_split, pane_widths,
    side_by_side_column_widths, MIN_PANE_COLS,
};
use super::state::{AppState, FocusPane};
use super::theme::{hex_color, Palette};
use super::tree::{row_segments, NodeKind, SegRole, TextSeg, VisibleRow};
use super::watch::flash_active;
use crate::helpers::visible_width;

/// Draw one frame. Updates `state.layout` for mouse hits.
pub fn draw(frame: &mut Frame<'_>, state: &mut AppState) {
    let area = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(3), Constraint::Length(1)])
        .split(area);
    let widths = pane_widths(area.width, state.tree_fraction);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(widths.tree_width),
            Constraint::Min(MIN_PANE_COLS),
        ])
        .split(chunks[0]);

    let left_is_graph = state.in_commit_drill();
    let left_title = if left_is_graph {
        if state.focus == FocusPane::Left {
            " graph "
        } else {
            " graph"
        }
    } else if state.focus == FocusPane::Left {
        " tree "
    } else {
        " tree"
    };
    let tree_block = Block::default()
        .borders(Borders::ALL)
        .title(left_title)
        .border_style(pane_border(
            state.focus == FocusPane::Left,
            state.theme.palette(),
        ));
    let tree_inner = tree_block.inner(panes[0]);
    frame.render_widget(tree_block, panes[0]);
    if left_is_graph {
        draw_graph(frame, tree_inner, state);
    } else {
        draw_tree(frame, tree_inner, state);
    }

    let focused = state.focus == FocusPane::Right;
    let right_title = if state.drill.is_files() {
        if focused {
            " files "
        } else {
            " files"
        }
    } else if state.drill.is_diff() || state.right_is_diff() {
        if focused {
            " diff "
        } else {
            " diff"
        }
    } else if focused {
        " graph "
    } else {
        " graph"
    };
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(right_title)
        .border_style(pane_border(
            state.focus == FocusPane::Right,
            state.theme.palette(),
        ));
    let right_inner = right_block.inner(panes[1]);
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    frame.render_widget(right_block, panes[1]);
    draw_right(frame, right_inner, state);

    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            state.status.clone(),
            Style::default().fg(state.theme.palette().muted),
        ))),
        chunks[1],
    );

    state.layout.tree_x = tree_inner.x;
    state.layout.tree_y = tree_inner.y;
    state.layout.tree_width = tree_inner.width;
    state.layout.tree_height = tree_inner.height;
    state.layout.right_x = panes[1].x;
    state.layout.term_cols = area.width;
    state.layout.pane_height = chunks[0].height;
    state.layout.outer_tree_width = panes[0].width;
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    state.layout.diff_content_x = right_inner.x;
    state.layout.diff_split_rule_x =
        if state.right_is_diff() && is_side_by_side_split(state.diff_mode, right_inner.width) {
            let split = side_by_side_column_widths(right_inner.width, state.diff_split_fraction);
            Some(diff_split_rule_x(panes[0].width, split.left_width).saturating_sub(1))
        } else {
            None
        };

    state.layout.right_y = right_inner.y;
    if let super::drill::DrillView::Files { cursor, .. } = &state.drill {
        let (title, subtitle) = state.commit_detail_meta();
        let mut header_h = 0u16;
        if !title.is_empty() {
            header_h += 1;
        }
        if subtitle.as_ref().is_some_and(|s| !s.is_empty()) {
            header_h += 1;
        }
        if header_h == 0 {
            header_h = 1;
        }
        header_h = header_h.min(right_inner.height);
        state.layout.files_list_y = right_inner.y.saturating_add(header_h);
        let list_h = right_inner.height.saturating_sub(header_h) as usize;
        let (start, _) = visible_window(state.commit_file_rows().len(), *cursor, list_h);
        state.layout.files_list_offset = start;
    }
    if state.help_open {
        draw_help(frame, area, state);
    } else if state.stash_menu.is_some() {
        draw_stash_menu(frame, area, state);
    } else if state.create_branch.is_some() {
        draw_create_branch(frame, area, state);
    } else if state.branch_picker.is_some() {
        draw_branch_picker(frame, area, state);
    }
}

fn pane_border(focused: bool, palette: Palette) -> Style {
    if focused {
        Style::default().fg(palette.heading)
    } else {
        Style::default().fg(palette.muted)
    }
}

fn draw_tree(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    let height = area.height as usize;
    let width = area.width as usize;
    let cursor = state.cursor;
    let (start, _) = visible_window(state.rows.len(), cursor, height);
    state.layout.list_offset = start;
    let motion = tree_easy_motion_labels(state, start, height);
    let palette = state.theme.palette();
    let mut lines = Vec::new();
    for (i, row) in state.rows.iter().enumerate().skip(start).take(height) {
        let motion_label = motion
            .as_ref()
            .and_then(|labels| labels.get(i - start))
            .cloned();
        let viewed = row.kind == NodeKind::File && state.reviewed.contains(&row.id);
        let flashing = state
            .flashes
            .get(&row.id)
            .is_some_and(|at| flash_active(Instant::now().saturating_duration_since(*at)));
        lines.push(paint_tree_row(
            row,
            width,
            i == cursor,
            flashing,
            state.ascii,
            viewed,
            motion_label.as_deref(),
            palette,
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn paint_tree_row(
    row: &VisibleRow,
    width: usize,
    selected: bool,
    flashing: bool,
    ascii: bool,
    viewed: bool,
    motion_label: Option<&str>,
    palette: Palette,
) -> Line<'static> {
    let bg = if selected {
        Some(palette.cursor_bg)
    } else if flashing {
        Some(palette.flash)
    } else {
        None
    };
    let segs = row_segments(row, ascii, viewed);
    let trailing_text: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    let trailing_width = visible_width(&trailing_text);
    let pad = usize::from(trailing_width > 0);

    let mut spans: Vec<Span> = Vec::new();
    let edge = if selected { CURSOR_BAR } else { " " };
    spans.push(styled_span(
        edge,
        Style::default()
            .fg(palette.cursor)
            .add_modifier(Modifier::BOLD),
        bg,
    ));

    let prefix_width = if let Some(label) = motion_label {
        let padded = format!("{label:<2}");
        spans.push(styled_span(
            &padded,
            Style::default()
                .fg(palette.cursor)
                .add_modifier(Modifier::BOLD),
            bg,
        ));
        1 + visible_width(&padded)
    } else {
        let indent = "  ".repeat(row.depth);
        spans.push(styled_span(&indent, Style::default(), bg));
        let chevron = fold_chevron(row, ascii);
        spans.push(styled_span(
            &format!("{chevron} "),
            Style::default().fg(palette.muted),
            bg,
        ));
        1 + visible_width(&indent) + 2
    };

    let label_budget = width
        .saturating_sub(prefix_width)
        .saturating_sub(trailing_width)
        .saturating_sub(pad);
    let label = truncate_segs(&segs.segments, label_budget.max(1));
    let label_width: usize = label.iter().map(|s| visible_width(&s.text)).sum();
    for seg in &label {
        spans.push(styled_span(&seg.text, seg_style(seg, palette), bg));
    }
    let gap = pad + label_budget.saturating_sub(label_width);
    if gap > 0 {
        spans.push(styled_span(&" ".repeat(gap), Style::default(), bg));
    }
    for seg in &segs.trailing {
        spans.push(styled_span(&seg.text, seg_style(seg, palette), bg));
    }
    let used: usize = spans
        .iter()
        .map(|s| visible_width(s.content.as_ref()))
        .sum();
    if used < width {
        spans.push(styled_span(&" ".repeat(width - used), Style::default(), bg));
    }
    Line::from(spans)
}

fn fold_chevron(row: &VisibleRow, ascii: bool) -> &'static str {
    if !row.foldable {
        return " ";
    }
    if row.folded {
        if ascii {
            FOLD_COLLAPSED_ASCII
        } else {
            FOLD_COLLAPSED
        }
    } else if ascii {
        FOLD_EXPANDED_ASCII
    } else {
        FOLD_EXPANDED
    }
}

fn styled_span(text: &str, mut style: Style, bg: Option<ratatui::style::Color>) -> Span<'static> {
    if let Some(bg) = bg {
        style = style.bg(bg);
    }
    Span::styled(text.to_string(), style)
}

fn seg_style(seg: &TextSeg, palette: Palette) -> Style {
    let fg = if let Some(hex) = seg.hex {
        hex_color(hex)
    } else {
        match seg.role {
            SegRole::Heading => palette.heading,
            SegRole::Repo => palette.repo,
            SegRole::Dir => palette.dir,
            SegRole::File => palette.file,
            SegRole::Muted => palette.muted,
            SegRole::Added => palette.added,
            SegRole::Modified => palette.modified,
            SegRole::Deleted => palette.deleted,
            SegRole::Renamed => palette.renamed,
            SegRole::BranchDefault => palette.branch_default,
            SegRole::BranchFeature => palette.branch_feature,
        }
    };
    let mut style = Style::default().fg(fg);
    if seg.bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if seg.dim {
        style = style.add_modifier(Modifier::DIM);
    }
    style
}

fn truncate_segs(segs: &[TextSeg], width: usize) -> Vec<TextSeg> {
    if width == 0 {
        return Vec::new();
    }
    let total: usize = segs.iter().map(|s| visible_width(&s.text)).sum();
    if total <= width {
        return segs.to_vec();
    }
    let budget = width.saturating_sub(1);
    let mut out = Vec::new();
    let mut used = 0;
    for seg in segs {
        let sw = visible_width(&seg.text);
        if used + sw <= budget {
            out.push(seg.clone());
            used += sw;
            continue;
        }
        let mut cut = seg.clone();
        cut.text = truncate_visible(&seg.text, budget.saturating_sub(used));
        if !cut.text.is_empty() {
            out.push(cut);
        }
        break;
    }
    out.push(TextSeg {
        text: "…".into(),
        role: SegRole::Muted,
        hex: None,
        bold: false,
        dim: true,
    });
    out
}

fn draw_right(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    match &state.drill {
        DrillView::Files { cursor, .. } => {
            draw_commit_detail(frame, area, state, *cursor);
            return;
        }
        DrillView::Diff { .. } => {
            draw_diff_pane(frame, area, state);
            return;
        }
        DrillView::Graph => {}
    }
    if state.right_is_diff() {
        draw_diff_pane(frame, area, state);
        return;
    }
    draw_graph(frame, area, state);
    if state.graph.is_some() {
        return;
    }
    frame.render_widget(
        Paragraph::new("focus a repo for the graph, or a file for its diff"),
        area,
    );
}

fn draw_graph(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(model) = state.graph.as_ref() else {
        frame.render_widget(Paragraph::new("focus a repo for the graph"), area);
        return;
    };
    GraphWidget::new(model)
        .ascii(state.ascii)
        .selected(Some(state.graph_cursor))
        .scroll(state.graph_scroll)
        .render(area, frame.buffer_mut());
    overlay_graph_easy_motion(frame, area, state);
}

fn draw_commit_detail(frame: &mut Frame<'_>, area: Rect, state: &AppState, cursor: usize) {
    let (title, subtitle) = state.commit_detail_meta();
    let mut header: Vec<String> = Vec::new();
    if !title.is_empty() {
        header.push(title);
    }
    if let Some(sub) = subtitle {
        if !sub.is_empty() {
            header.push(sub);
        }
    }
    if header.is_empty() {
        header.push(String::new());
    }
    let header_h = header.len().min(area.height as usize);
    let palette = state.theme.palette();
    let header_lines: Vec<Line> = header
        .iter()
        .take(header_h)
        .enumerate()
        .map(|(i, line)| {
            let style = if i == 0 {
                Style::default().fg(palette.repo)
            } else {
                Style::default().fg(palette.muted)
            };
            Line::from(Span::styled(line.clone(), style))
        })
        .collect();
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: header_h as u16,
    };
    frame.render_widget(Paragraph::new(header_lines), header_area);

    let list_y = area.y.saturating_add(header_h as u16);
    let list_h = area.height.saturating_sub(header_h as u16);
    if list_h == 0 {
        return;
    }
    let list_area = Rect {
        x: area.x,
        y: list_y,
        width: area.width,
        height: list_h,
    };
    let rows = state.commit_file_rows();
    if rows.is_empty() {
        frame.render_widget(Paragraph::new("no files in this commit"), list_area);
        return;
    }
    let height = list_h as usize;
    let (start, _) = visible_window(rows.len(), cursor, height);
    let motion = file_easy_motion_labels(state, start, height);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, row)| {
            let label = motion
                .as_ref()
                .and_then(|labels| labels.get(i - start))
                .map(|s| format!("{s:<2}"))
                .unwrap_or_default();
            let chevron = if row.foldable {
                if row.folded {
                    if state.ascii {
                        ">"
                    } else {
                        "▸"
                    }
                } else if state.ascii {
                    "v"
                } else {
                    "▾"
                }
            } else {
                " "
            };
            let indent = "  ".repeat(row.depth);
            let text = format!("{label}{indent}{chevron} {}", row.label);
            let mut style = if row.is_dir() {
                Style::default().fg(palette.repo)
            } else {
                Style::default().fg(palette.file)
            };
            if i == cursor {
                style = style.fg(palette.cursor).bg(palette.cursor_bg);
            } else if !label.is_empty() {
                style = style.fg(palette.heading);
            }
            Line::from(Span::styled(text, style))
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), list_area);
}

fn draw_diff_pane(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let path = state.diff_header_path();
    let effective = effective_diff_mode(state.diff_mode, area.width);
    let rows = state.current_diff_rows();
    let body_h = area.height.saturating_sub(1).max(1) as usize;
    let max_start = rows.len().saturating_sub(body_h);
    let skip = (state.diff_scroll as usize).min(max_start);
    let mode_label = diff_pane_mode_label(state.diff_mode, effective);
    let header = diff_pane_header(
        &path,
        mode_label,
        state.full_context_active(),
        state.diff_col_offset,
        skip,
        body_h,
        rows.len(),
    );
    let title = if path.is_empty() {
        "Diff"
    } else {
        path.as_str()
    };
    let extra = header.strip_prefix(title).unwrap_or("").to_string();
    let header_line = Line::from(vec![
        Span::styled(
            title.to_string(),
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(extra, Style::default().fg(palette.muted)),
    ]);
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(header_line), header_area);
    if area.height <= 1 {
        return;
    }
    let body = Rect {
        x: area.x,
        y: area.y.saturating_add(1),
        width: area.width,
        height: area.height.saturating_sub(1),
    };
    if rows.is_empty() {
        let msg = if path.is_empty() {
            "select a dirty file"
        } else {
            "(no diff)"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::default().fg(palette.muted))),
            body,
        );
        return;
    }
    let gutter = gutter_width(&rows);
    let split = is_side_by_side_split(state.diff_mode, area.width);
    let off = state.diff_col_offset as usize;
    let painted: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(skip)
        .take(body.height as usize)
        .map(|(i, row)| {
            paint_diff_row(
                row,
                area.width,
                gutter,
                split,
                off,
                state,
                state.search_hit == Some(i),
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(painted), body);
}

fn paint_diff_row(
    row: &DiffRow,
    width: u16,
    gutter: usize,
    split: bool,
    col_offset: usize,
    state: &AppState,
    search_hit: bool,
) -> Line<'static> {
    let palette = state.theme.palette();
    let mut line = match row {
        DiffRow::Section(section) => {
            let color = section_style(*section, palette);
            Line::from(Span::styled(
                format!(" {} ", section_header(*section)),
                color.add_modifier(Modifier::BOLD),
            ))
        }
        DiffRow::Hunk { text } => Line::from(Span::styled(
            slice_visible(text, 0, width as usize),
            Style::default().fg(palette.diff_hunk),
        )),
        DiffRow::Line { left, right } if split && right.is_some() => {
            let cols = side_by_side_column_widths(width, state.diff_split_fraction);
            let mut spans = paint_cell_spans(left, cols.left_width, gutter, col_offset, palette);
            spans.push(Span::styled(
                DIFF_RULE.to_string(),
                Style::default().fg(Color::DarkGray),
            ));
            spans.extend(paint_cell_spans(
                right.as_ref().unwrap(),
                cols.right_width,
                gutter,
                col_offset,
                palette,
            ));
            Line::from(spans)
        }
        DiffRow::Line { left, .. } => {
            Line::from(paint_cell_spans(left, width, gutter, col_offset, palette))
        }
    };
    if search_hit {
        line.spans = line
            .spans
            .into_iter()
            .map(|span| {
                let style = span.style.bg(palette.flash);
                Span::styled(span.content.to_string(), style)
            })
            .collect();
    }
    line
}

fn section_style(section: DiffSection, palette: Palette) -> Style {
    match section {
        DiffSection::Staged => Style::default().fg(palette.added),
        DiffSection::Unstaged => Style::default().fg(palette.modified),
        DiffSection::New => Style::default().fg(palette.heading),
    }
}

fn paint_cell_spans(
    cell: &DiffCell,
    width: u16,
    gutter: usize,
    col_offset: usize,
    palette: Palette,
) -> Vec<Span<'static>> {
    let width = width as usize;
    let code_w = cell_code_width(width, gutter);
    let line_no = cell
        .line_no
        .map(|n| format!("{n:>gutter$}"))
        .unwrap_or_else(|| " ".repeat(gutter));
    let plain = slice_visible(&cell.text, col_offset, code_w);
    let sign = cell_sign(cell.kind);
    let accent = cell_accent(cell.kind, palette);
    let code_style = accent.unwrap_or(Style::default().fg(palette.repo));
    let muted = Style::default()
        .fg(palette.muted)
        .add_modifier(Modifier::DIM);
    let used = gutter + 4 + plain.chars().count();
    let pad = width.saturating_sub(used);
    vec![
        Span::styled(line_no, muted),
        Span::styled(format!(" {DIFF_RULE} "), muted),
        Span::styled(
            sign.to_string(),
            accent
                .unwrap_or(Style::default())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(plain, code_style),
        Span::raw(" ".repeat(pad)),
    ]
}

fn cell_accent(kind: DiffCellKind, palette: Palette) -> Option<Style> {
    match kind {
        DiffCellKind::Add => Some(Style::default().fg(palette.added)),
        DiffCellKind::Del => Some(Style::default().fg(palette.deleted)),
        DiffCellKind::Meta => Some(Style::default().fg(palette.muted)),
        DiffCellKind::Ctx | DiffCellKind::Empty => None,
    }
}

fn help_span_style(hit: bool, current: bool, palette: Palette) -> Style {
    if current {
        Style::default().fg(palette.cursor).bg(palette.cursor_bg)
    } else if hit {
        Style::default().fg(palette.heading)
    } else {
        Style::default()
    }
}

fn help_entry_text(entry: &super::help::HelpEntry) -> String {
    format!("{}  {}", entry.keys, entry.desc)
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    use super::help::{help_entry_matches, HELP_ENTRIES};
    let query = state.help_search_query.as_deref().unwrap_or("");
    let palette = state.theme.palette();
    let mut lines = Vec::new();
    let mut i = 0;
    while i < HELP_ENTRIES.len() {
        let left = &HELP_ENTRIES[i];
        let left_text = format!("{:<22}", help_entry_text(left));
        let left_hit = help_entry_matches(left.keys, left.desc, query);
        let left_cur = state.help_search_hit == Some(i);
        if let Some(right) = HELP_ENTRIES.get(i + 1) {
            let right_text = help_entry_text(right);
            let right_hit = help_entry_matches(right.keys, right.desc, query);
            let right_cur = state.help_search_hit == Some(i + 1);
            let left_style = help_span_style(left_hit, left_cur, palette);
            let right_style = help_span_style(right_hit, right_cur, palette);
            lines.push(Line::from(vec![
                Span::styled(left_text, left_style),
                Span::styled(right_text, right_style),
            ]));
            i += 2;
        } else {
            lines.push(Line::from(Span::styled(
                left_text.trim_end().to_string(),
                help_span_style(left_hit, left_cur, palette),
            )));
            i += 1;
        }
    }
    let width = 52.min(area.width.saturating_sub(4));
    let height = ((lines.len() as u16) + 2).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let rect = Rect::new(x, y, width, height);
    frame.render_widget(Clear, rect);
    let title = if let Some(q) = &state.help_search_query {
        format!(" keys  /{q} ")
    } else {
        " keys ".into()
    };
    frame.render_widget(
        Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL).title(title))
            .wrap(Wrap { trim: false }),
        rect,
    );
}

fn tree_easy_motion_labels(state: &AppState, start: usize, height: usize) -> Option<Vec<String>> {
    let motion = state.easy_motion.as_ref()?;
    if state.focus != FocusPane::Left {
        return None;
    }
    let (_win_start, count) = visible_window(state.rows.len(), state.cursor, height);
    if _win_start != start {
        return None;
    }
    Some(filter_labels(easy_motion_labels(count), &motion.typed))
}

fn file_easy_motion_labels(state: &AppState, start: usize, height: usize) -> Option<Vec<String>> {
    let motion = state.easy_motion.as_ref()?;
    if state.focus != FocusPane::Right || !state.drill.is_files() {
        return None;
    }
    let DrillView::Files { cursor, .. } = &state.drill else {
        return None;
    };
    let rows = state.commit_file_rows();
    let (_win_start, count) = visible_window(rows.len(), *cursor, height);
    if _win_start != start {
        return None;
    }
    Some(filter_labels(easy_motion_labels(count), &motion.typed))
}

fn overlay_graph_easy_motion(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(motion) = state.easy_motion.as_ref() else {
        return;
    };
    if !state.graph_pane_focused() {
        return;
    }
    let Some(model) = state.graph.as_ref() else {
        return;
    };
    let n = model.visible_rows().len();
    let height = area.height as usize;
    let (start, count) = visible_window(n, state.graph_cursor, height);
    let labels = filter_labels(easy_motion_labels(count), &motion.typed);
    let palette = state.theme.palette();
    let glyphs = if state.ascii { &ASCII } else { &UNICODE };
    let mut y = area.y;
    let bottom = area.y.saturating_add(area.height);
    if model.sync.is_some() {
        y = y.saturating_add(1);
    }
    for line in paint_model(model, glyphs, None)
        .into_iter()
        .skip(state.graph_scroll as usize)
    {
        if y >= bottom {
            break;
        }
        if line.selectable {
            if let Some(idx) = line.row_index {
                if idx >= start && idx < start + labels.len() {
                    let label = &labels[idx - start];
                    if !label.is_empty() {
                        frame.buffer_mut().set_stringn(
                            area.x,
                            y,
                            &format!("{label:<2}"),
                            2,
                            Style::default().fg(palette.heading),
                        );
                    }
                }
            }
        }
        y = y.saturating_add(1);
    }
}

fn filter_labels(labels: Vec<String>, typed: &str) -> Vec<String> {
    if typed.is_empty() {
        return labels;
    }
    labels
        .into_iter()
        .map(|label| {
            if label == typed || label.starts_with(typed) {
                label
            } else {
                String::new()
            }
        })
        .collect()
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
        Style::default()
            .fg(Color::Yellow)
            .add_modifier(Modifier::BOLD),
    ))];
    for op in ops {
        let detail = op.stash_ref.as_deref().unwrap_or("");
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
    let at = create
        .commit_id
        .as_deref()
        .map(|id| format!(" at {}", id.get(..7).unwrap_or(id)))
        .unwrap_or_default();
    let body = format!(
        " Create branch{at}\n  name: {}\n Enter confirm · Esc cancel",
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
        let snapshot =
            build_workspace_snapshot(&[repo("app", true), repo("lib", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: "seed".into(),
                parents: Vec::new(),
                refs: vec!["main".into()],
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: true,
            ..GraphModel::default()
        });
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("app"), "{text}");
        assert!(text.contains("README.md"), "{text}");
        assert!(text.contains("changed ·"), "{text}");
        let file_line = text
            .lines()
            .find(|line| line.contains("README.md"))
            .unwrap_or("");
        let name_at = file_line
            .find("README.md")
            .expect("README.md on a tree row");
        let after_name = &file_line[name_at + "README.md".len()..];
        assert!(
            after_name.contains('M'),
            "status badge should sit to the right of the name: {file_line:?}"
        );
        assert!(
            !file_line.contains("? README") && !file_line.contains("M README"),
            "badge must not prefix the file name: {file_line:?}"
        );
        assert!(
            text.contains("seed") || text.contains("dirty") || text.contains("aaa1111"),
            "{text}"
        );
    }

    #[test]
    fn help_overlay_is_short() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("q  quit"), "{text}");
        assert!(text.contains(".  show ignored"), "{text}");
        assert!(text.contains("S  stash menu"), "{text}");
        assert!(text.contains("P  push"), "{text}");
        assert!(text.contains("W  remove worktree"), "{text}");
        assert!(text.contains("EasyMotion"), "{text}");
        assert!(text.contains("cycle theme"), "{text}");
    }

    #[test]
    fn easy_motion_paints_labels_on_visible_tree_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.dispatch(super::super::action::Action::EasyMotionStart);
        assert!(state.easy_motion.is_some());
        let backend = TestBackend::new(80, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("a "), "{text}");
        assert!(text.contains("b "), "{text}");
    }

    #[test]
    fn paints_draggable_side_by_side_rule_on_wide_diff() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.diff_mode = crate::tui::split::DiffMode::SideBySide;
        let file = state
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file row");
        state.cursor = file;
        state.set_diff(
            "app".into(),
            "README.md".into(),
            super::super::diff::DiffContent::from_lines(vec![
                "@@ -1,1 +1,1 @@".into(),
                "-old line".into(),
                "+new line".into(),
            ]),
        );
        let backend = TestBackend::new(220, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(state.layout.diff_split_rule_x.is_some(), "rule missing");
        assert!(state.layout.outer_tree_width >= 20);
        let text = buffer_text(&terminal);
        assert!(text.contains("│") || text.contains("|"), "{text}");
        assert!(
            text.contains("old") || text.contains("new") || text.contains("README"),
            "{text}"
        );
        assert!(text.contains("README.md"), "{text}");
        assert!(text.contains("split") || text.contains("inline"), "{text}");
        assert!(
            text.contains("UNSTAGED") || text.contains("STAGED"),
            "{text}"
        );
    }
}

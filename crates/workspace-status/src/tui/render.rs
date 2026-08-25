//! Ratatui paint for the tree, graph / diff pane, status, and help overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Padding, Paragraph, Widget, Wrap};
use ratatui::Frame;
use workspace_status_graph::{
    graph_chrome_budget, graph_gutter_cap, paint_model, GraphLabelPalette, GraphWidget, ASCII,
    UNICODE,
};

use std::collections::HashSet;

use super::chrome::{
    breadcrumb_line, breadcrumb_rows, ctrl_c_prompt_line, ctrl_c_prompt_rows,
    overlay_status_rows_for, status_line,
};
use super::diff::{
    cell_code_width, cell_sign, diff_pane_header, diff_pane_mode_label, gutter_width,
    section_header, DiffCell, DiffCellKind, DiffRow, DiffSection, DIFF_RULE,
};
use super::drill::DrillView;
use super::easy_motion::{easy_motion_labels, visible_window};
use super::help::{
    help_chip_gap_spaces, help_column_width, help_entry_matches, help_entry_visual_lines,
    help_idle_footer_lines, help_inner_width, help_version_label, HELP_GROUPS,
    HELP_SEARCH_ESC_HINT,
};
use super::icons::{
    icon_branch, icon_diff, icon_merged_into_default, icon_move, icon_open_vs_default,
    truncate_visible, CURSOR_BAR, FOLD_COLLAPSED, FOLD_COLLAPSED_ASCII, FOLD_EXPANDED,
    FOLD_EXPANDED_ASCII,
};
use super::search::{
    collect_commit_file_match_indices, collect_graph_match_indices, collect_match_ids, slice_cols,
    slice_visible, SearchPane,
};
use super::split::{
    diff_split_rule_x, effective_diff_mode, is_side_by_side_split, pane_widths,
    side_by_side_column_widths, MIN_PANE_COLS,
};
use super::state::{AppState, FocusPane, PendingConfirm};
use super::theme::{hex_color, Palette};
use super::tree::{row_segments, NodeKind, NodeSegments, SegRole, TextSeg, VisibleRow};
use crate::helpers::visible_width;

/// Empty tree / empty commit-file list.
const NO_MATCHING_ROWS: &str = "No matching rows";
/// Commit-file list while git is still listing.
const LOADING_FILES: &str = "loading files…";

fn muted_copy(text: &'static str, palette: Palette) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(palette.muted)))
}

/// Cursor → search match → flash.
fn row_match_bg(
    selected: bool,
    search_match: bool,
    flash: Option<Color>,
    palette: Palette,
    search_bg: Color,
) -> Option<Color> {
    if selected {
        Some(palette.cursor_bg)
    } else if search_match {
        Some(search_bg)
    } else {
        flash
    }
}

/// Draw one frame. Updates `state.layout` for mouse hits.
pub fn draw(frame: &mut Frame<'_>, state: &mut AppState) {
    state.prune_expired_flashes();
    let area = frame.area();
    let overlay_h = overlay_status_rows_for(state, area.width);
    let crumb_h = breadcrumb_rows(state);
    let prompt_h = ctrl_c_prompt_rows(state);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(crumb_h),
            Constraint::Length(prompt_h),
            Constraint::Length(overlay_h),
        ])
        .split(area);
    let widths = pane_widths(area.width, state.tree_fraction);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(widths.tree_width),
            Constraint::Min(MIN_PANE_COLS),
        ])
        .split(chunks[0]);

    let left_is_files = state.drill.is_diff();
    let left_is_graph = state.drill.is_files();
    state.layout.graph_scrollbar_x = None;
    state.layout.graph_scrollbar_y = 0;
    state.layout.graph_scrollbar_height = 0;
    state.layout.graph_content_len = 0;
    let left_title = if left_is_files {
        if state.focus == FocusPane::Left {
            " files "
        } else {
            " files"
        }
    } else if left_is_graph {
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
    if left_is_files {
        draw_commit_file_list(
            frame,
            tree_inner,
            state,
            state.commit_files_cursor(),
            state.left_col_offset as usize,
        );
    } else if left_is_graph {
        draw_graph(frame, tree_inner, state, state.left_col_offset);
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

    if crumb_h > 0 {
        frame.render_widget(
            Paragraph::new(breadcrumb_line(state, chunks[1].width)),
            chunks[1],
        );
    }
    if prompt_h > 0 {
        frame.render_widget(
            Paragraph::new(ctrl_c_prompt_line(state, chunks[2].width)),
            chunks[2],
        );
    }
    let overlay = chunks[3];
    if state.help_open {
        draw_help(frame, overlay, state);
    } else if state.confirm.is_some() {
        draw_confirm(frame, overlay, state);
    } else if state.stash_menu.is_some() {
        draw_stash_menu(frame, overlay, state);
    } else if state.create_branch.is_some() {
        draw_create_branch(frame, overlay, state);
    } else if state.branch_picker.is_some() {
        draw_branch_picker(frame, overlay, state);
    } else {
        frame.render_widget(Paragraph::new(status_line(state, overlay.width)), overlay);
    }

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
    if let super::drill::DrillView::Diff { file_cursor, .. } = &state.drill {
        let cursor = *file_cursor;
        state.layout.files_list_y = tree_inner.y;
        let list_h = tree_inner.height as usize;
        let (start, _) = visible_window(state.painted_commit_file_rows().len(), cursor, list_h);
        state.layout.files_list_offset = start;
    } else if let super::drill::DrillView::Files { cursor, .. } = &state.drill {
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
        let painted_n = state.painted_commit_file_rows().len();
        let (start, _) = visible_window(painted_n, *cursor, list_h);
        state.layout.files_list_offset = start;
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
    let palette = state.theme.palette();
    let painted = state.painted_tree_rows();
    if painted.is_empty() {
        frame.render_widget(Paragraph::new(muted_copy(NO_MATCHING_ROWS, palette)), area);
        return;
    }
    let height = area.height as usize;
    let width = area.width as usize;
    let focus_id = state.rows.get(state.cursor).map(|row| row.id.as_str());
    let painted_cursor = focus_id
        .and_then(|id| painted.iter().position(|row| row.id == id))
        .unwrap_or(0);
    let (start, _) = visible_window(painted.len(), painted_cursor, height);
    state.layout.list_offset = start;
    let motion = tree_easy_motion_labels(state, start, height);
    let search_bg = state.theme.pills().filter.bg;
    let match_ids: HashSet<String> = if state.search_target == SearchPane::Tree {
        collect_match_ids(&state.tree, &state.search_query)
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let mut lines = Vec::new();
    for (i, row) in painted.iter().enumerate().skip(start).take(height) {
        let motion_label = motion
            .as_ref()
            .and_then(|labels| labels.get(i - start))
            .cloned();
        let viewed = row.kind == NodeKind::File && state.reviewed.contains(&row.id);
        lines.push(paint_tree_row(
            row,
            width,
            Some(row.id.as_str()) == focus_id,
            state.flash_color(&row.id),
            match_ids.contains(&row.id),
            search_bg,
            state.ascii,
            viewed,
            motion_label.as_deref(),
            palette,
            state.left_col_offset as usize,
        ));
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn paint_tree_row(
    row: &VisibleRow,
    width: usize,
    selected: bool,
    flash: Option<Color>,
    search_match: bool,
    search_bg: Color,
    ascii: bool,
    viewed: bool,
    motion_label: Option<&str>,
    palette: Palette,
    col_offset: usize,
) -> Line<'static> {
    let segs = row_segments(row, ascii, viewed);
    paint_segmented_row(
        row.depth,
        row.foldable,
        row.folded,
        &segs,
        width,
        selected,
        flash,
        search_match,
        search_bg,
        ascii,
        motion_label,
        palette,
        col_offset,
    )
}

fn paint_segmented_row(
    depth: usize,
    foldable: bool,
    folded: bool,
    segs: &NodeSegments,
    width: usize,
    selected: bool,
    flash: Option<Color>,
    search_match: bool,
    search_bg: Color,
    ascii: bool,
    motion_label: Option<&str>,
    palette: Palette,
    col_offset: usize,
) -> Line<'static> {
    let bg = row_match_bg(selected, search_match, flash, palette, search_bg);
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
        let indent = "  ".repeat(depth);
        spans.push(styled_span(&indent, Style::default(), bg));
        let chevron = fold_chevron(foldable, folded, ascii);
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
    let label = slice_segs(&segs.segments, col_offset, label_budget.max(1));
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

fn fold_chevron(foldable: bool, folded: bool, ascii: bool) -> &'static str {
    if !foldable {
        return " ";
    }
    if folded {
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

fn slice_segs(segs: &[TextSeg], offset: usize, width: usize) -> Vec<TextSeg> {
    if width == 0 {
        return Vec::new();
    }
    let mut skip = offset;
    let mut remain = width;
    let mut out = Vec::new();
    for seg in segs {
        let sw = visible_width(&seg.text);
        if skip >= sw {
            skip -= sw;
            continue;
        }
        let sliced = slice_cols(&seg.text, skip, remain);
        skip = 0;
        let used = visible_width(&sliced);
        if !sliced.is_empty() {
            let mut cut = seg.clone();
            cut.text = sliced;
            out.push(cut);
        }
        remain = remain.saturating_sub(used);
        if remain == 0 {
            break;
        }
    }
    out
}

fn draw_right(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
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
    draw_graph(frame, area, state, state.right_col_offset);
    if state.graph.is_some() {
        return;
    }
    frame.render_widget(
        Paragraph::new("focus a repo for the graph, or a file for its diff"),
        area,
    );
}

fn draw_graph(frame: &mut Frame<'_>, area: Rect, state: &mut AppState, col_offset: u16) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let Some(model) = state.graph.as_ref() else {
        frame.render_widget(Paragraph::new("focus a repo for the graph"), area);
        return;
    };
    let matches = graph_search_matches(state);
    let pal = state.theme.palette();
    let flash_rows = state.graph_flash_rows();
    GraphWidget::new(model)
        .ascii(state.ascii)
        .selected(Some(state.graph_cursor))
        .scroll(state.graph_scroll)
        .col_offset(col_offset)
        .loading_older(state.graph_loading_older)
        .search_matches(&matches, state.theme.pills().filter.bg)
        .flash_rows(&flash_rows)
        .cursor_style(pal.cursor, pal.cursor_bg)
        .label_palette(GraphLabelPalette {
            subject: pal.repo,
            meta: pal.muted,
            branch_local: pal.branch_feature,
            branch_default: pal.branch_default,
            remote: pal.dir,
            tag: pal.modified,
            head_mark: pal.head_mark,
            overflow: pal.heading,
        })
        .render(area, frame.buffer_mut());
    overlay_graph_easy_motion(frame, area, state);
    record_graph_scrollbar(state, area);
}

fn graph_search_matches(state: &AppState) -> Vec<usize> {
    if state.search_target != SearchPane::Graph {
        return Vec::new();
    }
    let Some(model) = state.graph.as_ref() else {
        return Vec::new();
    };
    collect_graph_match_indices(&model.visible_rows(), &state.search_query)
}

fn record_graph_scrollbar(state: &mut AppState, area: Rect) {
    let Some(model) = state.graph.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let chrome = graph_chrome_budget(area.height, state.graph_loading_older, model.sync.is_some());
    let glyphs = if state.ascii { &ASCII } else { &UNICODE };
    let content_len = paint_model(model, glyphs, None).len();
    state.layout.graph_scrollbar_x = Some(area.x.saturating_add(area.width.saturating_sub(1)));
    state.layout.graph_scrollbar_y = area.y.saturating_add(u16::from(chrome.header));
    state.layout.graph_scrollbar_height = chrome.list_height;
    state.layout.graph_content_len = content_len;
}

fn draw_commit_detail(frame: &mut Frame<'_>, area: Rect, state: &mut AppState, cursor: usize) {
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
    draw_commit_file_list(
        frame,
        list_area,
        state,
        cursor,
        state.right_col_offset as usize,
    );
}

fn draw_commit_file_list(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut AppState,
    cursor: usize,
    col_offset: usize,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let rows = state.painted_commit_file_rows();
    if rows.is_empty() {
        let copy = if state.commit_files_loading {
            LOADING_FILES
        } else {
            NO_MATCHING_ROWS
        };
        frame.render_widget(Paragraph::new(muted_copy(copy, palette)), area);
        return;
    }
    let height = area.height as usize;
    let width = area.width as usize;
    let focus_id = state
        .commit_file_rows()
        .get(cursor)
        .map(|row| row.id.clone());
    let painted_cursor = focus_id
        .as_deref()
        .and_then(|id| rows.iter().position(|row| row.id == id))
        .unwrap_or(0);
    let (start, _) = visible_window(rows.len(), painted_cursor, height);
    state.layout.files_list_offset = start;
    let motion = file_easy_motion_labels(state, start, height);
    let search_bg = state.theme.pills().filter.bg;
    let searching_files =
        state.search_target == SearchPane::CommitFiles && !state.search_query.trim().is_empty();
    let match_paths = commit_file_search_match_paths(state);
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(i, row)| {
            let motion_label = motion
                .as_ref()
                .and_then(|labels| labels.get(i - start))
                .cloned();
            let segs = NodeSegments {
                segments: row.segments.clone(),
                trailing: row.trailing_segs.clone(),
            };
            let search_match = searching_files
                && (match_paths.contains(&row.path)
                    || commit_file_label_matches(&row.label, &state.search_query));
            paint_segmented_row(
                row.depth,
                row.foldable,
                row.folded,
                &segs,
                width,
                Some(row.id.as_str()) == focus_id.as_deref(),
                state.commit_file_flash_color(&row.id),
                search_match,
                search_bg,
                state.ascii,
                motion_label.as_deref(),
                palette,
                col_offset,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn commit_file_search_match_paths(state: &AppState) -> HashSet<String> {
    if state.search_target != SearchPane::CommitFiles {
        return HashSet::new();
    }
    let files = match &state.drill {
        DrillView::Files { files, .. } | DrillView::Diff { files, .. } => files,
        DrillView::Graph => return HashSet::new(),
    };
    collect_commit_file_match_indices(files, &state.search_query)
        .into_iter()
        .filter_map(|i| files.get(i).map(|file| file.path.clone()))
        .collect()
}

fn commit_file_label_matches(label: &str, query: &str) -> bool {
    let q = query.trim().to_lowercase();
    !q.is_empty() && label.to_lowercase().contains(&q)
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

fn help_group_chrome(title: &str, ascii: bool, palette: Palette) -> (&'static str, Color) {
    match title {
        "MOVE" => (icon_move(ascii), palette.cursor),
        "GIT" => (icon_branch(ascii), palette.added),
        _ => (icon_diff(ascii), palette.modified),
    }
}

fn clamp_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for span in spans {
        let w = visible_width(&span.content);
        if used >= width {
            break;
        }
        if used + w <= width {
            used += w;
            out.push(span);
            continue;
        }
        let take = width - used;
        let cut = truncate_visible(&span.content, take);
        out.push(Span::styled(cut, span.style));
        used = width;
        break;
    }
    if used < width {
        out.push(Span::raw(" ".repeat(width - used)));
    }
    out
}

fn with_bg(spans: Vec<Span<'static>>, bg: Option<Color>) -> Vec<Span<'static>> {
    let Some(bg) = bg else {
        return spans;
    };
    spans
        .into_iter()
        .map(|span| span.patch_style(Style::default().bg(bg)))
        .collect()
}

fn help_chip_spans(keys: &str, color: Color, surface: Color) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for chip in keys.split(' ').filter(|part| !part.is_empty()) {
        spans.push(key_chip(chip, color, surface));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(" ".repeat(help_chip_gap_spaces(keys))));
    spans
}

fn help_visual_cell_spans(
    entry: Option<&super::help::HelpEntry>,
    line: Option<&super::help::HelpVisualLine>,
    color: Color,
    surface: Color,
    muted: Color,
    width: usize,
) -> Vec<Span<'static>> {
    let Some(vis) = line else {
        return clamp_spans(vec![Span::raw("")], width);
    };
    if vis.chips {
        let Some(entry) = entry else {
            return clamp_spans(vec![Span::raw("")], width);
        };
        let mut spans = help_chip_spans(entry.keys, color, surface);
        if !vis.text.is_empty() {
            spans.push(Span::styled(vis.text.clone(), Style::default().fg(muted)));
        }
        return clamp_spans(spans, width);
    }
    let mut spans = Vec::new();
    if vis.indent > 0 {
        spans.push(Span::raw(" ".repeat(vis.indent)));
    }
    if !vis.text.is_empty() {
        spans.push(Span::styled(vis.text.clone(), Style::default().fg(muted)));
    }
    clamp_spans(spans, width)
}

fn key_chip(key: &str, bg: Color, fg: Color) -> Span<'static> {
    Span::styled(
        format!(" {key} "),
        Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD),
    )
}

fn overlay_surface(state: &AppState) -> Color {
    hex_color(state.theme.theme().surface)
}

fn help_spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| visible_width(&span.content)).sum()
}

/// Append the package version on the right of `left`, or on the next row.
fn help_footer_with_version(
    mut left: Vec<Span<'static>>,
    inner: usize,
    muted: Color,
) -> Vec<Line<'static>> {
    let version = help_version_label();
    let vw = visible_width(&version);
    let lw = help_spans_width(&left);
    let inner = inner.max(1);
    let gap_needed = usize::from(lw > 0);
    if lw + gap_needed + vw <= inner {
        left.push(Span::raw(" ".repeat(inner - lw - vw)));
        left.push(Span::styled(version, Style::default().fg(muted)));
        vec![Line::from(left)]
    } else {
        let gap = inner.saturating_sub(vw);
        vec![
            Line::from(left),
            Line::from(vec![
                Span::raw(" ".repeat(gap)),
                Span::styled(version, Style::default().fg(muted)),
            ]),
        ]
    }
}

fn overlay_block(accent: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .padding(Padding::horizontal(1))
}

fn draw_help(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let query = state.help_search_query.as_deref().unwrap_or("");
    let searching = state.help_search_query.is_some();
    let palette = state.theme.palette();
    let pills = state.theme.pills();
    let surface = overlay_surface(state);
    let max_rows = HELP_GROUPS
        .iter()
        .map(|group| group.entries.len())
        .max()
        .unwrap_or(0);
    let mut lines: Vec<Line> = Vec::new();

    let term_width = area.width as usize;
    let inner = help_inner_width(term_width).max(1);
    let col_w = help_column_width(term_width);

    let mut title_spans = Vec::new();
    for group in HELP_GROUPS {
        let (icon, color) = help_group_chrome(group.title, state.ascii, palette);
        title_spans.extend(clamp_spans(
            vec![Span::styled(
                format!("{icon}  {}", group.title),
                Style::default().fg(color).add_modifier(Modifier::BOLD),
            )],
            col_w,
        ));
    }
    lines.push(Line::from(title_spans));

    for row in 0..max_rows {
        let cells: Vec<Vec<super::help::HelpVisualLine>> = HELP_GROUPS
            .iter()
            .map(|group| match group.entries.get(row) {
                Some(entry) => help_entry_visual_lines(entry.desc, col_w, entry.keys),
                None => vec![super::help::HelpVisualLine {
                    chips: false,
                    indent: 0,
                    text: String::new(),
                }],
            })
            .collect();
        let height = cells.iter().map(|cell| cell.len()).max().unwrap_or(1);
        for vis_row in 0..height {
            let mut spans = Vec::new();
            for (group_idx, group) in HELP_GROUPS.iter().enumerate() {
                let (_, color) = help_group_chrome(group.title, state.ascii, palette);
                let entry = group.entries.get(row);
                let hit = searching
                    && entry.is_some_and(|item| help_entry_matches(item.keys, item.desc, query));
                let bg = hit.then_some(pills.filter.bg);
                spans.extend(with_bg(
                    help_visual_cell_spans(
                        entry,
                        cells[group_idx].get(vis_row),
                        color,
                        surface,
                        palette.muted,
                        col_w,
                    ),
                    bg,
                ));
            }
            lines.push(Line::from(spans));
        }
    }

    if searching {
        let q = state.help_search_query.as_deref().unwrap_or("");
        lines.extend(help_footer_with_version(
            vec![
                key_chip("HELP", pills.filter.bg, pills.filter.fg),
                Span::styled(format!(" /{q}"), Style::default().fg(palette.repo)),
                Span::styled("▏", Style::default().fg(palette.cursor)),
                Span::styled(
                    format!("   {HELP_SEARCH_ESC_HINT}"),
                    Style::default().fg(palette.muted),
                ),
            ],
            inner,
            palette.muted,
        ));
    } else {
        for part in help_idle_footer_lines(inner) {
            lines.push(Line::from(Span::styled(
                part,
                Style::default().fg(palette.muted),
            )));
        }
    }

    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(palette.cursor))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn files_word(n: usize) -> &'static str {
    if n == 1 {
        "file"
    } else {
        "files"
    }
}

fn confirm_action_row(
    yes: &str,
    yes_label: &str,
    extra: Option<(&str, Color, &str)>,
    accent: Color,
    muted: Color,
    surface: Color,
) -> Line<'static> {
    let mut spans = vec![
        key_chip(yes, accent, surface),
        Span::styled(format!(" {yes_label}   "), Style::default().fg(muted)),
    ];
    if let Some((key, bg, label)) = extra {
        spans.push(key_chip(key, bg, surface));
        spans.push(Span::styled(
            format!(" {label}   "),
            Style::default().fg(muted),
        ));
    }
    spans.push(key_chip("n", muted, surface));
    spans.push(Span::styled(" cancel", Style::default().fg(muted)));
    Line::from(spans)
}

fn draw_confirm(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(pending) = state.confirm.as_ref() else {
        return;
    };
    let palette = state.theme.palette();
    let surface = overlay_surface(state);
    let (accent, lines) = match pending {
        PendingConfirm::Revert { targets, label } => {
            let tracked = targets.iter().filter(|t| !t.untracked).count();
            let untracked = targets.iter().filter(|t| t.untracked).count();
            let single_untracked = tracked == 0 && untracked == 1;
            let accent = if single_untracked {
                palette.deleted
            } else {
                palette.modified
            };
            let fate = if single_untracked { "deleted" } else { "kept" };
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        "Revert ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(label.clone(), Style::default().fg(palette.file)),
                    Span::styled("?", Style::default().fg(accent)),
                ]),
                Line::from(Span::styled(
                    format!("  {tracked} tracked {} → discarded", files_word(tracked)),
                    Style::default().fg(palette.muted),
                )),
                Line::from(Span::styled(
                    format!("  {untracked} untracked {} → {fate}", files_word(untracked)),
                    Style::default().fg(if single_untracked {
                        accent
                    } else {
                        palette.muted
                    }),
                )),
                confirm_action_row(
                    "y",
                    "revert",
                    Some(("Y", palette.deleted, "revert + delete untracked")),
                    accent,
                    palette.muted,
                    surface,
                ),
            ];
            (accent, lines)
        }
        PendingConfirm::StashDrop { stash_ref, .. } => {
            let accent = palette.deleted;
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        "Drop ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(stash_ref.clone(), Style::default().fg(palette.file)),
                    Span::styled("?", Style::default().fg(accent)),
                ]),
                confirm_action_row("y", "drop", None, accent, palette.muted, surface),
            ];
            (accent, lines)
        }
        PendingConfirm::RemoveWorktree {
            path,
            force,
            branch,
            merged_into_default,
            ..
        } => {
            let accent = palette.deleted;
            let merge_text = match merged_into_default {
                Some(true) => format!(
                    "merged into default {}",
                    icon_merged_into_default(state.ascii)
                ),
                Some(false) => format!(
                    "NOT merged into default {}",
                    icon_open_vs_default(state.ascii)
                ),
                None => "merge status unknown".into(),
            };
            let dirty_line = if *force {
                Line::from(Span::styled(
                    "  dirty worktree — will use --force",
                    Style::default().fg(accent),
                ))
            } else {
                Line::from(Span::styled(
                    "  clean worktree",
                    Style::default().fg(palette.muted),
                ))
            };
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        "Remove worktree ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(path.clone(), Style::default().fg(palette.file)),
                    Span::styled("?", Style::default().fg(accent)),
                ]),
                Line::from(Span::styled(
                    format!("  branch {branch} — {merge_text}"),
                    Style::default().fg(palette.muted),
                )),
                dirty_line,
                confirm_action_row("y", "remove", None, accent, palette.muted, surface),
            ];
            (accent, lines)
        }
        PendingConfirm::CheckoutOutOfSync {
            branch, remote_ref, ..
        } => {
            let accent = palette.modified;
            let lines = vec![
                Line::from(vec![
                    Span::styled(branch.clone(), Style::default().fg(palette.file)),
                    Span::styled(" is not in sync with ", Style::default().fg(palette.muted)),
                    Span::styled(remote_ref.clone(), Style::default().fg(palette.file)),
                ]),
                Line::from(Span::styled(
                    "Checkout local then pull?",
                    Style::default().fg(accent),
                )),
                confirm_action_row(
                    "y",
                    "checkout then pull",
                    None,
                    accent,
                    palette.muted,
                    surface,
                ),
            ];
            (accent, lines)
        }
        PendingConfirm::MergeIntoHead { label, into, .. } => {
            let accent = palette.modified;
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        "Merge ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(label.clone(), Style::default().fg(palette.file)),
                    Span::styled(" into ", Style::default().fg(palette.muted)),
                    Span::styled(into.clone(), Style::default().fg(palette.file)),
                    Span::styled("?", Style::default().fg(accent)),
                ]),
                Line::from(Span::styled(
                    "  fast-forward if possible, otherwise a merge commit",
                    Style::default().fg(palette.muted),
                )),
                confirm_action_row("y", "merge", None, accent, palette.muted, surface),
            ];
            (accent, lines)
        }
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(accent))
            .wrap(Wrap { trim: false }),
        area,
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
    if !state.commit_files_list_focused() {
        return None;
    }
    let cursor = state.commit_files_cursor();
    let rows = state.commit_file_rows();
    let (_win_start, count) = visible_window(rows.len(), cursor, height);
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
    let chrome = graph_chrome_budget(area.height, state.graph_loading_older, model.sync.is_some());
    let list_h = chrome.list_height.max(1) as usize;
    let (start, count) = visible_window(n, state.graph_cursor, list_h);
    let labels = filter_labels(easy_motion_labels(count), &motion.typed);
    let palette = state.theme.palette();
    let glyphs = if state.ascii { &ASCII } else { &UNICODE };
    let mut y = area.y;
    if chrome.header {
        y = y.saturating_add(1);
    }
    let list_bottom = y.saturating_add(chrome.list_height);
    let gutter_cap = graph_gutter_cap(area.width.saturating_sub(1) as usize);
    for line in paint_model(model, glyphs, Some(gutter_cap))
        .into_iter()
        .skip(state.graph_scroll as usize)
    {
        if y >= list_bottom {
            break;
        }
        if line.selectable {
            if let Some(idx) = line.row_index {
                if idx >= start && idx < start + labels.len() {
                    let label = &labels[idx - start];
                    if !label.is_empty() {
                        frame.buffer_mut().set_stringn(
                            area.x.saturating_add(1),
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

fn overlay_status_color(status: &str, palette: Palette) -> Color {
    let lower = status.to_ascii_lowercase();
    if lower.contains("failed")
        || lower.contains("error")
        || lower.contains("invalid")
        || lower.contains("dirty")
    {
        palette.deleted
    } else {
        palette.muted
    }
}

fn draw_stash_menu(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(ops) = state.stash_menu.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let surface = overlay_surface(state);
    let accent = palette.modified;
    let subtitle = state.stash_repo.as_deref().unwrap_or("");
    let mut lines = vec![Line::from(vec![
        Span::styled(
            "Stash ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(subtitle.to_string(), Style::default().fg(palette.muted)),
    ])];
    for op in ops {
        let detail = match op.id {
            super::stash::StashOpId::Apply
            | super::stash::StashOpId::Pop
            | super::stash::StashOpId::Drop => op.stash_ref.as_deref().unwrap_or(""),
            super::stash::StashOpId::Create => "",
        };
        let mut spans = vec![
            key_chip(&op.key.to_string(), accent, surface),
            Span::styled(format!(" {}", op.label), Style::default().fg(palette.file)),
        ];
        if !detail.is_empty() {
            spans.push(Span::styled(
                format!(" {detail}"),
                Style::default().fg(palette.muted),
            ));
        }
        lines.push(Line::from(spans));
    }
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.clone(),
            Style::default().fg(overlay_status_color(&state.status, palette)),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Esc cancel",
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(accent))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_branch_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(picker) = state.branch_picker.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let accent = palette.branch_feature;
    let visible = picker.visible();
    let max_rows = 12usize;
    let start = if visible.len() <= max_rows {
        0
    } else {
        picker
            .cursor
            .saturating_sub(max_rows / 2)
            .min(visible.len() - max_rows)
    };
    let window = if visible.is_empty() {
        Vec::new()
    } else {
        visible
            .iter()
            .skip(start)
            .take(max_rows)
            .copied()
            .collect::<Vec<_>>()
    };
    let filter = if picker.filter.is_empty() {
        "…"
    } else {
        picker.filter.as_str()
    };
    let graph = picker.commit_id.is_some();
    let show_filter = !graph || visible.len() > 8 || !picker.filter.is_empty();
    let mut title = Vec::new();
    if graph {
        let short = picker
            .commit_id
            .as_deref()
            .map(|id| id.get(..7).unwrap_or(id).to_string())
            .unwrap_or_default();
        title.push(Span::styled(
            "Checkout ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
        title.push(Span::styled("at ", Style::default().fg(palette.muted)));
        title.push(Span::styled(short, Style::default().fg(palette.repo)));
    } else {
        title.push(Span::styled(
            "Branch ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ));
        title.push(Span::styled(
            picker.repo.clone(),
            Style::default().fg(palette.repo),
        ));
    }
    if show_filter {
        title.push(Span::styled(
            "  filter: ",
            Style::default().fg(palette.muted),
        ));
        title.push(Span::styled(
            filter.to_string(),
            Style::default().fg(palette.cursor),
        ));
    }
    let mut lines = vec![Line::from(title)];
    if window.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No matching branches",
            Style::default().fg(palette.muted),
        )));
    } else {
        for (i, branch) in window.iter().enumerate() {
            let index = start + i;
            let selected = index == picker.cursor;
            let mark = if branch.current { "* " } else { "  " };
            let cursor = if selected { "❯ " } else { "  " };
            let row_bg = if selected {
                palette.cursor_bg
            } else {
                Color::Reset
            };
            let name_fg = if branch.current {
                palette.added
            } else if selected {
                palette.file
            } else {
                palette.muted
            };
            lines.push(Line::from(vec![
                Span::styled(
                    cursor.to_string(),
                    Style::default()
                        .fg(if selected {
                            palette.cursor
                        } else {
                            palette.muted
                        })
                        .bg(row_bg),
                ),
                Span::styled(
                    format!("{mark}{}", branch.name),
                    Style::default().fg(name_fg).bg(row_bg),
                ),
            ]));
        }
    }
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.clone(),
            Style::default().fg(overlay_status_color(&state.status, palette)),
        )));
    }
    let footer = if graph && !show_filter {
        "j/k move · Enter checkout · C create · Esc cancel"
    } else if graph {
        "j/k move · type to filter · Enter checkout · C create · Esc cancel"
    } else {
        "j/k move · type to filter · Enter checkout · C create · Esc close"
    };
    lines.push(Line::from(Span::styled(
        footer,
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(accent))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_create_branch(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(create) = state.create_branch.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let accent = palette.branch_feature;
    let short = create
        .commit_id
        .as_deref()
        .map(|id| id.get(..7).unwrap_or(id).to_string())
        .unwrap_or_default();
    let name = if create.name.is_empty() {
        "…"
    } else {
        create.name.as_str()
    };
    let mut title = vec![Span::styled(
        "Create branch ",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )];
    if !short.is_empty() {
        title.push(Span::styled("at ", Style::default().fg(palette.muted)));
        title.push(Span::styled(short, Style::default().fg(palette.repo)));
    }
    let mut lines = vec![
        Line::from(title),
        Line::from(vec![
            Span::styled("  name: ", Style::default().fg(palette.muted)),
            Span::styled(name.to_string(), Style::default().fg(palette.cursor)),
        ]),
    ];
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.clone(),
            Style::default().fg(overlay_status_color(&state.status, palette)),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Enter confirm · Esc cancel",
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(accent))
            .wrap(Wrap { trim: false }),
        area,
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

    fn assert_help_version_lower_right(text: &str) {
        let version = crate::APP_VERSION;
        assert!(
            text.contains(version),
            "help overlay should show Cargo package version {version}:\n{text}"
        );
        let line = text
            .lines()
            .rev()
            .find(|line| line.contains(version))
            .unwrap_or_else(|| panic!("expected version {version} in:\n{text}"));
        let idx = line.rfind(version).expect("version");
        let after = &line[idx + version.len()..];
        assert!(
            after
                .chars()
                .all(|c| c.is_whitespace() || matches!(c, '│' | '╯' | '╮' | '┘' | '┐' | '║' | '┤')),
            "package version should sit in the help overlay lower-right:\n{line}"
        );
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
            uncommitted: Some(true),
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
    fn commit_file_rows_reuse_trailing_status_badge() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                super::super::drill::CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                },
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                },
            ],
        );
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        let added = text
            .lines()
            .find(|line| line.contains("lib.rs"))
            .unwrap_or("");
        let name_at = added.find("lib.rs").expect("lib.rs on a commit-file row");
        let after_name = &added[name_at + "lib.rs".len()..];
        assert!(
            after_name.contains('A'),
            "commit-file A badge should sit to the right of the name: {added:?}"
        );
        assert!(
            !added.contains("A  lib") && !added.contains("A lib"),
            "badge must not prefix the file name: {added:?}"
        );
        let readme = text
            .lines()
            .find(|line| line.contains("README.md"))
            .unwrap_or("");
        let readme_at = readme.find("README.md").expect("README.md");
        assert!(
            readme[readme_at + "README.md".len()..].contains('M'),
            "commit-file M badge should sit to the right: {readme:?}"
        );
    }

    #[test]
    fn help_overlay_is_short() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        let backend = TestBackend::new(200, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("quit"), "{text}");
        assert!(text.contains("show / hide ignored"), "{text}");
        assert!(text.contains("stash menu"), "{text}");
        assert!(text.contains("push ahead"), "{text}");
        assert!(text.contains("remove linked worktree"), "{text}");
        assert!(text.contains("EasyMotion"), "{text}");
        assert!(text.contains("cycle theme"), "{text}");
        assert!(text.contains("/ search help"), "{text}");
        assert!(text.contains("MOVE"), "{text}");
        assert!(text.contains("GIT"), "{text}");
        assert!(text.contains("VIEW"), "{text}");
        let header = text
            .lines()
            .find(|line| line.contains("MOVE") && line.contains("GIT") && line.contains("VIEW"));
        assert!(
            header.is_some(),
            "help overlay should paint MOVE / GIT / VIEW on one row:\n{text}"
        );
        assert_help_version_lower_right(&text);
    }

    #[test]
    fn help_search_highlights_without_hiding_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        state.help_search_query = Some("quit".into());
        let backend = TestBackend::new(200, 40);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("MOVE"), "{text}");
        assert!(text.contains("stage scope"), "{text}");
        assert!(text.contains("quit"), "{text}");
        assert!(text.contains("Esc clears search"), "{text}");
        assert!(!text.contains("n/N wrap"), "{text}");
        assert_help_version_lower_right(&text);
    }

    #[test]
    fn confirm_overlays_are_boxed_not_status_line() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.confirm = Some(PendingConfirm::Revert {
            label: "README.md".into(),
            targets: vec![
                crate::tui::state::RevertTarget {
                    repo: "app".into(),
                    path: "README.md".into(),
                    untracked: false,
                    old_path: None,
                },
                crate::tui::state::RevertTarget {
                    repo: "app".into(),
                    path: "tmp.log".into(),
                    untracked: true,
                    old_path: None,
                },
            ],
        });
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Revert"), "{text}");
        assert!(text.contains("README.md"), "{text}");
        assert!(text.contains("tracked"), "{text}");
        assert!(text.contains("untracked"), "{text}");
        assert!(text.contains("discarded"), "{text}");
        assert!(text.contains("kept"), "{text}");
        assert!(text.contains("revert + delete untracked"), "{text}");
        assert!(!text.contains("? y/n"), "{text}");
        assert!(!text.contains("revert README.md? y/n"), "{text}");

        state.confirm = Some(PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Drop"), "{text}");
        assert!(text.contains("stash@{0}"), "{text}");
        assert!(text.contains("drop"), "{text}");
        assert!(text.contains("cancel"), "{text}");

        state.confirm = Some(PendingConfirm::RemoveWorktree {
            primary: "app".into(),
            path: ".worktrees/topic".into(),
            force: true,
            branch: "topic".into(),
            merged_into_default: Some(false),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Remove worktree"), "{text}");
        assert!(text.contains(".worktrees/topic"), "{text}");
        assert!(text.contains("NOT merged"), "{text}");
        assert!(text.contains("--force"), "{text}");

        state.confirm = Some(PendingConfirm::CheckoutOutOfSync {
            repo: "app".into(),
            branch: "main".into(),
            remote_ref: "origin/main".into(),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("is not in sync with"), "{text}");
        assert!(text.contains("Checkout local then pull?"), "{text}");
        assert!(text.contains("checkout then pull"), "{text}");

        state.confirm = Some(PendingConfirm::MergeIntoHead {
            repo: "app".into(),
            rev: "topic".into(),
            label: "topic".into(),
            into: "main".into(),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Merge"), "{text}");
        assert!(text.contains("topic"), "{text}");
        assert!(text.contains("into"), "{text}");
        assert!(text.contains("main"), "{text}");
        assert!(text.contains("fast-forward"), "{text}");
        assert!(text.contains("merge commit"), "{text}");
        assert!(!text.contains("? y/n"), "{text}");
    }

    #[test]
    fn row_match_bg_prefers_cursor_then_search_then_flash() {
        let palette = crate::tui::theme::ThemeId::TokyoNight.palette();
        let search_bg = crate::tui::theme::ThemeId::TokyoNight.pills().filter.bg;
        assert_eq!(
            row_match_bg(true, true, Some(palette.flash), palette, search_bg),
            Some(palette.cursor_bg)
        );
        assert_eq!(
            row_match_bg(false, true, Some(palette.flash), palette, search_bg),
            Some(search_bg)
        );
        assert_eq!(
            row_match_bg(false, false, Some(palette.flash), palette, search_bg),
            Some(palette.flash)
        );
        assert_eq!(row_match_bg(false, false, None, palette, search_bg), None);
    }

    #[test]
    fn flash_background_keeps_status_foreground() {
        let palette = crate::tui::theme::ThemeId::TokyoNight.palette();
        let segs = NodeSegments {
            segments: vec![TextSeg {
                text: "M".into(),
                role: SegRole::Modified,
                hex: None,
                bold: false,
                dim: false,
            }],
            trailing: Vec::new(),
        };
        let line = paint_segmented_row(
            0,
            false,
            false,
            &segs,
            20,
            false,
            Some(palette.flash),
            false,
            search_bg_unused(),
            true,
            None,
            palette,
            0,
        );
        assert!(
            line.spans
                .iter()
                .any(|span| span.style.fg == Some(palette.modified)),
            "status colour must survive flash background"
        );
        assert!(
            line.spans
                .iter()
                .any(|span| span.style.bg == Some(palette.flash)),
            "flash should paint background"
        );
    }

    fn search_bg_unused() -> Color {
        crate::tui::theme::ThemeId::TokyoNight.pills().filter.bg
    }

    #[test]
    fn search_match_paints_filter_bg_on_non_cursor_tree_rows() {
        let mut snapshot = repo("app", true);
        snapshot.changes = vec![
            FileChange {
                path: "a.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "b.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            },
        ];
        snapshot.has_unstaged = true;
        let snapshot = build_workspace_snapshot(&[snapshot], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.dispatch(super::super::action::Action::SearchStart);
        for c in "md".chars() {
            state.dispatch(super::super::action::Action::SearchChar(c));
        }
        let search_bg = state.theme.pills().filter.bg;
        let cursor_bg = state.theme.palette().cursor_bg;
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buf = terminal.backend().buffer();
        let mut a_bg = None;
        let mut b_bg = None;
        for y in 0..buf.area().height {
            let mut line = String::new();
            for x in 0..buf.area().width {
                line.push_str(buf[(x, y)].symbol());
            }
            if line.contains("a.md") {
                let col = line.find("a.md").unwrap();
                a_bg = Some(buf[(col as u16, y)].bg);
            }
            if line.contains("b.md") {
                let col = line.find("b.md").unwrap();
                b_bg = Some(buf[(col as u16, y)].bg);
            }
        }
        let a_bg = a_bg.expect("a.md row");
        let b_bg = b_bg.expect("b.md row");
        assert!(
            a_bg == cursor_bg || b_bg == cursor_bg,
            "one match should keep the cursor: a={a_bg:?} b={b_bg:?}"
        );
        assert!(
            a_bg == search_bg || b_bg == search_bg,
            "the other match should use search bg: a={a_bg:?} b={b_bg:?} search={search_bg:?}"
        );
        assert_ne!(a_bg, b_bg, "cursor and search-match paint must differ");
    }

    #[test]
    fn search_match_paints_filter_bg_on_non_cursor_commit_file_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "a.md".into(),
                    old_path: None,
                },
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "b.md".into(),
                    old_path: None,
                },
            ],
        );
        state.focus = FocusPane::Right;
        state.dispatch(super::super::action::Action::SearchStart);
        for c in "md".chars() {
            state.dispatch(super::super::action::Action::SearchChar(c));
        }
        let search_bg = state.theme.pills().filter.bg;
        let cursor_bg = state.theme.palette().cursor_bg;
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buf = terminal.backend().buffer();
        let mut a_bg = None;
        let mut b_bg = None;
        for y in 0..buf.area().height {
            let mut line = String::new();
            for x in 0..buf.area().width {
                line.push_str(buf[(x, y)].symbol());
            }
            if line.contains("a.md") {
                let col = line.find("a.md").unwrap();
                a_bg = Some(buf[(col as u16, y)].bg);
            }
            if line.contains("b.md") {
                let col = line.find("b.md").unwrap();
                b_bg = Some(buf[(col as u16, y)].bg);
            }
        }
        let a_bg = a_bg.expect("a.md file row");
        let b_bg = b_bg.expect("b.md file row");
        assert!(
            a_bg == cursor_bg || b_bg == cursor_bg,
            "one file match should keep the cursor: a={a_bg:?} b={b_bg:?}"
        );
        assert!(
            a_bg == search_bg || b_bg == search_bg,
            "the other file match should use search bg: a={a_bg:?} b={b_bg:?} search={search_bg:?}"
        );
        assert_ne!(a_bg, b_bg, "cursor and search-match paint must differ");
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

    #[test]
    fn draw_relayouts_panes_gutter_help_and_lists() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        let mut wide = Terminal::new(TestBackend::new(200, 40)).unwrap();
        wide.draw(|frame| draw(frame, &mut state)).unwrap();
        let wide_tree = state.layout.outer_tree_width;
        let wide_diff = state.layout.diff_pane_width;
        let wide_list = state.layout.tree_height;
        let wide_gutter = graph_gutter_cap(wide_diff.saturating_sub(1) as usize);

        let mut narrow = Terminal::new(TestBackend::new(80, 24)).unwrap();
        narrow.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(
            state.layout.outer_tree_width < wide_tree,
            "tree pane should shrink: {} vs {wide_tree}",
            state.layout.outer_tree_width
        );
        let narrow_gutter =
            graph_gutter_cap(state.layout.diff_pane_width.saturating_sub(1) as usize);
        assert!(
            narrow_gutter < wide_gutter,
            "graph gutter cap should follow pane width: {narrow_gutter} vs {wide_gutter}"
        );
        assert!(
            state.layout.tree_height < wide_list,
            "list viewport should shrink: {} vs {wide_list}",
            state.layout.tree_height
        );
        let text = buffer_text(&narrow);
        assert!(text.contains("MOVE"), "{text}");
        assert!(text.contains("GIT"), "{text}");
        assert!(text.contains("VIEW"), "{text}");
    }

    #[test]
    fn empty_tree_paints_no_matching_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.rows.clear();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains(NO_MATCHING_ROWS), "{text}");
        assert!(!text.contains("no files in this commit"), "{text}");
    }

    #[test]
    fn empty_commit_files_paint_loading_then_no_matching_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let source = super::super::drill::CommitFileSource::Commit {
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        };
        state.begin_commit_files("app".into(), source.clone());
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let loading = buffer_text(&terminal);
        assert!(loading.contains(LOADING_FILES), "{loading}");
        assert!(!loading.contains(NO_MATCHING_ROWS), "{loading}");
        assert!(!loading.contains("no files in this commit"), "{loading}");

        state.open_commit_files("app".into(), source, Vec::new());
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let empty = buffer_text(&terminal);
        assert!(empty.contains(NO_MATCHING_ROWS), "{empty}");
        assert!(!empty.contains(LOADING_FILES), "{empty}");
        assert!(!empty.contains("no files in this commit"), "{empty}");
    }
}

//! Ratatui paint for the tree, graph / diff pane, status, and help overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, StatefulWidget, Widget, Wrap,
};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;
use workspace_status_graph::{
    footer_message_scroll_max, graph_col_max, graph_hscroll_visible, graph_vscroll_visible,
    painted_line_count, short_id, GraphLabelPalette, GraphWidget,
};

use std::cell::RefCell;
use std::collections::HashSet;

use super::chrome::{
    breadcrumb_line, breadcrumb_rows, ctrl_c_prompt_line, ctrl_c_prompt_rows, export_shows_status,
    overlay_status_rows_for, status_line,
};
use super::comments::{
    comment_overlay_footer_save, commit_file_row_comments_resolved, commit_file_row_has_comment,
    graph_row_comments_resolved, graph_row_has_comment, tree_row_comments_resolved,
    tree_row_has_comment, CommentPrompt, COMMENT_OVERLAY_FOOTER_EDIT,
};
use super::commit_files::FolderSummary;
use super::diff::{
    cell_code_width, cell_sign, diff_failed_text, diff_pane_header, diff_pane_header_rows,
    diff_pane_mode_label, diff_row_content_width, diff_wrap_row_heights, gutter_width,
    section_header, wrap_viewport_start, DiffCell, DiffCellKind, DiffRow, DiffSection, DIFF_RULE,
};
use super::drill::DrillView;
use super::help::{
    help_chip_gap_spaces, help_column_content_width, help_column_widths, help_entry_matches,
    help_entry_visual_lines, help_groups, help_idle_footer_lines, help_inner_width, help_key_width,
    help_version_label, HELP_SEARCH_ESC_HINT,
};
use super::icons::{
    comment_mark_cols, glyph, icon_branch, icon_comment, icon_comment_resolved, icon_diff,
    icon_merged_into_default, icon_move, icon_open_vs_default, truncate_visible, CURSOR_BAR,
    CURSOR_BAR_INACTIVE, FOLD_COLLAPSED, FOLD_COLLAPSED_ASCII, FOLD_EXPANDED, FOLD_EXPANDED_ASCII,
};
use super::ops::RevertScope;
use super::search::{
    collect_commit_file_match_indices, collect_graph_match_indices, collect_match_ids, slice_cols,
    wrap_col_starts, wrap_cols, SearchPane,
};
use super::split::{
    diff_paint_width, diff_split_rule_x, pane_widths, side_by_side_column_widths, DiffMode,
    MIN_PANE_COLS, MIN_TERM_COLS, MIN_TERM_ROWS,
};
use super::state::{revert_scope, AppState, CompareRevertTarget, FocusPane, PendingConfirm};
use super::syntax::{
    cached_highlight_diff_rows, slice_styled_cols, CachedDiffSyntax, CodeSpan, DiffBackgrounds,
    DiffSyntaxKey,
};
use super::tabs::{
    compare_picker_empty, no_committed_changes_vs, ComparePickerState, NO_COMMITTED_CHANGES,
};
use super::theme::{hex_color, Palette, Pill};
use super::tree::{
    file_change_from_name_status, file_change_segments, row_segments, visible_window,
    with_comment_mark, with_viewed_mark, workspace_trailing_fit, NodeKind, NodeSegments, SegRole,
    TextSeg, VisibleRow,
};
use crate::helpers::{is_detached_head_branch, visible_width};

/// Empty tree / empty commit-file list.
const NO_MATCHING_ROWS: &str = "No matching rows";
/// Right pane while the focused repo's graph loads.
const LOADING_GRAPH: &str = "loading graph…";
/// Empty commit-file list for a commit, after its files loaded.
const NO_FILES_IN_COMMIT: &str = "no files in this commit";
/// Empty commit-file list for a stash, after its files loaded.
const NO_FILES_IN_STASH: &str = "no files in this stash";
/// Empty commit-file list for the uncommitted row, after its files loaded.
const NO_FILES_IN_WORKTREE: &str = "no uncommitted changes";
/// Commit-file list while git is still listing.
const LOADING_FILES: &str = "loading files…";

/// Pane `Block::title`: the plain pane name for focused and unfocused.
///
/// Names are exactly `tree`, `graph`, `files`, or `diff`. Focus is the
/// border colour, not a title glyph or space-pad. Title text uses
/// `palette.heading` via `title_style` so it does not inherit `border_style`.
fn pane_title(name: &str) -> String {
    name.to_string()
}

fn muted_copy(text: &'static str, palette: Palette) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(palette.muted)))
}

/// Flash → focused cursor → inactive cursor → search match.
fn row_match_bg(
    selected: bool,
    focused: bool,
    search_match: bool,
    flash: Option<Color>,
    palette: Palette,
    search_bg: Color,
) -> Option<Color> {
    if flash.is_some() {
        flash
    } else if selected && focused {
        Some(palette.cursor_bg)
    } else if selected {
        Some(palette.cursor_bg_inactive)
    } else if search_match {
        Some(search_bg)
    } else {
        None
    }
}

fn selection_marker(selected: bool, focused: bool) -> &'static str {
    if !selected {
        " "
    } else if focused {
        CURSOR_BAR
    } else {
        CURSOR_BAR_INACTIVE
    }
}

/// Draw one frame. Updates `state.layout` for mouse hits.
pub fn draw(frame: &mut Frame<'_>, state: &mut AppState) {
    state.prune_expired_flashes();
    let area = frame.area();
    state.too_small = area.width < MIN_TERM_COLS || area.height < MIN_TERM_ROWS;
    if state.too_small {
        draw_too_small(frame, area, state.theme.palette());
        return;
    }
    let overlay_h = overlay_status_rows_for(state, area.width);
    let crumb_h = breadcrumb_rows(state);
    let prompt_h = ctrl_c_prompt_rows(state);
    let tab_h = 1u16;
    let chrome_h = crumb_h
        .saturating_add(prompt_h)
        .saturating_add(overlay_h)
        .saturating_add(tab_h);
    // Help keeps its wrapped row budget. Panes take leftover rows (this
    // can be fewer than the idle Min(3)). A fixed Min(3) clips the last
    // GIT wrap at the default 140×32 PTY.
    let pane_min = if state.help_open {
        area.height.saturating_sub(chrome_h)
    } else {
        3
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(tab_h),
            Constraint::Min(pane_min),
            Constraint::Length(crumb_h),
            Constraint::Length(prompt_h),
            Constraint::Length(overlay_h),
        ])
        .split(area);
    draw_tab_strip(frame, chunks[0], state);
    let widths = pane_widths(area.width, state.tree_fraction);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(widths.tree_width),
            Constraint::Min(MIN_PANE_COLS),
        ])
        .split(chunks[1]);

    let left_is_files = state.drill.is_diff() || state.is_compare_tab();
    let left_is_graph = !state.is_compare_tab() && state.drill.is_files();
    state.layout.graph_scrollbar_x = None;
    state.layout.graph_scrollbar_y = 0;
    state.layout.graph_scrollbar_height = 0;
    state.layout.graph_content_len = 0;
    state.layout.graph_hscrollbar_y = None;
    state.layout.graph_hscrollbar_x = 0;
    state.layout.graph_hscrollbar_width = 0;
    state.layout.graph_col_max = 0;
    state.layout.graph_footer_y = None;
    state.layout.graph_footer_scroll_max = 0;
    state.layout.diff_scrollbar_x = None;
    state.layout.diff_scrollbar_y = 0;
    state.layout.diff_scrollbar_height = 0;
    state.layout.diff_hscrollbar_y = None;
    state.layout.diff_hscrollbar_x = 0;
    state.layout.diff_hscrollbar_width = 0;
    state.layout.diff_col_max = 0;
    let left_name = state.left_pane_title();
    let palette = state.theme.palette();
    let title_style = Style::default().fg(palette.heading);
    let left_title = pane_title(left_name);
    let tree_block = Block::default()
        .borders(Borders::ALL)
        .title(left_title)
        .title_style(title_style)
        .border_style(pane_border(state.focus == FocusPane::Left, palette));
    let tree_inner = tree_block.inner(panes[0]);
    frame.render_widget(tree_block, panes[0]);
    if left_is_files {
        let cursor = state.commit_files_cursor();
        let col_offset = state.left_col_offset as usize;
        if state.is_compare_tab() {
            draw_commit_file_list(frame, tree_inner, state, cursor, col_offset);
        } else {
            draw_commit_detail(frame, tree_inner, state, cursor, col_offset);
        }
    } else if left_is_graph {
        draw_graph(frame, tree_inner, state, state.left_col_offset);
    } else {
        draw_tree(frame, tree_inner, state);
    }

    let right_name = if state.is_compare_tab() {
        "diff"
    } else if state.drill.is_files() {
        "files"
    } else if state.drill.is_diff() || state.right_is_diff() {
        "diff"
    } else {
        "graph"
    };
    let right_title = pane_title(right_name);
    let right_block = Block::default()
        .borders(Borders::ALL)
        .title(right_title)
        .title_style(title_style)
        .border_style(pane_border(state.focus == FocusPane::Right, palette));
    let right_inner = right_block.inner(panes[1]);
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    frame.render_widget(right_block, panes[1]);
    draw_right(frame, right_inner, state);

    if crumb_h > 0 {
        frame.render_widget(
            Paragraph::new(breadcrumb_line(state, chunks[2].width)),
            chunks[2],
        );
    }
    if prompt_h > 0 {
        frame.render_widget(
            Paragraph::new(ctrl_c_prompt_line(state, chunks[3].width)),
            chunks[3],
        );
    }
    let overlay = chunks[4];
    if state.help_open {
        draw_help(frame, overlay, state);
    } else if state.confirm.is_some() {
        draw_confirm(frame, overlay, state);
    } else if state.stash_menu.is_some() {
        draw_stash_menu(frame, overlay, state);
    } else if state.create_branch.is_some() {
        draw_create_branch(frame, overlay, state);
    } else if state.comment.is_some() {
        draw_comment(frame, overlay, state);
    } else if state.comment_export.is_some() {
        draw_comment_export(frame, overlay, state);
    } else if state.branch_picker.is_some() {
        draw_branch_picker(frame, overlay, state);
    } else if state.compare_picker.is_some() {
        draw_compare_picker(frame, overlay, state);
    } else if state.graph_focus_picker.is_some() {
        draw_graph_focus_picker(frame, overlay, state);
    } else if state.command_palette.is_some() {
        draw_command_palette(frame, overlay, state);
    } else {
        frame.render_widget(Paragraph::new(status_line(state, overlay.width)), overlay);
    }

    state.layout.tree_x = tree_inner.x;
    state.layout.tree_y = tree_inner.y;
    state.layout.tree_width = tree_inner.width;
    state.layout.tree_height = tree_inner.height;
    state.layout.right_x = panes[1].x;
    state.layout.term_cols = area.width;
    state.layout.pane_height = chunks[1].height;
    state.layout.outer_tree_width = panes[0].width;
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    state.layout.diff_content_x = right_inner.x;
    // No rule to drag while a folder summary stands in for the diff.
    state.layout.diff_split_rule_x = if state.right_is_diff()
        && state.folder_summary().is_none()
        && state.diff_layout() == DiffMode::SideBySide
    {
        let split = side_by_side_column_widths(right_inner.width, state.diff_split_fraction);
        Some(diff_split_rule_x(panes[0].width, split.left_width).saturating_sub(1))
    } else {
        None
    };

    state.layout.right_y = right_inner.y;
    if state.is_compare_tab() {
        let cursor = state.commit_files_cursor();
        state.layout.files_list_y = tree_inner.y;
        state.layout.files_list_height = tree_inner.height;
        let list_h = tree_inner.height as usize;
        let (start, _) = visible_window(state.painted_commit_file_rows().len(), cursor, list_h);
        state.layout.files_list_offset = start;
    } else if let super::drill::DrillView::Diff { file_cursor, .. } = &state.drill {
        let cursor = *file_cursor;
        let footer_len = state
            .commit_detail_footer_lines(tree_inner.width as usize)
            .len();
        let footer_h = commit_detail_footer_height(footer_len, tree_inner.height);
        let list_h = tree_inner.height.saturating_sub(footer_h);
        state.layout.files_list_y = tree_inner.y;
        state.layout.files_list_height = list_h;
        let painted_n = state.painted_commit_file_rows().len();
        let (start, _) = visible_window(painted_n, cursor, list_h as usize);
        state.layout.files_list_offset = start;
    } else if let super::drill::DrillView::Files { cursor, .. } = &state.drill {
        state.layout.files_list_y = right_inner.y;
        state.layout.files_list_height = right_inner.height;
        let list_h = right_inner.height as usize;
        let painted_n = state.painted_commit_file_rows().len();
        let (start, _) = visible_window(painted_n, *cursor, list_h);
        state.layout.files_list_offset = start;
    }

    // Keep the frame as painted so a mouse release copies what is on screen,
    // then reverse the selected cells on top of it.
    state.painted_frame = frame.buffer_mut().clone();
    if let Some(selection) = state.text_selection.as_ref() {
        selection.highlight(frame.buffer_mut());
    }
}

fn pane_border(focused: bool, palette: Palette) -> Style {
    if focused {
        Style::default().fg(palette.heading)
    } else {
        Style::default().fg(palette.border_dim)
    }
}

/// Resize notice in place of the panes when the terminal is below
/// [`MIN_TERM_COLS`] × [`MIN_TERM_ROWS`]. Keys still dispatch (`q` quits).
fn draw_too_small(frame: &mut Frame<'_>, area: Rect, palette: Palette) {
    let text = too_small_notice(area.width, area.height);
    let lines = wrap_cols(&text, area.width.max(1) as usize);
    let top = area.height.saturating_sub(lines.len() as u16) / 2;
    let body = Rect {
        x: area.x,
        y: area.y.saturating_add(top),
        width: area.width,
        height: area.height.saturating_sub(top),
    };
    let lines: Vec<Line> = lines
        .into_iter()
        .map(|line| Line::from(Span::styled(line, Style::default().fg(palette.modified))))
        .collect();
    frame.render_widget(
        Paragraph::new(lines).alignment(ratatui::layout::Alignment::Center),
        body,
    );
}

/// `terminal too small (W×H — need ≥ 46×12)`.
fn too_small_notice(cols: u16, rows: u16) -> String {
    format!("terminal too small ({cols}×{rows} — need ≥ {MIN_TERM_COLS}×{MIN_TERM_ROWS})")
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
    let search = state.theme.pills().filter;
    let match_ids: HashSet<String> = if state.search_target == SearchPane::Tree {
        collect_match_ids(&state.tree, &state.search_query)
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let mut lines = Vec::new();
    for row in painted.iter().skip(start).take(height) {
        let viewed = row.kind == NodeKind::File && state.reviewed.contains(&row.id);
        let commented = tree_row_has_comment(&state.comment_store, &state.snapshot, row);
        let resolved =
            commented && tree_row_comments_resolved(&state.comment_store, &state.snapshot, row);
        lines.push(paint_tree_row(
            row,
            width,
            Some(row.id.as_str()) == focus_id,
            state.focus == FocusPane::Left,
            state.flash_color(&row.id),
            match_ids.contains(&row.id),
            search,
            state.ascii,
            viewed,
            commented,
            resolved,
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
    focused: bool,
    flash: Option<Color>,
    search_match: bool,
    search: Pill,
    ascii: bool,
    viewed: bool,
    commented: bool,
    resolved: bool,
    palette: Palette,
    col_offset: usize,
) -> Line<'static> {
    let segs_width =
        |segs: &[TextSeg]| -> usize { segs.iter().map(|s| visible_width(&s.text)).sum() };
    let mut segs = row_segments(row, ascii, viewed, commented, resolved);
    if row.kind == NodeKind::Workspace {
        // The summary gives way before the name. Same prefix as
        // `paint_segmented_row` (edge, indent, chevron); comment marks
        // stay ahead of the summary.
        let prefix_width = 1 + 2 * row.depth + 2;
        let name_width = segs_width(&row.segments).saturating_sub(col_offset);
        let marks = segs.trailing.len().saturating_sub(row.trailing_segs.len());
        let marks_width = segs_width(&segs.trailing[..marks]);
        let room = width
            .saturating_sub(prefix_width)
            .saturating_sub(name_width)
            .saturating_sub(marks_width);
        segs.trailing.truncate(marks);
        segs.trailing
            .extend(workspace_trailing_fit(&row.chrome, room));
    }
    paint_segmented_row(
        row.depth,
        row.foldable,
        row.folded,
        &segs,
        width,
        selected,
        focused,
        flash,
        search_match,
        search,
        ascii,
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
    focused: bool,
    flash: Option<Color>,
    search_match: bool,
    search: Pill,
    ascii: bool,
    palette: Palette,
    col_offset: usize,
) -> Line<'static> {
    let bg = row_match_bg(selected, focused, search_match, flash, palette, search.bg);
    // A search-match row paints its text in the filter foreground: the
    // filter background can equal a segment colour (branch names).
    let match_fg = (flash.is_none() && !selected && search_match).then_some(search.fg);
    let trailing_text: String = segs.trailing.iter().map(|s| s.text.as_str()).collect();
    let trailing_width = visible_width(&trailing_text);
    let pad = usize::from(trailing_width > 0);

    let mut spans: Vec<Span> = Vec::new();
    let edge = selection_marker(selected, focused);
    let mut edge_style = Style::default().fg(if focused {
        palette.cursor
    } else {
        palette.muted
    });
    if focused {
        edge_style = edge_style.add_modifier(Modifier::BOLD);
    }
    spans.push(styled_span(edge, edge_style, bg));

    let indent = "  ".repeat(depth);
    spans.push(styled_span(&indent, Style::default(), bg));
    let chevron = fold_chevron(foldable, folded, ascii);
    spans.push(styled_span(
        &format!("{chevron} "),
        Style::default().fg(palette.muted),
        bg,
    ));
    let prefix_width = 1 + visible_width(&indent) + 2;

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
    if let Some(fg) = match_fg {
        for span in &mut spans {
            span.style = span.style.fg(fg).remove_modifier(Modifier::DIM);
        }
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
            SegRole::Viewed => palette.viewed,
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
    // A focused folder beside a diff wins over the kept file diff and the
    // compare empty copy (a range reload can leave the folder with no path).
    if let Some(summary) = state.folder_summary() {
        draw_folder_summary(frame, area, state, &summary);
        return;
    }
    if state.is_compare_tab() {
        draw_diff_pane(frame, area, state);
        return;
    }
    match &state.drill {
        DrillView::Files { cursor, .. } => {
            let col_offset = state.right_col_offset as usize;
            draw_commit_file_list(frame, area, state, *cursor, col_offset);
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
    // A focused repo always has a graph load in flight until the graph
    // lands (a file diff clears the graph it replaced).
    let copy = if state.focused_graph_repo().is_some() {
        LOADING_GRAPH
    } else {
        "focus a repo for the graph, or a file for its diff"
    };
    frame.render_widget(Paragraph::new(copy), area);
}

/// Folder summary in the diff pane: `dir/` and its totals, then one row per
/// changed file under it. Rows past the pane height fold into `… N more`.
fn draw_folder_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    summary: &FolderSummary,
) {
    let palette = state.theme.palette();
    let ascii = state.ascii;
    let minus = glyph(ascii, "−", "-");
    let title = format!("{}/", summary.dir);
    let n = summary.files.len();
    let mut extra = format!("  {n} {}", if n == 1 { "file" } else { "files" });
    if summary.files.iter().any(|file| file.stat.is_some()) {
        extra.push_str(&format!(" · +{} {minus}{}", summary.added, summary.deleted));
    }
    // Same header shape as the file diff: the title wraps by columns and
    // the muted extras follow on its last row.
    let header_h = diff_pane_header_rows(&title, area.width, area.height);
    let heading = Style::default()
        .fg(palette.heading)
        .add_modifier(Modifier::BOLD);
    let mut header_lines: Vec<Line> = wrap_cols(&title, area.width as usize)
        .into_iter()
        .take(header_h as usize)
        .map(|chunk| Line::from(Span::styled(chunk, heading)))
        .collect();
    if let Some(last) = header_lines.last_mut() {
        last.spans
            .push(Span::styled(extra, Style::default().fg(palette.muted)));
    }
    let header_area = Rect {
        height: header_h.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(header_lines), header_area);
    if area.height <= header_h {
        return;
    }
    let body = Rect {
        y: area.y.saturating_add(header_h),
        height: area.height - header_h,
        ..area
    };
    let list_h = body.height as usize;
    let shown = if n > list_h {
        list_h.saturating_sub(1)
    } else {
        n
    };
    let width = body.width as usize;
    let search = state.theme.pills().filter;
    let paint = |segs: &NodeSegments| {
        paint_segmented_row(
            0, false, false, segs, width, false, false, None, false, search, ascii, palette, 0,
        )
    };
    let mut lines: Vec<Line> = summary.files[..shown]
        .iter()
        .map(|file| {
            let change = file_change_from_name_status(
                &file.status,
                file.path.clone(),
                file.old_path.clone(),
            );
            let mut segs = file_change_segments(&change, false, ascii);
            if let Some(stat) = file.stat {
                // Counts sit before the badge so badges line up as in the list.
                let badge = std::mem::take(&mut segs.trailing);
                segs.trailing = vec![
                    TextSeg {
                        text: format!("+{}", stat.added),
                        role: SegRole::Added,
                        hex: None,
                        bold: false,
                        dim: false,
                    },
                    TextSeg {
                        text: format!(" {minus}{}  ", stat.deleted),
                        role: SegRole::Deleted,
                        hex: None,
                        bold: false,
                        dim: false,
                    },
                ];
                segs.trailing.extend(badge);
            }
            paint(&segs)
        })
        .collect();
    if shown < n {
        lines.push(paint(&NodeSegments {
            segments: vec![TextSeg {
                text: format!("… {} more", n - shown),
                role: SegRole::Muted,
                hex: None,
                bold: false,
                dim: false,
            }],
            trailing: Vec::new(),
        }));
    }
    frame.render_widget(Paragraph::new(lines), body);
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
    let lane_colors = state.theme.lane_colors();
    let commented_rows = graph_commented_row_indices(state, false);
    let resolved_comment_rows = graph_commented_row_indices(state, true);
    let graph_focused = if state.drill.is_files() {
        state.focus == FocusPane::Left
    } else {
        state.focus == FocusPane::Right
    };
    GraphWidget::new(model)
        .ascii(state.ascii)
        .selected(Some(state.graph_cursor))
        .cursor_bar(graph_focused)
        .scroll(state.graph_scroll)
        .col_offset(col_offset)
        .loading_older(state.graph_loading_older)
        .search_matches(
            &matches,
            state.theme.pills().filter.bg,
            state.theme.pills().filter.fg,
        )
        .flash_rows(&flash_rows)
        .commented_rows(&commented_rows)
        .resolved_comment_rows(&resolved_comment_rows)
        .comment_glyph(icon_comment(state.ascii))
        .resolved_comment_glyph(icon_comment_resolved(state.ascii))
        .cursor_style(pal.cursor, pal.cursor_bg)
        .cursor_inactive_style(pal.muted, pal.cursor_bg_inactive)
        .commit_msg_expand(state.commit_msg_expand)
        .commit_msg_scroll(state.graph_footer_msg_scroll())
        .lane_colors(&lane_colors)
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
    record_graph_scrollbar(state, area, col_offset);
}

fn graph_commented_row_indices(state: &AppState, resolved_only: bool) -> Vec<usize> {
    let Some(model) = state.graph.as_ref() else {
        return Vec::new();
    };
    let Some((repo, _)) = state.graph_identity.as_ref() else {
        return Vec::new();
    };
    let primary = state
        .snapshot
        .repos
        .iter()
        .find(|r| r.repo == *repo)
        .and_then(|r| r.primary_repo.as_deref());
    let branch = state
        .snapshot
        .repos
        .iter()
        .find(|r| r.repo == *repo)
        .map(|r| r.branch.as_str());
    model
        .visible_rows()
        .iter()
        .enumerate()
        .filter_map(|(i, row)| {
            if !graph_row_has_comment(&state.comment_store, repo, primary, row, branch) {
                return None;
            }
            let resolved =
                graph_row_comments_resolved(&state.comment_store, repo, primary, row, branch);
            if resolved_only {
                resolved.then_some(i)
            } else {
                (!resolved).then_some(i)
            }
        })
        .collect()
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

fn record_graph_scrollbar(state: &mut AppState, area: Rect, col_offset: u16) {
    if state.graph.is_none() {
        return;
    }
    if area.width == 0 || area.height == 0 {
        return;
    }
    let chrome = state.graph_chrome_in(area.height, area.width);
    let Some(model) = state.graph.as_ref() else {
        return;
    };
    // The widget already painted this frame; count without a second paint.
    let content_len = painted_line_count(model);
    state.layout.graph_content_len = content_len;
    let vscroll = graph_vscroll_visible(content_len, chrome.list_height, state.graph_scroll);
    let hscroll = graph_hscroll_visible(col_offset);
    let list_top = area.y.saturating_add(u16::from(chrome.header));
    let list_height = chrome.list_height;
    let footer_scroll_max = footer_message_scroll_max(
        state.graph_footer_line_count(area.width as usize),
        chrome.footer_height,
    );
    if chrome.footer && footer_scroll_max > 0 {
        let bottom = area
            .y
            .saturating_add(area.height)
            .saturating_sub(u16::from(chrome.older));
        state.layout.graph_footer_y = Some(bottom.saturating_sub(chrome.footer_height));
        state.layout.graph_footer_x = area.x;
        state.layout.graph_footer_width = area.width;
        state.layout.graph_footer_height = chrome.footer_height;
        state.layout.graph_footer_scroll_max = footer_scroll_max;
    }
    if vscroll && list_height > 0 {
        state.layout.graph_scrollbar_x = Some(area.x.saturating_add(area.width.saturating_sub(1)));
        state.layout.graph_scrollbar_y = list_top;
        state.layout.graph_scrollbar_height = list_height;
    }
    if hscroll && list_height > 0 {
        let v_cols = u16::from(vscroll);
        let max = graph_col_max(model, state.ascii, area.width, vscroll);
        if max > 0 {
            state.layout.graph_hscrollbar_y =
                Some(list_top.saturating_add(list_height).saturating_sub(1));
            state.layout.graph_hscrollbar_x = area.x;
            state.layout.graph_hscrollbar_width = area.width.saturating_sub(v_cols).max(1);
            state.layout.graph_col_max = max.min(u16::MAX as usize) as u16;
        }
    }
}

/// Rows the commit-message footer takes at the bottom of a commit-files pane
/// `pane_h` rows tall, for a footer of `footer_len` lines.
///
/// The file list keeps at least one row whenever the pane has two or more,
/// so a short terminal never hides the list for the message. Draw and
/// layout both size the footer here so the list rows and click mapping
/// agree.
fn commit_detail_footer_height(footer_len: usize, pane_h: u16) -> u16 {
    let footer_len = footer_len.min(u16::MAX as usize) as u16;
    let max = if pane_h >= 2 { pane_h - 1 } else { pane_h };
    footer_len.min(max)
}

/// Commit files beside the file diff (depth 2, left pane): the file list on
/// top, the selected commit's title, meta, and message pinned to the bottom
/// rows as a footer (like the graph selection footer on the graph pane).
///
/// `col_offset` is the horizontal pan of the file list.
fn draw_commit_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut AppState,
    cursor: usize,
    col_offset: usize,
) {
    let footer = state.commit_detail_footer_lines(area.width as usize);
    let footer_h = commit_detail_footer_height(footer.len(), area.height);
    let list_h = area.height.saturating_sub(footer_h);
    if list_h > 0 {
        let list_area = Rect {
            x: area.x,
            y: area.y,
            width: area.width,
            height: list_h,
        };
        draw_commit_file_list(frame, list_area, state, cursor, col_offset);
    }
    if footer_h == 0 {
        return;
    }
    let palette = state.theme.palette();
    // Clip keeps the first lines: title, meta, then message lines.
    let footer_lines: Vec<Line> = footer
        .iter()
        .take(footer_h as usize)
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
    let footer_area = Rect {
        x: area.x,
        y: area.y.saturating_add(list_h),
        width: area.width,
        height: footer_h,
    };
    frame.render_widget(Paragraph::new(footer_lines), footer_area);
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
        if let Some(tab) = state.tabs.active_compare() {
            if let Some(err) = tab.error.as_deref() {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        err.to_string(),
                        Style::default().fg(palette.muted),
                    )),
                    area,
                );
                return;
            }
            let copy = if tab.loading {
                LOADING_FILES
            } else {
                NO_COMMITTED_CHANGES
            };
            frame.render_widget(Paragraph::new(muted_copy(copy, palette)), area);
            return;
        }
        // Commit-file lists have no row filter: empty after the load means
        // the source has no files.
        let copy = if state.commit_files_loading {
            LOADING_FILES
        } else {
            match state.commit_drill_source().map(|(_, source)| source) {
                Some(super::drill::CommitFileSource::Stash { .. }) => NO_FILES_IN_STASH,
                Some(super::drill::CommitFileSource::Worktree) => NO_FILES_IN_WORKTREE,
                _ => NO_FILES_IN_COMMIT,
            }
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
    let search = state.theme.pills().filter;
    let searching_files =
        state.search_target == SearchPane::CommitFiles && !state.search_query.trim().is_empty();
    let match_paths = commit_file_search_match_paths(state);
    let comment_scope = commit_file_comment_scope(state);
    let files_focused = if state.drill.is_diff() || state.is_compare_tab() {
        state.focus == FocusPane::Left
    } else {
        state.focus == FocusPane::Right
    };
    let lines: Vec<Line> = rows
        .iter()
        .skip(start)
        .take(height)
        .map(|row| {
            let commented = comment_scope.is_some_and(|(repo, primary, branch, source)| {
                row.is_file()
                    && commit_file_row_has_comment(
                        &state.comment_store,
                        repo,
                        primary,
                        source,
                        &row.path,
                        branch,
                    )
            });
            let resolved = commented
                && comment_scope.is_some_and(|(repo, primary, branch, source)| {
                    commit_file_row_comments_resolved(
                        &state.comment_store,
                        repo,
                        primary,
                        source,
                        &row.path,
                        branch,
                    )
                });
            let segs = NodeSegments {
                segments: row.segments.clone(),
                trailing: with_viewed_mark(
                    with_comment_mark(row.trailing_segs.clone(), state.ascii, commented, resolved),
                    state.ascii,
                    state.compare_file_reviewed(row),
                ),
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
                files_focused,
                state.commit_file_flash_color(&row.id),
                search_match,
                search,
                state.ascii,
                palette,
                col_offset,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

fn commit_file_comment_scope(
    state: &AppState,
) -> Option<(
    &str,
    Option<&str>,
    Option<&str>,
    &super::drill::CommitFileSource,
)> {
    let (repo, source) = state.commit_drill_source()?;
    let snap = state.snapshot.repos.iter().find(|r| r.repo == repo);
    Some((
        repo,
        snap.and_then(|r| r.primary_repo.as_deref()),
        snap.map(|r| r.branch.as_str()),
        source,
    ))
}

fn commit_file_search_match_paths(state: &AppState) -> HashSet<String> {
    if state.search_target != SearchPane::CommitFiles {
        return HashSet::new();
    }
    let Some(files) = state.commit_drill_files() else {
        return HashSet::new();
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

fn draw_diff_pane(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    state.drop_stale_diff_visual();
    let palette = state.theme.palette();
    let path = state.diff_header_path();
    // One split decision for row build, header, and paint (`diff_pane_mode`
    // reserves the scrollbar column whether or not the bar shows).
    let effective = state.diff_layout();
    let split = effective == DiffMode::SideBySide;
    let rows = state.current_diff_rows();
    if !rows.is_empty() {
        state.diff_cursor = state.diff_cursor.min(rows.len() - 1);
    }
    let wrap = state.diff_wrap;
    let hscroll = state.diff_hscroll_shown();
    let h_rows = u16::from(hscroll);
    // Rows always leave the scrollbar column, whether or not the bar shows,
    // so widths (split, wrap, pan) never change with the bar.
    let line_width = diff_paint_width(area.width);
    let header_h = diff_pane_header_rows(&path, area.width, area.height);
    let list_h = area.height.saturating_sub(header_h).max(1);
    let line_h = list_h.saturating_sub(h_rows).max(1) as usize;
    let gutter = gutter_width(&rows);
    let gutter_with_mark = gutter.saturating_add(comment_mark_cols(state.ascii));
    // Viewport start and the visual rows the whole diff paints.
    let (start, content_rows) = if wrap {
        let heights = diff_wrap_row_heights(
            &rows,
            diff_row_content_width(line_width as usize) as u16,
            gutter_with_mark,
            split,
            state.diff_split_fraction,
        );
        let start = wrap_viewport_start(&heights, state.diff_cursor, line_h);
        (start, heights.iter().map(|h| (*h).max(1)).sum::<usize>())
    } else {
        (
            visible_window(rows.len(), state.diff_cursor, line_h).0,
            rows.len(),
        )
    };
    state.diff_scroll = start as u16;
    let skip = start;
    let vscroll = graph_vscroll_visible(content_rows, line_h as u16, state.diff_scroll);
    let v_cols = u16::from(vscroll);
    let mode_label = diff_pane_mode_label(state.diff_mode, effective);
    let header = diff_pane_header(
        &path,
        mode_label,
        state.full_context_active(),
        wrap,
        if wrap { 0 } else { state.diff_col_offset },
        skip,
        line_h,
        rows.len(),
    );
    let title = if path.is_empty() {
        "Diff"
    } else {
        path.as_str()
    };
    let extra = header.strip_prefix(title).unwrap_or("").to_string();
    let heading = Style::default()
        .fg(palette.heading)
        .add_modifier(Modifier::BOLD);
    // Break the title anywhere by display columns so the row count
    // matches `diff_pane_header_rows`; the extras follow on the last row.
    let mut header_lines: Vec<Line> = wrap_cols(title, area.width as usize)
        .into_iter()
        .take(header_h as usize)
        .map(|chunk| Line::from(Span::styled(chunk, heading)))
        .collect();
    if let Some(last) = header_lines.last_mut() {
        last.spans
            .push(Span::styled(extra, Style::default().fg(palette.muted)));
    }
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: header_h.min(area.height),
    };
    frame.render_widget(Paragraph::new(header_lines), header_area);
    if area.height <= header_h {
        return;
    }
    let body = Rect {
        x: area.x,
        y: area.y.saturating_add(header_h),
        width: area.width,
        height: list_h,
    };
    if rows.is_empty() {
        let mut color = palette.muted;
        let msg = if let Some(tab) = state.tabs.active_compare() {
            tab.error
                .clone()
                .unwrap_or_else(|| no_committed_changes_vs(&tab.base_ref))
        } else if path.is_empty() {
            "select a dirty file".to_string()
        } else if let Some(err) = state.current_diff_content().error.as_deref() {
            color = palette.deleted;
            diff_failed_text(err)
        } else {
            "(no diff)".to_string()
        };
        frame.render_widget(
            Paragraph::new(Span::styled(msg, Style::default().fg(color))),
            body,
        );
        return;
    }
    let off = if wrap {
        0
    } else {
        state.diff_col_offset as usize
    };
    let content_w = diff_row_content_width(line_width as usize) as u16;
    let content_len = rows.len();
    let paint_heights = if wrap {
        diff_wrap_row_heights(
            &rows,
            content_w,
            gutter_with_mark,
            split,
            state.diff_split_fraction,
        )
    } else {
        vec![1; rows.len()]
    };
    let mut logical_end = skip;
    let mut vis = 0usize;
    while logical_end < rows.len() && vis < line_h {
        vis = vis.saturating_add(paint_heights.get(logical_end).copied().unwrap_or(1).max(1));
        logical_end = logical_end.saturating_add(1);
    }
    let syntax_path = diff_syntax_path(state);
    let syntax = DIFF_SYNTAX_CACHE.with(|slot| {
        let mut cache = slot.borrow_mut();
        cached_highlight_diff_rows(
            &mut cache,
            DiffSyntaxKey {
                path: syntax_path.to_string(),
                theme: state.theme,
                fallback: palette.repo,
                bgs: DiffBackgrounds {
                    add: palette.diff_add_bg,
                    del: palette.diff_del_bg,
                    add_word: palette.diff_add_word_bg,
                    del_word: palette.diff_del_word_bg,
                },
                visible_start: skip,
                visible_end: logical_end,
                cache_id: diff_syntax_cache_id(state, split),
            },
            &rows,
        )
    });
    // Armed diff search marks every matching row; the cursor bar sits on
    // the current hit.
    let search_hits: HashSet<usize> = if state.search_target == SearchPane::Diff {
        state
            .diff_search_hits(&state.search_query)
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let search = state.theme.pills().filter;
    let mut painted: Vec<Line> = Vec::new();
    let mut i = skip;
    while painted.len() < line_h && i < rows.len() {
        let search_match = search_hits.contains(&i);
        let lines = paint_diff_row(
            &rows[i],
            content_w,
            gutter,
            split,
            off,
            wrap,
            state,
            i == state.diff_cursor,
            state.focus == FocusPane::Right,
            state.diff_visual_contains(i),
            search_match.then_some(search),
            syntax.left(i),
            syntax.right(i),
        );
        for line in lines {
            if painted.len() >= line_h {
                break;
            }
            painted.push(line);
        }
        i = i.saturating_add(1);
    }
    let lines_area = Rect {
        x: body.x,
        y: body.y,
        width: line_width,
        height: line_h as u16,
    };
    frame.render_widget(Paragraph::new(painted), lines_area);
    let buf = frame.buffer_mut();
    if vscroll && body.height > 0 && area.width > 0 {
        state.layout.diff_scrollbar_x = Some(area.x.saturating_add(area.width.saturating_sub(1)));
        state.layout.diff_scrollbar_y = body.y;
        state.layout.diff_scrollbar_height = body.height;
        let mut sb_state = ScrollbarState::new(content_len.saturating_sub(1))
            .position(skip.min(content_len.saturating_sub(1)));
        let sb_area = Rect {
            x: area.x.saturating_add(area.width.saturating_sub(1)),
            y: body.y,
            width: 1,
            height: body.height,
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
    let col_max = if hscroll {
        state.diff_pan_max_for(&rows)
    } else {
        0
    };
    if col_max > 0 && body.height > 0 && area.width > 0 {
        state.layout.diff_hscrollbar_y = Some(body.y.saturating_add(body.height.saturating_sub(1)));
        state.layout.diff_hscrollbar_x = area.x;
        state.layout.diff_hscrollbar_width = area.width.saturating_sub(v_cols).max(1);
        state.layout.diff_col_max = col_max.min(u16::MAX as usize) as u16;
        let mut sb_state =
            ScrollbarState::new(col_max).position((state.diff_col_offset as usize).min(col_max));
        let sb_area = Rect {
            x: area.x,
            y: body.y.saturating_add(body.height.saturating_sub(1)),
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

fn paint_diff_row(
    row: &DiffRow,
    width: u16,
    gutter: usize,
    split: bool,
    col_offset: usize,
    wrap: bool,
    state: &AppState,
    selected: bool,
    focused: bool,
    visual: bool,
    search: Option<Pill>,
    left_syntax: &[CodeSpan],
    right_syntax: &[CodeSpan],
) -> Vec<Line<'static>> {
    let palette = state.theme.palette();
    let parts: Vec<Line<'static>> = match row {
        DiffRow::Section(section) => {
            let color = section_style(*section, palette);
            let text = format!(" {} ", section_header(*section));
            let chunks = if wrap {
                wrap_cols(&text, width as usize)
            } else {
                vec![text]
            };
            chunks
                .into_iter()
                .map(|chunk| Line::from(Span::styled(chunk, color.add_modifier(Modifier::BOLD))))
                .collect()
        }
        DiffRow::Hunk { text } => {
            let chunks = if wrap {
                wrap_cols(text, width as usize)
            } else {
                vec![slice_cols(text, 0, width as usize)]
            };
            chunks
                .into_iter()
                .map(|chunk| {
                    Line::from(Span::styled(chunk, Style::default().fg(palette.diff_hunk)))
                })
                .collect()
        }
        DiffRow::Error { text } => {
            let chunks = if wrap {
                wrap_cols(text, width as usize)
            } else {
                vec![slice_cols(text, 0, width as usize)]
            };
            chunks
                .into_iter()
                .map(|chunk| Line::from(Span::styled(chunk, Style::default().fg(palette.deleted))))
                .collect()
        }
        DiffRow::Line { left, right } if split && right.is_some() => {
            let cols = side_by_side_column_widths(width, state.diff_split_fraction);
            let n = if wrap {
                let mark = comment_mark_cols(state.ascii);
                let gutter_with_mark = gutter.saturating_add(mark);
                let left_h = wrap_col_starts(
                    &left.text,
                    cell_code_width(cols.left_width as usize, gutter_with_mark),
                )
                .len()
                .max(1);
                let right_h = wrap_col_starts(
                    &right.as_ref().unwrap().text,
                    cell_code_width(cols.right_width as usize, gutter_with_mark),
                )
                .len()
                .max(1);
                left_h.max(right_h)
            } else {
                1
            };
            (0..n)
                .map(|part| {
                    let mut spans = paint_cell_spans(
                        left,
                        cols.left_width,
                        gutter,
                        col_offset,
                        wrap,
                        part,
                        palette,
                        state,
                        state.ascii,
                        left_syntax,
                    );
                    spans.push(Span::styled(
                        DIFF_RULE.to_string(),
                        Style::default().fg(Color::DarkGray),
                    ));
                    spans.extend(paint_cell_spans(
                        right.as_ref().unwrap(),
                        cols.right_width,
                        gutter,
                        col_offset,
                        wrap,
                        part,
                        palette,
                        state,
                        state.ascii,
                        right_syntax,
                    ));
                    Line::from(spans)
                })
                .collect()
        }
        DiffRow::Line { left, .. } => {
            let n = if wrap {
                let mark = comment_mark_cols(state.ascii);
                wrap_col_starts(
                    &left.text,
                    cell_code_width(width as usize, gutter.saturating_add(mark)),
                )
                .len()
                .max(1)
            } else {
                1
            };
            (0..n)
                .map(|part| {
                    Line::from(paint_cell_spans(
                        left,
                        width,
                        gutter,
                        col_offset,
                        wrap,
                        part,
                        palette,
                        state,
                        state.ascii,
                        left_syntax,
                    ))
                })
                .collect()
        }
    };
    parts
        .into_iter()
        .map(|line| finish_diff_line(line, selected, focused, visual, search, palette))
        .collect()
}

fn finish_diff_line(
    mut line: Line<'static>,
    selected: bool,
    focused: bool,
    visual: bool,
    search: Option<Pill>,
    palette: Palette,
) -> Line<'static> {
    // A search match off the cursor paints the filter pill; its foreground
    // replaces syntax colours so the text stays readable on that background.
    let match_pill = search.filter(|_| !selected && !visual);
    let bg = if selected && focused {
        Some(palette.cursor_bg)
    } else if selected {
        Some(palette.cursor_bg_inactive)
    } else if visual {
        Some(palette.cursor_bg)
    } else {
        match_pill.map(|pill| pill.bg)
    };
    if let Some(bg) = bg {
        line.spans = line
            .spans
            .into_iter()
            .map(|span| {
                let mut style = span.style.bg(bg);
                if let Some(pill) = match_pill {
                    style = style.fg(pill.fg);
                }
                Span::styled(span.content.to_string(), style)
            })
            .collect();
    }
    let edge = selection_marker(selected, focused);
    let mut edge_style = Style::default().fg(if focused {
        palette.cursor
    } else {
        palette.muted
    });
    if focused {
        edge_style = edge_style.add_modifier(Modifier::BOLD);
    }
    if let Some(bg) = bg {
        edge_style = edge_style.bg(bg);
    }
    let mut spans = vec![Span::styled(edge, edge_style)];
    spans.extend(line.spans);
    Line::from(spans)
}

fn section_style(section: DiffSection, palette: Palette) -> Style {
    match section {
        DiffSection::Staged => Style::default().fg(palette.added),
        DiffSection::Unstaged => Style::default().fg(palette.modified),
        DiffSection::New | DiffSection::Committed => Style::default().fg(palette.heading),
    }
}

fn paint_cell_spans(
    cell: &DiffCell,
    width: u16,
    gutter: usize,
    col_offset: usize,
    wrap: bool,
    wrap_part: usize,
    palette: Palette,
    state: &AppState,
    ascii: bool,
    syntax: &[CodeSpan],
) -> Vec<Span<'static>> {
    let width = width as usize;
    let mark_w = comment_mark_cols(ascii);
    let code_w = cell_code_width(width, gutter.saturating_add(mark_w));
    let first = !wrap || wrap_part == 0;
    let comment = if first {
        cell.line_no.and_then(|n| state.diff_line_comment(n))
    } else {
        None
    };
    let line_no = format_line_gutter(
        if first { cell.line_no } else { None },
        gutter,
        comment,
        ascii,
    );
    let sign = if first { cell_sign(cell.kind) } else { ' ' };
    let accent = cell_accent(cell.kind, palette);
    let row_bg = cell_row_bg(cell.kind, palette);
    let word_bg = cell_word_bg(cell.kind, palette).or(row_bg);
    let gutter_style = with_row_bg(diff_gutter_style(palette), row_bg);
    let sign_style = with_row_bg(
        accent.unwrap_or_default().add_modifier(Modifier::BOLD),
        row_bg,
    );
    let code_off = if wrap {
        wrap_col_starts(&cell.text, code_w)
            .get(wrap_part)
            .copied()
            .unwrap_or(usize::MAX)
    } else {
        col_offset
    };
    let code_parts = if wrap && code_off == usize::MAX {
        Vec::new()
    } else {
        paint_code_parts(cell, syntax, code_off, code_w, palette)
    };
    let used = visible_width(&line_no)
        + 4
        + code_parts
            .iter()
            .map(|part| visible_width(&part.text))
            .sum::<usize>();
    let pad = width.saturating_sub(used);
    let mut spans = vec![
        Span::styled(line_no, gutter_style),
        Span::styled(format!(" {DIFF_RULE} "), gutter_style),
        Span::styled(sign.to_string(), sign_style),
    ];
    for part in code_parts {
        let bg = if part.word { word_bg } else { row_bg };
        spans.push(Span::styled(
            part.text,
            with_row_bg(Style::default().fg(part.fg), bg),
        ));
    }
    spans.push(Span::styled(
        " ".repeat(pad),
        with_row_bg(Style::default(), row_bg),
    ));
    spans
}

fn paint_code_parts(
    cell: &DiffCell,
    syntax: &[CodeSpan],
    col_offset: usize,
    code_w: usize,
    palette: Palette,
) -> Vec<CodeSpan> {
    match cell.kind {
        DiffCellKind::Empty => Vec::new(),
        DiffCellKind::Meta => vec![CodeSpan {
            text: slice_cols(&cell.text, col_offset, code_w),
            fg: palette.muted,
            word: false,
        }],
        _ => slice_styled_cols(syntax, col_offset, code_w),
    }
}

fn with_row_bg(style: Style, row_bg: Option<Color>) -> Style {
    match row_bg {
        Some(bg) => style.bg(bg),
        None => style,
    }
}

fn cell_row_bg(kind: DiffCellKind, palette: Palette) -> Option<Color> {
    match kind {
        DiffCellKind::Add => Some(palette.diff_add_bg),
        DiffCellKind::Del => Some(palette.diff_del_bg),
        DiffCellKind::Ctx | DiffCellKind::Meta | DiffCellKind::Empty => None,
    }
}

/// Changed-word background: only add and del cells have one.
fn cell_word_bg(kind: DiffCellKind, palette: Palette) -> Option<Color> {
    match kind {
        DiffCellKind::Add => Some(palette.diff_add_word_bg),
        DiffCellKind::Del => Some(palette.diff_del_word_bg),
        DiffCellKind::Ctx | DiffCellKind::Meta | DiffCellKind::Empty => None,
    }
}

fn diff_syntax_path(state: &AppState) -> &str {
    if let Some(tab) = state.tabs.active_compare() {
        return tab.path.as_deref().unwrap_or("");
    }
    match &state.drill {
        DrillView::Diff { path, .. } => path.as_str(),
        _ => state.diff_path.as_deref().unwrap_or(""),
    }
}

/// Identity for syntax-span reuse.
///
/// Hash the painted [`super::diff::DiffContent`] plus split layout. Do not
/// use the `DiffContent` address: [`AppState::set_diff`] and
/// [`AppState::apply_compare_diff`] replace text in place, so the pointer
/// and row count can stay the same while the source changes.
fn diff_syntax_cache_id(state: &AppState, split: bool) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    state
        .current_diff_content()
        .syntax_fingerprint()
        .hash(&mut hasher);
    split.hash(&mut hasher);
    hasher.finish()
}

thread_local! {
    static DIFF_SYNTAX_CACHE: RefCell<Option<CachedDiffSyntax>> = const { RefCell::new(None) };
}

/// Comment-mark column plus right-aligned line number.
///
/// The mark column is reserved on every numbered cell so a comment cannot
/// steal width from the numbers. `comment` is `None` when unmarked,
/// `Some(false)` for an open comment, `Some(true)` when resolved.
fn format_line_gutter(
    line_no: Option<u32>,
    gutter: usize,
    comment: Option<bool>,
    ascii: bool,
) -> String {
    let mark_w = comment_mark_cols(ascii);
    let mark = if let (Some(resolved), Some(_)) = (comment, line_no) {
        let glyph = if resolved {
            icon_comment_resolved(ascii)
        } else {
            icon_comment(ascii)
        };
        let pad = mark_w.saturating_sub(visible_width(glyph));
        format!("{glyph}{}", " ".repeat(pad))
    } else {
        " ".repeat(mark_w)
    };
    let nums = match line_no {
        Some(n) => format!("{n:>gutter$}"),
        None => " ".repeat(gutter),
    };
    format!("{mark}{nums}")
}

/// Line-number gutter and rule. Muted only. DIM on a dark terminal washes
/// the numbers out.
fn diff_gutter_style(palette: Palette) -> Style {
    Style::default().fg(palette.muted)
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
        "COMPARE" => (icon_branch(ascii), palette.repo),
        _ => (icon_diff(ascii), palette.modified),
    }
}

/// Cut or pad `spans` to exactly `width` painted columns.
///
/// Counts columns the way ratatui paints them (`Span::width`), so a help
/// cell with `←→` or `✗` ends where the next column starts.
fn clamp_spans(spans: Vec<Span<'static>>, width: usize) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut used = 0usize;
    for span in spans {
        if used >= width {
            break;
        }
        let w = span.width();
        if used + w <= width {
            used += w;
            out.push(span);
            continue;
        }
        let mut cut = String::new();
        let mut buf = [0u8; 4];
        for ch in span.content.chars() {
            let cw = UnicodeWidthStr::width(&*ch.encode_utf8(&mut buf));
            if used + cw > width {
                break;
            }
            used += cw;
            cut.push(ch);
        }
        out.push(Span::styled(cut, span.style));
        break;
    }
    if used < width {
        out.push(Span::raw(" ".repeat(width - used)));
    }
    out
}

/// Paint a help search hit with the search pill's fg/bg pair on every span
/// (chips, descriptions, padding), as pane match rows do. Modifiers stay.
fn with_search_pill(spans: Vec<Span<'static>>, pill: Option<Pill>) -> Vec<Span<'static>> {
    let Some(pill) = pill else {
        return spans;
    };
    spans
        .into_iter()
        .map(|span| span.patch_style(Style::default().fg(pill.fg).bg(pill.bg)))
        .collect()
}

fn help_chip_spans(
    keys: &str,
    key_width: usize,
    color: Color,
    surface: Color,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for chip in keys.split(' ').filter(|part| !part.is_empty()) {
        spans.push(key_chip(chip, color, surface));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(" ".repeat(help_chip_gap_spaces(keys, key_width))));
    spans
}

/// One painted line of a help entry, padded to `width` (the column text area).
fn help_visual_cell_spans(
    entry: &super::help::HelpEntry,
    vis: &super::help::HelpVisualLine,
    key_width: usize,
    color: Color,
    surface: Color,
    muted: Color,
    width: usize,
) -> Vec<Span<'static>> {
    if vis.chips {
        let mut spans = help_chip_spans(entry.keys, key_width, color, surface);
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
    // A compare tab swaps GIT for the COMPARE column.
    let groups = help_groups(state.is_compare_tab());
    let mut lines: Vec<Line> = Vec::new();

    let term_width = area.width as usize;
    let inner = help_inner_width(term_width).max(1);
    let widths = help_column_widths(groups, inner);

    let mut title_spans = Vec::new();
    for (group, &col_w) in groups.iter().zip(&widths) {
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

    // Each column stacks its own entries; a wrapped entry never pads the
    // other columns. The gutter stays outside the search highlight.
    let columns: Vec<Vec<Vec<Span<'static>>>> = groups
        .iter()
        .zip(&widths)
        .map(|(group, &col_w)| {
            let (_, color) = help_group_chrome(group.title, state.ascii, palette);
            let content = help_column_content_width(col_w);
            let key_width = help_key_width(group);
            let gutter = col_w.saturating_sub(content);
            let mut rows = Vec::new();
            for entry in group.entries {
                let hit = searching && help_entry_matches(entry.keys, entry.desc, query);
                let pill = hit.then_some(pills.filter);
                for vis in help_entry_visual_lines(entry.desc, content, key_width) {
                    let mut spans = with_search_pill(
                        help_visual_cell_spans(
                            entry,
                            &vis,
                            key_width,
                            color,
                            surface,
                            palette.muted,
                            content,
                        ),
                        pill,
                    );
                    spans.push(Span::raw(" ".repeat(gutter)));
                    rows.push(spans);
                }
            }
            rows
        })
        .collect();
    let body_rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    for row in 0..body_rows {
        let mut spans = Vec::new();
        for (column, &col_w) in columns.iter().zip(&widths) {
            match column.get(row) {
                Some(cell) => spans.extend(cell.iter().cloned()),
                None => spans.push(Span::raw(" ".repeat(col_w))),
            }
        }
        lines.push(Line::from(spans));
    }

    let footer = if searching {
        let q = state.help_search_query.as_deref().unwrap_or("");
        help_footer_with_version(
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
        )
    } else {
        help_idle_footer_lines(inner)
            .into_iter()
            .map(|part| Line::from(Span::styled(part, Style::default().fg(palette.muted))))
            .collect()
    };

    frame.render_widget(Clear, area);
    let block = overlay_block(palette.cursor);
    let inner_area = block.inner(area);
    frame.render_widget(block, area);
    if inner_area.width == 0 || inner_area.height == 0 {
        return;
    }
    let footer_h = (footer.len() as u16).min(inner_area.height).max(1);
    let body_h = inner_area.height.saturating_sub(footer_h);
    if body_h > 0 {
        frame.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }),
            Rect {
                x: inner_area.x,
                y: inner_area.y,
                width: inner_area.width,
                height: body_h,
            },
        );
    }
    frame.render_widget(
        Paragraph::new(footer).wrap(Wrap { trim: false }),
        Rect {
            x: inner_area.x,
            y: inner_area.y.saturating_add(body_h),
            width: inner_area.width,
            height: footer_h,
        },
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
    // Enter does not confirm, so the row lists every key that answers.
    spans.push(key_chip("n", muted, surface));
    spans.push(Span::raw(" "));
    spans.push(key_chip("Esc", muted, surface));
    spans.push(Span::styled(" cancel", Style::default().fg(muted)));
    Line::from(spans)
}

/// Detail row of a compare revert confirm that changes content only.
const COMPARE_REVERT_WORKTREE_ONLY: &str =
    "  worktree only · the change shows on the Workspace tab";

/// `Revert <what><path> to the <base_ref> merge base?`
fn compare_revert_title(
    what: &str,
    target: &CompareRevertTarget,
    accent: Color,
    muted: Color,
    file: Color,
) -> Line<'static> {
    let mut spans = vec![Span::styled(
        "Revert ",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )];
    if !what.is_empty() {
        spans.push(Span::styled(what.to_string(), Style::default().fg(muted)));
    }
    spans.extend([
        Span::styled(target.path.clone(), Style::default().fg(file)),
        Span::styled(" to the ", Style::default().fg(muted)),
        Span::styled(target.base_ref.clone(), Style::default().fg(file)),
        Span::styled(" merge base?", Style::default().fg(accent)),
    ]);
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
            let untracked = targets.iter().filter(|t| t.untracked).count();
            let tracked = targets.len() - untracked;
            let scope = revert_scope(targets);
            let deletes_only = matches!(
                scope,
                RevertScope::SingleUntracked | RevertScope::UntrackedOnly
            );
            let accent = if deletes_only {
                palette.deleted
            } else {
                palette.modified
            };
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    "Revert ",
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(label.clone(), Style::default().fg(palette.file)),
                Span::styled("?", Style::default().fg(accent)),
            ])];
            if tracked > 0 || untracked == 0 {
                lines.push(Line::from(Span::styled(
                    format!("  {tracked} tracked {} → discarded", files_word(tracked)),
                    Style::default().fg(palette.muted),
                )));
            }
            if untracked > 0 {
                let fate = if deletes_only { "deleted" } else { "kept" };
                lines.push(Line::from(Span::styled(
                    format!("  {untracked} untracked {} → {fate}", files_word(untracked)),
                    Style::default().fg(if deletes_only { accent } else { palette.muted }),
                )));
            }
            // Chips come from the same scope rule `confirm_yes` applies.
            let chip_label = |clean: bool| match (scope, clean) {
                (RevertScope::SingleUntracked, _) => "delete",
                (RevertScope::UntrackedOnly, _) => "delete untracked",
                (RevertScope::Mixed, true) => "revert + delete untracked",
                _ => "revert",
            };
            let chips: Vec<(&str, &str)> = [("y", false), ("Y", true)]
                .into_iter()
                .filter_map(|(key, clean)| {
                    scope
                        .key_deletes_untracked(clean)
                        .map(|_| (key, chip_label(clean)))
                })
                .collect();
            let (key, key_label) = chips[0];
            let extra = chips
                .get(1)
                .map(|&(key, key_label)| (key, palette.deleted, key_label));
            lines.push(confirm_action_row(
                key,
                key_label,
                extra,
                accent,
                palette.muted,
                surface,
            ));
            (accent, lines)
        }
        PendingConfirm::RevertRange { path, .. } => {
            let accent = palette.modified;
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        "Discard ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled("highlighted lines in ", Style::default().fg(palette.muted)),
                    Span::styled(path.clone(), Style::default().fg(palette.file)),
                    Span::styled("?", Style::default().fg(accent)),
                ]),
                confirm_action_row("y", "revert", None, accent, palette.muted, surface),
            ];
            (accent, lines)
        }
        PendingConfirm::CompareRevertRange { target, .. } => {
            let accent = palette.modified;
            let lines = vec![
                compare_revert_title(
                    "highlighted lines in ",
                    target,
                    accent,
                    palette.muted,
                    palette.file,
                ),
                Line::from(Span::styled(
                    COMPARE_REVERT_WORKTREE_ONLY,
                    Style::default().fg(palette.muted),
                )),
                confirm_action_row("y", "revert", None, accent, palette.muted, surface),
            ];
            (accent, lines)
        }
        PendingConfirm::CompareRevertFile { target } => {
            let deletes = target.status == "A";
            let accent = if deletes {
                palette.deleted
            } else {
                palette.modified
            };
            let path = &target.path;
            let fate = match (target.status.as_str(), target.old_path.as_deref()) {
                ("A", _) => format!("  added on HEAD → {path} will be deleted"),
                ("D", _) => format!("  deleted on HEAD → {path} will be restored"),
                ("R", Some(old)) => {
                    format!("  renamed on HEAD → {path} will be deleted, {old} restored")
                }
                _ => COMPARE_REVERT_WORKTREE_ONLY.to_string(),
            };
            let fate_color = if deletes { accent } else { palette.muted };
            let lines = vec![
                compare_revert_title("", target, accent, palette.muted, palette.file),
                Line::from(Span::styled(fate, Style::default().fg(fate_color))),
                confirm_action_row(
                    "y",
                    if deletes { "delete" } else { "revert" },
                    None,
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
            changed,
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
            // A detached worktree has no branch to keep or report.
            let detached = is_detached_head_branch(branch);
            let kept = if detached {
                String::new()
            } else {
                format!(" · branch {branch} is kept")
            };
            let dirty_line = if *force {
                Line::from(Span::styled(
                    format!(
                        "  {changed} changed {} will be deleted permanently{kept}",
                        files_word(*changed)
                    ),
                    Style::default().fg(accent),
                ))
            } else {
                Line::from(Span::styled(
                    format!("  clean worktree{kept}"),
                    Style::default().fg(palette.muted),
                ))
            };
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    "Remove worktree ",
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(path.clone(), Style::default().fg(palette.file)),
                Span::styled("?", Style::default().fg(accent)),
            ])];
            if !detached {
                lines.push(Line::from(Span::styled(
                    format!("  branch {branch} — {merge_text}"),
                    Style::default().fg(palette.muted),
                )));
            }
            lines.push(dirty_line);
            lines.push(confirm_action_row(
                "y",
                "remove",
                None,
                accent,
                palette.muted,
                surface,
            ));
            (accent, lines)
        }
        PendingConfirm::CheckoutOutOfSync {
            branch,
            remote_ref,
            ahead_behind,
            ..
        } => {
            let accent = palette.modified;
            // `y` fast-forwards to the remote-tracking ref already fetched; it never fetches.
            let mut lines = vec![Line::from(vec![
                Span::styled(
                    "Check out ",
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(branch.clone(), Style::default().fg(palette.file)),
                Span::styled(" and fast-forward to ", Style::default().fg(palette.muted)),
                Span::styled(remote_ref.clone(), Style::default().fg(palette.file)),
                Span::styled(" (no fetch)?", Style::default().fg(accent)),
            ])];
            if let Some((ahead, behind)) = ahead_behind {
                let mut detail = format!("  local is {ahead} ahead, {behind} behind {remote_ref}");
                if *ahead > 0 {
                    detail.push_str(" · cannot fast-forward");
                }
                lines.push(Line::from(Span::styled(
                    detail,
                    Style::default().fg(palette.muted),
                )));
            }
            lines.push(confirm_action_row(
                "y",
                "check out + fast-forward",
                None,
                accent,
                palette.muted,
                surface,
            ));
            (accent, lines)
        }
        PendingConfirm::SwitchToDefault { repos } => {
            let accent = palette.modified;
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        "Switch ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!("{} repos", repos.len()),
                        Style::default().fg(palette.file),
                    ),
                    Span::styled(" to their default branch?", Style::default().fg(accent)),
                ]),
                Line::from(Span::styled(
                    "  dirty repos are skipped",
                    Style::default().fg(palette.muted),
                )),
                confirm_action_row("y", "switch", None, accent, palette.muted, surface),
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
            state.status.to_string(),
            Style::default().fg(state.status.kind().color(palette)),
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

/// Compare-tab close control as painted and hit-tested: brackets around
/// U+2717 BALLOT X. Three display columns. Paint and hit boxes both derive
/// their width from this constant.
pub(super) const TAB_CLOSE_GLYPH: &str = "[\u{2717}]";

/// Columns ratatui paints for `text` (unicode-width, same as `Span::width`).
///
/// The tab strip hit boxes must match painted cells. `visible_width` counts
/// `↔` as two columns; ratatui and terminals paint it as one.
fn painted_width(text: &str) -> u16 {
    u16::try_from(Span::raw(text).width()).unwrap_or(u16::MAX)
}

/// Separator painted before every tab whose index is above 0.
const TAB_SEPARATOR: &str = "│";

/// Overflow marker for `hidden` tabs left of the painted window.
fn tab_left_marker(hidden: usize) -> String {
    format!("\u{2039}{hidden}")
}

/// Overflow marker for `hidden` tabs right of the painted window.
fn tab_right_marker(hidden: usize) -> String {
    format!("{hidden}\u{203a}")
}

/// Tabs `start..end` that the strip paints this frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TabWindow {
    start: usize,
    end: usize,
}

/// Pick the painted tab window.
///
/// `costs[i]` is tab `i`'s painted width with its separator. When every tab
/// fits, the window is all tabs. Else it keeps `stored` as the start while
/// `active` still fits, scrolls the minimum to bring `active` in, and then
/// pulls the start back left while that hides no tab painted now. Marker
/// columns are reserved before tabs are fitted. An `active` tab wider than
/// the space is still in the window; the caller clips it.
fn tab_window(costs: &[u16], width: u16, stored: usize, active: usize) -> TabWindow {
    let len = costs.len();
    let total: u32 = costs.iter().map(|cost| u32::from(*cost)).sum();
    if len == 0 || total <= u32::from(width) {
        return TabWindow { start: 0, end: len };
    }
    let active = active.min(len - 1);
    // One past the last tab that fits from `start`, markers reserved.
    let fit_end = |start: usize| {
        let mut used = if start > 0 {
            u32::from(painted_width(&tab_left_marker(start)))
        } else {
            0
        };
        let mut end = start;
        while end < len {
            let next = end + 1;
            let right = if next < len {
                u32::from(painted_width(&tab_right_marker(len - next)))
            } else {
                0
            };
            if used + u32::from(costs[end]) + right > u32::from(width) {
                break;
            }
            used += u32::from(costs[end]);
            end = next;
        }
        end
    };
    let mut start = stored.min(active);
    while start < active && fit_end(start) <= active {
        start += 1;
    }
    let mut end = fit_end(start);
    while start > 0 {
        let wider = fit_end(start - 1);
        if wider <= active || wider < end {
            break;
        }
        start -= 1;
        end = wider;
    }
    TabWindow {
        start,
        end: end.max(active + 1),
    }
}

fn draw_tab_strip(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    state.layout.tab_y = area.y;
    state.layout.tab_hits.clear();
    state.layout.tab_close_hits.clear();
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let labels = state.tabs.labels();
    let active = state.tabs.active;
    let texts: Vec<String> = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            if index > 0 {
                format!(" {label} {TAB_CLOSE_GLYPH} ")
            } else {
                format!(" {label} ")
            }
        })
        .collect();
    let sep_w = painted_width(TAB_SEPARATOR);
    let costs: Vec<u16> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let sep = if index > 0 { sep_w } else { 0 };
            painted_width(text).saturating_add(sep)
        })
        .collect();
    let window = tab_window(&costs, area.width, state.layout.tab_scroll, active);
    state.layout.tab_scroll = window.start;
    let muted = Style::default().fg(palette.muted);
    let mut spans = Vec::new();
    let mut x = area.x;
    // Tabs paint left of `limit`; the right marker, if any, owns the rest.
    let mut limit = area.x.saturating_add(area.width);
    if window.start > 0 {
        let marker = tab_left_marker(window.start);
        let width = painted_width(&marker).min(area.width);
        state.layout.tab_hits.push((x, width, window.start - 1));
        spans.push(Span::styled(marker, muted));
        x = x.saturating_add(width);
    }
    if window.end < labels.len() {
        let marker = tab_right_marker(labels.len() - window.end);
        let width = painted_width(&marker).min(limit.saturating_sub(x));
        limit = limit.saturating_sub(width);
        if width > 0 {
            state.layout.tab_hits.push((limit, width, window.end));
            frame.render_widget(
                Paragraph::new(Span::styled(marker, muted)),
                Rect {
                    x: limit,
                    width,
                    ..area
                },
            );
        }
    }
    for index in window.start..window.end {
        if index > 0 {
            spans.push(Span::styled(TAB_SEPARATOR, muted));
            x = x.saturating_add(sep_w);
        }
        let label = &labels[index];
        let text = &texts[index];
        let width = painted_width(text);
        // Only an active tab wider than the space clips; hit boxes stop at
        // the last painted cell.
        let visible = width.min(limit.saturating_sub(x));
        if visible > 0 {
            state.layout.tab_hits.push((x, visible, index));
        }
        let selected = index == active;
        let tab_style = if selected {
            Style::default()
                .fg(palette.cursor)
                .bg(palette.cursor_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(palette.muted)
        };
        if index > 0 {
            let prefix = format!(" {label} ");
            let close = TAB_CLOSE_GLYPH;
            let close_x = x.saturating_add(painted_width(&prefix));
            let close_w = painted_width(close);
            let close_painted = close_x.saturating_add(close_w) <= limit;
            if close_painted {
                state.layout.tab_close_hits.push((close_x, close_w, index));
            }
            // Hover comes from this paint's box, never from a stored index.
            // A clipped close has no box, so it never paints hovered.
            let hovered = close_painted
                && state.pointer.is_some_and(|(col, row)| {
                    row == area.y && col >= close_x && col < close_x.saturating_add(close_w)
                });
            let close_style = if hovered {
                tab_style
                    .fg(palette.tab_close_hover)
                    .add_modifier(Modifier::BOLD)
            } else {
                tab_style
                    .fg(palette.tab_close)
                    .remove_modifier(Modifier::BOLD)
            };
            spans.push(Span::styled(prefix, tab_style));
            spans.push(Span::styled(close, close_style));
            spans.push(Span::styled(" ", tab_style));
        } else {
            spans.push(Span::styled(text.clone(), tab_style));
        }
        x = x.saturating_add(width);
    }
    frame.render_widget(
        Paragraph::new(Line::from(spans)),
        Rect {
            width: limit.saturating_sub(area.x),
            ..area
        },
    );
}

/// `value` cut to `width` columns, ending in `…` when it was cut.
fn fit_with_ellipsis(value: &str, width: usize) -> String {
    if visible_width(value) <= width {
        value.to_string()
    } else {
        format!("{}…", truncate_visible(value, width.saturating_sub(1)))
    }
}

fn draw_compare_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(picker) = state.compare_picker.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let accent = palette.branch_feature;
    let visible_len = picker.visible_len();
    let max_rows = 12usize;
    let start = if visible_len <= max_rows {
        0
    } else {
        picker
            .cursor()
            .saturating_sub(max_rows / 2)
            .min(visible_len - max_rows)
    };
    let window = picker.window_labels(start, max_rows);
    // One line per row: the overlay height counts rows, so a label wider
    // than the box (borders, padding, `❯ ` and its two-space indent) is cut
    // with an ellipsis instead of wrapping.
    let label_cols = usize::from(area.width).saturating_sub(4 + 4);
    let filter = if picker.filter().is_empty() {
        "…"
    } else {
        picker.filter()
    };
    let heading = match picker {
        ComparePickerState::Branch(_) => "Compare ",
        ComparePickerState::Commit(_) => "Compare vs commit ",
    };
    let title = vec![
        Span::styled(
            heading,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(picker.repo().to_string(), Style::default().fg(palette.repo)),
        Span::styled("  filter: ", Style::default().fg(palette.muted)),
        Span::styled(filter.to_string(), Style::default().fg(palette.cursor)),
    ];
    let mut lines = vec![Line::from(title)];
    if window.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {}", compare_picker_empty(picker)),
            Style::default().fg(palette.muted),
        )));
    } else {
        for (i, label) in window.iter().enumerate() {
            let index = start + i;
            let selected = index == picker.cursor();
            let cursor = if selected { "❯ " } else { "  " };
            let row_bg = if selected {
                palette.cursor_bg
            } else {
                Color::Reset
            };
            let name_fg = if selected {
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
                    format!("  {}", fit_with_ellipsis(label, label_cols)),
                    Style::default().fg(name_fg).bg(row_bg),
                ),
            ]));
        }
    }
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.to_string(),
            Style::default().fg(state.status.kind().color(palette)),
        )));
    }
    lines.push(Line::from(Span::styled(
        "↑↓ move · type to filter · Enter compare · Esc close",
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
    let create = picker.create_name();
    let rows = picker.row_count();
    let max_rows = 12usize;
    let start = if rows <= max_rows {
        0
    } else {
        picker
            .cursor
            .saturating_sub(max_rows / 2)
            .min(rows - max_rows)
    };
    let window = visible
        .iter()
        .skip(start)
        .take(max_rows)
        .copied()
        .collect::<Vec<_>>();
    // The create row sits after the last branch, inside the same window.
    let create_painted = create.is_some() && visible.len() < start + max_rows;
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
            .map(|id| short_id(id).to_string())
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
    if window.is_empty() && !create_painted {
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
        if let Some(name) = create.filter(|_| create_painted) {
            lines.push(branch_create_row(
                name,
                picker.commit_id.as_deref(),
                picker.on_create_row(),
                palette,
                accent,
            ));
        }
    }
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.to_string(),
            Style::default().fg(state.status.kind().color(palette)),
        )));
    }
    // Enter names what it does on the cursor row: the create row makes a
    // branch (tree: and checks it out; graph: at the commit, no checkout).
    let enter = match (picker.on_create_row(), picker.commit_id.as_deref()) {
        (true, Some(id)) => format!("Enter create at {}", short_id(id)),
        (true, None) => "Enter create and check out".to_string(),
        (false, _) => "Enter checkout".to_string(),
    };
    let footer = if graph && !show_filter {
        format!("↑↓ move · type a name to create · {enter} · Esc cancel")
    } else if graph {
        format!("↑↓ move · type to filter · {enter} · Esc cancel")
    } else {
        format!("↑↓ move · type to filter · {enter} · Esc close")
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

/// `+ create branch <name>` (graph picker: `… at <short>`) after the
/// branch rows. Accent on the cursor, muted otherwise.
fn branch_create_row(
    name: &str,
    commit_id: Option<&str>,
    selected: bool,
    palette: Palette,
    accent: Color,
) -> Line<'static> {
    let row_bg = if selected {
        palette.cursor_bg
    } else {
        Color::Reset
    };
    let label_fg = if selected { accent } else { palette.muted };
    let mut spans = vec![
        Span::styled(
            if selected { "❯ " } else { "  " }.to_string(),
            Style::default()
                .fg(if selected {
                    palette.cursor
                } else {
                    palette.muted
                })
                .bg(row_bg),
        ),
        Span::styled(
            "  + create branch ".to_string(),
            Style::default().fg(label_fg).bg(row_bg),
        ),
        Span::styled(
            name.to_string(),
            Style::default()
                .fg(label_fg)
                .bg(row_bg)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    if let Some(id) = commit_id {
        spans.push(Span::styled(
            format!(" at {}", short_id(id)),
            Style::default().fg(label_fg).bg(row_bg),
        ));
    }
    Line::from(spans)
}

fn draw_graph_focus_picker(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(picker) = state.graph_focus_picker.as_ref() else {
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
    let mut lines = vec![Line::from(vec![
        Span::styled(
            "Focus branches ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(picker.repo.clone(), Style::default().fg(palette.repo)),
        Span::styled("  filter: ", Style::default().fg(palette.muted)),
        Span::styled(filter.to_string(), Style::default().fg(palette.cursor)),
    ])];
    if window.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No matching branches",
            Style::default().fg(palette.muted),
        )));
    } else {
        for (i, branch) in window.iter().enumerate() {
            let index = start + i;
            let selected = index == picker.cursor;
            let marked = picker.marked.contains(&branch.name);
            let mark = if marked { "[x] " } else { "[ ] " };
            let current = if branch.current { "* " } else { "  " };
            let cursor = if selected { "❯ " } else { "  " };
            let row_bg = if selected {
                palette.cursor_bg
            } else {
                Color::Reset
            };
            let name_fg = if marked || branch.current {
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
                    format!("{mark}{current}{}", branch.name),
                    Style::default().fg(name_fg).bg(row_bg),
                ),
            ]));
        }
    }
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.to_string(),
            Style::default().fg(state.status.kind().color(palette)),
        )));
    }
    lines.push(Line::from(Span::styled(
        "↑↓ move · type to filter · space toggle · Enter apply · Ctrl-o clear · Esc cancel",
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

fn draw_command_palette(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(palette) = state.command_palette.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette_theme = state.theme.palette();
    let accent = palette_theme.cursor;
    let surface = overlay_surface(state);
    let rows = palette.paint_rows();
    // Rounded border plus one column of padding on each side.
    let inner_width = area.width.saturating_sub(4) as usize;
    let max_rows = 12usize;
    let cursor_paint = rows.iter().position(|row| match row {
        super::command_palette::PalettePaintRow::Command { index, .. } => *index == palette.cursor,
        _ => false,
    });
    let start = if rows.len() <= max_rows {
        0
    } else {
        let focus = cursor_paint.unwrap_or(0);
        focus
            .saturating_sub(max_rows / 2)
            .min(rows.len() - max_rows)
    };
    let window = if rows.is_empty() {
        Vec::new()
    } else {
        rows.iter().skip(start).take(max_rows).cloned().collect()
    };
    let prefix = match palette.opened_by {
        super::action::PaletteOpenedBy::Colon => ":",
        super::action::PaletteOpenedBy::CtrlK => "Ctrl-k",
    };
    let query = if palette.filter.is_empty() {
        "…"
    } else {
        palette.filter.as_str()
    };
    let mut lines = vec![Line::from(vec![
        Span::styled(
            prefix.to_string(),
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(" ", Style::default()),
        Span::styled(query.to_string(), Style::default().fg(accent)),
    ])];
    if window.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No matching commands",
            Style::default().fg(palette_theme.muted),
        )));
    } else {
        // Rows under a painted group header do not repeat the group. A
        // window scrolled into the middle of a group lost its header, so
        // those rows keep the label.
        let mut header_painted = false;
        for row in window {
            match row {
                super::command_palette::PalettePaintRow::Header(title) => {
                    header_painted = true;
                    lines.push(Line::from(Span::styled(
                        title.to_string(),
                        Style::default()
                            .fg(palette_theme.heading)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                super::command_palette::PalettePaintRow::Command { command, index } => {
                    let selected = index == palette.cursor;
                    let reason = state.palette_disabled_reason(command);
                    let disabled = reason.is_some();
                    let cursor = if selected { "❯ " } else { "  " };
                    let row_bg = if selected {
                        palette_theme.cursor_bg
                    } else {
                        Color::Reset
                    };
                    let mut style = Style::default()
                        .fg(if selected {
                            palette_theme.file
                        } else {
                            palette_theme.muted
                        })
                        .bg(row_bg);
                    if disabled {
                        style = style.add_modifier(Modifier::DIM);
                    }
                    let dim = if disabled {
                        Modifier::DIM
                    } else {
                        Modifier::empty()
                    };
                    let mut spans = vec![
                        Span::styled(
                            cursor.to_string(),
                            Style::default()
                                .fg(if selected {
                                    accent
                                } else {
                                    palette_theme.muted
                                })
                                .bg(row_bg),
                        ),
                        Span::styled(command.title.to_string(), style),
                    ];
                    // Palette-only rows (Diff … in new tab, Close tab) have no key.
                    if !command.keys.is_empty() {
                        spans.push(Span::raw(" "));
                        spans.push(key_chip(
                            command.keys,
                            if disabled {
                                palette_theme.muted
                            } else {
                                accent
                            },
                            surface,
                        ));
                    }
                    if !header_painted {
                        spans.push(Span::styled(
                            format!("  {}", command.group.title()),
                            Style::default()
                                .fg(palette_theme.muted)
                                .bg(row_bg)
                                .add_modifier(dim),
                        ));
                    }
                    // The reason sits dimmed at the right edge when it fits,
                    // so every disabled row says why, not only the cursor row.
                    if let Some(why) = reason.as_deref() {
                        let used = help_spans_width(&spans);
                        let room = inner_width.saturating_sub(used);
                        let need = visible_width(why) + 2;
                        if room >= need {
                            spans.push(Span::styled(
                                format!("{}{why}", " ".repeat(room - need + 2)),
                                Style::default()
                                    .fg(palette_theme.muted)
                                    .bg(row_bg)
                                    .add_modifier(Modifier::DIM),
                            ));
                        }
                    }
                    lines.push(Line::from(spans));
                }
            }
        }
    }
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.to_string(),
            Style::default().fg(state.status.kind().color(palette_theme)),
        )));
    }
    let reason = palette
        .selected()
        .and_then(|command| state.palette_disabled_reason(command));
    let footer = match reason {
        Some(why) => format!("Enter run · Esc close · {why}"),
        None => "Enter run · Esc close".into(),
    };
    lines.push(Line::from(Span::styled(
        footer,
        Style::default().fg(palette_theme.muted),
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
    let short = short_id(&create.commit_id).to_string();
    let name = if create.name.is_empty() {
        "…"
    } else {
        create.name.as_str()
    };
    let mut title = vec![Span::styled(
        "Create branch ",
        Style::default().fg(accent).add_modifier(Modifier::BOLD),
    )];
    // Graph `c` only creates the ref; it never checks the branch out.
    let footer = format!("Enter create at {short} (no checkout) · Esc cancel");
    title.push(Span::styled("at ", Style::default().fg(palette.muted)));
    title.push(Span::styled(short, Style::default().fg(palette.repo)));
    let mut lines = vec![
        Line::from(title),
        Line::from(vec![
            Span::styled("  name: ", Style::default().fg(palette.muted)),
            Span::styled(name.to_string(), Style::default().fg(palette.cursor)),
        ]),
    ];
    if !state.status.is_empty() {
        lines.push(Line::from(Span::styled(
            state.status.to_string(),
            Style::default().fg(state.status.kind().color(palette)),
        )));
    }
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

fn overlay_block_filled(accent: Color, surface: Color) -> Block<'static> {
    overlay_block(accent).style(Style::default().bg(surface))
}

fn comment_body_lines(prompt: &CommentPrompt, palette: Palette) -> Vec<Line<'static>> {
    let (start, _) = prompt.visible_line_range();
    prompt
        .painted_lines()
        .into_iter()
        .enumerate()
        .map(|(i, line)| {
            let prefix = if start + i == 0 {
                "  body: "
            } else {
                "        "
            };
            match line.caret {
                Some(col) => {
                    let before: String = line.text.chars().take(col).collect();
                    let after: String = line.text.chars().skip(col).collect();
                    Line::from(vec![
                        Span::styled(prefix, Style::default().fg(palette.muted)),
                        Span::styled(before, Style::default().fg(palette.cursor)),
                        Span::styled("▏", Style::default().fg(palette.cursor)),
                        Span::styled(after, Style::default().fg(palette.cursor)),
                    ])
                }
                None => Line::from(vec![
                    Span::styled(prefix, Style::default().fg(palette.muted)),
                    Span::styled(line.text, Style::default().fg(palette.cursor)),
                ]),
            }
        })
        .collect()
}

fn draw_comment(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(prompt) = state.comment.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let surface = overlay_surface(state);
    let accent = palette.heading;
    let title = if prompt.resolved {
        "Comment · resolved"
    } else {
        "Comment"
    };
    let mut lines = vec![
        Line::from(Span::styled(
            title,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            prompt.label.clone(),
            Style::default().fg(palette.muted),
        )),
    ];
    lines.extend(comment_body_lines(prompt, palette));
    lines.push(Line::from(Span::styled(
        comment_overlay_footer_save(prompt.resolved),
        Style::default().fg(palette.muted),
    )));
    lines.push(Line::from(Span::styled(
        COMMENT_OVERLAY_FOOTER_EDIT,
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block_filled(accent, surface))
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_comment_export(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(export) = state.comment_export.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let accent = palette.heading;
    let mut lines = vec![
        Line::from(Span::styled(
            "Comments",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        match export.copied {
            Some(true) => Line::from(Span::styled(
                "copied to clipboard",
                Style::default().fg(palette.added),
            )),
            Some(false) => Line::from(Span::styled(
                "copy failed (no TTY or clipboard tool)",
                Style::default().fg(palette.deleted),
            )),
            None => Line::from(Span::styled("copying…", Style::default().fg(palette.muted))),
        },
    ];
    for row in export.markdown.lines() {
        lines.push(Line::from(Span::styled(
            row.to_string(),
            Style::default().fg(palette.repo),
        )));
    }
    if export_shows_status(&state.status) {
        lines.push(Line::from(Span::styled(
            state.status.to_string(),
            Style::default().fg(state.status.kind().color(palette)),
        )));
    }
    lines.push(Line::from(Span::styled(
        "Esc close",
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
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::action::{Action, Effect};
    use crate::tui::comments::{put_comment, CommentKey};
    use crate::tui::icons::{icon_linked_worktree, icon_repo};
    use crate::tui::split::SplitDrag;
    use crate::tui::state::AppState;
    use crate::tui::tree::{build_tree, flatten_with, visible_for_tree};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::collections::HashSet;
    use std::path::PathBuf;
    use workspace_status_graph::{graph_gutter_cap, Commit, GraphModel};

    fn repo(name: &str, dirty: bool) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
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
            default_tip_ref: None,
            local_branches: Vec::new(),
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

    fn assert_pane_title_is_plain_name(name: &str) {
        let title = pane_title(name);
        assert_eq!(title, name);
        assert!(!title.contains('●'), "{title:?}");
        assert!(!title.starts_with('*'), "{title:?}");
        assert!(!title.contains(' '), "{title:?}");
        assert!(!title.starts_with(' '), "{title:?}");
        assert!(!title.ends_with(' '), "{title:?}");
    }

    #[test]
    fn pane_title_focused_is_plain_name() {
        for name in ["tree", "graph", "files", "diff"] {
            assert_pane_title_is_plain_name(name);
        }
    }

    #[test]
    fn pane_title_unfocused_is_plain_name() {
        for name in ["tree", "graph", "files", "diff"] {
            assert_pane_title_is_plain_name(name);
        }
    }

    #[test]
    fn pane_border_focused_is_heading_unfocused_is_border_dim() {
        for id in crate::tui::theme::THEME_IDS {
            let palette = id.palette();
            let focused = pane_border(true, palette);
            let unfocused = pane_border(false, palette);
            assert_eq!(focused.fg, Some(palette.heading), "{id:?} focused");
            assert_eq!(unfocused.fg, Some(palette.border_dim), "{id:?} unfocused");
            assert_ne!(
                unfocused.fg,
                Some(palette.muted),
                "{id:?} unfocused border is not muted"
            );
            assert!(
                !unfocused.add_modifier.contains(Modifier::DIM),
                "{id:?} unfocused border uses border_dim, not DIM"
            );
        }
    }

    fn two_pane_diff_state() -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
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
        state
    }

    fn long_panning_diff_state(offset: u16) -> AppState {
        let mut state = two_pane_diff_state();
        let mut body = format!("@@ -0,0 +1,41 @@\n+{}UNIQUE_DIFF_TAIL\n", "n".repeat(80));
        for i in 0..40 {
            body.push_str(&format!("+line {i}\n"));
        }
        state.set_diff(
            "app".into(),
            "unique-diffline.rs".into(),
            super::super::diff::DiffContent::from_unified(body),
        );
        state.diff_wrap = false;
        state.diff_col_offset = offset;
        state
    }

    /// A diff that overflows shows its vertical bar at the top, before any
    /// scroll; the horizontal bar waits until the view leaves the left
    /// edge. A diff that fits shows neither.
    #[test]
    fn diff_vertical_bar_shows_on_overflow_horizontal_after_pan() {
        let mut state = long_panning_diff_state(0);
        state.diff_cursor = 0;
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        assert_eq!((state.diff_scroll, state.diff_col_offset), (0, 0));
        assert!(
            state.layout.diff_scrollbar_x.is_some(),
            "vertical bar at the top"
        );
        assert!(
            state.layout.diff_hscrollbar_y.is_none(),
            "no horizontal bar at pan 0"
        );
        state.diff_col_offset = 1;
        draw_state(&mut terminal, &mut state);
        assert!(
            state.layout.diff_hscrollbar_y.is_some(),
            "horizontal bar once panned"
        );

        // A frame builds the diff rows once; the h-bar pan reuses them.
        for offset in [1, 0] {
            state.diff_col_offset = offset;
            state.diff_row_builds.set(0);
            draw_state(&mut terminal, &mut state);
            assert_eq!(state.diff_row_builds.get(), 1, "pan {offset}");
        }

        let mut state = two_pane_diff_state();
        state.set_diff(
            "app".into(),
            "README.md".into(),
            super::super::diff::DiffContent::from_lines(vec![
                "@@ -1,1 +1,1 @@".into(),
                "-old".into(),
                "+new".into(),
            ]),
        );
        draw_state(&mut terminal, &mut state);
        assert!(
            state.layout.diff_scrollbar_x.is_none(),
            "short diff: no bar"
        );
        assert!(
            state.layout.diff_hscrollbar_y.is_none(),
            "narrow diff: no bar"
        );
    }

    /// Text of one right-pane row, `width` cells from `x`.
    fn row_cells(terminal: &Terminal<TestBackend>, x: u16, y: u16, width: u16) -> String {
        let buf = terminal.backend().buffer();
        (x..x + width).map(|col| buf[(col, y)].symbol()).collect()
    }

    #[test]
    fn long_diff_path_header_wraps_and_the_body_starts_below_it() {
        let mut state = two_pane_diff_state();
        let path = format!("{}/leaf-file-TAIL.rs", "nested-folder".repeat(6));
        state.set_diff(
            "app".into(),
            path.clone(),
            super::super::diff::DiffContent::from_lines(vec![
                "@@ -1,1 +1,1 @@".into(),
                "-old line".into(),
                "+new line".into(),
            ]),
        );
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (x, y, width) = (
            state.layout.diff_content_x,
            state.layout.right_y,
            state.layout.diff_pane_width,
        );
        let title = format!("app/{path}");
        let header_h = diff_pane_header_rows(&title, width, state.layout.diff_pane_height);
        assert!(header_h > 1, "fixture path must wrap at width {width}");
        let header: String = (y..y + header_h)
            .map(|row| row_cells(&terminal, x, row, width))
            .collect();
        assert!(
            header.starts_with(&title),
            "header rows must hold the full path `{title}`:\n{}",
            buffer_text(&terminal)
        );
        let buf = terminal.backend().buffer();
        for row in y..y + header_h {
            assert!(
                buf[(x, row)].modifier.contains(Modifier::BOLD),
                "header row {row} is a bold heading"
            );
        }
        let first = super::super::diff::row_search_text(
            &state.current_diff_rows()[state.diff_scroll as usize],
        );
        let body = row_cells(&terminal, x, y + header_h, width);
        assert!(
            body.contains(&first),
            "first diff row `{first}` paints right below the header, got `{body}`:\n{}",
            buffer_text(&terminal)
        );
    }

    /// Diff search marks every matching row with the filter pill, not only
    /// the current hit, and the cursor bar stays on the current hit.
    #[test]
    fn diff_search_marks_every_matching_row() {
        let mut state = two_pane_diff_state();
        state.set_diff(
            "app".into(),
            "README.md".into(),
            super::super::diff::DiffContent::from_lines(vec![
                "@@ -1,3 +1,3 @@".into(),
                "-old needle".into(),
                "+new needle".into(),
                " plain context".into(),
            ]),
        );
        state.focus = FocusPane::Right;
        state.dispatch(super::super::action::Action::SearchStart);
        for c in "needle".chars() {
            state.dispatch(super::super::action::Action::SearchChar(c));
        }
        assert_eq!(state.search_target, SearchPane::Diff);
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let pill = state.theme.pills().filter;
        let cursor_bg = state.theme.palette().cursor_bg;
        let buf = terminal.backend().buffer();
        let mut seen = Vec::new();
        for y in 0..buf.area().height {
            let line: String = (0..buf.area().width)
                .map(|x| buf[(x, y)].symbol())
                .collect();
            for needle in ["old needle", "new needle", "plain context"] {
                if let Some(byte) = line.find(needle) {
                    let col = line[..byte].chars().count() as u16;
                    let cell = &buf[(col, y)];
                    seen.push((needle, cell.bg, cell.fg));
                }
            }
        }
        let bgs: Vec<_> = seen
            .iter()
            .filter(|(n, _, _)| n.contains("needle"))
            .map(|(_, bg, _)| *bg)
            .collect();
        assert!(
            bgs.contains(&cursor_bg),
            "current hit keeps the cursor: {seen:?}"
        );
        assert!(
            seen.iter()
                .any(|(n, bg, fg)| n.contains("needle") && *bg == pill.bg && *fg == pill.fg),
            "the other hit paints the filter pill: {seen:?}"
        );
        assert!(
            seen.iter()
                .any(|(n, bg, _)| *n == "plain context" && *bg != pill.bg),
            "a row without the query stays unmarked: {seen:?}"
        );
    }

    #[test]
    fn painted_file_diff_hscrollbar_thumb_cells_are_hit_as_thumb() {
        let mut state = long_panning_diff_state(1);
        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let y = state
            .layout
            .diff_hscrollbar_y
            .expect("h-bar row after a non-zero pan");
        let x0 = state.layout.diff_hscrollbar_x;
        let width = state.layout.diff_hscrollbar_width;
        let thumbs: Vec<u16> = {
            let buf = terminal.backend().buffer();
            (x0..x0.saturating_add(width))
                .filter(|&x| buf[(x, y)].symbol() == "█")
                .collect()
        };
        assert!(
            !thumbs.is_empty(),
            "expected a painted █ on the h-bar row {y}:\n{}",
            buffer_text(&terminal)
        );
        let start = state.diff_col_offset;
        for x in thumbs {
            assert_eq!(
                state.dispatch(Action::Click { col: x, row: y }),
                Effect::None
            );
            assert_eq!(
                state.diff_col_offset,
                start,
                "painted █ at ({x},{y}) must grab, not jump to origin:\n{}",
                buffer_text(&terminal)
            );
            assert!(
                matches!(
                    state.drag,
                    SplitDrag::DiffHScrollbar {
                        origin_col,
                        origin_offset
                    } if origin_col == x && origin_offset == start
                ),
                "drag at ({x},{y}): {:?}",
                state.drag
            );
            assert_eq!(state.dispatch(Action::Release), Effect::None);
        }
    }

    fn json_named_lines(name: &str) -> Vec<String> {
        vec![
            "@@ -1,3 +1,4 @@".into(),
            " {".into(),
            r#"-  "ttlMs": 5000"#.into(),
            r#"+  "ttlMs": 2000"#.into(),
            format!(r#"+  "name": "{name}""#),
            " }".into(),
        ]
    }

    /// A 100-column right pane with the vertical bar showing paints 99
    /// columns. Split needs 100, so rows are built and painted inline and
    /// the added line still paints (it used to be built split and dropped).
    #[test]
    fn boundary_width_with_scrollbar_paints_added_lines() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let file = state
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file row");
        state.cursor = file;
        let mut unified = String::from("@@ -1,60 +1,60 @@\n");
        for i in 0..60 {
            if i == 35 {
                unified.push_str("-old-line-gone\n+new-line-added\n");
            } else {
                unified.push_str(&format!(" ctx{i}\n"));
            }
        }
        state.set_diff(
            "app".into(),
            "README.md".into(),
            super::super::diff::DiffContent::from_unified(unified),
        );
        state.focus = FocusPane::Right;
        state.diff_cursor = 40;
        let mut terminal = Terminal::new(TestBackend::new(170, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(state.layout.diff_pane_width, 100);
        assert!(state.diff_scroll > 0, "the vertical bar shows");
        assert_eq!(state.diff_layout(), DiffMode::Inline);
        let text = buffer_text(&terminal);
        assert!(text.contains("inline (too narrow)"), "{text}");
        assert!(text.contains("new-line-added"), "{text}");
        assert!(text.contains("old-line-gone"), "{text}");
        let status = text.lines().last().unwrap_or_default();
        assert!(status.contains("split→inline"), "{status}");
    }

    fn json_syntax_diff_state() -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let file = state
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file row");
        state.cursor = file;
        state.focus = FocusPane::Left;
        state.set_diff(
            "app".into(),
            "pack.json".into(),
            super::super::diff::DiffContent::from_lines(json_named_lines("alpha-syntax")),
        );
        state
    }

    fn first_row_with(buf: &ratatui::buffer::Buffer, needle: &str) -> Option<u16> {
        (0..buf.area().height).find(|&y| buf_line(buf, y).contains(needle))
    }

    fn needle_cells<'a>(
        buf: &'a ratatui::buffer::Buffer,
        y: u16,
        needle: &str,
    ) -> Vec<&'a ratatui::buffer::Cell> {
        let start = find_cell_col(buf, y, needle).expect(needle);
        (0..needle.chars().count() as u16)
            .map(|i| &buf[(start + i, y)])
            .collect()
    }

    #[test]
    fn json_diff_keeps_signs_row_backgrounds_and_syntax_fg() {
        let mut state = json_syntax_diff_state();
        let palette = state.theme.palette();
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        assert!(text.contains("pack.json"), "{text}");
        assert!(text.contains("alpha-syntax"), "{text}");
        assert!(text.contains("ttlMs"), "{text}");

        let add_y = first_row_with(buf, "alpha-syntax").expect("add JSON line");
        let del_y = first_row_with(buf, r#""ttlMs": 5000"#).expect("del JSON line");
        let add_cells = needle_cells(buf, add_y, "alpha-syntax");
        let add_span = needle_cells(buf, add_y, r#""name": "alpha-syntax""#);
        assert!(
            add_cells.iter().all(|cell| cell.bg == palette.diff_add_bg),
            "add-line syntax must keep diff_add_bg, not a syntect background:\n{text}"
        );
        let del_cells = needle_cells(buf, del_y, "ttlMs");
        assert!(
            del_cells.iter().all(|cell| cell.bg == palette.diff_del_bg),
            "del-line syntax must keep diff_del_bg:\n{text}"
        );
        // `5000` -> `2000` is the changed word of the paired del/add lines.
        assert!(
            needle_cells(buf, del_y, "5000")
                .iter()
                .all(|cell| cell.bg == palette.diff_del_word_bg),
            "changed del word sits on diff_del_word_bg:\n{text}"
        );

        let add_line = buf_line(buf, add_y);
        let plus_at = find_cell_col(buf, add_y, "+").expect("add sign");
        let plus = &buf[(plus_at, add_y)];
        assert_eq!(
            plus.fg, palette.added,
            "add sign stays added fg: {add_line}"
        );
        assert_eq!(
            plus.bg, palette.diff_add_bg,
            "add sign sits on the add row background"
        );

        let del_line = buf_line(buf, del_y);
        let minus_at = find_cell_col(buf, del_y, "-").expect("del sign");
        let minus = &buf[(minus_at, del_y)];
        assert_eq!(
            minus.fg, palette.deleted,
            "del sign stays deleted fg: {del_line}"
        );
        assert_eq!(
            minus.bg, palette.diff_del_bg,
            "del sign sits on the del row background"
        );

        let add_fgs: std::collections::HashSet<_> = add_span.iter().map(|cell| cell.fg).collect();
        assert!(
            add_fgs.len() >= 2,
            "JSON tokens on an add line should use more than one fg: {add_fgs:?}\n{text}"
        );
        assert!(
            add_cells.iter().any(|cell| cell.fg != palette.added),
            "syntax fg must not wash the add line with the added accent: {add_line}"
        );
    }

    /// A paired modified line, a pure add, and a pure del.
    fn word_diff_lines(old: &str, new: &str) -> Vec<String> {
        vec![
            "@@ -1,5 +1,5 @@".into(),
            " fn main() {".into(),
            format!("-{old}"),
            format!("+{new}"),
            "+let fresh_only = 1;".into(),
            " let keep = 0;".into(),
            "-let stale_only = 2;".into(),
            " }".into(),
        ]
    }

    fn word_diff_state(lines: Vec<String>, mode: DiffMode) -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let file = state
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file row");
        state.cursor = file;
        state.focus = FocusPane::Left;
        state.diff_mode = mode;
        state.set_diff(
            "app".into(),
            "calc.rs".into(),
            super::super::diff::DiffContent::from_lines(lines),
        );
        state
    }

    /// Columns of row `y` painted on `bg`.
    fn cols_with_bg(buf: &ratatui::buffer::Buffer, y: u16, bg: Color) -> Vec<u16> {
        (0..buf.area().width)
            .filter(|&x| buf[(x, y)].bg == bg)
            .collect()
    }

    /// Columns of the first `needle` on row `y` (one cell per char).
    fn needle_cols(buf: &ratatui::buffer::Buffer, y: u16, needle: &str) -> Vec<u16> {
        let start = find_cell_col(buf, y, needle).expect(needle);
        (start..start + needle.chars().count() as u16).collect()
    }

    /// Rows that hold at least one cell on `bg`.
    fn rows_with_bg(buf: &ratatui::buffer::Buffer, bg: Color) -> Vec<u16> {
        (0..buf.area().height)
            .filter(|&y| !cols_with_bg(buf, y, bg).is_empty())
            .collect()
    }

    /// Diff row index whose left or right cell text contains `needle`.
    fn diff_row_index(state: &AppState, needle: &str) -> usize {
        state
            .current_diff_rows()
            .iter()
            .position(|row| match row {
                DiffRow::Line { left, right } => {
                    left.text.contains(needle)
                        || right.as_ref().is_some_and(|r| r.text.contains(needle))
                }
                _ => false,
            })
            .expect(needle)
    }

    const QTY_LINE: &str = "let total = price * qty;";
    const COUNT_LINE: &str = "let total = price * count;";

    #[test]
    fn inline_word_diff_paints_word_bg_on_changed_text_only() {
        let mut state = word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        assert_eq!(state.diff_layout(), DiffMode::Inline);
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let add_y = first_row_with(buf, "price * count").expect("add line");
        let del_y = first_row_with(buf, "price * qty").expect("del line");
        assert_ne!(add_y, del_y);

        for (y, word, row_bg, word_bg, sign, sign_fg) in [
            (
                add_y,
                "count",
                palette.diff_add_bg,
                palette.diff_add_word_bg,
                "+",
                palette.added,
            ),
            (
                del_y,
                "qty",
                palette.diff_del_bg,
                palette.diff_del_word_bg,
                "-",
                palette.deleted,
            ),
        ] {
            assert_eq!(
                cols_with_bg(buf, y, word_bg),
                needle_cols(buf, y, word),
                "only `{word}` sits on the word bg:\n{text}"
            );
            assert!(
                needle_cells(buf, y, "let total = price * ")
                    .iter()
                    .all(|cell| cell.bg == row_bg),
                "unchanged text keeps the row bg:\n{text}"
            );
            let after = needle_cols(buf, y, word).last().unwrap() + 1;
            assert_eq!(buf[(after, y)].symbol(), ";");
            assert_eq!(buf[(after, y)].bg, row_bg, "`;` after `{word}`");
            // The sign sits right before the code text.
            let sign_at = find_cell_col(buf, y, "let total").expect("code") - 1;
            let sign_cell = &buf[(sign_at, y)];
            assert_eq!(sign_cell.symbol(), sign);
            assert_eq!((sign_cell.fg, sign_cell.bg), (sign_fg, row_bg));
            assert!(sign_cell.modifier.contains(Modifier::BOLD));
            // Line number gutter and rule, right after the edge marker.
            let x0 = state.layout.diff_content_x + 1;
            for x in x0..sign_at {
                assert_eq!(buf[(x, y)].bg, row_bg, "gutter col {x} on row {y}:\n{text}");
            }
        }
        assert!(
            rows_with_bg(buf, palette.diff_add_word_bg) == vec![add_y]
                && rows_with_bg(buf, palette.diff_del_word_bg) == vec![del_y],
            "pure add / pure del rows have no word bg:\n{text}"
        );
        for pure in ["fresh_only", "stale_only"] {
            let y = first_row_with(buf, pure).expect(pure);
            let cells = needle_cells(buf, y, pure);
            assert!(
                cells
                    .iter()
                    .all(|c| c.bg == palette.diff_add_bg || c.bg == palette.diff_del_bg),
                "{pure} stays on its row bg:\n{text}"
            );
        }
    }

    #[test]
    fn split_word_diff_paints_word_bg_on_both_sides() {
        let mut state =
            word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::SideBySide);
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(220, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        assert_eq!(state.diff_layout(), DiffMode::SideBySide);
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let y = first_row_with(buf, "price * count").expect("split row");
        assert!(buf_line(buf, y).contains("price * qty"), "{text}");
        assert_eq!(
            cols_with_bg(buf, y, palette.diff_del_word_bg),
            needle_cols(buf, y, "qty"),
            "{text}"
        );
        assert_eq!(
            cols_with_bg(buf, y, palette.diff_add_word_bg),
            needle_cols(buf, y, "count"),
            "{text}"
        );
        for (needle, bg) in [
            ("let total = price * qty", palette.diff_del_bg),
            ("let total = price * count", palette.diff_add_bg),
        ] {
            let prefix = &needle[..needle.len() - needle.rsplit(' ').next().unwrap().len()];
            let start = find_cell_col(buf, y, needle).expect(needle);
            for i in 0..prefix.len() as u16 {
                assert_eq!(buf[(start + i, y)].bg, bg, "{needle} col {i}:\n{text}");
            }
        }
        for (needle, sign, bg) in [
            ("let total = price * qty", "-", palette.diff_del_bg),
            ("let total = price * count", "+", palette.diff_add_bg),
        ] {
            let at = find_cell_col(buf, y, needle).expect(needle) - 1;
            assert_eq!((buf[(at, y)].symbol(), buf[(at, y)].bg), (sign, bg));
        }
        assert_eq!(rows_with_bg(buf, palette.diff_add_word_bg), vec![y]);
        assert_eq!(rows_with_bg(buf, palette.diff_del_word_bg), vec![y]);
    }

    #[test]
    fn two_edits_on_one_line_keep_the_middle_on_the_row_bg() {
        let mut state = word_diff_state(
            word_diff_lines("call(alpha, middle, beta);", "call(gamma, middle, delta);"),
            DiffMode::Inline,
        );
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let y = first_row_with(buf, "call(gamma").expect("add line");
        let mut expected = needle_cols(buf, y, "gamma");
        expected.extend(needle_cols(buf, y, "delta"));
        assert_eq!(
            cols_with_bg(buf, y, palette.diff_add_word_bg),
            expected,
            "{text}"
        );
        assert!(
            needle_cells(buf, y, ", middle, ")
                .iter()
                .all(|cell| cell.bg == palette.diff_add_bg),
            "text between two edits stays on the row bg:\n{text}"
        );
    }

    #[test]
    fn wrapped_word_diff_keeps_word_bg_on_continuation_row() {
        let shared = format!("let total = {}", "price + ".repeat(14));
        let mut state = word_diff_state(
            word_diff_lines(&format!("{shared}qty;"), &format!("{shared}count;")),
            DiffMode::Inline,
        );
        state.diff_wrap = true;
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let first_y = first_row_with(buf, "let total = price").expect("first add row");
        let add_first_y = (first_y..buf.area().height)
            .find(|&y| {
                buf_line(buf, y).contains("let total = price")
                    && cols_with_bg(buf, y, palette.diff_add_bg).len() > 1
            })
            .expect("add line first row");
        let word_rows = rows_with_bg(buf, palette.diff_add_word_bg);
        assert!(!word_rows.is_empty(), "{text}");
        assert!(
            word_rows.iter().all(|&y| y > add_first_y),
            "the changed word wraps onto a continuation row: {word_rows:?}\n{text}"
        );
        let painted: String = word_rows
            .iter()
            .flat_map(|&y| {
                cols_with_bg(buf, y, palette.diff_add_word_bg)
                    .into_iter()
                    .map(move |x| buf[(x, y)].symbol().to_string())
            })
            .collect();
        assert_eq!(painted, "count", "{text}");
        let y = word_rows[0];
        let first_word = cols_with_bg(buf, y, palette.diff_add_word_bg)[0];
        let x0 = state.layout.diff_content_x + 1;
        for x in x0..first_word {
            assert_eq!(
                buf[(x, y)].bg,
                palette.diff_add_bg,
                "continuation gutter / text col {x} stays on the row bg:\n{text}"
            );
        }
    }

    #[test]
    fn panned_word_diff_stays_on_changed_glyphs_past_an_emoji() {
        let tail = format!(" // {}", "z".repeat(150));
        let mut state = word_diff_state(
            word_diff_lines(
                &format!("🙂 let total = price * qty;{tail}"),
                &format!("🙂 let total = price * count;{tail}"),
            ),
            DiffMode::Inline,
        );
        state.diff_wrap = false;
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        for offset in [0u16, 1, 2, 5] {
            state.diff_col_offset = offset;
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let y = first_row_with(buf, "price * count").expect("add line");
            assert_eq!(
                cols_with_bg(buf, y, palette.diff_add_word_bg),
                needle_cols(buf, y, "count"),
                "pan {offset}:\n{text}"
            );
            let del_y = first_row_with(buf, "price * qty").expect("del line");
            assert_eq!(
                cols_with_bg(buf, del_y, palette.diff_del_word_bg),
                needle_cols(buf, del_y, "qty"),
                "pan {offset}:\n{text}"
            );
            if offset == 0 {
                let emoji = find_cell_col(buf, y, "🙂").expect("whole emoji at pan 0");
                assert_eq!(buf[(emoji, y)].bg, palette.diff_add_bg);
            } else {
                assert!(
                    !buf_line(buf, y).contains('🙂'),
                    "a panned emoji is dropped whole, never half-painted:\n{text}"
                );
            }
        }
    }

    #[test]
    fn cursor_row_overlay_replaces_the_word_bg() {
        let mut state = word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
        let palette = state.theme.palette();
        state.focus = FocusPane::Right;
        state.diff_cursor = diff_row_index(&state, "price * count");
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let add_y = first_row_with(buf, "price * count").expect("add line");
        let del_y = first_row_with(buf, "price * qty").expect("del line");
        assert!(
            cols_with_bg(buf, add_y, palette.diff_add_word_bg).is_empty(),
            "cursor row has no word bg:\n{text}"
        );
        assert!(
            needle_cells(buf, add_y, "count")
                .iter()
                .all(|cell| cell.bg == palette.cursor_bg),
            "cursor bg wins over the word bg:\n{text}"
        );
        assert_eq!(
            cols_with_bg(buf, del_y, palette.diff_del_word_bg),
            needle_cols(buf, del_y, "qty"),
            "non-cursor row keeps its word bg:\n{text}"
        );
    }

    #[test]
    fn same_size_reload_recomputes_word_ranges() {
        let mut state = word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let rows_before = state.current_diff_rows().len();
        state.set_diff(
            "app".into(),
            "calc.rs".into(),
            super::super::diff::DiffContent::from_lines(word_diff_lines(
                QTY_LINE,
                "let total = cost * qty;",
            )),
        );
        assert_eq!(state.current_diff_rows().len(), rows_before);
        draw_state(&mut terminal, &mut state);
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let add_y = first_row_with(buf, "cost * qty").expect("reloaded add line");
        let del_y = first_row_with(buf, "price * qty").expect("del line");
        assert_eq!(
            cols_with_bg(buf, add_y, palette.diff_add_word_bg),
            needle_cols(buf, add_y, "cost"),
            "{text}"
        );
        assert_eq!(
            cols_with_bg(buf, del_y, palette.diff_del_word_bg),
            needle_cols(buf, del_y, "price"),
            "{text}"
        );
    }

    fn compare_json_over_stale_workspace_state() -> AppState {
        let mut state = two_pane_diff_state();
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![super::super::drill::CommitFile {
                status: "M".into(),
                path: "parked-drill.md".into(),
                old_path: None,
                stat: None,
            }],
        );
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.loading = false;
            tab.path = Some("pack.json".into());
            tab.files = vec![super::super::drill::CommitFile {
                status: "M".into(),
                path: "pack.json".into(),
                old_path: None,
                stat: None,
            }];
            tab.content = super::super::diff::DiffContent::from_compare_lines(json_named_lines(
                "alpha-syntax",
            ));
        }
        state
    }

    #[test]
    fn compare_space_paints_viewed_eye_on_file_row() {
        let mut state = compare_json_over_stale_workspace_state();
        state.ascii = false;
        state.focus = FocusPane::Left;
        state.tabs.active_compare_mut().unwrap().source =
            Some(super::super::drill::CommitFileSource::Compare {
                base_ref: "main".into(),
                head_ref: "HEAD".into(),
                base_tip: "bbb".into(),
                merge_base: "ccc".into(),
                head: "ddd".into(),
            });
        let eye = super::super::icons::icon_viewed(false);
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(!buffer_text(&terminal).contains(eye));
        state.dispatch(crate::tui::action::Action::ToggleReviewed);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let painted = buffer_text(&terminal);
        let row = painted
            .lines()
            .find(|line| line.contains("pack.json") && line.contains(eye))
            .unwrap_or_else(|| panic!("eye on pack.json row:\n{painted}"));
        assert!(row.find(eye) > row.find("pack.json"), "{row}");
    }

    #[test]
    fn compare_tab_syntax_path_uses_active_file_not_workspace() {
        let state = compare_json_over_stale_workspace_state();
        assert_eq!(state.diff_path.as_deref(), Some("README.md"));
        assert!(matches!(state.drill, DrillView::Files { .. }));
        let path = diff_syntax_path(&state);
        assert_eq!(path, "pack.json");
        let name = crate::tui::syntax::language_name(path, None);
        assert!(
            name.to_ascii_lowercase().contains("json"),
            "compare path must select JSON, got {name}"
        );
        let parked = crate::tui::syntax::language_name("README.md", None);
        assert!(
            !parked.to_ascii_lowercase().contains("json"),
            "stale workspace markdown must not select JSON, got {parked}"
        );
    }

    #[test]
    fn compare_tab_diff_paints_json_tokens_from_active_path() {
        let mut state = compare_json_over_stale_workspace_state();
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let add_y = first_row_with(buf, "alpha-syntax").expect("compare JSON add line");
        let add_span = needle_cells(buf, add_y, r#""name": "alpha-syntax""#);
        let add_fgs: std::collections::HashSet<_> = add_span.iter().map(|cell| cell.fg).collect();
        assert!(
            add_fgs.len() >= 2,
            "compare-tab JSON must use the .json path, not parked markdown: {add_fgs:?}\n{text}"
        );
    }

    fn assert_json_name_paints_fresh_syntax(
        terminal: &Terminal<TestBackend>,
        palette: crate::tui::theme::Palette,
        name: &str,
        stale: &str,
    ) {
        let buf = terminal.backend().buffer();
        let text = buffer_text(terminal);
        assert!(text.contains(name), "expected {name} after reload:\n{text}");
        assert!(
            !text.contains(stale),
            "stale cached text {stale} must not remain:\n{text}"
        );
        let add_y = first_row_with(buf, name).expect("JSON name line");
        let add_span = needle_cells(buf, add_y, &format!(r#""name": "{name}""#));
        let add_fgs: std::collections::HashSet<_> = add_span.iter().map(|cell| cell.fg).collect();
        assert!(
            add_fgs.len() >= 2,
            "replacement JSON must get new token colours, not a stale span list: {add_fgs:?}\n{text}"
        );
        let add_cells = needle_cells(buf, add_y, name);
        assert!(
            add_cells.iter().all(|cell| cell.bg == palette.diff_add_bg),
            "same-size reload must keep diff_add_bg:\n{text}"
        );
        let plus_at = find_cell_col(buf, add_y, "+").expect("add sign");
        let plus = &buf[(plus_at, add_y)];
        assert_eq!(plus.fg, palette.added, "add sign stays added fg");
        assert_eq!(plus.bg, palette.diff_add_bg, "add sign stays on add bg");
    }

    #[test]
    fn same_size_watch_replacement_invalidates_syntax_cache() {
        let mut state = json_syntax_diff_state();
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let first = buffer_text(&terminal);
        assert!(first.contains("alpha-syntax"), "{first}");
        let rows_before = state.current_diff_rows().len();
        let ptr_before = std::ptr::from_ref(state.current_diff_content());
        state.set_diff(
            "app".into(),
            "pack.json".into(),
            super::super::diff::DiffContent::from_lines(json_named_lines("omega-syntax")),
        );
        assert_eq!(
            std::ptr::from_ref(state.current_diff_content()),
            ptr_before,
            "set_diff keeps the DiffContent field address; cache must not key on the pointer"
        );
        assert_eq!(
            state.current_diff_rows().len(),
            rows_before,
            "same-size replacement keeps the row count"
        );
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_json_name_paints_fresh_syntax(
            &terminal,
            state.theme.palette(),
            "omega-syntax",
            "alpha-syntax",
        );
    }

    #[test]
    fn same_size_compare_replacement_invalidates_syntax_cache() {
        let mut state = compare_json_over_stale_workspace_state();
        let backend = TestBackend::new(120, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let first = buffer_text(&terminal);
        assert!(first.contains("alpha-syntax"), "{first}");
        let source = super::super::drill::CommitFileSource::Compare {
            base_ref: "main".into(),
            head_ref: "HEAD".into(),
            base_tip: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            merge_base: "cccccccccccccccccccccccccccccccccccccccc".into(),
            head: "dddddddddddddddddddddddddddddddddddddddd".into(),
        };
        let (tab_id, gen) = {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.source = Some(source.clone());
            (tab.id, tab.generation)
        };
        let rows_before = state.current_diff_rows().len();
        let ptr_before = std::ptr::from_ref(state.current_diff_content());
        state.apply_compare_diff(
            tab_id,
            gen,
            &source,
            "pack.json",
            Ok(super::super::diff::DiffContent::from_compare_lines(
                json_named_lines("omega-syntax"),
            )),
        );
        assert_eq!(
            std::ptr::from_ref(state.current_diff_content()),
            ptr_before,
            "apply_compare_diff keeps CompareTab.content address; cache must not key on the pointer"
        );
        assert_eq!(
            state.current_diff_rows().len(),
            rows_before,
            "same-size compare replacement keeps the row count"
        );
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_json_name_paints_fresh_syntax(
            &terminal,
            state.theme.palette(),
            "omega-syntax",
            "alpha-syntax",
        );
    }

    fn unfocused_inner(state: &AppState) -> (u16, u16, u16, u16) {
        let layout = &state.layout;
        match state.focus {
            FocusPane::Left => (
                layout.diff_content_x,
                layout.right_y,
                layout.diff_pane_width,
                layout.diff_pane_height,
            ),
            FocusPane::Right => (
                layout.tree_x,
                layout.tree_y,
                layout.tree_width,
                layout.tree_height,
            ),
        }
    }

    fn assert_unfocused_body_not_dimmed(state: &mut AppState) {
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
        let buf = terminal.backend().buffer();
        let (x, y, w, h) = unfocused_inner(state);
        assert!(
            w > 0 && h > 0,
            "unfocused inner must exist after paint ({x},{y} {w}x{h})"
        );
        let mut saw_body = false;
        for row in y..y.saturating_add(h) {
            for col in x..x.saturating_add(w) {
                let cell = &buf[(col, row)];
                if cell.symbol().chars().all(|c| c.is_whitespace()) {
                    continue;
                }
                saw_body = true;
                assert!(
                    !cell.modifier.contains(Modifier::DIM),
                    "unfocused body {:?} at ({col},{row}) must not DIM:\n{}",
                    cell.symbol(),
                    buffer_text(&terminal)
                );
            }
        }
        assert!(
            saw_body,
            "expected body cells in unfocused pane:\n{}",
            buffer_text(&terminal)
        );
    }

    #[test]
    fn unfocused_pane_body_is_not_dimmed() {
        let mut state = two_pane_diff_state();
        state.focus = FocusPane::Left;
        assert_unfocused_body_not_dimmed(&mut state);
        state.focus = FocusPane::Right;
        assert_unfocused_body_not_dimmed(&mut state);
    }

    fn buf_line(buf: &ratatui::buffer::Buffer, y: u16) -> String {
        let mut line = String::new();
        for x in 0..buf.area().width {
            line.push_str(buf[(x, y)].symbol());
        }
        line
    }

    fn find_cell_col(buf: &ratatui::buffer::Buffer, y: u16, needle: &str) -> Option<u16> {
        let chars: Vec<char> = needle.chars().collect();
        if chars.is_empty() {
            return None;
        }
        let width = buf.area().width;
        let n = chars.len() as u16;
        if n > width {
            return None;
        }
        for x in 0..=width - n {
            let mut ok = true;
            for (i, ch) in chars.iter().enumerate() {
                if buf[(x + i as u16, y)].symbol() != ch.to_string() {
                    ok = false;
                    break;
                }
            }
            if ok {
                return Some(x);
            }
        }
        None
    }

    fn title_fg(buf: &ratatui::buffer::Buffer, name: &str) -> Color {
        let y = 1;
        let col = find_cell_col(buf, y, name)
            .unwrap_or_else(|| panic!("title {name:?} on row {y}:\n{}", buf_line(buf, y)));
        buf[(col, y)].fg
    }

    fn pane_inner_has_symbol(
        buf: &ratatui::buffer::Buffer,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        symbol: &str,
    ) -> bool {
        for row in y..y.saturating_add(h) {
            for col in x..x.saturating_add(w) {
                if buf[(col, row)].symbol() == symbol {
                    return true;
                }
            }
        }
        false
    }

    fn pane_row_has_cursor_bg(
        buf: &ratatui::buffer::Buffer,
        x: u16,
        y: u16,
        w: u16,
        h: u16,
        needle: &str,
        cursor_bg: Color,
    ) -> bool {
        for row in y..y.saturating_add(h) {
            let mut line = String::new();
            for col in x..x.saturating_add(w) {
                line.push_str(buf[(col, row)].symbol());
            }
            if !line.contains(needle) {
                continue;
            }
            for col in x..x.saturating_add(w) {
                if buf[(col, row)].bg == cursor_bg {
                    return true;
                }
            }
        }
        false
    }

    fn two_pane_graph_state() -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let repo_row = state
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Repo)
            .expect("repo row");
        state.cursor = repo_row;
        state.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: "seed graph".into(),
                refs: vec!["main".into()],
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
                ..Commit::default()
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: Some(true),
            ..GraphModel::default()
        });
        state
    }

    #[test]
    fn right_graph_chrome_wraps_the_footer_at_the_painted_width() {
        let mut state = two_pane_graph_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let inner = state.layout.diff_pane_width as usize;
        assert!(inner > 10);
        // Exactly the inner width: the widget wraps at inner - 1, so the
        // subject takes 2 rows. The outer pane width would fit it in 1.
        let subject = format!("{}Z", "s".repeat(inner - 1));
        if let Some(model) = state.graph.as_mut() {
            model.commits[0].subject = subject;
            model.commits[0].body = "b1\nb2".into();
        }
        state.graph_cursor = 1;
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        // subject (2) + blank + b1 + b2 + meta
        assert_eq!(state.graph_chrome().footer_height, 6);
        let text = buffer_text(&terminal);
        assert!(
            text.lines().any(|line| line.contains("│Z")),
            "subject tail wraps to its own footer row:\n{text}"
        );
    }

    /// A graph frame paints the model once: the widget paints it and the
    /// scrollbar record counts its lines without a second paint. Both a
    /// list that fits and one that overflows (vertical bar) hold.
    #[test]
    fn graph_frame_paints_the_model_once() {
        let mut state = two_pane_graph_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        for commits in [1usize, 40] {
            if let Some(model) = state.graph.as_mut() {
                let seed = model.commits[0].clone();
                model.commits = (0..commits)
                    .map(|i| Commit {
                        id: format!("{i:040x}"),
                        ..seed.clone()
                    })
                    .collect();
            }
            let before = workspace_status_graph::paint_calls();
            draw_state(&mut terminal, &mut state);
            assert_eq!(
                workspace_status_graph::paint_calls() - before,
                1,
                "{commits} commits"
            );
            let model = state.graph.as_ref().unwrap();
            assert_eq!(
                state.layout.graph_content_len,
                workspace_status_graph::paint_model(model, &workspace_status_graph::UNICODE, None)
                    .len()
            );
            assert_eq!(state.layout.graph_scrollbar_x.is_some(), commits > 1);
        }
    }

    #[test]
    fn graph_footer_shows_long_message_by_default_and_wheel_scrolls_it() {
        let mut state = two_pane_graph_state();
        let body = (0..30)
            .map(|i| format!("BODYLINE{i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        if let Some(model) = state.graph.as_mut() {
            model.commits[0].body = body;
        }
        state.graph_cursor = 1;
        assert!(state.commit_msg_expand, "multiline is the default");
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("BODYLINE00"),
            "body shows with no key:\n{text}"
        );
        assert!(
            !text.contains("BODYLINE29"),
            "tail is below the fold:\n{text}"
        );
        let footer_y = state
            .layout
            .graph_footer_y
            .expect("overflowing footer records its hit box");
        let col = state.layout.graph_footer_x + 4;
        let max = state.layout.graph_footer_scroll_max;
        assert!(max > 0);

        for _ in 0..max + 5 {
            state.dispatch(Action::ScrollWheel {
                col,
                row: footer_y,
                delta: 1,
                horizontal: false,
            });
        }
        assert_eq!(state.graph_cursor, 1, "footer wheel must not move the list");
        assert_eq!(state.graph_footer_msg_scroll(), max, "clamped at the end");
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("BODYLINE29"),
            "wheel reveals the tail:\n{text}"
        );
        assert!(!text.contains("BODYLINE00"), "{text}");
        assert!(text.contains("aaa1111"), "meta line stays pinned:\n{text}");

        state.dispatch(Action::ScrollWheel {
            col,
            row: footer_y,
            delta: -1,
            horizontal: false,
        });
        assert_eq!(
            state.graph_footer_msg_scroll(),
            max - 1,
            "wheel up scrolls back"
        );

        state.graph_cursor = 0;
        assert_eq!(
            state.graph_footer_msg_scroll(),
            0,
            "another row starts at the top"
        );
        state.graph_cursor = 1;
        assert_eq!(
            state.graph_footer_msg_scroll(),
            max - 1,
            "same row keeps its scroll"
        );
    }

    fn two_pane_files_state() -> AppState {
        let mut state = two_pane_graph_state();
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![super::super::drill::CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
        );
        state
    }

    #[test]
    fn commit_file_list_fills_pty_height_and_does_not_paint_hbar() {
        let mut state = two_pane_graph_state();
        let files = (0..40)
            .map(|i| super::super::drill::CommitFile {
                status: "A".into(),
                path: format!("keepmid-{i:02}.txt"),
                old_path: None,
                stat: None,
            })
            .collect();
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files,
        );
        state.focus = FocusPane::Right;
        let backend = TestBackend::new(140, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        let file_rows = text
            .lines()
            .filter(|line| line.contains("keepmid-") && line.contains(".txt"))
            .count();
        assert!(
            file_rows >= 8,
            "commit-file list should fill the pane, not a short strip ({file_rows}):\n{text}"
        );
        assert!(
            text.contains("keepmid-00.txt") && text.contains("keepmid-07.txt"),
            "row 0 window must include the first eight files:\n{text}"
        );
        assert!(
            state.layout.diff_hscrollbar_y.is_none(),
            "commit-file list must not paint a file-diff h-bar: y={:?}",
            state.layout.diff_hscrollbar_y
        );
        assert_eq!(
            state.layout.diff_col_max, 0,
            "commit-file list must not arm a file-diff h-bar track"
        );
    }

    fn two_pane_commit_diff_state() -> AppState {
        let mut state = two_pane_files_state();
        state.open_commit_diff(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![super::super::drill::CommitFile {
                status: "M".into(),
                path: "README.md".into(),
                old_path: None,
                stat: None,
            }],
            0,
            "README.md".into(),
            super::super::diff::DiffContent::from_lines(vec![
                "@@ -1,1 +1,1 @@".into(),
                "-old line".into(),
                "+new line".into(),
            ]),
        );
        state
    }

    /// Text of the `w`×`h` buffer region at `(x, y)`, one line per row.
    fn region_text(terminal: &Terminal<TestBackend>, x: u16, y: u16, w: u16, h: u16) -> String {
        let buf = terminal.backend().buffer();
        let mut out = String::new();
        for row in y..y + h {
            for col in x..x + w {
                out.push_str(buf[(col, row)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn left_inner_text(terminal: &Terminal<TestBackend>, state: &AppState) -> String {
        let l = &state.layout;
        region_text(terminal, l.tree_x, l.tree_y, l.tree_width, l.tree_height)
    }

    fn right_inner_text(terminal: &Terminal<TestBackend>, state: &AppState) -> String {
        let l = &state.layout;
        region_text(
            terminal,
            l.diff_content_x,
            l.right_y,
            l.diff_pane_width,
            l.diff_pane_height,
        )
    }

    const FOOTER_BODY_TOKEN: &str = "FOOTERBODYTOKEN";

    fn set_seed_commit_body(state: &mut AppState, body: &str) {
        if let Some(model) = state.graph.as_mut() {
            model.commits[0].body = body.into();
        }
    }

    #[test]
    fn depth_1_files_pane_has_no_commit_footer_beside_the_graph_footer() {
        let mut state = two_pane_files_state();
        set_seed_commit_body(&mut state, FOOTER_BODY_TOKEN);
        // Select the seed commit row (row 0 is uncommitted changes).
        state.graph_cursor = 1;
        assert!(state.commit_msg_expand, "multiline is the default");
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let left = left_inner_text(&terminal, &state);
        let right = right_inner_text(&terminal, &state);
        assert!(
            left.contains(FOOTER_BODY_TOKEN),
            "graph footer keeps the message:\n{left}"
        );
        assert!(right.contains("README.md"), "files list paints:\n{right}");
        assert!(
            !right.contains(FOOTER_BODY_TOKEN),
            "files pane has no commit footer at depth 1:\n{right}"
        );
        assert_eq!(state.layout.files_list_y, state.layout.right_y);
        assert_eq!(
            state.layout.files_list_height, state.layout.diff_pane_height,
            "file list takes the full files pane"
        );
    }

    #[test]
    fn depth_2_left_files_pane_pins_commit_footer_under_the_list() {
        let mut state = two_pane_commit_diff_state();
        set_seed_commit_body(&mut state, FOOTER_BODY_TOKEN);
        state.focus = FocusPane::Left;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let left = left_inner_text(&terminal, &state);
        let right = right_inner_text(&terminal, &state);
        assert!(
            !right.contains(FOOTER_BODY_TOKEN),
            "diff pane has no commit footer:\n{right}"
        );
        let rows: Vec<&str> = left.lines().collect();
        let file_row = rows
            .iter()
            .position(|row| row.contains("README.md"))
            .unwrap_or_else(|| panic!("file list paints:\n{left}"));
        let token_row = rows
            .iter()
            .position(|row| row.contains(FOOTER_BODY_TOKEN))
            .unwrap_or_else(|| panic!("footer shows the message:\n{left}"));
        assert!(token_row > file_row, "footer sits under the list:\n{left}");

        let layout = &state.layout;
        let footer_len = state
            .commit_detail_footer_lines(layout.tree_width as usize)
            .len();
        let footer_h = commit_detail_footer_height(footer_len, layout.tree_height);
        assert!(footer_h > 0);
        assert_eq!(layout.files_list_y, layout.tree_y);
        assert_eq!(
            layout.files_list_height,
            layout.tree_height - footer_h,
            "click map matches the painted list"
        );
        assert!(
            token_row >= layout.files_list_height as usize,
            "footer rows are below the list rows:\n{left}"
        );
    }

    #[test]
    fn depth_2_left_footer_keeps_one_list_row_on_a_short_pane() {
        let mut state = two_pane_commit_diff_state();
        let body = (0..30)
            .map(|i| format!("{FOOTER_BODY_TOKEN}{i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        set_seed_commit_body(&mut state, &body);
        let mut terminal = Terminal::new(TestBackend::new(120, MIN_TERM_ROWS)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let layout = &state.layout;
        let footer_len = state
            .commit_detail_footer_lines(layout.tree_width as usize)
            .len();
        assert!(
            footer_len >= layout.tree_height as usize,
            "footer is taller than the pane ({footer_len} >= {})",
            layout.tree_height
        );
        assert_eq!(layout.files_list_height, 1, "list keeps one row");
        let left = left_inner_text(&terminal, &state);
        let first = left.lines().next().unwrap_or_default();
        assert!(first.contains("README.md"), "list row paints:\n{left}");
        assert!(left.contains(FOOTER_BODY_TOKEN), "footer paints:\n{left}");
    }

    fn assert_titles_heading_borders_split(state: &mut AppState) {
        let palette = state.theme.palette();
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let title_y = 1;
        let top = buf_line(buf, title_y);
        let left_name = if state.drill.is_diff() {
            "files"
        } else if state.drill.is_files() {
            "graph"
        } else {
            "tree"
        };
        let right_name = if state.drill.is_files() {
            "files"
        } else if state.drill.is_diff() || state.right_is_diff() {
            "diff"
        } else {
            "graph"
        };
        assert!(
            top.contains(left_name) && top.contains(right_name),
            "title row names {left_name}/{right_name}:\n{text}"
        );
        let left_title = title_fg(buf, left_name);
        let right_title = title_fg(buf, right_name);
        assert_eq!(
            left_title, palette.heading,
            "left title {left_name:?} fg must be heading, got {left_title:?}:\n{text}"
        );
        assert_eq!(
            right_title, palette.heading,
            "right title {right_name:?} fg must be heading, got {right_title:?}:\n{text}"
        );
        assert_ne!(
            left_title, palette.border_dim,
            "title fg must not be border_dim:\n{text}"
        );
        assert_ne!(
            right_title, palette.border_dim,
            "unfocused title must not inherit border_dim:\n{text}"
        );
        let right_x = find_cell_col(buf, title_y, right_name)
            .unwrap_or_else(|| panic!("right title {right_name:?}:\n{text}"))
            .saturating_sub(1);
        let left_border = buf[(0, title_y)].fg;
        let right_border = buf[(right_x, title_y)].fg;
        match state.focus {
            FocusPane::Left => {
                assert_eq!(left_border, palette.heading, "focused left border:\n{text}");
                assert_eq!(
                    right_border, palette.border_dim,
                    "unfocused right border is border_dim:\n{text}"
                );
            }
            FocusPane::Right => {
                assert_eq!(
                    left_border, palette.border_dim,
                    "unfocused left border is border_dim:\n{text}"
                );
                assert_eq!(
                    right_border, palette.heading,
                    "focused right border:\n{text}"
                );
            }
        }
    }

    #[test]
    fn unfocused_pane_title_uses_heading_not_border_dim() {
        let mut state = two_pane_diff_state();
        state.focus = FocusPane::Left;
        assert_titles_heading_borders_split(&mut state);
        state.focus = FocusPane::Right;
        assert_titles_heading_borders_split(&mut state);

        let mut graph = two_pane_graph_state();
        graph.focus = FocusPane::Left;
        assert_titles_heading_borders_split(&mut graph);
        graph.focus = FocusPane::Right;
        assert_titles_heading_borders_split(&mut graph);

        let mut files = two_pane_files_state();
        files.focus = FocusPane::Left;
        assert_titles_heading_borders_split(&mut files);
        files.focus = FocusPane::Right;
        assert_titles_heading_borders_split(&mut files);
    }

    fn assert_cursor_chrome_strong_on_focused_weak_on_unfocused(
        state: &mut AppState,
        left_needle: &str,
        right_needle: &str,
    ) {
        let palette = state.theme.palette();
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let left = (
            state.layout.tree_x,
            state.layout.tree_y,
            state.layout.tree_width,
            state.layout.tree_height,
        );
        let right = (
            state.layout.diff_content_x,
            state.layout.right_y,
            state.layout.diff_pane_width,
            state.layout.diff_pane_height,
        );
        let left_bar = pane_inner_has_symbol(buf, left.0, left.1, left.2, left.3, CURSOR_BAR);
        let right_bar = pane_inner_has_symbol(buf, right.0, right.1, right.2, right.3, CURSOR_BAR);
        let left_inactive =
            pane_inner_has_symbol(buf, left.0, left.1, left.2, left.3, CURSOR_BAR_INACTIVE);
        let right_inactive =
            pane_inner_has_symbol(buf, right.0, right.1, right.2, right.3, CURSOR_BAR_INACTIVE);
        let left_bg = pane_row_has_cursor_bg(
            buf,
            left.0,
            left.1,
            left.2,
            left.3,
            left_needle,
            palette.cursor_bg,
        );
        let right_bg = pane_row_has_cursor_bg(
            buf,
            right.0,
            right.1,
            right.2,
            right.3,
            right_needle,
            palette.cursor_bg,
        );
        let left_inactive_bg = pane_row_has_cursor_bg(
            buf,
            left.0,
            left.1,
            left.2,
            left.3,
            left_needle,
            palette.cursor_bg_inactive,
        );
        let right_inactive_bg = pane_row_has_cursor_bg(
            buf,
            right.0,
            right.1,
            right.2,
            right.3,
            right_needle,
            palette.cursor_bg_inactive,
        );
        match state.focus {
            FocusPane::Left => {
                assert!(
                    left_bar,
                    "focused left list must paint {CURSOR_BAR} near {left_needle:?}:\n{text}"
                );
                assert!(
                    left_bg,
                    "focused left list must paint cursor_bg on {left_needle:?}:\n{text}"
                );
                assert!(
                    !left_inactive,
                    "focused left list must not paint {CURSOR_BAR_INACTIVE}:\n{text}"
                );
                assert!(
                    !right_bar,
                    "unfocused right list must not paint {CURSOR_BAR}:\n{text}"
                );
                assert!(
                    right_inactive,
                    "unfocused right list must paint {CURSOR_BAR_INACTIVE} near {right_needle:?}:\n{text}"
                );
                assert!(
                    !right_bg,
                    "unfocused right list must not paint focused cursor_bg:\n{text}"
                );
                assert!(
                    right_inactive_bg,
                    "unfocused right list must paint cursor_bg_inactive on {right_needle:?}:\n{text}"
                );
            }
            FocusPane::Right => {
                assert!(
                    !left_bar,
                    "unfocused left list must not paint {CURSOR_BAR}:\n{text}"
                );
                assert!(
                    left_inactive,
                    "unfocused left list must paint {CURSOR_BAR_INACTIVE} near {left_needle:?}:\n{text}"
                );
                assert!(
                    !left_bg,
                    "unfocused left list must not paint focused cursor_bg:\n{text}"
                );
                assert!(
                    left_inactive_bg,
                    "unfocused left list must paint cursor_bg_inactive on {left_needle:?}:\n{text}"
                );
                assert!(
                    right_bar,
                    "focused right list must paint {CURSOR_BAR} near {right_needle:?}:\n{text}"
                );
                assert!(
                    !right_inactive,
                    "focused right list must not paint {CURSOR_BAR_INACTIVE}:\n{text}"
                );
                assert!(
                    right_bg,
                    "focused right list must paint cursor_bg on {right_needle:?}:\n{text}"
                );
            }
        }
    }

    #[test]
    fn cursor_chrome_strong_on_focused_weak_on_unfocused_tree_and_diff() {
        let mut state = two_pane_diff_state();
        state.focus = FocusPane::Left;
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(
            &mut state,
            "README.md",
            "UNSTAGED",
        );
        state.focus = FocusPane::Right;
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(
            &mut state,
            "README.md",
            "UNSTAGED",
        );
    }

    #[test]
    fn cursor_chrome_strong_on_focused_weak_on_unfocused_graph() {
        let mut state = two_pane_graph_state();
        state.focus = FocusPane::Left;
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(&mut state, "app", "uncommitted");
        state.focus = FocusPane::Right;
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(&mut state, "app", "uncommitted");
    }

    #[test]
    fn cursor_chrome_strong_on_focused_weak_on_unfocused_commit_files() {
        let mut state = two_pane_files_state();
        state.focus = FocusPane::Left;
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(
            &mut state,
            "uncommitted",
            "README.md",
        );
        state.focus = FocusPane::Right;
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(
            &mut state,
            "uncommitted",
            "README.md",
        );
    }

    #[test]
    fn cursor_chrome_strong_on_focused_weak_on_unfocused_commit_diff() {
        let mut state = two_pane_commit_diff_state();
        assert!(state.drill.is_diff());
        state.focus = FocusPane::Left;
        assert!(state.commit_files_list_focused());
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(
            &mut state,
            "README.md",
            "UNSTAGED",
        );
        state.focus = FocusPane::Right;
        assert!(!state.commit_files_list_focused());
        assert_cursor_chrome_strong_on_focused_weak_on_unfocused(
            &mut state,
            "README.md",
            "UNSTAGED",
        );
    }

    #[test]
    fn tab_close_glyph_is_three_display_columns() {
        assert_eq!(painted_width(TAB_CLOSE_GLYPH), 3);
    }

    #[test]
    fn tab_strip_hit_boxes_match_painted_cells_after_arrow_labels() {
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        state
            .tabs
            .open_or_focus("app".into(), "develop".into(), "HEAD".into());
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let tab_y = state.layout.tab_y;
        let buf = terminal.backend().buffer();
        let row: Vec<String> = (0..120u16)
            .map(|cx| buf[(cx, tab_y)].symbol().to_string())
            .collect();
        let line = row.concat();
        assert_eq!(line.matches('↔').count(), 2, "{line}");
        let painted_close: Vec<u16> = (0..118u16)
            .filter(|cx| row[*cx as usize..*cx as usize + 3].concat() == TAB_CLOSE_GLYPH)
            .collect();
        assert_eq!(painted_close.len(), 2, "{line}");
        let close_hits = state.layout.tab_close_hits.clone();
        for (nth, (x, width, index)) in close_hits.iter().enumerate() {
            assert_eq!(*index, nth + 1);
            assert_eq!(
                (*x, *width),
                (painted_close[nth], 3),
                "tab {index} close hit must cover the painted close: {line}"
            );
        }
        let tab_hits = state.layout.tab_hits.clone();
        assert_eq!(tab_hits.len(), 3);
        let labels = state.tabs.labels();
        for (x, width, index) in tab_hits {
            assert_eq!(
                row[x as usize], " ",
                "tab {index} starts at its leading space"
            );
            let text: String = row[x as usize..(x + width) as usize].concat();
            let want = if index == 0 {
                format!(" {} ", labels[0])
            } else {
                format!(" {} {TAB_CLOSE_GLYPH} ", labels[index])
            };
            assert_eq!(text, want, "tab {index} hit box spans its painted text");
            if index > 0 {
                assert_eq!(
                    row[x as usize - 1],
                    "│",
                    "separator sits left of tab {index}"
                );
            }
        }
    }

    #[test]
    fn tab_close_is_dim_until_hovered_on_active_and_inactive_tabs() {
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        state
            .tabs
            .open_or_focus("app".into(), "develop".into(), "HEAD".into());
        assert_eq!(state.tabs.active, 2);
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let hits = state.layout.tab_close_hits.clone();
        let tab_y = state.layout.tab_y;
        assert_eq!(hits.len(), 2, "{hits:?}");
        // Painted close-control columns, left to right.
        let painted = |terminal: &Terminal<TestBackend>| {
            let buf = terminal.backend().buffer();
            let row: Vec<&str> = (0..120).map(|cx| buf[(cx, tab_y)].symbol()).collect();
            (0..118u16)
                .filter(|cx| row[*cx as usize..*cx as usize + 3].concat() == TAB_CLOSE_GLYPH)
                .collect::<Vec<_>>()
        };
        let close_cells = |terminal: &Terminal<TestBackend>, nth: usize| {
            let x = painted(terminal)[nth];
            let buf = terminal.backend().buffer();
            (x..x + 3)
                .map(|cx| buf[(cx, tab_y)].style())
                .collect::<Vec<_>>()
        };
        let label_style = |terminal: &Terminal<TestBackend>, nth: usize| {
            terminal.backend().buffer()[(painted(terminal)[nth] - 2, tab_y)].style()
        };
        let assert_idle = |styles: Vec<Style>, bg: Option<Color>| {
            for style in styles {
                assert_eq!(style.fg, Some(palette.tab_close), "{style:?}");
                assert_eq!(style.bg.filter(|c| *c != Color::Reset), bg, "{style:?}");
                assert!(!style.add_modifier.contains(Modifier::BOLD), "{style:?}");
            }
        };
        let assert_hover = |styles: Vec<Style>, bg: Option<Color>| {
            for style in styles {
                assert_eq!(style.fg, Some(palette.tab_close_hover), "{style:?}");
                assert_eq!(style.bg.filter(|c| *c != Color::Reset), bg, "{style:?}");
                assert!(style.add_modifier.contains(Modifier::BOLD), "{style:?}");
            }
        };
        let (inactive, active) = (hits[0], hits[1]);
        assert_eq!((inactive.2, active.2), (1, 2));
        assert_eq!(
            painted(&terminal),
            vec![inactive.0, active.0],
            "hover boxes sit on the painted close control"
        );
        assert_idle(close_cells(&terminal, 0), None);
        assert_idle(close_cells(&terminal, 1), Some(palette.cursor_bg));
        assert_eq!(label_style(&terminal, 0).fg, Some(palette.muted));
        assert_eq!(label_style(&terminal, 1).fg, Some(palette.cursor));

        state.pointer = Some((inactive.0, tab_y));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(state.layout.tab_close_hits, hits, "hit boxes stay put");
        assert_eq!(state.hovered_tab_close(), Some(1));
        assert_hover(close_cells(&terminal, 0), None);
        assert_idle(close_cells(&terminal, 1), Some(palette.cursor_bg));
        assert_eq!(label_style(&terminal, 0).fg, Some(palette.muted));

        state.pointer = Some((active.0 + active.1 - 1, tab_y));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(state.hovered_tab_close(), Some(2));
        assert_idle(close_cells(&terminal, 0), None);
        assert_hover(close_cells(&terminal, 1), Some(palette.cursor_bg));
        assert_eq!(label_style(&terminal, 1).fg, Some(palette.cursor));

        state.pointer = Some((active.0 + active.1, tab_y));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(state.hovered_tab_close(), None);
        assert_idle(close_cells(&terminal, 1), Some(palette.cursor_bg));
    }

    /// Workspace plus five compare tabs `alpha` … `echo` against `main`.
    /// The last opened (`echo`) is active.
    fn five_compare_tabs() -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        for checkout in ["alpha", "bravo", "charlie", "delta", "echo"] {
            state
                .tabs
                .open_or_focus(checkout.into(), "main".into(), "HEAD".into());
        }
        assert_eq!(state.tabs.active, 5);
        state
    }

    /// Paint and return the tab row, one string per cell.
    /// Paint only the tab strip (row 0), so narrow widths below the
    /// terminal-too-small guard still exercise the strip's windowing.
    fn paint_tab_row(terminal: &mut Terminal<TestBackend>, state: &mut AppState) -> Vec<String> {
        terminal
            .draw(|frame| {
                let area = frame.area();
                draw_tab_strip(frame, Rect { height: 1, ..area }, state);
            })
            .unwrap();
        let cols = terminal.backend().buffer().area.width;
        let buf = terminal.backend().buffer();
        (0..cols)
            .map(|cx| buf[(cx, state.layout.tab_y)].symbol().to_string())
            .collect()
    }

    /// Every tab, `[✗]`, and marker hit box covers exactly its painted
    /// cells, and every painted `[✗]` has a close hit box.
    fn assert_tab_hits_match_paint(state: &AppState, row: &[String]) {
        let line = row.concat();
        let labels = state.tabs.labels();
        let painted: Vec<usize> = state
            .layout
            .tab_hits
            .iter()
            .map(|(_, _, index)| *index)
            .filter(|index| row_has_tab(&line, &labels, *index))
            .collect();
        let first = *painted.iter().min().unwrap();
        let end = *painted.iter().max().unwrap() + 1;
        for (x, width, index) in state.layout.tab_hits.clone() {
            let text: String = row[x as usize..(x + width) as usize].concat();
            if text.starts_with('\u{2039}') {
                assert_eq!(text, tab_left_marker(first), "{line}");
                assert_eq!(x, 0, "left marker is leftmost: {line}");
                assert_eq!(index, first - 1, "left marker targets nearest hidden");
            } else if text.ends_with('\u{203a}') {
                assert_eq!(text, tab_right_marker(labels.len() - end), "{line}");
                assert_eq!(
                    x + width,
                    row.len() as u16,
                    "right marker is right-aligned: {line}"
                );
                assert_eq!(index, end, "right marker targets nearest hidden");
            } else {
                let want = if index == 0 {
                    format!(" {} ", labels[0])
                } else {
                    format!(" {} {TAB_CLOSE_GLYPH} ", labels[index])
                };
                assert_eq!(text, want, "tab {index} hit box spans its text: {line}");
                if index > 0 {
                    assert_eq!(row[x as usize - 1], "│", "separator left of {index}");
                }
            }
        }
        let painted_close: Vec<u16> = (0..row.len().saturating_sub(2) as u16)
            .filter(|cx| row[*cx as usize..*cx as usize + 3].concat() == TAB_CLOSE_GLYPH)
            .collect();
        let close_x: Vec<u16> = state
            .layout
            .tab_close_hits
            .iter()
            .map(|(x, width, _)| {
                assert_eq!(*width, 3);
                *x
            })
            .collect();
        assert_eq!(
            close_x, painted_close,
            "close hits sit on painted [✗]: {line}"
        );
    }

    fn row_has_tab(line: &str, labels: &[String], index: usize) -> bool {
        let text = if index == 0 {
            format!(" {} ", labels[0])
        } else {
            format!(" {} {TAB_CLOSE_GLYPH} ", labels[index])
        };
        line.contains(&text)
    }

    /// Below the minimum only the resize notice paints, so only Esc, `q`,
    /// Ctrl-c, and resize map: pane keys and write keys (`s`, `P`, `p`)
    /// drop, and an open confirm or overlay cannot take a hidden `y`.
    #[test]
    fn terminal_below_minimum_drops_keys_for_a_hidden_overlay() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let key = |code, mods| Event::Key(KeyEvent::new(code, mods));
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        draw_state(&mut terminal, &mut state);
        assert!(state.too_small);
        for blind in ['j', 's', 'u', 'P', 'p', 'f'] {
            let ev = key(KeyCode::Char(blind), KeyModifiers::NONE);
            assert_eq!(
                super::super::app::map_event(&state, &ev),
                Action::None,
                "{blind}"
            );
        }
        assert_eq!(
            super::super::app::map_event(&state, &key(KeyCode::Char('q'), KeyModifiers::NONE)),
            Action::Quit
        );
        for allowed in [
            key(KeyCode::Esc, KeyModifiers::NONE),
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Event::Resize(MIN_TERM_COLS, MIN_TERM_ROWS),
        ] {
            assert_ne!(
                super::super::app::map_event(&state, &allowed),
                Action::None,
                "{allowed:?}"
            );
        }

        state.confirm = Some(PendingConfirm::Revert {
            label: "README.md".into(),
            targets: vec![crate::tui::state::RevertTarget {
                repo: "app".into(),
                path: "README.md".into(),
                untracked: false,
                old_path: None,
            }],
        });
        let y = key(KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(super::super::app::map_event(&state, &y), Action::None);
        let enter = key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(super::super::app::map_event(&state, &enter), Action::None);
        let esc = key(KeyCode::Esc, KeyModifiers::NONE);
        assert_ne!(super::super::app::map_event(&state, &esc), Action::None);

        state.confirm = None;
        state.help_open = true;
        let slash = key(KeyCode::Char('/'), KeyModifiers::NONE);
        assert_eq!(super::super::app::map_event(&state, &slash), Action::None);
        for allowed in [
            esc,
            key(KeyCode::Char('q'), KeyModifiers::NONE),
            key(KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            assert_ne!(
                super::super::app::map_event(&state, &allowed),
                Action::None,
                "{allowed:?}"
            );
        }

        // At the minimum size the overlay paints and takes every key again.
        let mut terminal = Terminal::new(TestBackend::new(MIN_TERM_COLS, MIN_TERM_ROWS)).unwrap();
        draw_state(&mut terminal, &mut state);
        assert!(!state.too_small);
        assert_eq!(
            super::super::app::map_event(&state, &slash),
            Action::SearchStart
        );
    }

    /// Below the minimum the frame is only the resize notice, and mouse
    /// events stop mapping; at the minimum the panes paint again.
    #[test]
    fn terminal_below_minimum_paints_only_the_resize_notice() {
        use crossterm::event::{
            Event, KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        for (cols, rows) in [
            (MIN_TERM_COLS - 1, MIN_TERM_ROWS),
            (MIN_TERM_COLS, MIN_TERM_ROWS - 1),
            (20, 5),
        ] {
            let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
            draw_state(&mut terminal, &mut state);
            let text = buffer_text(&terminal);
            let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(state.too_small, "{cols}×{rows}");
            assert!(
                flat.contains(&format!("{cols}×{rows}")) && flat.contains("terminal too small"),
                "{cols}×{rows}:\n{text}"
            );
            assert!(!text.contains("tree"), "no panes at {cols}×{rows}:\n{text}");
            let click = Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            });
            assert_eq!(super::super::app::map_event(&state, &click), Action::None);
            let q = Event::Key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
            assert_eq!(super::super::app::map_event(&state, &q), Action::Quit);
        }
        let mut terminal = Terminal::new(TestBackend::new(MIN_TERM_COLS, MIN_TERM_ROWS)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        assert!(!state.too_small);
        assert!(!text.contains("terminal too small"), "{text}");
        assert!(text.contains("tree"), "{text}");
    }

    #[test]
    fn narrow_tab_strip_paints_active_tab_and_left_marker() {
        let mut state = five_compare_tabs();
        let mut terminal = Terminal::new(TestBackend::new(40, 24)).unwrap();
        let row = paint_tab_row(&mut terminal, &mut state);
        let line = row.concat();
        assert_eq!(
            line, "\u{2039}4│ delta ↔ main [✗] │ echo ↔ main [✗]  ",
            "active echo stays painted"
        );
        assert_eq!(state.layout.tab_scroll, 4);
        assert_tab_hits_match_paint(&state, &row);
        assert!(state.layout.tab_hits.contains(&(0, 2, 3)));
        assert_eq!(state.layout.tab_hits.len(), 3);
        assert_eq!(state.layout.tab_close_hits.len(), 2);
    }

    #[test]
    fn narrow_tab_strip_window_follows_the_active_tab() {
        let mut state = five_compare_tabs();
        let mut terminal = Terminal::new(TestBackend::new(40, 24)).unwrap();
        paint_tab_row(&mut terminal, &mut state);
        assert_eq!(state.layout.tab_scroll, 4);

        // `g5`: delta is already painted, so the window does not move.
        state.dispatch(Action::JumpToTab(5));
        assert_eq!(state.tabs.active, 4);
        let row = paint_tab_row(&mut terminal, &mut state);
        assert_eq!(state.layout.tab_scroll, 4);
        assert_eq!(
            row.concat(),
            "\u{2039}4│ delta ↔ main [✗] │ echo ↔ main [✗]  "
        );
        assert_tab_hits_match_paint(&state, &row);

        // `g2`: alpha is off the left edge; the window scrolls to it.
        state.dispatch(Action::JumpToTab(2));
        assert_eq!(state.tabs.active, 1);
        let row = paint_tab_row(&mut terminal, &mut state);
        let line = row.concat();
        assert_eq!(
            line, " Workspace │ alpha ↔ main [✗]         4\u{203a}",
            "alpha painted, right marker counts the hidden tabs"
        );
        assert_eq!(state.layout.tab_scroll, 0);
        assert!(!line.contains('\u{2039}'), "{line}");
        assert_tab_hits_match_paint(&state, &row);
        assert!(state.layout.tab_hits.contains(&(38, 2, 2)));

        // `gT` to Workspace: inside the window, start stays.
        state.dispatch(Action::PreviousTab);
        assert_eq!(state.tabs.active, 0);
        let row = paint_tab_row(&mut terminal, &mut state);
        assert_eq!(state.layout.tab_scroll, 0);
        assert_eq!(row.concat(), line);
    }

    #[test]
    fn tab_strip_marker_click_focuses_the_nearest_hidden_tab() {
        let mut state = five_compare_tabs();
        let mut terminal = Terminal::new(TestBackend::new(40, 24)).unwrap();
        paint_tab_row(&mut terminal, &mut state);
        let tab_y = state.layout.tab_y;

        state.dispatch(Action::Click { col: 0, row: tab_y });
        assert_eq!(state.tabs.active, 3, "left marker focuses charlie");
        let row = paint_tab_row(&mut terminal, &mut state);
        let line = row.concat();
        assert_eq!(
            line, "\u{2039}3│ charlie ↔ main [✗]                2\u{203a}",
            "window scrolls exactly one tab"
        );
        assert_tab_hits_match_paint(&state, &row);

        state.dispatch(Action::Click {
            col: 39,
            row: tab_y,
        });
        assert_eq!(state.tabs.active, 4, "right marker focuses delta");
        let row = paint_tab_row(&mut terminal, &mut state);
        assert!(row.concat().contains(" delta ↔ main [✗] "), "{row:?}");
        assert_tab_hits_match_paint(&state, &row);
    }

    #[test]
    fn wide_tab_strip_paints_every_tab_without_markers() {
        let mut state = five_compare_tabs();
        state.layout.tab_scroll = 3;
        let mut terminal = Terminal::new(TestBackend::new(160, 24)).unwrap();
        let row = paint_tab_row(&mut terminal, &mut state);
        let line = row.concat();
        assert!(
            !line.contains('\u{2039}') && !line.contains('\u{203a}'),
            "{line}"
        );
        assert_eq!(line.matches('↔').count(), 5, "{line}");
        assert_eq!(state.layout.tab_hits.len(), 6);
        assert_eq!(state.layout.tab_scroll, 0, "all fit resets the window");
        assert_tab_hits_match_paint(&state, &row);
    }

    #[test]
    fn tab_strip_clips_an_active_tab_wider_than_the_space() {
        let mut state = five_compare_tabs();
        let mut terminal = Terminal::new(TestBackend::new(16, 24)).unwrap();
        let row = paint_tab_row(&mut terminal, &mut state);
        let line = row.concat();
        assert_eq!(line, "\u{2039}5│ echo ↔ main ");
        assert_eq!(state.layout.tab_hits, vec![(0, 2, 4), (3, 13, 5)]);
        assert!(
            state.layout.tab_close_hits.is_empty(),
            "a clipped [✗] has no close hit"
        );
    }

    #[test]
    fn tab_strip_clipped_close_never_paints_hovered() {
        let mut state = five_compare_tabs();
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(17, 24)).unwrap();
        let row = paint_tab_row(&mut terminal, &mut state);
        let tab_y = state.layout.tab_y;
        assert_eq!(row[16], "[", "only the first cell of [✗] is painted");
        assert!(state.layout.tab_close_hits.is_empty());

        state.pointer = Some((16, tab_y));
        paint_tab_row(&mut terminal, &mut state);
        let style = terminal.backend().buffer()[(16, tab_y)].style();
        assert_eq!(style.fg, Some(palette.tab_close), "{style:?}");
        assert!(!style.add_modifier.contains(Modifier::BOLD), "{style:?}");
    }

    #[test]
    fn tab_strip_records_no_zero_width_hit_box() {
        for width in [2u16, 3] {
            let mut state = five_compare_tabs();
            state.tabs.active = 2;
            let mut terminal = Terminal::new(TestBackend::new(width, 24)).unwrap();
            paint_tab_row(&mut terminal, &mut state);
            assert!(
                state.layout.tab_hits.iter().all(|(_, w, _)| *w > 0),
                "width {width}: {:?}",
                state.layout.tab_hits
            );
        }
    }

    #[test]
    fn tab_window_keeps_start_until_active_leaves_it() {
        // Workspace 11, then four tabs of 10 with their separators.
        let costs = [11, 10, 10, 10, 10];
        // All fit: start resets.
        assert_eq!(tab_window(&costs, 51, 3, 4), TabWindow { start: 0, end: 5 });
        // Active 4 off the right edge: minimum scroll, left marker `‹3`.
        assert_eq!(tab_window(&costs, 25, 0, 4), TabWindow { start: 3, end: 5 });
        // Active inside a stored window: no move.
        assert_eq!(tab_window(&costs, 25, 3, 3), TabWindow { start: 3, end: 5 });
        // Active left of the window: start follows it.
        assert_eq!(tab_window(&costs, 25, 3, 2).start, 2);
        // Wider than the space: still the active tab alone.
        assert_eq!(tab_window(&costs, 5, 0, 2), TabWindow { start: 2, end: 3 });
    }

    #[test]
    fn paints_tree_and_graph() {
        let snapshot =
            build_workspace_snapshot(&[repo("app", true), repo("lib", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.graph = Some(GraphModel {
            commits: vec![Commit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: "seed".into(),
                refs: vec!["main".into()],
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
                ..Commit::default()
            }],
            head_id: Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            uncommitted: Some(true),
            ..GraphModel::default()
        });
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("app"), "{text}");
        assert!(text.contains("README.md"), "{text}");
        // A 30-column tree keeps the workspace name and drops the sync part.
        assert!(text.contains("# tmp"), "{text}");
        assert!(text.contains("1 changed"), "{text}");
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
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                super::super::drill::CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                    stat: None,
                },
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                    stat: None,
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
    fn commit_file_row_paints_comment_mark_when_file_has_comments() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.comment_store = put_comment(
            &state.comment_store,
            CommentKey::CommitLine {
                repo: "app".into(),
                sha: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                path: "README.md".into(),
                line: 1,
                end_line: 1,
            },
            "note",
        );
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                super::super::drill::CommitFile {
                    status: "A".into(),
                    path: "src/lib.rs".into(),
                    old_path: None,
                    stat: None,
                },
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "README.md".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        let readme = text
            .lines()
            .find(|line| line.contains("README.md"))
            .unwrap_or("");
        let name_at = readme.find("README.md").expect("README.md");
        let after = &readme[name_at + "README.md".len()..];
        assert!(
            after.contains('"'),
            "commented commit-file should paint ASCII \": {readme:?}"
        );
        let lib = text
            .lines()
            .find(|line| line.contains("lib.rs"))
            .unwrap_or("");
        let lib_at = lib.find("lib.rs").expect("lib.rs");
        assert!(
            !lib[lib_at + "lib.rs".len()..].contains('"'),
            "uncommented commit-file must not paint \": {lib:?}"
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
        assert!(text.contains("search focused pane"), "{text}");
        assert!(text.contains("cycle theme"), "{text}");
        assert!(text.contains("Staged split"), "{text}");
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
    fn help_overlay_paints_last_git_row_at_pty_size() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        let backend = TestBackend::new(140, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        let header = text
            .lines()
            .find(|line| line.contains("MOVE") && line.contains("GIT") && line.contains("VIEW"));
        assert!(
            header.is_some(),
            "help overlay should paint MOVE / GIT / VIEW on one row:\n{text}"
        );
        assert!(
            text.contains("apply/pop/drop"),
            "last GIT wrap must paint at the default PTY size (140×32):\n{text}"
        );
        assert!(text.contains("focused stash"), "{text}");
        assert_help_version_lower_right(&text);
    }

    /// At 140×40 the help takes at most 24 rows and the tree keeps 13
    /// (row-aligned columns left 5), and no column's text runs into the
    /// next column. `overlay_height_grows_when_columns_narrow` holds the
    /// same 24-row bound.
    #[test]
    fn help_columns_keep_a_gutter_and_the_panes_rows() {
        use super::super::help::{help_column_widths, HELP_GROUPS};
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        let mut terminal = Terminal::new(TestBackend::new(140, 40)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        let lines: Vec<&str> = text.lines().collect();
        let header = lines
            .iter()
            .position(|line| line.contains("MOVE") && line.contains("GIT") && line.contains("VIEW"))
            .unwrap_or_else(|| panic!("help header:\n{text}"));
        let footer = lines
            .iter()
            .position(|line| line.contains("/ search help"))
            .unwrap_or_else(|| panic!("help footer:\n{text}"));
        let overlay_rows = footer + 2 - (header - 1);
        assert!(
            overlay_rows <= 24,
            "help takes {overlay_rows} rows:\n{text}"
        );
        assert!(
            state.layout.tree_height >= 13,
            "panes keep {} rows:\n{text}",
            state.layout.tree_height
        );
        assert!(text.contains("quit (press twice)"), "{text}");
        assert!(text.contains("apply/pop/drop"), "{text}");

        // Border + padding put the first column at x = 2.
        let widths = help_column_widths(HELP_GROUPS, help_inner_width(140));
        let mut starts = vec![2usize];
        for width in &widths[..widths.len() - 1] {
            starts.push(starts.last().unwrap() + width);
        }
        let buf = terminal.backend().buffer();
        for y in header + 1..footer {
            for &start in &starts[1..] {
                for x in start - 2..start {
                    assert_eq!(
                        buf[(x as u16, y as u16)].symbol(),
                        " ",
                        "gutter at x={x} y={y}:\n{}",
                        lines[y]
                    );
                }
            }
        }
    }

    /// On a compare tab the overlay paints exactly the rows
    /// `help_status_lines(cols, true)` reserves: the box, title row, the
    /// tallest COMPARE / MOVE / VIEW column, and the footer, with the last
    /// entry of each column on screen.
    #[test]
    fn compare_help_paints_its_reserved_rows() {
        use super::super::help::{help_body_line_count, help_status_lines, HELP_COMPARE_GROUPS};
        for cols in [64u16, 100, 140] {
            let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
            let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
            state
                .tabs
                .open_or_focus("alpha".into(), "main".into(), "HEAD".into());
            assert!(state.is_compare_tab());
            state.help_open = true;
            let mut terminal = Terminal::new(TestBackend::new(cols, 120)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let text = buffer_text(&terminal);
            let lines: Vec<&str> = text.lines().collect();
            let header = lines
                .iter()
                .position(|l| l.contains("MOVE") && l.contains("COMPARE") && l.contains("VIEW"))
                .unwrap_or_else(|| panic!("{cols} cols, compare help header:\n{text}"));
            let top = header - 1;
            assert!(lines[top].starts_with('╭'), "{cols} cols:\n{text}");
            let bottom = (header..lines.len())
                .find(|&y| lines[y].starts_with('╰'))
                .unwrap_or_else(|| panic!("{cols} cols, help bottom border:\n{text}"));
            let reserved = usize::from(help_status_lines(cols, true));
            assert_eq!(bottom + 1 - top, reserved, "{cols} cols:\n{text}");
            let inner = help_inner_width(usize::from(cols));
            let body = help_body_line_count(
                HELP_COMPARE_GROUPS,
                &help_column_widths(HELP_COMPARE_GROUPS, inner),
            );
            let footer_rows = help_idle_footer_lines(inner).len();
            assert_eq!(
                bottom - header - 1,
                body + footer_rows,
                "{cols} cols:\n{text}"
            );
            let last_body = lines[header + body];
            assert!(
                !last_body.trim_matches(|c| c == '│' || c == ' ').is_empty(),
                "{cols} cols: last body row is blank:\n{text}"
            );
            for needle in ["refresh now", "(1=Workspace)", "(press twice)"] {
                assert!(text.contains(needle), "{cols} cols {needle}:\n{text}");
            }
        }
    }

    #[test]
    fn help_search_highlights_without_hiding_rows() {
        for id in crate::tui::theme::THEME_IDS {
            let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
            let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
            state.theme = id;
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

            // A hit paints the filter pill's fg/bg pair on chips and the muted
            // description alike, so the text stays readable on every theme.
            let pill = id.pills().filter;
            let buf = terminal.backend().buffer();
            let lines: Vec<&str> = text.lines().collect();
            let hit_y = lines
                .iter()
                .position(|l| l.contains("quit (press twice)"))
                .expect("quit (press twice) row") as u16;
            let desc = needle_cols(buf, hit_y, "quit (press twice)");
            let chip = needle_cols(buf, hit_y, "Ctrl-c");
            for x in desc.iter().chain(&chip) {
                let cell = &buf[(*x, hit_y)];
                assert_eq!(cell.bg, pill.bg, "{id:?} hit cell {x} bg");
                assert_eq!(cell.fg, pill.fg, "{id:?} hit cell {x} fg");
            }
            assert!(
                buf[(chip[0], hit_y)].modifier.contains(Modifier::BOLD),
                "{id:?} hit chip keeps bold"
            );
            for y in rows_with_bg(buf, pill.bg) {
                for x in cols_with_bg(buf, y, pill.bg) {
                    assert_eq!(buf[(x, y)].fg, pill.fg, "{id:?} ({x},{y}) on pill bg");
                }
            }
            // A non-matching entry keeps its normal paint.
            let miss_y = lines
                .iter()
                .position(|l| l.contains("stage scope"))
                .expect("stage scope row") as u16;
            for x in needle_cols(buf, miss_y, "stage scope") {
                assert_ne!(buf[(x, miss_y)].bg, pill.bg, "{id:?} miss cell {x}");
            }
        }
    }

    /// Draw a whole-file revert confirm over `(path, untracked)` targets and
    /// return the screen text plus its chip row (the line with `cancel`).
    fn draw_revert_confirm(targets: &[(&str, bool)]) -> (String, String) {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.confirm = Some(PendingConfirm::Revert {
            label: targets[0].0.into(),
            targets: targets
                .iter()
                .map(|&(path, untracked)| crate::tui::state::RevertTarget {
                    repo: "app".into(),
                    path: path.into(),
                    untracked,
                    old_path: None,
                })
                .collect(),
        });
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        let chips = text
            .lines()
            .find(|line| line.contains("cancel"))
            .unwrap_or_default()
            .to_string();
        (text, chips)
    }

    #[test]
    fn revert_confirm_offers_only_keys_for_scope() {
        // Tracked only: no untracked line, no `Y` chip.
        let (text, chips) = draw_revert_confirm(&[("README.md", false)]);
        assert!(text.contains("Revert README.md?"), "{text}");
        assert!(text.contains("1 tracked file → discarded"), "{text}");
        assert!(!text.contains("untracked"), "{text}");
        assert!(chips.contains(" y  revert"), "{chips}");
        assert!(!chips.contains(" Y "), "{chips}");
        assert!(chips.contains(" n   Esc  cancel"), "{chips}");

        // One untracked file: `y` deletes it, no tracked line, no `Y` chip.
        let (text, chips) = draw_revert_confirm(&[("scratch.txt", true)]);
        assert!(text.contains("1 untracked file → deleted"), "{text}");
        assert!(!text.contains(" tracked file"), "{text}");
        assert!(!text.contains("discarded"), "{text}");
        assert!(chips.contains(" y  delete"), "{chips}");
        assert!(!chips.contains(" Y "), "{chips}");
        assert!(!chips.contains("revert"), "{chips}");

        // Many untracked files: only `Y` deletes them.
        let (text, chips) = draw_revert_confirm(&[("a.txt", true), ("b.txt", true)]);
        assert!(text.contains("2 untracked files → deleted"), "{text}");
        assert!(!text.contains("discarded"), "{text}");
        assert!(chips.contains(" Y  delete untracked"), "{chips}");
        assert!(!chips.contains(" y "), "{chips}");
        assert!(!chips.contains("revert"), "{chips}");

        // Mixed: both keys, untracked kept by `y`.
        let (text, chips) = draw_revert_confirm(&[("README.md", false), ("tmp.log", true)]);
        assert!(text.contains("1 tracked file → discarded"), "{text}");
        assert!(text.contains("1 untracked file → kept"), "{text}");
        assert!(chips.contains(" y  revert"), "{chips}");
        assert!(chips.contains(" Y  revert + delete untracked"), "{chips}");
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

        state.confirm = Some(PendingConfirm::RevertRange {
            repo: "app".into(),
            path: "README.md".into(),
            patch: String::new(),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("Discard highlighted lines in README.md?"),
            "{text}"
        );
        assert!(text.contains("revert"), "{text}");
        assert!(!text.contains("revert + delete untracked"), "{text}");

        state.confirm = Some(PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Drop"), "{text}");
        assert!(text.contains("stash@{0}"), "{text}");
        assert!(text.contains("drop"), "{text}");
        assert!(text.contains(" n   Esc  cancel"), "{text}");

        state.confirm = Some(PendingConfirm::RemoveWorktree {
            primary: "app".into(),
            path: ".worktrees/topic".into(),
            force: true,
            branch: "topic".into(),
            merged_into_default: Some(false),
            changed: 3,
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Remove worktree"), "{text}");
        assert!(text.contains(".worktrees/topic"), "{text}");
        assert!(text.contains("NOT merged"), "{text}");
        assert!(
            text.contains("3 changed files will be deleted permanently · branch topic is kept"),
            "{text}"
        );
        assert!(!text.contains("--force"), "{text}");

        // Detached: no branch line and nothing about a kept branch.
        state.confirm = Some(PendingConfirm::RemoveWorktree {
            primary: "app".into(),
            path: ".worktrees/topic".into(),
            force: true,
            branch: crate::helpers::DETACHED_HEAD_BRANCH.into(),
            merged_into_default: None,
            changed: 1,
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("1 changed file will be deleted permanently"),
            "{text}"
        );
        assert!(!text.contains("is kept"), "{text}");
        assert!(!text.contains("merge status"), "{text}");

        state.confirm = Some(PendingConfirm::RemoveWorktree {
            primary: "app".into(),
            path: ".worktrees/topic".into(),
            force: false,
            branch: "topic".into(),
            merged_into_default: Some(true),
            changed: 0,
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("clean worktree · branch topic is kept"),
            "{text}"
        );

        state.confirm = Some(PendingConfirm::CheckoutOutOfSync {
            repo: "app".into(),
            branch: "main".into(),
            remote_ref: "origin/main".into(),
            ahead_behind: Some((0, 2)),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("Check out main and fast-forward to origin/main (no fetch)?"),
            "{text}"
        );
        assert!(
            text.contains("local is 0 ahead, 2 behind origin/main"),
            "{text}"
        );
        assert!(!text.contains("cannot fast-forward"), "{text}");
        assert!(!text.contains("pull"), "{text}");

        // Local-only commits: say the fast-forward cannot happen. Unknown counts: no line.
        state.confirm = Some(PendingConfirm::CheckoutOutOfSync {
            repo: "app".into(),
            branch: "main".into(),
            remote_ref: "origin/main".into(),
            ahead_behind: Some((1, 2)),
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("local is 1 ahead, 2 behind origin/main · cannot fast-forward"),
            "{text}"
        );
        state.confirm = Some(PendingConfirm::CheckoutOutOfSync {
            repo: "app".into(),
            branch: "main".into(),
            remote_ref: "origin/main".into(),
            ahead_behind: None,
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(!text.contains("local is"), "{text}");

        state.confirm = Some(PendingConfirm::SwitchToDefault {
            repos: vec!["app".into(), "lib".into(), "web".into()],
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            text.contains("Switch 3 repos to their default branch?"),
            "{text}"
        );
        assert!(text.contains("dirty repos are skipped"), "{text}");
        assert!(text.contains(" y  switch"), "{text}");
        assert!(text.contains(" n   Esc  cancel"), "{text}");

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
    fn compare_revert_confirms_name_the_merge_base_and_deletions() {
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let target = |path: &str, status: &str, old_path: Option<&str>| CompareRevertTarget {
            repo: "app".into(),
            path: path.into(),
            old_path: old_path.map(str::to_string),
            status: status.into(),
            base_ref: "origin/main".into(),
            merge_base: "aaa".into(),
            head: "ccc".into(),
        };
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut paint = |state: &mut AppState, confirm: PendingConfirm| {
            state.confirm = Some(confirm);
            terminal.draw(|frame| draw(frame, state)).unwrap();
            buffer_text(&terminal)
        };

        let text = paint(
            &mut state,
            PendingConfirm::CompareRevertRange {
                target: target("regions.txt", "M", None),
                patch: String::new(),
            },
        );
        assert!(
            text.contains("Revert highlighted lines in regions.txt to the origin/main merge base?"),
            "{text}"
        );
        assert!(text.contains("worktree only"), "{text}");
        assert!(text.contains("revert"), "{text}");
        assert!(!text.contains("delete"), "{text}");

        let text = paint(
            &mut state,
            PendingConfirm::CompareRevertFile {
                target: target("regions.txt", "M", None),
            },
        );
        assert!(
            text.contains("Revert regions.txt to the origin/main merge base?"),
            "{text}"
        );
        assert!(text.contains("worktree only"), "{text}");

        let text = paint(
            &mut state,
            PendingConfirm::CompareRevertFile {
                target: target("summary.txt", "A", None),
            },
        );
        assert!(
            text.contains("Revert summary.txt to the origin/main merge base?"),
            "{text}"
        );
        assert!(
            text.contains("added on HEAD → summary.txt will be deleted"),
            "{text}"
        );
        assert!(text.contains("delete"), "{text}");

        let text = paint(
            &mut state,
            PendingConfirm::CompareRevertFile {
                target: target("gone.txt", "D", None),
            },
        );
        assert!(text.contains("gone.txt will be restored"), "{text}");

        let text = paint(
            &mut state,
            PendingConfirm::CompareRevertFile {
                target: target("new.txt", "R", Some("old.txt")),
            },
        );
        assert!(
            text.contains("new.txt will be deleted, old.txt restored"),
            "{text}"
        );
    }

    #[test]
    fn comment_overlay_paints_caret_and_hides_idle_status() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.comment = Some(CommentPrompt::new(
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 1,
                end_line: 1,
            },
            "hello".into(),
            "app · branch main · README.md:1".into(),
        ));
        state.status = "body: hello".into();
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("Comment"), "{text}");
        assert!(text.contains("hello▏"), "{text}");
        assert!(text.contains("Ctrl-r resolve"), "{text}");
        assert!(!text.contains("Comment · resolved"), "{text}");
        assert!(!text.contains("▏hello"), "{text}");
        assert_eq!(
            text.matches("hello").count(),
            1,
            "typed body must not also echo as status inside the overlay:\n{text}"
        );
        let last = text.lines().last().unwrap_or("");
        assert!(
            !last.contains("? help") && !last.contains("focus right"),
            "idle status must not paint on the last row:\n{last}"
        );
        assert!(
            !text.contains("? help"),
            "idle hint chips must not paint through the comment overlay:\n{text}"
        );

        assert!(
            text.contains("Shift-Enter newline") && text.contains("Ctrl-Left/Right word"),
            "overlay must advertise textarea keys:\n{text}"
        );
        assert!(text.contains("Ctrl-r resolve"), "{text}");
        assert!(!text.contains("Comment · resolved"), "{text}");

        if let Some(prompt) = state.comment.as_mut() {
            prompt.move_home();
        }
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let home = buffer_text(&terminal);
        assert!(home.contains("▏hello"), "{home}");
        assert!(!home.contains("hello▏"), "{home}");

        if let Some(prompt) = state.comment.as_mut() {
            prompt.resolved = true;
        }
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let resolved = buffer_text(&terminal);
        assert!(resolved.contains("Comment · resolved"), "{resolved}");
        assert!(resolved.contains("Ctrl-r unresolve"), "{resolved}");
        assert!(!resolved.contains("Ctrl-r resolve ·"), "{resolved}");
        assert!(
            resolved.contains("▏hello"),
            "resolve toggle must keep the caret:\n{resolved}"
        );

        if let Some(prompt) = state.comment.as_mut() {
            prompt.move_end();
            prompt.insert_newline();
            prompt.insert_char('x');
        }
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let multi = buffer_text(&terminal);
        assert!(multi.contains("hello") && multi.contains("x▏"), "{multi}");
        assert!(!multi.contains("hellox"), "{multi}");
        assert!(
            !multi.contains("? help"),
            "multiline overlay must still occlude idle chips:\n{multi}"
        );
        assert!(
            multi.contains("Comment · resolved") && multi.contains("Ctrl-r unresolve"),
            "multiline overlay must keep resolve chrome:\n{multi}"
        );
    }

    #[test]
    fn row_match_bg_prefers_flash_then_cursor_then_search() {
        let palette = crate::tui::theme::ThemeId::TokyoNight.palette();
        let search_bg = crate::tui::theme::ThemeId::TokyoNight.pills().filter.bg;
        assert_eq!(
            row_match_bg(true, true, true, Some(palette.flash), palette, search_bg),
            Some(palette.flash)
        );
        assert_eq!(
            row_match_bg(true, false, false, Some(palette.flash), palette, search_bg),
            Some(palette.flash)
        );
        assert_eq!(
            row_match_bg(false, false, true, Some(palette.flash), palette, search_bg),
            Some(palette.flash)
        );
        assert_eq!(
            row_match_bg(true, true, true, None, palette, search_bg),
            Some(palette.cursor_bg)
        );
        assert_eq!(
            row_match_bg(true, false, true, None, palette, search_bg),
            Some(palette.cursor_bg_inactive)
        );
        assert_eq!(
            row_match_bg(false, false, true, None, palette, search_bg),
            Some(search_bg)
        );
        assert_eq!(
            row_match_bg(false, false, false, Some(palette.flash), palette, search_bg),
            Some(palette.flash)
        );
        assert_eq!(
            row_match_bg(false, false, false, None, palette, search_bg),
            None
        );
    }

    #[test]
    fn selected_tree_row_keeps_flash_bg() {
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
            true,
            true,
            Some(palette.flash),
            true,
            search_bg_unused(),
            true,
            palette,
            0,
        );
        assert!(
            line.spans.iter().any(|span| span.content == CURSOR_BAR),
            "cursor bar stays on the selected row"
        );
        assert!(
            line.spans
                .iter()
                .any(|span| span.style.bg == Some(palette.flash)),
            "flash bg must paint on the selected row"
        );
        assert!(
            line.spans
                .iter()
                .all(|span| span.style.bg != Some(palette.cursor_bg)),
            "cursor_bg must not hide the flash"
        );
        let search_bg = search_bg_unused().bg;
        assert!(
            line.spans
                .iter()
                .all(|span| span.style.bg != Some(search_bg)),
            "search bg must not hide the flash"
        );
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
            false,
            Some(palette.flash),
            false,
            search_bg_unused(),
            true,
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

    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn paint_row(row: &VisibleRow, width: usize, col_offset: usize) -> String {
        let palette = crate::tui::theme::ThemeId::TokyoNight.palette();
        line_text(&paint_tree_row(
            row,
            width,
            false,
            false,
            None,
            false,
            search_bg_unused(),
            true,
            false,
            false,
            false,
            palette,
            col_offset,
        ))
    }

    /// A narrowed tree keeps the workspace name: the root summary drops its
    /// sync part, then shortens `N changed` to `N`, then goes, before the
    /// name clips.
    #[test]
    fn narrow_workspace_root_row_shortens_summary_before_the_name() {
        const NAME: &str = "demo-workspace";
        let built = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, NAME);
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let mut row = rows
            .iter()
            .find(|r| r.kind == NodeKind::Workspace)
            .expect("workspace row")
            .clone();
        row.chrome.change_count = 5;
        row.chrome.sync_summary = "1 ahead, 1 diverged".into();
        let name_end = 3 + row
            .segments
            .iter()
            .map(|s| visible_width(&s.text))
            .sum::<usize>();
        let full = "5 changed · 1 ahead, 1 diverged";
        for (width, trailing) in [
            (name_end + 1 + full.chars().count(), full),
            (name_end + 1 + full.chars().count() - 1, "5 changed"),
            (name_end + 1 + "5 changed".len(), "5 changed"),
            (name_end + 2, "5"),
            (name_end + 1, ""),
            (name_end, ""),
        ] {
            let text = paint_row(&row, width, 0);
            let name_at = text
                .find(NAME)
                .unwrap_or_else(|| panic!("width {width} keeps the name:\n{text}"));
            assert_eq!(
                text[name_at + NAME.len()..].trim(),
                trailing,
                "width {width}:\n{text}"
            );
        }
        // Narrower than the name: the name clips, no summary competes.
        let text = paint_row(&row, name_end - 4, 0);
        assert!(!text.contains('5'), "{text}");
        assert!(text.contains("demo-work"), "{text}");
    }

    #[test]
    fn narrow_repo_row_clips_name_and_keeps_kind_and_status_icons() {
        const NAME: &str = "services/very-long-repo-name-that-must-clip";
        const TAIL: &str = "must-clip";
        let mut snap = repo(NAME, true);
        snap.sync_status = SyncStatus::Ahead;
        snap.sync_note = "ahead by 2".into();
        let built = build_workspace_snapshot(&[snap], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let row = rows
            .iter()
            .find(|r| r.id == format!("repo:{NAME}"))
            .expect("long repo");
        assert!(
            trailing_has(row, icon_repo(true)),
            "repo glyph must sit in trailing, got {}",
            row.trailing
        );
        assert!(
            row.segments
                .iter()
                .all(|s| s.text.trim() != icon_repo(true)),
            "repo glyph must leave the left run"
        );

        let text = paint_row(row, 28, 0);
        let kind = text.find('@').expect("repo glyph on the narrow row");
        let status = text.find('^').expect("ahead mark on the narrow row");
        assert!(
            !text.contains(TAIL),
            "narrow pane may clip the name tail:\n{text}"
        );
        assert!(
            text.contains("very-long") || text.contains("services"),
            "clipped prefix should remain:\n{text}"
        );
        assert!(
            kind < status,
            "kind glyph sits with status on the right:\n{text}"
        );
        let panned = paint_row(row, 28, 18);
        assert!(
            panned.contains('@') && panned.contains('^'),
            "horizontal pan must not drop trailing kind or status:\n{panned}"
        );
        assert!(
            !panned.contains("very-long"),
            "pan hides the name prefix while indicators stay:\n{panned}"
        );
    }

    #[test]
    fn narrow_linked_checkout_row_keeps_leading_worktree_and_trailing_status() {
        let mut primary = repo("app", true);
        primary.branch = "main".into();
        let mut linked = repo("app/.worktrees/feat", true);
        linked.checkout_kind = CheckoutKind::Linked;
        linked.primary_repo = Some("app".into());
        linked.branch = "feature/very-long-linked-branch-name".into();
        linked.sync_status = SyncStatus::Ahead;
        linked.sync_note = "ahead by 1".into();
        linked.merged_into_default = Some(false);
        let built = build_workspace_snapshot(&[primary, linked], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let row = rows
            .iter()
            .find(|r| r.id == "checkout:app/.worktrees/feat")
            .expect("linked checkout");
        assert!(
            left_has(row, icon_linked_worktree(true)),
            "worktree glyph must lead the name, got {:?}",
            row.segments.first().map(|s| s.text.as_str())
        );
        assert!(
            !trailing_has(row, icon_linked_worktree(true)),
            "worktree glyph must not sit in trailing, got {}",
            row.trailing
        );
        assert!(
            row.trailing.contains('^'),
            "sync mark stays trailing, got {}",
            row.trailing
        );

        let text = paint_row(row, 26, 0);
        let kind = text.find('L').expect("worktree glyph on the narrow row");
        let name = text
            .find("feature")
            .expect("clipped worktree name on the narrow row");
        let status = text.find('^').expect("ahead mark on the narrow row");
        assert!(
            kind < name,
            "worktree glyph stays leading before the name:\n{text}"
        );
        assert!(
            kind < status,
            "leading kind icon stays left of trailing status:\n{text}"
        );
        assert!(
            !text.contains("branch-name"),
            "narrow pane may clip the branch tail:\n{text}"
        );
        let panned = paint_row(row, 26, 18);
        assert!(
            panned.contains('^'),
            "horizontal pan must not drop trailing status:\n{panned}"
        );
        assert!(
            !panned.contains("very-long"),
            "pan hides the name prefix while trailing status stays:\n{panned}"
        );
    }

    #[test]
    fn narrow_file_row_keeps_status_badge_when_name_clips() {
        let mut snap = repo("app", true);
        snap.changes = vec![FileChange {
            path: "src/very-long-file-name-that-must-clip.rs".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        }];
        snap.has_unstaged = true;
        let built = build_workspace_snapshot(&[snap], &[], false, &[]);
        let tree = build_tree(&visible_for_tree(&built), true, "ws");
        let rows = flatten_with(&tree, &HashSet::new(), true);
        let row = rows
            .iter()
            .find(|r| r.id.ends_with("very-long-file-name-that-must-clip.rs"))
            .expect("long file");
        let text = paint_row(row, 24, 0);
        assert!(
            !text.contains("must-clip"),
            "narrow pane may clip the file name:\n{text}"
        );
        assert!(
            text.trim_end().ends_with('M'),
            "status badge stays on the right:\n{text}"
        );
        assert!(
            !text.contains('@') && !text.contains('L'),
            "file rows must not gain repo or worktree glyphs:\n{text}"
        );
    }

    fn left_has(row: &VisibleRow, glyph: &str) -> bool {
        row.segments.iter().any(|s| s.text.trim() == glyph)
    }

    fn trailing_has(row: &VisibleRow, glyph: &str) -> bool {
        row.trailing_segs.iter().any(|s| s.text.trim() == glyph)
    }

    fn search_bg_unused() -> Pill {
        crate::tui::theme::ThemeId::TokyoNight.pills().filter
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
                let col = line[..line.find("a.md").unwrap()].chars().count();
                let cell = &buf[(col as u16, y)];
                a_bg = Some((cell.bg, cell.fg));
            }
            if line.contains("b.md") {
                let col = line[..line.find("b.md").unwrap()].chars().count();
                let cell = &buf[(col as u16, y)];
                b_bg = Some((cell.bg, cell.fg));
            }
        }
        let (a_bg, a_fg) = a_bg.expect("a.md row");
        let (b_bg, b_fg) = b_bg.expect("b.md row");
        assert!(
            a_bg == cursor_bg || b_bg == cursor_bg,
            "one match should keep the cursor: a={a_bg:?} b={b_bg:?}"
        );
        assert!(
            a_bg == search_bg || b_bg == search_bg,
            "the other match should use search bg: a={a_bg:?} b={b_bg:?} search={search_bg:?}"
        );
        assert_ne!(a_bg, b_bg, "cursor and search-match paint must differ");
        let match_fg = if a_bg == search_bg { a_fg } else { b_fg };
        assert_eq!(
            match_fg,
            state.theme.pills().filter.fg,
            "search-match text paints in the filter foreground"
        );
    }

    /// Every theme keeps search-match text readable: the filter foreground
    /// differs from the filter background by a clear luminance step, so
    /// a tree, commit-file, graph, or diff match row stays legible.
    #[test]
    fn search_match_text_contrasts_with_search_bg_in_every_theme() {
        fn luminance(color: Color) -> f64 {
            let Color::Rgb(r, g, b) = color else {
                panic!("theme colours are RGB: {color:?}");
            };
            let lin = |c: u8| {
                let c = f64::from(c) / 255.0;
                if c <= 0.039_28 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * lin(r) + 0.7152 * lin(g) + 0.0722 * lin(b)
        }
        for id in crate::tui::theme::THEME_IDS {
            let pill = id.pills().filter;
            assert_ne!(pill.fg, pill.bg, "{id:?}");
            let (hi, lo) = {
                let (a, b) = (luminance(pill.fg), luminance(pill.bg));
                (a.max(b), a.min(b))
            };
            let ratio = (hi + 0.05) / (lo + 0.05);
            assert!(ratio >= 4.5, "{id:?} search text contrast {ratio:.2} < 4.5");
            // The row paint must use that pill, not the segment colour.
            let palette = id.palette();
            let segs = NodeSegments {
                segments: vec![TextSeg {
                    text: "feature".into(),
                    role: SegRole::BranchFeature,
                    hex: None,
                    bold: false,
                    dim: true,
                }],
                trailing: Vec::new(),
            };
            let line = paint_segmented_row(
                0, false, false, &segs, 20, false, true, None, true, pill, true, palette, 0,
            );
            for span in &line.spans {
                assert_eq!(span.style.bg, Some(pill.bg), "{id:?} {span:?}");
                assert_eq!(span.style.fg, Some(pill.fg), "{id:?} {span:?}");
            }
        }
    }

    #[test]
    fn search_match_paints_filter_bg_on_non_cursor_commit_file_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "a.md".into(),
                    old_path: None,
                    stat: None,
                },
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "b.md".into(),
                    old_path: None,
                    stat: None,
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
    fn compare_tab_paints_diff_pane_while_workspace_drill_is_files() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.open_commit_files(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![super::super::drill::CommitFile {
                status: "M".into(),
                path: "parked-drill.md".into(),
                old_path: None,
                stat: None,
            }],
        );
        assert!(state.drill.is_files());
        // The `1 file in aaa1111` note is status, not the parked subtitle.
        state.status.clear();
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.loading = false;
            tab.path = Some("compare-only.md".into());
            tab.files = vec![super::super::drill::CommitFile {
                status: "M".into(),
                path: "compare-only.md".into(),
                old_path: None,
                stat: None,
            }];
            tab.content = super::super::diff::DiffContent::from_compare_lines(vec![
                "@@ -1,1 +1,1 @@".into(),
                "-old compare".into(),
                "+new compare".into(),
            ]);
        }
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(
            state.drill.is_files(),
            "Workspace drill stays parked: {:?}",
            state.drill
        );
        assert!(
            text.contains("diff"),
            "compare right pane title must be diff:\n{text}"
        );
        assert!(
            text.contains("COMMITTED"),
            "compare right pane must paint DiffPane COMMITTED:\n{text}"
        );
        assert!(
            text.contains("compare-only.md"),
            "compare file list stays on the left:\n{text}"
        );
        assert!(
            !text.contains("parked-drill.md"),
            "parked Files drill must not paint as a second file list:\n{text}"
        );
        assert!(
            !text.contains("aaa1111"),
            "parked commit-detail subtitle must not paint:\n{text}"
        );
    }

    #[test]
    fn search_match_paints_filter_bg_on_compare_commit_file_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        state.focus = FocusPane::Left;
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.files = vec![
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "a.md".into(),
                    old_path: None,
                    stat: None,
                },
                super::super::drill::CommitFile {
                    status: "M".into(),
                    path: "b.md".into(),
                    old_path: None,
                    stat: None,
                },
            ];
        }
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
    fn empty_commit_files_paint_loading_then_no_files_in_this_commit() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let source = super::super::drill::CommitFileSource::Commit {
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        };
        state.begin_commit_files("app".into(), source.clone());
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let loading = buffer_text(&terminal);
        assert!(loading.contains(LOADING_FILES), "{loading}");
        assert!(!loading.contains(NO_MATCHING_ROWS), "{loading}");
        assert!(!loading.contains(NO_FILES_IN_COMMIT), "{loading}");

        state.open_commit_files("app".into(), source, Vec::new());
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let empty = buffer_text(&terminal);
        assert!(empty.contains(NO_FILES_IN_COMMIT), "{empty}");
        assert!(!empty.contains(NO_MATCHING_ROWS), "{empty}");
        assert!(!empty.contains(LOADING_FILES), "{empty}");
    }

    /// A failed `git diff` names git's reason; a focused repo whose graph
    /// is still loading says so instead of asking to focus a repo.
    #[test]
    fn diff_failure_and_graph_load_have_their_own_empty_states() {
        let mut state = two_pane_diff_state();
        state.set_diff(
            "app".into(),
            "README.md".into(),
            super::super::diff::DiffContent {
                error: Some("index file corrupt".into()),
                ..Default::default()
            },
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        assert!(
            text.contains("git diff failed: index file corrupt"),
            "{text}"
        );
        assert!(!text.contains("(no diff)"), "{text}");

        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.cursor = state
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::Repo)
            .expect("repo row");
        assert!(state.graph.is_none());
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        assert!(text.contains(LOADING_GRAPH), "{text}");
        assert!(!text.contains("focus a repo for the graph"), "{text}");
    }

    #[test]
    fn format_line_gutter_reserves_mark_column() {
        let blank_1 = format_line_gutter(Some(1), 2, None, true);
        let marked_1 = format_line_gutter(Some(1), 2, Some(false), true);
        assert_eq!(visible_width(&blank_1), visible_width(&marked_1));
        assert_eq!(&blank_1[1..], " 1");
        assert_eq!(&marked_1[1..], " 1");
        assert_eq!(blank_1.chars().next(), Some(' '));
        assert_eq!(marked_1.chars().next(), Some('"'));

        let blank_12 = format_line_gutter(Some(12), 2, None, true);
        let marked_12 = format_line_gutter(Some(12), 2, Some(false), true);
        assert_eq!(visible_width(&blank_12), visible_width(&marked_12));
        assert_eq!(&blank_12[1..], "12");
        assert_eq!(&marked_12[1..], "12");
        assert_eq!(blank_12, " 12");
        assert_eq!(marked_12, "\"12");

        let resolved_12 = format_line_gutter(Some(12), 2, Some(true), true);
        assert_eq!(visible_width(&resolved_12), visible_width(&marked_12));
        assert_eq!(resolved_12, "'12");

        let empty = format_line_gutter(None, 2, None, true);
        assert_eq!(visible_width(&empty), visible_width(&blank_1));
        assert_eq!(empty, "   ");
    }

    fn number_rule_cols(text: &str) -> Vec<usize> {
        text.lines().filter_map(|line| line.find(" │ ")).collect()
    }

    #[test]
    fn comment_mark_does_not_shift_line_numbers() {
        use crate::tui::comments::{put_comment, CommentKey};

        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
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
                "@@ -10,1 +10,1 @@".into(),
                "-old line".into(),
                "+new line".into(),
            ]),
        );
        let backend = TestBackend::new(120, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let before = buffer_text(&terminal);
        let before_cols = number_rule_cols(&before);
        assert!(
            !before_cols.is_empty(),
            "expected numbered gutter rules before a comment:\n{before}"
        );
        assert!(
            before.contains(" 10 │") && !before.contains("\"10 │"),
            "number column should already include the reserved mark space:\n{before}"
        );

        state.comment_store = put_comment(
            &state.comment_store,
            CommentKey::WorktreeLine {
                repo: "app".into(),
                branch: "main".into(),
                path: "README.md".into(),
                line: 10,
                end_line: 10,
            },
            "keep numbers still",
        );
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let after = buffer_text(&terminal);
        let after_cols = number_rule_cols(&after);
        assert_eq!(
            before_cols, after_cols,
            "comment mark must not shift │ after line numbers:\nbefore={before}\nafter={after}"
        );
        assert!(
            after.contains("\"10 │") && !after.contains(" 10 │"),
            "comment mark should occupy the reserved column:\n{after}"
        );
    }

    #[test]
    fn diff_gutter_uses_muted_without_dim() {
        for id in crate::tui::theme::THEME_IDS {
            let palette = id.palette();
            let style = diff_gutter_style(palette);
            assert_eq!(style.fg, Some(palette.muted), "{id:?}");
            assert!(
                !style.add_modifier.contains(Modifier::DIM),
                "{id:?} line numbers must not DIM"
            );
        }
    }
    fn draw_state(terminal: &mut Terminal<TestBackend>, state: &mut AppState) {
        terminal.draw(|frame| draw(frame, state)).unwrap();
    }

    /// Press, drag, and release with a paint after each event (the live loop).
    fn drag_select(
        terminal: &mut Terminal<TestBackend>,
        state: &mut AppState,
        from: (u16, u16),
        to: (u16, u16),
    ) -> Effect {
        draw_state(terminal, state);
        state.dispatch(Action::Click {
            col: from.0,
            row: from.1,
        });
        draw_state(terminal, state);
        state.dispatch(Action::Drag {
            col: to.0,
            row: to.1,
        });
        draw_state(terminal, state);
        state.dispatch(Action::Release)
    }

    /// Trimmed text of `rows` inside the `x0..x0 + width` column band.
    fn band_text(terminal: &Terminal<TestBackend>, x0: u16, width: u16, rows: Vec<u16>) -> String {
        let buf = terminal.backend().buffer();
        rows.into_iter()
            .map(|y| {
                (x0..x0 + width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn reversed_cells(terminal: &Terminal<TestBackend>) -> HashSet<(u16, u16)> {
        let buf = terminal.backend().buffer();
        let area = buf.area;
        (0..area.height)
            .flat_map(|y| (0..area.width).map(move |x| (x, y)))
            .filter(|&(x, y)| buf[(x, y)].modifier.contains(Modifier::REVERSED))
            .collect()
    }

    #[test]
    fn drag_in_tree_copies_tree_text_only() {
        let mut state = two_pane_diff_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (x, y) = (state.layout.tree_x, state.layout.tree_y);
        let width = state.layout.tree_width;
        let effect = drag_select(&mut terminal, &mut state, (x, y), (110, y + 2));
        let Effect::CopyClipboard { text, announce } = effect else {
            panic!("expected CopyClipboard, got {effect:?}");
        };
        assert!(announce, "drag copy announces");
        let expected = band_text(&terminal, x, width, vec![y, y + 1, y + 2]);
        assert_eq!(text, expected, "{}", buffer_text(&terminal));
        assert!(text.contains("README.md"), "{text}");
        assert!(!text.contains('│'), "no border glyphs: {text}");
        assert!(!text.contains("new line"), "no right-pane text: {text}");
        assert!(!text.contains("UNSTAGED"), "no right-pane text: {text}");
        assert!(state.text_selection.is_none(), "release ends the selection");
    }

    #[test]
    fn drag_in_right_pane_clamps_at_its_left_edge() {
        let mut state = two_pane_diff_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let x = state.layout.diff_content_x;
        let y = state.layout.right_y;
        let width = state.layout.diff_pane_width;
        let last = x + width - 1;
        let effect = drag_select(&mut terminal, &mut state, (last, y + 3), (0, y));
        let Effect::CopyClipboard { text, announce } = effect else {
            panic!("expected CopyClipboard, got {effect:?}");
        };
        assert!(announce);
        let expected = band_text(&terminal, x, width, vec![y, y + 1, y + 2, y + 3]);
        assert_eq!(text, expected, "{}", buffer_text(&terminal));
        assert!(
            text.starts_with("app/README.md"),
            "clamped to the right pane's first column, not the tree: {text}"
        );
    }

    #[test]
    fn plain_click_and_release_does_not_copy() {
        let mut state = two_pane_diff_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let baseline = reversed_cells(&terminal);
        let (x, y) = (state.layout.tree_x + 2, state.layout.tree_y);
        let effect = drag_select(&mut terminal, &mut state, (x, y), (x, y));
        assert_eq!(effect, Effect::None);
        assert!(state.text_selection.is_none());
        draw_state(&mut terminal, &mut state);
        assert_eq!(reversed_cells(&terminal), baseline);
    }

    #[test]
    fn selection_paints_reversed_on_selected_cells_only() {
        let mut state = two_pane_diff_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let baseline = reversed_cells(&terminal);
        let (x, y) = (state.layout.tree_x, state.layout.tree_y);
        let right = x + state.layout.tree_width - 1;
        state.dispatch(Action::Click { col: x + 3, row: y });
        state.dispatch(Action::Drag {
            col: x + 1,
            row: y + 1,
        });
        draw_state(&mut terminal, &mut state);
        let painted: HashSet<(u16, u16)> = reversed_cells(&terminal)
            .difference(&baseline)
            .copied()
            .collect();
        let expected: HashSet<(u16, u16)> = (x + 3..=right)
            .map(|c| (c, y))
            .chain((x..=x + 1).map(|c| (c, y + 1)))
            .collect();
        assert_eq!(painted, expected);
    }

    #[test]
    fn mouse_off_drag_neither_selects_nor_copies() {
        let mut state = two_pane_diff_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        state.dispatch(Action::ToggleMouse);
        assert!(!state.mouse_enabled);
        let (x, y) = (state.layout.tree_x, state.layout.tree_y);
        let baseline = reversed_cells(&terminal);
        let effect = drag_select(&mut terminal, &mut state, (x, y), (x + 5, y + 2));
        assert_eq!(effect, Effect::None);
        assert!(state.text_selection.is_none());
        assert_eq!(reversed_cells(&terminal), baseline);
    }

    #[test]
    fn palette_rows_paint_their_reason_and_skip_empty_chips() {
        use crate::tui::action::{Action, PaletteOpenedBy};
        use crate::tui::tabs::{ONLY_WORKSPACE_TAB_OPEN, WORKSPACE_TAB_CANNOT_CLOSE};
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.dispatch(Action::ToggleCommandPalette(PaletteOpenedBy::CtrlK));
        for c in "tab".chars() {
            state.dispatch(Action::CommandPaletteChar(c));
        }
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        let row = |title: &str| {
            text.lines()
                .find(|line| line.contains(title))
                .unwrap_or_else(|| panic!("no {title} row:\n{text}"))
                .trim_end_matches(['│', ' '])
                .to_string()
        };
        let next = row("Next tab");
        assert!(next.ends_with(ONLY_WORKSPACE_TAB_OPEN), "{next}");
        let close = row("Close tab");
        let after_title = close.split("Close tab").nth(1).unwrap_or_default();
        assert_eq!(
            after_title.trim_start(),
            WORKSPACE_TAB_CANNOT_CLOSE,
            "no empty chip, no group label: {close}"
        );
        let other = row("Other pane");
        assert!(
            other.ends_with("Other pane  Tab"),
            "an enabled row paints no reason: {other}"
        );
        // Rows sit under their painted group header and do not repeat it.
        let lines: Vec<&str> = text.lines().collect();
        let header = |name: &str| {
            lines
                .iter()
                .position(|line| line.trim_matches(['│', ' ']) == name)
                .unwrap_or_else(|| panic!("no {name} header:\n{text}"))
        };
        let at = |title: &str| lines.iter().position(|l| l.contains(title)).unwrap();
        assert!(header("MOVE") < at("Other pane"), "{text}");
        assert!(header("GIT") < at("Close tab"), "{text}");
        for title in ["Next tab", "Close tab", "Other pane"] {
            let line = row(title);
            assert!(
                !line.contains("  MOVE") && !line.contains("  GIT") && !line.contains("  VIEW"),
                "{title} repeats its group: {line}"
            );
        }
    }

    /// A palette window scrolled into the middle of a group lost that
    /// group's header, so its top rows keep the group label.
    #[test]
    fn palette_rows_keep_their_group_when_the_header_scrolled_off() {
        use crate::tui::action::{Action, PaletteOpenedBy};
        const GROUPS: [&str; 4] = ["HIGHLIGHT", "MOVE", "GIT", "VIEW"];
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.dispatch(Action::ToggleCommandPalette(PaletteOpenedBy::CtrlK));
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        for _ in 0..60 {
            state.dispatch(Action::CommandPaletteMove(1));
            draw_state(&mut terminal, &mut state);
            let text = buffer_text(&terminal);
            let lines: Vec<&str> = text.lines().collect();
            let prompt = lines
                .iter()
                .position(|line| line.contains("Ctrl-k …"))
                .unwrap_or_else(|| panic!("no palette prompt:\n{text}"));
            let first = lines[prompt + 1];
            if GROUPS.contains(&first.trim_matches(['│', ' '])) {
                continue;
            }
            assert!(
                GROUPS.iter().any(|g| first.contains(&format!("  {g}"))),
                "a row whose header scrolled off keeps its group: {first}\n{text}"
            );
            return;
        }
        panic!("the palette window never started inside a group");
    }

    #[test]
    fn compare_commit_picker_paints_short_sha_and_subject_rows() {
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.open_compare_commit_picker(
            "app".into(),
            vec![crate::git::AncestorCommit {
                id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                subject: "Initial import".into(),
            }],
        );
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        assert!(text.contains("Compare vs commit app"), "{text}");
        assert!(text.contains("❯   aaa1111  Initial import"), "{text}");
        assert!(
            !text.contains("aaa1111b"),
            "rows paint the short sha:\n{text}"
        );

        state
            .compare_picker
            .as_mut()
            .unwrap()
            .set_filter("zz".into());
        draw_state(&mut terminal, &mut state);
        assert!(buffer_text(&terminal).contains("no commit matches zz"));
        state.open_compare_commit_picker("app".into(), Vec::new());
        draw_state(&mut terminal, &mut state);
        assert!(buffer_text(&terminal).contains("No commits to compare"));
    }

    #[test]
    fn compare_commit_picker_cuts_long_subjects_to_one_line_per_row() {
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let long = "word ".repeat(40);
        let commits: Vec<_> = (0..5)
            .map(|i| crate::git::AncestorCommit {
                id: format!("{i}{}", "a".repeat(39)),
                subject: format!("{i} {long}tail"),
            })
            .collect();
        state.open_compare_commit_picker("app".into(), commits);
        state.compare_picker.as_mut().unwrap().move_cursor(4);
        let mut terminal = Terminal::new(TestBackend::new(80, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        assert!(!text.contains("tail"), "subjects are cut:\n{text}");
        for i in 0..5 {
            let short = format!("{i}aaaaaa  {i} word");
            let rows: Vec<_> = text.lines().filter(|l| l.contains(&short)).collect();
            assert_eq!(rows.len(), 1, "row {i} paints once:\n{text}");
            assert!(
                rows[0].contains('…'),
                "row {i} ends in an ellipsis:\n{text}"
            );
        }
        assert_eq!(
            text.lines().filter(|l| l.contains("word")).count(),
            5,
            "no row wraps onto a second line:\n{text}"
        );
        assert!(
            text.contains("❯   4aaaaaa  4 word"),
            "the selected row stays visible:\n{text}"
        );
        assert!(
            text.contains("Enter compare"),
            "footer stays in the box:\n{text}"
        );
    }

    #[test]
    fn branch_picker_paints_the_create_row_last() {
        use crate::git::LocalBranch;
        use crate::tui::branches::BranchPickerState;
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let branch = |name: &str| LocalBranch {
            name: name.into(),
            current: false,
            authordate: 0,
        };
        let mut picker = BranchPickerState::checkout("app".into(), vec![branch("topic/a")]);
        picker.set_filter("topic".into());
        state.branch_picker = Some(picker);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        let topic = text.find("❯   topic/a").expect("cursor on the branch row");
        let create = text.find("+ create branch topic").expect("create row");
        assert!(
            topic < create,
            "create row comes after the branches:\n{text}"
        );
        assert!(text.contains("Enter checkout"), "{text}");
        state.branch_picker.as_mut().unwrap().move_cursor(1);
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        assert!(
            text.contains("❯   + create branch topic"),
            "the cursor lands on the create row:\n{text}"
        );
        assert!(
            text.contains("Enter create and check out") && !text.contains("Enter checkout"),
            "the footer names the create on its row:\n{text}"
        );

        let mut graph = BranchPickerState::from_names(
            "app".into(),
            vec!["main".into()],
            Some("aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
            vec!["main".into()],
        );
        graph.set_filter("new".into());
        state.branch_picker = Some(graph);
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        assert!(
            text.contains("+ create branch new at aaa1111"),
            "graph picker names the commit:\n{text}"
        );
        assert!(
            text.contains("Enter create at aaa1111") && !text.contains("Enter checkout"),
            "graph create row creates at the commit:\n{text}"
        );
        assert!(!text.contains("No matching branches"), "{text}");
    }

    /// Hidden body line of the `src/a.rs` diff in the folder-summary tests.
    const SUMMARY_DIFF_BODY: &str = "summary-hidden-body";

    /// `README.md`, then `src/` with `a.rs` (+3 −1), `b.rs` (+2 −0), and
    /// `deep/c.rs` (binary: no counts).
    fn summary_files() -> Vec<super::super::drill::CommitFile> {
        [
            ("README.md", Some((1, 1))),
            ("src/a.rs", Some((3, 1))),
            ("src/b.rs", Some((2, 0))),
            ("src/deep/c.rs", None),
        ]
        .into_iter()
        .map(|(path, stat)| super::super::drill::CommitFile {
            status: "M".into(),
            path: path.into(),
            old_path: None,
            stat: stat.map(|(added, deleted)| crate::git::LineStat { added, deleted }),
        })
        .collect()
    }

    /// Move the focused file list onto the row for `path`.
    fn summary_move_to(state: &mut AppState, path: &str) -> Effect {
        let row = state
            .commit_file_rows()
            .iter()
            .position(|row| row.path == path)
            .unwrap_or_else(|| panic!("row {path}"));
        let delta = row as i32 - state.commit_files_cursor() as i32;
        state.dispatch(Action::Move(delta))
    }

    /// Depth-2 commit drill on `files` with the `src/a.rs` diff open and the
    /// file list focused on it.
    fn summary_drill_state(files: Vec<super::super::drill::CommitFile>) -> AppState {
        let mut state = two_pane_files_state();
        state.ascii = false;
        state.open_commit_diff(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files,
            0,
            "src/a.rs".into(),
            super::super::diff::DiffContent::from_lines(vec![
                "@@ -1,1 +1,1 @@".into(),
                "-old line".into(),
                format!("+{SUMMARY_DIFF_BODY}"),
            ]),
        );
        state.focus = FocusPane::Left;
        let _ = summary_move_to(&mut state, "src/a.rs");
        state
    }

    /// Compare tab on `files` with the `src/a.rs` diff loaded and the file
    /// list focused on it.
    fn summary_compare_state(files: Vec<super::super::drill::CommitFile>) -> AppState {
        let mut state = compare_json_over_stale_workspace_state();
        state.ascii = false;
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.files = files;
            tab.path = Some("src/a.rs".into());
            tab.content = super::super::diff::DiffContent::from_compare_lines(vec![
                "diff --git a/src/a.rs b/src/a.rs".into(),
                "--- a/src/a.rs".into(),
                "+++ b/src/a.rs".into(),
                "@@ -1,1 +1,1 @@".into(),
                "-old line".into(),
                format!("+{SUMMARY_DIFF_BODY}"),
            ]);
        }
        state.focus = FocusPane::Left;
        assert_eq!(summary_move_to(&mut state, "src/a.rs"), Effect::None);
        state
    }

    /// Right-pane text (inside the border) of the last frame.
    fn summary_pane_text(terminal: &Terminal<TestBackend>, state: &AppState) -> String {
        let text = buffer_text(terminal);
        let x0 = state.layout.diff_content_x as usize;
        let w = state.layout.diff_pane_width as usize;
        let y0 = state.layout.right_y as usize;
        let h = state.layout.diff_pane_height as usize;
        text.lines()
            .skip(y0)
            .take(h)
            .map(|line| line.chars().skip(x0).take(w).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn summary_status_line(terminal: &Terminal<TestBackend>) -> String {
        buffer_text(terminal)
            .lines()
            .last()
            .unwrap_or_default()
            .to_string()
    }

    /// File → folder → same file on a depth-2 drill and a compare tab: the
    /// folder paints its summary in place of the kept diff, and the file
    /// repaints its diff.
    #[test]
    fn folder_row_paints_summary_and_file_row_repaints_diff() {
        for (name, mut state) in [
            ("drill", summary_drill_state(summary_files())),
            ("compare", summary_compare_state(summary_files())),
        ] {
            // Wide enough for a split diff, so the file view has a rule.
            let mut terminal = Terminal::new(TestBackend::new(220, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            let pane = summary_pane_text(&terminal, &state);
            assert!(
                pane.contains(SUMMARY_DIFF_BODY),
                "{name} file diff:\n{pane}"
            );
            assert!(
                state.layout.diff_split_rule_x.is_some(),
                "{name} split rule on a file"
            );
            let pill = crate::tui::chrome::diff_pill_label(state.diff_mode, state.diff_layout());
            assert!(
                summary_status_line(&terminal).contains(pill),
                "{name} diff pill on a file"
            );

            assert_eq!(summary_move_to(&mut state, "src"), Effect::None, "{name}");
            draw_state(&mut terminal, &mut state);
            let pane = summary_pane_text(&terminal, &state);
            let lines: Vec<&str> = pane.lines().collect();
            assert!(
                lines[0].starts_with("src/  3 files · +5 −1"),
                "{name} header:\n{pane}"
            );
            for (file, counts) in [
                ("a.rs", Some("+3 −1")),
                ("b.rs", Some("+2 −0")),
                ("c.rs", None),
            ] {
                let row = lines
                    .iter()
                    .find(|line| line.contains(file))
                    .unwrap_or_else(|| panic!("{name} row {file}:\n{pane}"));
                assert!(row.trim_end().ends_with('M'), "{name} badge: {row}");
                match counts {
                    Some(counts) => assert!(row.contains(counts), "{name} counts: {row}"),
                    None => assert!(
                        !row.contains('+') && !row.contains('−'),
                        "{name} no counts without a stat: {row}"
                    ),
                }
            }
            assert!(!pane.contains("README.md"), "{name} sibling file:\n{pane}");
            assert!(
                !pane.contains(SUMMARY_DIFF_BODY),
                "{name} hidden diff:\n{pane}"
            );
            assert!(!pane.contains("No diff"), "{name}:\n{pane}");
            assert!(
                !summary_status_line(&terminal).contains(pill),
                "{name} no diff pill over a summary"
            );
            assert_eq!(state.layout.diff_split_rule_x, None, "{name} no split rule");

            assert_eq!(
                summary_move_to(&mut state, "src/a.rs"),
                Effect::None,
                "{name}"
            );
            draw_state(&mut terminal, &mut state);
            let pane = summary_pane_text(&terminal, &state);
            assert!(
                pane.contains(SUMMARY_DIFF_BODY),
                "{name} diff back:\n{pane}"
            );
            assert!(!pane.contains("3 files"), "{name} summary gone:\n{pane}");
        }
    }

    /// More files than rows: the last row folds the rest into `… N more`.
    #[test]
    fn folder_summary_overflow_ends_with_more_row() {
        let mut files = summary_files();
        files.extend((0..40).map(|i| super::super::drill::CommitFile {
            status: "A".into(),
            path: format!("src/many/f{i:02}.rs"),
            old_path: None,
            stat: Some(crate::git::LineStat {
                added: 1,
                deleted: 0,
            }),
        }));
        for (name, mut state) in [
            ("drill", summary_drill_state(files.clone())),
            ("compare", summary_compare_state(files.clone())),
        ] {
            let _ = summary_move_to(&mut state, "src");
            let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            let pane = summary_pane_text(&terminal, &state);
            let lines: Vec<&str> = pane.lines().collect();
            assert!(
                lines[0].starts_with("src/  43 files · +45 −1"),
                "{name}:\n{pane}"
            );
            // One header row; the last body row is the overflow row.
            let shown = lines.len() - 2;
            let more = format!("… {} more", 43 - shown);
            assert!(
                lines.last().unwrap().contains(&more),
                "{name} expected {more}:\n{pane}"
            );
            assert!(lines[1].contains("a.rs"), "{name} sorted by path:\n{pane}");
        }
    }

    /// No file with a stat: no totals in the header and no counts on rows.
    /// ASCII mode paints a plain `-` for deletions.
    #[test]
    fn folder_summary_counts_follow_stats_and_ascii() {
        let mut state = summary_drill_state(summary_files());
        let _ = summary_move_to(&mut state, "src/deep");
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let pane = summary_pane_text(&terminal, &state);
        let header = pane.lines().next().unwrap();
        assert_eq!(header.trim_end(), "src/deep/  1 file", "{pane}");
        assert!(!pane.contains('+') && !pane.contains('−'), "{pane}");

        let mut state = summary_drill_state(summary_files());
        state.ascii = true;
        let _ = summary_move_to(&mut state, "src");
        draw_state(&mut terminal, &mut state);
        let pane = summary_pane_text(&terminal, &state);
        assert!(pane.starts_with("src/  3 files · +5 -1"), "{pane}");
        assert!(pane.contains("+3 -1"), "{pane}");
        assert!(!pane.contains('−'), "{pane}");
    }

    /// A compare reload that keeps a folder focused drops the old diff
    /// (`path` is `None`); the summary still wins over the empty copy.
    #[test]
    fn compare_folder_summary_wins_over_no_committed_changes() {
        let mut state = summary_compare_state(summary_files());
        let _ = summary_move_to(&mut state, "src");
        {
            let tab = state.tabs.active_compare_mut().unwrap();
            tab.path = None;
            tab.content = super::super::diff::DiffContent::default();
        }
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let pane = summary_pane_text(&terminal, &state);
        assert!(pane.starts_with("src/  3 files"), "{pane}");
        assert!(!pane.contains("No committed changes"), "{pane}");
    }
}

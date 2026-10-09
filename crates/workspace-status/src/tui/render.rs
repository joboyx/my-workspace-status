//! Ratatui paint for the tree, graph / diff pane, status, and help overlay.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, StatefulWidget, Wrap,
};
use ratatui::Frame;
use unicode_width::UnicodeWidthStr;
use workspace_status_graph::{
    footer_message_scroll_max, graph_col_max, graph_hscroll_visible, graph_vscroll_visible,
    painted_line_count, short_id, GraphIconKind, GraphLabelPalette, GraphRow, GraphWidget,
    FOOTER_RULE_ROWS,
};

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;

use super::chrome::{
    breadcrumb_line, breadcrumb_rows, ctrl_c_prompt_line, ctrl_c_prompt_rows, dialog_height,
    dialog_rect, dialog_width, export_shows_status, help_tab, open_dialog, status_line, DialogKind,
    LIST_OVERLAY_MAX_ROWS,
};
use super::command_palette::{CommandPaletteState, PalettePaintRow};
use super::comments::{
    comment_overlay_footer_save, graph_row_comments_resolved, graph_row_has_comment, CommentPrompt,
    COMMENT_OVERLAY_FOOTER_EDIT,
};
use super::commit_files::FolderSummary;
use super::diff::{
    cell_code_width, cell_sign, diff_failed_text, diff_pane_header, diff_pane_header_rows,
    diff_pane_mode_label, diff_row_content_width, diff_wrap_row_heights, gutter_width,
    section_header, wrap_viewport_start, DiffCell, DiffCellKind, DiffRow, DiffSection, DIFF_RULE,
};
use super::drill::DrillView;
use super::explorer::{ExplorerRow, ExplorerStatus};
use super::gates::ListFocusTarget;
use super::help::{
    attach_help_version, help_chip_gap_spaces, help_column_content_width, help_column_widths,
    help_entry_matches, help_entry_visual_lines, help_groups, help_idle_footer,
    help_idle_footer_lines, help_inner_width, help_key_width, help_legend_glyph_width,
    help_legend_layout, help_legend_matches, help_legend_name_width, help_legend_visual_lines,
    help_version_label, wrap_help_footer, HELP_LEGEND_HEAD_ROWS, HELP_LEGEND_TITLE,
    HELP_SEARCH_ESC_HINT,
};
use super::icons::{
    comment_mark_cols, glyph, icon_branch, icon_comment, icon_comment_resolved, icon_diff,
    icon_merged_into_default, icon_move, icon_open_vs_default, truncate_visible, IconKind,
    IconSpec, CURSOR_BAR, CURSOR_BAR_INACTIVE, FOLD_COLLAPSED, FOLD_COLLAPSED_ASCII, FOLD_EXPANDED,
    FOLD_EXPANDED_ASCII,
};
use super::line_blame::{fit_annotation, BlameSide, BLAME_MENU_ROWS};
use super::ops::RevertScope;
use super::popover::{
    flat_lines, focused_line, graph_icon, graph_row_id, graph_sync_kind, popover_rect,
    tree_icon_target, IconTarget, PopoverLine, POPOVER_FOOTER, POPOVER_MAX_WIDTH,
};
use super::pull_request::PrState;
use super::quick_open::{
    files_row_shows_status, FileIndexState, QuickOpenMode, QuickOpenScope, QuickOpenState,
};
use super::search::{
    collect_commit_file_match_indices, collect_graph_match_indices, collect_match_ids, slice_cols,
    wrap_col_starts, wrap_col_starts_capped, wrap_cols, SearchPane,
};
use super::search_files::{
    search_row_shows_status, SearchFilesState, SearchRow, SearchZone, SEARCH_PREVIEW_MIN_COLS,
};
use super::split::{
    diff_paint_width, diff_split_rule_x, pane_widths, side_by_side_column_widths, DiffMode,
    MIN_PANE_COLS, MIN_TERM_COLS, MIN_TERM_ROWS,
};
use super::state::{
    revert_scope, AppState, CompareRevertTarget, FocusPane, IconHit, PendingConfirm, PopoverHit,
};
use super::syntax::{
    cached_highlight_diff_rows, highlight_file_window, readable_fg, slice_styled_cols,
    CachedDiffSyntax, CodeSpan, DiffBackgrounds, DiffSyntaxKey,
};
use super::tabs::{
    compare_picker_empty, file_gutter_width, file_too_large, ComparePickerState, ExplorerPreview,
    FileTab, EXPLORER_FOLDER_HINT, FILE_IS_BINARY,
};
use super::theme::{hex_color, BackgroundMode, Palette, Pill, ThemeId};
use super::tree::{
    file_change_from_name_status, file_change_segments, painted_row_segments, pr_badge_kind,
    pr_badge_mark, visible_window, workspace_trailing_fit, NodeKind, NodeSegments, SegRole,
    TextSeg, VisibleRow,
};
use crate::file_index::{FileIndex, FileRead};
use crate::file_search::{SearchHit, SearchOptions};
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

/// Pane title: the plain pane name for focused and unfocused.
///
/// Names are exactly `tree`, `graph`, `files`, or `diff`. Focus is the
/// border colour (boxed) or the accent row ([`flat_accent_line`]), never a
/// title glyph or space-pad. Boxed title text uses `palette.heading` via
/// `title_style` so it does not inherit `border_style`.
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
    // Every frame starts with no icon; only a pane that paints one records
    // it again, so a hidden badge never opens a PR or a popover.
    state.layout.icon_hits.clear();
    state.layout.popover = None;
    let area = frame.area();
    state.too_small = area.width < MIN_TERM_COLS || area.height < MIN_TERM_ROWS;
    if state.too_small {
        draw_too_small(frame, area, state);
        return;
    }
    // Dialogs paint over the panes (see the end of this fn), so no open
    // overlay changes these rows.
    let crumb_h = breadcrumb_rows(state);
    let prompt_h = ctrl_c_prompt_rows(state);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(crumb_h),
            Constraint::Length(prompt_h),
            Constraint::Length(1),
        ])
        .split(area);
    let palette = state.theme.palette();
    let painted = state.background == BackgroundMode::Paint;
    if painted {
        for row in [chunks[0], chunks[2], chunks[3], chunks[4]] {
            fill_bg(frame, row, palette.chrome);
        }
    }
    draw_tab_strip(frame, chunks[0], state);
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
    state.layout.file_view_row_lines.clear();
    state.layout.explorer_tree = Rect::default();
    state.layout.explorer_preview = Rect::default();
    state.layout.term_cols = area.width;
    state.layout.pane_height = chunks[1].height;
    if state.is_file_tab() {
        draw_file_tab(frame, chunks[1], state);
    } else if state.is_explorer_tab() {
        draw_explorer_tab(frame, chunks[1], state);
    } else {
        draw_panes(frame, chunks[1], state);
    }

    frame.render_widget(
        Paragraph::new(breadcrumb_line(state, chunks[2].width)),
        chunks[2],
    );
    if prompt_h > 0 {
        frame.render_widget(
            Paragraph::new(ctrl_c_prompt_line(state, chunks[3].width)),
            chunks[3],
        );
    }
    frame.render_widget(
        Paragraph::new(status_line(state, chunks[4].width)),
        chunks[4],
    );

    // Dialogs paint last: a fixed-height box centered over the panes.
    state.layout.help_scroll_max = 0;
    if let Some(kind) = open_dialog(state) {
        let width = dialog_width(chunks[1], kind);
        let rect = dialog_rect(chunks[1], width, dialog_height(state, kind, width));
        match kind {
            DialogKind::Help => {
                let max = draw_help(frame, rect, state);
                state.layout.help_scroll_max = max;
                // A resize can lower the max: keep the next `k` live.
                state.help_scroll = state.help_scroll.min(max);
            }
            DialogKind::Confirm => draw_confirm(frame, rect, state),
            DialogKind::StashMenu => draw_stash_menu(frame, rect, state),
            DialogKind::BlameMenu => draw_blame_menu(frame, rect, state),
            DialogKind::CreateBranch => draw_create_branch(frame, rect, state),
            DialogKind::Comment => draw_comment(frame, rect, state),
            DialogKind::CommentExport => draw_comment_export(frame, rect, state),
            DialogKind::BranchPicker => draw_branch_picker(frame, rect, state),
            DialogKind::ComparePicker => draw_compare_picker(frame, rect, state),
            DialogKind::GraphFocusPicker => draw_graph_focus_picker(frame, rect, state),
            DialogKind::QuickOpen => draw_quick_open(frame, rect, state),
            DialogKind::SearchFiles => draw_search_files(frame, rect, state),
        }
    }

    // The icon popover hangs over the panes and any dialog.
    draw_popover(frame, chunks[1], state);

    // Keep the frame as painted so a mouse release copies what is on screen,
    // then reverse the selected cells on top of it.
    state.painted_frame = frame.buffer_mut().clone();
    if let Some(selection) = state.text_selection.as_ref() {
        selection.highlight(frame.buffer_mut());
    }
}

/// Tree / graph / files pane on the left and graph / files / diff on the
/// right (Workspace and compare tabs), plus their layout for mouse hits.
fn draw_panes(frame: &mut Frame<'_>, pane_area: Rect, state: &mut AppState) {
    let widths = pane_widths(pane_area.width, state.tree_fraction);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(widths.tree_width),
            Constraint::Min(MIN_PANE_COLS),
        ])
        .split(pane_area);

    let left_is_files = state.drill.is_diff() || state.is_compare_tab();
    let left_is_graph = !state.is_compare_tab() && state.drill.is_files();
    let left_name = state.left_pane_title();
    let palette = state.theme.palette();
    let tree_inner = draw_pane_chrome(
        frame,
        panes[0],
        left_name,
        state.focus == FocusPane::Left,
        palette.sidebar,
        state,
    );
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
    let right_inner = draw_pane_chrome(
        frame,
        panes[1],
        right_name,
        state.focus == FocusPane::Right,
        palette.surface,
        state,
    );
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    draw_right(frame, right_inner, state);

    state.layout.tree_x = tree_inner.x;
    state.layout.tree_y = tree_inner.y;
    state.layout.tree_width = tree_inner.width;
    state.layout.tree_height = tree_inner.height;
    state.layout.right_x = panes[1].x;
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
        Some(diff_split_rule_x(right_inner.x, split.left_width))
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
        let footer_h =
            commit_detail_footer_height(state.commit_detail_footer_request(), tree_inner.height);
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
}

/// File tab body copy while it loads.
const LOADING_FILE: &str = "loading…";
/// Explorer tree copy for a checkout with no entries.
const EMPTY_FOLDER: &str = "empty folder";
/// Explorer badge of an untracked file (the Workspace tree says `A`).
const UNTRACKED_BADGE: &str = "??";
/// File tab body for an empty file.
const EMPTY_FILE: &str = "empty file";

/// Syntax spans of one file-tab window: tab id, load generation, theme,
/// first and end line.
type FileSyntaxKey = (u64, u64, ThemeId, usize, usize);

/// Spans per painted file-tab line, shared between the cache and paint.
type FileSyntaxSpans = Arc<Vec<Vec<CodeSpan>>>;

thread_local! {
    static FILE_SYNTAX_CACHE: RefCell<Option<(FileSyntaxKey, FileSyntaxSpans)>> =
        const { RefCell::new(None) };
}

/// Syntax spans for `lines[window]` of `tab`, reused while the window,
/// load, and theme stay the same.
fn file_window_spans(
    tab: &FileTab,
    lines: &[String],
    theme: ThemeId,
    fallback: Color,
    window: std::ops::Range<usize>,
) -> FileSyntaxSpans {
    let key = (tab.id, tab.generation, theme, window.start, window.end);
    FILE_SYNTAX_CACHE.with(|slot| {
        let mut cache = slot.borrow_mut();
        if let Some((hit_key, spans)) = cache.as_ref() {
            if *hit_key == key {
                return Arc::clone(spans);
            }
        }
        let spans = Arc::new(highlight_file_window(
            &tab.rel, lines, theme, fallback, window,
        ));
        *cache = Some((key, Arc::clone(&spans)));
        spans
    })
}

/// First line to paint so `cursor` shows in `height` rows.
///
/// Keeps `scroll` when the cursor already shows. `rows_of(line)` is the
/// painted row count of a line (more than 1 only while wrap is on); the
/// walk reads at most one screen of lines, never the whole file.
fn file_view_scroll(
    scroll: usize,
    cursor: usize,
    height: usize,
    rows_of: impl Fn(usize) -> usize,
) -> usize {
    let height = height.max(1);
    if cursor < scroll {
        return cursor;
    }
    let mut used = 0usize;
    for line in scroll..=cursor {
        used += rows_of(line);
        if used > height {
            break;
        }
    }
    if used <= height {
        return scroll;
    }
    let mut top = cursor;
    let mut used = rows_of(cursor);
    while top > 0 {
        let above = rows_of(top - 1);
        if used + above > height {
            break;
        }
        used += above;
        top -= 1;
    }
    top
}

/// The active file tab: one pane over the full width (boxed or flat, see
/// [`draw_pane_chrome`]), titled with `<checkout>/<rel>` (snapshot `repo`
/// path), with line numbers and highlighted code.
fn draw_file_tab(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let palette = state.theme.palette();
    let Some(tab) = state.tabs.active_file() else {
        return;
    };
    let inner = draw_pane_chrome(frame, area, &tab.display, true, palette.surface, state);
    draw_file_body(frame, inner, state);
}

/// The active read-only file body (a file tab, or an Explorer tab's
/// clean-file preview) inside `inner`: line numbers and highlighted code,
/// or a loading / binary / too-large / empty notice.
fn draw_file_body(frame: &mut Frame<'_>, inner: Rect, state: &mut AppState) {
    let palette = state.theme.palette();
    state.layout.file_view_x = inner.x;
    state.layout.file_view_y = inner.y;
    state.layout.file_view_width = inner.width;
    state.layout.file_view_height = inner.height;
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let Some(tab) = state.tabs.active_file_view() else {
        return;
    };
    let notice = match tab.body.as_deref() {
        None => Some((LOADING_FILE.to_string(), palette.muted)),
        Some(FileRead::Binary) => Some((FILE_IS_BINARY.to_string(), palette.muted)),
        Some(FileRead::TooLarge { bytes }) => Some((file_too_large(*bytes), palette.muted)),
        Some(FileRead::Failed(err)) => Some((err.clone(), palette.deleted)),
        Some(FileRead::Text { lines, .. })
            if lines.is_empty() || (lines.len() == 1 && lines[0].is_empty()) =>
        {
            Some((EMPTY_FILE.to_string(), palette.muted))
        }
        Some(FileRead::Text { .. }) => None,
    };
    if let Some((text, color)) = notice {
        frame.render_widget(
            Paragraph::new(Span::styled(text, Style::default().fg(color))),
            inner,
        );
        return;
    }
    let lines = tab.lines();
    let height = inner.height as usize;
    let gutter = file_gutter_width(lines.len());
    let code_w = (inner.width as usize).saturating_sub(gutter).max(1);
    let wrap = state.diff_wrap;
    // A row count past the view height is never needed, so a 2 MiB
    // single-line file scans at most one screen of its text per call.
    let row_cap = height + 1;
    let rows_of = |line: usize| {
        if wrap {
            wrap_col_starts_capped(&lines[line], code_w, row_cap).len()
        } else {
            1
        }
    };
    let scroll = file_view_scroll(tab.scroll, tab.cursor, height, rows_of);
    let mut end = scroll;
    let mut used = 0usize;
    while end < lines.len() && used < height {
        used += rows_of(end);
        end += 1;
    }
    let spans = file_window_spans(tab, lines, state.theme, palette.repo, scroll..end);
    let hits = if state.search_target == SearchPane::File {
        state.file_search_hits(&state.search_query)
    } else {
        std::rc::Rc::from([])
    };
    let search = state.theme.pills().filter;
    let gutter_style = diff_gutter_style(palette);
    let col_offset = if wrap { 0 } else { tab.col_offset as usize };
    let blame = state.painted_line_annotation();
    let mut painted: Vec<Line> = Vec::with_capacity(height);
    let mut row_lines = Vec::with_capacity(height);
    for (line, text) in lines.iter().enumerate().take(end).skip(scroll) {
        let selected = line == tab.cursor;
        let hit = hits.binary_search(&line).is_ok();
        let bg = row_match_bg(selected, true, hit, None, palette, search.bg);
        let match_fg = (hit && !selected).then_some(search.fg);
        let code = spans.get(line - scroll).map(Vec::as_slice).unwrap_or(&[]);
        let starts = if wrap {
            wrap_col_starts_capped(text, code_w, row_cap)
        } else {
            vec![col_offset]
        };
        for (part, &start) in starts.iter().enumerate() {
            if painted.len() >= height {
                break;
            }
            let number = if part == 0 {
                format!("{:>width$} ", line + 1, width = gutter - 1)
            } else {
                " ".repeat(gutter)
            };
            let with_bg = |style: Style| bg.map_or(style, |bg| style.bg(bg));
            let mut row = vec![Span::styled(number, with_bg(gutter_style))];
            let mut used = 0usize;
            for span in slice_styled_cols(code, start, code_w) {
                used += visible_width(&span.text);
                let fg = match_fg.unwrap_or(span.fg);
                row.push(Span::styled(span.text, with_bg(Style::default().fg(fg))));
            }
            if bg.is_some() && used < code_w {
                row.push(Span::styled(
                    " ".repeat(code_w - used),
                    with_bg(Style::default()),
                ));
                if selected && part + 1 == starts.len() {
                    if let Some((text, _)) = &blame {
                        put_line_annotation(&mut row, text, palette, false);
                    }
                }
            }
            painted.push(Line::from(row));
            row_lines.push(line);
        }
    }
    frame.render_widget(Paragraph::new(painted), inner);
    state.layout.file_view_row_lines = row_lines;
    if let Some(tab) = state.tabs.active_file_view_mut() {
        tab.scroll = scroll;
    }
}

/// Pane chrome shared by the left pane, the right pane, the file tab, and
/// the Explorer tab's two panes (so the Workspace, compare, file, and
/// Explorer tabs look the same).
///
/// [`BackgroundMode::Terminal`]: a boxed [`Block`] with the title in
/// `heading` and a [`pane_border`] colour that shows focus (the v0.1.244
/// look). [`BackgroundMode::Paint`]: `area` is filled with `bg`, no border
/// glyphs, row 0 is the title row ([`flat_title_line`]) and row 1 the
/// accent row ([`flat_accent_line`]), focused or not, so focus never moves
/// the content. Returns the content rect: the block's inner rect, or
/// `area` less its first two rows.
fn draw_pane_chrome(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    focused: bool,
    bg: Color,
    state: &AppState,
) -> Rect {
    let palette = state.theme.palette();
    if state.background == BackgroundMode::Terminal {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(pane_title(title))
            .title_style(Style::default().fg(palette.heading))
            .border_style(pane_border(focused, palette));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        return inner;
    }
    fill_bg(frame, area, bg);
    let rows = [
        flat_title_line(title, focused, area.width, palette),
        flat_accent_line(focused, area.width, state.ascii, palette),
    ];
    for (row, line) in (area.y..area.bottom()).zip(rows) {
        frame.render_widget(
            Paragraph::new(line),
            Rect {
                y: row,
                height: 1,
                ..area
            },
        );
    }
    let chrome = area.height.min(FLAT_CHROME_ROWS);
    Rect {
        y: area.y + chrome,
        height: area.height - chrome,
        ..area
    }
}

/// Rows a flat pane's chrome takes above its content: the title row and
/// the accent row.
const FLAT_CHROME_ROWS: u16 = 2;

/// Title row of a flat pane, `width` cells: one blank cell, the title,
/// then blanks to the right edge.
///
/// Focused: the title is `heading`, bold. Unfocused: `muted`, normal
/// weight. Never underlined; the blank cells are plain spaces and the
/// background is left to the pane fill.
fn flat_title_line(title: &str, focused: bool, width: u16, palette: Palette) -> Line<'static> {
    let title_w = painted_width(title).min(width.saturating_sub(1));
    let tail = usize::from(width.saturating_sub(1).saturating_sub(title_w));
    let title_style = if focused {
        Style::default()
            .fg(palette.heading)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(palette.muted)
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(title.to_string(), title_style),
        Span::raw(" ".repeat(tail)),
    ])
}

/// Accent row of a flat pane (the row under the title), `width` cells.
///
/// Focused: a full-width line (`▁`, `_` in ASCII mode) in `cursor`.
/// Unfocused: blank. The background is left to the pane fill.
fn flat_accent_line(focused: bool, width: u16, ascii: bool, palette: Palette) -> Line<'static> {
    let width = usize::from(width);
    if focused {
        Line::from(Span::styled(
            glyph(ascii, "\u{2581}", "_").repeat(width),
            Style::default().fg(palette.cursor),
        ))
    } else {
        Line::from(" ".repeat(width))
    }
}

/// Set the background of every cell in `area` to `bg` (paint mode). Text
/// and spans without their own background keep it.
fn fill_bg(frame: &mut Frame<'_>, area: Rect, bg: Color) {
    frame.buffer_mut().set_style(area, Style::default().bg(bg));
}

/// The active Explorer tab: the checkout's file tree on the left (titled
/// with the checkout path, on `sidebar` in paint mode) and the focused
/// file's preview on the right (on `surface`) — the worktree diff of a
/// changed file, the body of a clean one, or a short hint on a folder.
/// Both panes go through [`draw_pane_chrome`], so they are boxed or flat
/// as the Workspace panes are, and every recorded rect is the content
/// rect.
fn draw_explorer_tab(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let widths = pane_widths(area.width, state.tree_fraction);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(widths.tree_width),
            Constraint::Min(MIN_PANE_COLS),
        ])
        .split(area);
    let palette = state.theme.palette();
    let Some(tab) = state.tabs.active_explorer() else {
        return;
    };
    let preview_title = match &tab.preview {
        ExplorerPreview::None => "preview".to_string(),
        ExplorerPreview::Diff { .. } => "diff".to_string(),
        ExplorerPreview::File(file) => file.display.clone(),
    };
    // One status index and one row walk per frame, shared by the tree,
    // the folder hint, and the breadcrumb.
    let status = state.explorer_status(&tab.checkout);
    let rows = tab.tree.rows(&status);
    let cursor = tab.tree.cursor_index(&rows);
    let folder = cursor.is_some_and(|index| rows[index].is_dir);
    let tree_inner = draw_pane_chrome(
        frame,
        panes[0],
        &tab.checkout,
        state.focus == FocusPane::Left,
        palette.sidebar,
        state,
    );
    state.layout.explorer_tree = tree_inner;
    draw_explorer_tree(frame, tree_inner, state, &rows, cursor, &status);
    if let Some(tab) = state.tabs.active_explorer_mut() {
        tab.painted_rel = cursor.map(|index| rows[index].rel.clone());
    }

    let right_inner = draw_pane_chrome(
        frame,
        panes[1],
        &preview_title,
        state.focus == FocusPane::Right,
        palette.surface,
        state,
    );
    state.layout.explorer_preview = right_inner;
    state.layout.right_x = panes[1].x;
    state.layout.right_y = right_inner.y;
    state.layout.diff_content_x = right_inner.x;
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    state.layout.diff_split_rule_x = None;
    if right_inner.width == 0 || right_inner.height == 0 {
        return;
    }
    match state.tabs.active_explorer().map(|tab| &tab.preview) {
        Some(ExplorerPreview::Diff { .. }) => draw_diff_pane(frame, right_inner, state),
        Some(ExplorerPreview::File(_)) => draw_file_body(frame, right_inner, state),
        _ if folder => frame.render_widget(
            Paragraph::new(muted_copy(EXPLORER_FOLDER_HINT, palette)),
            right_inner,
        ),
        _ => {}
    }
}

/// Explorer tree `rows` (the cursor on `cursor`) inside `area`, keeping
/// the cursor row in view. `status` gives each row its mark.
fn draw_explorer_tree(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut AppState,
    rows: &[ExplorerRow],
    cursor: Option<usize>,
    status: &ExplorerStatus,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    if rows.is_empty() {
        frame.render_widget(Paragraph::new(muted_copy(EMPTY_FOLDER, palette)), area);
        return;
    }
    let height = area.height as usize;
    let (start, _) = visible_window(rows.len(), cursor.unwrap_or(0), height);
    if let Some(tab) = state.tabs.active_explorer_mut() {
        tab.tree_scroll = start;
    }
    let focused = state.focus == FocusPane::Left;
    let search = state.theme.pills().filter;
    let lines: Vec<Line> = rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .map(|(index, row)| {
            paint_segmented_row(
                row.depth,
                row.is_dir,
                !row.expanded,
                &explorer_row_segments(row, status, state.ascii),
                area.width as usize,
                Some(index) == cursor,
                focused,
                None,
                false,
                search,
                state.ascii,
                palette,
                0,
            )
        })
        .collect();
    frame.render_widget(Paragraph::new(lines), area);
}

/// Name and right-hand mark of one Explorer row.
///
/// A changed file paints in its status colour with its letter badge
/// (`??` for an untracked file); a folder with changes under it gets a
/// `●` in the modified colour. Ignored entries are muted and dim; a
/// loading row says so.
fn explorer_row_segments(row: &ExplorerRow, status: &ExplorerStatus, ascii: bool) -> NodeSegments {
    let seg = |text: &str, role: SegRole, dim: bool| TextSeg {
        text: text.to_string(),
        role,
        hex: None,
        bold: false,
        dim,
        icon: None,
    };
    if row.placeholder {
        return NodeSegments {
            segments: vec![seg(LOADING_FILE, SegRole::Muted, false)],
            trailing: Vec::new(),
        };
    }
    if row.ignored {
        return NodeSegments {
            segments: vec![seg(&row.name, SegRole::Muted, true)],
            trailing: Vec::new(),
        };
    }
    if row.is_dir {
        let dirty = status.dir_dirty(&row.rel);
        let dot = if ascii { "* " } else { "● " };
        return NodeSegments {
            segments: vec![seg(&row.name, SegRole::Dir, false)],
            trailing: if dirty {
                vec![seg(dot, SegRole::Modified, false)]
            } else {
                Vec::new()
            },
        };
    }
    let (role, badge) = if status.is_untracked(&row.rel) {
        (SegRole::Added, Some(UNTRACKED_BADGE))
    } else if let Some(letter) = status.letter(&row.rel) {
        (SegRole::from(letter.color_role()), Some(letter.badge()))
    } else {
        (SegRole::File, None)
    };
    NodeSegments {
        segments: vec![seg(&row.name, role, false)],
        trailing: badge
            .map(|badge| vec![seg(badge, role, false)])
            .unwrap_or_default(),
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
fn draw_too_small(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let palette = state.theme.palette();
    if state.background == BackgroundMode::Paint {
        fill_bg(frame, area, palette.surface);
    }
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
    let mut hits = Vec::new();
    for (y, row) in (area.y..).zip(painted.iter().skip(start).take(height)) {
        let (viewed, commented, resolved, pr) = state.tree_row_marks(row);
        let (line, icons) = paint_tree_row(
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
            pr,
            palette,
            state.left_col_offset as usize,
        );
        lines.push(line);
        for (kind, x, w) in icons {
            hits.push(IconHit {
                y,
                x: area.x.saturating_add(u16::try_from(x).unwrap_or(u16::MAX)),
                width: u16::try_from(w).unwrap_or(u16::MAX),
                kind,
                target: tree_icon_target(kind, row),
            });
        }
    }
    state.layout.icon_hits.extend(hits);
    frame.render_widget(Paragraph::new(lines), area);
}

/// Painted icon of a row: catalog kind, column offset from the row start,
/// and width.
type IconSpan = (IconKind, usize, usize);

/// Paint one tree row. Also returns an [`IconSpan`] for each segment the
/// row paints with a catalog icon tag ([`TextSeg::icon`]) that is still on
/// screen after `col_offset`, the label budget, and the row `width`.
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
    pr: Option<PrState>,
    palette: Palette,
    col_offset: usize,
) -> (Line<'static>, Vec<IconSpan>) {
    let segs_width =
        |segs: &[TextSeg]| -> usize { segs.iter().map(|s| visible_width(&s.text)).sum() };
    let mut segs = painted_row_segments(row, ascii, viewed, commented, resolved, pr);
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
    let icons = segmented_icon_spans(row.depth, &segs, width, col_offset);
    let line = paint_segmented_row(
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
    );
    (line, icons)
}

/// Painted span of every icon segment of a row that
/// [`paint_segmented_row`] paints `width` columns wide at `col_offset`.
///
/// A span covers the glyph columns only: spaces at either end of the
/// segment text stay out. Label icons pan and clip like the label; trailing
/// icons clip at the row width. An icon with no cell on screen has no
/// span.
fn segmented_icon_spans(
    depth: usize,
    segs: &NodeSegments,
    width: usize,
    col_offset: usize,
) -> Vec<IconSpan> {
    let (prefix_width, label_budget) = segmented_label_area(depth, segs, width);
    let mut spans = Vec::new();
    for (index, seg) in segs.segments.iter().enumerate() {
        let Some(kind) = seg.icon else {
            continue;
        };
        // Split the padding off the glyph so the label slice places the
        // glyph exactly as paint does.
        let (lead, core, tail) = split_seg_padding(seg);
        let mut split: Vec<TextSeg> = segs.segments[..index].to_vec();
        split.extend([lead, core, tail]);
        split.extend_from_slice(&segs.segments[index + 1..]);
        if let Some((x, w)) = painted_seg_span(
            &split,
            index + 1,
            prefix_width,
            col_offset,
            label_budget,
            width,
        ) {
            spans.push((kind, x, w));
        }
    }
    let label = slice_segs(&segs.segments, col_offset, label_budget.max(1));
    let label_width: usize = label.iter().map(|s| visible_width(&s.text)).sum();
    let trailing_width: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
    let pad = usize::from(trailing_width > 0);
    let mut x = prefix_width + label_width + pad + label_budget.saturating_sub(label_width);
    for seg in &segs.trailing {
        let seg_width = visible_width(&seg.text);
        if let Some(kind) = seg.icon {
            let (lead, core, _) = split_seg_padding(seg);
            let start = x + visible_width(&lead.text);
            let shown = clipped_width(&core.text, width.saturating_sub(start));
            if shown > 0 {
                spans.push((kind, start, shown));
            }
        }
        x += seg_width;
    }
    spans
}

/// Columns of `text` that fit in `room`, less the spaces a cut leaves at
/// the end (`2 wt` cut after `2 ` is one column).
fn clipped_width(text: &str, room: usize) -> usize {
    let (mut used, mut end) = (0, 0);
    for (at, ch) in text.char_indices() {
        used += visible_width(ch.encode_utf8(&mut [0; 4]));
        if used > room {
            break;
        }
        end = at + ch.len_utf8();
    }
    visible_width(text[..end].trim_end())
}

/// `seg` as leading spaces, the text between, and trailing spaces, each
/// with the segment's style.
fn split_seg_padding(seg: &TextSeg) -> (TextSeg, TextSeg, TextSeg) {
    let core = seg.text.trim_matches(' ');
    let lead = seg.text.len() - seg.text.trim_start_matches(' ').len();
    let tail = seg.text.len() - lead - core.len();
    let part = |text: String| TextSeg {
        text,
        ..seg.clone()
    };
    (
        part(" ".repeat(lead)),
        part(core.to_string()),
        part(" ".repeat(tail)),
    )
}

/// Prefix width (edge, indent, chevron) and label budget of a row that
/// [`paint_segmented_row`] paints `width` columns wide.
fn segmented_label_area(depth: usize, segs: &NodeSegments, width: usize) -> (usize, usize) {
    let trailing_width: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
    let pad = usize::from(trailing_width > 0);
    let prefix_width = 1 + 2 * depth + 2;
    let label_budget = width
        .saturating_sub(prefix_width)
        .saturating_sub(trailing_width)
        .saturating_sub(pad);
    (prefix_width, label_budget)
}

/// Painted span (column offset from the row start, width) of label
/// segment `index` after [`slice_segs`] cuts the label at `col_offset` and
/// the label budget, clipped to the row `width`. `None` when no cell of
/// that segment is on screen.
fn painted_seg_span(
    segs: &[TextSeg],
    index: usize,
    prefix_width: usize,
    col_offset: usize,
    label_budget: usize,
    width: usize,
) -> Option<(usize, usize)> {
    // Slice exactly as paint does, so a wide glyph cut by the offset
    // moves the span the same way it moves the painted cells.
    let painted_width = |end: usize| -> usize {
        slice_segs(&segs[..end], col_offset, label_budget.max(1))
            .iter()
            .map(|s| visible_width(&s.text))
            .sum()
    };
    let start = prefix_width + painted_width(index);
    let end = (prefix_width + painted_width(index + 1)).min(width);
    (start < end).then(|| (start, end - start))
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
    let (_, label_budget) = segmented_label_area(depth, segs, width);
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

fn seg_role_color(role: SegRole, palette: Palette) -> Color {
    match role {
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
}

fn seg_style(seg: &TextSeg, palette: Palette) -> Style {
    let fg = match seg.hex {
        Some(hex) => hex_color(hex),
        None => seg_role_color(seg.role, palette),
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
                        icon: None,
                    },
                    TextSeg {
                        text: format!(" {minus}{}  ", stat.deleted),
                        role: SegRole::Deleted,
                        hex: None,
                        bold: false,
                        dim: false,
                        icon: None,
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
                icon: None,
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
    let rows = model.visible_rows();
    let matches = graph_search_matches(state, &rows);
    let pal = state.theme.palette();
    let flash_rows = state.graph_flash_rows();
    let lane_colors = state.theme.lane_colors();
    let commented_rows = graph_commented_row_indices(state, &rows, false);
    let resolved_comment_rows = graph_commented_row_indices(state, &rows, true);
    let graph_focused = if state.drill.is_files() {
        state.focus == FocusPane::Left
    } else {
        state.focus == FocusPane::Right
    };
    let badge_rows = graph_pr_badges(state);
    let badges: Vec<(usize, &str, Color)> = badge_rows
        .iter()
        .map(|(index, _, _, glyph, color)| (*index, *glyph, *color))
        .collect();
    let spans = GraphWidget::new(model)
        .ascii(state.ascii)
        .selected(Some(state.graph_cursor))
        .cursor_bar(graph_focused)
        .scroll(state.graph_scroll)
        .col_offset(col_offset)
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
        .commit_msg_lines(state.commit_msg_lines)
        .commit_msg_scroll(state.graph_footer_msg_scroll())
        .footer_rule(footer_rule_glyph(state.ascii), pal.border_dim)
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
            worktree: pal.heading,
            dirty: pal.modified,
            comment: pal.heading,
            comment_resolved: pal.muted,
            ahead: pal.added,
            behind: pal.deleted,
        })
        .row_badges(&badges)
        .render_with_icon_spans(area, frame.buffer_mut());
    let row_of = |span: &workspace_status_graph::IconSpan| rows.get(span.row_index?);
    let hits: Vec<IconHit> = spans
        .iter()
        .filter_map(|span| {
            let (kind, target) = match span.kind {
                GraphIconKind::Badge => badge_rows
                    .iter()
                    .find(|row| Some(row.0) == span.row_index)
                    .map(|(_, repo, pr, _, _)| {
                        (pr_badge_kind(*pr), IconTarget::PullRequest(repo.clone()))
                    })?,
                GraphIconKind::SyncHeader => (
                    graph_sync_kind(model.sync.as_ref()?.status),
                    IconTarget::GraphSync,
                ),
                GraphIconKind::MoreBelow => (
                    IconKind::GraphMoreBelow,
                    IconTarget::GraphMoreLines(graph_row_id(row_of(span)?)),
                ),
                kind => graph_icon(kind, span.part, span.target.as_ref(), row_of(span)?)?,
            };
            Some(IconHit {
                y: span.y,
                x: span.x,
                width: span.width,
                kind,
                target,
            })
        })
        .collect();
    state.layout.icon_hits.extend(hits);
    record_graph_scrollbar(state, area, col_offset, &badges);
}

/// One graph worktree row's PR badge: visible row index, checkout,
/// state, glyph, and colour.
type GraphPrBadge = (usize, PathBuf, PrState, &'static str, Color);

/// [`AppState::graph_pr_badges`] with the glyph and colour to paint.
fn graph_pr_badges(state: &AppState) -> Vec<GraphPrBadge> {
    let palette = state.theme.palette();
    state
        .graph_pr_badges()
        .into_iter()
        .map(|(index, path, pr)| {
            let (glyph, role) = pr_badge_mark(state.ascii, pr);
            (index, path, pr, glyph, seg_role_color(role, palette))
        })
        .collect()
}

fn graph_commented_row_indices(
    state: &AppState,
    rows: &[GraphRow],
    resolved_only: bool,
) -> Vec<usize> {
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
    rows.iter()
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

fn graph_search_matches(state: &AppState, rows: &[GraphRow]) -> Vec<usize> {
    if state.search_target != SearchPane::Graph {
        return Vec::new();
    }
    collect_graph_match_indices(rows, &state.search_query)
}

fn record_graph_scrollbar(
    state: &mut AppState,
    area: Rect,
    col_offset: u16,
    badges: &[(usize, &str, Color)],
) {
    if state.graph.is_none() {
        return;
    }
    if area.width == 0 || area.height == 0 {
        return;
    }
    let chrome = state.graph_chrome_in(area.height);
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
        chrome.footer_body_height(),
    );
    if chrome.footer && footer_scroll_max > 0 {
        let bottom = area.y.saturating_add(area.height);
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
        let max = graph_col_max(model, state.ascii, area.width, vscroll, badges);
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
/// `pane_h` rows tall, for a footer body of `body_len` rows
/// ([`AppState::commit_detail_footer_request`], fixed whatever the message).
///
/// The footer is the [`FOOTER_RULE_ROWS`] rule row, then the body, so it
/// asks for `body_len + 1` rows. The file list keeps at least one row
/// whenever the pane has two or more, so a short terminal never hides the
/// list for the message; the body clips first, down to the rule only.
/// Draw and layout both size the footer here so the list rows and click
/// mapping agree.
fn commit_detail_footer_height(body_len: usize, pane_h: u16) -> u16 {
    let want = body_len
        .min(usize::from(u16::MAX - FOOTER_RULE_ROWS))
        .saturating_add(usize::from(FOOTER_RULE_ROWS)) as u16;
    let max = if pane_h >= 2 { pane_h - 1 } else { pane_h };
    want.min(max)
}

/// Rule glyph between a list and its footer: `▁` (`_` in ASCII mode).
fn footer_rule_glyph(ascii: bool) -> &'static str {
    glyph(ascii, "\u{2581}", "_")
}

/// The rule row that tops a list footer, `width` cells of
/// [`footer_rule_glyph`] in `border_dim`. Boxed and flat panes alike.
fn footer_rule_line(width: u16, ascii: bool, palette: Palette) -> Line<'static> {
    Line::from(Span::styled(
        footer_rule_glyph(ascii).repeat(usize::from(width)),
        Style::default().fg(palette.border_dim),
    ))
}

/// Commit files beside the file diff (depth 2, left pane): the file list on
/// top, the selected commit's title, meta, and message pinned to the bottom
/// rows as a footer (like the graph selection footer on the graph pane).
/// The footer's first row is a rule ([`footer_rule_line`]) under the list.
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
    let footer_h = commit_detail_footer_height(state.commit_detail_footer_request(), area.height);
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
    let rule_y = area.y.saturating_add(list_h);
    frame.render_widget(
        Paragraph::new(footer_rule_line(area.width, state.ascii, palette)),
        Rect {
            x: area.x,
            y: rule_y,
            width: area.width,
            height: 1,
        },
    );
    let body_h = footer_h.saturating_sub(FOOTER_RULE_ROWS);
    if body_h == 0 {
        return;
    }
    // Clip keeps the first lines: title, meta, then message lines.
    let footer_lines: Vec<Line> = footer
        .iter()
        .take(body_h as usize)
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
        y: rule_y.saturating_add(FOOTER_RULE_ROWS),
        width: area.width,
        height: body_h,
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
                tab.empty_files_copy()
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
    let files_focused = if state.drill.is_diff() || state.is_compare_tab() {
        state.focus == FocusPane::Left
    } else {
        state.focus == FocusPane::Right
    };
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    let comment_scope = state.commit_file_comment_scope();
    for (y, row) in (area.y..).zip(rows.iter().skip(start).take(height)) {
        let segs = state.commit_file_row_paint_segments_in(row, comment_scope);
        let search_match = searching_files
            && (match_paths.contains(&row.path)
                || commit_file_label_matches(&row.label, &state.search_query));
        for (kind, x, w) in segmented_icon_spans(row.depth, &segs, width, col_offset) {
            hits.push(IconHit {
                y,
                x: area.x.saturating_add(u16::try_from(x).unwrap_or(u16::MAX)),
                width: u16::try_from(w).unwrap_or(u16::MAX),
                kind,
                target: IconTarget::CommitFileRow(row.id.clone()),
            });
        }
        lines.push(paint_segmented_row(
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
        ));
    }
    state.layout.icon_hits.extend(hits);
    frame.render_widget(Paragraph::new(lines), area);
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
            tab.error.clone().unwrap_or_else(|| tab.empty_diff_copy())
        } else if state.explorer_diff_loading() {
            LOADING_FILE.to_string()
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
    let overlay = diff_row_overlay(selected, focused, visual, palette);
    // Current-line blame: only the focused row of a focused diff pane.
    let blame = if selected && focused {
        state.painted_line_annotation()
    } else {
        None
    };
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
            let (left_h, right_h) = if wrap {
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
                (left_h, right_h)
            } else {
                (1, 1)
            };
            (0..left_h.max(right_h))
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
                        overlay,
                        left_syntax,
                    );
                    if let Some((text, BlameSide::Old)) = &blame {
                        if part + 1 == left_h {
                            put_line_annotation(&mut spans, text, palette, true);
                        }
                    }
                    spans.push(Span::styled(
                        DIFF_RULE.to_string(),
                        Style::default().fg(diff_rule_fg(state)),
                    ));
                    let mut right_spans = paint_cell_spans(
                        right.as_ref().unwrap(),
                        cols.right_width,
                        gutter,
                        col_offset,
                        wrap,
                        part,
                        palette,
                        state,
                        overlay,
                        right_syntax,
                    );
                    if let Some((text, BlameSide::New)) = &blame {
                        if part + 1 == right_h {
                            put_line_annotation(&mut right_spans, text, palette, true);
                        }
                    }
                    spans.extend(right_spans);
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
                    let mut spans = paint_cell_spans(
                        left,
                        width,
                        gutter,
                        col_offset,
                        wrap,
                        part,
                        palette,
                        state,
                        overlay,
                        left_syntax,
                    );
                    if let Some((text, _)) = &blame {
                        if part + 1 == n {
                            put_line_annotation(&mut spans, text, palette, true);
                        }
                    }
                    Line::from(spans)
                })
                .collect()
        }
    };
    parts
        .into_iter()
        .map(|line| finish_diff_line(line, selected, focused, visual, search, palette))
        .collect()
}

/// Side-by-side rule colour: `border_dim` on a painted pane, the v0.1.244
/// `DarkGray` on the terminal background.
fn diff_rule_fg(state: &AppState) -> Color {
    match state.background {
        BackgroundMode::Paint => state.theme.palette().border_dim,
        BackgroundMode::Terminal => Color::DarkGray,
    }
}

/// Paint `text` dimmed at the end of a cell or file line, inside its
/// trailing blank pad, as `"  " + text` cut to fit.
///
/// The pad keeps its width, so row heights, the gutter, and the pan range
/// never change. No pad (code fills the width, or the line is panned) or
/// fewer than 12 free columns paints nothing.
///
/// `on_cursor_bar` (diff rows) drops the pad's bg from the note and the pad
/// after it, so the row's flat cursor overlay paints behind them instead of
/// the tinted add/del bg. The two-column gap before the note keeps it.
fn put_line_annotation(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    palette: Palette,
    on_cursor_bar: bool,
) {
    let Some(pad) = spans.last() else {
        return;
    };
    if pad.content.is_empty() || pad.content.chars().any(|c| c != ' ') {
        return;
    }
    let pad_w = pad.content.len();
    let Some(fitted) = fit_annotation(text, pad_w.saturating_sub(2)) else {
        return;
    };
    let mut style = pad.style;
    let rest = pad_w - 2 - fitted.width();
    spans.pop();
    if on_cursor_bar {
        // The two-column gap keeps the pad's bg; the note and the rest drop it.
        spans.push(Span::styled("  ", style));
        style.bg = None;
        spans.push(Span::styled(fitted, style.fg(palette.muted)));
    } else {
        spans.push(Span::styled(format!("  {fitted}"), style.fg(palette.muted)));
    }
    if rest > 0 {
        spans.push(Span::styled(" ".repeat(rest), style));
    }
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
    let overlay = diff_row_overlay(selected, focused, visual, palette);
    let bg = overlay.or(match_pill.map(|pill| pill.bg));
    if let Some(overlay) = overlay {
        // `paint_cell_spans` already tinted add/del code and pad; every
        // other span takes the flat overlay.
        line.spans = line
            .spans
            .into_iter()
            .map(|span| {
                let style = match span.style.bg {
                    Some(bg) if is_tinted_change_bg(bg, overlay, palette) => span.style,
                    _ => span.style.bg(overlay),
                };
                Span::styled(span.content.to_string(), style)
            })
            .collect();
    } else if let Some(pill) = match_pill {
        line.spans = line
            .spans
            .into_iter()
            .map(|span| Span::styled(span.content.to_string(), span.style.bg(pill.bg).fg(pill.fg)))
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

/// Cursor bar over a diff row: [`Palette::cursor_bg`] on the focused cursor
/// row and on visual-line rows, [`Palette::cursor_bg_inactive`] on the
/// selected row of an unfocused pane, none otherwise.
fn diff_row_overlay(
    selected: bool,
    focused: bool,
    visual: bool,
    palette: Palette,
) -> Option<Color> {
    if selected && focused {
        Some(palette.cursor_bg)
    } else if selected {
        Some(palette.cursor_bg_inactive)
    } else if visual {
        Some(palette.cursor_bg)
    } else {
        None
    }
}

/// True when `bg` is an add/del row or changed-word background tinted with
/// `overlay` by [`Palette::cursor_tint`].
fn is_tinted_change_bg(bg: Color, overlay: Color, palette: Palette) -> bool {
    [
        palette.diff_add_bg,
        palette.diff_del_bg,
        palette.diff_add_word_bg,
        palette.diff_del_word_bg,
    ]
    .into_iter()
    .any(|change| palette.cursor_tint(change, overlay) == bg)
}

fn section_style(section: DiffSection, palette: Palette) -> Style {
    match section {
        DiffSection::Staged => Style::default().fg(palette.added),
        DiffSection::Unstaged => Style::default().fg(palette.modified),
        DiffSection::New | DiffSection::Committed | DiffSection::Worktree => {
            Style::default().fg(palette.heading)
        }
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
    overlay: Option<Color>,
    syntax: &[CodeSpan],
) -> Vec<Span<'static>> {
    let ascii = state.ascii;
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
    // Under a cursor overlay the gutter and sign take the flat cursor bar;
    // add/del code and pad keep their row and word bg, tinted.
    let tint = |bg: Option<Color>| match (bg, overlay) {
        (Some(bg), Some(overlay)) => Some(palette.cursor_tint(bg, overlay)),
        _ => bg,
    };
    let plain_row_bg = cell_row_bg(cell.kind, palette);
    let chrome_bg = overlay.or(plain_row_bg);
    let row_bg = tint(plain_row_bg);
    let word_bg = tint(cell_word_bg(cell.kind, palette).or(plain_row_bg));
    let gutter_style = with_row_bg(diff_gutter_style(palette), chrome_bg);
    let sign_style = with_row_bg(
        accent.unwrap_or_default().add_modifier(Modifier::BOLD),
        chrome_bg,
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
        // Syntax fg met the contrast floor on the plain bg; check it again
        // on the tinted one.
        let fg = if overlay.is_some() {
            readable_fg(part.fg, bg, palette.repo)
        } else {
            part.fg
        };
        spans.push(Span::styled(
            part.text,
            with_row_bg(Style::default().fg(fg), bg),
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
    if let Some((_, rel, _)) = state.explorer_diff() {
        return rel;
    }
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

/// Rounded popup box with an `accent` border, filled with `panel` so no
/// cell under the popup keeps the terminal background. Callers render
/// [`Clear`] first to drop the pane glyphs underneath.
fn overlay_block(accent: Color, panel: Color) -> Block<'static> {
    Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(accent))
        .style(Style::default().bg(panel))
        .padding(Padding::horizontal(1))
}

/// Colour the panes paint icon `kind` in, so legend rows that only colour
/// tells apart (ref chips, status letters, sync marks, PR badges) read as
/// they do on screen. Glyphs the panes paint in a lane or neutral colour
/// use the heading accent.
fn icon_legend_color(kind: IconKind, palette: Palette) -> Color {
    use IconKind as K;
    match kind {
        K::Branch | K::PrOpen | K::ChipLocal => palette.branch_feature,
        K::ChipDefault => palette.branch_default,
        K::MergedIntoDefault | K::PrApproved | K::Ahead => palette.added,
        K::StatusAdded | K::StatusStaged => palette.added,
        K::Behind | K::StatusFailed | K::StatusDeleted | K::StatusConflict => palette.deleted,
        K::Diverged | K::StatusModified | K::StatusStagedModified => palette.modified,
        K::GraphUncommitted | K::ChipTag => palette.modified,
        K::StatusRenamed | K::StatusCopied => palette.renamed,
        K::OpenVsDefault | K::PrMerged | K::NoUpstream | K::Clean | K::Synced => palette.muted,
        K::Ignored | K::ChangeCount | K::WorktreeCount | K::CommentResolved => palette.muted,
        K::GraphMoreBelow | K::FoldExpanded | K::FoldCollapsed => palette.muted,
        K::CursorBarInactive => palette.muted,
        K::CursorBar => palette.cursor,
        K::Folder | K::ChipRemote | K::ChipSynced => palette.dir,
        K::FileType => palette.file,
        K::Viewed => palette.viewed,
        K::ChipCheckout | K::ChipDetachedHead => palette.head_mark,
        K::Workspace
        | K::Repo
        | K::LinkedWorktree
        | K::Staged
        | K::Changes
        | K::Comment
        | K::GraphCommit
        | K::GraphHeadCommit
        | K::GraphStash
        | K::ChipOverflow
        | K::GraphRails
        | K::HelpMove
        | K::HelpView
        | K::FolderOpen => palette.heading,
    }
}

/// One painted line of an icon legend row, cut or padded to `width`:
/// glyph and muted name on the first line, the meaning beside them or
/// under them (`vis.indent`).
fn help_legend_cell_spans(
    spec: &IconSpec,
    vis: &super::help::HelpVisualLine,
    ascii: bool,
    palette: Palette,
    width: usize,
) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    if vis.chips {
        let glyph = spec.glyph(ascii);
        let glyph_width = spec.group.map_or(0, help_legend_glyph_width);
        let glyph_pad = glyph_width.saturating_sub(visible_width(glyph)) + 1;
        let name_pad = help_legend_name_width().saturating_sub(spec.name.chars().count()) + 1;
        spans.push(Span::styled(
            glyph,
            Style::default().fg(icon_legend_color(spec.kind, palette)),
        ));
        spans.push(Span::raw(" ".repeat(glyph_pad)));
        spans.push(Span::styled(spec.name, Style::default().fg(palette.muted)));
        if !vis.text.is_empty() {
            spans.push(Span::raw(" ".repeat(name_pad)));
        }
    } else if vis.indent > 0 {
        spans.push(Span::raw(" ".repeat(vis.indent)));
    }
    if !vis.text.is_empty() {
        spans.push(Span::styled(
            vis.text.clone(),
            Style::default().fg(palette.file),
        ));
    }
    clamp_spans(spans, width)
}

/// Icon legend rows under the help key columns: a blank row, the
/// `ICONS` title, then [`help_legend_layout`] columns. Row count is
/// [`help_legend_line_count`]. Also returns the row (index into the
/// lines) where each search hit starts.
fn help_legend_lines(
    state: &AppState,
    inner: usize,
    query: &str,
    searching: bool,
) -> (Vec<Line<'static>>, Vec<usize>) {
    let palette = state.theme.palette();
    let pills = state.theme.pills();
    let legend = help_legend_layout(inner);
    let col_w = legend.column_width;
    let content = help_column_content_width(col_w);
    let gutter = col_w.saturating_sub(content);
    let mut hit_rows = Vec::new();
    let columns: Vec<Vec<Vec<Span<'static>>>> = legend
        .columns
        .iter()
        .map(|column| {
            let mut rows = Vec::new();
            for block in column {
                if let Some(group) = block.heading {
                    rows.push(clamp_spans(
                        vec![Span::styled(
                            group.title(),
                            Style::default()
                                .fg(palette.repo)
                                .add_modifier(Modifier::BOLD),
                        )],
                        col_w,
                    ));
                }
                let hit = searching && help_legend_matches(block.spec, query);
                if hit {
                    hit_rows.push(HELP_LEGEND_HEAD_ROWS + rows.len());
                }
                for vis in help_legend_visual_lines(block.spec, col_w) {
                    let mut spans = with_search_pill(
                        help_legend_cell_spans(block.spec, &vis, state.ascii, palette, content),
                        hit.then_some(pills.filter),
                    );
                    spans.push(Span::raw(" ".repeat(gutter)));
                    rows.push(spans);
                }
            }
            rows
        })
        .collect();
    let tallest = columns.iter().map(Vec::len).max().unwrap_or(0);
    if tallest == 0 {
        return (Vec::new(), Vec::new());
    }
    let mut lines = vec![
        Line::default(),
        Line::from(Span::styled(
            HELP_LEGEND_TITLE,
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
        )),
    ];
    for row in 0..tallest {
        let mut spans = Vec::new();
        for column in &columns {
            match column.get(row) {
                Some(cell) => spans.extend(cell.iter().cloned()),
                None => spans.push(Span::raw(" ".repeat(col_w))),
            }
        }
        lines.push(Line::from(spans));
    }
    (lines, hit_rows)
}

/// Paint `?` help in `area` and return its max body scroll (0 when every
/// body row fits). The group-title row stays pinned above the scrolled body.
/// While a search has hits below the shown rows, the search footer counts
/// them.
fn draw_help(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> usize {
    if area.width == 0 || area.height == 0 {
        return 0;
    }
    let query = state.help_search_query.as_deref().unwrap_or("");
    let searching = state.help_search_query.is_some();
    let palette = state.theme.palette();
    let pills = state.theme.pills();
    let panel = palette.panel;
    // A compare tab swaps GIT for the COMPARE column, a file tab for FILE.
    let groups = help_groups(help_tab(state));
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
    // `hit_rows`: the body row where each search hit starts.
    let mut hit_rows: Vec<usize> = Vec::new();
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
                if hit {
                    hit_rows.push(rows.len());
                }
                let pill = hit.then_some(pills.filter);
                for vis in help_entry_visual_lines(entry.desc, content, key_width) {
                    let mut spans = with_search_pill(
                        help_visual_cell_spans(
                            entry,
                            &vis,
                            key_width,
                            color,
                            panel,
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
    let key_rows = columns.iter().map(Vec::len).max().unwrap_or(0);
    for row in 0..key_rows {
        let mut spans = Vec::new();
        for (column, &col_w) in columns.iter().zip(&widths) {
            match column.get(row) {
                Some(cell) => spans.extend(cell.iter().cloned()),
                None => spans.push(Span::raw(" ".repeat(col_w))),
            }
        }
        lines.push(Line::from(spans));
    }
    let (legend, legend_hits) = help_legend_lines(state, inner, query, searching);
    lines.extend(legend);
    hit_rows.extend(legend_hits.into_iter().map(|row| key_rows + row));
    let body_rows = lines.len() - 1;

    let idle_footer = |parts: Vec<String>| -> Vec<Line<'static>> {
        parts
            .into_iter()
            .map(|part| Line::from(Span::styled(part, Style::default().fg(palette.muted))))
            .collect()
    };
    // `below`: search hits that start under the shown body rows.
    let search_footer = |below: usize| {
        let q = state.help_search_query.as_deref().unwrap_or("");
        let mut spans = vec![
            key_chip("HELP", pills.filter.bg, pills.filter.fg),
            Span::styled(format!(" /{q}"), Style::default().fg(palette.repo)),
            Span::styled("▏", Style::default().fg(palette.cursor)),
            Span::styled(
                format!("   {HELP_SEARCH_ESC_HINT}"),
                Style::default().fg(palette.muted),
            ),
        ];
        if below > 0 {
            spans.push(Span::styled(
                format!(
                    "   {}{below} more below · PgDn",
                    IconKind::GraphMoreBelow.glyph(state.ascii)
                ),
                Style::default().fg(palette.cursor),
            ));
        }
        help_footer_with_version(spans, inner, palette.muted)
    };
    let mut footer = if searching {
        search_footer(0)
    } else {
        idle_footer(help_idle_footer_lines(inner))
    };

    frame.render_widget(Clear, area);
    let block = overlay_block(palette.cursor, panel);
    let inner_area = block.inner(area);
    frame.render_widget(block, area);
    if inner_area.width == 0 || inner_area.height == 0 {
        return 0;
    }
    // Body rows under the pinned title row once `footer` takes its rows.
    let body_room = |footer: &[Line<'static>]| {
        let footer_h = (footer.len() as u16).min(inner_area.height).max(1);
        usize::from(inner_area.height.saturating_sub(footer_h).saturating_sub(1))
    };
    let mut scroll_max = body_rows.saturating_sub(body_room(&footer));
    if scroll_max > 0 && !searching {
        let mut parts = wrap_help_footer(&format!("j/k scroll · {}", help_idle_footer()), inner);
        attach_help_version(&mut parts, inner);
        footer = idle_footer(parts);
        scroll_max = body_rows.saturating_sub(body_room(&footer));
    }
    // Hits under the shown rows. Two passes: the cue can wrap the footer
    // onto one more row, which shows one body row less.
    let hits_below = |footer: &[Line<'static>], scroll_max: usize| {
        let shown_end = state.help_scroll.min(scroll_max) + body_room(footer);
        hit_rows.iter().filter(|&&row| row >= shown_end).count()
    };
    if searching {
        for _ in 0..2 {
            let below = hits_below(&footer, scroll_max);
            footer = search_footer(below);
            scroll_max = body_rows.saturating_sub(body_room(&footer));
        }
    }
    let footer_h = (footer.len() as u16).min(inner_area.height).max(1);
    let body_h = inner_area.height.saturating_sub(footer_h);
    if body_h > 0 {
        let scroll = state.help_scroll.min(scroll_max);
        let mut lines = lines.into_iter();
        let title = lines.next().into_iter();
        let visible: Vec<Line> = title
            .chain(lines.skip(scroll).take(usize::from(body_h) - 1))
            .collect();
        frame.render_widget(
            Paragraph::new(visible),
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
    scroll_max
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
    let panel = palette.panel;
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
                panel,
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
                confirm_action_row("y", "revert", None, accent, palette.muted, panel),
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
                confirm_action_row("y", "revert", None, accent, palette.muted, panel),
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
                    panel,
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
                confirm_action_row("y", "drop", None, accent, palette.muted, panel),
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
                panel,
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
                panel,
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
                confirm_action_row("y", "switch", None, accent, palette.muted, panel),
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
                confirm_action_row("y", "merge", None, accent, palette.muted, panel),
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
            .block(overlay_block(accent, panel))
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
    let panel = palette.panel;
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
            key_chip(&op.key.to_string(), accent, panel),
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
    // The status row is always there, so typing never grows the box.
    lines.push(Line::from(Span::styled(
        state.status.to_string(),
        Style::default().fg(state.status.kind().color(palette)),
    )));
    lines.push(Line::from(Span::styled(
        "Esc cancel",
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(accent, panel))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// `A` blame-actions menu: the focused line's annotation (the same text
/// the pane paints at the line's end, cut to one row), then one row per
/// action with its key.
fn draw_blame_menu(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let panel = palette.panel;
    let accent = palette.modified;
    const TITLE: &str = "Blame ";
    // Inside the border, after the title.
    let room = usize::from(area.width.saturating_sub(2)).saturating_sub(TITLE.len());
    let header = state
        .painted_line_annotation()
        .and_then(|(text, _)| fit_annotation(&text, room))
        .unwrap_or_default();
    let mut lines = vec![Line::from(vec![
        Span::styled(
            TITLE,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(header, Style::default().fg(palette.muted)),
    ])];
    for row in &BLAME_MENU_ROWS {
        lines.push(Line::from(vec![
            key_chip(&row.key.to_string(), accent, panel),
            Span::styled(format!(" {}", row.label), Style::default().fg(palette.file)),
        ]));
    }
    lines.push(Line::from(Span::styled(
        "Esc cancel",
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines).block(overlay_block(accent, panel)),
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

/// List rows a list dialog paints at inner height `inner_h`: the box less
/// its query, status, and footer rows, capped at [`LIST_OVERLAY_MAX_ROWS`].
fn list_dialog_rows(inner_h: u16) -> usize {
    usize::from(inner_h.saturating_sub(3)).min(LIST_OVERLAY_MAX_ROWS)
}

/// Status row of a list dialog: `state.status`, blank when empty.
fn list_dialog_status(state: &AppState, palette: Palette) -> Line<'static> {
    Line::from(Span::styled(
        state.status.to_string(),
        Style::default().fg(state.status.kind().color(palette)),
    ))
}

/// Paint a list dialog at fixed rows: `header` (the query / title row) on
/// the first inner row, `rows` under it (cut to fit), `status` on the row
/// above the last, and `footer` on the last. Result count never moves the
/// header or the footer.
fn paint_list_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    block: Block<'static>,
    header: Line<'static>,
    rows: Vec<Line<'static>>,
    status: Line<'static>,
    footer: Line<'static>,
) {
    frame.render_widget(Clear, area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let row_at = |offset: u16| Rect {
        x: inner.x,
        y: inner.y + offset,
        width: inner.width,
        height: 1,
    };
    let h = inner.height;
    frame.render_widget(Paragraph::new(header), row_at(0));
    for (offset, line) in (1..h.saturating_sub(2)).zip(rows) {
        frame.render_widget(Paragraph::new(line), row_at(offset));
    }
    if h >= 3 {
        frame.render_widget(Paragraph::new(status), row_at(h - 2));
    }
    if h >= 2 {
        let footer = fit_list_dialog_footer(footer, usize::from(inner.width));
        frame.render_widget(Paragraph::new(footer), row_at(h - 1));
    }
}

/// `footer` cut to `width` columns with its `Esc …` chip kept.
///
/// A footer that fits paints as is. Otherwise its ` · `-separated chips
/// drop from the left, the `Esc` chip never, until the rest fits; a lone
/// chip still too wide ends in `…`. The line keeps its first span's style.
fn fit_list_dialog_footer(footer: Line<'static>, width: usize) -> Line<'static> {
    if footer.width() <= width {
        return footer;
    }
    let style = footer
        .spans
        .first()
        .map(|span| span.style)
        .unwrap_or_default();
    let text: String = footer
        .spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect();
    let mut chips: Vec<&str> = text.split(" · ").collect();
    while chips.len() > 1 && visible_width(&chips.join(" · ")) > width {
        let Some(drop) = chips.iter().position(|chip| !chip.starts_with("Esc")) else {
            break;
        };
        chips.remove(drop);
    }
    Line::from(Span::styled(
        fit_with_ellipsis(&chips.join(" · "), width),
        style,
    ))
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
    let max_rows = list_dialog_rows(area.height.saturating_sub(2));
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
    let mut rows = Vec::new();
    if window.is_empty() {
        rows.push(Line::from(Span::styled(
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
                palette.panel
            };
            let name_fg = if selected {
                palette.file
            } else {
                palette.muted
            };
            rows.push(Line::from(vec![
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
    let footer = Line::from(Span::styled(
        "↑↓ move · type to filter · Enter compare · Esc close",
        Style::default().fg(palette.muted),
    ));
    paint_list_dialog(
        frame,
        area,
        overlay_block(accent, palette.panel),
        Line::from(title),
        rows,
        list_dialog_status(state, palette),
        footer,
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
    let row_count = picker.row_count();
    let max_rows = list_dialog_rows(area.height.saturating_sub(2));
    let start = if row_count <= max_rows {
        0
    } else {
        picker
            .cursor
            .saturating_sub(max_rows / 2)
            .min(row_count - max_rows)
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
    let mut rows = Vec::new();
    if window.is_empty() && !create_painted {
        rows.push(Line::from(Span::styled(
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
                palette.panel
            };
            let name_fg = if branch.current {
                palette.added
            } else if selected {
                palette.file
            } else {
                palette.muted
            };
            rows.push(Line::from(vec![
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
            rows.push(branch_create_row(
                name,
                picker.commit_id.as_deref(),
                picker.on_create_row(),
                palette,
                accent,
            ));
        }
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
    paint_list_dialog(
        frame,
        area,
        overlay_block(accent, palette.panel),
        Line::from(title),
        rows,
        list_dialog_status(state, palette),
        Line::from(Span::styled(footer, Style::default().fg(palette.muted))),
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
        palette.panel
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
    let max_rows = list_dialog_rows(area.height.saturating_sub(2));
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
    let header = Line::from(vec![
        Span::styled(
            "Focus branches ",
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        ),
        Span::styled(picker.repo.clone(), Style::default().fg(palette.repo)),
        Span::styled("  filter: ", Style::default().fg(palette.muted)),
        Span::styled(filter.to_string(), Style::default().fg(palette.cursor)),
    ]);
    let mut rows = Vec::new();
    if window.is_empty() {
        rows.push(Line::from(Span::styled(
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
                palette.panel
            };
            let name_fg = if marked || branch.current {
                palette.added
            } else if selected {
                palette.file
            } else {
                palette.muted
            };
            rows.push(Line::from(vec![
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
    let footer = Line::from(Span::styled(
        "↑↓ move · type to filter · space toggle · Enter apply · Ctrl-o clear · Esc cancel",
        Style::default().fg(palette.muted),
    ));
    paint_list_dialog(
        frame,
        area,
        overlay_block(accent, palette.panel),
        header,
        rows,
        list_dialog_status(state, palette),
        footer,
    );
}

fn draw_quick_open(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(quick) = state.quick_open.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette_theme = state.theme.palette();
    let accent = palette_theme.cursor;
    let muted = Style::default().fg(palette_theme.muted);
    // Rounded border plus one column of padding on each side.
    let inner_width = area.width.saturating_sub(4) as usize;
    let max_rows = list_dialog_rows(area.height.saturating_sub(2));
    let files = quick.mode() == QuickOpenMode::Files;
    let title = if files {
        quick.scope.title()
    } else {
        "Commands".to_string()
    };
    let block = overlay_block(accent, palette_theme.panel).title(Span::styled(
        title,
        Style::default()
            .fg(palette_theme.heading)
            .add_modifier(Modifier::BOLD),
    ));
    let caret = Span::styled("▏", Style::default().fg(accent));
    let header = if files && quick.query.is_empty() {
        Line::from(vec![caret, Span::styled("type a file name…", muted)])
    } else {
        let (query, _) = cut_left(&quick.query, inner_width.saturating_sub(1));
        Line::from(vec![
            Span::styled(query, Style::default().fg(accent)),
            caret,
        ])
    };
    let (rows, status, footer) = if files {
        let status = if files_row_shows_status(&state.status) {
            list_dialog_status(state, palette_theme)
        } else {
            Line::from(Span::styled(quick.file_status_text(), muted))
        };
        (
            quick_open_file_rows(quick, palette_theme, max_rows, inner_width),
            status,
            "↑↓ move · Enter open · > commands · # search · Esc close".to_string(),
        )
    } else {
        let commands = &quick.commands;
        let reason = commands
            .selected()
            .and_then(|command| state.palette_disabled_reason(command));
        let footer = match reason {
            Some(why) => format!("Enter run · Esc close · {why}"),
            None => "Enter run · Esc close".into(),
        };
        (
            quick_open_command_rows(state, commands, max_rows, inner_width),
            list_dialog_status(state, palette_theme),
            footer,
        )
    };
    paint_list_dialog(
        frame,
        area,
        block,
        header,
        rows,
        status,
        Line::from(Span::styled(footer, muted)),
    );
}

/// `text` cut from the left to `width` columns, a leading `…` in place of
/// the dropped head. Also returns how many chars were dropped.
fn cut_left(text: &str, width: usize) -> (String, usize) {
    if visible_width(text) <= width {
        return (text.to_string(), 0);
    }
    let chars: Vec<char> = text.chars().collect();
    let budget = width.saturating_sub(1);
    let mut used = 0;
    let mut keep_from = chars.len();
    let mut buf = [0u8; 4];
    while keep_from > 0 {
        let w = visible_width(chars[keep_from - 1].encode_utf8(&mut buf));
        if used + w > budget {
            break;
        }
        used += w;
        keep_from -= 1;
    }
    let kept: String = chars[keep_from..].iter().collect();
    (format!("…{kept}"), keep_from)
}

/// Files-mode rows: `❯ ` on the cursor row, each path cut from the left so
/// the file name stays, matched chars bold in the accent color.
fn quick_open_file_rows(
    quick: &QuickOpenState,
    palette: Palette,
    max_rows: usize,
    inner_width: usize,
) -> Vec<Line<'static>> {
    let FileIndexState::Ready(index) = &quick.index else {
        return Vec::new();
    };
    let len = quick.hits.len();
    let start = if len <= max_rows {
        0
    } else {
        quick
            .file_cursor
            .saturating_sub(max_rows / 2)
            .min(len - max_rows)
    };
    let path_cols = inner_width.saturating_sub(2);
    let mut rows = Vec::new();
    for (offset, hit) in quick.hits.iter().skip(start).take(max_rows).enumerate() {
        let Some(entry) = index.entries.get(hit.entry) else {
            continue;
        };
        let selected = start + offset == quick.file_cursor;
        let row_bg = if selected {
            palette.cursor_bg
        } else {
            palette.panel
        };
        let plain = Style::default()
            .fg(if selected {
                palette.file
            } else {
                palette.muted
            })
            .bg(row_bg);
        let matched = Style::default()
            .fg(palette.cursor)
            .bg(row_bg)
            .add_modifier(Modifier::BOLD);
        let mut spans = vec![Span::styled(
            if selected { "❯ " } else { "  " },
            Style::default()
                .fg(if selected {
                    palette.cursor
                } else {
                    palette.muted
                })
                .bg(row_bg),
        )];
        let (shown, dropped) = cut_left(&entry.display, path_cols);
        // With a cut, painted char 0 is the `…` and char i is source char
        // `i - 1 + dropped`.
        let skip = usize::from(dropped > 0);
        let mut run = String::new();
        let mut run_matched = false;
        for (i, ch) in shown.chars().enumerate() {
            let is_match = i >= skip
                && u32::try_from(i - skip + dropped)
                    .is_ok_and(|pos| hit.indices.binary_search(&pos).is_ok());
            if is_match != run_matched && !run.is_empty() {
                let style = if run_matched { matched } else { plain };
                spans.push(Span::styled(std::mem::take(&mut run), style));
            }
            run_matched = is_match;
            run.push(ch);
        }
        if !run.is_empty() {
            spans.push(Span::styled(run, if run_matched { matched } else { plain }));
        }
        rows.push(Line::from(spans));
    }
    rows
}

/// Commands-mode rows: group headers, command titles, key chips, and the
/// disabled reason at the right edge when it fits.
fn quick_open_command_rows(
    state: &AppState,
    palette: &CommandPaletteState,
    max_rows: usize,
    inner_width: usize,
) -> Vec<Line<'static>> {
    let palette_theme = state.theme.palette();
    let accent = palette_theme.cursor;
    let panel = palette_theme.panel;
    let paint_rows = palette.paint_rows();
    let cursor_paint = paint_rows.iter().position(|row| match row {
        PalettePaintRow::Command { index, .. } => *index == palette.cursor,
        _ => false,
    });
    let start = if paint_rows.len() <= max_rows {
        0
    } else {
        let focus = cursor_paint.unwrap_or(0);
        focus
            .saturating_sub(max_rows / 2)
            .min(paint_rows.len() - max_rows)
    };
    let window: Vec<PalettePaintRow> = paint_rows.into_iter().skip(start).take(max_rows).collect();
    let mut lines = Vec::new();
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
                PalettePaintRow::Header(title) => {
                    header_painted = true;
                    lines.push(Line::from(Span::styled(
                        title.to_string(),
                        Style::default()
                            .fg(palette_theme.heading)
                            .add_modifier(Modifier::BOLD),
                    )));
                }
                PalettePaintRow::Command { command, index } => {
                    let selected = index == palette.cursor;
                    let reason = state.palette_disabled_reason(command);
                    let disabled = reason.is_some();
                    let cursor = if selected { "❯ " } else { "  " };
                    let row_bg = if selected {
                        palette_theme.cursor_bg
                    } else {
                        palette_theme.panel
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
                    // Palette-only rows (Diff … in new tab, Blame: …) have no key.
                    if !command.keys.is_empty() {
                        spans.push(Span::raw(" "));
                        spans.push(key_chip(
                            command.keys,
                            if disabled {
                                palette_theme.muted
                            } else {
                                accent
                            },
                            panel,
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
    lines
}

/// Search-in-files preview body while its read runs.
const PREVIEW_LOADING: &str = "loading…";
/// Search-in-files preview body for a binary file.
const PREVIEW_BINARY: &str = "binary file";
/// Search-in-files preview body for a file over the 2 MiB read cap.
const PREVIEW_TOO_LARGE: &str = "file is over 2 MiB";
/// Query row placeholder while the search query is empty.
const SEARCH_FILES_PLACEHOLDER: &str = "type to search…";

/// Syntax spans of one preview window: preview generation, theme, first
/// and end line.
type SearchPreviewSyntaxKey = (u64, ThemeId, usize, usize);

thread_local! {
    static SEARCH_PREVIEW_SYNTAX_CACHE: RefCell<Option<(SearchPreviewSyntaxKey, FileSyntaxSpans)>> =
        const { RefCell::new(None) };
}

/// Search-in-files dialog: title and option chips on the top border, the
/// query row, the status row, the results grouped by file, and the preview
/// pane on the right when the terminal is at least
/// [`SEARCH_PREVIEW_MIN_COLS`] wide. Records the results height for PgUp /
/// PgDn and keeps the highlighted hit in view.
fn draw_search_files(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    let wide = frame.area().width >= SEARCH_PREVIEW_MIN_COLS;
    let palette = state.theme.palette();
    let theme = state.theme;
    let panel = palette.panel;
    let Some(dialog) = state.search_files.as_ref() else {
        return;
    };
    let accent = palette.cursor;
    let muted = Style::default().fg(palette.muted);
    let block = overlay_block(accent, panel)
        .title(Span::styled(
            dialog.title(),
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
        ))
        .title_top(search_option_chips(dialog.options, palette, panel).right_aligned());
    frame.render_widget(Clear, area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let row_at = |offset: u16| Rect {
        x: inner.x,
        y: inner.y + offset,
        width: inner.width,
        height: 1,
    };
    let h = inner.height;
    let width = usize::from(inner.width);
    let caret = Span::styled(
        "▏",
        Style::default().fg(if dialog.zone == SearchZone::Query {
            accent
        } else {
            palette.muted
        }),
    );
    let query = if dialog.query.is_empty() {
        Line::from(vec![caret, Span::styled(SEARCH_FILES_PLACEHOLDER, muted)])
    } else {
        let (query, _) = cut_left(&dialog.query, width.saturating_sub(1));
        Line::from(vec![
            Span::styled(query, Style::default().fg(accent)),
            caret,
        ])
    };
    frame.render_widget(Paragraph::new(query), row_at(0));
    if h >= 2 {
        let status = if search_row_shows_status(&state.status) {
            list_dialog_status(state, palette)
        } else {
            search_status_line(dialog, palette)
        };
        frame.render_widget(Paragraph::new(clamp_line(status, width)), row_at(1));
    }
    if h >= 3 {
        let footer = Line::from(Span::styled(dialog.footer_hints(), muted));
        frame.render_widget(
            Paragraph::new(fit_list_dialog_footer(footer, width)),
            row_at(h - 1),
        );
    }
    let body_h = h.saturating_sub(3);
    state.layout.search_files_rows = body_h;
    if body_h == 0 {
        return;
    }
    let body = Rect {
        x: inner.x,
        y: inner.y + 2,
        width: inner.width,
        height: body_h,
    };
    let (list_area, preview_area) = if wide {
        let list_w = inner.width * 9 / 20;
        (
            Rect {
                width: list_w,
                ..body
            },
            Some(Rect {
                x: body.x + list_w,
                width: body.width - list_w,
                ..body
            }),
        )
    } else {
        (body, None)
    };
    let rows = match state.search_files.as_mut() {
        Some(dialog) => {
            let rows = dialog.result_rows();
            dialog.scroll_to_cursor(&rows, usize::from(body_h));
            rows
        }
        None => return,
    };
    let Some(dialog) = state.search_files.as_ref() else {
        return;
    };
    let FileIndexState::Ready(index) = &dialog.index else {
        return;
    };
    let lines = search_result_lines(dialog, index, &rows, palette, theme, list_area);
    frame.render_widget(Paragraph::new(lines), list_area);
    if let Some(preview_area) = preview_area {
        let block = Block::default()
            .borders(Borders::LEFT)
            .border_style(Style::default().fg(palette.border_dim))
            .padding(Padding::left(1));
        let preview_inner = block.inner(preview_area);
        frame.render_widget(block, preview_area);
        let lines = search_preview_lines(dialog, index, palette, theme, preview_inner);
        frame.render_widget(Paragraph::new(lines), preview_inner);
    }
}

/// `line` cut to `width` columns.
fn clamp_line(line: Line<'static>, width: usize) -> Line<'static> {
    Line::from(clamp_spans(line.spans, width))
}

/// Top-border chips for the match options: `Aa` case, `ab` whole word,
/// `.*` regex. An option that is on paints as a filled chip, an off one
/// as muted text.
fn search_option_chips(options: SearchOptions, palette: Palette, surface: Color) -> Line<'static> {
    let chip = |label: &str, on: bool| {
        if on {
            key_chip(label, palette.cursor, surface)
        } else {
            Span::styled(format!(" {label} "), Style::default().fg(palette.muted))
        }
    };
    Line::from(vec![
        chip("Aa", options.case_sensitive),
        Span::raw(" "),
        chip("ab", options.whole_word),
        Span::raw(" "),
        chip(".*", options.regex),
    ])
}

/// Status row of the search dialog: its [`SearchStatus`] parts joined by
/// ` · `; errors red, the skipped count dim, the rest muted.
fn search_status_line(dialog: &SearchFilesState, palette: Palette) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, part) in dialog.status().iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" · ", Style::default().fg(palette.muted)));
        }
        let style = if part.is_error() {
            Style::default().fg(palette.deleted)
        } else if part.is_dim() {
            Style::default()
                .fg(palette.muted)
                .add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(palette.muted)
        };
        spans.push(Span::styled(part.text(), style));
    }
    Line::from(spans)
}

/// Header text of `entry`: `(Some(checkout), path)` when the dialog
/// searches all repos (painted `repo › path`), else `(None, path)`.
fn search_file_label(
    dialog: &SearchFilesState,
    index: &FileIndex,
    entry: usize,
) -> Option<(Option<String>, String)> {
    let file = index.entries.get(entry)?;
    let repo =
        (dialog.scope() == QuickOpenScope::Workspace).then(|| index.checkout(file).to_string());
    Some((repo, file.rel().to_string()))
}

/// Results rows from [`SearchFilesState::scroll`]: file headers (`path`,
/// or `repo › path` across all repos) and hit rows `❯ <line>: <text>` with
/// every match in the search highlight colours.
fn search_result_lines(
    dialog: &SearchFilesState,
    index: &FileIndex,
    rows: &[SearchRow],
    palette: Palette,
    theme: ThemeId,
    area: Rect,
) -> Vec<Line<'static>> {
    let width = usize::from(area.width);
    let window = rows
        .iter()
        .skip(dialog.scroll)
        .take(usize::from(area.height));
    let number_w = window
        .clone()
        .filter_map(|row| match row {
            SearchRow::Hit(idx) => dialog.hits.get(*idx).map(|hit| hit.line),
            SearchRow::Header(_) => None,
        })
        .max()
        .map_or(1, |line| line.to_string().len());
    let filter = theme.pills().filter;
    let matched = Style::default()
        .fg(filter.fg)
        .bg(filter.bg)
        .add_modifier(Modifier::BOLD);
    let mut lines = Vec::new();
    for row in window {
        match *row {
            SearchRow::Header(entry) => {
                let Some((repo, rel)) = search_file_label(dialog, index, entry) else {
                    lines.push(Line::default());
                    continue;
                };
                let path = Style::default()
                    .fg(palette.heading)
                    .add_modifier(Modifier::BOLD);
                let full = match &repo {
                    Some(repo) => format!("{repo} › {rel}"),
                    None => rel.clone(),
                };
                let line = match repo {
                    Some(repo) if visible_width(&full) <= width => Line::from(vec![
                        Span::styled(repo, Style::default().fg(palette.repo)),
                        Span::styled(" › ", Style::default().fg(palette.muted)),
                        Span::styled(rel, path),
                    ]),
                    _ => Line::from(Span::styled(cut_left(&full, width).0, path)),
                };
                lines.push(line);
            }
            SearchRow::Hit(idx) => {
                let Some(hit) = dialog.hits.get(idx) else {
                    lines.push(Line::default());
                    continue;
                };
                let selected = idx == dialog.cursor;
                let row_bg = if selected {
                    palette.cursor_bg
                } else {
                    palette.panel
                };
                let text_fg = if selected {
                    palette.file
                } else {
                    palette.muted
                };
                let mut spans = vec![
                    Span::styled(
                        if selected { "❯ " } else { "  " },
                        Style::default().fg(if selected {
                            palette.cursor
                        } else {
                            palette.muted
                        }),
                    ),
                    Span::styled(
                        format!("{:>number_w$}: ", hit.line),
                        Style::default().fg(palette.muted),
                    ),
                ];
                let room = width.saturating_sub(help_spans_width(&spans));
                let (text, ranges) = hit_text_window(&hit.text, &hit.ranges, room);
                spans.extend(mark_byte_ranges(
                    vec![(text, Style::default().fg(text_fg))],
                    &ranges,
                    matched,
                ));
                let mut spans = clamp_spans(spans, width);
                for span in &mut spans {
                    if span.style.bg.is_none() {
                        span.style = span.style.bg(row_bg);
                    }
                }
                lines.push(Line::from(spans));
            }
        }
    }
    lines
}

/// Valid byte `ranges` of `text` as `usize`, in order.
fn hit_byte_ranges(text: &str, ranges: &[(u32, u32)]) -> Vec<(usize, usize)> {
    ranges
        .iter()
        .map(|&(start, end)| (start as usize, end as usize))
        .filter(|&(start, end)| {
            start < end
                && end <= text.len()
                && text.is_char_boundary(start)
                && text.is_char_boundary(end)
        })
        .collect()
}

/// Hit text to paint in `width` columns and its match byte ranges.
///
/// When the first match would end past `width`, the head is cut to a
/// leading `…` so that match starts about a quarter of the way in.
fn hit_text_window(
    text: &str,
    ranges: &[(u32, u32)],
    width: usize,
) -> (String, Vec<(usize, usize)>) {
    let ranges = hit_byte_ranges(text, ranges);
    let Some(&(first_start, first_end)) = ranges.first() else {
        return (text.to_string(), ranges);
    };
    if visible_width(&text[..first_end]) <= width {
        return (text.to_string(), ranges);
    }
    let lead = width / 4;
    let mut from = first_start;
    let mut used = 0usize;
    for (at, ch) in text[..first_start].char_indices().rev() {
        let mut buf = [0u8; 4];
        let w = visible_width(ch.encode_utf8(&mut buf));
        if used + w > lead {
            break;
        }
        used += w;
        from = at;
    }
    let ellipsis = '…'.len_utf8();
    let shifted = ranges
        .into_iter()
        .filter(|&(_, end)| end > from)
        .map(|(start, end)| (start.max(from) - from + ellipsis, end - from + ellipsis))
        .collect();
    (format!("…{}", &text[from..]), shifted)
}

/// `parts` as spans, with the bytes inside `ranges` (offsets across the
/// joined parts) painted `marked`. A range edge that is not a char
/// boundary of its part is ignored.
fn mark_byte_ranges(
    parts: Vec<(String, Style)>,
    ranges: &[(usize, usize)],
    marked: Style,
) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    let mut offset = 0usize;
    for (text, style) in parts {
        let len = text.len();
        let mut cuts = vec![0, len];
        for &(start, end) in ranges {
            for edge in [start, end] {
                if edge > offset && edge < offset + len && text.is_char_boundary(edge - offset) {
                    cuts.push(edge - offset);
                }
            }
        }
        cuts.sort_unstable();
        cuts.dedup();
        for pair in cuts.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            if from == to {
                continue;
            }
            let at = offset + from;
            let inside = ranges.iter().any(|&(start, end)| start <= at && at < end);
            out.push(Span::styled(
                text[from..to].to_string(),
                if inside { marked } else { style },
            ));
        }
        offset += len;
    }
    out
}

/// Syntax spans for `lines[window]` of the preview read `gen`, reused while
/// the window, read, and theme stay the same.
fn search_preview_spans(
    gen: u64,
    rel: &str,
    lines: &[String],
    theme: ThemeId,
    fallback: Color,
    window: std::ops::Range<usize>,
) -> FileSyntaxSpans {
    let key = (gen, theme, window.start, window.end);
    SEARCH_PREVIEW_SYNTAX_CACHE.with(|slot| {
        let mut cache = slot.borrow_mut();
        if let Some((hit_key, spans)) = cache.as_ref() {
            if *hit_key == key {
                return Arc::clone(spans);
            }
        }
        let spans = Arc::new(highlight_file_window(rel, lines, theme, fallback, window));
        *cache = Some((key, Arc::clone(&spans)));
        spans
    })
}

/// Preview pane: a `path:line` header, then the highlighted hit's file
/// around the hit line (centered when the file allows), syntax-highlighted
/// with a line-number gutter. The hit line has the cursor background; every
/// hit of that file in view has its matches in the search colours. A
/// binary, too large, or unreadable file is one dim line.
fn search_preview_lines(
    dialog: &SearchFilesState,
    index: &FileIndex,
    palette: Palette,
    theme: ThemeId,
    area: Rect,
) -> Vec<Line<'static>> {
    let width = usize::from(area.width);
    let height = usize::from(area.height);
    let Some(hit) = dialog.selected_hit() else {
        return Vec::new();
    };
    let Some((repo, rel)) = search_file_label(dialog, index, hit.entry) else {
        return Vec::new();
    };
    let header = match repo {
        Some(repo) => format!("{repo} › {rel}:{}", hit.line),
        None => format!("{rel}:{}", hit.line),
    };
    let mut out = vec![Line::from(Span::styled(
        cut_left(&header, width).0,
        Style::default().fg(palette.muted),
    ))];
    let body = dialog
        .preview
        .as_ref()
        .filter(|preview| {
            index
                .entries
                .get(hit.entry)
                .is_some_and(|entry| preview.is_for(index.checkout(entry), entry.rel()))
        })
        .and_then(|preview| preview.body.as_ref().map(|body| (preview.gen, body)));
    let dim = Style::default()
        .fg(palette.muted)
        .add_modifier(Modifier::DIM);
    let notice = |text: String, style: Style| {
        Line::from(Span::styled(fit_with_ellipsis(&text, width), style))
    };
    let (gen, lines) = match body {
        None => {
            out.push(notice(
                PREVIEW_LOADING.to_string(),
                Style::default().fg(palette.muted),
            ));
            return out;
        }
        Some((_, FileRead::Binary)) => {
            out.push(notice(PREVIEW_BINARY.to_string(), dim));
            return out;
        }
        Some((_, FileRead::TooLarge { .. })) => {
            out.push(notice(PREVIEW_TOO_LARGE.to_string(), dim));
            return out;
        }
        Some((_, FileRead::Failed(err))) => {
            out.push(notice(err.clone(), dim));
            return out;
        }
        Some((_, FileRead::Text { lines, .. }))
            if lines.is_empty() || (lines.len() == 1 && lines[0].is_empty()) =>
        {
            out.push(notice(EMPTY_FILE.to_string(), dim));
            return out;
        }
        Some((gen, FileRead::Text { lines, .. })) => (gen, lines),
    };
    let rows = height.saturating_sub(1);
    if rows == 0 {
        return out;
    }
    let focus = usize::try_from(hit.line.saturating_sub(1))
        .unwrap_or(usize::MAX)
        .min(lines.len() - 1);
    let end = (focus.saturating_sub(rows / 2) + rows).min(lines.len());
    let start = end.saturating_sub(rows);
    let spans = search_preview_spans(gen, &rel, lines, theme, palette.repo, start..end);
    let gutter = file_gutter_width(lines.len());
    let code_w = width.saturating_sub(gutter);
    let filter = theme.pills().filter;
    let matched = Style::default()
        .fg(filter.fg)
        .bg(filter.bg)
        .add_modifier(Modifier::BOLD);
    // Hits are in index order then line order: this file's hits in view.
    let in_view: Vec<&SearchHit> = dialog
        .hits
        .iter()
        .filter(|other| other.entry == hit.entry)
        .filter(|other| usize::try_from(other.line).is_ok_and(|line| line > start && line <= end))
        .collect();
    for line in start..end {
        let focused = line == focus;
        let mut row = vec![Span::styled(
            format!("{:>w$} ", line + 1, w = gutter.saturating_sub(1)),
            diff_gutter_style(palette),
        )];
        let code = spans.get(line - start).map(Vec::as_slice).unwrap_or(&[]);
        let parts: Vec<(String, Style)> = slice_styled_cols(code, 0, code_w)
            .into_iter()
            .map(|span| (span.text, Style::default().fg(span.fg)))
            .collect();
        let ranges: Vec<(usize, usize)> = in_view
            .iter()
            .filter(|other| usize::try_from(other.line).is_ok_and(|n| n == line + 1))
            .flat_map(|other| hit_byte_ranges(&other.text, &other.ranges))
            .collect();
        row.extend(mark_byte_ranges(parts, &ranges, matched));
        let mut row = clamp_spans(row, width);
        if focused {
            for span in &mut row {
                if span.style.bg.is_none() {
                    span.style = span.style.bg(palette.cursor_bg);
                }
            }
        }
        out.push(Line::from(row));
    }
    out
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
    // The status row is always there, so typing never grows the box.
    lines.push(Line::from(Span::styled(
        state.status.to_string(),
        Style::default().fg(state.status.kind().color(palette)),
    )));
    lines.push(Line::from(Span::styled(
        footer,
        Style::default().fg(palette.muted),
    )));
    frame.render_widget(Clear, area);
    frame.render_widget(
        Paragraph::new(lines)
            .block(overlay_block(accent, palette.panel))
            .wrap(Wrap { trim: false }),
        area,
    );
}

/// One painted row of the icon popover.
#[derive(Clone, Copy)]
enum PopoverRow {
    /// Section heading (index into the sections).
    Heading(usize),
    /// Section line (index into [`flat_lines`]).
    Line(usize),
    /// Blank row between sections.
    Gap,
    /// Pinned key hints.
    Footer,
}

/// Icon popover (peek or pinned) over the panes, hung from its icon.
///
/// Same rounded surface as Quick Open. Each section paints a heading (the
/// glyph in the row's colour and the catalog name), the meaning, fields
/// (muted label padded to the section's widest label, value in its role
/// colour), and action lines (palette title, key chip at the right edge;
/// a disabled one dims and names its gate reason when it fits). A pinned
/// popover marks its focused line with the cursor background and ends
/// with [`POPOVER_FOOTER`]. A box the frame cuts names the rows below the
/// shown ones on its bottom border. Records [`PopoverHit`] for clicks and
/// drags.
fn draw_popover(frame: &mut Frame<'_>, bounds: Rect, state: &mut AppState) {
    state.sync_popover_with_layout();
    let Some(popover) = state.popover.as_ref() else {
        return;
    };
    let sections = state.open_popover_sections();
    if sections.is_empty() || bounds.width < 8 || bounds.height < 3 {
        state.close_popover();
        return;
    }
    let pinned = popover.is_pinned();
    let anchor = popover
        .anchor
        .unwrap_or_else(|| popover_fallback_anchor(state));
    let lines = flat_lines(&sections);
    let focus = if pinned {
        focused_line(&lines, popover.focus_line)
    } else {
        None
    };
    let palette = state.theme.palette();
    let panel = palette.panel;
    let accent = palette.cursor;
    let muted = Style::default().fg(palette.muted);

    let mut rows = Vec::new();
    let mut line_section = Vec::new();
    for (index, section) in sections.iter().enumerate() {
        if index > 0 {
            rows.push(PopoverRow::Gap);
        }
        rows.push(PopoverRow::Heading(index));
        for _ in &section.lines {
            rows.push(PopoverRow::Line(line_section.len()));
            line_section.push(index);
        }
    }
    if pinned {
        rows.push(PopoverRow::Footer);
    }
    // Whether each section's target is the focused row, asked once per
    // section: only those action lines show their gate reasons.
    let gated: Vec<bool> = sections
        .iter()
        .map(|section| pinned || state.popover_target_focused(&section.target))
        .collect();
    let reasons: Vec<Option<String>> = lines
        .iter()
        .zip(&line_section)
        .map(|(line, &section)| match line {
            PopoverLine::Action(command) if gated[section] => {
                state.palette_disabled_reason(command)
            }
            _ => None,
        })
        .collect();
    // Field labels pad to the widest label of their own section.
    let label_widths: Vec<usize> = sections
        .iter()
        .map(|section| {
            section
                .lines
                .iter()
                .filter_map(|line| match line {
                    PopoverLine::Field { label, .. } => Some(visible_width(label)),
                    _ => None,
                })
                .max()
                .unwrap_or(0)
        })
        .collect();
    let label_width = |index: usize| label_widths[line_section[index]];
    let chip_width = |keys: &str| {
        if keys.is_empty() {
            0
        } else {
            visible_width(keys) + 3
        }
    };
    let natural = rows
        .iter()
        .map(|row| match *row {
            PopoverRow::Heading(index) => {
                let kind = sections[index].icon;
                visible_width(kind.glyph(state.ascii)) + 1 + visible_width(kind.spec().name)
            }
            PopoverRow::Line(index) => {
                2 + match lines[index] {
                    PopoverLine::Text(text) | PopoverLine::Note(text) => visible_width(text),
                    PopoverLine::Field { value, .. } => {
                        label_width(index) + 1 + visible_width(value)
                    }
                    PopoverLine::Action(command) => {
                        visible_width(command.title)
                            + reasons[index]
                                .as_deref()
                                .map_or(0, |why| 2 + visible_width(why))
                            + chip_width(command.keys)
                    }
                }
            }
            PopoverRow::Gap => 0,
            PopoverRow::Footer => visible_width(POPOVER_FOOTER),
        })
        .max()
        .unwrap_or(0);
    let width = u16::try_from(natural + 4)
        .unwrap_or(u16::MAX)
        .min(POPOVER_MAX_WIDTH)
        .min(bounds.width.saturating_sub(4))
        .max(8);
    let inner_width = usize::from(width.saturating_sub(4));
    let height = u16::try_from(rows.len() + 2).unwrap_or(u16::MAX);
    let rect = popover_rect(anchor, bounds, width, height);

    let painted: Vec<Line<'static>> = rows
        .iter()
        .map(|row| match *row {
            PopoverRow::Heading(index) => {
                let section = &sections[index];
                let bold = Modifier::BOLD;
                Line::from(clamp_spans(
                    vec![
                        Span::styled(
                            section.icon.glyph(state.ascii),
                            Style::default()
                                .fg(seg_role_color(section.role, palette))
                                .add_modifier(bold),
                        ),
                        Span::raw(" "),
                        Span::styled(
                            section.icon.spec().name,
                            Style::default().fg(palette.heading).add_modifier(bold),
                        ),
                    ],
                    inner_width,
                ))
            }
            PopoverRow::Line(index) => {
                let focused = focus == Some(index);
                let bg = focused.then_some(palette.cursor_bg);
                let marker = Span::styled(
                    if focused { "❯ " } else { "  " },
                    Style::default().fg(accent),
                );
                let mut spans = vec![marker];
                let mut chip = None;
                match lines[index] {
                    PopoverLine::Text(text) => {
                        spans.push(Span::styled(
                            text.clone(),
                            Style::default().fg(palette.file),
                        ));
                    }
                    PopoverLine::Note(text) => {
                        spans.push(Span::styled(text.clone(), muted));
                    }
                    PopoverLine::Field { label, value, role } => {
                        let pad = label_width(index).saturating_sub(visible_width(label));
                        spans.push(Span::styled(format!("{label}{} ", " ".repeat(pad)), muted));
                        spans.push(Span::styled(
                            value.clone(),
                            Style::default().fg(seg_role_color(*role, palette)),
                        ));
                    }
                    PopoverLine::Action(command) => {
                        let reason = reasons[index].as_deref();
                        let mut title = Style::default().fg(palette.file);
                        if reason.is_some() {
                            title = title.fg(palette.muted).add_modifier(Modifier::DIM);
                        }
                        spans.push(Span::styled(command.title, title));
                        let chip_cols = chip_width(command.keys);
                        if let Some(why) = reason {
                            let used = help_spans_width(&spans);
                            if used + 2 + visible_width(why) + chip_cols <= inner_width {
                                spans.push(Span::styled(
                                    format!("  {why}"),
                                    muted.add_modifier(Modifier::DIM),
                                ));
                            }
                        }
                        if !command.keys.is_empty() {
                            let chip_bg = if reason.is_some() {
                                palette.muted
                            } else {
                                accent
                            };
                            chip = Some(key_chip(command.keys, chip_bg, panel));
                        }
                    }
                }
                // The chip sits at the right edge, one column clear of the
                // title, which is cut to make room. With no room it drops.
                let chip = chip.filter(|chip| chip.width() + 4 <= inner_width);
                let room = chip
                    .as_ref()
                    .map_or(inner_width, |chip| inner_width - chip.width() - 1);
                let mut spans = clamp_spans(spans, room);
                if let Some(chip) = chip {
                    spans.push(Span::raw(" "));
                    spans.push(chip);
                }
                if let Some(bg) = bg {
                    for span in &mut spans {
                        if span.style.bg.is_none() {
                            span.style = span.style.bg(bg);
                        }
                    }
                }
                Line::from(spans)
            }
            PopoverRow::Gap => Line::default(),
            PopoverRow::Footer => {
                fit_list_dialog_footer(Line::from(Span::styled(POPOVER_FOOTER, muted)), inner_width)
            }
        })
        .collect();

    // A box cut by the frame scrolls its body around the focused line
    // (a peek shows the top); a pinned footer stays on the last row.
    let footer = usize::from(pinned);
    let body = rows.len() - footer;
    let body_height = usize::from(rect.height.saturating_sub(2)).saturating_sub(footer);
    let focus_row = focus
        .and_then(|line| {
            rows.iter()
                .position(|row| matches!(row, PopoverRow::Line(index) if *index == line))
        })
        .unwrap_or(0);
    let (start, shown) = visible_window(body, focus_row, body_height);
    let mut block = overlay_block(accent, panel);
    let below = body - (start + shown);
    if below > 0 {
        // The rows under the cut: the same glyph the graph footer uses
        // for message lines below.
        let more = format!(
            " {}{below} more ",
            IconKind::GraphMoreBelow.glyph(state.ascii)
        );
        if visible_width(&more) + 2 <= usize::from(rect.width) {
            block = block.title_bottom(
                Line::from(Span::styled(more, Style::default().fg(accent))).right_aligned(),
            );
        }
    }
    frame.render_widget(Clear, rect);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    let shown_rows = (start..start + shown).chain(body..rows.len());
    let mut painted: Vec<Option<Line<'static>>> = painted.into_iter().map(Some).collect();
    let mut hit_lines = Vec::new();
    for (offset, index) in shown_rows.enumerate() {
        let (row, Some(line)) = (&rows[index], painted[index].take()) else {
            continue;
        };
        let Ok(offset) = u16::try_from(offset) else {
            break;
        };
        if offset >= inner.height {
            break;
        }
        let y = inner.y + offset;
        frame.render_widget(Paragraph::new(line), Rect::new(inner.x, y, inner.width, 1));
        if let PopoverRow::Line(index) = *row {
            if lines[index].focusable() {
                hit_lines.push((y, index));
            }
        }
    }
    state.layout.popover = Some(PopoverHit {
        rect,
        inner,
        lines: hit_lines,
    });
}

/// Where a popover with no painted icon hangs: the focused tree row, else
/// the top of the right pane.
fn popover_fallback_anchor(state: &AppState) -> Rect {
    let layout = &state.layout;
    if state.list_focus_target() == ListFocusTarget::Tree {
        let id = state.rows.get(state.cursor).map(|row| row.id.as_str());
        let index = state
            .painted_tree_rows()
            .iter()
            .position(|row| Some(row.id.as_str()) == id);
        if let Some(offset) = index.and_then(|index| index.checked_sub(layout.list_offset)) {
            let y = layout
                .tree_y
                .saturating_add(u16::try_from(offset).unwrap_or(u16::MAX));
            return Rect::new(layout.tree_x.saturating_add(2), y, 1, 1);
        }
    }
    Rect::new(layout.diff_content_x, layout.right_y, 1, 1)
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

/// Paint `body` from the top of a text dialog and pin `footer` to its last
/// inner rows. A body taller than the room left is cut at the bottom.
fn paint_text_dialog(
    frame: &mut Frame<'_>,
    area: Rect,
    block: Block<'static>,
    body: Vec<Line<'static>>,
    footer: Vec<Line<'static>>,
) {
    frame.render_widget(Clear, area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let footer_h = (footer.len() as u16).min(inner.height);
    let body_h = inner.height - footer_h;
    frame.render_widget(
        Paragraph::new(body).wrap(Wrap { trim: false }),
        Rect {
            height: body_h,
            ..inner
        },
    );
    frame.render_widget(
        Paragraph::new(footer),
        Rect {
            y: inner.y + body_h,
            height: footer_h,
            ..inner
        },
    );
}

fn draw_comment(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(prompt) = state.comment.as_ref() else {
        return;
    };
    if area.width == 0 || area.height == 0 {
        return;
    }
    let palette = state.theme.palette();
    let panel = palette.panel;
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
    let footer = vec![
        Line::from(Span::styled(
            comment_overlay_footer_save(prompt.resolved),
            Style::default().fg(palette.muted),
        )),
        Line::from(Span::styled(
            COMMENT_OVERLAY_FOOTER_EDIT,
            Style::default().fg(palette.muted),
        )),
    ];
    paint_text_dialog(frame, area, overlay_block(accent, panel), lines, footer);
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
    let footer = vec![Line::from(Span::styled(
        "Esc close",
        Style::default().fg(palette.muted),
    ))];
    paint_text_dialog(
        frame,
        area,
        overlay_block(accent, palette.panel),
        lines,
        footer,
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
    use crate::tui::pull_request::{PrLookup, PullRequest};
    use crate::tui::split::SplitDrag;
    use crate::tui::state::{AppState, PrBadgeHit};
    use crate::tui::tree::{build_tree, flatten_with, visible_for_tree};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::collections::HashSet;
    use std::path::{Path, PathBuf};
    use workspace_status_graph::{graph_gutter_cap, Commit, GraphModel, GraphRow};

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
        let palette = state.theme.palette();
        // The current hit is the paired del row; its first glyph is the
        // changed word `old`, which the cursor tints.
        let cursor_bg = palette.cursor_tint(palette.diff_del_word_bg, palette.cursor_bg);
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

    /// Assert row `y` of a paired line keeps its changed `word` on the
    /// tinted word bg, a shade apart from the tinted row bg under the
    /// unchanged `let total = price * `, and that the word is not on the
    /// flat `overlay`.
    fn assert_tinted_word_row(
        buf: &ratatui::buffer::Buffer,
        y: u16,
        word: &str,
        (row_bg, word_bg): (Color, Color),
        overlay: Color,
        palette: Palette,
        ctx: &str,
    ) {
        let tinted_row = palette.cursor_tint(row_bg, overlay);
        let tinted_word = palette.cursor_tint(word_bg, overlay);
        assert_ne!(tinted_word, overlay, "{ctx}: word tint must not be flat");
        assert_ne!(tinted_word, tinted_row, "{ctx}: word tint vs row tint");
        assert_eq!(
            cols_with_bg(buf, y, tinted_word),
            needle_cols(buf, y, word),
            "{ctx}: only `{word}` sits on the tinted word bg"
        );
        assert!(
            needle_cells(buf, y, "let total = price * ")
                .iter()
                .all(|cell| cell.bg == tinted_row),
            "{ctx}: unchanged text sits on the tinted row bg"
        );
    }

    /// Assert the line-number gutter, rule, and sign of inline row `y` sit
    /// on the flat `overlay` with their own fg, and the code on the tinted
    /// `row_bg`.
    fn assert_flat_chrome(
        buf: &ratatui::buffer::Buffer,
        y: u16,
        x0: u16,
        (row_bg, sign_fg): (Color, Color),
        overlay: Color,
        palette: Palette,
        ctx: &str,
    ) {
        let code_x = find_cell_col(buf, y, "let total").expect("code");
        let sign = &buf[(code_x - 1, y)];
        assert_eq!((sign.bg, sign.fg), (overlay, sign_fg), "{ctx}: sign cell");
        let digits: Vec<u16> = (x0..code_x - 1)
            .filter(|&x| {
                let sym = buf[(x, y)].symbol();
                !sym.is_empty() && sym.chars().all(|c| c.is_ascii_digit())
            })
            .collect();
        assert!(!digits.is_empty(), "{ctx}: line number in the gutter");
        for x in x0..code_x - 1 {
            assert_eq!(buf[(x, y)].bg, overlay, "{ctx}: gutter col {x}");
        }
        for x in digits {
            assert_eq!(buf[(x, y)].fg, palette.muted, "{ctx}: line number col {x}");
        }
        let tinted_row = palette.cursor_tint(row_bg, overlay);
        assert_eq!(buf[(code_x, y)].bg, tinted_row, "{ctx}: code cell");
    }

    #[test]
    fn tinted_cursor_rows_keep_flat_gutter_and_sign_colours() {
        for id in crate::tui::theme::THEME_IDS {
            let palette = id.palette();
            for (needle, row_bg, sign_fg) in [
                ("price * count", palette.diff_add_bg, palette.added),
                ("price * qty", palette.diff_del_bg, palette.deleted),
            ] {
                for (focus, overlay) in [
                    (FocusPane::Right, palette.cursor_bg),
                    (FocusPane::Left, palette.cursor_bg_inactive),
                ] {
                    let mut state =
                        word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
                    state.theme = id;
                    state.focus = focus;
                    state.diff_cursor = diff_row_index(&state, needle);
                    let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
                    draw_state(&mut terminal, &mut state);
                    let buf = terminal.backend().buffer();
                    let text = buffer_text(&terminal);
                    let y = first_row_with(buf, needle).expect(needle);
                    assert_flat_chrome(
                        buf,
                        y,
                        state.layout.diff_content_x + 1,
                        (row_bg, sign_fg),
                        overlay,
                        palette,
                        &format!("{id:?} {needle} {focus:?}\n{text}"),
                    );
                }
            }
        }
    }

    #[test]
    fn focused_cursor_row_tints_the_word_bg_inline() {
        for id in crate::tui::theme::THEME_IDS {
            let mut state =
                word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
            state.theme = id;
            let palette = id.palette();
            state.focus = FocusPane::Right;
            state.diff_cursor = diff_row_index(&state, "price * count");
            let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let add_y = first_row_with(buf, "price * count").expect("add line");
            let del_y = first_row_with(buf, "price * qty").expect("del line");
            let ctx = format!("{id:?} cursor row\n{text}");
            assert_tinted_word_row(
                buf,
                add_y,
                "count",
                (palette.diff_add_bg, palette.diff_add_word_bg),
                palette.cursor_bg,
                palette,
                &ctx,
            );
            assert_eq!(
                cols_with_bg(buf, del_y, palette.diff_del_word_bg),
                needle_cols(buf, del_y, "qty"),
                "{id:?} non-cursor row keeps its plain word bg:\n{text}"
            );
        }
    }

    #[test]
    fn focused_cursor_row_tints_the_word_bg_on_both_split_sides() {
        for id in crate::tui::theme::THEME_IDS {
            let mut state =
                word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::SideBySide);
            state.theme = id;
            let palette = id.palette();
            state.focus = FocusPane::Right;
            let mut terminal = Terminal::new(TestBackend::new(220, 24)).unwrap();
            // The first paint settles the split layout and its paired rows.
            draw_state(&mut terminal, &mut state);
            assert_eq!(state.diff_layout(), DiffMode::SideBySide);
            state.diff_cursor = diff_row_index(&state, "price * count");
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let y = first_row_with(buf, "price * count").expect("split row");
            assert!(buf_line(buf, y).contains("price * qty"), "{text}");
            let ctx = format!("{id:?} split cursor row\n{text}");
            for (word, bgs) in [
                ("qty", (palette.diff_del_bg, palette.diff_del_word_bg)),
                ("count", (palette.diff_add_bg, palette.diff_add_word_bg)),
            ] {
                let needle = format!("let total = price * {word}");
                let start = find_cell_col(buf, y, &needle).expect("code");
                let prefix_cells: Vec<_> = (0..20u16).map(|i| &buf[(start + i, y)]).collect();
                let tinted_row = palette.cursor_tint(bgs.0, palette.cursor_bg);
                let tinted_word = palette.cursor_tint(bgs.1, palette.cursor_bg);
                assert!(
                    prefix_cells.iter().all(|c| c.bg == tinted_row),
                    "{ctx}: `{word}` side unchanged text on tinted row bg"
                );
                assert!(
                    needle_cells(buf, y, word)
                        .iter()
                        .all(|c| c.bg == tinted_word),
                    "{ctx}: `{word}` on tinted word bg"
                );
                assert_eq!(cols_with_bg(buf, y, tinted_word), needle_cols(buf, y, word));
                assert_ne!(tinted_word, palette.cursor_bg, "{ctx}");
                assert_ne!(tinted_word, tinted_row, "{ctx}");
            }
        }
    }

    #[test]
    fn focused_wrapped_continuation_row_keeps_the_tinted_word_bg() {
        for id in [ThemeId::TokyoNight, ThemeId::GruvboxDark] {
            let shared = format!("let total = {}", "price + ".repeat(14));
            let mut state = word_diff_state(
                word_diff_lines(&format!("{shared}qty;"), &format!("{shared}count;")),
                DiffMode::Inline,
            );
            state.theme = id;
            state.diff_wrap = true;
            state.focus = FocusPane::Right;
            state.diff_cursor = diff_row_index(&state, "count;");
            let palette = id.palette();
            let tinted_row = palette.cursor_tint(palette.diff_add_bg, palette.cursor_bg);
            let tinted_word = palette.cursor_tint(palette.diff_add_word_bg, palette.cursor_bg);
            let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let add_first_y = (0..buf.area().height)
                .find(|&y| {
                    buf_line(buf, y).contains("let total = price")
                        && cols_with_bg(buf, y, tinted_row).len() > 1
                })
                .expect("cursor add line first row");
            let word_rows = rows_with_bg(buf, tinted_word);
            assert!(!word_rows.is_empty(), "{id:?}:\n{text}");
            assert!(
                word_rows.iter().all(|&y| y > add_first_y),
                "{id:?} the changed word wraps onto a continuation row: {word_rows:?}\n{text}"
            );
            let painted: String = word_rows
                .iter()
                .flat_map(|&y| {
                    cols_with_bg(buf, y, tinted_word)
                        .into_iter()
                        .map(move |x| buf[(x, y)].symbol().to_string())
                })
                .collect();
            assert_eq!(painted, "count", "{id:?}:\n{text}");
            let y = word_rows[0];
            let first_word = cols_with_bg(buf, y, tinted_word)[0];
            let x0 = state.layout.diff_content_x + 1;
            let code_x = cols_with_bg(buf, y, tinted_row)[0];
            assert!(code_x > x0, "{id:?} continuation gutter:\n{text}");
            for x in x0..code_x {
                assert_eq!(
                    buf[(x, y)].bg,
                    palette.cursor_bg,
                    "{id:?} continuation gutter col {x} on the flat cursor bar:\n{text}"
                );
            }
            for x in code_x..first_word {
                assert_eq!(
                    buf[(x, y)].bg,
                    tinted_row,
                    "{id:?} continuation code col {x} on the tinted row bg:\n{text}"
                );
            }
        }
    }

    #[test]
    fn visual_line_row_tints_the_word_bg() {
        for id in crate::tui::theme::THEME_IDS {
            let mut state =
                word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
            state.theme = id;
            let palette = id.palette();
            state.focus = FocusPane::Right;
            let count_row = diff_row_index(&state, "price * count");
            state.diff_cursor = count_row;
            state.dispatch(Action::DiffVisualStart);
            assert_eq!(state.diff_visual_anchor, Some(count_row));
            // Cursor moves off: the paired row stays in the selection only.
            state.diff_cursor = diff_row_index(&state, "fresh_only");
            let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            assert!(state.diff_visual_contains(count_row));
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let add_y = first_row_with(buf, "price * count").expect("add line");
            assert_tinted_word_row(
                buf,
                add_y,
                "count",
                (palette.diff_add_bg, palette.diff_add_word_bg),
                palette.cursor_bg,
                palette,
                &format!("{id:?} visual row\n{text}"),
            );
        }
    }

    #[test]
    fn unfocused_selected_row_tints_the_word_bg_with_the_inactive_cursor() {
        for id in crate::tui::theme::THEME_IDS {
            let mut state =
                word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
            state.theme = id;
            let palette = id.palette();
            assert_eq!(state.focus, FocusPane::Left);
            state.diff_cursor = diff_row_index(&state, "price * count");
            let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let add_y = first_row_with(buf, "price * count").expect("add line");
            assert_tinted_word_row(
                buf,
                add_y,
                "count",
                (palette.diff_add_bg, palette.diff_add_word_bg),
                palette.cursor_bg_inactive,
                palette,
                &format!("{id:?} unfocused selected row\n{text}"),
            );
        }
    }

    #[test]
    fn search_match_off_the_cursor_still_paints_the_pill_over_word_bg() {
        for id in [ThemeId::TokyoNight, ThemeId::Dracula] {
            let mut state =
                word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
            state.theme = id;
            state.focus = FocusPane::Right;
            state.dispatch(super::super::action::Action::SearchStart);
            for c in "let total".chars() {
                state.dispatch(super::super::action::Action::SearchChar(c));
            }
            assert_eq!(state.search_target, SearchPane::Diff);
            let count_row = diff_row_index(&state, "price * count");
            assert_ne!(
                state.diff_cursor, count_row,
                "match must sit off the cursor"
            );
            let pill = id.pills().filter;
            let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let add_y = first_row_with(buf, "price * count").expect("add line");
            assert!(
                needle_cells(buf, add_y, COUNT_LINE)
                    .iter()
                    .all(|c| (c.bg, c.fg) == (pill.bg, pill.fg)),
                "{id:?} search pill replaces the whole row:\n{text}"
            );
        }
    }

    #[test]
    fn focused_context_row_keeps_the_flat_cursor_bg() {
        for id in crate::tui::theme::THEME_IDS {
            let mut state =
                word_diff_state(word_diff_lines(QTY_LINE, COUNT_LINE), DiffMode::Inline);
            state.theme = id;
            let palette = id.palette();
            state.focus = FocusPane::Right;
            state.diff_cursor = diff_row_index(&state, "let keep");
            let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let y = first_row_with(buf, "let keep = 0;").expect("context line");
            assert!(
                needle_cells(buf, y, "let keep = 0;")
                    .iter()
                    .all(|c| c.bg == palette.cursor_bg),
                "{id:?} context cursor row is the flat cursor bar:\n{text}"
            );
        }
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
    fn right_graph_footer_wraps_at_the_painted_width_and_keeps_a_fixed_height() {
        let mut state = two_pane_graph_state();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let fixed = state.graph_chrome();
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
        // The footer height is fixed: subject (2) + blank + b1 + b2 + meta
        // does not change it, nor the list height.
        assert_eq!(state.graph_chrome(), fixed);
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
        let footer_h =
            commit_detail_footer_height(state.commit_detail_footer_request(), layout.tree_height);
        assert_eq!(
            footer_h as usize,
            1 + 2 + workspace_status_graph::COMMIT_MSG_LINES_DEFAULT,
            "rule + title + meta + N message rows for a one-line body"
        );
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
        // The tallest N, so the footer outgrows the shortest pane.
        state.commit_msg_lines = workspace_status_graph::COMMIT_MSG_LINES_MAX;
        let mut terminal = Terminal::new(TestBackend::new(120, MIN_TERM_ROWS)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let layout = &state.layout;
        let footer_len = state.commit_detail_footer_request();
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

    /// The depth-2 commit-files footer keeps one fixed height whatever the
    /// message: a one-line subject pads, a long body clips with `…`, in
    /// both expand states, and `+` grows it by one row.
    #[test]
    fn depth_2_left_footer_height_is_fixed_for_short_and_long_messages() {
        let long = (0..30)
            .map(|i| format!("{FOOTER_BODY_TOKEN}{i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        let list_height = |body: &str, expand: bool, lines: usize| {
            let mut state = two_pane_commit_diff_state();
            set_seed_commit_body(&mut state, body);
            state.commit_msg_expand = expand;
            state.commit_msg_lines = lines;
            let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let footer = state.commit_detail_footer_lines(state.layout.tree_width as usize);
            assert_eq!(footer.len(), state.commit_detail_footer_request());
            let left = left_inner_text(&terminal, &state);
            (
                state.layout.tree_height,
                state.layout.files_list_height,
                left,
            )
        };
        let n = workspace_status_graph::COMMIT_MSG_LINES_DEFAULT;
        let (pane, short_h, _) = list_height("", true, n);
        let (_, long_h, left) = list_height(&long, true, n);
        assert_eq!(short_h, long_h, "expanded: same list height");
        assert_eq!(
            short_h as usize,
            pane as usize - (1 + 2 + n),
            "rule + title + meta + N"
        );
        assert!(left.contains('…'), "a long body clips with …:\n{left}");
        assert!(
            !left.contains(&format!("{FOOTER_BODY_TOKEN}{n:02}")),
            "rows past N stay hidden:\n{left}"
        );
        let (_, short_c, _) = list_height("", false, n);
        let (_, long_c, _) = list_height(&long, false, n);
        assert_eq!(short_c, long_c, "collapsed: same list height");
        assert_eq!(short_c, pane - 3, "collapsed: rule + title + subtitle");
        let (_, grown, _) = list_height("", true, n + 1);
        assert_eq!(grown + 1, short_h, "one more message row");
    }

    /// Commit files beside a file diff, with `n` files so the list
    /// overflows any test pane.
    fn commit_diff_state_with_files(n: usize) -> AppState {
        let mut state = two_pane_files_state();
        let files: Vec<super::super::drill::CommitFile> = (0..n)
            .map(|i| super::super::drill::CommitFile {
                status: "M".into(),
                path: format!("f{i:02}.txt"),
                old_path: None,
                stat: None,
            })
            .collect();
        state.open_commit_diff(
            "app".into(),
            super::super::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            files,
            0,
            "f00.txt".into(),
            super::super::diff::DiffContent::from_lines(vec!["@@ -1,1 +1,1 @@".into()]),
        );
        state
    }

    /// The depth-2 commit-files footer starts with a full-width rule
    /// (`▁`, `_` in ASCII mode) in `border_dim`, in paint and terminal
    /// mode, expanded and collapsed. The list gives up one row for it and
    /// the footer body under it is unchanged.
    #[test]
    fn commit_detail_footer_rule_sits_between_list_and_footer() {
        for background in [BackgroundMode::Paint, BackgroundMode::Terminal] {
            for ascii in [false, true] {
                for expand in [true, false] {
                    let case = format!("{background:?} ascii={ascii} expand={expand}");
                    let mut state = two_pane_commit_diff_state();
                    set_seed_commit_body(&mut state, FOOTER_BODY_TOKEN);
                    state.background = background;
                    state.ascii = ascii;
                    state.commit_msg_expand = expand;
                    state.focus = FocusPane::Left;
                    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
                    draw_state(&mut terminal, &mut state);
                    let l = state.layout.clone();
                    let body_len = state.commit_detail_footer_request();
                    assert_eq!(
                        l.files_list_height as usize,
                        l.tree_height as usize - body_len - 1,
                        "{case}: the list gives up one row"
                    );
                    let rule_y = l.files_list_y + l.files_list_height;
                    let want = if ascii { "_" } else { "\u{2581}" };
                    let palette = state.theme.palette();
                    let buf = terminal.backend().buffer();
                    for x in l.tree_x..l.tree_x + l.tree_width {
                        assert_eq!(buf[(x, rule_y)].symbol(), want, "{case}: x={x}");
                        assert_eq!(buf[(x, rule_y)].fg, palette.border_dim, "{case}: x={x}");
                    }
                    assert_ne!(
                        buf[(l.tree_x, rule_y - 1)].symbol(),
                        want,
                        "{case}: the row above the rule is the list"
                    );
                    let footer = state.commit_detail_footer_lines(l.tree_width as usize);
                    let painted = region_text(
                        &terminal,
                        l.tree_x,
                        rule_y + 1,
                        l.tree_width,
                        body_len as u16,
                    );
                    for (i, (row, line)) in painted.lines().zip(&footer).enumerate() {
                        // The pane clips a long line at its width.
                        let clipped: String = line.chars().take(l.tree_width as usize).collect();
                        assert_eq!(row.trim_end(), clipped.trim_end(), "{case}: body row {i}");
                    }
                    assert_eq!(rule_y + 1 + body_len as u16, l.tree_y + l.tree_height);
                }
            }
        }
    }

    /// A footer capped to one row is the rule only; a zero-row footer
    /// paints nothing.
    #[test]
    fn commit_detail_footer_is_rule_only_or_nothing_on_tiny_panes() {
        assert_eq!(commit_detail_footer_height(4, 0), 0);
        assert_eq!(commit_detail_footer_height(4, 1), 1);
        assert_eq!(
            commit_detail_footer_height(4, 2),
            1,
            "rule only, one list row"
        );
        assert_eq!(commit_detail_footer_height(4, 3), 2);
        assert_eq!(commit_detail_footer_height(4, 40), 5, "rule + 4 body rows");
        assert_eq!(commit_detail_footer_height(usize::MAX, 40), 39);

        let mut state = two_pane_commit_diff_state();
        state.ascii = true;
        let mut terminal = Terminal::new(TestBackend::new(30, 4)).unwrap();
        terminal
            .draw(|frame| {
                draw_commit_detail(
                    frame,
                    Rect {
                        x: 0,
                        y: 0,
                        width: 30,
                        height: 2,
                    },
                    &mut state,
                    0,
                    0,
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        assert!(
            buf_line(buf, 0).contains("README.md"),
            "{}",
            buf_line(buf, 0)
        );
        assert_eq!(buf_line(buf, 1), "_".repeat(30), "rule only");
        assert_eq!(buf_line(buf, 2).trim(), "", "nothing under the pane");

        let mut terminal = Terminal::new(TestBackend::new(30, 2)).unwrap();
        terminal
            .draw(|frame| {
                draw_commit_detail(
                    frame,
                    Rect {
                        x: 0,
                        y: 0,
                        width: 30,
                        height: 0,
                    },
                    &mut state,
                    0,
                    0,
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();
        for y in 0..2 {
            assert_eq!(buf_line(buf, y).trim(), "", "zero-height footer row {y}");
        }
    }

    /// A click on the commit-files footer rule selects nothing; the last
    /// list row above it still selects the file painted there.
    #[test]
    fn commit_detail_click_on_the_rule_selects_nothing() {
        for background in [BackgroundMode::Paint, BackgroundMode::Terminal] {
            let mut state = commit_diff_state_with_files(60);
            state.background = background;
            state.focus = FocusPane::Left;
            let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
            draw_state(&mut terminal, &mut state);
            let l = state.layout.clone();
            let rule_y = l.files_list_y + l.files_list_height;
            let col = l.tree_x + 4;
            assert_eq!(state.commit_files_cursor(), 0);
            state.dispatch(Action::Click { col, row: rule_y });
            assert_eq!(state.commit_files_cursor(), 0, "{background:?}: rule row");
            let last = rule_y - 1;
            let label = buf_line(terminal.backend().buffer(), last);
            let idx = l.files_list_offset + usize::from(l.files_list_height) - 1;
            let want = state.painted_commit_file_rows()[idx].id.clone();
            state.dispatch(Action::Click { col, row: last });
            let live = state.commit_file_rows();
            assert_eq!(
                live[state.commit_files_cursor()].id,
                want,
                "{background:?}: last list row {label}"
            );
        }
    }

    /// Long body on the seed commit and enough commits that the graph list
    /// overflows the pane.
    fn tall_graph_state() -> AppState {
        let mut state = two_pane_graph_state();
        if let Some(model) = state.graph.as_mut() {
            let seed = model.commits[0].clone();
            model.commits = (0..60)
                .map(|i| Commit {
                    id: format!("{i:040x}"),
                    subject: format!("commit {i:02}"),
                    body: (0..30)
                        .map(|b| format!("BODY{b:02}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                    ..seed.clone()
                })
                .collect();
            model.head_id = Some(format!("{:040x}", 0));
        }
        state.graph_cursor = 1;
        state
    }

    /// The graph selection footer starts with a full-width rule in
    /// `border_dim` (`_` in ASCII mode), in both modes. The recorded footer
    /// hit box starts on the rule, the list scrollbar ends above it, and a
    /// click on the rule selects nothing while the last list row still
    /// selects its commit.
    #[test]
    fn graph_footer_rule_tops_the_footer_and_takes_no_click() {
        for background in [BackgroundMode::Paint, BackgroundMode::Terminal] {
            for ascii in [false, true] {
                let case = format!("{background:?} ascii={ascii}");
                let mut state = tall_graph_state();
                state.background = background;
                state.ascii = ascii;
                let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
                draw_state(&mut terminal, &mut state);
                let l = state.layout.clone();
                let chrome = state.graph_chrome_in(l.diff_pane_height);
                assert_eq!(chrome, state.graph_chrome(), "{case}");
                let list_top = l.right_y + u16::from(chrome.header);
                let rule_y = list_top + chrome.list_height;
                assert_eq!(
                    rule_y + chrome.footer_height,
                    l.right_y + l.diff_pane_height,
                    "{case}: footer ends on the pane bottom"
                );
                let want = if ascii { "_" } else { "\u{2581}" };
                let palette = state.theme.palette();
                let buf = terminal.backend().buffer();
                for x in l.diff_content_x..l.diff_content_x + l.diff_pane_width {
                    assert_eq!(buf[(x, rule_y)].symbol(), want, "{case}: x={x}");
                    assert_eq!(buf[(x, rule_y)].fg, palette.border_dim, "{case}: x={x}");
                }
                let body = buf_line(buf, rule_y + 1);
                // Row 1 is the first commit (row 0 is the uncommitted row).
                assert!(
                    body.contains("commit 00"),
                    "{case}: subject under the rule: {body}"
                );
                assert_eq!(l.graph_footer_y, Some(rule_y), "{case}: footer hit box");
                assert_eq!(l.graph_footer_height, chrome.footer_height, "{case}");
                assert_eq!(l.graph_scrollbar_y, list_top, "{case}");
                assert_eq!(l.graph_scrollbar_height, chrome.list_height, "{case}");
                assert!(l.graph_footer_scroll_max > 0, "{case}");
                assert_eq!(
                    l.graph_footer_scroll_max,
                    footer_message_scroll_max(
                        state.graph_footer_line_count(l.diff_pane_width as usize),
                        chrome.footer_body_height(),
                    ),
                    "{case}"
                );

                let col = l.diff_content_x + 2;
                state.dispatch(Action::Click { col, row: rule_y });
                assert_eq!(state.graph_cursor, 1, "{case}: rule row selects nothing");
                let glyphs = if ascii {
                    &workspace_status_graph::ASCII
                } else {
                    &workspace_status_graph::UNICODE
                };
                let painted = workspace_status_graph::paint_model(
                    state.graph.as_ref().unwrap(),
                    glyphs,
                    None,
                );
                let last = rule_y - 1;
                let line =
                    &painted[state.graph_scroll as usize + usize::from(chrome.list_height) - 1];
                state.dispatch(Action::Click { col, row: last });
                let idx = line.row_index.expect("last list row maps to a graph row");
                assert_eq!(state.graph_cursor, idx, "{case}: last list row");
                assert_ne!(idx, 1, "{case}: the click moved the cursor");
            }
        }
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

    /// At 140×40 every key row of the help dialog paints without scrolling
    /// (only the icon legend sits below the fold), the panes keep their
    /// full height under it, and no column's text runs into the next column.
    #[test]
    fn help_columns_keep_a_gutter_and_the_panes_rows() {
        use super::super::help::{
            help_body_line_count, help_column_widths, help_legend_line_count, HELP_GROUPS,
        };
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
            overlay_rows <= usize::from(state.layout.pane_height),
            "help takes {overlay_rows} rows:\n{text}"
        );
        let inner = help_inner_width(136);
        let key_rows = help_body_line_count(HELP_GROUPS, &help_column_widths(HELP_GROUPS, inner));
        let legend_rows = help_legend_line_count(inner);
        assert!(
            state.layout.help_scroll_max > 0 && state.layout.help_scroll_max <= legend_rows,
            "only legend rows scroll: {}\n{text}",
            state.layout.help_scroll_max
        );
        assert!(footer > header + key_rows, "{text}");
        assert_eq!(
            state.layout.tree_height,
            40 - 3 - 2,
            "panes keep every row under the dialog:\n{text}"
        );
        assert!(text.contains("quit (press twice)"), "{text}");
        assert!(text.contains("apply/pop/drop"), "{text}");

        // The box sits at x = 2; border + padding put the first column at x = 4.
        let widths = help_column_widths(HELP_GROUPS, inner);
        let mut starts = vec![4usize];
        for width in &widths[..widths.len() - 1] {
            starts.push(starts.last().unwrap() + width);
        }
        let buf = terminal.backend().buffer();
        for y in header + 1..=header + key_rows {
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

    /// Cells `x0..x1` of buffer row `y`.
    fn buffer_row(terminal: &Terminal<TestBackend>, y: u16, x0: u16, x1: u16) -> String {
        let buf = terminal.backend().buffer();
        (x0..x1).map(|x| buf[(x, y)].symbol()).collect()
    }

    /// On a compare tab help paints centered over the panes at the box
    /// width: the rows `help_status_lines(box, true)` reserves when they
    /// fit, and on a short terminal every body row is reachable by scroll.
    #[test]
    fn compare_help_paints_centered_and_scrolls_every_body_row() {
        use super::super::chrome::{dialog_height, dialog_rect, dialog_width, DialogKind};
        use super::super::help::{
            help_body_line_count, help_legend_line_count, help_status_lines, HELP_COMPARE_GROUPS,
        };
        // Box widths 64, 100, and 140.
        for cols in [68u16, 104, 144] {
            let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
            let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
            state
                .tabs
                .open_or_focus("alpha".into(), "main".into(), "HEAD".into());
            assert!(state.is_compare_tab());
            state.help_open = true;
            let mut terminal = Terminal::new(TestBackend::new(cols, 200)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let text = buffer_text(&terminal);
            let lines: Vec<&str> = text.lines().collect();
            let header = lines
                .iter()
                .position(|l| l.contains("MOVE") && l.contains("COMPARE") && l.contains("VIEW"))
                .unwrap_or_else(|| panic!("{cols} cols, compare help header:\n{text}"));
            let top = header - 1;
            let box_w = cols - 4;
            let panes = Rect::new(0, 1, cols, state.layout.pane_height);
            assert_eq!(dialog_width(panes, DialogKind::Help), box_w);
            let rect = dialog_rect(panes, box_w, dialog_height(&state, DialogKind::Help, box_w));
            assert_eq!(rect.y as usize, top, "{cols} cols:\n{text}");
            assert_eq!(
                buffer_row(&terminal, rect.y, 2, 3),
                "╭",
                "{cols} cols:\n{text}"
            );
            let bottom = (header..lines.len())
                .find(|&y| buffer_row(&terminal, y as u16, 2, 3) == "╰")
                .unwrap_or_else(|| panic!("{cols} cols, help bottom border:\n{text}"));
            let reserved =
                usize::from(help_status_lines(box_w, crate::tui::help::HelpTab::Compare));
            assert_eq!(bottom + 1 - top, reserved, "{cols} cols:\n{text}");
            let inner = help_inner_width(usize::from(box_w));
            let body = help_body_line_count(
                HELP_COMPARE_GROUPS,
                &help_column_widths(HELP_COMPARE_GROUPS, inner),
            ) + help_legend_line_count(inner);
            let footer_rows = help_idle_footer_lines(inner).len();
            assert_eq!(
                bottom - header - 1,
                body + footer_rows,
                "{cols} cols:\n{text}"
            );
            assert_eq!(state.layout.help_scroll_max, 0, "{cols} cols fits");
            let (x0, x1) = (rect.x + 2, rect.right() - 2);
            let full: Vec<String> = (0..body)
                .map(|row| buffer_row(&terminal, (header + 1 + row) as u16, x0, x1))
                .collect();
            assert!(
                !full[body - 1].trim().is_empty(),
                "{cols} cols: last body row is blank:\n{text}"
            );
            for needle in ["refresh now", "(1=Workspace)", "(press twice)"] {
                assert!(text.contains(needle), "{cols} cols {needle}:\n{text}");
            }

            // A short terminal clips the box; scrolling shows every row.
            let mut terminal = Terminal::new(TestBackend::new(cols, 20)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let max = state.layout.help_scroll_max;
            assert!(max > 0, "{cols} cols × 20 rows scrolls");
            let short = buffer_text(&terminal);
            let header = short
                .lines()
                .position(|l| l.contains("MOVE") && l.contains("COMPARE"))
                .unwrap_or_else(|| panic!("{cols} cols short header:\n{short}"));
            let visible = body - max;
            for scroll in 0..=max {
                state.help_scroll = scroll;
                terminal.draw(|frame| draw(frame, &mut state)).unwrap();
                for row in 0..visible {
                    assert_eq!(
                        buffer_row(&terminal, (header + 1 + row) as u16, x0, x1),
                        full[scroll + row],
                        "{cols} cols, scroll {scroll}, row {row}:\n{}",
                        buffer_text(&terminal)
                    );
                }
            }
        }
    }

    /// The `?` legend paints under the key columns in the same scroll
    /// body: an ICONS title, Tree / Graph / Chrome headings, and glyph,
    /// name, and meaning per row in the active glyph mode. A help search
    /// hit on a legend row takes the filter pill.
    #[test]
    fn help_legend_paints_under_the_columns() {
        use super::super::icons::IconKind;
        for ascii in [true, false] {
            let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
            let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, ascii);
            state.help_open = true;
            let mut terminal = Terminal::new(TestBackend::new(200, 120)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let text = buffer_text(&terminal);
            assert_eq!(state.layout.help_scroll_max, 0, "{text}");
            let lines: Vec<&str> = text.lines().collect();
            let header = lines
                .iter()
                .position(|l| l.contains("MOVE") && l.contains("GIT") && l.contains("VIEW"))
                .expect("help header");
            let title = lines
                .iter()
                .position(|l| l.trim_matches(|c| c == '│' || c == ' ') == HELP_LEGEND_TITLE)
                .unwrap_or_else(|| panic!("ICONS title:\n{text}"));
            assert!(title > header, "{text}");
            assert!(
                lines[title - 1]
                    .trim_matches(|c| c == '│' || c == ' ')
                    .is_empty(),
                "blank row above ICONS:\n{text}"
            );
            for heading in ["Tree", "Graph", "Chrome"] {
                assert!(
                    lines[title..].iter().any(|l| l.contains(heading)),
                    "{heading}:\n{text}"
                );
            }
            for kind in [
                IconKind::LinkedWorktree,
                IconKind::GraphStash,
                IconKind::FoldCollapsed,
            ] {
                let spec = kind.spec();
                let row = lines[title..]
                    .iter()
                    .find(|l| l.contains(spec.name) && l.contains(spec.meaning))
                    .unwrap_or_else(|| panic!("{kind:?} row:\n{text}"));
                assert!(
                    row.contains(&format!("{} ", spec.glyph(ascii))),
                    "{kind:?} glyph ascii={ascii}: {row}"
                );
            }
            assert_help_version_lower_right(&text);
        }

        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        state.help_search_query = Some("worktree".into());
        let mut terminal = Terminal::new(TestBackend::new(200, 120)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        let pill = state.theme.pills().filter;
        let buf = terminal.backend().buffer();
        let meaning = IconKind::LinkedWorktree.spec().meaning;
        let y = text
            .lines()
            .position(|l| l.contains(meaning))
            .expect("legend hit row") as u16;
        for x in needle_cols(buf, y, meaning) {
            assert_eq!(buf[(x, y)].bg, pill.bg, "legend hit cell {x}");
        }
        let miss = IconKind::GraphStash.spec().meaning;
        let y = text
            .lines()
            .position(|l| l.contains(miss))
            .expect("miss row") as u16;
        for x in needle_cols(buf, y, miss) {
            assert_ne!(buf[(x, y)].bg, pill.bg, "legend miss cell {x}");
        }
    }

    /// A help search hit below the shown rows puts a count on the search
    /// footer; once it scrolls into view the cue goes.
    #[test]
    fn help_search_counts_hits_below_the_fold() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.help_open = true;
        state.help_search_query = Some("rails".into());
        // The rails meaning's first words (the row wraps at this width).
        let rails = "Graph lanes";
        assert!(IconKind::GraphRails.spec().meaning.starts_with(rails));
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(state.layout.help_scroll_max > 0, "{text}");
        assert!(!text.contains(rails), "below the fold:\n{text}");
        assert!(text.contains("v1 more below · PgDn"), "{text}");
        let buf = terminal.backend().buffer();
        let y = text
            .lines()
            .position(|line| line.contains("more below"))
            .expect("cue row") as u16;
        let x = find_cell_col(buf, y, "v1 more below").expect("cue");
        assert_eq!(buf[(x, y)].fg, state.theme.palette().cursor);

        state.help_scroll = state.layout.help_scroll_max;
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains(rails), "scrolled into view:\n{text}");
        assert!(!text.contains("more below"), "{text}");
    }

    /// Legend glyphs paint in the colour their pane paints them, so rows
    /// only colour tells apart (the ref chips) read apart in every theme.
    /// Each group's glyph column is as wide as its own widest glyph.
    #[test]
    fn legend_glyphs_paint_their_pane_colours_per_group_column() {
        use super::super::help::help_legend_key_width;
        use super::super::icons::IconGroup;
        for id in crate::tui::theme::THEME_IDS {
            let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
            let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
            state.theme = id;
            state.help_open = true;
            let mut terminal = Terminal::new(TestBackend::new(200, 120)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let text = buffer_text(&terminal);
            let buf = terminal.backend().buffer();
            let pal = state.theme.palette();
            // Glyph cell of legend row `kind`: its group's key width left of
            // the meaning.
            let glyph_cell = |kind: IconKind| {
                let spec = kind.spec();
                let y = text
                    .lines()
                    .position(|line| line.contains(spec.meaning))
                    .unwrap_or_else(|| panic!("{kind:?} row:\n{text}"))
                    as u16;
                let meaning = find_cell_col(buf, y, spec.meaning).expect("meaning");
                let x = meaning - help_legend_key_width(spec.group.expect("legend")) as u16;
                let first = spec.glyph(true).chars().next().expect("glyph");
                assert_eq!(
                    buf[(x, y)].symbol(),
                    first.to_string(),
                    "{id:?} {kind:?} glyph column"
                );
                &buf[(x, y)]
            };
            let chips = [
                (IconKind::ChipLocal, pal.branch_feature),
                (IconKind::ChipDefault, pal.branch_default),
                (IconKind::ChipRemote, pal.dir),
                (IconKind::ChipTag, pal.modified),
                (IconKind::ChipDetachedHead, pal.head_mark),
            ];
            for (kind, color) in chips {
                assert_eq!(glyph_cell(kind).fg, color, "{id:?} {kind:?}");
            }
            for (kind, color) in [
                (IconKind::Behind, pal.deleted),
                (IconKind::Ahead, pal.added),
                (IconKind::StatusModified, pal.modified),
                (IconKind::Viewed, pal.viewed),
                (IconKind::CursorBar, pal.cursor),
                (IconKind::Repo, pal.heading),
            ] {
                assert_eq!(glyph_cell(kind).fg, color, "{id:?} {kind:?}");
            }
            // Some themes share a hex between chip roles (the graph pane
            // shares it too); the tag chip always stands apart.
            let colours: HashSet<_> = chips.iter().map(|(_, color)| *color).collect();
            assert!(colours.len() > 1, "{id:?} chips are not one colour");
            assert_ne!(pal.modified, pal.branch_feature, "{id:?} tag vs local");
        }
        assert!(
            help_legend_key_width(IconGroup::Tree) < help_legend_key_width(IconGroup::Graph),
            "one-column tree glyphs do not pad to the chip samples"
        );
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
            // A key chip outside a hit paints `panel` on its group colour,
            // which can be the pill bg (Dracula: `cursor` == filter bg).
            let panel = id.palette().panel;
            for y in rows_with_bg(buf, pill.bg) {
                for x in cols_with_bg(buf, y, pill.bg) {
                    let fg = buf[(x, y)].fg;
                    assert!(
                        fg == pill.fg || fg == panel,
                        "{id:?} ({x},{y}) on pill bg: {fg:?}"
                    );
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
        // The comment box has no status row; `status` stays on the last row.
        let rows: Vec<&str> = text.lines().collect();
        let (last, boxed) = rows.split_last().expect("rows");
        assert_eq!(
            boxed.concat().matches("hello").count(),
            1,
            "typed body must not also echo as status inside the overlay:\n{text}"
        );
        assert_eq!(last.trim(), "body: hello", "{text}");
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
                icon: None,
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
                icon: None,
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
        line_text(
            &paint_tree_row(
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
                None,
                palette,
                col_offset,
            )
            .0,
        )
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
                    icon: None,
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
    fn recorded_split_rule_is_the_painted_rule_column() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.diff_mode = crate::tui::split::DiffMode::SideBySide;
        state.cursor = state
            .rows
            .iter()
            .position(|r| r.kind == NodeKind::File)
            .expect("file row");
        state.set_diff(
            "app".into(),
            "README.md".into(),
            super::super::diff::DiffContent::from_lines(vec![
                "@@ -1,1 +1,1 @@".into(),
                "-old line".into(),
                "+new line".into(),
            ]),
        );
        for fraction in [0.3, 0.5, 0.7] {
            state.diff_split_fraction = fraction;
            let mut terminal = Terminal::new(TestBackend::new(220, 20)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let rule = state.layout.diff_split_rule_x.expect("rule");
            assert_eq!(
                state.layout.diff_content_x,
                state.layout.outer_tree_width + 1,
                "boxed content starts after the right pane's left border"
            );
            let buf = terminal.backend().buffer();
            let y = first_row_with(buf, "old line").expect("diff row");
            let rule_glyph = DIFF_RULE.to_string();
            let painted: Vec<u16> = (0..buf.area.width)
                .filter(|&x| {
                    buf[(x, y)].symbol() == rule_glyph && buf[(x, y)].fg == Color::DarkGray
                })
                .collect();
            assert_eq!(painted.len(), 1, "one split rule: {}", buf_line(buf, y));
            // The paint sizes the columns from the row width (less the
            // cursor bar and the scrollbar column), the layout from the pane
            // width, so off the default fraction the two can be one column
            // apart. The ±1 grab band still covers the painted rule.
            if fraction == crate::tui::split::DIFF_SPLIT_FRACTION {
                assert_eq!(painted[0], rule, "{}", buf_line(buf, y));
            } else {
                assert!(
                    painted[0].abs_diff(rule) <= 1,
                    "fraction {fraction}: painted {} recorded {rule}",
                    painted[0]
                );
            }
        }
    }

    /// `state` in paint mode on Slate with Unicode glyphs, the shipped
    /// default look.
    fn painted(mut state: AppState) -> AppState {
        state.background = BackgroundMode::Paint;
        state.theme = ThemeId::Slate;
        state.ascii = false;
        state
    }

    /// Left and right pane rects of the last frame (rows 1..=pane_height).
    fn pane_rects(state: &AppState, cols: u16) -> (Rect, Rect) {
        let height = state.layout.pane_height;
        let left = Rect::new(0, 1, state.layout.outer_tree_width, height);
        let right = Rect::new(state.layout.right_x, 1, cols - state.layout.right_x, height);
        (left, right)
    }

    /// Outer rects of the Explorer tree and preview panes in a flat
    /// frame: the recorded content rects grown by their title and accent
    /// rows.
    fn explorer_pane_rects(state: &AppState) -> (Rect, Rect) {
        let grow = |inner: Rect| Rect {
            y: inner.y - FLAT_CHROME_ROWS,
            height: inner.height + FLAT_CHROME_ROWS,
            ..inner
        };
        (
            grow(state.layout.explorer_tree),
            grow(state.layout.explorer_preview),
        )
    }

    const BORDER_GLYPHS: &[&str] = &["│", "─", "┌", "┐", "└", "┘", "╭", "╮", "╰", "╯", "├", "┤"];

    /// Every edge cell of `pane` that is a border glyph.
    fn pane_edge_borders(buf: &ratatui::buffer::Buffer, pane: Rect) -> Vec<(u16, u16, String)> {
        let right = pane.x + pane.width - 1;
        let bottom = pane.y + pane.height - 1;
        let mut edges: Vec<(u16, u16)> = Vec::new();
        for x in pane.x..=right {
            edges.push((x, pane.y));
            edges.push((x, bottom));
        }
        for y in pane.y..=bottom {
            edges.push((pane.x, y));
            edges.push((right, y));
        }
        edges
            .into_iter()
            .filter(|&(x, y)| BORDER_GLYPHS.contains(&buf[(x, y)].symbol()))
            .map(|(x, y)| (x, y, buf[(x, y)].symbol().to_string()))
            .collect()
    }

    /// Check the flat title and accent rows of `pane`. Title row: `title`
    /// after one blank cell, `heading` and bold when `focused`, else
    /// `muted` and plain; every other cell a plain space; nothing
    /// underlined. Accent row: a full-width `▁` in `cursor` when `focused`,
    /// else blank. Both rows keep `bg` throughout.
    fn assert_flat_title_row(
        buf: &ratatui::buffer::Buffer,
        pane: Rect,
        title: &str,
        focused: bool,
        bg: Color,
        palette: Palette,
    ) {
        let y = pane.y;
        let title_end = pane.x + 1 + title.chars().count() as u16;
        let text: String = (pane.x + 1..title_end)
            .map(|x| buf[(x, y)].symbol())
            .collect();
        assert_eq!(text, title, "title row: {}", buf_line(buf, y));
        for x in pane.x..pane.x + pane.width {
            let cell = &buf[(x, y)];
            assert_eq!(cell.bg, bg, "title row bg at x={x}");
            assert!(
                !cell.modifier.contains(Modifier::UNDERLINED),
                "no underline on the title row at x={x}"
            );
            let in_title = x > pane.x && x < title_end;
            if focused && in_title {
                assert_eq!(cell.fg, palette.heading, "title fg at x={x}");
                assert!(cell.modifier.contains(Modifier::BOLD), "bold at x={x}");
            } else if in_title {
                assert_eq!(cell.fg, palette.muted, "muted title at x={x}");
                assert!(cell.modifier.is_empty(), "plain title at x={x}");
            } else {
                assert_eq!(cell.symbol(), " ", "plain blank on the title row x={x}");
                assert!(cell.modifier.is_empty(), "x={x}");
            }
            let accent = &buf[(x, y + 1)];
            assert_eq!(accent.bg, bg, "accent row bg at x={x}");
            if focused {
                assert_eq!(accent.symbol(), "\u{2581}", "accent line at x={x}");
                assert_eq!(accent.fg, palette.cursor, "accent line fg at x={x}");
            } else {
                assert_eq!(accent.symbol(), " ", "blank accent row at x={x}");
            }
        }
    }

    /// Paint mode draws flat panes: no border glyph on any pane edge, the
    /// title on row 0, the accent row on row 1, and the content rect from
    /// the pane's first column and row 2 to its bottom row.
    #[test]
    fn paint_mode_panes_are_flat_with_content_from_the_pane_edge() {
        let mut state = painted(two_pane_diff_state());
        state.focus = FocusPane::Left;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = pane_rects(&state, 120);
        let buf = terminal.backend().buffer();
        assert_eq!(pane_edge_borders(buf, left), vec![], "left pane edges");
        assert_eq!(pane_edge_borders(buf, right), vec![], "right pane edges");
        assert_eq!(right.x, left.x + left.width, "panes abut");

        let layout = &state.layout;
        assert_eq!((layout.tree_x, layout.tree_y), (left.x, left.y + 2));
        assert_eq!(layout.tree_width, left.width);
        assert_eq!(layout.tree_height, left.height - 2, "uses the bottom row");
        assert_eq!(layout.diff_content_x, right.x);
        assert_eq!(layout.right_y, right.y + 2);
        assert_eq!(layout.diff_pane_width, right.width);
        assert_eq!(layout.diff_pane_height, right.height - 2);
    }

    /// Terminal mode keeps the boxed panes: border glyphs, no accent line,
    /// and the content rect inside the border.
    #[test]
    fn terminal_mode_keeps_boxed_pane_chrome() {
        let mut state = two_pane_diff_state();
        state.focus = FocusPane::Left;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = pane_rects(&state, 120);
        let buf = terminal.backend().buffer();
        assert_ne!(pane_edge_borders(buf, left), vec![], "left pane is boxed");
        assert_ne!(pane_edge_borders(buf, right), vec![], "right pane is boxed");
        for pane in [left, right] {
            for y in pane.y..pane.y + 2 {
                assert!(
                    !buf_line(buf, y).contains('\u{2581}'),
                    "{}",
                    buf_line(buf, y)
                );
            }
        }
        let layout = &state.layout;
        assert_eq!((layout.tree_x, layout.tree_y), (left.x + 1, left.y + 1));
        assert_eq!(layout.tree_height, left.height - 2);
        assert_eq!(layout.right_y, right.y + 1);
    }

    /// A flat pane too short for its title and accent rows draws what fits
    /// and returns an empty content rect without panicking.
    #[test]
    fn paint_mode_pane_chrome_fits_tiny_panes() {
        let state = painted(two_pane_diff_state());
        for height in 0..=3u16 {
            let mut terminal = Terminal::new(TestBackend::new(10, 4)).unwrap();
            let area = Rect::new(0, 0, 10, height);
            let mut inner = Rect::default();
            terminal
                .draw(|frame| {
                    inner = draw_pane_chrome(frame, area, "tree", true, Color::Black, &state);
                })
                .unwrap();
            let chrome = height.min(2);
            assert_eq!(
                inner,
                Rect::new(0, chrome, 10, height - chrome),
                "h={height}"
            );
        }
    }

    /// Flat focus: the active pane's accent row carries the line and its
    /// title is `heading`, bold; `Tab` moves both to the other pane. The
    /// content rects stay put. Pane backgrounds and body text colours do
    /// not change with focus.
    #[test]
    fn paint_mode_tab_moves_the_accent_line_and_keeps_pane_colours() {
        let mut state = painted(two_pane_diff_state());
        let palette = state.theme.palette();
        state.dispatch(Action::FocusLeft);
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = pane_rects(&state, 120);
        let body_style = |terminal: &Terminal<TestBackend>, needle: &str| {
            let buf = terminal.backend().buffer();
            let y = first_row_with(buf, needle).expect(needle);
            let x = find_cell_col(buf, y, needle).expect(needle);
            buf[(x, y)].fg
        };
        let before = (
            body_style(&terminal, "new line"),
            body_style(&terminal, "old line"),
        );
        let content_rects = |state: &AppState| {
            let l = &state.layout;
            (l.tree_y, l.tree_height, l.right_y, l.diff_pane_height)
        };
        let rects_before = content_rects(&state);
        {
            let buf = terminal.backend().buffer();
            assert_flat_title_row(buf, left, "tree", true, palette.sidebar, palette);
            assert_flat_title_row(buf, right, "diff", false, palette.surface, palette);
        }

        state.dispatch(Action::FocusRight);
        draw_state(&mut terminal, &mut state);
        {
            let buf = terminal.backend().buffer();
            assert_flat_title_row(buf, left, "tree", false, palette.sidebar, palette);
            assert_flat_title_row(buf, right, "diff", true, palette.surface, palette);
        }
        let after = (
            body_style(&terminal, "new line"),
            body_style(&terminal, "old line"),
        );
        assert_eq!(before, after, "diff text colours do not follow focus");
        assert_eq!(
            content_rects(&state),
            rects_before,
            "Tab does not move the content rects"
        );

        // Blank body cells keep each pane's own bg in both focus states.
        let buf = terminal.backend().buffer();
        let bottom = left.y + left.height - 1;
        assert_eq!(buf[(left.x + left.width - 2, bottom)].bg, palette.sidebar);
        assert_eq!(buf[(right.x + right.width - 2, bottom)].bg, palette.surface);
    }

    /// ASCII glyph mode draws the accent row with `_`; the title row keeps
    /// plain blanks.
    #[test]
    fn paint_mode_ascii_accent_line_is_underscore() {
        let mut state = painted(two_pane_diff_state());
        state.ascii = true;
        state.focus = FocusPane::Right;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (_, right) = pane_rects(&state, 120);
        let title = row_cells(&terminal, right.x, right.y, right.width);
        assert!(title.starts_with(" diff   "), "{title}");
        let accent = row_cells(&terminal, right.x, right.y + 1, right.width);
        assert_eq!(accent, "_".repeat(usize::from(right.width)), "{accent}");
    }

    /// Paint mode backgrounds: tab strip, breadcrumb, and status rows on
    /// `chrome` (the active tab keeps `cursor_bg`), the left pane on
    /// `sidebar`, the right pane on `surface`. No cell keeps the terminal
    /// background.
    #[test]
    fn paint_mode_fills_chrome_rows_and_pane_backgrounds() {
        let mut state = painted(two_pane_diff_state());
        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        state.tabs.active = 0;
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = pane_rects(&state, 120);
        let buf = terminal.backend().buffer();
        for y in 0..24 {
            for x in 0..120 {
                assert_ne!(buf[(x, y)].bg, Color::Reset, "terminal bg at ({x},{y})");
            }
        }
        let (active_x, active_w, _) = state.layout.tab_hits[0];
        let (inactive_x, inactive_w, _) = state.layout.tab_hits[1];
        for x in active_x..active_x + active_w {
            assert_eq!(buf[(x, 0)].bg, palette.cursor_bg, "active tab x={x}");
        }
        for x in inactive_x..inactive_x + inactive_w {
            assert_eq!(buf[(x, 0)].bg, palette.chrome, "inactive tab x={x}");
        }
        assert_eq!(buf[(119, 0)].bg, palette.chrome, "tab strip tail");
        let crumb_y = 1 + state.layout.pane_height;
        assert_eq!(buf[(119, crumb_y)].bg, palette.chrome, "breadcrumb row");
        assert_eq!(buf[(119, 23)].bg, palette.chrome, "status row");
        let bottom = left.y + left.height - 1;
        for x in left.x..left.x + left.width {
            assert_eq!(buf[(x, bottom)].bg, palette.sidebar, "left x={x}");
        }
        for x in right.x..right.x + right.width {
            assert_eq!(buf[(x, bottom)].bg, palette.surface, "right x={x}");
        }
    }

    /// Terminal mode keeps the v0.1.244 boxed panes with no fills.
    #[test]
    fn terminal_mode_keeps_boxed_panes_on_the_terminal_background() {
        let mut state = two_pane_diff_state();
        state.theme = ThemeId::Slate;
        assert_eq!(state.background, BackgroundMode::Terminal);
        state.focus = FocusPane::Left;
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = pane_rects(&state, 120);
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(left.x, left.y)].symbol(), "┌");
        assert_eq!(buf[(left.x, left.y)].fg, palette.heading, "focused border");
        assert_eq!(buf[(right.x, right.y)].symbol(), "┌");
        assert_eq!(buf[(right.x, right.y)].fg, palette.border_dim);
        assert_eq!(
            buf[(left.x + 1, left.y)].symbol(),
            "t",
            "title after corner"
        );
        assert_eq!(state.layout.tree_x, left.x + 1);
        assert_eq!(state.layout.diff_content_x, right.x + 1);
        for (x, y) in [(119, 0), (left.x + 2, left.y + 8), (right.x + 30, 12)] {
            assert_eq!(buf[(x, y)].bg, Color::Reset, "no fill at ({x},{y})");
        }
    }

    /// Rect of the rounded popup box in `buf` (`╭` … `╯`), if any. Flat
    /// panes paint no border glyphs, so the only rounded corner is a popup.
    fn rounded_box(buf: &ratatui::buffer::Buffer) -> Option<Rect> {
        let area = buf.area;
        let (x0, y0) = (area.top()..area.bottom())
            .flat_map(|y| (area.left()..area.right()).map(move |x| (x, y)))
            .find(|&(x, y)| buf[(x, y)].symbol() == "╭")?;
        let x1 = (x0 + 1..area.right()).find(|&x| buf[(x, y0)].symbol() == "╮")?;
        let y1 = (y0 + 1..area.bottom()).find(|&y| buf[(x0, y)].symbol() == "╰")?;
        Some(Rect::new(x0, y0, x1 - x0 + 1, y1 - y0 + 1))
    }

    /// Paint-mode screens for the every-cell sweep: each pane screen in
    /// both focus states (Explorer folder / file / diff previews too), each
    /// [`DialogKind`], the icon popover, and the too-small notice, as
    /// (label, state, cols, rows).
    fn paint_sweep_screens() -> Vec<(String, AppState, u16, u16)> {
        use crate::git::{AncestorCommit, LocalBranch};
        use crate::tui::action::QuickOpenEntry;
        use crate::tui::branches::{BranchPickerState, CreateBranchState};
        use crate::tui::comments::CommentExport;
        use crate::tui::graph_focus::GraphFocusPickerState;
        use crate::tui::quick_open::{QuickOpenScope, QuickOpenState};
        use crate::tui::search_files::SearchPreview;
        use crate::tui::stash::{stash_ops_for_context, StashOpsContext};

        let mut panes: Vec<(&str, AppState)> = vec![
            ("tree + graph", two_pane_graph_state()),
            ("files drill", two_pane_files_state()),
            ("commit diff", two_pane_commit_diff_state()),
            ("inline diff", two_pane_diff_state()),
            ("compare tab", summary_compare_state(summary_files())),
        ];
        let mut summary = summary_drill_state(summary_files());
        let _ = summary_move_to(&mut summary, "src");
        panes.push(("folder summary", summary));
        let mut split = two_pane_diff_state();
        split.diff_mode = crate::tui::split::DiffMode::SideBySide;
        panes.push(("side-by-side diff", split));
        let mut prompt = two_pane_diff_state();
        prompt.status = crate::tui::ctrl_c_exit::CTRL_C_EXIT_PROMPT.into();
        panes.push(("ctrl-c prompt", prompt));

        let mut screens = Vec::new();
        for (label, state) in panes {
            for focus in [FocusPane::Left, FocusPane::Right] {
                let mut state = painted(state.clone());
                state.focus = focus;
                screens.push((format!("{label} / {focus:?}"), state, 120, 30));
            }
        }
        let explorer: Vec<(&str, AppState)> = vec![
            ("explorer folder", explorer_state()),
            ("explorer file", explorer_preview_state("new.txt", None)),
            (
                "explorer diff",
                explorer_preview_state("README.md", Some("@@ -1 +1 @@\n-# seed\n+# dirty\n")),
            ),
        ];
        for (label, state) in explorer {
            for focus in [FocusPane::Left, FocusPane::Right] {
                let mut state = painted(state.clone());
                state.focus = focus;
                screens.push((format!("{label} / {focus:?}"), state, 120, 30));
            }
        }
        // One pane: focus does not change the file tab.
        let file_tab = painted(file_tab_state(&["# app", "dirty"]));
        screens.push(("file tab".into(), file_tab, 120, 30));

        let branch = |name: &str, current: bool| LocalBranch {
            name: name.into(),
            current,
            authordate: 0,
        };
        let base = two_pane_diff_state;
        let mut dialogs: Vec<(&str, AppState)> = Vec::new();
        let mut state = base();
        state.help_open = true;
        dialogs.push(("help", state));
        let mut state = base();
        state.confirm = Some(PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        });
        dialogs.push(("confirm", state));
        let mut state = base();
        state.stash_repo = Some("app".into());
        state.stash_menu = Some(stash_ops_for_context(&StashOpsContext {
            dirty: true,
            dirty_paths: None,
            focused_stash_ref: Some("stash@{0}".into()),
            latest_stash_ref: Some("stash@{0}".into()),
        }));
        dialogs.push(("stash menu", state));
        let mut state = base();
        state.blame_menu = true;
        dialogs.push(("blame menu", state));
        let mut state = base();
        state.create_branch = Some(CreateBranchState {
            repo: "app".into(),
            name: "topic".into(),
            commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        });
        dialogs.push(("create branch", state));
        let mut state = base();
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
        dialogs.push(("comment", state));
        let mut state = base();
        state.comment_export = Some(CommentExport {
            markdown: "- README.md:1 hello".into(),
            copied: Some(true),
        });
        dialogs.push(("comment export", state));
        let mut state = base();
        // `topic/a` selected, `topic/b` and the create row not.
        let mut picker = BranchPickerState::checkout(
            "app".into(),
            vec![branch("topic/a", false), branch("topic/b", false)],
        );
        picker.set_filter("topic".into());
        assert_eq!(picker.create_name(), Some("topic"));
        state.branch_picker = Some(picker);
        dialogs.push(("branch picker", state));
        let mut state = base();
        let commits = (0..3)
            .map(|i| AncestorCommit {
                id: format!("{i:02}{}", "a".repeat(38)),
                subject: format!("commit {i}"),
            })
            .collect();
        state.open_compare_commit_picker("app".into(), commits);
        dialogs.push(("compare picker", state));
        let mut state = base();
        state.graph_focus_picker = Some(GraphFocusPickerState::new(
            "app".into(),
            vec![branch("main", true), branch("topic/a", false)],
            &[],
        ));
        dialogs.push(("graph focus picker", state));
        dialogs.push(("quick open files", quick_open_files_state(5)));
        let mut state = base();
        state.quick_open = Some(QuickOpenState::new(
            QuickOpenEntry::Commands,
            QuickOpenScope::Workspace,
        ));
        dialogs.push(("quick open commands", state));
        let mut state = search_files_state(false);
        let mut lines: Vec<String> = (1..=20).map(|n| format!("// line {n}")).collect();
        lines[2] = "let needle = needle();".into();
        search_dialog(&mut state).preview = Some(SearchPreview {
            checkout: "app".into(),
            rel: "src/lib.rs".into(),
            gen: 1,
            body: Some(FileRead::Text {
                lines,
                max_cols: 22,
            }),
        });
        dialogs.push(("search files", state));
        for (label, state) in dialogs {
            screens.push((format!("dialog {label}"), painted(state), 120, 30));
        }

        let mut state = painted(pr_state(&[branch_repo("app", "feature")], false));
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        focus_tree_row(&mut state, "repo:app");
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        state.dispatch(Action::PopoverOpenFocused);
        screens.push(("icon popover".into(), state, 120, 30));

        let too_small = painted(two_pane_diff_state());
        screens.push(("too small".into(), too_small, 40, 10));
        screens
    }

    /// Paint mode leaves no cell on the terminal background, for every
    /// theme and screen: panes, chrome rows, each dialog, the icon
    /// popover, and the too-small notice. Pane title rows sit on their
    /// role colour (`sidebar` left, `surface` right and file tab), no pane
    /// cell carries another pane's role, chrome rows end on `chrome`, and a
    /// popup's border sits on `panel` with no pane role inside it.
    #[test]
    fn paint_mode_sets_a_background_on_every_cell_for_every_theme() {
        let mut screens = paint_sweep_screens();
        // Every dialog kind and the popover are in the sweep.
        let mut kinds = HashSet::new();
        for (label, state, ..) in &screens {
            if let Some(kind) = open_dialog(state) {
                kinds.insert(format!("{kind:?}"));
            }
            assert!(
                !label.starts_with("dialog") || open_dialog(state).is_some(),
                "{label}"
            );
        }
        assert_eq!(kinds.len(), 12, "{kinds:?}");
        for id in crate::tui::theme::THEME_IDS {
            let palette = id.palette();
            for (label, state, cols, rows) in &mut screens {
                state.theme = id;
                let mut terminal = Terminal::new(TestBackend::new(*cols, *rows)).unwrap();
                draw_state(&mut terminal, state);
                let buf = terminal.backend().buffer();
                let at = |x: u16, y: u16| format!("{id:?} {label} ({x},{y})");
                for y in 0..*rows {
                    for x in 0..*cols {
                        assert_ne!(buf[(x, y)].bg, Color::Reset, "{}", at(x, y));
                    }
                }
                if state.too_small {
                    for y in 0..*rows {
                        for x in 0..*cols {
                            assert_eq!(buf[(x, y)].bg, palette.surface, "{}", at(x, y));
                        }
                    }
                    continue;
                }
                let popup = rounded_box(buf);
                if label.starts_with("dialog") || label.as_str() == "icon popover" {
                    let popup = popup.unwrap_or_else(|| panic!("{id:?} {label}: no popup"));
                    for y in popup.top()..popup.bottom() {
                        for x in popup.left()..popup.right() {
                            let edge = y == popup.top()
                                || y == popup.bottom() - 1
                                || x == popup.left()
                                || x == popup.right() - 1;
                            let bg = buf[(x, y)].bg;
                            if edge && buf[(x, y)].symbol() != " " {
                                assert_eq!(bg, palette.panel, "{} border", at(x, y));
                            }
                            for role in [palette.sidebar, palette.surface, palette.chrome] {
                                if role != palette.panel {
                                    assert_ne!(bg, role, "{} pane role in popup", at(x, y));
                                }
                            }
                        }
                    }
                }
                let outside = |x: u16, y: u16| popup.is_none_or(|p| !p.contains((x, y).into()));
                let pane_h = state.layout.pane_height;
                let panes: Vec<(Rect, Color)> = if state.is_file_tab() {
                    vec![(Rect::new(0, 1, *cols, pane_h), palette.surface)]
                } else if state.is_explorer_tab() {
                    let (left, right) = explorer_pane_rects(state);
                    vec![(left, palette.sidebar), (right, palette.surface)]
                } else {
                    let (left, right) = pane_rects(state, *cols);
                    vec![(left, palette.sidebar), (right, palette.surface)]
                };
                let roles = [
                    palette.sidebar,
                    palette.surface,
                    palette.chrome,
                    palette.panel,
                ];
                for (pane, role) in panes {
                    for y in pane.top()..pane.bottom() {
                        for x in pane.left()..pane.right() {
                            if !outside(x, y) {
                                continue;
                            }
                            let bg = buf[(x, y)].bg;
                            if y == pane.top() {
                                assert_eq!(bg, role, "{} title row", at(x, y));
                            }
                            for other in roles {
                                if other != role {
                                    assert_ne!(bg, other, "{} wrong role", at(x, y));
                                }
                            }
                        }
                    }
                }
                let crumb_y = 1 + pane_h;
                let mut chrome_rows = vec![0, crumb_y, *rows - 1];
                if super::super::chrome::ctrl_c_prompt_rows(state) > 0 {
                    chrome_rows.push(crumb_y + breadcrumb_rows(state));
                }
                for y in chrome_rows {
                    assert_eq!(
                        buf[(*cols - 1, y)].bg,
                        palette.chrome,
                        "{} chrome",
                        at(*cols - 1, y)
                    );
                }
            }
        }
    }

    /// Terminal mode: popups sit on `panel` (border and blank inner cells)
    /// while the boxed panes keep the terminal background.
    #[test]
    fn terminal_mode_popups_sit_on_panel_over_unfilled_panes() {
        let mut state = two_pane_diff_state();
        assert_eq!(state.background, BackgroundMode::Terminal);
        state.confirm = Some(PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        });
        for id in crate::tui::theme::THEME_IDS {
            state.theme = id;
            let palette = id.palette();
            let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let popup = rounded_box(buf).expect("confirm box");
            for y in popup.top()..popup.bottom() {
                for x in popup.left()..popup.right() {
                    assert_ne!(buf[(x, y)].bg, Color::Reset, "{id:?} popup ({x},{y})");
                }
            }
            assert_eq!(buf[(popup.x, popup.y)].bg, palette.panel, "{id:?} corner");
            let inner_blank = (popup.left() + 1..popup.right() - 1)
                .find(|&x| buf[(x, popup.bottom() - 2)].symbol() == " ")
                .expect("blank inner cell");
            assert_eq!(buf[(inner_blank, popup.bottom() - 2)].bg, palette.panel);
            let (left, right) = pane_rects(&state, 120);
            for (x, y) in [(119, 0), (left.x + 2, left.y + 2), (right.right() - 2, 2)] {
                assert!(!popup.contains((x, y).into()));
                assert_eq!(buf[(x, y)].bg, Color::Reset, "{id:?} pane ({x},{y})");
            }
        }
    }

    /// Clicks on the flat left pane's title and accent rows select nothing;
    /// clicks on its first column, first content row, and bottom row
    /// select the tree row painted there.
    #[test]
    fn paint_mode_clicks_on_flat_pane_edges_select_the_painted_row() {
        let repos: Vec<RepoSnapshot> = (0..40).map(|i| repo(&format!("r{i:02}"), true)).collect();
        let snapshot = build_workspace_snapshot(&repos, &[], false, &[]);
        let mut state = painted(AppState::new(PathBuf::from("/tmp"), snapshot, true));
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, _) = pane_rects(&state, 120);
        let first = left.y + 2;
        let last = left.y + left.height - 1;
        assert_eq!(state.layout.tree_y, first);
        state.cursor = 2;
        draw_state(&mut terminal, &mut state);
        for row in [left.y, left.y + 1] {
            state.dispatch(Action::Click { col: left.x, row });
            assert_eq!(state.cursor, 2, "chrome row {row} selects nothing");
        }
        for row in [first, last, first + 3] {
            let idx = state.layout.list_offset + (row - first) as usize;
            let want = state.painted_tree_rows()[idx].id.clone();
            let label = buf_line(terminal.backend().buffer(), row);
            state.dispatch(Action::Click { col: left.x, row });
            assert_eq!(state.rows[state.cursor].id, want, "row {row}: {label}");
            assert_eq!(state.focus, FocusPane::Left);
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let bar_row = (first..=last)
                .find(|&y| buf[(left.x, y)].symbol() == CURSOR_BAR)
                .expect("cursor bar");
            let left_text = |y: u16| -> String {
                (left.x + 1..left.x + left.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect()
            };
            assert_eq!(
                left_text(bar_row),
                label
                    .chars()
                    .skip(usize::from(left.x) + 1)
                    .take(usize::from(left.width) - 1)
                    .collect::<String>(),
                "the clicked row is selected"
            );
        }
    }

    /// The file tab paints flat in paint mode: title and accent rows on
    /// `surface`, code from the pane's first column, last row used.
    #[test]
    fn paint_mode_file_tab_is_flat() {
        let mut state = painted(file_tab_state(&["# app", "dirty"]));
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        draw_state(&mut terminal, &mut state);
        let pane = Rect::new(0, 1, 80, state.layout.pane_height);
        let buf = terminal.backend().buffer();
        assert_eq!(pane_edge_borders(buf, pane), vec![]);
        assert_flat_title_row(buf, pane, "app/README.md", true, palette.surface, palette);
        assert!(
            buf_line(buf, 3).starts_with("1 # app"),
            "{}",
            buf_line(buf, 3)
        );
        assert_eq!((state.layout.file_view_x, state.layout.file_view_y), (0, 3));
        assert_eq!(state.layout.file_view_width, 80);
        assert_eq!(state.layout.file_view_height, pane.height - 2);
        let bottom = pane.y + pane.height - 1;
        assert_eq!(buf[(40, bottom)].bg, palette.surface);
    }

    /// The compare tab uses the same flat chrome: file list on `sidebar`,
    /// diff on `surface`, no border glyphs.
    #[test]
    fn paint_mode_compare_tab_is_flat() {
        let mut state = painted(compare_json_over_stale_workspace_state());
        state.focus = FocusPane::Left;
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = pane_rects(&state, 120);
        let buf = terminal.backend().buffer();
        assert_eq!(pane_edge_borders(buf, left), vec![]);
        assert_eq!(pane_edge_borders(buf, right), vec![]);
        let left_title = state.left_pane_title();
        assert_flat_title_row(buf, left, left_title, true, palette.sidebar, palette);
        assert_flat_title_row(buf, right, "diff", false, palette.surface, palette);
        assert_eq!(state.layout.diff_content_x, right.x);
        assert_eq!(state.layout.files_list_y, left.y + 2);
    }

    /// The Explorer tab paints flat in paint mode through the shared pane
    /// chrome: tree on `sidebar`, preview on `surface`, no border glyphs,
    /// the accent row under the focused title, and every recorded rect is
    /// the content rect (pane less its title and accent rows).
    #[test]
    fn paint_mode_explorer_tab_is_flat() {
        let mut state = painted(explorer_preview_state("new.txt", None));
        state.focus = FocusPane::Left;
        let palette = state.theme.palette();
        let checkout = state.tabs.active_explorer().unwrap().checkout.clone();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = explorer_pane_rects(&state);
        assert_eq!((left.x, left.y), (0, 1));
        assert_eq!(left.height, state.layout.pane_height);
        assert_eq!(right.x, left.right(), "panes abut");
        assert_eq!(right.right(), 120);
        {
            let buf = terminal.backend().buffer();
            assert_eq!(pane_edge_borders(buf, left), vec![], "tree edges");
            assert_eq!(pane_edge_borders(buf, right), vec![], "preview edges");
            assert_flat_title_row(buf, left, &checkout, true, palette.sidebar, palette);
            assert_flat_title_row(buf, right, "app/new.txt", false, palette.surface, palette);
            let body: String = (right.x..right.right())
                .map(|x| buf[(x, right.y + 2)].symbol())
                .collect();
            assert!(body.starts_with(" 1 line 1"), "{body}");
            let bottom = left.bottom() - 1;
            assert_eq!(buf[(left.right() - 2, bottom)].bg, palette.sidebar);
            assert_eq!(buf[(right.right() - 2, bottom)].bg, palette.surface);
        }
        let layout = &state.layout;
        assert_eq!(
            (layout.file_view_x, layout.file_view_y),
            (right.x, right.y + 2)
        );
        assert_eq!(layout.file_view_width, right.width);
        assert_eq!(layout.file_view_height, right.height - 2);
        assert_eq!(layout.right_x, right.x);
        assert_eq!(layout.right_y, right.y + 2);
        assert_eq!(layout.diff_content_x, right.x);
        assert_eq!(layout.diff_pane_width, right.width);
        assert_eq!(layout.diff_pane_height, right.height - 2);

        state.dispatch(Action::FocusRight);
        assert_eq!(state.focus, FocusPane::Right);
        draw_state(&mut terminal, &mut state);
        let buf = terminal.backend().buffer();
        assert_flat_title_row(buf, left, &checkout, false, palette.sidebar, palette);
        assert_flat_title_row(buf, right, "app/new.txt", true, palette.surface, palette);
    }

    /// Mouse in a flat Explorer: title and accent rows are not content, the first
    /// column and first content row of each pane hit, and the wheel moves
    /// the pane under the pointer.
    #[test]
    fn paint_mode_explorer_mouse_hits_the_flat_content_rects() {
        let mut state = painted(explorer_preview_state("new.txt", None));
        state.focus = FocusPane::Left;
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let (left, right) = explorer_pane_rects(&state);
        let file_cursor = |state: &AppState| match state.tabs.active_explorer().map(|t| &t.preview)
        {
            Some(ExplorerPreview::File(file)) => file.cursor,
            other => panic!("file preview, got {other:?}"),
        };
        let painted_rel = |state: &AppState| {
            state
                .tabs
                .active_explorer()
                .and_then(|tab| tab.painted_rel.clone())
        };

        // The preview's title and accent rows are not the body.
        for row in [right.y, right.y + 1] {
            state.dispatch(Action::Click { col: right.x, row });
            assert_eq!(state.focus, FocusPane::Left, "row {row}");
        }
        // Its first column, third body row, takes focus and that line.
        state.dispatch(Action::Click {
            col: right.x,
            row: right.y + 4,
        });
        assert_eq!(state.focus, FocusPane::Right);
        assert_eq!(file_cursor(&state), 2);
        state.dispatch(Action::ScrollWheel {
            col: right.x,
            row: right.y + 2,
            delta: 1,
            horizontal: false,
        });
        assert_eq!(file_cursor(&state), 3);
        draw_state(&mut terminal, &mut state);

        // The tree's title and accent rows are not rows; its first column is.
        assert_eq!(painted_rel(&state).as_deref(), Some("new.txt"));
        for row in [left.y, left.y + 1] {
            state.dispatch(Action::Click { col: left.x, row });
            assert_eq!(state.focus, FocusPane::Right, "chrome row {row}: no hit");
        }
        let tree = state.layout.explorer_tree;
        let readme = (tree.y..tree.bottom())
            .find(|&y| buf_line(terminal.backend().buffer(), y).contains("README.md"))
            .expect("README.md row");
        state.dispatch(Action::Click {
            col: left.x,
            row: readme,
        });
        assert_eq!(state.focus, FocusPane::Left);
        draw_state(&mut terminal, &mut state);
        assert_eq!(painted_rel(&state).as_deref(), Some("README.md"));
        state.dispatch(Action::ScrollWheel {
            col: left.x,
            row: left.y + 2,
            delta: -1,
            horizontal: false,
        });
        draw_state(&mut terminal, &mut state);
        assert_eq!(painted_rel(&state).as_deref(), Some("new.txt"));
    }

    /// Terminal mode keeps the MYWS-047 boxed Explorer panes: corners,
    /// `heading` border on the focused pane, `border_dim` on the other, no
    /// fills, and content rects inside the borders.
    #[test]
    fn terminal_mode_explorer_tab_keeps_boxed_panes() {
        let mut state = explorer_preview_state("new.txt", None);
        assert_eq!(state.background, BackgroundMode::Terminal);
        state.focus = FocusPane::Left;
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let tree = state.layout.explorer_tree;
        let preview = state.layout.explorer_preview;
        assert_eq!((tree.x, tree.y), (1, 2));
        assert_eq!(preview.y, 2);
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(0, 1)].symbol(), "┌");
        assert_eq!(buf[(0, 1)].fg, palette.heading, "focused border");
        assert_eq!(buf[(preview.x - 1, 1)].symbol(), "┌");
        assert_eq!(buf[(preview.x - 1, 1)].fg, palette.border_dim);
        assert_eq!(state.layout.right_x, preview.x - 1);
        assert_eq!(state.layout.diff_content_x, preview.x);
        assert_eq!(state.layout.right_y, 2);
        for (x, y) in [(119, 0), (tree.x + 2, tree.bottom() - 1), (118, 12)] {
            assert_eq!(buf[(x, y)].bg, Color::Reset, "no fill at ({x},{y})");
        }
    }

    /// Below the minimum size, paint mode puts the notice on `surface`.
    #[test]
    fn paint_mode_too_small_notice_sits_on_surface() {
        let mut state = painted(two_pane_diff_state());
        let mut terminal = Terminal::new(TestBackend::new(20, 5)).unwrap();
        draw_state(&mut terminal, &mut state);
        assert!(state.too_small);
        let surface = state.theme.palette().surface;
        let buf = terminal.backend().buffer();
        for y in 0..5 {
            for x in 0..20 {
                assert_eq!(buf[(x, y)].bg, surface, "({x},{y})");
            }
        }
    }

    /// Paint-mode twin of [`recorded_split_rule_is_the_painted_rule_column`]:
    /// flat content starts at the right pane's first column, the recorded
    /// rule is the painted `border_dim` rule, and the diff h-bar sits on the
    /// pane's last row.
    #[test]
    fn paint_mode_recorded_split_rule_is_the_painted_rule_column() {
        let mut state = painted(two_pane_diff_state());
        state.diff_mode = crate::tui::split::DiffMode::SideBySide;
        let rule_fg = state.theme.palette().border_dim;
        for fraction in [0.3, 0.5, 0.7] {
            state.diff_split_fraction = fraction;
            let mut terminal = Terminal::new(TestBackend::new(220, 20)).unwrap();
            draw_state(&mut terminal, &mut state);
            let rule = state.layout.diff_split_rule_x.expect("rule");
            assert_eq!(
                state.layout.diff_content_x, state.layout.right_x,
                "flat content starts on the right pane's first column"
            );
            let buf = terminal.backend().buffer();
            let y = first_row_with(buf, "old line").expect("diff row");
            let rule_glyph = DIFF_RULE.to_string();
            let painted: Vec<u16> = (0..buf.area.width)
                .filter(|&x| buf[(x, y)].symbol() == rule_glyph && buf[(x, y)].fg == rule_fg)
                .collect();
            assert_eq!(painted.len(), 1, "one split rule: {}", buf_line(buf, y));
            if fraction == crate::tui::split::DIFF_SPLIT_FRACTION {
                assert_eq!(painted[0], rule, "{}", buf_line(buf, y));
            } else {
                assert!(
                    painted[0].abs_diff(rule) <= 1,
                    "fraction {fraction}: painted {} recorded {rule}",
                    painted[0]
                );
            }
        }

        let mut state = painted(long_panning_diff_state(1));
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        let last_pane_row = state.layout.pane_height;
        assert_eq!(
            state.layout.diff_hscrollbar_y,
            Some(last_pane_row),
            "flat diff h-bar on the pane's last row"
        );
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
        // Column 5 is the workspace name: column 3 is its glyph, an icon
        // whose click pins a popover.
        state.dispatch(Action::Click { col: x + 5, row: y });
        state.dispatch(Action::Drag {
            col: x + 1,
            row: y + 1,
        });
        draw_state(&mut terminal, &mut state);
        let painted: HashSet<(u16, u16)> = reversed_cells(&terminal)
            .difference(&baseline)
            .copied()
            .collect();
        let expected: HashSet<(u16, u16)> = (x + 5..=right)
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
        use crate::tui::action::{Action, QuickOpenEntry};
        use crate::tui::tabs::{ONLY_WORKSPACE_TAB_OPEN, WORKSPACE_TAB_CANNOT_CLOSE};
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in "tab".chars() {
            state.dispatch(Action::QuickOpenChar(c));
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
        assert!(
            after_title.trim_start().starts_with("Ctrl-w"),
            "Close tab paints its key chip: {close}"
        );
        assert!(
            after_title.ends_with(WORKSPACE_TAB_CANNOT_CLOSE),
            "then its reason, no group label: {close}"
        );
        let diff = row("Diff commit vs parent in new tab");
        let after_diff = diff
            .split("Diff commit vs parent in new tab")
            .nth(1)
            .unwrap_or_default();
        assert!(
            !after_diff.contains("Ctrl-") && !after_diff.contains('['),
            "a key-less row paints no empty chip: {diff}"
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
        use crate::tui::action::{Action, QuickOpenEntry};
        const GROUPS: [&str; 4] = ["HIGHLIGHT", "MOVE", "GIT", "VIEW"];
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        for _ in 0..60 {
            state.dispatch(Action::QuickOpenMove(1));
            draw_state(&mut terminal, &mut state);
            let text = buffer_text(&terminal);
            let lines: Vec<&str> = text.lines().collect();
            let prompt = lines
                .iter()
                .position(|line| line.contains(">▏"))
                .unwrap_or_else(|| panic!("no `>` query row:\n{text}"));
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

    /// Row index of the first buffer line that contains `needle`.
    fn row_of(text: &str, needle: &str) -> usize {
        text.lines()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no row with {needle:?}:\n{text}"))
    }

    #[test]
    fn list_dialog_input_row_stays_when_results_change() {
        let commits = |n: usize| {
            (0..n)
                .map(|i| crate::git::AncestorCommit {
                    id: format!("{i:02}{}", "a".repeat(38)),
                    subject: format!("commit {i}"),
                })
                .collect::<Vec<_>>()
        };
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        state.open_compare_commit_picker("app".into(), commits(2));
        draw_state(&mut terminal, &mut state);
        let few = buffer_text(&terminal);
        state.open_compare_commit_picker("app".into(), commits(30));
        draw_state(&mut terminal, &mut state);
        let many = buffer_text(&terminal);
        assert_eq!(
            row_of(&few, "Compare vs commit"),
            row_of(&many, "Compare vs commit"),
            "input row moved:\n{few}\n{many}"
        );
        assert_eq!(
            row_of(&few, "Enter compare"),
            row_of(&many, "Enter compare"),
            "footer row moved:\n{few}\n{many}"
        );
        assert!(many.contains("commit 11"), "{many}");
        assert!(!many.contains("commit 12"), "12 rows at most:\n{many}");
    }

    #[test]
    fn dialog_open_keeps_pane_height() {
        use crate::tui::action::QuickOpenEntry;
        use crate::tui::quick_open::{QuickOpenScope, QuickOpenState};
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let idle = state.layout.pane_height;
        state.help_open = true;
        draw_state(&mut terminal, &mut state);
        assert_eq!(state.layout.pane_height, idle, "help");
        state.help_open = false;
        state.confirm = Some(PendingConfirm::StashDrop {
            repo: "app".into(),
            stash_ref: "stash@{0}".into(),
        });
        draw_state(&mut terminal, &mut state);
        assert_eq!(state.layout.pane_height, idle, "confirm");
        state.confirm = None;
        state.quick_open = Some(QuickOpenState::new(
            QuickOpenEntry::Commands,
            QuickOpenScope::Workspace,
        ));
        draw_state(&mut terminal, &mut state);
        assert_eq!(state.layout.pane_height, idle, "palette");
    }

    /// Files-mode Quick Open on `app` with 50 indexed paths, `hits` of them
    /// ranked for the query `file`.
    fn quick_open_files_state(hits: usize) -> AppState {
        use crate::file_index::{FileEntry, FileHit, FileIndex, IndexRoot};
        use crate::tui::action::QuickOpenEntry;
        use crate::tui::quick_open::QuickOpenScope;
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let index = FileIndex {
            roots: vec![IndexRoot {
                checkout: "app".into(),
                prefix: String::new(),
            }],
            entries: (0..50)
                .map(|i| FileEntry {
                    root: 0,
                    display: format!("src/file{i:02}.rs"),
                    rel_start: 0,
                })
                .collect(),
            truncated: false,
            errors: Vec::new(),
        };
        let mut quick = QuickOpenState::new(
            QuickOpenEntry::Files,
            QuickOpenScope::Checkout("app".into()),
        );
        quick.query = "file".into();
        quick.index = FileIndexState::Ready(std::sync::Arc::new(index));
        quick.hits = (0..hits)
            .map(|entry| FileHit {
                entry,
                score: 1,
                indices: vec![4, 5, 6, 7],
            })
            .collect();
        state.quick_open = Some(quick);
        state
    }

    #[test]
    fn quick_open_input_row_stays_when_hit_count_changes() {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut state = quick_open_files_state(1);
        draw_state(&mut terminal, &mut state);
        let few = buffer_text(&terminal);
        let mut state = quick_open_files_state(50);
        draw_state(&mut terminal, &mut state);
        let many = buffer_text(&terminal);
        assert!(few.contains("Go to file · app"), "{few}");
        assert_eq!(
            row_of(&few, "file▏"),
            row_of(&many, "file▏"),
            "query row moved:\n{few}\n{many}"
        );
        assert_eq!(
            row_of(&few, "Enter open"),
            row_of(&many, "Enter open"),
            "footer row moved:\n{few}\n{many}"
        );
        assert!(few.contains("50 files"), "status row: {few}");
        // A status from outside the overlay never hides the file count.
        let mut state = quick_open_files_state(1);
        state.status = "Fetched 2 repos".into();
        draw_state(&mut terminal, &mut state);
        let leftover = buffer_text(&terminal);
        assert!(leftover.contains("50 files"), "{leftover}");
        assert!(!leftover.contains("Fetched 2 repos"), "{leftover}");
        assert!(many.contains("src/file11.rs"), "{many}");
        assert!(!many.contains("src/file12.rs"), "12 rows at most:\n{many}");
    }

    #[test]
    fn quick_open_files_footer_is_not_enter_run() {
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        let mut state = quick_open_files_state(3);
        draw_state(&mut terminal, &mut state);
        let files = buffer_text(&terminal);
        assert!(files.contains("Enter open"), "{files}");
        assert!(!files.contains("Enter run"), "{files}");
        assert!(
            files.contains("↑↓ move · Enter open · > commands · # search · Esc close"),
            "{files}"
        );
        // A narrow box drops chips from the left; `Esc close` stays.
        let mut narrow = Terminal::new(TestBackend::new(50, 30)).unwrap();
        draw_state(&mut narrow, &mut state);
        let text = buffer_text(&narrow);
        let footer = text
            .lines()
            .find(|line| line.contains("Esc close"))
            .unwrap_or_else(|| panic!("footer keeps `Esc close`:\n{text}"));
        assert!(!footer.contains("↑↓ move"), "{text}");
        assert!(footer.contains("# search"), "{text}");
        state.dispatch(Action::QuickOpenBackspace);
        state.dispatch(Action::QuickOpenBackspace);
        state.dispatch(Action::QuickOpenBackspace);
        state.dispatch(Action::QuickOpenBackspace);
        state.dispatch(Action::QuickOpenChar('>'));
        draw_state(&mut terminal, &mut state);
        let commands = buffer_text(&terminal);
        assert!(commands.contains("Enter run"), "{commands}");
        assert!(!commands.contains("Enter open"), "{commands}");
        assert!(commands.contains(">▏"), "the `>` stays in the query row");
    }

    #[test]
    fn quick_open_cuts_long_paths_from_the_left_and_bolds_matches() {
        use crate::file_index::{FileEntry, FileHit};
        let mut state = quick_open_files_state(0);
        let long = format!("{}/main.rs", "deep".repeat(40));
        let quick = state.quick_open.as_mut().unwrap();
        let FileIndexState::Ready(index) = &mut quick.index else {
            unreachable!()
        };
        std::sync::Arc::make_mut(index).entries.push(FileEntry {
            root: 0,
            display: long.clone(),
            rel_start: 0,
        });
        let last = long.chars().count() as u32;
        quick.query = "main".into();
        quick.hits = vec![FileHit {
            entry: 50,
            score: 1,
            indices: (last - 7..last - 3).collect(),
        }];
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        let y = row_of(&text, "/main.rs") as u16;
        let line = text.lines().nth(y as usize).unwrap();
        assert!(
            line.contains("❯ …"),
            "head cut with a leading ellipsis: {line}"
        );
        assert!(!line.contains(&"deep".repeat(40)), "{line}");
        let buf = terminal.backend().buffer();
        let x = (0..buf.area().width)
            .find(|&x| {
                (0..7u16).all(|i| buf[(x + i, y)].symbol() == &"main.rs"[i as usize..=i as usize])
            })
            .expect("main.rs cells");
        for i in 0..4 {
            assert!(
                buf[(x + i, y)].modifier.contains(Modifier::BOLD),
                "matched char {i} is bold"
            );
        }
        assert!(
            !buf[(x + 4, y)].modifier.contains(Modifier::BOLD),
            "`.` is not"
        );
    }

    /// Search dialog over `app` (or all repos) with `needle` hits in two
    /// files: `src/lib.rs` lines 3 and 12, `src/main.rs` line 7.
    fn search_files_state(all_repos: bool) -> AppState {
        use crate::file_index::{FileEntry, IndexRoot};
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let (scope, prefix) = if all_repos {
            (QuickOpenScope::Workspace, "app/")
        } else {
            (QuickOpenScope::Checkout("app".into()), "")
        };
        let index = FileIndex {
            roots: vec![IndexRoot {
                checkout: "app".into(),
                prefix: prefix.into(),
            }],
            entries: ["src/lib.rs", "src/main.rs"]
                .iter()
                .map(|rel| FileEntry {
                    root: 0,
                    display: format!("{prefix}{rel}"),
                    rel_start: prefix.len() as u32,
                })
                .collect(),
            truncated: false,
            errors: Vec::new(),
        };
        let mut dialog = SearchFilesState::new(scope, "needle".into(), SearchOptions::default());
        dialog.index = FileIndexState::Ready(Arc::new(index));
        dialog.hits = vec![
            SearchHit {
                entry: 0,
                line: 3,
                text: "let needle = needle();".into(),
                ranges: vec![(4, 10), (13, 19)],
            },
            SearchHit {
                entry: 0,
                line: 12,
                text: "// needle".into(),
                ranges: vec![(3, 9)],
            },
            SearchHit {
                entry: 1,
                line: 7,
                text: "needle".into(),
                ranges: vec![(0, 6)],
            },
        ];
        dialog.hits_query = "needle".into();
        state.search_files = Some(dialog);
        state
    }

    fn draw_search(state: &mut AppState, cols: u16, rows: u16) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        draw_state(&mut terminal, state);
        terminal
    }

    /// Column of the first cell run on row `y` that spells `needle`.
    fn cells_x(terminal: &Terminal<TestBackend>, y: usize, needle: &str) -> u16 {
        let buf = terminal.backend().buffer();
        let chars: Vec<char> = needle.chars().collect();
        (0..buf.area().width)
            .find(|&x| {
                chars.iter().enumerate().all(|(i, ch)| {
                    x + (i as u16) < buf.area().width
                        && buf[(x + i as u16, y as u16)].symbol() == ch.to_string()
                })
            })
            .unwrap_or_else(|| panic!("no {needle:?} on row {y}"))
    }

    fn search_dialog(state: &mut AppState) -> &mut SearchFilesState {
        state.search_files.as_mut().expect("search dialog open")
    }

    #[test]
    fn search_files_paints_title_query_grouped_hits_and_footer() {
        let mut state = search_files_state(false);
        let terminal = draw_search(&mut state, 120, 30);
        let text = buffer_text(&terminal);
        assert!(text.contains("Search · app"), "{text}");
        assert!(
            text.contains("needle▏"),
            "query row with its caret:\n{text}"
        );
        let rows = [
            row_of(&text, "needle▏"),
            row_of(&text, " src/lib.rs "),
            row_of(&text, "❯  3: let needle = needle();"),
            row_of(&text, "   12: // needle"),
            row_of(&text, " src/main.rs "),
            row_of(&text, "    7: needle"),
        ];
        assert!(rows.windows(2).all(|w| w[0] < w[1]), "{rows:?}\n{text}");
        assert!(
            text.contains("↑↓ move · Enter open · Tab all repos · Alt-c/w/r · Esc close"),
            "{text}"
        );
        assert_eq!(
            row_of(&text, "Esc close"),
            row_of(&text, "Search · app") + 23,
            "footer on the last inner row of a 25-row box"
        );
        assert_eq!(state.layout.search_files_rows, 20);

        search_dialog(&mut state).zone = SearchZone::Results;
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert!(
            text.contains("j/k move · Enter open · e edit · Tab all repos · Alt-c/w/r · Esc close"),
            "{text}"
        );

        let mut state = search_files_state(true);
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert!(text.contains("Search · all repos"), "{text}");
        assert!(text.contains("app › src/lib.rs"), "{text}");
        assert!(text.contains("app › src/main.rs"), "{text}");
        assert!(!text.contains("Tab all repos"), "{text}");

        search_dialog(&mut state).query.clear();
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert!(text.contains("▏type to search…"), "{text}");
    }

    #[test]
    fn search_files_chips_show_which_options_are_on() {
        let mut state = search_files_state(false);
        let palette = state.theme.palette();
        let terminal = draw_search(&mut state, 120, 30);
        let text = buffer_text(&terminal);
        let y = row_of(&text, "Search · app");
        let line = text.lines().nth(y).unwrap();
        assert!(line.contains(" Aa   ab   .* "), "{line}");
        let buf = terminal.backend().buffer();
        for chip in ["Aa", "ab", ".*"] {
            let x = cells_x(&terminal, y, chip);
            assert_eq!(buf[(x, y as u16)].fg, palette.muted, "{chip} off");
            assert_eq!(buf[(x, y as u16)].bg, palette.panel, "{chip} off");
        }

        search_dialog(&mut state).options = SearchOptions {
            case_sensitive: true,
            whole_word: false,
            regex: true,
        };
        let terminal = draw_search(&mut state, 120, 30);
        let buf = terminal.backend().buffer();
        for (chip, on) in [("Aa", true), ("ab", false), (".*", true)] {
            let x = cells_x(&terminal, y, chip);
            let cell = &buf[(x, y as u16)];
            assert_eq!(cell.bg == palette.cursor, on, "{chip}: {cell:?}");
            assert_eq!(cell.modifier.contains(Modifier::BOLD), on, "{chip}");
        }
    }

    #[test]
    fn search_files_highlights_every_match_on_a_hit_row() {
        let mut state = search_files_state(false);
        let palette = state.theme.palette();
        let filter = state.theme.pills().filter;
        let terminal = draw_search(&mut state, 99, 30);
        let text = buffer_text(&terminal);
        let buf = terminal.backend().buffer();

        let y = row_of(&text, "let needle = needle();");
        let x = cells_x(&terminal, y, "let needle = needle();");
        let at = |dx: u16| &buf[(x + dx, y as u16)];
        for dx in (4..10).chain(13..19) {
            assert_eq!(at(dx).bg, filter.bg, "match cell {dx}");
            assert_eq!(at(dx).fg, filter.fg, "match cell {dx}");
        }
        for dx in [0, 3, 10, 12, 19] {
            assert_eq!(at(dx).bg, palette.cursor_bg, "highlighted row cell {dx}");
        }

        let y = row_of(&text, "// needle");
        let x = cells_x(&terminal, y, "// needle");
        assert_eq!(buf[(x, y as u16)].bg, palette.panel, "plain text");
        assert_eq!(buf[(x, y as u16)].fg, palette.muted);
        assert_eq!(buf[(x + 3, y as u16)].bg, filter.bg, "match");
    }

    #[test]
    fn search_files_cuts_a_long_hit_line_to_show_its_match() {
        let mut state = search_files_state(false);
        let long = format!("{}needle", "x".repeat(200));
        search_dialog(&mut state).hits[0] = SearchHit {
            entry: 0,
            line: 3,
            text: long,
            ranges: vec![(200, 206)],
        };
        let text = buffer_text(&draw_search(&mut state, 99, 30));
        let line = text.lines().nth(row_of(&text, "❯  3: …")).unwrap();
        assert!(line.contains("xneedle"), "the match shows: {line}");
    }

    #[test]
    fn search_files_status_row_texts() {
        let mut state = search_files_state(false);
        let palette = state.theme.palette();
        search_dialog(&mut state).hits.clear();
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert_eq!(
            row_of(&text, "no matches in app"),
            row_of(&text, "needle▏") + 1,
            "status row under the query:\n{text}"
        );

        let dialog = search_dialog(&mut state);
        dialog.hits.push(SearchHit {
            entry: 0,
            line: 1,
            text: "needle".into(),
            ranges: vec![(0, 6)],
        });
        dialog.capped = true;
        dialog.skipped = 2;
        dialog.error = Some("unclosed group".into());
        let terminal = draw_search(&mut state, 120, 30);
        let text = buffer_text(&terminal);
        let y = row_of(&text, "invalid regex: unclosed group");
        let line = text.lines().nth(y).unwrap();
        assert!(
            line.contains(
                "invalid regex: unclosed group · 5000+ hits · showing first 5000 · narrow the query · 2 skipped (binary / >2 MiB)"
            ),
            "{line}"
        );
        let buf = terminal.backend().buffer();
        let x = cells_x(&terminal, y, "invalid regex");
        assert_eq!(buf[(x, y as u16)].fg, palette.deleted, "error is red");
        let x = cells_x(&terminal, y, "2 skipped");
        assert!(buf[(x, y as u16)].modifier.contains(Modifier::DIM), "dim");
        let x = cells_x(&terminal, y, "5000+");
        assert!(!buf[(x, y as u16)].modifier.contains(Modifier::DIM));

        // Enter's miss warning and the quit prompt show in the dialog.
        state.status =
            crate::tui::status::StatusMessage::warn(crate::tui::search_files::NO_SEARCH_MATCHES);
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert_eq!(row_of(&text, "no matches"), y, "{text}");
        assert!(!text.contains("invalid regex"), "{text}");
        state.status = crate::tui::ctrl_c_exit::CTRL_C_EXIT_PROMPT.into();
        assert_eq!(super::super::chrome::ctrl_c_prompt_rows(&state), 0);
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert_eq!(row_of(&text, "Press Ctrl-c again to exit"), y, "{text}");
        // A leftover status from outside the dialog never hides its own.
        state.status = "Fetched 2 repos".into();
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert!(!text.contains("Fetched 2 repos"), "{text}");
        assert!(text.contains("invalid regex"), "{text}");
    }

    #[test]
    fn search_files_preview_shows_at_100_columns_and_hides_at_99() {
        use crate::tui::search_files::SearchPreview;
        let mut state = search_files_state(false);
        let palette = state.theme.palette();
        let filter = state.theme.pills().filter;
        let mut lines: Vec<String> = (1..=40).map(|n| format!("// line {n}")).collect();
        lines[2] = "let needle = needle();".into();
        lines[11] = "// needle".into();
        search_dialog(&mut state).preview = Some(SearchPreview {
            checkout: "app".into(),
            rel: "src/lib.rs".into(),
            gen: 1,
            body: Some(FileRead::Text {
                lines,
                max_cols: 22,
            }),
        });

        let terminal = draw_search(&mut state, 100, 30);
        let text = buffer_text(&terminal);
        assert!(text.contains("src/lib.rs:3"), "preview header:\n{text}");
        let y = row_of(&text, " 3 let needle = needle();");
        assert_eq!(y, row_of(&text, "❯  3:") + 2, "hit line near the top");
        assert!(text.contains("12 // needle"), "{text}");
        assert!(text.contains(" 1 // line 1"), "{text}");
        let buf = terminal.backend().buffer();
        let x = cells_x(&terminal, y, " 3 let needle");
        assert_eq!(buf[(x + 1, y as u16)].bg, palette.cursor_bg, "hit line");
        assert_eq!(buf[(x + 7, y as u16)].bg, filter.bg, "match in preview");
        let y12 = row_of(&text, "12 // needle");
        let x = cells_x(&terminal, y12, "12 // needle");
        assert_eq!(buf[(x + 6, y12 as u16)].bg, filter.bg, "other hit in view");
        assert_ne!(buf[(x, y12 as u16)].bg, palette.cursor_bg);

        let text = buffer_text(&draw_search(&mut state, 99, 30));
        assert!(!text.contains("src/lib.rs:3"), "{text}");
        assert!(!text.contains("// line 1"), "{text}");

        // A hit deep in the file centers its line.
        search_dialog(&mut state).hits[1].line = 30;
        search_dialog(&mut state).cursor = 1;
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        let header = row_of(&text, "src/lib.rs:30");
        let hit = row_of(&text, "30 // line 30");
        assert_eq!(hit, header + 1 + 9, "19 code rows, hit in the middle");
    }

    #[test]
    fn search_files_preview_notices() {
        use crate::tui::search_files::SearchPreview;
        let mut state = search_files_state(false);
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert!(text.contains("loading…"), "no read yet:\n{text}");
        let preview = |body| SearchPreview {
            checkout: "app".into(),
            rel: "src/lib.rs".into(),
            gen: 1,
            body: Some(body),
        };
        for (body, want) in [
            (FileRead::Binary, "binary file"),
            (FileRead::TooLarge { bytes: 3 << 20 }, "file is over 2 MiB"),
            (
                FileRead::Failed("permission denied".into()),
                "permission denied",
            ),
        ] {
            search_dialog(&mut state).preview = Some(preview(body));
            let terminal = draw_search(&mut state, 120, 30);
            let text = buffer_text(&terminal);
            let y = row_of(&text, want);
            let x = cells_x(&terminal, y, want);
            let cell = &terminal.backend().buffer()[(x, y as u16)];
            assert!(cell.modifier.contains(Modifier::DIM), "{want} is dim");
        }
        // A preview of another file is not shown for this hit.
        search_dialog(&mut state).cursor = 2;
        let text = buffer_text(&draw_search(&mut state, 120, 30));
        assert!(text.contains("src/main.rs:7"), "{text}");
        assert!(text.contains("loading…"), "{text}");
    }

    #[test]
    fn search_files_scroll_keeps_the_highlighted_hit_in_view() {
        let mut state = search_files_state(false);
        let dialog = search_dialog(&mut state);
        dialog.hits = (1..=60)
            .map(|line| SearchHit {
                entry: 0,
                line,
                text: format!("needle {line}"),
                ranges: vec![(0, 6)],
            })
            .collect();
        dialog.cursor = 59;
        let text = buffer_text(&draw_search(&mut state, 120, 20));
        assert!(text.contains("❯ 60: needle 60"), "{text}");
        let scroll = search_dialog(&mut state).scroll;
        assert!(scroll > 0, "scrolled to the last hit");
        let rows = usize::from(state.layout.search_files_rows);
        assert_eq!(scroll, 61 - rows, "last row at the bottom");

        // Moving up inside the window keeps the scroll.
        state.dispatch(Action::SearchFilesMove(-2));
        draw_search(&mut state, 120, 20);
        assert_eq!(search_dialog(&mut state).scroll, scroll);
        // PgUp moves by the painted height.
        state.dispatch(Action::SearchFilesPage(-1));
        assert_eq!(search_dialog(&mut state).cursor, 57 - (rows - 1));
    }

    #[test]
    fn status_row_stays_last_with_dialog_open() {
        use crate::tui::action::QuickOpenEntry;
        use crate::tui::quick_open::{QuickOpenScope, QuickOpenState};
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.quick_open = Some(QuickOpenState::new(
            QuickOpenEntry::Commands,
            QuickOpenScope::Workspace,
        ));
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        let lines: Vec<&str> = text.lines().collect();
        assert!(text.contains("Enter run"), "{text}");
        assert_eq!(lines[29].trim(), "", "status row is blank:\n{text}");
        let crumb: String = breadcrumb_line(&state, 120)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(!crumb.trim().is_empty());
        assert_eq!(
            lines[28].trim_end(),
            crumb.trim_end(),
            "breadcrumb row:\n{text}"
        );
    }

    #[test]
    fn help_dialog_scrolls_when_columns_exceed_box() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        assert_eq!(state.dispatch(Action::ToggleHelp), Effect::None);
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        draw_state(&mut terminal, &mut state);
        let max = state.layout.help_scroll_max;
        assert!(max > 3, "help is taller than 16 rows: {max}");
        let text = buffer_text(&terminal);
        assert!(text.contains("j/k scroll"), "{text}");
        let header = row_of(&text, "MOVE");
        let first = text.lines().nth(header + 1).unwrap().to_string();
        assert_eq!(state.dispatch(Action::HelpScroll(3)), Effect::None);
        assert_eq!(state.help_scroll, 3);
        draw_state(&mut terminal, &mut state);
        let scrolled = buffer_text(&terminal);
        assert_eq!(row_of(&scrolled, "MOVE"), header, "title row stays pinned");
        assert_ne!(
            scrolled.lines().nth(header + 1).unwrap(),
            first,
            "body scrolled:\n{text}\n{scrolled}"
        );
        state.dispatch(Action::HelpScroll(1000));
        assert_eq!(state.help_scroll, max);

        // A taller terminal lowers the max; the scroll follows it down so
        // the first `k` still moves the body.
        let mut taller = Terminal::new(TestBackend::new(80, 24)).unwrap();
        draw_state(&mut taller, &mut state);
        let lower = state.layout.help_scroll_max;
        assert!(lower < max, "{lower} < {max}");
        assert_eq!(state.help_scroll, lower);
        state.dispatch(Action::HelpScroll(-1));
        assert_eq!(state.help_scroll, lower.saturating_sub(1));
        state.dispatch(Action::HelpScroll(-1000));
        assert_eq!(state.help_scroll, 0);
    }

    #[test]
    fn list_dialog_footer_keeps_esc_on_narrow_terminals() {
        use crate::git::LocalBranch;
        use crate::tui::graph_focus::GraphFocusPickerState;
        let snapshot = build_workspace_snapshot(&[repo("app", false)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.graph_focus_picker = Some(GraphFocusPickerState::new(
            "app".into(),
            vec![LocalBranch {
                name: "main".into(),
                current: true,
                authordate: 0,
            }],
            &[],
        ));
        let mut terminal = Terminal::new(TestBackend::new(50, 20)).unwrap();
        draw_state(&mut terminal, &mut state);
        let text = buffer_text(&terminal);
        let footer = text
            .lines()
            .find(|line| line.contains("Enter apply"))
            .unwrap_or_else(|| panic!("footer row:\n{text}"));
        assert!(footer.contains("Esc cancel"), "{text}");
        assert!(!footer.contains("↑↓ move"), "{text}");

        // A footer that fits keeps its exact text.
        let mut wide = Terminal::new(TestBackend::new(120, 30)).unwrap();
        draw_state(&mut wide, &mut state);
        assert!(buffer_text(&wide).contains(
            "↑↓ move · type to filter · space toggle · Enter apply · Ctrl-o clear · Esc cancel"
        ));
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

    /// `app` with a file tab on `README.md` loaded as `lines`.
    fn file_tab_state(lines: &[&str]) -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        let Effect::LoadFileTab { tab_id, gen, .. } =
            state.open_file_tab("app".into(), "README.md".into())
        else {
            panic!("expected a load");
        };
        let body = FileRead::Text {
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
            max_cols: lines.iter().map(|line| line.len()).max().unwrap_or(0),
        };
        assert!(state.apply_file_tab(tab_id, gen, body));
        state
    }

    #[test]
    fn file_tab_paints_full_width_pane_titled_with_path() {
        let mut state = file_tab_state(&["# app", "dirty"]);
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buf = terminal.backend().buffer();
        let top = buf_line(buf, 1);
        let corners: Vec<(usize, char)> = top
            .chars()
            .enumerate()
            .filter(|(_, c)| matches!(c, '╭' | '╮' | '┌' | '┐'))
            .collect();
        assert_eq!(corners.len(), 2, "one border pair:\n{top}");
        assert_eq!(corners[0].0, 0, "{top}");
        assert_eq!(corners[1].0, 79, "{top}");
        assert!(top.contains("app/README.md"), "{top}");
        assert!(
            buf_line(buf, 2).starts_with("│1 # app"),
            "{}",
            buf_line(buf, 2)
        );
        assert!(
            buf_line(buf, 3).starts_with("│2 dirty"),
            "{}",
            buf_line(buf, 3)
        );
        assert_eq!(state.layout.file_view_x, 1);
        assert_eq!(state.layout.file_view_y, 2);
        assert_eq!(state.layout.file_view_width, 78);
        assert_eq!(state.layout.file_view_row_lines, vec![0, 1]);
        let crumb = buffer_text(&terminal);
        assert!(crumb.contains("app › README.md"), "{crumb}");
        assert!(crumb.contains("edit") && crumb.contains("quit"), "{crumb}");
    }

    /// `app` with an Explorer tab at its root, listed as: `src/` (with a
    /// change inside), `target/` (ignored), `new.txt` (untracked),
    /// `README.md` (modified).
    fn explorer_state() -> AppState {
        let mut app = repo("app", true);
        app.changes.push(FileChange {
            path: "new.txt".into(),
            staged_status: None,
            unstaged_status: None,
            untracked: true,
            old_path: None,
        });
        app.changes.push(FileChange {
            path: "src/a.rs".into(),
            staged_status: None,
            unstaged_status: Some("M".into()),
            untracked: false,
            old_path: None,
        });
        let snapshot = build_workspace_snapshot(&[app], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, false);
        state.cursor = state
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::Repo)
            .expect("repo row");
        let Effect::LoadExplorerDir { tab_id, req, .. } = state.dispatch(Action::ExplorerReveal)
        else {
            panic!("a root listing");
        };
        let entry = |name: &str, is_dir: bool, ignored: bool| crate::tui::explorer::ExplorerEntry {
            name: name.into(),
            is_dir,
            ignored,
        };
        let listing = vec![
            entry("src", true, false),
            entry("target", true, true),
            entry("new.txt", false, false),
            entry("README.md", false, false),
        ];
        state
            .apply_explorer_dir(tab_id, req, "", Ok(listing))
            .expect("fresh listing");
        state
    }

    /// [`explorer_state`] with the cursor on `rel` and its preview loaded:
    /// a diff when `diff` is set, else a clean-file body (`new.txt` then
    /// counts as clean).
    fn explorer_preview_state(rel: &str, diff: Option<&str>) -> AppState {
        use crate::tui::effect::ExplorerPreviewBody;
        let mut state = explorer_state();
        if diff.is_none() {
            state.snapshot.repos[0]
                .changes
                .retain(|change| change.path != rel);
        }
        // Each step moves one row down, so the tree's row count bounds the
        // walk; a fixture without `rel` fails here instead of hanging.
        let rows = state.explorer_rows().expect("an Explorer tab").0.len();
        let found = (0..rows).find_map(|_| match state.dispatch(Action::Move(1)) {
            Effect::LoadExplorerPreview {
                tab_id, gen, path, ..
            } if path == rel => Some((tab_id, gen)),
            _ => None,
        });
        let Some((tab_id, gen)) = found else {
            panic!("Explorer tree has no row for {rel} within {rows} moves");
        };
        let body = match diff {
            Some(unified) => {
                ExplorerPreviewBody::Diff(super::super::diff::DiffContent::from_unified(unified))
            }
            None => ExplorerPreviewBody::File(FileRead::Text {
                lines: (1..=40).map(|n| format!("line {n}")).collect(),
                max_cols: 7,
            }),
        };
        assert!(state.apply_explorer_preview(tab_id, gen, body));
        state
    }

    /// The Explorer tree paints status letters (`??` for untracked), a `●`
    /// on a folder with changes, and dims ignored entries.
    #[test]
    fn explorer_tree_paints_status_marks_and_dims_ignored() {
        let mut state = explorer_state();
        let palette = state.theme.palette();
        let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buf = terminal.backend().buffer();
        assert!(buf_line(buf, 1).contains("app"), "{}", buf_line(buf, 1));
        let tree = state.layout.explorer_tree;
        let row_of = |name: &str| {
            (tree.y..tree.bottom())
                .find(|y| find_cell_col(buf, *y, name).is_some_and(|x| x < tree.right()))
                .unwrap_or_else(|| panic!("no {name} row"))
        };
        let tree_text = |y: u16| -> String {
            (tree.x..tree.right())
                .map(|x| buf[(x, y)].symbol().to_string())
                .collect()
        };
        let name_style = |name: &str| {
            let y = row_of(name);
            buf[(find_cell_col(buf, y, name).unwrap(), y)].style()
        };

        let src = tree_text(row_of("src"));
        assert!(src.trim_end().ends_with('●'), "{src}");
        let readme = tree_text(row_of("README.md"));
        assert!(readme.trim_end().ends_with('M'), "{readme}");
        assert_eq!(name_style("README.md").fg, Some(palette.modified));
        let new = tree_text(row_of("new.txt"));
        assert!(new.trim_end().ends_with("??"), "{new}");
        assert_eq!(name_style("new.txt").fg, Some(palette.added));

        let ignored = name_style("target");
        assert_eq!(ignored.fg, Some(palette.muted));
        assert!(ignored.add_modifier.contains(Modifier::DIM), "{ignored:?}");
        let normal = name_style("README.md");
        assert!(!normal.add_modifier.contains(Modifier::DIM), "{normal:?}");
        let target = tree_text(row_of("target"));
        assert!(!target.contains('●'), "{target}");

        // The cursor sits on `src/`, a folder: the preview shows a hint.
        let preview = state.layout.explorer_preview;
        assert!(buf_line(buf, preview.y).contains(EXPLORER_FOLDER_HINT));
        let crumb = buffer_text(&terminal);
        assert!(crumb.contains("app › src"), "{crumb}");
        assert!(crumb.contains("parent"), "{crumb}");
    }

    /// The Explorer preview paints a changed file with the diff pane and a
    /// clean file with the file-tab body.
    #[test]
    fn explorer_preview_paints_diff_or_file_body() {
        use crate::tui::effect::ExplorerPreviewBody;
        let mut state = explorer_state();
        let mut terminal = Terminal::new(TestBackend::new(110, 16)).unwrap();
        let to = |state: &mut AppState, rel: &str| -> (u64, u64) {
            loop {
                let effect = state.dispatch(Action::Move(1));
                if let Effect::LoadExplorerPreview {
                    tab_id, gen, path, ..
                } = effect
                {
                    if path == rel {
                        return (tab_id, gen);
                    }
                }
            }
        };
        let (tab_id, gen) = to(&mut state, "README.md");
        let diff =
            super::super::diff::DiffContent::from_unified("@@ -1 +1 @@\n-# seed\n+# dirty\n");
        assert!(state.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::Diff(diff)));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let preview = state.layout.explorer_preview;
        let text = |terminal: &Terminal<TestBackend>| -> String {
            let buf = terminal.backend().buffer();
            (preview.y..preview.bottom())
                .map(|y| {
                    (preview.x..preview.right())
                        .map(|x| buf[(x, y)].symbol().to_string())
                        .collect::<String>()
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let pane = text(&terminal);
        assert!(pane.contains("app/README.md"), "{pane}");
        assert!(pane.contains("# dirty"), "{pane}");

        state.dispatch(Action::MoveToStart);
        let (tab_id, gen) = to(&mut state, "README.md");
        assert!(state.explorer_diff_loading(), "a new pick loads again");
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(text(&terminal).contains("loading…"), "{}", text(&terminal));
        assert!(!state.apply_explorer_preview(
            tab_id,
            gen,
            ExplorerPreviewBody::File(FileRead::Binary)
        ));

        // `new.txt` is untracked, so it previews as a diff too; check a
        // clean file through the file body instead.
        state.snapshot.repos[0]
            .changes
            .retain(|change| change.path != "new.txt");
        state.dispatch(Action::MoveToStart);
        let (tab_id, gen) = to(&mut state, "new.txt");
        let body = FileRead::Text {
            lines: vec!["hello".into()],
            max_cols: 5,
        };
        assert!(state.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::File(body)));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let pane = text(&terminal);
        assert!(pane.starts_with("1 hello"), "{pane}");
        let buf = terminal.backend().buffer();
        assert!(
            buf_line(buf, 1).contains("app/new.txt"),
            "{}",
            buf_line(buf, 1)
        );
    }

    /// Cache `summary` as the blame of the line `state` focuses now.
    fn cache_focused_blame(state: &mut AppState, summary: &str) {
        let key = state.line_blame_want().expect("focused line asks");
        state.line_blame.insert(
            key,
            Some(crate::git::LineBlame {
                sha: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
                author: "Ada".into(),
                author_time: 0,
                summary: summary.into(),
                orig_line: 1,
                filename: "README.md".into(),
                previous: None,
                boundary: false,
                uncommitted: false,
            }),
        );
    }

    #[test]
    fn blame_annotation_on_a_focused_add_row_sits_on_the_flat_cursor_bar() {
        for id in [ThemeId::Dracula, ThemeId::TokyoNight] {
            let mut state = two_pane_diff_state();
            state.theme = id;
            state.diff_mode = DiffMode::Inline;
            state.set_diff(
                "app".into(),
                "README.md".into(),
                super::super::diff::DiffContent::from_unified(
                    "@@ -1,3 +1,3 @@\n keep one\n-old line\n+new line\n keep two\n",
                ),
            );
            state.focus = FocusPane::Right;
            let palette = id.palette();
            let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            state.diff_cursor = diff_row_index(&state, "new line");
            // An unstaged added line reads `You · uncommitted` without git.
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let y = first_row_with(buf, "new line").expect("focused add row");
            let tinted_row = palette.cursor_tint(palette.diff_add_bg, palette.cursor_bg);
            let code_end = needle_cols(buf, y, "new line").last().copied().unwrap();
            assert_eq!(
                buf[(code_end + 1, y)].bg,
                tinted_row,
                "{id:?} pad before the note:\n{text}"
            );
            for cell in needle_cells(buf, y, "You · uncommitted") {
                assert_eq!(
                    (cell.bg, cell.fg),
                    (palette.cursor_bg, palette.muted),
                    "{id:?} note on the flat cursor bar:\n{text}"
                );
            }
            let after = needle_cols(buf, y, "You · uncommitted")
                .last()
                .copied()
                .unwrap()
                + 1;
            assert_eq!(
                (buf[(after, y)].symbol(), buf[(after, y)].bg),
                (" ", palette.cursor_bg),
                "{id:?} pad after the note:\n{text}"
            );
        }
    }

    #[test]
    fn focused_diff_row_ends_with_the_dimmed_blame_annotation() {
        for mode in [DiffMode::Inline, DiffMode::SideBySide] {
            let mut state = two_pane_diff_state();
            state.diff_mode = mode;
            state.set_diff(
                "app".into(),
                "README.md".into(),
                super::super::diff::DiffContent::from_unified(
                    "@@ -1,3 +1,3 @@\n keep one\n-old line\n+new line\n keep two\n",
                ),
            );
            state.focus = FocusPane::Right;
            let palette = state.theme.palette();
            let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
            draw_state(&mut terminal, &mut state);
            state.diff_cursor = diff_row_index(&state, "keep one");
            cache_focused_blame(&mut state, "fix the parser");
            draw_state(&mut terminal, &mut state);
            let buf = terminal.backend().buffer();
            let text = buffer_text(&terminal);
            let y = first_row_with(buf, "keep one").expect("focused row");
            let row = buf_line(buf, y);
            assert!(
                row.contains("Ada, ") && row.contains(" · aaa1111 · fix the parser"),
                "{mode:?}:\n{text}"
            );
            let sha = find_cell_col(buf, y, "aaa1111").unwrap();
            assert_eq!(buf[(sha, y)].fg, palette.muted, "{mode:?}");
            assert_eq!(
                text.matches("aaa1111").count(),
                1,
                "only the focused row:\n{text}"
            );
            if mode == DiffMode::SideBySide {
                let rule = row.find(DIFF_RULE).expect("split rule");
                assert!(row.find("aaa1111").unwrap() > rule, "new side: {row}");
            }

            // A drag selection copies screen cells: no annotation then.
            state.text_selection = Some(super::super::selection::TextSelection {
                pane: Rect::default(),
                anchor: (0, 0),
                head: (0, 0),
            });
            draw_state(&mut terminal, &mut state);
            assert!(!buffer_text(&terminal).contains("aaa1111"), "{mode:?}");
            state.text_selection = None;

            // The left pane focused: the diff row is not focused.
            state.focus = FocusPane::Left;
            draw_state(&mut terminal, &mut state);
            assert!(!buffer_text(&terminal).contains("aaa1111"), "{mode:?}");
        }
    }

    #[test]
    fn file_tab_cursor_line_ends_with_the_blame_annotation() {
        let mut state = file_tab_state(&["# app", "dirty"]);
        let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
        draw_state(&mut terminal, &mut state);
        cache_focused_blame(&mut state, "seed");
        draw_state(&mut terminal, &mut state);
        let buf = terminal.backend().buffer();
        assert!(
            buf_line(buf, 2).contains(" · aaa1111 · seed"),
            "{}",
            buf_line(buf, 2)
        );
        assert!(!buf_line(buf, 3).contains("aaa1111"));
        state.dispatch(Action::ToggleLineBlame);
        draw_state(&mut terminal, &mut state);
        assert!(!buffer_text(&terminal).contains("aaa1111"), "B hides it");
    }

    #[test]
    fn blame_menu_shows_the_annotation_and_one_row_per_action() {
        let mut state = file_tab_state(&["# app", "dirty"]);
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        draw_state(&mut terminal, &mut state);
        cache_focused_blame(&mut state, "seed the app");
        assert_eq!(state.dispatch(Action::BlameMenu), Effect::None);
        assert_eq!(open_dialog(&state), Some(DialogKind::BlameMenu));
        draw_state(&mut terminal, &mut state);
        let buf = terminal.backend().buffer();
        let text = buffer_text(&terminal);
        let (note, _) = state.painted_line_annotation().expect("annotation");
        let header = first_row_with(buf, "Blame ").expect("menu header");
        assert!(buf_line(buf, header).contains(&note), "{text}");
        for (offset, row) in BLAME_MENU_ROWS.iter().enumerate() {
            let line = buf_line(buf, header + 1 + offset as u16);
            assert!(
                line.contains(&format!(" {}  {}", row.key, row.label)),
                "{text}"
            );
        }
        assert!(
            buf_line(buf, header + 1 + BLAME_MENU_ROWS.len() as u16).contains("Esc cancel"),
            "{text}"
        );
        assert_eq!(state.dispatch(Action::BlameMenuCancel), Effect::None);
        draw_state(&mut terminal, &mut state);
        assert!(!buffer_text(&terminal).contains("Esc cancel"));
    }

    #[test]
    fn file_tab_wraps_and_keeps_the_cursor_in_view() {
        let lines: Vec<String> = (0..30).map(|i| format!("line {i}")).collect();
        let mut refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let long = "w".repeat(150);
        refs[25] = &long;
        let mut state = file_tab_state(&refs);
        assert!(
            state.diff_wrap,
            "a file tab opens wrapped (diff wrap default)"
        );
        state.dispatch(Action::MoveToEnd);
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let rows = state.layout.file_view_row_lines.clone();
        assert_eq!(rows.last(), Some(&29), "cursor line paints: {rows:?}");
        assert_eq!(
            rows.iter().filter(|&&line| line == 25).count(),
            2,
            "the long line wraps onto two rows: {rows:?}"
        );
        let buf = terminal.backend().buffer();
        let second = rows.iter().position(|&line| line == 25).unwrap() + 1;
        let continuation = buf_line(buf, state.layout.file_view_y + second as u16);
        assert!(
            continuation.starts_with("│   w"),
            "blank gutter: {continuation}"
        );

        state.dispatch(Action::ToggleDiffWrap);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let rows = state.layout.file_view_row_lines.clone();
        assert_eq!(rows.last(), Some(&29), "{rows:?}");
        let scroll = state.tabs.active_file().unwrap().scroll;
        assert_eq!(rows.first(), Some(&scroll));
        assert_eq!(
            rows.iter().filter(|&&line| line == 25).count(),
            1,
            "unwrapped, the long line takes one row: {rows:?}"
        );
    }

    #[test]
    fn file_tab_paints_binary_and_loading_notices() {
        let mut state = file_tab_state(&["x"]);
        let id = state.tabs.active_file().unwrap().id;
        let Effect::LoadFileTab { gen, .. } = state.dispatch(Action::Refresh) else {
            panic!("r reloads");
        };
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(buffer_text(&terminal).contains("loading…"));
        state.apply_file_tab(id, gen, FileRead::Binary);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(buffer_text(&terminal).contains(FILE_IS_BINARY));
    }

    #[test]
    fn compare_tab_opened_after_file_tab_paints_right() {
        let mut state = file_tab_state(&["# app"]);
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
        assert!(state.is_compare_tab());
        let backend = TestBackend::new(100, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let text = buffer_text(&terminal);
        assert!(text.contains("README.md"), "file tab label: {text}");
        assert!(text.contains("app ↔ main"), "{text}");
        assert!(text.contains("compare-only.md"), "{text}");
        assert!(text.contains("new compare"), "{text}");
        assert!(!text.contains("# app"), "file body is not painted: {text}");
        assert!(state.layout.file_view_row_lines.is_empty());
    }

    #[test]
    fn file_tab_wrap_scan_of_a_huge_line_is_bounded_by_the_view() {
        let huge = "m".repeat(1024 * 1024);
        let mut state = file_tab_state(&[huge.as_str(), "tail"]);
        assert!(
            state.diff_wrap,
            "a file tab opens wrapped (diff wrap default)"
        );
        let backend = TestBackend::new(80, 16);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let height = usize::from(state.layout.file_view_height);
        let rows = &state.layout.file_view_row_lines;
        assert_eq!(rows.len(), height, "{rows:?}");
        assert!(
            rows.iter().all(|&line| line == 0),
            "the long line fills the view"
        );
        let code_w = usize::from(state.layout.file_view_width) - file_gutter_width(2);
        let capped = wrap_col_starts_capped(&huge, code_w, height + 1);
        assert_eq!(
            capped.len(),
            height + 1,
            "the scan stops one row past the view"
        );
        assert_eq!(capped[height], height * code_w);
        assert!(buf_line(terminal.backend().buffer(), state.layout.file_view_y).contains("mmmm"));
    }

    /// On a file tab help paints MOVE / FILE / VIEW at the rows
    /// `help_status_lines(box, HelpTab::File)` reserves.
    #[test]
    fn file_help_paints_its_reserved_rows() {
        use super::super::chrome::{dialog_height, dialog_rect, dialog_width, DialogKind};
        use super::super::help::{
            help_body_line_count, help_legend_line_count, help_status_lines, HelpTab,
            HELP_FILE_GROUPS,
        };
        for cols in [68u16, 104, 144] {
            let mut state = file_tab_state(&["# app"]);
            state.help_open = true;
            let mut terminal = Terminal::new(TestBackend::new(cols, 200)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let text = buffer_text(&terminal);
            let lines: Vec<&str> = text.lines().collect();
            let header = lines
                .iter()
                .position(|l| l.contains("MOVE") && l.contains("FILE") && l.contains("VIEW"))
                .unwrap_or_else(|| panic!("{cols} cols, file help header:\n{text}"));
            let top = header - 1;
            let box_w = cols - 4;
            let panes = Rect::new(0, 1, cols, state.layout.pane_height);
            assert_eq!(dialog_width(panes, DialogKind::Help), box_w);
            let rect = dialog_rect(panes, box_w, dialog_height(&state, DialogKind::Help, box_w));
            assert_eq!(rect.y as usize, top, "{cols} cols:\n{text}");
            let bottom = (header..lines.len())
                .find(|&y| buffer_row(&terminal, y as u16, 2, 3) == "╰")
                .unwrap_or_else(|| panic!("{cols} cols, help bottom border:\n{text}"));
            let reserved = usize::from(help_status_lines(box_w, HelpTab::File));
            assert_eq!(bottom + 1 - top, reserved, "{cols} cols:\n{text}");
            let inner = help_inner_width(usize::from(box_w));
            let body = help_body_line_count(
                HELP_FILE_GROUPS,
                &help_column_widths(HELP_FILE_GROUPS, inner),
            ) + help_legend_line_count(inner);
            let footer_rows = help_idle_footer_lines(inner).len();
            assert_eq!(
                bottom - header - 1,
                body + footer_rows,
                "{cols} cols:\n{text}"
            );
            assert_eq!(state.layout.help_scroll_max, 0, "{cols} cols fits");
            for needle in ["editor at line", "reload", "(1=Workspace)"] {
                assert!(text.contains(needle), "{cols} cols {needle}:\n{text}");
            }
            assert!(!text.contains("COMPARE"), "{cols} cols:\n{text}");
        }
    }

    // ── PR badge ──────────────────────────────────────────────────────────

    const PR_REMOTE: &str = "git@github.com:octo/demo.git";

    fn branch_repo(name: &str, branch: &str) -> RepoSnapshot {
        let mut row = repo(name, false);
        row.branch = branch.into();
        row
    }

    fn linked_repo(name: &str, primary: &str, branch: &str) -> RepoSnapshot {
        let mut row = branch_repo(name, branch);
        row.checkout_kind = crate::snapshot::CheckoutKind::Linked;
        row.primary_repo = Some(primary.into());
        row
    }

    /// State with every checkout's PR lookup in flight.
    fn pr_state(repos: &[RepoSnapshot], ascii: bool) -> AppState {
        let snapshot = build_workspace_snapshot(repos, &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, ascii);
        assert_eq!(state.due_pr_lookups().len(), repos.len());
        state
    }

    fn set_pr(state: &mut AppState, repo: &str, branch: &str, lookup: PrLookup) {
        assert!(state.apply_pr_lookup(Path::new(repo), branch, Some(PR_REMOTE.into()), lookup));
    }

    fn found_pr(state: PrState) -> PrLookup {
        PrLookup::Found(PullRequest {
            number: 7,
            url: "https://github.com/octo/demo/pull/7".into(),
            state,
        })
    }

    fn pr_terminal() -> Terminal<TestBackend> {
        Terminal::new(TestBackend::new(120, 24)).unwrap()
    }

    /// Screen row of tree row `id`.
    fn tree_row_y(state: &AppState, id: &str) -> u16 {
        let index = state
            .painted_tree_rows()
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"));
        state.layout.tree_y + u16::try_from(index - state.layout.list_offset).unwrap()
    }

    fn focus_tree_row(state: &mut AppState, id: &str) {
        state.cursor = state
            .rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"));
    }

    /// The PR this effect opens, if any.
    fn pr_opened(effect: &Effect) -> Option<(PathBuf, String)> {
        match effect {
            Effect::OpenPullRequest { repo, branch } => Some((repo.clone(), branch.clone())),
            Effect::Batch(effects) => effects.iter().find_map(pr_opened),
            _ => None,
        }
    }

    #[test]
    fn tree_pr_badge_paints_after_the_branch_and_records_its_cell() {
        use crate::tui::icons::{icon_pr_approved, icon_pr_merged, icon_pr_open};
        let palette = ThemeId::TokyoNight.palette();
        let cases = [
            (
                PrState::Open,
                icon_pr_open as fn(bool) -> &'static str,
                palette.branch_feature,
            ),
            (PrState::Approved, icon_pr_approved, palette.added),
            (PrState::Merged, icon_pr_merged, palette.muted),
        ];
        for ascii in [true, false] {
            for (pr, icon, color) in cases {
                let what = format!("{pr:?} ascii={ascii}");
                let mut state = pr_state(
                    &[branch_repo("app", "feature"), branch_repo("lib", "main")],
                    ascii,
                );
                assert_eq!(state.theme.palette(), palette);
                set_pr(&mut state, "app", "feature", found_pr(pr));
                let mut terminal = pr_terminal();
                terminal.draw(|frame| draw(frame, &mut state)).unwrap();
                let buf = terminal.backend().buffer();
                let y = tree_row_y(&state, "repo:app");
                let branch_x = find_cell_col(buf, y, "feature")
                    .unwrap_or_else(|| panic!("{what}: branch on\n{}", buf_line(buf, y)));
                let x = branch_x + 8;
                assert_eq!(
                    state.layout.pr_badge_hits(),
                    vec![PrBadgeHit {
                        y,
                        x,
                        width: 1,
                        repo: PathBuf::from("app"),
                    }],
                    "{what}"
                );
                assert_eq!(buf[(x - 1, y)].symbol(), " ", "{what}");
                assert_eq!(buf[(x, y)].symbol(), icon(ascii), "{what}");
                assert_eq!(buf[(x, y)].fg, color, "{what}");
                for word in ["open", "approved", "merged"] {
                    assert!(!buf_line(buf, y).contains(word), "{what}: {word}");
                }
            }
        }
        assert_eq!(icon_pr_open(true), "P");
        assert_eq!(icon_pr_approved(true), "A");
        assert_eq!(icon_pr_merged(true), "m");
    }

    #[test]
    fn tree_pr_badge_needs_a_ready_pr() {
        for (lookup, what) in [
            (None, "in flight"),
            (Some(PrLookup::Failed), "failed"),
            (Some(PrLookup::NoPr), "no PR"),
        ] {
            let mut state = pr_state(&[branch_repo("app", "feature")], true);
            if let Some(lookup) = lookup {
                set_pr(&mut state, "app", "feature", lookup);
            }
            let mut terminal = pr_terminal();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            assert!(state.layout.pr_badge_hits().is_empty(), "{what}");
            let buf = terminal.backend().buffer();
            let y = tree_row_y(&state, "repo:app");
            let x = find_cell_col(buf, y, "feature").expect("branch") + 7;
            assert_eq!(buf[(x, y)].symbol(), " ", "{what}");
            assert_eq!(buf[(x + 1, y)].symbol(), " ", "{what}");
        }
    }

    #[test]
    fn tree_pr_badge_is_on_checkout_rows_only() {
        let mut state = pr_state(
            &[
                branch_repo("fam", "main"),
                linked_repo("fam/.worktrees/feat", "fam", "feat"),
                repo("dirty", true),
            ],
            true,
        );
        set_pr(&mut state, "fam", "main", found_pr(PrState::Open));
        set_pr(
            &mut state,
            "fam/.worktrees/feat",
            "feat",
            found_pr(PrState::Merged),
        );
        set_pr(&mut state, "dirty", "main", found_pr(PrState::Approved));
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let mut got: Vec<(u16, PathBuf)> = state
            .layout
            .pr_badge_hits()
            .iter()
            .map(|hit| (hit.y, hit.repo.clone()))
            .collect();
        got.sort();
        let mut want = vec![
            (tree_row_y(&state, "checkout:fam"), PathBuf::from("fam")),
            (
                tree_row_y(&state, "checkout:fam/.worktrees/feat"),
                PathBuf::from("fam/.worktrees/feat"),
            ),
            (tree_row_y(&state, "repo:dirty"), PathBuf::from("dirty")),
        ];
        want.sort();
        assert_eq!(
            got, want,
            "family container, file and workspace rows stay bare"
        );
        let buf = terminal.backend().buffer();
        let family = buf_line(buf, tree_row_y(&state, "repo:fam"));
        assert!(!family.contains(" P"), "{family}");
        let file_y = state
            .painted_tree_rows()
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .map(|index| state.layout.tree_y + index as u16)
            .expect("file row");
        assert!(!buf_line(buf, file_y).contains(" A"), "file row");
    }

    #[test]
    fn tree_pr_badge_hit_follows_hscroll_and_drops_when_cut() {
        let mut state = pr_state(&[branch_repo("app", "feature")], true);
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let x = state.layout.pr_badge_hits()[0].x;

        state.left_col_offset = 3;
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(state.layout.pr_badge_hits().len(), 1);
        let hit = state.layout.pr_badge_hits()[0].clone();
        assert_eq!(hit.x, x - 3);
        assert_eq!(terminal.backend().buffer()[(hit.x, hit.y)].symbol(), "P");

        // Scrolled past the badge: nothing painted, nothing recorded.
        state.left_col_offset = 40;
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(state.layout.pr_badge_hits().is_empty());
        let buf = terminal.backend().buffer();
        let row = buf_line(buf, tree_row_y(&state, "repo:app"));
        let tree: String = row.chars().take(state.layout.tree_width as usize).collect();
        assert!(!tree.contains('P'), "{row}");
    }

    #[test]
    fn tree_pr_badge_span_matches_paint_at_every_width() {
        let state = pr_state(&[branch_repo("app", "feature")], true);
        let row = state
            .rows
            .iter()
            .find(|row| row.id == "repo:app")
            .expect("repo row")
            .clone();
        let palette = ThemeId::TokyoNight.palette();
        let (mut shown, mut cut) = (0, 0);
        for width in 8..48 {
            for col_offset in [0, 2, 9] {
                let (line, icons) = paint_tree_row(
                    &row,
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
                    Some(PrState::Open),
                    palette,
                    col_offset,
                );
                let span = icons
                    .iter()
                    .find(|(kind, _, _)| *kind == IconKind::PrOpen)
                    .map(|&(_, x, w)| (x, w));
                // ASCII row: one column per char.
                let text: Vec<char> = line_text(&line).chars().collect();
                let what = format!("width {width} offset {col_offset}: {text:?}");
                match span {
                    Some((x, w)) => {
                        shown += 1;
                        assert_eq!(w, 1, "{what}");
                        assert!(x + w <= width, "{what}");
                        assert_eq!(text[x], 'P', "{what}");
                    }
                    None => {
                        cut += 1;
                        assert!(!text.contains(&'P'), "{what}");
                    }
                }
            }
        }
        assert!(shown > 0 && cut > 0, "shown {shown} cut {cut}");
    }

    #[test]
    fn ctrl_click_on_the_painted_tree_badge_opens_its_pr_only_there() {
        let mut state = pr_state(&[branch_repo("app", "feature"), repo("lib", true)], true);
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        focus_tree_row(&mut state, "repo:lib");
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let hit = state.layout.pr_badge_hits()[0].clone();
        let want = Some((PathBuf::from("app"), "feature".to_string()));

        let effect = state.dispatch(Action::CtrlClick {
            col: hit.x,
            row: hit.y,
        });
        assert_eq!(pr_opened(&effect), want);
        assert_eq!(state.rows[state.cursor].id, "repo:app");

        // One column left is the branch text: a plain click.
        let effect = state.dispatch(Action::CtrlClick {
            col: hit.x - 1,
            row: hit.y,
        });
        assert_eq!(pr_opened(&effect), None);
        // A plain click on the badge only selects.
        let effect = state.dispatch(Action::Click {
            col: hit.x,
            row: hit.y,
        });
        assert_eq!(pr_opened(&effect), None);
        assert_eq!(state.rows[state.cursor].id, "repo:app");
    }

    #[test]
    fn pr_badge_hits_clear_when_the_frame_paints_no_badge() {
        let mut state = pr_state(&[branch_repo("app", "feature")], true);
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let hit = state.layout.pr_badge_hits()[0].clone();

        state
            .tabs
            .open_or_focus("app".into(), "main".into(), "HEAD".into());
        assert!(state.is_compare_tab());
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(state.layout.pr_badge_hits().is_empty());
        let effect = state.dispatch(Action::CtrlClick {
            col: hit.x,
            row: hit.y,
        });
        assert_eq!(pr_opened(&effect), None);

        // A too-small frame paints no pane and keeps no hit either.
        state.tabs.active = 0;
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(state.layout.pr_badge_hits().len(), 1);
        let mut tiny = Terminal::new(TestBackend::new(20, 5)).unwrap();
        tiny.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(state.layout.pr_badge_hits().is_empty());
    }

    #[test]
    fn graph_worktree_badge_paints_and_ctrl_click_opens_its_pr() {
        use workspace_status_graph::Worktree;
        const HEAD: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut state = pr_state(
            &[
                branch_repo("app", "main"),
                linked_repo("app/.worktrees/feat", "app", "feat"),
            ],
            true,
        );
        set_pr(&mut state, "app", "main", found_pr(PrState::Approved));
        set_pr(
            &mut state,
            "app/.worktrees/feat",
            "feat",
            found_pr(PrState::Open),
        );
        focus_tree_row(&mut state, "checkout:app");
        state.graph = Some(GraphModel {
            commits: vec![Commit {
                id: HEAD.into(),
                subject: "seed graph".into(),
                refs: vec!["main".into()],
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
                ..Commit::default()
            }],
            head_id: Some(HEAD.into()),
            // The commit row lists `app`; only the worktree row is badged.
            worktrees: vec![
                Worktree {
                    path: "app".into(),
                    head_id: Some(HEAD.into()),
                    branch: Some("main".into()),
                    ignored: false,
                    is_current: true,
                },
                Worktree {
                    path: "app/.worktrees/feat".into(),
                    head_id: None,
                    branch: Some("feat".into()),
                    ignored: false,
                    is_current: false,
                },
            ],
            ..GraphModel::default()
        });
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let right_x = state.layout.right_x;
        let graph_hits: Vec<PrBadgeHit> = state
            .layout
            .pr_badge_hits()
            .iter()
            .filter(|hit| hit.x >= right_x)
            .cloned()
            .collect();
        assert_eq!(graph_hits.len(), 1, "{graph_hits:?}");
        let hit = graph_hits[0].clone();
        assert_eq!(
            (hit.width, hit.repo.as_path()),
            (1, Path::new("app/.worktrees/feat"))
        );
        let buf = terminal.backend().buffer();
        let row = buf_line(buf, hit.y);
        let label_x = find_cell_col(buf, hit.y, "app/.worktrees/feat feat P")
            .unwrap_or_else(|| panic!("worktree label then badge:\n{row}"));
        assert_eq!(hit.x, label_x + 25);
        assert_eq!(buf[(hit.x, hit.y)].symbol(), "P");
        assert_eq!(buf[(hit.x, hit.y)].fg, state.theme.palette().branch_feature);
        let commit_y = (0..24)
            .find(|y| buf_line(buf, *y).contains("seed graph"))
            .expect("commit row");
        let commit_row: String = buf_line(buf, commit_y)
            .chars()
            .skip(right_x as usize)
            .collect();
        assert!(!commit_row.contains(" A"), "{commit_row}");

        let effect = state.dispatch(Action::CtrlClick {
            col: hit.x,
            row: hit.y,
        });
        assert_eq!(
            pr_opened(&effect),
            Some((PathBuf::from("app/.worktrees/feat"), "feat".to_string()))
        );
        let effect = state.dispatch(Action::CtrlClick {
            col: hit.x - 1,
            row: hit.y,
        });
        assert_eq!(pr_opened(&effect), None);
    }

    /// `app` on `main` plus linked checkout `linked` on `feat` (both with an
    /// open PR), and a graph whose worktree row for `linked` paints
    /// `graph_branch`.
    fn pr_graph_state(linked: &str, graph_branch: &str, with_pr: bool) -> AppState {
        use workspace_status_graph::Worktree;
        const HEAD: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut state = pr_state(
            &[
                branch_repo("app", "main"),
                linked_repo(linked, "app", "feat"),
            ],
            true,
        );
        if with_pr {
            set_pr(&mut state, linked, "feat", found_pr(PrState::Open));
        }
        focus_tree_row(&mut state, "checkout:app");
        state.graph = Some(GraphModel {
            commits: vec![Commit {
                id: HEAD.into(),
                subject: "seed graph".into(),
                author_name: "Ada".into(),
                author_date_unix: 1_700_000_000,
                ..Commit::default()
            }],
            head_id: Some(HEAD.into()),
            worktrees: vec![Worktree {
                path: linked.into(),
                head_id: None,
                branch: Some(graph_branch.into()),
                ignored: false,
                is_current: false,
            }],
            ..GraphModel::default()
        });
        state
    }

    /// Text rows of `rect` in `buf`, trailing spaces trimmed.
    fn rect_lines(buf: &ratatui::buffer::Buffer, rect: Rect) -> Vec<String> {
        (rect.y..rect.bottom())
            .map(|y| {
                (rect.x..rect.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn tree_icon_spans_match_paint_at_every_width_and_offset() {
        let mut behind = branch_repo("app", "feature");
        behind.sync_status = SyncStatus::Behind;
        behind.sync_note = "behind by 12 commits".into();
        let state = pr_state(&[behind], true);
        let row = state
            .rows
            .iter()
            .find(|row| row.id == "repo:app")
            .expect("repo row")
            .clone();
        let palette = ThemeId::TokyoNight.palette();
        let (mut both, mut sync_only) = (0, 0);
        for width in 6..60 {
            for col_offset in [0, 3, 11, 40] {
                let (line, icons) = paint_tree_row(
                    &row,
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
                    Some(PrState::Merged),
                    palette,
                    col_offset,
                );
                let text: Vec<char> = line_text(&line).chars().collect();
                let what = format!("width {width} offset {col_offset}: {text:?}");
                for &(kind, x, w) in &icons {
                    assert!(x + w <= width, "{what}");
                    let cells: String = text[x..x + w].iter().collect();
                    match kind {
                        IconKind::Behind => assert!("v12".starts_with(&cells), "{what}"),
                        IconKind::PrMerged => assert_eq!(cells, "m", "{what}"),
                        IconKind::Branch => assert_eq!(cells, "&", "{what}"),
                        IconKind::Repo => assert_eq!(cells, "@", "{what}"),
                        other => panic!("{other:?}: {what}"),
                    }
                }
                let kinds: Vec<IconKind> = icons
                    .iter()
                    .map(|icon| icon.0)
                    .filter(|kind| matches!(kind, IconKind::PrMerged | IconKind::Behind))
                    .collect();
                if kinds == [IconKind::PrMerged, IconKind::Behind] {
                    both += 1;
                } else if kinds == [IconKind::Behind] {
                    sync_only += 1;
                }
                // The pane clips the line at `width`.
                let shown: String = text.iter().take(width).collect();
                if !kinds.contains(&IconKind::Behind) {
                    assert!(!shown.contains('v'), "{what}");
                }
            }
        }
        assert!(
            both > 0 && sync_only > 0,
            "both {both} sync only {sync_only}"
        );
    }

    /// Every branch-level icon kind on screen: a family with a merged
    /// primary and an open linked checkout, a repo whose status failed, an
    /// ignored dirty repo, and an idle repo under an open No updates group.
    fn branch_rows_state() -> AppState {
        let mut primary = branch_repo("app", "feature/landed");
        primary.merged_into_default = Some(true);
        primary.sync_status = SyncStatus::Behind;
        primary.sync_note = "behind by 12 commits".into();
        let mut linked = linked_repo("app/.worktrees/feat", "app", "feature/side");
        linked.merged_into_default = Some(false);
        let mut broken = repo("broken", false);
        broken.sync_note = crate::helpers::STATUS_FAILED_NOTE.into();
        let mut idle = repo("idle", false);
        idle.sync_status = SyncStatus::UpToDate;
        let snapshot = build_workspace_snapshot(
            &[primary, linked, broken, repo("notes", true), idle],
            &["notes".into()],
            true,
            &[],
        );
        let mut state = AppState::new(PathBuf::from("/tmp/ws"), snapshot, true);
        state.show_ignored = true;
        state.folds.clear();
        state.rebuild_rows();
        state
    }

    #[test]
    fn branch_row_icon_spans_match_paint_at_every_width_and_offset() {
        let state = branch_rows_state();
        let palette = ThemeId::TokyoNight.palette();
        let mut seen = HashSet::new();
        for row in &state.rows {
            let cores: Vec<(IconKind, String)> = row
                .segments
                .iter()
                .chain(&row.trailing_segs)
                .filter_map(|seg| seg.icon.map(|kind| (kind, seg.text.trim().to_string())))
                .collect();
            for width in 6..72 {
                for col_offset in [0, 3, 11, 40] {
                    let (line, icons) = paint_tree_row(
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
                        None,
                        palette,
                        col_offset,
                    );
                    // ASCII rows: one column per char.
                    let text: Vec<char> = line_text(&line).chars().collect();
                    let what = format!("{} width {width} offset {col_offset}: {text:?}", row.id);
                    for &(kind, x, w) in &icons {
                        assert!(w > 0 && x + w <= width, "{kind:?} {what}");
                        let cells: String = text[x..x + w].iter().collect();
                        assert!(
                            !cells.starts_with(' ') && !cells.ends_with(' '),
                            "{kind:?} span is the glyph only: {cells:?} {what}"
                        );
                        assert!(
                            cores
                                .iter()
                                .any(|(tag, core)| *tag == kind && core.contains(&cells)),
                            "{kind:?} {cells:?} {what}"
                        );
                        seen.insert(kind);
                    }
                }
            }
        }
        use IconKind as K;
        for kind in [
            K::Workspace,
            K::Repo,
            K::LinkedWorktree,
            K::Branch,
            K::MergedIntoDefault,
            K::OpenVsDefault,
            K::Behind,
            K::NoUpstream,
            K::Clean,
            K::StatusFailed,
            K::Ignored,
            K::ChangeCount,
            K::WorktreeCount,
        ] {
            assert!(seen.contains(&kind), "{kind:?} never painted");
        }
    }

    #[test]
    fn merge_mark_paints_its_own_colour_and_hit_after_the_branch() {
        let mut state = branch_rows_state();
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let palette = state.theme.palette();
        let buf = terminal.backend().buffer();
        for (id, branch, kind, mark, fg) in [
            (
                "checkout:app",
                "feature/landed",
                IconKind::MergedIntoDefault,
                "M",
                palette.added,
            ),
            (
                "checkout:app/.worktrees/feat",
                "feature/side",
                IconKind::OpenVsDefault,
                "o",
                palette.muted,
            ),
        ] {
            let y = tree_row_y(&state, id);
            let x = find_cell_col(buf, y, branch).expect("branch") + branch.len() as u16 + 1;
            assert_eq!(buf[(x - 1, y)].symbol(), " ", "{id}");
            assert_eq!(buf[(x, y)].symbol(), mark, "{id}");
            assert_eq!(buf[(x, y)].fg, fg, "{id}: the mark's own role");
            assert_ne!(buf[(x - 2, y)].fg, fg, "{id}: the branch keeps its colour");
            let hit = state
                .layout
                .icon_hits
                .iter()
                .find(|hit| hit.kind == kind)
                .cloned()
                .expect("merge hit");
            assert_eq!(
                (hit.x, hit.y, hit.width),
                (x, y, 1),
                "{id}: glyph cell only"
            );
        }
    }

    /// The icon hits of `target`, as (kind, cells under the hit, width).
    fn hit_cells(
        state: &AppState,
        terminal: &Terminal<TestBackend>,
        target: &IconTarget,
    ) -> Vec<(IconKind, String, u16)> {
        let buf = terminal.backend().buffer();
        state
            .layout
            .icon_hits
            .iter()
            .filter(|hit| &hit.target == target)
            .map(|hit| {
                let cells: String = (hit.x..hit.x + hit.width)
                    .map(|x| buf[(x, hit.y)].symbol().to_string())
                    .collect();
                (hit.kind, cells, hit.width)
            })
            .collect()
    }

    /// A badge hit is its letter, not the pad column; a devicon hit is the
    /// glyph, not the space after it. Nerd and ASCII glyph modes.
    #[test]
    fn file_row_badge_and_devicon_hits_cover_the_glyph_only() {
        for ascii in [true, false] {
            let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
            let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, ascii);
            focus_tree_row(&mut state, "file:app:README.md");
            let mut terminal = pr_terminal();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let target = IconTarget::TreeRow("file:app:README.md".into());
            let hits = hit_cells(&state, &terminal, &target);
            let devicon = crate::tui::icons::file_icon(ascii, "README.md")
                .glyph
                .to_string();
            assert_eq!(
                hits,
                vec![
                    (IconKind::FileType, devicon, 1),
                    (IconKind::StatusModified, "M".to_string(), 1),
                ],
                "ascii={ascii}"
            );
            let badge = state
                .layout
                .icon_hits
                .iter()
                .find(|hit| hit.kind == IconKind::StatusModified)
                .expect("badge hit");
            let buf = terminal.backend().buffer();
            assert_eq!(buf[(badge.x + 1, badge.y)].symbol(), " ", "pad stays out");
        }
    }

    /// Commit-file list rows record icon hits on their own row ids: the
    /// folder, devicon, and status letter.
    #[test]
    fn commit_file_rows_record_icon_hits() {
        let snapshot = build_workspace_snapshot(&[repo("app", true)], &[], false, &[]);
        let mut state = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        state.commit_tree_mode = true;
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
                    path: "src/main.rs".into(),
                    old_path: None,
                    stat: None,
                },
            ],
        );
        let mut terminal = Terminal::new(TestBackend::new(100, 16)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let row = |id: &str| IconTarget::CommitFileRow(id.into());
        assert_eq!(
            hit_cells(&state, &terminal, &row("dir:src")),
            vec![(IconKind::Folder, "/".to_string(), 1)]
        );
        assert_eq!(
            hit_cells(&state, &terminal, &row("file:src/lib.rs")),
            vec![
                (IconKind::FileType, "·".to_string(), 1),
                (IconKind::StatusAdded, "A".to_string(), 1),
            ]
        );
        assert_eq!(
            hit_cells(&state, &terminal, &row("file:src/main.rs"))[1],
            (IconKind::StatusModified, "M".to_string(), 1)
        );
    }

    #[test]
    fn pinned_popover_paints_its_sections_chips_and_footer() {
        let mut state = pr_state(&[branch_repo("app", "feature")], true);
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        focus_tree_row(&mut state, "repo:app");
        // Tall enough for every section of the row: no scroll.
        let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let first = state
            .layout
            .icon_hits
            .iter()
            .find(|hit| hit.target == IconTarget::TreeRow("repo:app".into()))
            .cloned()
            .expect("repo row icon");
        assert_eq!(first.kind, IconKind::Branch, "the row's first icon");
        state.dispatch(Action::PopoverOpenFocused);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let painted = state.layout.popover.clone().expect("painted popover");
        let rect = painted.rect;
        assert_eq!(
            (rect.x, rect.y),
            (first.x, first.y + 1),
            "hangs below the row's first icon"
        );
        let text = rect_lines(&state.painted_frame, rect).join("\n");
        for want in [
            "& branch",
            "Branch picker",
            "@ repo",
            "P PR open",
            "Pull request is open",
            // Labels pad to the widest label of their own section: the
            // PR section to `number`, the repo section to `changes`.
            "number #7",
            "url    https://github.com/octo/demo/pull/7",
            "path    app",
            "changes 0 staged · 0 unstaged · 0 untracked",
            "Open PR",
            " gx ",
            "? no upstream",
            "Push",
            " P ",
            "Fetch remotes",
            "y copy line · Enter run · Esc close",
        ] {
            assert!(text.contains(want), "{want:?} in\n{text}");
        }
        let palette = state.theme.palette();
        // From the branch picker past the branch actions and the PR fields.
        state.dispatch(Action::PopoverMove(6));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let buf = &state.painted_frame;
        let open_y = (rect.y..rect.bottom())
            .find(|y| buf_line(buf, *y).contains("Open PR"))
            .expect("Open PR row");
        assert_eq!(buf[(painted.inner.x, open_y)].symbol(), "❯", "focused line");
        assert_eq!(buf[(painted.inner.x + 2, open_y)].bg, palette.cursor_bg);
        // The key chip ends at the right edge of the text area.
        let chip_end = painted.inner.right() - 1;
        assert_eq!(buf[(chip_end, open_y)].symbol(), " ");
        assert_eq!(buf[(chip_end - 1, open_y)].symbol(), "x");
        assert_eq!(buf[(chip_end - 2, open_y)].symbol(), "g");
        assert_eq!(buf[(chip_end - 2, open_y)].bg, palette.cursor);
        let sections = state.open_popover_sections();
        assert_eq!(
            painted.lines.len(),
            flat_lines(&sections)
                .iter()
                .filter(|line| line.focusable())
                .count(),
            "every field and action row is clickable: {:?}",
            painted.lines
        );
        assert!(rect.width <= POPOVER_MAX_WIDTH);
    }

    /// Popover text of `state` after one frame.
    fn popover_text(state: &mut AppState, terminal: &mut Terminal<TestBackend>) -> String {
        terminal.draw(|frame| draw(frame, state)).unwrap();
        let rect = state.layout.popover.as_ref().expect("popover").rect;
        rect_lines(&state.painted_frame, rect).join("\n")
    }

    #[test]
    fn pr_popover_shows_loading_then_the_detail() {
        use crate::tui::pull_request::{
            ChecksSummary, PrDetailLookup, PrReview, PullRequestDetail,
        };
        for ascii in [true, false] {
            let mut state = pr_state(&[branch_repo("app", "feature")], ascii);
            set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
            focus_tree_row(&mut state, "repo:app");
            let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            let effect = state.dispatch(Action::PopoverOpenFocused);
            assert!(matches!(
                effect,
                Effect::LookupPullRequestDetail { number: 7, .. }
            ));
            let text = popover_text(&mut state, &mut terminal);
            for want in [
                "number #7",
                "loading…",
                "url    https://github.com/octo/demo/pull/7",
                "Open PR",
            ] {
                assert!(text.contains(want), "{want:?} in\n{text}");
            }
            let buf = &state.painted_frame;
            let rect = state.layout.popover.as_ref().unwrap().rect;
            let y = (rect.y..rect.bottom())
                .find(|y| buf_line(buf, *y).contains("loading…"))
                .expect("loading row");
            let x = (rect.x..rect.right())
                .find(|x| buf[(*x, y)].symbol() == "l")
                .expect("loading text");
            assert_eq!(buf[(x, y)].fg, state.theme.palette().muted, "muted");

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs() as i64;
            assert!(state.apply_pr_detail(
                Path::new("app"),
                "feature",
                PR_REMOTE,
                7,
                1,
                PrDetailLookup::Found(PullRequestDetail {
                    number: 7,
                    title: "Add login".into(),
                    state: "OPEN".into(),
                    draft: true,
                    author: Some("octocat".into()),
                    head: Some("feature".into()),
                    base: Some("main".into()),
                    review: Some(PrReview::ReviewRequired),
                    checks: ChecksSummary {
                        pass: 3,
                        fail: 1,
                        pending: 2,
                    },
                    updated_at: Some(now - 2 * 3600),
                }),
            ));
            let text = popover_text(&mut state, &mut terminal);
            let (arrow, checks) = if ascii {
                ("->", "3 pass · 1 fail · 2 pending")
            } else {
                ("→", "✓ 3 · ✗ 1 · … 2")
            };
            for want in [
                "number  #7 Add login".to_string(),
                "state   OPEN · DRAFT · REVIEW_REQUIRED".into(),
                "author  octocat".into(),
                format!("branch  feature {arrow} main"),
                format!("checks  {checks}"),
                "(2h)".into(),
                "url     https://github.com/octo/demo/pull/7".into(),
                "Open PR".into(),
            ] {
                assert!(text.contains(&want), "{want:?} in\n{text}");
            }
            assert!(!text.contains("loading…"), "{text}");
        }
    }

    #[test]
    fn pr_popover_says_when_the_detail_could_not_load() {
        use crate::tui::pull_request::PrDetailLookup;
        let mut state = pr_state(&[branch_repo("app", "feature")], true);
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        focus_tree_row(&mut state, "repo:app");
        let mut terminal = Terminal::new(TestBackend::new(120, 60)).unwrap();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let _ = state.dispatch(Action::PopoverOpenFocused);
        assert!(state.apply_pr_detail(
            Path::new("app"),
            "feature",
            PR_REMOTE,
            7,
            1,
            PrDetailLookup::Failed
        ));
        let text = popover_text(&mut state, &mut terminal);
        for want in [
            "number #7",
            "could not load details",
            "url    https://github.com/octo/demo/pull/7",
            "Open PR",
        ] {
            assert!(text.contains(want), "{want:?} in\n{text}");
        }
        assert!(!text.contains("loading…"), "{text}");
    }

    #[test]
    fn peek_paints_no_footer_and_no_focus() {
        let mut state = pr_state(&[branch_repo("app", "feature")], true);
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let badge = state.layout.pr_badge_hits()[0].clone();
        let at = std::time::Instant::now();
        state.set_pointer_at(Some((badge.x, badge.y)), at);
        assert!(state.expire_peek(at + std::time::Duration::from_secs(1)));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let rect = state.layout.popover.as_ref().expect("peek").rect;
        let text = rect_lines(&state.painted_frame, rect).join("\n");
        assert!(text.contains("Open PR"), "{text}");
        assert!(!text.contains("Esc close"), "{text}");
        assert!(!text.contains('❯'), "{text}");
    }

    #[test]
    fn popover_flips_above_a_bottom_row_and_stays_in_the_frame() {
        let repos: Vec<RepoSnapshot> = (0..30)
            .map(|i| branch_repo(&format!("r{i:02}"), "feature"))
            .collect();
        let mut state = pr_state(&repos, true);
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let last = state
            .layout
            .icon_hits
            .iter()
            .filter(|hit| hit.kind == IconKind::NoUpstream)
            .max_by_key(|hit| hit.y)
            .cloned()
            .expect("sync marks");
        state.dispatch(Action::Click {
            col: last.x,
            row: last.y,
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let hit = state
            .layout
            .icon_hits
            .iter()
            .find(|hit| hit.target == last.target && hit.kind == last.kind)
            .cloned()
            .expect("icon still painted");
        let rect = state.layout.popover.as_ref().expect("pinned").rect;
        assert!(rect.bottom() <= hit.y, "above the icon: {rect:?} {hit:?}");
        assert!(rect.right() <= 120 && rect.x <= hit.x, "{rect:?}");
    }

    /// A popover the frame cuts names the rows below the shown ones on
    /// its bottom border; with the focus on the last line nothing is
    /// below. Field values paint in their role colour, labels pad per
    /// section.
    #[test]
    fn a_cut_popover_counts_the_rows_below_and_values_take_their_role() {
        let mut state = pr_state(&[branch_repo("app", "feature")], true);
        set_pr(&mut state, "app", "feature", found_pr(PrState::Open));
        focus_tree_row(&mut state, "repo:app");
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let _ = state.dispatch(Action::PopoverOpenFocused);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let rect = state.layout.popover.as_ref().expect("pinned").rect;
        let buf = &state.painted_frame;
        let bottom = buf_line(buf, rect.bottom() - 1);
        let more = bottom
            .split(" more ")
            .next()
            .and_then(|head| head.rsplit('v').next())
            .and_then(|count| count.parse::<usize>().ok())
            .unwrap_or_else(|| panic!("more cue on the bottom border: {bottom:?}"));
        assert!(more > 0, "{bottom:?}");
        assert!(bottom.contains(&format!("v{more} more ╯")), "{bottom:?}");

        // Branch name in the feature-branch colour, label padded to `local`.
        let palette = state.theme.palette();
        let y = (rect.y..rect.bottom())
            .find(|y| buf_line(buf, *y).contains("name  feature"))
            .unwrap_or_else(|| panic!("{}", rect_lines(buf, rect).join("\n")));
        let x = find_cell_col(buf, y, "name  feature").unwrap() + 6;
        assert_eq!(buf[(x, y)].symbol(), "f");
        assert_eq!(buf[(x, y)].fg, palette.branch_feature);
        assert_eq!(buf[(x - 6, y)].fg, palette.muted, "label");

        state.dispatch(Action::PopoverMove(999));
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let rect = state.layout.popover.as_ref().expect("pinned").rect;
        let bottom = buf_line(&state.painted_frame, rect.bottom() - 1);
        assert!(!bottom.contains(" more "), "last line in view: {bottom:?}");
    }

    /// A popover hung from an icon near the right edge moves left so its
    /// right border sits on the frame's last column, and the key chip
    /// still ends at the text area's right edge.
    #[test]
    fn popover_at_the_right_edge_moves_left_to_the_last_column() {
        let mut state = pr_graph_state("app/.worktrees/wt", "feat", true);
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let badge = graph_badge_hits(&state)
            .into_iter()
            .next()
            .expect("graph badge");
        state.dispatch(Action::Click {
            col: badge.x,
            row: badge.y,
        });
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        let painted = state.layout.popover.clone().expect("pinned");
        let rect = painted.rect;
        let frame_right = 120;
        assert!(
            badge.x + rect.width > frame_right,
            "the test needs a box that would overflow: {rect:?} {badge:?}"
        );
        assert_eq!(rect.right(), frame_right, "right border on the last column");
        assert_eq!(rect.x, frame_right - rect.width);
        let buf = &state.painted_frame;
        assert_eq!(buf[(rect.x, rect.y)].symbol(), "╭");
        assert_eq!(buf[(rect.right() - 1, rect.y)].symbol(), "╮");
        assert_eq!(buf[(rect.right() - 1, rect.bottom() - 1)].symbol(), "╯");
        for y in rect.y + 1..rect.bottom() - 1 {
            assert_eq!(buf[(rect.right() - 1, y)].symbol(), "│", "row {y}");
        }
        let open_y = (rect.y..rect.bottom())
            .find(|y| buf_line(buf, *y).contains("Open PR"))
            .expect("Open PR row");
        let chip_end = painted.inner.right() - 1;
        assert_eq!(buf[(chip_end - 1, open_y)].symbol(), "x");
        assert_eq!(buf[(chip_end - 2, open_y)].symbol(), "g");
    }

    fn graph_badge_hits(state: &AppState) -> Vec<PrBadgeHit> {
        state
            .layout
            .pr_badge_hits()
            .iter()
            .filter(|hit| hit.x >= state.layout.right_x)
            .cloned()
            .collect()
    }

    #[test]
    fn stale_graph_worktree_branch_gets_no_badge_and_opens_only_its_own_branch() {
        // The checkout moved to `feat`; the loaded graph still paints `old`.
        let mut state = pr_graph_state("app/.worktrees/wt", "old", true);
        let mut terminal = pr_terminal();
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert!(graph_badge_hits(&state).is_empty());
        assert_eq!(state.graph_pr_badges(), Vec::new());
        let buf = terminal.backend().buffer();
        let y = (0..24)
            .find(|y| find_cell_col(buf, *y, "app/.worktrees/wt old").is_some())
            .expect("stale worktree row");
        let x = find_cell_col(buf, y, "app/.worktrees/wt old").unwrap() + 22;
        assert_eq!(buf[(x, y)].symbol(), " ", "no badge after the stale label");
        // The tree row shows the snapshot branch, so it keeps its badge.
        assert_eq!(state.layout.pr_badge_hits().len(), 1);

        // Ctrl+click where a badge would be: a plain click, nothing opens.
        let effect = state.dispatch(Action::CtrlClick { col: x, row: y });
        assert_eq!(pr_opened(&effect), None);
        // `gx` on the row opens the branch the row paints, never `feat`'s PR.
        let rows = state.graph.as_ref().unwrap().visible_rows();
        state.focus = FocusPane::Right;
        state.graph_cursor = rows
            .iter()
            .position(|row| matches!(row, GraphRow::Worktree(_)))
            .expect("worktree row");
        assert_eq!(
            pr_opened(&state.dispatch(Action::OpenPullRequest)),
            Some((PathBuf::from("app/.worktrees/wt"), "old".to_string()))
        );

        // Once the graph paints the snapshot branch, the badge is back.
        let mut state = pr_graph_state("app/.worktrees/wt", "feat", true);
        terminal.draw(|frame| draw(frame, &mut state)).unwrap();
        assert_eq!(graph_badge_hits(&state).len(), 1);
    }

    #[test]
    fn graph_pan_reaches_the_badge_on_a_clipped_worktree_label() {
        let linked = "app/.worktrees/a-linked-worktree-path-that-runs-well-past-the-graph-pane";
        let pan_max = |with_pr: bool| {
            let mut state = pr_graph_state(linked, "feat", with_pr);
            let mut terminal = pr_terminal();
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            assert!(graph_badge_hits(&state).is_empty(), "clipped at offset 0");
            state.mouse_pan(state.layout.right_x + 1, 10_000);
            let offset = state.right_col_offset;
            terminal.draw(|frame| draw(frame, &mut state)).unwrap();
            (offset, state, terminal)
        };
        let (plain, _, _) = pan_max(false);
        let (badged, state, terminal) = pan_max(true);
        assert!(plain > 0);
        assert_eq!(badged, plain + 2, "pan reaches the space and glyph");
        let hits = graph_badge_hits(&state);
        assert_eq!(hits.len(), 1, "badge in view at the pan max");
        assert_eq!(
            terminal.backend().buffer()[(hits[0].x, hits[0].y)].symbol(),
            "P"
        );
    }
}

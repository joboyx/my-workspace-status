//! Ratatui paint for the tree, graph / diff pane, status, and help overlay.

use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Borders, Clear, Padding, Paragraph, Scrollbar, ScrollbarOrientation,
    ScrollbarState, StatefulWidget, Wrap,
};
use ratatui::Frame;
use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use unicode_width::UnicodeWidthStr;
use workspace_status_graph::{
    commit_msg_lines_fit, graph_chrome_budget_for_meta, graph_col_max, graph_hscroll_visible,
    graph_vscroll_visible, paint_selection_footer, painted_line_count, short_id, GraphIconKind,
    GraphLabelPalette, GraphRow, GraphWidget, ASCII, COMMIT_MSG_LINES_MAX, FOOTER_RULE_ROWS,
    UNICODE,
};

use super::action::SearchFilesOption;
use super::activity::{row_activity_key, sparkline, SPARK_MIN_TREE_COLS};
use super::anim::{glide_step, spinner_glyph, GlidePane};
use super::chrome::{
    breadcrumb_line, breadcrumb_rows, ctrl_c_prompt_line, ctrl_c_prompt_rows, dialog_height,
    dialog_rect, dialog_width, export_shows_status, help_tab, open_dialog, status_line, DialogKind,
    LIST_OVERLAY_MAX_ROWS,
};
use super::command_palette::{
    go_to_line_command, go_to_line_title, CommandPaletteState, PaletteCommand, PalettePaintRow,
};
use super::comments::{
    comment_overlay_footer_save, graph_row_comments_resolved, graph_row_has_comment, CommentPrompt,
    COMMENT_OVERLAY_FOOTER_EDIT,
};
use super::commit_files::FolderSummary;
use super::diff::{
    cell_code_width, cell_sign, diff_failed_text, diff_pane_header, diff_pane_header_rows,
    diff_pane_mode_label, diff_row_content_width, diff_row_gutter_cells, diff_wrap_row_heights,
    gutter_width, section_header, wrap_viewport_start, DiffCell, DiffCellKind, DiffRow,
    DiffSection, DIFF_RULE,
};
use super::doctor::{summary as doctor_summary, Check, CheckGroup, CheckStatus};
use super::drill::DrillView;
use super::explorer::{ExplorerRow, ExplorerStatus};
use super::gates::ListFocusTarget;
use super::help::{
    attach_help_version, help_chip_gap_spaces, help_column_content_width, help_column_widths,
    help_entry_matches, help_entry_visual_lines, help_groups, help_idle_footer,
    help_idle_footer_lines, help_inner_width, help_key_width, help_legend_glyph_width,
    help_legend_layout, help_legend_matches, help_legend_name_width, help_legend_visual_lines,
    help_version_label, wrap_help_description, wrap_help_footer, HELP_LEGEND_HEAD_ROWS,
    HELP_LEGEND_TITLE, HELP_SEARCH_ESC_HINT,
};
use super::hyperlink::LinkHit;
use super::icons::{
    comment_mark_cols, glyph, icon_branch, icon_comment, icon_comment_resolved, icon_diff,
    icon_merged_into_default, icon_move, icon_open_vs_default, truncate_visible, IconKind,
    IconSpec, CURSOR_BAR, CURSOR_BAR_INACTIVE, FOLD_COLLAPSED, FOLD_COLLAPSED_ASCII, FOLD_EXPANDED,
    FOLD_EXPANDED_ASCII,
};
use super::line_blame::{fit_annotation, relative_age, short_sha, BlameSide, BLAME_MENU_ROWS};
use super::ops::RevertScope;
use super::outage::outage_header_text;
use super::popover::{
    commit_meta_icon, flat_lines, focused_line, graph_icon, graph_row_id, graph_sync_kind,
    popover_rect, tree_icon_target, IconTarget, PopoverLine, POPOVER_FOOTER, POPOVER_MAX_WIDTH,
};
use super::pr_status::PrState;
use super::quick_open::{
    files_row_shows_status, FileIndexState, QuickOpenMode, QuickOpenScope, QuickOpenState,
};
use super::search::{
    collect_graph_match_indices, collect_match_ids, explorer_row_match_indices, list_prefix_cols,
    slice_cols, wrap_col_starts, wrap_col_starts_capped, wrap_cols, SearchPane,
};
use super::search_files::{
    option_is_on, option_tooltip, search_row_shows_status, SearchFilesState, SearchRow, SearchZone,
    SEARCH_PREVIEW_MIN_COLS,
};
use super::split::{
    diff_paint_width, diff_split_rule_x, list_hbar_rows, pane_widths, side_by_side_column_widths,
    DiffMode, HScrollbar, HScrollbarLayout, ResizeEdge, VScrollbarLayout, DIFF_CURSOR_BAR_COLS,
    MIN_PANE_COLS, MIN_TERM_COLS, MIN_TERM_ROWS,
};
use super::state::{
    revert_scope, unix_now, AppState, CompareRevertTarget, FocusPane, IconHit, LayoutHit,
    PendingConfirm, PopoverHit,
};
use super::syntax::{
    cached_highlight_diff_rows, highlight_file_window, readable_fg, slice_styled_cols,
    CachedDiffSyntax, CodeSpan, DiffBackgrounds, DiffSyntaxKey, StyledColCursor,
};
use super::tabs::{
    compare_picker_empty, file_gutter_width, file_too_large, file_view_gutter_width,
    file_view_scroll, short_rev, ComparePickerState, ExplorerPreview, FileTab,
    EXPLORER_FOLDER_HINT, FILE_IS_BINARY,
};
use super::theme::{hex_color, mix_toward, BackgroundMode, Palette, Pill, ThemeId};
use super::toast::{Toast, ToastMark, ToastTick};
use super::tree::{
    fit_upstream_rewrite_text, painted_row_segments, pr_badge_kind, pr_badge_role, visible_window,
    workspace_trailing_fit, NodeKind, NodeSegments, PrBadge, PrMark, PrTotals, SegRole, TextSeg,
    VisibleRow, PR_REF_MIN_TREE_COLS,
};
use super::undercurl::{cut_link_cells, drop_undercurls_under, underlined_runs, UndercurlHit};
use super::watch::FlashKind;
use crate::file_index::{FileIndex, FileRead};
use crate::file_search::{SearchHit, SearchOptions};
use crate::git::LineBlame;
use crate::helpers::{is_detached_head_branch, visible_width};

mod prs_tab;

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
/// border colour (boxed) or the title row colours ([`flat_title_line`]),
/// never a title glyph or space-pad. Boxed title text uses
/// `palette.heading` via `title_style` so it does not inherit
/// `border_style`.
fn pane_title(name: &str) -> String {
    name.to_string()
}

fn muted_copy(text: &'static str, palette: Palette) -> Line<'static> {
    Line::from(Span::styled(text, Style::default().fg(palette.muted)))
}

/// `text` behind the current spinner frame (`⠋ loading graph…`). Marks
/// the frame animating so the loop redraws on the spinner tick.
fn loading_line(state: &mut AppState, text: &str) -> String {
    spinner_text(state.anim_epoch, state.ascii, &mut state.layout, text)
}

/// [`loading_line`] for a paint site that holds a borrow of another
/// `state` field: takes the spinner clock, glyph set, and layout apart.
fn spinner_text(epoch: Instant, ascii: bool, layout: &mut LayoutHit, text: &str) -> String {
    layout.animating = true;
    format!("{} {text}", spinner_glyph(epoch, Instant::now(), ascii))
}

/// Foreground of spinner-prefixed loading copy: muted, or the terminal
/// foreground under `NO_COLOR`.
fn loading_fg(no_color: bool, palette: Palette) -> Color {
    if no_color {
        Color::Reset
    } else {
        palette.muted
    }
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
    draw_at(frame, state, Instant::now());
}

/// [`draw`] with the frame clock pinned to `now`: a scroll glide paints
/// its step for that instant.
pub(crate) fn draw_at(frame: &mut Frame<'_>, state: &mut AppState, now: Instant) {
    state.frame_now = now;
    state.prune_expired_flashes();
    // One-shot acks leave the status slot for a toast before anything paints.
    state.route_ack_to_toast(Instant::now());
    // Every frame starts with no icon, link, or search option chip; only a
    // pane or dialog that paints one records it again, so a hidden badge
    // never opens a PR or a popover and a closed dialog's chip never
    // toggles an option.
    state.layout.icon_hits.clear();
    state.layout.cursor_line_marks.clear();
    state.layout.search_option_hits.clear();
    state.layout.links.clear();
    state.layout.undercurls.clear();
    // A painter that shows a time-based frame sets these again.
    state.layout.animating = false;
    state.layout.gliding = false;
    state.layout.wake_ms = None;
    state.layout.popover = None;
    let area = frame.area();
    // The whole frame, also when it is too small to paint (the Doctor
    // reports this size).
    state.layout.term_cols = area.width;
    state.layout.term_rows = area.height;
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
    state.layout.graph_painted_top = None;
    state.layout.graph_hscrollbar_y = None;
    state.layout.graph_hscrollbar_x = 0;
    state.layout.graph_hscrollbar_width = 0;
    state.layout.graph_col_max = 0;
    state.layout.graph_footer_y = None;
    state.layout.graph_footer_scroll_max = 0;
    state.layout.commit_detail_footer_y = None;
    state.layout.commit_detail_footer_scroll_max = 0;
    state.layout.commit_msg_rule_y = None;
    state.layout.commit_msg_rule_x = 0;
    state.layout.commit_msg_rule_width = 0;
    state.layout.commit_msg_lines_fit = COMMIT_MSG_LINES_MAX;
    state.layout.diff_vbar = VScrollbarLayout::default();
    state.layout.diff_hscrollbar_y = None;
    state.layout.diff_hscrollbar_x = 0;
    state.layout.diff_hscrollbar_width = 0;
    state.layout.diff_col_max = 0;
    state.layout.tree_vbar = VScrollbarLayout::default();
    state.layout.files_vbar = VScrollbarLayout::default();
    state.layout.explorer_tree_vbar = VScrollbarLayout::default();
    state.layout.tree_hbar = HScrollbarLayout::default();
    state.layout.files_hbar = HScrollbarLayout::default();
    state.layout.explorer_tree_hbar = HScrollbarLayout::default();
    state.layout.file_view_vbar = VScrollbarLayout::default();
    state.layout.file_view_hbar = HScrollbarLayout::default();
    state.layout.summary_vbar = VScrollbarLayout::default();
    state.layout.summary_hbar = HScrollbarLayout::default();
    state.layout.prs_list_vbar = VScrollbarLayout::default();
    state.layout.prs_detail_vbar = VScrollbarLayout::default();
    state.layout.summary_list_height = 0;
    state.layout.file_view_row_lines.clear();
    state.layout.explorer_tree = Rect::default();
    state.layout.explorer_preview = Rect::default();
    state.layout.explorer_tree_rows = Rect::default();
    state.layout.explorer_preview_rows = Rect::default();
    state.layout.prs_list = Rect::default();
    state.layout.prs_list_rows = Rect::default();
    state.layout.prs_detail = Rect::default();
    state.layout.prs_detail_rows = Rect::default();
    state.layout.pr_total_hit = None;
    state.layout.file_view_rows = Rect::default();
    state.layout.pane_height = chunks[1].height;
    let edge_area = if state.is_explorer_tab() {
        draw_explorer_tab(frame, chunks[1], state)
    } else if state.is_prs_tab() {
        prs_tab::draw_prs_tab(frame, chunks[1], state);
        None
    } else {
        Some(draw_panes(frame, chunks[1], state))
    };

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

    // The painted ref of each PR badge links to its full PR URL. A box
    // painted over the panes below drops the links it covers.
    let badge_links = pr_badge_links(state, frame.buffer_mut());
    state.layout.links.extend(badge_links);

    // Dialogs paint last: a fixed-height box centered over the panes.
    state.layout.help_scroll_max = 0;
    if let Some(kind) = open_dialog(state) {
        let width = dialog_width(chunks[1], kind);
        let rect = dialog_rect(chunks[1], width, dialog_height(state, kind, width));
        // Everything outside the box recedes. The copy frame below keeps
        // the dim: only colours change, so drag-copy reads the same text.
        dim_backdrop(frame.buffer_mut(), area, rect, palette, state.no_color);
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
            DialogKind::Doctor => {
                let top = draw_doctor(frame, rect, state);
                if let Some(doctor) = state.doctor.as_mut() {
                    doctor.scroll = top;
                }
            }
        }
        drop_hits_under(&mut state.layout, rect);
        // The curls left are outside the box: they recede with the cells.
        recede_undercurls(&mut state.layout.undercurls, palette);
    }

    // The icon popover hangs over the panes and any dialog.
    draw_popover(frame, chunks[1], state);
    // After the popover records its rect: the edge the pointer shape reads
    // after this draw is the edge this frame lights.
    if let Some(edge_area) = edge_area {
        paint_resize_edge_hover(frame, state, edge_area);
    }

    // Keep the frame as painted so a mouse release copies what is on screen,
    // then reverse the selected cells on top of it. The copy is taken before
    // the toasts: a drag over a toast copies the pane text under it.
    state.painted_frame = frame.buffer_mut().clone();

    // Toasts paint last, over any dialog, popover, or backdrop, so they
    // stay crisp. Keep this after every overlay.
    draw_toasts(frame, chunks[1], state);

    // Links are final now. A cell that is both linked and curled keeps its
    // link (and the plain underline); the curl writer skips it.
    state.layout.undercurls = cut_link_cells(&state.layout.undercurls, &state.layout.links);

    if let Some(selection) = state.text_selection.as_ref() {
        selection.highlight(frame.buffer_mut());
    }
}

/// Widest a toast row may be (columns).
const TOAST_MAX_COLS: u16 = 48;

/// Widest an outage toast row may be (columns): it names a tool, a host,
/// and a reason, and keeps its dismiss hint.
const OUTAGE_TOAST_MAX_COLS: u16 = 72;

/// Toast stack in the bottom-right of `pane_area`, newest on the bottom
/// row, one row each: ` <mark> <text> · <detail> ` ([`toast_spans`]).
///
/// An ack sits on the add-flash ramp step for its fade strength. A PR
/// change and an outage sit on `panel`, where the state glyph and the
/// muted detail stay readable; a PR change fades by mixing its colours
/// toward `surface`, an outage never fades. An outage row may be up to
/// [`OUTAGE_TOAST_MAX_COLS`] wide, the others [`TOAST_MAX_COLS`], and none
/// wider than the pane less 2.
/// Under `NO_COLOR` the row is reversed terminal colours with no fade.
/// While a toast fades the frame is marked animating, so the loop ticks.
/// While every toast holds full strength it asks for one wake at the first
/// fade start (`NO_COLOR`: at the first expiry) and no frame ticks.
fn draw_toasts(frame: &mut Frame<'_>, pane_area: Rect, state: &mut AppState) {
    let now = Instant::now();
    if state.toasts.is_empty() || pane_area.width < 4 || pane_area.height < 3 {
        return;
    }
    let palette = state.theme.palette();
    // Inside the pane border: one column in from the right, one row up.
    let bottom = pane_area.bottom() - 2;
    let rows = usize::from(pane_area.height - 2);
    let toasts: Vec<(Toast, f32)> = state
        .toasts
        .visible(now)
        .map(|(toast, strength)| (toast.clone(), strength))
        .collect();
    for (offset, (toast, strength)) in toasts.iter().rev().take(rows).enumerate() {
        let colors = if state.no_color {
            None
        } else if toast.mark == ToastMark::Ack {
            let Some(bg) = palette.flash_bg_for(FlashKind::Add, *strength) else {
                continue;
            };
            Some((palette, bg, 0))
        } else {
            let fade = ((1.0 - strength) * 100.0).round() as i32;
            let bg = mix_toward(palette.panel, palette.surface, fade);
            Some((palette, bg, fade))
        };
        let max_cols = if toast.is_outage() {
            OUTAGE_TOAST_MAX_COLS
        } else {
            TOAST_MAX_COLS
        };
        let cap = max_cols.min(pane_area.width - 2);
        let spans = toast_spans(toast, state.ascii, usize::from(cap), colors);
        // Cell widths as ratatui paints them, so the row ends one column in
        // from the pane edge with no blank cell.
        let width = spans.iter().map(Span::width).sum::<usize>();
        let width = u16::try_from(width).unwrap_or(cap).min(cap);
        let rect = Rect::new(
            pane_area.right() - 1 - width,
            bottom - offset as u16,
            width,
            1,
        );
        frame.render_widget(Clear, rect);
        frame.render_widget(Paragraph::new(Line::from(spans)), rect);
        drop_hits_under(&mut state.layout, rect);
    }
    match state.toasts.next_tick(now, state.no_color) {
        ToastTick::Frames => state.layout.animating = true,
        ToastTick::WakeIn(ms) => {
            state.layout.wake_ms = Some(state.layout.wake_ms.map_or(ms, |at| at.min(ms)));
        }
        ToastTick::Idle => {}
    }
}

/// Spans of one toast row, at most `cap` columns: the mark, the text, and
/// ` · <detail>`, cut from the end with `…`, then one space.
///
/// `colors` is the palette, the row background, and how far (percent)
/// each foreground mixes toward that background as the toast fades;
/// `None` (`NO_COLOR`) paints reversed terminal colours. The ack mark
/// (` ✓ `, ASCII ` ok `) takes the text colour; a glyph mark paints bold
/// in its own role (a PR state colour). The text paints in the toast's
/// role, the ` · <detail>` tail muted. An outage toast keeps its whole
/// detail (the dismiss hint) and cuts its text instead.
fn toast_spans(
    toast: &Toast,
    ascii: bool,
    cap: usize,
    colors: Option<(Palette, Color, i32)>,
) -> Vec<Span<'static>> {
    let style = |role: SegRole| match colors {
        Some((palette, bg, fade)) => Style::default()
            .fg(mix_toward(seg_role_color(role, palette), bg, fade))
            .bg(bg),
        None => Style::default().add_modifier(Modifier::REVERSED),
    };
    let (mark, mark_style) = match toast.mark {
        ToastMark::Ack => {
            let mark = if ascii { " ok " } else { " ✓ " };
            (mark.to_string(), style(toast.role))
        }
        ToastMark::Glyph(glyph, role) => (
            format!(" {glyph} "),
            style(role).add_modifier(Modifier::BOLD),
        ),
        ToastMark::None => (" ".to_string(), style(toast.role)),
    };
    let body_max = cap.saturating_sub(UnicodeWidthStr::width(mark.as_str()) + 1);
    if toast.is_outage() && !toast.detail.is_empty() {
        let tail = format!(" · {}", toast.detail);
        let text_max = body_max.saturating_sub(UnicodeWidthStr::width(tail.as_str()));
        return vec![
            Span::styled(mark, mark_style),
            Span::styled(fit_with_ellipsis(&toast.text, text_max), style(toast.role)),
            Span::styled(tail, style(SegRole::Muted)),
            Span::styled(" ", style(toast.role)),
        ];
    }
    let body = if toast.detail.is_empty() {
        toast.text.clone()
    } else {
        format!("{} · {}", toast.text, toast.detail)
    };
    let body = fit_with_ellipsis(&body, body_max);
    // A cut takes the detail first; a cut into the text keeps its colour.
    let (text, tail) = match body.strip_prefix(toast.text.as_str()) {
        Some(tail) => (toast.text.clone(), tail.to_string()),
        None => (body, String::new()),
    };
    vec![
        Span::styled(mark, mark_style),
        Span::styled(text, style(toast.role)),
        Span::styled(tail, style(SegRole::Muted)),
        Span::styled(" ", style(toast.role)),
    ]
}

/// How far (percent) each cell behind a centered dialog blends toward the
/// theme `surface`.
pub(crate) const BACKDROP_PERCENT: i32 = 50;

/// Dim every cell of `area` outside `keep` so the dialog in `keep` stands
/// out and the rest of the screen recedes.
///
/// An RGB fg or bg moves [`BACKDROP_PERCENT`] toward `palette.surface`.
/// A non-RGB fg (`Reset`, indexed) gets [`Modifier::DIM`] instead; a
/// non-RGB bg is the terminal's own fill and stays (SGR faint only dims the
/// fg). Under `no_color` no colour changes: every cell gets `DIM` only.
/// Text never changes.
fn dim_backdrop(buf: &mut Buffer, area: Rect, keep: Rect, palette: Palette, no_color: bool) {
    let area = area.intersection(buf.area);
    let surface_rgb = matches!(palette.surface, Color::Rgb(..));
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if keep.contains(Position::new(x, y)) {
                continue;
            }
            let cell = &mut buf[(x, y)];
            if no_color {
                cell.modifier.insert(Modifier::DIM);
                continue;
            }
            if surface_rgb && matches!(cell.fg, Color::Rgb(..)) {
                cell.fg = mix_toward(cell.fg, palette.surface, BACKDROP_PERCENT);
            } else {
                cell.modifier.insert(Modifier::DIM);
            }
            if surface_rgb && matches!(cell.bg, Color::Rgb(..)) {
                cell.bg = mix_toward(cell.bg, palette.surface, BACKDROP_PERCENT);
            }
        }
    }
}

/// Recede the curly underlines behind an open dialog the way
/// [`dim_backdrop`] recedes their cells: each RGB curl colour moves
/// [`BACKDROP_PERCENT`] toward `palette.surface`.
///
/// With a non-RGB surface or curl colour there is no mix to make, so the
/// curls go; the cells keep their plain underline.
fn recede_undercurls(hits: &mut Vec<UndercurlHit>, palette: Palette) {
    if !matches!(palette.surface, Color::Rgb(..)) {
        hits.clear();
        return;
    }
    hits.retain_mut(|hit| {
        if !matches!(hit.color, Color::Rgb(..)) {
            return false;
        }
        hit.color = mix_toward(hit.color, palette.surface, BACKDROP_PERCENT);
        true
    });
}

/// Drop each link and curly-underline span in `layout` that overlaps
/// `rect`: a box painted there hides its cells, so the post-draw writers
/// (OSC 8, undercurl) must not print them again.
fn drop_hits_under(layout: &mut LayoutHit, rect: Rect) {
    layout
        .links
        .retain(|link| !rect.intersects(Rect::new(link.x, link.y, link.width, 1)));
    drop_undercurls_under(&mut layout.undercurls, rect);
}

/// One link per PR badge painted so far this frame in `buf`, to the full
/// URL of its PR, for [`LayoutHit::links`].
///
/// The link covers only the PR ref (`#318`, `!12`) painted inside the
/// badge's icon hit, not the state glyph. The icon hit stays the whole
/// mark, so hover, a click, and Ctrl+click still work on the glyph. A badge
/// that paints no whole ref (the glyph alone, or a ref cut for room) and a
/// badge whose PR is no longer cached get no link.
fn pr_badge_links(state: &AppState, buf: &Buffer) -> Vec<LinkHit> {
    state
        .layout
        .pr_badge_hits()
        .into_iter()
        .filter_map(|hit| {
            let pr = state.pull_request_for(&hit.repo)?;
            painted_text_link(
                buf,
                hit.y,
                hit.x,
                hit.x.saturating_add(hit.width),
                &pr.ref_text(),
                pr.url.clone(),
            )
        })
        .collect()
}

/// Link over the rightmost `needle` painted in buffer row `y`, columns
/// `x0..x1`, or `None` when the row does not show it.
///
/// Matches painted cell symbols, so a hash the widget dropped for room
/// gets no link.
fn painted_text_link(
    buf: &Buffer,
    y: u16,
    x0: u16,
    x1: u16,
    needle: &str,
    url: String,
) -> Option<LinkHit> {
    let (x, width) = painted_text_span(buf, y, x0, x1, needle)?;
    Some(LinkHit { y, x, width, url })
}

/// Column and width of the rightmost `needle` painted in buffer row `y`,
/// columns `x0..x1`, or `None` when the row does not show it whole.
///
/// Matches painted cell symbols, so `x` and the width count cells, not
/// bytes: a multi-byte or wide glyph maps back to the cells it painted.
fn painted_text_span(buf: &Buffer, y: u16, x0: u16, x1: u16, needle: &str) -> Option<(u16, u16)> {
    let (text, starts) = painted_row_text(buf, y, x0, x1)?;
    if needle.is_empty() {
        return None;
    }
    let at = text.rfind(needle)?;
    painted_byte_span(&starts, at, at + needle.len())
}

/// Painted cell symbols of buffer row `y`, columns `x0..x1` (clipped to the
/// buffer), and the byte offset in that text where each cell starts.
/// `None` when the row is outside the buffer.
fn painted_row_text(buf: &Buffer, y: u16, x0: u16, x1: u16) -> Option<(String, Vec<(usize, u16)>)> {
    let area = buf.area;
    if y < area.top() || y >= area.bottom() {
        return None;
    }
    let (x0, x1) = (x0.max(area.left()), x1.min(area.right()));
    let mut text = String::new();
    let mut starts: Vec<(usize, u16)> = Vec::new();
    for x in x0..x1 {
        starts.push((text.len(), x));
        text.push_str(buf[(x, y)].symbol());
    }
    Some((text, starts))
}

/// Column and width of the cells that paint bytes `at..end` of a row text
/// from [`painted_row_text`]; `None` when `at` is not a cell start.
fn painted_byte_span(starts: &[(usize, u16)], at: usize, end: usize) -> Option<(u16, u16)> {
    let x = starts.iter().find(|(start, _)| *start == at)?.1;
    let last = starts.iter().rev().find(|(start, _)| *start < end)?.1;
    Some((x, last - x + 1))
}

/// Pane geometry [`paint_resize_edge_hover`] lights, from [`draw_panes`].
#[derive(Clone, Copy, Debug)]
struct ResizeEdgeArea {
    /// The left pane's outer rect.
    left: Rect,
    /// The left pane's body rows ([`PaneArea::rows`]).
    left_rows: Rect,
    /// The right pane's content rect ([`PaneArea::inner`]).
    right_inner: Rect,
}

/// Tree / graph / files pane on the left and graph / files / diff on the
/// right (Workspace and compare tabs), plus their layout for mouse hits.
/// Returns the geometry the resize-edge highlight paints over, once the
/// overlays have painted.
fn draw_panes(frame: &mut Frame<'_>, pane_area: Rect, state: &mut AppState) -> ResizeEdgeArea {
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
    let left_pane = draw_pane_chrome(
        frame,
        panes[0],
        left_name,
        state.focus == FocusPane::Left,
        palette.sidebar,
        state,
    );
    let tree_inner = left_pane.inner;
    if left_is_files {
        let cursor = state.commit_files_cursor();
        let col_offset = state.left_col_offset as usize;
        // A range compare has no single commit message: its list takes
        // the whole pane. A Diff commit vs parent tab pins the commit's
        // footer like a depth-2 drill.
        if state.is_range_compare_tab() {
            draw_commit_file_list(frame, tree_inner, state, cursor, col_offset);
        } else {
            draw_commit_detail(frame, tree_inner, state, cursor, col_offset);
        }
    } else if left_is_graph {
        draw_graph(frame, tree_inner, state, state.left_col_offset);
    } else {
        draw_tree(frame, tree_inner, state);
    }
    bleed_flat_pad(frame, left_pane);

    let right_name = if state.is_compare_tab() {
        "diff"
    } else if state.drill.is_files() {
        "files"
    } else if state.drill.is_diff() || state.right_is_diff() {
        "diff"
    } else {
        "graph"
    };
    let right_pane = draw_pane_chrome(
        frame,
        panes[1],
        right_name,
        state.focus == FocusPane::Right,
        palette.surface,
        state,
    );
    let right_inner = right_pane.inner;
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    draw_right(frame, right_inner, state);
    bleed_flat_pad(frame, right_pane);

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
        Some(diff_split_rule_x(
            right_inner.x,
            right_inner.width,
            state.diff_split_fraction,
        ))
    } else {
        None
    };
    state.layout.right_y = right_inner.y;
    let left_files_cursor = match &state.drill {
        _ if state.is_compare_tab() => Some(state.commit_files_cursor()),
        super::drill::DrillView::Diff { file_cursor, .. } => Some(*file_cursor),
        _ => None,
    };
    if let Some(cursor) = left_files_cursor {
        // The commit footer under the list: a depth-2 drill or a Diff
        // commit vs parent tab; a range compare tab has none.
        let footer_h = if state.is_range_compare_tab() {
            0
        } else {
            commit_detail_footer_height(
                state.commit_detail_footer_request(),
                state.commit_detail_footer_meta_rows(usize::from(tree_inner.width)),
                tree_inner.height,
            )
        };
        // List rows only: not the horizontal bar row a panned list keeps.
        let list_h = tree_inner.height.saturating_sub(footer_h);
        let list_h = list_h.saturating_sub(list_hbar_rows(list_h, state.left_col_offset));
        state.layout.files_list_y = tree_inner.y;
        state.layout.files_list_height = list_h;
        let painted_n = state.painted_commit_file_rows().len();
        let (start, _) = visible_window(painted_n, cursor, list_h as usize);
        state.layout.files_list_offset = start;
    } else if let super::drill::DrillView::Files { cursor, .. } = &state.drill {
        let height = right_inner.height;
        let list_h = height.saturating_sub(list_hbar_rows(height, state.right_col_offset));
        state.layout.files_list_y = right_inner.y;
        state.layout.files_list_height = list_h;
        let list_h = list_h as usize;
        let painted_n = state.painted_commit_file_rows().len();
        let (start, _) = visible_window(painted_n, *cursor, list_h);
        state.layout.files_list_offset = start;
    }
    ResizeEdgeArea {
        left: panes[0],
        left_rows: left_pane.rows,
        right_inner,
    }
}

/// File view body copy while it loads.
const LOADING_FILE: &str = "loading…";
/// Explorer tree copy for a checkout with no entries.
const EMPTY_FOLDER: &str = "empty folder";
/// File view body for an empty file.
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

/// First line to paint so `cursor` sits near the vertical middle of
/// `height` rows (Ctrl-d / Ctrl-u, [`FileTab::recenter`]).
///
/// At most half the rows go above the cursor, and the top never passes the
/// one that shows the last of `line_count` lines on the bottom row, as
/// `tree::list_viewport_start` does for the tree. Reads at most two
/// screens of lines.
///
/// [`FileTab::recenter`]: super::tabs::FileTab::recenter
fn file_view_middle_top(
    cursor: usize,
    line_count: usize,
    height: usize,
    rows_of: impl Fn(usize) -> usize,
) -> usize {
    let height = height.max(1);
    let budget = (height / 2).min(height.saturating_sub(rows_of(cursor)));
    let mut top = cursor;
    let mut above = 0usize;
    while top > 0 {
        let rows = rows_of(top - 1);
        if above + rows > budget {
            break;
        }
        above += rows;
        top -= 1;
    }
    let last = line_count.saturating_sub(1);
    let max_top = (last + 1).saturating_sub(file_lines_ending_at(last, height, rows_of));
    top.min(max_top)
}

/// File lines counted back from `cursor` (included) whose painted rows fit
/// in `height`. At least 1.
fn file_lines_ending_at(cursor: usize, height: usize, rows_of: impl Fn(usize) -> usize) -> usize {
    let mut used = rows_of(cursor);
    let mut count = 1;
    for line in (0..cursor).rev() {
        used += rows_of(line);
        if used > height {
            break;
        }
        count += 1;
    }
    count
}

/// The active read-only file body (an Explorer inner file tab of a clean
/// file) inside `pane.inner`: line numbers and highlighted
/// code, or a loading / binary / too-large / empty notice.
///
/// The text keeps the scrollbars of the Explorer changed-file diff
/// (`draw_diff_pane`) in the same cells, so switching between a changed
/// and a clean file does not move the chrome: the last column is kept for
/// the vertical bar whether or not it shows, the bar paints there whenever
/// the lines overflow (or the top is not the first line), and once the
/// view pans off the left edge the horizontal bar takes the last row, left
/// of the vertical bar's corner (a 1-row body keeps its row and shows no
/// horizontal bar). Each painted comment mark records an icon hit
/// ([`IconTarget::LineComment`]).
fn draw_file_body(frame: &mut Frame<'_>, pane: PaneArea, state: &mut AppState) {
    // Like the file diff, the popover paint reads the cursor line's marks
    // from here.
    state.layout.cursor_line_marks = mark_targets(state.file_cursor_comment_marks());
    let palette = state.theme.palette();
    let inner = pane.inner;
    state.layout.file_view_rows = pane.rows;
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
        None => Some((
            spinner_text(
                state.anim_epoch,
                state.ascii,
                &mut state.layout,
                LOADING_FILE,
            ),
            loading_fg(state.no_color, palette),
        )),
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
    let wrap = state.diff_wrap;
    // The pan clamp of Shift-←/→ ([`FileTab::pan_max`]; it reads no file).
    // A pan it no longer allows (the pane widened since the pan) drops to
    // it here, as the next Shift-←/→ would clamp it; at 0 the view keeps
    // no bar row. Wrap paints unpanned and keeps the pan for later.
    let col_max = if wrap {
        0
    } else {
        tab.pan_max(inner.width, state.ascii)
    };
    if !wrap && usize::from(tab.col_offset) > col_max {
        let clamped = col_max.min(usize::from(u16::MAX)) as u16;
        state.scroll_hbar_to(HScrollbar::FileView, clamped);
    }
    let Some(tab) = state.tabs.active_file_view() else {
        return;
    };
    let lines = tab.lines();
    let col_offset = if wrap { 0 } else { tab.col_offset };
    // Line rows: not the horizontal bar row a panned view keeps. A 1-row
    // body keeps its row and paints no bar, as a 1-row list does.
    let hbar_rows = list_hbar_rows(inner.height, col_offset);
    let rows_h = inner.height - hbar_rows;
    state.layout.file_view_height = rows_h;
    let height = usize::from(rows_h);
    // Comment mark, line number, one blank column.
    let digits = file_gutter_width(lines.len()) - 1;
    let gutter = file_view_gutter_width(lines.len(), state.ascii);
    let code_w = tab.code_width(inner.width, state.ascii);
    // A row count past the view height is never needed, so a 2 MiB
    // single-line file scans at most one screen of its text per call.
    let row_cap = height + 1;
    let rows_of = |line: usize| tab.line_rows(line, code_w, wrap, row_cap);
    // `tab.scroll` keeps the jump top (it is sticky: the next frame
    // starts from it); only the painted window glides. `j` / `k` move the
    // top only as far as the cursor needs; Ctrl-d / Ctrl-u put the cursor
    // near the middle, so the glide has rows to cover.
    let target = if tab.recenter {
        file_view_middle_top(tab.cursor, lines.len(), height, rows_of)
    } else {
        file_view_scroll(tab.scroll, tab.cursor, height, rows_of)
    };
    let scroll = if state.glide_pending(GlidePane::FileView) {
        let span = file_lines_ending_at(tab.cursor, height, rows_of);
        let (scroll, gliding) = glide_step(
            &mut state.scroll_glide,
            GlidePane::FileView,
            target,
            tab.cursor,
            span,
            state.frame_now,
        );
        state.layout.gliding |= gliding;
        scroll
    } else {
        target
    };
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
    let blame = state.painted_line_annotation();
    // A `V` range paints like the highlighted rows of a diff.
    let visual = state.file_visual_range();
    let mut painted: Vec<Line> = Vec::with_capacity(height);
    let mut row_lines = Vec::with_capacity(height);
    // Screen row the blame note painted on, for its commit popover hits.
    let mut note_y: Option<u16> = None;
    let mut mark_hits = Vec::new();
    let rows_right = inner.x.saturating_add(diff_paint_width(inner.width));
    for (line, text) in lines.iter().enumerate().take(end).skip(scroll) {
        let selected = line == tab.cursor;
        let hit = hits.binary_search(&line).is_ok();
        let in_visual = visual.is_some_and(|(lo, hi)| (lo..=hi).contains(&line));
        let bg = if in_visual && !selected {
            Some(palette.cursor_bg)
        } else {
            row_match_bg(selected, true, hit, None, palette, search.bg)
        };
        let match_fg = (hit && !selected).then_some(search.fg);
        let n = u32::try_from(line + 1).unwrap_or(u32::MAX);
        let comment = state.file_line_comment(n);
        // Every painted row of a line with an open comment underlines its
        // code; the runs get the curly upgrade after the draw.
        let underline = if comment == Some(false) {
            Modifier::UNDERLINED
        } else {
            Modifier::empty()
        };
        let code = spans.get(line - scroll).map(Vec::as_slice).unwrap_or(&[]);
        let starts = if wrap {
            wrap_col_starts_capped(text, code_w, row_cap)
        } else {
            vec![usize::from(col_offset)]
        };
        for (part, &start) in starts.iter().enumerate() {
            if painted.len() >= height {
                break;
            }
            if part == 0 {
                // The mark is the gutter's first column; it does not pan.
                let y = inner.y.saturating_add(painted.len() as u16);
                let mark = state.line_comment_mark(n, comment);
                mark_hits.extend(mark.and_then(|(kind, target)| {
                    line_mark_hit(kind, target, inner.x, y, rows_right, state.ascii)
                }));
            }
            let number = if part == 0 {
                format!(
                    "{} ",
                    format_line_gutter(Some(n), digits, comment, state.ascii)
                )
            } else {
                " ".repeat(gutter)
            };
            let with_bg = |style: Style| bg.map_or(style, |bg| style.bg(bg));
            let mut row = vec![Span::styled(number, with_bg(gutter_style))];
            let mut used = 0usize;
            for span in slice_styled_cols(code, start, code_w) {
                used += visible_width(&span.text);
                let fg = match_fg.unwrap_or(span.fg);
                let style = Style::default().fg(fg).add_modifier(underline);
                row.push(Span::styled(span.text, with_bg(style)));
            }
            if bg.is_some() && used < code_w {
                row.push(Span::styled(
                    " ".repeat(code_w - used),
                    with_bg(Style::default()),
                ));
                if selected && part + 1 == starts.len() {
                    if let Some((text, _)) = &blame {
                        put_line_annotation(&mut row, text, palette, false);
                        note_y = Some(inner.y.saturating_add(painted.len() as u16));
                    }
                }
            }
            painted.push(Line::from(row));
            row_lines.push(line);
        }
    }
    // The rows stop left of the vertical bar column and above the
    // horizontal bar row, as the file diff's rows do.
    let rows_area = Rect {
        width: diff_paint_width(inner.width),
        height: rows_h,
        ..inner
    };
    frame.render_widget(Paragraph::new(painted), rows_area);
    state.layout.icon_hits.extend(mark_hits);
    if let (Some(y), Some((note, _)), Some((repo, line_blame))) =
        (note_y, &blame, state.painted_line_blame())
    {
        let hits = blame_note_hits(
            frame.buffer_mut(),
            &[y],
            rows_area.left(),
            rows_area.right(),
            note,
            &line_blame,
            &repo,
        );
        state.layout.icon_hits.extend(hits);
    }
    let curls = underlined_runs(frame.buffer_mut(), rows_area, palette.comment_underline());
    state.layout.undercurls.extend(curls);
    if state.background == BackgroundMode::Paint {
        bleed_diff_row_edges(frame, rows_area, inner.right());
    }
    let vscroll = scroll > 0 || end < lines.len() || used > height;
    let buf = frame.buffer_mut();
    if vscroll {
        let track = Rect {
            x: inner.right() - 1,
            width: 1,
            ..inner
        };
        // The top that shows the last line on the bottom row: a track
        // click on the last row, or a drag to the end, shows it.
        let last = lines.len() - 1;
        let scroll_max = (last + 1).saturating_sub(file_lines_ending_at(last, height, rows_of));
        state.layout.file_view_vbar = paint_vscrollbar(buf, track, lines.len(), scroll, scroll_max);
    }
    if hbar_rows > 0 && col_max > 0 {
        let track = Rect {
            y: inner.bottom() - 1,
            width: inner.width.saturating_sub(u16::from(vscroll)).max(1),
            height: 1,
            ..inner
        };
        state.layout.file_view_hbar = paint_hscrollbar(buf, track, col_max, col_offset);
    }
    state.layout.file_view_row_lines = row_lines;
    // Store the jump top, not a glide step: the next frame keeps gliding
    // toward it, and the sticky rule starts from it once the glide ends.
    if let Some(tab) = state.tabs.active_file_view_mut() {
        tab.scroll = target;
        tab.recenter = false;
    }
}

/// Pane chrome shared by the left pane, the right pane, and the Explorer
/// tab's two panes (so the Workspace, compare, and Explorer tabs look the
/// same).
///
/// [`BackgroundMode::Terminal`]: a boxed [`Block`] with the title in
/// `heading` and a [`pane_border`] colour that shows focus (the v0.1.244
/// look). [`BackgroundMode::Paint`]: `area` is filled with `bg`, no border
/// glyphs, and row 0 is the title row ([`flat_title_line`]): the title and
/// a rule to the right edge whose colours show focus. Focus never moves
/// the content. Returns the pane's [`PaneArea`]: boxed, the block's inner
/// rect; flat, `area` less its first row and [`FLAT_PAD_COLS`] on each
/// side. Call [`bleed_flat_pad`] once the content has drawn.
fn draw_pane_chrome(
    frame: &mut Frame<'_>,
    area: Rect,
    title: &str,
    focused: bool,
    bg: Color,
    state: &AppState,
) -> PaneArea {
    let palette = state.theme.palette();
    if state.background == BackgroundMode::Terminal {
        let block = Block::default()
            .borders(Borders::ALL)
            .title(pane_title(title))
            .title_style(Style::default().fg(palette.heading))
            .border_style(pane_border(focused, palette));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        return PaneArea { inner, rows: inner };
    }
    fill_bg(frame, area, bg);
    if area.height > 0 {
        frame.render_widget(
            Paragraph::new(flat_title_line(
                title,
                focused,
                area.width,
                state.ascii,
                palette,
            )),
            Rect { height: 1, ..area },
        );
    }
    let chrome = area.height.min(FLAT_CHROME_ROWS);
    let rows = Rect {
        y: area.y + chrome,
        height: area.height - chrome,
        ..area
    };
    // A pane too narrow to keep a content column between its pads drops
    // the pads.
    let pad = if area.width > 2 * FLAT_PAD_COLS {
        FLAT_PAD_COLS
    } else {
        0
    };
    PaneArea {
        inner: Rect {
            x: rows.x + pad,
            width: rows.width - 2 * pad,
            ..rows
        },
        rows,
    }
}

/// Rows a flat pane's chrome takes above its content: the title row.
const FLAT_CHROME_ROWS: u16 = 1;

/// Blank columns a flat pane keeps on each side of its content. Row
/// backgrounds and the selection bar run into them ([`bleed_flat_pad`]).
const FLAT_PAD_COLS: u16 = 1;

/// Where a pane's rows go, from [`draw_pane_chrome`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct PaneArea {
    /// Content rect: rows draw here. Inside the border when boxed, inside
    /// the [`FLAT_PAD_COLS`] pad columns when flat.
    inner: Rect,
    /// The content rows across the full pane width, pad cells included
    /// (flat). Equal to `inner` when boxed. A press anywhere in it picks
    /// the row.
    rows: Rect,
}

/// Finish a flat pane after its content has drawn: each content row's pad
/// cells take that row's background (the left pad from the first content
/// cell, the right pad from the last), so selection, flash, search, and
/// diff tints run to both pane edges.
///
/// A selection bar ([`CURSOR_BAR`] / [`CURSOR_BAR_INACTIVE`]) in the first
/// content column moves to the left pad cell, on the pane edge, and leaves
/// a blank on the row background, so row text lines up with unselected
/// rows. Bars in other columns stay. A no-op when boxed (`rows == inner`).
fn bleed_flat_pad(frame: &mut Frame<'_>, pane: PaneArea) {
    let PaneArea { inner, rows } = pane;
    if inner.width == 0 || inner == rows {
        return;
    }
    let buf = frame.buffer_mut();
    let first = inner.x;
    let last = inner.right() - 1;
    for y in inner.top()..inner.bottom() {
        if rows.x < first {
            let cell = buf[(first, y)].clone();
            if matches!(cell.symbol(), CURSOR_BAR | CURSOR_BAR_INACTIVE) {
                let bg = cell.bg;
                buf[(rows.x, y)] = cell;
                let blank = &mut buf[(first, y)];
                blank.reset();
                blank.set_bg(bg);
            } else {
                buf[(rows.x, y)].set_bg(cell.bg);
            }
        }
        if rows.right() > inner.right() {
            let bg = buf[(last, y)].bg;
            buf[(rows.right() - 1, y)].set_bg(bg);
        }
    }
}

/// Flat-pane resize line glyph: a right one-eighth block hugs the pane
/// join from the left pane's right pad cell.
const RESIZE_EDGE_FLAT: &str = "▕";

/// [`RESIZE_EDGE_FLAT`] in ASCII mode.
const RESIZE_EDGE_FLAT_ASCII: &str = "|";

/// Light the resize edge [`AppState::hovered_resize_edge`] names, in the
/// focus accent (`cursor`), over the pane body rows only.
///
/// Pane divider: boxed panes recolour the left pane's right border glyph;
/// flat panes paint [`RESIZE_EDGE_FLAT`] in the left pane's right pad cell
/// and keep its background. The right pane's left pad cell carries the
/// selected-row bar, so it never changes. Side-by-side RULE: the
/// [`DIFF_RULE`] glyphs in the recorded RULE column take the accent.
/// Commit-message footer: the recorded top rule row (`▁`, `_` in ASCII
/// mode) takes the accent across the graph pane width; its glyph and
/// background stay.
///
/// Call after the panes and the icon popover paint, so the layout holds
/// this frame's hit geometry and popover rect: the edge lit here is the
/// edge the pointer shape reads after the draw. A hover popover keeps its
/// cells; a pointer inside it hovers no edge.
fn paint_resize_edge_hover(frame: &mut Frame<'_>, state: &AppState, edges: ResizeEdgeArea) {
    let Some(edge) = state.hovered_resize_edge() else {
        return;
    };
    let ResizeEdgeArea {
        left,
        left_rows,
        right_inner,
    } = edges;
    let popover = state.layout.popover.as_ref().map(|hit| hit.rect);
    let covered = |x: u16, y: u16| popover.is_some_and(|rect| rect.contains(Position::new(x, y)));
    let accent = state.theme.palette().cursor;
    let buf = frame.buffer_mut();
    match edge {
        ResizeEdge::Pane => {
            // The left pane's last column: its right border when boxed,
            // its right pad cell when flat.
            let Some(x) = left.right().checked_sub(1) else {
                return;
            };
            let flat = state.background == BackgroundMode::Paint;
            let glyph = if state.ascii {
                RESIZE_EDGE_FLAT_ASCII
            } else {
                RESIZE_EDGE_FLAT
            };
            for y in (left_rows.top()..left_rows.bottom()).filter(|&y| !covered(x, y)) {
                let cell = &mut buf[(x, y)];
                if flat {
                    cell.set_symbol(glyph);
                }
                cell.set_fg(accent);
            }
        }
        ResizeEdge::DiffSplit => {
            let Some(x) = state.layout.diff_split_rule_x else {
                return;
            };
            if x < right_inner.left() || x >= right_inner.right() {
                return;
            }
            let rule = DIFF_RULE.to_string();
            for y in (right_inner.top()..right_inner.bottom()).filter(|&y| !covered(x, y)) {
                let cell = &mut buf[(x, y)];
                if cell.symbol() == rule {
                    cell.set_fg(accent);
                }
            }
        }
        ResizeEdge::CommitMsg => {
            let Some(y) = state.layout.commit_msg_rule_y else {
                return;
            };
            let area = buf.area;
            if y < area.top() || y >= area.bottom() {
                return;
            }
            let x0 = state.layout.commit_msg_rule_x.max(area.left());
            let x1 = state
                .layout
                .commit_msg_rule_x
                .saturating_add(state.layout.commit_msg_rule_width)
                .min(area.right());
            for x in (x0..x1).filter(|&x| !covered(x, y)) {
                buf[(x, y)].set_fg(accent);
            }
        }
    }
}

/// Title row of a flat pane, `width` cells: one blank cell, the title,
/// one blank cell, then a rule (`▁`, `_` in ASCII mode) to the right edge.
///
/// Focused: the title is `heading`, bold, and the rule `cursor`.
/// Unfocused: the title is `muted`, normal weight, and the rule
/// `border_dim`. Focus changes only these colours, never the layout. The
/// title is never underlined. A narrow pane clips the title first, then
/// drops the trailing blank and the rule. The background is left to the
/// pane fill.
fn flat_title_line(
    title: &str,
    focused: bool,
    width: u16,
    ascii: bool,
    palette: Palette,
) -> Line<'static> {
    let title_w = painted_width(title).min(width.saturating_sub(1));
    let rest = width.saturating_sub(1).saturating_sub(title_w);
    let rule_w = usize::from(rest.saturating_sub(1));
    let (title_style, rule_fg) = if focused {
        (
            Style::default()
                .fg(palette.heading)
                .add_modifier(Modifier::BOLD),
            palette.cursor,
        )
    } else {
        (Style::default().fg(palette.muted), palette.border_dim)
    };
    let mut spans = vec![Span::raw(" "), Span::styled(title.to_string(), title_style)];
    if rest > 0 {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            glyph(ascii, "\u{2581}", "_").repeat(rule_w),
            Style::default().fg(rule_fg),
        ));
    }
    Line::from(spans)
}

/// Set the background of every cell in `area` to `bg` (paint mode). Text
/// and spans without their own background keep it.
fn fill_bg(frame: &mut Frame<'_>, area: Rect, bg: Color) {
    frame.buffer_mut().set_style(area, Style::default().bg(bg));
}

/// The active Explorer tab: the checkout's file tree on the left (titled
/// with the checkout path, on `sidebar` in paint mode) and the inner file
/// tabs on the right (on `surface`): a row of inner tab labels
/// ([`draw_explorer_file_tabs`]) over the active one — the whole-file
/// worktree diff of a changed file or the body of a clean one. With no
/// inner tab, a folder row shows a short hint. Both panes go through
/// [`draw_pane_chrome`], so they are boxed or flat as the Workspace panes
/// are. `explorer_tree` / `explorer_preview` record the content rects
/// (the preview below the inner tab row), `explorer_tree_rows` /
/// `explorer_preview_rows` the full-width rows (pad cells included when
/// flat). The pane divider and, on a side-by-side diff, the in-diff RULE
/// are recorded as on the Workspace, and the returned geometry is what the
/// resize-edge highlight paints over (`None` with no Explorer tab).
fn draw_explorer_tab(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut AppState,
) -> Option<ResizeEdgeArea> {
    let widths = pane_widths(area.width, state.tree_fraction);
    let panes = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(widths.tree_width),
            Constraint::Min(MIN_PANE_COLS),
        ])
        .split(area);
    let palette = state.theme.palette();
    let tab = state.tabs.active_explorer()?;
    let preview_title = match tab.active_inner().map(|inner| &inner.view) {
        None => "preview".to_string(),
        Some(ExplorerPreview::Diff { .. }) => "diff".to_string(),
        Some(ExplorerPreview::File(file)) => file.display.clone(),
    };
    let has_files = !tab.files.is_empty();
    // One status index and one row walk per frame, shared by the tree,
    // the folder hint, and the breadcrumb. A panned tree walks its rows
    // once more for its pan clamp (`explorer_tree_max_col_in`).
    let status = state.explorer_status(&tab.checkout);
    let rows = tab.tree.rows(&status);
    let cursor = tab.tree.cursor_index(&rows);
    let folder = cursor.is_some_and(|index| rows[index].is_dir);
    let tree_pane = draw_pane_chrome(
        frame,
        panes[0],
        &tab.checkout,
        state.focus == FocusPane::Left,
        palette.sidebar,
        state,
    );
    let tree_inner = tree_pane.inner;
    state.layout.explorer_tree = tree_inner;
    state.layout.explorer_tree_rows = tree_pane.rows;
    draw_explorer_tree(frame, tree_inner, state, &rows, cursor, &status);
    bleed_flat_pad(frame, tree_pane);
    if let Some(tab) = state.tabs.active_explorer_mut() {
        tab.painted_rel = cursor.map(|index| rows[index].rel.clone());
    }

    let right_pane = draw_pane_chrome(
        frame,
        panes[1],
        &preview_title,
        state.focus == FocusPane::Right,
        palette.surface,
        state,
    );
    let mut body_pane = right_pane;
    if has_files && right_pane.inner.height > 0 {
        draw_explorer_file_tabs(
            frame,
            Rect {
                height: 1,
                ..right_pane.inner
            },
            state,
        );
        for rect in [&mut body_pane.inner, &mut body_pane.rows] {
            rect.y += 1;
            rect.height -= 1;
        }
    }
    let right_inner = body_pane.inner;
    state.layout.explorer_preview = right_inner;
    state.layout.explorer_preview_rows = body_pane.rows;
    state.layout.right_x = panes[1].x;
    state.layout.outer_tree_width = panes[0].width;
    state.layout.right_y = right_inner.y;
    state.layout.diff_content_x = right_inner.x;
    state.layout.diff_pane_width = right_inner.width;
    state.layout.diff_pane_height = right_inner.height;
    let edges = ResizeEdgeArea {
        left: panes[0],
        left_rows: tree_pane.rows,
        right_inner,
    };
    let view = state
        .tabs
        .active_explorer()
        .and_then(|tab| tab.active_inner())
        .map(|inner| matches!(inner.view, ExplorerPreview::File(_)));
    // A changed file's diff paints side by side as on the Workspace, with
    // the same RULE to drag.
    state.layout.diff_split_rule_x = (view == Some(false)
        && right_inner.width > 0
        && state.diff_layout() == DiffMode::SideBySide)
        .then(|| diff_split_rule_x(right_inner.x, right_inner.width, state.diff_split_fraction));
    // The inner tab row keeps the pane fill in its pad cells; only the
    // body rows bleed.
    if right_inner.width == 0 || right_inner.height == 0 {
        bleed_flat_pad(frame, body_pane);
        return Some(edges);
    }
    match view {
        Some(false) => draw_diff_pane(frame, right_inner, state),
        Some(true) => draw_file_body(frame, body_pane, state),
        None if folder => frame.render_widget(
            Paragraph::new(muted_copy(EXPLORER_FOLDER_HINT, palette)),
            right_inner,
        ),
        None => {}
    }
    bleed_flat_pad(frame, body_pane);
    Some(edges)
}

/// The active Explorer tab's inner file tab row in `area` (one row): each
/// tab's file name between blanks, [`TAB_SEPARATOR`] between tabs. The
/// active tab paints as the active top-level tab does; the preview tab
/// (unpinned) is italic and dim. The row scrolls so the active tab shows,
/// with the strip's overflow markers for hidden tabs.
fn draw_explorer_file_tabs(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
    let Some(tab) = state.tabs.active_explorer() else {
        return;
    };
    if area.width == 0 || tab.files.is_empty() {
        return;
    }
    let palette = state.theme.palette();
    let muted = Style::default().fg(palette.muted);
    let sep_w = painted_width(TAB_SEPARATOR);
    let texts: Vec<String> = tab
        .files
        .iter()
        .map(|file| format!(" {} ", file.label()))
        .collect();
    let costs: Vec<u16> = texts
        .iter()
        .enumerate()
        .map(|(index, text)| {
            let sep = if index > 0 { sep_w } else { 0 };
            painted_width(text).saturating_add(sep)
        })
        .collect();
    let active = tab.active_file.unwrap_or(0);
    let window = tab_window(&costs, area.width, 0, active);
    let mut spans = Vec::new();
    if window.start > 0 {
        spans.push(Span::styled(tab_left_marker(window.start), muted));
    }
    let painted = tab.files.iter().zip(texts).enumerate();
    for (index, (file, text)) in painted.take(window.end).skip(window.start) {
        // As in the top strip: `tab_window` counts a separator before
        // every tab after the first.
        if index > 0 {
            spans.push(Span::styled(TAB_SEPARATOR, muted));
        }
        let mut style = if index == active {
            Style::default()
                .fg(palette.cursor)
                .bg(palette.cursor_bg)
                .add_modifier(Modifier::BOLD)
        } else {
            muted
        };
        if !file.pinned {
            style = style
                .remove_modifier(Modifier::BOLD)
                .add_modifier(Modifier::ITALIC | Modifier::DIM);
        }
        spans.push(Span::styled(text, style));
    }
    let mut line_area = area;
    if window.end < tab.files.len() {
        let marker = tab_right_marker(tab.files.len() - window.end);
        let width = painted_width(&marker).min(area.width);
        line_area.width = area.width - width;
        frame.render_widget(
            Paragraph::new(Span::styled(marker, muted)),
            Rect {
                x: area.right() - width,
                width,
                ..area
            },
        );
    }
    frame.render_widget(Paragraph::new(Line::from(spans)), line_area);
}

/// Explorer tree `rows` (the cursor on `cursor`) inside `area`, keeping
/// the cursor row in view. `status` gives each row its mark. The rows pan
/// by the tab's tree pan, clamped as Shift-→ clamps it
/// ([`clamped_list_pan`] over [`AppState::explorer_tree_max_col_in`]), and
/// keep their bars as the Workspace tree does ([`ListBars`]).
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
    let offset = state
        .tabs
        .active_explorer()
        .map_or(0, |tab| tab.tree_col_offset);
    let (pan, col_max) = clamped_list_pan(
        state,
        HScrollbar::ExplorerTree,
        area,
        rows.len(),
        offset,
        |state, vscroll| state.explorer_tree_max_col_in(area.width, vscroll),
    );
    let bars = ListBars::new(area, rows.len(), pan);
    let rows_area = bars.rows_area();
    let height = rows_area.height as usize;
    let (start, _) = visible_window(rows.len(), cursor.unwrap_or(0), height);
    if let Some(tab) = state.tabs.active_explorer_mut() {
        tab.tree_scroll = start;
    }
    let focused = state.focus == FocusPane::Left;
    let search = state.theme.pills().filter;
    let matches: HashSet<usize> = if state.search_target == SearchPane::ExplorerTree {
        explorer_row_match_indices(rows, &state.search_query)
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let loading_text = if rows
        .iter()
        .skip(start)
        .take(height)
        .any(|row| row.placeholder)
    {
        loading_line(state, LOADING_FILE)
    } else {
        LOADING_FILE.to_string()
    };
    let loading = TextSeg {
        text: loading_text,
        role: if state.no_color {
            SegRole::Plain
        } else {
            SegRole::Muted
        },
        hex: None,
        bold: false,
        dim: false,
        icon: None,
    };
    let checkout = state
        .tabs
        .active_explorer()
        .map(|tab| tab.checkout.clone())
        .unwrap_or_default();
    let marks = state.explorer_row_marks(&checkout);
    let width = rows_area.width as usize;
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    for ((index, row), y) in rows
        .iter()
        .enumerate()
        .skip(start)
        .take(height)
        .zip(rows_area.y..)
    {
        let segs = if row.placeholder {
            NodeSegments {
                segments: vec![loading.clone()],
                trailing: Vec::new(),
            }
        } else {
            state.explorer_row_paint_segments(row, status, &marks)
        };
        // Glyph-only hits, panned and clipped as the row paints them
        // (the vertical bar's column is outside `width`).
        for (kind, x, w) in segmented_icon_spans(row.depth, &segs, width, usize::from(pan)) {
            hits.push(IconHit {
                y,
                x: rows_area
                    .x
                    .saturating_add(u16::try_from(x).unwrap_or(u16::MAX)),
                width: u16::try_from(w).unwrap_or(u16::MAX),
                kind,
                target: IconTarget::ExplorerRow(row.rel.clone()),
            });
        }
        lines.push(paint_segmented_row(
            row.depth,
            row.is_dir,
            !row.expanded,
            &segs,
            width,
            Some(index) == cursor,
            focused,
            state.explorer_row_flash_color(row, &marks),
            matches.contains(&index),
            search,
            state.ascii,
            palette,
            usize::from(pan),
        ));
    }
    state.layout.icon_hits.extend(hits);
    frame.render_widget(Paragraph::new(lines), rows_area);
    let flat = state.background == BackgroundMode::Paint;
    let (vbar, hbar) = bars.paint(frame, start, (pan, col_max), flat);
    state.layout.explorer_tree_vbar = vbar;
    state.layout.explorer_tree_hbar = hbar;
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
    let (pan, col_max) = clamped_list_pan(
        state,
        HScrollbar::Tree,
        area,
        painted.len(),
        state.left_col_offset,
        |state, vscroll| state.tree_max_col_in(area.width, vscroll),
    );
    let bars = ListBars::new(area, painted.len(), pan);
    let rows_area = bars.rows_area();
    let height = rows_area.height as usize;
    let width = rows_area.width as usize;
    let focus_id = state.rows.get(state.cursor).map(|row| row.id.as_str());
    let painted_cursor = focus_id
        .and_then(|id| painted.iter().position(|row| row.id == id))
        .unwrap_or(0);
    let (target, _) = visible_window(painted.len(), painted_cursor, height);
    let (start, gliding) = glide_step(
        &mut state.scroll_glide,
        GlidePane::Tree,
        target,
        painted_cursor,
        height,
        state.frame_now,
    );
    state.layout.gliding |= gliding;
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
    let now = Instant::now();
    let running = &state.remote_activity.running;
    let running_families: HashSet<String> = running
        .iter()
        .map(|checkout| {
            state
                .snapshot
                .repos
                .iter()
                .find(|snap| &snap.repo == checkout)
                .and_then(|snap| snap.primary_repo.clone())
                .unwrap_or_else(|| checkout.clone())
        })
        .collect();
    let pr_totals = state.pr_totals();
    for (y, row) in (area.y..).zip(painted.iter().skip(start).take(height)) {
        let (viewed, commented, resolved, pr) = state.tree_row_marks(row, width, now);
        let spinner = tree_row_runs_remote(row, &state.remote_activity.running, &running_families)
            .then(|| spinner_glyph(state.anim_epoch, now, state.ascii));
        state.layout.animating |= spinner.is_some();
        // Commit-activity bars: repo rows only, while the sparkline is on,
        // once the worker has counted, and only when the pane leaves the
        // label room (the pane width, scrollbar column included). Off, the
        // label keeps the bar columns.
        let bars = (state.activity_sparkline && usize::from(area.width) >= SPARK_MIN_TREE_COLS)
            .then(|| row_activity_key(row))
            .flatten()
            .and_then(|key| state.activity.get(key))
            .and_then(|days| sparkline(days, state.ascii));
        let decor = TreeRowDecor {
            viewed,
            commented,
            resolved,
            pr,
            pr_totals,
            spinner,
            sparkline: bars,
            no_color: state.no_color,
        };
        let (line, icons, total_cols) = paint_tree_row(
            row,
            width,
            Some(row.id.as_str()) == focus_id,
            state.focus == FocusPane::Left,
            state.flash_color(&row.id),
            match_ids.contains(&row.id),
            search,
            state.ascii,
            decor,
            palette,
            state.left_col_offset as usize,
        );
        lines.push(line);
        // The PR total ends the workspace row flush right, left of the
        // vertical bar's column; a click on it opens the PRs tab.
        if let Ok(total_w) = u16::try_from(total_cols) {
            if total_w > 0 && total_w <= rows_area.width {
                state.layout.pr_total_hit =
                    Some(Rect::new(rows_area.right() - total_w, y, total_w, 1));
            }
        }
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
    frame.render_widget(Paragraph::new(lines), rows_area);
    let flat = state.background == BackgroundMode::Paint;
    let (vbar, hbar) = bars.paint(frame, start, (pan, col_max), flat);
    state.layout.tree_vbar = vbar;
    state.layout.tree_hbar = hbar;
}

/// Painted icon of a row: catalog kind, column offset from the row start,
/// and width.
type IconSpan = (IconKind, usize, usize);

/// Per-row marks and live decorations a tree row paints beside its own
/// segments.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct TreeRowDecor {
    /// File row is marked reviewed.
    viewed: bool,
    /// Row has a comment.
    commented: bool,
    /// Every comment on the row is resolved.
    resolved: bool,
    /// PR badge of the row's branch: `<ref> <glyph>` at the end of the
    /// trailing run.
    pr: Option<PrBadge>,
    /// Workspace PR total; only the workspace row paints it.
    pr_totals: PrTotals,
    /// Spinner frame while a fetch / pull / push runs on the row's
    /// checkout. Paints ahead of the marks, after the sparkline.
    spinner: Option<&'static str>,
    /// Commit-activity bars of a repo row ([`sparkline`]). Paints first in
    /// the trailing run, ahead of the spinner and the marks: the run is
    /// right-aligned, so the bars take label room and the marks never move.
    sparkline: Option<String>,
    /// `NO_COLOR`: the spinner and the sparkline take the terminal
    /// foreground.
    no_color: bool,
}

/// A repo or checkout row with remote git in flight.
///
/// A checkout row, or a repo row of one checkout, spins while its own
/// checkout runs (`running`). A family repo row (a primary with linked
/// checkouts) spins while any member runs: `running_families` holds the
/// family key (`primary_repo`, else the checkout) of each running
/// checkout, and the family row's `repo` is that key.
fn tree_row_runs_remote(
    row: &VisibleRow,
    running: &HashSet<String>,
    running_families: &HashSet<String>,
) -> bool {
    let Some(repo) = row.repo.as_ref() else {
        return false;
    };
    match row.kind {
        NodeKind::Repo if row.chrome.is_family => running_families.contains(repo),
        NodeKind::Repo | NodeKind::Checkout => running.contains(repo),
        _ => false,
    }
}

/// Paint one tree row. Also returns an [`IconSpan`] for each segment the
/// row paints with a catalog icon tag ([`TextSeg::icon`]) that is still on
/// screen after `col_offset`, the label budget, and the row `width`; and
/// the columns of the workspace PR total, which ends the workspace row
/// flush right (`0` when the row paints none).
fn paint_tree_row(
    row: &VisibleRow,
    width: usize,
    selected: bool,
    focused: bool,
    flash: Option<Color>,
    search_match: bool,
    search: Pill,
    ascii: bool,
    decor: TreeRowDecor,
    palette: Palette,
    col_offset: usize,
) -> (Line<'static>, Vec<IconSpan>, usize) {
    let segs_width =
        |segs: &[TextSeg]| -> usize { segs.iter().map(|s| visible_width(&s.text)).sum() };
    let mut total_cols = 0;
    let mut segs = painted_row_segments(
        row,
        ascii,
        decor.viewed,
        decor.commented,
        decor.resolved,
        decor.pr.as_ref(),
    );
    // The force-push text gives way before the label clips (as in
    // `tree_row_paint_segments`, so the pan limit matches).
    fit_upstream_rewrite_text(
        &mut segs,
        &row.chrome,
        width.saturating_sub(list_prefix_cols(row.depth)),
    );
    // The spinner, then the bars left of it, lead the trailing run: each is
    // a bare token one space before the rest, as the run packs its marks.
    // The run is right-aligned, so they come out of the label budget and
    // the marks keep their columns when a fetch starts or the counts land.
    let live_role = if decor.no_color {
        SegRole::Plain
    } else {
        SegRole::Muted
    };
    let live = [decor.spinner.map(str::to_string), decor.sparkline];
    for text in live.into_iter().flatten() {
        let seg = |text: String, role: SegRole| TextSeg {
            text,
            role,
            hex: None,
            bold: false,
            dim: false,
            icon: None,
        };
        if !segs.trailing.is_empty() {
            segs.trailing
                .insert(0, seg(" ".to_string(), SegRole::Muted));
        }
        segs.trailing.insert(0, seg(text, live_role));
    }
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
        let narrow = width < PR_REF_MIN_TREE_COLS;
        let (summary, cols) = workspace_trailing_fit(&row.chrome, decor.pr_totals, narrow, room);
        if summary.is_empty() && segs.trailing.last().is_some_and(|seg| seg.text == " ") {
            // No summary fits: drop the space after the marks so the last
            // mark stays in the last column.
            segs.trailing.pop();
        }
        total_cols = cols;
        segs.trailing.extend(summary);
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
    (line, icons, total_cols)
}

/// Painted span of every icon segment of a row that
/// [`paint_segmented_row`] paints `width` columns wide at `col_offset`.
///
/// A span covers the glyph columns only: spaces at either end of the
/// segment text stay out. Label icons pan and clip like the label; trailing
/// icons stay flush right and clip from the left with the trailing run. An
/// icon with no cell on screen has no span. The PR badge ref and glyph,
/// two trailing segments of one PR kind one blank column apart, are one
/// span over both.
fn segmented_icon_spans(
    depth: usize,
    segs: &NodeSegments,
    width: usize,
    col_offset: usize,
) -> Vec<IconSpan> {
    let layout = segmented_row_layout(depth, segs, width);
    let (prefix_width, label_budget) = (layout.prefix_width, layout.label_budget);
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
    // Trailing columns are counted on the full run; the first
    // `trailing_skip` of them are cut off at the left.
    let mut col = 0;
    let label_spans = spans.len();
    for seg in &segs.trailing {
        let seg_width = visible_width(&seg.text);
        if let Some(kind) = seg.icon {
            let (lead, core, _) = split_seg_padding(seg);
            let core_start = col + visible_width(&lead.text);
            let core_width = visible_width(&core.text);
            // The run is cut at its left only, so what shows is a suffix
            // of the glyph text, less the spaces a cut leaves at its start
            // (`2 wt` cut after `2` is two columns).
            let cut = layout.trailing_skip.saturating_sub(core_start);
            let shown = slice_cols(&core.text, cut, core_width);
            let shown = visible_width(shown.trim_start_matches(' '));
            if shown > 0 {
                let x = layout.trailing_x + core_start + core_width - shown - layout.trailing_skip;
                match spans[label_spans..].last_mut() {
                    // The PR badge ref and glyph: one icon over both and
                    // the space between.
                    Some((last, last_x, last_w))
                        if kind.is_pull_request()
                            && *last == kind
                            && *last_x + *last_w + 1 == x =>
                    {
                        *last_w = x + shown - *last_x;
                    }
                    _ => spans.push((kind, x, shown)),
                }
            }
        }
        col += seg_width;
    }
    spans
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

/// Columns of a row that [`paint_segmented_row`] paints `width` columns
/// wide.
struct SegmentedRowLayout {
    /// Edge marker, indent, and chevron.
    prefix_width: usize,
    /// Columns the label may paint.
    label_budget: usize,
    /// Trailing run as painted, flush with the right edge.
    trailing: Vec<TextSeg>,
    /// Row column where the painted trailing run starts.
    trailing_x: usize,
    /// Columns cut off the left of the trailing run.
    trailing_skip: usize,
}

/// Lay out a segmented row `width` columns wide.
///
/// The trailing run sits flush right after a one-column pad and the label
/// gets the columns left after the prefix. When the trailing run alone is
/// wider than the room after the prefix, the label gets no columns and the
/// run is cut from the left, so its right edge stays on screen. A wide
/// glyph cut by that edge drops whole, and a space fills its column.
fn segmented_row_layout(depth: usize, segs: &NodeSegments, width: usize) -> SegmentedRowLayout {
    let prefix_width = list_prefix_cols(depth);
    let room = width.saturating_sub(prefix_width);
    let trailing_width: usize = segs.trailing.iter().map(|s| visible_width(&s.text)).sum();
    let pad = usize::from(trailing_width > 0);
    if trailing_width + pad <= room {
        return SegmentedRowLayout {
            prefix_width,
            label_budget: room - trailing_width - pad,
            trailing: segs.trailing.clone(),
            trailing_x: prefix_width + room - trailing_width,
            trailing_skip: 0,
        };
    }
    let trailing = slice_segs(&segs.trailing, trailing_width - room, room);
    let shown: usize = trailing.iter().map(|s| visible_width(&s.text)).sum();
    SegmentedRowLayout {
        prefix_width,
        label_budget: 0,
        trailing,
        trailing_x: prefix_width + room - shown,
        trailing_skip: trailing_width - shown,
    }
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
        slice_segs(&segs[..end], col_offset, label_budget)
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
    let layout = segmented_row_layout(depth, segs, width);

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
    let label = slice_segs(&segs.segments, col_offset, layout.label_budget);
    let label_width: usize = label.iter().map(|s| visible_width(&s.text)).sum();
    for seg in &label {
        spans.push(styled_span(&seg.text, seg_style(seg, palette), bg));
    }
    let gap = layout
        .trailing_x
        .saturating_sub(layout.prefix_width + label_width);
    if gap > 0 {
        spans.push(styled_span(&" ".repeat(gap), Style::default(), bg));
    }
    for seg in &layout.trailing {
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
        SegRole::Plain => Color::Reset,
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
    let (copy, style) = if state.focused_graph_repo().is_some() {
        // NO_COLOR: the terminal foreground, never a fill's colour.
        let style = if state.no_color {
            Style::default().fg(Color::Reset)
        } else {
            Style::default()
        };
        (loading_line(state, LOADING_GRAPH), style)
    } else {
        (
            "focus a repo for the graph, or a file for its diff".to_string(),
            Style::default(),
        )
    };
    frame.render_widget(Paragraph::new(Span::styled(copy, style)), area);
}

/// Folder summary in the diff pane: `dir/` and its totals, then one row per
/// changed file under it. Rows past the pane height scroll (the wheel, the
/// right-pane move keys, and the vertical bar) and long rows pan, on the
/// summary's own viewport and pan ([`AppState::summary_scroll`]).
fn draw_folder_summary(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut AppState,
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
    // The file rows are the list's flat rows, painted and hit like them
    // (marks, eye, flash, icon hits), as a list with its own viewport and
    // pan: no cursor, so the wheel, keys, and bar move the viewport only.
    let rows = state.summary_rows(summary);
    let (pan, col_max) = clamped_list_pan(
        state,
        HScrollbar::Summary,
        body,
        rows.len(),
        state.summary_col_offset(),
        |state, vscroll| state.summary_max_col_in(body.width, vscroll),
    );
    let bars = ListBars::new(body, rows.len(), pan);
    let rows_area = bars.rows_area();
    let height = usize::from(rows_area.height);
    let width = usize::from(rows_area.width);
    let start = state
        .summary_scroll()
        .min(rows.len().saturating_sub(height));
    state.keep_summary_view(start, pan);
    state.layout.summary_list_height = rows_area.height;
    let col_offset = usize::from(pan);
    let search = state.theme.pills().filter;
    let comment_scope = state.commit_file_comment_scope();
    let mut lines = Vec::new();
    let mut hits = Vec::new();
    for (y, row) in (rows_area.y..).zip(rows.iter().skip(start).take(height)) {
        let segs = state.summary_row_paint_segments_in(row, comment_scope);
        for (kind, x, w) in segmented_icon_spans(row.depth, &segs, width, col_offset) {
            hits.push(IconHit {
                y,
                x: rows_area
                    .x
                    .saturating_add(u16::try_from(x).unwrap_or(u16::MAX)),
                width: u16::try_from(w).unwrap_or(u16::MAX),
                kind,
                target: IconTarget::CommitFileRow(row.id.clone()),
            });
        }
        lines.push(paint_segmented_row(
            row.depth,
            false,
            false,
            &segs,
            width,
            false,
            false,
            state.commit_file_flash_color(&row.id),
            false,
            search,
            ascii,
            palette,
            col_offset,
        ));
    }
    state.layout.icon_hits.extend(hits);
    frame.render_widget(Paragraph::new(lines), rows_area);
    let flat = state.background == BackgroundMode::Paint;
    let (vbar, hbar) = bars.paint(frame, start, (pan, col_max), flat);
    state.layout.summary_vbar = vbar;
    state.layout.summary_hbar = hbar;
}

/// Graph list top to paint in the graph pane `area`: `graph_scroll`, or
/// the step of a live Ctrl-d / Ctrl-u glide toward it.
fn graph_glide_scroll(state: &mut AppState, area: Rect) -> u16 {
    if !state.glide_pending(GlidePane::Graph) {
        return state.graph_scroll;
    }
    let target = usize::from(state.graph_scroll);
    let height = usize::from(state.graph_chrome_in(area.height, area.width).list_height);
    let cursor = state.graph_cursor_line().unwrap_or(target);
    let now = state.frame_now;
    let top = state.glide_start(GlidePane::Graph, target, cursor, height, now);
    u16::try_from(top).unwrap_or(state.graph_scroll)
}

fn draw_graph(frame: &mut Frame<'_>, area: Rect, state: &mut AppState, col_offset: u16) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    // The width the selected row's meta line wraps at between frames.
    state.layout.graph_width = area.width;
    // A footer that grew since the last paint (more wrapped meta rows)
    // must not hide the cursor row under it.
    let list_height = state.graph_chrome_in(area.height, area.width).list_height;
    state.keep_graph_cursor_in_view(state.layout.graph_list_height, list_height);
    state.layout.graph_list_height = list_height;
    // The painted top: `graph_scroll` itself stays the jump top.
    let scroll = graph_glide_scroll(state, area);
    if state.graph.is_some() {
        state.layout.graph_painted_top = Some(scroll);
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
    let chip = graph_pr_chip(state, unix_now());
    let spans = GraphWidget::new(model)
        .ascii(state.ascii)
        .selected(Some(state.graph_cursor))
        .cursor_bar(graph_focused)
        .scroll(scroll)
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
        .label_palette(graph_label_palette(pal))
        .row_badges(&badges)
        .header_chip(chip.as_ref().map_or(&[], |chip| chip.candidates.as_slice()))
        .render_with_icon_spans(area, frame.buffer_mut());
    let row_of = |span: &workspace_status_graph::IconSpan| rows.get(span.row_index?);
    // Commit meta text opens a commit popover of the graph's checkout; no
    // identity, no hit.
    let graph_repo = state.graph_identity.as_ref().map(|(repo, _)| repo.as_str());
    let hits: Vec<IconHit> = spans
        .iter()
        .filter_map(|span| {
            if span.kind == GraphIconKind::HeaderChip {
                // The `<glyph> <ref>` head: the PR icon, linked like a badge.
                let chip = chip.as_ref()?;
                return Some(IconHit {
                    y: span.y,
                    x: span.x,
                    width: span.width.min(chip.head_width),
                    kind: chip.mark.kind(),
                    target: IconTarget::PullRequest(chip.checkout.clone()),
                });
            }
            let (kind, target) = match span.kind {
                GraphIconKind::Badge => badge_rows
                    .iter()
                    .find(|row| Some(row.0) == span.row_index)
                    .map(|(_, repo, mark, _, _)| {
                        (mark.kind(), IconTarget::PullRequest(repo.clone()))
                    })?,
                GraphIconKind::SyncHeader => (
                    graph_sync_kind(model.sync.as_ref()?.status),
                    IconTarget::GraphSync,
                ),
                GraphIconKind::MoreBelow => (
                    IconKind::GraphMoreBelow,
                    IconTarget::GraphMoreLines(graph_row_id(row_of(span)?)),
                ),
                GraphIconKind::CommitHash
                | GraphIconKind::CommitDate
                | GraphIconKind::CommitAuthor => {
                    commit_meta_icon(span.kind, span.part, row_of(span)?, graph_repo?)?
                }
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
    let links = graph_commit_links(frame.buffer_mut(), area, state, &rows, &spans);
    state.layout.icon_hits.extend(hits);
    state.layout.links.extend(links);
    record_graph_scrollbar(state, area, col_offset, &badges);
    record_commit_msg_rule(state, area);
    if state.background == BackgroundMode::Paint {
        bleed_graph_scrollbar_column(frame, &state.layout);
    }
}

/// Graph label colours from the theme `pal`: the graph rows and every
/// commit-message footer (graph selection footer, commit-files footer)
/// paint their subject, meta, and ref chips with these.
fn graph_label_palette(pal: Palette) -> GraphLabelPalette {
    GraphLabelPalette {
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
    }
}

/// One link per painted graph commit row, over its short hash, to the
/// commit page on the forge of the graph's checkout. The selected commit's
/// hash in the selection footer links too.
///
/// A commit row is found by its node icon span. Its hash (the
/// right-anchored meta) is the rightmost `short_id` painted on the node
/// line or on a spacer line under it, above the next row's first icon and
/// the footer. No remote known for the checkout gives no link.
fn graph_commit_links(
    buf: &Buffer,
    area: Rect,
    state: &AppState,
    rows: &[GraphRow],
    spans: &[workspace_status_graph::IconSpan],
) -> Vec<LinkHit> {
    let Some((checkout, _)) = state.graph_identity.as_ref() else {
        return Vec::new();
    };
    let checkout = std::path::Path::new(checkout);
    let chrome = state.graph_chrome_in(area.height, area.width);
    let list_bottom = area
        .y
        .saturating_add(u16::from(chrome.header))
        .saturating_add(chrome.list_height)
        .min(area.bottom());
    let hash_link = |id: &str, lines: std::ops::Range<u16>| {
        let url = state.commit_link(checkout, id)?;
        lines.into_iter().find_map(|y| {
            painted_text_link(buf, y, area.x, area.right(), short_id(id), url.clone())
        })
    };
    let mut links: Vec<LinkHit> = spans
        .iter()
        .filter(|span| {
            matches!(
                span.kind,
                GraphIconKind::CommitNode | GraphIconKind::HeadNode
            )
        })
        .filter_map(|span| {
            let index = span.row_index?;
            let Some(GraphRow::Commit { commit, .. }) = rows.get(index) else {
                return None;
            };
            let next_row = spans
                .iter()
                .filter(|other| other.y > span.y && other.row_index != Some(index))
                .map(|other| other.y)
                .min()
                .unwrap_or(list_bottom)
                .min(list_bottom);
            hash_link(&commit.id, span.y..next_row)
        })
        .collect();
    if chrome.footer && chrome.footer_height > 0 {
        if let Some(GraphRow::Commit { commit, .. }) = rows.get(state.graph_cursor) {
            let top = area.bottom().saturating_sub(chrome.footer_height);
            links.extend(hash_link(&commit.id, top..area.bottom()));
        }
    }
    links
}

/// Flat graph list, before [`bleed_flat_pad`]: the widget paints rows one
/// column short of a shown vertical scrollbar, so each list row's
/// scrollbar cell takes the bg of the cell left of it
/// ([`bleed_vscrollbar_column`]).
fn bleed_graph_scrollbar_column(frame: &mut Frame<'_>, layout: &LayoutHit) {
    let bar = VScrollbarLayout {
        x: layout.graph_scrollbar_x,
        y: layout.graph_scrollbar_y,
        height: layout.graph_scrollbar_height,
        ..VScrollbarLayout::default()
    };
    bleed_vscrollbar_column(frame.buffer_mut(), bar);
}

/// Flat pane, before [`bleed_flat_pad`]: each row's cell of the painted
/// vertical scrollbar `bar` takes the bg of the cell left of it, so
/// selection, flash, and search backgrounds reach the pad beside a list
/// whose rows stop one column short of the bar. The scrollbar glyph stays.
fn bleed_vscrollbar_column(buf: &mut Buffer, bar: VScrollbarLayout) {
    let Some(x) = bar.x.filter(|&x| x > 0) else {
        return;
    };
    for y in bar.y..bar.y.saturating_add(bar.height) {
        let bg = buf[(x - 1, y)].bg;
        buf[(x, y)].set_bg(bg);
    }
}

/// One graph worktree row's PR badge: visible row index, checkout,
/// mark, glyph, and colour.
type GraphPrBadge = (usize, PathBuf, PrMark, &'static str, Color);

/// [`AppState::graph_pr_badges`] with the glyph and colour to paint: the
/// state colour, or muted when the badge is stale; the failure mark `!` in
/// the deleted colour.
fn graph_pr_badges(state: &AppState) -> Vec<GraphPrBadge> {
    let palette = state.theme.palette();
    let now = Instant::now();
    state
        .graph_pr_badges()
        .into_iter()
        .map(|(index, path, mark)| {
            let glyph = mark.kind().glyph(state.ascii);
            let role = mark.role(state.pr_is_stale(&path, now));
            (index, path, mark, glyph, seg_role_color(role, palette))
        })
        .collect()
}

/// The graph header chip: the checkout it describes, its mark, the columns
/// of its `<glyph> <ref>` head (the PR icon hit and link), and the
/// candidates for [`GraphWidget::header_chip`].
struct GraphPrChip {
    checkout: PathBuf,
    mark: PrMark,
    head_width: u16,
    candidates: Vec<Line<'static>>,
}

/// Label of the header chip of a failed lookup with no earlier answer.
const GRAPH_CHIP_LOOKUP_FAILED: &str = "lookup failed";

/// The header chip of the graph checkout's PR ([`AppState::graph_header_pr`]),
/// or `None` with no PR.
///
/// Candidates, widest first: `<glyph> <ref> <state label>  ▸ <next
/// action>` (merged: `✓ !12 merged · 3h ago`), then without the next action
/// (the age), then `<glyph> <ref>`. The glyph and the label paint in the
/// state colour, the ref in the repo colour, `▸` dim, the next action bold.
/// A stale PR paints every part muted. ASCII mode uses `>` for `▸`.
///
/// A failed lookup with no earlier answer
/// ([`AppState::graph_header_lookup_failed`]) paints the failure mark:
/// `! lookup failed`, then `!` alone, bold in the deleted colour, no ref.
fn graph_pr_chip(state: &AppState, now_unix: i64) -> Option<GraphPrChip> {
    let Some((checkout, pr)) = state.graph_header_pr() else {
        return graph_lookup_failed_chip(state);
    };
    let palette = state.theme.palette();
    let stale = state.pr_is_stale(&checkout, Instant::now());
    let glyph = pr_badge_kind(pr.state).glyph(state.ascii);
    let muted = Style::default().fg(palette.muted);
    let state_role = pr_badge_role(pr.state, stale);
    let state_style = Style::default().fg(seg_role_color(state_role, palette));
    let bold = state_style.add_modifier(Modifier::BOLD);
    // The ref takes the repo colour; a stale chip steps it back with the
    // state colour.
    let ref_role = if stale { state_role } else { SegRole::Repo };
    let ref_text = pr.ref_text();
    let head = vec![
        Span::styled(glyph.to_string(), bold),
        Span::raw(" "),
        Span::styled(
            ref_text.clone(),
            Style::default().fg(seg_role_color(ref_role, palette)),
        ),
    ];
    let labelled = [
        head.clone(),
        vec![Span::raw(" "), Span::styled(pr.state.label(), state_style)],
    ]
    .concat();
    let tail = match pr.next_action(state.pr_stack_base(&checkout)) {
        Some(next) => Some(vec![
            Span::raw("  "),
            Span::styled(
                if state.ascii { ">" } else { "▸" },
                muted.add_modifier(Modifier::DIM),
            ),
            Span::raw(" "),
            Span::styled(next.text(), bold),
        ]),
        None => pr.updated_at.map(|unix| {
            vec![
                Span::styled(" · ", muted.add_modifier(Modifier::DIM)),
                Span::styled(relative_age(now_unix, unix), muted),
            ]
        }),
    };
    let mut candidates = Vec::new();
    if let Some(tail) = tail {
        candidates.push(Line::from([labelled.clone(), tail].concat()));
    }
    candidates.push(Line::from(labelled));
    candidates.push(Line::from(head));
    Some(GraphPrChip {
        checkout,
        mark: PrMark::State(pr.state),
        head_width: u16::try_from(2 + UnicodeWidthStr::width(ref_text.as_str()))
            .unwrap_or(u16::MAX),
        candidates,
    })
}

/// The header chip of a failed lookup with no earlier answer: `! lookup
/// failed`, then `!`; the mark bold and the label in the deleted colour.
/// Its head (the icon hit) is the mark alone: there is no ref to link.
fn graph_lookup_failed_chip(state: &AppState) -> Option<GraphPrChip> {
    let checkout = state.graph_header_lookup_failed()?;
    let mark = PrMark::LookupFailed;
    let style = Style::default().fg(seg_role_color(mark.role(false), state.theme.palette()));
    let head = Span::styled(
        mark.kind().glyph(state.ascii),
        style.add_modifier(Modifier::BOLD),
    );
    Some(GraphPrChip {
        checkout,
        mark,
        head_width: 1,
        candidates: vec![
            Line::from(vec![
                head.clone(),
                Span::raw(" "),
                Span::styled(GRAPH_CHIP_LOOKUP_FAILED, style),
            ]),
            Line::from(head),
        ],
    })
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

/// Record the expanded graph commit-message footer's top rule, the
/// [`ResizeEdge::CommitMsg`] edge, and the most message rows the pane
/// shows, so a press on the rule arms its drag. A collapsed or unpainted
/// footer records no rule.
fn record_commit_msg_rule(state: &mut AppState, area: Rect) {
    if state.graph.is_none() || !state.commit_msg_expand || area.width == 0 {
        return;
    }
    let chrome = state.graph_chrome_in(area.height, area.width);
    if !chrome.footer || chrome.footer_height == 0 {
        return;
    }
    state.layout.commit_msg_rule_y = Some(area.bottom().saturating_sub(chrome.footer_height));
    state.layout.commit_msg_rule_x = area.x;
    state.layout.commit_msg_rule_width = area.width;
    state.layout.commit_msg_lines_fit = commit_msg_lines_fit(area.height);
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
    let chrome = state.graph_chrome_in(area.height, area.width);
    let Some(model) = state.graph.as_ref() else {
        return;
    };
    // The widget already painted this frame; count without a second paint.
    let content_len = painted_line_count(model);
    state.layout.graph_content_len = content_len;
    // The painted top, so the bar matches the rows during a glide.
    let vscroll = graph_vscroll_visible(content_len, chrome.list_height, state.graph_painted_top());
    let hscroll = graph_hscroll_visible(col_offset);
    let list_top = area.y.saturating_add(u16::from(chrome.header));
    let list_height = chrome.list_height;
    let width = usize::from(area.width);
    let footer_scroll_max = state
        .graph_selection_footer(width)
        .scroll_max(width, chrome.footer_body_height());
    if chrome.footer && footer_scroll_max > 0 {
        let bottom = area.y.saturating_add(area.height);
        state.layout.graph_footer_y = Some(bottom.saturating_sub(chrome.footer_height));
        state.layout.graph_footer_x = area.x;
        state.layout.graph_footer_width = area.width;
        state.layout.graph_footer_height = chrome.footer_height;
        state.layout.graph_footer_message_rows = chrome.message_rows();
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
/// ([`AppState::commit_detail_footer_request`], fixed whatever the message)
/// and a meta line of `meta_rows` wrapped rows
/// ([`AppState::commit_detail_footer_meta_rows`]).
///
/// The footer is the [`FOOTER_RULE_ROWS`] rule row, then the body, so it
/// asks for `body_len + 1` rows. From 3 rows up the graph selection
/// footer's budget sizes it ([`graph_chrome_budget_for_meta`]): the message
/// rows take at most half the pane, the extra meta rows add on top, and the
/// list keeps at least one row, so one request and meta row count give the
/// same footer height on both footers. A pane of 2 rows keeps one list row
/// and the rule only; 1 row is the rule only. Draw and layout both size the
/// footer here so the list rows and click mapping agree.
fn commit_detail_footer_height(body_len: usize, meta_rows: usize, pane_h: u16) -> u16 {
    let want = body_len
        .min(usize::from(u16::MAX - FOOTER_RULE_ROWS))
        .saturating_add(usize::from(FOOTER_RULE_ROWS)) as u16;
    if pane_h >= 3 {
        let meta_rows = u16::try_from(meta_rows).unwrap_or(u16::MAX);
        return graph_chrome_budget_for_meta(pane_h, false, want, meta_rows).footer_height;
    }
    want.min(pane_h.min(1))
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

/// Commit files beside the file diff (depth 2, or a Diff commit vs parent
/// compare tab; left pane): the file list on top, the commit's footer
/// pinned to the bottom rows ([`draw_commit_footer`], the graph selection
/// footer's rows for that commit, [`AppState::commit_footer_source`]).
///
/// `col_offset` is the horizontal pan of the file list.
fn draw_commit_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut AppState,
    cursor: usize,
    col_offset: usize,
) {
    let footer_h = commit_detail_footer_height(
        state.commit_detail_footer_request(),
        state.commit_detail_footer_meta_rows(usize::from(area.width)),
        area.height,
    );
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
    let (checkout, commit_id) = match state.commit_footer_source() {
        Some((repo, source)) => match source.as_ref() {
            super::drill::CommitFileSource::Commit { commit_id } => {
                (Some(repo.to_string()), Some(commit_id.clone()))
            }
            _ => (Some(repo.to_string()), None),
        },
        None => (None, None),
    };
    let footer = CommitFooter {
        row: state.commit_detail_graph_row(),
        fallback: state.commit_detail_fallback_name(),
        msg_identity: state.commit_detail_msg_identity(),
        checkout,
        commit_id,
    };
    let footer_area = Rect {
        y: area.y.saturating_add(list_h),
        height: footer_h,
        ..area
    };
    draw_commit_footer(frame, footer_area, area.height, state, &footer);
}

/// The commit a commit-files footer describes: the input of
/// [`draw_commit_footer`].
struct CommitFooter {
    /// Graph row whose message and meta the footer paints
    /// ([`AppState::commit_footer_parts`]); its ref chips' popovers resolve
    /// against it.
    row: Option<GraphRow>,
    /// Name the footer shows when [`Self::row`] is `None` (the drilled
    /// source a reload dropped from the graph); `None` too paints the
    /// no-selection footer.
    fallback: Option<String>,
    /// [`AppState::commit_msg_scroll`] key of the message, and the target
    /// of the more-below hint. `None` when the row has no message to
    /// scroll.
    msg_identity: Option<String>,
    /// Checkout whose forge the short hash links to.
    checkout: Option<String>,
    /// Full id of the commit whose short hash on the meta row links to its
    /// forge page, and in the fallback footer (no [`Self::row`]) opens its
    /// hash card. `None` for a stash, the working tree, or a range.
    commit_id: Option<String>,
}

/// Paint a commit-message footer under a commit-files list into `area`
/// (its rule row, then the body) of a pane `pane_h` rows tall, and record
/// its hits.
///
/// The body is the graph selection footer's: the same lines
/// ([`AppState::commit_footer_parts`]) through the same painter
/// ([`paint_selection_footer`]), so message rows come first in the subject
/// colour and the meta line (coloured ref chips, hash, `parent <id>`,
/// author, date), wrapped at the footer width, is pinned on the bottom
/// rows. Records the wheel box of an
/// overflowing message, the more-below hint, the ref chip hits (their
/// popovers), the commit popover hits of the hash, parent ids, author, and
/// date ([`commit_meta_icon`]), the hash link (in the fallback footer of a
/// commit with no graph row, also the hash's commit popover hit), and the
/// expanded footer's top rule ([`record_commit_detail_msg_rule`]), so a
/// drag on it resizes the same [`AppState::commit_msg_lines`] the graph
/// footer rule does.
fn draw_commit_footer(
    frame: &mut Frame<'_>,
    area: Rect,
    pane_h: u16,
    state: &mut AppState,
    footer: &CommitFooter,
) {
    let palette = state.theme.palette();
    frame.render_widget(
        Paragraph::new(footer_rule_line(area.width, state.ascii, palette)),
        Rect { height: 1, ..area },
    );
    record_commit_detail_msg_rule(state, area, pane_h);
    let body_h = area.height.saturating_sub(FOOTER_RULE_ROWS);
    if body_h == 0 {
        return;
    }
    let body = Rect {
        y: area.y.saturating_add(FOOTER_RULE_ROWS),
        height: body_h,
        ..area
    };
    let width = usize::from(area.width);
    let content = state.commit_footer_parts(footer.row.as_ref(), footer.fallback.as_deref(), width);
    let scroll_max = content.scroll_max(width, body_h);
    // The wrapped meta rows pinned at the bottom of the body.
    let meta_h = u16::try_from(content.meta_rows(width))
        .unwrap_or(u16::MAX)
        .min(body_h);
    let glyphs = if state.ascii { &ASCII } else { &UNICODE };
    let spans = paint_selection_footer(
        frame.buffer_mut(),
        body,
        &content,
        state.commit_msg_scroll_of(footer.msg_identity.as_deref()),
        glyphs.more_below,
        Some(graph_label_palette(palette)),
        None,
    );
    if scroll_max > 0 {
        state.layout.commit_detail_footer_y = Some(area.y);
        state.layout.commit_detail_footer_x = area.x;
        state.layout.commit_detail_footer_width = area.width;
        state.layout.commit_detail_footer_height = area.height;
        state.layout.commit_detail_footer_message_rows = body_h - meta_h;
        state.layout.commit_detail_footer_scroll_max = scroll_max;
    }
    // The hint hovers and pins like the graph footer's; the chips open the
    // graph chip popovers of the row, and the meta row's hash, parent ids,
    // author, and date open commit popovers on the footer's checkout.
    let hits = spans.iter().filter_map(|span| {
        let row = footer.row.as_ref();
        let (kind, target) = match span.kind {
            GraphIconKind::MoreBelow => (
                IconKind::GraphMoreBelow,
                IconTarget::CommitDetailMoreLines(footer.msg_identity.clone()?),
            ),
            GraphIconKind::CommitHash | GraphIconKind::CommitDate | GraphIconKind::CommitAuthor => {
                commit_meta_icon(span.kind, span.part, row?, footer.checkout.as_deref()?)?
            }
            kind => graph_icon(kind, span.part, span.target.as_ref(), row?)?,
        };
        Some(IconHit {
            y: span.y,
            x: span.x,
            width: span.width,
            kind,
            target,
        })
    });
    state.layout.icon_hits.extend(hits);
    // The meta rows (the bottom rows) hold the commit's short hash, also in
    // the fallback footer: link it to the commit page. The hash sits on the
    // first meta row that paints it.
    let (Some(commit_id), Some(checkout)) = (&footer.commit_id, &footer.checkout) else {
        return;
    };
    let Some((y, (x, width))) = (body.bottom() - meta_h..body.bottom()).find_map(|y| {
        painted_text_span(
            frame.buffer_mut(),
            y,
            body.x,
            body.right(),
            short_id(commit_id),
        )
        .map(|span| (y, span))
    }) else {
        return;
    };
    if let Some(url) = state.commit_link(std::path::Path::new(checkout), commit_id) {
        state.layout.links.push(LinkHit { y, x, width, url });
    }
    // The fallback footer (no graph row: a reload dropped the commit, after
    // an amend say) has no meta spans, so the loop above records no hash
    // hit. The commit still exists as an object: its hash opens the hash
    // card, which loads off the event thread.
    if footer.row.is_none() {
        state.layout.icon_hits.push(IconHit {
            y,
            x,
            width,
            kind: IconKind::CommitHash,
            target: IconTarget::Commit {
                repo: checkout.clone(),
                sha: commit_id.clone(),
            },
        });
    }
}

/// Record the expanded depth-2 commit-files footer's top rule (the first
/// row of `footer`), the [`ResizeEdge::CommitMsg`] edge, and the most
/// message rows a pane `pane_h` rows tall shows
/// ([`commit_msg_lines_fit`], the graph footer's fit: both footers share
/// one height budget), like [`record_commit_msg_rule`] for the graph
/// footer. A collapsed footer and a footer with no body row under its rule
/// record no rule. Unlike the graph footer it needs no loaded graph: it
/// paints from the drill's or compare tab's own commit, and a depth-0 file
/// diff drops the graph before a blame action opens a Diff commit vs
/// parent tab.
fn record_commit_detail_msg_rule(state: &mut AppState, footer: Rect, pane_h: u16) {
    if !state.commit_msg_expand || footer.width == 0 {
        return;
    }
    if footer.height <= FOOTER_RULE_ROWS {
        return;
    }
    state.layout.commit_msg_rule_y = Some(footer.y);
    state.layout.commit_msg_rule_x = footer.x;
    state.layout.commit_msg_rule_width = footer.width;
    state.layout.commit_msg_lines_fit = commit_msg_lines_fit(pane_h);
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
            let (copy, fg) = if tab.loading {
                let text = spinner_text(
                    state.anim_epoch,
                    state.ascii,
                    &mut state.layout,
                    LOADING_FILES,
                );
                (text, loading_fg(state.no_color, palette))
            } else {
                (tab.empty_files_copy().to_string(), palette.muted)
            };
            let line = Span::styled(copy, Style::default().fg(fg));
            frame.render_widget(Paragraph::new(line), area);
            return;
        }
        // Commit-file lists have no row filter: empty after the load means
        // the source has no files.
        let (copy, fg) = if state.commit_files_loading {
            (
                loading_line(state, LOADING_FILES),
                loading_fg(state.no_color, palette),
            )
        } else {
            let copy = match state.commit_drill_source().map(|(_, source)| source) {
                Some(super::drill::CommitFileSource::Stash { .. }) => NO_FILES_IN_STASH,
                Some(super::drill::CommitFileSource::Worktree) => NO_FILES_IN_WORKTREE,
                _ => NO_FILES_IN_COMMIT,
            };
            (copy.to_string(), palette.muted)
        };
        let line = Span::styled(copy, Style::default().fg(fg));
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    let (pan, col_max) = clamped_list_pan(
        state,
        HScrollbar::CommitFiles,
        area,
        rows.len(),
        u16::try_from(col_offset).unwrap_or(u16::MAX),
        |state, vscroll| state.commit_file_max_col_in(area.width, vscroll),
    );
    let col_offset = usize::from(pan);
    let bars = ListBars::new(area, rows.len(), pan);
    let rows_area = bars.rows_area();
    let height = rows_area.height as usize;
    let width = rows_area.width as usize;
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
    // The rows `n` / `N` step to and count: highlight and steps agree.
    let match_ids = state.commit_file_search_match_ids();
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
        let search_match = match_ids.contains(&row.id);
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
    frame.render_widget(Paragraph::new(lines), rows_area);
    let flat = state.background == BackgroundMode::Paint;
    let (vbar, hbar) = bars.paint(frame, start, (pan, col_max), flat);
    state.layout.files_vbar = vbar;
    state.layout.files_hbar = hbar;
}

/// Logical rows of a wrapped diff, counted back from `cursor` (included),
/// whose visual rows fit in `view_h`. At least 1.
fn wrap_rows_ending_at(heights: &[usize], cursor: usize, view_h: usize) -> usize {
    let Some(&own) = heights.get(cursor) else {
        return 1;
    };
    let mut used = own.max(1);
    let mut count = 1;
    for h in heights[..cursor].iter().rev() {
        used += (*h).max(1);
        if used > view_h {
            break;
        }
        count += 1;
    }
    count
}

/// True for a whole 40- or 64-hex object id.
fn is_full_object_id(rev: &str) -> bool {
    matches!(rev.len(), 40 | 64) && rev.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Commit popover hits ([`IconKind::CommitHash`]) on the commit tokens of
/// the compare tab's range in the diff pane header rows `header`.
///
/// A commit-vs-working-tree tab (`abc1234 ↔ working tree`) records its
/// base. A range tab (`abc1234^...abc1234`) records its base (a
/// `<full id>^` base names the loaded [`CommitFileSource::Compare`]
/// `base_tip`, so none before the source loads; a full-id base names
/// itself) and a full-id head. Branch names and `HEAD` record nothing.
/// The range is found in the painted rows; when the header wrapped it over
/// two rows, a token cut by the break records nothing.
///
/// [`CommitFileSource::Compare`]: super::drill::CommitFileSource::Compare
fn compare_header_hits(buf: &Buffer, header: Rect, state: &AppState) -> Vec<IconHit> {
    let Some(tab) = state.tabs.active_compare() else {
        return Vec::new();
    };
    let range = tab.range_header();
    let base = short_rev(&tab.base_ref);
    let base_sha = if is_full_object_id(&tab.base_ref) {
        Some(tab.base_ref.clone())
    } else if tab.worktree_file.is_none()
        && tab
            .base_ref
            .strip_suffix('^')
            .is_some_and(is_full_object_id)
    {
        match &tab.source {
            Some(super::drill::CommitFileSource::Compare { base_tip, .. }) => {
                Some(base_tip.clone())
            }
            _ => None,
        }
    } else {
        None
    };
    let head_sha = (tab.worktree_file.is_none() && is_full_object_id(&tab.head_ref))
        .then(|| tab.head_ref.clone());
    // Byte offset and length of each token in `range` (the token is ASCII,
    // so its length is also its width), with the commit it names.
    let mut tokens: Vec<(usize, usize, String)> = Vec::new();
    if let Some(sha) = base_sha.filter(|sha| !sha.is_empty()) {
        tokens.push((0, base.len(), sha));
    }
    if let Some(sha) = head_sha {
        tokens.push((
            base.len() + "...".len(),
            short_rev(&tab.head_ref).len(),
            sha,
        ));
    }
    if tokens.is_empty() {
        return Vec::new();
    }
    let hit = |y: u16, x: u16, width: u16, sha: &str| IconHit {
        y,
        x,
        width,
        kind: IconKind::CommitHash,
        target: IconTarget::Commit {
            repo: tab.checkout_path.clone(),
            sha: sha.to_string(),
        },
    };
    let rows = header.top()..header.bottom();
    for y in rows.clone() {
        if let Some((x, _)) = painted_text_span(buf, y, header.left(), header.right(), &range) {
            return tokens
                .iter()
                .map(|(at, len, sha)| {
                    let col = range[..*at].width() as u16;
                    hit(y, x + col, *len as u16, sha)
                })
                .collect();
        }
    }
    // Wrapped: the range ends row `y` and goes on at the start of row
    // `y + 1`; only the tokens wholly on one side record a hit.
    let painted: Vec<_> = rows
        .filter_map(|y| Some((y, painted_row_text(buf, y, header.left(), header.right())?)))
        .collect();
    for pair in painted.windows(2) {
        let [(y0, (text0, starts0)), (y1, (text1, starts1))] = pair else {
            continue;
        };
        let Some(cut) = (1..range.len()).find(|&cut| {
            range.is_char_boundary(cut)
                && text0.ends_with(&range[..cut])
                && text1.starts_with(&range[cut..])
        }) else {
            continue;
        };
        let head0 = text0.len() - cut;
        return tokens
            .iter()
            .filter_map(|(at, len, sha)| {
                let (y, span) = if at + len <= cut {
                    (
                        *y0,
                        painted_byte_span(starts0, head0 + at, head0 + at + len)?,
                    )
                } else if *at >= cut {
                    (*y1, painted_byte_span(starts1, at - cut, at - cut + len)?)
                } else {
                    return None;
                };
                Some(hit(y, span.0, span.1, sha))
            })
            .collect();
    }
    Vec::new()
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
    // The popover paint reads the cursor row's marks and the Comment
    // gate's line span from here, not from a second row build.
    state.prime_cursor_comment_lines(&rows);
    state.layout.cursor_line_marks = rows
        .get(state.diff_cursor)
        .map(|row| mark_targets(state.diff_row_comment_marks(row, split)))
        .unwrap_or_default();
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
    // Viewport start (a logical row), the logical rows that fit on screen
    // ending at the cursor (what a glide keeps the cursor inside), and the
    // visual rows the whole diff paints.
    let (target, cursor_span, content_rows) = if wrap {
        let heights = diff_wrap_row_heights(
            &rows,
            diff_row_content_width(line_width as usize) as u16,
            gutter_with_mark,
            split,
            state.diff_split_fraction,
        );
        let start = wrap_viewport_start(&heights, state.diff_cursor, line_h);
        (
            start,
            wrap_rows_ending_at(&heights, state.diff_cursor, line_h),
            heights.iter().map(|h| (*h).max(1)).sum::<usize>(),
        )
    } else {
        (
            visible_window(rows.len(), state.diff_cursor, line_h).0,
            line_h,
            rows.len(),
        )
    };
    let now = state.frame_now;
    let start = state.glide_start(GlidePane::Diff, target, state.diff_cursor, cursor_span, now);
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
    let title_chunks: Vec<String> = wrap_cols(title, area.width as usize)
        .into_iter()
        .take(header_h as usize)
        .collect();
    let mut header_lines: Vec<Line> = title_chunks
        .into_iter()
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
    if state.is_compare_tab() {
        let hits = compare_header_hits(frame.buffer_mut(), header_area, state);
        state.layout.icon_hits.extend(hits);
    }
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
            color = loading_fg(state.no_color, palette);
            loading_line(state, LOADING_FILE)
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
                cache_id: diff_syntax_cache_id(state, split),
            },
            &rows,
            skip..logical_end,
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
    // Comment marks sit in the first column of each cell's gutter: past
    // the cursor bar, and for the right cell of a side-by-side row past
    // the left cell and the RULE. The gutter does not pan.
    let left_cell_x = body.x.saturating_add(DIFF_CURSOR_BAR_COLS);
    let cell_xs = [
        left_cell_x,
        left_cell_x
            .saturating_add(
                side_by_side_column_widths(content_w, state.diff_split_fraction).left_width,
            )
            .saturating_add(1),
    ];
    let mut mark_hits = Vec::new();
    let mut painted: Vec<Line> = Vec::new();
    // Screen rows of the focused cursor row: its blame note paints there.
    let mut cursor_ys: Vec<u16> = Vec::new();
    let mut i = skip;
    while painted.len() < line_h && i < rows.len() {
        // A row's marks paint on its first visual line only.
        let y = body.y.saturating_add(painted.len() as u16);
        mark_hits.extend(diff_row_mark_hits(
            state,
            &rows[i],
            split,
            cell_xs,
            y,
            body.x.saturating_add(line_width),
        ));
        let search_match = search_hits.contains(&i);
        let blamed_row = i == state.diff_cursor && state.focus == FocusPane::Right;
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
            if blamed_row {
                cursor_ys.push(body.y.saturating_add(painted.len() as u16));
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
    if !cursor_ys.is_empty() {
        if let (Some((note, _)), Some((repo, blame))) =
            (state.painted_line_annotation(), state.painted_line_blame())
        {
            let hits = blame_note_hits(
                frame.buffer_mut(),
                &cursor_ys,
                lines_area.left(),
                lines_area.right(),
                &note,
                &blame,
                &repo,
            );
            state.layout.icon_hits.extend(hits);
        }
    }
    state.layout.icon_hits.extend(mark_hits);
    // Only commented code is underlined in the diff rows: those runs get
    // the curly upgrade after the draw.
    let curls = underlined_runs(frame.buffer_mut(), lines_area, palette.comment_underline());
    state.layout.undercurls.extend(curls);
    if state.background == BackgroundMode::Paint {
        bleed_diff_row_edges(frame, lines_area, area.right());
    }
    let buf = frame.buffer_mut();
    if vscroll && body.height > 0 && area.width > 0 {
        let track = Rect {
            x: area.x.saturating_add(area.width.saturating_sub(1)),
            y: body.y,
            width: 1,
            height: body.height,
        };
        // The top the last row's cursor gives: a track click on the last
        // row, or a drag to the end, shows the last row.
        let scroll_max = wrap_viewport_start(&paint_heights, content_len - 1, line_h);
        state.layout.diff_vbar = paint_vscrollbar(buf, track, content_len, skip, scroll_max);
    }
    let col_max = if hscroll {
        state.diff_pan_max_for(&rows)
    } else {
        0
    };
    if col_max > 0 && body.height > 0 && area.width > 0 {
        let track = Rect {
            x: area.x,
            y: body.y.saturating_add(body.height.saturating_sub(1)),
            width: area.width.saturating_sub(v_cols).max(1),
            height: 1,
        };
        let hbar = paint_hscrollbar(buf, track, col_max, state.diff_col_offset);
        state.layout.diff_hscrollbar_y = hbar.y;
        state.layout.diff_hscrollbar_x = hbar.x;
        state.layout.diff_hscrollbar_width = hbar.width;
        state.layout.diff_col_max = hbar.col_max;
    }
}

/// The targets of comment `marks`, for [`LayoutHit::cursor_line_marks`].
fn mark_targets(marks: Vec<(IconKind, IconTarget)>) -> Vec<IconTarget> {
    marks.into_iter().map(|(_, target)| target).collect()
}

/// Icon hits of the comment marks diff row `row` paints on screen row `y`:
/// the mark glyph of each gutter cell whose line has a comment
/// ([`AppState::diff_line_comment`]), the left cell's at `cell_xs[0]` and a
/// side-by-side right cell's at `cell_xs[1]`. A mark that would end past
/// `right` (the rows' right edge) records nothing.
fn diff_row_mark_hits(
    state: &AppState,
    row: &DiffRow,
    split: bool,
    cell_xs: [u16; 2],
    y: u16,
    right: u16,
) -> Vec<IconHit> {
    diff_row_gutter_cells(row, split)
        .into_iter()
        .zip(cell_xs)
        .filter_map(|(cell, x)| {
            let line = cell.line_no?;
            let (kind, target) = state.line_comment_mark(line, state.diff_line_comment(line))?;
            line_mark_hit(kind, target, x, y, right, state.ascii)
        })
        .collect()
}

/// The icon hit of a line gutter comment mark of `kind` at (`x`, `y`): the
/// glyph's columns only. `None` when the glyph would end past `right`.
fn line_mark_hit(
    kind: IconKind,
    target: IconTarget,
    x: u16,
    y: u16,
    right: u16,
    ascii: bool,
) -> Option<IconHit> {
    let width = u16::try_from(visible_width(kind.glyph(ascii))).ok()?;
    (x.saturating_add(width) <= right).then_some(IconHit {
        y,
        x,
        width,
        kind,
        target,
    })
}

/// Paint a 1-column vertical scrollbar in `track` for `content_len` lines
/// with the viewport at line `scroll`, and return its hit geometry for
/// [`super::split::hit_vscrollbar`]. The bar is the ratatui `Scrollbar`
/// with no end symbols, the same math as the graph widget's bar, so
/// [`workspace_status_graph::graph_scrollbar_thumb`] finds its thumb.
/// `scroll_max` is the largest viewport top, where a drag to the end lands.
fn paint_vscrollbar(
    buf: &mut Buffer,
    track: Rect,
    content_len: usize,
    scroll: usize,
    scroll_max: usize,
) -> VScrollbarLayout {
    let last = content_len.saturating_sub(1);
    let mut sb_state = ScrollbarState::new(last).position(scroll.min(last));
    StatefulWidget::render(
        Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None),
        track,
        buf,
        &mut sb_state,
    );
    let clamp = |n: usize| n.min(usize::from(u16::MAX)) as u16;
    VScrollbarLayout {
        x: Some(track.x),
        y: track.y,
        height: track.height,
        content_len,
        scroll: clamp(scroll),
        scroll_max: clamp(scroll_max),
    }
}

/// Paint a 1-row horizontal scrollbar in `track` for a pan of `col_offset`
/// out of `col_max` columns, and return its hit geometry for
/// [`super::split::hit_hscrollbar`]. The bar is the ratatui `Scrollbar`
/// with no end symbols, the same math as the graph widget's horizontal
/// bar, so [`workspace_status_graph::graph_scrollbar_thumb`] of
/// `col_max + 1` finds its thumb.
fn paint_hscrollbar(
    buf: &mut Buffer,
    track: Rect,
    col_max: usize,
    col_offset: u16,
) -> HScrollbarLayout {
    let mut sb_state = ScrollbarState::new(col_max).position(usize::from(col_offset).min(col_max));
    StatefulWidget::render(
        Scrollbar::new(ScrollbarOrientation::HorizontalBottom)
            .begin_symbol(None)
            .end_symbol(None),
        track,
        buf,
        &mut sb_state,
    );
    HScrollbarLayout {
        y: Some(track.y),
        x: track.x,
        width: track.width,
        col_max: col_max.min(usize::from(u16::MAX)) as u16,
        col_offset,
    }
}

/// Where a list (the Workspace tree, a commit-file or compare list, the
/// folder summary, the Explorer tree, the PRs tab list and detail) paints
/// its rows and its scrollbars in `area`, decided before the rows paint,
/// as the graph list decides its bars: the vertical bar takes the last
/// column whenever the rows overflow, and the horizontal bar takes the
/// last row once the list pans off the left edge ([`list_hbar_rows`]).
/// The rows never paint under a bar, and the pan clamp sizes rows to
/// [`ListBars::rows_area`]'s width.
struct ListBars {
    /// The list's whole area.
    area: Rect,
    /// Rows the list paints (the area less the horizontal bar row).
    rows: u16,
    /// Painted rows the bar measures.
    row_count: usize,
    /// The vertical bar shows.
    vscroll: bool,
    /// The horizontal bar row is kept.
    hscroll: bool,
}

impl ListBars {
    /// Bars for `row_count` painted rows in `area`, panned `col_offset`.
    fn new(area: Rect, row_count: usize, col_offset: u16) -> Self {
        let h_rows = list_hbar_rows(area.height, col_offset);
        let rows = area.height.saturating_sub(h_rows);
        Self {
            area,
            rows,
            row_count,
            vscroll: row_count > usize::from(rows),
            hscroll: h_rows > 0,
        }
    }

    /// Where the rows paint: left of the vertical bar, above the
    /// horizontal bar.
    fn rows_area(&self) -> Rect {
        Rect {
            width: self.area.width.saturating_sub(u16::from(self.vscroll)),
            height: self.rows,
            ..self.area
        }
    }

    /// Paint the bars over the painted rows: the vertical bar for the rows
    /// from `start`, the horizontal bar for a pan of `col_offset` out of
    /// `col_max` (none at 0). Flat panes give the vertical bar's cells the
    /// row backgrounds beside them, as the graph list does. Returns the hit
    /// geometry of each bar (`x` / `y` `None` when not painted).
    fn paint(
        &self,
        frame: &mut Frame<'_>,
        start: usize,
        pan: (u16, usize),
        flat: bool,
    ) -> (VScrollbarLayout, HScrollbarLayout) {
        let (col_offset, col_max) = pan;
        let area = self.area;
        let buf = frame.buffer_mut();
        let mut vbar = VScrollbarLayout::default();
        if self.vscroll && area.width > 0 && area.height > 0 {
            let track = Rect {
                x: area.right() - 1,
                width: 1,
                ..area
            };
            let max = self.row_count.saturating_sub(usize::from(self.rows));
            vbar = paint_vscrollbar(buf, track, self.row_count, start, max);
            if flat {
                bleed_vscrollbar_column(buf, vbar);
            }
        }
        let mut hbar = HScrollbarLayout::default();
        if self.hscroll && col_max > 0 && area.width > 0 {
            let track = Rect {
                y: area.bottom() - 1,
                width: self.rows_area().width.max(1),
                height: 1,
                ..area
            };
            hbar = paint_hscrollbar(buf, track, col_max, col_offset);
        }
        (vbar, hbar)
    }
}

/// The pan a list panned `col_offset` paints with, and its pan clamp.
/// A pan the clamp no longer allows (the pane widened since the pan) drops
/// to the clamp, in state too ([`AppState::scroll_hbar_to`] on `bar`), as
/// the next Shift-←/→ would clamp it; at 0 the list keeps no bar row.
/// `max_col(state, vscroll)` is the list's pan clamp beside a shown (or
/// hidden) vertical bar; it runs only while panned, as it walks every row.
fn clamped_list_pan(
    state: &mut AppState,
    bar: HScrollbar,
    area: Rect,
    row_count: usize,
    col_offset: u16,
    max_col: impl Fn(&AppState, bool) -> usize,
) -> (u16, usize) {
    if col_offset == 0 {
        return (0, 0);
    }
    let vscroll = ListBars::new(area, row_count, col_offset).vscroll;
    let col_max = max_col(state, vscroll);
    if usize::from(col_offset) <= col_max {
        return (col_offset, col_max);
    }
    let clamped = col_max.min(usize::from(u16::MAX)) as u16;
    state.scroll_hbar_to(bar, clamped);
    (clamped, col_max)
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
            let right_cell = right.as_ref().unwrap();
            let mut left_rows = CellRows::new(
                left,
                left_syntax,
                cols.left_width,
                gutter,
                state.ascii,
                wrap,
            );
            let mut right_rows = CellRows::new(
                right_cell,
                right_syntax,
                cols.right_width,
                gutter,
                state.ascii,
                wrap,
            );
            let (left_h, right_h) = (left_rows.height(), right_rows.height());
            (0..left_h.max(right_h))
                .map(|part| {
                    let mut spans = paint_cell_spans(
                        left,
                        cols.left_width,
                        gutter,
                        part == 0,
                        left_rows.code(part, col_offset, palette),
                        palette,
                        state,
                        overlay,
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
                        right_cell,
                        cols.right_width,
                        gutter,
                        part == 0,
                        right_rows.code(part, col_offset, palette),
                        palette,
                        state,
                        overlay,
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
            let mut rows = CellRows::new(left, left_syntax, width, gutter, state.ascii, wrap);
            let n = rows.height();
            (0..n)
                .map(|part| {
                    let mut spans = paint_cell_spans(
                        left,
                        width,
                        gutter,
                        part == 0,
                        rows.code(part, col_offset, palette),
                        palette,
                        state,
                        overlay,
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
    // Italic marks the note as metadata. It is a modifier, not a colour, so
    // NO_COLOR keeps it; terminals without italics paint plain text.
    let note = |style: Style| style.fg(palette.muted).add_modifier(Modifier::ITALIC);
    spans.push(Span::styled("  ", style));
    if on_cursor_bar {
        // The two-column gap keeps the pad's bg; the note and the rest drop it.
        style.bg = None;
    }
    spans.push(Span::styled(fitted, note(style)));
    if rest > 0 {
        spans.push(Span::styled(" ".repeat(rest), style));
    }
}

/// Commit popover hits on the line-blame note `note` painted on one of the
/// buffer rows `rows`, columns `x0..x1`: the author
/// ([`IconKind::CommitAuthor`]), the age ([`IconKind::CommitDate`]), and the
/// short hash ([`IconKind::CommitHash`]) of `blame` in checkout `repo`.
///
/// `note` is the `annotation_text` the row painted
/// (`{author}, {age} · {sha7} · {summary}`). [`put_line_annotation`] cuts
/// it from the end, so the row is searched for the longest painted prefix:
/// all three fields, else author and age, else the author alone. A field
/// cut short records no hit. Each hit covers its field's display cells from
/// the match start. No commit id, or no row shows the note: no hit.
fn blame_note_hits(
    buf: &Buffer,
    rows: &[u16],
    x0: u16,
    x1: u16,
    note: &str,
    blame: &LineBlame,
    repo: &str,
) -> Vec<IconHit> {
    if blame.sha.is_empty() {
        return Vec::new();
    }
    let author = blame.author.as_str();
    let Some(rest) = note
        .strip_prefix(author)
        .and_then(|rest| rest.strip_prefix(", "))
    else {
        return Vec::new();
    };
    let Some((age, after_age)) = rest.split_once(" · ") else {
        return Vec::new();
    };
    let sha7 = short_sha(&blame.sha);
    if !after_age.starts_with(sha7.as_str()) {
        return Vec::new();
    }
    let date_x = format!("{author}, ").width();
    let hash_x = format!("{author}, {age} · ").width();
    // Kind, start column inside the note, and width of each field.
    let fields = [
        (IconKind::CommitAuthor, 0, author.width()),
        (IconKind::CommitDate, date_x, age.width()),
        (IconKind::CommitHash, hash_x, sha7.width()),
    ];
    let prefixes = [
        (format!("{author}, {age} · {sha7}"), 3),
        (format!("{author}, {age}"), 2),
        (format!("{author}, "), 1),
    ];
    // The note sits on the last painted row of its line.
    for &y in rows.iter().rev() {
        for (prefix, shown) in &prefixes {
            let Some((x, _)) = painted_text_span(buf, y, x0, x1, prefix) else {
                continue;
            };
            return fields[..*shown]
                .iter()
                .filter(|(_, _, width)| *width > 0)
                .map(|(kind, at, width)| IconHit {
                    y,
                    x: x + *at as u16,
                    width: *width as u16,
                    kind: *kind,
                    target: IconTarget::Commit {
                        repo: repo.to_string(),
                        sha: blame.sha.clone(),
                    },
                })
                .collect();
        }
    }
    Vec::new()
}

/// Flat diff rows, before [`bleed_flat_pad`]: the two cells a row leaves
/// unpainted take its background, so add / remove tints and the cursor
/// row reach the pane edges. A blank edge-marker cell (column 0, no
/// selection bar) takes the bg of the cell after it; the reserved
/// scrollbar column (from `lines.right()` up to `right`) the bg of the
/// row's last cell. The scrollbar glyphs paint on top.
fn bleed_diff_row_edges(frame: &mut Frame<'_>, lines: Rect, right: u16) {
    if lines.width < 2 {
        return;
    }
    let buf = frame.buffer_mut();
    for y in lines.top()..lines.bottom() {
        if buf[(lines.x, y)].symbol() == " " {
            let bg = buf[(lines.x + 1, y)].bg;
            buf[(lines.x, y)].set_bg(bg);
        }
        let bg = buf[(lines.right() - 1, y)].bg;
        for x in lines.right()..right {
            buf[(x, y)].set_bg(bg);
        }
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

/// One diff cell's code, cut into the screen rows the cell paints.
///
/// With wrap on, the wrap starts are scanned once per cell and a
/// forward-only [`StyledColCursor`] slices each row from the last one, so
/// a line of `n` wrap rows costs one scan, not `n`.
struct CellRows<'a> {
    cell: &'a DiffCell,
    syntax: &'a [CodeSpan],
    /// Code columns after the gutter, comment mark, and sign.
    code_w: usize,
    /// Wrap-row start columns; `None` with wrap off.
    starts: Option<Vec<usize>>,
    cursor: StyledColCursor<'a>,
}

impl<'a> CellRows<'a> {
    fn new(
        cell: &'a DiffCell,
        syntax: &'a [CodeSpan],
        width: u16,
        gutter: usize,
        ascii: bool,
        wrap: bool,
    ) -> Self {
        let code_w = cell_code_width(
            width as usize,
            gutter.saturating_add(comment_mark_cols(ascii)),
        );
        Self {
            cell,
            syntax,
            code_w,
            starts: wrap.then(|| wrap_col_starts(&cell.text, code_w)),
            cursor: StyledColCursor::new(syntax),
        }
    }

    /// Screen rows the cell paints: its wrap rows, else one.
    fn height(&self) -> usize {
        self.starts.as_ref().map_or(1, |starts| starts.len().max(1))
    }

    /// Code of screen row `part`, in increasing `part` order. With wrap off
    /// the one row starts at the pan `col_offset`; a wrap row past the
    /// cell's last one (the taller side of a split row) is empty.
    fn code(&mut self, part: usize, col_offset: usize, palette: Palette) -> Vec<CodeSpan> {
        let offset = match &self.starts {
            None => col_offset,
            Some(starts) => match starts.get(part) {
                Some(&offset) => offset,
                None => return Vec::new(),
            },
        };
        match (self.starts.is_some(), self.cell.kind) {
            (true, DiffCellKind::Add | DiffCellKind::Del | DiffCellKind::Ctx) => {
                self.cursor.slice(offset, self.code_w)
            }
            _ => paint_code_parts(self.cell, self.syntax, offset, self.code_w, palette),
        }
    }
}

/// Spans of one painted row of `cell`: gutter, rule, sign, `code_parts`
/// (from [`CellRows::code`]), and the blank pad to `width`. `first` marks
/// the cell's first row, which carries the line number, comment mark, and
/// sign.
fn paint_cell_spans(
    cell: &DiffCell,
    width: u16,
    gutter: usize,
    first: bool,
    code_parts: Vec<CodeSpan>,
    palette: Palette,
    state: &AppState,
    overlay: Option<Color>,
) -> Vec<Span<'static>> {
    let ascii = state.ascii;
    let width = width as usize;
    let line_comment = cell.line_no.and_then(|n| state.diff_line_comment(n));
    let comment = if first { line_comment } else { None };
    // Every painted row of a line with an open comment underlines its code
    // (`draw_diff_pane` records the runs for the curly upgrade).
    let underline = if line_comment == Some(false) {
        Modifier::UNDERLINED
    } else {
        Modifier::empty()
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
            with_row_bg(Style::default().fg(fg).add_modifier(underline), bg),
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
    Span::styled(format!(" {key} "), key_chip_style(bg, fg))
}

/// Style of a filled key chip: bold `fg` on `bg`.
fn key_chip_style(bg: Color, fg: Color) -> Style {
    Style::default().fg(fg).bg(bg).add_modifier(Modifier::BOLD)
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
        K::Branch | K::ChipLocal => palette.branch_feature,
        K::ChipDefault => palette.branch_default,
        K::MergedIntoDefault | K::Ahead => palette.added,
        K::StatusAdded | K::StatusStaged => palette.added,
        K::Behind | K::StatusFailed | K::StatusDeleted | K::StatusConflict => palette.deleted,
        // Closed only alerts, on the remove ramp; the lookup failure mark
        // paints in the deleted colour.
        K::PrClosed | K::PrLookupFailed => palette.deleted,
        // A PR state row paints in its badge colour.
        K::PrCiFailed | K::PrReady | K::PrCiRunning | K::PrDraft | K::PrInReview | K::PrMerged => {
            PrState::ALL
                .into_iter()
                .find(|state| pr_badge_kind(*state) == kind)
                .map_or(palette.heading, |state| {
                    seg_role_color(pr_badge_role(state, false), palette)
                })
        }
        K::Diverged | K::StatusModified | K::StatusStagedModified => palette.modified,
        K::GraphUncommitted | K::ChipTag => palette.modified,
        K::StatusRenamed | K::StatusCopied => palette.renamed,
        K::OpenVsDefault | K::NoUpstream | K::Clean | K::Synced => palette.muted,
        K::Hidden | K::ChangeCount | K::WorktreeCount | K::CommentResolved => palette.muted,
        K::GraphMoreBelow | K::FoldExpanded | K::FoldCollapsed => palette.muted,
        K::CursorBarInactive => palette.muted,
        // Not in the legend; muted like their popover heading.
        K::CommitHash | K::CommitDate | K::CommitAuthor => palette.muted,
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
    // A compare tab swaps GIT for the COMPARE column, an Explorer tab for
    // EXPLORER.
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
        PendingConfirm::StashDrop { stash, .. } => {
            let stash_ref = &stash.stash_ref;
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
        PendingConfirm::Push {
            branch,
            remote,
            publish,
            ahead_behind,
            ..
        } => {
            let accent = palette.modified;
            let mut title = vec![
                Span::styled(
                    "Push ",
                    Style::default().fg(accent).add_modifier(Modifier::BOLD),
                ),
                Span::styled(branch.clone(), Style::default().fg(palette.file)),
                Span::styled(" → ", Style::default().fg(palette.muted)),
                Span::styled(remote.clone(), Style::default().fg(palette.file)),
            ];
            if let Some((ahead, _)) = ahead_behind {
                let noun = if *ahead == 1 { "commit" } else { "commits" };
                title.push(Span::styled(
                    format!(" ({ahead} {noun})"),
                    Style::default().fg(palette.muted),
                ));
            }
            title.push(Span::styled("?", Style::default().fg(accent)));
            let mut details = Vec::new();
            if *publish {
                details.push(format!(
                    "  creates the remote branch {remote}/{branch} (git push -u)"
                ));
            }
            if let Some((ahead, behind)) = ahead_behind.filter(|(_, behind)| *behind > 0) {
                details.push(format!(
                    "  diverged (ahead {ahead}, behind {behind}) · git may reject the push"
                ));
            }
            if details.is_empty() {
                details.push("  git push, no --force".to_string());
            }
            let mut lines = vec![Line::from(title)];
            lines.extend(
                details
                    .into_iter()
                    .map(|d| Line::from(Span::styled(d, Style::default().fg(palette.muted)))),
            );
            lines.push(confirm_action_row(
                "y",
                "push",
                None,
                accent,
                palette.muted,
                panel,
            ));
            (accent, lines)
        }
        PendingConfirm::ResetToUpstream {
            repo,
            branch,
            upstream,
            dropped,
            ..
        } => {
            let accent = palette.modified;
            let noun = if *dropped == 1 { "commit" } else { "commits" };
            // The repo leads the title: `p` on the workspace or family row
            // can open this box for a repo the cursor is not on.
            let lines = vec![
                Line::from(vec![
                    Span::styled(
                        repo.clone(),
                        Style::default()
                            .fg(palette.file)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(": ", Style::default().fg(palette.muted)),
                    Span::styled(
                        upstream.clone(),
                        Style::default()
                            .fg(palette.file)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        " was rewritten. Move ",
                        Style::default().fg(accent).add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(branch.clone(), Style::default().fg(palette.file)),
                    Span::styled(" to ", Style::default().fg(palette.muted)),
                    Span::styled(upstream.clone(), Style::default().fg(palette.file)),
                    Span::styled("?", Style::default().fg(accent)),
                ]),
                Line::from(Span::styled(
                    format!("  Drops {dropped} {noun} that {upstream} dropped; none are yours."),
                    Style::default().fg(palette.muted),
                )),
                confirm_action_row("y", "reset", None, accent, palette.muted, panel),
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
            | super::stash::StashOpId::Drop => op.stash_ref().unwrap_or(""),
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

/// Doctor dialog title.
const DOCTOR_TITLE: &str = "Doctor · setup checkup";

/// Doctor footer chips.
const DOCTOR_FOOTER: &str = "j/k move · y copy fix · r re-run · Esc close";

/// Columns before a Doctor row's name and under it: the cursor mark (2),
/// the status chip (4), and a space.
const DOCTOR_INDENT: usize = 7;

/// Section header the Doctor paints above `group`'s rows.
fn doctor_group_title(group: CheckGroup) -> &'static str {
    match group {
        CheckGroup::Terminal => "TERMINAL",
        CheckGroup::Tools => "TOOLS",
        CheckGroup::Files => "FILES",
    }
}

/// Status chip text, so a row reads without colour, and its theme colour.
fn doctor_status_chip(status: CheckStatus, palette: Palette) -> (&'static str, Color) {
    match status {
        CheckStatus::Ok => ("ok", palette.added),
        CheckStatus::Warn => ("warn", palette.modified),
        CheckStatus::Fail => ("fail", palette.deleted),
    }
}

/// `<n> ok · <n> warn · <n> fail`, each count in its chip colour.
fn doctor_summary_line(checks: &[Check], palette: Palette) -> Line<'static> {
    let counts = doctor_summary(checks);
    let sep = || Span::styled(" · ", Style::default().fg(palette.muted));
    let part = |n: usize, status: CheckStatus| {
        let (label, color) = doctor_status_chip(status, palette);
        Span::styled(format!("{n} {label}"), Style::default().fg(color))
    };
    Line::from(vec![
        part(counts.ok, CheckStatus::Ok),
        sep(),
        part(counts.warn, CheckStatus::Warn),
        sep(),
        part(counts.fail, CheckStatus::Fail),
    ])
}

/// Doctor body: a header per group, then per row its name line (cursor
/// mark, status chip, name), the wrapped why, the fix hint, and the copy
/// line. Returns the lines and the cursor row's line range, which starts at
/// its group header when the row is the first of its group.
fn doctor_body_lines(
    checks: &[Check],
    cursor: usize,
    width: usize,
    ascii: bool,
    palette: Palette,
) -> (Vec<Line<'static>>, std::ops::Range<usize>) {
    let text_width = width.saturating_sub(DOCTOR_INDENT).max(1);
    let indent = " ".repeat(DOCTOR_INDENT);
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut focused = 0..0;
    let mut group = None;
    for (index, check) in checks.iter().enumerate() {
        let start = lines.len();
        if group != Some(check.group) {
            group = Some(check.group);
            lines.push(Line::from(Span::styled(
                doctor_group_title(check.group),
                Style::default()
                    .fg(palette.heading)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        let selected = index == cursor;
        let bg = if selected {
            palette.cursor_bg
        } else {
            palette.panel
        };
        let mark = match (selected, ascii) {
            (false, _) => "  ",
            (true, true) => "> ",
            (true, false) => "❯ ",
        };
        let (chip, color) = doctor_status_chip(check.status, palette);
        let name = format!(" {}", check.name);
        let used = DOCTOR_INDENT - 1 + visible_width(&name);
        lines.push(Line::from(vec![
            Span::styled(mark, Style::default().fg(palette.cursor).bg(bg)),
            Span::styled(
                format!("{chip:<4}"),
                Style::default()
                    .fg(color)
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(name, Style::default().fg(palette.file).bg(bg)),
            // The cursor bar runs to the box edge.
            Span::styled(
                " ".repeat(width.saturating_sub(used)),
                Style::default().bg(bg),
            ),
        ]));
        let mut text = |body: &str, style: Style| {
            for row in wrap_help_description(body, text_width) {
                lines.push(Line::from(vec![
                    Span::raw(indent.clone()),
                    Span::styled(row, style),
                ]));
            }
        };
        text(&check.why, Style::default().fg(palette.repo));
        if let Some(fix) = check.fix.as_ref() {
            if let Some(hint) = fix.hint.as_deref() {
                text(hint, Style::default().fg(palette.muted));
            }
            if let Some(copy) = fix.copy.as_deref() {
                // The paste line keeps its spaces: cut by columns, not words.
                let code = Style::default()
                    .fg(palette.cursor)
                    .bg(palette.cursor_bg_inactive);
                for row in wrap_cols(copy, text_width) {
                    lines.push(Line::from(vec![
                        Span::raw(indent.clone()),
                        Span::styled(row, code),
                    ]));
                }
            }
        }
        if selected {
            focused = start..lines.len();
        }
    }
    (lines, focused)
}

/// First body line to paint so `focused` stays in view.
///
/// Starts from the last paint's `top` and moves only as far as it must; a
/// row taller than the view shows its top. Never scrolls past the end.
fn doctor_scroll_top(
    top: usize,
    focused: std::ops::Range<usize>,
    total: usize,
    view: usize,
) -> usize {
    if view == 0 {
        return 0;
    }
    let mut top = top.min(total.saturating_sub(view));
    if focused.start < top || focused.len() > view {
        top = focused.start;
    } else if focused.end > top + view {
        top = focused.end - view;
    }
    top
}

/// Doctor setup checkup: title, summary (or `checking…`), the rows under
/// TERMINAL / TOOLS / FILES headers scrolled to keep the cursor row in
/// view, and the key footer. Returns the first body line painted, for
/// the overlay's `scroll`.
fn draw_doctor(frame: &mut Frame<'_>, area: Rect, state: &AppState) -> usize {
    let Some(doctor) = state.doctor.as_ref() else {
        return 0;
    };
    if area.width == 0 || area.height == 0 {
        return doctor.scroll;
    }
    let palette = state.theme.palette();
    let accent = palette.heading;
    let block = overlay_block(accent, palette.panel);
    let inner = block.inner(area);
    frame.render_widget(Clear, area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return doctor.scroll;
    }
    let width = usize::from(inner.width);
    let summary = match doctor.checks.as_deref() {
        Some(checks) => doctor_summary_line(checks, palette),
        None => Line::from(Span::styled(
            "checking…",
            Style::default().fg(palette.muted),
        )),
    };
    let head = vec![
        Line::from(Span::styled(
            DOCTOR_TITLE,
            Style::default().fg(accent).add_modifier(Modifier::BOLD),
        )),
        summary,
    ];
    let (body, focused) = doctor
        .checks
        .as_deref()
        .map(|checks| doctor_body_lines(checks, doctor.cursor, width, state.ascii, palette))
        .unwrap_or_default();
    let footer = fit_list_dialog_footer(
        Line::from(Span::styled(
            DOCTOR_FOOTER,
            Style::default().fg(palette.muted),
        )),
        width,
    );
    let head_h = (head.len() as u16).min(inner.height);
    let footer_h = u16::from(inner.height > head_h);
    let body_h = inner.height - head_h - footer_h;
    let top = doctor_scroll_top(doctor.scroll, focused, body.len(), usize::from(body_h));
    frame.render_widget(
        Paragraph::new(head),
        Rect {
            height: head_h,
            ..inner
        },
    );
    frame.render_widget(
        Paragraph::new(
            body.into_iter()
                .skip(top)
                .take(usize::from(body_h))
                .collect::<Vec<_>>(),
        ),
        Rect {
            y: inner.y + head_h,
            height: body_h,
            ..inner
        },
    );
    frame.render_widget(
        Paragraph::new(footer),
        Rect {
            y: inner.y + head_h + body_h,
            height: footer_h,
            ..inner
        },
    );
    top
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
    let labels = state.tab_labels();
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
            // A clipped close has no box, so it never paints hovered. The
            // open search dialog keeps the pointer for its chips, so
            // pane_pointer hides it from the strip then.
            let hovered = close_painted
                && state.pane_pointer().is_some_and(|(col, row)| {
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
    draw_outage_header(frame, area, state, x, limit);
}

/// The failure mark of the failing background jobs ([`outage_header_text`]),
/// bold in `palette.deleted`, flush right in the free cells of the tab strip
/// row `area`: right of the last tab (which ends at `tabs_end`) and one
/// blank cell, left of `limit` (the right overflow marker, if any). The
/// tabs keep priority: with too little room the mark gets shorter, then
/// does not paint.
fn draw_outage_header(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &AppState,
    tabs_end: u16,
    limit: u16,
) {
    if state.outages.is_empty() {
        return;
    }
    let room = limit.saturating_sub(tabs_end).saturating_sub(1);
    let Some(text) = outage_header_text(&state.outages.kinds(), state.ascii, usize::from(room))
    else {
        return;
    };
    let width = painted_width(&text);
    let style = if state.no_color {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(state.theme.palette().deleted)
            .add_modifier(Modifier::BOLD)
    };
    frame.render_widget(
        Paragraph::new(Span::styled(text, style)),
        Rect {
            x: limit - width,
            width,
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
    // Esc clears typed text first and closes only an empty query.
    let esc = if quick.has_typed_query() {
        "Esc clear"
    } else {
        "Esc close"
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
            format!("↑↓ move · Enter open · > commands · # search · {esc}"),
        )
    } else {
        let commands = &quick.commands;
        let reason = commands
            .selected()
            .and_then(|command| state.palette_disabled_reason(&command));
        let footer = match reason {
            Some(why) => format!("Enter run · {esc} · {why}"),
            None => format!("Enter run · {esc}"),
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

/// Commands-mode rows: the go-to-line row while it shows, group headers,
/// command titles, key chips, and the disabled reason at the right edge
/// when it fits.
fn quick_open_command_rows(
    state: &AppState,
    palette: &CommandPaletteState,
    max_rows: usize,
    inner_width: usize,
) -> Vec<Line<'static>> {
    let palette_theme = state.theme.palette();
    let paint_rows = palette.paint_rows();
    let cursor_paint = paint_rows.iter().position(|row| match row {
        PalettePaintRow::GoToLine { .. } => palette.cursor == 0,
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
                // The synthetic row sits above every group: no label.
                PalettePaintRow::GoToLine { line } => lines.push(palette_command_line(
                    state,
                    &go_to_line_command(line),
                    &go_to_line_title(line),
                    palette.cursor == 0,
                    false,
                    inner_width,
                )),
                PalettePaintRow::Command { command, index } => lines.push(palette_command_line(
                    state,
                    command,
                    command.title,
                    index == palette.cursor,
                    !header_painted,
                    inner_width,
                )),
            }
        }
    }
    lines
}

/// One commands-mode row: cursor mark, `title`, key chip, the group label
/// when `group_label`, and the disabled reason at the right edge when it
/// fits.
fn palette_command_line(
    state: &AppState,
    command: &PaletteCommand,
    title: &str,
    selected: bool,
    group_label: bool,
    inner_width: usize,
) -> Line<'static> {
    let palette_theme = state.theme.palette();
    let accent = palette_theme.cursor;
    let panel = palette_theme.panel;
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
        Span::styled(title.to_string(), style),
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
    if group_label {
        spans.push(Span::styled(
            format!("  {}", command.group.title()),
            Style::default()
                .fg(palette_theme.muted)
                .bg(row_bg)
                .add_modifier(dim),
        ));
    }
    // The reason sits dimmed at the right edge when it fits, so every
    // disabled row says why, not only the cursor row.
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
    Line::from(spans)
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
    let title = Span::styled(
        dialog.title(),
        Style::default()
            .fg(palette.heading)
            .add_modifier(Modifier::BOLD),
    );
    // Record the chip boxes before paint, so the hover this frame paints
    // comes from the same boxes a click hits.
    let chip_xs = search_option_chip_xs(area, u16::try_from(title.width()).unwrap_or(u16::MAX));
    state.layout.search_option_y = area.y;
    state.layout.search_option_hits = chip_xs
        .iter()
        .map(|&(x, option)| (x, SEARCH_OPTION_CHIP_W, option))
        .collect();
    let hovered = state.hovered_search_option();
    let block = overlay_block(accent, panel).title(title);
    frame.render_widget(Clear, area);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let chips = search_option_chips(dialog.options, hovered, palette, panel);
    let buf = frame.buffer_mut();
    for (i, &(x, option)) in chip_xs.iter().enumerate() {
        if i > 0 {
            // The gap between two chips is blank, not border.
            buf[(x - 1, area.y)].set_symbol(" ");
        }
        if let Some((_, chip)) = chips.iter().find(|(o, _)| *o == option) {
            buf.set_span(x, area.y, chip, SEARCH_OPTION_CHIP_W);
        }
    }
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
        // A hovered chip says what it is and what a click does instead.
        let footer = match hovered {
            Some(option) => option_tooltip(option, dialog.options),
            None => dialog.footer_hints(state.mouse_enabled),
        };
        let footer = Line::from(Span::styled(footer, muted));
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
        let (epoch, ascii) = (state.anim_epoch, state.ascii);
        let fg = loading_fg(state.no_color, palette);
        let layout = &mut state.layout;
        let mut loading = |text: &str| (spinner_text(epoch, ascii, layout, text), fg);
        let lines =
            search_preview_lines(dialog, index, palette, theme, preview_inner, &mut loading);
        frame.render_widget(Paragraph::new(lines), preview_inner);
    }
}

/// `line` cut to `width` columns.
fn clamp_line(line: Line<'static>, width: usize) -> Line<'static> {
    Line::from(clamp_spans(line.spans, width))
}

/// Match option chips on the search dialog's top border, left to right:
/// `Aa` case, `ab` whole word, `.*` regex.
const SEARCH_OPTION_LABELS: [(SearchFilesOption, &str); 3] = [
    (SearchFilesOption::Case, "Aa"),
    (SearchFilesOption::WholeWord, "ab"),
    (SearchFilesOption::Regex, ".*"),
];

/// Cells one option chip takes: its two-cell label and a space each side.
const SEARCH_OPTION_CHIP_W: u16 = 4;

/// Left column of each option chip on the top border of the search dialog
/// box `area`, whose left title is `title_width` cells wide.
///
/// The chips sit right-aligned inside the corner, one blank cell apart,
/// as a right-aligned border title would. The title wins the row: a chip
/// that would not leave one blank cell after the title is left out, from
/// the left, so a narrow box keeps the rightmost chips whole.
fn search_option_chip_xs(area: Rect, title_width: u16) -> Vec<(u16, SearchFilesOption)> {
    // First free cell: past the left corner, the title, and one blank.
    let min_x = area.x.saturating_add(title_width).saturating_add(2);
    // Past the last chip cell: the right corner.
    let mut end = area.right().saturating_sub(1);
    let mut xs = Vec::new();
    for &(option, _) in SEARCH_OPTION_LABELS.iter().rev() {
        let Some(x) = end.checked_sub(SEARCH_OPTION_CHIP_W) else {
            break;
        };
        if x < min_x {
            break;
        }
        xs.push((x, option));
        end = x.saturating_sub(1);
    }
    xs.reverse();
    xs
}

/// Style of one option chip. Fill tells on from off; underline marks the
/// chip under the pointer, so both read without colour alone.
///
/// On: a filled accent chip ([`key_chip_style`]). Off: muted text, no fill.
/// Off and hovered: accent text on the highlighted-row background, a
/// fill other than on. On and hovered: the filled chip.
fn search_option_chip_style(on: bool, hovered: bool, palette: Palette, surface: Color) -> Style {
    let style = match (on, hovered) {
        (true, _) => key_chip_style(palette.cursor, surface),
        (false, false) => Style::default().fg(palette.muted),
        (false, true) => Style::default().fg(palette.cursor).bg(palette.cursor_bg),
    };
    if hovered {
        style.add_modifier(Modifier::UNDERLINED)
    } else {
        style
    }
}

/// Top-border chips for the match options, in [`SEARCH_OPTION_LABELS`]
/// order, each ` label ` in [`search_option_chip_style`]: filled when the
/// option is on, underlined when it is `hovered`.
fn search_option_chips(
    options: SearchOptions,
    hovered: Option<SearchFilesOption>,
    palette: Palette,
    surface: Color,
) -> [(SearchFilesOption, Span<'static>); 3] {
    SEARCH_OPTION_LABELS.map(|(option, label)| {
        let style = search_option_chip_style(
            option_is_on(options, option),
            hovered == Some(option),
            palette,
            surface,
        );
        (option, Span::styled(format!(" {label} "), style))
    })
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
/// binary, too large, or unreadable file is one dim line. A file still
/// loading shows `loading` of [`PREVIEW_LOADING`] (the spinner copy and its fg).
fn search_preview_lines(
    dialog: &SearchFilesState,
    index: &FileIndex,
    palette: Palette,
    theme: ThemeId,
    area: Rect,
    loading: &mut dyn FnMut(&str) -> (String, Color),
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
            let (text, fg) = loading(PREVIEW_LOADING);
            out.push(notice(text, Style::default().fg(fg)));
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
/// glyph in the row's colour and
/// [`super::popover::PopoverSection::heading`]), the meaning, fields (muted
/// label padded to the section's widest label, value in its role colour,
/// its bold head bold, cut with `…`), a muted rule where the section has
/// one, and action lines (title, key chip at the right edge; a disabled
/// one dims and names its gate reason when it fits). A pinned
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
            PopoverLine::Action { command, .. } if gated[section] => state.popover_line_refusal(
                command,
                sections[section].icon,
                &sections[section].target,
            ),
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
                let section = &sections[index];
                visible_width(section.icon.glyph(state.ascii))
                    + 1
                    + visible_width(section.heading())
            }
            PopoverRow::Line(index) => {
                2 + match lines[index] {
                    PopoverLine::Text(text) | PopoverLine::Note(text) => visible_width(text),
                    PopoverLine::Field { value, mark, .. } => {
                        label_width(index)
                            + 1
                            + mark.map_or(0, |(mark, _)| visible_width(mark) + 1)
                            + visible_width(value)
                    }
                    // A rule fills the width the other rows set.
                    PopoverLine::Rule => 0,
                    PopoverLine::Action { title, keys, .. } => {
                        visible_width(title)
                            + reasons[index]
                                .as_deref()
                                .map_or(0, |why| 2 + visible_width(why))
                            + chip_width(keys)
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

    // Per line: the painted value of a link field, as (inner column,
    // width, full URL), for the OSC 8 overlay.
    let mut line_links: Vec<Option<(usize, usize, String)>> = vec![None; lines.len()];
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
                            section.heading().to_string(),
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
                    PopoverLine::Field {
                        label,
                        value,
                        role,
                        link,
                        bold,
                        mark,
                    } => {
                        let pad = label_width(index).saturating_sub(visible_width(label));
                        spans.push(Span::styled(format!("{label}{} ", " ".repeat(pad)), muted));
                        if let Some((mark, mark_role)) = mark {
                            spans.push(Span::styled(
                                format!("{mark} "),
                                Style::default().fg(seg_role_color(*mark_role, palette)),
                            ));
                        }
                        let style = Style::default().fg(seg_role_color(*role, palette));
                        // A cut value ends in `…`; a link keeps the full
                        // URL.
                        let x = help_spans_width(&spans);
                        let room = inner_width.saturating_sub(x);
                        if room > 0 {
                            let shown = fit_with_ellipsis(value, room);
                            if *link {
                                line_links[index] = Some((x, visible_width(&shown), value.clone()));
                            }
                            let mut head = (*bold).min(shown.len());
                            while !shown.is_char_boundary(head) {
                                head -= 1;
                            }
                            let (bold_text, rest) = shown.split_at(head);
                            if !bold_text.is_empty() {
                                spans.push(Span::styled(
                                    bold_text.to_string(),
                                    style.add_modifier(Modifier::BOLD),
                                ));
                            }
                            if !rest.is_empty() {
                                spans.push(Span::styled(rest.to_string(), style));
                            }
                        }
                    }
                    PopoverLine::Rule => {
                        let rule = if state.ascii { "-" } else { "─" };
                        let width = inner_width.saturating_sub(help_spans_width(&spans));
                        spans.push(Span::styled(rule.repeat(width), muted));
                    }
                    PopoverLine::Action { title, keys, .. } => {
                        let reason = reasons[index].as_deref();
                        let mut title_style = Style::default().fg(palette.file);
                        if reason.is_some() {
                            title_style = title_style.fg(palette.muted).add_modifier(Modifier::DIM);
                        }
                        spans.push(Span::styled(*title, title_style));
                        let chip_cols = chip_width(keys);
                        if let Some(why) = reason {
                            let used = help_spans_width(&spans);
                            if used + 2 + visible_width(why) + chip_cols <= inner_width {
                                spans.push(Span::styled(
                                    format!("  {why}"),
                                    muted.add_modifier(Modifier::DIM),
                                ));
                            }
                        }
                        if !keys.is_empty() {
                            let chip_bg = if reason.is_some() {
                                palette.muted
                            } else {
                                accent
                            };
                            chip = Some(key_chip(keys, chip_bg, panel));
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
    drop_hits_under(&mut state.layout, rect);
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
            if let Some((x, width, url)) = line_links[index].take() {
                let width = width.min(usize::from(inner.width).saturating_sub(x));
                if let (Ok(x), Ok(width)) = (u16::try_from(x), u16::try_from(width)) {
                    if width > 0 {
                        state.layout.links.push(LinkHit {
                            y,
                            x: inner.x + x,
                            width,
                            url,
                        });
                    }
                }
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
mod tests;

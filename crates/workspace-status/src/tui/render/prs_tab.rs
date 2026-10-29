//! Paint of the PRs tab: the list pane (`prs`) beside the detail pane
//! (`PR`), or one of them in a narrow frame.
//!
//! The list pins the CLI error lines and the loading line at its top, then
//! scrolls the group headers and PR rows ([`PrsLine`]). A row paints
//! columns: glyph, ref, repository, title (the flexible one), CI cell,
//! reviews, state label, and stack mark; a narrow pane drops reviews, the
//! stack mark, then the `CI ` text, so the title keeps
//! [`TITLE_MIN_COLS`] ([`RowColumns::of`]). Its glyph and ref are one PR icon
//! hit, so hover, click, Ctrl+click, and the OSC 8 link work as on the
//! tree badge. The detail pane paints the focused PR: its title line, the
//! PR popover lines ([`pr_detail_lines`]), every checkout that shows it,
//! the repository, and the PR actions with their keys. Both panes paint
//! the shared vertical scrollbar while their lines overflow; the detail
//! keeps its own viewport per PR.

use super::*;

use super::super::popover::pr_detail_lines;
use super::super::pr_groups::{GroupHeadline, PrGroup, PrRow};
use super::super::pull_request::{PrReview, ReviewCount};
use super::super::state::{prs_row_flash_id, PrDetailState, PrsLine, PrsView, PRS_DETAIL_MIN_COLS};
use crate::snapshot::CheckoutKind;

/// List pane title.
const PRS_LIST_TITLE: &str = "prs";
/// Detail pane title.
const PRS_DETAIL_TITLE: &str = "PR";
/// The empty list: no checkout's branch has a PR.
const PRS_EMPTY: [&str; 3] = [
    "No PR for any checked-out branch.",
    "Open and merged PRs of each checkout's branch show here.",
    "Push a branch and open a PR, then press r.",
];
/// Repository column width; a longer path is cut with `…`.
const REPO_COLS: usize = 12;
/// Blank columns between two row columns.
const COLUMN_GAP: &str = "  ";

/// `looking up PRs · 3 of 8 checkouts` (one: `1 of 1 checkout`).
fn prs_loading_text(done: usize, total: usize) -> String {
    let noun = if total == 1 { "checkout" } else { "checkouts" };
    format!("looking up PRs · {done} of {total} {noun}")
}

/// The first header's poll age: `checked 40s ago`, or with a failing
/// lookup or a stale PR `checked 12m ago · lookups failing`.
fn prs_checked_text(age: &str, failing: bool) -> String {
    if failing {
        format!("checked {age} · lookups failing")
    } else {
        format!("checked {age}")
    }
}

/// The header text of a finished group of `n` PRs: `✓ all 2 merged`
/// (`m all 2 merged` in ASCII), and for one PR `✓ merged`.
fn all_merged_text(ascii: bool, n: usize) -> String {
    let mark = glyph(ascii, "✓", "m");
    if n == 1 {
        format!("{mark} merged")
    } else {
        format!("{mark} all {n} merged")
    }
}

/// CI mark of `row`, painted after `CI `: `✕` failed, `…` running, `✓`
/// passed, `—` none (ASCII `x`, `..`, `ok`, `-`), and its colour.
fn ci_mark(ascii: bool, row: &PrRow) -> (&'static str, SegRole) {
    let checks = row.pr.checks;
    let (unicode, plain, role) = if checks.fail > 0 {
        ("✕", "x", SegRole::Deleted)
    } else if checks.pending > 0 {
        ("…", "..", SegRole::Modified)
    } else if checks.pass > 0 {
        ("✓", "ok", SegRole::Added)
    } else {
        ("—", "-", SegRole::Muted)
    };
    (glyph(ascii, unicode, plain), role)
}

/// Reviews cell of the PR of checkout `checkout` from its detail: GitLab
/// `1/2` (given of required), GitHub `1` (approved count), else `—` (`-`
/// in ASCII). Added colour when the review rule is met.
fn reviews_cell(state: &AppState, checkout: &std::path::Path) -> (String, SegRole) {
    let detail = match state.pull_request_detail_for(checkout) {
        Some(PrDetailState::Ready(detail)) => detail,
        _ => return (glyph(state.ascii, "—", "-").to_string(), SegRole::Muted),
    };
    let role = if detail.review == Some(PrReview::Approved) {
        SegRole::Added
    } else {
        SegRole::Muted
    };
    match detail.reviews {
        Some(ReviewCount::Approvals { given, required }) => (format!("{given}/{required}"), role),
        Some(ReviewCount::Approved(n)) => (n.to_string(), role),
        None => (glyph(state.ascii, "—", "-").to_string(), SegRole::Muted),
    }
}

/// State label of a row: `CI failed`, `ready`, `CI running`, `draft`, `in
/// review`, or `merged` (the badge label, with ready to merge short).
fn row_state_label(state: PrState) -> &'static str {
    match state {
        PrState::Ready => "ready",
        state => state.label(),
    }
}

/// Stack mark of `row`: `⧗ after #318` (ASCII `after #318`), or empty.
fn stack_text(ascii: bool, row: &PrRow) -> String {
    match &row.stacked_on {
        Some(base) if ascii => format!("after {}", base.pr_ref),
        Some(base) => format!("⧗ after {}", base.pr_ref),
        None => String::new(),
    }
}

/// The title keeps at least this many columns while optional columns can
/// make room for it.
const TITLE_MIN_COLS: usize = 16;

/// `CI ` before the CI mark.
const CI_LABEL: &str = "CI ";

/// Widths of the row columns of one list, and the optional columns that
/// fit.
struct RowColumns {
    ref_w: usize,
    /// Widest CI mark.
    mark_w: usize,
    reviews_w: usize,
    state_w: usize,
    /// Widest stack mark; `0` when no row is stacked.
    stack_w: usize,
    /// The reviews column paints.
    reviews: bool,
    /// The stack column paints.
    stack: bool,
    /// The CI cell paints `CI ` before its mark.
    ci_label: bool,
}

impl RowColumns {
    /// The widest cell of each column over every row of `view`, for rows
    /// `width` columns wide after the row prefix. While the title would get
    /// fewer than [`TITLE_MIN_COLS`], the optional columns drop in this
    /// order: reviews, the stack mark, then the `CI ` text (the CI mark
    /// stays).
    fn of(state: &AppState, view: &PrsView, width: usize) -> Self {
        let rows = || view.groups.iter().flat_map(|group| &group.rows);
        let widest = |cell: &dyn Fn(&PrRow) -> String| {
            rows()
                .map(|row| visible_width(&cell(row)))
                .max()
                .unwrap_or(0)
        };
        let stack_w = widest(&|row| stack_text(state.ascii, row));
        let mut columns = Self {
            ref_w: widest(&|row| row.pr.ref_text()),
            mark_w: widest(&|row| ci_mark(state.ascii, row).0.to_string()),
            reviews_w: widest(&|row| reviews_cell(state, &row.checkout).0),
            state_w: widest(&|row| row_state_label(row.pr.state).to_string()),
            stack_w,
            reviews: true,
            stack: stack_w > 0,
            ci_label: true,
        };
        let drops: [fn(&mut Self); 3] = [
            |columns| columns.reviews = false,
            |columns| columns.stack = false,
            |columns| columns.ci_label = false,
        ];
        for drop in drops {
            if columns.title_w(width) >= TITLE_MIN_COLS {
                break;
            }
            drop(&mut columns);
        }
        columns
    }

    /// Columns every cell but the title takes, gaps included.
    fn fixed(&self) -> usize {
        let gap = COLUMN_GAP.len();
        let ci_label = if self.ci_label { CI_LABEL.len() } else { 0 };
        let reviews = if self.reviews {
            gap + self.reviews_w
        } else {
            0
        };
        let stack = if self.stack { gap + self.stack_w } else { 0 };
        2 + self.ref_w
            + gap
            + REPO_COLS
            + gap
            + gap
            + ci_label
            + self.mark_w
            + reviews
            + gap
            + self.state_w
            + stack
    }

    /// Columns the title gets in rows `width` columns wide.
    fn title_w(&self, width: usize) -> usize {
        width.saturating_sub(self.fixed())
    }
}

/// `text` cut to `width` columns with `…`, then padded to `width`.
fn cell(text: &str, width: usize) -> String {
    let shown = fit_with_ellipsis(text, width);
    let pad = width.saturating_sub(visible_width(&shown));
    format!("{shown}{}", " ".repeat(pad))
}

/// A plain segment.
fn seg(text: impl Into<String>, role: SegRole) -> TextSeg {
    TextSeg {
        text: text.into(),
        role,
        hex: None,
        bold: false,
        dim: false,
        icon: None,
    }
}

/// A bold segment.
fn bold_seg(text: impl Into<String>, role: SegRole) -> TextSeg {
    TextSeg {
        bold: true,
        ..seg(text, role)
    }
}

/// Segments of one PR row `width` columns wide after the row prefix, and
/// the width of its glyph and ref, its first cells (the PR icon hit).
fn row_segments(
    state: &AppState,
    row: &PrRow,
    columns: &RowColumns,
    width: usize,
) -> (Vec<TextSeg>, usize) {
    let ascii = state.ascii;
    let stale = state.pr_is_stale(&row.checkout, Instant::now());
    let glyph_role = pr_badge_role(row.pr.state, stale);
    let ref_role = if stale { SegRole::Muted } else { SegRole::Repo };
    let (mark, mark_role) = ci_mark(ascii, row);
    let label = row_state_label(row.pr.state).to_string();
    let gap = || seg(COLUMN_GAP, SegRole::Muted);
    let pr_ref = row.pr.ref_text();
    let mut segs = vec![
        bold_seg(pr_badge_kind(row.pr.state).glyph(ascii), glyph_role),
        seg(" ", SegRole::Muted),
        bold_seg(cell(&pr_ref, columns.ref_w), ref_role),
        gap(),
        seg(cell(&row.repo, REPO_COLS), SegRole::Muted),
        gap(),
        seg(cell(&row.title, columns.title_w(width)), SegRole::File),
        gap(),
    ];
    if columns.ci_label {
        segs.push(seg(CI_LABEL, SegRole::Muted));
    }
    segs.push(seg(cell(mark, columns.mark_w), mark_role));
    if columns.reviews {
        let (reviews, reviews_role) = reviews_cell(state, &row.checkout);
        segs.push(gap());
        segs.push(seg(cell(&reviews, columns.reviews_w), reviews_role));
    }
    segs.push(gap());
    segs.push(seg(
        cell(&label, columns.state_w),
        pr_badge_role(row.pr.state, false),
    ));
    if columns.stack {
        segs.push(gap());
        segs.push(seg(
            cell(&stack_text(ascii, row), columns.stack_w),
            SegRole::Modified,
        ));
    }
    (segs, 2 + visible_width(&pr_ref))
}

/// Segments of the header of `group`: the key, the title from the title
/// command, and the next action (`▸ fix CI #318`) or `✓ all 2 merged`;
/// and, as the trailing run of the first header, the poll age.
fn header_segments(
    state: &AppState,
    view: &PrsView,
    group: &PrGroup,
    first: bool,
    now_unix: i64,
) -> NodeSegments {
    let ascii = state.ascii;
    let dot = || TextSeg {
        dim: true,
        ..seg(" · ", SegRole::Muted)
    };
    let mut segments = vec![bold_seg(group.key.clone(), SegRole::Heading)];
    if let Some(title) = state.pr_group_title(&group.key) {
        segments.push(dot());
        segments.push(seg(title, SegRole::Repo));
    }
    match &group.headline {
        GroupHeadline::Next(step) => {
            segments.push(dot());
            segments.push(bold_seg(
                format!("{} {}", glyph(ascii, "▸", ">"), step.text()),
                pr_badge_role(step.state, false),
            ));
        }
        GroupHeadline::AllMerged(n) => {
            segments.push(dot());
            segments.push(seg(
                all_merged_text(ascii, *n),
                pr_badge_role(PrState::Merged, false),
            ));
        }
        GroupHeadline::Bare => {}
    }
    let trailing = match view.checked.filter(|_| first) {
        Some(checked) => {
            let secs = Instant::now()
                .saturating_duration_since(checked.at)
                .as_secs();
            let age = relative_age(now_unix, now_unix.saturating_sub_unsigned(secs));
            let role = if checked.failing {
                SegRole::Modified
            } else {
                SegRole::Muted
            };
            vec![seg(prs_checked_text(&age, checked.failing), role)]
        }
        None => Vec::new(),
    };
    NodeSegments { segments, trailing }
}

/// The active PRs tab in `area`: the list and the detail side by side
/// (the detail a third of the width, at least [`PRS_DETAIL_MIN_COLS`]), or
/// in a frame narrower than `PRS_DETAIL_MIN_TERM_COLS` the one `Tab` picked.
/// The list pane is on `sidebar` and the detail on `surface` in paint
/// mode, both through [`draw_pane_chrome`]. Records the pane rects, the PR
/// icon hits, and the detail URL link.
pub(super) fn draw_prs_tab(frame: &mut Frame<'_>, area: Rect, state: &mut AppState) {
    let Some(view) = state.sync_prs_view() else {
        return;
    };
    let show_detail = state.tabs.active_prs().is_some_and(|tab| tab.show_detail);
    let (list, detail) = if state.prs_narrow() {
        if show_detail {
            (None, Some(area))
        } else {
            (Some(area), None)
        }
    } else {
        let detail_w = (area.width / 3).max(PRS_DETAIL_MIN_COLS).min(area.width);
        let list = Rect {
            width: area.width - detail_w,
            ..area
        };
        let detail = Rect {
            x: area.x + list.width,
            width: detail_w,
            ..area
        };
        (Some(list), Some(detail))
    };
    let detail_focused = list.is_none();
    if let Some(list) = list {
        draw_prs_list(frame, list, state, &view);
    }
    if let Some(detail) = detail {
        draw_prs_detail(frame, detail, state, &view, detail_focused);
    }
}

/// The list pane: error and loading lines, then the empty copy or the
/// scrolled lines with the cursor kept in view, and the shared vertical
/// bar beside them while they overflow ([`ListBars`]).
fn draw_prs_list(frame: &mut Frame<'_>, area: Rect, state: &mut AppState, view: &PrsView) {
    let palette = state.theme.palette();
    let pane = draw_pane_chrome(frame, area, PRS_LIST_TITLE, true, palette.sidebar, state);
    let inner = pane.inner;
    state.layout.prs_list = inner;
    let mut pinned: Vec<Line<'static>> = view
        .errors
        .iter()
        .map(|line| {
            Line::from(Span::styled(
                line.clone(),
                Style::default().fg(palette.modified),
            ))
        })
        .collect();
    let (done, total) = view.lookups;
    if done < total {
        pinned.push(Line::from(Span::styled(
            prs_loading_text(done, total),
            Style::default().fg(palette.muted),
        )));
    }
    let pinned_h = u16::try_from(pinned.len())
        .unwrap_or(u16::MAX)
        .min(inner.height);
    frame.render_widget(
        Paragraph::new(pinned),
        Rect {
            height: pinned_h,
            ..inner
        },
    );
    let lines_area = Rect {
        y: inner.y + pinned_h,
        height: inner.height - pinned_h,
        ..inner
    };
    state.layout.prs_list_rows = Rect {
        y: lines_area.y,
        height: lines_area.height,
        ..pane.rows
    };
    if lines_area.width == 0 || lines_area.height == 0 {
        bleed_flat_pad(frame, pane);
        return;
    }
    if view.groups.is_empty() {
        if done >= total {
            let empty: Vec<Line<'static>> = PRS_EMPTY
                .iter()
                .enumerate()
                .map(|(index, text)| {
                    let fg = if index == 0 {
                        palette.file
                    } else {
                        palette.muted
                    };
                    Line::from(Span::styled(*text, Style::default().fg(fg)))
                })
                .collect();
            frame.render_widget(Paragraph::new(empty), lines_area);
        }
        bleed_flat_pad(frame, pane);
        return;
    }
    // The vertical bar takes the last column while the lines overflow;
    // the rows never pan, so there is no horizontal bar.
    let bars = ListBars::new(lines_area, view.lines.len(), 0);
    let rows_area = bars.rows_area();
    let height = usize::from(rows_area.height);
    let (start, _) = visible_window(view.lines.len(), view.cursor.unwrap_or(0), height);
    if let Some(tab) = state.tabs.active_prs_mut() {
        tab.scroll = start;
    }
    let width = usize::from(rows_area.width);
    let prefix = list_prefix_cols(0);
    let columns = RowColumns::of(state, view, width.saturating_sub(prefix));
    let search = state.theme.pills().filter;
    let now_unix = unix_now();
    let mut painted = Vec::new();
    for (index, line) in view.lines.iter().enumerate().skip(start).take(height) {
        let y = rows_area.y + (index - start) as u16;
        let selected = Some(index) == view.cursor;
        let (segs, foldable, folded, flash) = match *line {
            PrsLine::Header { group, folded } => {
                let segs = header_segments(state, view, &view.groups[group], group == 0, now_unix);
                (segs, true, folded, None)
            }
            PrsLine::Row { group, row } => {
                let row = &view.groups[group].rows[row];
                let (segments, hit_w) =
                    row_segments(state, row, &columns, width.saturating_sub(prefix));
                let x = rows_area.x + prefix as u16;
                let hit_w = u16::try_from(hit_w)
                    .unwrap_or(0)
                    .min(rows_area.right().saturating_sub(x));
                if hit_w > 0 {
                    state.layout.icon_hits.push(IconHit {
                        y,
                        x,
                        width: hit_w,
                        kind: pr_badge_kind(row.pr.state),
                        target: IconTarget::PullRequest(row.checkout.clone()),
                    });
                }
                let flash = state.flash_color(&prs_row_flash_id(&row.pr.url));
                let segs = NodeSegments {
                    segments,
                    trailing: Vec::new(),
                };
                (segs, false, false, flash)
            }
        };
        painted.push(paint_segmented_row(
            0,
            foldable,
            folded,
            &segs,
            width,
            selected,
            true,
            flash,
            false,
            search,
            state.ascii,
            palette,
            0,
        ));
    }
    frame.render_widget(Paragraph::new(painted), rows_area);
    let flat = state.background == BackgroundMode::Paint;
    let (vbar, _) = bars.paint(frame, start, (0, 0), flat);
    state.layout.prs_list_vbar = vbar;
    bleed_flat_pad(frame, pane);
}

/// The detail pane: the focused PR's title line, its PR lines, its
/// checkouts and repository, a rule, then the actions with their keys.
/// Lines past its height scroll ([`AppState::keep_prs_detail_scroll`]),
/// with the shared vertical bar in the last column while they overflow.
fn draw_prs_detail(
    frame: &mut Frame<'_>,
    area: Rect,
    state: &mut AppState,
    view: &PrsView,
    focused: bool,
) {
    let palette = state.theme.palette();
    let pane = draw_pane_chrome(
        frame,
        area,
        PRS_DETAIL_TITLE,
        focused,
        palette.surface,
        state,
    );
    let inner = pane.inner;
    state.layout.prs_detail = inner;
    state.layout.prs_detail_rows = pane.rows;
    let Some(row) = view.focused() else {
        bleed_flat_pad(frame, pane);
        return;
    };
    if inner.width == 0 || inner.height == 0 {
        bleed_flat_pad(frame, pane);
        return;
    }
    let ascii = state.ascii;
    let muted = Style::default().fg(palette.muted);
    let role = |role: SegRole| Style::default().fg(seg_role_color(role, palette));
    let mut fields = pr_detail_lines(state, &row.checkout, unix_now());
    for (nth, checkout) in view.checkouts_of(row).iter().enumerate() {
        let path = checkout.to_string_lossy().into_owned();
        let kind = state
            .snapshot
            .repos
            .iter()
            .find(|repo| repo.repo == path)
            .map(|repo| repo.checkout_kind);
        let kind = match kind {
            Some(CheckoutKind::Linked) => "worktree",
            _ => "primary",
        };
        fields.push(PopoverLine::Field {
            label: if nth == 0 { "checkout" } else { "" },
            value: format!("{path} · {kind}"),
            role: SegRole::Dir,
            link: false,
            bold: 0,
            mark: None,
        });
    }
    fields.push(PopoverLine::Field {
        label: "repo",
        value: row.repo.clone(),
        role: SegRole::Repo,
        link: false,
        bold: 0,
        mark: None,
    });
    let actions = [
        ("Enter", "Open PR"),
        ("gc", row.pr.ci_link.kind.label()),
        ("o", "Show in tree"),
        ("y", "Copy URL"),
    ];
    // The title, one line per field, the rule, and the actions; the
    // vertical bar takes the last column while they overflow, and the
    // viewport is the PR's own (another PR starts at the top).
    let shown_fields = fields
        .iter()
        .filter(|line| !matches!(line, PopoverLine::Rule | PopoverLine::Action { .. }))
        .count();
    let line_count = 1 + shown_fields + 1 + actions.len();
    let bars = ListBars::new(inner, line_count, 0);
    let body = bars.rows_area();
    let height = usize::from(body.height);
    let start = state.keep_prs_detail_scroll(&row.pr.url, line_count.saturating_sub(height));
    let width = usize::from(body.width);
    let mut lines: Vec<Line<'static>> = Vec::new();
    // Line index, column, painted width, and full URL of the `url` field.
    let mut link: Option<(usize, usize, usize, String)> = None;
    // `<glyph> <ref> <title>`.
    let mut title = vec![
        Span::styled(
            pr_badge_kind(row.pr.state).glyph(ascii).to_string(),
            role(pr_badge_role(row.pr.state, false)).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled(
            row.pr.ref_text(),
            role(SegRole::Repo).add_modifier(Modifier::BOLD),
        ),
    ];
    if !row.pr.title.is_empty() {
        title.push(Span::raw(" "));
        title.push(Span::styled(row.pr.title.clone(), role(SegRole::File)));
    }
    lines.push(Line::from(clamp_spans(title, width)));
    let label_w = fields
        .iter()
        .filter_map(|line| match line {
            PopoverLine::Field { label, .. } => Some(visible_width(label)),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    for line in &fields {
        let mut spans = Vec::new();
        match line {
            PopoverLine::Text(text) => {
                spans.push(Span::styled(text.clone(), role(SegRole::File)));
            }
            PopoverLine::Note(text) => spans.push(Span::styled(text.clone(), muted)),
            PopoverLine::Field {
                label,
                value,
                role: value_role,
                link: is_link,
                bold,
                mark,
            } => {
                let pad = label_w.saturating_sub(visible_width(label));
                spans.push(Span::styled(format!("{label}{} ", " ".repeat(pad)), muted));
                if let Some((mark, mark_role)) = mark {
                    spans.push(Span::styled(format!("{mark} "), role(*mark_role)));
                }
                let x = help_spans_width(&spans);
                let shown = fit_with_ellipsis(value, width.saturating_sub(x));
                // A cut URL keeps the full one in its link.
                if *is_link && !shown.is_empty() {
                    link = Some((lines.len(), x, visible_width(&shown), value.clone()));
                }
                let mut head = (*bold).min(shown.len());
                while !shown.is_char_boundary(head) {
                    head -= 1;
                }
                let (bold_text, rest) = shown.split_at(head);
                if !bold_text.is_empty() {
                    spans.push(Span::styled(
                        bold_text.to_string(),
                        role(*value_role).add_modifier(Modifier::BOLD),
                    ));
                }
                if !rest.is_empty() {
                    spans.push(Span::styled(rest.to_string(), role(*value_role)));
                }
            }
            PopoverLine::Rule | PopoverLine::Action { .. } => continue,
        }
        lines.push(Line::from(clamp_spans(spans, width)));
    }
    lines.push(Line::from(Span::styled(
        glyph(ascii, "─", "-").repeat(width),
        muted,
    )));
    let key_w = actions
        .iter()
        .map(|(key, _)| visible_width(key) + 2)
        .max()
        .unwrap_or(0);
    for (key, title) in actions {
        let chip = key_chip(key, palette.cursor, palette.panel);
        let pad = key_w.saturating_sub(chip.width());
        lines.push(Line::from(clamp_spans(
            vec![
                chip,
                Span::raw(" ".repeat(pad + 1)),
                Span::styled(title, role(SegRole::File)),
            ],
            width,
        )));
    }
    debug_assert_eq!(lines.len(), line_count, "the bar measures every line");
    // The `url` line links only while it is in the viewport.
    if let Some((y, x, shown_w, url)) = link.filter(|(y, ..)| (start..start + height).contains(y)) {
        if let (Ok(y), Ok(x), Ok(shown_w)) = (
            u16::try_from(y - start),
            u16::try_from(x),
            u16::try_from(shown_w),
        ) {
            state.layout.links.push(LinkHit {
                y: body.y + y,
                x: body.x + x,
                width: shown_w,
                url,
            });
        }
    }
    let shown: Vec<Line<'static>> = lines.into_iter().skip(start).take(height).collect();
    frame.render_widget(Paragraph::new(shown), body);
    let flat = state.background == BackgroundMode::Paint;
    let (vbar, _) = bars.paint(frame, start, (0, 0), flat);
    state.layout.prs_detail_vbar = vbar;
    bleed_flat_pad(frame, pane);
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::super::super::action::{Action, Effect};
    use super::super::super::pull_request::{PrLookup, PrPage};
    use super::super::super::state::prs_fixtures::{
        app_over, checkout, land, pr, workspace, GH_APP, GL_API,
    };
    use super::*;

    const LOOKS: [BackgroundMode; 2] = [BackgroundMode::Terminal, BackgroundMode::Paint];

    /// Paint `state` on a `cols` × `rows` frame; the terminal and its text,
    /// one line per row.
    fn paint(state: &mut AppState, cols: u16, rows: u16) -> (Terminal<TestBackend>, Vec<String>) {
        let mut terminal = Terminal::new(TestBackend::new(cols, rows)).unwrap();
        terminal.draw(|frame| draw(frame, state)).unwrap();
        let buf = terminal.backend().buffer().clone();
        let text = (0..rows)
            .map(|y| (0..cols).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        (terminal, text)
    }

    /// Row and text of the first painted line that contains `needle`.
    fn line_with<'a>(text: &'a [String], needle: &str) -> (u16, &'a str) {
        let y = text
            .iter()
            .position(|line| line.contains(needle))
            .unwrap_or_else(|| panic!("no line with {needle:?}:\n{}", text.join("\n")));
        (y as u16, text[y].as_str())
    }

    /// The columns of `rect` of each row of `text` (empty outside its rows).
    fn crop(text: &[String], rect: Rect) -> Vec<String> {
        text.iter()
            .enumerate()
            .map(|(y, line)| {
                if (rect.y..rect.bottom()).contains(&(y as u16)) {
                    line.chars()
                        .skip(usize::from(rect.x))
                        .take(usize::from(rect.width))
                        .collect()
                } else {
                    String::new()
                }
            })
            .collect()
    }

    /// The made-up workspace on its PRs tab.
    fn on_prs_tab(ascii: bool, background: BackgroundMode) -> AppState {
        let mut state = workspace(ascii);
        state.background = background;
        state.dispatch(Action::OpenPrsTab);
        state
    }

    /// Column of the first cell of `needle` in row `y` of `text`.
    fn col_of(text: &[String], y: u16, needle: &str) -> u16 {
        let line = &text[usize::from(y)];
        let byte = line.find(needle).expect("needle in the row");
        visible_width(&line[..byte]) as u16
    }

    #[test]
    fn the_prs_tab_paints_groups_rows_and_the_detail_in_both_looks() {
        for background in LOOKS {
            let what = format!("{background:?}");
            let mut state = on_prs_tab(false, background);
            state.dispatch(Action::Move(1));
            let (terminal, text) = paint(&mut state, 132, 30);
            let buf = terminal.backend().buffer();
            let palette = state.theme.palette();
            assert_eq!(text[0].trim_end(), " Workspace │ PRs 5 [✗]", "{what}");
            let (y, _) = line_with(&text, "prs");
            assert_eq!(y, 1, "{what}: the list pane title");
            assert!(text[1].contains("PR"), "{what}: the detail pane title");
            let list = state.layout.prs_list;
            let detail = state.layout.prs_detail;
            assert!(detail.width >= PRS_DETAIL_MIN_COLS, "{what}");
            assert!(
                detail.x > list.x + list.width,
                "{what}: detail on the right"
            );
            let left = crop(&text, list);
            let right = crop(&text, detail);

            // Group headers: key, next action, and the poll age.
            let (_, first) = line_with(&left, "ABC-123");
            assert!(first.contains("ABC-123 · ▸ fix CI #318"), "{what}: {first}");
            assert!(first.trim_end().ends_with("checked just now"), "{what}");
            let (_, waiting) = line_with(&left, "ABC-125");
            assert!(waiting.contains("ABC-125 · ▸ wait for CI !12"), "{what}");
            assert!(
                !waiting.contains("checked"),
                "{what}: only the first header"
            );
            assert!(line_with(&left, "ABC-120").1.contains("ABC-120 · ✓ merged"));
            assert_eq!(line_with(&left, "other").1.trim(), "▾ other", "{what}");

            // Rows: glyph, ref, repo, title, CI, reviews, state, stack.
            let (y318, failed) = line_with(&left, "✕ #318");
            for cell in [
                "✕ #318  acme/app      Retry failed uploads with…",
                "CI ✕",
                "CI failed",
            ] {
                assert!(failed.contains(cell), "{what}: {cell:?} in {failed}");
            }
            let (_, ready) = line_with(&left, "◉ #319");
            for cell in [
                "◉ #319  acme/app      Show the retry state",
                "CI ✓",
                "ready",
                "⧗ after #318",
            ] {
                assert!(ready.contains(cell), "{what}: {cell:?} in {ready}");
            }
            let (_, running) = line_with(&left, "◐ !12");
            for cell in [
                "◐ !12   acme/servic…  Rate limit the upload API",
                "CI …",
                "CI running",
            ] {
                assert!(running.contains(cell), "{what}: {cell:?} in {running}");
            }
            assert!(line_with(&left, "✓ #40").1.contains("✓ #40   acme/lib"));
            assert!(line_with(&left, "○ #320").1.contains("○ #320"));

            // The glyph is bold in its state colour; glyph and ref are one
            // PR icon hit and one OSC 8 link.
            let x = col_of(&text, y318, "✕ #318");
            assert_eq!(buf[(x, y318)].fg, palette.deleted, "{what}");
            assert!(buf[(x, y318)].modifier.contains(Modifier::BOLD), "{what}");
            let hit = state
                .layout
                .icon_hits
                .iter()
                .find(|hit| hit.y == y318)
                .expect("a PR icon hit")
                .clone();
            assert_eq!((hit.x, hit.width), (x, 6), "{what}");
            assert_eq!(hit.target, IconTarget::PullRequest(PathBuf::from("app")));
            let url = "https://github.com/acme/app/pull/318";
            assert!(
                state
                    .layout
                    .links
                    .iter()
                    .any(|link| link.y == y318 && link.x == x && link.url == url),
                "{what}: {:?}",
                state.layout.links
            );

            // The detail pane: the focused PR, its checkouts, the actions.
            let (_, head) = line_with(&right, "✕ #318 feat(ABC-123)");
            assert!(head.contains("✕ #318 feat(ABC-123): Retry"), "{what}");
            line_with(&right, "checkout app · primary");
            line_with(&right, "app-copy · primary");
            line_with(&right, "repo     acme/app");
            line_with(&right, " Enter  Open PR");
            line_with(&right, " gc     Open checks");
            line_with(&right, " o      Show in tree");
            line_with(&right, " y      Copy URL");
            let (url_y, _) = line_with(&right, "url      https://");
            assert!(
                state
                    .layout
                    .links
                    .iter()
                    .any(|link| link.y == url_y && link.url == url && link.x >= detail.x),
                "{what}: the url field links the full URL"
            );
        }
    }

    /// The wheel over a long detail scrolls it (the list cursor stays) up
    /// to its last page, the paint clamps a top the detail no longer
    /// needs, and another PR starts at the top, in both looks.
    #[test]
    fn the_detail_scrolls_with_the_wheel_clamps_and_starts_another_pr_at_the_top() {
        for background in LOOKS {
            let what = format!("{background:?}");
            let mut state = on_prs_tab(false, background);
            state.dispatch(Action::Move(1));
            let (_, text) = paint(&mut state, 132, 12);
            let detail = state.layout.prs_detail;
            let bar = state.layout.prs_detail_vbar;
            let x = bar.x.expect("the long detail paints its bar");
            assert_eq!(x, detail.right() - 1, "{what}: in the last column");
            assert_eq!((bar.y, bar.height), (detail.y, detail.height), "{what}");
            assert_eq!(bar.scroll, 0, "{what}");
            let max = usize::from(bar.scroll_max);
            assert_eq!(bar.content_len - usize::from(detail.height), max, "{what}");
            line_with(&crop(&text, detail), "✕ #318 feat(ABC-123)");

            let cursor = state.tabs.active_prs().unwrap().cursor_index;
            let rows = state.layout.prs_detail_rows;
            let wheel = |delta| Action::ScrollWheel {
                col: rows.x + 2,
                row: rows.y + 1,
                delta,
                horizontal: false,
            };
            state.dispatch(wheel(2));
            assert_eq!(state.prs_detail_scroll(), 2, "{what}: the wheel scrolls");
            assert_eq!(
                state.tabs.active_prs().unwrap().cursor_index,
                cursor,
                "{what}: the list cursor stays"
            );
            let (_, text) = paint(&mut state, 132, 12);
            assert_eq!(state.layout.prs_detail_vbar.scroll, 2, "{what}");
            let right = crop(&text, detail);
            assert!(
                !right.iter().any(|line| line.contains("✕ #318 feat")),
                "{what}: the title scrolled off"
            );
            state.dispatch(wheel(100));
            assert_eq!(state.prs_detail_scroll(), max, "{what}: stops at the end");
            let (_, text) = paint(&mut state, 132, 12);
            line_with(&crop(&text, detail), " y      Copy URL");
            state.dispatch(wheel(-100));
            assert_eq!(state.prs_detail_scroll(), 0, "{what}: and back to the top");

            // A taller frame needs a smaller top: the paint clamps it.
            state.dispatch(wheel(100));
            paint(&mut state, 132, 16);
            let taller = usize::from(state.layout.prs_detail_vbar.scroll_max);
            assert!(taller < max, "{what}: {taller} < {max}");
            assert_eq!(state.prs_detail_scroll(), taller, "{what}: clamped");
            assert_eq!(
                usize::from(state.layout.prs_detail_vbar.scroll),
                taller,
                "{what}"
            );

            // The next PR starts at the top, and so does this one again.
            state.dispatch(Action::Move(1));
            let (_, text) = paint(&mut state, 132, 12);
            assert_eq!(state.prs_detail_scroll(), 0, "{what}: another PR");
            line_with(&crop(&text, state.layout.prs_detail), "◉ #319");
            state.dispatch(wheel(2));
            state.dispatch(Action::Move(-1));
            let (_, text) = paint(&mut state, 132, 12);
            assert_eq!(state.prs_detail_scroll(), 0, "{what}: back on #318");
            line_with(&crop(&text, state.layout.prs_detail), "✕ #318 feat");
        }
    }

    /// The narrow detail (after `Tab`) is the focused pane: `j` / `k`, the
    /// page keys, and `gg` / `G` scroll it and leave the list cursor; back
    /// on the list (Esc) they move the cursor again. In both looks.
    #[test]
    fn the_narrow_detail_scrolls_with_the_move_keys() {
        use super::super::super::pr_status::PrState;
        for background in LOOKS {
            let what = format!("{background:?}");
            // One PR that eight checkouts show: a detail of 19 lines.
            let names: Vec<String> = (0..8).map(|i| format!("co{i}")).collect();
            let repos: Vec<_> = names.iter().map(|name| checkout(name, "topic")).collect();
            let mut state = app_over(&repos, false);
            state.background = background;
            let shared = pr(
                GH_APP,
                7,
                PrState::InReview,
                "Share one PR",
                "topic",
                "main",
            );
            for name in &names {
                land(&mut state, name, "topic", GH_APP, shared.clone());
            }
            state.dispatch(Action::OpenPrsTab);
            paint(&mut state, 90, 12);
            state.dispatch(Action::FocusRight);
            paint(&mut state, 90, 12);
            assert_eq!(
                state.layout.prs_list,
                Rect::default(),
                "{what}: detail only"
            );
            let max = usize::from(state.layout.prs_detail_vbar.scroll_max);
            let page = usize::from(state.layout.prs_detail.height) - 1;
            assert!(max > page, "{what}: a detail longer than a page ({max})");
            let cursor = state.tabs.active_prs().unwrap().cursor_index;
            state.dispatch(Action::Move(1));
            assert_eq!(state.prs_detail_scroll(), 1, "{what}: j");
            state.dispatch(Action::PageMove(1));
            assert_eq!(state.prs_detail_scroll(), 1 + page, "{what}: PageDown");
            state.dispatch(Action::Move(-1));
            assert_eq!(state.prs_detail_scroll(), page, "{what}: k");
            state.dispatch(Action::MoveToEnd);
            assert_eq!(state.prs_detail_scroll(), max, "{what}: G");
            state.dispatch(Action::PageMove(-1));
            assert_eq!(state.prs_detail_scroll(), max - page, "{what}: PageUp");
            state.dispatch(Action::MoveToStart);
            assert_eq!(state.prs_detail_scroll(), 0, "{what}: gg");
            assert_eq!(
                state.tabs.active_prs().unwrap().cursor_index,
                cursor,
                "{what}: the list cursor stays"
            );
            state.dispatch(Action::NavEsc);
            paint(&mut state, 90, 12);
            state.dispatch(Action::Move(1));
            assert_eq!(
                state.tabs.active_prs().unwrap().cursor_index,
                cursor + 1,
                "{what}: on the list the keys move the cursor"
            );
        }
    }

    #[test]
    fn a_narrow_frame_shows_the_list_or_the_detail() {
        for background in LOOKS {
            let what = format!("{background:?}");
            let mut state = on_prs_tab(false, background);
            let (_, text) = paint(&mut state, 90, 30);
            assert!(text[1].contains("prs"), "{what}");
            assert!(!text[1].contains("PR "), "{what}: no detail pane");
            assert_eq!(state.layout.prs_detail, Rect::default(), "{what}");
            line_with(&text, "Tab");
            state.dispatch(Action::FocusRight);
            let (_, text) = paint(&mut state, 90, 30);
            assert!(!text[1].contains("prs"), "{what}: no list pane");
            assert_eq!(state.layout.prs_list, Rect::default(), "{what}");
            assert!(
                state.layout.prs_detail.width > 80,
                "{what}: the whole width"
            );
            line_with(&text, "▸ fix CI");
            state.dispatch(Action::NavEsc);
            let (_, text) = paint(&mut state, 90, 30);
            assert!(text[1].contains("prs"), "{what}: Esc goes back");
        }
    }

    #[test]
    fn empty_loading_error_and_stale_states_use_their_copy() {
        for background in LOOKS {
            let what = format!("{background:?}");
            let repos = [
                checkout("api", "feature/ABC-125-limits"),
                checkout("app", "feature/ABC-123-retry"),
            ];
            let mut state = app_over(&repos, false);
            state.background = background;
            state.dispatch(Action::OpenPrsTab);
            let (_, text) = paint(&mut state, 132, 30);
            line_with(&text, "looking up PRs · 0 of 2 checkouts");

            for (name, branch, remote) in [
                ("api", "feature/ABC-125-limits", GL_API),
                ("app", "feature/ABC-123-retry", GH_APP),
            ] {
                assert!(state.apply_pr_lookup(
                    Path::new(name),
                    branch,
                    Some(remote.into()),
                    PrLookup::NoPr
                ));
            }
            let (terminal, text) = paint(&mut state, 132, 30);
            for copy in PRS_EMPTY {
                line_with(&text, copy);
            }
            assert!(
                !text.iter().any(|line| line.contains("looking up")),
                "{what}"
            );

            // A failing glab names its host; the GitHub PR still lists,
            // and an old answer makes the header say lookups fail.
            assert!(state.apply_pr_lookup(
                Path::new("api"),
                "feature/ABC-125-limits",
                Some(GL_API.into()),
                PrLookup::Failed(Some("401 Unauthorized".into())),
            ));
            let hour_ago = Instant::now() - std::time::Duration::from_secs(3600);
            assert!(state.apply_pr_lookup_at(
                Path::new("app"),
                "feature/ABC-123-retry",
                Some(GH_APP.into()),
                PrLookup::Found(pr(
                    GH_APP,
                    318,
                    PrState::InReview,
                    "Retry",
                    "feature/ABC-123-retry",
                    "main"
                )),
                hour_ago,
            ));
            drop(terminal);
            let (terminal, text) = paint(&mut state, 132, 30);
            let buf = terminal.backend().buffer();
            let palette = state.theme.palette();
            let (y, error) = line_with(&text, "! glab (gitlab.com): 401 Unauthorized");
            let x = col_of(&text, y, "! glab");
            assert_eq!(buf[(x, y)].fg, palette.modified, "{what}: {error}");
            let (y, header) = line_with(&text, "ABC-123");
            assert!(
                header.contains("checked 1h ago · lookups failing"),
                "{what}: {header}"
            );
            let x = col_of(&text, y, "checked");
            assert_eq!(buf[(x, y)].fg, palette.modified, "{what}");
            // The stale row paints its glyph and ref muted, shape unchanged.
            let (y, _) = line_with(&crop(&text, state.layout.prs_list), "○ #318");
            let x = col_of(&text, y, "○ #318");
            assert_eq!(buf[(x, y)].fg, palette.muted, "{what}: stale glyph");
            assert_eq!(buf[(x + 2, y)].fg, palette.muted, "{what}: stale ref");
        }
    }

    #[test]
    fn ascii_mode_paints_letters() {
        let mut state = on_prs_tab(true, BackgroundMode::Terminal);
        state.dispatch(Action::Move(1));
        let (_, text) = paint(&mut state, 132, 30);
        for cell in [
            "x #318",
            "CI x",
            "A #319",
            "CI ok",
            "after #318",
            "r !12",
            "CI ..",
            "m #40",
            "P #320",
            "CI -",
            "ABC-123 · > fix CI #318",
            "ABC-120 · m merged",
        ] {
            line_with(&text, cell);
        }
        assert!(!text.iter().any(|line| line.contains('⧗')));
    }

    #[test]
    fn the_mouse_selects_rows_folds_headers_and_opens_the_pr() {
        for background in LOOKS {
            let what = format!("{background:?}");
            let mut state = on_prs_tab(false, background);
            let (_, text) = paint(&mut state, 132, 30);
            let (y319, _) = line_with(&text, "◉ #319  ");
            let x = state.layout.prs_list.x + 30;
            assert_eq!(
                state.dispatch(Action::Click { col: x, row: y319 }),
                Effect::None
            );
            let (_, text) = paint(&mut state, 132, 30);
            line_with(&text, "◉ #319 feat(ABC-123): Show the retry state");
            // A header press folds its group; a second press on the same
            // cell (a double-click) does not undo it.
            let (y, _) = line_with(&text, "ABC-125");
            state.dispatch(Action::Click { col: x, row: y });
            state.dispatch(Action::Click { col: x, row: y });
            let (_, text) = paint(&mut state, 132, 30);
            let left = crop(&text, state.layout.prs_list);
            assert!(!left.iter().any(|line| line.contains("◐ !12")), "{what}");
            // Another cell is a new press: it unfolds.
            state.dispatch(Action::Click { col: x + 1, row: y });
            let (_, text) = paint(&mut state, 132, 30);
            line_with(&text, "◐ !12   ");
            // The wheel moves the cursor.
            state.dispatch(Action::ScrollWheel {
                col: x,
                row: y,
                delta: 1,
                horizontal: false,
            });
            let (_, text) = paint(&mut state, 132, 30);
            line_with(&text, "◐ !12 ABC-125: Rate limit the upload API");
            // Ctrl+click on a ref opens its PR.
            let (y318, _) = line_with(&text, "✕ #318  ");
            let hit = state
                .layout
                .icon_hits
                .iter()
                .find(|hit| hit.y == y318)
                .unwrap()
                .clone();
            assert_eq!(
                state.dispatch(Action::CtrlClick {
                    col: hit.x + 3,
                    row: y318
                }),
                Effect::OpenPullRequest {
                    repo: PathBuf::from("app"),
                    branch: "feature/ABC-123-retry".into(),
                    page: PrPage::Pr,
                },
                "{what}"
            );
            assert!(state.is_prs_tab(), "{what}");
            // A plain click on a ref selects its row and pins its popover.
            paint(&mut state, 132, 30);
            state.dispatch(Action::Click {
                col: hit.x + 3,
                row: y318,
            });
            assert!(state.popover_pinned(), "{what}");
            let (_, text) = paint(&mut state, 132, 30);
            line_with(&text, "PR · CI failed");
        }
    }

    #[test]
    fn a_click_on_the_workspace_pr_total_opens_the_prs_tab() {
        for background in LOOKS {
            let mut state = workspace(false);
            state.background = background;
            let (_, text) = paint(&mut state, 200, 30);
            let hit = state.layout.pr_total_hit.expect("the total painted");
            let painted: String = text[usize::from(hit.y)]
                .chars()
                .skip(usize::from(hit.x))
                .take(usize::from(hit.width))
                .collect();
            assert_eq!(painted, "5 PRs · 2 need you", "{background:?}");
            state.dispatch(Action::Click {
                col: hit.x + 1,
                row: hit.y,
            });
            assert!(state.is_prs_tab(), "{background:?}");
            paint(&mut state, 132, 30);
            assert_eq!(state.layout.pr_total_hit, None, "only the tree paints it");
        }
    }

    /// At every frame width the hit covers exactly the painted total, so a
    /// row with room for one part keeps `2 need you` and its cells, on a
    /// wide and on a narrow tree, and a click on them opens the PRs tab.
    #[test]
    fn the_pr_total_hit_follows_the_part_that_fits() {
        for background in LOOKS {
            // `need you` alone beside a change summary on a wide and on a
            // narrow tree, and with no change summary left beside it.
            let mut need_you_only = [false; 2];
            let mut bare = false;
            for cols in 40..=200 {
                let what = format!("{background:?} {cols} columns");
                let mut state = workspace(false);
                state.background = background;
                let (_, text) = paint(&mut state, cols, 30);
                let left = usize::from(state.layout.tree_x);
                let right = left + usize::from(state.layout.tree_width);
                let row: Vec<char> = text[usize::from(state.layout.tree_y)].chars().collect();
                let tree_row: String = row[left..right].iter().collect();
                let total_painted = tree_row.contains("need you") || tree_row.contains(" PRs");
                let Some(hit) = state.layout.pr_total_hit else {
                    assert!(
                        !total_painted,
                        "{what}: a painted total has no click area: {tree_row:?}"
                    );
                    continue;
                };
                assert_eq!(hit.y, state.layout.tree_y, "{what}: the workspace row");
                let painted: String = row[usize::from(hit.x)..usize::from(hit.right())]
                    .iter()
                    .collect();
                assert_eq!(row[usize::from(hit.x) - 1], ' ', "{what}: {painted:?}");
                assert_eq!(usize::from(hit.right()), right, "{what}: flush right");
                match painted.as_str() {
                    "5 PRs · 2 need you" => continue,
                    "2 need you" => {}
                    other => panic!("{what}: the hit covers {other:?}"),
                }
                let before: String = row[left..usize::from(hit.x)].iter().collect();
                if before.trim_end().ends_with("tmp") {
                    bare = true;
                } else {
                    let narrow = usize::from(state.layout.tree_width) < PR_REF_MIN_TREE_COLS;
                    need_you_only[usize::from(narrow)] = true;
                }
                state.dispatch(Action::Click {
                    col: hit.x,
                    row: hit.y,
                });
                assert!(state.is_prs_tab(), "{what}");
            }
            assert_eq!(need_you_only, [true, true], "{background:?}: wide, narrow");
            assert!(bare, "{background:?}: `need you` with no change summary");
        }
    }

    #[test]
    fn a_pr_change_flashes_its_row_on_the_prs_tab() {
        let mut state = on_prs_tab(false, BackgroundMode::Terminal);
        let now = Instant::now();
        land(
            &mut state,
            "app-docs",
            "docs/readme",
            GH_APP,
            pr(
                GH_APP,
                320,
                PrState::Ready,
                "Update the README",
                "docs/readme",
                "main",
            ),
        );
        assert!(state.alert_pr_transitions(now));
        let (terminal, text) = paint(&mut state, 132, 30);
        let (y, _) = line_with(&text, "◉ #320  ");
        let x = col_of(&text, y, "Update");
        let flash = state
            .flash_color(&prs_row_flash_id("https://github.com/acme/app/pull/320"))
            .expect("flashing");
        assert_eq!(terminal.backend().buffer()[(x, y)].bg, flash);
        let x = col_of(&text, y, "◉");
        assert_eq!(
            terminal.backend().buffer()[(x, y)].fg,
            state.theme.palette().added,
            "the glyph keeps its state colour"
        );
    }

    #[test]
    fn rows_keep_their_title_and_drop_optional_columns_as_the_pane_narrows() {
        // Glyph and ref, a title fragment, and the state label of each row.
        let rows = [
            ("✕ #318", "Retry fail", "CI failed"),
            ("◉ #319", "Show the r", "ready"),
            ("◐ !12", "Rate limit", "CI running"),
            ("✓ #40", "Cache the ", "merged"),
            ("○ #320", "Update the", "in review"),
        ];
        // Frame width, then whether the reviews, the stack mark, and the
        // `CI ` text paint.
        let cases = [(80, true), (100, false), (120, true)];
        for background in LOOKS {
            for (cols, all) in cases {
                let what = format!("{background:?} {cols} columns");
                let mut state = on_prs_tab(false, background);
                let (_, text) = paint(&mut state, cols, 30);
                let list = crop(&text, state.layout.prs_list);
                for (head, title, label) in rows {
                    let (_, line) = line_with(&list, head);
                    let at = |part: &str| {
                        line.find(part)
                            .unwrap_or_else(|| panic!("{what}: no {part:?} in {line:?}"))
                    };
                    // In order, each whole: nothing overlaps or is cut.
                    assert!(at(head) < at(title), "{what}: {line}");
                    assert!(at(title) < at(label), "{what}: {line}");
                }
                let (_, ready) = line_with(&list, "◉ #319");
                let (_, failed) = line_with(&list, "✕ #318");
                assert_eq!(ready.contains("⧗ after #318"), all, "{what}: {ready}");
                if all {
                    assert!(failed.contains("CI ✕  —  CI failed"), "{what}: {failed}");
                } else {
                    // Reviews, then the stack, then `CI ` gave way; the
                    // mark stays.
                    assert!(!failed.contains("CI ✕"), "{what}: {failed}");
                    assert!(failed.contains("  ✕  CI failed"), "{what}: {failed}");
                }
            }
        }
    }

    #[test]
    fn gh_pins_the_popover_of_the_focused_pr_row() {
        for background in LOOKS {
            let what = format!("{background:?}");
            let mut state = on_prs_tab(false, background);
            state.dispatch(Action::Move(1));
            paint(&mut state, 132, 30);
            state.dispatch(Action::PopoverOpenFocused);
            assert!(state.popover_pinned(), "{what}");
            let (_, text) = paint(&mut state, 132, 30);
            line_with(&text, "PR · CI failed");
            // The wheel stays live under a pinned popover: moving the
            // cursor to another row closes it (its owner is the row).
            let rows = state.layout.prs_list_rows;
            state.dispatch(Action::ScrollWheel {
                col: rows.x + 10,
                row: rows.y + 1,
                delta: 1,
                horizontal: false,
            });
            assert!(!state.popover_pinned(), "{what}: the cursor moved on");
            let (_, text) = paint(&mut state, 132, 30);
            assert!(
                !text.iter().any(|line| line.contains("PR · CI failed")),
                "{what}: the paint drops it"
            );
            line_with(&text, "◉ #319 feat(ABC-123): Show the retry state");
        }
    }
}

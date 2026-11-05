//! GraphPane chrome: header / list / selection footer.
//!
//! Header / footer budget (`graph_chrome_budget`) and selection footer copy.
//! The footer height follows the expand state, the message rows, the pane
//! size, and the selected row's wrapped meta rows; never the length of the
//! selected message.
//! Footer ref chips are the same [`LabelPart`] runs as the commit spacer,
//! with the same [`LabelPart::target`]s. A worktree row's branch run is a
//! local branch chip.

use crate::format::{
    commit_ref_chip_parts, format_commit_message, format_relative_date, is_default_branch,
    parts_text, short_id, trunc_label_parts, wrap_commit_message, wrap_label_parts, LabelKind,
    LabelPart, PartTarget, COMMIT_MSG_LINES_MAX, COMMIT_MSG_LINES_MIN,
};
use crate::glyphs::GlyphSet;
use crate::model::{GraphModel, GraphRef, GraphRow, RefKind};

/// Status-line copy while the next log page loads. The graph pane paints no
/// loading row, so the list keeps its height.
pub const LOADING_OLDER: &str = "loading older…";

/// Selection footer lines when no row is focused.
pub const FOOTER_NO_SELECTION: &str = "no selection";

/// Uncommitted footer meta.
pub const FOOTER_WORKTREE_NOT_A_COMMIT: &str = "worktree · not a commit";

/// Spacer footer meta.
pub const FOOTER_CONNECTOR_NOT_SELECTABLE: &str = "connector · not selectable";

/// Commit footer when the commit has no ref chips.
pub const FOOTER_NO_REFS: &str = "(no refs)";

/// Commit footer parents group when the commit has no parents.
pub const FOOTER_ROOT_COMMIT: &str = "root commit";

/// Spacer footer subject.
pub const FOOTER_SPACER_SUBJECT: &str = "…";

/// What GraphPane's selection footer describes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GraphFooterSelection<'a> {
    /// No focused list row.
    None,
    /// A [`GraphRow`] from [`GraphModel::visible_rows`].
    Row(&'a GraphRow),
    /// Spacer under a commit or stash (not in `visible_rows`).
    Spacer,
}

impl<'a> From<Option<&'a GraphRow>> for GraphFooterSelection<'a> {
    fn from(row: Option<&'a GraphRow>) -> Self {
        match row {
            Some(row) => Self::Row(row),
            None => Self::None,
        }
    }
}

/// How many header / footer / list rows GraphPane should reserve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GraphChromeBudget {
    /// Paint the sync header.
    pub header: bool,
    /// Paint the selection footer.
    pub footer: bool,
    /// Footer rows (0 when [`Self::footer`] is false), the
    /// [`FOOTER_RULE_ROWS`] rule row on top included. Collapsed with a
    /// one-row meta line is 3: the rule, the subject, and the meta row.
    pub footer_height: u16,
    /// Rows of the wrapped meta line at the bottom of the footer.
    ///
    /// `0` without a footer, else at least 1.
    pub meta_rows: u16,
    /// Rows left for the commit list (at least 1).
    pub list_height: u16,
}

impl GraphChromeBudget {
    /// Footer rows under the rule: the message rows plus the meta rows.
    ///
    /// `0` when the footer is not painted, or when it has room for the
    /// rule only. [`SelectionFooter::scroll_max`] takes this height.
    pub fn footer_body_height(&self) -> u16 {
        self.footer_height.saturating_sub(FOOTER_RULE_ROWS)
    }

    /// Message rows of the footer: the body less the meta rows.
    ///
    /// `0` without a footer, or when the meta rows take the whole body.
    pub fn message_rows(&self) -> u16 {
        self.footer_body_height().saturating_sub(self.meta_rows)
    }
}

/// Rows of the rule that tops the selection footer and parts it from the
/// commit list. It counts inside [`GraphChromeBudget::footer_height`] and
/// [`graph_footer_request`].
pub const FOOTER_RULE_ROWS: u16 = 1;

/// Requested selection-footer height for [`graph_chrome_budget_for`].
///
/// The [`FOOTER_RULE_ROWS`] rule row, then the body. Expanded body is
/// `msg_lines` message rows (clamped to
/// [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`]) plus the meta row.
/// Collapsed body is 2 (subject + meta), so collapsed asks for 3. The value
/// never depends on the selected row, and it counts one meta row:
/// [`graph_chrome_budget_for_meta`] adds the selected row's extra wrapped
/// meta rows, so the list height follows the meta row count at the pane
/// width, never the message length. The app and [`crate::GraphWidget`]
/// both call this so layout and paint agree.
pub fn graph_footer_request(expand: bool, msg_lines: usize) -> u16 {
    let body = if expand {
        msg_lines.clamp(COMMIT_MSG_LINES_MIN, COMMIT_MSG_LINES_MAX) as u16 + 1
    } else {
        2
    };
    FOOTER_RULE_ROWS + body
}

/// Max message scroll for a selection footer of `line_count` lines painted
/// in `body_height` rows (the footer under its rule,
/// [`GraphChromeBudget::footer_body_height`]).
///
/// The last line (meta) stays pinned under the message viewport, so this is
/// the scroll max of the other lines in the rows above it, the same value
/// [`crate::paint_footer_message`] reports for that viewport. `0` when every
/// line fits.
pub fn footer_message_scroll_max(line_count: usize, body_height: u16) -> usize {
    crate::footer::footer_message_rows_scroll_max(
        line_count.saturating_sub(1),
        body_height.saturating_sub(1),
    )
}

/// Footer first, then header. Collapsed footer: the rule, subject, and
/// meta rows.
pub fn graph_chrome_budget(height: u16, want_header: bool) -> GraphChromeBudget {
    graph_chrome_budget_for(height, want_header, graph_footer_request(false, 0))
}

/// Like [`graph_chrome_budget`] with a requested footer height.
///
/// `footer_lines` is the fixed requested footer height from
/// [`graph_footer_request`] (the rule row, then expanded: message rows plus
/// the meta row; collapsed: 2), never the selected message's line count.
/// The body under the rule gets its requested height, at most half the
/// pane (never under 2); the rule row comes on top. A message taller than
/// the body scrolls ([`footer_message_scroll_max`]); a shorter one leaves
/// blank rows. The list keeps at least one row, which can cut the body
/// short on a small pane (a 3-row pane: list, rule, meta). A pane shorter
/// than 3 rows still drops the footer. The result depends only on the pane
/// and the request. This is [`graph_chrome_budget_for_meta`] with one meta
/// row.
pub fn graph_chrome_budget_for(
    height: u16,
    want_header: bool,
    footer_lines: u16,
) -> GraphChromeBudget {
    graph_chrome_budget_for_meta(height, want_header, footer_lines, 1)
}

/// Like [`graph_chrome_budget_for`] with `meta_rows` wrapped meta rows
/// (0 counts as 1).
///
/// `meta_rows` is [`SelectionFooter::meta_rows`] of the selected row at the
/// pane width. The meta rows come first: they take body rows up to the
/// room that keeps one list row. The message rows keep the share
/// [`graph_chrome_budget_for`] gives them (the request less its rule and
/// one meta row, at most half the pane less one row) while the room left
/// under the meta rows allows, down to 0 on a tiny pane. So extra meta rows
/// grow the footer and come out of the list. Meta rows wider than the room
/// leave no message rows and the list one row; the footer paint cuts the
/// meta line with `…`.
pub fn graph_chrome_budget_for_meta(
    height: u16,
    want_header: bool,
    footer_lines: u16,
    meta_rows: u16,
) -> GraphChromeBudget {
    let mut avail = height.max(1);
    let footer = avail >= 3;
    let (footer_height, meta_rows) = if footer {
        // Body rows that keep one list row.
        let room = avail - 1 - FOOTER_RULE_ROWS;
        let meta = meta_rows.max(1).min(room);
        // The request less its rule and its one meta row.
        let msg_want = footer_lines.max(2 + FOOTER_RULE_ROWS) - FOOTER_RULE_ROWS - 1;
        // The message share of the half-pane body cap.
        let msg_cap = (avail / 2).max(2) - 1;
        let msg = msg_want.min(msg_cap).min(room - meta);
        let h = FOOTER_RULE_ROWS + meta + msg;
        avail -= h;
        (h, meta)
    } else {
        (0, 0)
    };
    let header = want_header && avail >= 2;
    if header {
        avail = avail.saturating_sub(1);
    }
    GraphChromeBudget {
        header,
        footer,
        footer_height,
        meta_rows,
        list_height: avail.max(1),
    }
}

/// Most expanded-footer message rows a graph pane `height` rows tall shows,
/// in [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`].
///
/// [`graph_chrome_budget_for`] caps the footer at half the pane and keeps
/// one list row, so a larger request paints no more rows. A footer resize
/// clamps here so it never stores rows the pane cannot show. The header
/// does not change the footer, so it is left out.
pub fn commit_msg_lines_fit(height: u16) -> usize {
    let most = graph_footer_request(true, COMMIT_MSG_LINES_MAX);
    // Body rows less the pinned meta row.
    let rows = graph_chrome_budget_for(height, false, most)
        .footer_body_height()
        .saturating_sub(1);
    usize::from(rows).clamp(COMMIT_MSG_LINES_MIN, COMMIT_MSG_LINES_MAX)
}

/// Two selection-detail lines (subject + meta). Truncated to `width`.
pub fn selection_detail_lines(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
) -> [String; 2] {
    let [subject, meta] = selection_detail_parts(model, selection, glyphs, width, now_unix);
    [parts_text(&subject), parts_text(&meta)]
}

/// Styled selection-footer runs. Chip kinds match the commit spacer so
/// [`crate::GraphWidget::label_palette`] can reuse row colours (HEAD /
/// default / local / remote / tag). The commit / stash hash is
/// [`LabelKind::MetaHash`] `{ parent: None }` and each parent short id
/// `{ parent: Some(n) }` (the `parent` / `parents` label stays
/// [`LabelKind::Meta`]); the author run is [`LabelKind::MetaAuthor`] and
/// the date run [`LabelKind::MetaDate`] (italic, same colour as meta).
/// Commit meta order: chips · hash · parents · author · date.
pub fn selection_detail_parts(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
) -> [Vec<LabelPart>; 2] {
    let width = width.max(1);
    let meta = selection_meta_line(model, selection, glyphs, now_unix);
    [
        subject_parts(selection_subject(selection), width),
        trunc_label_parts(&meta, width),
    ]
}

/// Subject text of the selection footer.
fn selection_subject(selection: GraphFooterSelection<'_>) -> &str {
    match selection {
        GraphFooterSelection::None => FOOTER_NO_SELECTION,
        GraphFooterSelection::Spacer => FOOTER_SPACER_SUBJECT,
        GraphFooterSelection::Row(GraphRow::Uncommitted { has_changes: true }) => {
            "Uncommitted changes"
        }
        GraphFooterSelection::Row(GraphRow::Uncommitted { has_changes: false }) => {
            "Working tree clean"
        }
        GraphFooterSelection::Row(GraphRow::Stash(stash)) => &stash.subject,
        GraphFooterSelection::Row(GraphRow::Worktree(wt)) => &wt.path,
        GraphFooterSelection::Row(GraphRow::Commit { commit, .. }) => &commit.subject,
    }
}

/// The whole selection-footer meta line, not cut to a width (see
/// [`selection_detail_parts`] for the runs and their kinds).
fn selection_meta_line(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    now_unix: i64,
) -> Vec<LabelPart> {
    match selection {
        GraphFooterSelection::None => Vec::new(),
        GraphFooterSelection::Spacer => {
            vec![meta_part(FOOTER_CONNECTOR_NOT_SELECTABLE.to_string())]
        }
        GraphFooterSelection::Row(GraphRow::Uncommitted { .. }) => {
            head_commit_ref_parts(model, glyphs)
                .unwrap_or_else(|| vec![meta_part(FOOTER_WORKTREE_NOT_A_COMMIT.to_string())])
        }
        GraphFooterSelection::Row(GraphRow::Stash(stash)) => join_meta_groups([
            vec![meta_part(stash.stash_ref.clone())],
            vec![hash_part(short_id(&stash.id).to_string(), None)],
            vec![date_part(format_relative_date(
                stash.author_date_unix,
                now_unix,
            ))],
        ]),
        GraphFooterSelection::Row(GraphRow::Worktree(wt)) => match wt.branch.as_deref() {
            Some(branch) if !branch.is_empty() => vec![LabelPart {
                text: branch.to_string(),
                kind: if is_default_branch(branch, model.default_branch_override.as_deref()) {
                    LabelKind::ChipDefault
                } else {
                    LabelKind::ChipLocal
                },
                target: Some(PartTarget::Chip(vec![GraphRef {
                    kind: RefKind::Local,
                    name: branch.to_string(),
                }])),
            }],
            _ => Vec::new(),
        },
        GraphFooterSelection::Row(GraphRow::Commit {
            commit, is_head, ..
        }) => {
            let chips = commit_ref_chip_parts(
                &commit.refs,
                *is_head,
                model.sync.as_ref().map(|s| s.branch.as_str()),
                glyphs,
                model.default_branch_override.as_deref(),
            );
            let mut groups: Vec<Vec<LabelPart>> = Vec::new();
            if chips.is_empty() {
                groups.push(vec![LabelPart {
                    text: FOOTER_NO_REFS.into(),
                    kind: LabelKind::Meta,
                    target: None,
                }]);
            } else {
                groups.push(chips);
            }
            groups.push(vec![hash_part(short_id(&commit.id).to_string(), None)]);
            groups.push(parents_parts(&commit.parents));
            if !commit.author_name.is_empty() {
                groups.push(vec![author_part(commit.author_name.clone())]);
            }
            if commit.author_date_unix > 0 {
                groups.push(vec![date_part(format_relative_date(
                    commit.author_date_unix,
                    now_unix,
                ))]);
            }
            join_meta_groups(groups)
        }
    }
}

/// Selection-footer content before layout: the message lines and the
/// whole meta line.
///
/// [`crate::paint_selection_footer`] paints it: the message rows scroll
/// on top and the meta line wraps at the footer width onto the rows under
/// them ([`Self::meta_rows`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectionFooter {
    /// Message lines.
    ///
    /// Collapsed: the subject cut to the width. Expanded: the subject and
    /// body wrapped one column short of the width (that column holds the
    /// footer scrollbar), with no line cap.
    pub message: Vec<Vec<LabelPart>>,
    /// The whole meta line, not cut to a width.
    ///
    /// Paint wraps it at the footer width. Same runs and kinds as the meta
    /// line of [`selection_detail_parts`].
    pub meta: Vec<LabelPart>,
}

impl SelectionFooter {
    /// Rows the meta line wraps to at `width` (at least 1).
    ///
    /// An empty meta line still owns one blank row.
    pub fn meta_rows(&self, width: usize) -> usize {
        wrap_label_parts(&self.meta, width).len()
    }

    /// Message lines, then the meta rows at `width`.
    ///
    /// What the footer paints when it has a row for every line.
    pub fn lines(&self, width: usize) -> Vec<Vec<LabelPart>> {
        let mut lines = self.message.clone();
        lines.extend(wrap_label_parts(&self.meta, width));
        lines
    }

    /// Max message scroll in a footer body of `body_height` rows at
    /// `width`.
    ///
    /// The meta rows stay pinned at the bottom
    /// (`min(meta_rows, body_height)` of them), so this is the message
    /// lines less the rows above them. It is the value
    /// [`crate::paint_selection_footer`] paints with. `0` when every
    /// message line fits.
    pub fn scroll_max(&self, width: usize, body_height: u16) -> usize {
        let meta = self.meta_rows(width).min(usize::from(body_height));
        let rows = usize::from(body_height) - meta;
        crate::footer::footer_message_rows_scroll_max(
            self.message.len(),
            u16::try_from(rows).unwrap_or(u16::MAX),
        )
    }
}

/// Selection-footer content of `selection` at `width`.
///
/// Collapsed (`expand` false), the message is the subject cut to `width`.
/// Expanded, it is the whole subject plus body of a commit or stash,
/// wrapped one column short of `width` (that column holds the footer
/// scrollbar), with no line cap: the footer scrolls a message taller than
/// its rows. Other rows keep the subject line. The meta line is never cut
/// (see [`SelectionFooter::meta`]). Graph list rows stay one line either
/// way.
pub fn selection_footer(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
    expand: bool,
) -> SelectionFooter {
    let width = width.max(1);
    let meta = selection_meta_line(model, selection, glyphs, now_unix);
    let subject = subject_parts(selection_subject(selection), width);
    let message_source = match selection {
        GraphFooterSelection::Row(GraphRow::Commit { commit, .. }) if expand => {
            Some((commit.subject.as_str(), commit.body.as_str()))
        }
        GraphFooterSelection::Row(GraphRow::Stash(stash)) if expand => {
            Some((stash.subject.as_str(), stash.body.as_str()))
        }
        _ => None,
    };
    let Some((message_subject, body)) = message_source else {
        return SelectionFooter {
            message: vec![subject],
            meta,
        };
    };
    let text = format_commit_message(message_subject, body);
    let wrapped = wrap_commit_message(&text, width.saturating_sub(1).max(1), usize::MAX);
    let mut message: Vec<Vec<LabelPart>> = wrapped
        .into_iter()
        .map(|text| {
            vec![LabelPart {
                text,
                kind: LabelKind::Subject,
                target: None,
            }]
        })
        .collect();
    if message.is_empty() {
        message.push(subject);
    }
    SelectionFooter { message, meta }
}

/// Selection-footer runs: [`selection_footer`] laid out with a row for
/// every line ([`SelectionFooter::lines`]).
///
/// Collapsed is the subject cut to `width`, then the meta rows. Expanded
/// wraps the whole subject plus body one column short of `width` (that
/// column holds the footer scrollbar), then the same meta rows. There is no
/// line cap: the widget scrolls a message taller than the footer. Graph
/// list rows stay one line either way.
pub fn selection_footer_parts(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
    expand: bool,
) -> Vec<Vec<LabelPart>> {
    let width = width.max(1);
    selection_footer(model, selection, glyphs, width, now_unix, expand).lines(width)
}

/// Text lines for [`selection_footer_parts`].
pub fn selection_footer_lines(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
    expand: bool,
) -> Vec<String> {
    selection_footer_parts(model, selection, glyphs, width, now_unix, expand)
        .iter()
        .map(|parts| parts_text(parts))
        .collect()
}

fn head_commit_ref_parts(model: &GraphModel, glyphs: &GlyphSet) -> Option<Vec<LabelPart>> {
    let id = model.head_id.as_deref()?;
    let commit = model.commits.iter().find(|c| c.id == id)?;
    let chips = commit_ref_chip_parts(
        &commit.refs,
        true,
        model.sync.as_ref().map(|s| s.branch.as_str()),
        glyphs,
        model.default_branch_override.as_deref(),
    );
    if chips.is_empty() {
        None
    } else {
        Some(chips)
    }
}

fn subject_parts(text: &str, width: usize) -> Vec<LabelPart> {
    trunc_label_parts(
        &[LabelPart {
            text: text.to_string(),
            kind: LabelKind::Subject,
            target: None,
        }],
        width,
    )
}

/// Commit footer parents group: `parent <id>`, `parents <id> <id> …`
/// (git parent order), or [`FOOTER_ROOT_COMMIT`]. The label and the blanks
/// are meta runs; each short id is its own
/// [`LabelKind::MetaHash`] run naming its parent index.
fn parents_parts(parents: &[String]) -> Vec<LabelPart> {
    let label = match parents.len() {
        0 => return vec![meta_part(FOOTER_ROOT_COMMIT.to_string())],
        1 => "parent ",
        _ => "parents ",
    };
    let mut parts = vec![meta_part(label.to_string())];
    for (i, id) in parents.iter().enumerate() {
        if i > 0 {
            parts.push(meta_part(" ".to_string()));
        }
        parts.push(hash_part(short_id(id).to_string(), Some(i)));
    }
    parts
}

fn meta_part(text: String) -> LabelPart {
    LabelPart {
        text,
        kind: LabelKind::Meta,
        target: None,
    }
}

/// A short commit id run: the row's own (`parent: None`) or parent `n`.
fn hash_part(text: String, parent: Option<usize>) -> LabelPart {
    LabelPart {
        kind: LabelKind::MetaHash { parent },
        ..meta_part(text)
    }
}

fn date_part(text: String) -> LabelPart {
    LabelPart {
        kind: LabelKind::MetaDate,
        ..meta_part(text)
    }
}

fn author_part(text: String) -> LabelPart {
    LabelPart {
        kind: LabelKind::MetaAuthor,
        ..meta_part(text)
    }
}

fn join_meta_groups(groups: impl IntoIterator<Item = Vec<LabelPart>>) -> Vec<LabelPart> {
    let mut out = Vec::new();
    let mut first = true;
    for group in groups {
        if group.is_empty() || parts_text(&group).is_empty() {
            continue;
        }
        if !first {
            out.push(LabelPart {
                text: " · ".into(),
                kind: LabelKind::Meta,
                target: None,
            });
        }
        first = false;
        out.extend(group);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::glyphs::UNICODE;
    use crate::model::{Commit, GraphRef, Stash, SyncState, SyncStatus};

    #[test]
    fn budget_prefers_footer_over_header() {
        let chrome = graph_chrome_budget(3, true);
        assert!(chrome.footer);
        assert!(!chrome.header);
        assert_eq!(chrome.footer_height, 2, "the rule and the meta row");
        assert_eq!(chrome.footer_body_height(), 1);
        assert_eq!(chrome.list_height, 1);
    }

    #[test]
    fn budget_keeps_header_when_tall() {
        let chrome = graph_chrome_budget(16, true);
        assert!(chrome.header);
        assert!(chrome.footer);
        assert_eq!(chrome.list_height, 12);
    }

    #[test]
    fn footer_request_is_rule_plus_fixed_n_plus_meta_or_three_collapsed() {
        use crate::format::COMMIT_MSG_LINES_DEFAULT;
        assert_eq!(FOOTER_RULE_ROWS, 1);
        assert_eq!(
            graph_footer_request(true, COMMIT_MSG_LINES_DEFAULT),
            COMMIT_MSG_LINES_DEFAULT as u16 + 2
        );
        assert_eq!(graph_footer_request(true, 3), 5);
        assert_eq!(graph_footer_request(true, 0), 3, "clamped to the minimum");
        assert_eq!(
            graph_footer_request(true, 99),
            COMMIT_MSG_LINES_MAX as u16 + 2,
            "clamped to the maximum"
        );
        assert_eq!(graph_footer_request(false, 3), 3);
        assert_eq!(graph_footer_request(false, 20), 3);
    }

    #[test]
    fn budget_gives_the_requested_footer_capped_at_half_the_pane() {
        let tall = graph_chrome_budget_for(40, false, graph_footer_request(true, 8));
        assert_eq!(tall.footer_height, 10, "rule + 8 message rows + meta");
        assert_eq!(tall.footer_body_height(), 9);
        assert_eq!(tall.list_height, 30);
        let max = graph_chrome_budget_for(60, false, graph_footer_request(true, 20));
        assert_eq!(max.footer_height, 22, "N=20 fits a 60-row pane");
        assert_eq!(max.list_height, 38);
        let short = graph_chrome_budget_for(10, false, graph_footer_request(true, 8));
        assert_eq!(short.footer_height, 6, "rule over half the pane");
        assert_eq!(short.footer_body_height(), 5);
        assert_eq!(short.list_height, 4);
        let tiny = graph_chrome_budget_for(3, false, graph_footer_request(true, 8));
        assert!(tiny.footer);
        assert_eq!(tiny.footer_height, 2, "the list keeps its one row");
        assert_eq!(tiny.footer_body_height(), 1);
        assert_eq!(tiny.list_height, 1);
        let small_n = graph_chrome_budget_for(40, false, graph_footer_request(true, 1));
        assert_eq!(small_n.footer_height, 3, "N=1: rule, one message row, meta");
        assert_eq!(small_n.list_height, 37);
    }

    #[test]
    fn budget_drops_footer_below_three_rows() {
        for height in [1, 2] {
            let chrome = graph_chrome_budget_for(height, false, graph_footer_request(true, 8));
            assert!(!chrome.footer, "{height}: {chrome:?}");
            assert_eq!(chrome.footer_height, 0);
            assert_eq!(chrome.list_height, height);
        }
    }

    #[test]
    fn budget_collapsed_footer_is_rule_plus_two_rows() {
        for height in [4, 10, 40] {
            let chrome = graph_chrome_budget_for(height, false, graph_footer_request(false, 8));
            assert_eq!(chrome.footer_height, 3, "{height}: {chrome:?}");
            assert_eq!(chrome.footer_body_height(), 2, "{height}");
            assert_eq!(chrome.list_height, height - 3, "{height}");
            assert_eq!(chrome, graph_chrome_budget(height, false));
        }
        let three = graph_chrome_budget(3, false);
        assert_eq!((three.footer_height, three.list_height), (2, 1));
    }

    #[test]
    fn rule_takes_one_list_row_and_keeps_the_body() {
        // Body under the rule = the old footer; the list gives up one row.
        for (height, expand, n) in [(40, true, 4), (30, true, 8), (12, false, 4), (60, true, 20)] {
            let chrome = graph_chrome_budget_for(height, false, graph_footer_request(expand, n));
            let body = graph_footer_request(expand, n) - FOOTER_RULE_ROWS;
            assert_eq!(chrome.footer_body_height(), body, "{height}/{expand}/{n}");
            assert_eq!(
                chrome.list_height,
                height - body - 1,
                "{height}/{expand}/{n}"
            );
        }
    }

    #[test]
    fn commit_msg_lines_fit_is_the_most_rows_the_pane_shows() {
        for (height, fit) in [
            (1, 1),
            (3, 1),
            (8, 3),
            (10, 4),
            (12, 5),
            (40, 19),
            (44, 20),
            (60, 20),
        ] {
            assert_eq!(commit_msg_lines_fit(height), fit, "{height}");
        }
        for height in 5..80 {
            let fit = commit_msg_lines_fit(height);
            let shown =
                |n: usize| graph_chrome_budget_for(height, false, graph_footer_request(true, n));
            assert_eq!(
                usize::from(shown(fit).footer_body_height()) - 1,
                fit,
                "{height}: every fitted row shows"
            );
            if fit < COMMIT_MSG_LINES_MAX {
                assert_eq!(
                    shown(fit + 1),
                    shown(fit),
                    "{height}: one more row paints nothing"
                );
            }
        }
    }

    #[test]
    fn budget_adds_wrapped_meta_rows_and_takes_them_from_the_list() {
        let wrapped = graph_chrome_budget_for_meta(40, false, graph_footer_request(true, 4), 3);
        assert_eq!(wrapped.footer_height, 8, "rule + 4 message rows + 3 meta");
        assert_eq!(wrapped.footer_body_height(), 7);
        assert_eq!(wrapped.meta_rows, 3);
        assert_eq!(wrapped.message_rows(), 4);
        assert_eq!(wrapped.list_height, 32);
        let one = graph_chrome_budget_for_meta(40, false, 6, 1);
        assert_eq!(one.footer_height, 6);
        assert_eq!(one.list_height, 34);
        assert_eq!(one, graph_chrome_budget_for(40, false, 6));
        assert_eq!(graph_chrome_budget_for_meta(40, false, 6, 0), one);
    }

    #[test]
    fn budget_keeps_the_message_share_of_half_the_pane_under_wrapped_meta() {
        let chrome = graph_chrome_budget_for_meta(10, false, graph_footer_request(true, 8), 3);
        assert_eq!(chrome.footer_height, 8);
        assert_eq!(chrome.message_rows(), 4);
        assert_eq!(chrome.meta_rows, 3);
        assert_eq!(chrome.list_height, 2);
    }

    #[test]
    fn budget_cuts_meta_wider_than_the_pane_and_keeps_one_list_row() {
        let chrome = graph_chrome_budget_for_meta(6, false, 6, 9);
        assert_eq!(chrome.footer_height, 5);
        assert_eq!(chrome.meta_rows, 4);
        assert_eq!(chrome.message_rows(), 0);
        assert_eq!(chrome.list_height, 1);
    }

    #[test]
    fn budget_collapsed_footer_with_wrapped_meta() {
        let three = graph_chrome_budget_for_meta(3, false, 3, 3);
        assert_eq!(three.footer_height, 2, "the rule and one meta row");
        assert_eq!(three.meta_rows, 1);
        assert_eq!(three.list_height, 1);
        let four = graph_chrome_budget_for_meta(4, false, 3, 2);
        assert_eq!(four.footer_height, 3, "meta rows before the subject");
        assert_eq!(four.meta_rows, 2);
        assert_eq!(four.message_rows(), 0);
        assert_eq!(four.list_height, 1);
        let ten = graph_chrome_budget_for_meta(10, false, 3, 2);
        assert_eq!(ten.footer_height, 4, "rule, subject, 2 meta rows");
        assert_eq!(ten.message_rows(), 1);
        assert_eq!(ten.list_height, 6);
        for height in [1, 2] {
            let chrome = graph_chrome_budget_for_meta(height, false, 3, 3);
            assert!(!chrome.footer, "{height}: {chrome:?}");
            assert_eq!(chrome.meta_rows, 0, "{height}");
        }
    }

    #[test]
    fn budget_with_wrapped_meta_keeps_the_header_when_tall() {
        let chrome = graph_chrome_budget_for_meta(16, true, 6, 2);
        assert_eq!(chrome.footer_height, 7);
        assert!(chrome.header);
        assert_eq!(chrome.list_height, 8);
    }

    /// Footer of a commit with one parent: meta
    /// `(no refs) · abcdefg · parent 1111111 · Ada · 2m`.
    fn one_parent_footer(width: usize, expand: bool) -> (SelectionFooter, Vec<String>) {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "subject".into(),
            parents: vec!["1111111aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let selection = GraphFooterSelection::Row(&row);
        let footer = selection_footer(&model, selection, &UNICODE, width, 1_700_000_000, expand);
        let lines =
            selection_footer_lines(&model, selection, &UNICODE, width, 1_700_000_000, expand);
        (footer, lines)
    }

    #[test]
    fn selection_footer_keeps_the_whole_meta_line() {
        let (footer, lines) = one_parent_footer(20, false);
        assert_eq!(
            parts_text(&footer.meta),
            "(no refs) · abcdefg · parent 1111111 · Ada · 2m"
        );
        assert_eq!(footer.meta_rows(20), 3);
        assert_eq!(
            lines,
            vec![
                "subject",
                "(no refs) · abcdefg",
                "parent 1111111 · Ada",
                "2m"
            ]
        );
    }

    #[test]
    fn selection_footer_scroll_max_counts_the_wrapped_meta_rows() {
        let (one_parent, _) = one_parent_footer(20, false);
        let footer = SelectionFooter {
            message: (0..11)
                .map(|i| vec![meta_part(format!("line {i}"))])
                .collect(),
            meta: one_parent.meta,
        };
        // 3 meta rows at 20 columns leave 4 message rows of 7.
        assert_eq!(footer.scroll_max(20, 7), 7);
        // One meta row at 120 columns leaves 4 message rows of 5.
        assert_eq!(footer.scroll_max(120, 5), 7);
        assert_eq!(footer_message_scroll_max(12, 5), 7);
    }

    #[test]
    fn footer_body_height_is_zero_without_a_footer() {
        let chrome = graph_chrome_budget(2, false);
        assert!(!chrome.footer);
        assert_eq!(chrome.footer_body_height(), 0);
    }

    #[test]
    fn footer_message_scroll_max_keeps_meta_pinned() {
        assert_eq!(footer_message_scroll_max(2, 2), 0);
        assert_eq!(footer_message_scroll_max(9, 9), 0);
        // 11 message lines + meta in 5 rows: 4 message rows, 7 hidden.
        assert_eq!(footer_message_scroll_max(12, 5), 7);
        assert_eq!(footer_message_scroll_max(3, 2), 1);
    }

    #[test]
    fn expanded_footer_keeps_every_message_line() {
        let body = (0..20)
            .map(|i| format!("body line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let commit = Commit {
            id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            subject: "subject".into(),
            body,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let lines = selection_footer_lines(
            &model,
            GraphFooterSelection::Row(&row),
            &UNICODE,
            40,
            0,
            true,
        );
        // subject, blank, 20 body lines, meta
        assert_eq!(lines.len(), 23, "{lines:?}");
        assert_eq!(lines[21], "body line 19");
        assert!(lines.iter().all(|l| !l.ends_with('…')), "{lines:?}");
        assert!(
            lines[..22].iter().all(|l| l.chars().count() <= 39),
            "message wraps one column short for the scrollbar: {lines:?}"
        );
    }

    #[test]
    fn footer_uncommitted_clean_and_dirty() {
        let model = GraphModel {
            uncommitted: Some(true),
            ..GraphModel::default()
        };
        let dirty = GraphRow::Uncommitted { has_changes: true };
        let [a, b] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&dirty), &UNICODE, 40, 0);
        assert_eq!(a, "Uncommitted changes");
        assert_eq!(b, FOOTER_WORKTREE_NOT_A_COMMIT);
        let clean = GraphRow::Uncommitted { has_changes: false };
        let [c, d] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&clean), &UNICODE, 40, 0);
        assert_eq!(c, "Working tree clean");
        assert_eq!(d, FOOTER_WORKTREE_NOT_A_COMMIT);
    }

    #[test]
    fn footer_uncommitted_lists_head_commit_refs() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "tip".into(),
            refs: vec![GraphRef::local("main"), GraphRef::tag("v1")],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            head_id: Some(commit.id.clone()),
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Uncommitted { has_changes: false };
        let [subject, meta] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&row), &UNICODE, 80, 0);
        assert_eq!(subject, "Working tree clean");
        assert!(meta.contains("main"), "{meta}");
        assert!(meta.contains("v1"), "{meta}");
        assert_ne!(meta, FOOTER_WORKTREE_NOT_A_COMMIT);
    }

    #[test]
    fn footer_no_selection_connector_and_no_refs() {
        let model = GraphModel::default();
        let [none, empty] =
            selection_detail_lines(&model, GraphFooterSelection::None, &UNICODE, 40, 0);
        assert_eq!(none, FOOTER_NO_SELECTION);
        assert_eq!(empty, "");
        let [dots, connector] =
            selection_detail_lines(&model, GraphFooterSelection::Spacer, &UNICODE, 40, 0);
        assert_eq!(dots, FOOTER_SPACER_SUBJECT);
        assert_eq!(connector, FOOTER_CONNECTOR_NOT_SELECTABLE);
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "untagged".into(),
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let [_, meta] =
            selection_detail_lines(&model, GraphFooterSelection::Row(&row), &UNICODE, 80, 0);
        assert!(
            meta.starts_with(FOOTER_NO_REFS),
            "commit with no chips uses (no refs): {meta}"
        );
    }

    #[test]
    fn footer_stash_ref_hash_date_without_author() {
        let stash = Stash {
            id: "abcdefghhhh".into(),
            stash_ref: "stash@{0}".into(),
            subject: "WIP on main".into(),
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            parent_id: None,
            ..Stash::default()
        };
        let model = GraphModel {
            stashes: vec![stash.clone()],
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let [subject, meta] = selection_detail_lines(
            &model,
            GraphFooterSelection::Row(&GraphRow::Stash(stash)),
            &UNICODE,
            80,
            1_700_000_000,
        );
        assert_eq!(subject, "WIP on main");
        assert_eq!(meta, "stash@{0} · abcdefg · 2m");
        assert!(!meta.contains("Ada"), "{meta}");
    }

    #[test]
    fn footer_commit_subject_and_meta() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "add footer".into(),
            refs: vec!["main".into()],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            head_id: Some(commit.id.clone()),
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: true,
            worktrees: Vec::new(),
        };
        let [subject, meta] = selection_detail_lines(
            &model,
            GraphFooterSelection::Row(&row),
            &UNICODE,
            80,
            1_700_000_000,
        );
        assert_eq!(subject, "add footer");
        assert!(meta.contains("abcdefg"), "{meta}");
        assert!(meta.contains("Ada"), "{meta}");
        assert!(meta.contains("2m") || meta.contains("just now"), "{meta}");
    }

    /// Meta line of a focused commit with `parents`, collapsed and expanded.
    fn commit_meta_with_parents(parents: &[&str]) -> (String, String) {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "subject".into(),
            body: "body".into(),
            parents: parents.iter().map(|p| p.to_string()).collect(),
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let lines = |expand| {
            selection_footer_lines(
                &model,
                GraphFooterSelection::Row(&row),
                &UNICODE,
                120,
                1_700_000_000,
                expand,
            )
        };
        let collapsed = lines(false);
        let expanded = lines(true);
        assert_eq!(collapsed.len(), 2, "{collapsed:?}");
        assert!(expanded.len() > 2, "{expanded:?}");
        (
            collapsed.last().unwrap().clone(),
            expanded.last().unwrap().clone(),
        )
    }

    #[test]
    fn footer_commit_shows_one_parent_after_hash() {
        let (collapsed, expanded) =
            commit_meta_with_parents(&["1111111aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"]);
        assert_eq!(collapsed, "(no refs) · abcdefg · parent 1111111 · Ada · 2m");
        assert_eq!(expanded, collapsed, "expanded footer keeps the same meta");
    }

    #[test]
    fn footer_merge_commit_lists_parents_in_git_order() {
        let (collapsed, expanded) = commit_meta_with_parents(&[
            "2222222aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "1111111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "3333333ccccccccccccccccccccccccccccccccc",
        ]);
        assert_eq!(
            collapsed, "(no refs) · abcdefg · parents 2222222 1111111 3333333 · Ada · 2m",
            "first parent first, every parent listed"
        );
        assert_eq!(expanded, collapsed, "expanded footer keeps the same meta");
    }

    #[test]
    fn footer_root_commit_shows_root_marker() {
        let (collapsed, expanded) = commit_meta_with_parents(&[]);
        assert_eq!(
            collapsed,
            format!("(no refs) · abcdefg · {FOOTER_ROOT_COMMIT} · Ada · 2m")
        );
        assert_eq!(expanded, collapsed, "expanded footer keeps the same meta");
    }

    #[test]
    fn footer_parents_group_tags_each_parent_id() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "merge".into(),
            parents: vec!["2222222aaaa".into(), "1111111bbbb".into()],
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let [_, meta] =
            selection_detail_parts(&model, GraphFooterSelection::Row(&row), &UNICODE, 80, 0);
        let at = meta
            .iter()
            .position(|p| p.text == "parents ")
            .unwrap_or_else(|| panic!("parents group: {meta:?}"));
        let group: Vec<(&str, LabelKind)> = meta[at..at + 4]
            .iter()
            .map(|p| (p.text.as_str(), p.kind))
            .collect();
        assert_eq!(
            group,
            vec![
                ("parents ", LabelKind::Meta),
                ("2222222", LabelKind::MetaHash { parent: Some(0) }),
                (" ", LabelKind::Meta),
                ("1111111", LabelKind::MetaHash { parent: Some(1) }),
            ]
        );
    }

    #[test]
    fn footer_author_and_date_are_italic_kinds() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "topic".into(),
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let [_, meta] = selection_detail_parts(
            &model,
            GraphFooterSelection::Row(&row),
            &UNICODE,
            80,
            1_700_000_000,
        );
        let kind_of = |text: &str| {
            meta.iter()
                .find(|p| p.text == text)
                .map(|p| p.kind)
                .unwrap_or_else(|| panic!("{text}: {meta:?}"))
        };
        assert_eq!(kind_of("Ada"), LabelKind::MetaAuthor);
        assert_eq!(kind_of("2m"), LabelKind::MetaDate);
        assert_eq!(kind_of("abcdefg"), LabelKind::MetaHash { parent: None });

        let stash = Stash {
            id: "1234567890".into(),
            stash_ref: "stash@{0}".into(),
            subject: "wip".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Stash::default()
        };
        let stash_row = GraphRow::Stash(stash);
        let [_, meta] = selection_detail_parts(
            &model,
            GraphFooterSelection::Row(&stash_row),
            &UNICODE,
            80,
            1_700_000_000,
        );
        let date = meta.iter().find(|p| p.text == "2m");
        assert_eq!(date.map(|p| p.kind), Some(LabelKind::MetaDate), "{meta:?}");
    }

    #[test]
    fn footer_commit_parts_reuse_row_chip_kinds() {
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: "palette".into(),
            refs: vec![
                GraphRef::local("main"),
                GraphRef::local("topic"),
                GraphRef::remote("origin/other"),
                GraphRef::tag("v1"),
            ],
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            head_id: Some(commit.id.clone()),
            sync: Some(SyncState {
                branch: "main".into(),
                status: SyncStatus::UpToDate,
                ahead: 0,
                behind: 0,
            }),
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: true,
            worktrees: Vec::new(),
        };
        let [subject, meta] = selection_detail_parts(
            &model,
            GraphFooterSelection::Row(&row),
            &UNICODE,
            80,
            1_700_000_000,
        );
        assert!(
            subject.iter().all(|p| p.kind == LabelKind::Subject),
            "{subject:?}"
        );
        let kinds_for = |needle: &str| -> Vec<LabelKind> {
            meta.iter()
                .filter(|p| p.text.contains(needle))
                .map(|p| p.kind)
                .collect()
        };
        assert!(
            kinds_for("main").contains(&LabelKind::ChipDefault),
            "default branch: {meta:?}"
        );
        assert!(
            kinds_for("topic").contains(&LabelKind::ChipLocal),
            "feature branch: {meta:?}"
        );
        assert!(
            kinds_for("origin/other").contains(&LabelKind::ChipRemote),
            "remote: {meta:?}"
        );
        assert!(
            kinds_for("v1").contains(&LabelKind::ChipTag),
            "tag: {meta:?}"
        );
        assert!(
            meta.iter().any(|p| p.kind == LabelKind::ChipHead),
            "HEAD checkout mark: {meta:?}"
        );
        assert!(
            kinds_for("Ada").iter().all(|k| *k == LabelKind::MetaAuthor),
            "author is the italic author kind: {meta:?}"
        );
        let hash_kinds = kinds_for("abcdefg");
        assert!(
            !hash_kinds.is_empty()
                && hash_kinds
                    .iter()
                    .all(|k| *k == LabelKind::MetaHash { parent: None }),
            "hash is the commit hash kind: {meta:?}"
        );

        let uncommitted = GraphRow::Uncommitted { has_changes: false };
        let [_, head_meta] = selection_detail_parts(
            &model,
            GraphFooterSelection::Row(&uncommitted),
            &UNICODE,
            80,
            1_700_000_000,
        );
        assert!(
            head_meta
                .iter()
                .any(|p| p.kind == LabelKind::ChipDefault && p.text.contains("main")),
            "uncommitted footer reuses HEAD chips: {head_meta:?}"
        );
        assert!(
            head_meta.iter().any(|p| p.kind == LabelKind::ChipTag),
            "uncommitted footer keeps tags: {head_meta:?}"
        );
        assert!(
            head_meta.iter().any(|p| p.kind == LabelKind::ChipHead),
            "uncommitted footer keeps HEAD mark: {head_meta:?}"
        );
        // Footer chips carry the same targets as the spacer chips; hash,
        // parents, author, and date carry none.
        let target_of = |needle: &str| {
            meta.iter()
                .find(|p| p.text == needle)
                .and_then(|p| p.target.clone())
        };
        assert_eq!(
            target_of("main"),
            Some(PartTarget::Chip(vec![GraphRef::local("main")]))
        );
        assert_eq!(
            target_of("[v1]"),
            Some(PartTarget::Chip(vec![GraphRef::tag("v1")]))
        );
        assert!(meta.iter().any(|p| matches!(
            &p.target,
            Some(PartTarget::Marks { branch, checkout: true, remote: None }) if branch == "main"
        )));
        assert!(meta
            .iter()
            .filter(|p| p.text.contains("abcdefg") || p.text.contains("Ada"))
            .all(|p| p.target.is_none()));
        assert_eq!(
            head_meta
                .iter()
                .filter_map(|p| p.target.as_ref())
                .collect::<Vec<_>>(),
            meta.iter()
                .filter_map(|p| p.target.as_ref())
                .collect::<Vec<_>>(),
            "the uncommitted footer names HEAD's chips"
        );
    }

    #[test]
    fn footer_worktree_branch_is_a_local_branch_chip() {
        let model = GraphModel::default();
        let row = GraphRow::Worktree(crate::model::Worktree {
            path: "app/.worktrees/feat".into(),
            head_id: None,
            branch: Some("feat".into()),
            hidden: false,
            is_current: false,
        });
        let [_, meta] =
            selection_detail_parts(&model, GraphFooterSelection::Row(&row), &UNICODE, 80, 0);
        assert_eq!(meta.len(), 1, "{meta:?}");
        assert_eq!(meta[0].kind, LabelKind::ChipLocal);
        assert_eq!(
            meta[0].target,
            Some(PartTarget::Chip(vec![GraphRef::local("feat")]))
        );
    }

    #[test]
    fn expanded_footer_wraps_long_subject_and_body() {
        let tail = "TAILTOKEN";
        let body = "UNIQUE_BODY_LINE";
        let commit = Commit {
            id: "abcdefghhhh".into(),
            subject: format!("{}{tail}", "n".repeat(24)),
            body: body.into(),
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            ..Commit::default()
        };
        let model = GraphModel {
            commits: vec![commit.clone()],
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let row = GraphRow::Commit {
            commit,
            is_head: false,
            worktrees: Vec::new(),
        };
        let width = 16;
        let collapsed = selection_footer_lines(
            &model,
            GraphFooterSelection::Row(&row),
            &UNICODE,
            width,
            1_700_000_000,
            false,
        );
        // The subject, then the meta line wrapped at 16 columns:
        // `(no refs)`, `abcdefg`, `root commit`, `Ada · 2m`.
        assert_eq!(collapsed.len(), 5, "{collapsed:?}");
        assert_eq!(
            &collapsed[1..],
            ["(no refs)", "abcdefg", "root commit", "Ada · 2m"],
            "{collapsed:?}"
        );
        let collapsed_text = collapsed.join("\n");
        assert!(
            collapsed_text.contains('…') || !collapsed_text.contains(tail),
            "collapsed subject clips: {collapsed_text}"
        );
        assert!(
            !collapsed_text.contains(body),
            "collapsed hides body: {collapsed_text}"
        );
        assert!(
            !collapsed_text.contains(tail),
            "collapsed hides subject tail: {collapsed_text}"
        );

        let expanded = selection_footer_lines(
            &model,
            GraphFooterSelection::Row(&row),
            &UNICODE,
            width,
            1_700_000_000,
            true,
        );
        assert!(expanded.len() > 2, "{expanded:?}");
        let expanded_text = expanded.join("");
        assert!(
            expanded_text.contains(tail),
            "expanded shows subject tail: {expanded:?}"
        );
        assert!(
            expanded_text.contains(body),
            "expanded shows body: {expanded:?}"
        );

        let chrome = graph_chrome_budget_for(16, true, graph_footer_request(true, 8));
        assert!(chrome.footer);
        assert_eq!(
            chrome.footer_height, 9,
            "rule + half of 16, not the message size"
        );
        assert!(chrome.list_height >= 1);
    }

    #[test]
    fn expanded_stash_without_body_stays_two_lines_when_subject_fits() {
        let stash = Stash {
            id: "abcdefghhhh".into(),
            stash_ref: "stash@{0}".into(),
            subject: "WIP on main".into(),
            body: String::new(),
            author_name: "Ada".into(),
            author_date_unix: 1_700_000_000 - 120,
            parent_id: None,
            ..Stash::default()
        };
        let model = GraphModel {
            stashes: vec![stash.clone()],
            uncommitted: Some(false),
            ..GraphModel::default()
        };
        let lines = selection_footer_lines(
            &model,
            GraphFooterSelection::Row(&GraphRow::Stash(stash)),
            &UNICODE,
            80,
            1_700_000_000,
            true,
        );
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[0], "WIP on main");
    }
}

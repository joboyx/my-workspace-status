//! GraphPane chrome: header / list / fixed-height selection footer.
//!
//! Header / footer budget (`graph_chrome_budget`) and selection footer copy.
//! Footer ref chips are the same [`LabelPart`] runs as the commit spacer.

use crate::format::{
    commit_ref_chip_parts, format_commit_message, format_relative_date, is_default_branch,
    parts_text, short_id, trunc_label_parts, wrap_commit_message, LabelKind, LabelPart,
    COMMIT_MSG_LINES_MAX, COMMIT_MSG_LINES_MIN,
};
use crate::glyphs::GlyphSet;
use crate::model::{GraphModel, GraphRow};

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
    /// Footer rows (0 when [`Self::footer`] is false). Collapsed is 2.
    pub footer_height: u16,
    /// Rows left for the commit list (at least 1).
    pub list_height: u16,
}

/// Requested selection-footer height for [`graph_chrome_budget_for`].
///
/// Expanded is `msg_lines` message rows (clamped to
/// [`COMMIT_MSG_LINES_MIN`]..=[`COMMIT_MSG_LINES_MAX`]) plus the meta row.
/// Collapsed is 2 (subject + meta). The value never depends on the selected
/// message, so moving between commits keeps the list height. The app and
/// [`crate::GraphWidget`] both call this so layout and paint agree.
pub fn graph_footer_request(expand: bool, msg_lines: usize) -> u16 {
    if expand {
        msg_lines.clamp(COMMIT_MSG_LINES_MIN, COMMIT_MSG_LINES_MAX) as u16 + 1
    } else {
        2
    }
}

/// Max message scroll for a selection footer of `line_count` lines painted
/// in `footer_height` rows.
///
/// The last line (meta) stays pinned under the message viewport. `0` when
/// every line fits.
pub fn footer_message_scroll_max(line_count: usize, footer_height: u16) -> usize {
    let rows = footer_height as usize;
    if line_count <= rows {
        return 0;
    }
    line_count
        .saturating_sub(1)
        .saturating_sub(rows.saturating_sub(1))
}

/// Footer first, then header. Collapsed footer is 2 lines.
pub fn graph_chrome_budget(height: u16, want_header: bool) -> GraphChromeBudget {
    graph_chrome_budget_for(height, want_header, 2)
}

/// Like [`graph_chrome_budget`] with a requested footer height.
///
/// `footer_lines` is the fixed requested footer height from
/// [`graph_footer_request`] (expanded: message rows plus the meta row;
/// collapsed: 2), never the selected message's line count. The footer gets
/// that height, at most half the pane (never under 2). A message taller
/// than the footer scrolls ([`footer_message_scroll_max`]); a shorter one
/// leaves blank rows. The list keeps at least one row. A pane shorter than
/// 3 rows still drops the footer. The result depends only on the pane and
/// the request.
pub fn graph_chrome_budget_for(
    height: u16,
    want_header: bool,
    footer_lines: u16,
) -> GraphChromeBudget {
    let mut avail = height.max(1);
    let footer = avail >= 3;
    let footer_height = if footer {
        let cap = (avail / 2).max(2);
        let h = footer_lines.clamp(2, cap).min(avail.saturating_sub(1));
        avail = avail.saturating_sub(h);
        h
    } else {
        0
    };
    let header = want_header && avail >= 2;
    if header {
        avail = avail.saturating_sub(1);
    }
    GraphChromeBudget {
        header,
        footer,
        footer_height,
        list_height: avail.max(1),
    }
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
/// default / local / remote / tag). Hash, parents, date, and author stay
/// [`LabelKind::Meta`]. Commit meta order: chips · hash · parents · author · date.
pub fn selection_detail_parts(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
) -> [Vec<LabelPart>; 2] {
    let width = width.max(1);
    match selection {
        GraphFooterSelection::None => [subject_parts(FOOTER_NO_SELECTION, width), Vec::new()],
        GraphFooterSelection::Spacer => [
            subject_parts(FOOTER_SPACER_SUBJECT, width),
            meta_parts(FOOTER_CONNECTOR_NOT_SELECTABLE, width),
        ],
        GraphFooterSelection::Row(GraphRow::Uncommitted { has_changes }) => {
            let line = if *has_changes {
                "Uncommitted changes"
            } else {
                "Working tree clean"
            };
            let meta = head_commit_ref_parts(model, glyphs).unwrap_or_else(|| {
                vec![LabelPart {
                    text: FOOTER_WORKTREE_NOT_A_COMMIT.to_string(),
                    kind: LabelKind::Meta,
                }]
            });
            [subject_parts(line, width), trunc_label_parts(&meta, width)]
        }
        GraphFooterSelection::Row(GraphRow::Stash(stash)) => {
            let meta = join_meta_groups([
                vec![meta_part(stash.stash_ref.clone())],
                vec![meta_part(short_id(&stash.id).to_string())],
                vec![meta_part(format_relative_date(
                    stash.author_date_unix,
                    now_unix,
                ))],
            ]);
            [
                subject_parts(&stash.subject, width),
                trunc_label_parts(&meta, width),
            ]
        }
        GraphFooterSelection::Row(GraphRow::Worktree(wt)) => {
            let meta = match wt.branch.as_deref() {
                Some(branch) if !branch.is_empty() => vec![LabelPart {
                    text: branch.to_string(),
                    kind: if is_default_branch(branch, model.default_branch_override.as_deref()) {
                        LabelKind::ChipDefault
                    } else {
                        LabelKind::ChipLocal
                    },
                }],
                _ => Vec::new(),
            };
            [
                subject_parts(&wt.path, width),
                trunc_label_parts(&meta, width),
            ]
        }
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
                }]);
            } else {
                groups.push(chips);
            }
            groups.push(vec![meta_part(short_id(&commit.id).to_string())]);
            groups.push(vec![meta_part(parents_text(&commit.parents))]);
            if !commit.author_name.is_empty() {
                groups.push(vec![meta_part(commit.author_name.clone())]);
            }
            if commit.author_date_unix > 0 {
                groups.push(vec![meta_part(format_relative_date(
                    commit.author_date_unix,
                    now_unix,
                ))]);
            }
            [
                subject_parts(&commit.subject, width),
                trunc_label_parts(&join_meta_groups(groups), width),
            ]
        }
    }
}

/// Selection-footer runs. Collapsed is the two truncated lines from
/// [`selection_detail_parts`]. Expanded wraps the whole subject plus body
/// one column short of `width` (that column holds the footer scrollbar),
/// then the same meta line. There is no line cap: the widget scrolls a
/// message taller than the footer. Graph list rows stay one line either way.
pub fn selection_footer_parts(
    model: &GraphModel,
    selection: GraphFooterSelection<'_>,
    glyphs: &GlyphSet,
    width: usize,
    now_unix: i64,
    expand: bool,
) -> Vec<Vec<LabelPart>> {
    let [subject, meta] = selection_detail_parts(model, selection, glyphs, width, now_unix);
    if !expand {
        return vec![subject, meta];
    }
    let (message_subject, body) = match selection {
        GraphFooterSelection::Row(GraphRow::Commit { commit, .. }) => {
            (commit.subject.as_str(), commit.body.as_str())
        }
        GraphFooterSelection::Row(GraphRow::Stash(stash)) => {
            (stash.subject.as_str(), stash.body.as_str())
        }
        _ => return vec![subject, meta],
    };
    let text = format_commit_message(message_subject, body);
    let wrapped = wrap_commit_message(&text, width.saturating_sub(1).max(1), usize::MAX);
    let mut lines: Vec<Vec<LabelPart>> = wrapped
        .into_iter()
        .map(|text| {
            vec![LabelPart {
                text,
                kind: LabelKind::Subject,
            }]
        })
        .collect();
    if lines.is_empty() {
        lines.push(subject);
    }
    lines.push(meta);
    lines
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
        }],
        width,
    )
}

fn meta_parts(text: &str, width: usize) -> Vec<LabelPart> {
    trunc_label_parts(&[meta_part(text.to_string())], width)
}

/// Commit footer parents group: `parent <id>`, `parents <id> <id> …`
/// (git parent order), or [`FOOTER_ROOT_COMMIT`].
fn parents_text(parents: &[String]) -> String {
    let ids = parents.iter().map(|id| short_id(id)).collect::<Vec<_>>();
    match ids.as_slice() {
        [] => FOOTER_ROOT_COMMIT.to_string(),
        [one] => format!("parent {one}"),
        _ => format!("parents {}", ids.join(" ")),
    }
}

fn meta_part(text: String) -> LabelPart {
    LabelPart {
        text,
        kind: LabelKind::Meta,
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
        assert_eq!(chrome.footer_height, 2);
        assert_eq!(chrome.list_height, 1);
    }

    #[test]
    fn budget_keeps_header_when_tall() {
        let chrome = graph_chrome_budget(16, true);
        assert!(chrome.header);
        assert!(chrome.footer);
        assert_eq!(chrome.list_height, 13);
    }

    #[test]
    fn footer_request_is_fixed_n_plus_meta_or_two_collapsed() {
        use crate::format::COMMIT_MSG_LINES_DEFAULT;
        assert_eq!(
            graph_footer_request(true, COMMIT_MSG_LINES_DEFAULT),
            COMMIT_MSG_LINES_DEFAULT as u16 + 1
        );
        assert_eq!(graph_footer_request(true, 3), 4);
        assert_eq!(graph_footer_request(true, 0), 2, "clamped to the minimum");
        assert_eq!(
            graph_footer_request(true, 99),
            COMMIT_MSG_LINES_MAX as u16 + 1,
            "clamped to the maximum"
        );
        assert_eq!(graph_footer_request(false, 3), 2);
        assert_eq!(graph_footer_request(false, 20), 2);
    }

    #[test]
    fn budget_gives_the_requested_footer_capped_at_half_the_pane() {
        let tall = graph_chrome_budget_for(40, false, graph_footer_request(true, 8));
        assert_eq!(tall.footer_height, 9);
        assert_eq!(tall.list_height, 31);
        let max = graph_chrome_budget_for(60, false, graph_footer_request(true, 20));
        assert_eq!(max.footer_height, 21, "N=20 fits a 60-row pane");
        assert_eq!(max.list_height, 39);
        let short = graph_chrome_budget_for(10, false, graph_footer_request(true, 8));
        assert_eq!(short.footer_height, 5, "half the pane");
        assert_eq!(short.list_height, 5);
        let tiny = graph_chrome_budget_for(3, false, graph_footer_request(true, 8));
        assert!(tiny.footer);
        assert_eq!(tiny.footer_height, 2);
        assert_eq!(tiny.list_height, 1);
        let small_n = graph_chrome_budget_for(40, false, graph_footer_request(true, 1));
        assert_eq!(small_n.footer_height, 2, "N=1: one message row plus meta");
        assert_eq!(small_n.list_height, 38);
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
    fn budget_collapsed_footer_is_two_rows() {
        for height in [3, 10, 40] {
            let chrome = graph_chrome_budget_for(height, false, graph_footer_request(false, 8));
            assert_eq!(chrome.footer_height, 2, "{height}: {chrome:?}");
            assert_eq!(chrome, graph_chrome_budget(height, false));
        }
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
    fn footer_parents_group_is_meta_kind() {
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
        let group = meta
            .iter()
            .find(|p| p.text.starts_with("parents "))
            .unwrap_or_else(|| panic!("parents group: {meta:?}"));
        assert_eq!(group.text, "parents 2222222 1111111");
        assert_eq!(group.kind, LabelKind::Meta);
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
            kinds_for("Ada").iter().all(|k| *k == LabelKind::Meta),
            "author stays meta: {meta:?}"
        );
        let hash_kinds = kinds_for("abcdefg");
        assert!(
            !hash_kinds.is_empty() && hash_kinds.iter().all(|k| *k == LabelKind::Meta),
            "hash stays meta: {meta:?}"
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
        assert_eq!(collapsed.len(), 2, "{collapsed:?}");
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
        assert_eq!(chrome.footer_height, 8, "half of 16, not the message size");
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

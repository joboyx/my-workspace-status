//! Icon popover state: peek timers, pin, `gh`, popover keys, and clicks.
//!
//! The model and the content builders live in [`super::super::popover`].

use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};

use super::super::action::{Action, Effect};
use super::super::chrome::open_dialog;
use super::super::command_palette::PaletteCommand;
use super::super::comments::{
    commit_file_row_comments, commit_file_row_comments_resolved, commit_file_row_has_comment,
    diff_line_comments, graph_row_comments, tree_row_comments_resolved, tree_row_has_comment,
    CommentEntry,
};
use super::super::commit_card::{commit_card_source, CommitCardSource};
use super::super::commit_files::CommitFileRow;
use super::super::diff::{diff_row_gutter_cells, DiffRow};
use super::super::drill::CommitFileSource;
use super::super::gates::ListFocusTarget;
use super::super::icons::IconKind;
use super::super::keys::InputMode;
use super::super::popover::{
    flat_lines, focused_line, graph_icon, graph_row_id, icon_targets, landing_line,
    popover_sections, step_line, tree_icon_target, IconTarget, PopoverLine, PopoverOrigin,
    PopoverOwner, PopoverSection, PopoverState, PEEK_DWELL_MS, PEEK_GRACE_MS,
};
use super::super::search::list_prefix_cols;
use super::super::selection::TextSelection;
use super::super::split::DiffMode;
use super::super::status::StatusMessage;
use super::super::tree::{
    fit_upstream_rewrite_text, painted_row_segments, pr_badge_repo, segment_icons,
    with_comment_mark, with_viewed_mark, NodeKind, NodeSegments, PrBadge, VisibleRow,
};
use super::{AppState, IconHit, PlainClick};
use workspace_status_graph::{paint_model, GraphRow, ASCII, UNICODE};

/// `gh` on a row that paints no icon.
pub(crate) const NO_ICONS_ON_ROW: &str = "no icons on this row";

/// Hover dwell and leave grace of the icon peek.
#[derive(Clone, Debug, Default)]
pub(crate) struct PeekTimers {
    /// Icon under the pointer and the time its peek opens.
    dwell: Option<(IconHit, Instant)>,
    /// Time the shown peek closes. Set while the pointer is off both the
    /// peek and its icon.
    grace: Option<Instant>,
}

/// Viewed, commented, resolved, and PR badge of one tree row.
pub(crate) type TreeRowMarks = (bool, bool, bool, Option<PrBadge>);

/// Checkout, primary checkout, branch, and source of the shown commit-file
/// list: the scope its line comments match in.
pub(crate) type CommitFileCommentScope<'a> = (
    &'a str,
    Option<&'a str>,
    Option<&'a str>,
    &'a CommitFileSource,
);

impl AppState {
    /// Tab, list, and row the focus is on now.
    pub(crate) fn popover_owner(&self) -> PopoverOwner {
        let list = self.list_focus_target();
        let row = match list {
            // The PRs tab list is no Workspace list: its cursor line.
            _ if self.is_prs_tab() => self.prs_popover_owner_row().unwrap_or_default(),
            // The Explorer tree's cursor row, never the parked Workspace
            // tree row.
            ListFocusTarget::Tree if self.is_explorer_tab() => self
                .explorer_cursor_rel()
                .map(|rel| format!("explorer:{rel}"))
                .unwrap_or_default(),
            ListFocusTarget::Tree => self
                .rows
                .get(self.cursor)
                .map(|row| row.id.clone())
                .unwrap_or_default(),
            ListFocusTarget::Graph => self
                .focused_graph_row()
                .map(|row| graph_row_id(&row))
                .unwrap_or_default(),
            ListFocusTarget::CommitFiles => self
                .focused_commit_file_row()
                .map(|row| row.id)
                .unwrap_or_default(),
            // A focused line viewer: its cursor line, so a line-mark
            // popover closes when the cursor leaves the line.
            ListFocusTarget::None => match self.tabs.active_file_view() {
                Some(file) => format!("file:{}", file.cursor),
                None => format!("diff:{}", self.diff_cursor),
            },
        };
        PopoverOwner {
            tab: self.tabs.active,
            list,
            row,
        }
    }

    /// True while a pinned popover is open on the focus it opened on.
    ///
    /// A pinned popover whose focus moved (another row, pane, or tab, or
    /// its row went away) is closed: it no longer takes keys, and the next
    /// paint drops it.
    pub fn popover_pinned(&self) -> bool {
        self.popover
            .as_ref()
            .is_some_and(|popover| popover.is_pinned() && popover.owner == self.popover_owner())
    }

    /// Sections of the open popover, rebuilt from live state. Empty with
    /// no popover.
    pub(crate) fn open_popover_sections(&self) -> Vec<PopoverSection> {
        self.popover
            .as_ref()
            .map(|popover| popover_sections(self, &popover.targets, super::unix_now()))
            .unwrap_or_default()
    }

    /// Viewed, comment, and PR badge marks tree row `row` paints in a tree
    /// `tree_width` columns wide at `now`.
    pub(crate) fn tree_row_marks(
        &self,
        row: &VisibleRow,
        tree_width: usize,
        now: Instant,
    ) -> TreeRowMarks {
        let viewed = row.kind == NodeKind::File && self.reviewed.contains(&row.id);
        let commented = tree_row_has_comment(&self.comment_store, &self.snapshot, row);
        let resolved =
            commented && tree_row_comments_resolved(&self.comment_store, &self.snapshot, row);
        let pr = pr_badge_repo(row)
            .and_then(|repo| self.tree_pr_badge(Path::new(repo), tree_width, now));
        (viewed, commented, resolved, pr)
    }

    /// Every segment tree row `row` paints this frame, before the spinner
    /// and the sparkline join the trailing run: at the painted row width,
    /// the tree's less the vertical bar's column while that bar shows.
    pub(crate) fn tree_row_paint_segments(&self, row: &VisibleRow) -> NodeSegments {
        let bar = u16::from(self.layout.tree_vbar.x.is_some());
        let width = self.layout.tree_width.saturating_sub(bar);
        self.tree_row_paint_segments_in(row, usize::from(width))
    }

    /// [`Self::tree_row_paint_segments`] for rows `width` columns wide, so
    /// a PR ref or an upstream note the paint drops at that width drops
    /// here too.
    pub(crate) fn tree_row_paint_segments_in(
        &self,
        row: &VisibleRow,
        width: usize,
    ) -> NodeSegments {
        let (viewed, commented, resolved, pr) = self.tree_row_marks(row, width, Instant::now());
        let mut segs =
            painted_row_segments(row, self.ascii, viewed, commented, resolved, pr.as_ref());
        let room = width.saturating_sub(list_prefix_cols(row.depth));
        fit_upstream_rewrite_text(&mut segs, &row.chrome, room);
        segs
    }

    /// Scope the line comments of the shown commit-file list match in. A
    /// list paint reads it once for every row
    /// ([`Self::commit_file_row_paint_segments_in`]).
    pub(crate) fn commit_file_comment_scope(&self) -> Option<CommitFileCommentScope<'_>> {
        let (repo, source) = self.commit_drill_source()?;
        let snap = self.snapshot.repos.iter().find(|r| r.repo == repo);
        Some((
            repo,
            snap.and_then(|r| r.primary_repo.as_deref()),
            snap.map(|r| r.branch.as_str()),
            source,
        ))
    }

    /// Comments on commit-file row `row`, in store order. A folder row has
    /// none.
    pub(crate) fn commit_file_row_comments(&self, row: &CommitFileRow) -> Vec<&CommentEntry> {
        match self.commit_file_comment_scope().filter(|_| row.is_file()) {
            Some((repo, primary, branch, source)) => commit_file_row_comments(
                &self.comment_store,
                repo,
                primary,
                source,
                &row.path,
                branch,
            ),
            None => Vec::new(),
        }
    }

    /// Every segment commit-file or compare list row `row` paints: its
    /// label and badge, then the comment mark and the viewed eye.
    pub(crate) fn commit_file_row_paint_segments(&self, row: &CommitFileRow) -> NodeSegments {
        self.commit_file_row_paint_segments_in(row, self.commit_file_comment_scope())
    }

    /// [`Self::commit_file_row_paint_segments`] with the list's comment
    /// `scope` already read ([`Self::commit_file_comment_scope`]).
    pub(crate) fn commit_file_row_paint_segments_in(
        &self,
        row: &CommitFileRow,
        scope: Option<CommitFileCommentScope<'_>>,
    ) -> NodeSegments {
        let scope = scope.filter(|_| row.is_file());
        let commented = scope.is_some_and(|(repo, primary, branch, source)| {
            commit_file_row_has_comment(
                &self.comment_store,
                repo,
                primary,
                source,
                &row.path,
                branch,
            )
        });
        let resolved = commented
            && scope.is_some_and(|(repo, primary, branch, source)| {
                commit_file_row_comments_resolved(
                    &self.comment_store,
                    repo,
                    primary,
                    source,
                    &row.path,
                    branch,
                )
            });
        NodeSegments {
            segments: row.segments.clone(),
            trailing: with_viewed_mark(
                with_comment_mark(row.trailing_segs.clone(), self.ascii, commented, resolved),
                self.ascii,
                self.compare_file_reviewed(row),
            ),
        }
    }

    /// Icons of the focused row in paint order, clipped ones included.
    ///
    /// On a focused file diff or clean-file view, the comment marks of the
    /// cursor line ([`Self::cursor_line_comment_icons`]); a cursor line
    /// with no mark has no icon.
    fn focused_row_icons(&self) -> Vec<(IconKind, IconTarget)> {
        let line_marks = self.cursor_line_comment_icons();
        if !line_marks.is_empty() {
            return line_marks;
        }
        if self.is_explorer_tab() {
            return match self.list_focus_target() {
                ListFocusTarget::Tree => self.explorer_cursor_row_icons(),
                _ => Vec::new(),
            };
        }
        match self.list_focus_target() {
            ListFocusTarget::Tree => {
                let Some(row) = self.focused_row() else {
                    return Vec::new();
                };
                segment_icons(&self.tree_row_paint_segments(row))
                    .into_iter()
                    .map(|kind| (kind, tree_icon_target(kind, row)))
                    .collect()
            }
            ListFocusTarget::Graph => self.graph_row_icons(self.graph_cursor),
            ListFocusTarget::CommitFiles => {
                let Some(row) = self.focused_commit_file_row() else {
                    return Vec::new();
                };
                segment_icons(&self.commit_file_row_paint_segments(&row))
                    .into_iter()
                    .map(|kind| (kind, IconTarget::CommitFileRow(row.id.clone())))
                    .collect()
            }
            ListFocusTarget::None => Vec::new(),
        }
    }

    /// Comments behind the gutter mark of 1-based `line` of `path` in
    /// `checkout`, in store order: the same match as the mark
    /// ([`Self::diff_line_comment`], [`Self::file_line_comment`]). Empty
    /// when the right pane's line viewer ([`Self::line_viewer_target`])
    /// shows another file.
    pub(crate) fn line_comments(
        &self,
        checkout: &str,
        path: &str,
        line: u32,
    ) -> Vec<&CommentEntry> {
        let Some((repo, rel, source)) = self.line_viewer_target() else {
            return Vec::new();
        };
        if repo != checkout || rel != path {
            return Vec::new();
        }
        let snap = self.snapshot.repos.iter().find(|r| r.repo == repo);
        diff_line_comments(
            &self.comment_store,
            repo,
            snap.and_then(|r| r.primary_repo.as_deref()),
            snap.map(|r| r.branch.as_str()),
            rel,
            source,
            line,
        )
    }

    /// Kind and target of the comment mark a line gutter paints beside
    /// 1-based `line` of the right pane's line viewer, from the line's
    /// comment state `comment` ([`Self::diff_line_comment`] or
    /// [`Self::file_line_comment`]). `None` when the line paints no mark.
    pub(crate) fn line_comment_mark(
        &self,
        line: u32,
        comment: Option<bool>,
    ) -> Option<(IconKind, IconTarget)> {
        let resolved = comment?;
        let (checkout, path, _) = self.line_viewer_target()?;
        let kind = if resolved {
            IconKind::CommentResolved
        } else {
            IconKind::Comment
        };
        let target = IconTarget::LineComment {
            checkout: checkout.to_string(),
            path: path.to_string(),
            line,
        };
        Some((kind, target))
    }

    /// Comment marks on the cursor line of the focused line viewer, in
    /// paint order: each gutter cell of the file diff's cursor row (both
    /// cells of a side-by-side row; one section per line), or the
    /// clean-file view's cursor line. Empty when no line viewer has focus
    /// (the PRs tab has none, though its list is no Workspace list), a
    /// folder summary stands in for the diff, or the line paints no mark.
    pub(crate) fn cursor_line_comment_icons(&self) -> Vec<(IconKind, IconTarget)> {
        if !self.line_viewer_has_focus() {
            return Vec::new();
        }
        if self.tabs.active_file_view().is_some() {
            return self.file_cursor_comment_marks();
        }
        if self.folder_summary().is_some() {
            return Vec::new();
        }
        let rows = self.current_diff_rows();
        let split = self.diff_layout() == DiffMode::SideBySide;
        rows.get(self.diff_cursor)
            .map(|row| self.diff_row_comment_marks(row, split))
            .unwrap_or_default()
    }

    /// True when the right pane's line viewer has focus: no list focus
    /// target and not the PRs tab (its list is no Workspace list).
    fn line_viewer_has_focus(&self) -> bool {
        !self.is_prs_tab() && self.list_focus_target() == ListFocusTarget::None
    }

    /// Comment marks of the clean-file view's cursor line (at most one).
    /// Empty with no clean-file view, a cursor past the last line, or a
    /// line with no mark.
    pub(crate) fn file_cursor_comment_marks(&self) -> Vec<(IconKind, IconTarget)> {
        let Some(file) = self.tabs.active_file_view() else {
            return Vec::new();
        };
        if file.cursor >= file.lines().len() {
            return Vec::new();
        }
        let line = u32::try_from(file.cursor + 1).unwrap_or(u32::MAX);
        self.line_comment_mark(line, self.file_line_comment(line))
            .into_iter()
            .collect()
    }

    /// Comment marks of diff row `row` in paint order, one per marked line
    /// (both gutter cells of a `split` side-by-side row), with no repeat.
    pub(crate) fn diff_row_comment_marks(
        &self,
        row: &DiffRow,
        split: bool,
    ) -> Vec<(IconKind, IconTarget)> {
        let mut icons = Vec::new();
        for line in diff_row_gutter_cells(row, split)
            .into_iter()
            .filter_map(|cell| cell.line_no)
        {
            let mark = self.line_comment_mark(line, self.diff_line_comment(line));
            if let Some(mark) = mark.filter(|mark| !icons.contains(mark)) {
                icons.push(mark);
            }
        }
        icons
    }

    /// Comments that paint their mark on graph row `row`, in store order.
    pub(crate) fn graph_row_comments(&self, row: &GraphRow) -> Vec<&CommentEntry> {
        let Some((repo, _)) = self.graph_identity.as_ref() else {
            return Vec::new();
        };
        let snap = self.snapshot.repos.iter().find(|r| r.repo == *repo);
        graph_row_comments(
            &self.comment_store,
            repo,
            snap.and_then(|r| r.primary_repo.as_deref()),
            row,
            snap.map(|r| r.branch.as_str()),
        )
    }

    /// Icons graph row `index` paints, in paint order: the node, the
    /// comment mark, the label glyph, the PR badge, then the worktree marks
    /// and ref chips of its spacer (a checkout / sync mark run gives both
    /// sections). Icons clipped by the pane are included. The sync header
    /// and the selection footer are not part of the row.
    fn graph_row_icons(&self, index: usize) -> Vec<(IconKind, IconTarget)> {
        let Some(model) = self.graph.as_ref() else {
            return Vec::new();
        };
        let Some(row) = model.visible_row_at(index) else {
            return Vec::new();
        };
        let row = &row;
        let id = graph_row_id(row);
        let mut icons = Vec::new();
        let node = match row {
            GraphRow::Commit { is_head: true, .. } => Some(IconKind::GraphHeadCommit),
            GraphRow::Commit { .. } => Some(IconKind::GraphCommit),
            GraphRow::Stash(_) => Some(IconKind::GraphStash),
            GraphRow::Uncommitted { .. } | GraphRow::Worktree(_) => None,
        };
        icons.extend(node.map(|kind| (kind, IconTarget::GraphRow(id.clone()))));
        let comments = self.graph_row_comments(row);
        if !comments.is_empty() {
            let kind = if comments.iter().all(|entry| entry.resolved) {
                IconKind::CommentResolved
            } else {
                IconKind::Comment
            };
            icons.push((kind, IconTarget::GraphRow(id)));
        }
        let glyphs = if self.ascii { &ASCII } else { &UNICODE };
        for line in paint_model(model, glyphs, None)
            .iter()
            .filter(|line| line.row_index == Some(index))
        {
            for part in &line.parts {
                let Some((kind, part_id)) = part.icon() else {
                    continue;
                };
                let Some((kind, target)) = graph_icon(kind, part_id, part.target.as_ref(), row)
                else {
                    continue;
                };
                // A chip's runs (`[`, name, `]`) share one target.
                for icon in icon_targets(kind, &target) {
                    if !icons.contains(&icon) {
                        icons.push(icon);
                    }
                }
            }
            if line.selectable {
                icons.extend(
                    self.graph_pr_badges()
                        .into_iter()
                        .filter(|(badged, _, _)| *badged == index)
                        .map(|(_, repo, mark)| (mark.kind(), IconTarget::PullRequest(repo))),
                );
            }
        }
        icons
    }

    /// True when `target` is on the focused row, so its action lines act
    /// on it and show their gate reasons. A pinned popover always is. The
    /// popover paint asks after the panes paint: a line comment mark
    /// compares against the cursor line marks that frame recorded.
    pub(crate) fn popover_target_focused(&self, target: &IconTarget) -> bool {
        match target {
            IconTarget::ExplorerRow(rel) => {
                self.list_focus_target() == ListFocusTarget::Tree
                    && self.explorer_cursor_rel().as_deref() == Some(rel.as_str())
            }
            IconTarget::TreeRow(id) => {
                self.list_focus_target() == ListFocusTarget::Tree
                    && self.focused_row().is_some_and(|row| &row.id == id)
            }
            IconTarget::CommitFileRow(id) => {
                self.list_focus_target() == ListFocusTarget::CommitFiles
                    && self
                        .focused_commit_file_row()
                        .is_some_and(|row| &row.id == id)
            }
            IconTarget::PullRequest(repo) => self
                .pr_target_for_focus()
                .is_some_and(|(focused, _)| &focused == repo),
            IconTarget::GraphRow(id)
            | IconTarget::GraphChip { row: id, .. }
            | IconTarget::GraphMoreLines(id) => {
                self.list_focus_target() == ListFocusTarget::Graph
                    && self
                        .focused_graph_row()
                        .is_some_and(|row| graph_row_id(&row) == *id)
            }
            IconTarget::GraphSync => self.list_focus_target() == ListFocusTarget::Graph,
            // The footer belongs to the shown commit-file source, not to a
            // list row: it is focused while that source shows.
            IconTarget::CommitDetailMoreLines(id) => {
                self.commit_detail_msg_identity().as_deref() == Some(id.as_str())
            }
            // The marks the last line viewer paint read for the cursor
            // line ([`LayoutHit::cursor_line_marks`]): asking
            // [`Self::cursor_line_comment_icons`] here would build every
            // diff row again on each popover frame.
            IconTarget::LineComment { .. } => {
                self.line_viewer_has_focus() && self.layout.cursor_line_marks.contains(target)
            }
            IconTarget::GraphWorktree(path) => {
                self.list_focus_target() == ListFocusTarget::Graph
                    && self.focused_graph_row().is_some_and(|row| match row {
                        GraphRow::Worktree(worktree) => worktree.path == *path,
                        GraphRow::Commit { worktrees, .. } => {
                            worktrees.iter().any(|worktree| worktree.path == *path)
                        }
                        GraphRow::Uncommitted { .. } | GraphRow::Stash(_) => false,
                    })
            }
            // The commit sections have no action line to gate.
            IconTarget::Commit { .. } => false,
        }
    }

    /// The commit popover card to look up next: the first
    /// [`IconTarget::Commit`] of the open popover (peek or pinned) that is
    /// neither in the loaded graph nor in [`AppState::commit_cards`].
    /// `None` when every shown commit has its data.
    pub(crate) fn commit_card_want(&self) -> Option<(String, String)> {
        self.popover
            .as_ref()?
            .targets
            .iter()
            .find_map(|(_, target)| {
                let IconTarget::Commit { repo, sha } = target else {
                    return None;
                };
                (!sha.is_empty()
                    && commit_card_source(self, repo, sha) == CommitCardSource::Loading)
                    .then(|| (repo.clone(), sha.clone()))
            })
    }

    /// Apply a popover [`Action`].
    pub(super) fn dispatch_popover(&mut self, action: Action) -> Effect {
        match action {
            Action::PopoverOpenFocused => self.open_focused_popover(),
            Action::PopoverMove(delta) => {
                self.move_popover(delta);
                Effect::None
            }
            Action::PopoverRun => self.run_popover_line(None),
            Action::PopoverCopyLine => self.copy_popover_line(),
            Action::PopoverClose => {
                self.close_popover();
                Effect::None
            }
            _ => Effect::None,
        }
    }

    /// `gh`: pin a popover with one section per icon of the focused row.
    fn open_focused_popover(&mut self) -> Effect {
        let targets = self.focused_row_icons();
        if targets.is_empty() {
            self.status = StatusMessage::warn(NO_ICONS_ON_ROW);
            return Effect::None;
        }
        self.pin(targets, None)
    }

    /// Pin a popover on `targets`. Returns the PR detail fetches it starts
    /// ([`Self::request_pr_details`]).
    ///
    /// The landing line is picked after the request, so it counts the
    /// `loading…` line the request adds.
    fn pin(&mut self, targets: Vec<(IconKind, IconTarget)>, anchor: Option<Rect>) -> Effect {
        self.peek = PeekTimers::default();
        self.popover_press = None;
        self.popover = Some(PopoverState {
            origin: PopoverOrigin::Pinned,
            targets,
            anchor,
            focus_line: 0,
            owner: self.popover_owner(),
        });
        let details = self.request_pr_details();
        let landing = landing_line(&flat_lines(&self.open_popover_sections()));
        if let Some(popover) = self.popover.as_mut() {
            popover.focus_line = landing;
        }
        details
    }

    /// The focused line of the pinned popover as its section target and
    /// line key (a field's label or an action's title), or `None`.
    pub(crate) fn popover_focus_key(&self) -> Option<PopoverLineRef> {
        let popover = self.popover.as_ref().filter(|_| self.popover_pinned())?;
        let sections = self.open_popover_sections();
        let focus = focused_line(&flat_lines(&sections), popover.focus_line)?;
        line_ref(&sections, focus)
    }

    /// Point the pinned popover's focus back at the line `key` named
    /// before its content changed (a PR detail landed and the section grew).
    /// A line that is gone leaves the focus where it is.
    pub(crate) fn restore_popover_focus(&mut self, key: Option<PopoverLineRef>) {
        let Some(key) = key else {
            return;
        };
        let found = line_index(&self.open_popover_sections(), &key);
        if let (Some(index), Some(popover)) = (found, self.popover.as_mut()) {
            popover.focus_line = index;
        }
    }

    /// Pin the popover of the clicked icon `hit`. Its row is already
    /// selected. Returns the PR detail fetches it starts.
    pub(super) fn pin_icon(&mut self, hit: &IconHit) -> Effect {
        self.pin(icon_targets(hit.kind, &hit.target), Some(hit.rect()))
    }

    /// Close any popover and drop the peek timers.
    pub(crate) fn close_popover(&mut self) {
        self.popover = None;
        self.popover_press = None;
        self.peek = PeekTimers::default();
    }

    /// Close a peek (never a pinned popover) and drop the peek timers.
    pub(crate) fn drop_peek(&mut self) {
        if self
            .popover
            .as_ref()
            .is_some_and(|popover| !popover.is_pinned())
        {
            self.popover = None;
        }
        self.peek = PeekTimers::default();
    }

    fn move_popover(&mut self, delta: i32) {
        if !self.popover_pinned() {
            self.close_popover();
            return;
        }
        let sections = self.open_popover_sections();
        let lines = flat_lines(&sections);
        if let Some(popover) = self.popover.as_mut() {
            popover.focus_line = step_line(&lines, popover.focus_line, delta);
        }
    }

    /// The focused line of the pinned popover, rebuilt from live state.
    fn focused_popover_line(&self) -> Option<PopoverLine> {
        let popover = self.popover.as_ref().filter(|_| self.popover_pinned())?;
        let sections = popover_sections(self, &popover.targets, super::unix_now());
        let lines = flat_lines(&sections);
        let index = focused_line(&lines, popover.focus_line)?;
        Some(lines[index].clone())
    }

    /// The action Enter would run in the pinned popover, if any. Busy
    /// gating classifies Enter as this action.
    pub(crate) fn popover_run_action(&self) -> Option<Action> {
        match self.focused_popover_line()? {
            PopoverLine::Action { command, .. } => Some(command.action.clone()),
            PopoverLine::Text(_)
            | PopoverLine::Note(_)
            | PopoverLine::Rule
            | PopoverLine::Field { .. } => None,
        }
    }

    /// The action a left release would run: the action line a pinned
    /// popover press landed on, while that press has not become a drag.
    /// Busy gating classifies the release as this action.
    pub(crate) fn popover_release_action(&self) -> Option<Action> {
        let press = self.popover_press.as_ref()?;
        if self.text_selection.is_some_and(|sel| sel.is_active()) || !self.popover_pinned() {
            return None;
        }
        let sections = self.open_popover_sections();
        match flat_lines(&sections).get(line_index(&sections, press)?)? {
            PopoverLine::Action { command, .. } => Some(command.action.clone()),
            PopoverLine::Text(_)
            | PopoverLine::Note(_)
            | PopoverLine::Rule
            | PopoverLine::Field { .. } => None,
        }
    }

    /// Gate reason of action line `command` in popover section `kind` on
    /// `target`, or `None` when it runs.
    ///
    /// View remotely asks the section's own entity
    /// ([`Self::view_remotely_refusal_at`]), so a tag chip in the
    /// uncommitted row's footer is not refused as an uncommitted row.
    /// Every other line asks the palette gate.
    pub(crate) fn popover_line_refusal(
        &self,
        command: &PaletteCommand,
        kind: IconKind,
        target: &IconTarget,
    ) -> Option<String> {
        if command.action != Action::ViewRemotely {
            return self.palette_disabled_reason(command);
        }
        self.palette_scope_refusal(command)
            .or_else(|| self.view_remotely_refusal_at(kind, target))
    }

    /// Run line `line` (Enter: the focused line) of the pinned popover.
    ///
    /// An action line closes the popover, then dispatches its action. A
    /// disabled one keeps the popover and shows the gate reason, like the
    /// palette ([`Self::popover_line_refusal`]). View remotely opens the
    /// entity of the line's own section ([`Self::view_remotely_at`]), not
    /// the focused row: a tag chip on a commit row opens the tag. The PR
    /// lines of a PR section (Open PR, the CI page, Copy URL, Refresh this
    /// PR) act on that section's PR ([`Self::run_pr_action_at`]), so the
    /// graph header chip's popover acts on its checkout, not the focused
    /// graph row. Any other line does nothing.
    fn run_popover_line(&mut self, line: Option<usize>) -> Effect {
        if !self.popover_pinned() {
            self.close_popover();
            return Effect::None;
        }
        let sections = self.open_popover_sections();
        let index = match (line, self.popover.as_ref()) {
            (Some(index), _) => Some(index),
            (None, Some(popover)) => focused_line(&flat_lines(&sections), popover.focus_line),
            (None, None) => None,
        };
        let Some((_, section, PopoverLine::Action { command, .. })) =
            index.and_then(|index| section_lines(&sections).nth(index))
        else {
            return Effect::None;
        };
        let (command, kind, target) = (*command, section.icon, section.target.clone());
        if let Some(reason) = self.popover_line_refusal(command, kind, &target) {
            self.status = StatusMessage::warn(reason);
            return Effect::None;
        }
        self.close_popover();
        match (&command.action, &target) {
            (Action::ViewRemotely, _) => self.view_remotely_at(kind, &target),
            (action, IconTarget::PullRequest(repo)) => self
                .run_pr_action_at(action, repo)
                .unwrap_or_else(|| self.dispatch_palette_action(action.clone())),
            (action, _) => self.dispatch_palette_action(action.clone()),
        }
    }

    /// `y`: copy the focused line of the pinned popover. It stays open.
    ///
    /// On the PR section's Copy URL line it copies the PR URL, as the
    /// line's `y` chip says, not the line's title.
    fn copy_popover_line(&mut self) -> Effect {
        if !self.popover_pinned() {
            self.close_popover();
            return Effect::None;
        }
        let sections = self.open_popover_sections();
        let focus = self
            .popover
            .as_ref()
            .and_then(|popover| focused_line(&flat_lines(&sections), popover.focus_line));
        let Some((_, section, line)) = focus.and_then(|index| section_lines(&sections).nth(index))
        else {
            return Effect::None;
        };
        if let (PopoverLine::Action { command, .. }, IconTarget::PullRequest(repo)) =
            (line, &section.target)
        {
            if command.action == Action::CopyPullRequestUrl {
                if let Some(effect) = self.run_pr_action_at(&command.action, repo) {
                    return effect;
                }
            }
        }
        Effect::CopyClipboard {
            text: line.copy_text(),
            announce: true,
        }
    }

    /// The painted icon at screen cell (`col`, `row`).
    pub(super) fn icon_hit_at(&self, col: u16, row: u16) -> Option<&IconHit> {
        self.layout
            .icon_hits
            .iter()
            .find(|hit| hit.contains(col, row))
    }

    pub(super) fn pointer_in_popover(&self) -> bool {
        let (Some((col, row)), Some(painted)) = (self.pointer, self.layout.popover.as_ref()) else {
            return false;
        };
        painted.rect.contains(Position::new(col, row))
    }

    /// The painted icon under [`Self::pane_pointer`]. An icon under the
    /// painted popover, or under the open search-in-files dialog, is not
    /// hovered.
    pub fn hovered_icon(&self) -> Option<&IconHit> {
        // The search dialog keeps the pointer for its chips; pane_pointer
        // hides it from the icons the dialog covers, so no peek arms.
        let (col, row) = self.pane_pointer()?;
        if self.pointer_in_popover() {
            return None;
        }
        self.icon_hit_at(col, row)
    }

    /// Arm or drop the peek dwell and leave grace for the pointer over
    /// `icon` (the hovered icon, if any).
    ///
    /// A new icon arms a [`PEEK_DWELL_MS`] dwell; leaving every icon drops
    /// it. While a peek shows, the pointer off both the peek and its icon
    /// starts a [`PEEK_GRACE_MS`] grace, and the pointer back on either
    /// cancels it. A pinned popover shows no peek.
    pub(super) fn track_peek(&mut self, icon: Option<&IconHit>, now: Instant) {
        if self.popover.as_ref().is_some_and(PopoverState::is_pinned) {
            self.peek = PeekTimers::default();
            return;
        }
        let shown = self
            .popover
            .as_ref()
            .filter(|popover| popover.origin == PopoverOrigin::Peek);
        let on_shown = icon.is_some_and(|hit| {
            shown.is_some_and(|popover| {
                popover.anchor == Some(hit.rect())
                    && popover.targets.first() == Some(&(hit.kind, hit.target.clone()))
            })
        });
        let peek_shown = shown.is_some();
        let inside = on_shown || self.pointer_in_popover();
        match icon {
            Some(hit) if !on_shown => {
                if self
                    .peek
                    .dwell
                    .as_ref()
                    .is_none_or(|(armed, _)| armed != hit)
                {
                    let at = now + Duration::from_millis(PEEK_DWELL_MS);
                    self.peek.dwell = Some((hit.clone(), at));
                }
            }
            _ => self.peek.dwell = None,
        }
        if !peek_shown || inside {
            self.peek.grace = None;
        } else if self.peek.grace.is_none() {
            self.peek.grace = Some(now + Duration::from_millis(PEEK_GRACE_MS));
        }
    }

    /// [`Self::expire_peek`], then the PR detail fetch of the peek it
    /// opened ([`Self::request_pr_details`]).
    ///
    /// The first value is true when the popover changed (the live loop
    /// redraws); the effect is what the loop schedules. A peek on a PR
    /// whose detail is already cached or loading asks nothing.
    pub(crate) fn expire_peek_with_details(&mut self, now: Instant) -> (bool, Effect) {
        if !self.expire_peek(now) {
            return (false, Effect::None);
        }
        (true, self.request_pr_details())
    }

    /// Milliseconds until the next peek dwell or grace deadline, or `None`
    /// when neither is armed.
    pub fn peek_remaining_ms(&self, now: Instant) -> Option<u64> {
        let dwell = self.peek.dwell.as_ref().map(|(_, at)| *at);
        [dwell, self.peek.grace]
            .into_iter()
            .flatten()
            .min()
            .map(|at| at.saturating_duration_since(now).as_millis() as u64)
    }

    fn peek_allowed(&self) -> bool {
        self.mouse_enabled
            && !self.too_small
            && !self.popover_pinned()
            && matches!(
                self.input_mode(),
                InputMode::Normal { .. }
                    | InputMode::ZPending { .. }
                    | InputMode::GPending { .. }
                    | InputMode::DiffVisual
            )
    }

    /// Open the peek whose dwell ended, close the peek whose grace ended.
    ///
    /// Returns true when the popover changed. The live loop calls it
    /// through [`Self::expire_peek_with_details`].
    pub fn expire_peek(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.peek.dwell.as_ref().is_some_and(|(_, at)| *at <= now) {
            let hit = self.peek.dwell.take().map(|(hit, _)| hit);
            if let Some(hit) = hit.filter(|_| self.peek_allowed()) {
                self.popover = Some(PopoverState {
                    origin: PopoverOrigin::Peek,
                    targets: icon_targets(hit.kind, &hit.target),
                    anchor: Some(hit.rect()),
                    focus_line: 0,
                    owner: self.popover_owner(),
                });
                self.peek.grace = None;
                changed = true;
            }
        }
        if self.peek.grace.is_some_and(|at| at <= now) {
            self.peek.grace = None;
            if self
                .popover
                .as_ref()
                .is_some_and(|popover| !popover.is_pinned())
            {
                self.popover = None;
                changed = true;
            }
        }
        changed
    }

    /// Check the open popover against this frame's icon hits.
    ///
    /// Paint calls this after the panes record their icons. A peek stays
    /// only while its icon still paints where it opened and no overlay is
    /// up. A pinned popover closes once its focus moved; otherwise it
    /// follows its first painted icon (it keeps its last place when every
    /// icon is scrolled off).
    pub(crate) fn sync_popover_with_layout(&mut self) {
        let Some(popover) = self.popover.as_ref() else {
            return;
        };
        // Cells of every painted icon of `target` (a kind can tag two
        // segments of a row: the workspace glyph and its summary).
        let rects_of = |(kind, target): &(IconKind, IconTarget)| -> Vec<Rect> {
            self.layout
                .icon_hits
                .iter()
                .filter(|hit| hit.kind == *kind && hit.target == *target)
                .map(IconHit::rect)
                .collect()
        };
        let at_anchor = popover.anchor.is_some_and(|anchor| {
            popover
                .targets
                .iter()
                .any(|target| rects_of(target).contains(&anchor))
        });
        if popover.is_pinned() {
            // A dialog that opened under it (a confirm from a popover
            // action, a picker that loaded later) owns the keys: close.
            if !self.popover_pinned() || open_dialog(self).is_some() {
                self.close_popover();
                return;
            }
            let anchor = popover
                .targets
                .iter()
                .find_map(|target| rects_of(target).first().copied());
            if let (false, Some(anchor), Some(popover)) = (at_anchor, anchor, self.popover.as_mut())
            {
                popover.anchor = Some(anchor);
            }
            return;
        }
        let still_there = popover
            .targets
            .first()
            .zip(popover.anchor)
            .is_some_and(|(target, anchor)| rects_of(target).contains(&anchor));
        if !still_there || !self.peek_allowed() {
            self.drop_peek();
        }
    }

    /// A left press while a popover is open, or `None` to handle it as
    /// usual.
    ///
    /// Inside a pinned popover the press focuses the line under it, arms a
    /// text selection on the popover body, and the release runs an action
    /// line unless the press became a drag. Outside it the press only
    /// closes the popover. Inside a peek the press selects the peek's row
    /// and pins the peek (`plain` at the peek's icon). Outside a peek the
    /// peek closes and the press goes on.
    pub(super) fn popover_click(
        &mut self,
        col: u16,
        row: u16,
        plain: PlainClick,
    ) -> Option<Effect> {
        let popover = self.popover.as_ref()?;
        let painted = self.layout.popover.clone();
        let inside = painted
            .as_ref()
            .is_some_and(|painted| painted.rect.contains(Position::new(col, row)));
        if popover.is_pinned() {
            if !self.popover_pinned() {
                self.close_popover();
                return None;
            }
            self.last_click = None;
            self.popover_press = None;
            let Some(painted) = painted.filter(|_| inside) else {
                self.close_popover();
                self.text_selection = None;
                return Some(Effect::None);
            };
            self.text_selection = TextSelection::arm(painted.inner, col, row);
            if let Some(&(_, line)) = painted.lines.iter().find(|(y, _)| *y == row) {
                if let Some(popover) = self.popover.as_mut() {
                    popover.focus_line = line;
                }
                // Keyed, not indexed: a PR detail that lands before the
                // release moves the lines.
                self.popover_press = line_ref(&self.open_popover_sections(), line);
            }
            return Some(Effect::None);
        }
        if !inside {
            self.drop_peek();
            return None;
        }
        let targets = popover.targets.clone();
        let anchor = popover.anchor?;
        let selected = plain(self, anchor.x, anchor.y);
        self.last_click = None;
        self.text_selection = None;
        let details = self.pin(targets, Some(anchor));
        Some(then(selected, details))
    }

    /// Left release: copy a drag selection, or run the popover action line
    /// the press landed on.
    pub(super) fn release_mouse(&mut self) -> Effect {
        let press = self.popover_press.take();
        let dragged = self.text_selection.is_some_and(|sel| sel.is_active());
        let copied = self.finish_text_selection();
        let line = press
            .filter(|_| !dragged)
            .and_then(|press| line_index(&self.open_popover_sections(), &press));
        match line {
            Some(line) => self.run_popover_line(Some(line)),
            None => copied,
        }
    }
}

/// What identifies a focusable popover line inside its section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PopoverLineKey {
    /// A field by its label.
    Field(&'static str),
    /// A field with a blank label (one of the lines that continue the
    /// field above it, such as the PR check names) by its value.
    Continued(String),
    /// An action by its palette title, so a section's own title for it
    /// (a CI label that changes) keeps the focus.
    Action(&'static str),
}

fn line_key(line: &PopoverLine) -> Option<PopoverLineKey> {
    match line {
        PopoverLine::Field {
            label: "", value, ..
        } => Some(PopoverLineKey::Continued(value.clone())),
        PopoverLine::Field { label, .. } => Some(PopoverLineKey::Field(label)),
        PopoverLine::Action { command, .. } => Some(PopoverLineKey::Action(command.title)),
        PopoverLine::Text(_) | PopoverLine::Note(_) | PopoverLine::Rule => None,
    }
}

/// A focusable popover line by its section target and [`PopoverLineKey`]:
/// it still names the same line after the content above it changes.
pub(crate) type PopoverLineRef = (IconTarget, PopoverLineKey);

/// The [`PopoverLineRef`] of [`flat_lines`] line `index` of `sections`;
/// `None` for a line that cannot take the focus.
fn line_ref(sections: &[PopoverSection], index: usize) -> Option<PopoverLineRef> {
    let (_, section, line) = section_lines(sections).nth(index)?;
    Some((section.target.clone(), line_key(line)?))
}

/// The [`flat_lines`] index of the line `key` names in `sections`.
fn line_index(sections: &[PopoverSection], (target, key): &PopoverLineRef) -> Option<usize> {
    section_lines(sections)
        .find(|(_, section, line)| {
            section.target == *target && line_key(line).as_ref() == Some(key)
        })
        .map(|(index, _, _)| index)
}

/// Every line of `sections` with its [`flat_lines`] index and section.
fn section_lines(
    sections: &[PopoverSection],
) -> impl Iterator<Item = (usize, &PopoverSection, &PopoverLine)> {
    sections
        .iter()
        .flat_map(|section| section.lines.iter().map(move |line| (section, line)))
        .enumerate()
        .map(|(index, (section, line))| (index, section, line))
}

/// `first`, then `second`, as one effect.
pub(super) fn then(first: Effect, second: Effect) -> Effect {
    match (first, second) {
        (Effect::None, second) => second,
        (first, Effect::None) => first,
        (first, second) => Effect::Batch(vec![first, second]),
    }
}

#[cfg(test)]
mod tests;

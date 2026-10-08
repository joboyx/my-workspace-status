//! Icon popover state: peek timers, pin, `gh`, popover keys, and clicks.
//!
//! The model and the content builders live in [`super::super::popover`].

use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};

use super::super::action::{Action, Effect};
use super::super::chrome::open_dialog;
use super::super::comments::{
    commit_file_row_comments, commit_file_row_comments_resolved, commit_file_row_has_comment,
    graph_row_comments, tree_row_comments_resolved, tree_row_has_comment, CommentEntry,
};
use super::super::commit_files::CommitFileRow;
use super::super::drill::CommitFileSource;
use super::super::gates::ListFocusTarget;
use super::super::icons::IconKind;
use super::super::keys::InputMode;
use super::super::popover::{
    flat_lines, focused_line, graph_icon, graph_row_id, icon_targets, landing_line,
    popover_sections, step_line, tree_icon_target, IconTarget, PopoverLine, PopoverOrigin,
    PopoverOwner, PopoverSection, PopoverState, PEEK_DWELL_MS, PEEK_GRACE_MS,
};
use super::super::pull_request::PrState;
use super::super::selection::TextSelection;
use super::super::status::StatusMessage;
use super::super::tree::{
    painted_row_segments, pr_badge_kind, pr_badge_repo, segment_icons, with_comment_mark,
    with_viewed_mark, NodeKind, NodeSegments, VisibleRow,
};
use super::{AppState, IconHit};
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

/// Viewed, commented, resolved, and PR badge state of one tree row.
pub(crate) type TreeRowMarks = (bool, bool, bool, Option<PrState>);

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
            ListFocusTarget::None => String::new(),
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

    /// Viewed, comment, and PR badge marks tree row `row` paints.
    pub(crate) fn tree_row_marks(&self, row: &VisibleRow) -> TreeRowMarks {
        let viewed = row.kind == NodeKind::File && self.reviewed.contains(&row.id);
        let commented = tree_row_has_comment(&self.comment_store, &self.snapshot, row);
        let resolved =
            commented && tree_row_comments_resolved(&self.comment_store, &self.snapshot, row);
        let pr = pr_badge_repo(row).and_then(|repo| self.pr_badge(Path::new(repo)));
        (viewed, commented, resolved, pr)
    }

    /// Every segment tree row `row` paints this frame.
    pub(crate) fn tree_row_paint_segments(&self, row: &VisibleRow) -> NodeSegments {
        let (viewed, commented, resolved, pr) = self.tree_row_marks(row);
        painted_row_segments(row, self.ascii, viewed, commented, resolved, pr)
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
    fn focused_row_icons(&self) -> Vec<(IconKind, IconTarget)> {
        if self.is_file_tab() || self.is_explorer_tab() {
            return Vec::new();
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
                        .map(|(_, repo, pr)| (pr_badge_kind(pr), IconTarget::PullRequest(repo))),
                );
            }
        }
        icons
    }

    /// True when `target` is on the focused row, so its action lines act
    /// on it and show their gate reasons. A pinned popover always is.
    pub(crate) fn popover_target_focused(&self, target: &IconTarget) -> bool {
        match target {
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
        }
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
    /// ([`Self::request_popover_pr_details`]).
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
        let details = self.request_popover_pr_details();
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
            PopoverLine::Action(command) => Some(command.action.clone()),
            PopoverLine::Text(_) | PopoverLine::Note(_) | PopoverLine::Field { .. } => None,
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
            PopoverLine::Action(command) => Some(command.action.clone()),
            PopoverLine::Text(_) | PopoverLine::Note(_) | PopoverLine::Field { .. } => None,
        }
    }

    /// Run line `line` (Enter: the focused line) of the pinned popover.
    ///
    /// An action line closes the popover, then dispatches its action. A
    /// disabled one keeps the popover and shows the gate reason, like the
    /// palette. Any other line does nothing.
    fn run_popover_line(&mut self, line: Option<usize>) -> Effect {
        if !self.popover_pinned() {
            self.close_popover();
            return Effect::None;
        }
        let line = match line {
            Some(index) => {
                let sections = self.open_popover_sections();
                flat_lines(&sections).get(index).map(|line| (*line).clone())
            }
            None => self.focused_popover_line(),
        };
        let Some(PopoverLine::Action(command)) = line else {
            return Effect::None;
        };
        if let Some(reason) = self.palette_disabled_reason(command) {
            self.status = StatusMessage::warn(reason);
            return Effect::None;
        }
        self.close_popover();
        self.dispatch_palette_action(command.action.clone())
    }

    /// `y`: copy the focused line of the pinned popover. It stays open.
    fn copy_popover_line(&mut self) -> Effect {
        if !self.popover_pinned() {
            self.close_popover();
            return Effect::None;
        }
        match self.focused_popover_line() {
            Some(line) => Effect::CopyClipboard {
                text: line.copy_text(),
                announce: true,
            },
            None => Effect::None,
        }
    }

    /// The painted icon at screen cell (`col`, `row`).
    pub(super) fn icon_hit_at(&self, col: u16, row: u16) -> Option<&IconHit> {
        self.layout
            .icon_hits
            .iter()
            .find(|hit| hit.contains(col, row))
    }

    fn pointer_in_popover(&self) -> bool {
        let (Some((col, row)), Some(painted)) = (self.pointer, self.layout.popover.as_ref()) else {
            return false;
        };
        painted.rect.contains(Position::new(col, row))
    }

    /// The painted icon under [`Self::pointer`]. An icon under the painted
    /// popover is not hovered.
    pub fn hovered_icon(&self) -> Option<&IconHit> {
        let (col, row) = self.pointer?;
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
    /// opened ([`Self::request_popover_pr_details`]).
    ///
    /// The first value is true when the popover changed (the live loop
    /// redraws); the effect is what the loop schedules. A peek on a PR
    /// whose detail is already cached or loading asks nothing.
    pub(crate) fn expire_peek_with_details(&mut self, now: Instant) -> (bool, Effect) {
        if !self.expire_peek(now) {
            return (false, Effect::None);
        }
        (true, self.request_popover_pr_details())
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
    /// and pins the peek. Outside a peek the peek closes and the press goes
    /// on.
    pub(super) fn popover_click(&mut self, col: u16, row: u16) -> Option<Effect> {
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
        let selected = self.click_at(anchor.x, anchor.y);
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

/// What identifies a focusable popover line inside its section: `true`
/// and the title for an action, `false` and the label for a field.
pub(crate) type PopoverLineKey = (bool, &'static str);

fn line_key(line: &PopoverLine) -> Option<PopoverLineKey> {
    match line {
        PopoverLine::Field { label, .. } => Some((false, label)),
        PopoverLine::Action(command) => Some((true, command.title)),
        PopoverLine::Text(_) | PopoverLine::Note(_) => None,
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
        .find(|(_, section, line)| section.target == *target && line_key(line) == Some(*key))
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
mod tests {
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    use super::super::super::command_palette::command_for;
    use super::super::super::event_pump::overlay_blocks_background_ticks;
    use super::super::super::keys::event_to_action;
    use super::super::super::pull_request::{PrLookup, PullRequest};
    use super::super::super::render::draw;
    use super::super::FocusPane;
    use super::*;
    use crate::helpers::STATUS_FAILED_NOTE;
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::comments::{put_comment, CommentKey};
    use crate::tui::tree::SegRole;
    use workspace_status_graph::{
        format_local_timestamp, format_relative_date, Commit, GraphModel, GraphRef, PartTarget,
        Stash, SyncState, SyncStatus as GraphSyncStatus, Worktree,
    };

    const PR_REMOTE: &str = "https://github.com/octo/demo.git";

    fn repo(name: &str, status: SyncStatus, note: &str) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "feature".into(),
            sync_status: status,
            sync_note: note.into(),
            head: "abc".into(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: false,
            changes: vec![FileChange {
                path: "src/lib.rs".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    /// `app` two commits behind, `lib` one ahead; cursor on `lib`.
    fn app() -> AppState {
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", SyncStatus::Behind, "behind by 2 commits"),
                repo("lib", SyncStatus::Ahead, "ahead by 1 commits"),
            ],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        focus(&mut app, "repo:lib");
        app
    }

    fn focus(app: &mut AppState, id: &str) {
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.id == id)
            .unwrap_or_else(|| panic!("missing row {id}"));
    }

    fn paint(app: &mut AppState) -> Terminal<TestBackend> {
        let mut terminal = Terminal::new(TestBackend::new(120, 24)).unwrap();
        terminal.draw(|frame| draw(frame, app)).unwrap();
        terminal
    }

    /// The sync mark hit on tree row `id`.
    fn sync_hit(app: &AppState, id: &str) -> IconHit {
        app.layout
            .icon_hits
            .iter()
            .find(|hit| {
                hit.target == IconTarget::TreeRow(id.into())
                    && matches!(hit.kind, IconKind::Ahead | IconKind::Behind)
            })
            .cloned()
            .unwrap_or_else(|| panic!("no icon hit on {id}: {:?}", app.layout.icon_hits))
    }

    fn focused_id(app: &AppState) -> String {
        app.focused_row().expect("focused row").id.clone()
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, KeyModifiers::NONE))
    }

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// Open a peek over `hit` with the pointer resting on it from `at`.
    fn peek_at(app: &mut AppState, hit: &IconHit, at: Instant) {
        app.set_pointer_at(Some((hit.x, hit.y)), at);
        assert!(app.expire_peek(at + ms(PEEK_DWELL_MS)));
    }

    fn section_lines(app: &AppState) -> Vec<Vec<PopoverLine>> {
        app.open_popover_sections()
            .into_iter()
            .map(|section| section.lines)
            .collect()
    }

    /// Pin the popover of the sync mark on tree row `id`, as a click on it
    /// does.
    fn pin_sync(app: &mut AppState, id: &str) {
        focus(app, id);
        paint(app);
        let hit = sync_hit(app, id);
        app.pin_icon(&hit);
        assert!(app.popover_pinned());
    }

    fn copy(text: &str) -> Effect {
        Effect::CopyClipboard {
            text: text.into(),
            announce: true,
        }
    }

    #[test]
    fn sync_mark_records_a_hit_on_its_glyph_cells() {
        let mut app = app();
        let terminal = paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        assert_eq!(hit.kind, IconKind::Behind);
        assert_eq!(hit.width, 2);
        let buf = terminal.backend().buffer();
        assert_eq!(buf[(hit.x, hit.y)].symbol(), "v");
        assert_eq!(buf[(hit.x + 1, hit.y)].symbol(), "2");
        assert_eq!(sync_hit(&app, "repo:lib").kind, IconKind::Ahead);
        assert!(
            app.layout.pr_badge_hits().is_empty(),
            "sync marks are not PRs"
        );
    }

    #[test]
    fn dwell_opens_a_peek_that_is_not_a_mode() {
        let mut app = app();
        paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        let t0 = Instant::now();
        assert!(!app.set_pointer_at(Some((0, 0)), t0), "off every icon");
        assert_eq!(app.peek_remaining_ms(t0), None);
        assert!(
            app.set_pointer_at(Some((hit.x, hit.y)), t0),
            "onto the icon"
        );
        assert!(
            !app.set_pointer_at(Some((hit.x + 1, hit.y)), t0),
            "motion on the same icon does not redraw"
        );
        assert_eq!(app.peek_remaining_ms(t0), Some(PEEK_DWELL_MS));
        assert!(!app.expire_peek(t0 + ms(PEEK_DWELL_MS - 1)));
        assert!(app.popover.is_none());
        assert!(app.expire_peek(t0 + ms(PEEK_DWELL_MS)));
        let popover = app.popover.as_ref().expect("peek");
        assert_eq!(popover.origin, PopoverOrigin::Peek);
        assert_eq!(popover.anchor, Some(hit.rect()));
        assert_eq!(
            app.input_mode(),
            InputMode::Normal {
                search_active: false
            },
            "a peek takes no keys"
        );
        assert_eq!(focused_id(&app), "repo:lib", "a peek selects nothing");
        assert_eq!(app.peek_remaining_ms(t0), None, "nothing left to time");
    }

    #[test]
    fn leaving_starts_the_grace_and_the_peek_itself_keeps_it() {
        let mut app = app();
        paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        let t0 = Instant::now();
        peek_at(&mut app, &hit, t0);
        paint(&mut app);
        let rect = app.layout.popover.as_ref().expect("painted peek").rect;

        let t1 = t0 + ms(1_000);
        app.set_pointer_at(Some((0, 0)), t1);
        assert_eq!(app.peek_remaining_ms(t1), Some(PEEK_GRACE_MS));
        app.set_pointer_at(Some((rect.x + 1, rect.y + 1)), t1 + ms(100));
        assert_eq!(app.peek_remaining_ms(t1), None, "pointer on the peek");
        assert!(!app.expire_peek(t1 + ms(PEEK_GRACE_MS)));
        assert!(app.popover.is_some());

        let t2 = t1 + ms(2_000);
        app.set_pointer_at(Some((0, 0)), t2);
        assert!(!app.expire_peek(t2 + ms(PEEK_GRACE_MS - 1)));
        assert!(app.expire_peek(t2 + ms(PEEK_GRACE_MS)));
        assert!(app.popover.is_none(), "grace ended");
    }

    #[test]
    fn mouse_off_and_overlays_drop_the_peek() {
        let mut app = app();
        paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        peek_at(&mut app, &hit, Instant::now());
        app.dispatch(Action::ToggleMouse);
        assert!(app.popover.is_none(), "mouse off");
        assert!(!app.set_pointer_at(Some((hit.x, hit.y)), Instant::now()));
        assert_eq!(app.peek_remaining_ms(Instant::now()), None);

        app.dispatch(Action::ToggleMouse);
        paint(&mut app);
        peek_at(&mut app, &hit, Instant::now());
        app.dispatch(Action::ToggleHelp);
        paint(&mut app);
        assert!(app.popover.is_none(), "help hides the peek");
    }

    #[test]
    fn click_on_an_icon_selects_its_row_and_pins_without_a_double_click() {
        let mut app = app();
        paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        assert_eq!(
            app.dispatch(Action::Click {
                col: hit.x,
                row: hit.y
            }),
            Effect::LoadRightPane
        );
        assert_eq!(focused_id(&app), "repo:app");
        assert_eq!(app.input_mode(), InputMode::Popover);
        assert!(
            app.last_click.is_none(),
            "an icon click arms no double-click"
        );
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        paint(&mut app);
        assert!(app.layout.popover.is_some());

        // A second click on the icon is outside the popover: it only closes.
        assert_eq!(
            app.dispatch(Action::Click {
                col: hit.x,
                row: hit.y
            }),
            Effect::None
        );
        assert!(app.popover.is_none());
        assert_eq!(app.focus, FocusPane::Left, "no drill");
        assert!(app.last_click.is_none());
    }

    #[test]
    fn click_outside_only_closes_and_double_click_elsewhere_still_drills() {
        let mut app = app();
        paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        app.dispatch(Action::Click {
            col: hit.x,
            row: hit.y,
        });
        paint(&mut app);
        let lib = sync_hit(&app, "repo:lib");
        // A press on another row while pinned closes and selects nothing.
        assert_eq!(
            app.dispatch(Action::Click { col: 8, row: lib.y }),
            Effect::None
        );
        assert!(app.popover.is_none());
        assert_eq!(focused_id(&app), "repo:app");

        paint(&mut app);
        app.dispatch(Action::Click { col: 8, row: lib.y });
        app.dispatch(Action::Click { col: 8, row: lib.y });
        assert_eq!(focused_id(&app), "repo:lib");
        assert_eq!(
            app.focus,
            FocusPane::Right,
            "double-click on the label drills"
        );
        assert!(app.popover.is_none());
    }

    #[test]
    fn click_inside_a_peek_pins_it_on_its_row() {
        let mut app = app();
        paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        peek_at(&mut app, &hit, Instant::now());
        paint(&mut app);
        let rect = app.layout.popover.as_ref().expect("peek").rect;
        assert_eq!(
            app.dispatch(Action::Click {
                col: rect.x + 2,
                row: rect.y + 1
            }),
            Effect::LoadRightPane
        );
        assert_eq!(focused_id(&app), "repo:app");
        assert!(app.popover_pinned());
        assert_eq!(app.input_mode(), InputMode::Popover);
    }

    #[test]
    fn esc_closes_the_pinned_popover_before_the_esc_chain() {
        let mut app = app();
        app.search_active = true;
        app.search_query = "app".into();
        focus(&mut app, "repo:app");
        app.dispatch(Action::PopoverOpenFocused);
        let mode = app.input_mode();
        assert_eq!(mode, InputMode::Popover);
        let esc = event_to_action(&key(KeyCode::Esc), mode, false, false);
        assert_eq!(esc, Action::PopoverClose);
        assert_eq!(app.dispatch(esc), Effect::None);
        assert!(app.popover.is_none());
        assert!(app.search_active, "the armed search outlives the first Esc");
        assert_eq!(focused_id(&app), "repo:app");
        assert_eq!(
            event_to_action(&key(KeyCode::Esc), app.input_mode(), false, false),
            Action::NavEsc,
            "the next Esc is the usual chain"
        );
    }

    #[test]
    fn focus_change_closes_the_pinned_popover() {
        let mut app = app();
        focus(&mut app, "repo:app");
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned());
        focus(&mut app, "repo:lib");
        assert!(!app.popover_pinned(), "another row");
        assert_ne!(app.input_mode(), InputMode::Popover);
        paint(&mut app);
        assert!(app.popover.is_none(), "paint drops it");

        focus(&mut app, "repo:app");
        app.dispatch(Action::PopoverOpenFocused);
        app.focus = FocusPane::Right;
        assert!(!app.popover_pinned(), "another pane");

        app.focus = FocusPane::Left;
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned());
        app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
        assert!(app.is_compare_tab());
        assert!(!app.popover_pinned(), "another tab");
        assert_ne!(app.input_mode(), InputMode::Popover);

        // The tab alone is enough: same list and row, other tab.
        app.dispatch(Action::JumpToTab(1));
        focus(&mut app, "repo:app");
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned());
        app.popover.as_mut().unwrap().owner.tab += 1;
        assert!(!app.popover_pinned(), "pinned on another tab");
    }

    #[test]
    fn ticks_keep_running_under_a_pinned_popover() {
        assert!(!overlay_blocks_background_ticks(InputMode::Popover));
        let mut app = app();
        focus(&mut app, "repo:app");
        app.dispatch(Action::PopoverOpenFocused);
        app.dispatch(Action::WatchTick);
        assert!(app.popover_pinned(), "a tick on the same row keeps it");
    }

    #[test]
    fn gh_lists_every_icon_of_the_row_in_paint_order() {
        let mut app = app();
        assert_eq!(app.due_pr_lookups().len(), 2);
        assert!(app.apply_pr_lookup(
            Path::new("app"),
            "feature",
            Some(PR_REMOTE.into()),
            PrLookup::Found(PullRequest {
                number: 7,
                url: "https://github.com/octo/demo/pull/7".into(),
                state: PrState::Open,
            }),
        ));
        focus(&mut app, "repo:app");
        assert_eq!(
            app.dispatch(Action::PopoverOpenFocused),
            Effect::LookupPullRequestDetail {
                repo: PathBuf::from("app"),
                branch: "feature".into(),
                remote: PR_REMOTE.into(),
                number: 7,
                request: 1,
            },
            "the pin starts the PR detail fetch"
        );
        let sections = app.open_popover_sections();
        let icons: Vec<IconKind> = sections.iter().map(|section| section.icon).collect();
        assert_eq!(
            icons,
            vec![
                IconKind::Branch,
                IconKind::PrOpen,
                IconKind::Repo,
                IconKind::Behind,
                IconKind::ChangeCount,
            ]
        );
        let titles = |lines: &[PopoverLine]| -> Vec<String> {
            lines.iter().map(PopoverLine::copy_text).collect()
        };
        let lines = section_lines(&app);
        assert_eq!(
            titles(&lines[1]),
            vec![
                IconKind::PrOpen.spec().meaning.to_string(),
                "#7".into(),
                "loading…".into(),
                "https://github.com/octo/demo/pull/7".into(),
                "Open PR".into(),
            ]
        );
        assert_eq!(
            titles(&lines[3]),
            vec![
                IconKind::Behind.spec().meaning.to_string(),
                "2 commits to pull".into(),
                "Pull behind".into(),
                "Fetch remotes".into(),
            ]
        );
        // The landing line is the first action: the branch picker.
        let focus = app.popover.as_ref().unwrap().focus_line;
        assert_eq!(flat_lines(&sections)[focus].copy_text(), "Branch picker");

        app.close_popover();
        app.status.clear();
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.kind == NodeKind::File)
            .expect("file row");
        app.dispatch(Action::PopoverOpenFocused);
        let kinds: Vec<IconKind> = app
            .popover
            .as_ref()
            .expect("file row popover")
            .targets
            .iter()
            .map(|(kind, _)| *kind)
            .collect();
        assert_eq!(kinds, vec![IconKind::FileType, IconKind::StatusModified]);

        // The right pane with no graph paints no icon.
        app.close_popover();
        app.focus = FocusPane::Right;
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover.is_none());
        assert_eq!(app.status, NO_ICONS_ON_ROW);
    }

    #[test]
    fn popover_keys_move_copy_and_run() {
        let mut app = app();
        pin_sync(&mut app, "repo:app");
        assert_eq!(app.dispatch(Action::PopoverCopyLine), copy("Pull behind"));
        app.dispatch(Action::PopoverMove(1));
        assert_eq!(app.dispatch(Action::PopoverCopyLine), copy("Fetch remotes"));
        app.dispatch(Action::PopoverMove(1));
        assert_eq!(
            app.dispatch(Action::PopoverCopyLine),
            copy("Fetch remotes"),
            "stops at the last line"
        );
        app.dispatch(Action::PopoverMove(-5));
        assert_eq!(
            app.dispatch(Action::PopoverCopyLine),
            copy("2 commits to pull"),
            "the meaning never focuses"
        );
        assert_eq!(app.dispatch(Action::PopoverRun), Effect::None, "a field");
        assert!(app.popover_pinned(), "Enter on a field keeps it");
        app.dispatch(Action::PopoverMove(1));
        assert_eq!(app.popover_run_action(), Some(Action::Pull));

        let mut twin = app.clone();
        twin.close_popover();
        let want = twin.dispatch(Action::Pull);
        assert_eq!(app.dispatch(Action::PopoverRun), want);
        assert!(app.popover.is_none(), "Enter closes, then runs");
        assert_eq!(app.status, twin.status);
    }

    #[test]
    fn release_on_an_action_line_runs_it_and_a_drag_copies_instead() {
        let mut app = app();
        pin_sync(&mut app, "repo:app");
        paint(&mut app);
        let painted = app.layout.popover.clone().expect("painted");
        let sections = app.open_popover_sections();
        let lines = flat_lines(&sections);
        let (fetch_y, fetch) = *painted
            .lines
            .iter()
            .find(|(_, index)| lines[*index].copy_text() == "Fetch remotes")
            .expect("fetch line");

        // Drag across the Fetch line: copies, runs nothing, stays open.
        let x = painted.inner.x;
        app.dispatch(Action::Click {
            col: x,
            row: fetch_y,
        });
        app.dispatch(Action::Drag {
            col: x + 6,
            row: fetch_y,
        });
        match app.dispatch(Action::Release) {
            Effect::CopyClipboard { text, .. } => assert!(text.contains("Fetch"), "{text}"),
            other => panic!("drag should copy, got {other:?}"),
        }
        assert!(app.popover_pinned());

        // A plain click on the line focuses it; the release runs it.
        assert_eq!(
            app.dispatch(Action::Click {
                col: x + 2,
                row: fetch_y
            }),
            Effect::None
        );
        assert_eq!(app.popover.as_ref().unwrap().focus_line, fetch);
        let mut twin = app.clone();
        twin.close_popover();
        let want = twin.dispatch(Action::Fetch);
        assert_eq!(app.dispatch(Action::Release), want);
        assert!(app.popover.is_none());
    }

    #[test]
    fn a_dialog_that_opens_closes_the_pinned_popover() {
        let mut app = app();
        pin_sync(&mut app, "repo:app");
        paint(&mut app);
        assert!(app.layout.popover.is_some());
        // A dialog that opens without a key (a picker that loaded, a
        // confirm from an effect) outranks the popover and owns the keys.
        app.help_open = true;
        assert_eq!(app.input_mode(), InputMode::Help);
        paint(&mut app);
        assert!(app.popover.is_none(), "paint closes the pinned popover");
        assert!(app.layout.popover.is_none());
        app.help_open = false;
        assert_ne!(app.input_mode(), InputMode::Popover);
    }

    #[test]
    fn a_press_on_an_action_line_exposes_the_action_the_release_runs() {
        use super::super::super::event_pump::{classify_busy_dispatch, BusyAction};
        let mut app = app();
        pin_sync(&mut app, "repo:app");
        paint(&mut app);
        let painted = app.layout.popover.clone().expect("painted");
        let sections = app.open_popover_sections();
        let lines = flat_lines(&sections);
        let row_of = |title: &str| {
            painted
                .lines
                .iter()
                .find(|(_, index)| lines[*index].copy_text() == title)
                .map(|(y, _)| *y)
                .expect("line")
        };
        let (pull_y, field_y) = (row_of("Pull behind"), row_of("2 commits to pull"));
        let x = painted.inner.x + 2;
        assert_eq!(app.popover_release_action(), None, "no press");
        app.dispatch(Action::Click {
            col: x,
            row: pull_y,
        });
        assert_eq!(app.popover_release_action(), Some(Action::Pull));
        // The busy loop classifies that release as the line's action.
        assert_eq!(
            classify_busy_dispatch(&Action::Release, app.popover_release_action().as_ref()),
            BusyAction::Handle
        );
        app.dispatch(Action::Drag {
            col: x + 4,
            row: pull_y,
        });
        assert_eq!(app.popover_release_action(), None, "a drag runs nothing");
        app.dispatch(Action::Release);
        app.dispatch(Action::Click {
            col: x,
            row: field_y,
        });
        assert_eq!(app.popover_release_action(), None, "a field runs nothing");
    }

    /// A family (`app` primary merged and dirty, linked `feat` merged and
    /// behind), a repo whose status failed, an ignored dirty repo shown,
    /// and an idle repo under No updates.
    fn branch_rows_app() -> AppState {
        let mut primary = repo("app", SyncStatus::NoUpstream, "");
        primary.branch = "feature/landed".into();
        primary.merged_into_default = Some(true);
        primary.default_tip_ref = Some("origin/main".into());
        primary.local_branches = vec!["main".into(), "feature/landed".into()];
        primary.has_staged = true;
        primary.has_untracked = true;
        primary.changes.extend([
            FileChange {
                path: "src/staged.rs".into(),
                staged_status: Some("M".into()),
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            },
            FileChange {
                path: "new.txt".into(),
                staged_status: None,
                unstaged_status: None,
                untracked: true,
                old_path: None,
            },
        ]);
        let mut linked = repo(
            "app/.worktrees/feat",
            SyncStatus::Behind,
            "behind by 3 commits",
        );
        linked.branch = "feature/side".into();
        linked.checkout_kind = CheckoutKind::Linked;
        linked.primary_repo = Some("app".into());
        linked.merged_into_default = Some(true);
        linked.changes.clear();
        linked.has_unstaged = false;
        let mut broken = repo("broken", SyncStatus::NoUpstream, STATUS_FAILED_NOTE);
        broken.changes.clear();
        broken.has_unstaged = false;
        let notes = repo("notes", SyncStatus::NoUpstream, "");
        let mut idle = repo("idle", SyncStatus::UpToDate, "");
        idle.branch = "main".into();
        idle.changes.clear();
        idle.has_unstaged = false;
        let snapshot = build_workspace_snapshot(
            &[primary, linked, broken, notes, idle],
            &["notes".into()],
            true,
            &[],
        );
        let mut app = AppState::new(PathBuf::from("/tmp/ws"), snapshot, true);
        app.show_ignored = true;
        app.rebuild_rows();
        app
    }

    /// `gh` on row `id`: each section's icon and the copy text of each line.
    fn gh_on(app: &mut AppState, id: &str) -> Vec<(IconKind, Vec<String>)> {
        app.close_popover();
        focus(app, id);
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned(), "gh pins on {id}");
        app.open_popover_sections()
            .into_iter()
            .map(|section| {
                let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
                (section.icon, lines)
            })
            .collect()
    }

    fn texts(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| (*line).to_string()).collect()
    }

    fn meaning(kind: IconKind) -> &'static str {
        kind.spec().meaning
    }

    #[test]
    fn every_branch_row_icon_has_its_fields_and_actions() {
        use IconKind as K;
        let mut app = branch_rows_app();
        let diff = "Diff vs default in new tab";
        assert_eq!(
            gh_on(&mut app, "workspace"),
            vec![(
                K::Workspace,
                texts(&[
                    meaning(K::Workspace),
                    "ws",
                    "4 changed",
                    "1 behind, 1 attention",
                    "4 repos",
                    "Fetch remotes",
                    "Refresh",
                    "Show ignored",
                ])
            )]
        );
        assert_eq!(
            gh_on(&mut app, "repo:app"),
            vec![
                (
                    K::Repo,
                    texts(&[
                        meaning(K::Repo),
                        "app",
                        "family of 2 checkouts",
                        "1 staged · 2 unstaged · 1 untracked",
                        "Fetch remotes",
                        "Pull behind",
                        "Push",
                        "Branch picker",
                    ])
                ),
                (
                    K::Behind,
                    texts(&[
                        meaning(K::Behind),
                        "3 commits to pull",
                        "Pull behind",
                        "Fetch remotes"
                    ])
                ),
                (
                    K::WorktreeCount,
                    texts(&[
                        meaning(K::WorktreeCount),
                        "feature/landed",
                        "feature/side",
                        "Fold row",
                        "Fold subtree",
                    ])
                ),
                (
                    K::ChangeCount,
                    texts(&[meaning(K::ChangeCount), "1", "2", "1", "Stash menu", diff])
                ),
            ]
        );
        assert_eq!(
            gh_on(&mut app, "checkout:app"),
            vec![
                (
                    K::Branch,
                    texts(&[
                        meaning(K::Branch),
                        "feature/landed",
                        "feature branch",
                        "2 branches",
                        "Branch picker",
                        "Default branch",
                        "Graph focus branches",
                        diff,
                    ])
                ),
                (
                    K::MergedIntoDefault,
                    texts(&[
                        meaning(K::MergedIntoDefault),
                        "origin/main",
                        diff,
                        "Default branch"
                    ])
                ),
                (
                    K::NoUpstream,
                    texts(&[meaning(K::NoUpstream), "Push", "Fetch remotes"])
                ),
                (
                    K::ChangeCount,
                    texts(&[meaning(K::ChangeCount), "1", "2", "1", "Stash menu", diff])
                ),
            ]
        );
        assert_eq!(
            gh_on(&mut app, "checkout:app/.worktrees/feat"),
            vec![
                (
                    K::LinkedWorktree,
                    texts(&[
                        meaning(K::LinkedWorktree),
                        "app/.worktrees/feat",
                        "app",
                        "feature/side",
                        "Remove worktree",
                        diff,
                        "Copy entity reference",
                    ])
                ),
                (
                    K::MergedIntoDefault,
                    texts(&[
                        meaning(K::MergedIntoDefault),
                        "the default branch",
                        diff,
                        "Default branch",
                        "Remove worktree",
                    ])
                ),
                (
                    K::Behind,
                    texts(&[
                        meaning(K::Behind),
                        "3 commits to pull",
                        "Pull behind",
                        "Fetch remotes"
                    ])
                ),
            ]
        );
        let broken = gh_on(&mut app, "repo:broken");
        assert_eq!(
            broken.last(),
            Some(&(
                K::StatusFailed,
                texts(&[meaning(K::StatusFailed), "Refresh"])
            ))
        );
        let notes = gh_on(&mut app, "repo:notes");
        assert_eq!(
            notes.first(),
            Some(&(K::Ignored, texts(&[meaning(K::Ignored), "Show ignored"])))
        );
        assert_eq!(
            gh_on(&mut app, "group:no-updates"),
            vec![(
                K::Clean,
                texts(&[
                    meaning(K::Clean),
                    "1 repo",
                    "idle",
                    "Fold row",
                    "Fold subtree"
                ])
            )]
        );
    }

    fn change(path: &str, staged: Option<&str>, unstaged: Option<&str>) -> FileChange {
        FileChange {
            path: path.into(),
            staged_status: staged.map(str::to_string),
            unstaged_status: unstaged.map(str::to_string),
            untracked: false,
            old_path: None,
        }
    }

    /// `app` with staged `src/a.rs` (M) and `src/c.rs` (renamed from
    /// `old.rs`), unstaged `src/b.rs` (M, reviewed, four comments: one
    /// resolved), and untracked `new.ts`.
    fn file_rows_app() -> AppState {
        let mut snap = repo("app", SyncStatus::NoUpstream, "");
        let mut renamed = change("src/c.rs", Some("R"), None);
        renamed.old_path = Some("old.rs".into());
        let mut untracked = change("new.ts", None, None);
        untracked.untracked = true;
        snap.changes = vec![
            change("src/a.rs", Some("M"), None),
            change("src/b.rs", None, Some("M")),
            renamed,
            untracked,
        ];
        snap.has_staged = true;
        snap.has_untracked = true;
        let snapshot = build_workspace_snapshot(&[snap], &[], false, &[]);
        let mut app = AppState::new(PathBuf::from("/tmp"), snapshot, true);
        for (line, body, resolved) in [
            (1, "first\nsecond line", false),
            (2, "fixed", true),
            (3, "third", false),
            (4, "fourth", false),
        ] {
            let key = crate::tui::comments::CommentKey::CommitLine {
                repo: "app".into(),
                sha: "abc".into(),
                path: "src/b.rs".into(),
                line,
                end_line: line,
            };
            app.comment_store =
                crate::tui::comments::put_comment_entry(&app.comment_store, key, body, resolved);
        }
        app.reviewed.insert("file:app:src/b.rs".into());
        app
    }

    #[test]
    fn every_file_row_icon_has_its_fields_and_actions() {
        use IconKind as K;
        let mut app = file_rows_app();
        let actions = ["Stage", "Unstage", "Revert", "Copy entity reference"];
        assert_eq!(
            gh_on(&mut app, "section:app:staged"),
            vec![(
                K::Staged,
                texts(&[meaning(K::Staged), "2 files", "Unstage", "Stash menu"])
            )]
        );
        assert_eq!(
            gh_on(&mut app, "section:app:changes"),
            vec![(
                K::Changes,
                texts(&[meaning(K::Changes), "2 files", "Stage", "Stash menu"])
            )]
        );
        let mut folder = texts(&[meaning(K::Folder), "src", "2 files"]);
        folder.extend(texts(&actions));
        assert_eq!(gh_on(&mut app, "dir:app:src"), vec![(K::Folder, folder)]);
        let devicon = |path: &str, kind: &str| {
            (
                K::FileType,
                texts(&[
                    meaning(K::FileType),
                    kind,
                    path,
                    "Open in editor",
                    "Open in diff tool",
                    "Copy entity reference",
                ]),
            )
        };
        assert_eq!(
            gh_on(&mut app, "file:app:src/b.rs"),
            vec![
                devicon("src/b.rs", "rs file"),
                (K::Viewed, texts(&[meaning(K::Viewed), "Mark reviewed"])),
                (
                    K::Comment,
                    texts(&[
                        meaning(K::Comment),
                        "3 open · 1 resolved",
                        "first",
                        "fixed",
                        "third",
                        "Comment",
                        "Copy comments",
                    ])
                ),
                (
                    K::StatusModified,
                    texts(&[
                        meaning(K::StatusModified),
                        "src/b.rs",
                        "no change",
                        "modified",
                        "Stage",
                        "Revert",
                        "Mark reviewed",
                    ])
                ),
            ]
        );
        assert_eq!(
            gh_on(&mut app, "file:app:src/a.rs")[1],
            (
                K::StatusStaged,
                texts(&[
                    meaning(K::StatusStaged),
                    "src/a.rs",
                    "modified",
                    "no change",
                    "Unstage",
                    "Revert",
                    "Mark reviewed",
                ])
            )
        );
        assert_eq!(
            gh_on(&mut app, "file:app:src/c.rs")[1],
            (
                K::StatusRenamed,
                texts(&[
                    meaning(K::StatusRenamed),
                    "old.rs → src/c.rs",
                    "renamed",
                    "no change",
                    "Unstage",
                    "Revert",
                    "Mark reviewed",
                ])
            )
        );
        assert_eq!(
            gh_on(&mut app, "file:app:new.ts"),
            vec![
                devicon("new.ts", "ts file"),
                (
                    K::StatusAdded,
                    texts(&[
                        meaning(K::StatusAdded),
                        "new.ts",
                        "no change",
                        "untracked",
                        "Stage",
                        "Revert",
                        "Mark reviewed",
                    ])
                ),
            ]
        );
    }

    /// Resolving every comment turns the mark into the resolved one and
    /// keeps the pinned section.
    #[test]
    fn a_resolved_comment_keeps_its_section() {
        let mut app = file_rows_app();
        focus(&mut app, "file:app:src/b.rs");
        paint(&mut app);
        let hit = app
            .layout
            .icon_hits
            .iter()
            .find(|hit| hit.kind == IconKind::Comment)
            .cloned()
            .expect("comment hit");
        app.pin_icon(&hit);
        for entry in app.comment_store.values_mut() {
            entry.resolved = true;
        }
        let sections = app.open_popover_sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].icon, IconKind::CommentResolved);
        assert_eq!(sections[0].lines[1].copy_text(), "0 open · 4 resolved");
    }

    fn commit_file(status: &str, path: &str) -> crate::tui::drill::CommitFile {
        crate::tui::drill::CommitFile {
            status: status.into(),
            path: path.into(),
            old_path: None,
            stat: None,
        }
    }

    /// A commit-file list row has the same popovers. Stage, Unstage, and
    /// Revert show their gate reason; a focused-row change closes the
    /// pinned popover.
    #[test]
    fn commit_file_rows_share_the_file_popovers_and_own_their_row() {
        use IconKind as K;
        let mut app = app();
        app.commit_tree_mode = true;
        app.open_commit_files(
            "app".into(),
            crate::tui::drill::CommitFileSource::Commit {
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            },
            vec![
                commit_file("A", "src/lib.rs"),
                commit_file("M", "src/main.rs"),
            ],
        );
        app.focus = FocusPane::Right;
        app.dispatch(Action::Move(1));
        assert_eq!(
            app.focused_commit_file_row().expect("row").id,
            "file:src/lib.rs"
        );
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned(), "gh pins on a commit-file row");
        assert_eq!(app.popover_owner().row, "file:src/lib.rs");
        let sections: Vec<(IconKind, Vec<String>)> = app
            .open_popover_sections()
            .into_iter()
            .map(|section| {
                let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
                (section.icon, lines)
            })
            .collect();
        assert_eq!(sections[0].0, K::FileType);
        assert_eq!(
            sections[1],
            (
                K::StatusAdded,
                texts(&[
                    meaning(K::StatusAdded),
                    "src/lib.rs",
                    "added",
                    "Stage",
                    "Unstage",
                    "Revert",
                    "Mark reviewed",
                ])
            )
        );
        for action in [Action::Stage, Action::Unstage, Action::Revert] {
            let command = command_for(&action).expect("row");
            assert_eq!(
                command.scope,
                crate::tui::command_palette::CommandScope::NoHighlight
            );
            assert!(
                app.palette_disabled_reason(command).is_some(),
                "{action:?} is refused on a commit-file list"
            );
        }
        let terminal = paint(&mut app);
        let painted = app.layout.popover.clone().expect("painted");
        let buf = terminal.backend().buffer();
        let stage = command_for(&Action::Stage).unwrap();
        let reason = app.palette_disabled_reason(stage).unwrap();
        let text: String = (painted.rect.y..painted.rect.bottom())
            .map(|y| {
                (painted.rect.x..painted.rect.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(&format!("Stage  {reason}")),
            "{reason:?} beside Stage:\n{text}"
        );
        assert!(app.popover_pinned(), "paint keeps it");

        app.dispatch(Action::PopoverClose);
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned());
        if let Some(tab_cursor) = match &mut app.drill {
            crate::tui::drill::DrillView::Files { cursor, .. } => Some(cursor),
            _ => None,
        } {
            *tab_cursor += 1;
        }
        assert!(
            !app.popover_pinned(),
            "another focused commit-file row closes the popover"
        );
        paint(&mut app);
        assert!(app.popover.is_none());
    }

    #[test]
    fn a_disabled_action_line_shows_and_keeps_the_gate_reason() {
        let mut app = branch_rows_app();
        // No HEAD sha: the compare gate refuses Diff vs default.
        app.snapshot.repos[0].head.clear();
        app.rebuild_rows();
        focus(&mut app, "checkout:app");
        let diff = command_for(&Action::CompareVsDefault).expect("diff row");
        let reason = app.palette_disabled_reason(diff).expect("gate refuses");
        app.dispatch(Action::PopoverOpenFocused);
        let terminal = paint(&mut app);
        let painted = app.layout.popover.clone().expect("painted");
        let buf = terminal.backend().buffer();
        let text: String = (painted.rect.y..painted.rect.bottom())
            .map(|y| {
                (painted.rect.x..painted.rect.right())
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(
            text.contains(&format!("{}  {reason}", diff.title)),
            "{reason:?} beside the line:\n{text}"
        );
        let sections = app.open_popover_sections();
        let line = flat_lines(&sections)
            .iter()
            .position(|line| line.copy_text() == diff.title)
            .expect("diff line");
        app.popover.as_mut().expect("pinned").focus_line = line;
        assert_eq!(app.dispatch(Action::PopoverRun), Effect::None);
        assert!(app.popover_pinned(), "a refused line keeps the popover");
        assert_eq!(app.status, reason.as_str());
    }

    #[test]
    fn a_peek_on_the_second_segment_of_a_kind_stays_open() {
        let mut app = branch_rows_app();
        paint(&mut app);
        let workspace: Vec<IconHit> = app
            .layout
            .icon_hits
            .iter()
            .filter(|hit| hit.kind == IconKind::Workspace)
            .cloned()
            .collect();
        assert_eq!(workspace.len(), 2, "glyph and summary: {workspace:?}");
        let summary = &workspace[1];
        assert!(summary.width > 1, "the summary text is one hit");
        peek_at(&mut app, summary, Instant::now());
        paint(&mut app);
        let popover = app.popover.as_ref().expect("peek survives paint");
        assert_eq!(
            popover.anchor,
            Some(summary.rect()),
            "hangs from the summary"
        );
        assert_eq!(app.open_popover_sections().len(), 1);

        // Pinned from the summary, it keeps that anchor across paints.
        app.close_popover();
        app.pin_icon(summary);
        paint(&mut app);
        assert_eq!(app.popover.as_ref().unwrap().anchor, Some(summary.rect()));
    }

    #[test]
    fn fold_lines_run_like_z_and_zz() {
        let mut app = branch_rows_app();
        focus(&mut app, "repo:app");
        let mut twin = app.clone();
        paint(&mut app);
        let hit = app
            .layout
            .icon_hits
            .iter()
            .find(|hit| hit.kind == IconKind::WorktreeCount)
            .cloned()
            .expect("N wt hit");
        app.pin_icon(&hit);
        assert_eq!(app.popover_run_action(), Some(Action::FoldToggle));
        app.dispatch(Action::PopoverRun);
        twin.dispatch(Action::FoldToggle);
        assert_eq!(app.rows, twin.rows, "Fold row folds like z");
        assert!(app.z_pending_at.is_none(), "and arms no zz");
        assert!(app.popover.is_none());

        let mut twin = app.clone();
        paint(&mut app);
        let hit = app
            .layout
            .icon_hits
            .iter()
            .find(|hit| hit.kind == IconKind::WorktreeCount)
            .cloned()
            .expect("N wt hit");
        app.pin_icon(&hit);
        app.dispatch(Action::PopoverMove(1));
        assert_eq!(app.popover_run_action(), Some(Action::FoldToggleSubtree));
        app.dispatch(Action::PopoverRun);
        twin.dispatch(Action::FoldToggle);
        twin.dispatch(Action::FoldToggleSubtree);
        assert_eq!(app.rows, twin.rows, "Fold subtree runs like zz");
    }

    #[test]
    fn a_prior_ctrl_click_with_no_pr_never_turns_an_icon_click_into_a_drill() {
        let mut app = app();
        paint(&mut app);
        let hit = sync_hit(&app, "repo:app");
        // A badge on the same cell whose checkout has no PR target: the
        // Ctrl+click is a plain click that arms `last_click`.
        app.layout.icon_hits.insert(
            0,
            IconHit {
                kind: IconKind::PrOpen,
                target: IconTarget::PullRequest(PathBuf::from("gone")),
                ..hit.clone()
            },
        );
        app.dispatch(Action::CtrlClick {
            col: hit.x,
            row: hit.y,
        });
        assert!(app.last_click.is_some(), "the no-PR Ctrl+click armed it");
        assert!(app.popover.is_none(), "Ctrl+click pins nothing");
        app.layout.icon_hits.remove(0);
        assert_eq!(
            app.dispatch(Action::Click {
                col: hit.x,
                row: hit.y
            }),
            Effect::LoadRightPane
        );
        assert_eq!(app.focus, FocusPane::Left, "no drill");
        assert!(app.popover_pinned());
    }

    const G_HEAD: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const G_ROOT: &str = "bbb2222bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const G_STASH: &str = "ccc3333ccccccccccccccccccccccccccccccccc";
    /// Author date of every graph fixture entry: old enough to paint as a
    /// local timestamp.
    const G_WHEN: i64 = 1_700_000_000;

    fn g_worktree(path: &str, head: &str, branch: &str, current: bool) -> Worktree {
        Worktree {
            path: path.into(),
            head_id: Some(head.into()),
            branch: Some(branch.into()),
            ignored: false,
            is_current: current,
        }
    }

    fn g_stash(id: &str, stash_ref: &str) -> Stash {
        Stash {
            id: id.into(),
            stash_ref: stash_ref.into(),
            subject: "WIP on main".into(),
            body: String::new(),
            author_name: "Ada".into(),
            author_date_unix: G_WHEN,
            parent_id: Some(G_HEAD.into()),
        }
    }

    /// Graph of `app`: uncommitted (0), a stash on HEAD (1), HEAD (2) with
    /// the linked `app/.worktrees/feat` mark on its spacer and an open
    /// comment, the root (3), and the worktree row `app/.worktrees/old`
    /// (4). The graph pane is focused on HEAD.
    fn graph_app() -> AppState {
        let mut app = app();
        let commit = |id: &str, subject: &str, parents: &[&str], refs: &[&str]| Commit {
            id: id.into(),
            subject: subject.into(),
            parents: parents.iter().map(|p| (*p).to_string()).collect(),
            refs: refs.iter().map(|r| (*r).into()).collect(),
            author_name: "Ada".into(),
            author_date_unix: G_WHEN,
            ..Commit::default()
        };
        app.graph = Some(GraphModel {
            commits: vec![
                commit(G_HEAD, "add graph", &[G_ROOT], &["main"]),
                commit(G_ROOT, "root", &[], &[]),
            ],
            stashes: vec![g_stash(G_STASH, "stash@{0}")],
            worktrees: vec![
                g_worktree("app", G_HEAD, "main", true),
                g_worktree("app/.worktrees/feat", G_HEAD, "feat", false),
                g_worktree("app/.worktrees/old", "elsewhere", "old", false),
            ],
            head_id: Some(G_HEAD.into()),
            sync: Some(SyncState {
                branch: "main".into(),
                status: GraphSyncStatus::Ahead,
                ahead: 1,
                behind: 0,
            }),
            uncommitted: Some(true),
            ..GraphModel::default()
        });
        app.graph_identity = Some(("app".into(), G_HEAD.into()));
        app.comment_store = put_comment(
            &app.comment_store,
            CommentKey::Commit {
                repo: "app".into(),
                sha: G_HEAD.into(),
            },
            "check this\nsecond line",
        );
        app.focus = FocusPane::Right;
        app.graph_cursor = 2;
        app
    }

    /// `gh` on graph row `index`: each section's icon and line texts.
    fn gh_on_graph(app: &mut AppState, index: usize) -> Vec<(IconKind, Vec<String>)> {
        app.close_popover();
        app.graph_cursor = index;
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned(), "gh pins on graph row {index}");
        app.open_popover_sections()
            .into_iter()
            .map(|section| {
                let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
                (section.icon, lines)
            })
            .collect()
    }

    fn title(action: Action) -> &'static str {
        command_for(&action).expect("palette row").title
    }

    #[test]
    fn every_graph_row_icon_has_its_fields_and_actions() {
        use IconKind as K;
        let mut app = graph_app();
        let date = format_local_timestamp(G_WHEN);
        let date = date.as_str();
        let head = gh_on_graph(&mut app, 2);
        assert_eq!(
            head,
            vec![
                (
                    K::GraphHeadCommit,
                    texts(&[
                        meaning(K::GraphHeadCommit),
                        "aaa1111",
                        "add graph",
                        "Ada",
                        date,
                        "bbb2222",
                        "main",
                        "on main",
                        "^1 v0",
                        title(Action::CompareCommitVsParent),
                        title(Action::GraphCreateBranch),
                    ])
                ),
                (
                    K::Comment,
                    texts(&[
                        meaning(K::Comment),
                        "1 open · 0 resolved",
                        "check this",
                        title(Action::CommentStart),
                        title(Action::ExportComments),
                    ])
                ),
                // A mark on the HEAD spacer: fields only. Its keys would act
                // on the focused commit row, not on the worktree.
                (
                    K::LinkedWorktree,
                    texts(&[meaning(K::LinkedWorktree), "app/.worktrees/feat", "feat"])
                ),
                // The spacer's `[+main]` chip: the chip, then its checkout
                // mark run.
                (
                    K::ChipDefault,
                    texts(&[
                        meaning(K::ChipDefault),
                        "main",
                        "local branch",
                        "yes",
                        title(Action::GraphFocusBranches),
                        title(Action::CompareVsBranch),
                        title(Action::GraphCheckout),
                    ])
                ),
                (
                    K::ChipCheckout,
                    texts(&[
                        meaning(K::ChipCheckout),
                        "on main",
                        title(Action::Pull),
                        title(Action::Push),
                        title(Action::Fetch),
                    ])
                ),
            ]
        );
        assert_eq!(app.popover_owner().row, format!("commit:{G_HEAD}"));
        assert_eq!(
            gh_on_graph(&mut app, 1),
            vec![(
                K::GraphStash,
                texts(&[
                    meaning(K::GraphStash),
                    "stash@{0}",
                    "WIP on main",
                    "Ada",
                    date,
                    "aaa1111",
                    title(Action::GraphStashApply),
                    title(Action::GraphStashPop),
                    title(Action::GraphStashDrop),
                ])
            )]
        );
        assert_eq!(
            gh_on_graph(&mut app, 0),
            vec![(
                K::GraphUncommitted,
                texts(&[
                    meaning(K::GraphUncommitted),
                    "uncommitted changes",
                    "0",
                    "1",
                    "0",
                    title(Action::StashMenu),
                    title(Action::Refresh),
                ])
            )]
        );
        assert_eq!(
            gh_on_graph(&mut app, 3),
            vec![(
                K::GraphCommit,
                texts(&[
                    meaning(K::GraphCommit),
                    "bbb2222",
                    "root",
                    "Ada",
                    date,
                    "none (root commit)",
                    title(Action::CompareCommitVsParent),
                    title(Action::GraphCreateBranch),
                    title(Action::GraphCheckout),
                    title(Action::GraphMerge),
                ])
            )]
        );
        // Its own worktree row keeps the worktree actions.
        let row = gh_on_graph(&mut app, 4);
        assert_eq!(
            row,
            vec![(
                K::LinkedWorktree,
                texts(&[
                    meaning(K::LinkedWorktree),
                    "app/.worktrees/old",
                    "old",
                    title(Action::OpenPullRequest),
                    title(Action::RemoveWorktree),
                    title(Action::CopyEntityReference),
                ])
            )]
        );
        // Disabled lines keep their gate reason: Drop stash off a stash row.
        let drop = command_for(&Action::GraphStashDrop).unwrap();
        assert!(app.palette_disabled_reason(drop).is_some());
        app.graph_cursor = 1;
        app.close_popover();
        assert_eq!(app.palette_disabled_reason(drop), None);
    }

    #[test]
    fn graph_dates_read_the_given_clock() {
        let app = graph_app();
        let date = |now: i64| -> String {
            let sections = popover_sections(
                &app,
                &[(
                    IconKind::GraphHeadCommit,
                    IconTarget::GraphRow(format!("commit:{G_HEAD}")),
                )],
                now,
            );
            let lines = flat_lines(&sections);
            match lines
                .iter()
                .find(|line| matches!(line, PopoverLine::Field { label, .. } if *label == "date"))
            {
                Some(PopoverLine::Field { value, .. }) => value.clone(),
                _ => panic!("no date field"),
            }
        };
        let local = format_local_timestamp(G_WHEN);
        // Two minutes old: the local time, then the relative age.
        let relative = format_relative_date(G_WHEN, G_WHEN + 120);
        assert_ne!(relative, local);
        assert_eq!(date(G_WHEN + 120), format!("{local} ({relative})"));
        // A day old: the graph paints the local time only.
        assert_eq!(date(G_WHEN + 86_400), local);
    }

    #[test]
    fn a_graph_popover_belongs_to_its_row_not_the_cursor_index() {
        let mut app = graph_app();
        app.dispatch(Action::PopoverOpenFocused);
        assert!(app.popover_pinned());
        // A reload adds a stash: HEAD moves to index 3 and the cursor
        // follows it. The popover stays on HEAD.
        let model = app.graph.as_mut().unwrap();
        model.stashes.insert(0, g_stash("ddd4444", "stash@{0}"));
        model.stashes[1].stash_ref = "stash@{1}".into();
        app.graph_cursor = 3;
        assert!(app.popover_pinned(), "same row, new index");
        paint(&mut app);
        assert!(app.popover.is_some());
        // The cursor index stays put but names another row now: closed.
        app.graph_cursor = 2;
        assert!(!app.popover_pinned(), "index 2 is a stash now");
        paint(&mut app);
        assert!(app.popover.is_none());
    }

    #[test]
    fn graph_icons_record_hits_and_a_click_pins_the_node_alone() {
        use IconKind as K;
        let mut app = graph_app();
        let terminal = paint(&mut app);
        let graph: Vec<(IconKind, IconTarget)> = app
            .layout
            .icon_hits
            .iter()
            .filter(|hit| hit.x >= app.layout.right_x)
            .filter(|hit| {
                // Chips, the sync header, and the footer hint: see
                // `graph_chips_header_and_footer_record_hits_and_pin`.
                matches!(
                    hit.target,
                    IconTarget::GraphRow(_) | IconTarget::GraphWorktree(_)
                )
            })
            .map(|hit| (hit.kind, hit.target.clone()))
            .collect();
        let row = |id: &str| IconTarget::GraphRow(id.into());
        assert_eq!(
            graph,
            vec![
                (K::GraphUncommitted, row("uncommitted")),
                (K::GraphStash, row(&format!("stash:{G_STASH}"))),
                (K::GraphHeadCommit, row(&format!("commit:{G_HEAD}"))),
                (K::Comment, row(&format!("commit:{G_HEAD}"))),
                (
                    K::LinkedWorktree,
                    IconTarget::GraphWorktree("app/.worktrees/feat".into())
                ),
                (K::GraphCommit, row(&format!("commit:{G_ROOT}"))),
                (
                    K::LinkedWorktree,
                    IconTarget::GraphWorktree("app/.worktrees/old".into())
                ),
            ]
        );
        let buf = terminal.backend().buffer();
        let hit_of = |app: &AppState, kind: IconKind| {
            app.layout
                .icon_hits
                .iter()
                .find(|hit| hit.kind == kind)
                .cloned()
                .expect("hit")
        };
        let comment = hit_of(&app, K::Comment);
        let palette = app.theme.palette();
        assert_eq!(buf[(comment.x, comment.y)].symbol(), "\"");
        assert_eq!(buf[(comment.x, comment.y)].fg, palette.heading);
        let dirty = hit_of(&app, K::GraphUncommitted);
        assert_eq!(buf[(dirty.x, dirty.y)].symbol(), "o");
        assert_eq!(buf[(dirty.x, dirty.y)].fg, palette.modified);
        let mark = hit_of(&app, K::LinkedWorktree);
        assert_eq!(buf[(mark.x, mark.y)].symbol(), "L");
        assert_eq!(buf[(mark.x, mark.y)].fg, palette.heading);

        // A click on the stash node selects its row and pins its popover.
        let stash = hit_of(&app, K::GraphStash);
        assert_eq!(buf[(stash.x, stash.y)].symbol(), "s");
        app.dispatch(Action::Click {
            col: stash.x,
            row: stash.y,
        });
        assert_eq!(app.graph_cursor, 1);
        assert!(app.last_click.is_none(), "an icon click never drills");
        assert!(app.popover_pinned());
        assert_eq!(
            app.popover.as_ref().unwrap().targets,
            vec![(K::GraphStash, row(&format!("stash:{G_STASH}")))]
        );
        // Enter on the landing line (Apply stash) runs the existing action.
        assert_eq!(
            app.popover_run_action(),
            Some(Action::GraphStashApply),
            "landing line"
        );
        app.dispatch(Action::PopoverClose);
        // The worktree mark on the HEAD spacer selects HEAD.
        paint(&mut app);
        let mark = hit_of(&app, K::LinkedWorktree);
        app.dispatch(Action::Click {
            col: mark.x,
            row: mark.y,
        });
        assert_eq!(app.graph_cursor, 2);
        assert_eq!(
            app.open_popover_sections()[0].target,
            IconTarget::GraphWorktree("app/.worktrees/feat".into())
        );
    }

    /// [`graph_app`] with HEAD on `main` synced with `origin/main`, a
    /// `feat` branch, and a `v0` tag on the root.
    fn chip_graph_app() -> AppState {
        let mut app = graph_app();
        let model = app.graph.as_mut().unwrap();
        model.commits[0].refs = vec![
            "main".into(),
            GraphRef::remote("origin/main"),
            "feat".into(),
        ];
        model.commits[1].refs = vec![GraphRef::tag("v0")];
        app
    }

    fn chip_hit(app: &AppState, kind: IconKind, row: &str) -> IconHit {
        app.layout
            .icon_hits
            .iter()
            .find(|hit| {
                hit.kind == kind
                    && matches!(&hit.target, IconTarget::GraphChip { row: id, .. } if id == row)
            })
            .cloned()
            .unwrap_or_else(|| panic!("no {kind:?} chip on {row}: {:?}", app.layout.icon_hits))
    }

    /// Text rows of the painted popover box, trailing spaces trimmed.
    fn popover_rows(app: &AppState) -> Vec<String> {
        let rect = app.layout.popover.as_ref().expect("painted popover").rect;
        (rect.y..rect.bottom())
            .map(|y| {
                (rect.x..rect.right())
                    .map(|x| app.painted_frame[(x, y)].symbol())
                    .collect::<String>()
                    .trim_end()
                    .to_string()
            })
            .collect()
    }

    fn section_texts(app: &AppState) -> Vec<(IconKind, Vec<String>)> {
        app.open_popover_sections()
            .into_iter()
            .map(|section| {
                let lines = section.lines.iter().map(PopoverLine::copy_text).collect();
                (section.icon, lines)
            })
            .collect()
    }

    #[test]
    fn graph_chips_header_and_footer_record_hits_and_pin() {
        use IconKind as K;
        let mut app = chip_graph_app();
        let terminal = paint(&mut app);
        let buf = terminal.backend().buffer();
        let head = format!("commit:{G_HEAD}");
        let root = format!("commit:{G_ROOT}");
        let palette = app.theme.palette();

        // The header is one hit, coloured with the theme.
        let header = app
            .layout
            .icon_hits
            .iter()
            .find(|hit| hit.target == IconTarget::GraphSync)
            .cloned()
            .expect("sync header hit");
        assert_eq!(header.kind, K::Ahead);
        let text: String = (header.x..header.x + header.width)
            .map(|x| buf[(x, header.y)].symbol())
            .collect();
        assert_eq!(text, "main ^1");
        assert_eq!(buf[(header.x, header.y)].fg, palette.branch_default);
        assert_eq!(buf[(header.x + 5, header.y)].fg, palette.added);

        // The marks run on the HEAD spacer is one hit; a click selects HEAD
        // and pins two sections.
        app.graph_cursor = 3;
        paint(&mut app);
        let marks = chip_hit(&app, K::ChipCheckout, &head);
        assert_eq!(buf[(marks.x, marks.y)].symbol(), "+");
        assert_eq!(marks.width, 2, "checkout + sync: one hit");
        app.dispatch(Action::Click {
            col: marks.x,
            row: marks.y,
        });
        assert_eq!(app.graph_cursor, 2, "the spacer chip selects its commit");
        assert!(app.last_click.is_none(), "an icon click never drills");
        assert_eq!(
            section_texts(&app),
            vec![
                (
                    K::ChipCheckout,
                    texts(&[
                        meaning(K::ChipCheckout),
                        "on main",
                        title(Action::Pull),
                        title(Action::Push),
                        title(Action::Fetch),
                    ])
                ),
                (
                    K::ChipSynced,
                    texts(&[meaning(K::ChipSynced), "main", "in sync with origin/main"])
                ),
            ]
        );
        // Off the tree the gates refuse Pull: the painted line keeps the
        // reason beside the title.
        let pull = command_for(&Action::Pull).unwrap();
        let reason = app
            .palette_disabled_reason(pull)
            .expect("gated off the tree");
        assert!(reason.contains("to pull"), "{reason}");
        paint(&mut app);
        let line = popover_rows(&app)
            .into_iter()
            .find(|line| line.contains(pull.title))
            .expect("painted Pull line");
        assert!(line.contains(&reason), "{line:?}");
        app.dispatch(Action::PopoverClose);

        // A tag chip on the root spacer: its own commit, so Checkout
        // commit refs acts on it.
        paint(&mut app);
        let tag = chip_hit(&app, K::ChipTag, &root);
        app.dispatch(Action::Click {
            col: tag.x,
            row: tag.y,
        });
        assert_eq!(app.graph_cursor, 3);
        assert_eq!(
            section_texts(&app),
            vec![(
                K::ChipTag,
                texts(&[
                    meaning(K::ChipTag),
                    "v0",
                    "tag",
                    title(Action::GraphCheckout)
                ])
            )]
        );
        assert_eq!(app.popover_run_action(), Some(Action::GraphCheckout));
        app.dispatch(Action::PopoverClose);

        // A click on the header pins the sync section.
        paint(&mut app);
        app.dispatch(Action::Click {
            col: header.x + 1,
            row: header.y,
        });
        assert_eq!(
            section_texts(&app),
            vec![(
                K::Ahead,
                texts(&[
                    meaning(K::Ahead),
                    "main",
                    "1 commit not pushed",
                    "0 commits to pull",
                    "ahead",
                    title(Action::Pull),
                    title(Action::Push),
                    title(Action::Fetch),
                ])
            )]
        );
        app.dispatch(Action::PopoverClose);

        // The uncommitted row's footer lists HEAD's chips: they name HEAD's
        // refs, and no line acts on the focused commit.
        app.graph_cursor = 0;
        paint(&mut app);
        let footer = chip_hit(&app, K::ChipLocal, "uncommitted");
        app.pin_icon(&footer);
        assert_eq!(
            section_texts(&app),
            vec![(
                K::ChipLocal,
                texts(&[
                    meaning(K::ChipLocal),
                    "feat",
                    "local branch",
                    title(Action::GraphFocusBranches),
                    title(Action::CompareVsBranch),
                ])
            )]
        );
    }

    #[test]
    fn overflow_detached_and_more_lines_popovers_read_live_state() {
        use IconKind as K;
        let mut app = chip_graph_app();
        let head = format!("commit:{G_HEAD}");
        let chip = |name: &str| PartTarget::Chip(vec![GraphRef::local(name)]);
        let sections = |app: &AppState, kind: IconKind, chip: PartTarget, row: &str| {
            popover_sections(
                app,
                &[(
                    kind,
                    IconTarget::GraphChip {
                        row: row.to_string(),
                        chip,
                    },
                )],
                0,
            )
            .into_iter()
            .map(|section| {
                let lines: Vec<String> = section.lines.iter().map(PopoverLine::copy_text).collect();
                (section.icon, lines)
            })
            .collect::<Vec<_>>()
        };
        // `[+N]` lists every hidden chip with its kind.
        let hidden = PartTarget::Overflow(vec![
            PartTarget::Chip(vec![
                GraphRef::local("main"),
                GraphRef::remote("origin/main"),
            ]),
            chip("feat"),
        ]);
        assert_eq!(
            sections(&app, K::ChipOverflow, hidden.clone(), &head),
            vec![(
                K::ChipOverflow,
                texts(&[
                    meaning(K::ChipOverflow),
                    "main + origin/main",
                    "feat",
                    title(Action::GraphCheckout),
                ])
            )]
        );
        // A chip whose ref left the commit drops its section.
        assert!(sections(&app, K::ChipLocal, chip("gone"), &head).is_empty());
        // The hidden list is the painted one: a ref gone since drops out,
        // and `[HEAD]` lists only while HEAD is detached here. With none
        // left, the section goes.
        let stale = |extra: Vec<PartTarget>| {
            let mut hidden = vec![chip("gone"), PartTarget::DetachedHead];
            hidden.extend(extra);
            PartTarget::Overflow(hidden)
        };
        assert_eq!(
            sections(&app, K::ChipOverflow, stale(vec![chip("feat")]), &head),
            vec![(
                K::ChipOverflow,
                texts(&[
                    meaning(K::ChipOverflow),
                    "feat",
                    title(Action::GraphCheckout),
                ])
            )]
        );
        assert!(sections(&app, K::ChipOverflow, stale(Vec::new()), &head).is_empty());
        // Detached HEAD.
        app.graph.as_mut().unwrap().sync = None;
        assert_eq!(
            sections(&app, K::ChipOverflow, stale(Vec::new()), &head),
            vec![(
                K::ChipOverflow,
                texts(&[
                    meaning(K::ChipOverflow),
                    "HEAD",
                    title(Action::GraphCheckout),
                ])
            )]
        );
        assert_eq!(
            sections(&app, K::ChipDetachedHead, PartTarget::DetachedHead, &head),
            vec![(
                K::ChipDetachedHead,
                texts(&[
                    meaning(K::ChipDetachedHead),
                    "detached at aaa1111",
                    title(Action::Branch),
                    title(Action::GraphCreateBranch),
                ])
            )]
        );

        // A long message on HEAD: the expanded footer hints the lines below.
        let mut app = chip_graph_app();
        app.graph.as_mut().unwrap().commits[0].body = (0..30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.commit_msg_expand = true;
        paint(&mut app);
        let hint = app
            .layout
            .icon_hits
            .iter()
            .find(|hit| hit.kind == K::GraphMoreBelow)
            .cloned()
            .unwrap_or_else(|| panic!("hint hit: {:?}", app.layout.icon_hits));
        assert_eq!(hint.target, IconTarget::GraphMoreLines(head.clone()));
        // K as the footer paints it: `v<K>` under the hint hit.
        let painted: String = (hint.x..hint.x + hint.width)
            .map(|x| app.painted_frame[(x, hint.y)].symbol().to_string())
            .collect();
        let below: usize = painted
            .strip_prefix('v')
            .and_then(|count| count.parse().ok())
            .unwrap_or_else(|| panic!("footer hint {painted:?}"));
        assert!(below > 0);
        assert_eq!(app.graph_footer_lines_below(), below);
        app.pin_icon(&hint);
        assert_eq!(
            section_texts(&app),
            vec![(
                K::GraphMoreBelow,
                texts(&[
                    meaning(K::GraphMoreBelow),
                    &format!("{below} more lines"),
                    title(Action::ToggleCommitMsgExpand),
                    title(Action::ResizeCommitMsg(1)),
                ])
            )]
        );
        // Collapsing the message (M) clears the hint and its section.
        app.dispatch(Action::PopoverRun);
        assert!(!app.commit_msg_expand);
        assert!(app.popover.is_none());
        assert_eq!(app.graph_footer_lines_below(), 0);
    }

    /// Enter or a click on an action line that a diff highlight disables
    /// keeps the pinned popover open and puts the gate reason on the
    /// status line, like the palette. The highlight stays.
    #[test]
    fn a_line_the_highlight_disables_keeps_the_popover_on_enter_and_click() {
        const EXIT_HIGHLIGHT: &str = "exit highlight first (Esc)";
        let mut app = app();
        pin_sync(&mut app, "repo:app");
        app.diff_visual_anchor = Some(0);
        let fetch = command_for(&Action::Fetch).unwrap();
        assert_eq!(
            app.palette_disabled_reason(fetch).as_deref(),
            Some(EXIT_HIGHLIGHT)
        );
        let sections = app.open_popover_sections();
        let line = flat_lines(&sections)
            .iter()
            .position(|line| line.copy_text() == fetch.title)
            .expect("fetch line");
        app.popover.as_mut().expect("pinned").focus_line = line;
        assert_eq!(app.popover_run_action(), Some(Action::Fetch));
        assert_eq!(app.dispatch(Action::PopoverRun), Effect::None);
        assert!(app.popover_pinned(), "Enter keeps the popover");
        assert_eq!(app.status, EXIT_HIGHLIGHT);
        assert_eq!(app.diff_visual_anchor, Some(0), "the highlight stays");

        app.status = StatusMessage::default();
        paint(&mut app);
        let painted = app.layout.popover.clone().expect("painted");
        let (y, _) = *painted
            .lines
            .iter()
            .find(|(_, index)| *index == line)
            .expect("painted fetch line");
        app.dispatch(Action::Click {
            col: painted.inner.x + 2,
            row: y,
        });
        assert_eq!(app.dispatch(Action::Release), Effect::None);
        assert!(app.popover_pinned(), "a click keeps the popover");
        assert_eq!(app.status, EXIT_HIGHLIGHT);
    }

    /// The `:` palette row Icon popover (`gh`) pins the same popover.
    #[test]
    fn the_palette_icon_popover_row_pins_like_gh() {
        use crate::tui::action::QuickOpenEntry;
        let mut app = app();
        focus(&mut app, "repo:app");
        app.dispatch(Action::PopoverOpenFocused);
        let want = section_lines(&app);
        app.close_popover();

        let row = command_for(&Action::PopoverOpenFocused).expect("palette row");
        assert_eq!((row.title, row.keys), ("Icon popover", "gh"));
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        for c in "icon popover".chars() {
            app.dispatch(Action::QuickOpenChar(c));
        }
        let palette = app.command_palette().expect("palette open");
        assert_eq!(
            palette.selected().map(|row| row.title),
            Some("Icon popover")
        );
        app.dispatch(Action::QuickOpenSubmit);
        assert!(app.command_palette().is_none(), "the palette closes");
        assert!(app.popover_pinned(), "the row pins the popover");
        assert_eq!(section_lines(&app), want);
    }

    /// A detached checkout says `detached HEAD` once. The merge mark
    /// names a local main, master, or develop when no tip ref or override
    /// is known.
    #[test]
    fn detached_branch_says_it_once_and_the_merge_mark_finds_the_default_name() {
        use IconKind as K;
        let mut app = branch_rows_app();
        let linked = app
            .snapshot
            .repos
            .iter_mut()
            .find(|repo| repo.repo == "app/.worktrees/feat")
            .expect("linked checkout");
        linked.local_branches = vec!["develop".into(), "master".into()];
        app.rebuild_rows();
        let sections = gh_on(&mut app, "checkout:app/.worktrees/feat");
        let merge = sections
            .iter()
            .find(|(kind, _)| *kind == K::MergedIntoDefault)
            .expect("merge section");
        assert_eq!(merge.1[1], "master", "main, then master, then develop");

        app.snapshot.repos[0].branch = "HEAD".into();
        app.rebuild_rows();
        let sections = gh_on(&mut app, "checkout:app");
        let branch = sections
            .iter()
            .find(|(kind, _)| *kind == K::Branch)
            .expect("branch section");
        let detached = branch
            .1
            .iter()
            .filter(|line| line.as_str() == "detached HEAD")
            .count();
        assert_eq!(detached, 1, "{branch:?}");
    }

    /// Repo and checkout rows open the comment popover for their object
    /// comments: open and resolved bodies in their colours.
    #[test]
    fn repo_and_checkout_rows_list_their_object_comments() {
        use IconKind as K;
        let mut app = branch_rows_app();
        app.comment_store = crate::tui::comments::put_comment_entry(
            &app.comment_store,
            CommentKey::Worktree {
                path: "app/.worktrees/feat".into(),
            },
            "clean up after merge",
            true,
        );
        app.comment_store = crate::tui::comments::put_comment_entry(
            &app.comment_store,
            CommentKey::Branch {
                repo: "notes".into(),
                branch: "feature".into(),
            },
            "release from here\nsecond line",
            false,
        );
        app.rebuild_rows();
        let comment = |kind: K, app: &mut AppState, id: &str| {
            gh_on(app, id)
                .into_iter()
                .find(|(icon, _)| *icon == kind)
                .unwrap_or_else(|| panic!("no {kind:?} on {id}"))
                .1
        };
        // The role a body field paints in: resolved bodies are muted.
        let body_role = |app: &AppState, label: &str| {
            app.open_popover_sections()
                .into_iter()
                .flat_map(|section| section.lines)
                .find_map(|line| match line {
                    PopoverLine::Field {
                        label: found, role, ..
                    } if found == label => Some(role),
                    _ => None,
                })
        };
        assert_eq!(
            comment(K::CommentResolved, &mut app, "checkout:app/.worktrees/feat"),
            texts(&[
                meaning(K::CommentResolved),
                "0 open · 1 resolved",
                "clean up after merge",
                "Comment",
                "Copy comments",
            ])
        );
        assert_eq!(body_role(&app, "resolved"), Some(SegRole::Muted));
        assert_eq!(
            comment(K::Comment, &mut app, "repo:notes"),
            texts(&[
                meaning(K::Comment),
                "1 open · 0 resolved",
                "release from here",
                "Comment",
                "Copy comments",
            ])
        );
        assert_eq!(body_role(&app, "open"), Some(SegRole::File));
    }

    /// A commit-file folder row counts the files under it; a commit-file
    /// row with a line comment on that commit opens the comment popover.
    #[test]
    fn commit_file_folders_count_their_files_and_rows_list_their_comments() {
        use IconKind as K;
        const SHA: &str = "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
        let mut app = app();
        app.comment_store = crate::tui::comments::put_comment_entry(
            &app.comment_store,
            CommentKey::CommitLine {
                repo: "app".into(),
                sha: SHA.into(),
                path: "src/main.rs".into(),
                line: 3,
                end_line: 3,
            },
            "why unwrap here?",
            false,
        );
        app.commit_tree_mode = true;
        app.open_commit_files(
            "app".into(),
            crate::tui::drill::CommitFileSource::Commit {
                commit_id: SHA.into(),
            },
            vec![
                commit_file("A", "src/lib.rs"),
                commit_file("M", "src/main.rs"),
                commit_file("M", "README.md"),
            ],
        );
        app.focus = FocusPane::Right;
        let gh_on_file_row = |app: &mut AppState, id: &str| {
            app.close_popover();
            let index = app
                .commit_file_rows()
                .iter()
                .position(|row| row.id == id)
                .unwrap_or_else(|| panic!("no commit-file row {id}"));
            if let crate::tui::drill::DrillView::Files { cursor, .. } = &mut app.drill {
                *cursor = index;
            }
            app.dispatch(Action::PopoverOpenFocused);
            assert!(app.popover_pinned(), "gh pins on {id}");
            section_texts(app)
        };
        assert_eq!(
            gh_on_file_row(&mut app, "dir:src"),
            vec![(
                K::Folder,
                texts(&[
                    meaning(K::Folder),
                    "src",
                    "2 files",
                    "Stage",
                    "Unstage",
                    "Revert",
                    "Copy entity reference",
                ])
            )]
        );
        let sections = gh_on_file_row(&mut app, "file:src/main.rs");
        assert_eq!(
            sections.iter().find(|(kind, _)| *kind == K::Comment),
            Some(&(
                K::Comment,
                texts(&[
                    meaning(K::Comment),
                    "1 open · 0 resolved",
                    "why unwrap here?",
                    "Comment",
                    "Copy comments",
                ])
            )),
            "{sections:?}"
        );
    }
}

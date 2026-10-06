//! Icon popover state: peek timers, pin, `gh`, popover keys, and clicks.
//!
//! The model and the content builders live in [`super::super::popover`].

use std::path::Path;
use std::time::{Duration, Instant};

use ratatui::layout::{Position, Rect};

use super::super::action::{Action, Effect};
use super::super::chrome::open_dialog;
use super::super::comments::{tree_row_comments_resolved, tree_row_has_comment};
use super::super::gates::ListFocusTarget;
use super::super::icons::IconKind;
use super::super::keys::InputMode;
use super::super::popover::{
    flat_lines, focused_line, landing_line, popover_sections, step_line, tree_icon_target,
    IconTarget, PopoverLine, PopoverOrigin, PopoverOwner, PopoverSection, PopoverState,
    PEEK_DWELL_MS, PEEK_GRACE_MS,
};
use super::super::pull_request::PrState;
use super::super::selection::TextSelection;
use super::super::status::StatusMessage;
use super::super::tree::{
    painted_row_segments, pr_badge_kind, pr_badge_repo, segment_icons, NodeKind, NodeSegments,
    VisibleRow,
};
use super::{AppState, IconHit};

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
            ListFocusTarget::Graph => self.graph_cursor.to_string(),
            ListFocusTarget::CommitFiles | ListFocusTarget::None => String::new(),
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
            .map(|popover| popover_sections(self, &popover.targets))
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
    fn tree_row_paint_segments(&self, row: &VisibleRow) -> NodeSegments {
        let (viewed, commented, resolved, pr) = self.tree_row_marks(row);
        painted_row_segments(row, self.ascii, viewed, commented, resolved, pr)
    }

    /// Icons of the focused row in paint order, clipped ones included.
    fn focused_row_icons(&self) -> Vec<(IconKind, IconTarget)> {
        if self.is_file_tab() {
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
            ListFocusTarget::Graph => self
                .graph_pr_badges()
                .into_iter()
                .filter(|(index, _, _)| *index == self.graph_cursor)
                .map(|(_, repo, pr)| (pr_badge_kind(pr), IconTarget::PullRequest(repo)))
                .collect(),
            ListFocusTarget::CommitFiles | ListFocusTarget::None => Vec::new(),
        }
    }

    /// True when `target` is on the focused row, so its action lines act
    /// on it and show their gate reasons. A pinned popover always is.
    pub(crate) fn popover_target_focused(&self, target: &IconTarget) -> bool {
        match target {
            IconTarget::TreeRow(id) => {
                self.list_focus_target() == ListFocusTarget::Tree
                    && self.focused_row().is_some_and(|row| &row.id == id)
            }
            IconTarget::PullRequest(repo) => self
                .pr_target_for_focus()
                .is_some_and(|(focused, _)| &focused == repo),
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
        self.pin(targets, None);
        Effect::None
    }

    fn pin(&mut self, targets: Vec<(IconKind, IconTarget)>, anchor: Option<Rect>) {
        self.peek = PeekTimers::default();
        self.popover_press = None;
        let focus_line = landing_line(&flat_lines(&popover_sections(self, &targets)));
        self.popover = Some(PopoverState {
            origin: PopoverOrigin::Pinned,
            targets,
            anchor,
            focus_line,
            owner: self.popover_owner(),
        });
    }

    /// Pin the popover of the clicked icon `hit`. Its row is already
    /// selected.
    pub(super) fn pin_icon(&mut self, hit: &IconHit) {
        self.pin(vec![(hit.kind, hit.target.clone())], Some(hit.rect()));
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
        let sections = popover_sections(self, &popover.targets);
        let lines = flat_lines(&sections);
        let index = focused_line(&lines, popover.focus_line)?;
        Some(lines[index].clone())
    }

    /// The action Enter would run in the pinned popover, if any. Busy
    /// gating classifies Enter as this action.
    pub(crate) fn popover_run_action(&self) -> Option<Action> {
        match self.focused_popover_line()? {
            PopoverLine::Action(command) => Some(command.action.clone()),
            PopoverLine::Text(_) | PopoverLine::Field { .. } => None,
        }
    }

    /// The action a left release would run: the action line a pinned
    /// popover press landed on, while that press has not become a drag.
    /// Busy gating classifies the release as this action.
    pub(crate) fn popover_release_action(&self) -> Option<Action> {
        let line = self.popover_press?;
        if self.text_selection.is_some_and(|sel| sel.is_active()) || !self.popover_pinned() {
            return None;
        }
        let sections = self.open_popover_sections();
        match flat_lines(&sections).get(line)? {
            PopoverLine::Action(command) => Some(command.action.clone()),
            PopoverLine::Text(_) | PopoverLine::Field { .. } => None,
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
    /// Returns true when the popover changed, so the live loop redraws.
    pub fn expire_peek(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.peek.dwell.as_ref().is_some_and(|(_, at)| *at <= now) {
            let hit = self.peek.dwell.take().map(|(hit, _)| hit);
            if let Some(hit) = hit.filter(|_| self.peek_allowed()) {
                self.popover = Some(PopoverState {
                    origin: PopoverOrigin::Peek,
                    targets: vec![(hit.kind, hit.target.clone())],
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
                self.popover_press = Some(line);
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
        self.pin(targets, Some(anchor));
        Some(selected)
    }

    /// Left release: copy a drag selection, or run the popover action line
    /// the press landed on.
    pub(super) fn release_mouse(&mut self) -> Effect {
        let press = self.popover_press.take();
        let dragged = self.text_selection.is_some_and(|sel| sel.is_active());
        let copied = self.finish_text_selection();
        match press {
            Some(line) if !dragged => self.run_popover_line(Some(line)),
            _ => copied,
        }
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
        assert_eq!(app.dispatch(Action::PopoverOpenFocused), Effect::None);
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
}

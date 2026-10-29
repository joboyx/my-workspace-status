//! Read-only file view: the body of an Explorer inner file tab whose file
//! is clean ([`super::super::tabs::ExplorerPreview::File`]).
//!
//! Cursor moves, pan, click, and the in-file `/` search over
//! [`super::super::tabs::TabStrip::active_file_view`]. Opening, loading,
//! and live reload live with the Explorer tab (`explorer_tab.rs`).

use std::rc::Rc;

use ratatui::layout::Rect;

use super::super::search::{apply_pan, match_diff_line_indices, step_from_anchor};
use super::super::selection::TextSelection;
use super::super::split::SplitDrag;
use super::super::tabs::file_view_scroll;
use super::super::tree::cursor_for_viewport_top;
use super::{AppState, FileSearchMemo};

impl AppState {
    /// Put `cursor` on the active file body, clamped to its lines.
    pub(super) fn set_file_cursor(&mut self, cursor: usize) {
        if let Some(tab) = self.tabs.active_file_view_mut() {
            let last = tab.lines().len().saturating_sub(1);
            tab.cursor = cursor.min(last);
        }
    }

    /// Move the active file body's cursor by `delta` lines, clamped.
    pub(super) fn step_file_cursor(&mut self, delta: i64) {
        let Some(cursor) = self.tabs.active_file_view().map(|tab| tab.cursor) else {
            return;
        };
        let next = (cursor as i64).saturating_add(delta).max(0);
        self.set_file_cursor(usize::try_from(next).unwrap_or(usize::MAX));
    }

    /// Ctrl-d / Ctrl-u on the file body: the next paint puts the cursor
    /// near the vertical middle ([`FileTab::recenter`]), as the tree,
    /// graph, and diff do. `j` / `k` keep the sticky top.
    ///
    /// [`FileTab::recenter`]: super::super::tabs::FileTab::recenter
    pub(super) fn recenter_file_view(&mut self) {
        if let Some(tab) = self.tabs.active_file_view_mut() {
            tab.recenter = true;
        }
    }

    /// Pan the active file body's code columns, clamped to
    /// [`FileTab::pan_max`] (the horizontal scrollbar's clamp too). A
    /// no-op while wrap is on.
    ///
    /// [`FileTab::pan_max`]: super::super::tabs::FileTab::pan_max
    pub(super) fn pan_file_tab(&mut self, delta: i32) {
        if self.diff_wrap {
            return;
        }
        let (view, ascii) = (self.layout.file_view_width, self.ascii);
        let Some(tab) = self.tabs.active_file_view_mut() else {
            return;
        };
        let max = tab.pan_max(view, ascii);
        tab.col_offset = apply_pan(tab.col_offset, delta, max);
    }

    /// Scroll the active file body so its viewport starts at line `top`
    /// (a scrollbar drag or track click).
    ///
    /// The view scrolls at the edge of its cursor (the sticky top of
    /// [`file_view_scroll`]), so the cursor moves to the line nearest it
    /// whose viewport starts at `top` ([`cursor_for_viewport_top`]): a drag
    /// down leaves it on the bottom row and a drag up on the top row, as
    /// the wheel leaves it. The next paint keeps that top.
    pub(super) fn scroll_file_view_to(&mut self, top: usize) {
        let (wrap, ascii) = (self.diff_wrap, self.ascii);
        let width = self.layout.file_view_width;
        let height = usize::from(self.layout.file_view_height).max(1);
        let Some(tab) = self.tabs.active_file_view_mut() else {
            return;
        };
        let n = tab.lines().len();
        if n == 0 {
            return;
        }
        let cursor = {
            let tab = &*tab;
            let code_w = tab.code_width(width, ascii);
            let rows_of = |line: usize| tab.line_rows(line, code_w, wrap, height + 1);
            cursor_for_viewport_top(n, tab.cursor, top, |cursor| {
                file_view_scroll(tab.scroll, cursor, height, rows_of)
            })
        };
        tab.cursor = cursor;
        tab.recenter = false;
    }

    /// Press in the file body: focus the clicked line and arm a text
    /// selection. A press on a flat pad cell beside the body picks its line
    /// too; the selection keeps to the body.
    pub(super) fn click_file_tab(&mut self, col: u16, row: u16) {
        self.drag = SplitDrag::None;
        self.pad_press = None;
        let layout = &self.layout;
        let body = Rect::new(
            layout.file_view_x,
            layout.file_view_y,
            layout.file_view_width,
            layout.file_view_height,
        );
        let rows = layout.file_view_rows;
        let line = row
            .checked_sub(layout.file_view_y)
            .and_then(|offset| layout.file_view_row_lines.get(usize::from(offset)))
            .copied()
            .filter(|_| col >= rows.x && col < rows.right());
        self.text_selection = TextSelection::arm(body, col, row);
        if let Some(line) = line {
            self.set_file_cursor(line);
        }
    }

    /// Lines of the active file body that contain `query`, ignoring case,
    /// ascending.
    ///
    /// Memoized per Explorer tab, load, and query; the paint and the status pill
    /// share one list instead of copying it each frame.
    pub(crate) fn file_search_hits(&self, query: &str) -> Rc<[usize]> {
        let query = query.trim().to_lowercase();
        let Some(tab) = self.tabs.active_file_view().filter(|_| !query.is_empty()) else {
            return Rc::from([]);
        };
        let key = (tab.id, tab.body_generation, query);
        if let Some(memo) = self.file_search_memo.borrow().as_ref() {
            if memo.key == key {
                return Rc::clone(&memo.hits);
            }
        }
        let hits: Rc<[usize]> = match_diff_line_indices(tab.lines(), &key.2).into();
        *self.file_search_memo.borrow_mut() = Some(FileSearchMemo {
            key,
            hits: Rc::clone(&hits),
        });
        hits
    }

    /// File search step from the file cursor, like the diff pane: the
    /// first matching line after it (`dir` 0 / 1) or the last one before
    /// it (`dir` -1). Returns true when it wrapped.
    pub(super) fn apply_file_search(&mut self, dir: i32) -> bool {
        let hits = self.file_search_hits(&self.search_query);
        let cursor = self.tabs.active_file_view().map(|tab| tab.cursor);
        let Some(landing) = cursor.and_then(|cursor| step_from_anchor(&hits, Some(cursor), dir))
        else {
            self.search_hit = None;
            self.set_search_status(false);
            return false;
        };
        if let Some(tab) = self.tabs.active_file_view_mut() {
            tab.cursor = landing.target;
        }
        self.search_hit = Some(landing.target);
        self.set_search_status(true);
        landing.wrapped
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::super::action::{Action, Effect};
    use super::super::super::chrome::search_pill_label;
    use super::super::super::effect::ExplorerPreviewBody;
    use super::super::super::search::SearchPane;
    use super::super::{FocusPane, SEARCH_WRAPPED_TO_BOTTOM, SEARCH_WRAPPED_TO_TOP};
    use super::*;
    use crate::file_index::FileRead;
    use crate::snapshot::{build_workspace_snapshot, CheckoutKind, RepoSnapshot, SyncStatus};

    /// A clean `app` checkout.
    fn state() -> AppState {
        let repo = RepoSnapshot {
            repo: "app".into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            upstream_rewrite: None,
            head: "abc".into(),
            has_unstaged: false,
            has_staged: false,
            has_untracked: false,
            changes: Vec::new(),
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: Some("main".into()),
            default_branch: None,
            local_branches: Vec::new(),
        };
        let snapshot = build_workspace_snapshot(&[repo], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn cursor(app: &AppState) -> usize {
        app.tabs.active_file_view().expect("a file view").cursor
    }

    #[test]
    fn file_view_keys_move_the_file_cursor_not_the_tree() {
        let mut app = state();
        let lines: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
        let mut refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let long = "x".repeat(200);
        refs[0] = &long;
        app.open_loaded_inner_file("app", "src/main.rs", &refs);
        let tree_cursor = app
            .tabs
            .active_explorer()
            .and_then(|tab| tab.tree.cursor_rel().map(str::to_string));
        app.dispatch(Action::FocusRight);
        assert_eq!(app.focus, FocusPane::Right);
        app.layout.file_view_height = 10;
        app.layout.file_view_width = 40;

        app.dispatch(Action::Move(3));
        assert_eq!(cursor(&app), 3);
        app.dispatch(Action::PageMove(1));
        assert_eq!(cursor(&app), 12, "a page is the view height less one");
        app.dispatch(Action::MoveToEnd);
        assert_eq!(cursor(&app), 39);
        app.dispatch(Action::Move(5));
        assert_eq!(cursor(&app), 39, "clamped to the last line");
        app.dispatch(Action::MoveToStart);
        assert_eq!(cursor(&app), 0);
        app.dispatch(Action::Move(7));
        app.dispatch(Action::ArmGChord);
        app.dispatch(Action::MoveToStart);
        assert_eq!(cursor(&app), 0, "gg");
        assert_eq!(
            app.tabs
                .active_explorer()
                .and_then(|tab| tab.tree.cursor_rel().map(str::to_string)),
            tree_cursor,
            "tree cursor untouched"
        );

        // Wrap is on by default and stops the pan; turn it off first.
        app.dispatch(Action::ToggleDiffWrap);
        assert!(!app.diff_wrap);
        app.dispatch(Action::PanDiff(5));
        assert_eq!(app.tabs.active_file_view().unwrap().col_offset, 5);
        app.dispatch(Action::PanDiff(-2));
        assert_eq!(app.tabs.active_file_view().unwrap().col_offset, 3);
        app.dispatch(Action::ToggleDiffWrap);
        app.dispatch(Action::PanDiff(5));
        assert_eq!(
            app.tabs.active_file_view().unwrap().col_offset,
            3,
            "wrap stops the pan"
        );
    }

    #[test]
    fn armed_file_search_steps_and_wraps() {
        let mut app = state();
        let effect = app.open_file_in_explorer(
            "app".into(),
            "src/main.rs".into(),
            Some(0),
            Some("FOO".into()),
        );
        let Effect::Batch(effects) = effect else {
            panic!("folder listings and the file load: {effect:?}");
        };
        let Some(Effect::LoadExplorerPreview { tab_id, gen, .. }) = effects.last().cloned() else {
            panic!("{effects:?}");
        };
        let body = FileRead::Text {
            lines: [
                "fn main() {",
                "  foo();",
                "}",
                "fn foo() {",
                "  bar();",
                "}",
            ]
            .map(str::to_string)
            .to_vec(),
            max_cols: 11,
        };
        assert!(app.apply_explorer_preview(tab_id, gen, ExplorerPreviewBody::File(body)));
        assert_eq!(app.search_target, SearchPane::File);
        assert_eq!(cursor(&app), 0);
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 1, "first match after the cursor");
        assert_eq!(app.search_match_position(), Some((Some(1), 2)));
        assert_eq!(search_pill_label(&app).as_deref(), Some("/FOO 1/2 · file"));
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 3);
        assert!(app.status.is_empty(), "{}", &*app.status);
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 1);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        app.dispatch(Action::SearchPrev);
        assert_eq!(cursor(&app), 3);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);
        assert_eq!(&*app.file_search_hits("foo"), &[1, 3]);
    }

    #[test]
    fn click_focuses_the_painted_line() {
        let mut app = state();
        app.open_loaded_inner_file("app", "README.md", &["a", "b", "c"]);
        app.layout.explorer_preview_rows = Rect::new(0, 2, 32, 5);
        app.layout.file_view_x = 1;
        app.layout.file_view_y = 2;
        app.layout.file_view_width = 30;
        app.layout.file_view_height = 5;
        app.layout.file_view_rows = Rect::new(0, 2, 32, 5);
        app.layout.file_view_row_lines = vec![0, 1, 1, 2];
        app.dispatch(Action::Click { col: 5, row: 4 });
        assert_eq!(app.focus, FocusPane::Right, "a press focuses the file");
        assert_eq!(cursor(&app), 1);
        assert!(app.text_selection.is_some());
        app.dispatch(Action::Release);
        assert!(app.text_selection.is_none());
        // Ctrl+click is a plain click here: the file view paints no PR badge.
        assert_eq!(
            app.dispatch(Action::CtrlClick { col: 5, row: 5 }),
            Effect::None
        );
        assert_eq!(cursor(&app), 2);
        assert!(app.text_selection.is_some());
    }
}

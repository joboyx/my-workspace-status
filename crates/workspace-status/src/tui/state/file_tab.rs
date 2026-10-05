//! Read-only file tabs: open / focus, keys, search, and loads.
//!
//! A file tab paints one file of one checkout. Its body is read on the
//! blocking pool ([`Effect::LoadFileTab`]) and lands through
//! [`AppState::apply_file_tab`], which drops a result for an older load
//! generation. Git writes refuse with the Workspace-tab copy.

use std::rc::Rc;
use std::sync::Arc;

use ratatui::layout::Rect;

use crate::file_index::FileRead;

use super::super::action::{Action, Effect};
use super::super::command_palette::{CommandScope, PaletteCommand};
use super::super::search::{apply_pan, match_diff_line_indices, step_from_anchor};
use super::super::selection::TextSelection;
use super::super::split::SplitDrag;
use super::super::status::StatusMessage;
use super::super::tabs::{
    checkout_leaf, file_gutter_width, OpenFile, ONLY_WORKSPACE_TAB_OPEN, SWITCH_TO_WORKSPACE_TAB,
};
use super::{AppState, FileSearchMemo, NO_SEARCH_ARMED, Z_FOLDS_TREE_ROWS};

impl AppState {
    /// Open or focus the file tab for `rel` in `checkout`.
    ///
    /// Parks the active tab's session first. A new tab returns its first
    /// [`Effect::LoadFileTab`]; an existing one is focused unchanged.
    pub(crate) fn open_file_tab(&mut self, checkout: String, rel: String) -> Effect {
        let before = self.tabs.active;
        self.park_active_session();
        let display = format!("{}/{rel}", checkout_leaf(&checkout));
        let opened = self
            .tabs
            .open_or_focus_file(checkout.clone(), rel.clone(), display);
        if self.tabs.active != before {
            self.clear_tab_transients();
        }
        self.apply_active_session();
        match opened {
            OpenFile::Focused => Effect::None,
            OpenFile::Created(tab_id) => Effect::LoadFileTab {
                tab_id,
                gen: 0,
                repo: checkout,
                path: rel,
            },
        }
    }

    /// Why `action` refuses on the active file tab, or `None` (also off a
    /// file tab).
    ///
    /// Git writes, comments, highlight, diff-only keys, and the commands
    /// that open a compare tab need the Workspace tab. So does a confirm:
    /// a late checkout / merge result can open one while a file tab is
    /// active, and `y` there must not write (n / Esc still close it). [`Self::dispatch`]
    /// puts the reason on the status line; `palette_disabled_reason` shows
    /// the same copy on the row.
    pub(crate) fn file_tab_refusal(&self, action: &Action) -> Option<String> {
        if !self.is_file_tab() {
            return None;
        }
        matches!(
            action,
            Action::Stage
                | Action::Unstage
                | Action::Revert
                | Action::StashMenu
                | Action::StashMenuEnter
                | Action::Fetch
                | Action::Pull
                | Action::Push
                | Action::DefaultBranch
                | Action::Branch
                | Action::BranchSubmit
                | Action::CreateBranchSubmit
                | Action::RemoveWorktree
                | Action::GraphStashApply
                | Action::GraphStashPop
                | Action::GraphStashDrop
                | Action::GraphCheckout
                | Action::GraphCreateBranch
                | Action::GraphMerge
                | Action::GraphFocusBranches
                | Action::GraphFocusClear
                | Action::GraphFocusSubmit
                | Action::ToggleReviewed
                | Action::CommentStart
                | Action::ExportComments
                | Action::DiffVisualStart
                | Action::ExternalDiff
                | Action::ToggleFullContext
                | Action::CompareVsDefault
                | Action::CompareVsBranch
                | Action::CompareVsCommit
                | Action::CompareCommitVsParent
                | Action::ConfirmYes
                | Action::ConfirmYesClean
        )
        .then(|| SWITCH_TO_WORKSPACE_TAB.to_string())
    }

    /// Palette row reason on a file tab after [`Self::file_tab_refusal`].
    ///
    /// The Workspace gates read the parked tree, so a file tab answers the
    /// few rows whose state lives here. View rows for the Workspace panes
    /// (tree / flat, ignored repos, inline / split, tree width, other pane,
    /// commit message) do nothing on a file tab and say so; the rest run.
    pub(crate) fn file_tab_palette_reason(&self, command: &PaletteCommand) -> Option<String> {
        if command.scope == CommandScope::Highlight {
            return Some("highlight diff lines first (V)".into());
        }
        match command.action {
            Action::SearchNext | Action::SearchPrev => {
                (!self.search_is_armed()).then(|| NO_SEARCH_ARMED.into())
            }
            Action::NextTab | Action::PreviousTab => {
                (self.tabs.len() <= 1).then(|| ONLY_WORKSPACE_TAB_OPEN.into())
            }
            Action::FoldToggleSubtree => Some(Z_FOLDS_TREE_ROWS.into()),
            Action::ToggleTreeMode
            | Action::ToggleShowIgnored
            | Action::ToggleDiffMode
            | Action::ResizeTree(_)
            | Action::FocusLeft
            | Action::FocusRight
            | Action::ToggleCommitMsgExpand => Some(SWITCH_TO_WORKSPACE_TAB.into()),
            _ => None,
        }
    }

    /// Keys the active file tab handles itself.
    ///
    /// `None` lets [`Self::dispatch`] fall through to the shared router
    /// (quit, help, theme, mouse, tabs, Quick Open, wrap, search, copy
    /// reference, the tab row, drag / release, resize, ticks).
    pub(super) fn dispatch_file_tab(&mut self, action: &Action) -> Option<Effect> {
        match *action {
            Action::Move(delta) => {
                self.step_file_cursor(i64::from(delta));
                Some(Effect::None)
            }
            Action::MoveToStart => {
                self.set_file_cursor(0);
                Some(Effect::None)
            }
            Action::MoveToEnd => {
                self.set_file_cursor(usize::MAX);
                Some(Effect::None)
            }
            Action::PageMove(pages) => {
                let page = self.layout.file_view_height.saturating_sub(1).max(1);
                self.step_file_cursor(i64::from(pages) * i64::from(page));
                Some(Effect::None)
            }
            Action::PanDiff(delta) => {
                self.pan_file_tab(delta);
                Some(Effect::None)
            }
            Action::ScrollWheel {
                delta, horizontal, ..
            } => {
                if self.mouse_enabled {
                    if horizontal {
                        self.pan_file_tab(delta);
                    } else {
                        self.step_file_cursor(i64::from(delta));
                    }
                }
                Some(Effect::None)
            }
            Action::Edit => Some(self.edit_file_tab()),
            Action::Refresh => Some(self.reload_active_file_tab()),
            Action::Click { col, row } if row != self.layout.tab_y => {
                if self.mouse_enabled {
                    self.click_file_tab(col, row);
                }
                Some(Effect::None)
            }
            Action::FoldToggle
            | Action::FoldToggleSubtree
            | Action::FoldOpen
            | Action::FoldClose => {
                self.status = StatusMessage::warn(Z_FOLDS_TREE_ROWS);
                Some(Effect::None)
            }
            // Esc (or a right click) clears an armed search in the router.
            Action::NavEsc if self.search_active => None,
            Action::BackClick if self.search_active && self.mouse_enabled => {
                Some(self.dispatch(Action::NavEsc))
            }
            Action::NavEnter
            | Action::NavEsc
            | Action::FocusLeft
            | Action::FocusRight
            | Action::ToggleTreeMode
            | Action::ToggleShowIgnored
            | Action::ToggleDiffMode
            | Action::ToggleCommitMsgExpand
            | Action::ResizeTree(_)
            | Action::BackClick => Some(Effect::None),
            _ => None,
        }
    }

    /// Put `cursor` on the active file tab, clamped to its lines.
    fn set_file_cursor(&mut self, cursor: usize) {
        if let Some(tab) = self.tabs.active_file_mut() {
            let last = tab.lines().len().saturating_sub(1);
            tab.cursor = cursor.min(last);
        }
    }

    fn step_file_cursor(&mut self, delta: i64) {
        let Some(cursor) = self.tabs.active_file().map(|tab| tab.cursor) else {
            return;
        };
        let next = (cursor as i64).saturating_add(delta).max(0);
        self.set_file_cursor(usize::try_from(next).unwrap_or(usize::MAX));
    }

    /// Pan the code columns. A no-op while wrap is on.
    fn pan_file_tab(&mut self, delta: i32) {
        if self.diff_wrap {
            return;
        }
        let view = usize::from(self.layout.file_view_width);
        let Some(tab) = self.tabs.active_file_mut() else {
            return;
        };
        let max_cols = match tab.body.as_deref() {
            Some(FileRead::Text { max_cols, .. }) => *max_cols,
            _ => 0,
        };
        let code_w = view.saturating_sub(file_gutter_width(tab.lines().len()));
        tab.col_offset = apply_pan(tab.col_offset, delta, max_cols.saturating_sub(code_w));
    }

    /// `e`: open the file in the editor at the cursor line (no line for a
    /// body that did not load as text).
    fn edit_file_tab(&mut self) -> Effect {
        let Some(tab) = self.tabs.active_file() else {
            return Effect::None;
        };
        let line = matches!(tab.body.as_deref(), Some(FileRead::Text { .. }))
            .then(|| u32::try_from(tab.cursor + 1).unwrap_or(u32::MAX));
        let (repo, path) = (tab.checkout.clone(), tab.rel.clone());
        self.status = StatusMessage::progress(format!("opening {path}…"));
        Effect::EditFile { repo, path, line }
    }

    /// `r`: read the active file tab again.
    fn reload_active_file_tab(&mut self) -> Effect {
        let Some(tab) = self.tabs.active_file_mut() else {
            return Effect::None;
        };
        let gen = tab.bump_generation();
        Effect::LoadFileTab {
            tab_id: tab.id,
            gen,
            repo: tab.checkout.clone(),
            path: tab.rel.clone(),
        }
    }

    /// Reload for the active file tab when the editor just saved its file.
    pub(crate) fn reload_file_tab_after_edit(&mut self, repo: &str, path: &str) -> Option<Effect> {
        self.tabs
            .active_file()
            .filter(|tab| tab.checkout == repo && tab.rel == path)?;
        Some(self.reload_active_file_tab())
    }

    /// Press in the body: focus the clicked line and arm a text selection.
    fn click_file_tab(&mut self, col: u16, row: u16) {
        self.drag = SplitDrag::None;
        let layout = &self.layout;
        let body = Rect::new(
            layout.file_view_x,
            layout.file_view_y,
            layout.file_view_width,
            layout.file_view_height,
        );
        let line = row
            .checked_sub(layout.file_view_y)
            .and_then(|offset| layout.file_view_row_lines.get(usize::from(offset)))
            .copied()
            .filter(|_| col >= body.x && col < body.right());
        self.text_selection = TextSelection::arm(body, col, row);
        if let Some(line) = line {
            self.set_file_cursor(line);
        }
    }

    /// Accept a finished read for tab `tab_id` at load generation `gen`.
    ///
    /// Dropped (false) when the tab closed or a newer load started. The
    /// cursor and scroll clamp to the new line count.
    pub(crate) fn apply_file_tab(&mut self, tab_id: u64, gen: u64, body: FileRead) -> bool {
        let active = self.tabs.active_file().is_some_and(|tab| tab.id == tab_id);
        let Some(tab) = self
            .tabs
            .get_file_id_mut(tab_id)
            .filter(|tab| tab.generation == gen)
        else {
            return false;
        };
        tab.body = Some(Arc::new(body));
        let len = tab.lines().len();
        tab.cursor = tab.cursor.min(len.saturating_sub(1));
        tab.scroll = tab.scroll.min(tab.cursor);
        if tab.search_hit.is_some_and(|hit| hit >= len) {
            tab.search_hit = None;
        }
        if active && self.search_hit.is_some_and(|hit| hit >= len) {
            self.search_hit = None;
        }
        true
    }

    /// Lines of the active file tab that contain `query`, ignoring case,
    /// ascending.
    ///
    /// Memoized per tab, load, and query; the paint and the status pill
    /// share one list instead of copying it each frame.
    pub(crate) fn file_search_hits(&self, query: &str) -> Rc<[usize]> {
        let query = query.trim().to_lowercase();
        let Some(tab) = self.tabs.active_file().filter(|_| !query.is_empty()) else {
            return Rc::from([]);
        };
        let key = (tab.id, tab.generation, query);
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
        let cursor = self.tabs.active_file().map(|tab| tab.cursor);
        let Some(landing) = cursor.and_then(|cursor| step_from_anchor(&hits, Some(cursor), dir))
        else {
            self.search_hit = None;
            self.set_search_status(false);
            return false;
        };
        if let Some(tab) = self.tabs.active_file_mut() {
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

    use super::super::super::action::QuickOpenEntry;
    use super::super::super::command_palette::PALETTE_COMMANDS;
    use super::super::super::quick_open::{FileIndexState, QuickOpenScope, QuickOpenState};
    use super::super::{FocusPane, SEARCH_WRAPPED_TO_BOTTOM, SEARCH_WRAPPED_TO_TOP};
    use super::*;
    use crate::file_index::{FileEntry, FileHit, FileIndex, IndexRoot};
    use crate::snapshot::{
        build_workspace_snapshot, CheckoutKind, FileChange, RepoSnapshot, SyncStatus,
    };
    use crate::tui::drill::{CommitFile, CommitFileSource};
    use crate::tui::search::SearchPane;

    fn repo(name: &str) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: "abc".into(),
            has_unstaged: true,
            has_staged: false,
            has_untracked: false,
            changes: vec![FileChange {
                path: "README.md".into(),
                staged_status: None,
                unstaged_status: Some("M".into()),
                untracked: false,
                old_path: None,
            }],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: Some("main".into()),
            local_branches: Vec::new(),
        }
    }

    fn state() -> AppState {
        let snapshot = build_workspace_snapshot(&[repo("app"), repo("lib")], &[], false, &[]);
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn text(lines: &[&str]) -> FileRead {
        FileRead::Text {
            lines: lines.iter().map(|line| (*line).to_string()).collect(),
            max_cols: lines.iter().map(|line| line.len()).max().unwrap_or(0),
        }
    }

    /// Open `rel` in `app` and land `lines` as its body.
    fn open_loaded(app: &mut AppState, rel: &str, lines: &[&str]) -> u64 {
        let Effect::LoadFileTab { tab_id, gen, .. } = app.open_file_tab("app".into(), rel.into())
        else {
            panic!("expected a load");
        };
        assert!(app.apply_file_tab(tab_id, gen, text(lines)));
        tab_id
    }

    fn cursor(app: &AppState) -> usize {
        app.tabs.active_file().expect("file tab").cursor
    }

    fn quick_open_with(app: &mut AppState, paths: &[&str], hits: &[usize]) {
        let mut quick = QuickOpenState::new(QuickOpenEntry::Files, QuickOpenScope::Workspace);
        quick.index = FileIndexState::Ready(Arc::new(FileIndex {
            roots: vec![IndexRoot {
                checkout: "app".into(),
                prefix: "app/".into(),
            }],
            entries: paths
                .iter()
                .map(|path| FileEntry {
                    root: 0,
                    display: format!("app/{path}"),
                    rel_start: 4,
                })
                .collect(),
            truncated: false,
            errors: Vec::new(),
        }));
        quick.hits = hits
            .iter()
            .map(|&entry| FileHit {
                entry,
                score: 1,
                indices: Vec::new(),
            })
            .collect();
        app.quick_open = Some(quick);
    }

    #[test]
    fn quick_open_enter_opens_file_tab() {
        let mut app = state();
        quick_open_with(&mut app, &["README.md", "src/main.rs"], &[1, 0]);
        let effect = app.dispatch(Action::QuickOpenSubmit);
        assert!(app.quick_open.is_none(), "Enter closes the overlay");
        assert!(app.is_file_tab());
        assert!(!app.is_compare_tab());
        let tab = app.tabs.active_file().unwrap();
        assert_eq!(
            (tab.checkout.as_str(), tab.rel.as_str()),
            ("app", "src/main.rs")
        );
        assert_eq!(tab.display, "app/src/main.rs");
        assert_eq!(
            effect,
            Effect::LoadFileTab {
                tab_id: tab.id,
                gen: 0,
                repo: "app".into(),
                path: "src/main.rs".into(),
            }
        );

        app.dispatch(Action::JumpToTab(1));
        quick_open_with(&mut app, &["README.md", "src/main.rs"], &[1]);
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert_eq!(app.tabs.len(), 2, "same file focuses, no new tab");
        assert!(app.is_file_tab());
    }

    #[test]
    fn file_tab_refuses_writes_with_switch_copy() {
        let mut app = state();
        open_loaded(&mut app, "README.md", &["# app"]);
        for action in [
            Action::Stage,
            Action::Unstage,
            Action::Revert,
            Action::StashMenu,
            Action::Fetch,
            Action::CommentStart,
        ] {
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(app.status, SWITCH_TO_WORKSPACE_TAB, "{action:?}");
        }
        let stage = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == "Stage")
            .expect("Stage row");
        assert_eq!(
            app.palette_disabled_reason(stage).as_deref(),
            Some(SWITCH_TO_WORKSPACE_TAB)
        );
        let edit = PALETTE_COMMANDS
            .iter()
            .find(|command| command.title == "Open in editor")
            .expect("Open in editor row");
        assert_eq!(app.palette_disabled_reason(edit), None);
    }

    #[test]
    fn file_tab_nav_moves_file_cursor_not_tree() {
        let mut app = state();
        let tree_cursor = app.cursor;
        let folds = app.folds.clone();
        let lines: Vec<String> = (0..40).map(|i| format!("line {i}")).collect();
        let mut refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let long = "x".repeat(200);
        refs[0] = &long;
        open_loaded(&mut app, "src/main.rs", &refs);
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
        assert_eq!(app.cursor, tree_cursor, "tree cursor untouched");

        assert!(!app.hl_folds());
        // Wrap is on by default and stops the pan; turn it off first.
        app.dispatch(Action::ToggleDiffWrap);
        assert!(!app.diff_wrap);
        app.dispatch(Action::PanDiff(5));
        assert_eq!(app.tabs.active_file().unwrap().col_offset, 5);
        app.dispatch(Action::PanDiff(-2));
        assert_eq!(app.tabs.active_file().unwrap().col_offset, 3);
        app.dispatch(Action::FoldToggle);
        assert_eq!(app.status, Z_FOLDS_TREE_ROWS);
        assert_eq!(app.folds, folds, "no fold change");
        app.dispatch(Action::ToggleDiffWrap);
        app.dispatch(Action::PanDiff(5));
        assert_eq!(
            app.tabs.active_file().unwrap().col_offset,
            3,
            "wrap stops the pan"
        );
    }

    #[test]
    fn file_tab_search_steps_and_wraps() {
        let mut app = state();
        open_loaded(
            &mut app,
            "src/main.rs",
            &[
                "fn main() {",
                "  foo();",
                "}",
                "fn foo() {",
                "  bar();",
                "}",
            ],
        );
        app.dispatch(Action::SearchStart);
        assert_eq!(app.search_target, SearchPane::File);
        for c in "FOO".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        app.dispatch(Action::SearchSubmit);
        assert_eq!(cursor(&app), 1, "first match after the cursor");
        assert_eq!(app.search_match_position(), Some((Some(1), 2)));
        assert_eq!(
            crate::tui::chrome::search_pill_label(&app).as_deref(),
            Some("/FOO 1/2 · file")
        );
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 3);
        assert!(app.status.is_empty(), "{}", &*app.status);
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 1);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);
        app.dispatch(Action::SearchPrev);
        assert_eq!(cursor(&app), 3);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_BOTTOM);

        // `n` / `N` step from wherever the cursor moved, not the last hit.
        app.dispatch(Action::MoveToStart);
        app.dispatch(Action::Move(2));
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 3, "next below line 2");
        app.dispatch(Action::MoveToEnd);
        app.dispatch(Action::SearchPrev);
        assert_eq!(cursor(&app), 3, "previous above the last line");
        assert!(app.status.is_empty(), "{}", &*app.status);
        app.dispatch(Action::SearchNext);
        assert_eq!(cursor(&app), 1);
        assert_eq!(app.status, SEARCH_WRAPPED_TO_TOP);

        app.dispatch(Action::NavEsc);
        assert!(!app.search_active, "Esc clears the armed search");
    }

    #[test]
    fn file_tab_e_opens_editor_at_cursor_line() {
        let mut app = state();
        open_loaded(&mut app, "src/main.rs", &["a", "b", "c", "d", "e", "f"]);
        app.dispatch(Action::Move(4));
        match app.dispatch(Action::Edit) {
            Effect::EditFile { repo, path, line } => {
                assert_eq!((repo.as_str(), path.as_str()), ("app", "src/main.rs"));
                assert_eq!(line, Some(5));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, "opening src/main.rs…");

        let Effect::LoadFileTab { tab_id, gen, .. } =
            app.open_file_tab("app".into(), "logo.png".into())
        else {
            panic!("expected a load");
        };
        app.apply_file_tab(tab_id, gen, FileRead::Binary);
        assert!(matches!(
            app.dispatch(Action::Edit),
            Effect::EditFile { line: None, .. }
        ));
    }

    #[test]
    fn closing_file_tab_restores_workspace_session() {
        let mut app = state();
        app.focus = FocusPane::Right;
        app.search_active = true;
        app.search_query = "readme".into();
        app.search_target = SearchPane::Tree;
        app.diff_cursor = 7;
        open_loaded(&mut app, "src/main.rs", &["fn main() {}", "// main"]);
        assert!(!app.search_active, "a new file tab has no search");
        app.dispatch(Action::SearchStart);
        for c in "main".chars() {
            app.dispatch(Action::SearchChar(c));
        }
        app.dispatch(Action::SearchSubmit);
        assert!(app.search_active);
        assert_eq!(app.search_target, SearchPane::File);

        app.dispatch(Action::CloseTab);
        assert!(app.tabs.is_workspace());
        assert_eq!(app.focus, FocusPane::Right);
        assert!(app.search_active);
        assert_eq!(app.search_query, "readme");
        assert_eq!(app.search_target, SearchPane::Tree);
        assert_eq!(app.diff_cursor, 7);
    }

    #[test]
    fn closing_file_tab_keeps_a_compare_tab_working() {
        let mut app = state();
        open_loaded(&mut app, "README.md", &["# app"]);
        app.open_compare_tab("app".into(), "main".into(), "HEAD".into());
        assert!(app.is_compare_tab());
        app.dispatch(Action::JumpToTab(2));
        assert!(app.is_file_tab());
        app.dispatch(Action::CloseTab);
        assert!(app.tabs.is_workspace());
        assert_eq!(app.tabs.compare_count(), 1);
        app.dispatch(Action::NextTab);
        assert!(app.is_compare_tab());
        assert_eq!(app.compare_probe_effects().len(), 0, "range still loading");

        let (tab_id, gen) = {
            let tab = app.tabs.active_compare().unwrap();
            (tab.id, tab.generation)
        };
        let load = crate::tui::app::CompareRangeLoad {
            source: CommitFileSource::Compare {
                base_ref: "main".into(),
                head_ref: "HEAD".into(),
                base_tip: "bbb".into(),
                merge_base: "aaa".into(),
                head: "ccc".into(),
            },
            files: vec![CommitFile {
                status: "M".into(),
                path: "src/a.rs".into(),
                old_path: None,
                stat: None,
            }],
            head: "ccc".into(),
            base_tip: "bbb".into(),
        };
        let follow = app.apply_compare_range(tab_id, gen, Ok(load));
        assert!(
            matches!(follow, Some(Effect::LoadCompareDiff { tab_id: id, ref path, .. }) if id == tab_id && path == "src/a.rs"),
            "{follow:?}"
        );
        let tab = app.tabs.get_id(tab_id).expect("compare tab by id");
        assert!(!tab.loading);
        assert_eq!(tab.files.len(), 1);
        assert_eq!(tab.files[0].path, "src/a.rs");
        assert_eq!(app.compare_probe_effects().len(), 1, "loaded tab probes");
    }

    #[test]
    fn late_checkout_confirm_cannot_write_from_a_file_tab() {
        let mut app = state();
        open_loaded(&mut app, "README.md", &["# app"]);
        crate::tui::app::apply_checkout_compute(
            &mut app,
            "app".into(),
            crate::tui::app::CheckoutCompute::Confirm {
                local_branch: "main".into(),
                remote_ref: "origin/main".into(),
                ahead_behind: Some((0, 1)),
            },
        );
        assert!(app.confirm.is_some(), "the late result opens its confirm");
        for action in [Action::ConfirmYes, Action::ConfirmYesClean] {
            app.status.clear();
            assert_eq!(app.dispatch(action.clone()), Effect::None, "{action:?}");
            assert_eq!(app.status, SWITCH_TO_WORKSPACE_TAB, "{action:?}");
            assert!(app.confirm.is_some(), "{action:?} does not consume it");
        }
        assert_eq!(app.dispatch(Action::ConfirmNo), Effect::None);
        assert!(app.confirm.is_none(), "n still closes it");
    }

    #[test]
    fn workspace_view_rows_are_disabled_on_a_file_tab() {
        let mut app = state();
        open_loaded(&mut app, "README.md", &["# app"]);
        let reason = |title: &str| {
            let command = PALETTE_COMMANDS
                .iter()
                .find(|command| command.title == title)
                .unwrap_or_else(|| panic!("no {title} row"));
            app.palette_disabled_reason(command)
        };
        for title in [
            "Flat / tree",
            "Show ignored",
            "Inline / split",
            "Narrow tree",
            "Widen tree",
            "Other pane",
            "Collapse / expand commit message",
        ] {
            assert_eq!(
                reason(title).as_deref(),
                Some(SWITCH_TO_WORKSPACE_TAB),
                "{title}"
            );
        }
        for title in [
            "Wrap / unwrap",
            "Cycle theme",
            "Toggle mouse",
            "Keymap help",
            "Quit",
            "Close tab",
            "Refresh",
            "Open in editor",
            "Copy entity reference",
            "Search focused pane",
        ] {
            assert_eq!(reason(title), None, "{title}");
        }
    }

    #[test]
    fn quick_open_scope_on_file_tab_is_its_checkout() {
        let mut app = state();
        app.cursor = 0;
        assert_eq!(app.quick_open_scope(), QuickOpenScope::Workspace);
        app.open_file_tab("lib".into(), "src/lib.rs".into());
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout("lib".into())
        );
    }

    #[test]
    fn stale_load_and_reload_follow_the_generation() {
        let mut app = state();
        let id = open_loaded(&mut app, "README.md", &["a", "b", "c"]);
        app.dispatch(Action::MoveToEnd);
        let Effect::LoadFileTab { gen, .. } = app.dispatch(Action::Refresh) else {
            panic!("r reloads");
        };
        assert_eq!(gen, 1);
        assert!(app.tabs.active_file().unwrap().body.is_none());
        assert!(!app.apply_file_tab(id, 0, text(&["old"])), "stale gen");
        assert!(app.apply_file_tab(id, 1, text(&["only"])));
        assert_eq!(cursor(&app), 0, "cursor clamps to the new lines");
        assert!(app.reload_file_tab_after_edit("app", "other.md").is_none());
        assert!(matches!(
            app.reload_file_tab_after_edit("app", "README.md"),
            Some(Effect::LoadFileTab { gen: 2, .. })
        ));
    }

    #[test]
    fn click_focuses_the_painted_line() {
        let mut app = state();
        open_loaded(&mut app, "README.md", &["a", "b", "c"]);
        app.layout.file_view_x = 1;
        app.layout.file_view_y = 2;
        app.layout.file_view_width = 30;
        app.layout.file_view_height = 5;
        app.layout.file_view_row_lines = vec![0, 1, 1, 2];
        app.dispatch(Action::Click { col: 5, row: 4 });
        assert_eq!(cursor(&app), 1);
        assert!(app.text_selection.is_some());
        app.dispatch(Action::Release);
        assert!(app.text_selection.is_none());
    }
}

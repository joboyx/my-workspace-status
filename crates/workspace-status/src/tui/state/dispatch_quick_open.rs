//! Quick Open overlay: open / close, query edits, submit, and job results.
//!
//! Index loads and scores leave here as [`Effect::LoadFileIndex`] /
//! [`Effect::ScoreFiles`] and come back through [`AppState::apply_file_index`]
//! / [`AppState::apply_file_score`], which drop any result whose generation
//! is no longer the overlay's latest. Files-mode Enter opens the picked file
//! as a pinned inner file tab of its checkout's Explorer tab; an Enter that
//! arrives before the query's score waits for it.

use std::sync::Arc;

use crate::file_index::{FileHit, FileIndex, IndexRoot};
use crate::snapshot::CheckoutKind;

use super::super::action::{Action, Effect, QuickOpenEntry};
use super::super::command_palette::CommandPaletteState;
use super::super::ctrl_c_exit::is_ctrl_c_exit_prompt;
use super::super::quick_open::{
    commands_filter, FileIndexState, QuickOpenMode, QuickOpenScope, QuickOpenState, NO_FILE_MATCHES,
};
use super::super::status::StatusMessage;
use super::{visible_snapshot, AppState, FocusPane};

impl AppState {
    /// Commands-mode list of the open Quick Open overlay.
    ///
    /// `None` when the overlay is closed or in files mode.
    pub fn command_palette(&self) -> Option<&CommandPaletteState> {
        self.quick_open
            .as_ref()
            .filter(|quick| quick.mode() == QuickOpenMode::Commands)
            .map(|quick| &quick.commands)
    }

    /// Mutable [`Self::command_palette`].
    ///
    /// `None` when the overlay is closed or in files mode.
    pub(crate) fn command_palette_mut(&mut self) -> Option<&mut CommandPaletteState> {
        self.quick_open
            .as_mut()
            .filter(|quick| quick.mode() == QuickOpenMode::Commands)
            .map(|quick| &mut quick.commands)
    }

    fn next_quick_open_gen(&mut self) -> u64 {
        self.quick_open_gen += 1;
        self.quick_open_gen
    }

    /// Checkouts Quick Open lists for the focused context.
    ///
    /// The active Explorer tab's checkout; the active compare tab's
    /// checkout; else the focused row's checkout
    /// ([`Self::focused_checkout_path`]); else (workspace row, group) the
    /// whole workspace.
    pub(crate) fn quick_open_scope(&self) -> QuickOpenScope {
        if let Some(tab) = self.tabs.active_explorer() {
            return QuickOpenScope::Checkout(tab.checkout.clone());
        }
        if let Some(tab) = self.tabs.active_compare() {
            return QuickOpenScope::Checkout(tab.checkout_path.clone());
        }
        match self.focused_checkout_path() {
            Some(path) => QuickOpenScope::Checkout(path),
            None => QuickOpenScope::Workspace,
        }
    }

    /// Index roots for `scope`.
    ///
    /// Workspace: every primary checkout of the visible snapshot, each
    /// prefixed with its snapshot `repo` path plus `/` (unique per checkout,
    /// unlike the leaf). One checkout: that checkout, no prefix.
    pub(crate) fn quick_open_roots(&self, scope: &QuickOpenScope) -> Vec<IndexRoot> {
        match scope {
            QuickOpenScope::Workspace => visible_snapshot(&self.snapshot, self.show_hidden)
                .repos
                .into_iter()
                .filter(|repo| repo.checkout_kind == CheckoutKind::Primary)
                .map(|repo| IndexRoot {
                    prefix: format!("{}/", repo.repo),
                    checkout: repo.repo,
                })
                .collect(),
            QuickOpenScope::Checkout(path) => vec![IndexRoot {
                checkout: path.clone(),
                prefix: String::new(),
            }],
        }
    }

    /// Route one Quick Open [`Action`].
    pub(crate) fn dispatch_quick_open(&mut self, action: Action) -> Effect {
        match action {
            Action::ToggleQuickOpen(entry) => self.toggle_quick_open(entry),
            Action::QuickOpenMove(delta) => {
                if let Some(quick) = self.quick_open.as_mut() {
                    quick.submit_on_ready = false;
                    match quick.mode() {
                        QuickOpenMode::Commands => quick.commands.move_cursor(delta),
                        QuickOpenMode::Files => quick.move_file_cursor(delta),
                    }
                }
                self.sync_theme_preview();
                Effect::None
            }
            Action::QuickOpenChar(c) => {
                let Some(mut query) = self.quick_open.as_ref().map(|q| q.query.clone()) else {
                    return Effect::None;
                };
                query.push(c);
                self.quick_open_edit(query)
            }
            Action::QuickOpenBackspace => {
                let Some(mut query) = self.quick_open.as_ref().map(|q| q.query.clone()) else {
                    return Effect::None;
                };
                query.pop();
                self.quick_open_edit(query)
            }
            Action::QuickOpenSubmit => self.submit_quick_open(),
            Action::QuickOpenCancel => {
                self.close_quick_open();
                Effect::None
            }
            _ => Effect::None,
        }
    }

    fn toggle_quick_open(&mut self, entry: QuickOpenEntry) -> Effect {
        if self.quick_open.is_some() {
            self.close_quick_open();
            return Effect::None;
        }
        self.cancel_mouse_drag();
        self.help_open = false;
        self.clear_help_search();
        // A leftover toast would sit in the status row and never expire
        // while the overlay is open. The quit prompt stays: it shows inline.
        if !is_ctrl_c_exit_prompt(&self.status) {
            self.status.clear();
        }
        let scope = self.quick_open_scope();
        self.quick_open = Some(QuickOpenState::new(entry, scope));
        // Files mode can turn into commands mode (`>`), so both entries
        // keep the theme that a preview would replace.
        self.theme_before_preview = Some(self.theme);
        match entry {
            QuickOpenEntry::Commands => {
                self.land_palette_cursor();
                self.sync_theme_preview();
                Effect::None
            }
            QuickOpenEntry::Files => self.ensure_file_index(),
        }
    }

    /// Close the overlay with no run. A disabled-row reason that Enter put
    /// on the status line goes too, so it does not linger after the close.
    /// A previewed theme goes back to the theme active on open.
    fn close_quick_open(&mut self) {
        self.end_theme_preview();
        let shown = self
            .quick_open
            .take()
            .and_then(|quick| quick.commands.shown_reason);
        if shown.is_some_and(|reason| self.status == reason) {
            self.status.clear();
        }
    }

    /// Paint the highlighted palette `Theme: …` row's theme, else the theme
    /// active when Quick Open opened. Runs after open, each move, and each
    /// query edit. It never saves: only Enter on the row
    /// ([`Action::SetTheme`]) does.
    fn sync_theme_preview(&mut self) {
        let Some(committed) = self.theme_before_preview else {
            return;
        };
        let focused = self
            .command_palette()
            .and_then(|palette| palette.selected())
            .and_then(|command| match command.action {
                Action::SetTheme(id) => Some(id),
                _ => None,
            });
        self.theme = focused.unwrap_or(committed);
    }

    /// Put back the theme active when Quick Open opened and drop the
    /// snapshot. Every close calls this before it runs anything.
    fn end_theme_preview(&mut self) {
        if let Some(committed) = self.theme_before_preview.take() {
            self.theme = committed;
        }
    }

    /// Request the scope's file index once per overlay.
    fn ensure_file_index(&mut self) -> Effect {
        let Some(scope) = self
            .quick_open
            .as_ref()
            .filter(|quick| quick.index == FileIndexState::NotLoaded)
            .map(|quick| quick.scope.clone())
        else {
            return Effect::None;
        };
        let roots = self.quick_open_roots(&scope);
        let gen = self.next_quick_open_gen();
        if let Some(quick) = self.quick_open.as_mut() {
            quick.index = FileIndexState::Loading;
            quick.index_gen = gen;
        }
        Effect::LoadFileIndex { gen, roots }
    }

    /// Replace the query, then refilter commands or request a file score.
    ///
    /// A query that starts with `#` hands off instead: Quick Open closes
    /// and the search-in-files dialog opens as Ctrl-f would, with the text
    /// after `#` (leading spaces trimmed) as its query.
    fn quick_open_edit(&mut self, query: String) -> Effect {
        if self.quick_open.is_none() {
            return Effect::None;
        }
        if let Some(text) = query.strip_prefix('#') {
            let text = text.trim_start().to_string();
            self.close_quick_open();
            return self.open_search_files_with(text);
        }
        let Some(quick) = self.quick_open.as_mut() else {
            return Effect::None;
        };
        let before = quick.mode();
        quick.query = query;
        quick.submit_on_ready = false;
        let mode = quick.mode();
        if mode != before {
            // A disabled-row reason belongs to the commands list it came from.
            let reason = quick.commands.shown_reason.take();
            if reason.is_some_and(|reason| self.status == reason) {
                self.status.clear();
            }
        }
        match mode {
            QuickOpenMode::Commands => {
                // The files miss warning does not carry into commands.
                if self.status == NO_FILE_MATCHES {
                    self.status.clear();
                }
                if let Some(quick) = self.quick_open.as_mut() {
                    quick.commands.set_filter(commands_filter(&quick.query));
                }
                self.land_palette_cursor();
                self.sync_theme_preview();
                Effect::None
            }
            QuickOpenMode::Files => {
                // Enter's miss warning is about the old query.
                if self.status == NO_FILE_MATCHES {
                    self.status.clear();
                }
                // Backspace over `>` leaves commands mode and its preview.
                self.sync_theme_preview();
                self.request_file_score()
            }
        }
    }

    /// Score the current query against a ready index, or load the index
    /// first when none was requested yet.
    fn request_file_score(&mut self) -> Effect {
        let index = match self.quick_open.as_ref().map(|quick| &quick.index) {
            Some(FileIndexState::NotLoaded) => return self.ensure_file_index(),
            Some(FileIndexState::Ready(index)) => Arc::clone(index),
            _ => return Effect::None,
        };
        let gen = self.next_quick_open_gen();
        let Some(quick) = self.quick_open.as_mut() else {
            return Effect::None;
        };
        quick.score_gen = gen;
        quick.score_pending = true;
        Effect::ScoreFiles {
            gen,
            index,
            query: quick.query.clone(),
        }
    }

    /// Put the commands cursor on the first enabled landing row, else on the
    /// first landing row.
    ///
    /// Runs on open and on each filter change, so the HIGHLIGHT rows that
    /// paint first do not take the cursor while they are disabled. Landing
    /// rows are [`CommandPaletteState::landing_rows`]: a row found only by
    /// an alias never takes the cursor from an in-scope row found by its
    /// name, even a disabled one. The go-to-line row (`:42`), while it
    /// shows, is the only landing row, so Enter runs it or says why not.
    /// Moves still go over every row.
    fn land_palette_cursor(&mut self) {
        let highlighted = self.diff_visual_anchor.is_some();
        let Some((rows, landing)) = self
            .command_palette()
            .map(|p| (p.rows(), p.landing_rows(highlighted)))
        else {
            return;
        };
        let cursor = landing
            .iter()
            .copied()
            .find(|&index| self.palette_disabled_reason(&rows[index]).is_none())
            .or_else(|| landing.first().copied())
            .unwrap_or(0);
        if let Some(palette) = self.command_palette_mut() {
            palette.cursor = cursor;
        }
    }

    fn submit_quick_open(&mut self) -> Effect {
        match self.quick_open.as_ref().map(|quick| quick.mode()) {
            Some(QuickOpenMode::Commands) => self.submit_command(),
            Some(QuickOpenMode::Files) => self.submit_file(),
            None => Effect::None,
        }
    }

    /// Files-mode Enter: close, then open the highlighted file in its
    /// checkout's Explorer tab with the file pane focused
    /// ([`Self::open_file_in_explorer`]). Warns `no file matches` without a
    /// hit.
    ///
    /// While the index loads or the latest score is pending the hits are
    /// stale, so Enter waits ([`QuickOpenState::submit_on_ready`]) and
    /// [`Self::apply_file_score`] submits once that score lands.
    fn submit_file(&mut self) -> Effect {
        let Some(quick) = self.quick_open.as_mut() else {
            return Effect::None;
        };
        if quick.score_pending || quick.index == FileIndexState::Loading {
            quick.submit_on_ready = true;
            return Effect::None;
        }
        quick.submit_on_ready = false;
        let pick = match &quick.index {
            FileIndexState::Ready(index) => quick
                .selected_hit()
                .and_then(|hit| index.entries.get(hit.entry))
                .map(|entry| (index.checkout(entry).to_string(), entry.rel().to_string())),
            _ => None,
        };
        match pick {
            Some((checkout, rel)) => {
                self.close_quick_open();
                self.open_file_in_explorer(checkout, rel, None, None)
            }
            None => {
                self.status = StatusMessage::warn(NO_FILE_MATCHES);
                Effect::None
            }
        }
    }

    /// Commands-mode Enter: close, then dispatch the highlighted command, or
    /// show why it is disabled and stay open.
    ///
    /// The close puts back the theme active on open; a `Theme: …` row then
    /// applies and saves its theme through [`Action::SetTheme`].
    fn submit_command(&mut self) -> Effect {
        let Some(command) = self
            .command_palette()
            .and_then(|palette| palette.selected())
        else {
            return Effect::None;
        };
        if let Some(reason) = self.palette_disabled_reason(&command) {
            self.status = StatusMessage::warn(reason.clone());
            if let Some(palette) = self.command_palette_mut() {
                palette.shown_reason = Some(reason);
            }
            return Effect::None;
        }
        let action = match command.action {
            // Other pane is Tab: it moves away from whichever pane has focus.
            Action::FocusRight if self.focus == FocusPane::Right => Action::FocusLeft,
            ref action => action.clone(),
        };
        self.end_theme_preview();
        self.quick_open = None;
        self.dispatch_palette_action(action)
    }

    /// Run the action of a palette row picked outside its key (Quick Open,
    /// an icon popover line).
    ///
    /// Fold subtree is `zz`: toggle this row, then match its descendants.
    /// Fold row is one `z` with no chord after it, so it arms no `zz`.
    pub(crate) fn dispatch_palette_action(&mut self, action: Action) -> Effect {
        match action {
            Action::FoldToggleSubtree => {
                // Both fold actions return `Effect::None`.
                self.dispatch(Action::FoldToggle);
                self.dispatch(action)
            }
            Action::FoldToggle => {
                let effect = self.dispatch(action);
                self.z_pending_at = None;
                effect
            }
            action => self.dispatch(action),
        }
    }

    /// Accept a finished index load for generation `gen`.
    ///
    /// Dropped unless the overlay is open and `gen` is its latest index
    /// request. An index with no entries and root errors becomes
    /// [`FileIndexState::Failed`]. In files mode the current query is then
    /// scored: the returned effect is that [`Effect::ScoreFiles`].
    ///
    /// `None` when the result was dropped; `Some(follow-up)` when accepted.
    pub(crate) fn apply_file_index(&mut self, gen: u64, index: FileIndex) -> Option<Effect> {
        let quick = self
            .quick_open
            .as_mut()
            .filter(|quick| quick.index_gen == gen)?;
        quick.index = if index.entries.is_empty() && !index.errors.is_empty() {
            // No score follows; the status row shows the error instead.
            quick.submit_on_ready = false;
            FileIndexState::Failed(index.errors.join("; "))
        } else {
            FileIndexState::Ready(Arc::new(index))
        };
        Some(if quick.mode() == QuickOpenMode::Files {
            self.request_file_score()
        } else {
            Effect::None
        })
    }

    /// Accept finished hits for score generation `gen`. Dropped unless the
    /// overlay is open and `gen` is its latest score request.
    ///
    /// `None` when the result was dropped; `Some(follow-up)` when accepted.
    /// The follow-up is the waiting Enter's open of the picked file in its
    /// checkout's Explorer tab (see
    /// [`QuickOpenState::submit_on_ready`]), else [`Effect::None`].
    pub(crate) fn apply_file_score(&mut self, gen: u64, hits: Vec<FileHit>) -> Option<Effect> {
        let quick = self
            .quick_open
            .as_mut()
            .filter(|quick| quick.score_gen == gen)?;
        quick.hits = hits;
        quick.file_cursor = 0;
        quick.score_pending = false;
        Some(if quick.submit_on_ready {
            self.submit_file()
        } else {
            Effect::None
        })
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::super::tree::NodeKind;
    use super::*;
    use crate::file_index::{
        build_file_index, score_files, FileEntry, MAX_INDEX_ENTRIES, MAX_RESULTS,
    };
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};
    use crate::testutil::{init_repo, unique_dir};

    fn repo(name: &str, kind: CheckoutKind, primary: Option<&str>) -> RepoSnapshot {
        RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
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
            checkout_kind: kind,
            primary_repo: primary.map(str::to_string),
            merged_into_default: None,
            default_branch_override: None,
            default_tip_ref: None,
            local_branches: Vec::new(),
        }
    }

    /// `app` (primary) with linked `app/.worktrees/feat`, plus primary `lib`.
    fn family_state() -> AppState {
        let snapshot = build_workspace_snapshot(
            &[
                repo("app", CheckoutKind::Primary, None),
                repo("app/.worktrees/feat", CheckoutKind::Linked, Some("app")),
                repo("lib", CheckoutKind::Primary, None),
            ],
            &[],
            false,
            &[],
        );
        AppState::new(PathBuf::from("/tmp"), snapshot, true)
    }

    fn focus_row(app: &mut AppState, kind: NodeKind, repo: Option<&str>) {
        app.cursor = app
            .rows
            .iter()
            .position(|row| row.kind == kind && row.repo.as_deref() == repo)
            .unwrap_or_else(|| panic!("no {kind:?} row for {repo:?}"));
    }

    fn index_of(paths: &[&str]) -> FileIndex {
        FileIndex {
            roots: vec![IndexRoot {
                checkout: "app".into(),
                prefix: String::new(),
            }],
            entries: paths
                .iter()
                .map(|path| FileEntry {
                    root: 0,
                    display: (*path).to_string(),
                    rel_start: 0,
                })
                .collect(),
            truncated: false,
            errors: Vec::new(),
        }
    }

    fn open_files(app: &mut AppState) -> u64 {
        match app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files)) {
            Effect::LoadFileIndex { gen, .. } => gen,
            other => panic!("expected an index load, got {other:?}"),
        }
    }

    fn type_text(app: &mut AppState, text: &str) -> Effect {
        let mut last = Effect::None;
        for c in text.chars() {
            last = app.dispatch(Action::QuickOpenChar(c));
        }
        last
    }

    fn quick(app: &AppState) -> &QuickOpenState {
        app.quick_open.as_ref().expect("Quick Open open")
    }

    #[test]
    fn files_entry_opens_files_mode_and_requests_index() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let effect = app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files));
        assert_eq!(quick(&app).query, "");
        assert_eq!(quick(&app).mode(), QuickOpenMode::Files);
        assert_eq!(quick(&app).index, FileIndexState::Loading);
        match effect {
            Effect::LoadFileIndex { gen, roots } => {
                assert_eq!(gen, quick(&app).index_gen);
                assert_eq!(
                    roots,
                    vec![IndexRoot {
                        checkout: "app".into(),
                        prefix: String::new(),
                    }]
                );
            }
            other => panic!("expected an index load, got {other:?}"),
        }
        assert!(app.command_palette().is_none());
    }

    #[test]
    fn colon_opens_commands_mode_with_gt_pretyped() {
        let mut app = family_state();
        let action = press_action(&app, ':');
        assert_eq!(action, Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        let effect = app.dispatch(action);
        assert_eq!(effect, Effect::None, "no index load in commands mode");
        assert_eq!(quick(&app).query, ">");
        assert_eq!(quick(&app).mode(), QuickOpenMode::Commands);
        assert_eq!(quick(&app).index, FileIndexState::NotLoaded);
        assert!(app.command_palette().is_some());
        assert_eq!(
            app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands)),
            Effect::None
        );
        assert!(app.quick_open.is_none(), "a second toggle closes");
    }

    #[test]
    fn gt_first_char_switches_to_commands_and_backspace_returns_to_files() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        let follow = app.apply_file_index(gen, index_of(&["README.md", "src/main.rs"]));
        assert!(
            matches!(follow, Some(Effect::ScoreFiles { .. })),
            "{follow:?}"
        );

        assert_eq!(type_text(&mut app, ">"), Effect::None, "no score for `>`");
        assert_eq!(quick(&app).mode(), QuickOpenMode::Commands);
        assert_eq!(type_text(&mut app, "pull"), Effect::None);
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(palette.filter, "pull");
        assert!(palette
            .visible()
            .iter()
            .any(|command| command.title == "Pull behind"));

        let mut last = Effect::None;
        for _ in 0..5 {
            last = app.dispatch(Action::QuickOpenBackspace);
        }
        assert_eq!(quick(&app).query, "");
        assert_eq!(quick(&app).mode(), QuickOpenMode::Files);
        assert!(app.command_palette().is_none());
        match last {
            Effect::ScoreFiles { gen, query, .. } => {
                assert_eq!(query, "");
                assert_eq!(gen, quick(&app).score_gen);
            }
            other => panic!("expected a score, got {other:?}"),
        }
    }

    #[test]
    fn commands_opened_by_colon_load_the_index_once_back_in_files() {
        let mut app = family_state();
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        let effect = app.dispatch(Action::QuickOpenBackspace);
        assert!(matches!(effect, Effect::LoadFileIndex { .. }), "{effect:?}");
        assert_eq!(quick(&app).index, FileIndexState::Loading);
        assert_eq!(type_text(&mut app, "a"), Effect::None, "still loading");
    }

    #[test]
    fn commands_mode_keeps_substring_filter() {
        let mut app = family_state();
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        type_text(&mut app, "exit");
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(palette.selected().map(|c| c.title), Some("Quit"));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::Quit);

        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        type_text(&mut app, "  compare");
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(palette.filter, "compare");
        let titles: Vec<&str> = palette.visible().iter().map(|c| c.title).collect();
        for title in [
            "Diff vs default in new tab",
            "Diff vs branch in new tab…",
            "Diff vs commit in new tab…",
            "Diff commit vs parent in new tab",
        ] {
            assert!(titles.contains(&title), "{title}: {titles:?}");
        }
    }

    #[test]
    fn scope_follows_focus() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Workspace, None);
        assert_eq!(app.quick_open_scope(), QuickOpenScope::Workspace);
        assert_eq!(
            app.quick_open_roots(&QuickOpenScope::Workspace),
            vec![
                IndexRoot {
                    checkout: "app".into(),
                    prefix: "app/".into(),
                },
                IndexRoot {
                    checkout: "lib".into(),
                    prefix: "lib/".into(),
                },
            ],
            "primary checkouts only, each prefixed with its repo path"
        );

        focus_row(&mut app, NodeKind::Repo, Some("app"));
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout("app".into()),
            "a family's repo row is its primary"
        );

        let linked = "app/.worktrees/feat";
        focus_row(&mut app, NodeKind::Checkout, Some(linked));
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout(linked.into())
        );
        focus_row(&mut app, NodeKind::File, Some(linked));
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout(linked.into()),
            "a file under the linked worktree"
        );

        focus_row(&mut app, NodeKind::Workspace, None);
        app.open_compare_tab(linked.into(), "main".into(), "HEAD".into());
        assert!(app.tabs.active_compare().is_some());
        assert_eq!(
            app.quick_open_scope(),
            QuickOpenScope::Checkout(linked.into()),
            "a compare tab scopes to its checkout"
        );
    }

    /// Map a plain key press the way the event loop does.
    fn press_action(app: &AppState, c: char) -> Action {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let event = Event::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        crate::tui::app::map_event(app, &event)
    }

    /// Map a plain key press the way the event loop does, then dispatch it.
    fn press(app: &mut AppState, c: char) {
        let action = press_action(app, c);
        app.dispatch(action);
    }

    #[test]
    fn angle_keys_type_in_quick_open_and_resize_the_tree_after_esc() {
        use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
        let mut app = family_state();
        app.layout.term_cols = 140;
        let fraction = app.tree_fraction;

        press(&mut app, 'F');
        assert!(app.quick_open.is_some(), "`F` opens Quick Open");
        press(&mut app, '>');
        press(&mut app, '<');
        assert_eq!(quick(&app).query, "><", "both keys type into the query");
        assert_eq!(app.tree_fraction, fraction, "no resize while open");

        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        let action = crate::tui::app::map_event(&app, &esc);
        app.dispatch(action);
        assert!(app.quick_open.is_none(), "Esc closes Quick Open");

        press(&mut app, '<');
        assert!(app.tree_fraction < fraction, "`<` narrows the tree again");
    }

    /// Two primaries that share the leaf `web`, each with a `README.md`:
    /// workspace Quick Open lists both under their repo path and Enter
    /// opens the picked checkout's file.
    #[test]
    fn workspace_scope_keeps_same_leaf_checkouts_apart() {
        let cwd = unique_dir("ws-quick-open-same-leaf");
        let (clients, legacy) = ("clients/a/web", "legacy/web");
        init_repo(&cwd.join(clients));
        init_repo(&cwd.join(legacy));
        let snapshot = build_workspace_snapshot(
            &[
                repo(clients, CheckoutKind::Primary, None),
                repo(legacy, CheckoutKind::Primary, None),
            ],
            &[],
            false,
            &[],
        );
        let mut app = AppState::new(cwd.clone(), snapshot, true);
        focus_row(&mut app, NodeKind::Workspace, None);

        let (gen, roots) = match app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Files)) {
            Effect::LoadFileIndex { gen, roots } => (gen, roots),
            other => panic!("expected an index load, got {other:?}"),
        };
        let prefixes: Vec<&str> = roots.iter().map(|root| root.prefix.as_str()).collect();
        assert_eq!(prefixes, vec!["clients/a/web/", "legacy/web/"]);

        let index = build_file_index(&cwd, roots, MAX_INDEX_ENTRIES);
        let listed: Vec<(&str, &str, &str)> = index
            .entries
            .iter()
            .map(|entry| (entry.display.as_str(), index.checkout(entry), entry.rel()))
            .collect();
        assert_eq!(
            listed,
            vec![
                ("clients/a/web/README.md", clients, "README.md"),
                ("legacy/web/README.md", legacy, "README.md"),
            ],
            "same rel in two same-leaf checkouts: two entries"
        );

        app.apply_file_index(gen, index);
        let Effect::ScoreFiles { gen, index, query } = type_text(&mut app, "legacy readme") else {
            panic!("expected a score");
        };
        let hits = score_files(&index, &query, MAX_RESULTS);
        assert_eq!(hits.len(), 1, "{hits:?}");
        app.apply_file_score(gen, hits);
        let effect = app.dispatch(Action::QuickOpenSubmit);
        assert_eq!(
            file_load(&effect),
            Some((legacy.to_string(), "README.md".to_string())),
            "{effect:?}"
        );
        let tab = app.tabs.active_explorer().expect("Explorer tab");
        assert_eq!(tab.checkout, legacy);
        assert_eq!(opened_rel(&app), "README.md");
        let _ = std::fs::remove_dir_all(&cwd);
    }

    #[test]
    fn stale_index_and_score_results_are_dropped() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let old = open_files(&mut app);
        app.dispatch(Action::QuickOpenCancel);
        let gen = open_files(&mut app);
        assert_ne!(old, gen);
        assert_eq!(app.apply_file_index(old, index_of(&["a.rs"])), None);
        assert_eq!(quick(&app).index, FileIndexState::Loading, "old gen");

        app.dispatch(Action::QuickOpenCancel);
        assert_eq!(app.apply_file_index(gen, index_of(&["a.rs"])), None);
        assert!(
            app.quick_open.is_none(),
            "a result after close opens nothing"
        );

        let gen = open_files(&mut app);
        app.apply_file_index(gen, index_of(&["a.rs", "b.rs"]));
        let first = match type_text(&mut app, "a") {
            Effect::ScoreFiles { gen, .. } => gen,
            other => panic!("expected a score, got {other:?}"),
        };
        let second = match type_text(&mut app, "b") {
            Effect::ScoreFiles { gen, .. } => gen,
            other => panic!("expected a score, got {other:?}"),
        };
        let hit = FileHit {
            entry: 0,
            score: 1,
            indices: vec![0],
        };
        assert_eq!(app.apply_file_score(first, vec![hit.clone()]), None);
        assert!(quick(&app).hits.is_empty(), "old score gen ignored");
        assert!(quick(&app).score_pending);
        assert_eq!(
            app.apply_file_score(second, vec![hit.clone()]),
            Some(Effect::None)
        );
        assert_eq!(quick(&app).hits, vec![hit]);
        assert!(!quick(&app).score_pending);
    }

    #[test]
    fn failed_index_reports_the_root_errors() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        let mut index = index_of(&[]);
        index.errors = vec!["app: not a git repository".into()];
        assert_eq!(app.apply_file_index(gen, index), Some(Effect::None));
        assert_eq!(
            quick(&app).index,
            FileIndexState::Failed("app: not a git repository".into())
        );
        assert_eq!(quick(&app).file_status_text(), "app: not a git repository");
    }

    fn hit(entry: usize) -> FileHit {
        FileHit {
            entry,
            score: 0,
            indices: Vec::new(),
        }
    }

    fn score_gen(effect: Option<Effect>) -> u64 {
        match effect {
            Some(Effect::ScoreFiles { gen, .. }) => gen,
            other => panic!("expected a score, got {other:?}"),
        }
    }

    /// Rel of the active Explorer tab's active inner file tab.
    fn opened_rel(app: &AppState) -> &str {
        app.tabs
            .active_explorer()
            .and_then(|tab| tab.active_inner())
            .map_or("", |inner| inner.rel.as_str())
    }

    /// `(checkout, rel)` of the inner file load in `effect` (a batch with
    /// the Explorer folder listings).
    fn file_load(effect: &Effect) -> Option<(String, String)> {
        let effects = match effect {
            Effect::Batch(effects) => effects.as_slice(),
            other => std::slice::from_ref(other),
        };
        effects.iter().find_map(|effect| match effect {
            Effect::LoadExplorerPreview { repo, path, .. } => Some((repo.clone(), path.clone())),
            _ => None,
        })
    }

    #[test]
    fn files_enter_opens_the_hit_or_warns_without_one() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        let score = score_gen(app.apply_file_index(gen, index_of(&["README.md", "src/main.rs"])));
        assert_eq!(app.apply_file_score(score, Vec::new()), Some(Effect::None));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert_eq!(app.status, NO_FILE_MATCHES);
        assert!(app.quick_open.is_some(), "a miss keeps the overlay");

        let score = score_gen(Some(type_text(&mut app, "m")));
        assert!(app.status.is_empty(), "typing clears the miss warning");
        app.apply_file_score(score, vec![hit(0), hit(1)]);
        app.dispatch(Action::QuickOpenMove(1));
        let effect = app.dispatch(Action::QuickOpenSubmit);
        assert!(app.quick_open.is_none());
        assert!(app.is_explorer_tab());
        assert_eq!(opened_rel(&app), "src/main.rs");
        assert_eq!(app.focus, FocusPane::Right, "the file view takes focus");
        assert_eq!(
            file_load(&effect).map(|(_, rel)| rel).as_deref(),
            Some("src/main.rs"),
            "{effect:?}"
        );
    }

    #[test]
    fn enter_before_the_score_lands_opens_the_new_query_hit() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        let old = score_gen(app.apply_file_index(gen, index_of(&["README.md", "src/main.rs"])));
        app.apply_file_score(old, vec![hit(0)]);
        let new = score_gen(Some(type_text(&mut app, "main")));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_some(), "Enter waits for the score");
        assert!(app.status.is_empty(), "no miss warning on stale hits");
        assert_eq!(app.apply_file_score(old, vec![hit(0)]), None, "stale score");
        assert!(app.quick_open.is_some());
        let follow = app.apply_file_score(new, vec![hit(1)]).expect("accepted");
        assert_eq!(
            file_load(&follow).map(|(_, rel)| rel).as_deref(),
            Some("src/main.rs"),
            "{follow:?}"
        );
        assert!(app.quick_open.is_none());
        assert_eq!(opened_rel(&app), "src/main.rs");
        assert_eq!(
            app.focus,
            FocusPane::Right,
            "the waiting Enter focuses the file view too"
        );
    }

    #[test]
    fn enter_while_indexing_opens_after_index_and_score_land() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.status.is_empty(), "no miss warning while indexing");
        let score = score_gen(app.apply_file_index(gen, index_of(&["README.md"])));
        assert!(app.quick_open.is_some(), "still waiting for the score");
        let follow = app.apply_file_score(score, vec![hit(0)]).expect("accepted");
        assert!(file_load(&follow).is_some(), "{follow:?}");
        assert_eq!(opened_rel(&app), "README.md");
        assert_eq!(app.focus, FocusPane::Right);

        // A waiting Enter with no hit warns once the score lands.
        let gen = open_files(&mut app);
        app.dispatch(Action::QuickOpenSubmit);
        let score = score_gen(app.apply_file_index(gen, index_of(&["README.md"])));
        assert_eq!(app.apply_file_score(score, Vec::new()), Some(Effect::None));
        assert_eq!(app.status, NO_FILE_MATCHES);
        assert!(app.quick_open.is_some());
    }

    #[test]
    fn edit_or_move_after_enter_cancels_the_wait() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        let score = score_gen(app.apply_file_index(gen, index_of(&["README.md"])));
        app.dispatch(Action::QuickOpenSubmit);
        assert!(quick(&app).submit_on_ready);
        app.dispatch(Action::QuickOpenMove(1));
        assert!(!quick(&app).submit_on_ready);
        assert_eq!(
            app.apply_file_score(score, vec![hit(0)]),
            Some(Effect::None)
        );
        assert!(app.quick_open.is_some(), "the move cancelled the open");

        let score = score_gen(Some(type_text(&mut app, "r")));
        app.dispatch(Action::QuickOpenSubmit);
        let next = score_gen(Some(type_text(&mut app, "e")));
        assert!(!quick(&app).submit_on_ready, "typing cancels the wait");
        assert_eq!(app.apply_file_score(score, vec![hit(0)]), None);
        assert_eq!(app.apply_file_score(next, vec![hit(0)]), Some(Effect::None));
        assert!(app.quick_open.is_some());
        assert!(app.tabs.is_workspace());
    }

    #[test]
    fn open_clears_a_leftover_status_but_keeps_the_quit_prompt() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        app.status = StatusMessage::info("Fetched 2 repos");
        open_files(&mut app);
        assert!(app.status.is_empty(), "{}", app.status);
        assert_eq!(quick(&app).file_status_text(), "indexing…");
        app.dispatch(Action::QuickOpenCancel);

        app.status = StatusMessage::info("Press Ctrl-c again to exit");
        open_files(&mut app);
        assert_eq!(app.status, "Press Ctrl-c again to exit");
    }

    #[test]
    fn files_warning_does_not_carry_into_commands() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        let score = score_gen(app.apply_file_index(gen, index_of(&["README.md"])));
        app.apply_file_score(score, Vec::new());
        app.dispatch(Action::QuickOpenSubmit);
        assert_eq!(app.status, NO_FILE_MATCHES);
        type_text(&mut app, ">");
        assert_eq!(quick(&app).mode(), QuickOpenMode::Commands);
        assert!(app.status.is_empty(), "{}", app.status);
    }

    #[test]
    fn commands_reason_does_not_carry_into_files() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        type_text(&mut app, "next match");
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(palette.selected().map(|c| c.title), Some("Next match"));
        app.dispatch(Action::QuickOpenSubmit);
        assert!(!app.status.is_empty(), "Enter on a disabled row shows why");
        for _ in 0.."next match".len() {
            app.dispatch(Action::QuickOpenBackspace);
        }
        assert_eq!(quick(&app).mode(), QuickOpenMode::Commands);
        assert!(!app.status.is_empty(), "same mode keeps the reason");
        app.dispatch(Action::QuickOpenBackspace);
        assert_eq!(quick(&app).mode(), QuickOpenMode::Files);
        assert!(app.status.is_empty(), "{}", app.status);
        assert_eq!(quick(&app).commands.shown_reason, None);
    }

    /// Open clean `app/src/lib.rs` as an Explorer inner file tab with
    /// `count` loaded lines, the way Quick Open does.
    fn inner_file_with_lines(app: &mut AppState, count: usize) {
        let lines: Vec<String> = (1..=count).map(|n| format!("line {n}")).collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        app.open_loaded_inner_file("app", "src/lib.rs", &lines);
    }

    fn file_cursor(app: &AppState) -> usize {
        app.tabs.active_file_view().expect("inner file").cursor
    }

    /// `:42` puts `Go to line 42` on top as the landing row; Enter closes
    /// Quick Open and moves the Explorer inner file's own cursor.
    #[test]
    fn colon_digits_land_on_go_to_line_and_enter_jumps() {
        use super::super::super::command_palette::PalettePaintRow;
        let mut app = family_state();
        inner_file_with_lines(&mut app, 50);
        let tabs = app.tabs.len();
        press(&mut app, ':');
        press(&mut app, '4');
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(
            palette.paint_rows()[0],
            PalettePaintRow::GoToLine { line: 4 }
        );
        press(&mut app, '2');
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(
            palette.paint_rows()[0],
            PalettePaintRow::GoToLine { line: 42 }
        );
        assert_eq!(palette.cursor, 0, "the landing row");
        assert_eq!(palette.selected_action(), Some(Action::GoToLine(42)));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none(), "Enter closes first");
        assert_eq!(file_cursor(&app), 41, "1-based line 42");
        assert_eq!(app.tabs.len(), tabs, "no second tab");

        // Leading zeros drop; `:0` clamps to line 1, `:9999` to the last.
        for (typed, want) in [("007", 6), ("0", 0), ("9999", 49)] {
            press(&mut app, ':');
            for c in typed.chars() {
                press(&mut app, c);
            }
            app.dispatch(Action::QuickOpenSubmit);
            assert_eq!(file_cursor(&app), want, ":{typed}");
        }
    }

    /// Only digits (trimmed at both ends) show the row; catalog filtering
    /// is unchanged.
    #[test]
    fn non_digit_filters_do_not_show_go_to_line() {
        let mut app = family_state();
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        let go_to_line = |app: &AppState| app.command_palette().and_then(|p| p.go_to_line());
        assert_eq!(go_to_line(&app), None, "`>` alone");
        type_text(&mut app, " ");
        assert_eq!(go_to_line(&app), None, "a blank filter");
        type_text(&mut app, "42 ");
        assert_eq!(go_to_line(&app), Some(42), "spaces trim at both ends");
        type_text(&mut app, "2");
        assert_eq!(go_to_line(&app), None, "`4 2`");
        app.dispatch(Action::QuickOpenBackspace);
        app.dispatch(Action::QuickOpenBackspace);
        type_text(&mut app, "x");
        assert_eq!(go_to_line(&app), None, "`42x`");
        app.dispatch(Action::QuickOpenCancel);
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        type_text(&mut app, "line");
        assert_eq!(go_to_line(&app), None, "`line`");
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(palette.rows().len(), palette.visible().len());
    }

    /// On the workspace tree the row still shows, disabled: Enter keeps
    /// Quick Open open and puts the reason on the status line.
    #[test]
    fn go_to_line_on_the_tree_is_disabled_and_stays_open() {
        use super::super::super::gates::FOCUS_A_FILE_DIFF;
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let cursor = app.cursor;
        app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
        type_text(&mut app, "12");
        let palette = app.command_palette().expect("commands mode");
        assert_eq!(palette.cursor, 0, "a disabled row still lands");
        let row = palette.selected().expect("the go-to-line row");
        assert_eq!(
            app.palette_disabled_reason(&row).as_deref(),
            Some(FOCUS_A_FILE_DIFF)
        );
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_some(), "stays open");
        assert_eq!(app.status, FOCUS_A_FILE_DIFF);
        assert_eq!(
            quick(&app).commands.shown_reason.as_deref(),
            Some(FOCUS_A_FILE_DIFF)
        );
        assert_eq!(app.cursor, cursor, "the tree did not move");
    }
}

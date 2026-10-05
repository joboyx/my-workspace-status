//! Quick Open overlay: open / close, query edits, submit, and job results.
//!
//! Index loads and scores leave here as [`Effect::LoadFileIndex`] /
//! [`Effect::ScoreFiles`] and come back through [`AppState::apply_file_index`]
//! / [`AppState::apply_file_score`], which drop any result whose generation
//! is no longer the overlay's latest.

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
use super::super::tabs::checkout_leaf;
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
    /// The active compare tab's checkout; else the focused row's checkout
    /// ([`Self::focused_checkout_path`]); else (workspace row, group) the
    /// whole workspace.
    pub(crate) fn quick_open_scope(&self) -> QuickOpenScope {
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
    /// prefixed `<leaf>/`. One checkout: that checkout, no prefix.
    pub(crate) fn quick_open_roots(&self, scope: &QuickOpenScope) -> Vec<IndexRoot> {
        match scope {
            QuickOpenScope::Workspace => visible_snapshot(&self.snapshot, self.show_ignored)
                .repos
                .into_iter()
                .filter(|repo| repo.checkout_kind == CheckoutKind::Primary)
                .map(|repo| IndexRoot {
                    prefix: format!("{}/", checkout_leaf(&repo.repo)),
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
                    match quick.mode() {
                        QuickOpenMode::Commands => quick.commands.move_cursor(delta),
                        QuickOpenMode::Files => quick.move_file_cursor(delta),
                    }
                }
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
        match entry {
            QuickOpenEntry::Commands => {
                self.land_palette_cursor();
                Effect::None
            }
            QuickOpenEntry::Files => self.ensure_file_index(),
        }
    }

    /// Close the overlay with no run. A disabled-row reason that Enter put
    /// on the status line goes too, so it does not linger after the close.
    fn close_quick_open(&mut self) {
        let shown = self
            .quick_open
            .take()
            .and_then(|quick| quick.commands.shown_reason);
        if shown.is_some_and(|reason| self.status == reason) {
            self.status.clear();
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
    fn quick_open_edit(&mut self, query: String) -> Effect {
        let Some(quick) = self.quick_open.as_mut() else {
            return Effect::None;
        };
        let before = quick.mode();
        quick.query = query;
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
                Effect::None
            }
            QuickOpenMode::Files => {
                // Enter's miss warning is about the old query.
                if self.status == NO_FILE_MATCHES {
                    self.status.clear();
                }
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

    /// Put the commands cursor on the first enabled visible row (0 if none).
    ///
    /// Runs on open and on each filter change, so the HIGHLIGHT rows that
    /// paint first do not take the cursor while they are disabled. Moves
    /// still go over every row.
    fn land_palette_cursor(&mut self) {
        let Some(visible) = self.command_palette().map(|p| p.visible()) else {
            return;
        };
        let cursor = visible
            .iter()
            .position(|command| self.palette_disabled_reason(command).is_none())
            .unwrap_or(0);
        if let Some(palette) = self.command_palette_mut() {
            palette.cursor = cursor;
        }
    }

    fn submit_quick_open(&mut self) -> Effect {
        match self.quick_open.as_ref().map(|quick| quick.mode()) {
            Some(QuickOpenMode::Commands) => self.submit_command(),
            Some(QuickOpenMode::Files) => {
                self.submit_file();
                Effect::None
            }
            None => Effect::None,
        }
    }

    /// Files-mode Enter: close and name the picked file on the status line.
    fn submit_file(&mut self) {
        let display = self.quick_open.as_ref().and_then(|quick| {
            let FileIndexState::Ready(index) = &quick.index else {
                return None;
            };
            let hit = quick.selected_hit()?;
            index.entries.get(hit.entry).map(|e| e.display.clone())
        });
        match display {
            Some(display) => {
                self.close_quick_open();
                self.status = StatusMessage::info(display);
            }
            None => self.status = StatusMessage::warn(NO_FILE_MATCHES),
        }
    }

    /// Commands-mode Enter: close, then dispatch the highlighted command, or
    /// show why it is disabled and stay open.
    fn submit_command(&mut self) -> Effect {
        let Some(command) = self
            .command_palette()
            .and_then(|palette| palette.selected())
        else {
            return Effect::None;
        };
        if let Some(reason) = self.palette_disabled_reason(command) {
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
        self.quick_open = None;
        if action == Action::FoldToggleSubtree {
            // Fold subtree is `zz`: toggle this row, then match its
            // descendants. Both fold actions return `Effect::None`.
            self.dispatch(Action::FoldToggle);
        }
        self.dispatch(action)
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
    /// True when the hits were accepted.
    pub(crate) fn apply_file_score(&mut self, gen: u64, hits: Vec<FileHit>) -> bool {
        let Some(quick) = self
            .quick_open
            .as_mut()
            .filter(|quick| quick.score_gen == gen)
        else {
            return false;
        };
        quick.hits = hits;
        quick.file_cursor = 0;
        quick.score_pending = false;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::super::super::tree::NodeKind;
    use super::*;
    use crate::file_index::FileEntry;
    use crate::snapshot::{build_workspace_snapshot, FileChange, RepoSnapshot, SyncStatus};

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
    fn colon_opens_files_mode_and_requests_index() {
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
    fn ctrl_k_opens_commands_mode_with_gt_pretyped() {
        let mut app = family_state();
        let effect = app.dispatch(Action::ToggleQuickOpen(QuickOpenEntry::Commands));
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
    fn commands_opened_by_ctrl_k_load_the_index_once_back_in_files() {
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
            "primary checkouts only, each prefixed with its leaf"
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
        assert!(!app.apply_file_score(first, vec![hit.clone()]));
        assert!(quick(&app).hits.is_empty(), "old score gen ignored");
        assert!(quick(&app).score_pending);
        assert!(app.apply_file_score(second, vec![hit.clone()]));
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

    #[test]
    fn files_enter_names_the_hit_or_warns_without_one() {
        let mut app = family_state();
        focus_row(&mut app, NodeKind::Checkout, Some("app"));
        let gen = open_files(&mut app);
        app.dispatch(Action::QuickOpenSubmit);
        assert_eq!(app.status, NO_FILE_MATCHES);
        assert!(app.quick_open.is_some(), "a miss keeps the overlay");

        let Some(Effect::ScoreFiles { gen: score, .. }) =
            app.apply_file_index(gen, index_of(&["README.md", "src/main.rs"]))
        else {
            panic!("expected a score");
        };
        assert!(app.apply_file_score(
            score,
            vec![
                FileHit {
                    entry: 0,
                    score: 0,
                    indices: Vec::new(),
                },
                FileHit {
                    entry: 1,
                    score: 0,
                    indices: Vec::new(),
                },
            ],
        ));
        type_text(&mut app, "m");
        assert!(app.status.is_empty(), "typing clears the miss warning");
        app.dispatch(Action::QuickOpenMove(1));
        assert_eq!(app.dispatch(Action::QuickOpenSubmit), Effect::None);
        assert!(app.quick_open.is_none());
        assert_eq!(app.status, "src/main.rs");
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
        open_files(&mut app);
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
}

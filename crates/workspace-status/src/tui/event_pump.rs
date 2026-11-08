//! Event-loop helpers that keep the TUI from going deaf.
//!
//! Long git work must not sit on the draw thread. Overlay modes must not
//! start background fetch/watch ticks. Graph autoload must not run on
//! resize, mouse noise, or swallowed overlay keys.
//!
//! The live TTY loop (`event_loop.rs`) is one async `tokio::select!` over
//! terminal input, watch/fetch deadlines, `JoinSet` completions, flash /
//! Ctrl-C, and presentation. Every git/process effect runs on
//! `spawn_blocking`. Fetch / pull / push of independent checkouts share the
//! same cap (resolved `fetchConcurrency`, default 10). Watch collect streams
//! `discover_checkouts` then per-repo `process_repo` and applies each
//! [`crate::snapshot::RepoSnapshot`] as it arrives. Applying a result must
//! not call `git log` / `git diff` on the loop thread.
//!
//! While exclusive writes are in flight, nav / pane switch / cancel / Ctrl-C
//! still dispatch (`BusyAction::Handle`); `q` arms the press-again prompt
//! (`BusyAction::Quit`). Only actions that would start another exclusive git
//! write are drained (`Ignore`), with a `busy: <op> running` status. Fetch /
//! pull / push / FetchTick stay Handle so they enqueue on the per-gitdir
//! remote queue.
//!
//! Held nav (`h`/`j`/`k`/`l`) maps Repeat to the same move as Press. After
//! each held nav key the input thread also reads the queued copies of that
//! key and forwards them marked. The loop thread drops a marked copy unless
//! it types into a focused text field ([`keep_held_nav_backlog`]), so a
//! hold cannot flush as a burst after release, and `ll` in `full` that
//! arrives in one read types both letters. Pane loads coalesce by request
//! id and [`super::app::RightPaneTarget`]; a stale result is discarded and
//! the latest target is scheduled.

use super::action::Action;
use super::app::map_event;
use super::keys::InputMode;
use super::state::AppState;

/// What to do with an event while a git subprocess owns a worker thread.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BusyAction {
    /// Drain the event and say `busy: <op> running`. Used for actions that
    /// would start another git write.
    Ignore,
    /// Dispatch and apply (nav, pane switch, cancel, overlay typing, …).
    Handle,
    /// Relayout while the worker continues.
    Resize { cols: u16, rows: u16 },
    /// `q`: arm the press-again prompt; a second `q` or Ctrl-C leaves.
    Quit,
}

/// Classify one dispatched action during an in-flight exclusive write.
///
/// Exclusive writes (`s` / confirm-yes / stash / checkout / merge / …) are
/// [`BusyAction::Ignore`] so they cannot start a second mutating child.
/// Fetch / pull / push / FetchTick are [`BusyAction::Handle`] so they
/// enqueue on the per-gitdir remote queue. Everything else stays live:
/// move, click, wheel, Tab, Esc, help, search, overlay cancel.
pub fn classify_busy_action(action: &Action) -> BusyAction {
    match action {
        Action::Quit => BusyAction::Quit,
        Action::Resize { cols, rows } => BusyAction::Resize {
            cols: *cols,
            rows: *rows,
        },
        Action::DefaultBranch
        | Action::Stage
        | Action::Unstage
        | Action::Revert
        | Action::ConfirmYes
        | Action::ConfirmYesClean
        | Action::Edit
        | Action::SearchFilesEdit
        | Action::ExternalDiff
        | Action::WatchTick
        | Action::StashMenuEnter
        | Action::GraphStashApply
        | Action::GraphStashPop
        | Action::GraphStashDrop
        | Action::GraphCheckout
        | Action::GraphMerge
        | Action::BranchSubmit
        | Action::CreateBranchSubmit
        | Action::GraphFocusSubmit
        | Action::GraphFocusClear => BusyAction::Ignore,
        _ => BusyAction::Handle,
    }
}

/// Classify one input while a git write is in flight, including Quick Open
/// and popover Enter.
///
/// In commands mode [`Action::QuickOpenSubmit`] then dispatches the
/// highlighted catalog action, [`Action::PopoverRun`] the focused popover
/// action, and an [`Action::Release`] on a pinned popover action line runs
/// that line, so busy classification uses that inner action when
/// provided. `y` / `Y` on the push confirm pass [`Action::Push`] as the
/// inner action: a push is a remote op, not an exclusive write. Quick Open
/// nav / typing stay [`BusyAction::Handle`]. Submit with no inner action
/// (files mode, empty list, a field line) stays Handle (same as
/// [`classify_busy_action`]).
pub fn classify_busy_dispatch(action: &Action, inner_action: Option<&Action>) -> BusyAction {
    match (action, inner_action) {
        (
            Action::QuickOpenSubmit
            | Action::PopoverRun
            | Action::Release
            | Action::ConfirmYes
            | Action::ConfirmYesClean,
            Some(inner),
        ) => classify_busy_action(inner),
        _ => classify_busy_action(action),
    }
}

/// True when a queued copy of a held nav key must still dispatch: in
/// `state` it types a character into a focused text field
/// ([`Action::types_text`]).
///
/// The input thread forwards every copy of a held nav key (`h` / `j` /
/// `k` / `l`, arrows, PgUp / PgDn, Ctrl-u / Ctrl-d) that it reads behind
/// the key, marked. A marked copy that does not type is dropped, so a hold
/// in Normal mode cannot flush as a burst of moves after release. A
/// Release never types.
pub(crate) fn keep_held_nav_backlog(state: &AppState, event: &crossterm::event::Event) -> bool {
    map_event(state, event).types_text()
}

/// True when the fetch, watch, and PR poll timers must not start (confirm,
/// help, pickers, …).
///
/// A pinned icon popover (also while its `g` chord waits) does not block
/// them: it is read-only, rebuilds its content every frame, and may stay
/// open a long time, for example to watch a PR's CI.
pub fn overlay_blocks_background_ticks(mode: InputMode) -> bool {
    !matches!(
        mode,
        InputMode::Normal { .. }
            | InputMode::ZPending { .. }
            | InputMode::GPending { .. }
            | InputMode::DiffVisual
            | InputMode::Popover
            | InputMode::PopoverGPending
    )
}

/// True when this action may have moved the graph cursor onto the last row.
pub fn action_triggers_graph_autoload(action: &Action) -> bool {
    matches!(
        action,
        Action::Move(_)
            | Action::PageMove(_)
            | Action::MoveToStart
            | Action::MoveToEnd
            | Action::SearchNext
            | Action::SearchPrev
            | Action::FocusRight
            | Action::NavEnter
            | Action::ScrollWheel {
                horizontal: false,
                ..
            }
            | Action::Click { .. }
            | Action::CtrlClick { .. }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::LocalBranch;
    use crate::testutil::{app_state, key, repo};
    use crate::tui::action::QuickOpenEntry;
    use crate::tui::branches::{BranchPickerState, CreateBranchState};
    use crate::tui::comments::{CommentKey, CommentPrompt};
    use crate::tui::graph_focus::GraphFocusPickerState;
    use crate::tui::quick_open::{QuickOpenScope, QuickOpenState};
    use crate::tui::search_files::{SearchFilesState, SearchZone};
    use crate::tui::theme::BackgroundMode;
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

    /// A state over one `app` checkout with no overlay open (Normal mode).
    fn normal() -> AppState {
        app_state(&[repo("app", true)])
    }

    fn branches() -> Vec<LocalBranch> {
        ["main", "topic/a"]
            .iter()
            .map(|name| LocalBranch {
                name: (*name).into(),
                current: *name == "main",
                authordate: 0,
            })
            .collect()
    }

    fn search_files(zone: SearchZone) -> AppState {
        let mut state = normal();
        let mut dialog = SearchFilesState::new(
            QuickOpenScope::Workspace,
            String::new(),
            crate::file_search::SearchOptions::default(),
        );
        dialog.zone = zone;
        state.search_files = Some(dialog);
        state
    }

    fn quick_open(entry: QuickOpenEntry) -> AppState {
        let mut state = normal();
        state.quick_open = Some(QuickOpenState::new(entry, QuickOpenScope::Workspace));
        state
    }

    /// Builds a state with one text field focused.
    type OpenTextField = fn() -> AppState;

    /// Every `text-fields` surface in `docs/parity.md`, with a state that
    /// has its text field focused.
    const TEXT_FIELD_SURFACES: [(&str, OpenTextField); 10] = [
        ("search-prompt", || {
            let mut state = normal();
            state.search_mode = true;
            state
        }),
        ("help-search", || {
            let mut state = normal();
            state.help_open = true;
            state.help_search_query = Some(String::new());
            state
        }),
        ("quick-open-commands", || {
            quick_open(QuickOpenEntry::Commands)
        }),
        ("quick-open-files", || quick_open(QuickOpenEntry::Files)),
        ("search-files", || search_files(SearchZone::Query)),
        ("comment", || {
            let mut state = normal();
            state.comment = Some(CommentPrompt::new(
                CommentKey::Branch {
                    repo: "app".into(),
                    branch: "main".into(),
                },
                String::new(),
                "app · branch main".into(),
            ));
            state
        }),
        ("create-branch", || {
            let mut state = normal();
            state.create_branch = Some(CreateBranchState {
                repo: "app".into(),
                name: String::new(),
                commit_id: "aaa1111bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            });
            state
        }),
        ("branch-picker", || {
            let mut state = normal();
            state.branch_picker = Some(BranchPickerState::checkout("app".into(), branches()));
            state
        }),
        ("compare-picker", || {
            let mut state = normal();
            state.open_compare_picker("app".into(), branches());
            state
        }),
        ("graph-focus-picker", || {
            let mut state = normal();
            state.graph_focus_picker =
                Some(GraphFocusPickerState::new("app".into(), branches(), &[]));
            state
        }),
    ];

    /// Both pane looks `docs/parity.md` names: boxed (`"terminal"`) and
    /// flat (`"paint"`).
    const PANE_LOOKS: [(&str, BackgroundMode); 2] = [
        ("boxed", BackgroundMode::Terminal),
        ("flat", BackgroundMode::Paint),
    ];

    fn key_kind(code: KeyCode, kind: KeyEventKind) -> Event {
        Event::Key(KeyEvent::new_with_kind(code, KeyModifiers::NONE, kind))
    }

    /// A queued copy of a held nav key types in every text field, and is
    /// dropped where it would move: Normal mode, the help body, and the
    /// search-in-files results zone. A Release never types.
    #[test]
    fn held_nav_backlog_types_in_a_text_field_and_drops_elsewhere() {
        let l = key(KeyCode::Char('l'));
        let release = key_kind(KeyCode::Char('l'), KeyEventKind::Release);
        for (surface, open) in TEXT_FIELD_SURFACES {
            let state = open();
            assert!(keep_held_nav_backlog(&state, &l), "{surface}: `l` types");
            assert!(
                keep_held_nav_backlog(&state, &key_kind(KeyCode::Char('l'), KeyEventKind::Repeat)),
                "{surface}: a Repeat of `l` types"
            );
            assert!(
                !keep_held_nav_backlog(&state, &release),
                "{surface}: a Release is dropped"
            );
            assert!(
                !keep_held_nav_backlog(&state, &key(KeyCode::Down)),
                "{surface}: Down never types"
            );
        }
        let mut help = normal();
        help.help_open = true;
        let results = search_files(SearchZone::Results);
        for (case, state, event) in [
            ("normal `l`", normal(), l.clone()),
            ("normal `j`", normal(), key(KeyCode::Char('j'))),
            ("help `j`", help, key(KeyCode::Char('j'))),
            ("search-files results `j`", results, key(KeyCode::Char('j'))),
        ] {
            assert!(!keep_held_nav_backlog(&state, &event), "{case} is dropped");
        }
    }

    /// A doubled nav letter that arrives in one read (`ll` in `full`) keeps
    /// its second copy on every `text-fields` surface `docs/parity.md`
    /// marks `supported` for `doubled-nav-letter`, in both looks. The
    /// registry's surface list and this test's must match, so a new text
    /// field gets a test case, and a surface this test drives gets a
    /// registry row.
    #[test]
    fn text_fields_keep_a_doubled_nav_letter_on_every_registry_surface() {
        let mut driven: Vec<(String, String)> = TEXT_FIELD_SURFACES
            .iter()
            .flat_map(|(id, _)| {
                PANE_LOOKS
                    .iter()
                    .map(|(look, _)| (id.to_string(), look.to_string()))
            })
            .collect();
        driven.sort();
        assert_eq!(
            crate::parity_registry::supported_surface_looks("text-fields", "doubled-nav-letter"),
            driven,
            "docs/parity.md `text-fields` / `doubled-nav-letter` surfaces and looks must match this test's"
        );
        for (surface, open) in TEXT_FIELD_SURFACES {
            for (look, background) in PANE_LOOKS {
                let mut state = open();
                state.background = background;
                assert!(
                    keep_held_nav_backlog(&state, &key(KeyCode::Char('l'))),
                    "{surface} {look}: the queued second `l` must type"
                );
            }
        }
    }

    #[test]
    fn resize_and_noise_do_not_autoload() {
        assert!(!action_triggers_graph_autoload(&Action::None));
        assert!(!action_triggers_graph_autoload(&Action::Resize {
            cols: 80,
            rows: 24
        }));
        assert!(!action_triggers_graph_autoload(&Action::WatchTick));
        assert!(!action_triggers_graph_autoload(&Action::FetchTick));
        assert!(!action_triggers_graph_autoload(&Action::ConfirmYes));
        assert!(!action_triggers_graph_autoload(&Action::ConfirmNo));
        assert!(!action_triggers_graph_autoload(&Action::Drag {
            col: 1,
            row: 1
        }));
        assert!(action_triggers_graph_autoload(&Action::Move(1)));
        assert!(action_triggers_graph_autoload(&Action::Click {
            col: 1,
            row: 1
        }));
        assert!(action_triggers_graph_autoload(&Action::CtrlClick {
            col: 1,
            row: 1
        }));
        assert!(action_triggers_graph_autoload(&Action::MoveToEnd));
        assert!(action_triggers_graph_autoload(&Action::PageMove(-1)));
        assert!(action_triggers_graph_autoload(&Action::ScrollWheel {
            col: 1,
            row: 1,
            delta: 1,
            horizontal: false,
        }));
        assert!(!action_triggers_graph_autoload(&Action::ScrollWheel {
            col: 1,
            row: 1,
            delta: 1,
            horizontal: true,
        }));
    }

    #[test]
    fn overlays_block_background_ticks() {
        assert!(overlay_blocks_background_ticks(InputMode::Confirm));
        assert!(overlay_blocks_background_ticks(InputMode::Help));
        assert!(overlay_blocks_background_ticks(InputMode::HelpSearch));
        assert!(overlay_blocks_background_ticks(InputMode::SearchPrompt));
        assert!(overlay_blocks_background_ticks(InputMode::StashMenu));
        assert!(overlay_blocks_background_ticks(InputMode::BlameMenu));
        assert!(overlay_blocks_background_ticks(InputMode::BranchPicker));
        assert!(overlay_blocks_background_ticks(InputMode::GraphFocusPicker));
        assert!(overlay_blocks_background_ticks(InputMode::CreateBranch));
        assert!(overlay_blocks_background_ticks(InputMode::Comment));
        assert!(overlay_blocks_background_ticks(InputMode::CommentExport));
        assert!(overlay_blocks_background_ticks(InputMode::Doctor));
        assert!(!overlay_blocks_background_ticks(InputMode::Normal {
            search_active: false
        }));
        assert!(!overlay_blocks_background_ticks(InputMode::ZPending {
            search_active: true
        }));
        assert!(!overlay_blocks_background_ticks(InputMode::GPending {
            search_active: false
        }));
        assert!(!overlay_blocks_background_ticks(InputMode::DiffVisual));
        assert!(overlay_blocks_background_ticks(InputMode::QuickOpen));
        assert!(overlay_blocks_background_ticks(InputMode::SearchFiles {
            results: false
        }));
        assert!(overlay_blocks_background_ticks(InputMode::SearchFiles {
            results: true
        }));
        assert!(
            !overlay_blocks_background_ticks(InputMode::Popover),
            "a pinned popover lets watch and fetch ticks run"
        );
    }

    /// The blame menu's keys stay live like the stash menu's letters and
    /// Esc. Unlike stash Enter, a blame pick only reads git, so its Enter
    /// stays live too.
    #[test]
    fn busy_loop_keeps_the_blame_menu_live() {
        for action in [
            Action::BlameMenu,
            Action::BlameMenuChar('c'),
            Action::BlameMenuEnter,
            Action::BlameMenuCancel,
            Action::BlameCommitVsParent,
            Action::BlamePreviousChange,
            Action::BlameCommitVsWorktree,
            Action::BlameRevealGraph,
            Action::StashMenuChar('s'),
            Action::StashMenuCancel,
        ] {
            assert_eq!(
                classify_busy_action(&action),
                BusyAction::Handle,
                "{action:?}"
            );
        }
    }

    #[test]
    fn busy_loop_keeps_nav_live_and_drops_nested_writes() {
        assert_eq!(classify_busy_action(&Action::Quit), BusyAction::Quit);
        assert_eq!(
            classify_busy_action(&Action::Resize {
                cols: 100,
                rows: 30
            }),
            BusyAction::Resize {
                cols: 100,
                rows: 30
            }
        );
        assert_eq!(classify_busy_action(&Action::Move(1)), BusyAction::Handle);
        assert_eq!(
            classify_busy_action(&Action::PanDiff(-1)),
            BusyAction::Handle
        );
        assert_eq!(classify_busy_action(&Action::FoldClose), BusyAction::Handle);
        assert_eq!(
            classify_busy_action(&Action::FocusRight),
            BusyAction::Handle
        );
        assert_eq!(classify_busy_action(&Action::NavEsc), BusyAction::Handle);
        assert_eq!(classify_busy_action(&Action::ConfirmNo), BusyAction::Handle);
        assert_eq!(
            classify_busy_action(&Action::Click { col: 4, row: 8 }),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::CtrlClick { col: 4, row: 8 }),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::ScrollWheel {
                col: 4,
                row: 8,
                delta: -1,
                horizontal: false,
            }),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::ConfirmYes),
            BusyAction::Ignore
        );
        assert_eq!(classify_busy_action(&Action::Fetch), BusyAction::Handle);
        assert_eq!(classify_busy_action(&Action::Pull), BusyAction::Handle);
        assert_eq!(classify_busy_action(&Action::Push), BusyAction::Handle);
        assert_eq!(classify_busy_action(&Action::FetchTick), BusyAction::Handle);
        assert_eq!(classify_busy_action(&Action::Stage), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Revert), BusyAction::Ignore);
        assert_eq!(classify_busy_action(&Action::Edit), BusyAction::Ignore);
        assert_eq!(
            classify_busy_action(&Action::ExternalDiff),
            BusyAction::Ignore
        );
        assert_eq!(
            classify_busy_action(&Action::GraphCheckout),
            BusyAction::Ignore
        );
        assert_eq!(
            classify_busy_action(&Action::CommentSubmit),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::ExportComments),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_action(&Action::CopyEntityReference),
            BusyAction::Handle
        );
        // `gx` writes no git state: its lookup and browser open run on the pool.
        assert_eq!(
            classify_busy_action(&Action::OpenPullRequest),
            BusyAction::Handle
        );
        // `gc`, Copy URL, and the PR refreshes write no git state either.
        for action in [
            Action::OpenPullRequestCi,
            Action::CopyPullRequestUrl,
            Action::RefreshPullRequest,
            Action::RefreshPullRequests,
        ] {
            assert_eq!(
                classify_busy_action(&action),
                BusyAction::Handle,
                "{action:?}"
            );
        }
        // `gw` too: its remote check and browser open run on the pool.
        assert_eq!(
            classify_busy_action(&Action::ViewRemotely),
            BusyAction::Handle
        );
    }

    #[test]
    fn quick_open_submit_classifies_as_inner_action() {
        assert_eq!(
            classify_busy_dispatch(&Action::QuickOpenSubmit, Some(&Action::Pull)),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_dispatch(&Action::QuickOpenSubmit, Some(&Action::ToggleHelp)),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_dispatch(&Action::QuickOpenMove(1), None),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_dispatch(&Action::QuickOpenSubmit, None),
            BusyAction::Handle
        );
    }

    /// `y` on the push confirm stays live during a write; other confirms drain.
    #[test]
    fn push_confirm_yes_classifies_as_push() {
        assert_eq!(
            classify_busy_dispatch(&Action::ConfirmYes, Some(&Action::Push)),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_dispatch(&Action::ConfirmYesClean, Some(&Action::Push)),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_dispatch(&Action::ConfirmYes, None),
            BusyAction::Ignore
        );
        assert_eq!(
            classify_busy_dispatch(&Action::ConfirmYesClean, None),
            BusyAction::Ignore
        );
    }

    /// The search-in-files dialog only reads files, so every key in it
    /// stays live while a git write runs, Enter (open the hit in the
    /// Explorer) too. `e` launches the editor, so it waits like
    /// [`Action::Edit`].
    #[test]
    fn search_files_actions_stay_live_while_busy() {
        use super::super::action::SearchFilesOption;
        for action in [
            Action::ToggleSearchFiles,
            Action::SearchFilesChar('a'),
            Action::SearchFilesBackspace,
            Action::SearchFilesMove(1),
            Action::SearchFilesPage(1),
            Action::SearchFilesToggleScope,
            Action::SearchFilesToggleOption(SearchFilesOption::Regex),
            Action::SearchFilesEscape,
            Action::SearchFilesSubmit,
        ] {
            assert_eq!(
                classify_busy_dispatch(&action, None),
                BusyAction::Handle,
                "{action:?}"
            );
        }
        assert_eq!(
            classify_busy_dispatch(&Action::SearchFilesEdit, None),
            classify_busy_dispatch(&Action::Edit, None),
        );
        assert_eq!(
            classify_busy_dispatch(&Action::SearchFilesEdit, None),
            BusyAction::Ignore
        );
    }

    #[test]
    fn popover_enter_classifies_as_the_focused_action() {
        assert_eq!(
            classify_busy_dispatch(&Action::PopoverRun, Some(&Action::Pull)),
            BusyAction::Handle
        );
        assert_eq!(
            classify_busy_dispatch(&Action::PopoverRun, Some(&Action::DefaultBranch)),
            BusyAction::Ignore
        );
        assert_eq!(
            classify_busy_dispatch(&Action::PopoverRun, None),
            BusyAction::Handle,
            "a field line runs nothing"
        );
        assert_eq!(
            classify_busy_dispatch(&Action::Release, Some(&Action::DefaultBranch)),
            BusyAction::Ignore,
            "a click on an exclusive popover line is drained like its key"
        );
        assert_eq!(
            classify_busy_dispatch(&Action::Release, None),
            BusyAction::Handle,
            "a release that runs nothing stays live"
        );
        for action in [
            Action::PopoverOpenFocused,
            Action::PopoverMove(1),
            Action::PopoverCopyLine,
            Action::PopoverClose,
        ] {
            assert_eq!(classify_busy_dispatch(&action, None), BusyAction::Handle);
        }
    }

    /// Fails CI if the live TTY path grows a nested pump or a sync pane /
    /// snapshot / write git call on the loop thread.
    #[test]
    fn tty_event_loop_must_not_call_sync_pane_git() {
        let app = include_str!("app.rs");
        let loop_src = include_str!("event_loop.rs");
        let effect = include_str!("effect.rs");
        let sched = include_str!("scheduler.rs");

        for pumped in [
            "fn run_loop(",
            "fn run_work_pumped",
            "fn pump_busy_events",
            "fn run_capped_pumped",
            "fn run_bulk_remote_pumped",
            "fn load_right_pumped",
            "fn apply_effect_inner",
            "fn reload_snapshot_pumped",
            "fn reload_repo_pumped",
            "fn load_commit_files_pumped",
            "fn load_commit_diff_pumped",
            "fn run_write_then_refresh_pumped",
        ] {
            assert!(
                !app.contains(pumped) && !loop_src.contains(pumped),
                "{pumped} must not return to the live TTY path"
            );
        }

        assert!(
            !app.contains("load_right(") && !loop_src.contains("load_right("),
            "TTY must not call load_right(; that puts git log / git diff on the loop thread"
        );
        assert!(
            app.contains("load_right_headless("),
            "unit tests may still call load_right_headless for pane compute"
        );
        assert_eq!(
            app.matches("reload_snapshot(state, opts").count(),
            0,
            "sync reload_snapshot must not return"
        );
        assert_eq!(
            app.matches("reload_repo(state, opts").count(),
            0,
            "sync reload_repo must not return"
        );
        assert_eq!(
            app.matches("load_commit_files(state, opts").count(),
            0,
            "sync load_commit_files must not return"
        );
        assert_eq!(
            app.matches("load_commit_diff(state, opts").count(),
            0,
            "sync load_commit_diff must not return"
        );

        assert!(
            loop_src.contains("spawn_blocking"),
            "TTY git/process work must use spawn_blocking"
        );
        assert!(
            loop_src.contains("JoinSet"),
            "TTY must join workers on a JoinSet"
        );
        assert!(
            loop_src.contains("interp.apply("),
            "live JoinSet completions must call Interpreter::apply"
        );
        let overlay_gate = loop_src
            .find("keep_overlay_if_gitdir_busy")
            .expect("Confirm Yes must keep the overlay when that gitdir is occupied");
        let dispatch = loop_src
            .find("let effect = ctx.state.dispatch(action)")
            .expect("live loop must dispatch after the overlay occupy gate");
        assert!(
            overlay_gate < dispatch,
            "keep_overlay_if_gitdir_busy must run before dispatch takes the overlay"
        );
        assert!(
            sched.contains("fn accept_repo_result"),
            "scheduler must gate stale collection generations"
        );
        assert!(
            !loop_src.contains(concat!("collect_full_", "snapshot("))
                && !effect.contains(concat!("collect_full_", "snapshot(")),
            "live watch/refresh must stream process_repo, not wait on collect_full_snapshot"
        );

        for banned in [
            "if let Err(err) = stage_file",
            "if let Err(err) = unstage_file",
            "apply_cached_patch(",
            "apply_worktree_patch_reverse(",
            "if let Err(err) = revert_tracked_file",
            "if let Err(err) = remove_untracked_file",
            "match stash_push(",
            "match stash_apply(",
            "match stash_pop(",
            "match stash_drop(",
            "match create_branch_checkout(",
            "match create_branch_at(",
            "match remove_worktree(",
            "if run_checkout_branch(state",
            "if run_merge_into_head(state",
            "blob_bytes(",
            "prepare_worktree_diff(",
            "prepare_rev_diff(",
            "prepare_rev_diff_paths(",
            "blame_line(",
            "previous_line_change(",
        ] {
            assert!(
                !app.contains(banned) && !loop_src.contains(banned),
                "{banned} must not run on the TTY loop thread"
            );
        }
        let render = include_str!("render.rs");
        let state_src = include_str!("state/mod.rs");
        let popover_src = include_str!("popover.rs");
        assert!(
            !render.contains("load_commit(")
                && !state_src.contains("load_commit(")
                && !popover_src.contains("load_commit(")
                && !loop_src.contains("load_commit("),
            "the commit-footer commit read must not run on paint, state, or the loop thread"
        );
        assert!(
            app.contains("graph_load::load_commit(dir") && effect.contains("compute_compare_range("),
            "the footer commit read runs in compute_compare_range, on spawn_blocking via Interpreter"
        );
        // A status result reads no worktree file and a store change writes
        // no store file on the loop thread: the status worker reads the
        // file facts, a save is a worker job, and quit finishes the saves.
        let watch_src = include_str!("watch.rs");
        let explorer_state = include_str!("state/explorer_tab.rs");
        for banned in [
            "fingerprint_file_change(",
            "collect_current_fingerprints(",
            "save_viewed_store(",
            "save_comment_store(",
            "persist_with_lock(",
        ] {
            for (name, src) in [
                ("state/mod.rs", state_src),
                ("state/explorer_tab.rs", explorer_state),
                ("app.rs", app),
                ("effect.rs", effect),
                ("event_loop.rs", loop_src),
            ] {
                assert!(
                    !src.contains(banned),
                    "{banned} must not run on the TTY loop thread ({name})"
                );
            }
        }
        // The facts read and the save job are blocking wrappers: the
        // reducer files must not call them either.
        for (name, src) in [
            ("state/mod.rs", state_src),
            ("state/explorer_tab.rs", explorer_state),
            ("app.rs", app),
            ("event_loop.rs", loop_src),
        ] {
            assert!(
                !src.contains("collect_repo_file_facts("),
                "collect_repo_file_facts( reads worktree files; it runs on the status worker, not the loop thread ({name})"
            );
        }
        for (name, src) in [
            ("state/explorer_tab.rs", explorer_state),
            ("app.rs", app),
            ("effect.rs", effect),
            ("event_loop.rs", loop_src),
        ] {
            assert!(
                !src.contains("FileFactsCache::from_disk("),
                "FileFactsCache::from_disk( reads every dirty file; only the launch may call it ({name})"
            );
        }
        assert_eq!(
            state_src.matches("FileFactsCache::from_disk(").count(),
            2,
            "state/mod.rs may read the file facts from disk only at launch and in the #[cfg(test)] helper"
        );
        assert_eq!(
            effect.matches("job.run()").count(),
            1,
            "effect.rs runs a store-save job only inside its spawned worker closure"
        );
        assert!(
            effect.contains("JobOutcome::StoreSaved { saved: job.run() }"),
            "the store-save job runs inside the spawned worker closure"
        );
        assert_eq!(
            state_src.matches("job.run()").count(),
            1,
            "state/mod.rs runs a store-save job only in the #[cfg(test)] flush helper"
        );
        assert!(
            !state_src.contains("file_disk_token("),
            "tree signatures and viewed marks take disk tokens from the file facts"
        );
        let row_signature = watch_src
            .split("pub fn row_signature(")
            .nth(1)
            .and_then(|rest| rest.split("pub fn tree_signatures(").next())
            .expect("row_signature in watch.rs");
        assert!(
            !row_signature.contains("file_disk_token(") && !row_signature.contains("fs::"),
            "row_signature must read the file facts, not stat the file"
        );
        assert!(
            effect.contains("collect_repo_file_facts(") && effect.contains("UserTag::StoreSave"),
            "file facts and store saves run on spawn_blocking via Interpreter"
        );
        let run_tail = loop_src
            .split("let result = drive(&mut ctx).await;")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("run drives the loop, then quits");
        assert!(
            run_tail.contains("flush_store_saves("),
            "quit must finish the viewed and comment saves before the runtime shuts down"
        );
        let blame_state = include_str!("state/line_blame.rs");
        assert!(
            !render.contains("blame_line(") && !blame_state.contains("blame_line("),
            "line blame paint and state must read the cache, never run git"
        );
        assert!(
            !blame_state.contains("previous_line_change(")
                && effect.contains("previous_line_change(&dir"),
            "blame previous-change git runs on spawn_blocking via Interpreter"
        );
        assert!(
            effect.contains("blame_line(") && effect.contains("UserTag::LineBlame"),
            "focused-line blame must run on spawn_blocking via Interpreter"
        );
        assert!(
            !app.contains("load_commit_card(") && !loop_src.contains("load_commit_card("),
            "commit popover card lookup must not run on the TTY loop thread"
        );
        assert!(
            effect.contains("load_commit_card(") && effect.contains("UserTag::CommitCard"),
            "commit popover card lookup must run on spawn_blocking via Interpreter"
        );
        for banned in ["commit_times_since(", "load_activity("] {
            assert!(
                !app.contains(banned) && !loop_src.contains(banned) && !render.contains(banned),
                "{banned} (sparkline git log) must not run on the TTY loop thread"
            );
        }
        assert!(
            effect.contains("load_activity(&cwd") && effect.contains("UserTag::Activity"),
            "sparkline counts must run on spawn_blocking via Interpreter"
        );
        assert!(
            effect.contains("prepare_worktree_diff(")
                && effect.contains("prepare_rev_diff_paths(")
                && effect.contains("UserTag::DiffPrepare"),
            "E blob/temp prepare must run on spawn_blocking via Interpreter"
        );

        // The clipboard arm writes OSC 52 only; the host tool (a process
        // that can hang) runs on a worker with a deadline.
        let copy_arm = effect
            .split("Effect::CopyClipboard { text, announce } =>")
            .nth(1)
            .and_then(|rest| rest.split("\n            }\n").next())
            .expect("the CopyClipboard arm of schedule_effect");
        for banned in [
            "copy_to_clipboard(",
            "copy_with_host_tool(",
            "pipe_to(",
            "clipboard_tool)(",
        ] {
            assert!(
                !copy_arm.contains(banned),
                "{banned} must not run in the CopyClipboard arm, on the TTY loop thread"
            );
        }
        assert!(
            effect.contains("UserTag::Clipboard =>") && copy_arm.contains("clipboard_job.offer("),
            "the host clipboard tool must run on spawn_blocking via Interpreter"
        );
        assert!(
            !app.contains("copy_with_host_tool(") && !loop_src.contains("copy_with_host_tool("),
            "the host clipboard tool must not run on the TTY loop thread"
        );

        let comments_target = include_str!("comments/target.rs");
        for banned in ["list_local_branches", "rev_parse", "exec_git"] {
            assert!(
                !comments_target.contains(banned),
                "comment GC / targeting must not run git ({banned}); use snapshot.local_branches"
            );
        }
    }

    /// Live loop must read and enable mouse through `tui/tty.rs`. Direct
    /// `event::read` / `EnableMouseCapture` would skip the shared sequence
    /// and SGR contract.
    #[test]
    fn tty_event_loop_must_use_shared_mouse_tty() {
        let app = include_str!("app.rs");
        let loop_src = include_str!("event_loop.rs");
        for (name, src) in [("app.rs", app), ("event_loop.rs", loop_src)] {
            assert!(
                !src.contains("event::read("),
                "{name} must call read_event(); event::read skips the shared mouse module"
            );
            assert!(
                !src.contains("event::poll("),
                "{name} must call poll_event(); event::poll skips the shared mouse module"
            );
            assert!(
                !src.contains("EnableMouseCapture"),
                "{name} must call enable_mouse(); EnableMouseCapture skips the shared sequence"
            );
        }
        assert!(
            loop_src.contains("read_event_origin("),
            "TTY input thread must read via tty::read_event_origin"
        );
        assert!(
            loop_src.contains("poll_event("),
            "TTY input thread must poll via tty::poll_event"
        );
        assert!(
            !loop_src.contains("poll_event(Duration::from_millis(16)).unwrap_or(false)"),
            "poll error (TTY hangup) must stop the input thread"
        );
        assert!(
            app.contains("enable_mouse("),
            "TTY must enable mouse via tty::enable_mouse"
        );
        assert!(
            app.contains("disable_mouse("),
            "TTY must disable mouse via tty::disable_mouse"
        );
    }
}

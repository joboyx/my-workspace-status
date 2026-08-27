//! Crossterm event loop. First paint happens before any network fetch.

use std::collections::BTreeSet;
use std::io::{self, stdout, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, KeyEvent, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size as terminal_size, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};
use workspace_status_graph::LOADING_OLDER;

use crate::actions::switch_repo_to_default_branch;
use crate::config::WorkspaceStatusConfig;
use crate::discovery::{collect_snapshots, process_repo, RepoCheckoutMeta};
use crate::git::{
    checkout_branch, create_branch_at, create_branch_checkout, diff_commit_file_ctx,
    diff_stash_file_ctx, exec_git_checked, fast_forward_to_remote_ref, git_diff_args,
    latest_stash_ref, list_commit_name_status, list_local_branches, list_stash_name_status,
    list_worktree_name_status, merge_into_head, pull_quiet_detailed, push_quiet,
    remove_untracked_file, remove_worktree, repo_has_local_changes, rev_parse_quiet,
    revert_tracked_file, stage_file, stash_apply, stash_drop, stash_pop, stash_push, unstage_file,
    MergeIntoHeadResult, NameStatus,
};
use crate::parallel::{env_fetch_concurrency, CappedBatch};
use crate::snapshot::{
    build_workspace_snapshot, repo_snapshots_from_workspace, CheckoutKind, FileChange,
    WorkspaceSnapshot,
};

use super::action::{Action, Effect};
use super::branches::{
    checkout_name_for_ref, is_origin_remote_ref, plan_graph_checkout, GraphCheckoutPlan,
    DIRTY_WORKTREE_STATUS,
};
use super::diff::{load_file_diff, DiffContent};
use super::drill::{CommitFile, CommitFileSource, DrillView};
use super::editor::{editor_command, is_detached_editor, resolve_editor};
use super::event_pump::{
    action_triggers_graph_autoload, classify_busy_action, overlay_blocks_background_ticks,
    BusyAction,
};
use super::fetch::fetch_interval_ms;
use super::graph_load::{
    autoload_limit, autoload_skip, load_graph_model, load_graph_model_window, merge_autoload,
    refresh_graph_limit, should_autoload, GraphIdentity, ShouldAutoload,
};
use super::keys::{event_to_action_with, held_nav_key, is_held_nav_backlog, NAV_REPEAT_POLL_MS};
use super::ops::{format_completed_op, format_running_op, RunningOp};
use super::render::draw;
use super::state::AppState;
use super::watch::{
    checkout_watch_identities, watch_interval_ms, watch_needs_pane_reload, watch_remain_ms,
    watch_tick_due, FLASH_TICK_MS,
};

/// Options for the interactive TUI.
pub struct TuiOpts {
    pub cwd: std::path::PathBuf,
    pub snapshot: WorkspaceSnapshot,
    pub config: WorkspaceStatusConfig,
    pub start_fetch: bool,
}

/// Open the alternate screen and run until quit.
pub fn run_tui(opts: TuiOpts) -> Result<(), u8> {
    let ascii = std::env::var("WS_STATUS_GLYPHS")
        .map(|v| v == "ascii")
        .unwrap_or(false);
    let mut state = AppState::new(opts.cwd.clone(), opts.snapshot.clone(), ascii);
    enable_raw_mode().map_err(|_| 1u8)?;
    let mut out = stdout();
    if execute!(out, EnterAlternateScreen).is_err() || enable_mouse_capture(&mut out).is_err() {
        let _ = disable_raw_mode();
        return Err(1);
    }
    push_keyboard_enhancement();
    let backend = CrosstermBackend::new(out);
    let mut terminal = match Terminal::with_options(
        backend,
        TerminalOptions {
            viewport: Viewport::Fixed(terminal_size_rect()),
        },
    ) {
        Ok(t) => t,
        Err(_) => {
            restore_terminal();
            return Err(1);
        }
    };
    let result = run_loop(&mut terminal, &mut state, &opts);
    restore_terminal();
    let _ = terminal.show_cursor();
    result
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut end = stdout();
    let _ = execute!(
        end,
        PopKeyboardEnhancementFlags,
        DisableMouseCapture,
        LeaveAlternateScreen
    );
}

fn keyboard_enhancement_flags() -> KeyboardEnhancementFlags {
    KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
        | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        | KeyboardEnhancementFlags::REPORT_ALL_KEYS_AS_ESCAPE_CODES
}

/// Ask the terminal for press / repeat / release on letter keys (`h`/`j`/`k`/`l`).
///
/// Terminals that do not support the protocol ignore the CSI. Failures stay
/// quiet; traditional byte-repeat still maps to Press.
fn push_keyboard_enhancement() {
    let mut out = stdout();
    let _ = execute!(
        out,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    );
}

fn terminal_size_rect() -> Rect {
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    Rect::new(0, 0, cols.max(1), rows.max(1))
}

fn map_event(state: &AppState, event: &crossterm::event::Event) -> Action {
    event_to_action_with(
        event,
        state.input_mode(),
        state.right_is_diff(),
        matches!(state.focus, super::state::FocusPane::Right),
        state.graph_stash_focused(),
        state.graph_commit_focused(),
        state.hl_folds(),
    )
}

fn apply_terminal_resize(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cols: u16,
    rows: u16,
) -> Result<(), u8> {
    terminal
        .resize(Rect::new(0, 0, cols.max(1), rows.max(1)))
        .map_err(|_| 1u8)
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
) -> Result<(), u8> {
    if apply_effect(state, Effect::LoadRightPane, opts, terminal, &Action::None) {
        return Ok(());
    }
    terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
    if opts.start_fetch {
        let effect = state.dispatch(Action::Fetch);
        if apply_effect(state, effect, opts, terminal, &Action::Fetch) {
            return Ok(());
        }
        terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
    }

    let watch_ms = watch_interval_ms(std::env::var("WS_STATUS_WATCH_MS").ok().as_deref());
    let fetch_ms = fetch_interval_ms(std::env::var("WS_STATUS_FETCH_MS").ok().as_deref());
    let mut last_watch = Instant::now();
    let mut last_fetch = Instant::now();
    let mut nav_repeat_poll = false;
    loop {
        let remain_watch = watch_remain_ms(last_watch, Instant::now(), watch_ms);
        let remain_fetch = if fetch_ms == 0 {
            u64::MAX
        } else {
            fetch_ms.saturating_sub(last_fetch.elapsed().as_millis() as u64)
        };
        let remain_ctrl_c = state
            .ctrl_c_remaining_ms(Instant::now())
            .unwrap_or(u64::MAX);
        let idle_ms = if nav_repeat_poll {
            NAV_REPEAT_POLL_MS
        } else if state.has_active_flashes() {
            FLASH_TICK_MS
        } else {
            200
        };
        let timeout = Duration::from_millis(
            remain_watch
                .min(remain_fetch)
                .min(remain_ctrl_c)
                .min(idle_ms)
                .max(10),
        );
        if event::poll(timeout).unwrap_or(false) {
            let mut pending: Option<crossterm::event::Event> = None;
            loop {
                let event = match pending.take() {
                    Some(event) => event,
                    None => {
                        let Ok(event) = event::read() else {
                            break;
                        };
                        event
                    }
                };
                if held_nav_key(&event).is_some() {
                    nav_repeat_poll = true;
                }
                let action = map_event(state, &event);
                let resized = matches!(action, Action::Resize { .. });
                if let Action::Resize { cols, rows } = &action {
                    apply_terminal_resize(terminal, *cols, *rows)?;
                }
                let mouse_before = state.mouse_enabled;
                let action_for_load = action.clone();
                let effect = state.dispatch(action);
                if state.mouse_enabled != mouse_before {
                    sync_mouse_capture(state.mouse_enabled);
                }
                if let Some(held) = held_nav_key(&event) {
                    pending = discard_held_nav_backlog(held);
                }
                if matches!(effect, Effect::Quit) {
                    return Ok(());
                }
                if apply_effect(state, effect, opts, terminal, &action_for_load) {
                    return Ok(());
                }
                terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
                if resized {
                    state.sync_graph_scroll();
                }
                if pending.is_none() && !event::poll(Duration::from_millis(0)).unwrap_or(false) {
                    break;
                }
            }
            continue;
        }
        nav_repeat_poll = false;
        let _ = state.expire_ctrl_c_prompt(Instant::now());
        if !overlay_blocks_background_ticks(state.input_mode()) {
            if watch_tick_due(last_watch, Instant::now(), watch_ms) {
                last_watch = Instant::now();
                let effect = state.dispatch(Action::WatchTick);
                if apply_effect(state, effect, opts, terminal, &Action::WatchTick) {
                    return Ok(());
                }
            }
            if fetch_ms > 0 && last_fetch.elapsed().as_millis() as u64 >= fetch_ms {
                let effect = state.dispatch(Action::FetchTick);
                if apply_effect(state, effect, opts, terminal, &Action::FetchTick) {
                    return Ok(());
                }
                last_fetch = Instant::now();
            }
        }
        terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
    }
}

fn paint_running_op(
    state: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    kind: RunningOp,
    done: usize,
    total: usize,
) {
    state.status = format_running_op(kind, done, total);
    let _ = terminal.draw(|frame| draw(frame, state));
}

enum WorkPump<T> {
    Done(T),
    Quit,
}

/// Drain crossterm while a worker or capped batch owns git children.
///
/// Nav / pane switch / cancel dispatch ([`BusyAction::Handle`]). Actions that
/// would start another git write are drained ([`BusyAction::Ignore`]). Sets
/// `quit` when the user asked to leave; the caller still waits for in-flight
/// git to exit. Held nav drops queued copies of the same key. Nested
/// [`Effect::LoadRightPane`] is skipped so a hold cannot start pane git per
/// repeat; [`load_right_pumped`] reloads if the target moved.
fn pump_busy_events(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
    quit: &mut bool,
) {
    if event::poll(Duration::from_millis(50)).unwrap_or(false) {
        let mut pending: Option<crossterm::event::Event> = None;
        loop {
            let event = match pending.take() {
                Some(event) => event,
                None => {
                    let Ok(event) = event::read() else {
                        break;
                    };
                    event
                }
            };
            let action = map_event(state, &event);
            match classify_busy_action(&action) {
                BusyAction::Quit => *quit = true,
                BusyAction::Resize { cols, rows } => {
                    let _ = apply_terminal_resize(terminal, cols, rows);
                    let _ = state.dispatch(Action::Resize { cols, rows });
                    let _ = terminal.draw(|frame| draw(frame, state));
                }
                BusyAction::Handle => {
                    let mouse_before = state.mouse_enabled;
                    let action_for_load = action.clone();
                    let effect = state.dispatch(action);
                    if state.mouse_enabled != mouse_before {
                        sync_mouse_capture(state.mouse_enabled);
                    }
                    if let Some(held) = held_nav_key(&event) {
                        pending = discard_held_nav_backlog(held);
                    }
                    if matches!(effect, Effect::Quit) {
                        *quit = true;
                    } else if matches!(effect, Effect::LoadRightPane) {
                        let _ = terminal.draw(|frame| draw(frame, state));
                    } else if apply_effect(state, effect, opts, terminal, &action_for_load) {
                        *quit = true;
                    } else {
                        let _ = terminal.draw(|frame| draw(frame, state));
                    }
                }
                BusyAction::Ignore => {}
            }
            if pending.is_none() && !event::poll(Duration::from_millis(0)).unwrap_or(false) {
                break;
            }
        }
    }
    let _ = terminal.draw(|frame| draw(frame, state));
}

/// Run `work` on a helper thread while this thread still polls crossterm.
///
/// Used for watch snapshot collect, follow-up pane git ([`load_right_pumped`]),
/// graph autoload, commit files/diff, TTY snapshot reloads, and local git
/// writes. Independent per-repo fetch / pull / push use [`run_capped_pumped`].
/// Nav / pane switch / cancel dispatch while the worker runs
/// ([`BusyAction::Handle`]). Actions that would start another git write are
/// drained ([`BusyAction::Ignore`]) so they cannot nest a second mutating
/// child or flush after the join.
fn run_work_pumped<T: Send + 'static>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
    work: impl FnOnce() -> T + Send + 'static,
) -> WorkPump<T> {
    let (tx, rx) = mpsc::sync_channel(1);
    let handle = thread::spawn(move || {
        let _ = tx.send(work());
    });
    let mut quit = false;
    loop {
        match rx.try_recv() {
            Ok(value) => {
                let _ = handle.join();
                return if quit {
                    WorkPump::Quit
                } else {
                    WorkPump::Done(value)
                };
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let _ = handle.join();
                return WorkPump::Quit;
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
        pump_busy_events(terminal, state, opts, &mut quit);
    }
}

/// Run independent per-repo work with [`env_fetch_concurrency`] workers.
///
/// Completions arrive as they finish (not as they start). `q` cancels queued
/// items; in-flight git still runs to completion, same as a single
/// [`run_work_pumped`] child. Nav / resize stay live via [`pump_busy_events`].
fn run_capped_pumped<I, T, F>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
    items: Vec<I>,
    work: F,
    mut on_progress: impl FnMut(
        &mut AppState,
        &mut Terminal<CrosstermBackend<io::Stdout>>,
        usize,
        usize,
    ),
) -> WorkPump<Vec<Option<T>>>
where
    I: Send + 'static,
    T: Send + 'static,
    F: Fn(I) -> T + Send + Sync + 'static,
{
    let total = items.len();
    if total == 0 {
        return WorkPump::Done(Vec::new());
    }
    let mut batch = CappedBatch::start(items, env_fetch_concurrency(), work);
    let mut quit = false;
    loop {
        while let Some(done) = batch.try_recv() {
            on_progress(state, terminal, done, total);
        }
        if batch.is_finished() {
            let results = batch.join();
            return if quit {
                WorkPump::Quit
            } else {
                WorkPump::Done(results)
            };
        }
        if quit {
            batch.cancel();
        }
        pump_busy_events(terminal, state, opts, &mut quit);
    }
}

/// Fetch / pull / push many checkouts in parallel. Exclusive writes stay serial.
///
/// Progress is `Verb n/N` as completions land. After the batch: `Verbed N
/// repos` / `(N failed)` — never names. Returns true when the user quit.
fn run_bulk_remote_pumped<T, F, S>(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repos: Vec<String>,
    kind: RunningOp,
    work: F,
    succeeded: S,
) -> bool
where
    T: Send + 'static,
    F: Fn(PathBuf) -> T + Send + Sync + 'static,
    S: Fn(&T) -> bool,
{
    let total = repos.len();
    let cwd = opts.cwd.clone();
    paint_running_op(state, terminal, kind, 0, total);
    match run_capped_pumped(
        terminal,
        state,
        opts,
        repos.clone(),
        move |repo| work(cwd.join(repo)),
        |state, terminal, done, total| {
            paint_running_op(state, terminal, kind, done, total);
        },
    ) {
        WorkPump::Quit => true,
        WorkPump::Done(results) => {
            let mut ok = 0usize;
            let mut failed = 0usize;
            for slot in results {
                match slot {
                    Some(value) if succeeded(&value) => ok += 1,
                    _ => failed += 1,
                }
            }
            if reload_snapshot_pumped(state, opts, terminal) {
                return true;
            }
            state.stamp_checkout_flashes(&repos);
            state.status = format_completed_op(kind, ok, failed);
            load_right_pumped(state, opts, terminal)
        }
    }
}

fn reload_snapshot_pumped(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> bool {
    let cwd = opts.cwd.clone();
    let config = opts.config.clone();
    let filter = state.snapshot.filter_repos.clone();
    let show_ignored = state.show_ignored;
    match run_work_pumped(terminal, state, opts, move || {
        collect_full_snapshot(&cwd, &config, &filter, show_ignored, false)
    }) {
        WorkPump::Quit => true,
        WorkPump::Done(snapshot) => {
            state.apply_snapshot(snapshot);
            false
        }
    }
}

/// Inputs for right-pane git (`git log` / `git diff` / commit files / commit
/// diff). Depth 0 loads a graph or worktree file diff. Depth 1 loads files for
/// the focused graph row. Depth 2 loads that file's commit diff.
///
/// Must stay `Send` so [`load_right_pumped`] can run [`Self::compute`] on a
/// worker. #60 moved fetch/pull/watch *children* off the draw thread; applying
/// that result still called this git on the event loop, so crossterm queued
/// keys and flushed them in a burst.
#[derive(Clone, Debug)]
struct RightPaneRequest {
    cwd: std::path::PathBuf,
    snapshot: WorkspaceSnapshot,
    show_ignored: bool,
    in_graph: bool,
    focused_file: Option<(String, FileChange)>,
    file_diff_context: Option<u32>,
    focused_graph_repo: Option<String>,
    same_repo: bool,
    graph_limit: usize,
    /// Depth 1: commit / stash / worktree files for the focused graph row.
    follow_files: Option<(String, CommitFileSource)>,
    /// Depth 2: commit-scoped diff for the focused commit-file row.
    follow_diff: Option<FollowDiffRequest>,
}

/// Identity of the row a [`RightPaneRequest`] would load. Used to detect
/// cursor movement while pane git is in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RightPaneTarget {
    in_graph: bool,
    file: Option<(String, String)>,
    graph_repo: Option<String>,
    follow_files: Option<(String, CommitFileSource)>,
    follow_diff: Option<(String, CommitFileSource, String)>,
}

/// Inputs for a depth-2 commit-file diff loaded through [`RightPaneRequest`].
#[derive(Clone, Debug)]
struct FollowDiffRequest {
    repo: String,
    source: CommitFileSource,
    path: String,
    context: Option<u32>,
    files: Vec<CommitFile>,
    file_cursor: usize,
    focused_file: Option<(String, FileChange)>,
}

/// Git result for [`RightPaneRequest::compute`]. Applied on the draw thread
/// with no further subprocesses.
#[derive(Debug)]
enum RightPaneLoad {
    Diff {
        repo: String,
        path: String,
        content: DiffContent,
    },
    Graph {
        model: workspace_status_graph::GraphModel,
        identity: GraphIdentity,
        files: Option<(String, CommitFileSource, Vec<NameStatus>)>,
    },
    Clear,
    CommitFiles {
        repo: String,
        source: CommitFileSource,
        files: Vec<NameStatus>,
    },
    CommitDiff {
        repo: String,
        source: CommitFileSource,
        files: Vec<CommitFile>,
        file_cursor: usize,
        path: String,
        content: DiffContent,
    },
    None,
}

impl RightPaneRequest {
    fn from_state(state: &AppState) -> Self {
        let focused_file = state.focused_file();
        let focused_graph_repo = state.focused_graph_repo();
        let same_repo = focused_graph_repo.as_ref().is_some_and(|repo| {
            state
                .graph_identity
                .as_ref()
                .is_some_and(|(r, _)| r == repo)
        });
        let follow_files = if state.drill.is_files() {
            state.follow_commit_source()
        } else {
            None
        };
        let follow_diff = match &state.drill {
            DrillView::Diff { repo, source, .. } => {
                state.focused_commit_file_row().and_then(|row| {
                    if !row.is_file() {
                        return None;
                    }
                    let (files, file_cursor) = commit_diff_list(state);
                    Some(FollowDiffRequest {
                        repo: repo.clone(),
                        source: source.clone(),
                        path: row.path.clone(),
                        context: state.commit_diff_context(repo, &row.path),
                        files,
                        file_cursor,
                        focused_file: focused_file.clone(),
                    })
                })
            }
            _ => None,
        };
        Self {
            cwd: state.cwd.clone(),
            snapshot: state.snapshot.clone(),
            show_ignored: state.show_ignored,
            in_graph: state.drill.is_graph(),
            file_diff_context: focused_file
                .as_ref()
                .and_then(|(repo, change)| state.workspace_diff_context(repo, &change.path)),
            focused_file,
            focused_graph_repo,
            same_repo,
            graph_limit: refresh_graph_limit(state.graph.as_ref()),
            follow_files,
            follow_diff,
        }
    }

    fn target(&self) -> RightPaneTarget {
        RightPaneTarget {
            in_graph: self.in_graph,
            file: self
                .focused_file
                .as_ref()
                .map(|(repo, change)| (repo.clone(), change.path.clone())),
            graph_repo: self.focused_graph_repo.clone(),
            follow_files: self.follow_files.clone(),
            follow_diff: self
                .follow_diff
                .as_ref()
                .map(|diff| (diff.repo.clone(), diff.source.clone(), diff.path.clone())),
        }
    }

    fn compute(&self) -> RightPaneLoad {
        if let Some(follow) = &self.follow_diff {
            let content = compute_commit_diff(
                &self.cwd,
                &follow.repo,
                &follow.source,
                &follow.path,
                follow.context,
                follow.focused_file.as_ref(),
            );
            return RightPaneLoad::CommitDiff {
                repo: follow.repo.clone(),
                source: follow.source.clone(),
                files: follow.files.clone(),
                file_cursor: follow.file_cursor,
                path: follow.path.clone(),
                content,
            };
        }
        if self.in_graph {
            if let Some((repo, change)) = &self.focused_file {
                let content = load_file_diff(&self.cwd, repo, change, self.file_diff_context);
                return RightPaneLoad::Diff {
                    repo: repo.clone(),
                    path: change.path.clone(),
                    content,
                };
            }
        }
        if let Some(repo) = &self.focused_graph_repo {
            let (model, identity) = if self.same_repo {
                load_graph_model_window(
                    &self.cwd,
                    &self.snapshot,
                    repo,
                    self.show_ignored,
                    0,
                    self.graph_limit,
                )
            } else {
                load_graph_model(&self.cwd, &self.snapshot, repo, self.show_ignored)
            };
            let files = self.follow_files.as_ref().map(|(file_repo, source)| {
                let listed = compute_commit_files(&self.cwd.join(file_repo), source);
                (file_repo.clone(), source.clone(), listed)
            });
            return RightPaneLoad::Graph {
                model,
                identity,
                files,
            };
        }
        if self.in_graph {
            RightPaneLoad::Clear
        } else if let Some((repo, source)) = &self.follow_files {
            let files = compute_commit_files(&self.cwd.join(repo), source);
            RightPaneLoad::CommitFiles {
                repo: repo.clone(),
                source: source.clone(),
                files,
            }
        } else {
            RightPaneLoad::None
        }
    }
}

fn apply_right_pane_load(state: &mut AppState, payload: RightPaneLoad) {
    match payload {
        RightPaneLoad::Diff {
            repo,
            path,
            content,
        } => state.set_diff(repo, path, content),
        RightPaneLoad::Graph {
            model,
            identity,
            files,
        } => {
            state.set_graph(model, identity.repo, identity.head);
            if let Some((repo, source, files)) = files {
                state.open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
            }
        }
        RightPaneLoad::Clear => state.clear_right(),
        RightPaneLoad::CommitFiles {
            repo,
            source,
            files,
        } => {
            state.open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
        }
        RightPaneLoad::CommitDiff {
            repo,
            source,
            files,
            file_cursor,
            path,
            content,
        } => state.open_commit_diff(repo, source, files, file_cursor, path, content),
        RightPaneLoad::None => {}
    }
}

/// Run right-pane git on a worker. Nav and pane switch stay live; nested
/// writes are drained. Returns true when the user asked to quit.
///
/// If the cursor moved while the worker ran, load again for the new target
/// instead of leaving the pane on the row that started the request.
fn load_right_pumped(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> bool {
    loop {
        let request = RightPaneRequest::from_state(state);
        let target = request.target();
        match run_work_pumped(terminal, state, opts, move || request.compute()) {
            WorkPump::Quit => return true,
            WorkPump::Done(payload) => {
                apply_right_pane_load(state, payload);
                if RightPaneRequest::from_state(state).target() == target {
                    return false;
                }
            }
        }
    }
}

/// Reload snapshot + pane after a local write. Always runs so an error
/// return cannot skip the pump and flush queued keys.
fn refresh_after_write_pumped(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> bool {
    if reload_snapshot_pumped(state, opts, terminal) {
        return true;
    }
    load_right_pumped(state, opts, terminal)
}

/// Pump a local git write, then always refresh. Error paths still refresh.
fn run_write_then_refresh_pumped<T: Send + 'static>(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
    status_ok: impl FnOnce(&T) -> String,
    status_err: impl FnOnce(&str) -> String,
) -> bool {
    let outcome = match run_work_pumped(terminal, state, opts, work) {
        WorkPump::Quit => return true,
        WorkPump::Done(result) => result,
    };
    let status = match &outcome {
        Ok(value) => status_ok(value),
        Err(err) => status_err(err),
    };
    if refresh_after_write_pumped(state, opts, terminal) {
        return true;
    }
    state.status = status;
    false
}

fn apply_effect(
    state: &mut AppState,
    effect: Effect,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    action: &Action,
) -> bool {
    if apply_effect_inner(state, effect, opts, terminal) {
        return true;
    }
    if action_triggers_graph_autoload(action) {
        return maybe_autoload_graph(state, opts, Some(terminal));
    }
    false
}

fn apply_effect_inner(
    state: &mut AppState,
    effect: Effect,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> bool {
    match effect {
        Effect::None | Effect::Quit => {}
        Effect::Batch(effects) => {
            for child in effects {
                if apply_effect_inner(state, child, opts, terminal) {
                    return true;
                }
            }
        }
        Effect::Fetch { repos } => {
            if run_bulk_remote_pumped(
                state,
                opts,
                terminal,
                repos,
                RunningOp::Fetch,
                |dir| exec_git_checked(&["fetch", "--quiet"], &dir),
                Result::is_ok,
            ) {
                return true;
            }
        }
        Effect::Pull { repos } => {
            if run_bulk_remote_pumped(
                state,
                opts,
                terminal,
                repos,
                RunningOp::Pull,
                |dir| pull_quiet_detailed(&dir),
                |result| result.ok,
            ) {
                return true;
            }
        }
        Effect::DefaultBranch { repos } => {
            let total = repos.len();
            let mut ok = 0;
            let mut failed = 0;
            paint_running_op(state, terminal, RunningOp::DefaultBranch, 0, total);
            for (i, repo) in repos.iter().enumerate() {
                let task = state
                    .snapshot
                    .repos
                    .iter()
                    .find(|r| r.repo == *repo)
                    .map(|snap| (snap.branch.clone(), snap.default_branch_override.clone()));
                if let Some((branch, override_name)) = task {
                    let cwd = opts.cwd.clone();
                    let repo = repo.clone();
                    match run_work_pumped(terminal, state, opts, move || {
                        switch_repo_to_default_branch(
                            &repo,
                            &branch,
                            &cwd,
                            override_name.as_deref(),
                        )
                    }) {
                        WorkPump::Quit => return true,
                        WorkPump::Done((success, _)) => {
                            if success {
                                ok += 1;
                            } else {
                                failed += 1;
                            }
                        }
                    }
                } else {
                    failed += 1;
                }
                paint_running_op(state, terminal, RunningOp::DefaultBranch, i + 1, total);
            }
            if reload_snapshot_pumped(state, opts, terminal) {
                return true;
            }
            state.stamp_checkout_flashes(&repos);
            state.status = format_completed_op(RunningOp::DefaultBranch, ok, failed);
            if load_right_pumped(state, opts, terminal) {
                return true;
            }
        }
        Effect::ReloadSnapshot => {
            if reload_snapshot_pumped(state, opts, terminal) {
                return true;
            }
            state.status = "refreshed workspace".into();
            if load_right_pumped(state, opts, terminal) {
                return true;
            }
        }
        Effect::ReloadRepo { repo } => {
            if reload_repo_pumped(state, opts, terminal, &repo) {
                return true;
            }
            state.status = format!("refreshed {repo}");
            if load_right_pumped(state, opts, terminal) {
                return true;
            }
        }
        Effect::LoadRightPane => {
            if load_right_pumped(state, opts, terminal) {
                return true;
            }
        }
        Effect::Stage { repo, paths } => {
            let dir = opts.cwd.join(&repo);
            let paths_work = paths.clone();
            let last = paths.last().cloned().unwrap_or_default();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || {
                    for path in &paths_work {
                        stage_file(&dir, path)?;
                    }
                    Ok(())
                },
                move |_| format!("staged {last}"),
                |err| format!("stage failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::Unstage { repo, paths } => {
            let dir = opts.cwd.join(&repo);
            let paths_work = paths.clone();
            let last = paths.last().cloned().unwrap_or_default();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || {
                    for path in &paths_work {
                        unstage_file(&dir, path)?;
                    }
                    Ok(())
                },
                move |_| format!("unstaged {last}"),
                |err| format!("unstage failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::Revert {
            repo,
            tracked,
            untracked,
        } => {
            let dir = opts.cwd.join(&repo);
            let tracked_work = tracked.clone();
            let untracked_work = untracked.clone();
            let ok_status = if tracked.len() + untracked.len() == 1 {
                if untracked.len() == 1 {
                    format!("deleted {}", untracked[0])
                } else {
                    format!("reverted {}", tracked[0])
                }
            } else {
                format!(
                    "reverted {} tracked, {} untracked",
                    tracked.len(),
                    untracked.len()
                )
            };
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || {
                    for path in &tracked_work {
                        revert_tracked_file(&dir, path)?;
                    }
                    for path in &untracked_work {
                        remove_untracked_file(&dir, path)?;
                    }
                    Ok(())
                },
                move |_| ok_status,
                |err| format!("revert failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::EditFile { repo, path } => {
            let editor = resolve_editor(
                opts.config.editor.as_deref(),
                std::env::var("EDITOR").ok().as_deref(),
                std::env::var("VISUAL").ok().as_deref(),
            );
            let abs = opts.cwd.join(&repo).join(&path);
            let (cmd, args) = editor_command(&editor, &abs.to_string_lossy(), None);
            if is_detached_editor(&editor) {
                let _ = Command::new(&cmd)
                    .args(&args)
                    .current_dir(opts.cwd.join(&repo))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
                state.status = format!("opened {path}");
            } else if let Err(err) = run_blocking_editor(
                terminal,
                &cmd,
                &args,
                &opts.cwd.join(&repo),
                state.mouse_enabled,
            ) {
                state.status = format!("edit failed: {err}");
            } else {
                state.status = format!("edited {path}");
                drain_pending_events();
                if reload_repo_pumped(state, opts, terminal, &repo) {
                    return true;
                }
                if load_right_pumped(state, opts, terminal) {
                    return true;
                }
            }
        }
        Effect::WatchRefresh => {
            let cwd = opts.cwd.clone();
            let config = opts.config.clone();
            let filter = state.snapshot.filter_repos.clone();
            let show_ignored = state.show_ignored;
            match run_work_pumped(terminal, state, opts, move || {
                collect_full_snapshot(&cwd, &config, &filter, show_ignored, false)
            }) {
                WorkPump::Quit => return true,
                WorkPump::Done(snapshot) => {
                    if apply_watch_snapshot_for_tick(state, snapshot)
                        && load_right_pumped(state, opts, terminal)
                    {
                        return true;
                    }
                }
            }
        }
        Effect::Push { repos } => {
            if run_bulk_remote_pumped(
                state,
                opts,
                terminal,
                repos,
                RunningOp::Push,
                |dir| push_quiet(&dir),
                Result::is_ok,
            ) {
                return true;
            }
        }
        Effect::PrepareStashMenu { repo } => {
            let dir = opts.cwd.join(&repo);
            match run_work_pumped(terminal, state, opts, move || latest_stash_ref(&dir)) {
                WorkPump::Quit => return true,
                WorkPump::Done(latest) => state.open_stash_menu(repo, latest),
            }
        }
        Effect::StashCreate { repo, paths } => {
            let dir = opts.cwd.join(&repo);
            let paths_work = paths.clone();
            let ok_status = if paths.len() == 1 {
                "Stashed 1 file".to_string()
            } else if paths.is_empty() {
                "Stashed".to_string()
            } else {
                format!("Stashed {} files", paths.len())
            };
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || stash_push(&dir, &paths_work),
                move |_| ok_status,
                |err| format!("stash failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::StashApply { repo, stash_ref } => {
            let dir = opts.cwd.join(&repo);
            let stash = stash_ref.clone();
            let stash_status = stash_ref.clone();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || stash_apply(&dir, &stash),
                move |_| format!("applied {stash_status}"),
                |err| format!("apply failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::StashPop { repo, stash_ref } => {
            let dir = opts.cwd.join(&repo);
            let stash = stash_ref.clone();
            let stash_status = stash_ref.clone();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || stash_pop(&dir, &stash),
                move |_| format!("popped {stash_status}"),
                |err| format!("pop failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::StashDrop { repo, stash_ref } => {
            let dir = opts.cwd.join(&repo);
            let stash = stash_ref.clone();
            let stash_status = stash_ref.clone();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || stash_drop(&dir, &stash),
                move |_| format!("dropped {stash_status}"),
                |err| format!("drop failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::PrepareBranchPicker { repo } => {
            let dir = opts.cwd.join(&repo);
            match run_work_pumped(terminal, state, opts, move || list_local_branches(&dir)) {
                WorkPump::Quit => return true,
                WorkPump::Done(branches) => state.open_branch_picker(repo, branches),
            }
        }
        Effect::CheckoutBranch {
            repo,
            selected_name,
            fast_forward_ref,
        } => {
            let dir = opts.cwd.join(&repo);
            let name = selected_name.clone();
            let ff = fast_forward_ref.clone();
            match run_work_pumped(terminal, state, opts, move || {
                compute_checkout(&dir, &name, ff.as_deref())
            }) {
                WorkPump::Quit => return true,
                WorkPump::Done(result) => {
                    let changed = apply_checkout_compute(state, repo, result);
                    if changed && reload_snapshot_pumped(state, opts, terminal) {
                        return true;
                    }
                    if load_right_pumped(state, opts, terminal) {
                        return true;
                    }
                }
            }
        }
        Effect::CreateBranch { repo, name } => {
            let dir = opts.cwd.join(&repo);
            let name_work = name.clone();
            let name_status = name.clone();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || create_branch_checkout(&dir, &name_work),
                move |_| format!("created {name_status}"),
                |err| format!("create branch failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::CreateBranchAt {
            repo,
            name,
            commit_id,
        } => {
            let dir = opts.cwd.join(&repo);
            let name_work = name.clone();
            let commit_work = commit_id.clone();
            let name_status = name.clone();
            let short = commit_id.get(..7).unwrap_or(&commit_id).to_string();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || create_branch_at(&dir, &name_work, &commit_work),
                move |_| format!("created {name_status} at {short}"),
                |err| format!("create branch failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::MergeIntoHead { repo, rev, label } => {
            let dir = opts.cwd.join(&repo);
            let rev_work = rev.clone();
            match run_work_pumped(terminal, state, opts, move || {
                compute_merge(&dir, &rev_work)
            }) {
                WorkPump::Quit => return true,
                WorkPump::Done(result) => {
                    let changed = apply_merge_compute(state, &label, result);
                    if changed && reload_snapshot_pumped(state, opts, terminal) {
                        return true;
                    }
                    if load_right_pumped(state, opts, terminal) {
                        return true;
                    }
                }
            }
        }
        Effect::RemoveWorktree {
            primary,
            path,
            force,
        } => {
            let primary_dir = opts.cwd.join(&primary);
            let path_dir = opts.cwd.join(&path);
            let path_status = path.clone();
            if run_write_then_refresh_pumped(
                state,
                opts,
                terminal,
                move || remove_worktree(&primary_dir, &path_dir, force),
                move |_| format!("removed worktree {path_status}"),
                |err| format!("remove worktree failed: {err}"),
            ) {
                return true;
            }
        }
        Effect::LoadCommitFiles { repo, source } => {
            if load_commit_files_pumped(state, opts, terminal, repo, source) {
                return true;
            }
        }
        Effect::LoadCommitDiff { repo, source, path } => {
            if load_commit_diff_pumped(state, opts, terminal, repo, source, path) {
                return true;
            }
        }
    }
    false
}

/// Git-only checkout work. TTY runs this on a worker; tests call it via
/// [`run_checkout_branch`].
enum CheckoutCompute {
    Dirty,
    Failed {
        status: String,
        clear_picker: bool,
    },
    Confirm {
        local_branch: String,
        remote_ref: String,
    },
    Done {
        status: String,
    },
}

/// Git-only merge work. TTY runs this on a worker; tests call it via
/// [`run_merge_into_head`].
enum MergeCompute {
    Dirty,
    AlreadyUpToDate,
    FastForward,
    MergeCommit,
    Conflict,
    Failed(String),
}

fn compute_checkout(
    dir: &Path,
    selected_name: &str,
    fast_forward_ref: Option<&str>,
) -> CheckoutCompute {
    if fast_forward_ref.is_none() && repo_has_local_changes(dir) {
        return CheckoutCompute::Dirty;
    }
    if let Some(remote_ref) = fast_forward_ref {
        if !checkout_branch(selected_name, dir) {
            return CheckoutCompute::Failed {
                status: format!("Checkout failed: {selected_name}"),
                clear_picker: true,
            };
        }
        let ff = fast_forward_to_remote_ref(remote_ref, dir);
        return CheckoutCompute::Done {
            status: if ff {
                format!("Checked out {selected_name} and fast-forwarded to {remote_ref}")
            } else {
                format!("Checked out {selected_name}; could not fast-forward to {remote_ref}")
            },
        };
    }

    let local_name = checkout_name_for_ref(selected_name);
    let local_sha = rev_parse_quiet(&format!("refs/heads/{local_name}"), dir);
    let remote_sha = if is_origin_remote_ref(selected_name) {
        rev_parse_quiet(&format!("refs/remotes/{selected_name}"), dir)
    } else {
        rev_parse_quiet(&format!("refs/remotes/origin/{local_name}"), dir)
    };
    match plan_graph_checkout(
        selected_name,
        local_sha.is_some(),
        local_sha.as_deref(),
        remote_sha.as_deref(),
    ) {
        GraphCheckoutPlan::ConfirmLocalThenPull {
            local_branch,
            remote_ref,
        } => CheckoutCompute::Confirm {
            local_branch,
            remote_ref,
        },
        GraphCheckoutPlan::Checkout { branch } => {
            if checkout_branch(&branch, dir) {
                CheckoutCompute::Done {
                    status: format!("Checked out {branch}"),
                }
            } else {
                CheckoutCompute::Failed {
                    status: format!("Checkout failed: {branch}"),
                    clear_picker: false,
                }
            }
        }
    }
}

fn apply_checkout_compute(state: &mut AppState, repo: String, result: CheckoutCompute) -> bool {
    match result {
        CheckoutCompute::Dirty => {
            state.status = DIRTY_WORKTREE_STATUS.into();
            false
        }
        CheckoutCompute::Failed {
            status,
            clear_picker,
        } => {
            if clear_picker {
                state.branch_picker = None;
            }
            state.status = status;
            false
        }
        CheckoutCompute::Confirm {
            local_branch,
            remote_ref,
        } => {
            state.branch_picker = None;
            let _ = state.confirm_checkout_if_out_of_sync(repo, local_branch, Some(remote_ref));
            false
        }
        CheckoutCompute::Done { status } => {
            state.branch_picker = None;
            state.status = status;
            true
        }
    }
}

fn compute_merge(dir: &Path, rev: &str) -> MergeCompute {
    if repo_has_local_changes(dir) {
        return MergeCompute::Dirty;
    }
    match merge_into_head(rev, dir) {
        MergeIntoHeadResult::AlreadyUpToDate => MergeCompute::AlreadyUpToDate,
        MergeIntoHeadResult::FastForward => MergeCompute::FastForward,
        MergeIntoHeadResult::MergeCommit => MergeCompute::MergeCommit,
        MergeIntoHeadResult::Conflict => MergeCompute::Conflict,
        MergeIntoHeadResult::Failed(err) => MergeCompute::Failed(err),
    }
}

fn apply_merge_compute(state: &mut AppState, label: &str, result: MergeCompute) -> bool {
    match result {
        MergeCompute::Dirty => {
            state.status = DIRTY_WORKTREE_STATUS.into();
            false
        }
        MergeCompute::AlreadyUpToDate => {
            state.status = "Already up to date".into();
            false
        }
        MergeCompute::FastForward => {
            state.status = format!("Fast-forwarded to {label}");
            true
        }
        MergeCompute::MergeCommit => {
            state.status = format!("Merged {label}");
            true
        }
        MergeCompute::Conflict => {
            state.status = "Merge conflict — resolve in the worktree".into();
            true
        }
        MergeCompute::Failed(err) => {
            state.status = format!("merge failed: {err}");
            false
        }
    }
}

/// Run graph/tree checkout. Returns true when HEAD changed and the snapshot should reload.
///
/// Origin out-of-sync confirm fires only for a selected `origin/…` name when a
/// local branch exists with a null or mismatched SHA. After confirm Yes: checkout
/// then `git merge --ff-only` of the already-fetched remote-tracking ref.
pub(crate) fn run_checkout_branch(
    state: &mut AppState,
    cwd: &Path,
    repo: String,
    selected_name: String,
    fast_forward_ref: Option<String>,
) -> bool {
    let dir = cwd.join(&repo);
    let result = compute_checkout(&dir, &selected_name, fast_forward_ref.as_deref());
    apply_checkout_compute(state, repo, result)
}

/// Merge `rev` into HEAD of `repo`. Fast-forward when possible, otherwise a
/// merge commit. Conflicts stay in the worktree (no abort, no continue).
pub(crate) fn run_merge_into_head(
    state: &mut AppState,
    cwd: &Path,
    repo: String,
    rev: String,
    label: String,
) -> bool {
    let dir = cwd.join(&repo);
    let result = compute_merge(&dir, &rev);
    apply_merge_compute(state, &label, result)
}

/// Apply pane-load effects without a TTY. Used by the headless e2e harness.
pub(crate) fn apply_headless_effect(
    state: &mut AppState,
    effect: Effect,
    opts: &TuiOpts,
    action: &Action,
) {
    apply_headless_inner(state, effect, opts);
    if action_triggers_graph_autoload(action) {
        let _ = maybe_autoload_graph(state, opts, None);
    }
}

fn apply_headless_inner(state: &mut AppState, effect: Effect, opts: &TuiOpts) {
    match effect {
        Effect::None | Effect::Quit => {}
        Effect::Batch(effects) => {
            for child in effects {
                apply_headless_inner(state, child, opts);
            }
        }
        Effect::LoadRightPane => load_right_headless(state),
        Effect::ReloadSnapshot => {
            reload_snapshot(state, opts);
            state.status = "refreshed workspace".into();
            load_right_headless(state);
        }
        Effect::ReloadRepo { repo } => {
            reload_repo(state, opts, &repo);
            state.status = format!("refreshed {repo}");
            load_right_headless(state);
        }
        Effect::LoadCommitFiles { repo, source } => {
            load_commit_files(state, opts, repo, source);
        }
        Effect::LoadCommitDiff { repo, source, path } => {
            load_commit_diff(state, opts, repo, source, path);
        }
        Effect::WatchRefresh => {
            let snapshot = collect_full_snapshot(
                &opts.cwd,
                &opts.config,
                &state.snapshot.filter_repos,
                state.show_ignored,
                false,
            );
            if apply_watch_snapshot_for_tick(state, snapshot) {
                load_right_headless(state);
            }
        }
        _ => {}
    }
}

/// Sync commit-file git. Headless e2e only — TTY must use [`load_commit_files_pumped`].
fn load_commit_files(state: &mut AppState, opts: &TuiOpts, repo: String, source: CommitFileSource) {
    let files = compute_commit_files(&opts.cwd.join(&repo), &source);
    state.open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
}

fn compute_commit_files(dir: &Path, source: &CommitFileSource) -> Vec<NameStatus> {
    match source {
        CommitFileSource::Commit { commit_id } => list_commit_name_status(dir, commit_id),
        CommitFileSource::Stash { stash_ref } => list_stash_name_status(dir, stash_ref),
        CommitFileSource::Worktree => list_worktree_name_status(dir),
    }
}

/// List commit files on a worker. Returns true when the user asked to quit.
fn load_commit_files_pumped(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo: String,
    source: CommitFileSource,
) -> bool {
    state.begin_commit_files(repo.clone(), source.clone());
    let _ = terminal.draw(|frame| draw(frame, state));
    let dir = opts.cwd.join(&repo);
    let source_work = source.clone();
    match run_work_pumped(terminal, state, opts, move || {
        compute_commit_files(&dir, &source_work)
    }) {
        WorkPump::Quit => true,
        WorkPump::Done(files) => {
            state.open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
            false
        }
    }
}

/// Sync commit-diff git. Headless e2e only — TTY must use [`load_commit_diff_pumped`].
fn load_commit_diff(
    state: &mut AppState,
    opts: &TuiOpts,
    repo: String,
    source: CommitFileSource,
    path: String,
) {
    let context = state.commit_diff_context(&repo, &path);
    let focused = state.focused_file();
    let content = compute_commit_diff(&opts.cwd, &repo, &source, &path, context, focused.as_ref());
    let (files, file_cursor) = commit_diff_list(state);
    state.open_commit_diff(repo, source, files, file_cursor, path, content);
}

fn commit_diff_list(state: &AppState) -> (Vec<super::drill::CommitFile>, usize) {
    match &state.drill {
        super::drill::DrillView::Files { files, cursor, .. } => (files.clone(), *cursor),
        super::drill::DrillView::Diff {
            files, file_cursor, ..
        } => (files.clone(), *file_cursor),
        super::drill::DrillView::Graph => (Vec::new(), 0),
    }
}

fn compute_commit_diff(
    cwd: &Path,
    repo: &str,
    source: &CommitFileSource,
    path: &str,
    context: Option<u32>,
    focused_file: Option<&(String, FileChange)>,
) -> DiffContent {
    let dir = cwd.join(repo);
    match source {
        CommitFileSource::Commit { commit_id } => {
            DiffContent::from_lines(diff_commit_file_ctx(&dir, commit_id, path, context))
        }
        CommitFileSource::Stash { stash_ref } => {
            DiffContent::from_lines(diff_stash_file_ctx(&dir, stash_ref, path, context))
        }
        CommitFileSource::Worktree => {
            if let Some((file_repo, change)) = focused_file {
                if file_repo == repo && change.path == path {
                    return load_file_diff(cwd, repo, change, context);
                }
            }
            head_file_diff(&dir, path, context)
        }
    }
}

/// Load a commit / stash / worktree file diff on a worker.
///
/// Returns true when the user asked to quit.
fn load_commit_diff_pumped(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo: String,
    source: CommitFileSource,
    path: String,
) -> bool {
    let context = state.commit_diff_context(&repo, &path);
    let focused_file = state.focused_file();
    let (files, file_cursor) = commit_diff_list(state);
    let cwd = opts.cwd.clone();
    let repo_work = repo.clone();
    let source_work = source.clone();
    let path_work = path.clone();
    match run_work_pumped(terminal, state, opts, move || {
        compute_commit_diff(
            &cwd,
            &repo_work,
            &source_work,
            &path_work,
            context,
            focused_file.as_ref(),
        )
    }) {
        WorkPump::Quit => true,
        WorkPump::Done(content) => {
            state.open_commit_diff(repo, source, files, file_cursor, path, content);
            false
        }
    }
}

fn head_file_diff(dir: &Path, path: &str, context: Option<u32>) -> DiffContent {
    let args = git_diff_args(&["diff", "HEAD"], path, context);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    DiffContent::from_unified(crate::git::exec_git(&refs, dir))
}

fn drain_pending_events() {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        if event::read().is_err() {
            break;
        }
    }
}

/// Enable mouse capture, then turn off SGR any-event tracking (DECSET 1003).
///
/// Crossterm's [`EnableMouseCapture`] also sets 1003. Trackpad horizontal
/// scroll then arrives as `CSI < 99 ; col ; row M` (wheel-right plus the
/// motion bit). Crossterm 0.28 drops that report, so the TTY never pans.
/// Button-event tracking (1002) still reports clicks, wheel, and splitter
/// drag. With 1003 off, the terminal emits clean wheel-right (`67`) that
/// the TTY parser already maps to a horizontal pan. Headless e2e still
/// parses SGR 99 as pan (the same bytes a 1003 terminal sends).
fn enable_mouse_capture(out: &mut impl Write) -> io::Result<()> {
    execute!(out, EnableMouseCapture)?;
    write!(out, "\x1b[?1003l")?;
    out.flush()
}

fn sync_mouse_capture(enabled: bool) {
    let mut out = stdout();
    if enabled {
        let _ = enable_mouse_capture(&mut out);
    } else {
        let _ = execute!(out, DisableMouseCapture);
    }
}

fn resume_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mouse_enabled: bool,
) -> Result<(), String> {
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut out = stdout();
    if mouse_enabled {
        execute!(out, EnterAlternateScreen).map_err(|e| e.to_string())?;
        enable_mouse_capture(&mut out).map_err(|e| e.to_string())?;
    } else {
        execute!(out, EnterAlternateScreen).map_err(|e| e.to_string())?;
    }
    push_keyboard_enhancement();
    let _ = terminal.hide_cursor();
    let _ = terminal.resize(terminal_size_rect());
    let _ = terminal.clear();
    drain_pending_events();
    Ok(())
}

/// Drop queued copies of a held nav key (press / repeat / release).
///
/// Returns the first event that is not that backlog so it is not lost
/// (crossterm cannot unread).
fn discard_held_nav_backlog(held: KeyEvent) -> Option<crossterm::event::Event> {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let Ok(event) = event::read() else {
            return None;
        };
        if !is_held_nav_backlog(held, &event) {
            return Some(event);
        }
    }
    None
}

fn run_blocking_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cmd: &str,
    args: &[String],
    cwd: &Path,
    mouse_enabled: bool,
) -> Result<(), String> {
    let _ = disable_raw_mode();
    let mut out = stdout();
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    drain_pending_events();
    let spawn = Command::new(cmd)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status();
    let restore = resume_tui(terminal, mouse_enabled);
    match (spawn, restore) {
        (Ok(status), Ok(())) if status.success() => Ok(()),
        (Ok(status), Ok(())) => Err(format!("editor exited {}", status.code().unwrap_or(-1))),
        (Err(err), Ok(())) => Err(err.to_string()),
        (Ok(_), Err(err)) | (Err(_), Err(err)) => Err(err),
    }
}

/// Sync snapshot collect. Headless e2e only — TTY must use [`reload_snapshot_pumped`].
fn reload_snapshot(state: &mut AppState, opts: &TuiOpts) {
    let snapshot = collect_full_snapshot(
        &opts.cwd,
        &opts.config,
        &state.snapshot.filter_repos,
        state.show_ignored,
        false,
    );
    state.apply_watch_snapshot(snapshot);
}

fn compute_reload_repo(
    cwd: &Path,
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    show_ignored: bool,
) -> WorkspaceSnapshot {
    let existing = snapshot.repos.iter().find(|row| row.repo == repo);
    let meta = RepoCheckoutMeta {
        checkout_kind: existing
            .map(|row| row.checkout_kind)
            .unwrap_or(CheckoutKind::Primary),
        primary_repo: existing.and_then(|row| row.primary_repo.clone()),
    };
    let override_name = existing.and_then(|row| row.default_branch_override.clone());
    let mut snaps = repo_snapshots_from_workspace(snapshot);
    match process_repo(repo, cwd, false, override_name.as_deref(), &meta) {
        Some(snap) => {
            if let Some(slot) = snaps.iter_mut().find(|row| row.repo == repo) {
                *slot = snap;
            } else {
                snaps.push(snap);
            }
        }
        None => {
            snaps.retain(|row| row.repo != repo);
        }
    }
    build_workspace_snapshot(
        &snaps,
        &snapshot.ignored_repos,
        show_ignored,
        &snapshot.filter_repos,
    )
}

/// Apply a watch poll. Returns true when the right pane must reload
/// (`HEAD` / sync note / dirty set / file signatures moved).
fn apply_watch_snapshot_for_tick(state: &mut AppState, snapshot: WorkspaceSnapshot) -> bool {
    let before_sigs = state.signatures.clone();
    let before_checkouts = checkout_watch_identities(&state.snapshot);
    state.apply_watch_snapshot(snapshot);
    watch_needs_pane_reload(
        &before_sigs,
        &state.signatures,
        &before_checkouts,
        &checkout_watch_identities(&state.snapshot),
    )
}

/// Refresh one checkout in place. Missing paths drop out of the snapshot
/// (focused checkout only). Headless e2e only — TTY must use [`reload_repo_pumped`].
fn reload_repo(state: &mut AppState, opts: &TuiOpts, repo: &str) {
    let snapshot = compute_reload_repo(&opts.cwd, &state.snapshot, repo, state.show_ignored);
    state.apply_watch_snapshot(snapshot);
}

/// Refresh one checkout on a worker. Returns true when the user asked to quit.
fn reload_repo_pumped(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    repo: &str,
) -> bool {
    let cwd = opts.cwd.clone();
    let snapshot = state.snapshot.clone();
    let repo = repo.to_string();
    let show_ignored = state.show_ignored;
    match run_work_pumped(terminal, state, opts, move || {
        compute_reload_repo(&cwd, &snapshot, &repo, show_ignored)
    }) {
        WorkPump::Quit => true,
        WorkPump::Done(snapshot) => {
            state.apply_watch_snapshot(snapshot);
            false
        }
    }
}

/// Sync pane git. Headless e2e only — TTY must use [`load_right_pumped`].
fn load_right_headless(state: &mut AppState) {
    let payload = RightPaneRequest::from_state(state).compute();
    apply_right_pane_load(state, payload);
}

/// Fetch the next `git log` page when the cursor sits on the last loaded row.
///
/// Returns true when the user asked to quit during pumped git.
fn maybe_autoload_graph(
    state: &mut AppState,
    opts: &TuiOpts,
    terminal: Option<&mut Terminal<CrosstermBackend<io::Stdout>>>,
) -> bool {
    let (repo, skip, limit) = {
        if state.graph_loading_older {
            return false;
        }
        if state.right_is_diff() && !state.in_commit_drill() {
            return false;
        }
        let Some(model) = state.graph.as_ref() else {
            return false;
        };
        if !should_autoload(ShouldAutoload {
            cursor_index: state.graph_cursor,
            loaded_count: model.visible_rows().len(),
            has_more: model.has_more,
            loading: false,
        }) {
            return false;
        }
        let Some((repo, _)) = state.graph_identity.as_ref() else {
            return false;
        };
        (repo.clone(), autoload_skip(model), autoload_limit(model))
    };
    let show_ignored = state.show_ignored;
    state.graph_loading_older = true;
    let prev_status = state.status.clone();
    state.status = LOADING_OLDER.to_string();
    let page_and_identity = if let Some(terminal) = terminal {
        let _ = terminal.draw(|frame| draw(frame, state));
        let cwd = opts.cwd.clone();
        let snapshot = state.snapshot.clone();
        match run_work_pumped(terminal, state, opts, move || {
            load_graph_model_window(&cwd, &snapshot, &repo, show_ignored, skip, limit)
        }) {
            WorkPump::Quit => {
                state.graph_loading_older = false;
                if state.status == LOADING_OLDER {
                    state.status = prev_status;
                }
                return true;
            }
            WorkPump::Done(loaded) => loaded,
        }
    } else {
        load_graph_model_window(&opts.cwd, &state.snapshot, &repo, show_ignored, skip, limit)
    };
    let (page, identity) = page_and_identity;
    let Some(current) = state.graph.clone() else {
        state.graph_loading_older = false;
        if state.status == LOADING_OLDER {
            state.status = prev_status;
        }
        return false;
    };
    let merged = merge_autoload(&current, page);
    state.set_graph(merged, identity.repo, identity.head);
    state.graph_loading_older = false;
    if state.status == LOADING_OLDER {
        state.status = prev_status;
    }
    false
}

/// Discover every repo (ignored included) so `.` can show them without a walk.
pub fn collect_full_snapshot(
    cwd: &Path,
    config: &WorkspaceStatusConfig,
    filter_repos: &[String],
    show_ignored: bool,
    do_fetch: bool,
) -> WorkspaceSnapshot {
    let discover = WorkspaceStatusConfig {
        ignored_repos: Vec::new(),
        max_depth: config.max_depth,
        default_branches: config.default_branches.clone(),
        editor: config.editor.clone(),
    };
    let only: Option<BTreeSet<String>> = if filter_repos.is_empty() {
        None
    } else {
        Some(filter_repos.iter().cloned().collect())
    };
    let snapshots = collect_snapshots(cwd, do_fetch, &discover, only.as_ref());
    build_workspace_snapshot(
        &snapshots,
        &config.ignored_repos,
        show_ignored,
        filter_repos,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkspaceStatusConfig;
    use crate::git::{exec_git, git_binary, list_local_branches, stage_file};
    use crate::tui::action::Action;
    use crate::tui::branches::DIRTY_WORKTREE_STATUS;
    use std::fs;
    use std::process::Command;
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    fn git_env() -> Vec<(&'static str, &'static str)> {
        vec![
            ("GIT_AUTHOR_NAME", "workspace-status test"),
            ("GIT_AUTHOR_EMAIL", "workspace-status-test@example.invalid"),
            ("GIT_COMMITTER_NAME", "workspace-status test"),
            (
                "GIT_COMMITTER_EMAIL",
                "workspace-status-test@example.invalid",
            ),
        ]
    }

    fn git(cwd: &Path, args: &[&str]) {
        let mut cmd = Command::new(git_binary());
        cmd.args(args).current_dir(cwd);
        for (k, v) in git_env() {
            cmd.env(k, v);
        }
        let status = cmd.status().expect("git");
        assert!(status.success(), "git {args:?}");
    }

    fn init_repo(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        let init = Command::new(git_binary())
            .args(["init", "-q", "-b", "main"])
            .current_dir(dir)
            .status();
        if init.map(|s| s.success()).unwrap_or(false) == false {
            git(dir, &["init", "-q"]);
            git(dir, &["checkout", "-q", "-b", "main"]);
        }
        fs::write(dir.join("README.md"), "# seed\n").unwrap();
        git(dir, &["add", "README.md"]);
        git(dir, &["commit", "-q", "-m", "seed"]);
    }

    #[test]
    fn tree_picker_dirty_refuses_tracked_only() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-checkout-dirty-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        git(&repo_dir, &["checkout", "-q", "-b", "feature/x"]);
        git(&repo_dir, &["checkout", "-q", "main"]);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        fs::write(repo_dir.join("untracked.txt"), "u\n").unwrap();

        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        app.open_branch_picker("app".into(), list_local_branches(&repo_dir));
        app.dispatch(Action::BranchChar('f'));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                repo,
                selected_name,
                fast_forward_ref,
            } => {
                assert_eq!(selected_name, "feature/x");
                assert!(fast_forward_ref.is_none());
                assert!(!run_checkout_branch(
                    &mut app,
                    &workspace,
                    repo,
                    selected_name,
                    fast_forward_ref,
                ));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(app.status, DIRTY_WORKTREE_STATUS);
        assert!(app.branch_picker.is_some());
        assert!(app.confirm.is_none());
        assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "main");

        git(&repo_dir, &["checkout", "-q", "--", "README.md"]);
        assert!(!repo_has_local_changes(&repo_dir));
        app.open_branch_picker("app".into(), list_local_branches(&repo_dir));
        app.dispatch(Action::BranchChar('f'));
        match app.dispatch(Action::BranchSubmit) {
            Effect::CheckoutBranch {
                repo,
                selected_name,
                fast_forward_ref,
            } => {
                assert!(run_checkout_branch(
                    &mut app,
                    &workspace,
                    repo,
                    selected_name,
                    fast_forward_ref,
                ));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            exec_git(&["branch", "--show-current"], &repo_dir),
            "feature/x"
        );
        assert!(repo_dir.join("untracked.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn origin_selection_confirms_then_ff_only_local_does_not() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-checkout-ff-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        let remote = root.join("remote.git");
        Command::new(git_binary())
            .args(["init", "-q", "--bare", remote.to_str().unwrap()])
            .status()
            .unwrap();
        git(
            &repo_dir,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&repo_dir, &["push", "-u", "origin", "main", "--quiet"]);
        git(&repo_dir, &["checkout", "-q", "-b", "foo"]);
        git(&repo_dir, &["push", "-u", "origin", "foo", "--quiet"]);
        git(&repo_dir, &["checkout", "-q", "main"]);
        let other = root.join("other");
        Command::new(git_binary())
            .args([
                "clone",
                "-q",
                remote.to_str().unwrap(),
                other.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        git(&other, &["checkout", "-q", "foo"]);
        fs::write(other.join("README.md"), "# origin-ahead\n").unwrap();
        git(&other, &["add", "README.md"]);
        git(&other, &["commit", "-q", "-m", "remote"]);
        git(&other, &["push", "--quiet"]);
        git(&repo_dir, &["fetch", "--quiet"]);

        let local = exec_git(&["rev-parse", "foo"], &repo_dir);
        let remote_sha = exec_git(&["rev-parse", "origin/foo"], &repo_dir);
        assert_ne!(local, remote_sha);

        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);

        assert!(run_checkout_branch(
            &mut app,
            &workspace,
            "app".into(),
            "foo".into(),
            None,
        ));
        assert!(app.confirm.is_none());
        assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "foo");
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &repo_dir), local);

        git(&repo_dir, &["checkout", "-q", "main"]);
        assert!(!run_checkout_branch(
            &mut app,
            &workspace,
            "app".into(),
            "origin/foo".into(),
            None,
        ));
        assert!(app.confirm.is_some());
        match app.dispatch(Action::ConfirmYes) {
            Effect::CheckoutBranch {
                selected_name,
                fast_forward_ref,
                repo,
            } => {
                assert_eq!(selected_name, "foo");
                assert_eq!(fast_forward_ref.as_deref(), Some("origin/foo"));
                assert!(run_checkout_branch(
                    &mut app,
                    &workspace,
                    repo,
                    selected_name,
                    fast_forward_ref,
                ));
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "foo");
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &repo_dir), remote_sha);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn graph_merge_dirty_refuses_then_ff_and_conflict_stay_uncommitted() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-merge-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        git(&repo_dir, &["config", "user.name", "workspace-status test"]);
        git(
            &repo_dir,
            &[
                "config",
                "user.email",
                "workspace-status-test@example.invalid",
            ],
        );
        git(&repo_dir, &["checkout", "-q", "-b", "topic"]);
        fs::write(repo_dir.join("topic.txt"), "topic\n").unwrap();
        git(&repo_dir, &["add", "topic.txt"]);
        git(&repo_dir, &["commit", "-q", "-m", "topic"]);
        let topic = exec_git(&["rev-parse", "HEAD"], &repo_dir);
        git(&repo_dir, &["checkout", "-q", "main"]);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        fs::write(repo_dir.join("untracked.txt"), "u\n").unwrap();

        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        assert!(!run_merge_into_head(
            &mut app,
            &workspace,
            "app".into(),
            topic.clone(),
            "topic".into(),
        ));
        assert_eq!(app.status, DIRTY_WORKTREE_STATUS);
        assert_eq!(exec_git(&["branch", "--show-current"], &repo_dir), "main");
        assert_ne!(exec_git(&["rev-parse", "HEAD"], &repo_dir), topic);

        git(&repo_dir, &["checkout", "-q", "--", "README.md"]);
        assert!(!repo_has_local_changes(&repo_dir));
        assert!(run_merge_into_head(
            &mut app,
            &workspace,
            "app".into(),
            topic.clone(),
            "topic".into(),
        ));
        assert_eq!(app.status, "Fast-forwarded to topic");
        assert_eq!(exec_git(&["rev-parse", "HEAD"], &repo_dir), topic);
        assert!(repo_dir.join("untracked.txt").exists());

        git(&repo_dir, &["reset", "--hard", "--quiet", "HEAD~1"]);
        fs::write(repo_dir.join("README.md"), "# main-side\n").unwrap();
        git(&repo_dir, &["add", "README.md"]);
        git(&repo_dir, &["commit", "-q", "-m", "main-side"]);
        git(&repo_dir, &["checkout", "-q", "-B", "other", "HEAD~1"]);
        fs::write(repo_dir.join("README.md"), "# other-side\n").unwrap();
        git(&repo_dir, &["add", "README.md"]);
        git(&repo_dir, &["commit", "-q", "-m", "other-side"]);
        let other = exec_git(&["rev-parse", "HEAD"], &repo_dir);
        git(&repo_dir, &["checkout", "-q", "main"]);
        assert!(run_merge_into_head(
            &mut app,
            &workspace,
            "app".into(),
            other,
            "other".into(),
        ));
        assert_eq!(app.status, "Merge conflict — resolve in the worktree");
        assert!(rev_parse_quiet("MERGE_HEAD", &repo_dir).is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn right_pane_load_types_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<RightPaneRequest>();
        assert_send::<RightPaneLoad>();
    }

    #[test]
    fn compute_right_pane_load_fills_graph_for_a_repo_row() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-pane-load-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        let idx = app
            .rows
            .iter()
            .position(|r| {
                r.kind == super::super::tree::NodeKind::Repo && r.repo.as_deref() == Some("app")
            })
            .expect("visible app repo row");
        app.cursor = idx;
        load_right_headless(&mut app);
        assert!(
            app.graph.is_some(),
            "repo row must load a graph from pane git"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn compute_right_pane_load_follows_graph_row_at_files_depth() {
        use super::super::drill::{CommitFileSource, DrillView};
        use super::super::state::FocusPane;

        let root = std::env::temp_dir().join(format!(
            "ws-tui-pane-follow-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        fs::write(repo_dir.join("second.txt"), "two\n").unwrap();
        git(&repo_dir, &["add", "second.txt"]);
        git(&repo_dir, &["commit", "-q", "-m", "second"]);
        git(&repo_dir, &["checkout", "-q", "-b", "feature/follow"]);
        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        let idx = app
            .rows
            .iter()
            .position(|r| {
                r.kind == super::super::tree::NodeKind::Repo && r.repo.as_deref() == Some("app")
            })
            .expect("visible app repo row");
        app.cursor = idx;
        load_right_headless(&mut app);
        assert!(app.graph.is_some());
        app.open_commit_files("app".into(), CommitFileSource::Worktree, Vec::new());
        app.focus = FocusPane::Left;
        let files_before = match &app.drill {
            DrillView::Files { files, .. } => files.clone(),
            other => panic!("expected files drill, got {other:?}"),
        };
        assert!(files_before.is_empty());
        app.graph_cursor = 1;
        load_right_headless(&mut app);
        assert_eq!(
            app.focus,
            FocusPane::Left,
            "follow must not steal left focus"
        );
        assert!(app.drill.is_files());
        let files_after = match &app.drill {
            DrillView::Files { files, .. } => files.clone(),
            other => panic!("expected files drill, got {other:?}"),
        };
        assert!(
            files_after.iter().any(|file| file.path == "second.txt"),
            "HEAD commit should list second.txt, got {files_after:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn event_thread_keeps_polling_while_right_pane_git_runs() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-pane-pump-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        let idx = app
            .rows
            .iter()
            .position(|r| {
                r.kind == super::super::tree::NodeKind::Repo && r.repo.as_deref() == Some("app")
            })
            .expect("visible app repo row");
        app.cursor = idx;
        let request = RightPaneRequest::from_state(&app);
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            let payload = request.compute();
            let _ = tx.send(payload);
        });
        let mut pumps = 0u32;
        let payload = loop {
            match rx.try_recv() {
                Ok(value) => break value,
                Err(mpsc::TryRecvError::Empty) => {
                    pumps += 1;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("right-pane worker disconnected")
                }
            }
        };
        assert!(
            pumps >= 5,
            "draw thread must poll while pane git runs (got {pumps} pumps); \
             inline load_right after fetch would stall until git log returns, \
             then crossterm would flush queued keys in a burst"
        );
        apply_right_pane_load(&mut app, payload);
        assert!(app.graph.is_some());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn checkout_and_merge_compute_are_send() {
        fn assert_send<T: Send>() {}
        assert_send::<CheckoutCompute>();
        assert_send::<MergeCompute>();
    }

    #[test]
    fn event_thread_keeps_polling_while_git_write_runs() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-write-pump-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        let dir = repo_dir.clone();
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            let result = stage_file(&dir, "README.md");
            let _ = tx.send(result);
        });
        let mut pumps = 0u32;
        let result = loop {
            match rx.try_recv() {
                Ok(value) => break value,
                Err(mpsc::TryRecvError::Empty) => {
                    pumps += 1;
                    thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("git-write worker disconnected")
                }
            }
        };
        assert!(
            pumps >= 5,
            "draw thread must poll while git add runs (got {pumps} pumps); \
             unpumped stage_file in apply_effect_inner freezes the TUI until the child exits"
        );
        result.expect("stage README.md");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn busy_wait_applies_nav_before_write_finishes() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-busy-nav-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        fs::write(repo_dir.join("README.md"), "# dirty\n").unwrap();
        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        assert!(
            app.rows.len() >= 2,
            "fixture must have more than one tree row"
        );
        app.cursor = 0;
        let start = app.cursor;
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            let _ = tx.send(());
        });
        let mut moved_during = false;
        loop {
            match rx.try_recv() {
                Ok(()) => break,
                Err(mpsc::TryRecvError::Empty) => {
                    let _ = app.dispatch(Action::Move(1));
                    if app.cursor != start {
                        moved_during = true;
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::TryRecvError::Disconnected) => panic!("nav worker disconnected"),
            }
        }
        assert!(
            moved_during,
            "nav must apply during the worker, not after join (BusyAction::Handle)"
        );
        assert_ne!(app.cursor, start);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn unchanged_watch_signatures_let_the_loop_skip_pane_git() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-watch-skip-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let repo_dir = workspace.join("app");
        init_repo(&repo_dir);
        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot.clone(), true);
        let before = app.signatures.clone();
        assert!(
            !apply_watch_snapshot_for_tick(&mut app, snapshot),
            "identical watch snapshot must skip load_right"
        );
        assert_eq!(
            before, app.signatures,
            "identical watch snapshot must keep signatures so load_right can be skipped"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn watch_tick_reloads_when_head_moves_without_tree_chrome_flip() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-watch-head-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let alpha = workspace.join("alpha");
        let beta = workspace.join("beta");
        init_repo(&alpha);
        init_repo(&beta);
        git(&alpha, &["checkout", "-q", "-b", "feature/watch"]);
        git(&beta, &["checkout", "-q", "-b", "feature/other"]);
        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        load_right_headless(&mut app);
        let before_head = app
            .snapshot
            .repos
            .iter()
            .find(|row| row.repo == "alpha")
            .map(|row| row.head.clone())
            .expect("alpha");
        let before_sigs = app.signatures.clone();

        fs::write(alpha.join("tick.txt"), "head-move\n").unwrap();
        git(&alpha, &["add", "tick.txt"]);
        git(&alpha, &["commit", "-q", "-m", "watch-head-move"]);
        let new_head = exec_git(&["rev-parse", "HEAD"], &alpha);
        assert_ne!(before_head, new_head);

        let next = collect_full_snapshot(&workspace, &config, &[], false, false);
        let alpha_row = next
            .repos
            .iter()
            .find(|row| row.repo == "alpha")
            .expect("alpha row");
        assert_eq!(alpha_row.branch, "feature/watch");
        assert_eq!(
            alpha_row.sync_status,
            crate::snapshot::SyncStatus::NoUpstream
        );
        assert!(alpha_row.changes.is_empty());
        assert_eq!(alpha_row.head, new_head);

        assert!(
            apply_watch_snapshot_for_tick(&mut app, next),
            "HEAD-only commit must not skip the pane reload"
        );
        assert_ne!(before_sigs, app.signatures);
        assert_eq!(
            app.snapshot
                .repos
                .iter()
                .find(|row| row.repo == "alpha")
                .map(|row| row.head.as_str()),
            Some(new_head.as_str())
        );

        fs::write(beta.join("dirty.txt"), "flash me\n").unwrap();
        let dirty = collect_full_snapshot(&workspace, &config, &[], false, false);
        let changed = app.apply_watch_snapshot(dirty);
        assert!(
            changed.iter().any(|id| id.contains("dirty.txt")),
            "dirty file on the other repo must flash: {changed:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn watch_tick_reloads_when_ahead_count_moves() {
        let root = std::env::temp_dir().join(format!(
            "ws-tui-watch-ahead-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let workspace = root.join("workspace");
        let remote = root.join("remote.git");
        let tracker = workspace.join("tracker");
        let sidecar = workspace.join("sidecar");
        fs::create_dir_all(&workspace).unwrap();
        Command::new(git_binary())
            .args(["init", "-q", "--bare", remote.to_str().unwrap()])
            .status()
            .unwrap();
        init_repo(&tracker);
        git(
            &tracker,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        );
        git(&tracker, &["push", "-u", "origin", "main", "--quiet"]);
        init_repo(&sidecar);
        git(&sidecar, &["checkout", "-q", "-b", "feature/sidecar"]);
        for i in 1..=2 {
            fs::write(tracker.join("count.txt"), format!("{i}\n")).unwrap();
            git(&tracker, &["add", "count.txt"]);
            git(&tracker, &["commit", "-q", "-m", &format!("ahead {i}")]);
        }
        let config = WorkspaceStatusConfig::with_defaults();
        let snapshot = collect_full_snapshot(&workspace, &config, &[], false, false);
        let tracker_row = snapshot
            .repos
            .iter()
            .find(|row| row.repo == "tracker")
            .expect("tracker");
        assert_eq!(tracker_row.sync_status, crate::snapshot::SyncStatus::Ahead);
        assert!(
            tracker_row.sync_note.contains("ahead by 2"),
            "{}",
            tracker_row.sync_note
        );
        let mut app = AppState::new(workspace.clone(), snapshot, true);
        fs::write(tracker.join("count.txt"), "3\n").unwrap();
        git(&tracker, &["add", "count.txt"]);
        git(&tracker, &["commit", "-q", "-m", "ahead 3"]);
        let next = collect_full_snapshot(&workspace, &config, &[], false, false);
        let after = next
            .repos
            .iter()
            .find(|row| row.repo == "tracker")
            .expect("tracker after");
        assert_eq!(after.sync_status, crate::snapshot::SyncStatus::Ahead);
        assert!(
            after.sync_note.contains("ahead by 3"),
            "{}",
            after.sync_note
        );
        assert!(
            apply_watch_snapshot_for_tick(&mut app, next),
            "ahead 2→3 must not skip the pane reload"
        );
        assert_eq!(
            app.snapshot
                .repos
                .iter()
                .find(|row| row.repo == "tracker")
                .map(|row| row.sync_note.as_str()),
            Some("ahead by 3 commits")
        );
        let _ = fs::remove_dir_all(&root);
    }
}

//! Crossterm event loop. First paint happens before any network fetch.

use std::collections::BTreeSet;
use std::io::{self, stdout};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture};
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
use super::drill::CommitFileSource;
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
use super::keys::event_to_action_with;
use super::ops::{format_completed_op, format_running_op, RunningOp};
use super::render::draw;
use super::state::AppState;
use super::watch::{watch_interval_ms, FLASH_TICK_MS};

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
    if execute!(out, EnterAlternateScreen, EnableMouseCapture).is_err() {
        let _ = disable_raw_mode();
        return Err(1);
    }
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
    let _ = execute!(end, DisableMouseCapture, LeaveAlternateScreen);
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
    loop {
        let remain_watch = if watch_ms == 0 {
            u64::MAX
        } else {
            watch_ms.saturating_sub(last_watch.elapsed().as_millis() as u64)
        };
        let remain_fetch = if fetch_ms == 0 {
            u64::MAX
        } else {
            fetch_ms.saturating_sub(last_fetch.elapsed().as_millis() as u64)
        };
        let remain_ctrl_c = state
            .ctrl_c_remaining_ms(Instant::now())
            .unwrap_or(u64::MAX);
        let idle_ms = if state.has_active_flashes() {
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
            loop {
                let Ok(event) = event::read() else {
                    break;
                };
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
                if !event::poll(Duration::from_millis(0)).unwrap_or(false) {
                    break;
                }
            }
            continue;
        }
        let _ = state.expire_ctrl_c_prompt(Instant::now());
        if !overlay_blocks_background_ticks(state.input_mode()) {
            if watch_ms > 0 && last_watch.elapsed().as_millis() as u64 >= watch_ms {
                let effect = state.dispatch(Action::WatchTick);
                if apply_effect(state, effect, opts, terminal, &Action::WatchTick) {
                    return Ok(());
                }
                last_watch = Instant::now();
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

/// Run `work` on a helper thread while this thread still polls crossterm.
///
/// Used for fetch / pull / watch snapshot collect **and** follow-up pane git
/// ([`load_right_pumped`], graph autoload). Keys other than quit / resize are
/// drained so they cannot sit in the TTY buffer and flush after the join.
fn run_work_pumped<T: Send + 'static>(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
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
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            loop {
                let Ok(event) = event::read() else {
                    break;
                };
                let action = map_event(state, &event);
                match classify_busy_action(&action) {
                    BusyAction::Quit => quit = true,
                    BusyAction::Resize { cols, rows } => {
                        let _ = apply_terminal_resize(terminal, cols, rows);
                        let _ = state.dispatch(Action::Resize { cols, rows });
                        let _ = terminal.draw(|frame| draw(frame, state));
                    }
                    BusyAction::Ignore => {}
                }
                if !event::poll(Duration::from_millis(0)).unwrap_or(false) {
                    break;
                }
            }
        }
        let _ = terminal.draw(|frame| draw(frame, state));
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
    match run_work_pumped(terminal, state, move || {
        collect_full_snapshot(&cwd, &config, &filter, show_ignored, false)
    }) {
        WorkPump::Quit => true,
        WorkPump::Done(snapshot) => {
            state.apply_snapshot(snapshot);
            false
        }
    }
}

/// Inputs for right-pane git (`git log` / `git diff` / worktree name-status).
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
        worktree_files: Option<(String, CommitFileSource, Vec<NameStatus>)>,
    },
    Clear,
    WorktreeFiles {
        repo: String,
        source: CommitFileSource,
        files: Vec<NameStatus>,
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
        }
    }

    fn worktree_refresh(drill: &super::drill::DrillView) -> Option<(String, CommitFileSource)> {
        match drill {
            super::drill::DrillView::Files { repo, source, .. }
            | super::drill::DrillView::Diff { repo, source, .. }
                if matches!(source, CommitFileSource::Worktree) =>
            {
                Some((repo.clone(), source.clone()))
            }
            _ => None,
        }
    }

    fn compute(&self, worktree: Option<(String, CommitFileSource)>) -> RightPaneLoad {
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
            let worktree_files = if !self.in_graph {
                worktree.map(|(wt_repo, source)| {
                    let files = list_worktree_name_status(&self.cwd.join(&wt_repo));
                    (wt_repo, source, files)
                })
            } else {
                None
            };
            return RightPaneLoad::Graph {
                model,
                identity,
                worktree_files,
            };
        }
        if self.in_graph {
            RightPaneLoad::Clear
        } else if let Some((repo, source)) = worktree {
            let files = list_worktree_name_status(&self.cwd.join(&repo));
            RightPaneLoad::WorktreeFiles {
                repo,
                source,
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
            worktree_files,
        } => {
            state.set_graph(model, identity.repo, identity.head);
            if let Some((repo, source, files)) = worktree_files {
                state.open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
            }
        }
        RightPaneLoad::Clear => state.clear_right(),
        RightPaneLoad::WorktreeFiles {
            repo,
            source,
            files,
        } => {
            state.open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
        }
        RightPaneLoad::None => {}
    }
}

/// Run right-pane git on a worker. Resize and quit still reach the loop;
/// other keys are drained so they cannot queue and flush after the join.
///
/// Returns true when the user asked to quit during that work.
fn load_right_pumped(
    state: &mut AppState,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) -> bool {
    let request = RightPaneRequest::from_state(state);
    let worktree = RightPaneRequest::worktree_refresh(&state.drill);
    match run_work_pumped(terminal, state, move || request.compute(worktree)) {
        WorkPump::Quit => true,
        WorkPump::Done(payload) => {
            apply_right_pane_load(state, payload);
            false
        }
    }
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
            let cwd = opts.cwd.clone();
            let total = repos.len();
            let mut ok = 0;
            let mut failed = 0;
            paint_running_op(state, terminal, RunningOp::Fetch, 0, total);
            for (i, repo) in repos.iter().enumerate() {
                let dir = cwd.join(repo);
                match run_work_pumped(terminal, state, move || {
                    exec_git_checked(&["fetch", "--quiet"], &dir)
                }) {
                    WorkPump::Quit => return true,
                    WorkPump::Done(Ok(())) => ok += 1,
                    WorkPump::Done(Err(_)) => failed += 1,
                }
                paint_running_op(state, terminal, RunningOp::Fetch, i + 1, total);
            }
            if reload_snapshot_pumped(state, opts, terminal) {
                return true;
            }
            state.stamp_checkout_flashes(&repos);
            state.status = format_completed_op(RunningOp::Fetch, ok, failed);
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::Pull { repos } => {
            let cwd = opts.cwd.clone();
            let total = repos.len();
            let mut ok = 0;
            let mut failed = 0;
            paint_running_op(state, terminal, RunningOp::Pull, 0, total);
            for (i, repo) in repos.iter().enumerate() {
                let dir = cwd.join(repo);
                match run_work_pumped(terminal, state, move || pull_quiet_detailed(&dir)) {
                    WorkPump::Quit => return true,
                    WorkPump::Done(result) => {
                        if result.ok {
                            ok += 1;
                        } else {
                            failed += 1;
                        }
                    }
                }
                paint_running_op(state, terminal, RunningOp::Pull, i + 1, total);
            }
            if reload_snapshot_pumped(state, opts, terminal) {
                return true;
            }
            state.stamp_checkout_flashes(&repos);
            state.status = format_completed_op(RunningOp::Pull, ok, failed);
            if load_right_pumped(state, terminal) {
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
                    match run_work_pumped(terminal, state, move || {
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
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::ReloadSnapshot => {
            if reload_snapshot_pumped(state, opts, terminal) {
                return true;
            }
            state.status = "refreshed workspace".into();
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::ReloadRepo { repo } => {
            reload_repo(state, opts, &repo);
            state.status = format!("refreshed {repo}");
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::LoadRightPane => {
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::Stage { repo, paths } => {
            let dir = opts.cwd.join(&repo);
            for path in &paths {
                if let Err(err) = stage_file(&dir, path) {
                    state.status = format!("stage failed: {err}");
                    return false;
                }
            }
            reload_snapshot(state, opts);
            state.status = format!("staged {}", paths.last().map(String::as_str).unwrap_or(""));
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::Unstage { repo, paths } => {
            let dir = opts.cwd.join(&repo);
            for path in &paths {
                if let Err(err) = unstage_file(&dir, path) {
                    state.status = format!("unstage failed: {err}");
                    return false;
                }
            }
            reload_snapshot(state, opts);
            state.status = format!(
                "unstaged {}",
                paths.last().map(String::as_str).unwrap_or("")
            );
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::Revert {
            repo,
            tracked,
            untracked,
        } => {
            let dir = opts.cwd.join(&repo);
            for path in &tracked {
                if let Err(err) = revert_tracked_file(&dir, path) {
                    state.status = format!("revert failed: {err}");
                    return false;
                }
            }
            for path in &untracked {
                if let Err(err) = remove_untracked_file(&dir, path) {
                    state.status = format!("revert failed: {err}");
                    return false;
                }
            }
            reload_snapshot(state, opts);
            if tracked.len() + untracked.len() == 1 {
                if untracked.len() == 1 {
                    state.status = format!("deleted {}", untracked[0]);
                } else {
                    state.status = format!("reverted {}", tracked[0]);
                }
            } else {
                state.status = format!(
                    "reverted {} tracked, {} untracked",
                    tracked.len(),
                    untracked.len()
                );
            }
            if load_right_pumped(state, terminal) {
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
            }
        }
        Effect::WatchRefresh => {
            let cwd = opts.cwd.clone();
            let config = opts.config.clone();
            let filter = state.snapshot.filter_repos.clone();
            let show_ignored = state.show_ignored;
            match run_work_pumped(terminal, state, move || {
                collect_full_snapshot(&cwd, &config, &filter, show_ignored, false)
            }) {
                WorkPump::Quit => return true,
                WorkPump::Done(snapshot) => {
                    let before = state.signatures.clone();
                    let _changed = state.apply_watch_snapshot(snapshot);
                    if before != state.signatures && load_right_pumped(state, terminal) {
                        return true;
                    }
                }
            }
        }
        Effect::Push { repos } => {
            let mut ok = 0;
            let mut failed = 0;
            let total = repos.len();
            paint_running_op(state, terminal, RunningOp::Push, 0, total);
            for (i, repo) in repos.iter().enumerate() {
                let dir = opts.cwd.join(repo);
                match run_work_pumped(terminal, state, move || push_quiet(&dir)) {
                    WorkPump::Quit => return true,
                    WorkPump::Done(Ok(())) => ok += 1,
                    WorkPump::Done(Err(_)) => failed += 1,
                }
                paint_running_op(state, terminal, RunningOp::Push, i + 1, total);
            }
            if reload_snapshot_pumped(state, opts, terminal) {
                return true;
            }
            state.stamp_checkout_flashes(&repos);
            state.status = format_completed_op(RunningOp::Push, ok, failed);
            if load_right_pumped(state, terminal) {
                return true;
            }
        }
        Effect::PrepareStashMenu { repo } => {
            let latest = latest_stash_ref(&opts.cwd.join(&repo));
            state.open_stash_menu(repo, latest);
        }
        Effect::StashCreate { repo, paths } => match stash_push(&opts.cwd.join(&repo), &paths) {
            Ok(()) => {
                reload_snapshot(state, opts);
                state.status = if paths.len() == 1 {
                    "Stashed 1 file".into()
                } else if paths.is_empty() {
                    "Stashed".into()
                } else {
                    format!("Stashed {} files", paths.len())
                };
                if load_right_pumped(state, terminal) {
                    return true;
                }
            }
            Err(err) => state.status = format!("stash failed: {err}"),
        },
        Effect::StashApply { repo, stash_ref } => {
            match stash_apply(&opts.cwd.join(&repo), &stash_ref) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("applied {stash_ref}");
                    if load_right_pumped(state, terminal) {
                        return true;
                    }
                }
                Err(err) => state.status = format!("apply failed: {err}"),
            }
        }
        Effect::StashPop { repo, stash_ref } => {
            match stash_pop(&opts.cwd.join(&repo), &stash_ref) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("popped {stash_ref}");
                    if load_right_pumped(state, terminal) {
                        return true;
                    }
                }
                Err(err) => state.status = format!("pop failed: {err}"),
            }
        }
        Effect::StashDrop { repo, stash_ref } => {
            match stash_drop(&opts.cwd.join(&repo), &stash_ref) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("dropped {stash_ref}");
                    if load_right_pumped(state, terminal) {
                        return true;
                    }
                }
                Err(err) => state.status = format!("drop failed: {err}"),
            }
        }
        Effect::PrepareBranchPicker { repo } => {
            let branches = list_local_branches(&opts.cwd.join(&repo));
            state.open_branch_picker(repo, branches);
        }
        Effect::CheckoutBranch {
            repo,
            selected_name,
            fast_forward_ref,
        } => {
            if run_checkout_branch(state, &opts.cwd, repo, selected_name, fast_forward_ref) {
                reload_snapshot(state, opts);
                if load_right_pumped(state, terminal) {
                    return true;
                }
            }
        }
        Effect::CreateBranch { repo, name } => {
            match create_branch_checkout(&opts.cwd.join(&repo), &name) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("created {name}");
                    if load_right_pumped(state, terminal) {
                        return true;
                    }
                }
                Err(err) => state.status = format!("create branch failed: {err}"),
            }
        }
        Effect::CreateBranchAt {
            repo,
            name,
            commit_id,
        } => match create_branch_at(&opts.cwd.join(&repo), &name, &commit_id) {
            Ok(()) => {
                reload_snapshot(state, opts);
                let short = commit_id.get(..7).unwrap_or(&commit_id);
                state.status = format!("created {name} at {short}");
                if load_right_pumped(state, terminal) {
                    return true;
                }
            }
            Err(err) => state.status = format!("create branch failed: {err}"),
        },
        Effect::MergeIntoHead { repo, rev, label } => {
            if run_merge_into_head(state, &opts.cwd, repo, rev, label) {
                reload_snapshot(state, opts);
                if load_right_pumped(state, terminal) {
                    return true;
                }
            }
        }
        Effect::RemoveWorktree {
            primary,
            path,
            force,
        } => match remove_worktree(&opts.cwd.join(&primary), &opts.cwd.join(&path), force) {
            Ok(()) => {
                reload_snapshot(state, opts);
                state.status = format!("removed worktree {path}");
                if load_right_pumped(state, terminal) {
                    return true;
                }
            }
            Err(err) => state.status = format!("remove worktree failed: {err}"),
        },
        Effect::LoadCommitFiles { repo, source } => {
            state.begin_commit_files(repo.clone(), source.clone());
            let _ = terminal.draw(|frame| draw(frame, state));
            load_commit_files(state, opts, repo, source);
        }
        Effect::LoadCommitDiff { repo, source, path } => {
            load_commit_diff(state, opts, repo, source, path);
        }
    }
    false
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
    if fast_forward_ref.is_none() && repo_has_local_changes(&dir) {
        state.status = DIRTY_WORKTREE_STATUS.into();
        return false;
    }
    if let Some(remote_ref) = fast_forward_ref {
        if !checkout_branch(&selected_name, &dir) {
            state.branch_picker = None;
            state.status = format!("Checkout failed: {selected_name}");
            return false;
        }
        let ff = fast_forward_to_remote_ref(&remote_ref, &dir);
        state.branch_picker = None;
        state.status = if ff {
            format!("Checked out {selected_name} and fast-forwarded to {remote_ref}")
        } else {
            format!("Checked out {selected_name}; could not fast-forward to {remote_ref}")
        };
        return true;
    }

    let local_name = checkout_name_for_ref(&selected_name);
    let local_sha = rev_parse_quiet(&format!("refs/heads/{local_name}"), &dir);
    let remote_sha = if is_origin_remote_ref(&selected_name) {
        rev_parse_quiet(&format!("refs/remotes/{selected_name}"), &dir)
    } else {
        rev_parse_quiet(&format!("refs/remotes/origin/{local_name}"), &dir)
    };
    match plan_graph_checkout(
        &selected_name,
        local_sha.is_some(),
        local_sha.as_deref(),
        remote_sha.as_deref(),
    ) {
        GraphCheckoutPlan::ConfirmLocalThenPull {
            local_branch,
            remote_ref,
        } => {
            state.branch_picker = None;
            let _ = state.confirm_checkout_if_out_of_sync(repo, local_branch, Some(remote_ref));
            false
        }
        GraphCheckoutPlan::Checkout { branch } => {
            if checkout_branch(&branch, &dir) {
                state.branch_picker = None;
                state.status = format!("Checked out {branch}");
                true
            } else {
                state.status = format!("Checkout failed: {branch}");
                false
            }
        }
    }
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
    if repo_has_local_changes(&dir) {
        state.status = DIRTY_WORKTREE_STATUS.into();
        return false;
    }
    match merge_into_head(&rev, &dir) {
        MergeIntoHeadResult::AlreadyUpToDate => {
            state.status = "Already up to date".into();
            false
        }
        MergeIntoHeadResult::FastForward => {
            state.status = format!("Fast-forwarded to {label}");
            true
        }
        MergeIntoHeadResult::MergeCommit => {
            state.status = format!("Merged {label}");
            true
        }
        MergeIntoHeadResult::Conflict => {
            state.status = "Merge conflict — resolve in the worktree".into();
            true
        }
        MergeIntoHeadResult::Failed(err) => {
            state.status = format!("merge failed: {err}");
            false
        }
    }
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
        Effect::LoadRightPane => load_right(state),
        Effect::ReloadSnapshot => {
            reload_snapshot(state, opts);
            state.status = "refreshed workspace".into();
            load_right(state);
        }
        Effect::ReloadRepo { repo } => {
            reload_repo(state, opts, &repo);
            state.status = format!("refreshed {repo}");
            load_right(state);
        }
        Effect::LoadCommitFiles { repo, source } => {
            load_commit_files(state, opts, repo, source);
        }
        Effect::LoadCommitDiff { repo, source, path } => {
            load_commit_diff(state, opts, repo, source, path);
        }
        _ => {}
    }
}

fn load_commit_files(state: &mut AppState, opts: &TuiOpts, repo: String, source: CommitFileSource) {
    let dir = opts.cwd.join(&repo);
    let files = match &source {
        CommitFileSource::Commit { commit_id } => list_commit_name_status(&dir, commit_id),
        CommitFileSource::Stash { stash_ref } => list_stash_name_status(&dir, stash_ref),
        CommitFileSource::Worktree => list_worktree_name_status(&dir),
    };
    state.open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
}

fn load_commit_diff(
    state: &mut AppState,
    opts: &TuiOpts,
    repo: String,
    source: CommitFileSource,
    path: String,
) {
    let dir = opts.cwd.join(&repo);
    let context = state.commit_diff_context(&repo, &path);
    let content = match &source {
        CommitFileSource::Commit { commit_id } => {
            DiffContent::from_lines(diff_commit_file_ctx(&dir, commit_id, &path, context))
        }
        CommitFileSource::Stash { stash_ref } => {
            DiffContent::from_lines(diff_stash_file_ctx(&dir, stash_ref, &path, context))
        }
        CommitFileSource::Worktree => {
            if let Some((_, change)) = state.focused_file() {
                if change.path == path {
                    load_file_diff(&state.cwd, &repo, &change, context)
                } else {
                    head_file_diff(&dir, &path, context)
                }
            } else {
                head_file_diff(&dir, &path, context)
            }
        }
    };
    let (files, file_cursor) = match &state.drill {
        super::drill::DrillView::Files { files, cursor, .. } => (files.clone(), *cursor),
        super::drill::DrillView::Diff {
            files, file_cursor, ..
        } => (files.clone(), *file_cursor),
        super::drill::DrillView::Graph => (Vec::new(), 0),
    };
    state.open_commit_diff(repo, source, files, file_cursor, path, content);
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

fn sync_mouse_capture(enabled: bool) {
    let mut out = stdout();
    if enabled {
        let _ = execute!(out, EnableMouseCapture);
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
        execute!(out, EnterAlternateScreen, EnableMouseCapture).map_err(|e| e.to_string())?;
    } else {
        execute!(out, EnterAlternateScreen).map_err(|e| e.to_string())?;
    }
    let _ = terminal.hide_cursor();
    let _ = terminal.resize(terminal_size_rect());
    let _ = terminal.clear();
    drain_pending_events();
    Ok(())
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

/// Refresh one checkout in place. Missing paths drop out of the snapshot
/// (focused checkout only).
fn reload_repo(state: &mut AppState, opts: &TuiOpts, repo: &str) {
    let existing = state.snapshot.repos.iter().find(|row| row.repo == repo);
    let meta = RepoCheckoutMeta {
        checkout_kind: existing
            .map(|row| row.checkout_kind)
            .unwrap_or(CheckoutKind::Primary),
        primary_repo: existing.and_then(|row| row.primary_repo.clone()),
    };
    let override_name = existing.and_then(|row| row.default_branch_override.clone());
    let mut snaps = repo_snapshots_from_workspace(&state.snapshot);
    match process_repo(repo, &opts.cwd, false, override_name.as_deref(), &meta) {
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
    let snapshot = build_workspace_snapshot(
        &snaps,
        &state.snapshot.ignored_repos,
        state.show_ignored,
        &state.snapshot.filter_repos,
    );
    state.apply_watch_snapshot(snapshot);
}

fn load_right(state: &mut AppState) {
    let worktree = RightPaneRequest::worktree_refresh(&state.drill);
    let payload = RightPaneRequest::from_state(state).compute(worktree);
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
        match run_work_pumped(terminal, state, move || {
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
    use crate::git::{exec_git, git_binary, list_local_branches};
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
        load_right(&mut app);
        assert!(
            app.graph.is_some(),
            "repo row must load a graph from pane git"
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
        let worktree = RightPaneRequest::worktree_refresh(&app.drill);
        let (tx, rx) = mpsc::sync_channel(1);
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(80));
            let payload = request.compute(worktree);
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
        let _ = app.apply_watch_snapshot(snapshot);
        assert_eq!(
            before, app.signatures,
            "identical watch snapshot must keep signatures so load_right can be skipped"
        );
        let _ = fs::remove_dir_all(&root);
    }
}

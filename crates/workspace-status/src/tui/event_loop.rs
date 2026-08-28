//! One async TTY loop: input, timers, `JoinSet` results, and a small presenter.
//!
//! `run_tui` stays synchronous via a current-thread Tokio runtime. Terminal
//! bytes are read on a dedicated thread through [`super::tty::poll_event`] /
//! [`super::tty::read_event`]. Every git/process effect runs on
//! `spawn_blocking`. The loop thread only dispatches, applies results, and draws.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::process::{Command, Stdio};
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration as TokioDuration};
use workspace_status_graph::LOADING_OLDER;

use crate::actions::switch_repo_to_default_branch;
use crate::discovery::{discover_checkouts, process_repo, RepoCheckoutMeta};
use crate::git::{
    create_branch_at, create_branch_checkout, exec_git_checked, latest_stash_ref,
    list_local_branches, pull_quiet_detailed, push_quiet, remove_untracked_file, remove_worktree,
    revert_tracked_file, stage_file, stash_apply, stash_drop, stash_pop, stash_push, unstage_file,
};
use crate::parallel::env_fetch_concurrency;
use crate::snapshot::RepoSnapshot;

use super::action::{Action, Effect};
use super::app::{
    apply_checkout_compute, apply_merge_compute, apply_one_repo_snapshot, apply_right_pane_load,
    apply_terminal_resize, commit_diff_list, compute_checkout, compute_commit_diff,
    compute_commit_files, compute_merge, compute_reload_repo, discard_held_nav_backlog,
    discover_config, drop_undiscovered_checkouts, filter_repo_set, focused_repo_needs_pane,
    map_event, run_blocking_editor, sync_mouse_capture, RightPaneRequest, RightPaneTarget, TuiOpts,
};
use super::drill::CommitFileSource;
use super::editor::{editor_command, is_detached_editor, resolve_editor};
use super::event_pump::{
    action_triggers_graph_autoload, classify_busy_action, overlay_blocks_background_ticks,
    BusyAction,
};
use super::fetch::fetch_interval_ms;
use super::graph_load::{
    autoload_limit, autoload_skip, load_graph_model_window, merge_autoload, should_autoload,
    GraphIdentity, ShouldAutoload,
};
use super::keys::held_nav_key;
use super::ops::{format_completed_op, format_running_op, RunningOp};
use super::render::draw;
use super::scheduler::{ApplyDecision, Scheduler, SpawnKind, UserTag};
use super::state::AppState;
use super::tty::{poll_event, read_event};
use super::watch::{watch_interval_ms, watch_remain_ms, FLASH_TICK_MS};

const INPUT_BATCH: usize = 8;
const DRAW_MIN_MS: u64 = 16;
const DRAW_MAX_MS: u64 = 33;

enum InputCmd {
    Pause,
    Resume,
    Shutdown,
}

struct InputBridge {
    rx: tokio::sync::mpsc::Receiver<crossterm::event::Event>,
    cmd: std_mpsc::Sender<InputCmd>,
    ack: std_mpsc::Receiver<()>,
}

impl InputBridge {
    fn spawn() -> Self {
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let (cmd_tx, cmd_rx) = std_mpsc::channel();
        let (ack_tx, ack_rx) = std_mpsc::channel();
        thread::spawn(move || input_thread(tx, cmd_rx, ack_tx));
        Self {
            rx,
            cmd: cmd_tx,
            ack: ack_rx,
        }
    }

    fn pause(&mut self) {
        let _ = self.cmd.send(InputCmd::Pause);
        let _ = self.ack.recv();
    }

    fn resume(&mut self) {
        let _ = self.cmd.send(InputCmd::Resume);
        let _ = self.ack.recv();
    }

    fn shutdown(&mut self) {
        let _ = self.cmd.send(InputCmd::Shutdown);
    }
}

fn input_thread(
    tx: tokio::sync::mpsc::Sender<crossterm::event::Event>,
    cmd_rx: std_mpsc::Receiver<InputCmd>,
    ack_tx: std_mpsc::Sender<()>,
) {
    let mut paused = false;
    loop {
        if paused {
            match cmd_rx.recv() {
                Ok(InputCmd::Resume) => {
                    paused = false;
                    let _ = ack_tx.send(());
                }
                Ok(InputCmd::Pause) => {
                    let _ = ack_tx.send(());
                }
                Ok(InputCmd::Shutdown) | Err(_) => break,
            }
            continue;
        }
        match cmd_rx.try_recv() {
            Ok(InputCmd::Pause) => {
                paused = true;
                let _ = ack_tx.send(());
                continue;
            }
            Ok(InputCmd::Shutdown) => break,
            Ok(InputCmd::Resume) => {}
            Err(std_mpsc::TryRecvError::Empty) => {}
            Err(std_mpsc::TryRecvError::Disconnected) => break,
        }
        if !poll_event(Duration::from_millis(16)).unwrap_or(false) {
            continue;
        }
        let Ok(event) = read_event() else {
            break;
        };
        let leftover = held_nav_key(&event).and_then(discard_held_nav_backlog);
        if tx.blocking_send(event).is_err() {
            break;
        }
        if let Some(extra) = leftover {
            if tx.blocking_send(extra).is_err() {
                break;
            }
        }
    }
}

struct Presenter {
    dirty: bool,
    last_draw: Instant,
}

impl Presenter {
    fn new() -> Self {
        Self {
            dirty: true,
            last_draw: Instant::now() - Duration::from_millis(DRAW_MAX_MS),
        }
    }

    fn mark(&mut self) {
        self.dirty = true;
    }

    fn remain_ms(&self, flashes: bool) -> u64 {
        if flashes {
            return FLASH_TICK_MS.min(self.dirty_remain());
        }
        if !self.dirty {
            return u64::MAX;
        }
        self.dirty_remain()
    }

    fn dirty_remain(&self) -> u64 {
        let elapsed = self.last_draw.elapsed().as_millis() as u64;
        if elapsed >= DRAW_MAX_MS {
            0
        } else if elapsed >= DRAW_MIN_MS {
            0
        } else {
            DRAW_MIN_MS.saturating_sub(elapsed)
        }
    }

    fn should_draw(&self) -> bool {
        self.dirty && self.last_draw.elapsed().as_millis() as u64 >= DRAW_MIN_MS
    }

    fn painted(&mut self) {
        self.dirty = false;
        self.last_draw = Instant::now();
    }
}

enum JobOutcome {
    Discovered {
        gen: u64,
        entries: Vec<(String, RepoCheckoutMeta, Option<String>)>,
    },
    RepoStatus {
        gen: u64,
        path: String,
        snap: Option<RepoSnapshot>,
    },
    RightPane {
        req_id: u64,
        target: RightPaneTarget,
        load: super::app::RightPaneLoad,
    },
    Write {
        status: String,
    },
    BulkRemote {
        kind: RunningOp,
        ok: bool,
    },
    DefaultBranch {
        ok: bool,
    },
    PrepareStash {
        repo: String,
        latest: Option<String>,
    },
    PrepareBranches {
        repo: String,
        branches: Vec<crate::git::LocalBranch>,
        graph_focus: bool,
    },
    Checkout {
        repo: String,
        result: super::app::CheckoutCompute,
    },
    Merge {
        label: String,
        result: super::app::MergeCompute,
    },
    Autoload {
        page: workspace_status_graph::GraphModel,
        identity: GraphIdentity,
        prev_status: String,
    },
    CommitFiles {
        repo: String,
        source: CommitFileSource,
        files: Vec<crate::git::NameStatus>,
    },
    CommitDiff {
        repo: String,
        source: CommitFileSource,
        files: Vec<super::drill::CommitFile>,
        file_cursor: usize,
        path: String,
        content: super::diff::DiffContent,
    },
}

struct BulkState {
    kind: RunningOp,
    remaining: VecDeque<String>,
    inflight: usize,
    done: usize,
    ok: usize,
    failed: usize,
    repos: Vec<String>,
}

struct WriteJob {
    work: Box<dyn FnOnce() -> Result<String, String> + Send>,
}

struct LoopCtx<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &'a mut AppState,
    opts: &'a TuiOpts,
    sched: Scheduler,
    join: JoinSet<(u64, JobOutcome)>,
    presenter: Presenter,
    input: InputBridge,
    last_watch: Instant,
    last_fetch: Instant,
    watch_ms: u64,
    fetch_ms: u64,
    metas: HashMap<String, (RepoCheckoutMeta, Option<String>)>,
    pane_req: Option<RightPaneRequest>,
    writes: VecDeque<WriteJob>,
    bulk: Option<BulkState>,
    default_queue: VecDeque<String>,
    prepare_stash: Option<String>,
    prepare_branches: Option<(String, bool)>,
    checkout: Option<(String, String, Option<String>)>,
    merge: Option<(String, String, String)>,
    commit_files: Option<(String, CommitFileSource)>,
    commit_diff: Option<(String, CommitFileSource, String)>,
    autoload: Option<String>,
    default_ok: usize,
    default_failed: usize,
    default_total: usize,
    default_repos: Vec<String>,
    quit: bool,
}

/// Run until quit. Caller owns terminal restore.
pub async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
) -> Result<(), u8> {
    let input = InputBridge::spawn();
    let mut presenter = Presenter::new();
    terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
    presenter.painted();

    let mut ctx = LoopCtx {
        terminal,
        state,
        opts,
        sched: Scheduler::new(env_fetch_concurrency()),
        join: JoinSet::new(),
        presenter,
        input,
        last_watch: Instant::now(),
        last_fetch: Instant::now(),
        watch_ms: watch_interval_ms(std::env::var("WS_STATUS_WATCH_MS").ok().as_deref()),
        fetch_ms: fetch_interval_ms(std::env::var("WS_STATUS_FETCH_MS").ok().as_deref()),
        metas: HashMap::new(),
        pane_req: None,
        writes: VecDeque::new(),
        bulk: None,
        default_queue: VecDeque::new(),
        prepare_stash: None,
        prepare_branches: None,
        checkout: None,
        merge: None,
        commit_files: None,
        commit_diff: None,
        autoload: None,
        default_ok: 0,
        default_failed: 0,
        default_total: 0,
        default_repos: Vec::new(),
        quit: false,
    };
    schedule_effect(&mut ctx, Effect::LoadRightPane, &Action::None);
    if opts.start_fetch {
        let effect = ctx.state.dispatch(Action::Fetch);
        schedule_effect(&mut ctx, effect, &Action::Fetch);
    }

    while !ctx.quit {
        spawn_ready(&mut ctx);
        let now = Instant::now();
        let watch_remain = if overlay_blocks_background_ticks(ctx.state.input_mode()) {
            u64::MAX
        } else {
            watch_remain_ms(ctx.last_watch, now, ctx.watch_ms)
        };
        let fetch_remain =
            if ctx.fetch_ms == 0 || overlay_blocks_background_ticks(ctx.state.input_mode()) {
                u64::MAX
            } else {
                ctx.fetch_ms
                    .saturating_sub(ctx.last_fetch.elapsed().as_millis() as u64)
            };
        let ctrl_remain = ctx.state.ctrl_c_remaining_ms(now).unwrap_or(u64::MAX);
        let present_remain = ctx.presenter.remain_ms(ctx.state.has_active_flashes());
        let join_empty = ctx.join.is_empty();

        tokio::select! {
            event = ctx.input.rx.recv() => {
                let Some(event) = event else { break; };
                handle_input(&mut ctx, event);
                for _ in 1..INPUT_BATCH {
                    match ctx.input.rx.try_recv() {
                        Ok(next) => handle_input(&mut ctx, next),
                        Err(_) => break,
                    }
                }
            }
            Some(joined) = ctx.join.join_next(), if !join_empty => {
                match joined {
                    Ok((id, outcome)) => apply_outcome(&mut ctx, id, outcome),
                    Err(_) => {}
                }
            }
            _ = sleep_ms(watch_remain) => {
                ctx.last_watch = Instant::now();
                if !overlay_blocks_background_ticks(ctx.state.input_mode()) {
                    let effect = ctx.state.dispatch(Action::WatchTick);
                    schedule_effect(&mut ctx, effect, &Action::WatchTick);
                }
            }
            _ = sleep_ms(fetch_remain) => {
                ctx.last_fetch = Instant::now();
                if !overlay_blocks_background_ticks(ctx.state.input_mode())
                    && !ctx.sched.busy_for_writes()
                {
                    let effect = ctx.state.dispatch(Action::FetchTick);
                    schedule_effect(&mut ctx, effect, &Action::FetchTick);
                }
            }
            _ = sleep_ms(ctrl_remain) => {
                if ctx.state.expire_ctrl_c_prompt(Instant::now()) {
                    ctx.presenter.mark();
                }
            }
            _ = sleep_ms(present_remain) => {
                ctx.state.prune_expired_flashes();
                if ctx.presenter.should_draw() || ctx.state.has_active_flashes() {
                    ctx.terminal
                        .draw(|frame| draw(frame, ctx.state))
                        .map_err(|_| 1u8)?;
                    ctx.presenter.painted();
                    if ctx.state.has_active_flashes() {
                        ctx.presenter.mark();
                    }
                }
            }
        }
    }
    ctx.input.shutdown();
    Ok(())
}

async fn sleep_ms(ms: u64) {
    if ms == u64::MAX {
        std::future::pending::<()>().await;
    } else {
        sleep(TokioDuration::from_millis(ms)).await;
    }
}

fn handle_input(ctx: &mut LoopCtx<'_>, event: crossterm::event::Event) {
    let action = map_event(ctx.state, &event);
    if ctx.sched.busy_for_writes() {
        match classify_busy_action(&action) {
            BusyAction::Quit => {
                ctx.quit = true;
                return;
            }
            BusyAction::Resize { cols, rows } => {
                let _ = apply_terminal_resize(ctx.terminal, cols, rows);
                let _ = ctx.state.dispatch(Action::Resize { cols, rows });
                ctx.presenter.mark();
                return;
            }
            BusyAction::Ignore => return,
            BusyAction::Handle => {}
        }
    }
    if let Action::Resize { cols, rows } = &action {
        let _ = apply_terminal_resize(ctx.terminal, *cols, *rows);
    }
    let mouse_before = ctx.state.mouse_enabled;
    let action_for_load = action.clone();
    let effect = ctx.state.dispatch(action);
    if ctx.state.mouse_enabled != mouse_before {
        sync_mouse_capture(ctx.state.mouse_enabled);
    }
    if matches!(effect, Effect::Quit) {
        ctx.quit = true;
        return;
    }
    schedule_effect(ctx, effect, &action_for_load);
    if action_triggers_graph_autoload(&action_for_load) {
        maybe_queue_autoload(ctx);
    }
    ctx.presenter.mark();
}

fn maybe_queue_autoload(ctx: &mut LoopCtx<'_>) {
    if ctx.state.graph_loading_older {
        return;
    }
    if ctx.state.right_is_diff() && !ctx.state.in_commit_drill() {
        return;
    }
    let Some(model) = ctx.state.graph.as_ref() else {
        return;
    };
    if !should_autoload(ShouldAutoload {
        cursor_index: ctx.state.graph_cursor,
        loaded_count: model.visible_rows().len(),
        has_more: model.has_more,
        loading: false,
    }) {
        return;
    }
    let Some((repo, _)) = ctx.state.graph_identity.as_ref() else {
        return;
    };
    ctx.state.graph_loading_older = true;
    ctx.autoload = Some(repo.clone());
    ctx.state.status = LOADING_OLDER.to_string();
    ctx.sched.enqueue_user(UserTag::Autoload);
    ctx.presenter.mark();
}

fn schedule_effect(ctx: &mut LoopCtx<'_>, effect: Effect, action: &Action) {
    match effect {
        Effect::None | Effect::Quit => {}
        Effect::Batch(effects) => {
            for child in effects {
                schedule_effect(ctx, child, action);
            }
        }
        Effect::WatchRefresh => {
            ctx.sched.on_watch_tick(ctx.state.focused_checkout_path());
        }
        Effect::ReloadSnapshot => {
            ctx.sched
                .on_reload_snapshot(ctx.state.focused_checkout_path());
        }
        Effect::ReloadRepo { repo } => {
            ctx.sched.on_reload_repo(repo);
        }
        Effect::LoadRightPane => {
            ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
            ctx.sched.request_pane();
        }
        Effect::Fetch { repos } => start_bulk(ctx, RunningOp::Fetch, repos),
        Effect::Pull { repos } => start_bulk(ctx, RunningOp::Pull, repos),
        Effect::Push { repos } => start_bulk(ctx, RunningOp::Push, repos),
        Effect::DefaultBranch { repos } => {
            ctx.default_total = repos.len();
            ctx.default_ok = 0;
            ctx.default_failed = 0;
            ctx.default_repos = repos.clone();
            ctx.state.status = format_running_op(RunningOp::DefaultBranch, 0, repos.len());
            ctx.presenter.mark();
            ctx.default_queue = repos.into();
            if !ctx.default_queue.is_empty() {
                ctx.sched.enqueue_user(UserTag::DefaultBranch);
            }
        }
        Effect::Stage { repo, paths } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let last = paths.last().cloned().unwrap_or_default();
            Box::new(move || {
                for path in &paths {
                    stage_file(&dir, path)?;
                }
                Ok(format!("staged {last}"))
            })
        }),
        Effect::Unstage { repo, paths } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let last = paths.last().cloned().unwrap_or_default();
            Box::new(move || {
                for path in &paths {
                    unstage_file(&dir, path)?;
                }
                Ok(format!("unstaged {last}"))
            })
        }),
        Effect::Revert {
            repo,
            tracked,
            untracked,
        } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
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
            Box::new(move || {
                for path in &tracked {
                    revert_tracked_file(&dir, path)?;
                }
                for path in &untracked {
                    remove_untracked_file(&dir, path)?;
                }
                Ok(ok_status)
            })
        }),
        Effect::EditFile { repo, path } => schedule_edit(ctx, repo, path),
        Effect::StashCreate { repo, paths } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let ok_status = if paths.len() == 1 {
                "Stashed 1 file".to_string()
            } else if paths.is_empty() {
                "Stashed".to_string()
            } else {
                format!("Stashed {} files", paths.len())
            };
            Box::new(move || stash_push(&dir, &paths).map(|_| ok_status))
        }),
        Effect::StashApply { repo, stash_ref } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let label = stash_ref.clone();
            Box::new(move || stash_apply(&dir, &stash_ref).map(|_| format!("applied {label}")))
        }),
        Effect::StashPop { repo, stash_ref } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let label = stash_ref.clone();
            Box::new(move || stash_pop(&dir, &stash_ref).map(|_| format!("popped {label}")))
        }),
        Effect::StashDrop { repo, stash_ref } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let label = stash_ref.clone();
            Box::new(move || stash_drop(&dir, &stash_ref).map(|_| format!("dropped {label}")))
        }),
        Effect::PrepareStashMenu { repo } => {
            ctx.prepare_stash = Some(repo);
            ctx.sched.enqueue_user(UserTag::Prepare);
        }
        Effect::PrepareBranchPicker { repo } => {
            ctx.prepare_branches = Some((repo, false));
            ctx.sched.enqueue_user(UserTag::Prepare);
        }
        Effect::PrepareGraphFocusPicker { repo } => {
            ctx.prepare_branches = Some((repo, true));
            ctx.sched.enqueue_user(UserTag::Prepare);
        }
        Effect::CheckoutBranch {
            repo,
            selected_name,
            fast_forward_ref,
        } => {
            ctx.checkout = Some((repo, selected_name, fast_forward_ref));
            ctx.sched.enqueue_user(UserTag::Write);
        }
        Effect::CreateBranch { repo, name } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let label = name.clone();
            Box::new(move || {
                create_branch_checkout(&dir, &name).map(|_| format!("created {label}"))
            })
        }),
        Effect::CreateBranchAt {
            repo,
            name,
            commit_id,
        } => enqueue_write(ctx, {
            let dir = ctx.opts.cwd.join(&repo);
            let label = name.clone();
            let short = commit_id.get(..7).unwrap_or(&commit_id).to_string();
            Box::new(move || {
                create_branch_at(&dir, &name, &commit_id)
                    .map(|_| format!("created {label} at {short}"))
            })
        }),
        Effect::MergeIntoHead { repo, rev, label } => {
            ctx.merge = Some((repo, rev, label));
            ctx.sched.enqueue_user(UserTag::Write);
        }
        Effect::RemoveWorktree {
            primary,
            path,
            force,
        } => enqueue_write(ctx, {
            let primary_dir = ctx.opts.cwd.join(&primary);
            let path_dir = ctx.opts.cwd.join(&path);
            let label = path.clone();
            Box::new(move || {
                remove_worktree(&primary_dir, &path_dir, force)
                    .map(|_| format!("removed worktree {label}"))
            })
        }),
        Effect::LoadCommitFiles { repo, source } => {
            ctx.state.begin_commit_files(repo.clone(), source.clone());
            ctx.commit_files = Some((repo, source));
            ctx.sched.enqueue_user_front(UserTag::Pane);
            ctx.presenter.mark();
        }
        Effect::LoadCommitDiff { repo, source, path } => {
            ctx.commit_diff = Some((repo, source, path));
            ctx.sched.enqueue_user_front(UserTag::Pane);
        }
    }
}

fn enqueue_write(ctx: &mut LoopCtx<'_>, work: Box<dyn FnOnce() -> Result<String, String> + Send>) {
    let _ = ctx.sched.bump_write_gen();
    ctx.writes.push_back(WriteJob { work });
    ctx.sched.enqueue_user(UserTag::Write);
}

fn start_bulk(ctx: &mut LoopCtx<'_>, kind: RunningOp, repos: Vec<String>) {
    if repos.is_empty() {
        return;
    }
    ctx.state.status = format_running_op(kind, 0, repos.len());
    ctx.presenter.mark();
    let n = repos.len();
    ctx.bulk = Some(BulkState {
        kind,
        remaining: repos.iter().cloned().collect(),
        inflight: 0,
        done: 0,
        ok: 0,
        failed: 0,
        repos,
    });
    for _ in 0..n {
        ctx.sched.enqueue_user(UserTag::BulkRemote);
    }
}

fn schedule_edit(ctx: &mut LoopCtx<'_>, repo: String, path: String) {
    let editor = resolve_editor(
        ctx.opts.config.editor.as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
        std::env::var("VISUAL").ok().as_deref(),
    );
    let abs = ctx.opts.cwd.join(&repo).join(&path);
    let (cmd, args) = editor_command(&editor, &abs.to_string_lossy(), None);
    if is_detached_editor(&editor) {
        let _ = Command::new(&cmd)
            .args(&args)
            .current_dir(ctx.opts.cwd.join(&repo))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        ctx.state.status = format!("opened {path}");
        ctx.presenter.mark();
        return;
    }
    ctx.input.pause();
    let mouse = ctx.state.mouse_enabled;
    let result = run_blocking_editor(ctx.terminal, &cmd, &args, &ctx.opts.cwd.join(&repo), mouse);
    ctx.input.resume();
    match result {
        Err(err) => {
            ctx.state.status = format!("edit failed: {err}");
            ctx.presenter.mark();
        }
        Ok(()) => {
            ctx.state.status = format!("edited {path}");
            ctx.sched.on_reload_repo(repo);
            ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
            ctx.sched.request_pane();
            ctx.presenter.mark();
        }
    }
}

fn spawn_ready(ctx: &mut LoopCtx<'_>) {
    while let Some(req) = ctx.sched.spawn_next() {
        let id = req.id;
        match req.kind {
            SpawnKind::Discover { gen, .. } => {
                let cwd = ctx.opts.cwd.clone();
                let config = discover_config(&ctx.opts.config);
                let filter = ctx.state.snapshot.filter_repos.clone();
                ctx.join.spawn_blocking(move || {
                    let only = filter_repo_set(&filter);
                    let entries = discover_checkouts(&cwd, &config, only.as_ref());
                    (id, JobOutcome::Discovered { gen, entries })
                });
            }
            SpawnKind::ProcessRepo { gen, path, .. } => {
                let cwd = ctx.opts.cwd.clone();
                let snapshot = ctx.state.snapshot.clone();
                let show_ignored = ctx.state.show_ignored;
                let meta = ctx.metas.get(&path).cloned().or_else(|| {
                    snapshot.repos.iter().find(|r| r.repo == path).map(|row| {
                        (
                            RepoCheckoutMeta {
                                checkout_kind: row.checkout_kind,
                                primary_repo: row.primary_repo.clone(),
                            },
                            row.default_branch_override.clone(),
                        )
                    })
                });
                ctx.join.spawn_blocking(move || {
                    let snap = if let Some((meta, override_name)) = meta {
                        process_repo(&path, &cwd, false, override_name.as_deref(), &meta)
                    } else {
                        let next = compute_reload_repo(&cwd, &snapshot, &path, show_ignored);
                        next.repos
                            .into_iter()
                            .find(|row| row.repo == path)
                            .map(|row| RepoSnapshot {
                                repo: row.repo,
                                branch: row.branch,
                                sync_status: row.sync_status,
                                sync_note: row.sync_note,
                                head: row.head,
                                has_unstaged: row.has_unstaged,
                                has_staged: row.has_staged,
                                has_untracked: row.has_untracked,
                                changes: row.changes,
                                checkout_kind: row.checkout_kind,
                                primary_repo: row.primary_repo,
                                merged_into_default: row.merged_into_default,
                                default_branch_override: row.default_branch_override,
                            })
                    };
                    (id, JobOutcome::RepoStatus { gen, path, snap })
                });
            }
            SpawnKind::LoadPane { req_id } => {
                let request = ctx
                    .pane_req
                    .clone()
                    .unwrap_or_else(|| RightPaneRequest::from_state(ctx.state));
                let target = request.target();
                ctx.join.spawn_blocking(move || {
                    let load = request.compute();
                    (
                        id,
                        JobOutcome::RightPane {
                            req_id,
                            target,
                            load,
                        },
                    )
                });
            }
            SpawnKind::UserWork { tag } => spawn_user(ctx, id, tag),
        }
    }
}

fn spawn_user(ctx: &mut LoopCtx<'_>, id: u64, tag: UserTag) {
    match tag {
        UserTag::Write => {
            if let Some((repo, name, ff)) = ctx.checkout.take() {
                let dir = ctx.opts.cwd.join(&repo);
                ctx.join.spawn_blocking(move || {
                    let result = compute_checkout(&dir, &name, ff.as_deref());
                    (id, JobOutcome::Checkout { repo, result })
                });
                return;
            }
            if let Some((repo, rev, label)) = ctx.merge.take() {
                let dir = ctx.opts.cwd.join(&repo);
                ctx.join.spawn_blocking(move || {
                    let result = compute_merge(&dir, &rev);
                    (id, JobOutcome::Merge { label, result })
                });
                return;
            }
            if let Some(job) = ctx.writes.pop_front() {
                ctx.join.spawn_blocking(move || {
                    let status = match (job.work)() {
                        Ok(s) => s,
                        Err(err) => err,
                    };
                    (id, JobOutcome::Write { status })
                });
                return;
            }
            ctx.sched.note_job_finished(id);
            ctx.sched.note_user_done(UserTag::Write);
        }
        UserTag::BulkRemote => {
            let Some(bulk) = ctx.bulk.as_mut() else {
                ctx.sched.note_job_finished(id);
                return;
            };
            let Some(repo) = bulk.remaining.pop_front() else {
                ctx.sched.note_job_finished(id);
                return;
            };
            bulk.inflight += 1;
            let kind = bulk.kind;
            let dir = ctx.opts.cwd.join(&repo);
            ctx.join.spawn_blocking(move || {
                let ok = match kind {
                    RunningOp::Fetch => exec_git_checked(&["fetch", "--quiet"], &dir).is_ok(),
                    RunningOp::Pull => pull_quiet_detailed(&dir).ok,
                    RunningOp::Push => push_quiet(&dir).is_ok(),
                    RunningOp::DefaultBranch => false,
                };
                (id, JobOutcome::BulkRemote { kind, ok })
            });
        }
        UserTag::DefaultBranch => {
            let Some(repo) = ctx.default_queue.pop_front() else {
                ctx.sched.note_job_finished(id);
                ctx.sched.note_user_done(UserTag::DefaultBranch);
                return;
            };
            let task = ctx
                .state
                .snapshot
                .repos
                .iter()
                .find(|r| r.repo == repo)
                .map(|snap| (snap.branch.clone(), snap.default_branch_override.clone()));
            let cwd = ctx.opts.cwd.clone();
            ctx.join.spawn_blocking(move || {
                let ok = match task {
                    Some((branch, override_name)) => {
                        switch_repo_to_default_branch(
                            &repo,
                            &branch,
                            &cwd,
                            override_name.as_deref(),
                        )
                        .0
                    }
                    None => false,
                };
                (id, JobOutcome::DefaultBranch { ok })
            });
        }
        UserTag::Prepare => {
            if let Some(repo) = ctx.prepare_stash.take() {
                let dir = ctx.opts.cwd.join(&repo);
                ctx.join.spawn_blocking(move || {
                    let latest = latest_stash_ref(&dir);
                    (id, JobOutcome::PrepareStash { repo, latest })
                });
                return;
            }
            if let Some((repo, graph_focus)) = ctx.prepare_branches.take() {
                let dir = ctx.opts.cwd.join(&repo);
                ctx.join.spawn_blocking(move || {
                    let branches = list_local_branches(&dir);
                    (
                        id,
                        JobOutcome::PrepareBranches {
                            repo,
                            branches,
                            graph_focus,
                        },
                    )
                });
                return;
            }
            ctx.sched.note_job_finished(id);
            ctx.sched.note_user_done(UserTag::Prepare);
        }
        UserTag::Pane => {
            if let Some((repo, source)) = ctx.commit_files.take() {
                let dir = ctx.opts.cwd.join(&repo);
                let source_work = source.clone();
                ctx.join.spawn_blocking(move || {
                    let files = compute_commit_files(&dir, &source_work);
                    (
                        id,
                        JobOutcome::CommitFiles {
                            repo,
                            source,
                            files,
                        },
                    )
                });
                return;
            }
            if let Some((repo, source, path)) = ctx.commit_diff.take() {
                let context = ctx.state.commit_diff_context(&repo, &path);
                let focused = ctx.state.focused_file();
                let (files, file_cursor) = commit_diff_list(ctx.state);
                let cwd = ctx.opts.cwd.clone();
                let repo_w = repo.clone();
                let source_w = source.clone();
                let path_w = path.clone();
                ctx.join.spawn_blocking(move || {
                    let content = compute_commit_diff(
                        &cwd,
                        &repo_w,
                        &source_w,
                        &path_w,
                        context,
                        focused.as_ref(),
                    );
                    (
                        id,
                        JobOutcome::CommitDiff {
                            repo,
                            source,
                            files,
                            file_cursor,
                            path,
                            content,
                        },
                    )
                });
                return;
            }
            ctx.sched.note_job_finished(id);
        }
        UserTag::Autoload => {
            let Some(repo) = ctx.autoload.take() else {
                ctx.sched.note_job_finished(id);
                ctx.state.graph_loading_older = false;
                return;
            };
            let Some(model) = ctx.state.graph.as_ref() else {
                ctx.sched.note_job_finished(id);
                ctx.state.graph_loading_older = false;
                return;
            };
            let skip = autoload_skip(model);
            let limit = autoload_limit(model);
            let cwd = ctx.opts.cwd.clone();
            let snapshot = ctx.state.snapshot.clone();
            let show_ignored = ctx.state.show_ignored;
            let focus = ctx.state.graph_focus_revs();
            let prev_status = ctx.state.status.clone();
            ctx.join.spawn_blocking(move || {
                let (page, identity) = load_graph_model_window(
                    &cwd,
                    &snapshot,
                    &repo,
                    show_ignored,
                    skip,
                    limit,
                    &focus,
                );
                (
                    id,
                    JobOutcome::Autoload {
                        page,
                        identity,
                        prev_status,
                    },
                )
            });
        }
    }
}

fn apply_outcome(ctx: &mut LoopCtx<'_>, id: u64, outcome: JobOutcome) {
    ctx.sched.note_job_finished(id);
    match outcome {
        JobOutcome::Discovered { gen, entries } => {
            if ctx
                .sched
                .on_discovered(gen, entries.iter().map(|(p, _, _)| p.clone()).collect())
                == ApplyDecision::Ignore
            {
                return;
            }
            let keep: Vec<String> = entries.iter().map(|(p, _, _)| p.clone()).collect();
            drop_undiscovered_checkouts(ctx.state, &keep);
            ctx.metas = entries
                .into_iter()
                .map(|(path, meta, ov)| (path, (meta, ov)))
                .collect();
            ctx.presenter.mark();
        }
        JobOutcome::RepoStatus { gen, path, snap } => {
            if !ctx.sched.accept_repo_result(gen, &path) {
                return;
            }
            let before_sigs = ctx.state.signatures.clone();
            let before_snap = ctx.state.snapshot.clone();
            let focused = ctx.state.focused_checkout_path();
            apply_one_repo_snapshot(ctx.state, &path, snap);
            let decision = ctx.sched.note_repo_done(gen, &path);
            if focused.as_deref() == Some(path.as_str())
                && focused_repo_needs_pane(&before_sigs, &before_snap, ctx.state, &path)
            {
                ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
                ctx.sched.request_pane();
            }
            if let ApplyDecision::StartDiscover { .. } = decision {
                // latched collect already queued
            }
            ctx.presenter.mark();
        }
        JobOutcome::RightPane {
            req_id,
            target,
            load,
        } => {
            let accepted = ctx.sched.accept_pane_result(req_id);
            let current = RightPaneRequest::from_state(ctx.state).target();
            if accepted && target == current {
                apply_right_pane_load(ctx.state, load);
                ctx.presenter.mark();
            } else if current != target {
                ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
                ctx.sched.request_pane();
            }
        }
        JobOutcome::Write { status } => {
            ctx.sched.note_user_done(UserTag::Write);
            ctx.state.status = status;
            ctx.sched
                .on_reload_snapshot(ctx.state.focused_checkout_path());
            ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
            ctx.sched.request_pane();
            ctx.presenter.mark();
        }
        JobOutcome::BulkRemote { kind, ok } => {
            if let Some(bulk) = ctx.bulk.as_mut() {
                bulk.inflight = bulk.inflight.saturating_sub(1);
                bulk.done += 1;
                if ok {
                    bulk.ok += 1;
                } else {
                    bulk.failed += 1;
                }
                ctx.state.status = format_running_op(kind, bulk.done, bulk.repos.len());
                ctx.presenter.mark();
                if bulk.remaining.is_empty() && bulk.inflight == 0 {
                    let repos = bulk.repos.clone();
                    let ok_n = bulk.ok;
                    let failed = bulk.failed;
                    ctx.bulk = None;
                    ctx.state.stamp_checkout_flashes(&repos);
                    ctx.state.status = format_completed_op(kind, ok_n, failed);
                    ctx.sched
                        .on_reload_snapshot(ctx.state.focused_checkout_path());
                    ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
                    ctx.sched.request_pane();
                }
            }
        }
        JobOutcome::DefaultBranch { ok } => {
            ctx.sched.note_user_done(UserTag::DefaultBranch);
            if ok {
                ctx.default_ok += 1;
            } else {
                ctx.default_failed += 1;
            }
            let done = ctx.default_ok + ctx.default_failed;
            ctx.state.status = format_running_op(RunningOp::DefaultBranch, done, ctx.default_total);
            if !ctx.default_queue.is_empty() {
                ctx.sched.enqueue_user(UserTag::DefaultBranch);
            } else {
                ctx.state.stamp_checkout_flashes(&ctx.default_repos);
                ctx.state.status = format_completed_op(
                    RunningOp::DefaultBranch,
                    ctx.default_ok,
                    ctx.default_failed,
                );
                ctx.default_repos.clear();
                ctx.sched
                    .on_reload_snapshot(ctx.state.focused_checkout_path());
                ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
                ctx.sched.request_pane();
            }
            ctx.presenter.mark();
        }
        JobOutcome::PrepareStash { repo, latest } => {
            ctx.sched.note_user_done(UserTag::Prepare);
            ctx.state.open_stash_menu(repo, latest);
            ctx.presenter.mark();
        }
        JobOutcome::PrepareBranches {
            repo,
            branches,
            graph_focus,
        } => {
            ctx.sched.note_user_done(UserTag::Prepare);
            if graph_focus {
                ctx.state.open_graph_focus_picker(repo, branches);
            } else {
                ctx.state.open_branch_picker(repo, branches);
            }
            ctx.presenter.mark();
        }
        JobOutcome::Checkout { repo, result } => {
            ctx.sched.note_user_done(UserTag::Write);
            if apply_checkout_compute(ctx.state, repo, result) {
                ctx.sched
                    .on_reload_snapshot(ctx.state.focused_checkout_path());
            }
            ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
            ctx.sched.request_pane();
            ctx.presenter.mark();
        }
        JobOutcome::Merge { label, result } => {
            ctx.sched.note_user_done(UserTag::Write);
            if apply_merge_compute(ctx.state, &label, result) {
                ctx.sched
                    .on_reload_snapshot(ctx.state.focused_checkout_path());
            }
            ctx.pane_req = Some(RightPaneRequest::from_state(ctx.state));
            ctx.sched.request_pane();
            ctx.presenter.mark();
        }
        JobOutcome::Autoload {
            page,
            identity,
            prev_status,
        } => {
            ctx.sched.note_user_done(UserTag::Autoload);
            if let Some(current) = ctx.state.graph.clone() {
                let merged = merge_autoload(&current, page);
                ctx.state.set_graph(merged, identity.repo, identity.head);
            }
            ctx.state.graph_loading_older = false;
            if ctx.state.status == LOADING_OLDER {
                ctx.state.status = prev_status;
            }
            ctx.presenter.mark();
        }
        JobOutcome::CommitFiles {
            repo,
            source,
            files,
        } => {
            ctx.state
                .open_commit_files(repo, source, files.into_iter().map(Into::into).collect());
            ctx.presenter.mark();
        }
        JobOutcome::CommitDiff {
            repo,
            source,
            files,
            file_cursor,
            path,
            content,
        } => {
            ctx.state
                .open_commit_diff(repo, source, files, file_cursor, path, content);
            ctx.presenter.mark();
        }
    }
}

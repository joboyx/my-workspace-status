//! One async TTY loop: input, timers, `JoinSet` results, and a small presenter.
//!
//! `run_tui` stays synchronous via a current-thread Tokio runtime. Terminal
//! bytes are read on a dedicated thread through [`super::tty::poll_event`] /
//! [`super::tty::read_event`]. Every git/process effect runs on
//! `spawn_blocking`. [`super::effect::Interpreter`] schedules and applies
//! those jobs. The loop thread only dispatches, applies results, and draws.

use std::io;
use std::process::{Command, Stdio};
use std::sync::mpsc as std_mpsc;
use std::thread;
use std::time::{Duration, Instant};

use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::task::JoinSet;
use tokio::time::{sleep, Duration as TokioDuration};

use super::action::{Action, Effect};
use super::app::{
    apply_terminal_resize, discard_held_nav_backlog, map_event, run_blocking_editor,
    sync_mouse_capture, TuiOpts,
};
use super::editor::{editor_command, is_detached_editor, resolve_editor};
use super::effect::{Interpreter, JobOutcome};
use super::event_pump::{
    action_triggers_graph_autoload, classify_busy_action, overlay_blocks_background_ticks,
    BusyAction,
};
use super::fetch::fetch_interval_ms;
use super::keys::held_nav_key;
use super::render::draw;
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

struct LoopCtx<'a> {
    terminal: &'a mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &'a mut AppState,
    opts: &'a TuiOpts,
    interp: Interpreter,
    join: JoinSet<(u64, JobOutcome)>,
    presenter: Presenter,
    input: InputBridge,
    last_watch: Instant,
    last_fetch: Instant,
    watch_ms: u64,
    fetch_ms: u64,
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
        interp: Interpreter::new(),
        join: JoinSet::new(),
        presenter,
        input,
        last_watch: Instant::now(),
        last_fetch: Instant::now(),
        watch_ms: watch_interval_ms(std::env::var("WS_STATUS_WATCH_MS").ok().as_deref()),
        fetch_ms: fetch_interval_ms(std::env::var("WS_STATUS_FETCH_MS").ok().as_deref()),
        quit: false,
    };
    ctx.interp
        .schedule(ctx.state, ctx.opts, Effect::LoadRightPane, &Action::None);
    if opts.start_fetch {
        let effect = ctx.state.dispatch(Action::Fetch);
        ctx.interp
            .schedule(ctx.state, ctx.opts, effect, &Action::Fetch);
    }

    while !ctx.quit {
        spawn_joinset(&mut ctx);
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
                    Ok((id, outcome)) => {
                        ctx.interp.apply(ctx.state, ctx.opts, id, outcome);
                        if ctx.interp.take_dirty() {
                            ctx.presenter.mark();
                        }
                    }
                    Err(_) => {}
                }
            }
            _ = sleep_ms(watch_remain) => {
                ctx.last_watch = Instant::now();
                if !overlay_blocks_background_ticks(ctx.state.input_mode()) {
                    let effect = ctx.state.dispatch(Action::WatchTick);
                    ctx.interp.schedule(ctx.state, ctx.opts, effect, &Action::WatchTick);
                    if ctx.interp.take_dirty() {
                        ctx.presenter.mark();
                    }
                }
            }
            _ = sleep_ms(fetch_remain) => {
                ctx.last_fetch = Instant::now();
                if !overlay_blocks_background_ticks(ctx.state.input_mode())
                    && !ctx.interp.busy_for_writes()
                {
                    let effect = ctx.state.dispatch(Action::FetchTick);
                    ctx.interp.schedule(ctx.state, ctx.opts, effect, &Action::FetchTick);
                    if ctx.interp.take_dirty() {
                        ctx.presenter.mark();
                    }
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
    if ctx.interp.busy_for_writes() {
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
    ctx.interp
        .schedule(ctx.state, ctx.opts, effect, &action_for_load);
    if let Some((repo, path)) = ctx.interp.take_pending_edit() {
        schedule_edit(ctx, repo, path);
    }
    if action_triggers_graph_autoload(&action_for_load) {
        ctx.interp.maybe_queue_autoload(ctx.state);
    }
    if ctx.interp.take_dirty() {
        ctx.presenter.mark();
    }
    ctx.presenter.mark();
}

fn spawn_joinset(ctx: &mut LoopCtx<'_>) {
    let LoopCtx {
        interp,
        state,
        opts,
        join,
        ..
    } = ctx;
    interp.spawn_ready(state, opts, &mut |id, work| {
        join.spawn_blocking(move || (id, work()));
    });
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
            ctx.interp.after_edit(ctx.state, repo);
            if ctx.interp.take_dirty() {
                ctx.presenter.mark();
            }
        }
    }
}

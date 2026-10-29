//! One async TTY loop: input, timers, `JoinSet` results, and a small presenter.
//!
//! `run_tui` stays synchronous via a current-thread Tokio runtime. Terminal
//! bytes are read on a dedicated thread through [`super::tty::poll_event`] /
//! [`super::tty::read_event_origin`]. Every git/process effect runs on
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

use super::action::{Action, Effect, ExternalDiffKind};
use super::activity::activity_targets;
use super::app::{
    apply_terminal_resize, discard_held_nav_backlog, map_event, run_blocking_editor,
    sync_mouse_capture, TuiOpts,
};
use super::diff_tool::{cleanup_prepared, diff_tool_command, wait_and_cleanup};
use super::editor::{editor_command, is_detached_editor, resolve_editor};
use super::effect::{DiffLaunch, Interpreter, JobOutcome};
use super::event_pump::{
    action_triggers_graph_autoload, classify_busy_dispatch, overlay_blocks_background_ticks,
    BusyAction,
};
use super::fetch_claim::unix_now_ms;
use super::hyperlink::{link_cells, write_links};
use super::keys::{drop_g_chord_echo_at, held_nav_key, is_half_page_chord, KeyStrokeOrigin};
use super::pointer_shape::{shape_for, TermPointer};
use super::render::draw;
use super::state::{AppState, PendingConfirm};
use super::status::StatusMessage;
use super::term_progress::{progress_for, TermProgress};
use super::tty::{poll_event, read_event_origin, TtyEvent};
use super::undercurl::{undercurl_cells, write_undercurls};
use super::watch::watch_remain_ms;

const INPUT_BATCH: usize = 8;

/// Shortest gap between draws, so a burst of keys coalesces into one frame.
///
/// There is no matching maximum: a dirty presenter always redraws once this
/// floor passes. A `DRAW_MAX_MS = 33` constant used to sit here, but both of
/// its branches in `dirty_remain` returned `0`, so it never capped anything.
const DRAW_MIN_MS: u64 = 16;

enum InputCmd {
    Pause,
    Resume,
    Shutdown,
}

struct InputBridge {
    rx: tokio::sync::mpsc::Receiver<TtyEvent>,
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
    tx: tokio::sync::mpsc::Sender<TtyEvent>,
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
        match poll_event(Duration::from_millis(16)) {
            Ok(true) => {}
            Ok(false) => continue,
            Err(_) => break,
        }
        let ready = match read_event_origin() {
            Ok(ready) => ready,
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => continue,
            Err(_) => break,
        };
        let leftover = held_nav_key(&ready.0).and_then(discard_held_nav_backlog);
        if tx.blocking_send(ready).is_err() {
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
            // Backdated past the floor so the first frame draws at once.
            last_draw: Instant::now() - Duration::from_millis(DRAW_MIN_MS),
        }
    }

    fn mark(&mut self) {
        self.dirty = true;
    }

    /// Milliseconds until the next draw: the draw floor when dirty, else
    /// the animation tick `anim_tick` after the last draw
    /// ([`AppState::next_anim_tick_ms`]). `u64::MAX` when clean and idle.
    fn remain_ms(&self, anim_tick: Option<u64>) -> u64 {
        self.remain_ms_after(self.elapsed_ms(), anim_tick)
    }

    /// [`Self::remain_ms`] with `elapsed` ms since the last draw.
    fn remain_ms_after(&self, elapsed: u64, anim_tick: Option<u64>) -> u64 {
        let floor = DRAW_MIN_MS.saturating_sub(elapsed);
        let dirty = if self.dirty { floor } else { u64::MAX };
        let tick = anim_tick.map_or(u64::MAX, |tick| tick.saturating_sub(elapsed).max(floor));
        dirty.min(tick)
    }

    fn elapsed_ms(&self) -> u64 {
        self.last_draw.elapsed().as_millis() as u64
    }

    /// A draw is due: past the draw floor, and dirty or one animation tick
    /// since the last draw.
    fn should_draw(&self, anim_tick: Option<u64>) -> bool {
        self.should_draw_after(self.elapsed_ms(), anim_tick)
    }

    /// [`Self::should_draw`] with `elapsed` ms since the last draw.
    fn should_draw_after(&self, elapsed: u64, anim_tick: Option<u64>) -> bool {
        elapsed >= DRAW_MIN_MS && (self.dirty || anim_tick.is_some_and(|tick| elapsed >= tick))
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
    /// Start of the last PR poll cycle (launch: the first badge lookups).
    last_pr_poll: Instant,
    watch_ms: u64,
    fetch_ms: u64,
    /// OSC 9;4 progress the terminal shows for the current batch.
    progress: TermProgress,
    /// OSC 22 pointer shape the terminal shows over a resize edge.
    pointer: TermPointer,
    quit: bool,
}

impl Drop for LoopCtx<'_> {
    /// Every exit from [`run`], an error included, ends here before the
    /// caller drops the runtime. That drop waits for running workers, so
    /// stop the PR lookups and opens, which can take many seconds, and the
    /// sparkline counts that have not started.
    fn drop(&mut self) {
        self.interp.cancel_pull_request_jobs();
        self.interp.cancel_activity_jobs();
    }
}

/// Run until quit. Caller owns terminal restore.
pub async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
) -> Result<(), u8> {
    let input = InputBridge::spawn();
    let mut presenter = Presenter::new();
    let mut progress = TermProgress::new(opts.settings.term_progress);
    let mut pointer = TermPointer::default();
    draw_frame(terminal, state, &mut progress, &mut pointer)?;
    presenter.painted();

    let mut ctx = LoopCtx {
        terminal,
        state,
        opts,
        interp: Interpreter::with_fetch_cap(opts.settings.fetch_concurrency),
        join: JoinSet::new(),
        presenter,
        input,
        last_watch: Instant::now(),
        last_fetch: Instant::now(),
        last_pr_poll: Instant::now(),
        watch_ms: opts.settings.watch_ms,
        fetch_ms: opts.settings.fetch_ms,
        progress,
        pointer,
        quit: false,
    };
    ctx.interp
        .schedule(ctx.state, ctx.opts, Effect::LoadRightPane, &Action::None);
    // Watch ticks look up only new or changed branches; the first snapshot
    // is all new. This is the first PR poll cycle: the next one runs one
    // poll period later.
    let lookups = Effect::LookupPullRequests {
        targets: ctx.state.due_pr_lookups(),
    };
    ctx.interp
        .schedule(ctx.state, ctx.opts, lookups, &Action::None);
    // Sparkline counts run on low-priority workers after the first frame,
    // so they never hold up the first paint or user work.
    let activity = Effect::LoadActivity {
        checkouts: activity_targets(&ctx.state.snapshot, None),
    };
    ctx.interp
        .schedule(ctx.state, ctx.opts, activity, &Action::None);
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
        // Hot or calm: the period follows the PR states each loop pass.
        let pr_poll_ms = ctx.state.pr_poll_interval_ms();
        let pr_poll_remain =
            if pr_poll_ms == 0 || overlay_blocks_background_ticks(ctx.state.input_mode()) {
                u64::MAX
            } else {
                pr_poll_ms.saturating_sub(ctx.last_pr_poll.elapsed().as_millis() as u64)
            };
        let ctrl_remain = ctx.state.ctrl_c_remaining_ms(now).unwrap_or(u64::MAX);
        let status_remain = ctx.state.status_expiry_ms(now).unwrap_or(u64::MAX);
        let peek_remain = ctx.state.peek_remaining_ms(now).unwrap_or(u64::MAX);
        let search_remain = ctx.state.search_files_due_ms(now).unwrap_or(u64::MAX);
        let present_remain = ctx.presenter.remain_ms(ctx.state.next_anim_tick_ms());
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
                if let Ok((id, outcome)) = joined {
                    ctx.interp.apply(ctx.state, ctx.opts, id, outcome);
                    if let Some(launch) = ctx.interp.take_pending_diff_launch() {
                        launch_diff(&mut ctx, launch);
                    }
                    if ctx.interp.take_dirty() {
                        ctx.presenter.mark();
                    }
                }
            }
            _ = sleep_ms(watch_remain) => {
                ctx.last_watch = Instant::now();
                if !overlay_blocks_background_ticks(ctx.state.input_mode()) {
                    // A background fetch another instance ran shows here.
                    if ctx.state.apply_background_fetch_record() {
                        ctx.presenter.mark();
                    }
                    let effect = ctx.state.dispatch(Action::WatchTick);
                    ctx.interp.schedule(ctx.state, ctx.opts, effect, &Action::WatchTick);
                    if ctx.interp.take_dirty() {
                        ctx.presenter.mark();
                    }
                }
            }
            _ = sleep_ms(fetch_remain) => {
                ctx.last_fetch = Instant::now();
                if !overlay_blocks_background_ticks(ctx.state.input_mode()) {
                    ctx.interp
                        .background_fetch_tick(ctx.state, ctx.opts, ctx.fetch_ms, unix_now_ms());
                    if ctx.interp.take_dirty() {
                        ctx.presenter.mark();
                    }
                }
            }
            _ = sleep_ms(pr_poll_remain) => {
                ctx.last_pr_poll = Instant::now();
                if !overlay_blocks_background_ticks(ctx.state.input_mode()) {
                    ctx.interp.pr_poll_tick(ctx.state, ctx.opts, ctx.last_pr_poll);
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
            _ = sleep_ms(status_remain) => {
                if ctx.state.expire_status(Instant::now()) {
                    ctx.presenter.mark();
                }
            }
            _ = sleep_ms(peek_remain) => {
                let (redraw, effect) = ctx.state.expire_peek_with_details(Instant::now());
                if redraw {
                    ctx.interp.schedule(ctx.state, ctx.opts, effect, &Action::None);
                    let _ = ctx.interp.take_dirty();
                    ctx.presenter.mark();
                }
            }
            _ = sleep_ms(search_remain) => {
                // Search-in-files debounce: the last query edit went quiet.
                if let Some(effect) = ctx.state.fire_search_files_due(Instant::now()) {
                    ctx.interp.schedule(ctx.state, ctx.opts, effect, &Action::None);
                    let _ = ctx.interp.take_dirty();
                    ctx.presenter.mark();
                }
            }
            _ = sleep_ms(present_remain) => {
                let flashing = ctx.state.has_active_flashes();
                ctx.state.prune_expired_flashes();
                if flashing && !ctx.state.has_active_flashes() {
                    // The last flash ended: one more frame clears its tint.
                    ctx.presenter.mark();
                }
                // Animation frames redraw on their own tick (spinner,
                // flash, glide); idle draws only when dirty.
                if ctx.presenter.should_draw(ctx.state.next_anim_tick_ms()) {
                    draw_frame(ctx.terminal, ctx.state, &mut ctx.progress, &mut ctx.pointer)?;
                    ctx.presenter.painted();
                }
            }
        }
    }
    ctx.input.shutdown();
    Ok(())
}

/// Draw one frame, then paint its hyperlinks over it (OSC 8), then its
/// curly underlines (SGR 4:3 + 58), then write the batch's terminal
/// progress (OSC 9;4) and the resize-edge pointer shape (OSC 22) when they
/// changed.
///
/// The links and curls are printed again on every frame: a diff that
/// rewrites such a cell clears its link or curl, and this puts it back.
/// Link cells and curl cells never overlap (`draw` cuts link cells out of
/// the curls), so the two writers' order does not matter. A batch that
/// ends sets `batch` to `None`, so the next frame writes the clear state.
/// The pointer shape follows `hovered_resize_edge` on the layout this
/// frame recorded.
fn draw_frame(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    progress: &mut TermProgress,
    pointer: &mut TermPointer,
) -> Result<(), u8> {
    let completed = terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
    let links = link_cells(completed.buffer, &state.layout.links);
    let curls = undercurl_cells(completed.buffer, &state.layout.undercurls);
    write_links(terminal.backend_mut(), &links).map_err(|_| 1u8)?;
    write_undercurls(terminal.backend_mut(), &curls, state.undercurl).map_err(|_| 1u8)?;
    progress
        .write_if_changed(
            terminal.backend_mut(),
            progress_for(state.remote_activity.batch.as_ref()),
        )
        .map_err(|_| 1u8)?;
    pointer
        .write_if_changed(
            terminal.backend_mut(),
            shape_for(state.hovered_resize_edge()),
        )
        .map_err(|_| 1u8)
}

async fn sleep_ms(ms: u64) {
    if ms == u64::MAX {
        std::future::pending::<()>().await;
    } else {
        sleep(TokioDuration::from_millis(ms)).await;
    }
}

/// Handle one input event on the clock of its arrival.
///
/// The `gg` / `zz` / `gt` chord windows, the typeless CSI-u echo window,
/// and the double-click window measure from the instant the event's bytes
/// were read. A busy loop that takes two keys late must not change what
/// they mean.
fn handle_input(ctx: &mut LoopCtx<'_>, (event, origin, read_at): TtyEvent) {
    ctx.state.begin_input(read_at);
    dispatch_input(ctx, event, origin, read_at);
    ctx.state.end_input();
}

fn dispatch_input(
    ctx: &mut LoopCtx<'_>,
    event: crossterm::event::Event,
    origin: KeyStrokeOrigin,
    read_at: Instant,
) {
    // CSI-u with REPORT_EVENT_TYPES sends a Release after every Press.
    // Do not dispatch it: `dispatch` would clear `GPending`. CSI-u
    // without event types sends that release as another Press. Drop a
    // protocol same-key Press only inside the echo window so `gt` / `gT`
    // stay tab actions and a later typeless `g` still completes `gg`.
    // A raw-byte second `g` is a new tap. Pending expiry clears a stale
    // arming `g`.
    ctx.state.expire_stale_g_chord();
    let input_mode = ctx.state.input_mode();
    if drop_g_chord_echo_at(
        &mut ctx.state.g_chord_echo,
        input_mode,
        &event,
        origin,
        read_at,
    ) {
        return;
    }
    let action = map_event(ctx.state, &event);
    if let Some(pointer) = pointer_motion(&event, &action) {
        // Any-event motion (DECSET 1003) arrives on every cell the pointer
        // crosses. Store it without dispatch: no chord reset, no status,
        // no effect. Redraw only when the hovered tab `[✗]`, icon, or
        // search option chip changes. The icon peek dwell and grace run on
        // `peek_remain`.
        if ctx.state.set_pointer(pointer) {
            ctx.presenter.mark();
        }
        return;
    }
    // Ctrl-d / Ctrl-u still dispatch `Move(±5)`; only paint glides the
    // viewport to the new top. Any other input ends a live glide.
    if is_half_page_chord(&event) && matches!(action, Action::Move(_)) {
        ctx.state.arm_scroll_glide(Instant::now());
    } else if action != Action::None {
        ctx.state.cancel_scroll_glide();
    }
    if ctx.interp.busy_for_writes() {
        let inner_action = match action {
            Action::QuickOpenSubmit => ctx
                .state
                .command_palette()
                .and_then(|palette| palette.selected_action()),
            Action::PopoverRun => ctx.state.popover_run_action(),
            Action::Release => ctx.state.popover_release_action(),
            // `Y` only keeps the push box open, so Handle is safe for it too.
            Action::ConfirmYes | Action::ConfirmYesClean
                if matches!(ctx.state.confirm, Some(PendingConfirm::Push { .. })) =>
            {
                Some(Action::Push)
            }
            _ => None,
        };
        match classify_busy_dispatch(&action, inner_action.as_ref()) {
            BusyAction::Quit => {
                // `q` mid-write arms the same press-again window as Ctrl-C.
                if matches!(ctx.state.quit_while_busy(Instant::now()), Effect::Quit) {
                    ctx.quit = true;
                }
                ctx.presenter.mark();
                return;
            }
            BusyAction::Resize { cols, rows } => {
                let _ = apply_terminal_resize(ctx.terminal, cols, rows);
                let _ = ctx.state.dispatch(Action::Resize { cols, rows });
                ctx.presenter.mark();
                return;
            }
            BusyAction::Ignore => {
                ctx.state.note_busy_drop(ctx.interp.running_write_op());
                ctx.presenter.mark();
                return;
            }
            BusyAction::Handle => {}
        }
    }
    if ctx.interp.keep_overlay_if_gitdir_busy(ctx.state, &action) {
        ctx.presenter.mark();
        return;
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
    if let Some((repo, path, line)) = ctx.interp.take_pending_edit() {
        schedule_edit(ctx, repo, path, line);
    }
    if let Some((repo, path, kind)) = ctx.interp.take_pending_diff() {
        ctx.interp.enqueue_diff_prepare(repo, path, kind, ctx.opts);
    }
    if action_triggers_graph_autoload(&action_for_load) {
        ctx.interp.maybe_queue_autoload(ctx.state);
    }
    if ctx.interp.take_dirty() {
        ctx.presenter.mark();
    }
    ctx.presenter.mark();
}

/// Pointer update for buttonless motion, or `None` for any other input.
///
/// [`Action::PointerMove`] stores the cell. The search-in-files dialog maps
/// motion to it too and keeps the pointer for its option chip hover (the
/// tab `[✗]` and icons ignore it while the dialog is open). Motion inside
/// any other overlay maps to [`Action::None`]; it clears the pointer so no
/// `[✗]` hover shows there.
fn pointer_motion(event: &crossterm::event::Event, action: &Action) -> Option<Option<(u16, u16)>> {
    match action {
        Action::PointerMove { col, row } => Some(Some((*col, *row))),
        Action::None
            if matches!(
                event,
                crossterm::event::Event::Mouse(crossterm::event::MouseEvent {
                    kind: crossterm::event::MouseEventKind::Moved,
                    ..
                })
            ) =>
        {
            Some(None)
        }
        _ => None,
    }
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
    // A started remote shows its row spinner and progress at once.
    if ctx.interp.take_dirty() {
        ctx.presenter.mark();
    }
}

fn schedule_edit(ctx: &mut LoopCtx<'_>, repo: String, path: String, line: Option<u32>) {
    let editor = resolve_editor(
        ctx.opts.config.editor.as_deref(),
        std::env::var("VISUAL").ok().as_deref(),
        std::env::var("EDITOR").ok().as_deref(),
    );
    let abs = ctx.opts.cwd.join(&repo).join(&path);
    let (cmd, args) = editor_command(&editor, &abs.to_string_lossy(), line);
    if is_detached_editor(&editor) {
        let spawned = Command::new(&cmd)
            .args(&args)
            .current_dir(ctx.opts.cwd.join(&repo))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        ctx.state.status = match spawned {
            Ok(_) => StatusMessage::ok(format!("opened {path}")),
            Err(err) => StatusMessage::error(format!("edit failed: {cmd}: {err}")),
        };
        ctx.presenter.mark();
        return;
    }
    ctx.input.pause();
    let mouse = ctx.state.mouse_enabled;
    let result = run_blocking_editor(
        ctx.terminal,
        &mut ctx.pointer,
        &cmd,
        &args,
        &ctx.opts.cwd.join(&repo),
        mouse,
    );
    ctx.input.resume();
    match result {
        Err(err) => {
            ctx.state.status = StatusMessage::error(format!("edit failed: {err}"));
            ctx.presenter.mark();
        }
        Ok(()) => {
            ctx.state.status = StatusMessage::ok(format!("edited {path}"));
            // An Explorer inner file tab showing this file reads it again.
            if let Some(reload) = ctx.state.reload_explorer_after_edit(&repo, &path) {
                ctx.interp
                    .schedule(ctx.state, ctx.opts, reload, &Action::None);
            }
            ctx.interp.after_edit(ctx.state, repo);
            if ctx.interp.take_dirty() {
                ctx.presenter.mark();
            }
        }
    }
}

fn launch_diff(ctx: &mut LoopCtx<'_>, launch: DiffLaunch) {
    let DiffLaunch {
        repo,
        path,
        kind,
        tool,
        repo_abs,
        prepared,
    } = launch;
    let prepared = match prepared {
        Ok(prepared) => prepared,
        Err(err) => {
            ctx.state.status = StatusMessage::error(format!("diff failed: {err}"));
            ctx.presenter.mark();
            return;
        }
    };
    let left = prepared.left.to_string_lossy().into_owned();
    let right = prepared.right.to_string_lossy().into_owned();
    let (cmd, args) = diff_tool_command(&tool, &left, &right);
    if is_detached_editor(&tool) {
        match Command::new(&cmd)
            .args(&args)
            .current_dir(&repo_abs)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(child) => {
                std::thread::spawn(move || {
                    wait_and_cleanup(child, prepared);
                });
                ctx.state.status = StatusMessage::ok(format!("opened diff {path}"));
            }
            Err(err) => {
                cleanup_prepared(&prepared);
                ctx.state.status = StatusMessage::error(format!("diff failed: {err}"));
            }
        }
        ctx.presenter.mark();
        return;
    }
    ctx.input.pause();
    let mouse = ctx.state.mouse_enabled;
    let result = run_blocking_editor(
        ctx.terminal,
        &mut ctx.pointer,
        &cmd,
        &args,
        &repo_abs,
        mouse,
    );
    ctx.input.resume();
    cleanup_prepared(&prepared);
    match result {
        Err(err) => {
            ctx.state.status = StatusMessage::error(format!("diff failed: {err}"));
            ctx.presenter.mark();
        }
        Ok(()) => {
            ctx.state.status = StatusMessage::ok(format!("diffed {path}"));
            if matches!(kind, ExternalDiffKind::Worktree) {
                ctx.interp.after_edit(ctx.state, repo);
                if ctx.interp.take_dirty() {
                    ctx.presenter.mark();
                }
            } else {
                ctx.presenter.mark();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{Event, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

    fn mouse(kind: MouseEventKind) -> Event {
        Event::Mouse(MouseEvent {
            kind,
            column: 7,
            row: 0,
            modifiers: KeyModifiers::NONE,
        })
    }

    /// A presenter with the given dirty flag. Tests pass the elapsed time
    /// since the last draw explicitly, so a stalled thread cannot move it.
    fn presenter(dirty: bool) -> Presenter {
        Presenter {
            dirty,
            last_draw: Instant::now(),
        }
    }

    #[test]
    fn presenter_has_no_timer_when_clean_and_not_animating() {
        let clean = presenter(false);
        assert_eq!(clean.remain_ms_after(10_000, None), u64::MAX);
        assert_eq!(clean.remain_ms_after(0, None), u64::MAX);
        assert!(!clean.should_draw_after(10_000, None));
        let dirty = presenter(true);
        assert_eq!(
            dirty.remain_ms_after(10_000, None),
            0,
            "dirty past the floor draws now"
        );
        assert_eq!(dirty.remain_ms_after(0, None), DRAW_MIN_MS);
        assert!(dirty.should_draw_after(DRAW_MIN_MS, None));
        assert!(!dirty.should_draw_after(DRAW_MIN_MS - 1, None));
    }

    #[test]
    fn presenter_ticks_at_spinner_rate_while_animating() {
        use super::super::anim::SPINNER_FRAME_MS;
        let tick = Some(SPINNER_FRAME_MS);
        let clean = presenter(false);
        assert_eq!(
            clean.remain_ms_after(0, tick),
            SPINNER_FRAME_MS,
            "just painted: next frame one spinner tick away, not at the draw floor"
        );
        assert_eq!(clean.remain_ms_after(30, tick), SPINNER_FRAME_MS - 30);
        assert!(!clean.should_draw_after(SPINNER_FRAME_MS - 1, tick));

        assert_eq!(clean.remain_ms_after(SPINNER_FRAME_MS, tick), 0);
        assert!(
            clean.should_draw_after(SPINNER_FRAME_MS, tick),
            "one tick after the last draw"
        );
        assert!(
            !clean.should_draw_after(SPINNER_FRAME_MS, None),
            "the same clean presenter idles"
        );

        assert_eq!(
            clean.remain_ms_after(0, Some(1)),
            DRAW_MIN_MS,
            "a tick never beats the draw floor"
        );
        assert!(!clean.should_draw_after(DRAW_MIN_MS - 1, Some(1)));
        assert_eq!(
            presenter(true).remain_ms_after(0, tick),
            DRAW_MIN_MS,
            "dirty draws at the floor even while animating"
        );
    }

    #[test]
    fn pointer_motion_stores_moves_and_clears_on_overlay_motion() {
        let moved = mouse(MouseEventKind::Moved);
        assert_eq!(
            pointer_motion(&moved, &Action::PointerMove { col: 7, row: 0 }),
            Some(Some((7, 0)))
        );
        assert_eq!(
            pointer_motion(&moved, &Action::None),
            Some(None),
            "overlay motion clears the hover"
        );
        let click = mouse(MouseEventKind::Down(MouseButton::Left));
        assert_eq!(pointer_motion(&click, &Action::None), None);
        assert_eq!(
            pointer_motion(&click, &Action::Click { col: 7, row: 0 }),
            None
        );
    }
}

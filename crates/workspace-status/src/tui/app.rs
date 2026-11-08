//! TUI helpers for the async event loop.
//!
//! Live TTY I/O lives in [`super::event_loop`]. Effect schedule / spawn / apply
//! live in [`super::effect`]. This module keeps `run_tui` terminal setup and
//! pane/compute helpers.

use std::collections::{BTreeSet, HashMap};
use std::io::{self, stdout, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crossterm::cursor::Show;
use crossterm::event::{
    KeyEvent, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size as terminal_size, BeginSynchronizedUpdate,
    EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::{execute, queue};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::config::{ThemeMode, WorkspaceStatusConfig};
use crate::discovery::{collect_snapshots, process_repo, RepoCheckoutMeta};
use crate::git::{
    ahead_behind, checkout_branch, classify_upstream_rewrite, diff_commit_file_ctx,
    diff_compare_file_ctx, diff_stash_file_ctx, diff_worktree_vs_commit_file_ctx,
    fast_forward_to_remote_ref, git_diff_args, list_commit_name_status, list_compare_name_status,
    list_compare_patch_keys, list_stash_name_status, list_worktree_name_status,
    list_worktree_vs_commit_name_status, merge_base, merge_into_head, repo_has_local_changes,
    rev_parse_commit, rev_parse_quiet, MergeIntoHeadResult, NameStatus,
};
use crate::settings::Settings;
use crate::snapshot::{
    build_workspace_snapshot, repo_snapshots_from_workspace, CheckoutKind, FileChange,
    WorkspaceSnapshot,
};

use super::action::Action;
use super::branches::{
    checkout_name_for_ref, is_origin_remote_ref, plan_graph_checkout, GraphCheckoutPlan,
    DIRTY_WORKTREE_STATUS,
};
use super::diff::{load_file_diff, DiffContent};
use super::drill::{CommitFile, CommitFileSource, DrillView};
use super::effect::{in_worker_job, note_worker_panic_at};
use super::graph_load::{
    load_graph_model, load_graph_model_window, refresh_graph_limit, GraphIdentity,
};
use super::keys::{event_to_action_with, is_held_nav_backlog, InputMode};
use super::pointer_shape::{TermPointer, POINTER_RESET, POINTER_SET};
use super::state::AppState;
use super::status::StatusMessage;
use super::tabs::{
    base_ref_not_found, head_ref_not_found, no_merge_base, WorktreeFile, COMPARE_HEAD_REF,
    HEAD_HAS_NO_COMMIT,
};
use super::term_bg::{is_light_background, pick_theme, query_background, QUERY_TIMEOUT};
use super::term_progress::{PROGRESS_CLEAR, PROGRESS_SHOWN};
use super::tty::{
    disable_mouse, enable_mouse, poll_event, read_event, read_event_origin, TtyEvent,
};
#[cfg(test)]
use super::watch::{checkout_watch_identities, watch_needs_pane_reload};

/// Options for the interactive TUI.
pub struct TuiOpts {
    pub cwd: std::path::PathBuf,
    pub snapshot: WorkspaceSnapshot,
    pub config: WorkspaceStatusConfig,
    /// Env-backed settings resolved once at startup.
    pub settings: Settings,
    pub start_fetch: bool,
}

/// Open the alternate screen and run until quit.
///
/// Every exit restores the terminal, clears a shown OSC 9;4 bar, and puts
/// back the default pointer after an OSC 22 resize shape:
/// quit, an error return, a panic on this thread (panic hook), and on Unix
/// an exit signal (`SIGINT`, `SIGTERM`, `SIGHUP`; see
/// [`super::term_restore`]).
pub fn run_tui(opts: TuiOpts) -> Result<(), u8> {
    install_panic_hook(&PROGRESS_SHOWN, &POINTER_SET, stdout);
    let mut state = AppState::launch(opts.cwd.clone(), opts.snapshot.clone(), &opts.settings);
    state.apply_view_defaults(&opts.config.view_defaults);
    #[cfg(unix)]
    {
        super::term_restore::save_terminal_modes();
        crate::git::set_exit_signal_hook(super::term_restore::restore_from_signal);
    }
    enable_raw_mode().map_err(|_| 1u8)?;
    TERMINAL_SET_UP.store(true, Ordering::SeqCst);
    // Raw mode keeps the terminal's reply off the screen. The input thread
    // starts later in `event_loop::run`, so this read cannot race it.
    let mode = opts.settings.theme_mode;
    let light_bg = match mode {
        ThemeMode::Auto => query_background(QUERY_TIMEOUT).map(is_light_background),
        ThemeMode::Dark | ThemeMode::Light => None,
    };
    state.theme = pick_theme(state.theme, mode, light_bg);
    let mut out = stdout();
    if execute!(out, EnterAlternateScreen).is_err() || enable_mouse(&mut out).is_err() {
        restore_terminal();
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
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .build();
    let result = match &rt {
        Ok(rt) => rt.block_on(super::event_loop::run(&mut terminal, &mut state, &opts)),
        Err(_) => Err(1),
    };
    restore_terminal();
    let _ = terminal.show_cursor();
    // Workers may still run (a git fetch, say). Stop their git first
    // (SIGTERM, then SIGKILL after a short grace), and shut the runtime
    // down only once the terminal is back to normal.
    crate::git::stop_running_git();
    if let Ok(rt) = rt {
        shut_down_runtime(
            rt,
            RUNTIME_SHUTDOWN_GRACE,
            crate::git::wait_for_user_work_restore,
        );
    }
    result
}

/// How long quit waits for worker jobs once their git is stopped.
const RUNTIME_SHUTDOWN_GRACE: Duration = Duration::from_millis(500);

/// Shut the loop's runtime down without waiting for ever on its blocking
/// workers. A plain drop waits for every one of them, and a job with no git
/// to stop (a slow file read, a stuck helper) would hold quit.
///
/// It first calls `wait_for_user_work`, a bounded wait while a pull still
/// has to give stashed changes back (`git::wait_for_user_work_restore` in
/// `run_tui`), so quit does not cut that pop off. Then it gives the
/// workers `grace` and returns; the ones still running end with the
/// process.
fn shut_down_runtime(
    rt: tokio::runtime::Runtime,
    grace: Duration,
    wait_for_user_work: impl FnOnce(),
) {
    wait_for_user_work();
    rt.shutdown_timeout(grace);
}

/// True while `run_tui` has the terminal in raw mode. The panic hook
/// restores only then, so a panic after the TUI closed leaves the shell
/// alone and the restore runs once. The exit-signal restore
/// (`term_restore::restore_from_signal`) follows the same rule.
pub(super) static TERMINAL_SET_UP: AtomicBool = AtomicBool::new(false);

/// Give the terminal back on stdout. Clears the OSC 9;4 bar only when one
/// shows ([`PROGRESS_SHOWN`]), and resets the OSC 22 pointer only while a
/// resize shape shows ([`POINTER_SET`]).
///
/// It clears [`TERMINAL_SET_UP`] only after the restore bytes are out. An
/// exit signal that comes during the write then still finds the flag set,
/// and its handler (`term_restore::restore_from_signal`) restores in full
/// before the signal ends `ws`; with the flag cleared first, the signal
/// would end `ws` part way through this restore. The panic hook and the
/// signal handler take the flag with a swap, so each of them restores at
/// most once. A restore that overlaps this one writes the same bytes
/// again, and each of those steps is safe to repeat.
fn restore_terminal() {
    restore_terminal_to(
        &mut stdout(),
        PROGRESS_SHOWN.swap(false, Ordering::SeqCst),
        POINTER_SET.swap(false, Ordering::SeqCst),
    );
    TERMINAL_SET_UP.store(false, Ordering::SeqCst);
}

/// Clear the OSC 9;4 bar first (when `clear_progress`), then put the
/// default pointer back (when `reset_pointer`), then leave raw mode, mouse
/// capture, the keyboard protocol, and the alternate screen, and show the
/// cursor. Each step is safe on a terminal that was never set up.
///
/// With no bar shown it writes no OSC 9;4: some terminals show any OSC 9
/// as a desktop notification. With no resize shape shown it writes no
/// OSC 22.
///
/// The alternate screen is left through [`leave_alternate_screen_to`],
/// which closes any open synchronized update first.
///
/// On Unix the exit-signal restore writes the same bytes from a constant
/// (`term_restore::RESTORE_SEQ`); a unit test keeps the two equal.
pub(super) fn restore_terminal_to<W: Write>(
    out: &mut W,
    clear_progress: bool,
    reset_pointer: bool,
) {
    if clear_progress {
        let _ = out.write_all(PROGRESS_CLEAR.as_bytes());
    }
    if reset_pointer {
        let _ = out.write_all(POINTER_RESET.as_bytes());
    }
    let _ = disable_raw_mode();
    let _ = disable_mouse(out);
    let _ = execute!(out, PopKeyboardEnhancementFlags);
    let _ = leave_alternate_screen_to(out);
    let _ = execute!(out, Show);
}

/// Close any open synchronized update (DEC private mode 2026), then leave
/// the alternate screen.
///
/// A resize opens an update that the next frame closes. When the TUI
/// quits, fails, or hands the terminal to an editor before that frame,
/// this end marker closes it, so the terminal never holds back output.
/// The end marker is written every time: with no update open, a terminal
/// ignores it.
fn leave_alternate_screen_to<W: Write>(out: &mut W) -> io::Result<()> {
    execute!(out, EndSynchronizedUpdate, LeaveAlternateScreen)
}

/// Chain a panic hook that restores the terminal to `target()` before the
/// previous hook prints the panic, so the message lands on the normal
/// screen with raw mode off.
///
/// It restores only for a panic on the thread that installed it (the TUI
/// thread) while [`TERMINAL_SET_UP`] is set. It clears the OSC 9;4 bar only
/// while `shown` is set ([`PROGRESS_SHOWN`] outside tests), and resets the
/// OSC 22 pointer only while `pointer_set` is set ([`POINTER_SET`] outside
/// tests).
///
/// A panic in a worker job (`run_job`, which catches it) while the
/// terminal is set up leaves the running TUI alone and is not printed: the
/// text would land in the alternate screen and break the frame. The hook
/// notes where it panicked instead, and the status line shows the job, the
/// message, and that location. A panic on any other thread (the input
/// thread, a helper thread) still goes to the previous hook, so it is never
/// silent.
fn install_panic_hook<W, F>(shown: &'static AtomicBool, pointer_set: &'static AtomicBool, target: F)
where
    W: Write,
    F: Fn() -> W + Send + Sync + 'static,
{
    let tui_thread = std::thread::current().id();
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == tui_thread {
            if TERMINAL_SET_UP.swap(false, Ordering::SeqCst) {
                restore_terminal_to(
                    &mut target(),
                    shown.swap(false, Ordering::SeqCst),
                    pointer_set.swap(false, Ordering::SeqCst),
                );
            }
        } else if in_worker_job() && TERMINAL_SET_UP.load(Ordering::SeqCst) {
            if let Some(at) = info.location() {
                note_worker_panic_at(at.to_string());
            }
            return;
        }
        prev(info);
    }));
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
pub(crate) fn push_keyboard_enhancement() {
    let mut out = stdout();
    let _ = execute!(
        out,
        PushKeyboardEnhancementFlags(keyboard_enhancement_flags())
    );
}

pub(crate) fn terminal_size_rect() -> Rect {
    let (cols, rows) = terminal_size().unwrap_or((80, 24));
    Rect::new(0, 0, cols.max(1), rows.max(1))
}

/// Map one terminal event to an [`Action`] for `state`: its input mode,
/// focus, and last paint. A left click in the search-in-files dialog maps
/// to [`Action::SearchFilesToggleOption`] on an option chip and to
/// [`Action::None`] anywhere else.
pub(crate) fn map_event(state: &AppState, event: &crossterm::event::Event) -> Action {
    use crossterm::event::{Event, KeyCode, KeyModifiers};
    // Below the minimum size only the resize notice paints. Panes,
    // overlays, confirms, and prompts are hidden, so only keys that close
    // or quit (and resize) map: a blind `s`, `P`, or hidden `y` does nothing.
    if state.too_small {
        let closes = match event {
            Event::Key(key) => match key.code {
                KeyCode::Esc => true,
                KeyCode::Char('q') => !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT),
                KeyCode::Char('c') => key.modifiers.contains(KeyModifiers::CONTROL),
                _ => false,
            },
            Event::Resize(..) => true,
            _ => false,
        };
        if !closes {
            return Action::None;
        }
    }
    let mode = state.input_mode();
    let action = event_to_action_with(
        event,
        mode,
        state.right_is_diff(),
        matches!(state.focus, super::state::FocusPane::Right),
        state.graph_stash_focused(),
        state.graph_commit_focused(),
        state.hl_folds(),
    );
    match action {
        // In the search-in-files dialog a click only toggles the option
        // chip under it. A raw click must never reach dispatch there: the
        // normal-mode click handlers would act on the panes under the box.
        Action::Click { col, row } if matches!(mode, InputMode::SearchFiles { .. }) => state
            .search_option_at(col, row)
            .map_or(Action::None, Action::SearchFilesToggleOption),
        action => action,
    }
}

/// Resize the viewport to `cols` × `rows`.
///
/// The resize clears the screen. A synchronized update (DEC private mode
/// 2026) opens before the clear and the next frame closes it, so a
/// terminal that knows the mode never shows the blank screen between the
/// two.
pub(crate) fn apply_terminal_resize(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cols: u16,
    rows: u16,
) -> Result<(), u8> {
    queue!(terminal.backend_mut(), BeginSynchronizedUpdate).map_err(|_| 1u8)?;
    terminal
        .resize(Rect::new(0, 0, cols.max(1), rows.max(1)))
        .map_err(|_| 1u8)
}

/// Inputs for right-pane git (`git log` / `git diff` / commit files / commit
/// diff). Depth 0 loads a graph or worktree file diff. Depth 1 loads files for
/// the focused graph row. Depth 2 loads that file's commit diff.
///
/// Must stay `Send` so the TTY loop can run [`Self::compute`] on `spawn_blocking`.
#[derive(Clone, Debug)]
pub(crate) struct RightPaneRequest {
    cwd: std::path::PathBuf,
    snapshot: WorkspaceSnapshot,
    show_hidden: bool,
    in_graph: bool,
    focused_file: Option<(String, FileChange)>,
    file_diff_context: Option<u32>,
    focused_graph_repo: Option<String>,
    same_repo: bool,
    graph_limit: usize,
    /// Local branch names for `git log` instead of `--all`. Empty = full graph.
    graph_focus_branches: Vec<String>,
    /// Depth 1: commit / stash / worktree files for the focused graph row.
    follow_files: Option<(String, CommitFileSource)>,
    /// Depth 2: commit-scoped diff for the focused commit-file row.
    follow_diff: Option<FollowDiffRequest>,
}

/// Identity of the row a [`RightPaneRequest`] would load. Used to detect
/// cursor movement while pane git is in flight.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RightPaneTarget {
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
pub(crate) enum RightPaneLoad {
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
    pub(crate) fn from_state(state: &AppState) -> Self {
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
            show_hidden: state.show_hidden,
            in_graph: state.drill.is_graph(),
            file_diff_context: focused_file
                .as_ref()
                .and_then(|(repo, change)| state.workspace_diff_context(repo, &change.path)),
            focused_file,
            focused_graph_repo,
            same_repo,
            graph_limit: refresh_graph_limit(state.graph.as_ref()),
            graph_focus_branches: state.graph_focus_revs(),
            follow_files,
            follow_diff,
        }
    }

    pub(crate) fn target(&self) -> RightPaneTarget {
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

    pub(crate) fn compute(&self) -> RightPaneLoad {
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
                    self.show_hidden,
                    0,
                    self.graph_limit,
                    &self.graph_focus_branches,
                )
            } else {
                load_graph_model(
                    &self.cwd,
                    &self.snapshot,
                    repo,
                    self.show_hidden,
                    &self.graph_focus_branches,
                )
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

/// Apply a coalesced right-pane load. Compare and Explorer tabs ignore this path.
pub(crate) fn apply_right_pane_load(state: &mut AppState, payload: RightPaneLoad) {
    if !state.tabs.is_workspace() {
        return;
    }
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

/// Git-only checkout work. TTY runs this on a worker; tests call it via
/// [`run_checkout_branch`].
pub(crate) enum CheckoutCompute {
    Dirty,
    Failed {
        status: String,
        clear_picker: bool,
    },
    Confirm {
        local_branch: String,
        remote_ref: String,
        ahead_behind: Option<(usize, usize)>,
    },
    /// HEAD moved. `status` is ok, or warn when the fast-forward did not happen.
    Done {
        status: StatusMessage,
    },
}

/// Git-only merge work. TTY runs this on a worker; tests call it via
/// [`run_merge_into_head`].
pub(crate) enum MergeCompute {
    Dirty,
    AlreadyUpToDate,
    FastForward,
    MergeCommit,
    Conflict,
    Failed(String),
}

pub(crate) fn compute_checkout(
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
        let status = if fast_forward_to_remote_ref(remote_ref, dir) {
            StatusMessage::ok(format!(
                "Checked out {selected_name} and fast-forwarded to {remote_ref}"
            ))
        } else if let Some(rewrite) = classify_upstream_rewrite(dir, selected_name, remote_ref) {
            // The remote was rewritten: `p` resets (nothing of yours) or names
            // the rebase (your commits on top). The checkout never resets.
            StatusMessage::warn(if rewrite.yours == 0 {
                format!(
                    "Checked out {selected_name}; {remote_ref} was force-pushed ({} dropped, none yours) — press p to move {selected_name} to {remote_ref}",
                    rewrite.dropped
                )
            } else {
                let noun = if rewrite.yours == 1 {
                    "commit"
                } else {
                    "commits"
                };
                format!(
                    "Checked out {selected_name}; {remote_ref} was force-pushed under your {} {noun} — press p for the rebase command",
                    rewrite.yours
                )
            })
        } else {
            let reason = match ahead_behind("HEAD", remote_ref, dir) {
                Some((ahead, _)) if ahead > 0 => format!(": local has commits {remote_ref} lacks"),
                _ => String::new(),
            };
            StatusMessage::warn(format!(
                "Checked out {selected_name}; could not fast-forward to {remote_ref}{reason}"
            ))
        };
        return CheckoutCompute::Done { status };
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
            ahead_behind: local_sha
                .as_deref()
                .zip(remote_sha.as_deref())
                .and_then(|(local, remote)| ahead_behind(local, remote, dir)),
            local_branch,
            remote_ref,
        },
        GraphCheckoutPlan::Checkout { branch } => {
            if checkout_branch(&branch, dir) {
                CheckoutCompute::Done {
                    status: StatusMessage::ok(format!("Checked out {branch}")),
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

pub(crate) fn apply_checkout_compute(
    state: &mut AppState,
    repo: String,
    result: CheckoutCompute,
) -> bool {
    match result {
        CheckoutCompute::Dirty => {
            state.status = StatusMessage::warn(DIRTY_WORKTREE_STATUS);
            false
        }
        CheckoutCompute::Failed {
            status,
            clear_picker,
        } => {
            if clear_picker {
                state.branch_picker = None;
            }
            state.status = StatusMessage::error(status);
            false
        }
        CheckoutCompute::Confirm {
            local_branch,
            remote_ref,
            ahead_behind,
        } => {
            state.branch_picker = None;
            let _ = state.confirm_checkout_if_out_of_sync(
                repo,
                local_branch,
                Some(remote_ref),
                ahead_behind,
            );
            false
        }
        CheckoutCompute::Done { status } => {
            state.branch_picker = None;
            state.status = status;
            true
        }
    }
}

pub(crate) fn compute_merge(dir: &Path, rev: &str) -> MergeCompute {
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

pub(crate) fn apply_merge_compute(state: &mut AppState, label: &str, result: MergeCompute) -> bool {
    match result {
        MergeCompute::Dirty => {
            state.status = StatusMessage::warn(DIRTY_WORKTREE_STATUS);
            false
        }
        MergeCompute::AlreadyUpToDate => {
            state.status = "Already up to date".into();
            false
        }
        MergeCompute::FastForward => {
            state.status = StatusMessage::ok(format!("Fast-forwarded to {label}"));
            true
        }
        MergeCompute::MergeCommit => {
            state.status = StatusMessage::ok(format!("Merged {label}"));
            true
        }
        MergeCompute::Conflict => {
            state.status = StatusMessage::warn("Merge conflict — resolve in the worktree");
            true
        }
        MergeCompute::Failed(err) => {
            state.status = StatusMessage::error(format!("merge failed: {err}"));
            false
        }
    }
}

#[cfg(test)]
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

#[cfg(test)]
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

pub(crate) fn compute_commit_files(dir: &Path, source: &CommitFileSource) -> Vec<NameStatus> {
    match source {
        CommitFileSource::Commit { commit_id } => list_commit_name_status(dir, commit_id),
        CommitFileSource::Stash { stash_ref } => list_stash_name_status(dir, stash_ref),
        CommitFileSource::Worktree => list_worktree_name_status(dir),
        CommitFileSource::Compare { .. } | CommitFileSource::CommitVsWorktree { .. } => Vec::new(),
    }
}

/// Resolved compare endpoints plus the committed `base...HEAD` file list.
pub(crate) struct CompareRangeLoad {
    /// Immutable compare endpoints for this load.
    pub source: CommitFileSource,
    /// Committed `base...HEAD` paths.
    pub files: Vec<super::drill::CommitFile>,
    /// Path → patch key from [`list_compare_patch_keys`]. Empty for a
    /// commit-vs-working-tree load, and when the raw diff call failed.
    pub patch_keys: HashMap<String, String>,
    /// HEAD SHA at load time. `None` for a commit-vs-working-tree load,
    /// which is never probed.
    pub head: Option<String>,
    /// Base tip SHA at load time.
    pub base_tip: String,
    /// The head commit of a Diff commit vs parent range
    /// ([`super::tabs::commit_vs_parent_head`]), read for the commit
    /// footer. `None` for any other range.
    pub commit: Option<workspace_status_graph::Commit>,
}

/// Resolve `base_ref` and list the one file of a commit-vs-working-tree
/// compare (`diff --name-status -z -M <base> -- [<old>] <path>`).
///
/// A file on disk that equals the commit lists nothing.
pub(crate) fn compute_worktree_compare_range(
    dir: &Path,
    base_ref: &str,
    file: &WorktreeFile,
) -> Result<CompareRangeLoad, String> {
    let base = rev_parse_commit(dir, base_ref)?.ok_or_else(|| base_ref_not_found(base_ref))?;
    let files =
        list_worktree_vs_commit_name_status(dir, &base, &file.path, file.old_path.as_deref())?
            .into_iter()
            .map(Into::into)
            .collect();
    Ok(CompareRangeLoad {
        source: CommitFileSource::CommitVsWorktree {
            base: base.clone(),
            path: file.path.clone(),
            old_path: file.old_path.clone(),
        },
        files,
        patch_keys: HashMap::new(),
        head: None,
        base_tip: base,
        commit: None,
    })
}

/// Resolve head / base / merge-base and list committed compare files.
///
/// `head_ref` is `HEAD` for a live tab or a full commit id for a pinned
/// one; a pinned `<sha>^...<sha>` lists that commit's first-parent changes.
pub(crate) fn compute_compare_range(
    dir: &Path,
    base_ref: &str,
    head_ref: &str,
) -> Result<CompareRangeLoad, String> {
    let head = rev_parse_commit(dir, head_ref)?.ok_or_else(|| {
        if head_ref == COMPARE_HEAD_REF {
            HEAD_HAS_NO_COMMIT.to_string()
        } else {
            head_ref_not_found(head_ref)
        }
    })?;
    let base_tip = rev_parse_commit(dir, base_ref)?.ok_or_else(|| base_ref_not_found(base_ref))?;
    let merge =
        merge_base(dir, &base_tip, &head)?.ok_or_else(|| no_merge_base(base_ref, head_ref))?;
    let files = list_compare_name_status(dir, &base_tip, &head)?
        .into_iter()
        .map(Into::into)
        .collect();
    let patch_keys = list_compare_patch_keys(dir, &base_tip, &head);
    // One commit against its parent: read it for the footer here, off the
    // event thread, so a commit outside the loaded graph still shows.
    let commit = super::tabs::commit_vs_parent_head(base_ref, head_ref)
        .and_then(|_| super::graph_load::load_commit(dir, &head));
    Ok(CompareRangeLoad {
        source: CommitFileSource::Compare {
            base_ref: base_ref.to_string(),
            head_ref: head_ref.to_string(),
            base_tip: base_tip.clone(),
            merge_base: merge,
            head: head.clone(),
        },
        files,
        patch_keys,
        head: Some(head),
        base_tip,
        commit,
    })
}

/// Probe whether compare endpoints moved. `Ok(true)` means reload.
///
/// Resolves `head_ref`, so a pinned tab (full sha head, `<sha>^` base)
/// never reloads when the checkout's HEAD moves.
pub(crate) fn probe_compare_range(
    dir: &Path,
    base_ref: &str,
    head_ref: &str,
    last_head: Option<&str>,
    last_base_tip: Option<&str>,
) -> Result<(bool, Option<String>, Option<String>), String> {
    let head = rev_parse_commit(dir, head_ref)?;
    let base_tip = rev_parse_commit(dir, base_ref)?;
    let changed = match (last_head, last_base_tip) {
        (Some(prev_head), Some(prev_base)) => {
            head.as_deref() != Some(prev_head) || base_tip.as_deref() != Some(prev_base)
        }
        _ => false,
    };
    Ok((changed, head, base_tip))
}

pub(crate) fn commit_diff_list(state: &AppState) -> (Vec<super::drill::CommitFile>, usize) {
    match &state.drill {
        super::drill::DrillView::Files { files, cursor, .. } => (files.clone(), *cursor),
        super::drill::DrillView::Diff {
            files, file_cursor, ..
        } => (files.clone(), *file_cursor),
        super::drill::DrillView::Graph => (Vec::new(), 0),
    }
}

pub(crate) fn compute_commit_diff(
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
            DiffContent::from_commit_lines(diff_commit_file_ctx(&dir, commit_id, path, context))
        }
        CommitFileSource::Stash { stash_ref } => {
            DiffContent::from_commit_lines(diff_stash_file_ctx(&dir, stash_ref, path, context))
        }
        CommitFileSource::Worktree => {
            if let Some((file_repo, change)) = focused_file {
                if file_repo == repo && change.path == path {
                    return load_file_diff(cwd, repo, change, context);
                }
            }
            head_file_diff(&dir, path, context)
        }
        CommitFileSource::Compare { .. } | CommitFileSource::CommitVsWorktree { .. } => {
            DiffContent::default()
        }
    }
}

/// Unified compare diff for one path. Failure is `Err`.
///
/// A commit-vs-working-tree source diffs its base commit against the file
/// on disk (`diff -M <base> -- [<old>] <path>`).
pub(crate) fn compute_compare_diff(
    dir: &Path,
    source: &CommitFileSource,
    path: &str,
    old_path: Option<&str>,
    context: Option<u32>,
) -> Result<DiffContent, String> {
    match source {
        CommitFileSource::Compare { base_tip, head, .. } => {
            let lines = diff_compare_file_ctx(dir, base_tip, head, path, old_path, context)?;
            Ok(DiffContent::from_compare_lines(lines))
        }
        CommitFileSource::CommitVsWorktree { base, .. } => {
            let lines = diff_worktree_vs_commit_file_ctx(dir, base, path, old_path, context)?;
            Ok(DiffContent::from_worktree_compare_lines(lines))
        }
        _ => Ok(DiffContent::default()),
    }
}

fn head_file_diff(dir: &Path, path: &str, context: Option<u32>) -> DiffContent {
    let args = git_diff_args(&["diff", "HEAD"], path, context);
    match super::diff::git_diff_text(&args, dir) {
        Ok(text) => DiffContent::from_unified(text),
        Err(reason) => DiffContent {
            error: Some(reason),
            ..DiffContent::default()
        },
    }
}

pub(crate) fn drain_pending_events() {
    while poll_event(Duration::from_millis(0)).unwrap_or(false) {
        if read_event().is_err() {
            break;
        }
    }
}

pub(crate) fn sync_mouse_capture(enabled: bool) {
    let mut out = stdout();
    if enabled {
        let _ = enable_mouse(&mut out);
    } else {
        let _ = disable_mouse(&mut out);
    }
}

pub(crate) fn resume_tui(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    mouse_enabled: bool,
) -> Result<(), String> {
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut out = stdout();
    if mouse_enabled {
        execute!(out, EnterAlternateScreen).map_err(|e| e.to_string())?;
        enable_mouse(&mut out).map_err(|e| e.to_string())?;
    } else {
        execute!(out, EnterAlternateScreen).map_err(|e| e.to_string())?;
    }
    push_keyboard_enhancement();
    let _ = terminal.hide_cursor();
    // The next frame closes this update, so the clear never shows alone.
    let _ = queue!(terminal.backend_mut(), BeginSynchronizedUpdate);
    let _ = terminal.resize(terminal_size_rect());
    let _ = terminal.clear();
    drain_pending_events();
    Ok(())
}

/// Read the queued copies of a held nav key (press / repeat / release)
/// and the first event after them, without dropping any.
///
/// Returns the copies in read order, then the first event that is not a
/// copy (`None` when the queue ran dry or a read failed). The reader
/// cannot unread, so the caller forwards all of them. The loop thread
/// decides per copy: it types in a text field, or it is dropped so a hold
/// cannot flush as a burst after release
/// ([`super::event_pump::keep_held_nav_backlog`]). The input thread cannot
/// make that call: the key that opens a text field may not be dispatched
/// yet.
pub(crate) fn take_held_nav_backlog(held: KeyEvent) -> (Vec<TtyEvent>, Option<TtyEvent>) {
    let mut backlog = Vec::new();
    while poll_event(Duration::from_millis(0)).unwrap_or(false) {
        let Ok(next) = read_event_origin() else {
            return (backlog, None);
        };
        if !is_held_nav_backlog(held, &next.0) {
            return (backlog, Some(next));
        }
        backlog.push(next);
    }
    (backlog, None)
}

/// Hand the terminal to a blocking editor or diff tool, wait for it, then
/// take the terminal back. The OSC 22 pointer goes back to the default
/// first, so a resize shape does not stay while the other program runs;
/// the next frame sends it again if the pointer still rests on an edge.
pub(crate) fn run_blocking_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    pointer: &mut TermPointer,
    cmd: &str,
    args: &[String],
    cwd: &Path,
    mouse_enabled: bool,
) -> Result<(), String> {
    let _ = pointer.reset(terminal.backend_mut());
    let _ = disable_raw_mode();
    let mut out = stdout();
    let _ = disable_mouse(&mut out);
    let _ = leave_alternate_screen_to(&mut out);
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

pub(crate) fn compute_reload_repo(
    cwd: &Path,
    snapshot: &WorkspaceSnapshot,
    repo: &str,
    show_hidden: bool,
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
        &snapshot.hidden_paths,
        show_hidden,
        &snapshot.filter_repos,
    )
}

/// Apply a watch poll. Returns true when the right pane must reload
/// (`HEAD` / sync note / dirty set / file signatures moved).
#[cfg(test)]
pub(crate) fn apply_watch_snapshot_for_tick(
    state: &mut AppState,
    snapshot: WorkspaceSnapshot,
) -> bool {
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

/// Sync pane git for unit tests of [`RightPaneRequest`].
#[cfg(test)]
fn load_right_headless(state: &mut AppState) {
    let payload = RightPaneRequest::from_state(state).compute();
    apply_right_pane_load(state, payload);
}

/// Discover every repo that is not under `ignoredPaths` (hidden ones
/// included) so `.` can show hidden repos without a walk.
///
/// `fetch_concurrency` caps the per-repo git work (resolved
/// `fetchConcurrency`).
pub fn collect_full_snapshot(
    cwd: &Path,
    config: &WorkspaceStatusConfig,
    filter_repos: &[String],
    show_hidden: bool,
    do_fetch: bool,
    fetch_concurrency: usize,
) -> WorkspaceSnapshot {
    let discover = discover_config(config);
    let only: Option<BTreeSet<String>> = if filter_repos.is_empty() {
        None
    } else {
        Some(filter_repos.iter().cloned().collect())
    };
    let snapshots = collect_snapshots(cwd, do_fetch, &discover, only.as_ref(), fetch_concurrency);
    build_workspace_snapshot(&snapshots, &config.hidden_paths, show_hidden, filter_repos)
}

/// Discovery config that still finds hidden repos so `.` can show them.
///
/// Clears only `hiddenPaths`; `ignoredPaths` stays a hard exclude.
pub(crate) fn discover_config(config: &WorkspaceStatusConfig) -> WorkspaceStatusConfig {
    WorkspaceStatusConfig {
        hidden_paths: Vec::new(),
        ignored_paths: config.ignored_paths.clone(),
        max_depth: config.max_depth,
        default_branches: config.default_branches.clone(),
        editor: config.editor.clone(),
        diff_tool: config.diff_tool.clone(),
        view_defaults: config.view_defaults,
        runtime: config.runtime.clone(),
    }
}

pub(crate) fn filter_repo_set(
    filter_repos: &[String],
) -> Option<std::collections::BTreeSet<String>> {
    if filter_repos.is_empty() {
        None
    } else {
        Some(filter_repos.iter().cloned().collect())
    }
}

/// Apply one checkout result; other repos stay on the previous generation.
pub(crate) fn apply_one_repo_snapshot(
    state: &mut AppState,
    path: &str,
    snap: Option<crate::snapshot::RepoSnapshot>,
) {
    let next =
        crate::snapshot::replace_repo_in_snapshot(&state.snapshot, path, snap, state.show_hidden);
    state.apply_watch_snapshot(next);
}

/// Drop checkouts the latest discovery no longer returned.
///
/// Streamed collect keeps unfinished paths from the new generation. Paths
/// that vanished (worktree remove, deleted repo) never get a `None` result,
/// so they must leave the snapshot when discovery completes.
pub(crate) fn drop_undiscovered_checkouts(state: &mut AppState, keep: &[String]) {
    let gone: Vec<String> = state
        .snapshot
        .repos
        .iter()
        .map(|row| row.repo.clone())
        .filter(|path| !keep.iter().any(|k| k == path))
        .collect();
    for path in gone {
        apply_one_repo_snapshot(state, &path, None);
    }
}

/// True when the focused checkout's identity or file signatures moved.
pub(crate) fn focused_repo_needs_pane(
    before_sigs: &std::collections::BTreeMap<String, String>,
    before_snap: &WorkspaceSnapshot,
    state: &AppState,
    repo: &str,
) -> bool {
    let before_id = before_snap
        .repos
        .iter()
        .find(|row| row.repo == repo)
        .map(super::watch::checkout_watch_identity);
    let after_id = state
        .snapshot
        .repos
        .iter()
        .find(|row| row.repo == repo)
        .map(super::watch::checkout_watch_identity);
    if before_id != after_id {
        return true;
    }
    let file = format!("file:{repo}:");
    let chrome_repo = format!("repo:{repo}");
    let chrome_co = format!("checkout:{repo}");
    let before: std::collections::BTreeMap<_, _> = before_sigs
        .iter()
        .filter(|(k, _)| k.starts_with(&file) || *k == &chrome_repo || *k == &chrome_co)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let after: std::collections::BTreeMap<_, _> = state
        .signatures
        .iter()
        .filter(|(k, _)| k.starts_with(&file) || *k == &chrome_repo || *k == &chrome_co)
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    before != after
}

#[cfg(test)]
mod tests;

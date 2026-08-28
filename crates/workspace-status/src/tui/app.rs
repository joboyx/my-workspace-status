//! TUI helpers shared by the async loop and Headless e2e.
//!
//! Live TTY I/O lives in [`super::event_loop`]. Effect schedule / spawn / apply
//! live in [`super::effect`]. This module keeps `run_tui` terminal setup and
//! pane/compute helpers.

use std::collections::BTreeSet;
use std::io::{self, stdout};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crossterm::event::{
    KeyEvent, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, size as terminal_size, EnterAlternateScreen,
    LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::Rect;
use ratatui::{Terminal, TerminalOptions, Viewport};

use crate::config::WorkspaceStatusConfig;
use crate::discovery::{collect_snapshots, process_repo, RepoCheckoutMeta};
use crate::git::{
    checkout_branch, diff_commit_file_ctx, diff_stash_file_ctx, fast_forward_to_remote_ref,
    git_diff_args, list_commit_name_status, list_stash_name_status, list_worktree_name_status,
    merge_into_head, repo_has_local_changes, rev_parse_quiet, MergeIntoHeadResult, NameStatus,
};
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
use super::graph_load::{
    load_graph_model, load_graph_model_window, refresh_graph_limit, GraphIdentity,
};
use super::keys::{event_to_action_with, is_held_nav_backlog};
use super::state::AppState;
use super::tty::{disable_mouse, enable_mouse, poll_event, read_event};
#[cfg(test)]
use super::watch::{checkout_watch_identities, watch_needs_pane_reload};

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
    if execute!(out, EnterAlternateScreen).is_err() || enable_mouse(&mut out).is_err() {
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
    let result = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .map_err(|_| 1u8)?;
        rt.block_on(super::event_loop::run(&mut terminal, &mut state, &opts))
    };
    restore_terminal();
    let _ = terminal.show_cursor();
    result
}

fn restore_terminal() {
    let _ = disable_raw_mode();
    let mut end = stdout();
    let _ = disable_mouse(&mut end);
    let _ = execute!(end, PopKeyboardEnhancementFlags, LeaveAlternateScreen);
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

pub(crate) fn map_event(state: &AppState, event: &crossterm::event::Event) -> Action {
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

pub(crate) fn apply_terminal_resize(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cols: u16,
    rows: u16,
) -> Result<(), u8> {
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
    show_ignored: bool,
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
            show_ignored: state.show_ignored,
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
                    self.show_ignored,
                    0,
                    self.graph_limit,
                    &self.graph_focus_branches,
                )
            } else {
                load_graph_model(
                    &self.cwd,
                    &self.snapshot,
                    repo,
                    self.show_ignored,
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

pub(crate) fn apply_right_pane_load(state: &mut AppState, payload: RightPaneLoad) {
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
    },
    Done {
        status: String,
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

pub(crate) fn apply_checkout_compute(
    state: &mut AppState,
    repo: String,
    result: CheckoutCompute,
) -> bool {
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

pub(crate) fn compute_commit_files(dir: &Path, source: &CommitFileSource) -> Vec<NameStatus> {
    match source {
        CommitFileSource::Commit { commit_id } => list_commit_name_status(dir, commit_id),
        CommitFileSource::Stash { stash_ref } => list_stash_name_status(dir, stash_ref),
        CommitFileSource::Worktree => list_worktree_name_status(dir),
    }
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

fn head_file_diff(dir: &Path, path: &str, context: Option<u32>) -> DiffContent {
    let args = git_diff_args(&["diff", "HEAD"], path, context);
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    DiffContent::from_unified(crate::git::exec_git(&refs, dir))
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
    let _ = terminal.resize(terminal_size_rect());
    let _ = terminal.clear();
    drain_pending_events();
    Ok(())
}

/// Drop queued copies of a held nav key (press / repeat / release).
///
/// Returns the first event that is not that backlog so it is not lost
/// (crossterm cannot unread).
pub(crate) fn discard_held_nav_backlog(held: KeyEvent) -> Option<crossterm::event::Event> {
    while poll_event(Duration::from_millis(0)).unwrap_or(false) {
        let Ok(event) = read_event() else {
            return None;
        };
        if !is_held_nav_backlog(held, &event) {
            return Some(event);
        }
    }
    None
}

pub(crate) fn run_blocking_editor(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    cmd: &str,
    args: &[String],
    cwd: &Path,
    mouse_enabled: bool,
) -> Result<(), String> {
    let _ = disable_raw_mode();
    let mut out = stdout();
    let _ = disable_mouse(&mut out);
    let _ = execute!(out, LeaveAlternateScreen);
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

/// Sync pane git for unit tests of [`RightPaneRequest`]. Headless e2e uses
/// [`super::effect::Interpreter::interpret_sync`].
#[cfg(test)]
fn load_right_headless(state: &mut AppState) {
    let payload = RightPaneRequest::from_state(state).compute();
    apply_right_pane_load(state, payload);
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

/// Discovery config that still finds ignored repos so `.` can show them.
pub(crate) fn discover_config(config: &WorkspaceStatusConfig) -> WorkspaceStatusConfig {
    WorkspaceStatusConfig {
        ignored_repos: Vec::new(),
        max_depth: config.max_depth,
        default_branches: config.default_branches.clone(),
        editor: config.editor.clone(),
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
        crate::snapshot::replace_repo_in_snapshot(&state.snapshot, path, snap, state.show_ignored);
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
mod tests {
    use super::*;
    use crate::config::WorkspaceStatusConfig;
    use crate::git::{exec_git, git_binary, list_local_branches, stage_file};
    use crate::tui::action::{Action, Effect};
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

    fn dummy_repo(name: &str) -> crate::snapshot::RepoSnapshot {
        crate::snapshot::RepoSnapshot {
            repo: name.into(),
            branch: "main".into(),
            sync_status: crate::snapshot::SyncStatus::NoUpstream,
            sync_note: String::new(),
            head: String::new(),
            has_unstaged: false,
            has_staged: false,
            has_untracked: false,
            changes: vec![],
            checkout_kind: CheckoutKind::Primary,
            primary_repo: None,
            merged_into_default: None,
            default_branch_override: None,
        }
    }

    #[test]
    fn drop_undiscovered_checkouts_removes_vanished_paths() {
        let snapshot =
            build_workspace_snapshot(&[dummy_repo("app"), dummy_repo("linked")], &[], false, &[]);
        let mut app = AppState::new(std::path::PathBuf::from("/tmp"), snapshot, true);
        assert_eq!(app.snapshot.repos.len(), 2);
        drop_undiscovered_checkouts(&mut app, &["app".into()]);
        let names: Vec<&str> = app
            .snapshot
            .repos
            .iter()
            .map(|row| row.repo.as_str())
            .collect();
        assert_eq!(names, vec!["app"]);
        drop_undiscovered_checkouts(&mut app, &["app".into()]);
        assert_eq!(app.snapshot.repos.len(), 1);
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
             a sync stage_file on the loop thread freezes the TUI until the child exits"
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

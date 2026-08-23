//! Crossterm event loop. First paint happens before any network fetch.

use std::collections::BTreeSet;
use std::io::{self, stdout};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::actions::{pull_behind_repos, switch_repo_to_default_branch};
use crate::config::WorkspaceStatusConfig;
use crate::discovery::collect_snapshots;
use crate::git::{
    checkout_branch, create_branch_at, create_branch_checkout, diff_commit_file_ctx,
    diff_stash_file_ctx, exec_git_checked, git_diff_args, latest_stash_ref,
    list_commit_name_status, list_local_branches, list_stash_name_status,
    list_worktree_name_status, origin_out_of_sync, pull_quiet, push_quiet, remove_untracked_file,
    remove_worktree, revert_tracked_file, stage_file, stash_apply, stash_drop, stash_pop,
    stash_push, unstage_file,
};
use crate::snapshot::{build_workspace_snapshot, WorkspaceSnapshot};

use super::action::{Action, Effect};
use super::diff::{load_file_diff, DiffContent};
use super::drill::CommitFileSource;
use super::editor::{editor_command, is_detached_editor, resolve_editor};
use super::fetch::fetch_interval_ms;
use super::graph_load::load_graph_model;
use super::keys::event_to_action_ex;
use super::render::draw;
use super::state::AppState;
use super::watch::watch_interval_ms;

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
    let mut terminal = match Terminal::new(backend) {
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

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
) -> Result<(), u8> {
    apply_effect(state, Effect::LoadRightPane, opts, terminal);
    terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
    if opts.start_fetch {
        let effect = state.dispatch(Action::Fetch);
        apply_effect(state, effect, opts, terminal);
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
        let timeout = Duration::from_millis(remain_watch.min(remain_fetch).min(200).max(10));
        if event::poll(timeout).unwrap_or(false) {
            let Ok(event) = event::read() else {
                continue;
            };
            if matches!(event, Event::Resize(_, _)) {
                terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
                continue;
            }
            let action = event_to_action_ex(
                &event,
                state.input_mode(),
                state.right_is_diff(),
                matches!(state.focus, super::state::FocusPane::Right),
                state.graph_stash_focused(),
                state.graph_commit_focused(),
            );
            let mouse_before = state.mouse_enabled;
            let effect = state.dispatch(action);
            if state.mouse_enabled != mouse_before {
                sync_mouse_capture(state.mouse_enabled);
            }
            if matches!(effect, Effect::Quit) {
                return Ok(());
            }
            apply_effect(state, effect, opts, terminal);
        } else {
            if watch_ms > 0 && last_watch.elapsed().as_millis() as u64 >= watch_ms {
                let effect = state.dispatch(Action::WatchTick);
                apply_effect(state, effect, opts, terminal);
                last_watch = Instant::now();
            }
            if fetch_ms > 0 && last_fetch.elapsed().as_millis() as u64 >= fetch_ms {
                let effect = state.dispatch(Action::FetchTick);
                apply_effect(state, effect, opts, terminal);
                last_fetch = Instant::now();
            }
        }
        terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
    }
}

fn apply_effect(
    state: &mut AppState,
    effect: Effect,
    opts: &TuiOpts,
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
) {
    match effect {
        Effect::None | Effect::Quit => {}
        Effect::Batch(effects) => {
            for child in effects {
                apply_effect(state, child, opts, terminal);
            }
        }
        Effect::Fetch { repos } => {
            for repo in &repos {
                let _ = exec_git_checked(&["fetch", "--quiet"], &opts.cwd.join(repo));
            }
            reload_snapshot(state, opts);
            state.status = format!("fetched {}", repos.join(" "));
            load_right(state);
        }
        Effect::Pull { repos } => {
            let _ = pull_behind_repos(&opts.cwd, &repos);
            reload_snapshot(state, opts);
            state.status = format!("pulled {}", repos.join(" "));
            load_right(state);
        }
        Effect::DefaultBranch { repos } => {
            for repo in &repos {
                let Some(snap) = state.snapshot.repos.iter().find(|r| r.repo == *repo) else {
                    continue;
                };
                let _ = switch_repo_to_default_branch(
                    repo,
                    &snap.branch,
                    &opts.cwd,
                    snap.default_branch_override.as_deref(),
                );
            }
            reload_snapshot(state, opts);
            state.status = format!("default-branch {}", repos.join(" "));
            load_right(state);
        }
        Effect::ReloadSnapshot => {
            reload_snapshot(state, opts);
            state.status = "refreshed".into();
            load_right(state);
        }
        Effect::LoadRightPane => load_right(state),
        Effect::Stage { repo, paths } => {
            let dir = opts.cwd.join(&repo);
            for path in &paths {
                if let Err(err) = stage_file(&dir, path) {
                    state.status = format!("stage failed: {err}");
                    return;
                }
            }
            reload_snapshot(state, opts);
            state.status = format!("staged {}", paths.last().map(String::as_str).unwrap_or(""));
            load_right(state);
        }
        Effect::Unstage { repo, paths } => {
            let dir = opts.cwd.join(&repo);
            for path in &paths {
                if let Err(err) = unstage_file(&dir, path) {
                    state.status = format!("unstage failed: {err}");
                    return;
                }
            }
            reload_snapshot(state, opts);
            state.status = format!(
                "unstaged {}",
                paths.last().map(String::as_str).unwrap_or("")
            );
            load_right(state);
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
                    return;
                }
            }
            for path in &untracked {
                if let Err(err) = remove_untracked_file(&dir, path) {
                    state.status = format!("revert failed: {err}");
                    return;
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
            load_right(state);
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
            let snapshot = collect_full_snapshot(
                &opts.cwd,
                &opts.config,
                &state.snapshot.filter_repos,
                state.show_ignored,
                false,
            );
            let _changed = state.apply_watch_snapshot(snapshot);
            load_right(state);
        }
        Effect::Push { repos } => {
            let mut ok = 0;
            let mut failed = 0;
            for repo in &repos {
                match push_quiet(&opts.cwd.join(repo)) {
                    Ok(()) => ok += 1,
                    Err(_) => failed += 1,
                }
            }
            reload_snapshot(state, opts);
            state.status = if failed > 0 && ok == 0 {
                format!("push: {failed} failed")
            } else if failed > 0 {
                format!("Pushed {ok} · {failed} failed")
            } else if ok == 1 {
                "Pushed".into()
            } else {
                format!("Pushed {ok}")
            };
            load_right(state);
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
                load_right(state);
            }
            Err(err) => state.status = format!("stash failed: {err}"),
        },
        Effect::StashApply { repo, stash_ref } => {
            match stash_apply(&opts.cwd.join(&repo), &stash_ref) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("applied {stash_ref}");
                    load_right(state);
                }
                Err(err) => state.status = format!("apply failed: {err}"),
            }
        }
        Effect::StashPop { repo, stash_ref } => {
            match stash_pop(&opts.cwd.join(&repo), &stash_ref) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("popped {stash_ref}");
                    load_right(state);
                }
                Err(err) => state.status = format!("pop failed: {err}"),
            }
        }
        Effect::StashDrop { repo, stash_ref } => {
            match stash_drop(&opts.cwd.join(&repo), &stash_ref) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("dropped {stash_ref}");
                    load_right(state);
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
            branch,
            pull_after,
        } => {
            let dir = opts.cwd.join(&repo);
            if !pull_after {
                if let Some(remote) = origin_out_of_sync(&dir, &branch) {
                    let _ = state.confirm_checkout_if_out_of_sync(repo, branch, Some(remote));
                    return;
                }
            }
            if checkout_branch(&branch, &dir) {
                if pull_after {
                    let _ = pull_quiet(&dir);
                }
                reload_snapshot(state, opts);
                state.status = format!("checked out {branch}");
                load_right(state);
            } else {
                state.status = format!("checkout failed: {branch}");
            }
        }
        Effect::CreateBranch { repo, name } => {
            match create_branch_checkout(&opts.cwd.join(&repo), &name) {
                Ok(()) => {
                    reload_snapshot(state, opts);
                    state.status = format!("created {name}");
                    load_right(state);
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
                load_right(state);
            }
            Err(err) => state.status = format!("create branch failed: {err}"),
        },
        Effect::RemoveWorktree {
            primary,
            path,
            force,
        } => match remove_worktree(&opts.cwd.join(&primary), &opts.cwd.join(&path), force) {
            Ok(()) => {
                reload_snapshot(state, opts);
                state.status = format!("removed worktree {path}");
                load_right(state);
            }
            Err(err) => state.status = format!("remove worktree failed: {err}"),
        },
        Effect::LoadCommitFiles { repo, source } => {
            load_commit_files(state, opts, repo, source);
        }
        Effect::LoadCommitDiff { repo, source, path } => {
            load_commit_diff(state, opts, repo, source, path);
        }
    }
}

/// Apply pane-load effects without a TTY. Used by the headless e2e harness.
pub(crate) fn apply_headless_effect(state: &mut AppState, effect: Effect, opts: &TuiOpts) {
    match effect {
        Effect::None | Effect::Quit => {}
        Effect::Batch(effects) => {
            for child in effects {
                apply_headless_effect(state, child, opts);
            }
        }
        Effect::LoadRightPane => load_right(state),
        Effect::ReloadSnapshot => {
            reload_snapshot(state, opts);
            state.status = "refreshed".into();
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
        let _ = event::read();
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
    state.apply_snapshot(snapshot);
}

fn load_right(state: &mut AppState) {
    if !state.drill.is_graph() {
        return;
    }
    if let Some((repo, change)) = state.focused_file() {
        let content = load_file_diff(
            &state.cwd,
            &repo,
            &change,
            state.workspace_diff_context(&repo, &change.path),
        );
        state.set_diff(repo, change.path, content);
        return;
    }
    if let Some(repo) = state.focused_graph_repo() {
        let (model, identity) =
            load_graph_model(&state.cwd, &state.snapshot, &repo, state.show_ignored);
        state.set_graph(model, identity.repo, identity.head);
        return;
    }
    state.clear_right();
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

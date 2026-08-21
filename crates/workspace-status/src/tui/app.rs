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
    checkout_branch, create_branch_checkout, exec_git_checked, latest_stash_ref,
    list_local_branches, origin_out_of_sync, pull_quiet, push_quiet, remove_untracked_file,
    revert_tracked_file, stage_file, stash_apply, stash_drop, stash_pop, stash_push,
    unstage_file,
};
use crate::snapshot::{build_workspace_snapshot, WorkspaceSnapshot};

use super::action::{Action, Effect};
use super::diff::load_file_diff;
use super::editor::{editor_command, is_detached_editor, resolve_editor};
use super::graph_load::load_graph_model;
use super::keys::event_to_action;
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
    terminal
        .draw(|frame| draw(frame, state))
        .map_err(|_| 1u8)?;
    if opts.start_fetch {
        let effect = state.dispatch(Action::Fetch);
        apply_effect(state, effect, opts, terminal);
        terminal
            .draw(|frame| draw(frame, state))
            .map_err(|_| 1u8)?;
    }

    let watch_ms = watch_interval_ms(std::env::var("WS_STATUS_WATCH_MS").ok().as_deref());
    let mut last_watch = Instant::now();
    loop {
        let timeout = if watch_ms == 0 {
            Duration::from_millis(200)
        } else {
            let remain = watch_ms.saturating_sub(last_watch.elapsed().as_millis() as u64);
            Duration::from_millis(remain.min(200).max(10))
        };
        if event::poll(timeout).unwrap_or(false) {
            let Ok(event) = event::read() else {
                continue;
            };
            if matches!(event, Event::Resize(_, _)) {
                terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
                continue;
            }
            let action = event_to_action(
                &event,
                state.input_mode(),
                state.right_is_diff(),
                matches!(state.focus, super::state::FocusPane::Right),
            );
            let effect = state.dispatch(action);
            if matches!(effect, Effect::Quit) {
                return Ok(());
            }
            apply_effect(state, effect, opts, terminal);
        } else if watch_ms > 0 && last_watch.elapsed().as_millis() as u64 >= watch_ms {
            let effect = state.dispatch(Action::WatchTick);
            apply_effect(state, effect, opts, terminal);
            last_watch = Instant::now();
        }
        terminal
            .draw(|frame| draw(frame, state))
            .map_err(|_| 1u8)?;
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
            state.status = format!("unstaged {}", paths.last().map(String::as_str).unwrap_or(""));
            load_right(state);
        }
        Effect::Revert {
            repo,
            paths,
            untracked,
        } => {
            let dir = opts.cwd.join(&repo);
            for path in &paths {
                let result = if untracked {
                    remove_untracked_file(&dir, path)
                } else {
                    revert_tracked_file(&dir, path)
                };
                if let Err(err) = result {
                    state.status = format!("revert failed: {err}");
                    return;
                }
            }
            reload_snapshot(state, opts);
            state.status = if untracked {
                format!("deleted {}", paths.last().map(String::as_str).unwrap_or(""))
            } else {
                format!("reverted {}", paths.last().map(String::as_str).unwrap_or(""))
            };
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
            } else if let Err(err) = run_blocking_editor(terminal, &cmd, &args, &opts.cwd.join(&repo))
            {
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
        Effect::StashCreate { repo, paths } => {
            match stash_push(&opts.cwd.join(&repo), &paths) {
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
            }
        }
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
    }
}

fn drain_pending_events() {
    while event::poll(Duration::from_millis(0)).unwrap_or(false) {
        let _ = event::read();
    }
}

fn resume_tui(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<(), String> {
    enable_raw_mode().map_err(|e| e.to_string())?;
    let mut out = stdout();
    execute!(out, EnterAlternateScreen, EnableMouseCapture).map_err(|e| e.to_string())?;
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
    let restore = resume_tui(terminal);
    match (spawn, restore) {
        (Ok(status), Ok(())) if status.success() => Ok(()),
        (Ok(status), Ok(())) => Err(format!(
            "editor exited {}",
            status.code().unwrap_or(-1)
        )),
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
    if let Some((repo, change)) = state.focused_file() {
        let lines = load_file_diff(&state.cwd, &repo, &change);
        state.set_diff(repo, change.path, lines);
        return;
    }
    if let Some(repo) = state.focused_graph_repo() {
        let (model, identity) = load_graph_model(
            &state.cwd,
            &state.snapshot,
            &repo,
            state.show_ignored,
        );
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
    build_workspace_snapshot(&snapshots, &config.ignored_repos, show_ignored, filter_repos)
}

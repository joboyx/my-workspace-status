//! Crossterm event loop. First paint happens before any network fetch.

use std::collections::BTreeSet;
use std::io::{self, stdout};
use std::path::Path;
use std::time::Duration;

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
use crate::git::exec_git_checked;
use crate::snapshot::{build_workspace_snapshot, WorkspaceSnapshot};

use super::action::{Action, Effect};
use super::diff::load_file_diff;
use super::graph_load::load_graph_model;
use super::keys::event_to_action;
use super::render::draw;
use super::state::AppState;

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
    execute!(out, EnterAlternateScreen, EnableMouseCapture).map_err(|_| 1u8)?;
    let backend = CrosstermBackend::new(out);
    let mut terminal = Terminal::new(backend).map_err(|_| 1u8)?;
    let result = run_loop(&mut terminal, &mut state, &opts);
    let _ = disable_raw_mode();
    let mut end = stdout();
    let _ = execute!(end, DisableMouseCapture, LeaveAlternateScreen);
    let _ = terminal.show_cursor();
    result
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
    opts: &TuiOpts,
) -> Result<(), u8> {
    apply_effect(state, Effect::LoadRightPane, opts);
    terminal
        .draw(|frame| draw(frame, state))
        .map_err(|_| 1u8)?;
    if opts.start_fetch {
        let effect = state.dispatch(Action::Fetch);
        apply_effect(state, effect, opts);
        terminal
            .draw(|frame| draw(frame, state))
            .map_err(|_| 1u8)?;
    }

    loop {
        if event::poll(Duration::from_millis(200)).unwrap_or(false) {
            let Ok(event) = event::read() else {
                continue;
            };
            if matches!(event, Event::Resize(_, _)) {
                terminal.draw(|frame| draw(frame, state)).map_err(|_| 1u8)?;
                continue;
            }
            let action = event_to_action(
                &event,
                state.help_open,
                state.right_is_diff(),
                matches!(state.focus, super::state::FocusPane::Right),
            );
            let effect = state.dispatch(action);
            if matches!(effect, Effect::Quit) {
                return Ok(());
            }
            apply_effect(state, effect, opts);
        }
        terminal
            .draw(|frame| draw(frame, state))
            .map_err(|_| 1u8)?;
    }
}

fn apply_effect(state: &mut AppState, effect: Effect, opts: &TuiOpts) {
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

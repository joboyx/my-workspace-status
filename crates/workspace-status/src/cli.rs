//! Headless workspace-status CLI (`workspace-status` / `ws`).

use std::collections::BTreeSet;
use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use crate::actions::{pull_behind_repos, switch_repo_to_default_branch};
use crate::config::load_workspace_status_config;
use crate::discovery::{collect_snapshots, validate_filter_repos};
use crate::helpers::{normalize_filter_repo, sorted_unique};
use crate::render::render_workspace_status;
use crate::snapshot::{
    build_summary_state, build_verbose_rows, build_workspace_snapshot, non_default_branch_repos,
    repo_snapshots_from_workspace, serialize_workspace_snapshot, visible_workspace_snapshot,
};
use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "workspace-status",
    about = "Workspace git status. TUI on a TTY. --plain / --json for agents.",
    long_about = "Display git repository status across repos in the workspace.\n\n\
On a TTY, this binary opens a ratatui TUI unless you pass --plain, --json,\n\
-v, -p, or -d. -i / --tui forces the TUI even when stdout is not a TTY.\n\
Those headless flags still win over --tui. Agents must pass --plain or --json.\n\
A non-TTY run without those flags prints --plain.\n\n\
--json pretty-prints the snapshot on stdout. Progress from --fetch, --pull,\n\
or --default-branch goes to stderr. --json wins when both --json and --plain\n\
are set. -v applies to --plain only.\n\n\
--update runs the cargo-dist updater (workspace-status-update) and exits.\n\
That run does not open the TUI or apply repo filters."
)]
struct Cli {
    /// Include ignored repos (`showIgnored`).
    #[arg(short = 'a', long = "all")]
    all: bool,

    /// Fetch remotes before status. Progress goes to stderr when --json.
    #[arg(short = 'f', long = "fetch")]
    fetch: bool,

    /// Verbose table in --plain only.
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Pull repos that are behind their upstream.
    #[arg(short = 'p', long = "pull")]
    pull: bool,

    /// Switch non-default branches to the default branch and pull.
    #[arg(short = 'd', long = "default-branch")]
    default_branch: bool,

    /// Human text of the workspace snapshot.
    #[arg(long = "plain")]
    plain: bool,

    /// Pretty-printed JSON snapshot on stdout.
    #[arg(long = "json")]
    json: bool,

    /// Force the TUI even when stdout is not a TTY.
    #[arg(short = 'i', long = "tui")]
    tui: bool,

    /// Run the installed updater and exit. Does not open the TUI.
    #[arg(long = "update")]
    update: bool,

    /// Optional workspace-relative repo filters. Named repos bypass ignoredRepos.
    #[arg(value_name = "REPO")]
    filter_repos: Vec<String>,
}

fn say(json: bool, line: &str) {
    if json {
        let _ = writeln!(io::stderr(), "{line}");
    } else {
        let _ = writeln!(io::stdout(), "{line}");
    }
}

pub fn cli_main() -> ExitCode {
    let cli = Cli::parse();
    if cli.update {
        return run_self_update();
    }
    let cwd = match env::current_dir() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    match run(cli, cwd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(code) => ExitCode::from(code),
    }
}

fn run(cli: Cli, cwd: PathBuf) -> Result<(), u8> {
    let filter_repos = sorted_unique(
        cli.filter_repos
            .iter()
            .map(|r| normalize_filter_repo(r))
            .filter(|r| !r.is_empty()),
    );
    let only_repos: Option<BTreeSet<String>> = if filter_repos.is_empty() {
        None
    } else {
        Some(filter_repos.iter().cloned().collect())
    };

    if let Some(only) = &only_repos {
        if let Err(unknown) = validate_filter_repos(&cwd, &only.iter().cloned().collect::<Vec<_>>())
        {
            eprintln!("Unknown repo: {unknown}");
            return Err(1);
        }
    }

    let loaded = match load_workspace_status_config(&cwd) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("{err}");
            return Err(1);
        }
    };

    let mut config = loaded.clone();
    if cli.all {
        config.ignored_repos = Vec::new();
    }

    let flags = crate::tui::HeadlessFlags {
        plain: cli.plain,
        json: cli.json,
        verbose: cli.verbose,
        pull: cli.pull,
        default_branch: cli.default_branch,
        force_tui: cli.tui,
    };
    if crate::tui::should_open_tui(io::stdout().is_terminal(), flags) {
        let snapshot =
            crate::tui::collect_full_snapshot(&cwd, &loaded, &filter_repos, cli.all, false);
        return crate::tui::run_tui(crate::tui::TuiOpts {
            cwd,
            snapshot,
            config: loaded,
            start_fetch: cli.fetch,
        });
    }

    let force_json = cli.json;
    if cli.fetch {
        say(
            force_json,
            "🔄 Fetching from remotes (this may take a moment)...",
        );
        say(force_json, "");
    }

    let mut snapshots = collect_snapshots(&cwd, cli.fetch, &config, only_repos.as_ref());
    let mut summary = build_summary_state(&snapshots);

    if cli.pull && !summary.sync_behind.is_empty() {
        say(force_json, "⬇️ Pulling repos that are behind...");
        say(force_json, "");
        for line in pull_behind_repos(&cwd, &sorted_unique(summary.sync_behind.iter().cloned())) {
            say(force_json, &line);
        }
        say(force_json, "");
        say(force_json, "🔄 Re-checking status after pull...");
        say(force_json, "");
        snapshots = collect_snapshots(&cwd, false, &config, only_repos.as_ref());
        summary = build_summary_state(&snapshots);
    }

    if cli.default_branch {
        let to_switch = non_default_branch_repos(&summary);
        if to_switch.is_empty() {
            say(force_json, "  ℹ️ No non-default branches found to switch");
        } else {
            say(force_json, "🔄 Switching to default branch and pulling...");
            say(force_json, "");
            let mut switched = 0u32;
            for repo in &to_switch {
                let Some(snapshot) = snapshots.iter().find(|s| s.repo == *repo) else {
                    continue;
                };
                let (ok, lines) = switch_repo_to_default_branch(
                    repo,
                    &snapshot.branch,
                    &cwd,
                    snapshot.default_branch_override.as_deref(),
                );
                for line in lines {
                    say(force_json, &line);
                }
                if ok {
                    switched += 1;
                }
            }
            if switched > 0 {
                say(force_json, "");
                say(force_json, "🔄 Re-checking status after switch...");
                say(force_json, "");
                snapshots = collect_snapshots(&cwd, false, &config, only_repos.as_ref());
            }
        }
    }

    let workspace =
        build_workspace_snapshot(&snapshots, &loaded.ignored_repos, cli.all, &filter_repos);
    let published = visible_workspace_snapshot(&workspace);

    if force_json {
        print!("{}", serialize_workspace_snapshot(&published));
        return Ok(());
    }

    let visible = repo_snapshots_from_workspace(&published);
    let visible_summary = build_summary_state(&visible);
    let verbose = build_verbose_rows(&visible);
    for line in render_workspace_status(&visible, &visible_summary, &verbose, cli.verbose) {
        println!("{line}");
    }
    Ok(())
}

fn updater_file_name() -> &'static str {
    if cfg!(windows) {
        "workspace-status-update.exe"
    } else {
        "workspace-status-update"
    }
}

fn sibling_updater(current_exe: &Path) -> Option<PathBuf> {
    let path = current_exe.parent()?.join(updater_file_name());
    path.is_file().then_some(path)
}

fn updater_command(current_exe: &Path) -> Command {
    if let Some(path) = sibling_updater(current_exe) {
        Command::new(path)
    } else {
        Command::new("workspace-status-update")
    }
}

fn run_self_update() -> ExitCode {
    let exe = match env::current_exe() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("failed to resolve current executable: {err}");
            return ExitCode::from(1);
        }
    };
    let mut cmd = updater_command(&exe);
    exec_updater(&mut cmd)
}

#[cfg(unix)]
fn exec_updater(cmd: &mut Command) -> ExitCode {
    use std::os::unix::process::CommandExt;
    let err = cmd.exec();
    eprintln!("failed to run workspace-status-update: {err}");
    ExitCode::from(1)
}

#[cfg(not(unix))]
fn exec_updater(cmd: &mut Command) -> ExitCode {
    match cmd.status() {
        Ok(status) => status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .map(ExitCode::from)
            .unwrap_or(ExitCode::from(1)),
        Err(err) => {
            eprintln!("failed to run workspace-status-update: {err}");
            ExitCode::from(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir() -> PathBuf {
        let path = env::temp_dir().join(format!(
            "ws-cli-update-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn sibling_updater_finds_file_next_to_exe() {
        let dir = temp_dir();
        let exe = dir.join("ws");
        fs::write(&exe, b"").unwrap();
        let sidecar = dir.join(updater_file_name());
        fs::write(&sidecar, b"").unwrap();
        assert_eq!(sibling_updater(&exe).as_deref(), Some(sidecar.as_path()));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sibling_updater_none_without_sidecar() {
        let dir = temp_dir();
        let exe = dir.join("ws");
        fs::write(&exe, b"").unwrap();
        assert_eq!(sibling_updater(&exe), None);
        let _ = fs::remove_dir_all(dir);
    }
}

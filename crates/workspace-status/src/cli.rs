//! Headless workspace-status CLI (`workspace-status` / `ws`).

use std::collections::BTreeSet;
use std::env;
use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use crate::actions::{pull_behind_repos, switch_repo_to_default_branch};
use crate::config::load_workspace_status_config;
use crate::discovery::{collect_snapshots, validate_filter_repos};
use crate::helpers::{normalize_filter_repo, sorted_unique};
use crate::render::render_workspace_status;
use crate::snapshot::{
    build_summary_state, build_verbose_rows, build_workspace_snapshot, non_default_branch_repos,
    repo_snapshots_from_workspace, serialize_workspace_snapshot, visible_workspace_snapshot,
};
use crate::update::run_self_update;
use crate::update_check::{offer_startup_update, StartupUpdateOffer};
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
On a TTY TUI launch, if the last GitHub Release check is older than 6 hours\n\
and a newer published release exists, the process asks whether to update\n\
before the TUI mounts. --plain, --json, and --update skip that check.\n\n\
--update prints GitHub Release notes for versions newer than this install,\n\
then runs the cargo-dist updater (workspace-status-update) and exits.\n\
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

    /// Print GitHub Release notes since this version, then run the updater.
    #[arg(long = "update")]
    update: bool,

    /// Pin the workspace root (absolute or relative path).
    #[arg(short = 'C', long = "workspace", value_name = "PATH")]
    workspace: Option<PathBuf>,

    /// Optional workspace-relative repo filters. Named repos bypass ignoredRepos.
    #[arg(value_name = "REPO")]
    filter_repos: Vec<String>,
}

/// Resolve the workspace root to a canonical absolute directory.
///
/// Precedence: CLI `--workspace` / `-C`, then `WS_STATUS_WORKSPACE`, then
/// `current_dir`. Blank or whitespace-only env is unset. A whitespace-only
/// CLI path is an error because the flag was passed. Relative paths join onto
/// `current_dir` before canonicalize. Missing paths and non-directories error;
/// there is no silent fallback.
pub(crate) fn resolve_workspace_root(
    cli_workspace: Option<&Path>,
    env_workspace: Option<&str>,
    current_dir: &Path,
) -> Result<PathBuf, String> {
    let chosen = if let Some(cli) = cli_workspace {
        if cli.as_os_str().is_empty() || cli.to_string_lossy().trim().is_empty() {
            return Err(format!("workspace path is missing: {}", cli.display()));
        }
        join_workspace_path(cli, current_dir)
    } else if let Some(env) = env_workspace.map(str::trim).filter(|s| !s.is_empty()) {
        join_workspace_path(Path::new(env), current_dir)
    } else {
        current_dir.to_path_buf()
    };
    canonicalize_workspace_dir(&chosen)
}

fn join_workspace_path(path: &Path, current_dir: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        current_dir.join(path)
    }
}

fn canonicalize_workspace_dir(path: &Path) -> Result<PathBuf, String> {
    let canonical = path
        .canonicalize()
        .map_err(|_| format!("workspace path is missing: {}", path.display()))?;
    if !canonical.is_dir() {
        return Err(format!(
            "workspace path is not a directory: {}",
            canonical.display()
        ));
    }
    Ok(canonical)
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
    let env_workspace = env::var("WS_STATUS_WORKSPACE").ok();
    let cwd = match resolve_workspace_root(cli.workspace.as_deref(), env_workspace.as_deref(), &cwd)
    {
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
        if offer_startup_update() == StartupUpdateOffer::RunUpdater {
            // Unix exec replaces the process. If the sidecar returns (Windows, or
            // exec failed), continue into the TUI instead of exiting.
            let _ = run_self_update();
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn unique_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ws-root-{tag}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn canonical(path: &Path) -> PathBuf {
        path.canonicalize().unwrap()
    }

    #[test]
    fn resolve_workspace_root_cli_beats_env_and_cwd() {
        let cli_dir = unique_dir("cli");
        let env_dir = unique_dir("env");
        let cwd_dir = unique_dir("cwd");
        let got = resolve_workspace_root(
            Some(cli_dir.as_path()),
            Some(env_dir.to_str().unwrap()),
            &cwd_dir,
        )
        .unwrap();
        assert_eq!(got, canonical(&cli_dir));
        let _ = fs::remove_dir_all(&cli_dir);
        let _ = fs::remove_dir_all(&env_dir);
        let _ = fs::remove_dir_all(&cwd_dir);
    }

    #[test]
    fn resolve_workspace_root_env_beats_cwd_when_flag_is_none() {
        let env_dir = unique_dir("env");
        let cwd_dir = unique_dir("cwd");
        let got = resolve_workspace_root(None, Some(env_dir.to_str().unwrap()), &cwd_dir).unwrap();
        assert_eq!(got, canonical(&env_dir));
        let _ = fs::remove_dir_all(&env_dir);
        let _ = fs::remove_dir_all(&cwd_dir);
    }

    #[test]
    fn resolve_workspace_root_blank_and_whitespace_env_are_ignored() {
        let cwd_dir = unique_dir("cwd");
        let expected = canonical(&cwd_dir);
        let blank = resolve_workspace_root(None, Some(""), &cwd_dir).unwrap();
        let whitespace = resolve_workspace_root(None, Some(" \t\n"), &cwd_dir).unwrap();
        assert_eq!(blank, expected);
        assert_eq!(whitespace, expected);
        let _ = fs::remove_dir_all(&cwd_dir);
    }

    #[test]
    fn resolve_workspace_root_missing_path_errors() {
        let cwd_dir = unique_dir("cwd");
        let missing = cwd_dir.join("no-such-workspace");
        let err = resolve_workspace_root(Some(missing.as_path()), None, &cwd_dir).unwrap_err();
        assert!(
            err.contains("missing"),
            "missing path should say it is missing: {err}"
        );
        assert!(
            err.contains("no-such-workspace"),
            "missing path error should include the path: {err}"
        );
        let env_err =
            resolve_workspace_root(None, Some(missing.to_str().unwrap()), &cwd_dir).unwrap_err();
        assert!(
            env_err.contains("missing"),
            "missing env path should say it is missing: {env_err}"
        );
        let _ = fs::remove_dir_all(&cwd_dir);
    }

    #[test]
    fn resolve_workspace_root_file_path_errors() {
        let cwd_dir = unique_dir("cwd");
        let file = cwd_dir.join("not-a-dir");
        fs::write(&file, b"x").unwrap();
        let err = resolve_workspace_root(Some(file.as_path()), None, &cwd_dir).unwrap_err();
        assert!(
            err.contains("not a directory"),
            "file path should say it is not a directory: {err}"
        );
        assert!(
            err.contains("not-a-dir"),
            "file path error should include the path: {err}"
        );
        let _ = fs::remove_dir_all(&cwd_dir);
    }

    #[test]
    fn resolve_workspace_root_relative_path_joins_current_dir() {
        let cwd_dir = unique_dir("cwd");
        let child = cwd_dir.join("child");
        fs::create_dir(&child).unwrap();
        let got = resolve_workspace_root(Some(Path::new("child")), None, &cwd_dir).unwrap();
        assert_eq!(got, canonical(&child));
        let env_got = resolve_workspace_root(None, Some("child"), &cwd_dir).unwrap();
        assert_eq!(env_got, canonical(&child));
        let _ = fs::remove_dir_all(&cwd_dir);
    }

    #[test]
    fn resolve_workspace_root_whitespace_cli_is_error() {
        let cwd_dir = unique_dir("cwd");
        let err = resolve_workspace_root(Some(Path::new("   ")), None, &cwd_dir).unwrap_err();
        assert_ne!(err, String::new());
        let _ = fs::remove_dir_all(&cwd_dir);
    }
}

//! Headless workspace-status CLI (`workspace-status` / `ws`).

use std::collections::BTreeSet;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use crate::actions::{pull_behind_repos, switch_repo_to_default_branch};
use crate::config::load_workspace_status_config;
use crate::discovery::{collect_snapshots, validate_filter_repos};
use crate::helpers::{normalize_filter_repo, sorted_unique};
use crate::render::render_workspace_status;
use crate::snapshot::{
    build_summary_state, build_verbose_rows, build_workspace_snapshot, non_default_branch_repos,
    repo_snapshots_from_workspace, serialize_workspace_snapshot, visible_workspace_snapshot,
};

#[derive(Parser, Debug)]
#[command(
    name = "workspace-status",
    about = "Workspace git status. TUI on a TTY. --plain / --json for agents.",
    long_about = "Display git repository status across repos in the workspace.\n\n\
On a TTY, this binary opens a ratatui TUI unless you pass --plain, --json,\n\
-v, -p, or -d. Agents must pass --plain or --json.\n\
A non-TTY run without those flags prints --plain.\n\n\
--json pretty-prints the snapshot on stdout. Progress from --fetch, --pull,\n\
or --default-branch goes to stderr. --json wins when both --json and --plain\n\
are set. -v applies to --plain only."
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
    let cwd = match std::env::current_dir() {
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
    };
    if crate::tui::should_open_tui(io::stdout().is_terminal(), flags) {
        let snapshot = crate::tui::collect_full_snapshot(
            &cwd,
            &loaded,
            &filter_repos,
            cli.all,
            false,
        );
        return crate::tui::run_tui(crate::tui::TuiOpts {
            cwd,
            snapshot,
            config: loaded,
            start_fetch: cli.fetch,
        });
    }

    let force_json = cli.json;
    if cli.fetch {
        say(force_json, "🔄 Fetching from remotes (this may take a moment)...");
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

    let workspace = build_workspace_snapshot(
        &snapshots,
        &loaded.ignored_repos,
        cli.all,
        &filter_repos,
    );
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

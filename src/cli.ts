/**
 * CLI argument parsing for workspace-status.
 */

import { normalizeFilterRepo, sortedUnique } from './helpers.js';
import type { CliFlags } from './types.js';

export const HELP = `Usage: workspace-status.sh [OPTIONS] [REPO...]

Display git repository status across all repos in the workspace.

CONFIG:
    .workspace-status-config.json in the current workspace root:
    {
      "ignoredRepos": ["notes"],
      "maxDepth": 3,
      "defaultBranches": {
        "acme/acme-main": "develop"
      }
    }
    ignoredRepos skips listed repos. maxDepth (default 3) is how many path
    segments below cwd to search for git repos (e.g. acme/light-modules/spa).
    defaultBranches optionally overrides the default branch per repo path
    (classification, markers, and --default-branch / TUI d). Absent entries
    keep git/heuristic defaults (main|master|develop classification; git
    origin/HEAD for switch).

REPOS:
    Optional repo paths relative to the workspace root (e.g. dotfiles notes).
    When specified, only those repos are checked. Named repos are included even
    if listed in ignoredRepos.

OPTIONS:
    -a, --all                Include all repos, ignoring .workspace-status-config.json ignoredRepos
    -f, --fetch              Fetch from remotes before checking status
    -v, --verbose            Show detailed repository table before the summary
    -p, --pull               Pull repos that are behind their upstream
    -d, --default-branch     Switch non-default branches to default branch and pull
    -i, --tui                Force interactive TUI (even when stdout is not a TTY)
        --plain              Force plain text report (required for agents; disables TUI)
        --json               Print the workspace snapshot as JSON (disables TUI)
    -h, --help               Show this help message

EXAMPLES:
    workspace-status.sh                    # Show status summary
    workspace-status.sh dotfiles             # Status for dotfiles only
    workspace-status.sh dotfiles notes -v  # Verbose status for two repos
    workspace-status.sh -a                 # Include repos ignored by workspace config
    workspace-status.sh -f                 # Fetch remotes first, then show status
    workspace-status.sh -v                 # Show detailed table plus summary
    workspace-status.sh -d                 # Switch non-default branches to default
    workspace-status.sh -f -v              # Fetch and show verbose output
    workspace-status.sh --plain            # Plain text report (no TUI)
    workspace-status.sh --json             # Workspace snapshot as JSON (no TUI)
    workspace-status.sh -i                 # Force TUI

OUTPUT:
    By default, the script displays a categorized summary:
    - 📁 File changes: Repos with uncommitted, staged, or untracked files
    - 🔄 Sync status: Repos behind, ahead, or diverged from upstream
    - 🌿 Branches: Repos on any non-default branch (feature, bugfix, chore, release, unknown)

    With --verbose, it also shows an aligned repo table with branch, sync, and change indicators.

    On a TTY (without -v/-p/-d/--plain/--json), an interactive TUI is used instead.
    TUI keys: j/k move · h/l fold · s/u/x stage/unstage/revert · i diff mode ·
    t tree · r refresh · / filter · ? help · Ctrl-C twice quit.
    Agents MUST pass --plain or --json (TTY without one hangs the shell on the TUI).
    --plain is the human renderer of the snapshot. --json prints the same snapshot.

TUI REQUIREMENTS:
    The TUI needs a Nerd Font in the terminal. Without one, file-type icons and
    git glyphs render as empty boxes. Recommended: MesloLGM Nerd Font Mono.
    Use the "Mono" variant — the proportional build renders icons ~2 columns
    wide and breaks the layout.

    VS Code / Cursor use their own terminal font, separate from the one the
    system terminal uses. Set it in User Settings (not the WSL Machine
    settings, since the font is resolved on the client):

        "terminal.integrated.fontFamily": "'MesloLGM Nerd Font Mono', monospace"

TUI ENVIRONMENT:
    WS_STATUS_GLYPHS=ascii   Replace Nerd Font icons with plain ASCII markers
    WS_STATUS_WATCH_MS=<ms>  Live-refresh poll period (default 3000, 0 disables)
`;

/**
 * Parse CLI argv into typed flags. Does not include the node/script path.
 */
export function parseArgs(argv: string[]): CliFlags {
  const flags: CliFlags = {
    doFetch: false,
    verbose: false,
    doPull: false,
    doDefaultBranch: false,
    includeAll: false,
    forcePlain: false,
    forceJson: false,
    forceTui: false,
    filterRepos: [],
  };

  const applyShortFlag = (flag: string): void => {
    if (flag === 'h') {
      console.log(HELP);
      process.exit(0);
    }
    if (flag === 'a') flags.includeAll = true;
    else if (flag === 'f') flags.doFetch = true;
    else if (flag === 'v') flags.verbose = true;
    else if (flag === 'p') flags.doPull = true;
    else if (flag === 'd') flags.doDefaultBranch = true;
    else if (flag === 'i') flags.forceTui = true;
    else {
      console.error(`Unknown option: -${flag}`);
      process.exit(1);
    }
  };

  for (const arg of argv) {
    if (arg === '--help') {
      console.log(HELP);
      process.exit(0);
    }
    if (arg === '--all') flags.includeAll = true;
    else if (arg === '--fetch') flags.doFetch = true;
    else if (arg === '--verbose') flags.verbose = true;
    else if (arg === '--pull') flags.doPull = true;
    else if (arg === '--default-branch') flags.doDefaultBranch = true;
    else if (arg === '--plain') flags.forcePlain = true;
    else if (arg === '--json') flags.forceJson = true;
    else if (arg === '--tui') flags.forceTui = true;
    else if (arg.startsWith('-') && arg.length > 1) {
      for (const flag of arg.slice(1)) applyShortFlag(flag);
    } else {
      flags.filterRepos.push(normalizeFilterRepo(arg));
    }
  }
  flags.filterRepos = sortedUnique(flags.filterRepos);
  return flags;
}

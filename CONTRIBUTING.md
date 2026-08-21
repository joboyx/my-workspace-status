# Contributing

## Setup

1. Install Node 20 or later, npm, and git.
2. Clone this repository.
3. Run `npm ci` in the repository root.

The launcher also installs and builds when node_modules or dist is missing or stale.

After npm link, the commands are workspace-status and ws.

## Checks

Run the same suite that CI runs:

```bash
npm test
```

Build with `npm run build`.

Run the tool with `./workspace-status.sh --plain`.

Agents and CI must use --plain. A TTY run without --plain opens the TUI.

## Pull requests

Open a pull request against main. GitHub Actions runs `npm test` on pull requests and on main.

Keep SAMPLE_OUTPUT.md in sync with test/workspace-status.e2e.ts when output changes.

New node:test files must be listed in the test script in package.json.

## Code map

See docs/architecture.md and AGENTS.md.

## Known test skips

These three tests fail on the current suite and are skipped so CI stays green:

- pushQuiet advances remote when ahead and fails when diverged
- pullQuietDetailed stashes dirty changes, pulls, then reapplies
- linked .worktrees show Files / merge marks, then after merge into main

They come from the extracted tool. Do not treat a skip as a new regression until someone owns a fix.

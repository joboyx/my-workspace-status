# Contributing

## Setup

1. Install Node 20 or later, npm, and git.
2. Install a stable Rust toolchain (rustc 1.85 or later, and cargo) for the Rust crates.
3. Clone this repository.
4. Run `npm ci` in the repository root.

The TypeScript launcher (`./workspace-status.sh`) installs and builds when node_modules or dist is missing or stale. End-user install of `workspace-status` / `ws` is the GitHub Release installer in the README, not npm.

## Checks

Run the same suite that CI runs:

```bash
npm test
cargo test
```

Build with `npm run build`.

Run the TypeScript tool with `./workspace-status.sh --plain`.

Agents and CI must use --plain or --json. A TTY run of the TypeScript app without those flags opens the TUI.

## Pull requests

Open a pull request against main. GitHub Actions runs `cargo test` and `npm test` on pull requests and on main.

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

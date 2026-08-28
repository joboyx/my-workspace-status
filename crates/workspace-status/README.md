# workspace-status

Git status for every repo and worktree under the current directory, in one TUI.

On a TTY, `workspace-status` and `ws` open a ratatui TUI.
`--plain` prints human text. `--json` prints the same snapshot as JSON.
`-v`, `-p`, and `-d` stay headless. A non-TTY run without those flags prints `--plain`.
A TTY launch may ask to install a newer GitHub Release before the TUI; `--plain` / `--json` / `--update` skip that.

Screenshots of the tree, graph, diff, and overlays: [root README](../../README.md#screenshots).

Install from the public GitHub Release installer (see the [root README](../../README.md#install)), or from this crate:

```bash
cargo install --path crates/workspace-status --locked
```

From git (this repository is a Cargo workspace, so name the package):

```bash
cargo install --git https://github.com/joboyx/my-workspace-status --locked --package workspace-status
```

GitHub Releases and `cargo install` both install `workspace-status` and `ws`.
A GitHub Release install also includes `workspace-status-update`. `ws --update` prints GitHub Release notes since the installed version, then runs that sidecar.

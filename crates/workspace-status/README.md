# workspace-status

Rust CLI for the workspace snapshot contract.

On a TTY, `workspace-status` and `ws` open a ratatui TUI.
`--plain` prints human text. `--json` prints the same snapshot as JSON.
`-v`, `-p`, and `-d` stay headless. A non-TTY run without those flags prints `--plain`.

Screenshots of the tree, graph, diff, and overlays: [root README](../../README.md#screenshots).

The TypeScript Ink app is still in this repository for features the Rust TUI does not cover. See [docs/tui-rust.md](../../docs/tui-rust.md).

Install from the public GitHub Release installer (see the [root README](../../README.md#install)), or from this crate:

```bash
cargo install --path crates/workspace-status --locked
```

From git (this repository is a Cargo workspace, so name the package):

```bash
cargo install --git https://github.com/joboyx/my-workspace-status --locked --package workspace-status
```

GitHub Releases and `cargo install` both install `workspace-status` and `ws`.

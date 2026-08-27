# Contributing

## Setup

1. Install a stable Rust toolchain (rustc 1.85 or later, and cargo). This repository pins 1.85 in `rust-toolchain.toml`.
2. Clone this repository.
3. Install git.

End-user install of `workspace-status` / `ws` is the GitHub Release installer in the README.

From a local clone:

```bash
cargo install --path crates/workspace-status --locked
```

Or run without installing:

```bash
cargo run -p workspace-status -- --plain
```

## Checks

Run the same suite that CI runs:

```bash
cargo test --workspace
```

That includes the real-TTY PTY e2e on Unix (`crates/workspace-status/tests/tui_tty_e2e/`). GitHub Actions also runs an xfce4-terminal + xdotool job (`tui-tty-desktop`). See [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md).

Agents and CI must use `--plain` or `--json`. A TTY run without those flags opens the TUI.

## Pull requests

Open a pull request against main. GitHub Actions runs `cargo test --workspace` and the xfce TTY e2e job on pull requests and on main.

Keep `SAMPLE_OUTPUT.md` in sync with `crates/workspace-status/tests/snapshot_contract.rs` when output changes.

## Code map

See `docs/architecture.md` and `AGENTS.md`.

Tag `ink-tui` on this repository is a historical snapshot only. It is not an install path.

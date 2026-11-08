# Contributing

## Setup

1. Install a stable Rust toolchain (rustc 1.85 or later, and cargo). This repository pins 1.85 in `rust-toolchain.toml`.
2. Clone this repository.
3. Install git.

With rustup installed, `scripts/agent-bootstrap.sh` does the rest: it installs the pinned toolchain with clippy and rustfmt, fetches crates, and builds the test binaries.

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

CI runs this gate (jobs `lint` and `cargo`). Run all three before you push:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

`--locked` fails when `Cargo.lock` does not match `Cargo.toml`, as it does in CI. The test run includes the real-TTY PTY e2e on Unix (`crates/workspace-status/tests/tui_tty_e2e/`). GitHub Actions also runs xfce4-terminal (keys) and xterm (XTEST wheel) under `tui-tty-desktop` via `scripts/with-desktop-session.sh`. See [docs/tui-tty-e2e.md](./docs/tui-tty-e2e.md).

Commands to run one test: **Start here** in [AGENTS.md](./AGENTS.md).

Agents and CI must use `--plain` or `--json`. A TTY run without those flags opens the TUI.

## cargo-dist generate

`.github/workflows/release.yml` is generated. Before you run `dist generate`, read **`dist generate`** in [AGENTS.md](./AGENTS.md).

## Pull requests

Open a pull request against main and fill in the template (`.github/pull_request_template.md`). GitHub Actions runs `lint`, `cargo test --workspace --locked` on Linux, macOS and Windows (`cargo`, `cargo-macos`, `cargo-windows`) and the desktop TTY e2e job (`tui-tty-desktop`: xfce keys + xterm XTEST wheel) on pull requests and on main. A push to main is tagged and released only after all of them pass. See **Release gate** in [docs/architecture.md](./docs/architecture.md).

Keep `SAMPLE_OUTPUT.md` in sync with `crates/workspace-status/tests/snapshot_contract.rs` when output changes.

## Code map

See `docs/architecture.md` and `AGENTS.md`.

Tag `ink-tui` on this repository is a historical snapshot only. It is not an install path.

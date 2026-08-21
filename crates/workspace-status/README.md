# workspace-status

Headless CLI for the workspace snapshot contract.

`--plain` prints human text. `--json` prints the same snapshot as JSON.
The TypeScript Ink app is still the interactive TUI.

Install from this crate:

```bash
cargo install --path crates/workspace-status --locked
```

From git (this repository is a Cargo workspace, so name the package):

```bash
cargo install --git https://github.com/joboyx/my-workspace-status --locked --package workspace-status
```

Both commands install `workspace-status` and `ws`.

# workspace-status-graph

Ratatui widget for the workspace-status git graph.

The widget paints HEAD, sync, stash, and worktree markers from a
`GraphModel`. Hidden ignored worktree rows stay omitted unless
`show_ignored` is true.

See [docs/graph.md](../../docs/graph.md) for the widget contract.

```bash
cargo test -p workspace-status-graph
```

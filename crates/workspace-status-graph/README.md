# workspace-status-graph

Ratatui widget for the workspace-status git graph.

The widget paints a multi-lane gutter plus HEAD, sync, stash, and
worktree markers from a `GraphModel`. Worktree marks are linked extras
only (not the main checkout). Commit rows are two lines:
subject on the node, then refs / short hash / relative date / author
on the spacer. A leftover chip whose name still partly fits is
truncated with `…` (brackets stay). `[+N]` counts only chips that are
fully hidden after that. The
selection footer lists every ref in the same chip colours as the row.
Hidden ignored worktree rows stay omitted unless `show_ignored` is true.

See [docs/graph.md](../../docs/graph.md) for the widget contract.

```bash
cargo test -p workspace-status-graph
```

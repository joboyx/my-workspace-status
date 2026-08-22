# Demo workspace

Use one seeded workspace for every screenshot and video of the Rust TUI.

## Seed

From this repository:

    ./scripts/seed-demo-workspace.sh

The script wipes and recreates `tmp/demo-workspace` under the repository root.
Pass a directory to seed somewhere else. Remotes are local bare repos under
`.remotes/`, so ahead, behind, and diverged states do not need the network.

`tmp/` is gitignored. Do not commit the seed output.

## Run

    cd tmp/demo-workspace
    workspace-status

On a TTY this opens the Rust TUI (`workspace-status` / `ws`).
Use `--plain` for a smoke check. Keys live in [tui-rust.md](./tui-rust.md).
Shot steps live in [../scripts/demo-shots.md](../scripts/demo-shots.md).

Default theme is Tokyo Night. Leave `WS_STATUS_THEME` unset, or set
`WS_STATUS_THEME=tokyo-night`.

## Workspace

| Path | Why it is here |
| --- | --- |
| `app` | Feature branch, ahead of origin, dirty (staged, unstaged, untracked). Linked worktree at `app/.worktrees/feat-login`. |
| `services/api` | Feature branch, dirty, diverged from its local remote. |
| `lib` | Clean default branch. Folds under **No updates**. |
| `notes` | Dirty and listed in `ignoredRepos`. Hidden until `.` or `-a`. |
| `merger` | Non-default branch with a merge commit and a stash. Graph shows two lanes and a stash spur. |

Config at the workspace root:

```json
{
  "ignoredRepos": ["notes"],
  "maxDepth": 3
}
```

## Frames to shoot

Shoot these views only.

| Frame | Why |
| --- | --- |
| Tree + file diff | Focus a dirty file on `app`. The right pane loads the unified diff. |
| Graph: merge / stash / HEAD | Focus `merger`. The graph shows the merge, HEAD, and stash spur. |
| Graph: worktree | Focus `app` or `app/.worktrees/feat-login`. The graph shows the linked checkout. |
| Help | `?` opens the short key list. |
| Search | `/` then Enter. `n` / `N` step matches. |
| Stash menu | `S` on `merger`. |
| Branch picker | `b` on `merger` or `app`. |
| Reviewed | Space on a dirty file. Glyph is `◉` / `*`, not the clean `✓`. |
| Show ignored | `.` so `notes` appears. |
| Commit-files drill | Enter on a `merger` graph commit, Enter on a file, Esc pops. |
| EasyMotion | `;` or Ctrl+Space on the focused list. |

Existing stills in [the root README](../README.md#screenshots) cover these frames.
Do not duplicate them unless you replace a shot from this seed.

## Skip

Do not shoot these:

- In-flight fetch, pull, or push
- `y` / `n` confirms
- Create-branch prompt
- Every theme (`T` cycles themes in the session only)
- Watch poll

## Video

Use the same seed. Follow [../scripts/demo-shots.md](../scripts/demo-shots.md)
for the key sequence of each frame.

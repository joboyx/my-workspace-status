# Demo shot list

Reproduce each frame from a freshly seeded workspace.
Seed first:

    ./scripts/seed-demo-workspace.sh
    cd tmp/demo-workspace
    workspace-status

Keys match [docs/tui-rust.md](../docs/tui-rust.md).
Theme: Tokyo Night (`WS_STATUS_THEME` unset, or `tokyo-night`).
`T` cycles themes. Do not shoot every theme.

`j` / `k` move. `z` / `h` / `l` fold. `q` quits. `r` reloads.

## 1. Tree + file diff

1. Seed and launch the TUI.
2. `j` / `k` until `app` is focused, then open it (`l` if folded).
3. Focus `src/checkout.ts` (unstaged `M`).
4. Right pane: unified file diff for checkout pickup copy.

## 2. Graph: merge, stash, HEAD

1. `j` / `k` to `merger`.
2. Right pane: graph with the merge commit, HEAD on `feature/release-cut`, and a stash spur (`◇`).

## 3. Graph: worktree

1. Focus `app`, then `app/.worktrees/feat-login` (or search `/login`).
2. Right pane: graph for the linked login checkout.

## 4. Help

1. Press `?`.
2. Overlay: short key list. Esc closes.

## 5. Search

1. Press `/`.
2. Type `checkout`. Enter arms the query.
3. `n` / `N` step matches. Rows stay visible.

## 6. Stash menu

1. Focus `merger`.
2. Press `S`.
3. Overlay: create / apply / pop / drop. Esc closes. Do not confirm.

## 7. Branch picker

1. Focus `merger` or `app`.
2. Press `b`.
3. Overlay: local branches. Type to filter. Esc closes. Do not press `C`.

## 8. Reviewed

1. Focus a dirty file on `app` (`src/checkout.ts` or `src/app.ts`).
2. Press Space.
3. Glyph becomes `◉` (or `*` in ASCII). This is not the clean `✓` on **No updates**.

## 9. Show ignored

1. From the default tree, `notes` is absent.
2. Press `.`.
3. `notes` appears (dirty standup file). Press `.` again to hide.

## 10. Commit-files drill

1. Focus `merger` so the graph is in the right pane.
2. `j` / `k` on the graph to the merge commit.
3. Enter opens the commit file list.
4. Enter on a file opens that file's commit diff.
5. Esc pops one level. Esc again returns to the graph.

## 11. EasyMotion

1. Focus the tree (or the `merger` graph).
2. Press `;` or Ctrl+Space.
3. Labels `a`-`z` appear on the current viewport. Type a label to jump. Esc cancels.

## Skip

Do not record:

- `f` / `p` / `P` in flight
- `y` / `n` on revert, drop, pull, or worktree remove
- Create-branch prompt from `b` then `C`
- Every theme
- Watch poll (`WS_STATUS_WATCH_MS`)

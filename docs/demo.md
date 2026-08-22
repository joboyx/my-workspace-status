# Demo workspace

Seed one workspace, then capture the stills below.

## Seed

From the repository root:

    ./scripts/seed-demo-workspace.sh

The script wipes and recreates `tmp/demo-workspace`.
Pass a directory as the first argument to seed somewhere else.
Local remotes live under `DEST/.remotes`. Scratch clones live under `DEST/.scratch`.
Both go away when DEST is wiped.

`tmp/` is gitignored. Do not commit the seed output.

## Launch

    cd tmp/demo-workspace
    unset NO_COLOR FORCE_COLOR
    WS_STATUS_WATCH_MS=0 WS_STATUS_FETCH_MS=0 workspace-status

- Theme: default Tokyo Night. Do not press `T`.
- Font: `MesloLGS NF` (romkatv/powerlevel10k-media). Do not set `WS_STATUS_GLYPHS=ascii` when that font is present. Set it only if the font is missing. Do not use MesloLGM Nerd Font Mono — it letter-spaces in xfce4-terminal (VTE sizes cells off the widest Nerd glyph).
- Some hosts export `NO_COLOR=1`, which paints the first frame gray. Unset `NO_COLOR` and `FORCE_COLOR` before launch.
- Terminal: at least 140x40. Side-by-side diff needs 100 or more columns. Stay in the default inline diff for stills.
- Watch and background fetch stay off so frames do not flicker.
- Re-run the seed script after any write (`s` / `u` / `x`, stash apply/pop/drop, checkout, reviewed mark).
- Reviewed marks live in `$XDG_STATE_HOME/my-workspace-status/viewed-files.json` (fallback `~/.local/state/my-workspace-status/viewed-files.json`). Delete that file if a `◉` survives a reseed.

Each shot starts from a fresh launch unless noted. The first cursor is `app` → `src/auth.ts` (unstaged `M`) with the file diff on the right.

## Workspace

| Path | State |
| --- | --- |
| `app` | Dirty `feature/auth-refresh`, ahead of origin. Staged `session.ts`, unstaged `auth.ts`, untracked `login.ts`. Linked worktree at `app/.worktrees/feat-login`. |
| `services/api` | Dirty `feature/rate-limit`, diverged from origin. |
| `lib` | Clean `main`. Folds under No updates. |
| `notes` | Dirty and listed in `ignoredRepos`. Hidden until `.` or `-a`. |
| `merger` | `feature/reconciliation` with a merge commit and a stash. |

## 01 — tree + file diff

Focus stays on `src/auth.ts`. The right pane is the unified diff (refresh window `5m` → `2m`, plus `withRefreshedExpiry`).

Keys: none (or `g` then `j` onto `auth.ts` if the cursor moved).

Show: dirty `app` files (`M` / `A` / staged `session.ts`), linked `feat-login`, `services/api`, folded No updates, and the auth diff.

## 02 — git graph

Focus `merger` so the right pane is the graph: merge elbows, stash `◇`, HEAD `⊙`.

Keys: `/` `merger` Enter. If the cursor landed on a file, `k` to the repo row.

Show: `merge billing into main` join, `stash@{0}` diamond + short spur, `feature/reconciliation` HEAD.

## 03 — help

Keys: `?`

Show: the short key overlay, not a wall of text. Tree + pane still visible behind it.

## 04 — search then next

Keys: `/` `auth` Enter, then `n`.

Show: search prompt / armed query, match highlight on `auth.ts` (and the next hit after `n`). Do not hide rows.

## 05 — stash menu

Focus `merger` (has `stash@{0}`) or `app` (also has one).

Keys: `/` `merger` Enter, then `S`.

Show: stash overlay (`s` create, `a` apply, `p` pop, `d` drop). Do not confirm apply/pop/drop.

## 06 — branch picker

Focus `app`.

Keys: `g`, `k` onto the `app` repo row if needed, then `b`.

Show: local branches (`main`, `feature/auth-refresh`). Type-to-filter is optional. Do not press `C` (create-branch is a skip).

## 07 — reviewed mark

Focus `src/auth.ts`.

Keys: Space.

Show: viewed glyph `◉` / `*` on that dirty file. Not the clean `✓`.

## 08 — show ignored

Keys: `.`

Show: ignored dirty `notes` (`inbox.md`) entering the tree. Press `.` again only if you need the hidden-state contrast. The still is the revealed tree.

## 09 — commit-files drill

Focus `merger`, move the graph cursor onto a commit (the merge or `Start reconciliation job`), then drill.

Keys: `/` `merger` Enter, `j`/`k` to the commit, Enter.

Show: commit file list in the right pane. Do not Enter again into a file diff unless you want a second crop. The named still is the file list.

## 10 — EasyMotion

Focus the tree (Esc until the left pane is active).

Keys: `;`

Show: `a`–`z` labels on the current viewport. Do not type a label (that jumps and dismisses). Esc cancels after the shot.

## Skip as stills

- Fetch / pull / push in-flight (`f`, `p`, `P`)
- `y`/`n` confirms (revert, pop/drop, worktree remove, out-of-sync checkout)
- Create-branch prompt (`b` then `C`)
- Theme cycle (`T`) — stay on Tokyo Night
- Watch poll (already disabled)

# Demo workspace

Seed one workspace, then capture the stills below.

Refresh README/demo PNGs from the repo root:

    ./scripts/capture-demo-stills.sh

That script seeds, installs MesloLGS NF if needed, starts Xvfb and Openbox through `scripts/with-desktop-session.sh`, and types the hardcoded keys below. Do not invent a fixture or a second capture pipeline.

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
- Font: `MesloLGS NF` 13 (romkatv/powerlevel10k-media). Do not set `WS_STATUS_GLYPHS=ascii` when that font is present. Set it only if the font is missing. Do not use MesloLGM Nerd Font Mono — it letter-spaces in xfce4-terminal (VTE sizes cells off the widest Nerd glyph).
- Graph dates: operator local timezone (relative through 3 hours, then `YYYY-MM-DD HH:MM`). Seed timestamps are Asia/Manila (UTC+8). `capture-demo-stills.sh` sets `TZ=Asia/Manila` so stills match that clock.
- Some hosts export `NO_COLOR=1`, which paints the first frame gray. Unset `NO_COLOR` and `FORCE_COLOR` before launch.
- Terminal: at least 140x40. Side-by-side diff needs 100 or more columns. Stay in the default inline diff for stills.
- Watch and background fetch stay off so frames do not flicker.
- Re-run the seed script after any write (`s` / `u` / `x`, stash apply/pop/drop, checkout, reviewed mark).
- Reviewed marks live in `$XDG_STATE_HOME/my-workspace-status/viewed-files.json` (fallback `~/.local/state/my-workspace-status/viewed-files.json`). Delete that file if a `` / `*` survives a reseed. `capture-demo-stills.sh` points `WS_STATUS_VIEWED_STORE` and `WS_STATUS_UPDATE_CHECK_STORE` at `tmp/demo-stills-stage/state` so it does not write those operator files.

Each shot starts from a fresh launch unless noted. The first cursor is `app` → `src/auth.ts` (unstaged `M`) with the file diff on the right.

## Workspace

| Path | State |
| --- | --- |
| `app` | Dirty `feature/auth-refresh`, ahead of origin. Staged `session.ts`, unstaged `auth.ts`, untracked `login.ts`. Linked worktree at `app/.worktrees/feat-login`. |
| `services/api` | Dirty `feature/rate-limit`, diverged from origin. |
| `lib` | Clean `main`. Folds under No updates. |
| `notes` | Dirty and listed in `ignoredRepos`. Hidden until `.` or `-a`. |
| `merger` | `feature/reconciliation` with a merge commit, a stash, and a linked worktree at `merger/.worktrees/recon` on the same branch. |

## 01 — tree + file diff

Focus stays on `src/auth.ts`. The right pane is the unified diff (refresh window `5m` → `2m`, plus `withRefreshedExpiry`).

Keys: none. Fresh launch leaves the cursor on `auth.ts`.

Show: dirty `app` files (`M` / `A` / staged `session.ts`), linked `feat-login`, `services/api`, folded No updates, and the auth diff.

## 02 — git graph

Focus `merger` so the right pane is the graph: merge elbows, stash `◇`, HEAD `⊙`.

Keys: `/` `merger` Enter. Do not press `k` after.

Show: `merge billing into main` join, `stash@{0}` diamond + short spur, `feature/reconciliation` HEAD.

## 03 — help

Keys: `?`

Show: the short key overlay, not a wall of text. Tree + pane still visible behind it.

## 04 — search

Keys: `/` `auth` Enter.

Show: armed `/auth`, match highlight on `auth.ts`. Rows stay visible.

## 05 — boxed confirm

Focus `merger`, move the graph cursor onto `stash@{0}`, then drop.

Keys: `/` `merger` Enter, Tab, `j` onto the stash diamond, `D`. Do not press `y`.

Show: rounded boxed overlay `Drop stash@{0}?` with `y` drop / `n` cancel.

## 06 — stash menu

Tree `S` on dirty `app` is create-only (`s` stash). Full apply / pop / drop needs a graph-focused stash, then `S` (or `a` / `p` / `D`).

Keys (create-only still): `S` on dirty `app` (fresh launch is already on `auth.ts` under `app`).

Show: `Stash app` overlay, `s` create, Esc cancel. Do not confirm.

## 07 — reviewed mark

Focus `src/auth.ts`.

Keys: Space.

Show: viewed glyph `` / `*` (`ICON_VIEWED`) on that dirty file, trailing before the status badge. Not the clean `` / `.`.

## 08 — show ignored

Keys: `.`

Show: ignored dirty `notes` (`inbox.md`) entering the tree. Press `.` again only if you need the hidden-state contrast. The still is the revealed tree.

## 09 — commit-files drill

Focus `merger`, move the graph cursor onto a commit (the merge or `Start reconciliation job`), then drill.

Keys: `/` `merger` Enter, Tab, `j` to a commit, Enter.

Show: commit file list in the right pane. Do not Enter again into a file diff unless you want a second crop. The named still is the file list.

## Skip as stills

- Fetch / pull / push in-flight (`f`, `p`, `P`)
- Completing a confirm with `y` (shot 05 shows the overlay; do not press `y`)
- Create-branch prompt (`b` then `C`) — not a README still
- Theme cycle (`T`) — stay on Tokyo Night
- Watch poll (already disabled)

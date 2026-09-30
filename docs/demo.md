# Demo workspace

Seed one workspace, then record the animated GIF clips below.

Refresh the README/demo GIFs from the repo root:

    ./scripts/capture-demo-stills.sh

That script seeds, installs MesloLGS NF and ffmpeg if needed, starts Xvfb and Openbox through `scripts/with-desktop-session.sh`, and types the hardcoded keys below. ffmpeg `x11grab` records only the terminal window at 10 fps. The script then encodes each clip to `docs/images/NN-name.gif` (`palettegen` / `paletteuse`, loops forever, no downscale). Do not invent a fixture or a second capture pipeline.

The script rejects a clip and keeps the old GIF when the last frame is gray or too small, when no frame differs from the first (the keys did nothing), or when a GIF is over 4 MB.

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
- Graph dates: operator local timezone (relative through 3 hours, then `YYYY-MM-DD HH:MM`). Seed timestamps are Asia/Manila (UTC+8). `capture-demo-stills.sh` sets `TZ=Asia/Manila` so clips match that clock.
- Some hosts export `NO_COLOR=1`, which paints the first frame gray. Unset `NO_COLOR` and `FORCE_COLOR` before launch.
- Terminal: at least 140x40. Side-by-side diff needs 100 or more columns. Stay in the default inline diff for clips.
- Watch and background fetch stay off so frames do not flicker.
- Re-run the seed script after any write (`s` / `u` / `x`, stash apply/pop/drop, checkout, reviewed mark).
- Reviewed marks live in `$XDG_STATE_HOME/my-workspace-status/viewed-files.json` (fallback `~/.local/state/my-workspace-status/viewed-files.json`). Delete that file if a `` / `*` survives a reseed. Comments live in `$XDG_STATE_HOME/my-workspace-status/comments.json`. `capture-demo-stills.sh` points `WS_STATUS_VIEWED_STORE`, `WS_STATUS_COMMENT_STORE`, and `WS_STATUS_UPDATE_CHECK_STORE` at `tmp/demo-stills-stage/state` so it does not write those operator files.

Each clip starts from a fresh launch. The first cursor is `app` → Staged → `src/session.ts` (`S`) with its diff on the right. Recording starts after the first paint. The script holds about 1 s before the first key, about 1 s after each step, and about 2 s at the end.

## Workspace

| Path | State |
| --- | --- |
| `app` | Dirty `feature/auth-refresh`, ahead of origin. Staged `session.ts`, unstaged `auth.ts`, untracked `login.ts`. Linked worktree at `app/.worktrees/feat-login`. |
| `services/api` | Dirty `feature/rate-limit`, diverged from origin. |
| `lib` | Clean `main`. Folds under No updates. |
| `notes` | Dirty and listed in `ignoredRepos`. Hidden until `.` or `-a`. |
| `merger` | `feature/reconciliation` with a merge commit, a stash, and a linked worktree at `merger/.worktrees/recon` on the same branch. |

## 01 — tree + live diff

Keys: `j` `j` `j` `j` `k`.

Show: the cursor walks from `session.ts` over the `Changes` and `src` folders to `auth.ts` and `login.ts`, then back to `auth.ts`. The right pane follows: diff on a file, graph on a folder. The clip ends on the `auth.ts` diff (refresh window `5m` → `2m`, plus `withRefreshedExpiry`).

## 02 — git graph + commit files

Keys: `/` `merger` Enter, Tab, `j` `j`, Enter.

Show: `merger` graph with the `merge billing into main` join, `stash@{0}` diamond + short spur, and `feature/reconciliation` HEAD. Tab focuses the graph, `j` passes the stash onto `Start reconciliation job`, and Enter opens its file list (`reconcile.ts`).

## 03 — unstage / stage

Keys: `u`, pause, `s`.

Show: `session.ts` leaves Staged and shows as unstaged `M`, then goes back to Staged `S`. Row flashes mark each move. Real git writes: the script reseeds after this clip.

## 04 — search

Keys: `/` `auth` (typed), Enter, `n`.

Show: matches highlight while the query is typed. Enter arms `/auth` on the `feature/auth-refresh` row. `n` steps to `auth.ts` and its diff. Rows stay visible.

## 05 — reviewed marks

Keys: `j` `j` `j` before recording (cursor on `auth.ts`), then Space, `j`, Space.

Show: viewed glyph `` / `*` (`ICON_VIEWED`) on `auth.ts`, then on `login.ts`, before the status badge. Not the clean `` / `.`. The script clears the viewed store after this clip.

## 06 — stash

Tree `S` on dirty `app` is create-only (`s` stash). Apply / pop / drop needs a graph-focused stash, then `S` (or `a` / `p` / `D`).

Keys: `S`, Esc, `/` `merger` Enter, Tab, `j` onto the stash diamond, `D`, `n`. Never press `y`.

Show: `Stash app` overlay (`s` create, Esc cancel), then the rounded boxed `Drop stash@{0}?` confirm with `y` drop / `n` cancel, then `drop cancelled`.

## 07 — show ignored

Keys: `.`, pause, `.`.

Show: ignored dirty `notes` (`inbox.md`) enters the tree, then hides again.

## 08 — help

Keys: `?`, hold, Esc.

Show: the short MOVE / GIT / VIEW key overlay over the tree and pane, then back to the tree.

## Skip as clips

- Fetch / pull / push in-flight (`f`, `p`, `P`)
- Completing a confirm with `y` (clip 06 shows the overlay and cancels with `n`)
- Create-branch prompt (`b` then `C`) — not a README clip
- Theme cycle (`T`) — stay on Tokyo Night
- Watch poll (already disabled)
- `V` line-range stage — the PTY e2e proves it on a separate two-hunk fixture, not on this seed

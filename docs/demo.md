# Demo workspace

Seed one workspace, then record the animated GIF clips below.

Refresh the README/demo GIFs from the repo root:

    ./scripts/capture-demo-stills.sh

That script seeds, installs MesloLGS NF and ffmpeg if needed, starts Xvfb and Openbox through `scripts/with-desktop-session.sh`, and types the hardcoded keys below. ffmpeg `x11grab` records only the terminal window at 10 fps. The script then encodes each clip to `docs/images/NN-name.gif` (`palettegen` / `paletteuse`, loops forever, no downscale). Do not invent a fixture or a second capture pipeline.

Each clip also takes full-colour PNG stills from the same recording (not from the GIF palette): `NN-name.png` is the last frame, and `NN-name-mid.png` is the middle of the settled hold after the step where the clip calls `mark_mid` (the **Mid still** line below). The mid offset subtracts ffmpeg's start-up delay. Stills are lossless-optimized when `optipng` is installed.

The script rejects a clip and keeps the old GIF when the last or mid frame is gray or too small, when no frame differs from the first (the keys did nothing), or when a GIF is over 4 MB. A rejected clip stops the run. Stills go to `tmp/demo-stills-stage/stills` first and replace `docs/images/stills/` as one set only when every clip passes.

The script runs Xvfb on `:99`. Set `WS_STATUS_STILLS_DISPLAY` to another number when `:99` is busy. It stops only the `xfce4-terminal` it started on that display.

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
- Terminal: at least 140x40. The default diff mode is split, but split needs a diff pane of 100 or more columns, so at 140 columns the diff paints `inline (too narrow)`. Only clip 13 widens the pane with `<`. Do not press `i`.
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

Mid still: after `u` (`unstaged src/session.ts`).

## 04 — search

Keys: `/` `auth` (typed), Enter, `n`.

Show: matches highlight while the query is typed. Enter jumps from the `session.ts` cursor to the first match after it, `auth.ts`, and arms `/auth` (chip `/auth 2/2 · tree`). `n` wraps to the `feature/auth-refresh` row (`search wrapped to top`, `1/2`). Rows stay visible.

## 05 — reviewed marks

Keys: `j` `j` `j` before recording (cursor on `auth.ts`), then Space, `j`, Space.

Show: viewed glyph `` / `*` (`ICON_VIEWED`) on `auth.ts`, then on `login.ts`, before the status badge. Not the clean `` / `.`. The script clears the viewed store after this clip.

## 06 — stash

Tree `S` on dirty `app` is create-only (`s` stash). Apply / pop / drop needs a graph-focused stash, then `S` (or `a` / `p` / `D`).

Keys: `S`, Esc, `/` `merger` Enter, Tab, `j` onto the stash diamond, `D`, `n`. Never press `y`.

Show: `Stash app` overlay (`s` stash, `Esc` cancel), then the boxed `Drop stash@{0}?` confirm with `y` drop / `n` `Esc` cancel, then `drop cancelled`.

Mid still: the drop confirm.

## 07 — show ignored

Keys: `.`, pause, `.`.

Show: ignored dirty `notes` (`inbox.md`) enters the tree (`showing ignored repos`), then hides again.

Mid still: `notes` shown.

## 08 — help

Keys: `?`, hold, Esc.

Show: the MOVE / GIT / VIEW key overlay (each column flows on its own, version in the footer) over the tree and pane, then back to the tree.

Mid still: the help overlay.

## 09 — commands (`:`)

Keys: `:`, `compare` (typed), Backspace ×7, `checkout` (typed), Esc.

Show: the commands list. The alias `compare` finds Diff vs default in new tab / Diff vs branch in new tab… / Diff vs commit in new tab… / Diff commit vs parent in new tab / Close tab (with its disabled reason). `checkout` types its `c` and `k` (letters no longer move the cursor) and finds Branch picker / Checkout commit refs with their disabled reasons. Esc closes with no run.

Mid still: the `compare` alias match.

## 10 — branch picker

Keys: `k` `k` `k` before recording (cursor on the `app` checkout row `feature/auth-refresh`), then `b`, `login` (typed), Backspace ×5, `fix/banner` (typed), Esc.

Show: the `Branch app` picker. `login` filters to `feature/login-page` plus a `+ create branch login` row. `fix/banner` matches nothing and leaves only `+ create branch fix/banner`. Esc closes with `branch cancelled`: nothing is created or checked out.

Mid still: the `+ create branch fix/banner` row.

## 11 — confirm

Keys: `j` `j` `j` before recording (cursor on `auth.ts`), then `x`, Enter, `n`. Never press `y`.

Show: the boxed `Revert src/auth.ts?` confirm (`1 tracked file → discarded`, chips `y` revert / `n` `Esc` cancel). Enter does not confirm: the status says `press y to confirm · n or Esc to cancel`. `n` closes with `revert cancelled`.

Mid still: the confirm after Enter.

## 12 — blocked keys say why

Keys: Tab, `s`, Tab, `/` `zzz` (typed), Enter, Esc.

Show: `s` with the diff pane focused stages nothing and says `focus the tree (Tab) to stage`. Back on the tree, `/zzz` Enter says `no match` with the chip `/zzz -/0 · tree`. Esc says `search cleared`.

Mid still: `focus the tree (Tab) to stage`.

## 13 — pane resize and split diff

Keys: `<` `<` `<`, `>`.

Show: each `<` narrows the tree pane by 5%. At the third press the diff pane reaches 100 columns, the header turns from `inline (too narrow)` to `split`, and the pill from `split→inline` to `split`. `>` widens the tree once and the diff goes back to inline.

Mid still: the split diff.

## 14 — search position

Keys: `/` `auth` (typed), Enter, `n`, `n`.

Show: the armed chip `/auth 2/2 · tree` (position, count, bound pane) on `auth.ts`, the first match after the `session.ts` cursor. `n` wraps to `1/2` on the `feature/auth-refresh` row with `search wrapped to top`, then the next `n` returns to `2/2`.

Mid still: `/auth 1/2` after the wrap.

## Skip as clips

- Fetch / pull / push in-flight (`f`, `p`, `P`)
- Completing a confirm with `y` (clips 06 and 11 show the overlay and cancel with `n`)
- Creating a branch (Enter on the `+ create branch` row, or graph `c`) — clip 10 shows the row and closes with Esc
- Theme cycle (`T`) — stay on Tokyo Night
- Watch poll (already disabled)
- `V` line-range stage — the PTY e2e proves it on a separate two-hunk fixture, not on this seed
